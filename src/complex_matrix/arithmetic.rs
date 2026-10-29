//! Complex arithmetic over immutable column-major operands. Real-only engine
//! calls retain their established kernels; mixed GEMM packs bounded panels.
use super::{Complex64, ComplexMatrix, build, check};
use crate::{Array, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;
mod division;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComplexOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Matmul,
}

const ROW_TILE: usize = 128;
const COL_TILE: usize = 32;
const INNER_TILE: usize = 128;
// Provisional scheduling threshold, not an accepted performance result. Exactly
// one level of parallelism: independent column jobs use serial GEMM kernels.
const PAR_WORK: usize = 8 * 1024 * 1024;
const MAX_JOBS: usize = 64;

fn shape(a: &Array) -> (usize, usize) {
    (a.rows(), a.cols())
}
fn at(a: &Array, index: usize) -> Complex64 {
    match a {
        Array::Complex(a) => a.data()[index],
        Array::Numeric(a) => Complex64::new(a.data()[index], 0.),
        Array::Logical(a) => Complex64::new(f64::from(a.data()[index]), 0.),
        Array::Typed(a) => Complex64::new(a.real(index), 0.),
        Array::Character(_)
        | Array::Cell(_)
        | Array::Graphics(_)
        | Array::Struct(_)
        | Array::Handle(_) => {
            unreachable!("arithmetic_shape rejects nonnumeric input before kernel dispatch")
        }
    }
}
fn jobs(a: &Array, b: &Array) -> usize {
    if a.rows().saturating_mul(a.cols()).saturating_mul(b.cols()) >= PAR_WORK {
        rayon::current_num_threads()
            .min(MAX_JOBS)
            .min(b.cols())
            .max(1)
    } else {
        1
    }
}

impl ComplexMatrix {
    /// Supported element-wise shapes are equal sizes or scalar expansion.
    /// Matmul treats either scalar as element-wise multiplication, as MATLAB does.
    pub fn arithmetic_shape(a: &Array, b: &Array, op: ComplexOp) -> Result<(usize, usize)> {
        a.require_numeric_array()?;
        b.require_numeric_array()?;
        let output = if op == ComplexOp::Matmul && a.len() != 1 && b.len() != 1 {
            if a.cols() != b.rows() {
                return Err("Complex matrix product inner dimensions must agree".into());
            }
            (a.rows(), b.cols())
        } else if shape(a) == shape(b) || b.len() == 1 {
            shape(a)
        } else if a.len() == 1 {
            shape(b)
        } else {
            return Err(
                "Complex array dimensions must agree (only scalar expansion is supported)".into(),
            );
        };
        Self::checked_len(output.0, output.1)?;
        Ok(output)
    }
    /// Application-owned temporary panels; GEMM also owns bounded internal
    /// packing for a <=128x32x128 tile. This is not an allocator/RSS estimate.
    pub fn arithmetic_scratch_bytes(a: &Array, b: &Array, op: ComplexOp) -> usize {
        if op != ComplexOp::Matmul || a.len() == 1 || b.len() == 1 || a.is_empty() || b.is_empty() {
            return 0;
        }
        let count = jobs(a, b);
        let inner = a.cols().min(INNER_TILE);
        let per_job = usize::from(!a.is_complex()) * a.rows().min(ROW_TILE) * inner
            + usize::from(!b.is_complex()) * inner * b.cols().div_ceil(count).min(COL_TILE);
        per_job * 16 * count
    }
    pub fn arithmetic(a: &Array, b: &Array, op: ComplexOp, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let (rows, cols) = Self::arithmetic_shape(a, b, op)?;
        let len = Self::checked_len(rows, cols)?;
        if op == ComplexOp::Matmul && a.len() != 1 && b.len() != 1 {
            return matrix_product(a, b, rows, cols, len, cancel);
        }
        let output = build(len, cancel, |index| {
            let left = at(a, if a.len() == 1 { 0 } else { index });
            let right = at(b, if b.len() == 1 { 0 } else { index });
            match op {
                ComplexOp::Add => left + right,
                ComplexOp::Subtract => left - right,
                ComplexOp::Multiply | ComplexOp::Matmul => left * right,
                ComplexOp::Divide => division::divide(left, right),
            }
        })?;
        Self::from_column_vec(rows, cols, output, cancel)
    }
}

fn scratch(len: usize) -> Result<Vec<Complex64>> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(len)
        .map_err(|_| "Cannot allocate complex GEMM panel")?;
    output.resize(len, Complex64::new(0., 0.));
    Ok(output)
}

fn matrix_product(
    a: &Array,
    b: &Array,
    rows: usize,
    cols: usize,
    len: usize,
    cancel: &AtomicBool,
) -> Result<ComplexMatrix> {
    // Safe initialized output: initialization is cancellable and measured.
    let mut output = build(len, cancel, |_| Complex64::new(0., 0.))?;
    if len == 0 || a.cols() == 0 {
        return ComplexMatrix::from_column_vec(rows, cols, output, cancel);
    }
    let count = jobs(a, b);
    let columns = cols.div_ceil(count);
    let work = |(job, out): (usize, &mut [Complex64])| -> Result<()> {
        check(cancel)?;
        let first_column = job * columns;
        let job_columns = out.len() / rows;
        // No full mixed-input conversion: at most320KiB of owned panels per job.
        // Complex operands are borrowed directly with their logical row stride.
        let mut left = scratch(if a.is_complex() {
            0
        } else {
            rows.min(ROW_TILE) * a.cols().min(INNER_TILE)
        })?;
        let mut right = scratch(if b.is_complex() {
            0
        } else {
            a.cols().min(INNER_TILE) * job_columns.min(COL_TILE)
        })?;
        for column in (0..job_columns).step_by(COL_TILE) {
            let n = (job_columns - column).min(COL_TILE);
            for row in (0..rows).step_by(ROW_TILE) {
                let m = (rows - row).min(ROW_TILE);
                for inner in (0..a.cols()).step_by(INNER_TILE) {
                    check(cancel)?;
                    let k = (a.cols() - inner).min(INNER_TILE);
                    let (left_ptr, left_stride) = match a {
                        Array::Complex(a) => (a.data()[inner * rows + row..].as_ptr(), rows),
                        _ => {
                            for depth in 0..k {
                                for r in 0..m {
                                    left[depth * m + r] = at(a, (inner + depth) * rows + row + r);
                                }
                            }
                            (left.as_ptr(), m)
                        }
                    };
                    let (right_ptr, right_stride) = match b {
                        Array::Complex(b) => (
                            b.data()[(first_column + column) * b.rows() + inner..].as_ptr(),
                            b.rows(),
                        ),
                        _ => {
                            for c in 0..n {
                                for depth in 0..k {
                                    right[c * k + depth] = at(
                                        b,
                                        (first_column + column + c) * b.rows() + inner + depth,
                                    );
                                }
                            }
                            (right.as_ptr(), k)
                        }
                    };
                    check(cancel)?;
                    let dst = out[column * rows + row..].as_mut_ptr();
                    // SAFETY: nonempty bounded m/n/k tiles address initialized
                    // contiguous column-major slices or the checked local panels.
                    // The destination job exclusively owns these columns; inputs
                    // are immutable and cannot alias output. No pointer escapes.
                    // nalgebra::Complex<f64> and gemm::c64 are the same locked
                    // num-complex type; this typed assertion avoids layout casts.
                    let left_ptr: *const gemm::c64 = left_ptr;
                    let right_ptr: *const gemm::c64 = right_ptr;
                    unsafe {
                        gemm::gemm(
                            m,
                            n,
                            k,
                            dst,
                            rows as isize,
                            1,
                            inner != 0,
                            left_ptr,
                            left_stride as isize,
                            1,
                            right_ptr,
                            right_stride as isize,
                            1,
                            Complex64::new(if inner == 0 { 0. } else { 1. }, 0.),
                            Complex64::new(1., 0.),
                            false,
                            false,
                            false,
                            gemm::Parallelism::None,
                        );
                    }
                    #[cfg(test)]
                    gemm_cancel::completed(cancel);
                    check(cancel)?;
                }
            }
        }
        Ok(())
    };
    if count > 1 {
        output
            .par_chunks_mut(rows * columns)
            .enumerate()
            .try_for_each(work)?;
    } else {
        work((0, &mut output))?;
    }
    ComplexMatrix::from_column_vec(rows, cols, output, cancel)
}

#[cfg(test)]
mod gemm_cancel {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};
    thread_local! { static LEFT: Cell<Option<usize>> = const { Cell::new(None) }; static DONE: Cell<usize> = const { Cell::new(0) }; }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            LEFT.set(None);
        }
    }
    pub(super) fn completed(cancel: &AtomicBool) {
        if let Some(left) = LEFT.get() {
            DONE.set(DONE.get() + 1);
            if left == 1 {
                LEFT.set(None);
                cancel.store(true, Ordering::Relaxed);
            } else {
                LEFT.set(Some(left - 1));
            }
        }
    }
    #[test]
    fn cancelled_gemm_tiles_never_publish_partial_output_or_mutate_inputs() {
        let cancel = AtomicBool::new(false);
        let a: Array =
            ComplexMatrix::filled(5, INNER_TILE * 3 + 1, Complex64::new(1., 2.), &cancel)
                .unwrap()
                .into();
        let b: Array = ComplexMatrix::filled(a.cols(), 3, Complex64::new(2., -1.), &cancel)
            .unwrap()
            .into();
        let aliases = (a.clone(), b.clone());
        let expected = ComplexMatrix::arithmetic(&a, &b, ComplexOp::Matmul, &cancel).unwrap();
        DONE.set(0);
        LEFT.set(Some(2));
        let _reset = Reset;
        assert!(ComplexMatrix::arithmetic(&a, &b, ComplexOp::Matmul, &cancel).is_err());
        assert_eq!(DONE.get(), 2);
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(LEFT.get(), None);
        assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
        cancel.store(false, Ordering::Relaxed);
        let retry = ComplexMatrix::arithmetic(&a, &b, ComplexOp::Matmul, &cancel).unwrap();
        assert_eq!(retry.data(), expected.data());
    }
}
