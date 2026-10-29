//! Real column-major multirate primitives. No zero-expanded FIR intermediate.
use crate::{Matrix, Result, matrix::MAX_ELEMENTS};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 4096;
// Provisional scheduling policy; release one/eight-worker measurements required.
const PARALLEL_WORK: usize = 512 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Allocate,
    Validate,
    Compute,
}

fn checkpoint(cancel: &AtomicBool, _phase: Phase) -> Result<()> {
    #[cfg(test)]
    CANCEL_AFTER.with(|hook| {
        if let Some((phase, remaining)) = hook.get()
            && phase == _phase
        {
            if remaining == 0 {
                hook.set(None);
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                hook.set(Some((phase, remaining - 1)));
            }
        }
    });
    super::check_cancel(cancel)
}

#[cfg(test)]
thread_local! { static CANCEL_AFTER: std::cell::Cell<Option<(Phase, usize)>> = const { std::cell::Cell::new(None) }; }

fn factor(n: usize) -> Result<()> {
    if n == 0 || n > MAX_ELEMENTS {
        return Err(format!(
            "Rate factor must be an integer from 1 to {MAX_ELEMENTS}"
        ));
    }
    Ok(())
}

fn zero_output(length: usize, cancel: &AtomicBool) -> Result<Vec<f64>> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| "Unable to allocate rate-conversion output")?;
    while output.len() < length {
        checkpoint(cancel, Phase::Allocate)?;
        output.resize((output.len() + CHUNK).min(length), 0.);
    }
    Ok(output)
}

/// Samples in a row vector are contiguous; other 2D arrays are column signals.
fn signal_shape(x: &Matrix) -> (usize, usize, bool) {
    if x.rows() == 1 {
        (x.cols(), 1, true)
    } else {
        (x.rows(), x.cols(), false)
    }
}

/// Result shape preflight without allocation. Empty axes use the same shape rule.
pub fn rate_output_shape(x: &Matrix, n: usize, phase: usize, up: bool) -> Result<(usize, usize)> {
    factor(n)?;
    if phase >= n {
        return Err("Rate phase must be an integer from 0 to n-1".into());
    }
    let (samples, channels, row) = signal_shape(x);
    let count = if up {
        samples
            .checked_mul(n)
            .ok_or("Upsampled dimension overflow")?
    } else if phase >= samples {
        0
    } else {
        (samples - 1 - phase) / n + 1
    };
    let shape = if row { (1, count) } else { (count, channels) };
    Matrix::checked_len(shape.0, shape.1)?;
    Ok(shape)
}

fn rate(x: &Matrix, n: usize, phase: usize, up: bool, cancel: &AtomicBool) -> Result<Matrix> {
    super::check_cancel(cancel)?;
    let (rows, cols) = rate_output_shape(x, n, phase, up)?;
    if n == 1 {
        return Ok(x.clone());
    }
    let (samples, _, row) = signal_shape(x);
    let count = if row { cols } else { rows };
    let length = Matrix::checked_len(rows, cols)?;
    let mut output = zero_output(length, cancel)?;
    let fill = |(chunk_index, values): (usize, &mut [f64])| -> Result<()> {
        checkpoint(cancel, Phase::Compute)?;
        let base = chunk_index * CHUNK;
        if up {
            // Initialization already provided zeros. Visit only retained sample
            // slots, while splitting chunks that cross a column boundary.
            let mut offset = 0;
            while offset < values.len() {
                let index = base + offset;
                let channel = index / count;
                let start = index % count;
                let end = count.min(start + values.len() - offset);
                let first = if start <= phase {
                    phase
                } else {
                    phase + (start - phase).div_ceil(n) * n
                };
                for sample in (first..end).step_by(n) {
                    values[offset + sample - start] =
                        x.data()[channel * samples + (sample - phase) / n];
                }
                offset += end - start;
            }
        } else {
            for (offset, value) in values.iter_mut().enumerate() {
                let index = base + offset;
                *value = x.data()[(index / count) * samples + phase + (index % count) * n];
            }
        }
        Ok(())
    };
    if length >= PARALLEL_WORK && rayon::current_num_threads() > 1 {
        output
            .par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(fill)?;
    } else {
        for chunk in output.chunks_mut(CHUNK).enumerate() {
            fill(chunk)?;
        }
    }
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(rows, cols, output)
}

/// Insert n-1 positive zeros per sample; phase shifts original samples within each group.
/// Numeric payload bits (including NaN/Inf/signed zero) are copied unchanged.
pub fn upsample(x: &Matrix, n: usize, phase: usize, cancel: &AtomicBool) -> Result<Matrix> {
    rate(x, n, phase, true, cancel)
}

/// Retain phase, phase+n, ... independently in each signal; no antialiasing filter.
pub fn downsample(x: &Matrix, n: usize, phase: usize, cancel: &AtomicBool) -> Result<Matrix> {
    rate(x, n, phase, false, cancel)
}

#[derive(Clone, Copy)]
struct Plan {
    samples: usize,
    taps: usize,
    x_channels: usize,
    h_channels: usize,
    count: usize,
    rows: usize,
    cols: usize,
    length: usize,
    work: usize,
}

fn plan(x: &Matrix, h: &Matrix, p: usize, q: usize) -> Result<Plan> {
    factor(p)?;
    factor(q)?;
    if x.is_empty() || h.is_empty() {
        return Err("upfirdn requires nonempty signal and filter arrays".into());
    }
    let (samples, x_channels, x_row) = signal_shape(x);
    let (taps, h_channels, _) = signal_shape(h);
    if x_channels > 1 && h_channels > 1 && x_channels != h_channels {
        return Err("upfirdn signal/filter matrices must have the same number of columns".into());
    }
    let channels = x_channels.max(h_channels);
    let expanded = (samples - 1)
        .checked_mul(p)
        .and_then(|v| v.checked_add(taps))
        .ok_or("upfirdn dimension overflow")?;
    let count = (expanded - 1) / q + 1;
    let (rows, cols) = if x_row && h_channels == 1 {
        (1, count)
    } else {
        (count, channels)
    };
    let length = Matrix::checked_len(rows, cols)?;
    let terms = samples.min((taps - 1) / p + 1);
    let work = length
        .checked_mul(terms)
        .filter(|&n| n <= super::MAX_WORK)
        .ok_or("upfirdn exceeds the bounded FIR work budget")?;
    Ok(Plan {
        samples,
        taps,
        x_channels,
        h_channels,
        count,
        rows,
        cols,
        length,
        work,
    })
}

pub fn upfirdn_output_shape(x: &Matrix, h: &Matrix, p: usize, q: usize) -> Result<(usize, usize)> {
    let plan = plan(x, h, p, q)?;
    Ok((plan.rows, plan.cols))
}

/// Polyphase FIR output y[m] = sum_i x[i] h[m*q-i*p], zero outside support.
/// Supports vector/vector, matrix/vector, vector/matrix and equal-column matrices.
/// Only finite real f64 data is admitted; overflowing intermediate arithmetic errors.
pub fn upfirdn(x: &Matrix, h: &Matrix, p: usize, q: usize, cancel: &AtomicBool) -> Result<Matrix> {
    super::check_cancel(cancel)?;
    let plan = plan(x, h, p, q)?;
    for data in [x.data(), h.data()] {
        for chunk in data.chunks(CHUNK) {
            checkpoint(cancel, Phase::Validate)?;
            if chunk.iter().any(|v| !v.is_finite()) {
                return Err("upfirdn requires finite signal/filter values".into());
            }
        }
    }
    let mut output = zero_output(plan.length, cancel)?;
    let fill = |(chunk_index, values): (usize, &mut [f64])| -> Result<()> {
        checkpoint(cancel, Phase::Compute)?;
        let mut since_check = 0;
        for (offset, value) in values.iter_mut().enumerate() {
            let index = chunk_index * CHUNK + offset;
            let channel = index / plan.count;
            let t = (index % plan.count) * q;
            let first = if t >= plan.taps {
                (t - plan.taps) / p + 1
            } else {
                0
            };
            let last = (t / p).min(plan.samples - 1);
            let x_base = if plan.x_channels == 1 {
                0
            } else {
                channel * plan.samples
            };
            let h_base = if plan.h_channels == 1 {
                0
            } else {
                channel * plan.taps
            };
            let mut sum = 0.0f64;
            let mut correction = 0.0f64;
            if first <= last {
                for i in first..=last {
                    let term = x.data()[x_base + i] * h.data()[h_base + t - i * p];
                    let next = sum + term;
                    if !term.is_finite() || !next.is_finite() {
                        return Err(
                            "upfirdn intermediate arithmetic overflow; rescale the signal/filter"
                                .into(),
                        );
                    }
                    // Neumaier compensated summation; no per-output scratch/allocation.
                    correction += if sum.abs() >= term.abs() {
                        (sum - next) + term
                    } else {
                        (term - next) + sum
                    };
                    sum = next;
                    since_check += 1;
                    if since_check == CHUNK {
                        checkpoint(cancel, Phase::Compute)?;
                        since_check = 0;
                    }
                }
            }
            *value = sum + correction;
            if !value.is_finite() {
                return Err(
                    "upfirdn compensated result overflow; rescale the signal/filter".into(),
                );
            }
        }
        Ok(())
    };
    if plan.work >= PARALLEL_WORK && plan.length >= CHUNK && rayon::current_num_threads() > 1 {
        output
            .par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(fill)?;
    } else {
        for chunk in output.chunks_mut(CHUNK).enumerate() {
            fill(chunk)?;
        }
    }
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(plan.rows, plan.cols, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancellation_during_copy_and_polyphase_work_discards_local_results() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            let x = Matrix::from_column_vec(16384, 1, vec![1.; 16384]).unwrap();
            let h = Matrix::from_column_vec(17, 1, vec![0.125; 17]).unwrap();
            for fir in [false, true] {
                let cancel = AtomicBool::new(false);
                let run = || {
                    if fir {
                        upfirdn(&x, &h, 3, 2, &cancel)
                    } else {
                        upsample(&x, 3, 1, &cancel)
                    }
                };
                let expected = run().unwrap();
                CANCEL_AFTER.with(|hook| hook.set(Some((Phase::Compute, 2))));
                assert!(run().unwrap_err().contains("cancel"));
                assert!(cancel.load(Ordering::Relaxed));
                CANCEL_AFTER.with(|hook| assert!(hook.get().is_none()));
                cancel.store(false, Ordering::Relaxed);
                let retry = run().unwrap();
                assert_eq!(
                    retry.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected
                        .data()
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>()
                );
                assert!(x.data().iter().all(|&v| v == 1.));
            }
        });
    }
}
