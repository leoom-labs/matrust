//! Two-dimensional compressed sparse column (CSC) storage.
//!
//! Column pointers and row indices are `usize`; row indices are strictly
//! increasing inside each column and explicit zeros are never stored. Values
//! are `f64` for double sparse arrays and `bool` for logical ones. The storage
//! sits behind an `Arc`: assignment and workspace inspection share it, and
//! every write builds a new storage (copy-on-write).
use crate::{LogicalMatrix, Matrix, Result};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Most stored entries in one sparse array.
pub const MAX_NNZ: u64 = 100_000_000;
/// Most column pointers (columns + 1) in one sparse array.
pub const MAX_COLUMN_POINTERS: u64 = 100_000_000;
const CHUNK: usize = 8192;

/// Stored values: doubles, or booleans for logical sparse arrays.
#[derive(Clone, Debug, PartialEq)]
pub enum SparseValues {
    Double(Vec<f64>),
    Logical(Vec<bool>),
}
impl SparseValues {
    fn len(&self) -> usize {
        match self {
            Self::Double(v) => v.len(),
            Self::Logical(v) => v.len(),
        }
    }
    fn get(&self, k: usize) -> f64 {
        match self {
            Self::Double(v) => v[k],
            Self::Logical(v) => f64::from(u8::from(v[k])),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Csc {
    rows: usize,
    cols: usize,
    colptr: Vec<usize>,
    rowidx: Vec<usize>,
    values: SparseValues,
}

/// Immutable shared CSC matrix.
#[derive(Clone, Debug)]
pub struct SparseMatrix {
    data: Arc<Csc>,
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Execution cancelled".into())
    } else {
        Ok(())
    }
}

/// Checked size rules shared by every producer, applied before allocating.
pub fn check_shape(rows: usize, cols: usize, nnz: u64) -> Result<()> {
    let pointers = (cols as u64)
        .checked_add(1)
        .ok_or("sparse: too many columns")?;
    if pointers > MAX_COLUMN_POINTERS {
        return Err(format!(
            "sparse: {cols} columns exceed the limit of {} columns",
            MAX_COLUMN_POINTERS - 1
        ));
    }
    if nnz > MAX_NNZ {
        return Err(format!(
            "sparse: {nnz} nonzeros exceed the limit of {MAX_NNZ}"
        ));
    }
    if (rows as u64).checked_mul(cols as u64).is_none() || rows > i64::MAX as usize {
        return Err("sparse: dimensions overflow the addressable element count".into());
    }
    Ok(())
}

/// Builder for one column at a time, in column order.
pub(crate) struct Builder {
    rows: usize,
    cols: usize,
    logical: bool,
    colptr: Vec<usize>,
    rowidx: Vec<usize>,
    values: Vec<f64>,
}
impl Builder {
    pub(crate) fn new(rows: usize, cols: usize, logical: bool, capacity: usize) -> Result<Self> {
        check_shape(rows, cols, capacity as u64)?;
        let mut colptr = Vec::with_capacity(cols + 1);
        colptr.push(0);
        Ok(Self {
            rows,
            cols,
            logical,
            colptr,
            rowidx: Vec::with_capacity(capacity),
            values: Vec::with_capacity(capacity),
        })
    }
    /// Push one entry of the current column; rows must increase. Zeros are dropped.
    pub(crate) fn push(&mut self, row: usize, value: f64) -> Result<()> {
        if value != 0. {
            if self.rowidx.len() as u64 >= MAX_NNZ {
                return Err(format!(
                    "sparse: more than {MAX_NNZ} nonzeros are not supported"
                ));
            }
            debug_assert!(row < self.rows);
            self.rowidx.push(row);
            self.values.push(value);
        }
        Ok(())
    }
    pub(crate) fn end_column(&mut self) {
        self.colptr.push(self.rowidx.len());
    }
    pub(crate) fn finish(mut self) -> SparseMatrix {
        while self.colptr.len() < self.cols + 1 {
            self.colptr.push(self.rowidx.len());
        }
        let values = if self.logical {
            SparseValues::Logical(self.values.iter().map(|&v| v != 0.).collect())
        } else {
            SparseValues::Double(self.values)
        };
        SparseMatrix {
            data: Arc::new(Csc {
                rows: self.rows,
                cols: self.cols,
                colptr: self.colptr,
                rowidx: self.rowidx,
                values,
            }),
        }
    }
}

/// Zero-based row positions for `select` and `delete_rows`. Ranges (`:`,
/// `a:b`, `a:s:b`) stay ranges, so a huge row count costs nothing.
#[derive(Clone, Copy, Debug)]
pub enum Rows<'a> {
    /// Every row of the source.
    All,
    /// `start, start+step, ...`: `len` positions, `step` may be negative or 0.
    Range {
        start: usize,
        step: isize,
        len: usize,
    },
    /// Explicit positions, in output order.
    List(&'a [usize]),
}
impl Rows<'_> {
    fn len(&self, all: usize) -> usize {
        match self {
            Self::All => all,
            Self::Range { len, .. } => *len,
            Self::List(list) => list.len(),
        }
    }
}

/// The distinct rows of a deletion as an ascending range or sorted list.
enum DeletedRows {
    Range {
        start: usize,
        step: usize,
        len: usize,
    },
    List(Vec<usize>),
}
impl DeletedRows {
    fn new(rows: Rows<'_>, all: usize) -> Self {
        match rows {
            Rows::All => Self::Range {
                start: 0,
                step: 1,
                len: all,
            },
            Rows::Range { len: 0, .. } => Self::List(Vec::new()),
            Rows::Range { start, step: 0, .. } => Self::List(vec![start]),
            Rows::Range { start, step, len } if step > 0 => Self::Range {
                start,
                step: step as usize,
                len,
            },
            Rows::Range { start, step, len } => Self::Range {
                start: start - step.unsigned_abs() * (len - 1),
                step: step.unsigned_abs(),
                len,
            },
            Rows::List(list) => {
                let mut rows = list.to_vec();
                rows.sort_unstable();
                rows.dedup();
                Self::List(rows)
            }
        }
    }
    fn count(&self) -> usize {
        match self {
            Self::Range { len, .. } => *len,
            Self::List(rows) => rows.len(),
        }
    }
    fn contains(&self, r: usize) -> bool {
        match self {
            Self::Range { start, step, len } => {
                r >= *start && (r - start).is_multiple_of(*step) && (r - start) / step < *len
            }
            Self::List(rows) => rows.binary_search(&r).is_ok(),
        }
    }
    /// Deleted rows below `r`.
    fn below(&self, r: usize) -> usize {
        match self {
            Self::Range { start, step, len } => {
                if r <= *start {
                    0
                } else {
                    (r - start).div_ceil(*step).min(*len)
                }
            }
            Self::List(rows) => rows.partition_point(|&x| x < r),
        }
    }
}

impl SparseMatrix {
    /// All-zero sparse array.
    pub fn zeros(rows: usize, cols: usize, logical: bool) -> Result<Self> {
        Ok(Builder::new(rows, cols, logical, 0)?.finish())
    }
    /// Validated construction from raw CSC parts (MAT files).
    pub fn from_parts(
        rows: usize,
        cols: usize,
        colptr: Vec<usize>,
        rowidx: Vec<usize>,
        values: SparseValues,
    ) -> Result<Self> {
        check_shape(rows, cols, rowidx.len() as u64)?;
        if colptr.len() != cols + 1
            || colptr[0] != 0
            || colptr[cols] != rowidx.len()
            || values.len() != rowidx.len()
        {
            return Err("sparse: column pointers do not match the stored entries".into());
        }
        for c in 0..cols {
            let (a, b) = (colptr[c], colptr[c + 1]);
            if a > b || b > rowidx.len() {
                return Err("sparse: column pointers must not decrease".into());
            }
            for k in a..b {
                if rowidx[k] >= rows || (k > a && rowidx[k] <= rowidx[k - 1]) {
                    return Err("sparse: row indices must increase within each column".into());
                }
            }
        }
        let mut s = Self {
            data: Arc::new(Csc {
                rows,
                cols,
                colptr,
                rowidx,
                values,
            }),
        };
        if (0..s.nnz()).any(|k| s.value(k) == 0.) {
            // Drop explicit zeros so the storage invariant holds.
            s = s.map(|v| v, s.is_logical());
        }
        Ok(s)
    }
    pub fn from_dense(m: &Matrix, cancel: &AtomicBool) -> Result<Self> {
        Self::from_column_major(m.rows(), m.cols(), false, cancel, |i| m.data()[i])
    }
    pub fn from_logical(m: &LogicalMatrix, cancel: &AtomicBool) -> Result<Self> {
        Self::from_column_major(m.rows(), m.cols(), true, cancel, |i| f64::from(m.data()[i]))
    }
    fn from_column_major(
        rows: usize,
        cols: usize,
        logical: bool,
        cancel: &AtomicBool,
        at: impl Fn(usize) -> f64,
    ) -> Result<Self> {
        let len = rows * cols;
        let mut count = 0u64;
        for first in (0..len).step_by(CHUNK) {
            cancelled(cancel)?;
            count += (first..(first + CHUNK).min(len))
                .filter(|&i| at(i) != 0.)
                .count() as u64;
        }
        let mut b = Builder::new(rows, cols, logical, count as usize)?;
        for c in 0..cols {
            if c % 64 == 0 {
                cancelled(cancel)?;
            }
            for r in 0..rows {
                b.push(r, at(c * rows + r))?;
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// `speye(m,n)`.
    pub fn identity(rows: usize, cols: usize) -> Result<Self> {
        let n = rows.min(cols);
        let mut b = Builder::new(rows, cols, false, n)?;
        for c in 0..cols {
            if c < n {
                b.push(c, 1.)?;
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Triplets in input order: duplicates are summed in that order (logical: OR)
    /// and sums of zero are not stored.
    pub fn from_triplets(
        rows: usize,
        cols: usize,
        entries: &[(usize, usize, f64)],
        logical: bool,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check_shape(rows, cols, entries.len() as u64)?;
        let mut order: Vec<u32> = Vec::new();
        let mut order64: Vec<usize> = Vec::new();
        let small = entries.len() <= u32::MAX as usize;
        if small {
            order = (0..entries.len() as u32).collect();
            order.sort_by_key(|&k| (entries[k as usize].1, entries[k as usize].0));
        } else {
            order64 = (0..entries.len()).collect();
            order64.sort_by_key(|&k| (entries[k].1, entries[k].0));
        }
        let at = |i: usize| {
            if small { order[i] as usize } else { order64[i] }
        };
        cancelled(cancel)?;
        let mut b = Builder::new(rows, cols, logical, entries.len())?;
        let mut i = 0;
        let mut column = 0;
        while i < entries.len() {
            if i % CHUNK == 0 {
                cancelled(cancel)?;
            }
            let (r, c, _) = entries[at(i)];
            while column < c {
                b.end_column();
                column += 1;
            }
            let mut sum = 0.;
            let mut any = false;
            while i < entries.len() && entries[at(i)].0 == r && entries[at(i)].1 == c {
                let v = entries[at(i)].2;
                if logical {
                    any |= v != 0.;
                } else {
                    sum += v;
                }
                i += 1;
            }
            b.push(
                r,
                if logical {
                    f64::from(u8::from(any))
                } else {
                    sum
                },
            )?;
        }
        while column < cols {
            b.end_column();
            column += 1;
        }
        Ok(b.finish())
    }

    pub fn rows(&self) -> usize {
        self.data.rows
    }
    pub fn cols(&self) -> usize {
        self.data.cols
    }
    pub fn nnz(&self) -> usize {
        self.data.rowidx.len()
    }
    /// Element count as u64: a sparse array may hold more elements than memory.
    pub fn numel(&self) -> u64 {
        self.rows() as u64 * self.cols() as u64
    }
    pub fn is_logical(&self) -> bool {
        matches!(self.data.values, SparseValues::Logical(_))
    }
    pub fn is_empty(&self) -> bool {
        self.rows() == 0 || self.cols() == 0
    }
    pub fn colptr(&self) -> &[usize] {
        &self.data.colptr
    }
    pub fn rowidx(&self) -> &[usize] {
        &self.data.rowidx
    }
    pub fn values(&self) -> &SparseValues {
        &self.data.values
    }
    pub fn value(&self, k: usize) -> f64 {
        self.data.values.get(k)
    }
    /// Stored range of column `c`.
    pub fn column(&self, c: usize) -> std::ops::Range<usize> {
        self.data.colptr[c]..self.data.colptr[c + 1]
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
    /// MatRust's byte count: 16 per double entry (value + row index) or 9 per
    /// logical entry, plus 8 per column pointer.
    pub fn bytes(&self) -> usize {
        let per = if self.is_logical() { 9 } else { 16 };
        per * self.nnz() + 8 * (self.cols() + 1)
    }
    pub fn get(&self, r: usize, c: usize) -> f64 {
        let range = self.column(c);
        match self.data.rowidx[range.clone()].binary_search(&r) {
            Ok(k) => self.value(range.start + k),
            Err(_) => 0.,
        }
    }
    /// Every stored entry `(row, col, value)` in column order.
    pub fn entries(&self) -> impl Iterator<Item = (usize, usize, f64)> + '_ {
        (0..self.cols()).flat_map(move |c| {
            self.column(c)
                .map(move |k| (self.data.rowidx[k], c, self.value(k)))
        })
    }
    /// Dense column-major values; the caller has bounded `numel`.
    pub fn dense_values(&self, cancel: &AtomicBool) -> Result<Vec<f64>> {
        let len = Matrix::checked_len(self.rows(), self.cols())?;
        let mut data = vec![0.; len];
        for c in 0..self.cols() {
            if c % 256 == 0 {
                cancelled(cancel)?;
            }
            for k in self.column(c) {
                data[c * self.rows() + self.data.rowidx[k]] = self.value(k);
            }
        }
        Ok(data)
    }
    pub fn to_dense(&self, cancel: &AtomicBool) -> Result<Matrix> {
        Matrix::from_column_vec(self.rows(), self.cols(), self.dense_values(cancel)?)
    }
    pub fn to_dense_logical(&self, cancel: &AtomicBool) -> Result<LogicalMatrix> {
        let len = Matrix::checked_len(self.rows(), self.cols())?;
        let mut data = vec![0u8; len];
        for c in 0..self.cols() {
            if c % 256 == 0 {
                cancelled(cancel)?;
            }
            for k in self.column(c) {
                let v = self.value(k);
                if v.is_nan() {
                    return Err(NAN_TO_LOGICAL.into());
                }
                data[c * self.rows() + self.data.rowidx[k]] = 1;
            }
        }
        LogicalMatrix::from_column_vec(self.rows(), self.cols(), data, cancel)
    }
    /// Apply `f` to every stored value (f(0) is taken as 0); results equal to 0 are dropped.
    pub fn map(&self, f: impl Fn(f64) -> f64, logical: bool) -> Self {
        let mut colptr = Vec::with_capacity(self.cols() + 1);
        colptr.push(0);
        let mut rowidx = Vec::with_capacity(self.nnz());
        let mut values = Vec::with_capacity(self.nnz());
        for c in 0..self.cols() {
            for k in self.column(c) {
                let v = f(self.value(k));
                if v != 0. {
                    rowidx.push(self.data.rowidx[k]);
                    values.push(v);
                }
            }
            colptr.push(rowidx.len());
        }
        let values = if logical {
            SparseValues::Logical(values.iter().map(|&v| v != 0.).collect())
        } else {
            SparseValues::Double(values)
        };
        Self {
            data: Arc::new(Csc {
                rows: self.rows(),
                cols: self.cols(),
                colptr,
                rowidx,
                values,
            }),
        }
    }
    /// Same pattern and values as double.
    pub fn to_double(&self) -> Self {
        if self.is_logical() {
            self.map(|v| v, false)
        } else {
            self.clone()
        }
    }
    /// Logical conversion; NaN fails like the dense conversion.
    pub fn to_logical(&self) -> Result<Self> {
        if self.is_logical() {
            return Ok(self.clone());
        }
        if (0..self.nnz()).any(|k| self.value(k).is_nan()) {
            return Err(NAN_TO_LOGICAL.into());
        }
        Ok(self.map(|v| f64::from(u8::from(v != 0.)), true))
    }
    /// `S.'`. The result's column pointers (one per source row) are bounded
    /// before anything is allocated.
    pub fn transpose(&self) -> Result<Self> {
        let (m, n) = (self.rows(), self.cols());
        check_shape(n, m, self.nnz() as u64)?;
        let mut counts = vec![0usize; m + 1];
        for &r in self.rowidx() {
            counts[r + 1] += 1;
        }
        for r in 0..m {
            counts[r + 1] += counts[r];
        }
        let colptr = counts.clone();
        let mut next = counts;
        let mut rowidx = vec![0; self.nnz()];
        let mut values = vec![0.; self.nnz()];
        for c in 0..n {
            for k in self.column(c) {
                let r = self.data.rowidx[k];
                let p = next[r];
                next[r] += 1;
                rowidx[p] = c;
                values[p] = self.value(k);
            }
        }
        let values = if self.is_logical() {
            SparseValues::Logical(values.iter().map(|&v| v != 0.).collect())
        } else {
            SparseValues::Double(values)
        };
        Ok(Self {
            data: Arc::new(Csc {
                rows: n,
                cols: m,
                colptr,
                rowidx,
                values,
            }),
        })
    }
    /// Column-major reshape; the storage order is kept.
    pub fn reshape(&self, rows: usize, cols: usize) -> Result<Self> {
        if rows as u64 * cols as u64 != self.numel() {
            return Err("reshape: the number of elements must not change".into());
        }
        let mut b = Builder::new(rows, cols, self.is_logical(), self.nnz())?;
        let mut column = 0;
        for (r, c, v) in self.entries() {
            let linear = c as u64 * self.rows() as u64 + r as u64;
            let (nr, nc) = (
                (linear % rows as u64) as usize,
                (linear / rows as u64) as usize,
            );
            while column < nc {
                b.end_column();
                column += 1;
            }
            b.push(nr, v)?;
        }
        Ok(b.finish())
    }
    /// S + sign*T for equal shapes.
    pub fn add(&self, other: &Self, sign: f64) -> Result<Self> {
        if (self.rows(), self.cols()) != (other.rows(), other.cols()) {
            return Err("Matrix dimensions must agree".into());
        }
        let mut b = Builder::new(
            self.rows(),
            self.cols(),
            false,
            (self.nnz() + other.nnz()).min(MAX_NNZ as usize),
        )?;
        for c in 0..self.cols() {
            let (mut i, ie) = (self.column(c).start, self.column(c).end);
            let (mut j, je) = (other.column(c).start, other.column(c).end);
            while i < ie || j < je {
                let ri = if i < ie { self.rowidx()[i] } else { usize::MAX };
                let rj = if j < je {
                    other.rowidx()[j]
                } else {
                    usize::MAX
                };
                if ri < rj {
                    b.push(ri, self.value(i))?;
                    i += 1;
                } else if rj < ri {
                    b.push(rj, sign * other.value(j))?;
                    j += 1;
                } else {
                    b.push(ri, self.value(i) + sign * other.value(j))?;
                    i += 1;
                    j += 1;
                }
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Elementwise product with another sparse array of the same shape.
    pub fn times_sparse(&self, other: &Self) -> Result<Self> {
        if (self.rows(), self.cols()) != (other.rows(), other.cols()) {
            return Err("Matrix dimensions must agree".into());
        }
        let mut b = Builder::new(self.rows(), self.cols(), false, self.nnz().min(other.nnz()))?;
        for c in 0..self.cols() {
            let (mut i, ie) = (self.column(c).start, self.column(c).end);
            let (mut j, je) = (other.column(c).start, other.column(c).end);
            while i < ie && j < je {
                let (ri, rj) = (self.rowidx()[i], other.rowidx()[j]);
                if ri < rj {
                    i += 1;
                } else if rj < ri {
                    j += 1;
                } else {
                    b.push(ri, self.value(i) * other.value(j))?;
                    i += 1;
                    j += 1;
                }
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Elementwise product with a full array of the same shape. A non-finite
    /// full value at a structural zero gives NaN (IEEE 0*Inf), as in Octave.
    pub fn times_dense(&self, f: &Matrix, cancel: &AtomicBool) -> Result<Self> {
        if (self.rows(), self.cols()) != (f.rows(), f.cols()) {
            return Err("Matrix dimensions must agree".into());
        }
        let m = self.rows();
        let mut b = Builder::new(m, self.cols(), false, self.nnz())?;
        for c in 0..self.cols() {
            if c % 64 == 0 {
                cancelled(cancel)?;
            }
            let column = &f.data()[c * m..(c + 1) * m];
            let mut k = self.column(c).start;
            let end = self.column(c).end;
            for (r, &x) in column.iter().enumerate() {
                if k < end && self.rowidx()[k] == r {
                    b.push(r, self.value(k) * x)?;
                    k += 1;
                } else if !x.is_finite() {
                    b.push(r, f64::NAN)?;
                }
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Sparse matrix product (column-by-column accumulation).
    pub fn matmul(&self, other: &Self, cancel: &AtomicBool) -> Result<Self> {
        if self.cols() != other.rows() {
            return Err("Inner matrix dimensions must agree".into());
        }
        let m = self.rows();
        let mut b = Builder::new(m, other.cols(), false, 0)?;
        let mut pairs: Vec<(usize, f64)> = Vec::new();
        for j in 0..other.cols() {
            cancelled(cancel)?;
            pairs.clear();
            for kb in other.column(j) {
                let (k, x) = (other.rowidx()[kb], other.value(kb));
                for ka in self.column(k) {
                    pairs.push((self.rowidx()[ka], self.value(ka) * x));
                    if pairs.len() as u64 > MAX_NNZ {
                        return Err(format!(
                            "mtimes: sparse product needs more than {MAX_NNZ} intermediate entries"
                        ));
                    }
                }
            }
            pairs.sort_by_key(|p| p.0);
            let mut i = 0;
            while i < pairs.len() {
                let r = pairs[i].0;
                let mut sum = 0.;
                while i < pairs.len() && pairs[i].0 == r {
                    sum += pairs[i].1;
                    i += 1;
                }
                b.push(r, sum)?;
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// S*F with a full F: full result. Structural zeros do not take part.
    pub fn matmul_dense(&self, f: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
        if self.cols() != f.rows() {
            return Err("Inner matrix dimensions must agree".into());
        }
        let (m, p) = (self.rows(), f.cols());
        let mut out = vec![0.; Matrix::checked_len(m, p)?];
        for j in 0..p {
            cancelled(cancel)?;
            let y = &mut out[j * m..(j + 1) * m];
            for k in 0..self.cols() {
                let x = f.data()[j * f.rows() + k];
                for ka in self.column(k) {
                    y[self.rowidx()[ka]] += self.value(ka) * x;
                }
            }
        }
        Matrix::from_column_vec(m, p, out)
    }
    /// F*S with a full F: full result.
    pub fn dense_matmul(f: &Matrix, s: &Self, cancel: &AtomicBool) -> Result<Matrix> {
        if f.cols() != s.rows() {
            return Err("Inner matrix dimensions must agree".into());
        }
        let (m, n) = (f.rows(), s.cols());
        let mut out = vec![0.; Matrix::checked_len(m, n)?];
        for j in 0..n {
            cancelled(cancel)?;
            let y = &mut out[j * m..(j + 1) * m];
            for kb in s.column(j) {
                let (k, x) = (s.rowidx()[kb], s.value(kb));
                let column = &f.data()[k * m..(k + 1) * m];
                for (yi, &a) in y.iter_mut().zip(column) {
                    *yi += a * x;
                }
            }
        }
        Matrix::from_column_vec(m, n, out)
    }
    /// Logical AND with another array value given by `at(r,c)`; `pattern`
    /// limits the candidates to this array's stored entries.
    pub fn and_with(&self, at: impl Fn(usize, usize) -> f64) -> Result<Self> {
        let mut b = Builder::new(self.rows(), self.cols(), true, self.nnz())?;
        for c in 0..self.cols() {
            for k in self.column(c) {
                let r = self.rowidx()[k];
                let (x, y) = (self.value(k), at(r, c));
                if x.is_nan() || y.is_nan() {
                    return Err(NAN_TO_LOGICAL.into());
                }
                b.push(r, f64::from(u8::from(y != 0.)))?;
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Rows/columns selection: `rows` in output order, `cols` zero-based.
    /// Work is O(stored entries of the selected columns + columns); a row range
    /// never becomes a per-row list.
    pub fn select(&self, rows: Rows<'_>, cols: &[usize], cancel: &AtomicBool) -> Result<Self> {
        let out_rows = rows.len(self.rows());
        // Source row -> output rows for an explicit list, sorted by source row.
        let mut map: Vec<(usize, usize)> = Vec::new();
        if let Rows::List(list) = rows {
            map = list.iter().enumerate().map(|(o, &r)| (r, o)).collect();
            map.sort_unstable();
        }
        let mut b = Builder::new(out_rows, cols.len(), self.is_logical(), 0)?;
        let mut pending: Vec<(usize, f64)> = Vec::new();
        for (j, &c) in cols.iter().enumerate() {
            if j % 64 == 0 {
                cancelled(cancel)?;
            }
            if let Rows::All = rows {
                for k in self.column(c) {
                    b.push(self.rowidx()[k], self.value(k))?;
                }
                b.end_column();
                continue;
            }
            pending.clear();
            for k in self.column(c) {
                let (r, v) = (self.rowidx()[k], self.value(k));
                match rows {
                    Rows::All => unreachable!("handled above"),
                    Rows::Range {
                        start,
                        step: 0,
                        len,
                    } => {
                        if r == start {
                            pending.extend((0..len).map(|o| (o, v)));
                        }
                    }
                    Rows::Range { start, step, len } => {
                        let offset = r as i128 - start as i128;
                        let step = step as i128;
                        if offset % step == 0 && (0..len as i128).contains(&(offset / step)) {
                            pending.push(((offset / step) as usize, v));
                        }
                    }
                    Rows::List(_) => {
                        let first = map.partition_point(|p| p.0 < r);
                        for &(_, o) in map[first..].iter().take_while(|p| p.0 == r) {
                            pending.push((o, v));
                        }
                    }
                }
            }
            pending.sort_unstable_by_key(|p| p.0);
            for &(o, v) in &pending {
                b.push(o, v)?;
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Values at zero-based linear indices, laid out column-major in an
    /// `out_rows x out_cols` result.
    pub fn select_linear(
        &self,
        indices: impl Iterator<Item = u64>,
        out_rows: usize,
        out_cols: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let mut b = Builder::new(out_rows, out_cols, self.is_logical(), 0)?;
        let m = self.rows().max(1) as u64;
        let mut column = 0;
        for (p, linear) in indices.enumerate() {
            if p % CHUNK == 0 {
                cancelled(cancel)?;
            }
            let (oc, or) = (p / out_rows.max(1), p % out_rows.max(1));
            while column < oc {
                b.end_column();
                column += 1;
            }
            let v = self.get((linear % m) as usize, (linear / m) as usize);
            b.push(or, v)?;
        }
        Ok(b.finish())
    }
    /// Apply writes `(row, col, value)` (last wins; zero deletes) to a matrix
    /// of the (possibly grown) shape `rows x cols`.
    pub fn with_writes(
        &self,
        rows: usize,
        cols: usize,
        writes: &mut [(usize, usize, f64)],
        cancel: &AtomicBool,
    ) -> Result<Self> {
        // Stable sort: equal positions keep their order, the last one wins.
        writes.sort_by_key(|w| (w.1, w.0));
        cancelled(cancel)?;
        let mut b = Builder::new(rows, cols, self.is_logical(), 0)?;
        let mut w = 0;
        for c in 0..cols {
            if c % 256 == 0 {
                cancelled(cancel)?;
            }
            let range = if c < self.cols() {
                self.column(c)
            } else {
                0..0
            };
            let mut k = range.start;
            loop {
                let rk = if k < range.end {
                    self.rowidx()[k]
                } else {
                    usize::MAX
                };
                let rw = if w < writes.len() && writes[w].1 == c {
                    writes[w].0
                } else {
                    usize::MAX
                };
                if rk == usize::MAX && rw == usize::MAX {
                    break;
                }
                if rk < rw {
                    b.push(rk, self.value(k))?;
                    k += 1;
                } else {
                    let mut v = writes[w].2;
                    while w + 1 < writes.len() && writes[w + 1].1 == c && writes[w + 1].0 == rw {
                        w += 1;
                        v = writes[w].2;
                    }
                    w += 1;
                    if rk == rw {
                        k += 1;
                    }
                    b.push(
                        rw,
                        if self.is_logical() {
                            f64::from(u8::from(v != 0.))
                        } else {
                            v
                        },
                    )?;
                }
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Delete a set of rows (positions may repeat). Work is O(stored entries
    /// x log(deleted rows)): rows are renumbered by counting the deleted rows
    /// below them, never through a per-row table.
    pub fn delete_rows(&self, rows: Rows<'_>, cancel: &AtomicBool) -> Result<Self> {
        let set = DeletedRows::new(rows, self.rows());
        let mut b = Builder::new(
            self.rows() - set.count(),
            self.cols(),
            self.is_logical(),
            self.nnz(),
        )?;
        for c in 0..self.cols() {
            if c % 256 == 0 {
                cancelled(cancel)?;
            }
            for k in self.column(c) {
                let r = self.rowidx()[k];
                if !set.contains(r) {
                    b.push(r - set.below(r), self.value(k))?;
                }
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Delete a set of zero-based columns (positions may repeat).
    pub fn delete_columns(&self, cols: &[usize], cancel: &AtomicBool) -> Result<Self> {
        let mut deleted = cols.to_vec();
        deleted.sort_unstable();
        deleted.dedup();
        let mut b = Builder::new(
            self.rows(),
            self.cols() - deleted.len(),
            self.is_logical(),
            self.nnz(),
        )?;
        let mut next = 0;
        for c in 0..self.cols() {
            if c % 256 == 0 {
                cancelled(cancel)?;
            }
            if deleted.get(next) == Some(&c) {
                next += 1;
                continue;
            }
            for k in self.column(c) {
                b.push(self.rowidx()[k], self.value(k))?;
            }
            b.end_column();
        }
        Ok(b.finish())
    }
    /// Horizontal concatenation of blocks with equal row counts.
    pub fn hcat(blocks: &[Self]) -> Result<Self> {
        let rows = blocks.first().map_or(0, Self::rows);
        if blocks.iter().any(|b| b.rows() != rows) {
            return Err("Horizontal concatenation requires matching row counts".into());
        }
        let cols = blocks
            .iter()
            .try_fold(0usize, |n, b| n.checked_add(b.cols()));
        let nnz = blocks.iter().map(|b| b.nnz() as u64).sum::<u64>();
        let logical = !blocks.is_empty() && blocks.iter().all(Self::is_logical);
        let mut out = Builder::new(rows, cols.ok_or("sparse: too many columns")?, logical, 0)?;
        check_shape(rows, out.cols, nnz)?;
        for block in blocks {
            for c in 0..block.cols() {
                for k in block.column(c) {
                    out.push(block.rowidx()[k], block.value(k))?;
                }
                out.end_column();
            }
        }
        Ok(out.finish())
    }
    /// Vertical concatenation of blocks with equal column counts.
    pub fn vcat(blocks: &[Self]) -> Result<Self> {
        let cols = blocks.first().map_or(0, Self::cols);
        if blocks.iter().any(|b| b.cols() != cols) {
            return Err("Vertical concatenation requires matching column counts".into());
        }
        let rows = blocks
            .iter()
            .try_fold(0usize, |n, b| n.checked_add(b.rows()))
            .ok_or("sparse: too many rows")?;
        let nnz = blocks.iter().map(|b| b.nnz() as u64).sum::<u64>();
        check_shape(rows, cols, nnz)?;
        let logical = !blocks.is_empty() && blocks.iter().all(Self::is_logical);
        let mut out = Builder::new(rows, cols, logical, nnz as usize)?;
        for c in 0..cols {
            let mut offset = 0;
            for block in blocks {
                for k in block.column(c) {
                    out.push(offset + block.rowidx()[k], block.value(k))?;
                }
                offset += block.rows();
            }
            out.end_column();
        }
        Ok(out.finish())
    }
    /// Structure and values equal (NaN equal to NaN when `nan_equal`).
    pub fn equals(&self, other: &Self, nan_equal: bool) -> bool {
        (self.rows(), self.cols()) == (other.rows(), other.cols())
            && self.colptr() == other.colptr()
            && self.rowidx() == other.rowidx()
            && (0..self.nnz()).all(|k| {
                let (a, b) = (self.value(k), other.value(k));
                a == b || (nan_equal && a.is_nan() && b.is_nan())
            })
    }
    /// Compare with a dense column-major array of the same shape.
    pub fn equals_dense(&self, data: &[f64], nan_equal: bool) -> bool {
        let m = self.rows();
        if data.len() != m * self.cols() {
            return false;
        }
        for c in 0..self.cols() {
            let mut k = self.column(c).start;
            for (r, &d) in data[c * m..(c + 1) * m].iter().enumerate() {
                let s = if k < self.column(c).end && self.rowidx()[k] == r {
                    k += 1;
                    self.value(k - 1)
                } else {
                    0.
                };
                if !(s == d || (nan_equal && s.is_nan() && d.is_nan())) {
                    return false;
                }
            }
        }
        true
    }
}

/// Dense logical conversion message for NaN (same as the dense path).
pub const NAN_TO_LOGICAL: &str = "NaN cannot be converted to logical";
