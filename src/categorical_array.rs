//! Immutable, bounded column-major categorical storage. Zero denotes undefined.
use crate::Result;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct CategoricalArray {
    rows: u32,
    cols: u32,
    nd: Option<Arc<Vec<usize>>>,
    // Sharing the Vec keeps workspace assignment O(1), like datetime.
    codes: Arc<Vec<u32>>,
    categories: Arc<Vec<String>>,
    ordinal: bool,
}
impl CategoricalArray {
    pub fn new(
        rows: usize,
        cols: usize,
        codes: Vec<u32>,
        categories: Arc<Vec<String>>,
        ordinal: bool,
    ) -> Result<Self> {
        if crate::datetime_array::checked_len(rows, cols, "categorical")? != codes.len() {
            return Err("categorical: shape does not match codes".into());
        }
        if categories.len() > crate::datetime_array::MAX_ELEMENTS
            || codes.iter().any(|&c| c as usize > categories.len())
        {
            return Err("categorical: invalid category code or too many categories".into());
        }
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0usize;
        for name in categories.iter() {
            bytes = bytes.saturating_add(name.len());
            if name.is_empty() || name == "<undefined>" || !seen.insert(name) {
                return Err("categorical: category names must be nonempty and unique".into());
            }
        }
        if bytes > 64 * 1024 * 1024 {
            return Err("categorical: category text exceeds 64 MiB".into());
        }
        Ok(Self {
            rows: rows as u32,
            cols: cols as u32,
            nd: None,
            codes: Arc::new(codes),
            categories,
            ordinal,
        })
    }
    pub fn rows(&self) -> usize {
        self.rows as usize
    }
    pub fn cols(&self) -> usize {
        self.cols as usize
    }
    pub fn dims(&self) -> [usize; 2] {
        [self.rows(), self.cols()]
    }
    pub fn nd_dims(&self) -> std::borrow::Cow<'_, [usize]> {
        self.nd.as_deref().map_or_else(
            || std::borrow::Cow::Owned(vec![self.rows(), self.cols()]),
            |dims| std::borrow::Cow::Borrowed(dims.as_slice()),
        )
    }
    pub fn ndims(&self) -> usize {
        self.nd.as_ref().map_or(2, |dims| dims.len())
    }
    pub fn is_nd(&self) -> bool {
        self.nd.is_some()
    }
    pub fn with_dims(&self, dims: &[usize]) -> Result<Self> {
        let dims = crate::array::normalize_dims(dims)?;
        if dims[0] != self.rows()
            || dims[1..].iter().try_fold(1usize, |n, &d| n.checked_mul(d)) != Some(self.cols())
        {
            return Err("reshape dimensions do not match folded categorical storage".into());
        }
        let mut result = self.clone();
        result.nd = (dims.len() > 2).then(|| Arc::new(dims));
        Ok(result)
    }
    pub fn len(&self) -> usize {
        self.codes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }
    pub fn codes(&self) -> &[u32] {
        &self.codes
    }
    /// A shape-preserving, shared uint32 view for label export. Zero is undefined.
    pub fn code_array(&self) -> Result<crate::TypedArray> {
        crate::TypedArray::checked(
            self.rows(),
            self.cols(),
            crate::typed_array::TypedData::Uint32(self.codes.clone()),
        )?
        .with_dims(&self.nd_dims())
    }
    pub fn categories(&self) -> &Arc<Vec<String>> {
        &self.categories
    }
    pub fn is_ordinal(&self) -> bool {
        self.ordinal
    }
    pub fn text(&self, i: usize) -> &str {
        match self.codes[i] {
            0 => "<undefined>",
            c => &self.categories[c as usize - 1],
        }
    }
    pub fn bytes(&self) -> usize {
        self.codes.len() * 4
            + self
                .categories
                .iter()
                .map(|s| s.len() + std::mem::size_of::<String>())
                .sum::<usize>()
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.codes, &other.codes)
            && Arc::ptr_eq(&self.categories, &other.categories)
            && self.ordinal == other.ordinal
    }
    pub fn with_codes(&self, rows: usize, cols: usize, codes: Vec<u32>) -> Result<Self> {
        if crate::datetime_array::checked_len(rows, cols, "categorical")? != codes.len()
            || codes.iter().any(|&c| c as usize > self.categories.len())
        {
            return Err("categorical: invalid shape or codes".into());
        }
        Ok(Self {
            rows: rows as u32,
            cols: cols as u32,
            nd: None,
            codes: Arc::new(codes),
            ..self.clone()
        })
    }
    pub fn reshaped(&self, rows: usize, cols: usize) -> Result<Self> {
        if crate::datetime_array::checked_len(rows, cols, "categorical")? != self.len() {
            return Err("reshape: number of elements must not change".into());
        }
        Ok(Self {
            rows: rows as u32,
            cols: cols as u32,
            nd: None,
            ..self.clone()
        })
    }
    pub fn select(&self, rows: usize, cols: usize, positions: &[usize]) -> Result<Self> {
        let codes = positions
            .iter()
            .map(|&i| {
                self.codes
                    .get(i)
                    .copied()
                    .ok_or_else(|| "Index exceeds array bounds".to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        self.with_codes(rows, cols, codes)
    }
}
impl std::fmt::Display for CategoricalArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}x{} categorical]", self.rows, self.cols)
    }
}
