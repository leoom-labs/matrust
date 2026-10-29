//! Genuine legacy object-reference cell datasets and UTF-16 leaves.
use super::{h, write_class, write_leaf, write_values};
use crate::{Array, Result};
use hdf5::{Dataset, File, ObjectReference1, types::FixedAscii};
use std::{collections::BTreeMap, sync::Arc};

// At most 10000 top-level variables plus 65536 recursively counted slots.
const MAX_OBJECTS: usize = 10000 + 65536;

pub(super) fn preflight(
    value: &Array,
    depth: usize,
    nodes: &mut usize,
    slots: &mut usize,
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    checkpoint()?;
    *nodes = nodes.checked_add(1).ok_or("Cell graph overflow")?;
    if *nodes > MAX_OBJECTS {
        return Err("MAT cell graph exceeds 75536 visited objects".into());
    }
    match value {
        Array::Handle(_) => return Err("save: function handles are not supported yet".into()),
        Array::Graphics(_) => return Err(
            "MAT persistence of graphics handles is not implemented, including cell descendants"
                .into(),
        ),
        Array::Cell(cell) => {
            if depth >= 16 {
                return Err("Cell nesting exceeds 16 levels".into());
            }
            *slots = slots.checked_add(cell.len()).ok_or("Cell slot overflow")?;
            if *slots > 65536 {
                return Err("MAT workspace exceeds 65536 recursive cell slots".into());
            }
            for i in 0..cell.len() {
                preflight(
                    cell.get_linear(i)?.as_ref(),
                    depth + 1,
                    nodes,
                    slots,
                    checkpoint,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(super) struct Writer<'a> {
    file: &'a File,
    group_created: bool,
    next: usize,
    // Arc slot identity is sufficient to preserve repeated cell children. All
    // source Arcs remain borrowed/live for the complete atomic save. Different
    // Arcs sharing an underlying matrix may be encoded separately, honestly.
    targets: BTreeMap<usize, String>,
}
impl<'a> Writer<'a> {
    pub(super) fn new(file: &'a File) -> Self {
        Self {
            file,
            group_created: false,
            next: 0,
            targets: BTreeMap::new(),
        }
    }
    pub(super) fn write_root(
        &mut self,
        name: &str,
        value: &Array,
        checkpoint: &mut impl FnMut() -> Result<()>,
    ) -> Result<Dataset> {
        let Array::Cell(cell) = value else {
            return write_leaf(self.file, name, value, checkpoint);
        };
        if cell.is_empty() {
            return write_leaf(self.file, name, value, checkpoint);
        }
        if !self.group_created {
            let group = h(self.file.create_group("#refs#"))?;
            let path = FixedAscii::<7>::from_ascii("/#refs#").map_err(|e| e.to_string())?;
            h(h(group.new_attr::<FixedAscii<7>>().create("H5PATH"))?.write_scalar(&path))?;
            self.group_created = true;
        }
        let mut refs = Vec::<ObjectReference1>::new();
        refs.try_reserve_exact(cell.len())
            .map_err(|_| "Cannot allocate bounded cell references")?;
        for i in 0..cell.len() {
            checkpoint()?;
            let child = cell.get_linear(i)?;
            let key = Arc::as_ptr(child) as usize;
            let target = if let Some(path) = self.targets.get(&key) {
                path.clone()
            } else {
                if self.next >= 65536 {
                    return Err("MAT cell targets exceed 65536".into());
                }
                let path = format!("/#refs#/r{:06}", self.next);
                self.next += 1;
                let dataset = self.write_root(&path, child.as_ref(), checkpoint)?;
                let text = FixedAscii::<128>::from_ascii(&path).map_err(|e| e.to_string())?;
                h(h(dataset.new_attr::<FixedAscii<128>>().create("H5PATH"))?.write_scalar(&text))?;
                self.targets.insert(key, path.clone());
                path
            };
            refs.push(h(self.file.reference(&target))?);
        }
        let dataset = write_values(self.file, name, cell.rows(), cell.cols(), &refs, checkpoint)?;
        write_class(&dataset, value)?;
        Ok(dataset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn maximum_flat_cell_preflight_counts_roots_separately_from_slots() {
        let cancel = AtomicBool::new(false);
        let items = (0..65536)
            .map(|_| Arc::new(Array::Numeric(crate::Matrix::scalar(0.))))
            .collect();
        let value: Array = crate::CellArray::from_shared(1, 65536, items, &cancel)
            .unwrap()
            .into();
        let mut nodes = 0;
        let mut slots = 0;
        preflight(&value, 0, &mut nodes, &mut slots, &mut || Ok(())).unwrap();
        assert_eq!((nodes, slots), (65537, 65536));
        let leaf = Array::Numeric(crate::Matrix::scalar(1.));
        for _ in 1..10000 {
            preflight(&leaf, 0, &mut nodes, &mut slots, &mut || Ok(())).unwrap();
        }
        assert_eq!(nodes, MAX_OBJECTS);
        assert!(preflight(&leaf, 0, &mut nodes, &mut slots, &mut || Ok(())).is_err());
        let extra: Array = crate::CellArray::empty(1, 1, &cancel).unwrap().into();
        nodes = 65537;
        assert!(
            preflight(&extra, 0, &mut nodes, &mut slots, &mut || Ok(()))
                .unwrap_err()
                .contains("recursive cell slots")
        );
    }
    #[test]
    fn active_cell_save_cancellation_after_first_child_preserves_destination_and_alias() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retained.mat");
        std::fs::write(&path, b"retained").unwrap();
        let cancel = AtomicBool::new(false);
        let leaf = Arc::new(Array::Numeric(crate::Matrix::scalar(-0.)));
        let text = Arc::new(Array::Character(
            crate::CharacterRow::from_units(vec![0, 0xd800], &cancel).unwrap(),
        ));
        let value: Array = crate::CellArray::from_shared(1, 2, vec![leaf, text], &cancel)
            .unwrap()
            .into();
        let values = BTreeMap::from([("C".to_owned(), value.clone())]);
        let mut calls = 0;
        let error = super::super::save(&path, &values, || {
            calls += 1;
            if calls == 8 {
                Err("cancel after first cell child write".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.contains("after first cell child"));
        assert_eq!(calls, 8);
        assert_eq!(std::fs::read(&path).unwrap(), b"retained");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(value.same_snapshot(&values["C"]));
        super::super::save_array_workspace_v73(&path, &values, &cancel).unwrap();
        let catalog = crate::files::inspect_workspace(&path, &cancel).unwrap();
        let actual = crate::files::load_selected_arrays(
            &catalog,
            &std::collections::BTreeSet::from(["C".to_owned()]),
            &cancel,
        )
        .unwrap();
        let Array::Cell(c) = &actual["C"] else {
            panic!("Cell lost")
        };
        assert_eq!(
            c.get_linear(0).unwrap().as_numeric().unwrap().data()[0].to_bits(),
            (-0f64).to_bits()
        );
        let Array::Character(t) = c.get_linear(1).unwrap().as_ref() else {
            panic!("Character lost")
        };
        assert_eq!(t.units(), &[0, 0xd800]);
    }
}
