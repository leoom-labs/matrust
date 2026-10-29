//! Real vector correlation: direct sums for small work, zero-padded FFTs for
//! long inputs. Independent direct lags share the caller's Rayon pool.
use super::{Engine, Value};
use crate::{Array, ComplexMatrix, Matrix, Result};
use rayon::prelude::*;

const MAX_WORK: usize = 256_000_000;
// Release measurements favor later admission for cross-correlation's extra
// forward FFT. Short lag windows remain cheaper directly (research/xcorr-fft).
const FFT_WORK: usize = 256_000;
const PAR_WORK: usize = 1_000_000;
const CHUNK: usize = 4096;

#[derive(Clone, Copy, PartialEq)]
enum Scale {
    None,
    Biased,
    Unbiased,
    Coeff,
}

impl Engine {
    pub(super) fn xcorr_call(&self, args: &[Value], outputs: usize) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if !(1..=2).contains(&outputs) {
            return Err("xcorr supports at most two outputs [r, lags]".into());
        }
        Self::arity_check("xcorr", args, &[1, 2, 3, 4])?;
        let (args, scale) = match args.last().unwrap() {
            Value::Text(s) | Value::QuotedString(s) => (&args[..args.len() - 1], parse_scale(s)?),
            Value::Container(Array::Character(s)) => (
                &args[..args.len() - 1],
                parse_scale(&s.to_utf8(&self.cancel)?)?,
            ),
            _ => (args, Scale::None),
        };
        Self::arity_check("xcorr", args, &[1, 2, 3])?;
        let x = self.xcorr_vector(&args[0])?;
        let (y, maxlag, auto) = match args.len() {
            1 => (x.clone(), None, true),
            2 => {
                let second = self.xcorr_vector(&args[1])?;
                if second.len() == 1 {
                    (x.clone(), Some(&args[1]), true)
                } else {
                    (second, None, false)
                }
            }
            _ => (self.xcorr_vector(&args[1])?, Some(&args[2]), false),
        };
        if x.len() != y.len() && scale != Scale::None {
            return Err(
                "xcorr: scale options other than 'none' require equal input lengths".into(),
            );
        }
        let n = x.len().max(y.len());
        let maxlag = match maxlag {
            None => n - 1,
            Some(value) => {
                let m = self.real_arg("xcorr", value)?;
                if m.len() != 1
                    || !m.data()[0].is_finite()
                    || m.data()[0] < 0.
                    || m.data()[0].fract() != 0.
                    || m.data()[0] > (crate::matrix::MAX_ELEMENTS / 2 - 1) as f64
                {
                    return Err("xcorr: maxlag must be a bounded nonnegative integer scalar".into());
                }
                m.data()[0] as usize
            }
        };
        let len = maxlag
            .checked_mul(2)
            .and_then(|v| v.checked_add(1))
            .ok_or("xcorr: output size overflow")?;
        Matrix::checked_len(1, len)?;
        self.preflight_elements(len.saturating_mul(outputs))?;
        let work = n.saturating_mul(len.min(2 * n - 1));
        let nfft = (x.len() + y.len() - 1).next_power_of_two();
        // Narrow lag windows remain cheaper directly even for very long inputs.
        let minimum_work = FFT_WORK * if auto { 1 } else { 2 };
        let fft_work = (if auto { 16 } else { 24 }) * nfft * nfft.ilog2() as usize;
        let mut data = if (work >= minimum_work && work >= fft_work || work > MAX_WORK)
            && self.xcorr_finite(&x)?
            && self.xcorr_finite(&y)?
        {
            match self.xcorr_fft(&x, &y, maxlag, auto, outputs) {
                Ok(data) => data,
                // Within the direct cap, a budget-limited FFT falls back to the direct sum.
                Err(_) if work <= MAX_WORK => self.xcorr_direct(&x, &y, maxlag)?,
                Err(error) => return Err(error),
            }
        } else {
            if work > MAX_WORK {
                return Err("xcorr: direct correlation exceeds the work limit; use finite inputs and a supported FFT length, or a smaller maxlag".into());
            }
            self.xcorr_direct(&x, &y, maxlag)?
        };
        let denominator = if scale == Scale::Coeff && auto {
            // Use the very same computed zero-lag value: finite nonzero r(0) is
            // exactly one, including when FFT roundoff perturbs the raw energy.
            data[maxlag]
        } else if scale == Scale::Coeff {
            let energy = |data: &[f64]| -> Result<f64> {
                let mut sum = 0.;
                for chunk in data.chunks(CHUNK) {
                    self.check_cancel()?;
                    sum += chunk.iter().map(|v| v * v).sum::<f64>();
                }
                Ok(sum)
            };
            // MATLAB's sqrt(cxx0*cyy0): an overflowing product gives 0 at every lag.
            (energy(x.data())? * energy(y.data())?).sqrt()
        } else {
            1.
        };
        for (block, chunk) in data.chunks_mut(CHUNK).enumerate() {
            self.check_cancel()?;
            for (offset, sum) in chunk.iter_mut().enumerate() {
                let lag = (block * CHUNK + offset).abs_diff(maxlag);
                *sum = match scale {
                    Scale::None => *sum,
                    Scale::Biased => *sum / n as f64,
                    Scale::Unbiased if lag >= n => 0.,
                    Scale::Unbiased => *sum / (n - lag) as f64,
                    // A zero-energy input deliberately produces NaN at every lag.
                    Scale::Coeff => *sum / denominator,
                };
            }
        }
        let (rows, cols) = if x.rows() == 1 { (1, len) } else { (len, 1) };
        let mut result = vec![Value::Matrix(Matrix::from_column_vec(rows, cols, data)?)];
        if outputs == 2 {
            let mut lags = Vec::with_capacity(len);
            for start in (0..len).step_by(CHUNK) {
                self.check_cancel()?;
                lags.extend((start..(start + CHUNK).min(len)).map(|i| i as f64 - maxlag as f64));
            }
            result.push(Value::Matrix(Matrix::from_column_vec(1, len, lags)?));
        }
        self.check_cancel()?;
        Ok(result)
    }

    fn xcorr_finite(&self, x: &Matrix) -> Result<bool> {
        for chunk in x.data().chunks(CHUNK) {
            self.check_cancel()?;
            if chunk.iter().any(|v| !v.is_finite()) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn xcorr_direct(&self, x: &Matrix, y: &Matrix, maxlag: usize) -> Result<Vec<f64>> {
        let len = 2 * maxlag + 1;
        let n = x.len().max(y.len());
        let work = n.saturating_mul(len.min(2 * n - 1));
        let at_lag = |index: usize| -> Result<f64> {
            self.check_cancel()?;
            let lag = index as isize - maxlag as isize;
            // sum_j x[j+lag]*y[j]. Zero padding is implicit, never copied.
            let (xi, yi) = if lag >= 0 {
                (lag as usize, 0)
            } else {
                (0, (-lag) as usize)
            };
            let overlap = x.len().saturating_sub(xi).min(y.len().saturating_sub(yi));
            let mut sum = 0.;
            for start in (0..overlap).step_by(CHUNK) {
                self.check_cancel()?;
                let end = (start + CHUNK).min(overlap);
                sum += x.data()[xi + start..xi + end]
                    .iter()
                    .zip(&y.data()[yi + start..yi + end])
                    .map(|(a, b)| a * b)
                    .sum::<f64>();
            }
            Ok(sum)
        };
        let mut data = vec![0.; len];
        if work >= PAR_WORK && crate::parallel::current_worker_count() > 1 {
            data.par_iter_mut()
                .enumerate()
                .try_for_each(|(i, value)| -> Result<()> {
                    *value = at_lag(i)?;
                    Ok(())
                })?;
        } else {
            for (i, value) in data.iter_mut().enumerate() {
                *value = at_lag(i)?;
            }
        }
        Ok(data)
    }

    fn xcorr_fft(
        &self,
        x: &Matrix,
        y: &Matrix,
        maxlag: usize,
        auto: bool,
        outputs: usize,
    ) -> Result<Vec<f64>> {
        // Padding to the linear support prevents circular wraparound. Powers of
        // two use the existing FFT's smaller planning allowance and fast kernels.
        let nfft = (x.len() + y.len() - 1).next_power_of_two();
        let len = 2 * maxlag + 1;
        let retained = len * outputs;
        let transform = |name: &str, input: Value, extra: usize| {
            self.fourier_with_retained(
                name,
                &[input, Value::Matrix(Matrix::scalar(nfft as f64))],
                retained + extra,
            )
        };
        let Value::Complex(fx) = transform("fft", Value::Matrix(x.clone()), 0)? else {
            unreachable!("forward FFT is complex")
        };
        let fy = if auto {
            fx.clone()
        } else {
            let Value::Complex(fy) = transform("fft", Value::Matrix(y.clone()), 2 * nfft)? else {
                unreachable!("forward FFT is complex")
            };
            fy
        };
        self.preflight_elements(retained + if auto { 4 * nfft } else { 6 * nfft })?;
        let mut product = Vec::with_capacity(nfft);
        for (a, b) in fx.data().chunks(CHUNK).zip(fy.data().chunks(CHUNK)) {
            self.check_cancel()?;
            product.extend(a.iter().zip(b).map(|(a, b)| *a * b.conj()));
        }
        drop(fx);
        drop(fy);
        let product = ComplexMatrix::from_column_vec(1, nfft, product, &self.cancel)?;
        let inverse = transform("ifft", Value::Complex(product), 2 * nfft)?;
        let mut data = Vec::with_capacity(len);
        for first in (0..len).step_by(CHUNK) {
            self.check_cancel()?;
            for index in first..(first + CHUNK).min(len) {
                let lag = index as isize - maxlag as isize;
                // Exact zeros outside either input's support, including unequal
                // lengths; never expose circular aliases or FFT residuals there.
                let value = if lag >= x.len() as isize || -lag >= y.len() as isize {
                    0.
                } else {
                    let i = if lag < 0 {
                        nfft - (-lag) as usize
                    } else {
                        lag as usize
                    };
                    match &inverse {
                        Value::Matrix(m) => m.data()[i],
                        Value::Complex(m) => m.data()[i].re,
                        _ => unreachable!("inverse FFT is numeric"),
                    }
                };
                data.push(value);
            }
        }
        Ok(data)
    }

    fn xcorr_vector(&self, value: &Value) -> Result<Matrix> {
        let m = self.real_arg("xcorr", value)?;
        if m.rows() > 1 && m.cols() > 1 {
            return Err("xcorr: matrix input is not supported yet; use a real vector".into());
        }
        if m.is_empty() {
            return Err("xcorr: input vectors must be nonempty".into());
        }
        Ok(m)
    }
}

fn parse_scale(s: &str) -> Result<Scale> {
    match s.to_ascii_lowercase().as_str() {
        "none" => Ok(Scale::None),
        "biased" => Ok(Scale::Biased),
        "unbiased" => Ok(Scale::Unbiased),
        "coeff" => Ok(Scale::Coeff),
        _ => Err("xcorr: scale must be 'none', 'biased', 'unbiased', or 'coeff'".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Reproduce kernel crossover measurements without exposing implementation
    /// controls in the public xcorr API. Includes FFT planning and allocations.
    #[test]
    #[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_crossover() {
        use std::{hint::black_box, time::Instant};
        println!("kernel,workers,size,maxlag,repetitions,median_us");
        for workers in [1, 4] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    let e = Engine::default();
                    for n in [128, 256, 358, 360, 506, 508, 512, 1024, 4096, 100000] {
                        let x = Matrix::from_column_vec(
                            1,
                            n,
                            (0..n).map(|i| (i as f64 * 0.1).sin()).collect(),
                        )
                        .unwrap();
                        for maxlag in [8, if n <= 4096 { n - 1 } else { 128 }] {
                            for kernel in ["direct", "fft_auto", "fft_cross"] {
                                let run = || {
                                    if kernel == "direct" {
                                        e.xcorr_direct(&x, &x, maxlag).unwrap()
                                    } else {
                                        e.xcorr_fft(&x, &x, maxlag, kernel == "fft_auto", 1)
                                            .unwrap()
                                    }
                                };
                                black_box(run());
                                let mut times = Vec::new();
                                for _ in 0..9 {
                                    let start = Instant::now();
                                    black_box(run());
                                    times.push(start.elapsed().as_secs_f64() * 1e6);
                                }
                                times.sort_by(f64::total_cmp);
                                println!("{kernel},{workers},{n},{maxlag},9,{:.3}", times[4]);
                            }
                        }
                    }
                });
        }
    }

    #[test]
    fn both_kernels_agree_on_identical_inputs_around_crossover() {
        let e = Engine::default();
        for n in [352, 358, 360, 504, 506, 508] {
            for signed in [false, true] {
                let x = Matrix::from_column_vec(
                    1,
                    n,
                    (0..n)
                        .map(|i| (i as f64 * 1.7).sin() + if signed { 0. } else { 2. })
                        .collect(),
                )
                .unwrap();
                let y = Matrix::from_column_vec(
                    n,
                    1,
                    (0..n)
                        .map(|i| (i as f64 * 0.31).cos() + if signed { 0. } else { 2. })
                        .collect(),
                )
                .unwrap();
                for (y, auto) in [(&x, true), (&y, false)] {
                    let direct = e.xcorr_direct(&x, y, n - 1).unwrap();
                    let fft = e.xcorr_fft(&x, y, n - 1, auto, 1).unwrap();
                    let norm = direct.iter().copied().map(f64::abs).fold(0., f64::max);
                    for (&a, &b) in fft.iter().zip(&direct) {
                        let scale = if signed { norm } else { b.abs() };
                        assert!((a - b).abs() <= 1e-12 * scale, "n={n}: {a} vs {b}");
                    }
                }
            }
        }
    }

    #[test]
    fn fft_temporary_budget_is_checked_before_planning() {
        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 10000,
            ..Engine::default()
        };
        let ones = Matrix::filled(1, 1024, 1.).unwrap();
        assert!(
            e.xcorr_fft(&ones, &ones, 1023, true, 1)
                .unwrap_err()
                .contains("budget")
        );
        // The public call stays within the direct cap and falls back to the direct sum.
        let r = e.xcorr_call(&[Value::Matrix(ones)], 1).unwrap();
        let Value::Matrix(r) = &r[0] else {
            panic!("xcorr returns a matrix")
        };
        assert_eq!(r.len(), 2047);
    }

    #[test]
    fn coeff_cross_energy_product_overflow_gives_zero() {
        // Reference behavior: xcorr([1e100 0],[0 1e100],'coeff') displays 0 0 0.
        let e = Engine::default();
        let x = Value::Matrix(Matrix::from_column_vec(1, 2, vec![1e100, 0.]).unwrap());
        let y = Value::Matrix(Matrix::from_column_vec(1, 2, vec![0., 1e100]).unwrap());
        let r = e
            .xcorr_call(&[x, y, Value::Text("coeff".into())], 1)
            .unwrap();
        let Value::Matrix(r) = &r[0] else {
            panic!("xcorr returns a matrix")
        };
        assert!(r.data().iter().all(|&v| v == 0.), "{:?}", r.data());
    }

    #[test]
    fn reserves_both_outputs_before_allocation() {
        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 4,
            ..Engine::default()
        };
        let x = Value::Matrix(Matrix::from_column_vec(1, 2, vec![1., 2.]).unwrap());
        assert!(e.xcorr_call(std::slice::from_ref(&x), 1).is_ok());
        assert!(e.xcorr_call(&[x], 2).unwrap_err().contains("budget"));
    }

    #[test]
    fn observes_cancellation_before_work() {
        let e = Engine::default();
        e.cancel.store(true, Ordering::Relaxed);
        assert!(
            e.xcorr_call(&[Value::Matrix(Matrix::scalar(1.))], 1)
                .is_err()
        );
    }
}
