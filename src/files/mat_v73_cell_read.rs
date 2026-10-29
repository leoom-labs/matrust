//! Character/cell reference schema; bounded metadata traversal precedes payload allocation.
use super::{MatVariable, h, load_matrix, metadata, tiles};
use crate::{Array, CellArray, CharacterRow, Result, dsp::check_cancel};
use hdf5::{Dataset, File, ObjectReference, ObjectReference1, ReferencedObject};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
const MAX_SLOTS: usize = 65536;
// Root variables and recursively reached targets have a separate object bound.
const MAX_OBJECTS: usize = 10000 + MAX_SLOTS;
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_VISITS: usize = 262144;

// The locked hdf5-metno 0.15 ObjectReference1 is repr(transparent) over
// hobj_ref_t (the legacy file-local object address), with its exact H5Type.
// Read its scalar address only; never dereference this address as a pointer.
fn key(file: &File, dataset: &Dataset) -> Result<u64> {
    if h(dataset.loc_info())?.fileno != h(file.loc_info())?.fileno {
        return Err("Cell reference belongs to another HDF5 file".into());
    }
    // Bound the native path length before the high-level name allocation.
    let name_len = hdf5::sync::sync(|| unsafe {
        hdf5_sys::h5i::H5Iget_name(dataset.id(), std::ptr::null_mut(), 0)
    });
    if !(1..=4096).contains(&name_len) {
        return Err("Invalid cell target path length".into());
    }
    let name = dataset.name();
    if name.is_empty() || name.len() > 4096 {
        return Err("Invalid cell target path".into());
    }
    let reference: ObjectReference1 = h(file.reference(&name))?;
    let address = unsafe {
        reference
            .ptr()
            .cast::<hdf5_sys::h5r::hobj_ref_t>()
            .read_unaligned()
    };
    if address == 0 {
        return Err("Null cell object reference".into());
    }
    Ok(address)
}
fn children(
    _file: &File,
    dataset: &Dataset,
    info: &MatVariable,
    cancel: &AtomicBool,
) -> Result<Vec<ObjectReference1>> {
    check_cancel(cancel)?;
    let count = info.dimensions[0]
        .checked_mul(info.dimensions[1])
        .ok_or("Cell shape overflow")?;
    if count > MAX_SLOTS {
        return Err("Cell reference count exceeds 65536".into());
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    // Storage/type/chunk validation has already run in metadata(). This native
    // reference read is at most 512 KiB; no arbitrary payload is followed here.
    let references = h(dataset.read_raw::<ObjectReference1>())?;
    #[cfg(test)]
    super::read_probe::completed(cancel);
    check_cancel(cancel)?;
    if references.len() != count {
        return Err("Cell reference count changed".into());
    }
    check_cancel(cancel)?;
    Ok(references)
}
fn target(file: &File, reference: &ObjectReference1) -> Result<Dataset> {
    match h(file.dereference(reference))? {
        ReferencedObject::Dataset(target) => {
            if h(target.loc_info())?.fileno != h(file.loc_info())?.fileno {
                return Err("External cell reference".into());
            }
            Ok(target)
        }
        _ => Err("Cell reference must target a dataset".into()),
    }
}

// Reference unset-cell sentinel. Public mxGetCell returns null for it.
// MatRust has no null cell-slot value: import explicitly maps this exact sentinel
// to the same immutable numeric 0x0 value used by CellArray::empty. This is an
// importer policy, not evidence of MATLAB engine class/size behavior.
fn node_metadata(dataset: &Dataset, depth: usize) -> Result<MatVariable> {
    let mut info = metadata(dataset, "cell child")?;
    if info.class != "canonical empty" || depth == 0 {
        return Ok(info);
    }
    super::storage(dataset)?;
    let empty = h(dataset.attr("MATLAB_empty"))?;
    if dataset.shape() != [2]
        || h(h(dataset.dtype())?.to_descriptor())?
            != hdf5::types::TypeDescriptor::Unsigned(hdf5::types::IntSize::U8)
        || empty.size() != 1
        || h(h(empty.dtype())?.to_descriptor())?
            != hdf5::types::TypeDescriptor::Unsigned(hdf5::types::IntSize::U1)
        || h(empty.read_scalar::<u8>())? != 1
        || info.dimensions != [0, 0]
        || dataset.chunk().is_some()
        || !dataset.filters().is_empty()
    {
        return Err("Unsupported canonical-empty cell sentinel".into());
    }
    // metadata() already validated and read the exact two uint64 zero dimensions.
    info.decoded_bytes = Some(0);
    info.unavailable = None;
    Ok(info)
}

#[derive(Default)]
struct Budget {
    bytes: usize,
    slots: usize,
}
impl Budget {
    fn charge(&mut self, bytes: usize, slots: usize) -> Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or("Cell byte overflow")?;
        self.slots = self.slots.checked_add(slots).ok_or("Cell slot overflow")?;
        if self.bytes > MAX_BYTES || self.slots > MAX_SLOTS {
            return Err("Cell exceeds 256 MiB logical storage or 65536 recursive slots".into());
        }
        Ok(())
    }
}
/// Expanded logical cost, not unique physical object size. No Dataset or payload
/// ownership is retained. Cache identity is valid only within `file`'s lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Summary {
    bytes: usize,
    slots: usize,
    visits: usize,
    height: usize,
}

pub(super) struct Inspector<'a> {
    file: &'a File,
    completed: BTreeMap<u64, Summary>,
    #[cfg(test)]
    probe: InspectionProbe,
}

#[cfg(test)]
#[derive(Default)]
struct InspectionProbe {
    metadata: usize,
    dereferences: usize,
    reference_reads: usize,
    hits: usize,
    capacity: Option<usize>,
    cancel_on_hit: bool,
    cancel_on_publish: bool,
}

fn reference_address(reference: &ObjectReference1) -> u64 {
    // Same locked ObjectReference1 ABI as key(): an integer address, not a pointer.
    unsafe {
        reference
            .ptr()
            .cast::<hdf5_sys::h5r::hobj_ref_t>()
            .read_unaligned()
    }
}

impl<'a> Inspector<'a> {
    pub(super) fn new(file: &'a File) -> Self {
        Self {
            file,
            completed: BTreeMap::new(),
            #[cfg(test)]
            probe: InspectionProbe::default(),
        }
    }

    pub(super) fn inspect(
        &mut self,
        dataset: &Dataset,
        cancel: &AtomicBool,
        visits: &mut usize,
    ) -> Result<usize> {
        check_cancel(cancel)?;
        let mut budget = Budget::default();
        let mut active = Vec::new();
        // A root lookup needs its own file-local proof. Failure is not a new
        // rejection: ordinary cell traversal retains the old ordered key check.
        let address = key(self.file, dataset).ok();
        if let Some(address) = address
            && self
                .hit(address, 0, &active, &mut budget, visits, cancel)?
                .is_some()
        {
            return Ok(budget.bytes);
        }
        self.walk(dataset, None, 0, &mut active, &mut budget, visits, cancel)?;
        check_cancel(cancel)?;
        Ok(budget.bytes)
    }

    fn hit(
        &mut self,
        address: u64,
        depth: usize,
        active: &[u64],
        budget: &mut Budget,
        visits: &mut usize,
        cancel: &AtomicBool,
    ) -> Result<Option<Summary>> {
        check_cancel(cancel)?;
        // For an active address, use the original walk rather than returning an
        // early cycle error: metadata/depth/visit errors must keep their priority.
        if address == 0 || active.contains(&address) {
            return Ok(None);
        }
        let Some(summary) = self.completed.get(&address).copied() else {
            return Ok(None);
        };
        let projected = depth
            .checked_add(summary.height)
            .zip(budget.bytes.checked_add(summary.bytes))
            .zip(budget.slots.checked_add(summary.slots))
            .zip(visits.checked_add(summary.visits));
        let Some((((levels, bytes), slots), count)) = projected else {
            return Ok(None);
        };
        if levels > 16 || bytes > MAX_BYTES || slots > MAX_SLOTS || count > MAX_VISITS {
            // Do not charge anything speculatively. Ordered fallback must reach
            // the same failing prefix, including its global visit side effects.
            return Ok(None);
        }
        #[cfg(test)]
        {
            self.probe.hits += 1;
            if self.probe.cancel_on_hit {
                self.probe.cancel_on_hit = false;
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        check_cancel(cancel)?;
        budget.bytes = bytes;
        budget.slots = slots;
        *visits = count;
        check_cancel(cancel)?;
        Ok(Some(summary))
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        dataset: &Dataset,
        reference: Option<u64>,
        depth: usize,
        active: &mut Vec<u64>,
        budget: &mut Budget,
        visits: &mut usize,
        cancel: &AtomicBool,
    ) -> Result<Summary> {
        check_cancel(cancel)?;
        let before = (budget.bytes, budget.slots, *visits);
        *visits = visits.checked_add(1).ok_or("Cell inspection overflow")?;
        if *visits > MAX_VISITS {
            return Err("MAT cell inspection exceeds 262144 visited objects".into());
        }
        #[cfg(test)]
        {
            self.probe.metadata += 1;
        }
        let info = node_metadata(dataset, depth)?;
        if let Some(reason) = &info.unavailable {
            return Err(reason.clone());
        }
        let (address, height) = if info.class != "cell" {
            budget.charge(
                usize::try_from(info.storage_bytes().ok_or("Unknown cell child size")?)
                    .map_err(|_| "Cell child size overflow")?,
                0,
            )?;
            // Original inspection admitted numeric leaves without a bounded
            // name-to-reference proof. A failed proof only disables admission.
            let address = key(self.file, dataset)
                .ok()
                .filter(|&address| reference.is_none_or(|expected| expected == address));
            (address, 0)
        } else {
            if depth >= 16 {
                return Err("Cell nesting exceeds 16 levels".into());
            }
            let address = key(self.file, dataset)?;
            if active.contains(&address) {
                return Err("Cyclic HDF5 cell references are not supported".into());
            }
            if reference.is_some_and(|expected| expected != address) {
                return Err("MAT cell reference identity changed during inspection".into());
            }
            let count = info.dimensions[0] * info.dimensions[1];
            budget.charge(
                count
                    .checked_mul(crate::containers::SLOT)
                    .ok_or("Cell slot byte overflow")?,
                count,
            )?;
            active.push(address);
            #[cfg(test)]
            {
                self.probe.reference_reads += usize::from(count != 0);
            }
            let mut height = 1;
            for reference in children(self.file, dataset, &info, cancel)? {
                check_cancel(cancel)?;
                let address = reference_address(&reference);
                let summary = match self.hit(address, depth + 1, active, budget, visits, cancel)? {
                    Some(summary) => summary,
                    None => {
                        #[cfg(test)]
                        {
                            self.probe.dereferences += 1;
                        }
                        // Zero, group and foreign references retain the original
                        // target validation and error ordering before node entry.
                        let child = target(self.file, &reference)?;
                        self.walk(
                            &child,
                            Some(address),
                            depth + 1,
                            active,
                            budget,
                            visits,
                            cancel,
                        )?
                    }
                };
                height = height.max(summary.height.checked_add(1).ok_or("Cell depth overflow")?);
            }
            active.pop();
            (Some(address), height)
        };
        let summary = Summary {
            bytes: budget.bytes - before.0,
            slots: budget.slots - before.1,
            visits: *visits - before.2,
            height,
        };
        #[cfg(test)]
        if self.probe.cancel_on_publish {
            self.probe.cancel_on_publish = false;
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        check_cancel(cancel)?;
        let capacity = MAX_OBJECTS;
        #[cfg(test)]
        let capacity = self.probe.capacity.unwrap_or(capacity).min(MAX_OBJECTS);
        if let Some(address) = address
            && self.completed.len() < capacity
        {
            self.completed.insert(address, summary);
        }
        Ok(summary)
    }
}

pub(super) struct Loader<'a> {
    file: &'a File,
    cancel: &'a AtomicBool,
    inspector: Inspector<'a>,
    // Only reached targets are retained. Repeated object references share the
    // same immutable Array and leaf storage; no matrix content scan is needed.
    memo: BTreeMap<u64, Arc<Array>>,
}
impl<'a> Loader<'a> {
    pub(super) fn new(file: &'a File, cancel: &'a AtomicBool) -> Self {
        Self {
            file,
            cancel,
            inspector: Inspector::new(file),
            memo: BTreeMap::new(),
        }
    }
    pub(super) fn load(&mut self, dataset: &Dataset, expected: &MatVariable) -> Result<Array> {
        let mut actual = metadata(dataset, &expected.name)?;
        if actual.class == "cell" && actual.unavailable.is_none() {
            let mut visits = 0;
            actual.decoded_bytes =
                Some(self.inspector.inspect(dataset, self.cancel, &mut visits)? as u64);
        }
        if actual != *expected {
            return Err("MAT container metadata changed since inspection".into());
        }
        if let Some(reason) = &actual.unavailable {
            return Err(reason.clone());
        }
        let mut remaining =
            usize::try_from(expected.storage_bytes().ok_or("Unknown container size")?)
                .map_err(|_| "Container byte overflow")?;
        if remaining > MAX_BYTES {
            return Err("Container exceeds workspace bound".into());
        }
        let value = self.node(dataset, 0, &mut Vec::new(), &mut remaining)?;
        if remaining != 0 {
            return Err("Container accounting changed during loading".into());
        }
        if value.bytes() as u64 != expected.storage_bytes().ok_or("Unknown container size")? {
            return Err("MAT container size changed while loading".into());
        }
        check_cancel(self.cancel)?;
        Ok(value.as_ref().clone())
    }
    fn node(
        &mut self,
        dataset: &Dataset,
        depth: usize,
        active: &mut Vec<u64>,
        remaining: &mut usize,
    ) -> Result<Arc<Array>> {
        check_cancel(self.cancel)?;
        let address = key(self.file, dataset)?;
        if active.contains(&address) {
            return Err("Cyclic cell references during load".into());
        }
        if let Some(value) = self.memo.get(&address) {
            *remaining = remaining
                .checked_sub(value.bytes())
                .ok_or("Container exceeds inspected budget")?;
            return Ok(value.clone());
        }
        // Include unfinished ancestors, which are inserted only after their children.
        if self.memo.len() + active.len() >= MAX_OBJECTS {
            return Err("Selected cell objects exceed 75536".into());
        }
        let info = node_metadata(dataset, depth)?;
        if let Some(reason) = &info.unavailable {
            return Err(reason.clone());
        }
        let rows = info.dimensions[0];
        let cols = info.dimensions[1];
        let charge = if info.class == "cell" {
            rows * cols * crate::containers::SLOT
        } else {
            usize::try_from(info.storage_bytes().ok_or("Unknown leaf size")?)
                .map_err(|_| "Leaf size overflow")?
        };
        *remaining = remaining
            .checked_sub(charge)
            .ok_or("Container exceeds inspected budget")?;
        let value = match info.class.as_str() {
            "canonical empty" => Array::Numeric(crate::Matrix::from_column_vec(0, 0, Vec::new())?),
            "char" => {
                let mut units = vec![0u16; rows * cols];
                tiles::<u16>(dataset, rows, cols, self.cancel, |offset, values| {
                    units[offset..offset + values.len()].copy_from_slice(values);
                    Ok(())
                })?;
                Array::Character(CharacterRow::from_column_major(
                    rows,
                    cols,
                    units,
                    self.cancel,
                )?)
            }
            "cell" => {
                if depth >= 16 {
                    return Err("Cell nesting exceeds 16 levels".into());
                }
                active.push(address);
                let mut slots = Vec::new();
                slots
                    .try_reserve_exact(rows * cols)
                    .map_err(|_| "Cannot allocate cell slots")?;
                for reference in children(self.file, dataset, &info, self.cancel)? {
                    check_cancel(self.cancel)?;
                    let child = target(self.file, &reference)?;
                    slots.push(self.node(&child, depth + 1, active, remaining)?);
                }
                active.pop();
                Array::Cell(CellArray::from_shared(rows, cols, slots, self.cancel)?)
            }
            _ => load_matrix(dataset, &info, self.cancel, true)?,
        };
        if self.memo.len() >= MAX_OBJECTS {
            return Err("Selected cell objects exceed 75536".into());
        }
        let value = Arc::new(value);
        self.memo.insert(address, value.clone());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn authentic_unset_reference_cancellation_rejects_then_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unset.mat");
        std::fs::write(
            &path,
            include_bytes!("../../tests/fixtures/character-cell-unset-v73.mat"),
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let catalog = crate::files::inspect_workspace(&path, &cancel).unwrap();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                super::super::read_probe::ARMED.with(|v| v.set(false));
            }
        }
        let _reset = Reset;
        super::super::read_probe::ARMED.with(|v| v.set(true));
        let selected = std::collections::BTreeSet::from(["AllUnset".to_owned()]);
        let error = crate::files::load_selected_arrays(&catalog, &selected, &cancel).unwrap_err();
        assert!(error.contains("cancel"));
        assert!(cancel.load(Ordering::Relaxed));
        assert!(!super::super::read_probe::ARMED.with(|v| v.get()));
        cancel.store(false, Ordering::Relaxed);
        let retry = crate::files::load_selected_arrays(&catalog, &selected, &cancel).unwrap();
        let Array::Cell(c) = &retry["AllUnset"] else {
            panic!("Cell lost")
        };
        assert_eq!((c.rows(), c.cols()), (2, 3));
        for i in 0..c.len() {
            let value = c.get_linear(i).unwrap();
            assert!(Arc::ptr_eq(c.get_linear(0).unwrap(), value));
            let Array::Numeric(a) = value.as_ref() else {
                panic!("Importer policy lost")
            };
            assert_eq!((a.rows(), a.cols()), (0, 0));
        }
    }
    #[test]
    fn active_reference_read_cancellation_rejects_then_retries_exact_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cell.mat");
        let cancel = AtomicBool::new(false);
        let leaf = Arc::new(Array::Character(
            CharacterRow::from_units(vec![0, 0xd800, 65], &cancel).unwrap(),
        ));
        let value: Array = CellArray::from_shared(1, 1024, vec![leaf; 1024], &cancel)
            .unwrap()
            .into();
        let source = BTreeMap::from([("C".to_owned(), value.clone())]);
        crate::files::save_array_workspace_v73(&path, &source, &cancel).unwrap();
        let catalog = crate::files::inspect_workspace(&path, &cancel).unwrap();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                super::super::read_probe::ARMED.with(|v| v.set(false));
            }
        }
        let _reset = Reset;
        super::super::read_probe::ARMED.with(|v| v.set(true));
        let selected = std::collections::BTreeSet::from(["C".to_owned()]);
        let error = crate::files::load_selected_arrays(&catalog, &selected, &cancel).unwrap_err();
        assert!(error.contains("cancel"));
        assert!(cancel.load(Ordering::Relaxed));
        assert!(!super::super::read_probe::ARMED.with(|v| v.get()));
        assert!(source["C"].same_snapshot(&value));
        cancel.store(false, Ordering::Relaxed);
        let retry = crate::files::load_selected_arrays(&catalog, &selected, &cancel).unwrap();
        let Array::Cell(c) = &retry["C"] else {
            panic!("Cell lost")
        };
        assert_eq!(c.len(), 1024);
        for i in 0..c.len() {
            assert!(Arc::ptr_eq(
                c.get_linear(0).unwrap(),
                c.get_linear(i).unwrap()
            ));
            let Array::Character(text) = c.get_linear(i).unwrap().as_ref() else {
                panic!("Character lost")
            };
            assert_eq!(text.units(), &[0, 0xd800, 65]);
        }
    }
}

#[cfg(test)]
#[path = "mat_v73_inspection_tests.rs"]
mod inspection_tests;
