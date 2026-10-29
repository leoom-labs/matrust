//! Bounded character/cell values. Numeric payloads remain shared Array objects.
use crate::{
    Array, Matrix, Result,
    script_containers::{CellTable, CharacterRow, MAX_CELL_SLOTS, MAX_CHARACTER_UNITS},
};
use std::sync::{Arc, atomic::AtomicBool};

const LIMIT: usize = 256 * 1024 * 1024;
pub(crate) const MAX_DEPTH: usize = 16;
pub(crate) const SLOT: usize = std::mem::size_of::<Arc<Array>>() + std::mem::size_of::<Array>();

impl CharacterRow {
    /// Bounded shape observation without allocating a UTF-16 row.
    pub(crate) fn text_shape(text: &str, cancel: &AtomicBool) -> Result<(usize, usize)> {
        crate::dsp::check_cancel(cancel)?;
        if text.len() > MAX_CHARACTER_UNITS * 3 {
            return Err("Character row exceeds bounded UTF-16 size".into());
        }
        let mut count = 0usize;
        for _ in text.encode_utf16() {
            if count.is_multiple_of(1024) {
                crate::dsp::check_cancel(cancel)?;
            }
            if count == MAX_CHARACTER_UNITS {
                return Err("Character row exceeds 65536 UTF-16 units".into());
            }
            count += 1;
        }
        crate::dsp::check_cancel(cancel)?;
        Ok((usize::from(count != 0), count))
    }
    pub fn from_text(text: &str, cancel: &AtomicBool) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        if text.len() > MAX_CHARACTER_UNITS * 3 {
            return Err("Character row exceeds bounded UTF-16 size".into());
        }
        let mut units = Vec::new();
        units
            .try_reserve_exact(text.len().min(MAX_CHARACTER_UNITS))
            .map_err(|_| "Unable to allocate character row")?;
        for (i, unit) in text.encode_utf16().enumerate() {
            if i.is_multiple_of(1024) {
                crate::dsp::check_cancel(cancel)?;
            }
            if units.len() == MAX_CHARACTER_UNITS {
                return Err("Character row exceeds 65536 UTF-16 units".into());
            }
            units.push(unit);
        }
        Self::from_units(units, cancel)
    }
    pub fn len(&self) -> usize {
        self.units().len()
    }
    pub fn is_empty(&self) -> bool {
        self.units().is_empty()
    }
    /// Inspection never loses invalid code units; non-ASCII/control units use an
    /// escaped UTF-16 representation, independent of UI font availability.
    pub fn preview(&self, limit: usize) -> String {
        use std::fmt::Write;
        let mut output = if self.is_text_row() {
            String::from("'")
        } else {
            format!("char {}x{} column-major units [", self.rows(), self.cols())
        };
        for &unit in self.units().iter().take(limit.min(256)) {
            match unit {
                39 => output.push_str("''"),
                32..=126 => output.push(char::from_u32(u32::from(unit)).unwrap()),
                _ => {
                    let _ = write!(output, "\\u{unit:04X}");
                }
            }
        }
        if self.len() > limit.min(256) {
            output.push_str("...");
        }
        output.push(if self.is_text_row() { '\'' } else { ']' });
        output
    }
}
impl std::fmt::Display for CharacterRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.preview(128))
    }
}

#[derive(Clone, Debug)]
pub struct CellArray {
    table: CellTable<Array>,
    bytes: usize,
    // Both are checked against 256 MiB / 65,536 before narrowing. Packing
    // keeps the 64-bit enum-slot layout unchanged; no metadata allocation hides here.
    retained_bytes: u32,
    nodes: u32,
    depth: usize,
}
pub(crate) fn contribution(value: &Array) -> (usize, usize, usize) {
    match value {
        Array::Cell(a) => (a.bytes, a.nodes as usize, a.depth),
        Array::Struct(a) => (a.bytes(), a.nodes(), a.depth()),
        _ => (value.bytes(), 0, 0),
    }
}
pub(crate) fn checked_totals(bytes: usize, nodes: usize, depth: usize) -> Result<()> {
    if bytes > LIMIT || nodes > MAX_CELL_SLOTS || depth > MAX_DEPTH {
        return Err(
            "Cell value exceeds 256 MiB logical payload, 65536 total slots or 16 nesting levels"
                .into(),
        );
    }
    Ok(())
}
impl CellArray {
    pub fn empty(rows: usize, cols: usize, cancel: &AtomicBool) -> Result<Self> {
        let n = rows.checked_mul(cols).ok_or("Cell shape overflow")?;
        checked_totals(n.checked_mul(SLOT).ok_or("Cell byte overflow")?, n, 1)?;
        let value = Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?));
        let table = CellTable::filled(rows, cols, value, cancel)?;
        Ok(Self {
            table,
            bytes: n * SLOT,
            retained_bytes: u32::try_from(n * SLOT).map_err(|_| "Cell retained bytes overflow")?,
            nodes: u32::try_from(n).map_err(|_| "Cell node overflow")?,
            depth: 1,
        })
    }
    pub fn from_shared(
        rows: usize,
        cols: usize,
        slots: Vec<Arc<Array>>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        let count = rows.checked_mul(cols).ok_or("Cell shape overflow")?;
        if rows > MAX_CELL_SLOTS
            || cols > MAX_CELL_SLOTS
            || count > MAX_CELL_SLOTS
            || count != slots.len()
            || slots.capacity() > MAX_CELL_SLOTS
        {
            return Err("Cell shape, payload or capacity exceeds 65536 slots".into());
        }
        let mut bytes = slots.len().checked_mul(SLOT).ok_or("Cell byte overflow")?;
        let mut retained_bytes = bytes;
        let mut nodes = slots.len();
        let mut depth = 1;
        for (i, value) in slots.iter().enumerate() {
            if i.is_multiple_of(1024) {
                metadata_checkpoint(i, cancel)?;
            }
            retained_bytes = retained_bytes
                .checked_add(value.retained_bytes())
                .ok_or("Cell retained bytes overflow")?;
            let (b, n, d) = contribution(value);
            bytes = bytes.checked_add(b).ok_or("Cell byte overflow")?;
            nodes = nodes.checked_add(n).ok_or("Cell node overflow")?;
            depth = depth.max(d + 1);
            checked_totals(bytes, nodes, depth)?;
        }
        checked_totals(bytes, nodes, depth)?;
        if retained_bytes > LIMIT {
            let slots = crate::array::coordinate_retention::strip_slots(slots, cancel)?;
            return Self::from_shared(rows, cols, slots, cancel);
        }
        let table = CellTable::from_column_major(rows, cols, slots, cancel)?;
        crate::dsp::check_cancel(cancel)?;
        Ok(Self {
            table,
            bytes,
            retained_bytes: u32::try_from(retained_bytes)
                .map_err(|_| "Cell retained bytes overflow")?,
            nodes: u32::try_from(nodes).map_err(|_| "Cell node overflow")?,
            depth,
        })
    }
    pub fn rows(&self) -> usize {
        self.table.rows()
    }
    pub fn cols(&self) -> usize {
        self.table.cols()
    }
    pub fn len(&self) -> usize {
        self.table.len()
    }
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes as usize
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        self.table.same_storage(&other.table)
    }
    pub(crate) fn weak_storage(&self) -> std::sync::Weak<Vec<Arc<Array>>> {
        self.table.weak_storage()
    }
    pub fn get_linear(&self, index: usize) -> Result<&Arc<Array>> {
        if index >= self.len() {
            return Err("Cell content index exceeds shape".into());
        }
        self.table.get(index % self.rows(), index / self.rows())
    }
    pub fn reshape(&self, rows: usize, cols: usize, cancel: &AtomicBool) -> Result<Self> {
        Ok(Self {
            table: self.table.reshape(rows, cols, cancel)?,
            bytes: self.bytes,
            retained_bytes: self.retained_bytes,
            nodes: self.nodes,
            depth: self.depth,
        })
    }
    pub fn replaced(&self, index: usize, value: Array, cancel: &AtomicBool) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        self.get_linear(index)?;
        // Preflight the complete future logical value before allocating slots.
        // A single replaced slot may lower maximum depth, so inspect metadata of
        // every immediate child; cached descendants make this O(top-level slots).
        let mut bytes = self.len() * SLOT;
        let mut retained_bytes = bytes;
        let mut nodes = self.len();
        let mut depth = 1;
        for (i, old) in self.table.contents().enumerate() {
            if i.is_multiple_of(1024) {
                metadata_checkpoint(i, cancel)?;
            }
            let child = if i == index { &value } else { old };
            retained_bytes = retained_bytes
                .checked_add(child.retained_bytes())
                .ok_or("Cell retained bytes overflow")?;
            let (b, n, d) = contribution(child);
            bytes = bytes.checked_add(b).ok_or("Cell byte overflow")?;
            nodes = nodes.checked_add(n).ok_or("Cell node overflow")?;
            depth = depth.max(d + 1);
            checked_totals(bytes, nodes, depth)?;
        }
        if retained_bytes > LIMIT {
            let mut slots = Vec::with_capacity(self.len());
            for i in 0..self.len() {
                if i.is_multiple_of(256) {
                    crate::dsp::check_cancel(cancel)?;
                }
                slots.push(if i == index {
                    Arc::new(value.clone())
                } else {
                    self.get_linear(i)?.clone()
                });
            }
            let slots = crate::array::coordinate_retention::strip_slots(slots, cancel)?;
            return Self::from_shared(self.rows(), self.cols(), slots, cancel);
        }
        let table = self.table.replaced(
            index % self.rows(),
            index / self.rows(),
            Arc::new(value),
            cancel,
        )?;
        Ok(Self {
            table,
            bytes,
            retained_bytes: u32::try_from(retained_bytes)
                .map_err(|_| "Cell retained bytes overflow")?,
            nodes: u32::try_from(nodes).map_err(|_| "Cell node overflow")?,
            depth,
        })
    }
    pub fn preview(&self, limit: usize) -> String {
        use std::fmt::Write;
        let mut text = format!("{}x{} cell {{", self.rows(), self.cols());
        for (i, value) in self.table.contents().take(limit.min(16)).enumerate() {
            if i != 0 {
                text.push_str(", ");
            }
            let _ = write!(text, "{}x{} {}", value.rows(), value.cols(), value.class());
        }
        if self.len() > limit.min(16) {
            text.push_str(", ...");
        }
        text.push('}');
        text
    }
}
impl std::fmt::Display for CellArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.preview(16))
    }
}

// No test counter exists in a production build. Each checkpoint follows at most
// 1024 completed metadata contributions and precedes allocation/publication.
pub(crate) fn metadata_checkpoint(_completed: usize, cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    STOP_AFTER.with(|slot| {
        if slot.get() == Some(_completed) {
            slot.set(None);
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    crate::dsp::check_cancel(cancel)
}
#[cfg(test)]
thread_local! { static STOP_AFTER:std::cell::Cell<Option<usize>>=const { std::cell::Cell::new(None) }; }
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    struct Probe;
    impl Drop for Probe {
        fn drop(&mut self) {
            STOP_AFTER.with(|s| s.set(None));
        }
    }
    #[test]
    fn active_cell_metadata_cancel_is_bounded_atomic_and_retryable() {
        let cancel = AtomicBool::new(false);
        let payload: Array = Matrix::from_column_slice(1, 2, &[-0., 3.]).unwrap().into();
        let shared = Arc::new(payload.clone());
        let original =
            CellArray::from_shared(1, 4096, vec![shared.clone(); 4096], &cancel).unwrap();
        let alias = original.clone();
        for replacing in [false, true] {
            let _guard = Probe;
            STOP_AFTER.with(|s| s.set(Some(1024)));
            let result = if replacing {
                original.replaced(4095, Matrix::scalar(9.).into(), &cancel)
            } else {
                CellArray::from_shared(1, 4096, vec![shared.clone(); 4096], &cancel)
            };
            assert!(result.unwrap_err().contains("cancel"));
            assert!(cancel.load(Ordering::Relaxed));
            STOP_AFTER.with(|s| {
                assert_eq!(
                    s.get(),
                    None,
                    "Actual metadata checkpoint consumed the probe"
                )
            });
            assert!(original.same_storage(&alias));
            assert!(original.get_linear(4095).unwrap().same_snapshot(&payload));
            cancel.store(false, Ordering::Relaxed);
        }
        let retry = original
            .replaced(4095, Matrix::scalar(9.).into(), &cancel)
            .unwrap();
        assert_eq!(
            retry.get_linear(4095).unwrap().as_numeric().unwrap().data(),
            &[9.]
        );
        assert!(retry.get_linear(0).unwrap().same_snapshot(&payload));
        assert!(original.same_storage(&alias));
    }
    #[test]
    fn repeated_shared_payloads_count_toward_logical_budget_without_copying() {
        let cancel = AtomicBool::new(false);
        let payload: Array = Matrix::filled(1, 65536, 7.).unwrap().into();
        let retained = payload.clone();
        let shared = Arc::new(payload);
        let error = CellArray::from_shared(1, 513, vec![shared; 513], &cancel).unwrap_err();
        assert!(error.contains("256 MiB"), "{error}");
        assert_eq!(retained.as_numeric().unwrap().data()[65535], 7.);
    }
}
