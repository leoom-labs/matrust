//! Complex results of everyday math (log2 and log10 live in `logarithms.rs`): real input whose MATLAB result is complex
//! (`sqrt(-4)`, `log(-1)`, `(-8)^(1/3)`, `acos(2)`), complex input for the
//! elementary functions, and complex scalar and matrix powers.
//!
//! The formulas avoid the overflow of the textbook ones: `ln|z|` and the
//! square root scale before squaring, `tan` switches to its limit for large
//! imaginary parts, and `acos`/`asin` use Kahan's square-root products instead
//! of `1 - z^2`. Results whose imaginary parts are all zero are real, as in
//! MATLAB (`cos([1i 0])` is real).
use super::{Engine, Execution, Value};
use crate::{Complex64 as C, ComplexMatrix, Matrix, Result};
use nalgebra::DMatrix;

const PI: f64 = std::f64::consts::PI;
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;
/// Integer powers up to this magnitude use repeated squaring (exact for
/// Gaussian integers, as `(1+2i)^2` is in MATLAB).
const SQUARING_LIMIT: f64 = 1024.;

pub(super) const NAMES: &[&str] = &[
    "sqrt", "log", "exp", "sin", "cos", "tan", "acos", "asin", "atan", "realsqrt", "reallog",
    "realpow",
];

/// A numeric operand: real (double, logical, char codes) or complex.
enum Operand {
    Real(Matrix),
    Complex(ComplexMatrix),
}
impl Operand {
    fn len(&self) -> usize {
        match self {
            Self::Real(m) => m.len(),
            Self::Complex(z) => z.len(),
        }
    }
    fn shape(&self) -> (usize, usize) {
        match self {
            Self::Real(m) => (m.rows(), m.cols()),
            Self::Complex(z) => (z.rows(), z.cols()),
        }
    }
    fn get(&self, k: usize) -> C {
        match self {
            Self::Real(m) => C::new(m.data()[k], 0.),
            Self::Complex(z) => z.data()[k],
        }
    }
    fn is_complex(&self) -> bool {
        matches!(self, Self::Complex(_))
    }
}

fn zero() -> C {
    C::new(0., 0.)
}

/// ln|z| without overflow or underflow in |z|.
fn ln_abs(z: C) -> f64 {
    let (a, b) = (z.re.abs(), z.im.abs());
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let (big, small) = if a >= b { (a, b) } else { (b, a) };
    if big == 0. {
        return f64::NEG_INFINITY;
    }
    if big.is_infinite() {
        return f64::INFINITY;
    }
    let r = small / big;
    big.ln() + 0.5 * (r * r).ln_1p()
}

fn clog(z: C) -> C {
    C::new(ln_abs(z), z.im.atan2(z.re))
}

/// Principal square root, scaling before both the hypot and the positive sum.
fn csqrt(z: C) -> C {
    let (x, y) = (z.re, z.im);
    if x == 0. && y == 0. {
        return C::new(0., y);
    }
    if y.is_infinite() {
        return C::new(f64::INFINITY, y);
    }
    if x.is_nan() || y.is_nan() {
        return C::new(f64::NAN, f64::NAN);
    }
    if x.is_infinite() {
        return if x > 0. {
            C::new(x, 0_f64.copysign(y))
        } else {
            C::new(0., f64::INFINITY.copysign(y))
        };
    }
    let scale = x.abs().max(y.abs());
    let t = scale.sqrt() * ((x.abs() / scale + (x / scale).hypot(y / scale)) / 2.).sqrt();
    if x >= 0. {
        C::new(t, y / (2. * t))
    } else {
        C::new(y.abs() / (2. * t), t.copysign(y))
    }
}

fn cexp(z: C) -> C {
    let e = z.re.exp();
    if z.im == 0. {
        return C::new(e, z.im);
    }
    let (s, c) = sin_cos(z.im);
    C::new(e * c, e * s)
}

/// `a * b`, keeping a zero factor exact when the other one overflows.
fn product(a: f64, b: f64) -> f64 {
    if a == 0. { a } else { a * b }
}

fn csin(z: C) -> C {
    let (s, c) = sin_cos(z.re);
    C::new(product(s, libm::cosh(z.im)), product(c, libm::sinh(z.im)))
}

fn ccos(z: C) -> C {
    let (s, c) = sin_cos(z.re);
    C::new(product(c, libm::cosh(z.im)), -product(s, libm::sinh(z.im)))
}

/// Portable libm sine and cosine: the platform `f64::sin_cos` differs between
/// systems (Windows gave cos(pi/2) = 6.123031769111886e-17 instead of
/// 6.123233995736766e-17), which broke results near tan poles.
fn sin_cos(x: f64) -> (f64, f64) {
    (libm::sin(x), libm::cos(x))
}

/// tan(a+bi) = (sin 2a + i sinh 2b) / (cos 2a + cosh 2b); for |b| > 20 the
/// hyperbolic terms overflow long before the result does, so use the limit.
fn ctan(z: C) -> C {
    let (a, b) = (z.re, z.im);
    let (s2, c2) = sin_cos(2. * a);
    if b.abs() > 20. {
        let e = (-2. * b.abs()).exp();
        return C::new(2. * s2 * e / (1. + 2. * c2 * e), 1_f64.copysign(b));
    }
    // cos(2a) + cosh(2b) = 2*(cos(a)^2 + sinh(b)^2).
    // The sum of squares preserves the small denominator near a pole.
    let (s, c) = sin_cos(a);
    let sh = libm::sinh(b);
    let d = c * c + sh * sh;
    C::new(s * c / d, sh * libm::cosh(b) / d)
}

/// Kahan: acos z = 2 atan2(Re sqrt(1-z), Re sqrt(1+z)) + i asinh Im(conj(sqrt(1+z)) sqrt(1-z)).
fn cacos(z: C) -> C {
    let one = C::new(1., 0.);
    let (s1, s2) = (csqrt(one - z), csqrt(one + z));
    C::new(2. * s1.re.atan2(s2.re), (s2.conj() * s1).im.asinh())
}

/// Kahan: asin z = atan2(Re z, Re(sqrt(1-z) sqrt(1+z))) + i asinh Im(conj(sqrt(1-z)) sqrt(1+z)).
fn casin(z: C) -> C {
    let one = C::new(1., 0.);
    let (s1, s2) = (csqrt(one - z), csqrt(one + z));
    C::new(z.re.atan2((s1 * s2).re), (s1.conj() * s2).im.asinh())
}

/// atan z = (i/2) (ln(1 - iz) - ln(1 + iz)).
fn catan(z: C) -> C {
    let one = C::new(1., 0.);
    let iz = C::new(-z.im, z.re);
    let d = clog(one - iz) - clog(one + iz);
    C::new(-d.im / 2., d.re / 2.)
}

/// Smith's scaled complex division.
fn cdiv(a: C, b: C) -> C {
    if b.im == 0. {
        return C::new(a.re / b.re, a.im / b.re);
    }
    // Smith's denominator (and numerator) can still overflow. Halving all
    // components preserves the quotient and leaves room for the sums.
    let largest = b.re.abs().max(b.im.abs());
    if largest.is_finite() && largest > f64::MAX / 2. {
        return cdiv(a * 0.5, b * 0.5);
    }
    if b.re.abs() >= b.im.abs() {
        let r = b.im / b.re;
        let d = b.re + b.im * r;
        C::new((a.re + a.im * r) / d, (a.im - a.re * r) / d)
    } else {
        let r = b.re / b.im;
        let d = b.re * r + b.im;
        C::new((a.re * r + a.im) / d, (a.im * r - a.re) / d)
    }
}

/// (sin(pi p), cos(pi p)) with exact values at the half and whole turns.
fn sin_cos_pi(p: f64) -> (f64, f64) {
    let r = p.rem_euclid(2.);
    match r {
        0. => (0., 1.),
        0.5 => (1., 0.),
        1. => (0., -1.),
        1.5 => (-1., 0.),
        _ => sin_cos(PI * r),
    }
}

/// x^p for real x and p; complex only for a negative x with a finite
/// non-integer p, where it is |x|^p (cos(pi p) + i sin(pi p)).
fn real_power(x: f64, p: f64) -> C {
    if x < 0. && p.is_finite() && p.fract() != 0. {
        let m = (-x).powf(p);
        let (s, c) = sin_cos_pi(p);
        C::new(m * c, m * s)
    } else {
        C::new(x.powf(p), 0.)
    }
}

fn integer_power(z: C, n: f64) -> C {
    let mut base = z;
    let mut result = C::new(1., 0.);
    let mut k = n.abs() as u64;
    while k > 0 {
        if k & 1 == 1 {
            result *= base;
        }
        k >>= 1;
        if k > 0 {
            base = base * base;
        }
    }
    if n < 0. {
        cdiv(C::new(1., 0.), result)
    } else {
        result
    }
}

/// Principal z^w.
fn cpow(z: C, w: C) -> C {
    if w.im == 0. {
        if z.im == 0. {
            return real_power(z.re, w.re);
        }
        if w.re.fract() == 0. && w.re.abs() <= SQUARING_LIMIT {
            return integer_power(z, w.re);
        }
    }
    if z.re == 0. && z.im == 0. {
        return if w.re > 0. {
            zero()
        } else if w.re < 0. {
            C::new(f64::INFINITY, 0.)
        } else {
            C::new(f64::NAN, f64::NAN)
        };
    }
    cexp(w * clog(z))
}

/// Real elementwise results that need a complex value for this element.
fn real_complex_value(name: &str, x: f64) -> Option<C> {
    let domain = |x: f64| x.abs() > 1.;
    Some(match name {
        "sqrt" if x < 0. => C::new(0., (-x).sqrt()),
        "log" if x < 0. => C::new((-x).ln(), PI),
        "acos" if domain(x) && x > 0. => C::new(0., x.acosh()),
        "acos" if domain(x) => C::new(PI, -(-x).acosh()),
        "asin" if domain(x) && x > 0. => C::new(FRAC_PI_2, -x.acosh()),
        "asin" if domain(x) => C::new(-FRAC_PI_2, (-x).acosh()),
        _ => return None,
    })
}

fn real_value(name: &str, x: f64) -> f64 {
    match name {
        "sqrt" => x.sqrt(),
        "log" => x.ln(),
        "acos" => x.acos(),
        "asin" => x.asin(),
        _ => x.atan(),
    }
}

fn complex_value(name: &str, z: C) -> C {
    match name {
        "sqrt" => csqrt(z),
        "log" => clog(z),
        "exp" => cexp(z),
        "sin" => csin(z),
        "cos" => ccos(z),
        "tan" => ctan(z),
        "acos" => cacos(z),
        "asin" => casin(z),
        _ => catan(z),
    }
}

impl Engine {
    /// Real view of double, logical and char values, or the complex array;
    /// None for other values so their usual errors remain.
    fn cmath_operand(&self, value: &Value) -> Result<Option<Operand>> {
        Ok(Some(match value {
            Value::Complex(z) => Operand::Complex(z.clone()),
            Value::Matrix(m) => Operand::Real(m.clone()),
            Value::Logical(l) => Operand::Real(l.to_numeric(&self.cancel)?),
            v if v.is_character() => Operand::Real(v.clone().matrix(&self.cancel)?),
            _ => return Ok(None),
        }))
    }

    /// A complex result, real when every imaginary part is zero.
    fn cmath_value(&self, rows: usize, cols: usize, data: Vec<C>) -> Result<Value> {
        if data.iter().all(|z| z.im == 0.) {
            let re = data.iter().map(|z| z.re).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, re)?));
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            rows,
            cols,
            data,
            &self.cancel,
        )?))
    }

    fn cmath_map(&self, a: &Operand, f: impl Fn(C) -> C) -> Result<Value> {
        let (rows, cols) = a.shape();
        self.preflight_elements(a.len().saturating_mul(2))?;
        let mut data = Vec::new();
        data.try_reserve_exact(a.len())
            .map_err(|_| "Cannot allocate complex result")?;
        for first in (0..a.len()).step_by(4096) {
            self.check_cancel()?;
            data.extend((first..a.len().min(first + 4096)).map(|k| f(a.get(k))));
        }
        self.cmath_value(rows, cols, data)
    }

    /// Elementwise functions whose result may be complex. None leaves real
    /// input with a real result to the existing real kernels.
    pub(super) fn complex_math_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if name == "diag"
            && let Some(Value::Complex(z)) = args.first()
        {
            Self::arity_check(name, args, &[1, 2])?;
            let offset = if args.len() == 2 {
                args[1].clone().matrix(&self.cancel)?.as_scalar()?
            } else {
                0.
            };
            if !offset.is_finite()
                || offset.fract() != 0.
                || offset.abs() > crate::matrix::MAX_ELEMENTS as f64
            {
                return Err("Diagonal offset must be a bounded signed integer".into());
            }
            let k = offset.abs() as usize;
            let (r0, c0) = if offset < 0. { (k, 0) } else { (0, k) };
            let vector = z.rows() == 1 || z.cols() == 1;
            let (rows, cols) = if vector {
                let n = z.len().checked_add(k).ok_or("Diagonal size overflow")?;
                (n, n)
            } else {
                (
                    z.rows().saturating_sub(r0).min(z.cols().saturating_sub(c0)),
                    1,
                )
            };
            let len = ComplexMatrix::checked_len(rows, cols)?;
            self.preflight_elements(len.saturating_mul(2))?;
            let mut data = vec![C::new(0., 0.); len];
            for i in 0..if vector { z.len() } else { rows } {
                if i % 4096 == 0 {
                    self.check_cancel()?;
                }
                if vector {
                    data[r0 + i + (c0 + i) * rows] = z.data()[i];
                } else {
                    data[i] = z.data()[r0 + i + (c0 + i) * z.rows()];
                }
            }
            return Ok(Some(self.cmath_value(rows, cols, data)?));
        }
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        if name == "realpow" {
            Self::arity_check(name, args, &[2])?;
            let a = self.real_arg(name, &args[0])?;
            let b = self.real_arg(name, &args[1])?;
            if a.len() == 1 || b.len() == 1 || (a.rows(), a.cols()) == (b.rows(), b.cols()) {
                let count = if a.len() == 1 { b.len() } else { a.len() };
                let complex = (0..count).any(|k| {
                    let x = a.data()[if a.len() == 1 { 0 } else { k }];
                    let p = b.data()[if b.len() == 1 { 0 } else { k }];
                    real_power(x, p).im != 0.
                });
                if complex {
                    return Err("Realpow produced complex result.".into());
                }
            }
            return Ok(Some(Value::Matrix(a.zip(&b, f64::powf)?)));
        }
        if args.len() != 1 {
            return Ok(None);
        }
        let Some(a) = self.cmath_operand(&args[0])? else {
            return Ok(None);
        };
        if let Some(real) = name.strip_prefix("real") {
            let Operand::Real(m) = &a else {
                return Err(format!("{name}: input must be real"));
            };
            if m.data().iter().any(|&x| x < 0.) {
                return Err(format!("Real{real} produced complex result."));
            }
            return Ok(Some(Value::Matrix(m.map(if real == "sqrt" {
                f64::sqrt
            } else {
                f64::ln
            }))));
        }
        match &a {
            Operand::Complex(_) => Ok(Some(self.cmath_map(&a, |z| complex_value(name, z))?)),
            Operand::Real(m) => {
                let complex = m
                    .data()
                    .iter()
                    .any(|&x| real_complex_value(name, x).is_some());
                if complex {
                    return Ok(Some(self.cmath_map(&a, |z| {
                        real_complex_value(name, z.re)
                            .unwrap_or_else(|| C::new(real_value(name, z.re), 0.))
                    })?));
                }
                if matches!(name, "acos" | "asin" | "atan") {
                    return Ok(Some(Value::Matrix(m.map(|x| real_value(name, x)))));
                }
                Ok(None)
            }
        }
    }

    /// `^` and `.^` when a complex value is involved or the result is complex,
    /// and the matrix powers the real kernel lacks (non-integer exponent,
    /// scalar base with matrix exponent). None leaves the real cases to it.
    pub(super) fn complex_power(
        &mut self,
        matrix: bool,
        a: &Value,
        b: &Value,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        self.check_cancel()?;
        let (Some(x), Some(y)) = (self.cmath_operand(a)?, self.cmath_operand(b)?) else {
            return Ok(None);
        };
        let complex = x.is_complex() || y.is_complex();
        if matrix && (x.len() != 1 || y.len() != 1) {
            return self.cmath_matrix_power(&x, &y, out);
        }
        let (xs, ys) = (x.shape(), y.shape());
        if x.len() != 1 && y.len() != 1 && xs != ys {
            if complex {
                return Err(
                    "Array dimensions must agree (only scalar expansion is supported)".into(),
                );
            }
            return Ok(None);
        }
        let (rows, cols) = if x.len() == 1 { ys } else { xs };
        let count = rows * cols;
        let at = |o: &Operand, k: usize| o.get(if o.len() == 1 { 0 } else { k });
        if !complex && !(0..count).any(|k| real_power(at(&x, k).re, at(&y, k).re).im != 0.) {
            return Ok(None);
        }
        self.preflight_elements(count.saturating_mul(2))?;
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .map_err(|_| "Cannot allocate complex result")?;
        for first in (0..count).step_by(4096) {
            self.check_cancel()?;
            data.extend((first..count.min(first + 4096)).map(|k| cpow(at(&x, k), at(&y, k))));
        }
        Ok(Some(self.cmath_value(rows, cols, data)?))
    }

    fn cmath_dense(&self, o: &Operand) -> DMatrix<C> {
        let (rows, cols) = o.shape();
        DMatrix::from_fn(rows, cols, |i, j| o.get(j * rows + i))
    }

    fn cmath_matrix_value(&self, m: DMatrix<C>, real: bool) -> Result<Value> {
        let (rows, cols) = (m.nrows(), m.ncols());
        if real {
            let re = m.iter().map(|z| z.re).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, re)?));
        }
        self.cmath_value(rows, cols, m.as_slice().to_vec())
    }

    /// A^p (square A, scalar p) and b^A (scalar b, square A).
    fn cmath_matrix_power(
        &mut self,
        x: &Operand,
        y: &Operand,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let complex = x.is_complex() || y.is_complex();
        let square = |o: &Operand| o.shape().0 == o.shape().1;
        let (a, scalar, base) = if x.len() != 1 && y.len() == 1 && square(x) {
            (x, y.get(0), false)
        } else if x.len() == 1 && y.len() != 1 && square(y) {
            (y, x.get(0), true)
        } else if complex {
            return Err("Inputs must be a scalar and a square matrix.".into());
        } else {
            // The real kernel reports its own shape errors.
            return Ok(None);
        };
        let n = a.shape().0;
        if n == 0 {
            return Ok(Some(
                self.cmath_matrix_value(DMatrix::zeros(0, 0), !complex)?,
            ));
        }
        self.preflight_elements(n.saturating_mul(n).saturating_mul(8))?;
        let integer = scalar.im == 0. && scalar.re.fract() == 0. && scalar.re.abs() <= 1e6;
        if !base && integer {
            if !complex {
                return Ok(None);
            }
            return Ok(Some(self.cmath_integer_matrix_power(a, scalar.re)?));
        }
        let f = |z: C| {
            if base {
                cpow(scalar, z)
            } else {
                cpow(z, scalar)
            }
        };
        let value = self.cmath_eigen_function(self.cmath_dense(a), !complex, f, out)?;
        Ok(Some(self.cmath_matrix_value(value, false)?))
    }

    fn cmath_integer_matrix_power(&self, a: &Operand, p: f64) -> Result<Value> {
        let n = a.shape().0;
        let mut base = self.cmath_dense(a);
        if p < 0. {
            base = base
                .try_inverse()
                .ok_or("Matrix is singular to working precision.")?;
        }
        let mut result = DMatrix::<C>::identity(n, n);
        let mut k = p.abs() as u64;
        while k > 0 {
            self.check_cancel()?;
            if k & 1 == 1 {
                result = &result * &base;
            }
            k >>= 1;
            if k > 0 {
                base = &base * &base;
            }
        }
        self.cmath_matrix_value(result, false)
    }

    /// The reference's mpower: V * diag(f(d)) / V. Schur supplies the eigenvalues;
    /// triangular back substitution supplies the right eigenvectors. Repeated
    /// roots use an epsilon pivot, as in eigenvector extraction, rather than
    /// derivatives of f. Thus defective inputs retain MATLAB's warning and
    /// potentially inaccurate eig-based result.
    fn cmath_eigen_function(
        &mut self,
        a: DMatrix<C>,
        real_input: bool,
        f: impl Fn(C) -> C,
        out: &mut Execution,
    ) -> Result<DMatrix<C>> {
        let n = a.nrows();
        if a.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
            return Ok(DMatrix::from_element(n, n, C::new(f64::NAN, f64::NAN)));
        }
        let norm = a
            .column_iter()
            .map(|c| c.iter().map(|z| z.norm()).sum::<f64>())
            .fold(0., f64::max);
        let upper = (0..n).all(|j| (j + 1..n).all(|i| a[(i, j)] == zero()));
        let (q, mut t) = if upper {
            (None, a)
        } else {
            let schur = nalgebra::linalg::Schur::try_new(a, f64::EPSILON, 1000 * n + 10000)
                .ok_or("Schur decomposition did not converge.")?;
            self.check_cancel()?;
            let (q, t) = schur.unpack();
            (Some(q), t)
        };
        if real_input {
            // Snap numerically real eigenvalues onto the real axis, as real Schur does.
            let snap = 100. * f64::EPSILON * norm.max(f64::MIN_POSITIVE);
            for i in 0..n {
                if t[(i, i)].im.abs() <= snap {
                    t[(i, i)].im = 0.;
                }
            }
        }
        for j in 0..n {
            for i in j + 1..n {
                t[(i, j)] = zero();
            }
        }
        let mut vectors = DMatrix::identity(n, n);
        for j in 0..n {
            self.check_cancel()?;
            let pivot_min = (f64::EPSILON * t[(j, j)].norm()).max(f64::MIN_POSITIVE);
            for i in (0..j).rev() {
                self.check_cancel()?;
                let mut sum = zero();
                for k in i + 1..=j {
                    sum += t[(i, k)] * vectors[(k, j)];
                }
                let mut pivot = t[(i, i)] - t[(j, j)];
                if pivot.norm() < pivot_min {
                    pivot = C::new(pivot_min, 0.);
                }
                // Scale the partial vector before division, keeping its
                // components bounded even for nearly defective matrices.
                let scale = sum.norm().max(pivot.norm());
                if scale > pivot.norm() {
                    let factor = pivot.norm() / scale;
                    for k in i + 1..=j {
                        vectors[(k, j)] *= factor;
                    }
                    sum *= factor;
                }
                vectors[(i, j)] = -cdiv(sum, pivot);
            }
            let length = vectors.column(j).norm();
            vectors.column_mut(j).unscale_mut(length);
        }
        let vectors = match q {
            Some(q) => q * vectors,
            None => vectors,
        };
        let mut rhs = vectors.clone();
        for j in 0..n {
            self.check_cancel()?;
            let value = f(t[(j, j)]);
            for i in 0..n {
                rhs[(i, j)] *= value;
            }
        }
        drop(t);
        // Right division solves V^T X^T = (V f(D))^T, without an inverse.
        let transposed = vectors.transpose();
        drop(vectors);
        let norm1 = |m: &DMatrix<C>| {
            m.column_iter()
                .map(|c| c.iter().map(|z| z.norm()).sum::<f64>())
                .fold(0., f64::max)
        };
        let vnorm = norm1(&transposed);
        let lu = transposed.lu();
        // Compute the inverse 1-norm by individual solves: no inverse matrix
        // is stored or used in the power. This also detects an ill-conditioned
        // basis whose small pivots alone would not reveal the condition.
        let mut inverse_norm: f64 = 0.;
        for j in 0..n {
            self.check_cancel()?;
            let mut unit = DMatrix::zeros(n, 1);
            unit[(j, 0)] = C::new(1., 0.);
            let column = lu
                .solve(&unit)
                .ok_or("Matrix is singular to working precision.")?;
            inverse_norm = inverse_norm.max(norm1(&column));
        }
        let rcond = (1. / vnorm) / inverse_norm;
        if rcond < f64::EPSILON {
            self.emit(out, format!("Warning: Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = {rcond:.6e}."))?;
        }
        self.check_cancel()?;
        // Reuse the square buffers and release decomposition temporaries
        // early, keeping the solve within the preflight workspace allowance.
        rhs.transpose_mut();
        let mut value = lu
            .solve(&rhs)
            .ok_or("Matrix is singular to working precision.")?;
        value.transpose_mut();
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: C, b: C) -> bool {
        (a - b).norm() <= 1e-12 * b.norm().max(1.)
    }

    #[test]
    fn elementary_formulas_match_the_textbook_ones_where_those_are_exact() {
        for z in [C::new(1., 2.), C::new(-0.5, 0.25), C::new(3., -4.)] {
            assert!(close(csqrt(z), z.sqrt()), "sqrt {z}");
            assert!(close(clog(z), z.ln()), "log {z}");
            assert!(close(csin(z), z.sin()), "sin {z}");
            assert!(close(ccos(z), z.cos()), "cos {z}");
            assert!(close(ctan(z), z.tan()), "tan {z}");
            assert!(close(cacos(z), z.acos()), "acos {z}");
            assert!(close(casin(z), z.asin()), "asin {z}");
            assert!(close(catan(z), z.atan()), "atan {z}");
            assert!(close(cpow(z, C::new(0.5, 1.)), z.powc(C::new(0.5, 1.))));
        }
    }

    #[test]
    fn large_arguments_do_not_overflow() {
        assert_eq!(cdiv(C::new(1., 0.), C::new(f64::INFINITY, 1.)), zero());
        assert_eq!(cdiv(C::new(1., 0.), C::new(1., f64::INFINITY)), zero());
        assert_eq!(ctan(C::new(1., 400.)).im, 1.);
        assert!(ctan(C::new(1., 400.)).re.abs() < 1e-300);
        assert!((ln_abs(C::new(1e300, 1e300)) - (1e300_f64.ln() + 0.5 * 2_f64.ln())).abs() < 1e-12);
        let s = csqrt(C::new(-1e308, 1e308));
        assert!(s.re.is_finite() && s.im.is_finite());
        let a = cacos(C::new(1e200, 1e200));
        assert!(a.re.is_finite() && a.im.is_finite());
        assert_eq!(csin(C::new(0., 1000.)), C::new(0., f64::INFINITY));
    }

    #[test]
    fn exact_cases() {
        assert_eq!(cpow(C::new(1., 2.), C::new(2., 0.)), C::new(-3., 4.));
        assert_eq!(real_power(-1., 0.5), C::new(0., 1.));
        assert_eq!(real_power(-2., 2.), C::new(4., 0.));
        assert_eq!(csqrt(C::new(3., 4.)), C::new(2., 1.));
    }
}
