//! Real swept-frequency cosine. Research draft: threshold and Rust behavior await validation.
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::{
    f64::consts::{LN_2, TAU},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChirpMethod {
    Linear,
    Quadratic,
    Logarithmic,
}
impl ChirpMethod {
    pub fn parse(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "" | "linear" | "lin" | "li" => Ok(Self::Linear),
            "quadratic" | "quad" | "q" => Ok(Self::Quadratic),
            "logarithmic" | "log" | "lo" => Ok(Self::Logarithmic),
            _ => Err("chirp method must be linear, quadratic or logarithmic".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChirpShape {
    #[default]
    Default,
    Convex,
    Concave,
}
impl ChirpShape {
    pub fn parse(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "" => Ok(Self::Default),
            "convex" => Ok(Self::Convex),
            "concave" => Ok(Self::Concave),
            _ => Err("Quadratic chirp shape must be convex or concave".into()),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ChirpOptions {
    pub f0: f64,
    pub t1: f64,
    pub f1: f64,
    pub method: ChirpMethod,
    pub phase_degrees: f64,
    pub shape: ChirpShape,
}
impl Default for ChirpOptions {
    fn default() -> Self {
        Self::for_method(ChirpMethod::Linear)
    }
}
impl ChirpOptions {
    pub fn for_method(method: ChirpMethod) -> Self {
        Self {
            f0: if method == ChirpMethod::Logarithmic {
                1e-6
            } else {
                0.
            },
            t1: 1.,
            f1: 100.,
            method,
            phase_degrees: 0.,
            shape: ChirpShape::Default,
        }
    }
    fn validate(self) -> Result<()> {
        if ![self.f0, self.t1, self.f1, self.phase_degrees]
            .iter()
            .all(|v| v.is_finite())
            || self.t1 <= 0.
        {
            return Err("chirp requires finite scalar frequencies/phase and a finite positive reference time".into());
        }
        if self.method != ChirpMethod::Quadratic && self.shape != ChirpShape::Default {
            return Err("chirp shape is supported only for quadratic sweeps".into());
        }
        if self.method == ChirpMethod::Logarithmic
            && (self.f0 == 0.
                || self.f1 == 0.
                || self.f0.is_sign_negative() != self.f1.is_sign_negative())
        {
            return Err(
                "Logarithmic chirp frequencies must be nonzero and have the same sign".into(),
            );
        }
        Ok(())
    }
}

// Mantissa/exponent arithmetic is a rare fallback for intermediate overflow or
// underflow. It is still f64 precision, not arbitrary-precision phase arithmetic.
#[derive(Clone, Copy)]
struct Scaled {
    mantissa: f64,
    exponent: i32,
}
impl Scaled {
    fn new(x: f64) -> Self {
        let (mantissa, exponent) = libm::frexp(x);
        Self { mantissa, exponent }
    }
    fn normalized(mantissa: f64, exponent: i32) -> Self {
        let mut result = Self::new(mantissa);
        if result.mantissa != 0. {
            result.exponent += exponent;
        }
        result
    }
    fn mul(self, b: Self) -> Self {
        Self::normalized(self.mantissa * b.mantissa, self.exponent + b.exponent)
    }
    fn div(self, b: Self) -> Self {
        Self::normalized(self.mantissa / b.mantissa, self.exponent - b.exponent)
    }
    fn value(self) -> f64 {
        libm::scalbn(self.mantissa, self.exponent)
    }
}

// Two-component arithmetic is reserved for range failures and cancellation.
// Scaling keeps products normal; FMA residuals retain terms lost near sweep roots.
#[derive(Clone, Copy)]
struct Compensated {
    hi: f64,
    lo: f64,
    exponent: i32,
}
impl Compensated {
    fn new(value: f64) -> Self {
        let (hi, exponent) = libm::frexp(value);
        Self {
            hi,
            lo: 0.,
            exponent,
        }
    }
    fn normalized(hi: f64, lo: f64, exponent: i32) -> Self {
        let sum = hi + lo;
        let recovered = sum - hi;
        let tail = (hi - (sum - recovered)) + (lo - recovered);
        if sum == 0. {
            let mut result = Self::new(tail);
            result.exponent += exponent;
            return result;
        }
        let (hi, shift) = libm::frexp(sum);
        Self {
            hi,
            lo: libm::scalbn(tail, -shift),
            exponent: exponent + shift,
        }
    }
    fn neg(self) -> Self {
        Self {
            hi: -self.hi,
            lo: -self.lo,
            exponent: self.exponent,
        }
    }
    fn add(self, other: Self) -> Self {
        if self.hi == 0. {
            return other;
        }
        if other.hi == 0. {
            return self;
        }
        let exponent = self.exponent.max(other.exponent);
        let a = libm::scalbn(self.hi, self.exponent - exponent);
        let b = libm::scalbn(other.hi, other.exponent - exponent);
        let sum = a + b;
        let recovered = sum - a;
        let residual = (a - (sum - recovered)) + (b - recovered);
        let tail = residual
            + libm::scalbn(self.lo, self.exponent - exponent)
            + libm::scalbn(other.lo, other.exponent - exponent);
        Self::normalized(sum, tail, exponent)
    }
    fn mul(self, other: Self) -> Self {
        let product = self.hi * other.hi;
        let residual = libm::fma(self.hi, other.hi, -product)
            + self.hi * other.lo
            + self.lo * other.hi
            + self.lo * other.lo;
        Self::normalized(product, residual, self.exponent + other.exponent)
    }
    fn div(self, other: Self) -> Self {
        let quotient = self.hi / other.hi;
        let residual = libm::fma(-quotient, other.hi, self.hi) + self.lo - quotient * other.lo;
        Self::normalized(
            quotient,
            residual / other.hi,
            self.exponent - other.exponent,
        )
    }
    fn fraction(self) -> f64 {
        if !libm::scalbn(self.hi, self.exponent).is_finite() {
            return f64::INFINITY;
        }
        let fraction = |value: f64| {
            let (_, exponent) = libm::frexp(value);
            if self.exponent + exponent >= 53 {
                0.
            } else {
                libm::scalbn(value, self.exponent) % 1.
            }
        };
        (fraction(self.hi) + fraction(self.lo)) % 1.
    }
}

fn cos_cycles(cycles: f64, offset: f64) -> f64 {
    let mut phase = (cycles % 1. + offset) % 1.;
    if phase > 0.5 {
        phase -= 1.;
    } else if phase < -0.5 {
        phase += 1.;
    }
    let phase = phase.abs();
    if phase == 0.25 {
        0.
    } else if phase > 0.25 {
        -libm::cos(TAU * (0.5 - phase))
    } else {
        libm::cos(TAU * phase)
    }
}
// The two-product residual recovers fractional cycles discarded by a rounded
// product. Very large exact dyadic products are integers; no overflow is needed.
fn constant_cycles(frequency: f64, time: f64) -> f64 {
    let (a, ae) = libm::frexp(frequency);
    let (b, be) = libm::frexp(time);
    let exponent = ae + be;
    if exponent >= 106 {
        return 0.;
    }
    let hi = a * b;
    let lo = libm::fma(a, b, -hi);
    (libm::scalbn(hi, exponent) % 1. + libm::scalbn(lo, exponent) % 1.) % 1.
}
fn log_ratio(f0: f64, f1: f64) -> f64 {
    let relative = (f1 - f0) / f0;
    if relative.abs() <= 0.5 {
        libm::log1p(relative)
    } else {
        let ratio = f1 / f0;
        if ratio.is_finite() && ratio > 0. {
            libm::log(ratio)
        } else {
            libm::log(f1.abs()) - libm::log(f0.abs())
        }
    }
}
fn polynomial_cycles(time: f64, options: ChirpOptions, vertex_zero: bool) -> f64 {
    let q = time / options.t1;
    let delta = options.f1 - options.f0;
    let factor = if options.method == ChirpMethod::Linear {
        q * 0.5
    } else if vertex_zero {
        q * q / 3.
    } else {
        q * ((3. - q) / 3.)
    };
    let weighted = (0. ..=1.).contains(&factor);
    let average = if weighted {
        libm::fma(options.f0, 1. - factor, options.f1 * factor)
    } else {
        libm::fma(delta, factor, options.f0)
    };
    let cycles = average * time;
    if q.is_normal()
        && (weighted || delta.is_normal())
        && factor.is_normal()
        && average.is_normal()
        && cycles.is_normal()
        && average.abs() >= options.f0.abs().max(options.f1.abs()) / 1024.
    {
        return cycles;
    }
    let t = Compensated::new(time);
    let q = t.div(Compensated::new(options.t1));
    let factor = if options.method == ChirpMethod::Linear {
        q.mul(Compensated::new(0.5))
    } else if vertex_zero {
        q.mul(q).div(Compensated::new(3.))
    } else {
        q.mul(Compensated::new(3.).add(q.neg()))
            .div(Compensated::new(3.))
    };
    let first_weight = Compensated::new(1.).add(factor.neg());
    let average = Compensated::new(options.f0)
        .mul(first_weight)
        .add(Compensated::new(options.f1).mul(factor));
    t.mul(average).fraction()
}
fn logarithmic_cycles(time: f64, options: ChirpOptions, ratio_log: f64) -> f64 {
    let q = time / options.t1;
    let mut z = q * ratio_log;
    if !q.is_normal() || !z.is_normal() {
        z = Scaled::new(time)
            .mul(Scaled::new(ratio_log))
            .div(Scaled::new(options.t1))
            .value();
    }
    if z == 0. {
        return constant_cycles(options.f0, time);
    }
    if z.is_finite() && z.abs() <= 50. {
        let relative = if z.abs() < 1e-5 {
            1. + z * (0.5 + z * (1. / 6. + z * (1. / 24. + z / 120.)))
        } else {
            libm::expm1(z) / z
        };
        let base = options.f0 * time;
        let cycles = base * relative;
        if base.is_normal() && cycles.is_normal() {
            return cycles;
        }
    }
    let prefactor = Scaled::new(options.f0)
        .mul(Scaled::new(options.t1))
        .div(Scaled::new(ratio_log));
    if z < -50. {
        return prefactor.mul(Scaled::new(-1.)).value();
    }
    if z <= 50. {
        return prefactor.mul(Scaled::new(libm::expm1(z))).value();
    }
    // Above 50, exp(z)-1 and exp(z) differ by < 2e-22 relatively. The exponent
    // guard follows the prefactor's range and leaves margin at the f64 overflow edge.
    if !z.is_finite() || z > (1025. - f64::from(prefactor.exponent)) * LN_2 + 2. {
        return f64::INFINITY;
    }
    let pieces = (z / 512.).ceil() as usize;
    let unit = Scaled::new(libm::exp(z / pieces as f64));
    let mut exponential = Scaled::new(1.);
    for _ in 0..pieces {
        exponential = exponential.mul(unit);
    }
    prefactor.mul(exponential).value()
}

/// Real linear, quadratic or logarithmic chirp; inputs and outputs preserve 2D shape.
/// Frequencies are cycles/time-unit and phase is degrees. No sampling/Nyquist cap is imposed.
/// The ordinary nonconstant path rounds to f64 cycles; a compensated fallback retains
/// two components. Large nonconstant phases can still lose fractional precision.
pub fn chirp(input: &Matrix, options: ChirpOptions, cancel: &AtomicBool) -> Result<Matrix> {
    options.validate()?;
    super::check_cancel(cancel)?;
    if input.len() > crate::matrix::MAX_ELEMENTS {
        return Err("Chirp exceeds matrix element limit".into());
    }
    let offset = (options.phase_degrees % 360.) / 360.;
    let vertex_zero = match options.shape {
        ChirpShape::Default => true,
        ChirpShape::Concave => options.f1 >= options.f0,
        ChirpShape::Convex => options.f1 < options.f0,
    };
    let ratio_log = if options.method == ChirpMethod::Logarithmic && options.f0 != options.f1 {
        log_ratio(options.f0, options.f1)
    } else {
        0.
    };
    let mut output = vec![0.; input.len()];
    let fill = |(index, chunk): (usize, &mut [f64])| -> Result<()> {
        super::check_cancel(cancel)?;
        let start = index * 4096;
        let end = start + chunk.len();
        for (value, &time) in chunk.iter_mut().zip(&input.data()[start..end]) {
            if !time.is_finite() {
                return Err("chirp time samples must be finite".into());
            }
            let cycles = if time == 0. {
                0.
            } else if options.f0 == options.f1 {
                constant_cycles(options.f0, time)
            } else if options.method == ChirpMethod::Logarithmic {
                logarithmic_cycles(time, options, ratio_log)
            } else {
                polynomial_cycles(time, options, vertex_zero)
            };
            if !cycles.is_finite() {
                return Err("chirp integrated phase exceeds finite f64 cycle range".into());
            }
            *value = cos_cycles(cycles, offset);
        }
        Ok(())
    };
    // Candidate only; measure separately from window/waveform thresholds.
    if input.len() >= 65_536 && rayon::current_num_threads() > 1 {
        output.par_chunks_mut(4096).enumerate().try_for_each(fill)?;
    } else {
        for chunk in output.chunks_mut(4096).enumerate() {
            fill(chunk)?;
        }
    }
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(input.rows(), input.cols(), output)
}
