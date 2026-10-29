//! FFTs use RustFFT's CPU-dispatched planner. No global plans or thread pools.
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use rayon::prelude::*;
use rustfft::{Fft, FftDirection, FftPlanner};
use std::sync::atomic::AtomicBool;

pub const MAX_FFT_LENGTH: usize = 1_048_576;
const MAX_BYTES: usize = 256 * 1024 * 1024;
const CHUNK: usize = 4096;
const MAX_JOBS: usize = 8;
// Provisional parallel admission. The release harness includes both sides;
// measurements must precede a speed claim or threshold promotion.
const PAR_ELEMENTS: usize = 262_144;

#[derive(Clone, Copy, Debug, Default)]
pub struct FftOptions {
    pub length: Option<usize>,
    /// One-based dimension; None selects the first non-singleton dimension.
    pub dimension: Option<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformDirection {
    Forward,
    Inverse,
}

/// Validates the shape without allocating or inspecting payload values.
pub fn fourier_shape(input: &Array, options: FftOptions) -> Result<(usize, usize)> {
    input.require_numeric_array()?;
    let dim = options
        .dimension
        .unwrap_or(if input.rows() != 1 { 1 } else { 2 });
    if !(1..=2).contains(&dim) {
        return Err(
            "FFT dimension must be 1 or 2; higher-dimensional transforms are not implemented"
                .into(),
        );
    }
    let n = options
        .length
        .unwrap_or(if dim == 1 { input.rows() } else { input.cols() });
    if n > MAX_FFT_LENGTH {
        return Err("FFT length exceeds the 1048576-point transform limit".into());
    }
    let shape = if dim == 1 {
        (n, input.cols())
    } else {
        (input.rows(), n)
    };
    ComplexMatrix::checked_len(shape.0, shape.1)?;
    Ok(shape)
}

fn zeroed<T: Default + Clone>(len: usize, cancel: &AtomicBool) -> Result<Vec<T>> {
    super::check_cancel(cancel)?;
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| "Cannot allocate FFT output or scratch")?;
    for first in (0..len).step_by(CHUNK) {
        super::check_cancel(cancel)?;
        v.resize((first + CHUNK).min(len), T::default());
    }
    super::check_cancel(cancel)?;
    Ok(v)
}

fn source(input: &Array, index: usize) -> Complex64 {
    match input {
        Array::Numeric(a) => Complex64::new(a.data()[index], 0.),
        Array::Logical(a) => Complex64::new(f64::from(a.data()[index]), 0.),
        Array::Complex(a) => a.data()[index],
        _ => unreachable!("numeric admission precedes payload access"),
    }
}
fn entry(input: &Array, dim: usize, lane: usize, k: usize) -> Complex64 {
    let source_length = if dim == 1 { input.rows() } else { input.cols() };
    if k >= source_length {
        return Complex64::new(0., 0.);
    }
    source(
        input,
        if dim == 1 {
            lane * input.rows() + k
        } else {
            k * input.rows() + lane
        },
    )
}

// Check the effective, padded/truncated transform, never the discarded tail.
// A single nonsymmetric vector keeps the entire matrix complex. Exact equality
// (no tolerance) is deliberate: arbitrary imaginary information is retained.
fn real_inverse(
    input: &Array,
    dim: usize,
    n: usize,
    lanes: usize,
    cancel: &AtomicBool,
) -> Result<bool> {
    for lane in 0..lanes {
        super::check_cancel(cancel)?;
        let dc = entry(input, dim, lane, 0);
        if !dc.re.is_finite() || dc.im != 0. {
            return Ok(false);
        }
        for k in 1..=n / 2 {
            if k.is_multiple_of(CHUNK / 2) {
                super::check_cancel(cancel)?;
            }
            let a = entry(input, dim, lane, k);
            let b = entry(input, dim, lane, n - k);
            if !a.re.is_finite() || !a.im.is_finite() || a.re != b.re || a.im != -b.im {
                return Ok(false);
            }
        }
    }
    super::check_cancel(cancel)?;
    Ok(true)
}

struct Lane {
    values: Vec<Complex64>,
    scratch: Vec<Complex64>,
}
fn execute_lane(
    lane: &mut Lane,
    input: &Array,
    dim: usize,
    index: usize,
    fft: &dyn Fft<f64>,
    inverse: bool,
    cancel: &AtomicBool,
) -> Result<()> {
    for (block, values) in lane.values.chunks_mut(CHUNK).enumerate() {
        super::check_cancel(cancel)?;
        for (j, value) in values.iter_mut().enumerate() {
            *value = entry(input, dim, index, block * CHUNK + j);
        }
    }
    super::check_cancel(cancel)?;
    // Trusted bounded library call: not interruptible internally.
    fft.process_with_scratch(&mut lane.values, &mut lane.scratch);
    super::check_cancel(cancel)?;
    if inverse {
        let denominator = lane.values.len() as f64;
        for chunk in lane.values.chunks_mut(CHUNK) {
            super::check_cancel(cancel)?;
            for z in chunk {
                *z /= denominator;
            }
        }
    }
    Ok(())
}

/// Allocates a fresh result, sharing only the read-only input. `available_bytes`
/// is the caller's remaining statement budget, capped at 256 MiB. Accounting
/// includes output, actual scratch, lane buffers and a conservative planning
/// allowance; it is NOT an allocator/RSS hard quota. RustFFT's planner itself
/// uses infallible allocations, is finite under MAX_FFT_LENGTH and is checked
/// for cancellation immediately before/after. See the pinned resource audit.
pub fn fourier(
    input: &Array,
    options: FftOptions,
    direction: TransformDirection,
    available_bytes: usize,
    cancel: &AtomicBool,
) -> Result<Array> {
    super::check_cancel(cancel)?;
    let (rows, cols) = fourier_shape(input, options)?;
    let dim = options
        .dimension
        .unwrap_or(if input.rows() != 1 { 1 } else { 2 });
    let n = if dim == 1 { rows } else { cols };
    let lanes = if dim == 1 { cols } else { rows };
    let count = rows
        .checked_mul(cols)
        .ok_or("FFT output dimensions overflow")?;
    let budget = available_bytes.min(MAX_BYTES);
    if count == 0 {
        // No planner (RustFFT's zero-length plan cannot be executed).
        return Ok(Matrix::from_column_vec(rows, cols, Vec::new())?.into());
    }
    if count.checked_mul(8).ok_or("FFT output size overflow")? > budget {
        return Err("FFT output exceeds remaining evaluation budget".into());
    }
    let inverse = direction == TransformDirection::Inverse;
    let real = inverse && real_inverse(input, dim, n, lanes, cancel)?;
    let output_bytes = count
        .checked_mul(if real { 8 } else { 16 })
        .ok_or("FFT output size overflow")?;
    // 128 bytes/point for powers of two, 512 otherwise, plus 1 MiB:
    // a source-informed policy allowance for
    // recipes, cached subplans, twiddle/mapping tables and construction scratch.
    // It does not purport to measure hidden allocator capacity or process peak.
    // The larger general-length allowance covers prime/Bluestein construction
    // buffers. Consequently some lengths below MAX_FFT_LENGTH fail the budget.
    let planning = n
        .checked_mul(if n.is_power_of_two() { 128 } else { 512 })
        .and_then(|b| b.checked_add(1024 * 1024))
        .ok_or("FFT planning budget overflow")?;
    let fixed = output_bytes
        .checked_add(planning)
        .ok_or("FFT budget overflow")?;
    if fixed.checked_add(n * 16).ok_or("FFT budget overflow")? >= budget {
        return Err("FFT exceeds remaining evaluation budget before planning".into());
    }
    super::check_cancel(cancel)?;
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft(
        n,
        if inverse {
            FftDirection::Inverse
        } else {
            FftDirection::Forward
        },
    );
    drop(planner); // Retain only this call's plan; no global/cache retention.
    super::check_cancel(cancel)?;
    let scratch_len = fft.get_inplace_scratch_len();
    let per_job = n
        .checked_add(scratch_len)
        .and_then(|v| v.checked_mul(16))
        .ok_or("FFT scratch budget overflow")?;
    let affordable = (budget - fixed) / per_job;
    if affordable == 0 {
        return Err(
            "FFT output, plan allowance and actual scratch exceed remaining evaluation budget"
                .into(),
        );
    }
    let wanted = if count >= PAR_ELEMENTS && n >= 32 {
        rayon::current_num_threads().min(MAX_JOBS).min(lanes)
    } else {
        1
    };
    let jobs = wanted.min(affordable);
    let mut buffers = Vec::new();
    buffers
        .try_reserve_exact(jobs)
        .map_err(|_| "Cannot allocate FFT lane metadata")?;
    for _ in 0..jobs {
        buffers.push(Lane {
            values: zeroed(n, cancel)?,
            scratch: zeroed(scratch_len, cancel)?,
        });
    }
    let mut complex_output = if real {
        Vec::new()
    } else {
        zeroed(count, cancel)?
    };
    let mut real_output = if real {
        zeroed(count, cancel)?
    } else {
        Vec::new()
    };
    // A bounded batch of independent lanes; no full transposed temporary,
    // nested pool, unsafe strided writes, or per-lane scratch allocation.
    for first in (0..lanes).step_by(jobs) {
        super::check_cancel(cancel)?;
        let active = (lanes - first).min(jobs);
        let work = |(j, buffer): (usize, &mut Lane)| {
            execute_lane(buffer, input, dim, first + j, fft.as_ref(), inverse, cancel)
        };
        if jobs > 1 {
            buffers[..active]
                .par_iter_mut()
                .enumerate()
                .try_for_each(work)?;
        } else {
            buffers[..active]
                .iter_mut()
                .enumerate()
                .try_for_each(work)?;
        }
        for (j, lane) in buffers[..active].iter().enumerate() {
            for (block, values) in lane.values.chunks(CHUNK).enumerate() {
                super::check_cancel(cancel)?;
                for (k, z) in values.iter().enumerate() {
                    let k = block * CHUNK + k;
                    let i = if dim == 1 {
                        (first + j) * rows + k
                    } else {
                        k * rows + first + j
                    };
                    if real {
                        real_output[i] = z.re;
                    } else {
                        complex_output[i] = *z;
                    }
                }
            }
        }
        #[cfg(test)]
        cancellation_probe::after_batch(cancel);
    }
    super::check_cancel(cancel)?;
    if real {
        Ok(Matrix::from_column_vec(rows, cols, real_output)?.into())
    } else {
        Ok(ComplexMatrix::from_column_vec(rows, cols, complex_output, cancel)?.into())
    }
}

#[cfg(test)]
mod cancellation_probe {
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
    };
    thread_local! { static ARMED: Cell<bool> = const { Cell::new(false) }; }
    pub(super) fn after_batch(cancel: &AtomicBool) {
        if ARMED.with(|armed| armed.replace(false)) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    #[test]
    fn completed_batch_cancellation_rejects_private_output_and_retry_is_exact() {
        use crate::{
            Array, Matrix,
            dsp::{FftOptions, TransformDirection, fourier},
        };
        let source: Array =
            Matrix::from_column_vec(32768, 9, (0..32768 * 9).map(|i| (i % 7) as f64).collect())
                .unwrap()
                .into();
        let alias = source.clone();
        let cancel = AtomicBool::new(false);
        for workers in [1, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            let error = pool.install(|| {
                ARMED.with(|a| a.set(true));
                fourier(
                    &source,
                    FftOptions::default(),
                    TransformDirection::Forward,
                    256 * 1024 * 1024,
                    &cancel,
                )
                .unwrap_err()
            });
            assert_eq!(error, "Calculation cancelled");
            assert!(source.same_snapshot(&alias));
            cancel.store(false, Ordering::Relaxed);
            let a = pool
                .install(|| {
                    fourier(
                        &source,
                        FftOptions::default(),
                        TransformDirection::Forward,
                        256 * 1024 * 1024,
                        &cancel,
                    )
                })
                .unwrap();
            // A small number of lanes' scratch, retaining same output and
            // planning allowance. This exercises budget-driven worker reduction
            // in an eight-worker caller pool, with no result change.
            let bounded = 9 * 32768 * 16 + 32768 * 128 + 1024 * 1024 + 32768 * 16 * 4;
            let b = pool
                .install(|| {
                    fourier(
                        &alias,
                        FftOptions::default(),
                        TransformDirection::Forward,
                        bounded,
                        &cancel,
                    )
                })
                .unwrap();
            let (Array::Complex(a), Array::Complex(b)) = (a, b) else {
                panic!("complex output")
            };
            assert!(
                a.data()
                    .iter()
                    .zip(b.data())
                    .all(|(a, b)| a.re.to_bits() == b.re.to_bits()
                        && a.im.to_bits() == b.im.to_bits())
            );
        }
    }
}
