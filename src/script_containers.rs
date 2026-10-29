//! Bounded storage primitives for future interpreted cell/character values.
//!
//! This module does not add cell syntax, workspace admission or graphics handles.
//! Cell payloads are immutable shared objects. Their transitive storage budget is
//! the responsibility of the future typed-value/workspace layer, not this table.
use crate::Result;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A deliberately separate metadata limit, not MATLAB's array dimension limit.
pub const MAX_CELL_SLOTS: usize = 65_536;
pub const MAX_CHARACTER_UNITS: usize = 65_536;
const CHUNK: usize = 1024;

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn shape(rows: usize, cols: usize) -> Result<usize> {
    let n = rows.checked_mul(cols).ok_or("Cell shape overflow")?;
    if rows > MAX_CELL_SLOTS || cols > MAX_CELL_SLOTS || n > MAX_CELL_SLOTS {
        return Err("Cell table exceeds 65536 slots or dimension bound".into());
    }
    Ok(n)
}

/// A column-major immutable table. Indices here are zero-based internal indices;
/// a future MATLAB evaluator must validate/translate one-based indices first.
/// T need not implement Clone: copying a table never clones a payload.
#[derive(Debug)]
pub struct CellTable<T> {
    rows: usize,
    cols: usize,
    slots: Arc<Vec<Arc<T>>>,
}

impl<T> Clone for CellTable<T> {
    fn clone(&self) -> Self {
        Self {
            rows: self.rows,
            cols: self.cols,
            slots: self.slots.clone(),
        }
    }
}

impl<T> CellTable<T> {
    /// The language adapter must supply its canonical empty-double value when
    /// implementing cell(m,n); None is not a substitute for MATLAB empty double.
    pub fn filled(rows: usize, cols: usize, value: Arc<T>, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let n = shape(rows, cols)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(n)
            .map_err(|_| "Unable to allocate cell table")?;
        while slots.len() < n {
            check(cancel)?;
            let end = (slots.len() + CHUNK).min(n);
            slots.resize_with(end, || value.clone());
        }
        check(cancel)?;
        Ok(Self {
            rows,
            cols,
            slots: Arc::new(slots),
        })
    }

    pub fn from_column_major(
        rows: usize,
        cols: usize,
        slots: Vec<Arc<T>>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        if shape(rows, cols)? != slots.len() || slots.capacity() > MAX_CELL_SLOTS {
            return Err("Cell payload length/capacity does not match bounded shape".into());
        }
        Ok(Self {
            rows,
            cols,
            slots: Arc::new(slots),
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub(crate) fn weak_storage(&self) -> std::sync::Weak<Vec<Arc<T>>> {
        Arc::downgrade(&self.slots)
    }
    /// Logical slot metadata only; excludes Arc headers, capacity and T payloads.
    pub fn slot_bytes(&self) -> usize {
        self.len() * std::mem::size_of::<Arc<T>>()
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.slots, &other.slots)
    }
    pub fn get(&self, row: usize, col: usize) -> Result<&Arc<T>> {
        if row >= self.rows || col >= self.cols {
            return Err("Cell index exceeds table shape".into());
        }
        Ok(&self.slots[col * self.rows + row])
    }
    /// Borrowed column-major contents, suitable for later bounded CSL expansion.
    /// This is not itself a language-level comma-separated list evaluator.
    pub fn contents(&self) -> impl ExactSizeIterator<Item = &Arc<T>> {
        self.slots.iter()
    }
    pub fn reshape(&self, rows: usize, cols: usize, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        if shape(rows, cols)? != self.len() {
            return Err("Cell reshape changes slot count".into());
        }
        Ok(Self {
            rows,
            cols,
            slots: self.slots.clone(),
        })
    }
    /// Fixed-shape content replacement. No implicit growth/deletion or payload
    /// mutation. The old table remains usable on every error/cancellation path.
    pub fn replaced(
        &self,
        row: usize,
        col: usize,
        value: Arc<T>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        self.replaced_checked(row, col, value, || check(cancel))
    }
    fn replaced_checked(
        &self,
        row: usize,
        col: usize,
        value: Arc<T>,
        mut checkpoint: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        checkpoint()?;
        self.get(row, col)?;
        let target = col * self.rows + row;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(self.len())
            .map_err(|_| "Unable to allocate replacement cell table")?;
        for chunk in self.slots.chunks(CHUNK) {
            checkpoint()?;
            slots.extend(chunk.iter().cloned());
        }
        slots[target] = value;
        checkpoint()?;
        Ok(Self {
            rows: self.rows,
            cols: self.cols,
            slots: Arc::new(slots),
        })
    }
}

/// Exact column-major UTF-16 character array (not a string array).
/// The public CharacterRow name is retained for source compatibility.
/// Lone surrogates remain representable in storage; conversion to a Rust/UI
/// string rejects them explicitly, rather than silently replacing characters.
#[derive(Clone, Debug)]
pub struct CharacterRow {
    rows: usize,
    cols: usize,
    units: Arc<Vec<u16>>,
}
impl CharacterRow {
    /// Historical constructor: nonempty row, or canonical 0x0 empty.
    pub fn from_units(units: Vec<u16>, cancel: &AtomicBool) -> Result<Self> {
        let cols = units.len();
        Self::from_column_major(usize::from(cols != 0), cols, units, cancel)
    }
    pub fn checked_len(rows: usize, cols: usize) -> Result<usize> {
        let count = rows.checked_mul(cols).ok_or("Character shape overflow")?;
        if rows > MAX_CHARACTER_UNITS || cols > MAX_CHARACTER_UNITS || count > MAX_CHARACTER_UNITS {
            return Err("Character array exceeds 65536 UTF-16 units or dimension bound".into());
        }
        Ok(count)
    }
    pub fn from_column_major(
        rows: usize,
        cols: usize,
        units: Vec<u16>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        if Self::checked_len(rows, cols)? != units.len() || units.capacity() > MAX_CHARACTER_UNITS {
            return Err("Character payload length/capacity does not match bounded shape".into());
        }
        Ok(Self {
            rows,
            cols,
            units: Arc::new(units),
        })
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    /// Text interpretation never flattens a matrix or a shaped nonrow empty.
    pub fn is_text_row(&self) -> bool {
        self.rows == 1 || (self.rows, self.cols) == (0, 0)
    }
    pub fn reshape(&self, rows: usize, cols: usize, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        if Self::checked_len(rows, cols)? != self.units.len() {
            return Err("Character reshape changes unit count".into());
        }
        Ok(Self {
            rows,
            cols,
            units: self.units.clone(),
        })
    }
    pub fn replaced(&self, index: usize, unit: u16, cancel: &AtomicBool) -> Result<Self> {
        self.replaced_checked(index, unit, || check(cancel))
    }
    fn replaced_checked(
        &self,
        index: usize,
        unit: u16,
        mut checkpoint: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        checkpoint()?;
        if index >= self.units.len() {
            return Err("Character index exceeds shape".into());
        }
        let mut units = Vec::new();
        units
            .try_reserve_exact(self.units.len())
            .map_err(|_| "Cannot allocate character replacement")?;
        for chunk in self.units.chunks(CHUNK) {
            checkpoint()?;
            units.extend_from_slice(chunk);
        }
        units[index] = unit;
        checkpoint()?;
        Ok(Self {
            rows: self.rows,
            cols: self.cols,
            units: Arc::new(units),
        })
    }
    pub(crate) fn weak_storage(&self) -> std::sync::Weak<Vec<u16>> {
        Arc::downgrade(&self.units)
    }
    pub fn units(&self) -> &[u16] {
        &self.units
    }
    pub fn bytes(&self) -> usize {
        self.units.len() * 2
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.units, &other.units)
    }
    pub fn to_utf8(&self, cancel: &AtomicBool) -> Result<String> {
        check(cancel)?;
        if !self.is_text_row() {
            return Err("Text conversion requires a character row or canonical 0x0 empty; matrices are not flattened".into());
        }
        let mut text = String::new();
        // At most three UTF-8 bytes per UTF-16 code unit (a pair uses four).
        text.try_reserve_exact(self.units.len() * 3)
            .map_err(|_| "Unable to allocate character projection")?;
        for (index, value) in char::decode_utf16(self.units.iter().copied()).enumerate() {
            if index.is_multiple_of(CHUNK) {
                check(cancel)?;
            }
            text.push(value.map_err(|_| "Character row contains an unpaired UTF-16 surrogate")?);
        }
        check(cancel)?;
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_major_contents_and_fixed_replacement_preserve_shared_payloads() {
        // No Clone implementation: no operation can accidentally copy payloads.
        #[derive(Debug)]
        struct Payload {
            marker: usize,
            data: Vec<f64>,
        }
        let cancel = AtomicBool::new(false);
        let values: Vec<_> = (0..6)
            .map(|marker| {
                Arc::new(Payload {
                    marker,
                    data: vec![marker as f64; 8],
                })
            })
            .collect();
        let table = CellTable::from_column_major(2, 3, values.clone(), &cancel).unwrap();
        let alias = table.clone();
        assert!(table.same_storage(&alias));
        assert_eq!(table.get(1, 2).unwrap().marker, 5);
        assert_eq!(
            table.contents().map(|v| v.marker).collect::<Vec<_>>(),
            (0..6).collect::<Vec<_>>()
        );
        let changed = table.replaced(0, 1, values[5].clone(), &cancel).unwrap();
        assert_eq!(table.get(0, 1).unwrap().marker, 2);
        assert!(Arc::ptr_eq(changed.get(0, 1).unwrap(), &values[5]));
        assert!(Arc::ptr_eq(
            changed.get(1, 1).unwrap(),
            table.get(1, 1).unwrap()
        ));
        assert_eq!(alias.get(0, 1).unwrap().data, vec![2.; 8]);
        let flat = table.reshape(1, 6, &cancel).unwrap();
        assert!(flat.same_storage(&table));
        assert_eq!(flat.get(0, 5).unwrap().marker, 5);
    }

    #[test]
    fn shape_empty_and_capacity_bounds_precede_table_publication() {
        let cancel = AtomicBool::new(false);
        let value = Arc::new(42);
        for (r, c) in [(0, 0), (0, 3), (4, 0)] {
            let table = CellTable::filled(r, c, value.clone(), &cancel).unwrap();
            assert_eq!((table.rows(), table.cols(), table.len()), (r, c, 0));
            assert!(table.get(0, 0).is_err());
        }
        assert!(CellTable::filled(usize::MAX, 0, value.clone(), &cancel).is_err());
        assert!(CellTable::filled(usize::MAX, 2, value.clone(), &cancel).is_err());
        assert!(CellTable::filled(257, 256, value.clone(), &cancel).is_err());
        assert!(CellTable::from_column_major(2, 2, vec![value.clone()], &cancel).is_err());
        let mut oversized = Vec::with_capacity(MAX_CELL_SLOTS + 1);
        oversized.push(value);
        assert!(CellTable::from_column_major(1, 1, oversized, &cancel).is_err());
        let mut characters = Vec::with_capacity(MAX_CHARACTER_UNITS + 1);
        characters.push(65);
        assert!(CharacterRow::from_units(characters, &cancel).is_err());
    }

    #[test]
    fn cancellation_after_actual_slot_progress_keeps_old_table_and_retry() {
        let cancel = AtomicBool::new(false);
        let old = Arc::new(7);
        let table = CellTable::filled(1, CHUNK * 3 + 1, old.clone(), &cancel).unwrap();
        let alias = table.clone();
        let mut checks = 0;
        let result = table.replaced_checked(0, CHUNK * 3, Arc::new(9), || {
            checks += 1;
            // Entry + first chunk succeeded; the next checkpoint sees Stop.
            if checks == 3 {
                cancel.store(true, Ordering::Relaxed);
            }
            check(&cancel)
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(checks, 3);
        assert!(table.same_storage(&alias));
        assert!(table.contents().all(|v| Arc::ptr_eq(v, &old)));
        cancel.store(false, Ordering::Relaxed);
        let retry = table.replaced(0, CHUNK * 3, Arc::new(9), &cancel).unwrap();
        assert_eq!(**retry.get(0, CHUNK * 3).unwrap(), 9);
        assert_eq!(**table.get(0, CHUNK * 3).unwrap(), 7);
    }

    #[test]
    fn utf16_keeps_code_units_and_rejects_lossy_projection() {
        let cancel = AtomicBool::new(false);
        let units = vec![65, 0, 10, 0x03b2, 0xd83d, 0xde00];
        let row = CharacterRow::from_units(units.clone(), &cancel).unwrap();
        assert_eq!(row.units(), units);
        assert_eq!(row.bytes(), 12);
        assert_eq!(row.to_utf8(&cancel).unwrap(), "A\0\n\u{03b2}\u{1f600}");
        assert!(row.same_storage(&row.clone()));
        let invalid = CharacterRow::from_units(vec![0xd800], &cancel).unwrap();
        assert_eq!(invalid.units(), &[0xd800]);
        assert!(invalid.to_utf8(&cancel).is_err());
        assert!(CharacterRow::from_units(vec![0; MAX_CHARACTER_UNITS + 1], &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(row.to_utf8(&cancel).is_err());
        assert!(CellTable::filled(1, 1, Arc::new(0), &cancel).is_err());
    }
}

#[cfg(test)]
mod character_shape_copy_tests {
    use super::*;
    #[test]
    fn interrupted_shaped_copy_preserves_all_aliases_and_retry() {
        let stop = AtomicBool::new(false);
        let value =
            CharacterRow::from_column_major(2, 2048, (0..4096).map(|x| x as u16).collect(), &stop)
                .unwrap();
        let view = value.reshape(64, 64, &stop).unwrap();
        for stop_at in [3, 6] {
            let mut checks = 0;
            let result = value.replaced_checked(3072, 0xffff, || {
                checks += 1;
                if checks == stop_at {
                    Err("Calculation cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.unwrap_err().contains("cancelled"));
            assert_eq!(checks, stop_at);
            assert!(value.same_storage(&view));
            assert_eq!(value.units()[3072], 3072);
        }
        let retry = value.replaced(3072, 0xffff, &stop).unwrap();
        assert_eq!((retry.rows(), retry.cols()), (2, 2048));
        assert_eq!(retry.units()[3072], 0xffff);
        assert!(!retry.same_storage(&value));
        assert_eq!(view.units()[3072], 3072);
    }
}
