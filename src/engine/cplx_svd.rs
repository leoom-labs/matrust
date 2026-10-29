//! Complex svd family (svd, norm, rank, cond, pinv, null, orth) and expm.
//! Every private method is prefixed `cxl_` (all `impl Engine` blocks share one
//! method namespace).
use super::{Engine, Value};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use nalgebra::{
    DMatrix,
    linalg::{QR, SVD},
};

const BUDGET: usize = 32 * 1024 * 1024;

// ---- svd family and expm (cplx1d) ----------------------------------------
// Mirrors svd_call/rank_call (linear_algebra.rs) and linalg2_svd, linalg2_pinv,
// linalg2_basis, linalg2_cond, linalg2_norm and linalg2_expm (linear_algebra2.rs)
// over DMatrix<Complex64> with nalgebra's SVD, LU and products.

type CxlSvd = SVD<Complex64, nalgebra::Dyn, nalgebra::Dyn>;

/// Bound on m*n*min(m,n), as linear_algebra2: about a 1600-square factorization.
const CXL_MAX_WORK: f64 = 4.3e9;
const CXL_SVD_NONFINITE: &str = "Input to SVD must not contain NaN or Inf.";
const CXL_MATRIX_NORMS: &str = "The only matrix norms available are 1, 2, inf, and 'fro'.";

#[derive(Clone, Copy)]
enum CxlNorm {
    P(f64),
    Fro,
}

/// MATLAB `eps(x)`.
fn cxl_eps(x: f64) -> f64 {
    let x = x.abs();
    f64::from_bits(x.to_bits() + 1) - x
}

fn cxl_abs(z: Complex64) -> f64 {
    libm::hypot(z.re, z.im)
}

fn cxl_finite(d: &DMatrix<Complex64>) -> bool {
    d.iter().all(|z| z.re.is_finite() && z.im.is_finite())
}

fn cxl_has_nan(d: &[Complex64]) -> bool {
    d.iter().any(|z| z.re.is_nan() || z.im.is_nan())
}

/// The real nrm2 of linear_algebra2 over the 2n real components (re, im, re, ...).
fn cxl_nrm2(d: &[Complex64]) -> f64 {
    let scale = d
        .iter()
        .fold(0.0f64, |m, z| m.max(z.re.abs()).max(z.im.abs()));
    if scale == 0.0 || !scale.is_finite() {
        return if cxl_has_nan(d) { f64::NAN } else { scale };
    }
    scale
        * d.iter()
            .flat_map(|z| [z.re, z.im])
            .map(|v| (v / scale).powi(2))
            .sum::<f64>()
            .sqrt()
}

/// linear_algebra2's scaled_p_norm on the magnitudes |z|.
fn cxl_p_norm(d: &[f64], p: f64) -> f64 {
    if p == 0.0 {
        return (d.len() as f64).powf(1.0 / p);
    }
    if p.is_nan() || d.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    let scale = if p < 0.0 {
        d.iter().fold(f64::INFINITY, |m, v| m.min(*v))
    } else {
        d.iter().fold(0.0_f64, |m, v| m.max(*v))
    };
    if scale == 0.0 || scale.is_infinite() {
        return scale;
    }
    scale
        * d.iter()
            .map(|v| (v / scale).powf(p))
            .sum::<f64>()
            .powf(1.0 / p)
}

fn cxl_norm1(d: &DMatrix<Complex64>) -> f64 {
    d.column_iter()
        .map(|c| c.iter().map(|z| cxl_abs(*z)).sum::<f64>())
        .fold(0.0, |a, b| if b.is_nan() || b > a { b } else { a })
}

fn cxl_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.to_lowercase()),
        _ => None,
    }
}

impl Engine {
    fn cxl_svd_dense(&self, name: &str, v: &Value) -> Result<DMatrix<Complex64>> {
        match v {
            Value::Complex(z) => Ok(DMatrix::from_column_slice(z.rows(), z.cols(), z.data())),
            _ => {
                let m = self.real_arg(name, v)?;
                Ok(DMatrix::from_iterator(
                    m.rows(),
                    m.cols(),
                    m.data().iter().map(|&x| Complex64::new(x, 0.)),
                ))
            }
        }
    }

    /// A complex result; all-zero imaginary parts (and empties) narrow to real.
    fn cxl_svd_value(&self, d: &DMatrix<Complex64>) -> Result<Value> {
        if d.iter().all(|z| z.im == 0.) {
            let re: Vec<f64> = d.iter().map(|z| z.re).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(
                d.nrows(),
                d.ncols(),
                re,
            )?));
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            d.nrows(),
            d.ncols(),
            d.as_slice().to_vec(),
            &self.cancel,
        )?))
    }

    fn cxl_svd_work(&self, m: usize, n: usize) -> Result<()> {
        if (m as f64) * (n as f64) * (m.min(n) as f64) > CXL_MAX_WORK {
            return Err("The factorization exceeds the dense work limit of this version".into());
        }
        Ok(())
    }

    /// Complex result shapes (two units per element) against the statement budget.
    fn cxl_svd_reserve(&self, shapes: &[(usize, usize)]) -> Result<()> {
        let mut total = self.evaluated_elements;
        for &(rows, cols) in shapes {
            total = total.saturating_add(ComplexMatrix::checked_len(rows, cols)?.saturating_mul(2));
        }
        if total > BUDGET {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    /// linalg2_svd: finite check, work bound, singular values in descending order.
    fn cxl_svd(&self, a: &DMatrix<Complex64>, vectors: bool) -> Result<CxlSvd> {
        if !cxl_finite(a) {
            return Err(CXL_SVD_NONFINITE.into());
        }
        self.cxl_svd_work(a.nrows(), a.ncols())?;
        self.check_cancel()?;
        // nalgebra scales a complex matrix by max(|re|+|im|), which overflows for
        // finite input near realmax. Scale by the power of two of the largest
        // component first (exact: same digits as the unscaled run) and undo it
        // on the singular values; U and V do not change.
        let largest = a
            .iter()
            .fold(0.0f64, |m, z| m.max(z.re.abs()).max(z.im.abs()));
        let exponent = if largest > 0. {
            libm::ilogb(largest)
        } else {
            0
        };
        let mut scaled = a.clone();
        if exponent != 0 {
            for z in scaled.iter_mut() {
                *z = Complex64::new(libm::scalbn(z.re, -exponent), libm::scalbn(z.im, -exponent));
            }
        }
        // try_new sorts inside and panics on a NaN singular value: sort here.
        let mut svd = SVD::try_new_unordered(scaled, vectors, vectors, f64::EPSILON, 0)
            .ok_or("SVD did not converge.")?;
        if svd.singular_values.iter().any(|s| s.is_nan()) {
            return Err("SVD did not converge.".into());
        }
        svd.sort_by_singular_values();
        if exponent != 0 {
            for s in svd.singular_values.iter_mut() {
                *s = libm::scalbn(*s, exponent);
            }
        }
        self.check_cancel()?;
        Ok(svd)
    }

    fn cxl_diagonal(&self, values: &[f64], rows: usize, cols: usize) -> Result<Matrix> {
        let mut data = vec![0.; Matrix::checked_len(rows, cols)?];
        for (i, &v) in values.iter().enumerate() {
            data[i * rows + i] = v;
        }
        Matrix::from_column_vec(rows, cols, data)
    }

    /// complete_basis over C64: the Householder reflectors of Q's QR applied to
    /// the identity give Q^H of a full unitary factor; its adjoint keeps Q's
    /// columns first.
    fn cxl_complete_basis(&self, q: &DMatrix<Complex64>) -> Result<DMatrix<Complex64>> {
        let (n, k) = (q.nrows(), q.ncols());
        if k == n {
            return Ok(q.clone());
        }
        let mut full = DMatrix::identity(n, n);
        QR::new(q.clone()).q_tr_mul(&mut full);
        self.check_cancel()?;
        let mut full = full.adjoint();
        full.columns_mut(0, k).copy_from(q);
        Ok(full)
    }

    /// svd with a complex argument, after svd_call's argument rules.
    pub(super) fn cxl_svd_call(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() || args.len() > 2 {
            return Err("svd expects 1 or 2 arguments".into());
        }
        if count == 2 || count > 3 {
            return Err(format!("svd does not support {count} outputs; use 1 or 3"));
        }
        let a = self.cxl_svd_dense("svd", &args[0])?;
        if !cxl_finite(&a) {
            return Err(CXL_SVD_NONFINITE.into());
        }
        let (m, n) = a.shape();
        let k = m.min(n);
        // 'econ' or 0: economy size (svd(A,0) is the full form when m <= n).
        let economy = match args.get(1) {
            None => false,
            Some(Value::Matrix(z)) if z.len() == 1 && z.data()[0] == 0. => m > n,
            Some(v) => match cxl_text(v).as_deref() {
                Some("econ") => true,
                _ => return Err("svd: only the 'econ' and 0 options are supported yet".into()),
            },
        };
        let want_vectors = count == 3;
        if want_vectors {
            let (u_cols, s_rows, s_cols, v_cols) =
                if economy { (k, k, k, k) } else { (m, m, n, n) };
            self.cxl_svd_reserve(&[(m, u_cols), (n, v_cols)])?;
            self.preflight_elements(Matrix::checked_len(s_rows, s_cols)?)?;
        }
        let (u, s, v) = if k == 0 {
            (DMatrix::identity(m, k), Vec::new(), DMatrix::identity(n, k))
        } else {
            let svd = self.cxl_svd(&a, want_vectors)?;
            let s: Vec<f64> = svd.singular_values.iter().copied().collect();
            if !want_vectors {
                return Ok(vec![Value::Matrix(Matrix::from_column_vec(k, 1, s)?)]);
            }
            let u = svd.u.ok_or("svd: left vectors were not computed")?;
            let vt = svd.v_t.ok_or("svd: right vectors were not computed")?;
            (u, s, vt.adjoint())
        };
        if !want_vectors {
            return Ok(vec![Value::Matrix(Matrix::from_column_vec(k, 1, s)?)]);
        }
        let (u, v, s_rows, s_cols) = if economy {
            (u, v, k, k)
        } else {
            (
                self.cxl_complete_basis(&u)?,
                self.cxl_complete_basis(&v)?,
                m,
                n,
            )
        };
        Ok(vec![
            self.cxl_svd_value(&u)?,
            Value::Matrix(self.cxl_diagonal(&s, s_rows, s_cols)?),
            self.cxl_svd_value(&v)?,
        ])
    }

    /// rank with a complex argument, as rank_call.
    pub(super) fn cxl_rank_call(&mut self, args: &[Value]) -> Result<Value> {
        Self::arity_check("rank", args, &[1, 2])?;
        let a = self.cxl_svd_dense("rank", &args[0])?;
        if !cxl_finite(&a) {
            return Err("rank: Input must not contain NaN or Inf.".into());
        }
        let tolerance = match args.get(1) {
            Some(v) => Some(
                self.real_arg("rank", v)?
                    .as_scalar()
                    .map_err(|_| "rank: the tolerance must be a scalar".to_string())?,
            ),
            None => None,
        };
        if a.is_empty() {
            return Ok(Value::Matrix(Matrix::scalar(0.)));
        }
        let svd = self.cxl_svd(&a, false)?;
        let largest = svd.singular_values.iter().copied().fold(0., f64::max);
        let tolerance =
            tolerance.unwrap_or_else(|| a.nrows().max(a.ncols()) as f64 * cxl_eps(largest));
        let rank = svd
            .singular_values
            .iter()
            .filter(|&&s| s > tolerance)
            .count();
        Ok(Value::Matrix(Matrix::scalar(rank as f64)))
    }

    /// pinv, null, orth, expm, cond and norm with a complex first argument,
    /// after linalg2_call's argument and output counts.
    pub(super) fn cxl_linalg2_call(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let a = self.cxl_svd_dense(name, &args[0])?;
        match name {
            "pinv" => {
                let tol = match args.get(1) {
                    None => None,
                    Some(v) => {
                        let t = self.real_arg(name, v)?;
                        if t.len() != 1 {
                            return Err("Tolerance must be a scalar.".into());
                        }
                        Some(t.data()[0])
                    }
                };
                self.cxl_pinv(&a, tol)
            }
            "null" | "orth" => self.cxl_basis(&a, name == "null"),
            "expm" => self.cxl_expm(&a),
            "cond" => {
                let p = self.cxl_norm_kind(name, args.get(1))?;
                Ok(Value::Matrix(Matrix::scalar(self.cxl_cond(&a, p)?)))
            }
            _ => self.cxl_norm_call(args),
        }
    }

    /// norm(Z) or norm(Z, p) with complex Z.
    pub(super) fn cxl_norm_call(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let a = self.cxl_svd_dense("norm", &args[0])?;
        let p = self.cxl_norm_kind("norm", args.get(1))?;
        Ok(Value::Matrix(Matrix::scalar(self.cxl_norm(&a, p)?)))
    }

    fn cxl_norm_kind(&self, name: &str, v: Option<&Value>) -> Result<CxlNorm> {
        let Some(v) = v else {
            return Ok(CxlNorm::P(2.0));
        };
        let text = match v {
            Value::Container(Array::Character(a)) => Some(a.to_utf8(&self.cancel)?.to_lowercase()),
            _ => cxl_text(v),
        };
        if let Some(s) = text {
            return match s.as_str() {
                "fro" => Ok(CxlNorm::Fro),
                "inf" => Ok(CxlNorm::P(f64::INFINITY)),
                _ => Err(CXL_MATRIX_NORMS.into()),
            };
        }
        let p = self.real_arg(name, v)?;
        if p.len() != 1 {
            return Err("Unknown norm option: p must be a scalar.".into());
        }
        Ok(CxlNorm::P(p.data()[0]))
    }

    fn cxl_norm(&self, a: &DMatrix<Complex64>, p: CxlNorm) -> Result<f64> {
        if a.is_empty() {
            return Ok(0.0);
        }
        let d = a.as_slice();
        if a.nrows() == 1 || a.ncols() == 1 {
            let abs = || d.iter().map(|z| cxl_abs(*z));
            return Ok(match p {
                CxlNorm::Fro | CxlNorm::P(2.0) => cxl_nrm2(d),
                CxlNorm::P(x) if x.is_infinite() && cxl_has_nan(d) => f64::NAN,
                CxlNorm::P(x) if x == f64::INFINITY => abs().fold(0.0, f64::max),
                CxlNorm::P(x) if x == f64::NEG_INFINITY => abs().fold(f64::INFINITY, f64::min),
                CxlNorm::P(1.0) => abs().sum(),
                CxlNorm::P(x) => {
                    let magnitudes: Vec<f64> = abs().collect();
                    cxl_p_norm(&magnitudes, x)
                }
            });
        }
        if cxl_has_nan(d) {
            return Ok(f64::NAN);
        }
        Ok(match p {
            CxlNorm::Fro => cxl_nrm2(d),
            CxlNorm::P(1.0) => cxl_norm1(a),
            CxlNorm::P(x) if x == f64::INFINITY => a
                .row_iter()
                .map(|r| r.iter().map(|z| cxl_abs(*z)).sum::<f64>())
                .fold(0.0, f64::max),
            CxlNorm::P(2.0) => {
                if !cxl_finite(a) {
                    return Ok(f64::INFINITY);
                }
                let svd = self.cxl_svd(a, false)?;
                svd.singular_values.iter().copied().fold(0.0, f64::max)
            }
            CxlNorm::P(_) => return Err(CXL_MATRIX_NORMS.into()),
        })
    }

    fn cxl_cond(&self, a: &DMatrix<Complex64>, p: CxlNorm) -> Result<f64> {
        if a.is_empty() {
            return Ok(0.0);
        }
        if let CxlNorm::P(x) = p
            && x == 2.0
        {
            let svd = self.cxl_svd(a, false)?;
            let s = &svd.singular_values;
            let (hi, lo) = (
                s.iter().copied().fold(0.0, f64::max),
                s.iter().copied().fold(f64::INFINITY, f64::min),
            );
            return Ok(if lo == 0.0 { f64::INFINITY } else { hi / lo });
        }
        if let CxlNorm::P(x) = p
            && !(x == 1.0 || x == f64::INFINITY)
        {
            return Err(CXL_MATRIX_NORMS.into());
        }
        let n = a.nrows();
        if n != a.ncols() {
            return Err("A is rectangular.  Use the 2 norm.".into());
        }
        if !cxl_finite(a) {
            return Ok(f64::NAN);
        }
        // norm(A,p)*norm(inv(A),p); the inverse comes from an LU solve against the identity.
        self.cxl_svd_work(n, n)?;
        self.cxl_svd_reserve(&[(n, n), (n, n)])?;
        let inverse = match a.clone().lu().solve(&DMatrix::identity(n, n)) {
            Some(i) => i,
            None => return Ok(f64::INFINITY),
        };
        self.check_cancel()?;
        Ok(self.cxl_norm(a, p)? * self.cxl_norm(&inverse, p)?)
    }

    fn cxl_pinv(&self, a: &DMatrix<Complex64>, tol: Option<f64>) -> Result<Value> {
        let (m, n) = a.shape();
        if m == 0 || n == 0 {
            return Ok(Value::Matrix(Matrix::filled(n, m, 0.0)?));
        }
        self.cxl_svd_reserve(&[(m, n), (n, m)])?;
        let svd = self.cxl_svd(a, true)?;
        let s = &svd.singular_values;
        let smax = s.iter().copied().fold(0.0, f64::max);
        let tol = tol.unwrap_or(m.max(n) as f64 * cxl_eps(smax));
        let (u, vt) = (svd.u.as_ref().unwrap(), svd.v_t.as_ref().unwrap());
        let r = s.iter().filter(|&&x| x > tol).count();
        if r == 0 {
            return Ok(Value::Matrix(Matrix::filled(n, m, 0.0)?));
        }
        let vs = DMatrix::from_fn(n, r, |i, j| vt[(j, i)].conj() / s[j]);
        self.check_cancel()?;
        self.cxl_svd_value(&(vs * u.columns(0, r).adjoint()))
    }

    fn cxl_basis(&self, a: &DMatrix<Complex64>, null: bool) -> Result<Value> {
        let (m, n) = a.shape();
        if m == 0 || n == 0 {
            return Ok(Value::Matrix(if null {
                Matrix::identity(n, n)?
            } else {
                Matrix::filled(if n == 0 { m } else { 0 }, 0, 0.0)?
            }));
        }
        // Pad wide matrices with zero rows so that V is the full n-by-n factor.
        let rows = m.max(n);
        self.cxl_svd_work(rows, n)?;
        self.cxl_svd_reserve(&[(rows, n), (rows, n), (n, n), (n, n)])?;
        let mut padded = a.clone();
        if m < n {
            padded = padded.insert_rows(m, n - m, Complex64::new(0., 0.));
        }
        let svd = self.cxl_svd(&padded, true)?;
        let s = &svd.singular_values;
        let smax = s.iter().copied().fold(0.0, f64::max);
        // rows * (smax * eps): rows * smax overflows for singular values near realmax.
        let tol = rows as f64 * (smax * f64::EPSILON);
        let r = s.iter().filter(|&&x| x > tol).count();
        if null {
            let vt = svd.v_t.as_ref().unwrap();
            let snap = |x: f64| if x.abs() < f64::EPSILON { 0.0 } else { x };
            let z = DMatrix::from_fn(n, n - r, |i, j| {
                let x = vt[(r + j, i)].conj();
                Complex64::new(snap(x.re), snap(x.im))
            });
            self.cxl_svd_value(&z)
        } else {
            let u = svd.u.as_ref().unwrap();
            self.cxl_svd_value(
                &u.columns(0, r)
                    .into_owned()
                    .resize_vertically(m, Complex64::new(0., 0.)),
            )
        }
    }

    fn cxl_gemm(
        &self,
        a: &DMatrix<Complex64>,
        b: &DMatrix<Complex64>,
    ) -> Result<DMatrix<Complex64>> {
        self.check_cancel()?;
        Ok(a * b)
    }

    /// linalg2_expm over C64: the same Pade orders, THETA bounds and B
    /// coefficients, scaling and squaring, LU solve of the Pade quotient.
    fn cxl_expm(&self, a: &DMatrix<Complex64>) -> Result<Value> {
        let n = a.nrows();
        if n != a.ncols() {
            return Err("Expected input to be a square matrix.".into());
        }
        if n == 0 {
            return Ok(Value::Matrix(Matrix::filled(0, 0, 0.)?));
        }
        if n == 1 {
            let z = a[(0, 0)];
            let r = libm::exp(z.re);
            let e = if z.im == 0. {
                Complex64::new(r, 0.)
            } else {
                Complex64::new(r * libm::cos(z.im), r * libm::sin(z.im))
            };
            return self.cxl_svd_value(&DMatrix::from_element(1, 1, e));
        }
        self.cxl_svd_work(n, n)?;
        self.cxl_svd_reserve(&[(n, n); 8])?;
        let nrm = cxl_norm1(a);
        if !nrm.is_finite() {
            return Ok(Value::Matrix(Matrix::filled(n, n, f64::NAN)?));
        }
        const THETA: [(usize, f64); 5] = [
            (3, 1.495585217958292e-2),
            (5, 2.53939833006323e-1),
            (7, 9.504178996162932e-1),
            (9, 2.097847961257068e0),
            (13, 5.371920351148152e0),
        ];
        let (order, scale) = match THETA.iter().find(|(_, t)| nrm <= *t) {
            Some((m, _)) => (*m, 0),
            None => (13, ((nrm / THETA[4].1).log2().ceil() as i32).clamp(0, 1100)),
        };
        let a = if scale > 0 {
            let f = 2f64.powi(scale);
            a.map(|z| z.unscale(f))
        } else {
            a.clone()
        };
        let times = |m: &DMatrix<Complex64>, b: f64| m.map(|z| z.scale(b));
        let id = DMatrix::<Complex64>::identity(n, n);
        let (u, v) = if order == 13 {
            const B: [f64; 14] = [
                64764752532480000.,
                32382376266240000.,
                7771770303897600.,
                1187353796428800.,
                129060195264000.,
                10559470521600.,
                670442572800.,
                33522128640.,
                1323241920.,
                40840800.,
                960960.,
                16380.,
                182.,
                1.,
            ];
            let a2 = self.cxl_gemm(&a, &a)?;
            let a4 = self.cxl_gemm(&a2, &a2)?;
            let a6 = self.cxl_gemm(&a2, &a4)?;
            let t1 = times(&a6, B[13]) + times(&a4, B[11]) + times(&a2, B[9]);
            let u0 = self.cxl_gemm(&a6, &t1)?
                + times(&a6, B[7])
                + times(&a4, B[5])
                + times(&a2, B[3])
                + times(&id, B[1]);
            let u = self.cxl_gemm(&a, &u0)?;
            let t2 = times(&a6, B[12]) + times(&a4, B[10]) + times(&a2, B[8]);
            let v = self.cxl_gemm(&a6, &t2)?
                + times(&a6, B[6])
                + times(&a4, B[4])
                + times(&a2, B[2])
                + times(&id, B[0]);
            (u, v)
        } else {
            let b: &[f64] = match order {
                3 => &[120., 60., 12., 1.],
                5 => &[30240., 15120., 3360., 420., 30., 1.],
                7 => &[
                    17297280., 8648640., 1995840., 277200., 25200., 1512., 56., 1.,
                ],
                _ => &[
                    17643225600.,
                    8821612800.,
                    2075673600.,
                    302702400.,
                    30270240.,
                    2162160.,
                    110880.,
                    3960.,
                    90.,
                    1.,
                ],
            };
            let a2 = self.cxl_gemm(&a, &a)?;
            let mut pows = vec![id.clone(), a2.clone()];
            for _ in 2..=order / 2 {
                let next = self.cxl_gemm(pows.last().unwrap(), &a2)?;
                pows.push(next);
            }
            let mut u0 = DMatrix::zeros(n, n);
            let mut v = DMatrix::zeros(n, n);
            for (j, p) in pows.iter().enumerate() {
                v += times(p, b[2 * j]);
                if 2 * j + 1 < b.len() {
                    u0 += times(p, b[2 * j + 1]);
                }
            }
            (self.cxl_gemm(&a, &u0)?, v)
        };
        let mut r = (&v - &u)
            .lu()
            .solve(&(&v + &u))
            .ok_or("expm: Pade denominator is singular")?;
        for _ in 0..scale {
            r = self.cxl_gemm(&r, &r)?;
        }
        self.cxl_svd_value(&r)
    }
}
