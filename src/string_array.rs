//! Immutable, bounded two-dimensional string storage, in column-major order.
use crate::Result;
use std::sync::Arc;

pub const MAX_STRINGS: usize = 65_536;
pub const MAX_STRING_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct StringArray {
    rows: usize,
    cols: usize,
    data: Arc<[Option<Arc<str>>]>,
    bytes: usize,
}
impl StringArray {
    pub fn checked_len(rows: usize, cols: usize) -> Result<usize> {
        let n = rows
            .checked_mul(cols)
            .ok_or("String array dimensions overflow")?;
        if rows > MAX_STRINGS || cols > MAX_STRINGS || n > MAX_STRINGS {
            return Err("String array exceeds 65536 elements".into());
        }
        Ok(n)
    }
    pub fn new(rows: usize, cols: usize, data: Vec<Option<Arc<str>>>) -> Result<Self> {
        if Self::checked_len(rows, cols)? != data.len() {
            return Err("String array shape does not match its elements".into());
        }
        let bytes = data
            .iter()
            .flatten()
            .try_fold(0usize, |n, s| n.checked_add(s.len()));
        if bytes.is_none_or(|n| n > MAX_STRING_BYTES) {
            return Err("String array text exceeds 2 MiB".into());
        }
        let bytes = data.len() * std::mem::size_of::<Option<Arc<str>>>() + bytes.unwrap_or(0);
        Ok(Self {
            rows,
            cols,
            data: data.into(),
            bytes,
        })
    }
    pub fn scalar(text: impl Into<Arc<str>>) -> Result<Self> {
        Self::new(1, 1, vec![Some(text.into())])
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn data(&self) -> &[Option<Arc<str>>] {
        &self.data
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
    pub(crate) fn reshaped(&self, rows: usize, cols: usize) -> Result<Self> {
        if Self::checked_len(rows, cols)? != self.len() {
            return Err("String reshape must preserve the number of elements".into());
        }
        Ok(Self {
            rows,
            cols,
            ..self.clone()
        })
    }
    pub fn quoted(item: &Option<Arc<str>>) -> String {
        item.as_ref().map_or_else(
            || "<missing>".into(),
            |s| format!("\"{}\"", s.replace('"', "\"\"")),
        )
    }
    /// Bounded per-entry text for the workspace and read-only inspector.
    pub fn preview_item(item: &Option<Arc<str>>) -> String {
        match item {
            None => "<missing>".into(),
            Some(s) => {
                let mut chars = s.chars();
                let text: String = chars.by_ref().take(32).collect();
                format!(
                    "\"{}{}\"",
                    text.replace('"', "\"\""),
                    if chars.next().is_some() { "..." } else { "" }
                )
            }
        }
    }
    pub fn preview(&self, limit: usize) -> String {
        let limit = limit.min(16);
        let mut text = self
            .data
            .iter()
            .take(limit)
            .map(Self::preview_item)
            .collect::<Vec<_>>()
            .join("  ");
        if self.len() > limit {
            text.push_str(" …");
        }
        text
    }
}
