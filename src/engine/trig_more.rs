//! Hyperbolic, reciprocal and degree-based trigonometry, angle-unit conversion
//! and coordinate transforms. Reciprocal functions invert their base function;
//! inverse degree functions convert radians, while direct degree functions
//! reduce the angle before conversion. Out-of-domain real input gives
//! complex principal values.
use super::{Engine, Value};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use std::f64::consts::{FRAC_PI_2, PI};

const CHUNK: usize = 4096;
const EVALUATION_UNITS: usize = 32 * 1024 * 1024;
const TO_DEGREES: f64 = 180. / PI;
const TO_RADIANS: f64 = PI / 180.;

#[derive(Clone, Copy)]
enum Base {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    SinD,
    CosD,
    TanD,
    Identity,
}

#[derive(Clone, Copy, PartialEq)]
enum Post {
    None,
    Reciprocal,
    Degrees,
    Radians,
}

#[derive(Clone, Copy)]
struct Op {
    reciprocal_input: bool,
    base: Base,
    post: Post,
}

fn op(name: &str) -> Option<Op> {
    use Base::*;
    let (reciprocal_input, base, post) = match name {
        "sinh" => (false, Sinh, Post::None),
        "cosh" => (false, Cosh, Post::None),
        "tanh" => (false, Tanh, Post::None),
        "asinh" => (false, Asinh, Post::None),
        "acosh" => (false, Acosh, Post::None),
        "atanh" => (false, Atanh, Post::None),
        "sec" => (false, Cos, Post::Reciprocal),
        "csc" => (false, Sin, Post::Reciprocal),
        "cot" => (false, Tan, Post::Reciprocal),
        "asec" => (true, Acos, Post::None),
        "acsc" => (true, Asin, Post::None),
        "acot" => (true, Atan, Post::None),
        "sech" => (false, Cosh, Post::Reciprocal),
        "csch" => (false, Sinh, Post::Reciprocal),
        "coth" => (false, Tanh, Post::Reciprocal),
        "asech" => (true, Acosh, Post::None),
        "acsch" => (true, Asinh, Post::None),
        "acoth" => (true, Atanh, Post::None),
        "sind" => (false, SinD, Post::None),
        "cosd" => (false, CosD, Post::None),
        "tand" => (false, TanD, Post::None),
        "secd" => (false, CosD, Post::Reciprocal),
        "cscd" => (false, SinD, Post::Reciprocal),
        "cotd" => (false, TanD, Post::Reciprocal),
        "asind" => (false, Asin, Post::Degrees),
        "acosd" => (false, Acos, Post::Degrees),
        "atand" => (false, Atan, Post::Degrees),
        "asecd" => (true, Acos, Post::Degrees),
        "acscd" => (true, Asin, Post::Degrees),
        "acotd" => (true, Atan, Post::Degrees),
        "deg2rad" => (false, Identity, Post::Radians),
        "rad2deg" => (false, Identity, Post::Degrees),
        _ => return None,
    };
    Some(Op {
        reciprocal_input,
        base,
        post,
    })
}

/// Names with more than one output: the coordinate transforms.
pub(super) fn has_multiple_outputs(name: &str) -> bool {
    matches!(name, "cart2pol" | "pol2cart" | "cart2sph" | "sph2cart")
}

/// Exact range reduction in degrees: `x = 360 m + 90 k + r` with `|r| <= 45`
/// (Cody & Waite, *Software Manual for the Elementary Functions*, 1980, ch. 8;
/// Muller, *Elementary Functions*, 3rd ed., 2016, ch. 11). Unlike radians, both
/// steps are exact in binary64: `fmod` is exact, and `r - 90 k` is a multiple of
/// the ulp of `r` no larger than `r`. Only the reduced `r` is converted to radians,
/// so multiples of 90 degrees give exact zeros and ones. Returns `k mod 4` and `r`.
fn reduce_degrees(x: f64) -> (u8, f64) {
    let r = x % 360.;
    let k = (r / 90.).round();
    // k == 0 keeps the sign of a zero argument.
    let r = if k == 0. { r } else { r - 90. * k };
    ((k as i64).rem_euclid(4) as u8, r)
}

fn sind(x: f64) -> f64 {
    quadrant(x, false)
}

fn cosd(x: f64) -> f64 {
    quadrant(x, true)
}

/// `sin(90 k + r)` is `sin r, cos r, -sin r, -cos r` for `k mod 4 = 0..3`, and
/// `cos t = sin(t + 90)` shifts the quadrant by one. Zeros at nonzero multiples
/// of 90 are +0; `sind(-0)` stays -0.
fn quadrant(x: f64, cosine: bool) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (k, r) = reduce_degrees(x);
    let k = if cosine { (k + 1) % 4 } else { k };
    if r == 0. {
        return match k {
            0 if x == 0. => x,
            0 | 2 => 0.,
            1 => 1.,
            _ => -1.,
        };
    }
    let t = r * TO_RADIANS;
    match k {
        0 => libm::sin(t),
        1 => libm::cos(t),
        2 => -libm::sin(t),
        _ => -libm::cos(t),
    }
}

/// `tan(90 k + r)` is `tan r` for even `k` and `-1/tan r` for odd `k`, with the
/// exact values `tan(+-45) = +-1`. At odd multiples of 90 the result is the
/// quotient `sind/cosd` with a +0 cosine: `tand(90)` is Inf, `tand(-90)` -Inf.
fn tand(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (k, r) = reduce_degrees(x);
    if r == 0. {
        return match k {
            0 if x == 0. => x,
            0 | 2 => 0.,
            1 => f64::INFINITY,
            _ => f64::NEG_INFINITY,
        };
    }
    let t = if r.abs() == 45. {
        r.signum()
    } else {
        libm::tan(r * TO_RADIANS)
    };
    if k % 2 == 0 { t } else { -1. / t }
}

/// The reference's values just past the branch points of real `asin`: `asin(2)` is
/// `pi/2 - 1.317i` and `asin(-2)` its negative.
fn asin_promoted(x: f64) -> Complex64 {
    let sign = if x < 0. { -1. } else { 1. };
    Complex64::new(sign * FRAC_PI_2, -sign * x.abs().acosh())
}

/// `acos(x) = pi/2 - asin(x)`: `acos(2)` is `0 + 1.317i`.
fn acos_promoted(x: f64) -> Complex64 {
    let a = asin_promoted(x);
    Complex64::new(FRAC_PI_2 - a.re, -a.im)
}

/// `atanh(x)` for real |x| > 1 is `atanh(1/x) + pi/2 i` for either sign
/// (reference: `atanh(2)` is `0.5493 + 1.5708i`); `acoth(0)` is `0 + pi/2 i`.
fn atanh_promoted(x: f64) -> Complex64 {
    Complex64::new((1. / x).atanh(), FRAC_PI_2)
}

/// `1/z` by Smith's method (R. L. Smith, "Algorithm 116: Complex division",
/// Comm. ACM 5(8), 1962): dividing through by the larger component of `z` keeps
/// `1/(1e200 + 0i)` and `1/cosh(400 + 1i)` from overflowing `|z|^2`. `z` is first
/// divided by a power of two (exact) that brings its larger component into
/// [0.5, 1), so Smith's denominator `re + im * ratio` cannot reach 2 * f64::MAX
/// (`1/(1e308 + 1e308i)` is `5e-309 - 5e-309i`, not 0); the quotient is scaled back.
/// A zero imaginary part divides as real (`1/(0 + 0i)` is `Inf - 0i`); the
/// imaginary sign is `-sign(im)` as for the exact conjugate `conj(z)/|z|^2`.
fn trigx_reciprocal(z: Complex64) -> Complex64 {
    if z.im == 0. {
        return Complex64::new(1. / z.re, -z.im);
    }
    let larger = z.re.abs().max(z.im.abs());
    let exponent = if larger.is_finite() {
        libm::frexp(larger).1
    } else {
        0
    };
    let (re, im) = (libm::scalbn(z.re, -exponent), libm::scalbn(z.im, -exponent));
    let (quotient_re, quotient_im) = if re.abs() >= im.abs() {
        let ratio = im / re;
        let denominator = re + im * ratio;
        (1. / denominator, -ratio / denominator)
    } else {
        let ratio = re / im;
        let denominator = im + re * ratio;
        (ratio / denominator, -1. / denominator)
    };
    Complex64::new(
        libm::scalbn(quotient_re, -exponent),
        libm::scalbn(quotient_im, -exponent),
    )
}

impl Base {
    fn real(self, x: f64) -> f64 {
        match self {
            Self::Sin => x.sin(),
            Self::Cos => x.cos(),
            Self::Tan => x.tan(),
            Self::Asin => x.asin(),
            Self::Acos => x.acos(),
            Self::Atan => x.atan(),
            Self::Sinh => x.sinh(),
            Self::Cosh => x.cosh(),
            Self::Tanh => x.tanh(),
            Self::Asinh => x.asinh(),
            Self::Acosh => x.acosh(),
            Self::Atanh => x.atanh(),
            Self::SinD => sind(x),
            Self::CosD => cosd(x),
            Self::TanD => tand(x),
            Self::Identity => x,
        }
    }

    /// Real input whose result is complex (NaN stays real).
    fn outside(self, x: f64) -> bool {
        match self {
            Self::Asin | Self::Acos | Self::Atanh => x.abs() > 1.,
            Self::Acosh => x < 1.,
            _ => false,
        }
    }

    fn promoted(self, x: f64) -> Complex64 {
        match self {
            Self::Asin => asin_promoted(x),
            Self::Acos => acos_promoted(x),
            Self::Atanh => atanh_promoted(x),
            // Exact branches: platform complex acosh leaves a -2e-16 real part on Windows.
            Self::Acosh if x >= -1. => Complex64::new(0., x.acos()),
            Self::Acosh => Complex64::new((-x).acosh(), std::f64::consts::PI),
            _ => Complex64::new(self.real(x), 0.),
        }
    }

    /// None: the reference's degree functions reject complex input.
    fn complex(self, z: Complex64) -> Option<Complex64> {
        Some(match self {
            Self::Sin => z.sin(),
            Self::Cos => z.cos(),
            Self::Tan => z.tan(),
            Self::Asin => z.asin(),
            Self::Acos => z.acos(),
            Self::Atan => z.atan(),
            Self::Sinh => z.sinh(),
            Self::Cosh => z.cosh(),
            // Beyond 20 the correction to +/-1 is below double precision.
            // Avoid num-complex's overflowing sinh(2*re)/cosh(2*re).
            Self::Tanh if z.re.abs() > 20. && z.im.is_finite() => {
                Complex64::new(1_f64.copysign(z.re), 0.)
            }
            Self::Tanh => z.tanh(),
            // Odd symmetry avoids cancellation in log(z + sqrt(z*z + 1))
            // on the negative-real side of the complex plane.
            Self::Asinh if z.re < 0. => -(-z).asinh(),
            Self::Asinh => z.asinh(),
            Self::Acosh => z.acosh(),
            Self::Atanh => z.atanh(),
            Self::SinD | Self::CosD | Self::TanD => return None,
            Self::Identity => z,
        })
    }
}

impl Op {
    fn input(self, x: f64) -> f64 {
        if self.reciprocal_input { 1. / x } else { x }
    }

    fn post_real(self, y: f64) -> f64 {
        match self.post {
            Post::None => y,
            Post::Reciprocal => 1. / y,
            Post::Degrees => TO_DEGREES * y,
            Post::Radians => TO_RADIANS * y,
        }
    }

    fn post_complex(self, w: Complex64) -> Complex64 {
        match self.post {
            Post::None => w,
            Post::Reciprocal => trigx_reciprocal(w),
            Post::Degrees => w * TO_DEGREES,
            Post::Radians => w * TO_RADIANS,
        }
    }
}

/// MATLAB's class word for an argument these functions reject.
fn rejected_class(value: &Value) -> Option<&'static str> {
    Some(match value {
        Value::Matrix(_) | Value::Complex(_) => return None,
        Value::Container(Array::Numeric(_) | Array::Complex(_)) => return None,
        // single goes through the shared numeric path; integer classes are rejected as in the reference.
        Value::Container(array @ Array::Typed(_)) if array.class() == "single" => return None,
        Value::Container(array @ Array::Typed(_)) => array.class(),
        Value::Text(_) | Value::Container(Array::Character(_)) => "char",
        Value::Logical(_) | Value::Container(Array::Logical(_)) => "logical",
        Value::QuotedString(_) => "string",
        Value::Container(Array::Cell(_)) => "cell",
        Value::Container(Array::Struct(_)) => "struct",
        Value::Container(Array::Handle(_)) => "function_handle",
        Value::Container(Array::Graphics(_)) => "matlab.graphics.Graphics",
        Value::Void => "double",
    })
}

fn undefined(name: &str, class: &str) -> String {
    format!("Undefined function '{name}' for input arguments of type '{class}'.")
}

impl Engine {
    /// One-output calls of every function in this module; `None` for other names.
    pub(super) fn trigx_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if has_multiple_outputs(name) {
            return Ok(Some(self.trigx_outputs(name, args, 1)?.remove(0)));
        }
        let Some(op) = op(name) else {
            return Ok(None);
        };
        self.check_cancel()?;
        let [value] = args else {
            return Err(if args.is_empty() {
                "Not enough input arguments.".into()
            } else {
                "Too many input arguments.".into()
            });
        };
        if let Some(class) = rejected_class(value) {
            return Err(undefined(name, class));
        }
        let value = match value {
            Value::Container(Array::Numeric(m)) => Value::Matrix(m.clone()),
            Value::Container(Array::Complex(z)) => Value::Complex(z.clone()),
            other => other.clone(),
        };
        match &value {
            Value::Matrix(input) => self.trigx_real(op, input).map(Some),
            Value::Complex(input) => self.trigx_complex(op, input).map(Some),
            _ => unreachable!("rejected_class admits only double arrays"),
        }
    }

    fn trigx_budget(&self, units: usize) -> Result<()> {
        if self
            .evaluated_elements
            .checked_add(units)
            .is_none_or(|n| n > EVALUATION_UNITS)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    fn trigx_map<T>(&self, len: usize, value: impl Fn(usize) -> T) -> Result<Vec<T>> {
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| "Cannot allocate trigonometric output")?;
        for first in (0..len).step_by(CHUNK) {
            self.check_cancel()?;
            out.extend((first..(first + CHUNK).min(len)).map(&value));
        }
        self.check_cancel()?;
        Ok(out)
    }

    fn trigx_real(&self, op: Op, input: &Matrix) -> Result<Value> {
        let data = input.data();
        self.trigx_budget(data.len())?;
        let mut outside = false;
        for chunk in data.chunks(CHUNK) {
            self.check_cancel()?;
            if chunk.iter().any(|&x| op.base.outside(op.input(x))) {
                outside = true;
                break;
            }
        }
        if !outside {
            let out = self.trigx_map(data.len(), |i| {
                op.post_real(op.base.real(op.input(data[i])))
            })?;
            return Ok(Value::Matrix(Matrix::from_column_vec(
                input.rows(),
                input.cols(),
                out,
            )?));
        }
        self.trigx_budget(data.len() * 2)?;
        let out = self.trigx_map(data.len(), |i| {
            let x = op.input(data[i]);
            if op.base.outside(x) {
                op.post_complex(op.base.promoted(x))
            } else {
                Complex64::new(op.post_real(op.base.real(x)), 0.)
            }
        })?;
        self.trigx_narrow(ComplexMatrix::from_column_vec(
            input.rows(),
            input.cols(),
            out,
            &self.cancel,
        )?)
    }

    fn trigx_complex(&self, op: Op, input: &ComplexMatrix) -> Result<Value> {
        let data = input.data();
        self.trigx_budget(data.len() * 2)?;
        if op.base.complex(Complex64::new(0., 0.)).is_none() {
            return Err("Argument should be real.".into());
        }
        let out = self.trigx_map(data.len(), |i| {
            let z = if op.reciprocal_input {
                trigx_reciprocal(data[i])
            } else {
                data[i]
            };
            op.post_complex(op.base.complex(z).unwrap_or(z))
        })?;
        self.trigx_narrow(ComplexMatrix::from_column_vec(
            input.rows(),
            input.cols(),
            out,
            &self.cancel,
        )?)
    }

    /// As for arithmetic, a result whose imaginary parts are all zero is real.
    fn trigx_narrow(&self, result: ComplexMatrix) -> Result<Value> {
        if !result.is_empty() && result.data().iter().all(|z| z.im == 0.) {
            return Ok(Value::Matrix(result.real(&self.cancel)?));
        }
        Ok(Value::Complex(result))
    }

    /// cart2pol, pol2cart, cart2sph and sph2cart with `count` outputs.
    pub(super) fn trigx_outputs(
        &self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let (inputs, outputs) = match name {
            "cart2pol" | "pol2cart" => (2..=3, if args.len() == 3 { 3 } else { 2 }),
            _ => (3..=3, 3),
        };
        if args.len() < *inputs.start() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > *inputs.end() {
            return Err("Too many input arguments.".into());
        }
        if count > outputs {
            return Err(if count > 3 {
                "Too many output arguments.".into()
            } else {
                format!(
                    "Output argument \"z\" (and maybe others) not assigned during call to \"{name}\"."
                )
            });
        }
        let mut real = Vec::with_capacity(args.len());
        for value in args {
            real.push(match value {
                Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
                Value::Complex(_) | Value::Container(Array::Complex(_)) => {
                    return Err(format!(
                        "{name} requires real double inputs; complex input is not supported"
                    ));
                }
                other => {
                    return Err(undefined(name, rejected_class(other).unwrap_or("double")));
                }
            });
        }
        // The transformed coordinates; cart2pol/pol2cart pass z through unchanged.
        let used = if outputs == 3 && args.len() == 3 && matches!(name, "cart2pol" | "pol2cart") {
            2
        } else {
            real.len()
        };
        let mut shape = (1, 1);
        for m in &real[..used] {
            if m.len() != 1 {
                if shape != (1, 1) && shape != (m.rows(), m.cols()) {
                    return Err("Matrix dimensions must agree.".into());
                }
                shape = (m.rows(), m.cols());
            }
        }
        let len = Matrix::checked_len(shape.0, shape.1)?;
        self.trigx_budget(len.saturating_mul(count.min(used)))?;
        let at = |k: usize, i: usize| {
            let m = &real[k];
            if m.len() == 1 {
                m.data()[0]
            } else {
                m.data()[i]
            }
        };
        let column =
            |data: Vec<f64>| Matrix::from_column_vec(shape.0, shape.1, data).map(Value::Matrix);
        let mut values = Vec::with_capacity(count);
        for output in 0..count.max(1) {
            if used == 2 && output == 2 {
                values.push(Value::Matrix(real[2].clone()));
                continue;
            }
            let data = match (name, output) {
                ("cart2pol", 0) => self.trigx_map(len, |i| at(1, i).atan2(at(0, i)))?,
                ("cart2pol", _) => self.trigx_map(len, |i| at(0, i).hypot(at(1, i)))?,
                ("pol2cart", 0) => self.trigx_map(len, |i| at(1, i) * at(0, i).cos())?,
                ("pol2cart", _) => self.trigx_map(len, |i| at(1, i) * at(0, i).sin())?,
                ("cart2sph", 0) => self.trigx_map(len, |i| at(1, i).atan2(at(0, i)))?,
                ("cart2sph", 1) => {
                    self.trigx_map(len, |i| at(2, i).atan2(at(0, i).hypot(at(1, i))))?
                }
                ("cart2sph", _) => {
                    self.trigx_map(len, |i| at(0, i).hypot(at(1, i)).hypot(at(2, i)))?
                }
                ("sph2cart", 0) => {
                    self.trigx_map(len, |i| at(2, i) * at(1, i).cos() * at(0, i).cos())?
                }
                ("sph2cart", 1) => {
                    self.trigx_map(len, |i| at(2, i) * at(1, i).cos() * at(0, i).sin())?
                }
                _ => self.trigx_map(len, |i| at(2, i) * at(1, i).sin())?,
            };
            values.push(column(data)?);
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degree_reduction_is_exact_at_quadrants() {
        for k in -8..=8 {
            let x = f64::from(k) * 90.;
            let s = sind(x);
            let c = cosd(x);
            assert!(s == 0. || s.abs() == 1., "sind({x}) = {s}");
            assert!(c == 0. || c.abs() == 1., "cosd({x}) = {c}");
        }
        assert_eq!(tand(90.), f64::INFINITY);
        assert_eq!(tand(-90.), f64::NEG_INFINITY);
        assert_eq!(tand(225.), 1.);
        assert!(sind(f64::INFINITY).is_nan() && tand(f64::NAN).is_nan());
        // rem(1e20, 360) is 280 exactly.
        assert_eq!(sind(1e20), sind(280.));
    }

    #[test]
    fn promoted_branches_follow_reference() {
        let a = asin_promoted(2.);
        assert_eq!(a.re, FRAC_PI_2);
        assert!(a.im < 0.);
        let a = acos_promoted(-2.);
        assert_eq!(a.re, PI);
        assert!(a.im < 0.);
        assert_eq!(acos_promoted(2.).re, 0.);
        let h = atanh_promoted(-2.);
        assert!(h.re < 0. && h.im > 0.);
    }

    #[test]
    fn reduced_degrees_track_radians_and_reciprocal_is_safe() {
        for x in [1., 30., 60., 89., 135.5, -200., 1e10] {
            let t = (x % 360.) * TO_RADIANS;
            assert!(
                (sind(x) - libm::sin(t)).abs() <= 4. * f64::EPSILON,
                "sind({x})"
            );
            assert!(
                (cosd(x) - libm::cos(t)).abs() <= 4. * f64::EPSILON,
                "cosd({x})"
            );
        }
        assert!((tand(60.) - 3_f64.sqrt()).abs() <= 8. * f64::EPSILON);
        assert_eq!(tand(270.), f64::NEG_INFINITY);
        assert_eq!(tand(-270.), f64::INFINITY);
        assert_eq!(cosd(-90.).to_bits(), 0);
        assert_eq!(sind(-0.).to_bits(), (-0_f64).to_bits());
        let r = trigx_reciprocal(Complex64::new(1e200, 0.));
        assert_eq!((r.re, r.im), (1e-200, 0.));
        let r = trigx_reciprocal(Complex64::new(0., 2.));
        assert_eq!((r.re, r.im), (0., -0.5));
        let r = trigx_reciprocal(Complex64::new(3., 4.));
        assert!((r.re - 0.12).abs() <= f64::EPSILON && (r.im + 0.16).abs() <= f64::EPSILON);
        let r = trigx_reciprocal(Complex64::new(1e300, 1e300));
        assert!((r.re - 5e-301).abs() <= 1e-15 * 5e-301 && (r.im + 5e-301).abs() <= 1e-15 * 5e-301);
        assert_eq!(trigx_reciprocal(Complex64::new(0., 0.)).re, f64::INFINITY);
    }

    #[test]
    fn reciprocal_survives_components_near_f64_max() {
        // Smith's denominator re + im * ratio is 2e308 for 1e308 + 1e308i; the quotient
        // 5e-309 * (1 - i) is subnormal-adjacent but nonzero (granularity 4.9e-324).
        for scale in [1e308, f64::MAX, 1e307] {
            let r = trigx_reciprocal(Complex64::new(scale, scale));
            let want = 0.5 / scale;
            assert!(
                (r.re - want).abs() <= 1e-14 * want && (r.im + want).abs() <= 1e-14 * want,
                "1/({scale:e}(1+i)) = {r:?}"
            );
            let r = trigx_reciprocal(Complex64::new(-scale, scale));
            assert!(r.re < 0. && r.im < 0., "{r:?}");
        }
        // Smaller component first, and a tiny z whose reciprocal exceeds f64::MAX.
        let r = trigx_reciprocal(Complex64::new(1e308, 5e307));
        assert!((r.re - 0.8e-308).abs() <= 1e-14 * 0.8e-308, "{r:?}");
        assert!((r.im + 0.4e-308).abs() <= 1e-14 * 0.4e-308, "{r:?}");
        let r = trigx_reciprocal(Complex64::new(1e-320, 1e-320));
        assert_eq!((r.re, r.im), (f64::INFINITY, f64::NEG_INFINITY));
        let r = trigx_reciprocal(Complex64::new(f64::INFINITY, 1.));
        assert_eq!((r.re, r.im), (0., -0.));
        assert!(trigx_reciprocal(Complex64::new(f64::NAN, 1.)).re.is_nan());
        // sech(710.3 + i*pi/4): cosh is about 1.07e308 (1 + i), so 1/cosh must not flush to 0.
        let sech = op("sech").unwrap();
        let z = Complex64::new(710.3, PI / 4.);
        let r = sech.post_complex(sech.base.complex(z).unwrap());
        let want = 4.69e-309;
        assert!(
            (r.re - want).abs() <= 0.01 * want && (r.im + want).abs() <= 0.01 * want,
            "sech({z:?}) = {r:?}"
        );
    }
}
