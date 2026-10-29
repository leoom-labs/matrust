//! Reductions of real, logical and complex 2-D storage: sum/mean, min/max with
//! first indices, any/all and find. Dimension 1 folds each column in storage
//! order; dimension 2 sweeps the columns once and updates one accumulator per
//! row, so no lane is read with a stride. Lanes run on the caller's Rayon pool
//! above PAR_REDUCE input elements. Every lane keeps its serial arithmetic
//! order, so results never depend on the worker count.
use super::{MAX_ELEMENTS, Matrix};
use crate::{ComplexMatrix, LogicalMatrix, Result, complex_matrix::Complex64, logical::ArrayRef};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const CHECK_EVERY: usize = 8192;
// Release sweep with the parallel path forced at 1 vs 8 workers (task notes):
// 128K-element inputs are mixed, 256K+ gain for every reduction. find stays
// serial: its chunked parallel scan was slower at every size up to 16M.
const PAR_REDUCE: usize = 256 * 1024;
// Smallest row block for a parallel dimension-2 sweep (one block per task).
const MIN_ROW_BLOCK: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extreme {
    Min,
    Max,
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

/// Explicit dimension, or `default` (the first nonsingleton dimension).
fn dimension(dim: Option<usize>, default: usize) -> Result<usize> {
    match dim {
        None => Ok(default),
        Some(dim) if (1..=MAX_ELEMENTS).contains(&dim) => Ok(dim),
        Some(_) => Err(format!(
            "Dimension must be a positive integer <= {MAX_ELEMENTS}"
        )),
    }
}

/// `Matrix::first_nonsingleton_dimension` for storage classes without a Matrix.
fn first_nonsingleton(rows: usize, cols: usize) -> usize {
    if rows == 1 && cols != 1 { 2 } else { 1 }
}

trait Fold: Sync {
    type Item: Copy + Send + Sync;
    type Acc: Copy + Send + Sync;
    /// Accumulator of a lane whose first element is `x`.
    fn first(&self, x: Self::Item) -> Self::Acc;
    /// Fold element `index` (zero-based within its lane, index >= 1).
    fn next(&self, acc: &mut Self::Acc, x: Self::Item, index: usize);
}

/// Fold every lane of a rows x cols column-major array along dimension 1 or 2.
/// Lanes must be nonempty: rows > 0 for dimension 1, cols > 0 for dimension 2.
fn fold<F: Fold>(
    f: &F,
    data: &[F::Item],
    (rows, cols): (usize, usize),
    dim: usize,
    cancel: &AtomicBool,
) -> Result<Vec<F::Acc>> {
    check(cancel)?;
    let workers = rayon::current_num_threads();
    let parallel = data.len() >= PAR_REDUCE && workers > 1;
    if dim == 1 {
        let column = |c: usize| -> Result<F::Acc> {
            let lane = &data[c * rows..(c + 1) * rows];
            let mut acc = f.first(lane[0]);
            for (block, chunk) in lane[1..].chunks(CHECK_EVERY).enumerate() {
                check(cancel)?;
                let offset = 1 + block * CHECK_EVERY;
                for (i, &x) in chunk.iter().enumerate() {
                    f.next(&mut acc, x, offset + i);
                }
            }
            Ok(acc)
        };
        let accs = if parallel && cols > 1 {
            (0..cols).into_par_iter().map(column).collect()
        } else {
            (0..cols).map(column).collect()
        };
        check(cancel)?;
        return accs;
    }
    let mut accs = Vec::new();
    accs.try_reserve_exact(rows)
        .map_err(|_| "Cannot allocate reduction output".to_owned())?;
    for chunk in data[..rows].chunks(CHECK_EVERY) {
        check(cancel)?;
        accs.extend(chunk.iter().map(|&x| f.first(x)));
    }
    let block = if parallel {
        rows.div_ceil(4 * workers).clamp(MIN_ROW_BLOCK, CHECK_EVERY)
    } else {
        CHECK_EVERY
    };
    let sweep = |(index, chunk): (usize, &mut [F::Acc])| -> Result<()> {
        let start = index * block;
        let mut pending = 0;
        for c in 1..cols {
            if pending >= CHECK_EVERY {
                check(cancel)?;
                pending = 0;
            }
            let column = &data[c * rows + start..c * rows + start + chunk.len()];
            for (acc, &x) in chunk.iter_mut().zip(column) {
                f.next(acc, x, c);
            }
            pending += chunk.len();
        }
        check(cancel)
    };
    if parallel && rows > block {
        accs.par_chunks_mut(block).enumerate().try_for_each(sweep)?;
    } else {
        accs.chunks_mut(block).enumerate().try_for_each(sweep)?;
    }
    Ok(accs)
}

struct Sum;
impl Fold for Sum {
    type Item = f64;
    type Acc = f64;
    fn first(&self, x: f64) -> f64 {
        x
    }
    fn next(&self, acc: &mut f64, x: f64, _: usize) {
        *acc += x;
    }
}

/// NaN is skipped unless the whole lane is NaN; ties keep the first index.
struct RealExtreme<const MAX: bool>;
impl<const MAX: bool> Fold for RealExtreme<MAX> {
    type Item = f64;
    type Acc = (f64, usize);
    fn first(&self, x: f64) -> (f64, usize) {
        (x, 0)
    }
    fn next(&self, acc: &mut (f64, usize), x: f64, index: usize) {
        let better = if MAX { x > acc.0 } else { x < acc.0 };
        if better || (acc.0.is_nan() && !x.is_nan()) {
            *acc = (x, index);
        }
    }
}

/// NaN magnitude for an element with a NaN component, else abs.
fn magnitude(x: Complex64) -> f64 {
    if x.re.is_nan() || x.im.is_nan() {
        f64::NAN
    } else {
        x.re.hypot(x.im)
    }
}

/// Complex order: abs, then angle in (-pi, pi]; first index wins exact ties.
fn complex_better<const MAX: bool>(x: Complex64, x_abs: f64, y: Complex64, y_abs: f64) -> bool {
    if x_abs != y_abs {
        return if MAX { x_abs > y_abs } else { x_abs < y_abs };
    }
    let (x_angle, y_angle) = (x.im.atan2(x.re), y.im.atan2(y.re));
    if MAX {
        x_angle > y_angle
    } else {
        x_angle < y_angle
    }
}

struct ComplexExtreme<const MAX: bool>;
impl<const MAX: bool> Fold for ComplexExtreme<MAX> {
    type Item = Complex64;
    type Acc = (Complex64, f64, usize);
    fn first(&self, x: Complex64) -> Self::Acc {
        (x, magnitude(x), 0)
    }
    fn next(&self, acc: &mut Self::Acc, x: Complex64, index: usize) {
        let x_abs = magnitude(x);
        if x_abs.is_nan() {
            return;
        }
        if acc.1.is_nan() || complex_better::<MAX>(x, x_abs, acc.0, acc.1) {
            *acc = (x, x_abs, index);
        }
    }
}

trait Truthy: Copy + Send + Sync {
    /// Nonzero test; `any` ignores NaN while `all` counts it as nonzero.
    fn truthy(self, all: bool) -> bool;
}
impl Truthy for f64 {
    fn truthy(self, all: bool) -> bool {
        self != 0. && (all || !self.is_nan())
    }
}
impl Truthy for u8 {
    fn truthy(self, _: bool) -> bool {
        self != 0
    }
}
/// Nonzero when either component is; `any` ignores a NaN component like a NaN real.
impl Truthy for Complex64 {
    fn truthy(self, all: bool) -> bool {
        self.re.truthy(all) || self.im.truthy(all)
    }
}

struct Truth<T, const ALL: bool>(std::marker::PhantomData<T>);
impl<T: Truthy, const ALL: bool> Fold for Truth<T, ALL> {
    type Item = T;
    type Acc = bool;
    fn first(&self, x: T) -> bool {
        x.truthy(ALL)
    }
    fn next(&self, acc: &mut bool, x: T, _: usize) {
        if ALL {
            *acc &= x.truthy(true);
        } else {
            *acc |= x.truthy(false);
        }
    }
}

/// min/max result shape: the reduced dimension becomes 1 unless it is empty.
fn extreme_shape(rows: usize, cols: usize, dim: usize) -> (usize, usize, usize) {
    if dim == 1 {
        (rows.min(1), cols, rows)
    } else {
        (rows, cols.min(1), cols)
    }
}

fn split_indices<T: Copy>(accs: &[(T, usize)]) -> Vec<f64> {
    accs.iter().map(|&(_, i)| (i + 1) as f64).collect()
}

impl Matrix {
    /// sum(A[,dim]) and mean(A[,dim]). The default form maps 0x0 to the scalar
    /// 0 or NaN; dimensions above 2 return the input itself, sharing storage.
    pub fn sum_along(&self, dim: Option<usize>, mean: bool, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let (rows, cols) = (self.rows(), self.cols());
        if dim.is_none() && rows == 0 && cols == 0 {
            return Ok(Self::scalar(if mean { f64::NAN } else { 0. }));
        }
        let dim = dimension(dim, self.first_nonsingleton_dimension())?;
        if dim > 2 {
            return Ok(self.clone());
        }
        let (out_rows, out_cols, n) = if dim == 1 {
            (1, cols, rows)
        } else {
            (rows, 1, cols)
        };
        if n == 0 {
            return Self::filled(out_rows, out_cols, if mean { f64::NAN } else { 0. });
        }
        let mut sums = fold(&Sum, self.data(), (rows, cols), dim, cancel)?;
        if mean {
            sums.iter_mut().for_each(|s| *s /= n as f64);
        }
        Self::from_column_vec(out_rows, out_cols, sums)
    }

    /// [M,I] = min/max(A[,[],dim]): NaN is ignored unless a lane is all NaN,
    /// and I holds the first index of the result. An empty reduced dimension
    /// stays empty; dimensions above 2 return A and ones.
    pub fn extreme_along(
        &self,
        kind: Extreme,
        dim: Option<usize>,
        indices: bool,
        cancel: &AtomicBool,
    ) -> Result<(Self, Option<Self>)> {
        check(cancel)?;
        let (rows, cols) = (self.rows(), self.cols());
        let dim = dimension(dim, self.first_nonsingleton_dimension())?;
        if dim > 2 {
            let ones = indices.then(|| Self::filled(rows, cols, 1.)).transpose()?;
            return Ok((self.clone(), ones));
        }
        let (out_rows, out_cols, n) = extreme_shape(rows, cols, dim);
        if n == 0 || Self::checked_len(out_rows, out_cols)? == 0 {
            let empty = || Self::filled(out_rows, out_cols, 0.);
            return Ok((empty()?, indices.then(empty).transpose()?));
        }
        let accs = match kind {
            Extreme::Min => fold(
                &RealExtreme::<false>,
                self.data(),
                (rows, cols),
                dim,
                cancel,
            )?,
            Extreme::Max => fold(&RealExtreme::<true>, self.data(), (rows, cols), dim, cancel)?,
        };
        let index = if indices {
            Some(Self::from_column_vec(
                out_rows,
                out_cols,
                split_indices(&accs),
            )?)
        } else {
            None
        };
        let values = accs.into_iter().map(|(v, _)| v).collect();
        Ok((Self::from_column_vec(out_rows, out_cols, values)?, index))
    }
}

impl ComplexMatrix {
    /// Complex [M,I] = min/max(A[,[],dim]) ordered by abs, then angle.
    pub fn extreme_along(
        &self,
        kind: Extreme,
        dim: Option<usize>,
        indices: bool,
        cancel: &AtomicBool,
    ) -> Result<(Self, Option<Matrix>)> {
        check(cancel)?;
        let (rows, cols) = (self.rows(), self.cols());
        let dim = dimension(dim, first_nonsingleton(rows, cols))?;
        if dim > 2 {
            let ones = indices
                .then(|| Matrix::filled(rows, cols, 1.))
                .transpose()?;
            return Ok((self.clone(), ones));
        }
        let (out_rows, out_cols, n) = extreme_shape(rows, cols, dim);
        if n == 0 || Self::checked_len(out_rows, out_cols)? == 0 {
            let index = indices
                .then(|| Matrix::filled(out_rows, out_cols, 0.))
                .transpose()?;
            return Ok((
                Self::from_column_vec(out_rows, out_cols, Vec::new(), cancel)?,
                index,
            ));
        }
        let accs = match kind {
            Extreme::Min => fold(
                &ComplexExtreme::<false>,
                self.data(),
                (rows, cols),
                dim,
                cancel,
            )?,
            Extreme::Max => fold(
                &ComplexExtreme::<true>,
                self.data(),
                (rows, cols),
                dim,
                cancel,
            )?,
        };
        let index = if indices {
            let positions: Vec<f64> = accs.iter().map(|&(_, _, i)| (i + 1) as f64).collect();
            Some(Matrix::from_column_vec(out_rows, out_cols, positions)?)
        } else {
            None
        };
        let values = accs.into_iter().map(|(v, _, _)| v).collect();
        Ok((
            Self::from_column_vec(out_rows, out_cols, values, cancel)?,
            index,
        ))
    }

    /// Elementwise complex min/max(A,B) with scalar expansion; NaN elements lose.
    pub fn extreme_pair(&self, other: &Self, kind: Extreme, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let (rows, cols) = if self.len() == 1 {
            (other.rows(), other.cols())
        } else if other.len() == 1 || (self.rows(), self.cols()) == (other.rows(), other.cols()) {
            (self.rows(), self.cols())
        } else {
            return Err("Array dimensions must agree (only scalar expansion is supported)".into());
        };
        let len = Self::checked_len(rows, cols)?;
        let at = |m: &Self, i: usize| m.data()[if m.len() == 1 { 0 } else { i }];
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| "Cannot allocate complex operation output".to_owned())?;
        for first in (0..len).step_by(CHECK_EVERY) {
            check(cancel)?;
            out.extend((first..(first + CHECK_EVERY).min(len)).map(|i| {
                let (a, b) = (at(self, i), at(other, i));
                let (a_abs, b_abs) = (magnitude(a), magnitude(b));
                let pick_b = !b_abs.is_nan()
                    && (a_abs.is_nan()
                        || match kind {
                            Extreme::Min => complex_better::<false>(b, b_abs, a, a_abs),
                            Extreme::Max => complex_better::<true>(b, b_abs, a, a_abs),
                        });
                if pick_b { b } else { a }
            }));
        }
        Self::from_column_vec(rows, cols, out, cancel)
    }
}

/// any/all(A[,dim]) with a logical result. The default form maps 0x0 to a
/// scalar (any false, all true); dimensions above 2 test each element.
pub fn truth_along(
    input: ArrayRef<'_>,
    all: bool,
    dim: Option<usize>,
    cancel: &AtomicBool,
) -> Result<LogicalMatrix> {
    match input {
        ArrayRef::Numeric(m) => truth_lanes(m.data(), (m.rows(), m.cols()), all, dim, cancel),
        ArrayRef::Logical(m) => match dim {
            // Each element is its own lane: logical input is returned as is.
            Some(d) if (3..=MAX_ELEMENTS).contains(&d) => {
                check(cancel)?;
                Ok(m.clone())
            }
            _ => truth_lanes(m.data(), (m.rows(), m.cols()), all, dim, cancel),
        },
    }
}

/// any/all of complex values: an element is nonzero when either component is.
pub fn truth_along_complex(
    input: &ComplexMatrix,
    all: bool,
    dim: Option<usize>,
    cancel: &AtomicBool,
) -> Result<LogicalMatrix> {
    truth_lanes(input.data(), (input.rows(), input.cols()), all, dim, cancel)
}

fn truth_lanes<T: Truthy>(
    data: &[T],
    (rows, cols): (usize, usize),
    all: bool,
    dim: Option<usize>,
    cancel: &AtomicBool,
) -> Result<LogicalMatrix> {
    check(cancel)?;
    if dim.is_none() && rows == 0 && cols == 0 {
        return Ok(LogicalMatrix::scalar(all));
    }
    let dim = dimension(dim, first_nonsingleton(rows, cols))?;
    let (out_rows, out_cols, n) = match dim {
        1 => (1, cols, rows),
        2 => (rows, 1, cols),
        _ => (rows, cols, 1),
    };
    if n == 0 {
        return LogicalMatrix::filled(out_rows, out_cols, all, cancel);
    }
    if dim > 2 {
        let mut truth = Vec::new();
        truth
            .try_reserve_exact(data.len())
            .map_err(|_| "Cannot allocate logical output".to_owned())?;
        for chunk in data.chunks(CHECK_EVERY) {
            check(cancel)?;
            truth.extend(chunk.iter().map(|&x| u8::from(x.truthy(all))));
        }
        return LogicalMatrix::from_canonical_vec(rows, cols, truth);
    }
    let truth = if all {
        fold(
            &Truth::<T, true>(Default::default()),
            data,
            (rows, cols),
            dim,
            cancel,
        )?
    } else {
        fold(
            &Truth::<T, false>(Default::default()),
            data,
            (rows, cols),
            dim,
            cancel,
        )?
    };
    LogicalMatrix::from_canonical_vec(
        out_rows,
        out_cols,
        truth.into_iter().map(u8::from).collect(),
    )
}

fn nonzero_indices<T: Copy + Send + Sync>(
    data: &[T],
    nonzero: impl Fn(T) -> bool + Sync,
    limit: Option<usize>,
    last: bool,
    cancel: &AtomicBool,
) -> Result<Vec<usize>> {
    check(cancel)?;
    let mut found = Vec::new();
    match limit {
        Some(n) if !last => {
            for (block, chunk) in data.chunks(CHECK_EVERY).enumerate() {
                check(cancel)?;
                let offset = block * CHECK_EVERY;
                for (i, &x) in chunk.iter().enumerate() {
                    if found.len() == n {
                        return Ok(found);
                    }
                    if nonzero(x) {
                        found.push(offset + i);
                    }
                }
            }
        }
        Some(n) => {
            for (block, chunk) in data.rchunks(CHECK_EVERY).enumerate() {
                check(cancel)?;
                let offset = data.len() - block * CHECK_EVERY - chunk.len();
                for (i, &x) in chunk.iter().enumerate().rev() {
                    if found.len() == n {
                        break;
                    }
                    if nonzero(x) {
                        found.push(offset + i);
                    }
                }
                if found.len() == n {
                    break;
                }
            }
            found.reverse();
        }
        None => {
            for (block, chunk) in data.chunks(CHECK_EVERY).enumerate() {
                check(cancel)?;
                let offset = block * CHECK_EVERY;
                found.extend(
                    chunk
                        .iter()
                        .enumerate()
                        .filter(|&(_, &x)| nonzero(x))
                        .map(|(i, _)| offset + i),
                );
            }
        }
    }
    check(cancel)?;
    Ok(found)
}

/// Zero-based ascending linear indices of nonzero elements (NaN is nonzero):
/// all of them, or at most `limit` from the start, or from the end if `last`.
pub fn find_nonzero(
    input: ArrayRef<'_>,
    limit: Option<usize>,
    last: bool,
    cancel: &AtomicBool,
) -> Result<Vec<usize>> {
    match input {
        ArrayRef::Numeric(m) => nonzero_indices(m.data(), |x: f64| x != 0., limit, last, cancel),
        ArrayRef::Logical(m) => nonzero_indices(m.data(), |x: u8| x != 0, limit, last, cancel),
    }
}

/// `find_nonzero` for complex input: an element is nonzero when either component is.
pub fn find_nonzero_complex(
    input: &ComplexMatrix,
    limit: Option<usize>,
    last: bool,
    cancel: &AtomicBool,
) -> Result<Vec<usize>> {
    nonzero_indices(
        input.data(),
        |z: Complex64| z.re != 0. || z.im != 0.,
        limit,
        last,
        cancel,
    )
}

/// MATLAB's find output shape: 0x0 for a 0x0 input and for a scalar with
/// no hit, a row for any other row input, otherwise a column.
pub fn find_shape(rows: usize, cols: usize, count: usize) -> (usize, usize) {
    if (rows == 0 && cols == 0) || (rows == 1 && cols == 1 && count == 0) {
        (0, 0)
    } else if rows == 1 {
        (1, count)
    } else {
        (count, 1)
    }
}
