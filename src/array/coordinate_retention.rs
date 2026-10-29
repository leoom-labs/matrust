//! Rare admission fallback: drop optional facts, never copy dense samples.
use super::Array;
use crate::{CellArray, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak, atomic::AtomicBool},
};

// One workspace plus function arguments, each with a 256 MiB logical bound.
const MAX_RECORDS: usize = 2 * 256 * 1024 * 1024 / crate::containers::SLOT;
type SharedCellEntry = (Weak<Vec<Arc<Array>>>, CellArray);

pub(crate) struct FactStripper<'a> {
    cancel: &'a AtomicBool,
    arrays: BTreeMap<usize, (Weak<Array>, Arc<Array>)>,
    cells: BTreeMap<usize, SharedCellEntry>,
    slots: usize,
}
impl<'a> FactStripper<'a> {
    pub(crate) fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            cancel,
            arrays: BTreeMap::new(),
            cells: BTreeMap::new(),
            slots: 0,
        }
    }
    fn check(&self) -> Result<()> {
        #[cfg(test)]
        STOP_AFTER.with(|remaining| {
            if let Some(n) = remaining.get() {
                if n == 0 {
                    remaining.set(None);
                    self.cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                } else {
                    remaining.set(Some(n - 1));
                }
            }
        });
        crate::dsp::check_cancel(self.cancel)
    }
    fn shared(&mut self, old: &Arc<Array>) -> Result<Arc<Array>> {
        self.check()?;
        if old.retained_bytes() == old.bytes() {
            return Ok(old.clone());
        }
        let key = Arc::as_ptr(old) as usize;
        if let Some((_, value)) = self.arrays.get(&key) {
            return Ok(value.clone());
        }
        if self.arrays.len() >= MAX_RECORDS {
            return Err("Coordinate fallback metadata limit".into());
        }
        let value = Arc::new(self.strip(old)?);
        // Weak pins allocation identity without retaining optional source payloads.
        self.arrays
            .insert(key, (Arc::downgrade(old), value.clone()));
        Ok(value)
    }
    pub(crate) fn strip(&mut self, value: &Array) -> Result<Array> {
        self.check()?;
        if value.retained_bytes() == value.bytes() {
            return Ok(value.clone());
        }
        match value {
            Array::Numeric(matrix) => Ok(Array::Numeric(matrix.clone().without_coordinate_proof())),
            Array::Cell(cell) => {
                let weak = cell.weak_storage();
                let key = weak.as_ptr() as usize;
                if let Some((_, converted)) = self.cells.get(&key) {
                    return Ok(Array::Cell(converted.reshape(
                        cell.rows(),
                        cell.cols(),
                        self.cancel,
                    )?));
                }
                self.slots = self
                    .slots
                    .checked_add(cell.len())
                    .ok_or("Coordinate fallback metadata overflow")?;
                if self.slots > MAX_RECORDS || self.cells.len() >= MAX_RECORDS {
                    return Err("Coordinate fallback metadata limit".into());
                }
                let mut slots = Vec::with_capacity(cell.len());
                for index in 0..cell.len() {
                    slots.push(self.shared(cell.get_linear(index)?)?);
                }
                self.check()?;
                let converted =
                    CellArray::from_shared(cell.rows(), cell.cols(), slots, self.cancel)?;
                self.cells.insert(key, (weak, converted.clone()));
                Ok(Array::Cell(converted))
            }
            Array::Struct(s) => {
                self.slots = self
                    .slots
                    .checked_add(s.slot_count())
                    .ok_or("Coordinate fallback metadata overflow")?;
                if self.slots > MAX_RECORDS {
                    return Err("Coordinate fallback metadata limit".into());
                }
                let mut slots = Vec::with_capacity(s.slot_count());
                for index in 0..s.slot_count() {
                    slots.push(self.shared(s.slot(index)?)?);
                }
                self.check()?;
                Ok(Array::Struct(crate::StructArray::from_slots(
                    s.rows(),
                    s.cols(),
                    s.field_names().to_vec(),
                    slots,
                    self.cancel,
                )?))
            }
            _ => Ok(value.clone()),
        }
    }
}
pub(crate) fn strip_slots(slots: Vec<Arc<Array>>, cancel: &AtomicBool) -> Result<Vec<Arc<Array>>> {
    let mut context = FactStripper::new(cancel);
    let output = slots
        .iter()
        .map(|old| context.shared(old))
        .collect::<Result<Vec<_>>>()?;
    context.check()?;
    Ok(output)
}
impl Array {
    pub fn without_coordinate_facts(&self, cancel: &AtomicBool) -> Result<Self> {
        FactStripper::new(cancel).strip(self)
    }
}
#[cfg(test)]
thread_local! {
    static STOP_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
pub(crate) fn stop_after_checkpoints(count: usize) {
    STOP_AFTER.with(|v| v.set(Some(count)));
}
