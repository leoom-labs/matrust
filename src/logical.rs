//! Compact logical-array kernels. Script/workspace integration is a separate layer.
//!
//! Storage is immutable, shared, column-major, and uses one byte per element.
//! Numeric inputs are borrowed; comparisons never materialize numeric copies of masks.
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const CHUNK: usize = 8192;
// Double-input/output kernels benefit earlier than byte-only operations.
// Release threshold measurements are retained under research/performance/.
const PAR_ELEMENTS: usize = 512 * 1024;
const PAR_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct LogicalMatrix {
    pub(crate) data: Arc<Vec<u8>>,
    rows: usize,
    cols: usize,
}

/// Borrow either storage type without converting it or acquiring another Arc.
#[derive(Clone, Copy)]
pub enum ArrayRef<'a> {
    Numeric(&'a Matrix),
    Logical(&'a LogicalMatrix),
}

#[derive(Clone, Copy, Debug)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Clone, Copy, Debug)]
pub enum Predicate {
    IsNan,
    IsInfinite,
    IsFinite,
}

#[derive(Clone, Copy, Debug)]
pub enum BooleanOp {
    And,
    Or,
    Xor,
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

/// Fill disjoint output chunks on the existing Rayon pool. Never publishes a
/// partial array on error/cancellation; check again before returning ownership.
fn build<T: Default + Clone + Send>(
    len: usize,
    parallel_at: usize,
    cancel: &AtomicBool,
    fill: impl Fn(usize, &mut [T]) -> Result<()> + Sync,
) -> Result<Vec<T>> {
    check(cancel)?;
    let mut data = vec![T::default(); len];
    let work = |(chunk, output): (usize, &mut [T])| {
        check(cancel)?;
        fill(chunk * CHUNK, output)
    };
    if len >= parallel_at && rayon::current_num_threads() > 1 {
        data.par_chunks_mut(CHUNK).enumerate().try_for_each(work)?;
    } else {
        data.chunks_mut(CHUNK).enumerate().try_for_each(work)?;
    }
    check(cancel)?;
    Ok(data)
}

fn mapped<T: Copy + Sync>(
    source: &[T],
    parallel_at: usize,
    cancel: &AtomicBool,
    op: impl Fn(T) -> u8 + Sync,
) -> Result<Vec<u8>> {
    build(source.len(), parallel_at, cancel, |first, out| {
        let n = out.len();
        for (dst, &v) in out.iter_mut().zip(&source[first..first + n]) {
            *dst = op(v);
        }
        Ok(())
    })
}

fn boolean_binary(
    left: &[u8],
    right: &[u8],
    cancel: &AtomicBool,
    op: impl Fn(u8, u8) -> u8 + Sync,
) -> Result<Vec<u8>> {
    build(left.len(), PAR_BYTES, cancel, |first, out| {
        let n = out.len();
        for ((dst, &a), &b) in out
            .iter_mut()
            .zip(&left[first..first + n])
            .zip(&right[first..first + n])
        {
            *dst = op(a, b);
        }
        Ok(())
    })
}

fn byte_comparison(
    left: &[u8],
    right: &[u8],
    cancel: &AtomicBool,
    op: impl Fn(u8, u8) -> bool + Sync,
) -> Result<Vec<u8>> {
    if left.len() == 1 {
        mapped(right, PAR_BYTES, cancel, |b| u8::from(op(left[0], b)))
    } else if right.len() == 1 {
        mapped(left, PAR_BYTES, cancel, |a| u8::from(op(a, right[0])))
    } else {
        boolean_binary(left, right, cancel, |a, b| u8::from(op(a, b)))
    }
}

fn compared_bytes(
    left: &[u8],
    right: &[u8],
    op: Comparison,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    match op {
        Comparison::Equal => byte_comparison(left, right, cancel, |a, b| a == b),
        Comparison::NotEqual => byte_comparison(left, right, cancel, |a, b| a != b),
        Comparison::Less => byte_comparison(left, right, cancel, |a, b| a < b),
        Comparison::LessEqual => byte_comparison(left, right, cancel, |a, b| a <= b),
        Comparison::Greater => byte_comparison(left, right, cancel, |a, b| a > b),
        Comparison::GreaterEqual => byte_comparison(left, right, cancel, |a, b| a >= b),
    }
}

trait Number: Copy + Sync {
    fn number(self) -> f64;
}
impl Number for f64 {
    fn number(self) -> f64 {
        self
    }
}
impl Number for u8 {
    fn number(self) -> f64 {
        f64::from(self)
    }
}

fn binary<A: Number, B: Number>(
    left: &[A],
    right: &[B],
    len: usize,
    cancel: &AtomicBool,
    op: impl Fn(f64, f64) -> bool + Sync,
) -> Result<Vec<u8>> {
    // Scalar dispatch stays outside the element loop, as does operator dispatch.
    if left.len() == 1 {
        let a = left[0].number();
        build(len, PAR_ELEMENTS, cancel, |first, output| {
            let n = output.len();
            for (dst, &b) in output.iter_mut().zip(&right[first..first + n]) {
                *dst = u8::from(op(a, b.number()));
            }
            Ok(())
        })
    } else if right.len() == 1 {
        let b = right[0].number();
        build(len, PAR_ELEMENTS, cancel, |first, output| {
            let n = output.len();
            for (dst, &a) in output.iter_mut().zip(&left[first..first + n]) {
                *dst = u8::from(op(a.number(), b));
            }
            Ok(())
        })
    } else {
        build(len, PAR_ELEMENTS, cancel, |first, output| {
            let n = output.len();
            for ((dst, &a), &b) in output
                .iter_mut()
                .zip(&left[first..first + n])
                .zip(&right[first..first + n])
            {
                *dst = u8::from(op(a.number(), b.number()));
            }
            Ok(())
        })
    }
}

fn compared<A: Number, B: Number>(
    left: &[A],
    right: &[B],
    len: usize,
    op: Comparison,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    match op {
        Comparison::Equal => binary(left, right, len, cancel, |a, b| a == b),
        Comparison::NotEqual => binary(left, right, len, cancel, |a, b| a != b),
        Comparison::Less => binary(left, right, len, cancel, |a, b| a < b),
        Comparison::LessEqual => binary(left, right, len, cancel, |a, b| a <= b),
        Comparison::Greater => binary(left, right, len, cancel, |a, b| a > b),
        Comparison::GreaterEqual => binary(left, right, len, cancel, |a, b| a >= b),
    }
}

impl ArrayRef<'_> {
    pub fn shape(self) -> (usize, usize) {
        match self {
            Self::Numeric(a) => (a.rows(), a.cols()),
            Self::Logical(a) => (a.rows(), a.cols()),
        }
    }
    pub fn len(self) -> usize {
        let (r, c) = self.shape();
        r * c
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

fn expanded_shape(left: ArrayRef<'_>, right: ArrayRef<'_>) -> Result<(usize, usize)> {
    if left.len() == 1 {
        Ok(right.shape())
    } else if right.len() == 1 || left.shape() == right.shape() {
        Ok(left.shape())
    } else {
        Err("Array dimensions must agree (only scalar expansion is supported)".into())
    }
}

impl LogicalMatrix {
    pub(crate) fn from_canonical_vec(rows: usize, cols: usize, data: Vec<u8>) -> Result<Self> {
        if Matrix::checked_len(rows, cols)? != data.len() {
            return Err("Data does not match logical dimensions".into());
        }
        Ok(Self::owned(rows, cols, data))
    }
    pub fn with_value(
        &self,
        row: usize,
        col: usize,
        value: f64,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        if row >= self.rows || col >= self.cols {
            return Err("Cell is outside the matrix dimensions".into());
        }
        if value.is_nan() {
            return Err("NaN cannot be converted to logical".into());
        }
        let mut data = build(self.len(), PAR_BYTES, cancel, |first, out| {
            let n = out.len();
            out.copy_from_slice(&self.data[first..first + n]);
            Ok(())
        })?;
        data[col * self.rows + row] = u8::from(value != 0.);
        Ok(Self::owned(self.rows, self.cols, data))
    }
    pub fn transpose(&self, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        if self.rows <= 1 || self.cols <= 1 {
            return self.reshape(self.cols, self.rows);
        }
        let data = build(self.len(), PAR_BYTES, cancel, |first, out| {
            for (i, v) in out.iter_mut().enumerate() {
                let index = first + i;
                *v = self.data[(index % self.cols) * self.rows + index / self.cols];
            }
            Ok(())
        })?;
        Ok(Self::owned(self.cols, self.rows, data))
    }
    pub(crate) fn write_unique(&mut self, changes: &[(usize, u8)]) -> bool {
        assert!(changes.len() <= 8192 && changes.iter().all(|(i, v)| *i < self.len() && *v <= 1));
        if let Some(data) = Arc::get_mut(&mut self.data) {
            for &(i, v) in changes {
                data[i] = v;
            }
            true
        } else {
            false
        }
    }
    pub(crate) fn write_unique_regions(
        &mut self,
        regions: &[std::ops::Range<usize>],
        values: &[u8],
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<bool> {
        let Some(storage) = Arc::get_mut(&mut self.data) else {
            return Ok(false);
        };
        let count: usize = regions.iter().map(|r| r.len()).sum();
        assert!(regions.len() <= 1024 && count <= 2 * 1024 * 1024);
        assert!(
            regions
                .iter()
                .all(|r| r.start <= r.end && r.end <= storage.len())
        );
        assert!(values.len() == 1 || values.len() == count);
        let mut undo = Vec::with_capacity(count);
        for region in regions {
            for chunk in storage[region.clone()].chunks(CHUNK) {
                check()?;
                undo.extend_from_slice(chunk);
            }
        }
        let result = (|| {
            let mut source = 0;
            for region in regions {
                for chunk in storage[region.clone()].chunks_mut(CHUNK) {
                    check()?;
                    if values.len() == 1 {
                        chunk.fill(values[0]);
                    } else {
                        chunk.copy_from_slice(&values[source..source + chunk.len()]);
                    }
                    source += chunk.len();
                }
            }
            check()
        })();
        if result.is_err() {
            let mut source = 0;
            for region in regions {
                storage[region.clone()].copy_from_slice(&undo[source..source + region.len()]);
                source += region.len();
            }
        }
        result.map(|_| true)
    }
    // All callers have checked dimensions and produced only canonical 0/1 bytes.
    fn owned(rows: usize, cols: usize, data: Vec<u8>) -> Self {
        debug_assert_eq!(rows.checked_mul(cols), Some(data.len()));
        Self {
            data: Arc::new(data),
            rows,
            cols,
        }
    }
    pub fn from_column_vec(
        rows: usize,
        cols: usize,
        data: Vec<u8>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        if Matrix::checked_len(rows, cols)? != data.len() {
            return Err("Data does not match logical array dimensions".into());
        }
        for chunk in data.chunks(CHUNK) {
            check(cancel)?;
            if chunk.iter().any(|&v| v > 1) {
                return Err("Logical storage requires bytes containing only 0 or 1".into());
            }
        }
        check(cancel)?;
        Ok(Self::owned(rows, cols, data))
    }
    pub fn scalar(value: bool) -> Self {
        Self::owned(1, 1, vec![u8::from(value)])
    }
    pub fn filled(rows: usize, cols: usize, value: bool, cancel: &AtomicBool) -> Result<Self> {
        let len = Matrix::checked_len(rows, cols)?;
        let data = build(len, PAR_BYTES, cancel, |_, out| {
            out.fill(u8::from(value));
            Ok(())
        })?;
        Ok(Self::owned(rows, cols, data))
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
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn get(&self, row: usize, col: usize) -> bool {
        assert!(
            row < self.rows && col < self.cols,
            "Logical subscript out of bounds"
        );
        self.data[col * self.rows + row] != 0
    }
    pub fn as_scalar(&self) -> Result<bool> {
        if self.len() == 1 {
            Ok(self.data[0] != 0)
        } else {
            Err("Expected a logical scalar".into())
        }
    }
    /// Reshape shares the original allocation, including for empty arrays.
    pub fn reshape(&self, rows: usize, cols: usize) -> Result<Self> {
        if Matrix::checked_len(rows, cols)? != self.len() {
            return Err("Reshape must preserve the number of elements".into());
        }
        Ok(Self {
            data: self.data.clone(),
            rows,
            cols,
        })
    }
    /// Convert real doubles to logicals; MATLAB rejects NaN conversions.
    pub fn from_numeric(source: &Matrix, cancel: &AtomicBool) -> Result<Self> {
        let data = build(source.len(), PAR_ELEMENTS, cancel, |first, out| {
            let n = out.len();
            let mut nan = false;
            for (dst, &v) in out.iter_mut().zip(&source.data()[first..first + n]) {
                nan |= v.is_nan();
                *dst = u8::from(v != 0.);
            }
            if nan {
                Err("NaN cannot be converted to logical".into())
            } else {
                Ok(())
            }
        })?;
        Ok(Self::owned(source.rows(), source.cols(), data))
    }
    pub fn to_numeric(&self, cancel: &AtomicBool) -> Result<Matrix> {
        let data = build(self.len(), PAR_ELEMENTS, cancel, |first, out| {
            let n = out.len();
            for (dst, &v) in out.iter_mut().zip(&self.data[first..first + n]) {
                *dst = f64::from(v);
            }
            Ok(())
        })?;
        Matrix::from_column_vec(self.rows, self.cols, data)
    }
    pub fn compare(
        left: ArrayRef<'_>,
        right: ArrayRef<'_>,
        op: Comparison,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let (rows, cols) = expanded_shape(left, right)?;
        let len = Matrix::checked_len(rows, cols)?;
        use ArrayRef::{Logical as L, Numeric as N};
        let data = match (left, right) {
            (N(a), N(b)) => compared(a.data(), b.data(), len, op, cancel)?,
            (N(a), L(b)) => compared(a.data(), b.data(), len, op, cancel)?,
            (L(a), N(b)) => compared(a.data(), b.data(), len, op, cancel)?,
            (L(a), L(b)) => compared_bytes(a.data(), b.data(), op, cancel)?,
        };
        Ok(Self::owned(rows, cols, data))
    }
    pub fn predicate(source: ArrayRef<'_>, op: Predicate, cancel: &AtomicBool) -> Result<Self> {
        let (rows, cols) = source.shape();
        let ArrayRef::Numeric(source) = source else {
            return Self::filled(rows, cols, matches!(op, Predicate::IsFinite), cancel);
        };
        let data = match op {
            Predicate::IsNan => mapped(source.data(), PAR_ELEMENTS, cancel, |v| {
                u8::from(v.is_nan())
            })?,
            Predicate::IsInfinite => mapped(source.data(), PAR_ELEMENTS, cancel, |v| {
                u8::from(v.is_infinite())
            })?,
            Predicate::IsFinite => mapped(source.data(), PAR_ELEMENTS, cancel, |v| {
                u8::from(v.is_finite())
            })?,
        };
        Ok(Self::owned(rows, cols, data))
    }
    pub fn not(&self, cancel: &AtomicBool) -> Result<Self> {
        let data = mapped(self.data(), PAR_BYTES, cancel, |v| v ^ 1)?;
        Ok(Self::owned(self.rows, self.cols, data))
    }
    pub fn combine(&self, right: &Self, op: BooleanOp, cancel: &AtomicBool) -> Result<Self> {
        let (rows, cols) = expanded_shape(ArrayRef::Logical(self), ArrayRef::Logical(right))?;
        // Both arrays are already logical: scalar identities can share the result.
        check(cancel)?;
        let scalar = if self.len() == 1 {
            Some((self.data[0] != 0, right))
        } else if right.len() == 1 {
            Some((right.data[0] != 0, self))
        } else {
            None
        };
        if let Some((value, array)) = scalar {
            return match (op, value) {
                (BooleanOp::And, true) | (BooleanOp::Or | BooleanOp::Xor, false) => {
                    Ok(array.clone())
                }
                (BooleanOp::Xor, true) => array.not(cancel),
                (BooleanOp::And, false) => Self::filled(rows, cols, false, cancel),
                (BooleanOp::Or, true) => Self::filled(rows, cols, true, cancel),
            };
        }
        let data = match op {
            BooleanOp::And => boolean_binary(self.data(), right.data(), cancel, |a, b| a & b)?,
            BooleanOp::Or => boolean_binary(self.data(), right.data(), cancel, |a, b| a | b)?,
            BooleanOp::Xor => boolean_binary(self.data(), right.data(), cancel, |a, b| a ^ b)?,
        };
        Ok(Self::owned(rows, cols, data))
    }
}

impl std::fmt::Display for LogicalMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} x {} logical", self.rows, self.cols)?;
        for r in 0..self.rows.min(8) {
            for c in 0..self.cols.min(8) {
                write!(f, "  {}", self.data[c * self.rows + r])?;
            }
            if self.cols > 8 {
                write!(f, "  ...")?;
            }
            writeln!(f)?;
        }
        if self.rows > 8 {
            writeln!(f, "  ...")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_during_fill_discards_partial_result() {
        // Trigger cancellation from inside the first chunk, without timing races.
        for threads in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let cancel = AtomicBool::new(false);
                    let result =
                        build::<u8>(2 * PAR_ELEMENTS, PAR_ELEMENTS, &cancel, |first, output| {
                            output.fill(1);
                            if first == 0 {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            Ok(())
                        });
                    assert!(result.unwrap_err().contains("cancelled"));
                });
        }
    }
}
