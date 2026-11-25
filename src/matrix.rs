use crate::Result;
use nalgebra::{DMatrix, DMatrixView, DMatrixViewMut};
use rayon::prelude::*;
use std::{borrow::Cow, fmt, sync::Arc};
mod calculus;
mod coordinate_proof;
mod elementwise;
mod median;
mod product;
pub use median::MedianAxis;
pub(crate) use median::MedianScratch;
mod reductions;
pub use reductions::{
    Extreme, complex_pair_picks_second, find_nonzero, find_nonzero_complex, find_shape,
    truth_along, truth_along_complex,
};
mod statistics;
pub(crate) use statistics::dispersion_lane;
pub use statistics::{Dispersion, StatisticsAxis};
mod transform;

/// Element bound per dense result of any class: 48 Mi native (a 16 MP RGB image),
/// 16 Mi on wasm. The engine's `Limits` also bound memory per statement and workspace.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_ELEMENTS: usize = 48 * 1024 * 1024;
#[cfg(target_arch = "wasm32")]
pub const MAX_ELEMENTS: usize = 16 * 1024 * 1024;
const PAR_ELEMENTS: usize = 64 * 1024;
const PAR_GEMM_WORK: usize = 8 * 1024 * 1024;
const FAST_GEMM_WORK: usize = 64 * 64 * 64;

pub(crate) const INCOMPATIBLE_SIZES: &str = "Arrays have incompatible sizes for this operation.";

/// The result extent is the non-singleton extent, including zero.
pub(crate) fn expansion_extent(a: usize, b: usize) -> Result<usize> {
    if a == b || b == 1 {
        Ok(a)
    } else if a == 1 {
        Ok(b)
    } else {
        Err(INCOMPATIBLE_SIZES.into())
    }
}

pub(crate) fn expansion_shape(a: (usize, usize), b: (usize, usize)) -> Result<(usize, usize)> {
    Ok((expansion_extent(a.0, b.0)?, expansion_extent(a.1, b.1)?))
}

/// Maps a column-major result index back to a compatible operand.
pub(crate) fn expanded_2d_index(i: usize, output: (usize, usize), input: (usize, usize)) -> usize {
    match input {
        _ if input == output => i,
        (1, 1) => 0,
        (1, _) => i / output.0,
        _ => i % output.0,
    }
}

/// Immutable, shared, column-major f64 storage. Assignment and lookup are O(1).
#[derive(Clone, Debug)]
pub struct Matrix {
    // The allocation's original shape is private: every kernel uses view() or
    // data() with the logical dimensions below. Reshape only changes metadata.
    pub(crate) storage: Arc<DMatrix<f64>>,
    rows: usize,
    cols: usize,
    nd: Option<Arc<Vec<usize>>>,
    coordinates: Option<Arc<coordinate_proof::CoordinateProof>>,
    uniform: Option<f64>,
}

impl Matrix {
    fn from_owned(matrix: DMatrix<f64>) -> Self {
        Self {
            rows: matrix.nrows(),
            cols: matrix.ncols(),
            nd: None,
            storage: Arc::new(matrix),
            coordinates: None,
            uniform: None,
        }
    }
    /// Internal immutable backing view for native/script export. No proof or
    /// uniform claim survives; the original column-major allocation is shared.
    pub(crate) fn from_shared_coordinate_storage(storage: Arc<DMatrix<f64>>) -> Result<Self> {
        Self::checked_len(storage.len(), 1)?;
        Ok(Self {
            rows: storage.len(),
            cols: 1,
            nd: None,
            storage,
            coordinates: None,
            uniform: None,
        })
    }
    /// The caller's ordinary repetition kernel copied a single scalar exactly.
    /// No arithmetic is substituted, including for NaN payloads and signed zero.
    pub(crate) fn inherit_scalar_repetition(&mut self, input: &Self) {
        if input.len() == 1 {
            self.coordinates = None;
            self.uniform = Some(input.data()[0]);
        }
    }
    /// The caller completed an exact scalar fill of every element and its final
    /// cancellation check. This records that fact without allocation or a scan;
    /// arbitrary data constructors must not call this method.
    pub(crate) fn inherit_completed_uniform_fill(&mut self, value: f64) {
        self.coordinates = None;
        self.uniform = Some(value);
    }
    fn view(&self) -> DMatrixView<'_, f64> {
        DMatrixView::from_slice(self.data(), self.rows(), self.cols())
    }
    pub fn checked_len(rows: usize, cols: usize) -> Result<usize> {
        let n = rows.checked_mul(cols).ok_or("Matrix dimensions overflow")?;
        if n > MAX_ELEMENTS || rows > MAX_ELEMENTS || cols > MAX_ELEMENTS {
            return Err(format!("Matrix exceeds the {MAX_ELEMENTS}-element limit"));
        }
        Ok(n)
    }
    pub fn from_column_slice(rows: usize, cols: usize, data: &[f64]) -> Result<Self> {
        if Self::checked_len(rows, cols)? != data.len() {
            return Err("Data does not match matrix dimensions".into());
        }
        Ok(Self::from_owned(DMatrix::from_column_slice(
            rows, cols, data,
        )))
    }
    /// Takes ownership of a column-major buffer, avoiding a second result allocation/copy.
    pub fn from_column_vec(rows: usize, cols: usize, data: Vec<f64>) -> Result<Self> {
        if Self::checked_len(rows, cols)? != data.len() {
            return Err("Data does not match matrix dimensions".into());
        }
        Ok(Self::from_owned(DMatrix::from_vec(rows, cols, data)))
    }
    pub fn filled(rows: usize, cols: usize, value: f64) -> Result<Self> {
        Self::checked_len(rows, cols)?;
        let mut result = Self::from_owned(DMatrix::from_element(rows, cols, value));
        result.uniform = Some(value);
        Ok(result)
    }
    pub fn scalar(value: f64) -> Self {
        Self::from_owned(DMatrix::from_element(1, 1, value))
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn dims(&self) -> Cow<'_, [usize]> {
        self.nd.as_deref().map_or_else(
            || Cow::Owned(vec![self.rows, self.cols]),
            |dims| Cow::Borrowed(dims.as_slice()),
        )
    }
    pub fn ndims(&self) -> usize {
        self.nd.as_ref().map_or(2, |dims| dims.len())
    }
    pub fn is_nd(&self) -> bool {
        self.nd.is_some()
    }
    /// Reshape only the metadata while retaining immutable shared storage.
    pub fn with_dims(&self, dims: &[usize]) -> Result<Self> {
        let dims = crate::array::normalize_dims(dims)?;
        if dims[0] != self.rows
            || dims[1..].iter().try_fold(1usize, |n, &d| n.checked_mul(d)) != Some(self.cols)
        {
            return Err("reshape dimensions do not match folded matrix storage".into());
        }
        let mut result = self.clone();
        result.nd = (dims.len() > 2).then(|| Arc::new(dims));
        Ok(result)
    }
    pub fn len(&self) -> usize {
        self.storage.len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }
    pub fn data(&self) -> &[f64] {
        self.storage.as_slice()
    }
    pub fn get(&self, row: usize, col: usize) -> f64 {
        self.view()[(row, col)]
    }
    pub fn with_value(&self, row: usize, col: usize, value: f64) -> Result<Self> {
        if row >= self.rows() || col >= self.cols() {
            return Err("Cell is outside the matrix dimensions".into());
        }
        let mut data = self.data().to_vec();
        data[col * self.rows() + row] = value;
        Self::from_column_vec(self.rows(), self.cols(), data)
    }
    /// A bounded, already-validated commit. Shared allocations are never mutated.
    pub(crate) fn write_unique(&mut self, changes: &[(usize, f64)]) -> bool {
        assert!(changes.len() <= 8192);
        assert!(changes.iter().all(|(i, _)| *i < self.len()));
        if let Some(storage) = Arc::get_mut(&mut self.storage) {
            for &(i, value) in changes {
                storage.as_mut_slice()[i] = value;
            }
            self.coordinates = None;
            self.uniform = None;
            true
        } else {
            false
        }
    }
    /// Update bounded contiguous regions without copying the complete matrix.
    /// Capture every original value first, so overlaps also roll back correctly.
    pub(crate) fn write_unique_regions(
        &mut self,
        regions: &[std::ops::Range<usize>],
        values: &[f64],
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<bool> {
        let Some(storage) = Arc::get_mut(&mut self.storage) else {
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
        let data = storage.as_mut_slice();
        let mut undo = Vec::with_capacity(count);
        for region in regions {
            for chunk in data[region.clone()].chunks(8192) {
                check()?;
                undo.extend_from_slice(chunk);
            }
        }
        let result = (|| -> Result<()> {
            let mut source = 0;
            for region in regions {
                for chunk in data[region.clone()].chunks_mut(8192) {
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
            // Rollback is bounded by 16 MiB and cannot itself be cancelled.
            // Duplicate/overlapping regions all captured the same original data.
            let mut source = 0;
            for region in regions {
                data[region.clone()].copy_from_slice(&undo[source..source + region.len()]);
                source += region.len();
            }
        }
        if result.is_ok() {
            self.coordinates = None;
            self.uniform = None;
        }
        result.map(|_| true)
    }
    pub fn as_scalar(&self) -> Result<f64> {
        if self.len() == 1 {
            Ok(self.data()[0])
        } else {
            Err("Expected a scalar".into())
        }
    }
    pub fn identity(rows: usize, cols: usize) -> Result<Self> {
        Self::checked_len(rows, cols)?;
        Ok(Self::from_owned(DMatrix::identity(rows, cols)))
    }
    pub fn map(&self, f: impl Fn(f64) -> f64 + Sync + Send) -> Self {
        let mut out = DMatrix::zeros(self.rows(), self.cols());
        if self.len() >= PAR_ELEMENTS {
            out.as_mut_slice()
                .par_iter_mut()
                .zip(self.data().par_iter())
                .for_each(|(o, &v)| *o = f(v));
        } else {
            out.as_mut_slice()
                .iter_mut()
                .zip(self.data())
                .for_each(|(o, &v)| *o = f(v));
        }
        Self::from_owned(out)
    }
    pub fn zip(&self, rhs: &Self, f: impl Fn(f64, f64) -> f64 + Sync + Send) -> Result<Self> {
        self.zip_checked(rhs, f, || Ok(()))
    }
    pub(crate) fn zip_checked(
        &self,
        rhs: &Self,
        f: impl Fn(f64, f64) -> f64 + Sync + Send,
        check: impl Fn() -> Result<()>,
    ) -> Result<Self> {
        if self.len() == 1 {
            return Ok(rhs.map(|b| f(self.data()[0], b)));
        }
        if rhs.len() == 1 {
            return Ok(self.map(|a| f(a, rhs.data()[0])));
        }
        if (self.rows(), self.cols()) != (rhs.rows(), rhs.cols()) {
            let (rows, cols) =
                expansion_shape((self.rows(), self.cols()), (rhs.rows(), rhs.cols()))?;
            let count = Self::checked_len(rows, cols)?;
            check()?;
            let mut data = Vec::with_capacity(count);
            if count != 0 {
                let (a_row, b_row) = (usize::from(self.rows() != 1), usize::from(rhs.rows() != 1));
                let (a_col, b_col) = (
                    if self.cols() == 1 { 0 } else { self.rows() },
                    if rhs.cols() == 1 { 0 } else { rhs.rows() },
                );
                let mut next_check = 8192;
                for col in 0..cols {
                    let (a, b) = (col * a_col, col * b_col);
                    for first in (0..rows).step_by(8192) {
                        data.extend((first..(first + 8192).min(rows)).map(|row| {
                            f(self.data()[a + row * a_row], rhs.data()[b + row * b_row])
                        }));
                        if data.len() >= next_check {
                            check()?;
                            next_check = data.len() + 8192;
                        }
                    }
                }
                check()?;
            }
            return Self::from_column_vec(rows, cols, data);
        }
        let mut out = DMatrix::zeros(self.rows(), self.cols());
        if self.len() >= PAR_ELEMENTS {
            out.as_mut_slice()
                .par_iter_mut()
                .zip(self.data().par_iter().zip(rhs.data().par_iter()))
                .for_each(|(o, (&a, &b))| *o = f(a, b));
        } else {
            for ((o, &a), &b) in out
                .as_mut_slice()
                .iter_mut()
                .zip(self.data())
                .zip(rhs.data())
            {
                *o = f(a, b);
            }
        }
        Ok(Self::from_owned(out))
    }
    pub fn mul(&self, rhs: &Self) -> Result<Self> {
        if self.len() == 1 || rhs.len() == 1 {
            return self.coordinate_multiply(rhs);
        }
        if self.cols() != rhs.rows() {
            return Err(format!(
                "Cannot multiply {}x{} by {}x{}",
                self.rows(),
                self.cols(),
                rhs.rows(),
                rhs.cols()
            ));
        }
        Self::checked_len(self.rows(), rhs.cols())?;
        let mut out = DMatrix::zeros(self.rows(), rhs.cols());
        if out.is_empty() || self.cols() == 0 {
            return Ok(Self::from_owned(out));
        }
        let work = self
            .rows()
            .saturating_mul(self.cols())
            .saturating_mul(rhs.cols());
        let workers = rayon::current_num_threads();
        let parallel = work >= PAR_GEMM_WORK && workers > 1;
        // Tiny/vector and shallow-inner cases retain the existing kernel: packing
        // can cost more than it saves there. Larger products use GEMM's scheduling.
        if work >= FAST_GEMM_WORK && self.rows() > 1 && rhs.cols() > 1 && self.cols() >= 64 {
            product::multiply(
                self,
                rhs,
                out.as_mut_slice(),
                if parallel { workers } else { 1 },
            );
        } else if work >= PAR_GEMM_WORK && rhs.cols() >= 32 && workers > 1 {
            // Each worker owns contiguous output columns; no locks or partial-sum reductions.
            let columns = rhs.cols().div_ceil(rayon::current_num_threads()).max(32);
            out.as_mut_slice()
                .par_chunks_mut(self.rows() * columns)
                .enumerate()
                .for_each(|(i, chunk)| {
                    let count = chunk.len() / self.rows();
                    let rhs_view = rhs.view();
                    let b = rhs_view.columns(i * columns, count);
                    let mut c = DMatrixViewMut::from_slice(chunk, self.rows(), count);
                    c.gemm(1.0, &self.view(), &b, 0.0);
                });
        } else {
            out.gemm(1.0, &self.view(), &rhs.view(), 0.0);
        }
        Ok(Self::from_owned(out))
    }
    pub fn transpose(&self) -> Self {
        let mut result = Self::from_owned(self.view().transpose());
        result.coordinates = self.transpose_proof();
        result.uniform = self.uniform;
        result
    }
    pub fn solve(&self, rhs: &Self) -> Result<Self> {
        if self.len() == 1 {
            return Ok(rhs.map(|v| v / self.data()[0]));
        }
        if self.rows() != self.cols() {
            return Err("This version supports square linear systems only".into());
        }
        if rhs.rows() != self.rows() {
            return Err("Linear system row dimensions must agree".into());
        }
        self.require_finite()?;
        rhs.require_finite()?;
        self.view()
            .into_owned()
            .lu()
            .solve(&rhs.view())
            .map(Self::from_owned)
            .ok_or_else(|| "Matrix is singular; no unique solution".into())
    }
    pub fn inverse(&self) -> Result<Self> {
        self.solve(&Self::identity(self.rows(), self.cols())?)
    }
    pub fn det(&self) -> Result<Self> {
        if self.rows() != self.cols() {
            return Err("det requires a square matrix".into());
        }
        self.require_finite()?;
        Ok(Self::scalar(self.view().into_owned().lu().determinant()))
    }
    fn require_finite(&self) -> Result<()> {
        if self.data().iter().any(|v| !v.is_finite()) {
            Err("Factorization requires finite values".into())
        } else {
            Ok(())
        }
    }
    pub fn power(&self, exponent: f64) -> Result<Self> {
        if self.len() == 1 {
            return Ok(Self::scalar(self.data()[0].powf(exponent)));
        }
        if self.rows() != self.cols()
            || !exponent.is_finite()
            || exponent.fract() != 0.0
            || exponent.abs() > 1_000_000.0
        {
            return Err("Matrix power requires a square matrix and an integer exponent of magnitude <= 1000000".into());
        }
        let mut base = if exponent < 0.0 {
            self.inverse()?
        } else {
            self.clone()
        };
        let mut result = Self::identity(self.rows(), self.cols())?;
        let mut n = exponent.abs() as u64;
        while n > 0 {
            if n & 1 == 1 {
                result = result.mul(&base)?;
            }
            n >>= 1;
            if n > 0 {
                base = base.mul(&base)?;
            }
        }
        Ok(result)
    }
    pub fn reduce(&self, mean: bool) -> Result<Self> {
        // MATLAB reduces along the first nonsingleton dimension.
        if self.rows() == 0 && self.cols() == 0 {
            return Ok(Self::scalar(if mean { f64::NAN } else { 0.0 }));
        }
        if self.rows() == 1 {
            let sum: f64 = self.data().iter().sum();
            return Ok(Self::scalar(if mean {
                sum / self.cols() as f64
            } else {
                sum
            }));
        }
        let values: Vec<_> = (0..self.cols())
            .map(|c| {
                let sum: f64 = self.view().column(c).iter().sum();
                if mean { sum / self.rows() as f64 } else { sum }
            })
            .collect();
        Self::from_column_slice(1, self.cols(), &values)
    }
    pub fn reshape(&self, rows: usize, cols: usize) -> Result<Self> {
        if Self::checked_len(rows, cols)? != self.len() {
            return Err("reshape must preserve the number of elements".into());
        }
        Ok(Self {
            storage: self.storage.clone(),
            coordinates: self.coordinates.clone(),
            uniform: self.uniform,
            rows,
            cols,
            nd: None,
        })
    }
    pub fn concatenate(mut blocks: Vec<Vec<Self>>) -> Result<Self> {
        // MATLAB ignores 0x0 operands, including rows made entirely of [].
        // Retain moves only shared matrix handles; it never copies payloads.
        for row in &mut blocks {
            row.retain(|block| (block.rows(), block.cols()) != (0, 0));
        }
        blocks.retain(|row| !row.is_empty());
        if blocks.is_empty() {
            return Self::filled(0, 0, 0.0);
        }
        let mut row_heights = Vec::new();
        let mut columns = None;
        let mut rows = 0usize;
        for row in &blocks {
            let height = row.first().ok_or("Empty matrix row")?.rows();
            let mut width = 0usize;
            for block in row {
                if block.rows() != height {
                    return Err("Horizontal concatenation requires matching row counts".into());
                }
                width = width
                    .checked_add(block.cols())
                    .ok_or("Dimension overflow")?;
            }
            if columns.is_some_and(|c| c != width) {
                return Err("Matrix rows must have matching widths".into());
            }
            columns = Some(width);
            row_heights.push(height);
            rows = rows.checked_add(height).ok_or("Dimension overflow")?;
        }
        let cols = columns.unwrap_or(0);
        Self::checked_len(rows, cols)?;
        let coordinates = coordinate_proof::CoordinateProof::concatenate(&blocks);
        let mut out = DMatrix::zeros(rows, cols);
        let mut r = 0;
        for (blocks, height) in blocks.into_iter().zip(row_heights) {
            let mut c = 0;
            for block in blocks {
                out.view_mut((r, c), (block.rows(), block.cols()))
                    .copy_from(&block.view());
                c += block.cols();
            }
            r += height;
        }
        let mut result = Self::from_owned(out);
        result.coordinates = coordinates;
        Ok(result)
    }
    pub fn preview(&self, limit: usize) -> String {
        let mut s = String::new();
        use std::fmt::Write;
        if self.is_empty() {
            return format!("[] ({}x{})", self.rows(), self.cols());
        }
        if self.rows() == 1 && self.cols() == 1 {
            return format_preview_cell(self.get(0, 0));
        }
        for r in 0..self.rows().min(limit) {
            for c in 0..self.cols().min(limit) {
                let _ = write!(s, "{:>12} ", format_preview_cell(self.get(r, c)));
            }
            if self.cols() > limit {
                s.push_str("...");
            }
            s.push('\n');
        }
        if self.rows() > limit {
            s.push_str("...\n");
        }
        s.trim_end().to_owned()
    }
}
fn format_preview_cell(v: f64) -> String {
    if v.is_finite() && v == v.trunc() && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v:.6}")
    }
}
impl fmt::Display for Matrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.preview(8))
    }
}

#[cfg(test)]
mod indexed_write_tests {
    use super::*;

    #[test]
    fn overlapping_regions_roll_back_on_mid_write_and_final_cancellation() {
        for fail_at in [1, 5, 6, 7, 8, 9] {
            let original: Vec<_> = (0..32768).map(|i| i as f64).collect();
            let mut matrix = Matrix::from_column_vec(32768, 1, original.clone()).unwrap();
            let pointer = matrix.data().as_ptr();
            // Four backup chunks, four write chunks, then the final check.
            let mut checks = 0;
            let result = matrix.write_unique_regions(&[0..16384, 8192..24576], &[7.], || {
                checks += 1;
                if checks == fail_at {
                    Err("cancelled test".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert_eq!(matrix.data().as_ptr(), pointer);
            assert_eq!(matrix.data(), original);
        }
        let mut matrix = Matrix::filled(32768, 1, 0.).unwrap();
        let alias = matrix.clone();
        assert!(
            !matrix
                .write_unique_regions(std::slice::from_ref(&(0..16384)), &[7.], || panic!(
                    "shared storage must not be touched"
                ))
                .unwrap()
        );
        assert_eq!(matrix.data().as_ptr(), alias.data().as_ptr());
    }

    #[test]
    fn preview_formats_whole_doubles_without_trailing_zeros_and_keeps_fractions() {
        let whole = Matrix::scalar(724.);
        let frac = Matrix::scalar(724.5);
        let row = Matrix::from_column_vec(1, 2, vec![724., 1.5]).unwrap();
        let s = crate::StructArray::from_slots(
            1,
            1,
            vec!["n".into(), "f".into(), "row".into()],
            vec![
                std::sync::Arc::new(whole.into()),
                std::sync::Arc::new(frac.into()),
                std::sync::Arc::new(row.into()),
            ],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(s.get(0, 0).unwrap().preview(32), "724");
        assert_eq!(s.get(1, 0).unwrap().preview(32), "724.500000");
        assert_eq!(
            s.get(2, 0).unwrap().preview(32),
            "         724     1.500000"
        );
        assert_eq!(Matrix::scalar(-0.).preview(32), "0");
    }
}
