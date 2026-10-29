//! Bounded real-f64 medians. Small lanes use the standard selection primitive;
//! large lanes select radix prefixes without a full input copy.
use super::{MAX_ELEMENTS, Matrix};
use crate::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const CHUNK: usize = 8192;
// Proposed only: parent release measurements must validate the scheduling seam.
const PAR_ELEMENTS: usize = 512 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum MedianAxis {
    Default,
    Dimension(usize),
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Copy,
    Select,
    Histogram,
    Lane,
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}
fn progress(phase: Phase, cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    cancellation_probe::visit(phase, cancel);
    #[cfg(not(test))]
    let _ = phase;
    check(cancel)
}

#[derive(Clone, Copy)]
struct Lane<'a> {
    data: &'a [f64],
    start: usize,
    stride: usize,
    len: usize,
}
impl Lane<'_> {
    fn get(self, index: usize) -> f64 {
        self.data[self.start + index * self.stride]
    }
    fn visit(self, cancel: &AtomicBool, mut f: impl FnMut(f64)) -> Result<()> {
        for first in (0..self.len).step_by(CHUNK) {
            check(cancel)?;
            for i in first..(first + CHUNK).min(self.len) {
                f(self.get(i));
            }
        }
        check(cancel)
    }
}

// Monotone key for non-NaN binary64 values, including -Inf, -0, +0 and +Inf.
fn key(value: f64) -> u64 {
    let bits = value.to_bits();
    if bits >> 63 == 0 {
        bits ^ (1 << 63)
    } else {
        !bits
    }
}
fn value(key: u64) -> f64 {
    f64::from_bits(if key >> 63 == 0 {
        !key
    } else {
        key ^ (1 << 63)
    })
}

fn bucket(histogram: &[usize; 256], rank: &mut usize) -> Result<u64> {
    for (index, &count) in histogram.iter().enumerate() {
        if *rank < count {
            return Ok(index as u64);
        }
        *rank -= count;
    }
    Err("Median internal rank invariant failed".into())
}

fn radix_median(lane: Lane<'_>, omit_nan: bool, cancel: &AtomicBool) -> Result<f64> {
    let (mut count, mut invalid) = (0usize, false);
    let (mut lowest, mut highest) = (u64::MAX, 0);
    lane.visit(cancel, |x| {
        if x.is_nan() {
            invalid = true;
        } else {
            count += 1;
            let k = key(x);
            lowest = lowest.min(k);
            highest = highest.max(k);
        }
    })?;
    if (invalid && !omit_nan) || count == 0 {
        return Ok(f64::NAN);
    }
    if lowest == highest {
        return Ok(value(lowest));
    }
    let (mut lower_rank, mut upper_rank) = ((count - 1) / 2, count / 2);
    let (mut lower_prefix, mut upper_prefix, mut mask) = (0u64, 0u64, 0u64);
    // At most eight full scans. Both middle ranks share one histogram until
    // their prefixes differ, then each counts only its own surviving prefix.
    for byte in (0..8).rev() {
        let shift = byte * 8;
        let same = lower_prefix == upper_prefix;
        let mut lower = [0usize; 256];
        let mut upper = [0usize; 256];
        lane.visit(cancel, |x| {
            if !x.is_nan() {
                let k = key(x);
                let index = ((k >> shift) & 255) as usize;
                if k & mask == lower_prefix {
                    lower[index] += 1;
                }
                if !same && k & mask == upper_prefix {
                    upper[index] += 1;
                }
            }
        })?;
        let lower_byte = bucket(&lower, &mut lower_rank)?;
        let upper_byte = bucket(if same { &lower } else { &upper }, &mut upper_rank)?;
        lower_prefix |= lower_byte << shift;
        upper_prefix |= upper_byte << shift;
        mask |= 255u64 << shift;
        progress(Phase::Histogram, cancel)?;
    }
    Ok(if count.is_multiple_of(2) {
        value(lower_prefix).midpoint(value(upper_prefix))
    } else {
        value(upper_prefix)
    })
}

#[derive(Default)]
struct Scratch {
    values: Vec<f64>,
}
impl Scratch {
    fn reduce(&mut self, lane: Lane<'_>, omit_nan: bool, cancel: &AtomicBool) -> Result<f64> {
        check(cancel)?;
        if lane.len > CHUNK {
            return radix_median(lane, omit_nan, cancel);
        }
        self.values.clear();
        if self.values.capacity() < lane.len {
            self.values
                .try_reserve_exact(lane.len)
                .map_err(|_| "Cannot allocate median selection scratch")?;
        }
        let mut invalid = false;
        lane.visit(cancel, |x| {
            if x.is_nan() {
                invalid = true;
            } else {
                self.values.push(x);
            }
        })?;
        progress(Phase::Copy, cancel)?;
        if (invalid && !omit_nan) || self.values.is_empty() {
            return Ok(f64::NAN);
        }
        let n = self.values.len();
        // A proven standard-library primitive; one uncancellable call sees at
        // most 8192 values. The input array is never mutated by selection.
        let (lower, middle, _) = self.values.select_nth_unstable_by(n / 2, f64::total_cmp);
        let answer = if n.is_multiple_of(2) {
            let left = lower.iter().copied().max_by(f64::total_cmp).unwrap();
            left.midpoint(*middle)
        } else {
            *middle
        };
        progress(Phase::Select, cancel)?;
        Ok(answer)
    }
}

impl Matrix {
    fn median_layout(&self, axis: MedianAxis) -> Result<(usize, usize, usize, usize)> {
        Ok(match axis {
            MedianAxis::All => (1, 1, self.len(), 1),
            MedianAxis::Default if self.rows() == 0 && self.cols() == 0 => (1, 1, 0, 1),
            _ => {
                let dim = match axis {
                    MedianAxis::Dimension(d) => d,
                    _ => self.first_nonsingleton_dimension(),
                };
                if dim == 0 || dim > MAX_ELEMENTS {
                    return Err(format!(
                        "median dimension must be a positive integer <= {MAX_ELEMENTS}"
                    ));
                }
                match dim {
                    1 => (1, self.cols(), self.rows(), 1),
                    2 => (self.rows(), 1, self.cols(), self.rows()),
                    _ => (self.rows(), self.cols(), 1, 1),
                }
            }
        })
    }

    pub(crate) fn median_output_len(&self, axis: MedianAxis) -> Result<usize> {
        let (rows, cols, _, _) = self.median_layout(axis)?;
        Self::checked_len(rows, cols)
    }

    /// Unweighted real-f64 2-D median. Missing values are NaNs only. Even pairs
    /// use f64::midpoint; opposite infinities yield NaN. Signed zeros follow
    /// total ordering (-0 before +0), a deterministic MatRust bit convention.
    pub fn median(&self, axis: MedianAxis, omit_nan: bool, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let (rows, cols, length, stride) = self.median_layout(axis)?;
        if length == 1 && (rows, cols) == (self.rows(), self.cols()) {
            return Ok(self.clone());
        }
        let output_len = Self::checked_len(rows, cols)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(output_len)
            .map_err(|_| "Cannot allocate median output")?;
        for first in (0..output_len).step_by(CHUNK) {
            check(cancel)?;
            output.resize((first + CHUNK).min(output_len), 0.);
        }
        let workers = if self.len() >= PAR_ELEMENTS && output_len > 1 {
            rayon::current_num_threads().min(output_len)
        } else {
            1
        };
        let lanes_per_task = output_len.div_ceil(workers).max(1);
        let calculate = |(task, out): (usize, &mut [f64])| -> Result<()> {
            let mut scratch = Scratch::default();
            for (offset, item) in out.iter_mut().enumerate() {
                let index = task * lanes_per_task + offset;
                let start = if stride == 1 { index * length } else { index };
                *item = scratch.reduce(
                    Lane {
                        data: self.data(),
                        start,
                        stride,
                        len: length,
                    },
                    omit_nan,
                    cancel,
                )?;
                progress(Phase::Lane, cancel)?;
            }
            Ok(())
        };
        if workers > 1 {
            output
                .par_chunks_mut(lanes_per_task)
                .enumerate()
                .try_for_each(calculate)?;
        } else {
            output
                .chunks_mut(lanes_per_task)
                .enumerate()
                .try_for_each(calculate)?;
        }
        check(cancel)?;
        Self::from_column_vec(rows, cols, output)
    }
}

#[cfg(test)]
mod cancellation_probe {
    use super::*;
    use std::cell::Cell;
    thread_local! { pub(super) static AT: Cell<Option<Phase>> = const { Cell::new(None) }; }
    pub(super) fn visit(phase: Phase, cancel: &AtomicBool) {
        AT.with(|at| {
            if at.get() == Some(phase) {
                at.set(None);
                cancel.store(true, Ordering::Relaxed);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_median_cancellation_after_copy_selection_and_radix_pass_is_atomic() {
        for (len, phase) in [
            (CHUNK, Phase::Copy),
            (CHUNK, Phase::Select),
            (CHUNK + 1, Phase::Histogram),
        ] {
            let input = Matrix::from_column_vec(
                1,
                len,
                (0..len).map(|i| ((i * 13) % 101) as f64).collect(),
            )
            .unwrap();
            let before = input.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            let pointer = input.data().as_ptr();
            let cancel = AtomicBool::new(false);
            let expected = input.median(MedianAxis::All, false, &cancel).unwrap();
            cancellation_probe::AT.with(|at| at.set(Some(phase)));
            assert!(
                input
                    .median(MedianAxis::All, false, &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert!(cancel.load(Ordering::Relaxed));
            cancellation_probe::AT.with(|at| assert_eq!(at.get(), None));
            assert_eq!(input.data().as_ptr(), pointer);
            assert_eq!(
                input.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                before
            );
            cancel.store(false, Ordering::Relaxed);
            assert_eq!(
                input
                    .median(MedianAxis::All, false, &cancel)
                    .unwrap()
                    .data()[0]
                    .to_bits(),
                expected.data()[0].to_bits()
            );
        }
    }
}
