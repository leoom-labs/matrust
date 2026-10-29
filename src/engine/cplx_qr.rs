//! Complex qr, chol and rectangular \ and / (pivoted Householder least squares).
//! Every private method is prefixed `cxl_` (all `impl Engine` blocks share one
//! method namespace).
use super::cplx_linalg::cxl_element;
use super::eig_more::cdense::{CMat, cabs, zladiv, zlarf_left, zlarfg, znrm2, zung2r};
use super::eig_more::dense::{self, EPS, SAFMIN};
use super::{Engine, Execution, Value};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use nalgebra::DMatrix;
use std::sync::atomic::Ordering;

const BUDGET: usize = 32 * 1024 * 1024;
const MAX_WORK: f64 = 4.3e9;
const BUDGET_ERROR: &str =
    "Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps";

/// Householder QR of an m x n matrix as LAPACK zgeqr2, or with column pivoting
/// as zlaqp2 (the norm downdate of the real `linalg2_qr`) when `pivot`.
struct CxlQr {
    h: CMat,
    tau: Vec<Complex64>,
    jpvt: Vec<usize>,
}

fn cxl_cmat(a: &Array) -> CMat {
    CMat {
        rows: a.rows(),
        cols: a.cols(),
        data: (0..a.len()).map(|i| cxl_element(a, i)).collect(),
    }
}

fn cxl_transpose(a: &CMat) -> CMat {
    let mut t = CMat::zeros(a.cols, a.rows);
    for j in 0..a.cols {
        for i in 0..a.rows {
            t.set(j, i, a.at(i, j));
        }
    }
    t
}

fn cxl_text(v: &Value, cancel: &std::sync::atomic::AtomicBool) -> Option<Result<String>> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(Ok(s.clone())),
        Value::Container(Array::Character(a)) => Some(a.to_utf8(cancel)),
        _ => None,
    }
}

/// dznrm2 as MKL's unpivoted zgeqrf evaluates it: the unscaled sum
/// of squares, which cplx1 case 91 needs for the last digit of R(2,2) (the
/// scaled reference-LAPACK `znrm2` is one ulp off there). The scaled form takes over
/// where squares would overflow or underflow (a zero sum with a nonzero entry
/// is an underflow, not an exact zero).
fn cxl_nrm2(x: &[Complex64]) -> f64 {
    let ssq: f64 = x.iter().map(|z| z.re * z.re + z.im * z.im).sum();
    let all_zero = x.iter().all(|z| z.re == 0. && z.im == 0.);
    if ssq.is_finite() && ((ssq == 0. && all_zero) || ssq >= f64::MIN_POSITIVE / f64::EPSILON) {
        ssq.sqrt()
    } else {
        znrm2(x.iter())
    }
}

fn cxl_lapy3(x: f64, y: f64, z: f64) -> f64 {
    let (xa, ya, za) = (x.abs(), y.abs(), z.abs());
    let w = xa.max(ya).max(za);
    if w == 0. {
        xa + ya + za
    } else {
        w * ((xa / w) * (xa / w) + (ya / w) * (ya / w) + (za / w) * (za / w)).sqrt()
    }
}

/// cdense.rs `zlarfg` with `cxl_nrm2` for the unpivoted factorization.
fn cxl_zlarfg(alpha: Complex64, x: &mut [Complex64]) -> (Complex64, Complex64) {
    let mut xnorm = cxl_nrm2(x);
    let (mut alphr, mut alphi) = (alpha.re, alpha.im);
    if xnorm == 0. && alphi == 0. {
        return (alpha, Complex64::new(0., 0.));
    }
    let mut beta = -dense::sign(cxl_lapy3(alphr, alphi, xnorm), alphr);
    let safmin = SAFMIN / EPS;
    let rsafmn = 1. / safmin;
    let mut knt = 0;
    let mut alpha = alpha;
    if beta.abs() < safmin {
        loop {
            knt += 1;
            for v in x.iter_mut() {
                *v = Complex64::new(rsafmn, 0.) * *v;
            }
            beta *= rsafmn;
            alphi *= rsafmn;
            alphr *= rsafmn;
            if beta.abs() >= safmin || knt >= 20 {
                break;
            }
        }
        xnorm = cxl_nrm2(x);
        alpha = Complex64::new(alphr, alphi);
        beta = -dense::sign(cxl_lapy3(alphr, alphi, xnorm), alphr);
    }
    let tau = Complex64::new((beta - alphr) / beta, -alphi / beta);
    let scal = zladiv(Complex64::new(1., 0.), alpha - Complex64::new(beta, 0.));
    for v in x.iter_mut() {
        *v = scal * *v;
    }
    for _ in 0..knt {
        beta *= safmin;
    }
    (Complex64::new(beta, 0.), tau)
}

impl Engine {
    fn cxl_cancelled(&self) -> impl Fn() -> bool + '_ {
        || self.cancel.load(Ordering::Relaxed)
    }

    /// Dense work and statement budget for a factorization of an m x n
    /// complex matrix producing `outputs` complex elements.
    fn cxl_bounds(&self, m: usize, n: usize, outputs: usize) -> Result<()> {
        let work = (m as f64) * (n as f64) * (m.min(n) as f64);
        if work > MAX_WORK {
            return Err("The factorization exceeds the dense work limit of this version".into());
        }
        let units = ComplexMatrix::checked_len(m, n)?
            .saturating_add(outputs)
            .saturating_mul(2);
        if self.evaluated_elements.saturating_add(units) > BUDGET {
            return Err(BUDGET_ERROR.into());
        }
        Ok(())
    }

    /// A complex result; all-zero imaginary parts narrow to real as MATLAB's
    /// factorizations return them (chol's 1x1 leading factor prints `1`).
    fn cxl_qr_value(&self, m: CMat) -> Result<Value> {
        self.check_cancel()?;
        if m.data.iter().all(|z| z.im == 0.) {
            let re = m.data.iter().map(|z| z.re).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(m.rows, m.cols, re)?));
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            m.rows,
            m.cols,
            m.data,
            &self.cancel,
        )?))
    }

    fn cxl_operand(&self, v: &Value) -> Result<CMat> {
        let a = v
            .clone()
            .numeric_operand(&self.cancel)?
            .array(&self.cancel)?;
        match a {
            Array::Complex(_) | Array::Numeric(_) | Array::Logical(_) => Ok(cxl_cmat(&a)),
            _ => Err("Complex linear algebra supports double and logical input only".into()),
        }
    }

    fn cxl_geqp3(&self, a: CMat, pivot: bool) -> Result<CxlQr> {
        let (m, n) = (a.rows, a.cols);
        let k = m.min(n);
        let mut h = a;
        let mut tau = vec![Complex64::new(0., 0.); k];
        let mut jpvt: Vec<usize> = (0..n).collect();
        let mut vn1: Vec<f64> = (0..n).map(|j| znrm2(h.col(j))).collect();
        let mut vn2 = vn1.clone();
        let tol3z = f64::EPSILON.sqrt();
        for i in 0..k {
            self.check_cancel()?;
            if pivot {
                let mut best = i;
                for j in i + 1..n {
                    if vn1[j] > vn1[best] {
                        best = j;
                    }
                }
                if best != i {
                    h.swap_cols(best, i, 0, m);
                    jpvt.swap(best, i);
                    vn1[best] = vn1[i];
                    vn2[best] = vn2[i];
                }
            }
            let alpha = h.at(i, i);
            // The pivoted path (zgeqp3) matches the expected values with the scaled
            // norm of reference LAPACK (cplx1 38-39); the unpivoted one (zgeqrf) needs MKL's.
            let x = &mut h.data[i * m + i + 1..(i + 1) * m];
            let (beta, t) = if pivot {
                zlarfg(alpha, x)
            } else {
                cxl_zlarfg(alpha, x)
            };
            tau[i] = t;
            if i + 1 < n {
                let v: Vec<Complex64> = std::iter::once(Complex64::new(1., 0.))
                    .chain(h.data[i * m + i + 1..(i + 1) * m].iter().copied())
                    .collect();
                zlarf_left(&mut h, &v, t.conj(), i, i + 1, n - i - 1);
            }
            h.set(i, i, beta);
            if pivot {
                for j in i + 1..n {
                    if vn1[j] != 0. {
                        let t = (cabs(h.at(i, j)) / vn1[j]).powi(2);
                        let t = (1. - t).max(0.);
                        let t2 = t * (vn1[j] / vn2[j]).powi(2);
                        if t2 <= tol3z {
                            vn1[j] = znrm2(&h.data[j * m + i + 1..(j + 1) * m]);
                            vn2[j] = vn1[j];
                        } else {
                            vn1[j] *= t.sqrt();
                        }
                    }
                }
            }
        }
        Ok(CxlQr { h, tau, jpvt })
    }

    /// The explicit m x qc factor Q (zung2r on the reflectors).
    fn cxl_q(&self, qr: &CxlQr, qc: usize) -> Result<CMat> {
        let (m, k) = (qr.h.rows, qr.tau.len());
        let mut q = CMat::zeros(m, qc);
        q.data[..m * k.min(qc)].copy_from_slice(&qr.h.data[..m * k.min(qc)]);
        zung2r(&mut q, k, &qr.tau, 0, 0, m, qc, &self.cxl_cancelled());
        self.check_cancel()?;
        Ok(q)
    }

    /// qr and chol with a complex first argument, after linalg2_call's
    /// argument and output counts.
    pub(super) fn cxl_factor_call(
        &self,
        name: &str,
        args: &[Value],
        outputs: usize,
    ) -> Result<Vec<Value>> {
        let a = self.cxl_operand(&args[0])?;
        if name == "chol" {
            let lower = match args.get(1).map(|v| cxl_text(v, &self.cancel)) {
                None => false,
                Some(Some(s)) => match s? {
                    s if s.eq_ignore_ascii_case("lower") => true,
                    s if s.eq_ignore_ascii_case("upper") => false,
                    _ => return Err("Unknown option: use 'lower' or 'upper'.".into()),
                },
                Some(None) => return Err("chol: the option must be a character vector".into()),
            };
            return self.cxl_chol(a, outputs, lower);
        }
        let (economy, vector) = match args.get(1) {
            None => (false, false),
            Some(v) => match cxl_text(v, &self.cancel) {
                Some(s) => match s?.as_str() {
                    "vector" => (false, true),
                    "matrix" => (false, false),
                    _ => return Err("Unknown qr option: use 'matrix' or 'vector'.".into()),
                },
                None => {
                    let z = match v {
                        Value::Matrix(m) => m.clone(),
                        Value::Logical(m) => m.to_numeric(&self.cancel)?,
                        _ => return Err("Second argument must be 0 (economy-size).".into()),
                    };
                    if z.len() != 1 || z.data()[0] != 0. {
                        return Err("Second argument must be 0 (economy-size).".into());
                    }
                    (true, true)
                }
            },
        };
        self.cxl_qr(a, outputs, economy, vector)
    }

    /// The complex copy of `linalg2_qr`: zgeqr2, or zlaqp2 pivoting for
    /// three outputs; one output returns the packed factor as the reference does.
    fn cxl_qr(&self, a: CMat, outputs: usize, economy: bool, vector: bool) -> Result<Vec<Value>> {
        let (m, n) = (a.rows, a.cols);
        let k = m.min(n);
        let (r_rows, qc) = if economy { (k, k) } else { (m, m) };
        let e_shape = if vector { (1, n) } else { (n, n) };
        let produced = match outputs {
            1 => m * n,
            2 => ComplexMatrix::checked_len(r_rows, n)?
                .saturating_add(ComplexMatrix::checked_len(m, qc)?),
            _ => ComplexMatrix::checked_len(r_rows, n)?
                .saturating_add(ComplexMatrix::checked_len(m, qc)?)
                .saturating_add(Matrix::checked_len(e_shape.0, e_shape.1)?),
        };
        self.cxl_bounds(m, n.max(1), produced)?;
        let qr = self.cxl_geqp3(a, outputs == 3)?;
        if outputs == 1 {
            return Ok(vec![self.cxl_qr_value(qr.h)?]);
        }
        let q = self.cxl_q(&qr, qc)?;
        let mut r = CMat::zeros(r_rows, n);
        for j in 0..n {
            for i in 0..r_rows.min(j + 1) {
                r.set(i, j, qr.h.at(i, j));
            }
        }
        let mut out = vec![self.cxl_qr_value(q)?, self.cxl_qr_value(r)?];
        if outputs == 3 {
            out.push(Value::Matrix(if vector {
                let v = qr.jpvt.iter().map(|&j| j as f64 + 1.).collect();
                Matrix::from_column_vec(1, n, v)?
            } else {
                let mut e = vec![0.; n * n];
                for (c, &j) in qr.jpvt.iter().enumerate() {
                    e[c * n + j] = 1.;
                }
                Matrix::from_column_vec(n, n, e)?
            }));
        }
        Ok(out)
    }

    /// LAPACK zpotf2 on the upper triangle ('lower' reads the lower triangle
    /// through its conjugate transpose), as the real `linalg2_chol`.
    fn cxl_chol(&self, a: CMat, outputs: usize, lower: bool) -> Result<Vec<Value>> {
        if a.rows != a.cols {
            return Err("Matrix must be square.".into());
        }
        let n = a.rows;
        self.cxl_bounds(n, n, n * n)?;
        let s = if lower { a.adjoint() } else { a };
        if (0..n).any(|j| s.at(j, j).im != 0.) {
            return Err("Matrix must be positive definite with real diagonal.".into());
        }
        let mut r = CMat::zeros(n, n);
        let mut fail = 0;
        for j in 0..n {
            self.check_cancel()?;
            let dot: f64 = (0..j).map(|i| (r.at(i, j).conj() * r.at(i, j)).re).sum();
            let ajj = s.at(j, j).re - dot;
            if ajj.is_nan() || ajj <= 0. {
                fail = j + 1;
                break;
            }
            let ajj = ajj.sqrt();
            r.set(j, j, Complex64::new(ajj, 0.));
            let scale = 1. / ajj;
            for c in j + 1..n {
                let t: Complex64 = (0..j).map(|i| r.at(i, c) * r.at(i, j).conj()).sum();
                r.set(j, c, (s.at(j, c) - t) * scale);
            }
        }
        if fail > 0 && outputs < 2 {
            return Err("Matrix must be positive definite.".into());
        }
        let size = if fail > 0 { fail - 1 } else { n };
        let mut f = CMat::zeros(size, size);
        for j in 0..size {
            for i in 0..=j {
                f.set(i, j, r.at(i, j));
            }
        }
        if lower {
            f = f.adjoint();
        }
        let mut v = vec![self.cxl_qr_value(f)?];
        if outputs == 2 {
            v.push(Value::Matrix(Matrix::scalar(fail as f64)));
        }
        Ok(v)
    }

    /// The complex copy of `lax_least_squares`: pivoted QR (economy, three
    /// outputs), rank r from |R(i,i)| > tol, and the basic solution
    /// x(p(1:r),:) = R(1:r,1:r) \ (Q(:,1:r)'*b) with the other rows zero.
    fn cxl_least_squares(&self, a: CMat, b: &CMat) -> Result<(CMat, usize, f64)> {
        let (m, n) = (a.rows, a.cols);
        let mut x = CMat::zeros(n, b.cols);
        if m == 0 || n == 0 {
            return Ok((x, 0, 0.));
        }
        let qr = self.cxl_geqp3(a, true)?;
        let k = m.min(n);
        let r0 = cabs(qr.h.at(0, 0));
        // The reference's complex rank tolerance is ten times the real one
        // (cplx1 case 48: tol = 2.492445e-14 for |R(1,1)| = sqrt(14), 3x2).
        let tol = 10. * m.max(n) as f64 * f64::EPSILON * r0;
        let rank = (0..k).take_while(|&i| cabs(qr.h.at(i, i)) > tol).count();
        if rank > 0 {
            // Q'*b by the reflectors (zunm2r), as the reference's solve applies them
            // (cplx1 case 39 differs in the last digit with an explicit Q).
            let mut qtb = b.clone();
            for i in 0..k {
                self.check_cancel()?;
                let v: Vec<Complex64> = std::iter::once(Complex64::new(1., 0.))
                    .chain(qr.h.data[i * m + i + 1..(i + 1) * m].iter().copied())
                    .collect();
                zlarf_left(&mut qtb, &v, qr.tau[i].conj(), i, 0, b.cols);
            }
            let qtb = DMatrix::from_fn(rank, b.cols, |i, j| qtb.at(i, j));
            let r11 = DMatrix::from_fn(rank, rank, |i, j| {
                if i <= j {
                    qr.h.at(i, j)
                } else {
                    Complex64::new(0., 0.)
                }
            });
            let y = r11
                .solve_upper_triangular(&qtb)
                .ok_or("Singular triangular factor.")?;
            for (i, &p) in qr.jpvt.iter().take(rank).enumerate() {
                for c in 0..b.cols {
                    x.set(p, c, y[(i, c)]);
                }
            }
        }
        Ok((x, rank, tol))
    }

    /// `a \ b` or `a / b` (operator or mldivide/mrdivide) when one operand is
    /// complex and the divisor is a non-square, non-scalar matrix of agreeing
    /// size: least squares with mldivide's rank warning. None leaves the
    /// operands to the existing paths (scalar divisors, errors).
    pub(super) fn cxl_rect_solve(
        &mut self,
        backslash: bool,
        a: &Value,
        b: &Value,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let numeric = |v: &Value| -> Option<Array> {
            let a = v
                .clone()
                .numeric_operand(&self.cancel)
                .ok()?
                .array(&self.cancel)
                .ok()?;
            matches!(a, Array::Complex(_) | Array::Numeric(_) | Array::Logical(_)).then_some(a)
        };
        let (Some(a), Some(b)) = (numeric(a), numeric(b)) else {
            return Ok(None);
        };
        let divisor = if backslash { &a } else { &b };
        let agree = if backslash {
            a.rows() == b.rows()
        } else {
            a.cols() == b.cols()
        };
        if divisor.len() == 1 || divisor.rows() == divisor.cols() || !agree {
            return Ok(None);
        }
        let (m, n, p) = if backslash {
            (a.rows(), a.cols(), b.cols())
        } else {
            (b.cols(), b.rows(), a.rows())
        };
        self.cxl_bounds(
            m,
            n,
            ComplexMatrix::checked_len(m.max(n), p)?.saturating_mul(3),
        )?;
        let (lhs, rhs) = if backslash {
            (cxl_cmat(&a), cxl_cmat(&b))
        } else {
            (cxl_transpose(&cxl_cmat(&b)), cxl_transpose(&cxl_cmat(&a)))
        };
        let (x, rank, tol) = self.cxl_least_squares(lhs, &rhs)?;
        if rank < m.min(n) {
            self.emit(
                out,
                format!(
                    "Warning: Rank deficient, rank = {rank}, tol = {:>13}.",
                    crate::display_format::exponent(tol, 6)
                ),
            )?;
        }
        let x = if backslash { x } else { cxl_transpose(&x) };
        self.cxl_qr_value(x).map(Some)
    }
}
