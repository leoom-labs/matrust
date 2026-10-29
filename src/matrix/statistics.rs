//! Real f64 dispersion with scalar normalization and deterministic lane reduction.
//! Shift after power-of-two scaling, then use compensated corrected two-pass sums.
use super::{MAX_ELEMENTS, Matrix};
use crate::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const CHECK_EVERY: usize = 8192;
// Proposed scheduling threshold only; release shape sweeps must validate it.
const PAR_STATISTICS: usize = 512 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum StatisticsAxis {
    Default,
    Dimension(usize),
    All,
}

#[derive(Clone, Copy, Debug)]
pub enum Dispersion {
    Variance,
    StandardDeviation,
}

fn check(cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    cancellation_probe::visit(cancel);
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct Sum {
    value: f64,
    correction: f64,
}
impl Sum {
    fn add(&mut self, x: f64) {
        let adjusted = x - self.correction;
        let next = self.value + adjusted;
        self.correction = (next - self.value) - adjusted;
        self.value = next;
    }
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
        for first in (0..self.len).step_by(CHECK_EVERY) {
            check(cancel)?;
            for i in first..(first + CHECK_EVERY).min(self.len) {
                f(self.get(i));
            }
        }
        check(cancel)
    }
}

fn reduce(
    lane: Lane<'_>,
    population: bool,
    omit_nan: bool,
    kind: Dispersion,
    cancel: &AtomicBool,
) -> Result<f64> {
    let mut count = 0usize;
    let mut maximum = 0.0f64;
    let mut anchor = 0.;
    let mut invalid = false;
    lane.visit(cancel, |x| {
        if x.is_nan() && omit_nan {
            return;
        }
        if count == 0 {
            anchor = x;
        }
        count += 1;
        invalid |= !x.is_finite();
        maximum = maximum.max(x.abs());
    })?;
    if count == 0 || invalid {
        return Ok(f64::NAN);
    }
    if count == 1 || maximum == 0. {
        return Ok(0.);
    }
    // Dividing by this exact power of two happens BEFORE subtraction, so
    // opposite-sign values near MAX never overflow while forming a shift.
    let (_, exponent) = libm::frexp(maximum);
    let exponent = exponent - 1;
    let scale = libm::scalbn(1., exponent);
    let anchor = anchor / scale;
    let mut shifted = Sum::default();
    lane.visit(cancel, |x| {
        if !x.is_nan() {
            shifted.add(x / scale - anchor);
        }
    })?;
    let center = shifted.value / count as f64;
    let mut squares = Sum::default();
    let mut residual = Sum::default();
    lane.visit(cancel, |x| {
        if !x.is_nan() {
            let deviation = (x / scale - anchor) - center;
            squares.add(deviation * deviation);
            residual.add(deviation);
        }
    })?;
    // Correct the small error of the estimated mean, never subtract raw E[x]^2.
    // The nonnegative clamp handles rounding at the mathematical zero boundary.
    let corrected = libm::fma(
        -residual.value / count as f64,
        residual.value,
        squares.value,
    )
    .max(0.);
    let denominator = if population { count } else { count - 1 } as f64;
    let normalized = corrected / denominator;
    Ok(match kind {
        Dispersion::Variance => libm::scalbn(normalized, 2 * exponent),
        // Rescale AFTER sqrt: variance may overflow while std is representable.
        Dispersion::StandardDeviation => libm::scalbn(normalized.sqrt(), exponent),
    })
}

impl Matrix {
    /// Real f64 std/var subset. NaNs are optional missing values; infinities
    /// produce NaN, including singleton lanes. Input storage is never copied.
    pub fn dispersion(
        &self,
        kind: Dispersion,
        population: bool,
        axis: StatisticsAxis,
        omit_nan: bool,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        let (rows, cols, length, stride) = match axis {
            StatisticsAxis::All => (1, 1, self.len(), 1),
            StatisticsAxis::Default if self.rows() == 0 && self.cols() == 0 => (1, 1, 0, 1),
            _ => {
                let dim = match axis {
                    StatisticsAxis::Dimension(dim) => dim,
                    _ => self.first_nonsingleton_dimension(),
                };
                if dim == 0 || dim > MAX_ELEMENTS {
                    return Err(format!(
                        "Statistics dimension must be a positive integer <= {MAX_ELEMENTS}"
                    ));
                }
                match dim {
                    1 => (1, self.cols(), self.rows(), 1),
                    2 => (self.rows(), 1, self.cols(), self.rows()),
                    _ => (self.rows(), self.cols(), 1, 1),
                }
            }
        };
        let output_len = Self::checked_len(rows, cols)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(output_len)
            .map_err(|_| "Cannot allocate statistics output".to_owned())?;
        for first in (0..output_len).step_by(CHECK_EVERY) {
            check(cancel)?;
            output.resize((first + CHECK_EVERY).min(output_len), 0.);
        }
        let calculate = |(index, out): (usize, &mut f64)| -> Result<()> {
            let start = if stride == 1 { index * length } else { index };
            *out = reduce(
                Lane {
                    data: self.data(),
                    start,
                    stride,
                    len: length,
                },
                population,
                omit_nan,
                kind,
                cancel,
            )?;
            Ok(())
        };
        // Independent complete lanes retain identical arithmetic at every worker
        // count. A single vector/all reduction remains serial in this first batch.
        if self.len() >= PAR_STATISTICS && output.len() > 1 && rayon::current_num_threads() > 1 {
            output.par_iter_mut().enumerate().try_for_each(calculate)?;
        } else {
            output.iter_mut().enumerate().try_for_each(calculate)?;
        }
        check(cancel)?;
        Self::from_column_vec(rows, cols, output)
    }
}

#[cfg(test)]
mod cancellation_probe {
    use super::*;
    use std::cell::Cell;
    thread_local! { pub(super) static REMAINING: Cell<Option<usize>> = const { Cell::new(None) }; }
    pub(super) fn visit(cancel: &AtomicBool) {
        REMAINING.with(|slot| {
            if let Some(n) = slot.get() {
                if n == 0 {
                    slot.set(None);
                    cancel.store(true, Ordering::Relaxed);
                } else {
                    slot.set(Some(n - 1));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_cancellation_between_reduction_passes_preserves_input_and_retry() {
        let input = Matrix::from_column_vec(
            1,
            CHECK_EVERY * 3,
            (0..CHECK_EVERY * 3).map(|i| (i % 17) as f64).collect(),
        )
        .unwrap();
        let pointer = input.data().as_ptr();
        let cancel = AtomicBool::new(false);
        let expected = input
            .dispersion(
                Dispersion::Variance,
                false,
                StatisticsAxis::All,
                false,
                &cancel,
            )
            .unwrap();
        // Entry/output-fill, three first-pass chunks and first-pass terminal
        // checks precede the centered-sum pass; no sleeps or extra pool.
        cancellation_probe::REMAINING.with(|slot| slot.set(Some(6)));
        assert!(
            input
                .dispersion(
                    Dispersion::Variance,
                    false,
                    StatisticsAxis::All,
                    false,
                    &cancel
                )
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(cancel.load(Ordering::Relaxed));
        cancellation_probe::REMAINING.with(|slot| assert_eq!(slot.get(), None));
        assert_eq!(input.data().as_ptr(), pointer);
        cancel.store(false, Ordering::Relaxed);
        let retry = input
            .dispersion(
                Dispersion::Variance,
                false,
                StatisticsAxis::All,
                false,
                &cancel,
            )
            .unwrap();
        assert_eq!(retry.data()[0].to_bits(), expected.data()[0].to_bits());
    }
}
