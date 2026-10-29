//! Bounded floating-double logarithms; existing natural `log` is unchanged.
use super::{Engine, Value};
use crate::{Complex64, ComplexMatrix, Matrix, Result};
use std::sync::atomic::{AtomicBool, Ordering};

// A literal compile-time edge makes the existing source/archive inventory
// preserve the complete license. No runtime allocation or Python dependency.
const _: &str = include_str!("../../notices/CPYTHON-LICENSE.txt");
const _: &str = include_str!("../../notices/FREEBSD-CLOG-LICENSE.txt");

const CHUNK: usize = 4096;
const EVALUATION_UNITS: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Base {
    Two,
    Ten,
}

impl Base {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "log2" => Some(Self::Two),
            "log10" => Some(Self::Ten),
            _ => None,
        }
    }

    fn logarithm(self, x: f64) -> f64 {
        match self {
            Self::Two => libm::log2(x),
            Self::Ten => libm::log10(x),
        }
    }

    fn divisor(self) -> f64 {
        match self {
            Self::Two => std::f64::consts::LN_2,
            Self::Ten => std::f64::consts::LN_10,
        }
    }
}

// Initial tuning measured on the recorded Windows i7-14700 one/eight-worker
// strategy experiment. Other pool sizes stay serial until measured. This uses
// the caller's existing Rayon pool; it never creates or nests a pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Serial,
    Parallel,
}

fn single_mode(value: &Value) -> Mode {
    let len = match value {
        Value::Matrix(input) => input.len(),
        Value::Complex(input) => input.len(),
        _ => 0,
    };
    if len >= 65_536 && rayon::current_num_threads() == 8 {
        Mode::Parallel
    } else {
        Mode::Serial
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Classification,
    Allocation,
    Mapping,
}

fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::checkpoint_probe(cancel, phase);
    let _ = phase;
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn preflight(engine: &Engine, elements: usize, components: usize) -> Result<()> {
    engine.check_cancel()?;
    if elements
        .checked_mul(components)
        .and_then(|units| engine.evaluated_elements.checked_add(units))
        .is_none_or(|units| units > EVALUATION_UNITS)
    {
        return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
    }
    Ok(())
}

fn reserve<T>(len: usize, cancel: &AtomicBool) -> Result<Vec<T>> {
    checkpoint(cancel, Phase::Allocation)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| "Cannot allocate logarithm output")?;
    #[cfg(test)]
    tests::allocation_probe();
    checkpoint(cancel, Phase::Allocation)?;
    Ok(out)
}

fn build<T: Copy + Default + Send>(
    len: usize,
    cancel: &AtomicBool,
    mode: Mode,
    value: impl Fn(usize) -> T + Sync,
) -> Result<Vec<T>> {
    let mut out = reserve(len, cancel)?;
    if matches!(mode, Mode::Parallel) {
        use rayon::prelude::*;
        for first in (0..len).step_by(CHUNK) {
            checkpoint(cancel, Phase::Allocation)?;
            out.resize((first + CHUNK).min(len), T::default());
        }
        out.par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(|(block, chunk)| -> Result<()> {
                checkpoint(cancel, Phase::Mapping)?;
                for (offset, slot) in chunk.iter_mut().enumerate() {
                    *slot = value(block * CHUNK + offset);
                }
                checkpoint(cancel, Phase::Mapping)
            })?;
        checkpoint(cancel, Phase::Mapping)?;
        return Ok(out);
    }
    for first in (0..len).step_by(CHUNK) {
        checkpoint(cancel, Phase::Mapping)?;
        out.extend((first..(first + CHUNK).min(len)).map(&value));
    }
    checkpoint(cancel, Phase::Mapping)?;
    Ok(out)
}

fn has_negative(input: &[f64], cancel: &AtomicBool) -> Result<bool> {
    for chunk in input.chunks(CHUNK) {
        checkpoint(cancel, Phase::Classification)?;
        if chunk.iter().any(|&x| x < 0.) {
            return Ok(true);
        }
    }
    checkpoint(cancel, Phase::Classification)?;
    Ok(false)
}

// Near-unit accumulation adapted from FreeBSD msun s_clog.c and
// math_private.h, commit 0dd5a5603e7a33d976f8e6015620bbc79839c609.
// Complete notices and local adaptation: notices/FREEBSD-CLOG-LICENSE.txt.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let part = sum - a;
    (sum, (a - (sum - part)) + (b - part))
}

// Only called where |a| >= |b| or a == 0, as in the upstream accumulation.
fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    (sum, (a - sum) + b)
}

fn compensated_near_unit(large: f64, small: f64) -> Option<f64> {
    // Conservative specialization: square residuals have lattice spacing at
    // least 2^-1004, above minimum normal. No residual underflow or overflow.
    const MIN_COMPONENT: f64 = f64::from_bits((1023_u64 - 450) << 52);
    if small < MIN_COMPONENT {
        return None;
    }
    let large_square = large * large;
    let small_square = small * small;
    let (mut high, mut low) = fast_two_sum(large_square, small_square);
    if !(0.5..=2.).contains(&high) {
        return None;
    }
    // TwoProductFMA (Ogita-Rump-Oishi Algorithm 3.5) replaces only the
    // upstream Dekker splitting. In this guarded domain these are exact lows.
    let mut large_error = libm::fma(large, large, -large_square);
    let mut small_error = libm::fma(small, small, -small_square);
    high -= 1.; // Exact by Sterbenz in the admitted high-sum range.
    (high, low) = two_sum(high, low);
    (large_error, small_error) = two_sum(large_error, small_error);
    // Original Briggs-Kahan accumulation order; do not reassociate.
    (high, large_error) = two_sum(high, large_error);
    (low, small_error) = two_sum(low, small_error);
    let mut tail = large_error + low;
    (high, tail) = fast_two_sum(high, tail);
    Some(libm::log1p((small_error + tail) + high) / 2.)
}

// Finite strategy adapted from CPython v3.13.0 Modules/cmathmodule.c c_log:
// Copyright (c) 2001-2024 Python Software Foundation. All rights reserved.
// Complete upstream license and local changes: notices/CPYTHON-LICENSE.txt.
// Keep the scaled huge/subnormal branches and factored near-unit correction.
// Python's exceptions and special-value table are deliberately not ported.
fn finite_log_magnitude(x: f64, y: f64) -> f64 {
    let ax = x.abs();
    let ay = y.abs();
    if ax > f64::MAX / 4. || ay > f64::MAX / 4. {
        libm::log(libm::hypot(ax / 2., ay / 2.)) + std::f64::consts::LN_2
    } else if ax < f64::MIN_POSITIVE && ay < f64::MIN_POSITIVE {
        // The caller handles both-zero inputs before reaching this branch.
        libm::log(libm::hypot(libm::scalbn(ax, 53), libm::scalbn(ay, 53)))
            - 53. * std::f64::consts::LN_2
    } else {
        let h = libm::hypot(ax, ay);
        if (0.71..=1.73).contains(&h) {
            let large = ax.max(ay);
            let small = ax.min(ay);
            if let Some(value) = compensated_near_unit(large, small) {
                value
            } else {
                libm::log1p((large - 1.) * (large + 1.) + small * small) / 2.
            }
        } else {
            libm::log(h)
        }
    }
}

fn complex_log(z: Complex64, base: Base) -> Complex64 {
    if z.re.is_nan() || z.im.is_nan() {
        return Complex64::new(f64::NAN, f64::NAN);
    }
    let real = if z.re.is_infinite() || z.im.is_infinite() {
        f64::INFINITY
    } else if z.re == 0. && z.im == 0. {
        f64::NEG_INFINITY
    } else {
        finite_log_magnitude(z.re, z.im) / base.divisor()
    };
    // Direct components preserve signed zero and avoid infinity*0 contamination.
    Complex64::new(real, libm::atan2(z.im, z.re) / base.divisor())
}

fn promoted_real_log(x: f64, base: Base) -> Complex64 {
    if x < 0. {
        Complex64::new(base.logarithm(-x), std::f64::consts::PI / base.divisor())
    } else {
        // Real NaN has +0 imaginary part even in an array promoted by a peer.
        Complex64::new(base.logarithm(x), 0.)
    }
}

fn single(engine: &Engine, base: Base, value: &Value, mode: Mode) -> Result<Value> {
    match value {
        Value::Matrix(input) => {
            let len = Matrix::checked_len(input.rows(), input.cols())?;
            preflight(engine, len, 1)?;
            if has_negative(input.data(), &engine.cancel)? {
                ComplexMatrix::checked_len(input.rows(), input.cols())?;
                preflight(engine, len, 2)?;
                let out = build(len, &engine.cancel, mode, |i| {
                    promoted_real_log(input.data()[i], base)
                })?;
                Ok(Value::Complex(ComplexMatrix::from_column_vec(
                    input.rows(),
                    input.cols(),
                    out,
                    &engine.cancel,
                )?))
            } else {
                let out = build(len, &engine.cancel, mode, |i| {
                    base.logarithm(input.data()[i])
                })?;
                engine.check_cancel()?;
                let result = Matrix::from_column_vec(
                    input.rows(),
                    input.cols(),
                    out,
                )?;
                engine.check_cancel()?;
                Ok(Value::Matrix(result))
            }
        }
        Value::Complex(input) => {
            let len = ComplexMatrix::checked_len(input.rows(), input.cols())?;
            preflight(engine, len, 2)?;
            let out = build(len, &engine.cancel, mode, |i| {
                complex_log(input.data()[i], base)
            })?;
            Ok(Value::Complex(ComplexMatrix::from_column_vec(
                input.rows(),
                input.cols(),
                out,
                &engine.cancel,
            )?))
        }
        _ => Err("log10/log2 require floating double arrays; logical, character, cell, graphics and string inputs are unsupported".into()),
    }
}

fn dissect(engine: &Engine, input: &Matrix, want: [bool; 2], mode: Mode) -> Result<Vec<Value>> {
    let len = Matrix::checked_len(input.rows(), input.cols())?;
    preflight(engine, len, usize::from(want[0]) + usize::from(want[1]))?;
    let mut fraction = if want[0] {
        Some(reserve(len, &engine.cancel)?)
    } else {
        None
    };
    let mut exponent = if want[1] {
        Some(reserve(len, &engine.cancel)?)
    } else {
        None
    };
    #[cfg(test)]
    if matches!(mode, Mode::Parallel) {
        tests::parallel_dissect(input.data(), &mut fraction, &mut exponent, &engine.cancel)?;
    } else {
        serial_dissect(input.data(), &mut fraction, &mut exponent, &engine.cancel)?;
    }
    #[cfg(not(test))]
    {
        let _ = mode;
        serial_dissect(input.data(), &mut fraction, &mut exponent, &engine.cancel)?;
    }
    engine.check_cancel()?;
    let mut outputs = Vec::with_capacity(2);
    for output in [fraction, exponent] {
        outputs.push(match output {
            Some(data) => Value::Matrix(Matrix::from_column_vec(input.rows(), input.cols(), data)?),
            None => Value::Void,
        });
    }
    engine.check_cancel()?;
    Ok(outputs)
}

fn serial_dissect(
    input: &[f64],
    fraction: &mut Option<Vec<f64>>,
    exponent: &mut Option<Vec<f64>>,
    cancel: &AtomicBool,
) -> Result<()> {
    // Omitted slots carry Void, not a full empty-valued or scratch payload.
    if fraction.is_some() || exponent.is_some() {
        for chunk in input.chunks(CHUNK) {
            checkpoint(cancel, Phase::Mapping)?;
            for &x in chunk {
                let (f, e) = libm::frexp(x);
                if let Some(out) = fraction.as_mut() {
                    out.push(f);
                }
                if let Some(out) = exponent.as_mut() {
                    out.push(f64::from(e));
                }
            }
        }
    }
    checkpoint(cancel, Phase::Mapping)
}

impl Engine {
    pub(super) fn logarithm_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        let Some(base) = Base::from_name(name) else {
            return Ok(None);
        };
        self.check_cancel()?;
        let [value] = args else {
            return Err(format!("{name} requires exactly one floating array input"));
        };
        single(self, base, value, single_mode(value)).map(Some)
    }

    pub(super) fn logarithm_dissection_call(
        &self,
        args: &[Value],
        want: [bool; 2],
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let [value] = args else {
            return Err("log2 requires exactly one floating array input".into());
        };
        let Value::Matrix(input) = value else {
            return Err("Two-output log2 requires real double storage; complex storage, including empty or zero-imaginary values, is rejected. Use real(X) explicitly if intended".into());
        };
        dissect(self, input, want, Mode::Serial)
    }
}

#[cfg(test)]
mod tests;
