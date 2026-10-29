//! Bounded 2-D struct arrays. Field payloads are immutable shared `Array`
//! objects; replacing an element or field copies slot pointers, never payloads.
use crate::{
    Array, Matrix, Result,
    containers::{SLOT, checked_totals, contribution, metadata_checkpoint},
    script_containers::MAX_CELL_SLOTS,
};
use std::sync::{Arc, atomic::AtomicBool};

pub const MAX_FIELDS: usize = 1024;
pub const MAX_FIELD_NAME: usize = 63;
const LIMIT: usize = 256 * 1024 * 1024;

/// MATLAB `isvarname` for ASCII names, within `namelengthmax`.
pub fn valid_field_name(name: &str) -> bool {
    storable_field_name(name) && !crate::syntax::keyword(name)
}
/// Builtins may return keyword fields (`functions` has `function`); `s.function`
/// reads them, but user code cannot add one.
fn storable_field_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_FIELD_NAME
        && (b[0].is_ascii_alphabetic())
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// A rows x cols array of structs sharing one ordered field list. Slots are laid
/// out element-major: slot (field f, element e) is `slots[e * fields + f]`, with
/// elements in column-major array order. Counters are packed so the value keeps
/// the 48-byte envelope of the other `Array` variants (all are checked against
/// 65,536 slots / 256 MiB before narrowing).
#[derive(Clone, Debug)]
pub struct StructArray {
    names: Arc<Vec<String>>,
    slots: Arc<Vec<Arc<Array>>>,
    rows: u32,
    cols: u32,
    bytes: u32,
    retained_bytes: u32,
    nodes: u32,
    depth: u32,
}

impl StructArray {
    /// A 1x1 struct without fields.
    pub fn scalar_empty(cancel: &AtomicBool) -> Result<Self> {
        Self::from_slots(1, 1, Vec::new(), Vec::new(), cancel)
    }
    /// `slots` holds element 0's fields, then element 1's fields, and so on.
    pub fn from_slots(
        rows: usize,
        cols: usize,
        names: Vec<String>,
        slots: Vec<Arc<Array>>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::from_slots_inner(rows, cols, names, slots, true, cancel)
    }
    fn from_slots_inner(
        rows: usize,
        cols: usize,
        names: Vec<String>,
        slots: Vec<Arc<Array>>,
        may_strip: bool,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        if names.len() > MAX_FIELDS {
            return Err(format!("Struct exceeds {MAX_FIELDS} fields"));
        }
        for (i, name) in names.iter().enumerate() {
            if !storable_field_name(name) {
                return Err(format!(
                    "Invalid field name '{name}': use a letter followed by letters, digits or underscores, at most {MAX_FIELD_NAME} characters"
                ));
            }
            if names[..i].contains(name) {
                return Err(format!("Duplicate field name '{name}'"));
            }
        }
        let count = rows.checked_mul(cols).ok_or("Struct shape overflow")?;
        if rows > MAX_CELL_SLOTS || cols > MAX_CELL_SLOTS || count > MAX_CELL_SLOTS {
            return Err("Struct array exceeds 65536 elements".into());
        }
        if slots.len() != count * names.len() || slots.len() > MAX_CELL_SLOTS {
            return Err("Struct payload does not match its shape and field list".into());
        }
        let names_bytes: usize = names.iter().map(String::len).sum();
        let mut bytes = slots
            .len()
            .checked_mul(SLOT)
            .and_then(|v| v.checked_add(names_bytes))
            .ok_or("Struct byte overflow")?;
        let mut retained_bytes = bytes;
        let mut nodes = slots.len().max(count);
        let mut depth = 1;
        for (i, value) in slots.iter().enumerate() {
            if i.is_multiple_of(1024) {
                metadata_checkpoint(i, cancel)?;
            }
            retained_bytes = retained_bytes
                .checked_add(value.retained_bytes())
                .ok_or("Struct retained bytes overflow")?;
            let (b, n, d) = contribution(value);
            bytes = bytes.checked_add(b).ok_or("Struct byte overflow")?;
            nodes = nodes.checked_add(n).ok_or("Struct node overflow")?;
            depth = depth.max(d + 1);
            checked_totals(bytes, nodes, depth)?;
        }
        checked_totals(bytes, nodes, depth)?;
        if retained_bytes > LIMIT {
            // Optional coordinate facts are dropped once; a second oversize
            // result is a real limit error.
            if !may_strip {
                return Err("Struct value exceeds 256 MiB retained storage".into());
            }
            let slots = crate::array::coordinate_retention::strip_slots(slots, cancel)?;
            return Self::from_slots_inner(rows, cols, names, slots, false, cancel);
        }
        Self::build(
            rows,
            cols,
            names,
            slots,
            (bytes, retained_bytes, nodes, depth),
            cancel,
        )
    }
    fn build(
        rows: usize,
        cols: usize,
        names: Vec<String>,
        slots: Vec<Arc<Array>>,
        (bytes, retained_bytes, nodes, depth): (usize, usize, usize, usize),
        cancel: &AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        let narrow = |v: usize| u32::try_from(v).map_err(|_| "Struct size overflow".to_owned());
        Ok(Self {
            names: Arc::new(names),
            slots: Arc::new(slots),
            rows: narrow(rows)?,
            cols: narrow(cols)?,
            bytes: narrow(bytes)?,
            retained_bytes: narrow(retained_bytes)?,
            nodes: narrow(nodes)?,
            depth: narrow(depth)?,
        })
    }
    pub fn rows(&self) -> usize {
        self.rows as usize
    }
    pub fn cols(&self) -> usize {
        self.cols as usize
    }
    /// Number of elements (not slots).
    pub fn len(&self) -> usize {
        self.rows() * self.cols()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn bytes(&self) -> usize {
        self.bytes as usize
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes as usize
    }
    pub(crate) fn nodes(&self) -> usize {
        self.nodes as usize
    }
    pub(crate) fn depth(&self) -> usize {
        self.depth as usize
    }
    pub fn field_names(&self) -> &[String] {
        &self.names
    }
    pub fn field_count(&self) -> usize {
        self.names.len()
    }
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.slots, &other.slots) && Arc::ptr_eq(&self.names, &other.names)
    }
    /// Field `field` of element `element` (both zero-based).
    pub fn get(&self, field: usize, element: usize) -> Result<&Arc<Array>> {
        if field >= self.field_count() || element >= self.len() {
            return Err("Struct field or element index exceeds shape".into());
        }
        Ok(&self.slots[element * self.field_count() + field])
    }
    /// Slot by linear storage index (element-major), for bounded walks.
    pub(crate) fn slot(&self, index: usize) -> Result<&Arc<Array>> {
        self.slots
            .get(index)
            .ok_or_else(|| "Struct slot index exceeds shape".into())
    }
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }
    pub fn reshape(&self, rows: usize, cols: usize, cancel: &AtomicBool) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        if rows.checked_mul(cols) != Some(self.len()) {
            return Err("Struct reshape changes element count".into());
        }
        Ok(Self {
            rows: u32::try_from(rows).map_err(|_| "Struct size overflow")?,
            cols: u32::try_from(cols).map_err(|_| "Struct size overflow")?,
            ..self.clone()
        })
    }
    /// Slot count for `count` elements of `fields` fields, rejected above the shared
    /// slot bound before any caller allocates.
    pub(crate) fn slot_total(count: usize, fields: usize) -> Result<usize> {
        count
            .checked_mul(fields)
            .filter(|total| *total <= MAX_CELL_SLOTS)
            .ok_or_else(|| "Struct array exceeds 65536 slots".into())
    }
    fn empty_slot() -> Result<Arc<Array>> {
        Ok(Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?)))
    }
    /// Gather the given zero-based elements into a new array of the given shape.
    pub fn select(
        &self,
        rows: usize,
        cols: usize,
        positions: &[usize],
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let fields = self.field_count();
        if positions.len() > MAX_CELL_SLOTS {
            return Err("Struct array exceeds 65536 elements".into());
        }
        let total = Self::slot_total(positions.len(), fields)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(total)
            .map_err(|_| "Cannot allocate struct selection")?;
        for (i, &element) in positions.iter().enumerate() {
            if i.is_multiple_of(256) {
                crate::dsp::check_cancel(cancel)?;
            }
            for f in 0..fields {
                slots.push(self.get(f, element)?.clone());
            }
        }
        Self::from_slots(rows, cols, self.names.to_vec(), slots, cancel)
    }
    /// One element as a 1x1 struct.
    pub fn element(&self, element: usize, cancel: &AtomicBool) -> Result<Self> {
        self.select(1, 1, &[element], cancel)
    }
    /// Replace or append field `name` of a 1x1 struct.
    pub fn with_field(&self, name: &str, value: Array, cancel: &AtomicBool) -> Result<Self> {
        if self.len() != 1 {
            return Err("Scalar structure required for this assignment".into());
        }
        let mut names = self.names.to_vec();
        let mut slots: Vec<Arc<Array>> = (0..names.len())
            .map(|f| self.get(f, 0).cloned())
            .collect::<Result<_>>()?;
        match names.iter().position(|n| n == name) {
            Some(index) => slots[index] = Arc::new(value),
            None => {
                if !valid_field_name(name) {
                    return Err(format!(
                        "Invalid field name '{name}': use a letter followed by letters, digits or underscores, at most {MAX_FIELD_NAME} characters"
                    ));
                }
                names.push(name.to_owned());
                slots.push(Arc::new(value));
            }
        }
        Self::from_slots(1, 1, names, slots, cancel)
    }
    /// Remove a field from every element.
    pub fn without_field(&self, name: &str, cancel: &AtomicBool) -> Result<Self> {
        let Some(remove) = self.field_index(name) else {
            return Err(format!("A field named '{name}' cannot be found"));
        };
        let names: Vec<String> = self
            .names
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != remove)
            .map(|(_, n)| n.clone())
            .collect();
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(self.len() * names.len())
            .map_err(|_| "Cannot allocate struct fields")?;
        for e in 0..self.len() {
            if e.is_multiple_of(256) {
                crate::dsp::check_cancel(cancel)?;
            }
            for f in (0..self.field_count()).filter(|f| *f != remove) {
                slots.push(self.get(f, e)?.clone());
            }
        }
        Self::from_slots(self.rows(), self.cols(), names, slots, cancel)
    }
    /// Store the 1x1 struct `element` at zero-based `position` inside a result of
    /// `shape` (growing with empty `[]` fields). Fields absent from `self` are added
    /// to every element; fields absent from `element` keep their old value, or `[]`
    /// for a new position. Existing storage is untouched on every error path.
    pub fn with_element(
        &self,
        position: usize,
        shape: (usize, usize),
        element: &Self,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        if element.len() != 1 {
            return Err("Struct element assignment requires a 1x1 struct".into());
        }
        let count = shape
            .0
            .checked_mul(shape.1)
            .ok_or("Struct shape overflow")?;
        if position >= count || count < self.len() {
            return Err("Struct element index exceeds the result shape".into());
        }
        let mut names = self.names.to_vec();
        for name in element.names.iter() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        let total = Self::slot_total(count, names.len())?;
        let empty = Self::empty_slot()?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(total)
            .map_err(|_| "Cannot allocate struct array")?;
        for e in 0..count {
            if e.is_multiple_of(256) {
                crate::dsp::check_cancel(cancel)?;
            }
            for name in &names {
                let value = if e == position
                    && let Some(i) = element.field_index(name)
                {
                    element.get(i, 0)?.clone()
                } else if e % shape.0 < self.rows()
                    && e / shape.0 < self.cols()
                    && let Some(i) = self.field_index(name)
                {
                    self.get(i, e % shape.0 + e / shape.0 * self.rows())?
                        .clone()
                } else {
                    empty.clone()
                };
                slots.push(value);
            }
        }
        Self::from_slots(shape.0, shape.1, names, slots, cancel)
    }
    /// Build `rows x cols` elements with the given names, every field `[]`.
    pub fn filled_empty(
        rows: usize,
        cols: usize,
        names: Vec<String>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let count = rows.checked_mul(cols).ok_or("Struct shape overflow")?;
        let total = Self::slot_total(count, names.len())?;
        let empty = Self::empty_slot()?;
        Self::from_slots(rows, cols, names, vec![empty; total], cancel)
    }
    pub fn preview(&self, _limit: usize) -> String {
        if self.is_empty() || self.len() > 1 {
            let names = self.names.join(", ");
            format!(
                "{}x{} struct array with fields: {names}",
                self.rows, self.cols
            )
        } else {
            format!("1x1 struct with fields: {}", self.names.join(", "))
        }
    }
}
impl std::fmt::Display for StructArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::struct_display::disp(
            self,
            &crate::display_format::DisplayFormat::default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_stays_within_the_shared_array_envelope() {
        assert!(std::mem::size_of::<StructArray>() <= 48);
        assert_eq!(std::mem::size_of::<Array>(), 56);
    }

    #[test]
    fn element_assignment_shares_payloads_and_leaves_the_source_untouched() {
        let cancel = AtomicBool::new(false);
        let big: Arc<Array> = Arc::new(Matrix::filled(1, 1000, 1.).unwrap().into());
        let one =
            StructArray::from_slots(1, 1, vec!["big".into()], vec![big.clone()], &cancel).unwrap();
        let grown = one.with_element(2, (1, 3), &one, &cancel).unwrap();
        assert_eq!((grown.rows(), grown.cols(), grown.len()), (1, 3, 3));
        assert!(Arc::ptr_eq(grown.get(0, 0).unwrap(), &big));
        assert!(Arc::ptr_eq(grown.get(0, 2).unwrap(), &big));
        assert!(grown.get(0, 1).unwrap().is_empty());
        assert_eq!(one.len(), 1);
        let widened = one
            .with_field("extra", Matrix::scalar(2.).into(), &cancel)
            .unwrap();
        assert!(Arc::ptr_eq(widened.get(0, 0).unwrap(), &big));
        assert_eq!(widened.field_names(), ["big", "extra"]);
        assert!(
            widened
                .without_field("big", &cancel)
                .unwrap()
                .field_index("big")
                .is_none()
        );
        assert!(one.reshape(1, 1, &cancel).unwrap().same_storage(&one));
    }
    #[test]
    fn selection_rejects_oversized_slot_counts_before_allocating() {
        let cancel = AtomicBool::new(false);
        let names: Vec<String> = (0..MAX_FIELDS).map(|i| format!("f{i}")).collect();
        let empty = StructArray::empty_slot().unwrap();
        let wide = StructArray::from_slots(1, 1, names, vec![empty; MAX_FIELDS], &cancel).unwrap();
        let positions = vec![0; MAX_CELL_SLOTS];
        let error = wide
            .select(1, MAX_CELL_SLOTS, &positions, &cancel)
            .unwrap_err();
        assert_eq!(error, "Struct array exceeds 65536 slots");
        assert_eq!(
            StructArray::slot_total(usize::MAX, 2).unwrap_err(),
            "Struct array exceeds 65536 slots"
        );
    }
}
