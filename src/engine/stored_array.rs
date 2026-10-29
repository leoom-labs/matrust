//! Static dispatch preserves the existing numerical indexing kernels.
use super::{Engine, Value};
use crate::{LogicalMatrix, Matrix, Result, display_format::DisplayFormat};
use std::fmt::Display;
pub(super) trait StoredArray: Clone + Display + Send + Sync {
    type Element: Copy + Default + Send + Sync;
    const BYTES: usize;
    const PAR_COPY: usize;
    fn checked_len(r: usize, c: usize) -> Result<usize> {
        Matrix::checked_len(r, c)
    }
    fn rows(&self) -> usize;
    fn cols(&self) -> usize;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn data(&self) -> &[Self::Element];
    fn reshape(&self, r: usize, c: usize) -> Result<Self>;
    fn from_vec(r: usize, c: usize, v: Vec<Self::Element>) -> Result<Self>;
    fn inherit_contiguous(&mut self, _source: &Self, _start: usize) {}
    fn inherit_stride(&mut self, _source: &Self, _start: usize, _step: usize) {}
    fn lookup<'a>(e: &'a Engine, n: &str) -> Option<&'a Self>;
    fn lookup_mut<'a>(e: &'a mut Engine, n: &str) -> Option<&'a mut Self>;
    fn value(self) -> Value;
    /// `name = ...` text for the current `format` settings.
    fn echo(&self, name: &str, fmt: &DisplayFormat) -> Result<String>;
    fn write_unique(&mut self, changes: &[(usize, Self::Element)]) -> bool;
    fn write_unique_regions(
        &mut self,
        regions: &[std::ops::Range<usize>],
        values: &[Self::Element],
        check: impl FnMut() -> Result<()>,
    ) -> Result<bool>;
}
impl StoredArray for Matrix {
    type Element = f64;
    const BYTES: usize = 8;
    const PAR_COPY: usize = 524288;
    fn rows(&self) -> usize {
        self.rows()
    }
    fn cols(&self) -> usize {
        self.cols()
    }
    fn len(&self) -> usize {
        self.len()
    }
    fn data(&self) -> &[f64] {
        self.data()
    }
    fn reshape(&self, r: usize, c: usize) -> Result<Self> {
        self.reshape(r, c)
    }
    fn inherit_stride(&mut self, source: &Self, start: usize, step: usize) {
        self.inherit_coordinate_stride(source, start, step);
    }
    fn inherit_contiguous(&mut self, source: &Self, start: usize) {
        self.inherit_coordinate_slice(source, start);
    }
    fn from_vec(r: usize, c: usize, v: Vec<f64>) -> Result<Self> {
        Self::from_column_vec(r, c, v)
    }
    fn lookup<'a>(e: &'a Engine, n: &str) -> Option<&'a Self> {
        e.variables.get(n)
    }
    fn lookup_mut<'a>(e: &'a mut Engine, n: &str) -> Option<&'a mut Self> {
        e.variables.get_mut(n)
    }
    fn value(self) -> Value {
        Value::Matrix(self)
    }
    fn echo(&self, name: &str, fmt: &DisplayFormat) -> Result<String> {
        crate::display_format::echo_real(name, self, fmt)
    }
    fn write_unique(&mut self, changes: &[(usize, f64)]) -> bool {
        self.write_unique(changes)
    }
    fn write_unique_regions(
        &mut self,
        regions: &[std::ops::Range<usize>],
        values: &[f64],
        check: impl FnMut() -> Result<()>,
    ) -> Result<bool> {
        self.write_unique_regions(regions, values, check)
    }
}
impl StoredArray for LogicalMatrix {
    type Element = u8;
    const BYTES: usize = 1;
    const PAR_COPY: usize = 4194304;
    fn rows(&self) -> usize {
        self.rows()
    }
    fn cols(&self) -> usize {
        self.cols()
    }
    fn len(&self) -> usize {
        self.len()
    }
    fn data(&self) -> &[u8] {
        self.data()
    }
    fn reshape(&self, r: usize, c: usize) -> Result<Self> {
        self.reshape(r, c)
    }
    fn from_vec(r: usize, c: usize, v: Vec<u8>) -> Result<Self> {
        Self::from_canonical_vec(r, c, v)
    }
    fn lookup<'a>(e: &'a Engine, n: &str) -> Option<&'a Self> {
        e.logical_variables.get(n)
    }
    fn lookup_mut<'a>(e: &'a mut Engine, n: &str) -> Option<&'a mut Self> {
        e.logical_variables.get_mut(n)
    }
    fn value(self) -> Value {
        Value::Logical(self)
    }
    fn echo(&self, name: &str, fmt: &DisplayFormat) -> Result<String> {
        crate::display_format::echo_logical(name, self, fmt)
    }
    fn write_unique(&mut self, changes: &[(usize, u8)]) -> bool {
        self.write_unique(changes)
    }
    fn write_unique_regions(
        &mut self,
        regions: &[std::ops::Range<usize>],
        values: &[u8],
        check: impl FnMut() -> Result<()>,
    ) -> Result<bool> {
        self.write_unique_regions(regions, values, check)
    }
}

impl StoredArray for crate::ComplexMatrix {
    type Element = crate::Complex64;
    const BYTES: usize = 16;
    const PAR_COPY: usize = 524288;
    fn checked_len(r: usize, c: usize) -> Result<usize> {
        Self::checked_len(r, c)
    }
    fn rows(&self) -> usize {
        self.rows()
    }
    fn cols(&self) -> usize {
        self.cols()
    }
    fn len(&self) -> usize {
        self.len()
    }
    fn data(&self) -> &[Self::Element] {
        self.data()
    }
    // These constructors only adopt a buffer or share metadata. The generic
    // caller polls its actual token before/after all allocating copy work.
    fn reshape(&self, r: usize, c: usize) -> Result<Self> {
        self.reshape(r, c, &std::sync::atomic::AtomicBool::new(false))
    }
    fn from_vec(r: usize, c: usize, v: Vec<Self::Element>) -> Result<Self> {
        Self::from_column_vec(r, c, v, &std::sync::atomic::AtomicBool::new(false))
    }
    fn lookup<'a>(e: &'a Engine, n: &str) -> Option<&'a Self> {
        e.complex_variables.get(n)
    }
    fn lookup_mut<'a>(e: &'a mut Engine, n: &str) -> Option<&'a mut Self> {
        e.complex_variables.get_mut(n)
    }
    fn value(self) -> Value {
        Value::Complex(self)
    }
    fn echo(&self, name: &str, fmt: &DisplayFormat) -> Result<String> {
        crate::display_format::echo_complex(name, self, fmt)
    }
    // Complex commits always construct a complete new immutable snapshot.
    fn write_unique(&mut self, _changes: &[(usize, Self::Element)]) -> bool {
        false
    }
    fn write_unique_regions(
        &mut self,
        _regions: &[std::ops::Range<usize>],
        _values: &[Self::Element],
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<bool> {
        check()?;
        Ok(false)
    }
}
impl Engine {
    pub(super) fn stored_buffer<T: StoredArray>(
        &self,
        len: usize,
        initialized: bool,
    ) -> Result<Vec<T::Element>> {
        // Preserve the existing real/logical allocation paths. Complex payloads
        // use fallible requested reservation and cancellable initialization.
        if T::BYTES != 16 {
            return Ok(if initialized {
                vec![T::Element::default(); len]
            } else {
                Vec::with_capacity(len)
            });
        }
        self.check_cancel()?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(len)
            .map_err(|_| "Cannot allocate complex indexed output")?;
        if initialized {
            for first in (0..len).step_by(4096) {
                self.check_cancel()?;
                values.resize((first + 4096).min(len), T::Element::default());
            }
        }
        self.check_cancel()?;
        Ok(values)
    }
}
