//! conv, deconv, roots, poly, polyder, polyint, conv2.
use super::{
    Engine, Value, complex_value, is_vector, matrix_value, narrowed_value, real_matrix, text_of,
};
use crate::{Complex64, Matrix, Result};
use nalgebra::{DMatrix, linalg::Schur};
use std::ops::{Add, Mul};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_WORK: u128 = 1 << 36;
const MAX_DEGREE: usize = 1000;

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Full,
    Same,
    Valid,
}
fn shape_of(value: &Value) -> Result<Shape> {
    match text_of(value).map(str::to_ascii_lowercase).as_deref() {
        Some("full") => Ok(Shape::Full),
        Some("same") => Ok(Shape::Same),
        Some("valid") => Ok(Shape::Valid),
        _ => Err("Unknown shape parameter.".into()),
    }
}
fn arg_count(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}
fn full_convolution<T: Copy + Default + Add<Output = T> + Mul<Output = T>>(
    a: &[T],
    b: &[T],
    cancel: &AtomicBool,
) -> Result<Vec<T>> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    if (a.len() as u128) * (b.len() as u128) > MAX_WORK {
        return Err("Convolution is too large (more than 2^36 products)".into());
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut y = vec![T::default(); a.len() + b.len() - 1];
    for (i, &s) in short.iter().enumerate() {
        cancelled(cancel)?;
        for (slot, &l) in y[i..].iter_mut().zip(long) {
            *slot = *slot + s * l;
        }
    }
    Ok(y)
}
fn crop<T: Copy>(full: &[T], na: usize, nb: usize, shape: Shape) -> Vec<T> {
    match shape {
        Shape::Full => full.to_vec(),
        Shape::Same => {
            let start = nb.saturating_sub(1).div_ceil(2);
            full[start..start + na].to_vec()
        }
        Shape::Valid => {
            if na >= nb {
                full[nb - 1..na].to_vec()
            } else {
                Vec::new()
            }
        }
    }
}
/// Convolution orientation: 'full' follows the longer input (the second on a tie),
/// 'same' and 'valid' follow the first; rows stay rows (size(x,1) == 1).
fn column_result(u: (usize, usize), v: (usize, usize), shape: Shape) -> bool {
    let follow = if shape == Shape::Full && u.0 * u.1 <= v.0 * v.1 {
        v
    } else {
        u
    };
    follow.0 != 1
}
fn complex_data(value: &Value, who: &str) -> Result<Vec<Complex64>> {
    match value {
        Value::Complex(c) => Ok(c.data().to_vec()),
        other => Ok(real_matrix(other, who)?
            .data()
            .iter()
            .map(|&x| Complex64::new(x, 0.))
            .collect()),
    }
}
fn dims(value: &Value) -> (usize, usize) {
    match value {
        Value::Matrix(m) => (m.rows(), m.cols()),
        Value::Logical(m) => (m.rows(), m.cols()),
        Value::Complex(m) => (m.rows(), m.cols()),
        _ => (0, 0),
    }
}
fn vector_dims(value: &Value, who: &str) -> Result<(usize, usize)> {
    match value {
        Value::Matrix(_) | Value::Logical(_) | Value::Complex(_) => {
            let (r, c) = dims(value);
            if r == 1 || c == 1 {
                Ok((r, c))
            } else {
                Err(format!("{who}: A and B must be vectors."))
            }
        }
        Value::Text(s) => Ok((1, s.chars().count())),
        _ => Err(format!("{who}: inputs must be numeric or logical arrays")),
    }
}
/// conv2 'full' length along one dimension; with an empty side it is the
/// longer one (reference: conv2([],[1 2]) is 1-by-2 zeros, conv2([],[]) 0-by-0).
fn full_len(a: usize, b: usize) -> usize {
    if a == 0 || b == 0 {
        a.max(b)
    } else {
        a + b - 1
    }
}
/// conv/conv2 accept only double data (reference errors on logical and char).
fn double_only(args: &[Value]) -> Result<()> {
    if args.iter().any(|v| matches!(v, Value::Text(_))) {
        return Err("Undefined function 'conv2' for input arguments of type 'char'.".into());
    }
    if args.iter().any(|v| matches!(v, Value::Logical(_))) {
        return Err("First and second arguments must be single or double.".into());
    }
    Ok(())
}
fn orient(len: usize, column: bool) -> (usize, usize) {
    if column { (len, 1) } else { (1, len) }
}
/// Eigenvalues of a real square matrix after diagonal (power-of-two) balancing.
pub(super) fn eigenvalues(mut a: DMatrix<f64>) -> Result<Vec<Complex64>> {
    let n = a.nrows();
    if n == 0 {
        return Ok(Vec::new());
    }
    if a.iter().any(|x| !x.is_finite()) {
        return Err("Input to EIG must not contain NaN or Inf.".into());
    }
    for _ in 0..64 {
        let mut converged = true;
        for i in 0..n {
            let (mut c, mut r) = (0., 0.);
            for j in 0..n {
                if j != i {
                    c += a[(j, i)].abs();
                    r += a[(i, j)].abs();
                }
            }
            if c == 0. || r == 0. {
                continue;
            }
            let s = c + r;
            let (mut f, mut scaled, mut g) = (1., c, r / 2.);
            while scaled < g {
                f *= 2.;
                scaled *= 4.;
            }
            g = r * 2.;
            while scaled >= g {
                f /= 2.;
                scaled /= 4.;
            }
            // Accept only when the scaled norms shrink by at least 5%.
            if c * f + r / f < 0.95 * s {
                for j in 0..n {
                    a[(i, j)] /= f;
                    a[(j, i)] *= f;
                }
                converged = false;
            }
        }
        if converged {
            break;
        }
    }
    let schur = Schur::try_new(a, f64::EPSILON, 0).ok_or("EIG did not converge")?;
    Ok(schur.complex_eigenvalues().iter().copied().collect())
}

impl Engine {
    pub(super) fn polyopt_conv(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 2, 3)?;
        let u = vector_dims(&args[0], "conv")?;
        let v = vector_dims(&args[1], "conv")?;
        let shape = args.get(2).map_or(Ok(Shape::Full), shape_of)?;
        double_only(&args[..2])?;
        let (nu, nv) = (u.0 * u.1, v.0 * v.1);
        self.polyopt_admit(nu + nv + (nu + nv).saturating_sub(1))?;
        let column = column_result(u, v, shape);
        let complex = matches!(args[0], Value::Complex(_)) || matches!(args[1], Value::Complex(_));
        let out_len = match shape {
            Shape::Full => (nu + nv).saturating_sub(1) * usize::from(nu > 0 && nv > 0),
            Shape::Same => nu,
            Shape::Valid => (nu + 1).saturating_sub(nv),
        };
        if nu == 0 || nv == 0 {
            let len = if shape == Shape::Full {
                full_len(nu, nv)
            } else {
                out_len
            };
            let (r, c) = orient(len, column);
            return matrix_value(r, c, vec![0.; len]);
        }
        let (r, c) = orient(out_len, column);
        if complex {
            let full = full_convolution(
                &complex_data(&args[0], "conv")?,
                &complex_data(&args[1], "conv")?,
                &self.cancel,
            )?;
            narrowed_value(self, r, c, crop(&full, nu, nv, shape))
        } else {
            let a = real_matrix(&args[0], "conv")?;
            let b = real_matrix(&args[1], "conv")?;
            let full = full_convolution(a.data(), b.data(), &self.cancel)?;
            matrix_value(r, c, crop(&full, nu, nv, shape))
        }
    }

    pub(super) fn polyopt_deconv(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        arg_count(args, 2, 2)?;
        let bd = vector_dims(&args[0], "deconv")?;
        vector_dims(&args[1], "deconv")?;
        let b = real_matrix(&args[0], "deconv")?;
        let a = real_matrix(&args[1], "deconv")?;
        let a = a.data();
        let Some(&lead) = a.first() else {
            return Err("Index exceeds matrix dimensions.".into());
        };
        if lead == 0. {
            return Err("First coefficient of A must be non-zero.".into());
        }
        let (nb, na) = (b.len(), a.len());
        self.polyopt_admit(2 * nb + 1)?;
        if na > nb {
            return Ok(vec![
                matrix_value(1, 1, vec![0.])?,
                matrix_value(bd.0, bd.1, b.data().to_vec())?,
            ]
            .into_iter()
            .take(nout.max(1))
            .collect());
        }
        let nq = nb - na + 1;
        let mut q = vec![0.; nq];
        for k in 0..nq {
            self.check_cancel()?;
            let mut s = b.data()[k];
            for j in 1..na.min(k + 1) {
                s -= a[j] * q[k - j];
            }
            q[k] = s / a[0];
        }
        // deconv: q is a column when b is not a row (mb ~= 1).
        let (qr, qc) = orient(nq, bd.0 != 1);
        let mut outputs = vec![matrix_value(qr, qc, q.clone())?];
        if nout > 1 {
            let prod = full_convolution(a, &q, &self.cancel)?;
            let r: Vec<f64> = b.data().iter().zip(&prod).map(|(x, y)| x - y).collect();
            outputs.push(matrix_value(bd.0, bd.1, r)?);
        }
        Ok(outputs)
    }

    pub(super) fn polyopt_roots(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 1, 1)?;
        let (rows, cols) = dims(&args[0]);
        if rows * cols == 0 {
            real_matrix(&args[0], "roots")?;
            return matrix_value(0, 1, Vec::new());
        }
        if rows != 1 && cols != 1 {
            return Err("Input must be a vector.".into());
        }
        let x = complex_data(&args[0], "roots")?;
        if x.iter().any(|v| !v.is_finite()) {
            return Err("Input to ROOTS must not contain NaN or Inf.".into());
        }
        let zero = Complex64::new(0., 0.);
        let Some(lead) = x.iter().position(|&v| v != zero) else {
            return matrix_value(0, 1, Vec::new());
        };
        let last = x.iter().rposition(|&v| v != zero).unwrap_or(lead);
        let c = &x[lead..=last];
        if c.len() > MAX_DEGREE + 1 {
            return Err(format!("roots supports degree up to {MAX_DEGREE}"));
        }
        // roots: the roots at zero (trailing zero coefficients) come first.
        let mut roots = vec![zero; x.len() - 1 - last];
        let m = c.len() - 1;
        if m > 0 {
            let d: Vec<Complex64> = c[1..].iter().map(|v| v / c[0]).collect();
            if d.iter().any(|v| !v.is_finite()) {
                return Err("roots: coefficients overflow when scaled by the leading term".into());
            }
            self.check_cancel()?;
            if d.iter().all(|v| v.im == 0.) {
                let mut a = DMatrix::<f64>::zeros(m, m);
                for j in 0..m {
                    a[(0, j)] = -d[j].re;
                }
                for i in 1..m {
                    a[(i, i - 1)] = 1.;
                }
                roots.extend(eigenvalues(a)?);
            } else {
                let mut a = DMatrix::<Complex64>::zeros(m, m);
                for j in 0..m {
                    a[(0, j)] = -d[j];
                }
                for i in 1..m {
                    a[(i, i - 1)] = Complex64::new(1., 0.);
                }
                let schur = Schur::try_new(a, f64::EPSILON, 0).ok_or("EIG did not converge")?;
                roots.extend(schur.eigenvalues().ok_or("EIG did not converge")?.iter());
            }
            self.check_cancel()?;
        }
        let n = roots.len();
        self.polyopt_admit(2 * n)?;
        narrowed_value(self, n, 1, roots)
    }

    pub(super) fn polyopt_poly(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 1, 1)?;
        let (rows, cols) = dims(&args[0]);
        if !matches!(
            args[0],
            Value::Matrix(_) | Value::Logical(_) | Value::Complex(_)
        ) {
            return Err("poly: input must be numeric or logical".into());
        }
        let e: Vec<Complex64> = if rows * cols == 0 {
            Vec::new()
        } else if rows == 1 || cols == 1 {
            complex_data(&args[0], "poly")?
        } else if rows == cols {
            if matches!(args[0], Value::Complex(_)) {
                return Err("poly: complex matrices are not supported".into());
            }
            let m = real_matrix(&args[0], "poly")?;
            if m.len() > MAX_DEGREE * MAX_DEGREE {
                return Err(format!(
                    "poly supports matrices up to {MAX_DEGREE}-by-{MAX_DEGREE}"
                ));
            }
            eigenvalues(DMatrix::from_column_slice(rows, cols, m.data()))?
        } else {
            return Err("Argument must be a vector or a square matrix.".into());
        };
        let n = e.len();
        self.polyopt_admit(2 * n + 2)?;
        let mut y = vec![Complex64::new(0., 0.); n + 1];
        y[0] = Complex64::new(1., 0.);
        for (j, &root) in e.iter().enumerate() {
            self.check_cancel()?;
            for i in (1..=j + 1).rev() {
                let t = root * y[i - 1];
                y[i] -= t;
            }
        }
        // Conjugate-closed root sets give a real polynomial.
        let real = e.iter().all(|z| z.im == 0.) || conjugate_closed(&e);
        if real {
            matrix_value(1, n + 1, y.iter().map(|z| z.re).collect())
        } else {
            complex_value(self, 1, n + 1, y)
        }
    }

    /// Read coefficients in column-major order. With two outputs, apply
    /// the quotient rule and return its numerator and squared denominator.
    pub(super) fn polyopt_polyder(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        arg_count(args, 1, 2)?;
        let u = real_matrix(&args[0], "polyder")?.data().to_vec();
        let v = match args.get(1) {
            Some(second) => real_matrix(second, "polyder")?.data().to_vec(),
            None => vec![1.],
        };
        self.polyopt_admit(2 * (u.len() + v.len()) + 2)?;
        let a1 = full_convolution(&derivative(&u), &v, &self.cancel)?;
        let a2 = full_convolution(&u, &derivative(&v), &self.cancel)?;
        let len = a1.len().max(a2.len());
        let sign = if nout < 2 { 1. } else { -1. };
        let a: Vec<f64> = (0..len)
            .map(|i| {
                let at = |p: &[f64]| (i + p.len()).checked_sub(len).map_or(0., |k| p[k]);
                at(&a1) + sign * at(&a2)
            })
            .collect();
        let a = strip_leading_zeros(a);
        let mut out = vec![matrix_value(1, a.len(), a)?];
        if nout > 1 {
            let b = strip_leading_zeros(full_convolution(&v, &v, &self.cancel)?);
            out.push(matrix_value(1, b.len(), b)?);
        }
        Ok(out)
    }

    pub(super) fn polyopt_polyint(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 1, 2)?;
        let (rows, _) = dims(&args[0]);
        let p = complex_data(&args[0], "polyint")?;
        // Integration divides each coefficient by its new exponent, using
        // a row of divisors; only row and scalar inputs fit.
        if rows != 1 && p.len() != 1 {
            return Err("Matrix dimensions must agree.".into());
        }
        let k = match args.get(1) {
            None => vec![Complex64::new(0., 0.)],
            Some(v) => {
                let (kr, kc) = dims(v);
                let k = complex_data(v, "polyint")?;
                if kr != 1 && kr * kc != 0 {
                    return Err(
                        "Dimensions of matrices being concatenated are not consistent.".into(),
                    );
                }
                k
            }
        };
        let n = p.len();
        self.polyopt_admit(n + k.len())?;
        let mut y: Vec<Complex64> = p
            .iter()
            .enumerate()
            .map(|(i, v)| v / (n - i) as f64)
            .collect();
        y.extend(k);
        narrowed_value(self, 1, y.len(), y)
    }

    pub(super) fn polyopt_conv2(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 2, 4)?;
        let separable = args.len() >= 3 && text_of(&args[2]).is_none();
        let shape = match args.len() {
            2 => Shape::Full,
            3 if !separable => shape_of(&args[2])?,
            4 => shape_of(&args[3])?,
            _ => Shape::Full,
        };
        if args.len() == 4 && !separable {
            return Err("Too many input arguments.".into());
        }
        if separable {
            let col = real_matrix(&args[0], "conv2")?;
            let row = real_matrix(&args[1], "conv2")?;
            let a = real_matrix(&args[2], "conv2")?;
            if !is_vector(&col) || !is_vector(&row) {
                return Err("conv2: the first two inputs must be vectors".into());
            }
            let step1 = self.polyopt_conv2_core(&a, col.data(), col.len(), 1, shape)?;
            let out = self.polyopt_conv2_core(&step1, row.data(), 1, row.len(), shape)?;
            return Ok(Value::Matrix(out));
        }
        double_only(&args[..2])?;
        let (ma, na) = dims(&args[0]);
        let (mb, nb) = dims(&args[1]);
        if matches!(args[0], Value::Complex(_)) || matches!(args[1], Value::Complex(_)) {
            let a = complex_data(&args[0], "conv2")?;
            let b = complex_data(&args[1], "conv2")?;
            let (r, c, data) = self.polyopt_conv2_data(&a, ma, na, &b, mb, nb, shape)?;
            return narrowed_value(self, r, c, data);
        }
        let a = real_matrix(&args[0], "conv2")?;
        let b = real_matrix(&args[1], "conv2")?;
        Ok(Value::Matrix(self.polyopt_conv2_core(
            &a,
            b.data(),
            b.rows(),
            b.cols(),
            shape,
        )?))
    }

    fn polyopt_conv2_core(
        &self,
        a: &Matrix,
        b: &[f64],
        mb: usize,
        nb: usize,
        shape: Shape,
    ) -> Result<Matrix> {
        let (r, c, data) =
            self.polyopt_conv2_data(a.data(), a.rows(), a.cols(), b, mb, nb, shape)?;
        Matrix::from_column_vec(r, c, data)
    }

    /// Column-major 2-D convolution of an ma-by-na A with an mb-by-nb B.
    #[allow(clippy::too_many_arguments)]
    fn polyopt_conv2_data<T: Copy + Default + Add<Output = T> + Mul<Output = T>>(
        &self,
        a: &[T],
        ma: usize,
        na: usize,
        b: &[T],
        mb: usize,
        nb: usize,
        shape: Shape,
    ) -> Result<(usize, usize, Vec<T>)> {
        let empty = ma == 0 || na == 0 || mb == 0 || nb == 0;
        let (fr, fc) = (full_len(ma, mb), full_len(na, nb));
        let (or, oc) = match shape {
            Shape::Full => (fr, fc),
            Shape::Same => (ma, na),
            Shape::Valid => ((ma + 1).saturating_sub(mb), (na + 1).saturating_sub(nb)),
        };
        self.polyopt_admit(fr * fc + or * oc)?;
        if (a.len() as u128) * (b.len() as u128) > MAX_WORK {
            return Err("Convolution is too large (more than 2^36 products)".into());
        }
        if empty {
            return Ok((or, oc, vec![T::default(); or * oc]));
        }
        let mut full = vec![T::default(); fr * fc];
        for cb in 0..nb {
            self.check_cancel()?;
            for rb in 0..mb {
                let w = b[cb * mb + rb];
                for ca in 0..na {
                    let src = &a[ca * ma..(ca + 1) * ma];
                    let dst = &mut full[(ca + cb) * fr + rb..(ca + cb) * fr + rb + ma];
                    for (d, &s) in dst.iter_mut().zip(src) {
                        *d = *d + s * w;
                    }
                }
            }
        }
        let (r0, c0) = match shape {
            Shape::Full => (0, 0),
            Shape::Same => (
                mb.saturating_sub(1).div_ceil(2),
                nb.saturating_sub(1).div_ceil(2),
            ),
            Shape::Valid => (mb.saturating_sub(1), nb.saturating_sub(1)),
        };
        let mut out = Vec::with_capacity(or * oc);
        for c in 0..oc {
            out.extend_from_slice(&full[(c + c0) * fr + r0..(c + c0) * fr + r0 + or]);
        }
        Ok((or, oc, out))
    }
}

/// Derivative coefficients; constant or empty polynomials yield [0].
fn derivative(p: &[f64]) -> Vec<f64> {
    let n = p.len();
    if n < 2 {
        return vec![0.];
    }
    p[..n - 1]
        .iter()
        .enumerate()
        .map(|(i, v)| v * (n - 1 - i) as f64)
        .collect()
}
fn strip_leading_zeros(p: Vec<f64>) -> Vec<f64> {
    match p.iter().position(|&x| x != 0.) {
        Some(first) => p[first..].to_vec(),
        None => vec![0.],
    }
}
fn conjugate_closed(e: &[Complex64]) -> bool {
    let mut pos: Vec<(f64, f64)> = e
        .iter()
        .filter(|z| z.im > 0.)
        .map(|z| (z.re, z.im))
        .collect();
    let mut neg: Vec<(f64, f64)> = e
        .iter()
        .filter(|z| z.im < 0.)
        .map(|z| (z.re, -z.im))
        .collect();
    if pos.len() != neg.len() {
        return false;
    }
    let key =
        |a: &(f64, f64), b: &(f64, f64)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    pos.sort_by(key);
    neg.sort_by(key);
    pos == neg
}
