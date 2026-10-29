//! Additional linear algebra and piecewise polynomial functions.
//! Dense factorizations use nalgebra; products use the existing bounded GEMM.
use super::{Engine, Execution, Value};
use crate::{Array, CharacterRow, Complex64 as C64, ComplexMatrix, Matrix, Result, StructArray};
use nalgebra::{DMatrix, linalg::Schur};

const NAMES: &[&str] = &[
    "rref",
    "hess",
    "schur",
    "ldl",
    "linsolve",
    "lscov",
    "lsqnonneg",
    "rcond",
    "condest",
    "normest",
    "subspace",
    "sylvester",
    "polyvalm",
    "balance",
    "planerot",
    "mkpp",
    "unmkpp",
    "ppval",
    "residue",
    "blkdiag",
    "cdf2rdf",
    "rsf2csf",
];
pub(super) fn lax_handles(name: &str) -> bool {
    NAMES.contains(&name)
}
fn lax_dm(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}
fn lax_mx(m: &DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_slice(m.nrows(), m.ncols(), m.as_slice())
}
fn lax_val(m: &DMatrix<f64>) -> Result<Value> {
    Ok(Value::Matrix(lax_mx(m)?))
}
fn lax_scalar(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}
fn lax_square(m: &Matrix, message: &str) -> Result<usize> {
    if m.rows() != m.cols() {
        Err(message.into())
    } else {
        Ok(m.rows())
    }
}
/// dgecon's RCOND from norm(A,1) and the estimate of norm(inv(A),1).
fn lax_rcond(norm: f64, inverse_norm: Option<f64>) -> f64 {
    match inverse_norm {
        Some(e) if norm != 0. && e != 0. => (1. / e) / norm,
        _ => 0.,
    }
}
fn lax_close_message(rcond: f64) -> String {
    format!(
        "Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = {}.",
        crate::display_format::exponent(rcond, 6)
    )
}
fn lax_norm1(a: &DMatrix<f64>) -> f64 {
    a.column_iter()
        .map(|c| c.iter().map(|v| v.abs()).sum::<f64>())
        .fold(0., f64::max)
}
/// The first index of the largest key: IDAMAX and MATLAB `max` keep the first tie.
fn lax_first_max(
    indices: impl Iterator<Item = usize>,
    key: impl Fn(usize) -> f64,
) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for i in indices {
        let value = key(i);
        if best.is_none_or(|(_, b)| value > b) {
            best = Some((i, value));
        }
    }
    best.map(|(i, _)| i)
}
fn lax_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(s)) => Some(String::from_utf16_lossy(s.units())),
        _ => None,
    }
}

/// A pivoted-QR least-squares solution and the factors lscov reuses.
struct LaxLeastSquares {
    x: DMatrix<f64>,
    rank: usize,
    tol: f64,
    /// The leading rank-by-rank block of R.
    r: DMatrix<f64>,
    /// Zero-based column permutation of the QR factorization.
    perm: Vec<usize>,
}

impl Engine {
    pub(super) fn lax_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        Ok(self
            .lax_outputs(name, args, 1, out)?
            .map(|mut v| v.remove(0)))
    }
    pub(super) fn lax_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        if !lax_handles(name) {
            return Ok(None);
        }
        self.check_cancel()?;
        let (min, max, outputs) = match name {
            "blkdiag" => (0, usize::MAX, 1),
            "rref" | "schur" | "normest" | "condest" => (1, 2, 2),
            "hess" | "planerot" => (1, 1, 2),
            "balance" | "ldl" => (1, 2, 3),
            "linsolve" => (2, 3, 2),
            "lscov" => (2, 4, 4),
            "lsqnonneg" => (2, 3, 6),
            "mkpp" | "residue" => (
                2,
                3,
                if name == "mkpp" {
                    1
                } else if args.len() == 3 {
                    2
                } else {
                    3
                },
            ),
            "unmkpp" => (1, 1, 5),
            "rcond" => (1, 1, 1),
            "sylvester" => (3, 3, 1),
            "cdf2rdf" | "rsf2csf" => (2, 2, 2),
            _ => (2, 2, 1),
        };
        if args.len() < min {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > max {
            return Err("Too many input arguments.".into());
        }
        if count > outputs {
            return Err("Too many output arguments.".into());
        }
        let mut result = match name {
            "rref" => self.lax_rref(args, out)?,
            "hess" | "schur" => self.lax_decompose(name, args, count)?,
            "ldl" => self.lax_ldl(args, count)?,
            "linsolve" => self.lax_linsolve(args, count, out)?,
            "lscov" => self.lax_lscov(args, count, out)?,
            "lsqnonneg" => self.lax_nnls(args, count, out)?,
            "rcond" | "condest" => self.lax_condition(name, args, count)?,
            "normest" => self.lax_normest(args, out)?,
            "subspace" => vec![self.lax_subspace(args)?],
            "sylvester" => vec![self.lax_sylvester(args)?],
            "polyvalm" => vec![self.lax_polyvalm(args)?],
            "balance" => self.lax_balance(args, count)?,
            "planerot" => self.lax_planerot(args)?,
            "mkpp" => vec![self.lax_mkpp(args)?],
            "unmkpp" => self.lax_unmkpp(&args[0])?,
            "ppval" => vec![self.lax_ppval(args)?],
            "residue" => self.lax_residue(args, out)?,
            "blkdiag" => vec![self.lax_blkdiag(args)?],
            "cdf2rdf" => self.lax_cdf2rdf(args)?,
            "rsf2csf" => self.lax_rsf2csf(args)?,
            _ => unreachable!(),
        };
        result.truncate(count);
        if name == "residue" && super::cplx_poly::complex_input(args) {
            result = self.cxp_narrow(args, result)?;
        }
        self.check_cancel()?;
        Ok(Some(result))
    }
    fn lax_real(&self, v: &Value) -> Result<Matrix> {
        match v {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(m) => m.to_numeric(&self.cancel),
            _ => Err("Inputs must be real double matrices.".into()),
        }
    }
    fn lax_complex(&self, v: &Value) -> Result<DMatrix<C64>> {
        match v {
            Value::Complex(m) => Ok(DMatrix::from_column_slice(m.rows(), m.cols(), m.data())),
            _ => Ok(lax_dm(&self.lax_real(v)?).map(|x| C64::new(x, 0.))),
        }
    }
    fn lax_cv(&self, a: &DMatrix<C64>, force: bool) -> Result<Value> {
        if !force && a.iter().all(|z| z.im == 0.) {
            return lax_val(&a.map(|z| z.re));
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            a.nrows(),
            a.ncols(),
            a.as_slice().to_vec(),
            &self.cancel,
        )?))
    }
    fn lax_budget(&self, rows: usize, cols: usize, copies: usize) -> Result<()> {
        let n = Matrix::checked_len(rows, cols)?;
        self.preflight_elements(n.checked_mul(copies).ok_or("Work array size overflow")?)?;
        self.check_cancel()
    }
    fn lax_work(&self, rows: usize, cols: usize) -> Result<()> {
        if rows as f64 * cols as f64 * rows.min(cols) as f64 > 4.3e9 {
            return Err("The factorization exceeds the dense work limit of this version".into());
        }
        self.lax_budget(rows, cols, 8)
    }
    fn lax_finite(&self, a: &Matrix) -> Result<()> {
        if a.data().iter().any(|x| !x.is_finite()) {
            Err("Input must not contain NaN or Inf.".into())
        } else {
            Ok(())
        }
    }
    /// Each entry rounded to the continued-fraction convergent that `rat`
    /// selects with its default tolerance (NIST DLMF §3.10), as num./den.
    fn lax_rational(&mut self, a: &Matrix, out: &mut Execution) -> Result<Matrix> {
        let parts = self.lang_utils_outputs("rat", &[Value::Matrix(a.clone())], 2, out)?;
        let num = self.lax_real(&parts[0])?;
        let den = self.lax_real(&parts[1])?;
        num.zip(&den, |n, d| n / d)
    }
    /// Gauss-Jordan elimination with partial pivoting (Golub & Van Loan,
    /// Matrix Computations, 4th ed., 2013, §3.2-3.4). A column whose largest
    /// remaining entry is at most tol (default max(m,n)*eps*norm(A,inf)) is
    /// negligible and zeroed below the current row. When every entry of A
    /// equals its continued-fraction approximation (ratios of small integers),
    /// the exact form is rational too and R is rounded to its convergents.
    fn lax_rref(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        let a = self.lax_real(&args[0])?;
        self.lax_finite(&a)?;
        let (m, n) = (a.rows(), a.cols());
        self.lax_work(m, n)?;
        let tol = match args.get(1) {
            Some(v) => {
                let t = self.lax_real(v)?;
                match t.data() {
                    &[t] if t.is_finite() && t >= 0. => t,
                    _ => return Err("Tolerance must be a finite nonnegative scalar.".into()),
                }
            }
            None => {
                let mut sums = vec![0.; m];
                for (i, v) in a.data().iter().enumerate() {
                    sums[i % m] += v.abs();
                }
                let norm = sums.into_iter().fold(0., f64::max);
                if norm.is_finite() {
                    m.max(n) as f64 * f64::EPSILON * norm
                } else {
                    // A row sum overflowed: sum the entries scaled by the
                    // largest one and apply the scale last, so a finite A
                    // never yields an infinite tolerance (which would zero
                    // every column).
                    let big = a.data().iter().fold(0., |x: f64, v| x.max(v.abs()));
                    let mut sums = vec![0.; m];
                    for (i, v) in a.data().iter().enumerate() {
                        sums[i % m] += v.abs() / big;
                    }
                    let scaled = sums.into_iter().fold(0., f64::max);
                    m.max(n) as f64 * f64::EPSILON * big * scaled
                }
            }
        };
        let rational = !a.is_empty() && self.lax_rational(&a, out)?.data() == a.data();
        let mut r = lax_dm(&a);
        let mut pivots = Vec::new();
        for j in 0..n {
            let i = pivots.len();
            if i == m {
                break;
            }
            self.check_cancel()?;
            let p = lax_first_max(i..m, |k| r[(k, j)].abs()).unwrap_or(i);
            if r[(p, j)].abs() <= tol {
                r.view_mut((i, j), (m - i, 1)).fill(0.);
                continue;
            }
            pivots.push(j);
            r.swap_rows(i, p);
            let pivot = r[(i, j)];
            let factors: Vec<f64> = r.column(j).iter().copied().collect();
            // Normalize the pivot row, then subtract its multiples from every
            // other row, one column of the trailing block at a time. (Columns
            // in parallel measured 3-5x slower at 600x600 with 4 workers.)
            let update = |column: &mut [f64]| {
                column[i] /= pivot;
                let v = column[i];
                for (k, x) in column.iter_mut().enumerate() {
                    if k != i {
                        *x -= factors[k] * v;
                    }
                }
            };
            r.as_mut_slice()[j * m..].chunks_mut(m).for_each(update);
        }
        let mut r = lax_mx(&r)?;
        if rational {
            r = self.lax_rational(&r, out)?;
        }
        let jb = if pivots.is_empty() {
            Matrix::from_column_vec(0, 0, Vec::new())?
        } else {
            let jb: Vec<f64> = pivots.iter().map(|&j| (j + 1) as f64).collect();
            Matrix::from_column_vec(1, jb.len(), jb)?
        };
        Ok(vec![Value::Matrix(r), Value::Matrix(jb)])
    }
    fn lax_decompose(&self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let a = self.lax_complex(&args[0])?;
        let n = a.nrows();
        if n != a.ncols() {
            return Err(if name == "hess" {
                "Inputs must be full, square matrices of type single or double."
            } else {
                "First input must be square."
            }
            .into());
        }
        self.lax_work(n, n)?;
        if a.iter().any(|x| !x.re.is_finite() || !x.im.is_finite()) {
            return Err("Input must not contain NaN or Inf.".into());
        }
        let complex = matches!(args[0], Value::Complex(_))
            || args
                .get(1)
                .and_then(lax_text)
                .is_some_and(|s| s == "complex");
        if let Some(v) = args.get(1) {
            if !matches!(lax_text(v).as_deref(), Some("real" | "complex")) {
                return Err("Unknown Schur option.".into());
            }
            if matches!(args[0], Value::Complex(_)) && lax_text(v).as_deref() == Some("real") {
                return Err("Real Schur form requires a real matrix.".into());
            }
        }
        let upper = (0..n).all(|j| (j + 1..n).all(|i| a[(i, j)] == C64::new(0., 0.)));
        if name == "hess" {
            return if complex {
                self.lax_complex_hessenberg(a, count)
            } else {
                self.lax_real_hessenberg(a.map(|z| z.re), count)
            };
        }
        let (q, t) = if n <= 1 || upper {
            (DMatrix::identity(n, n), a)
        } else if !complex {
            let (q, t) = self.lax_real_schur(a.map(|z| z.re))?;
            (q.map(|x| C64::new(x, 0.)), t.map(|x| C64::new(x, 0.)))
        } else if !matches!(args[0], Value::Complex(_)) {
            let real_args = [args[0].clone()];
            let real = self.lax_decompose("schur", &real_args, 2)?;
            return self.lax_rsf2csf(&real).map(|mut r| {
                if count == 1 {
                    r.remove(0);
                }
                r
            });
        } else {
            Schur::try_new(a, f64::EPSILON, 1000 * n + 10000)
                .ok_or("Schur decomposition did not converge.")?
                .unpack()
        };
        let t = self.lax_cv(&t, complex)?;
        Ok(if count == 1 {
            vec![t]
        } else {
            vec![self.lax_cv(&q, complex)?, t]
        })
    }
    /// Real Schur form of a matrix that is not upper triangular. A 2x2 input is
    /// reduced as LAPACK dgees/dlahqr do: dgebal('P') isolates a lower
    /// triangular one, otherwise a single dlanv2 rotation. Larger inputs use
    /// nalgebra's Francis iteration, then dlanv2 standardizes every 2x2 block
    /// left for a complex pair (equal diagonal, off-diagonal product < 0).
    fn lax_real_schur(&self, a: DMatrix<f64>) -> Result<(DMatrix<f64>, DMatrix<f64>)> {
        let n = a.nrows();
        if n == 2 {
            if a[(0, 1)] == 0. {
                let swap = DMatrix::from_column_slice(2, 2, &[0., 1., 1., 0.]);
                let t = DMatrix::from_column_slice(2, 2, &[a[(1, 1)], 0., a[(1, 0)], a[(0, 0)]]);
                return Ok((swap, t));
            }
            let (mut q, mut t) = (DMatrix::identity(2, 2), a);
            lax_standardize_block(&mut q, &mut t, 0);
            return Ok((q, t));
        }
        let limit = 1000 * n + 10000;
        let (mut q, mut t) = if let Some(schur) = Schur::try_new(a.clone(), f64::EPSILON, limit) {
            schur.unpack()
        } else {
            self.check_cancel()?;
            // Tight clusters of repeated real eigenvalues can stall at
            // epsilon. Retry with bounded deflation, then independently
            // check the backward error before accepting the factors.
            let (q, t) = Schur::try_new(a.clone(), 32. * f64::EPSILON, limit)
                .ok_or("Schur decomposition did not converge.")?
                .unpack();
            self.check_cancel()?;
            let reconstructed = &q * &t * q.transpose();
            let scale = a.iter().map(|x| x.abs()).fold(f64::MIN_POSITIVE, f64::max);
            let tolerance = 256. * f64::EPSILON * n as f64 * scale;
            if reconstructed
                .iter()
                .zip(a.iter())
                .any(|(&x, &y)| !x.is_finite() || (x - y).abs() > tolerance)
            {
                return Err("Schur decomposition failed its backward-error check.".into());
            }
            (q, t)
        };
        self.check_cancel()?;
        let mut j = 0;
        while j + 1 < n {
            if t[(j + 1, j)] == 0. {
                j += 1;
            } else {
                lax_standardize_block(&mut q, &mut t, j);
                j += 2;
            }
        }
        Ok((q, t))
    }
    /// LAPACK zgehrd/zunghr: each zlarfg reflector leaves a real subdiagonal
    /// entry, including the last one (a 1-element reflector removes its phase).
    /// O(n^3) rank-one updates; only the two-output form accumulates Q.
    fn lax_complex_hessenberg(&self, mut h: DMatrix<C64>, count: usize) -> Result<Vec<Value>> {
        let n = h.nrows();
        let mut q = (count == 2).then(|| DMatrix::<C64>::identity(n, n));
        for k in 0..n.saturating_sub(1) {
            self.check_cancel()?;
            let m = n - k - 1;
            let mut v: Vec<C64> = (k + 1..n).map(|i| h[(i, k)]).collect();
            let (tau, beta) = lax_zlarfg(&mut v);
            if tau == C64::new(0., 0.) {
                continue;
            }
            v[0] = C64::new(1., 0.);
            let v = nalgebra::DVector::from_vec(v);
            // Right: H(:,k+1:n) = H(:,k+1:n) * (I - tau v v').
            let w = h.columns(k + 1, m) * &v;
            h.columns_mut(k + 1, m).gerc(-tau, &w, &v, C64::new(1., 0.));
            // Left: H(k+1:n,k+1:n) = (I - conj(tau) v v') * H(k+1:n,k+1:n).
            let w = h.view((k + 1, k + 1), (m, m)).ad_mul(&v);
            h.view_mut((k + 1, k + 1), (m, m))
                .gerc(-tau.conj(), &v, &w, C64::new(1., 0.));
            h[(k + 1, k)] = C64::new(beta, 0.);
            for i in k + 2..n {
                h[(i, k)] = C64::new(0., 0.);
            }
            if let Some(q) = &mut q {
                let w = q.columns(k + 1, m) * &v;
                q.columns_mut(k + 1, m).gerc(-tau, &w, &v, C64::new(1., 0.));
            }
        }
        let mut result = Vec::new();
        if let Some(q) = q {
            result.push(self.lax_cv(&q, true)?);
        }
        result.push(self.lax_cv(&h, true)?);
        Ok(result)
    }
    /// Reuse packed QR reflectors, whose beta/tau convention matches LAPACK.
    /// Matrix-vector and rank-one updates are nalgebra kernels; O(n^3) work.
    /// Only the two-output form allocates/accumulates the orthogonal factor.
    fn lax_real_hessenberg(&self, mut h: DMatrix<f64>, count: usize) -> Result<Vec<Value>> {
        let n = h.nrows();
        let mut q = (count == 2).then(|| DMatrix::<f64>::identity(n, n));
        for k in 0..n.saturating_sub(2) {
            self.check_cancel()?;
            let m = n - k - 1;
            let column = Matrix::from_column_vec(m, 1, (k + 1..n).map(|i| h[(i, k)]).collect())?;
            let packed = self
                .linalg2_call("qr", &[Value::Matrix(column)], 1)?
                .unwrap();
            let packed = self.lax_real(&packed[0])?;
            let alpha = h[(k + 1, k)];
            let beta = packed.data()[0];
            let tau = if beta == alpha {
                0.
            } else if (beta - alpha).is_finite() {
                (beta - alpha) / beta
            } else {
                1. - alpha / beta
            };
            if tau == 0. {
                continue;
            }
            let mut v = nalgebra::DVector::from_column_slice(packed.data());
            v[0] = 1.;
            let w = h.columns(k + 1, m) * &v;
            h.columns_mut(k + 1, m).ger(-tau, &w, &v, 1.);
            let w = h.view((k + 1, k + 1), (m, m)).transpose() * &v;
            h.view_mut((k + 1, k + 1), (m, m)).ger(-tau, &v, &w, 1.);
            h[(k + 1, k)] = beta;
            for i in k + 2..n {
                h[(i, k)] = 0.;
            }
            if let Some(q) = &mut q {
                let w = q.columns(k + 1, m) * &v;
                q.columns_mut(k + 1, m).ger(-tau, &w, &v, 1.);
            }
        }
        let mut result = Vec::new();
        if let Some(q) = q {
            result.push(lax_val(&q)?);
        }
        result.push(lax_val(&h)?);
        Ok(result)
    }
    fn lax_linsolve(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let mut a = self.lax_real(&args[0])?;
        let b = self.lax_real(&args[1])?;
        let mut lower = false;
        let mut upper = false;
        if let Some(v) = args.get(2) {
            let Value::Container(Array::Struct(s)) = v else {
                return Err("Options must be a scalar structure.".into());
            };
            if s.len() != 1 {
                return Err("Options must be a scalar structure.".into());
            }
            let mut trans = false;
            for (i, key) in s.field_names().iter().enumerate() {
                let flag = self
                    .lax_real(&Value::from(s.get(i, 0)?.as_ref().clone()))?
                    .as_scalar()?
                    != 0.;
                match key.as_str() {
                    "LT" => lower = flag,
                    "UT" => upper = flag,
                    "TRANSA" => trans = flag,
                    "SYM" | "POSDEF" | "RECT" => {}
                    _ => return Err(format!("Unknown linsolve option '{key}'.")),
                }
            }
            if lower && upper {
                return Err("LT and UT cannot both be true.".into());
            }
            if trans {
                a = a.transpose();
                std::mem::swap(&mut lower, &mut upper);
            }
        }
        if a.rows() != b.rows() {
            return Err("Matrix dimensions must agree.".into());
        }
        if a.rows() == 0 && a.cols() == 0 {
            let mut result = vec![Value::Matrix(Matrix::filled(0, b.cols(), 0.)?)];
            if count == 2 {
                result.push(lax_scalar(if lower || upper { 1. } else { f64::INFINITY }));
            }
            return Ok(result);
        }
        self.lax_work(a.rows(), a.cols())?;
        self.lax_finite(&a)?;
        self.lax_finite(&b)?;
        let warn = count == 1;
        if a.rows() != a.cols() {
            // A triangular solve needs a square A; a rectangular A, flagged or
            // not, gets the general QR least-squares solution of size n-by-k.
            let solution = self.lax_least_squares(&a, &b)?;
            if warn {
                self.lax_rank_warning(&solution, a.rows().min(a.cols()), out)?;
            }
            let mut result = vec![lax_val(&solution.x)?];
            if count == 2 {
                result.push(lax_scalar(solution.rank as f64));
            }
            return Ok(result);
        }
        let ad = lax_dm(&a);
        let bd = lax_dm(&b);
        let mut triangular_rcond = None;
        let x = if lower || upper {
            // Only the flagged triangle is read, as LAPACK dtrtrs/dtrcon do.
            let t = DMatrix::from_fn(ad.nrows(), ad.ncols(), |i, j| {
                if (lower && i >= j) || (upper && i <= j) {
                    ad[(i, j)]
                } else {
                    0.
                }
            });
            let solve = |v: &DMatrix<f64>, transpose: bool| match (lower, transpose) {
                (true, false) => t.solve_lower_triangular(v),
                (true, true) => t.tr_solve_lower_triangular(v),
                (false, false) => t.solve_upper_triangular(v),
                (false, true) => t.tr_solve_upper_triangular(v),
            };
            let x = solve(&bd, false);
            if count == 2 {
                // The reference's r reads only the flagged triangle. The estimate of
                // norm(inv(T)) pairs the 1-norm of T with the A'/A solve order
                // reversed (the inverse infinity norm): [2 0;1 3] gives 2/3.
                let estimate = self.lax_inverse_norm1(t.nrows(), |v, tr| solve(v, !tr))?;
                triangular_rcond = Some(match estimate {
                    Some(e) if e != 0. && lax_norm1(&t) != 0. => 1. / (lax_norm1(&t) * e),
                    _ => 0.,
                });
            }
            if warn && x.is_some() {
                let estimate = self.lax_inverse_norm1(t.nrows(), solve)?;
                self.lax_condition_warning(lax_norm1(&t), estimate, out)?;
            }
            x
        } else {
            self.lax_lu_solve(&ad, &bd, warn, out)?
        };
        let x = match x {
            Some(x) => x,
            None => {
                if warn {
                    self.emit(
                        out,
                        "Warning: Matrix is singular to working precision.".into(),
                    )?;
                }
                DMatrix::from_element(a.cols(), b.cols(), f64::NAN)
            }
        };
        let mut result = vec![lax_val(&x)?];
        if count == 2 {
            result.push(lax_scalar(match triangular_rcond {
                Some(r) => r,
                None => self.lax_recip_condition(&ad)?.0,
            }));
        }
        Ok(result)
    }
    /// LU solve with partial pivoting (dgesv). None for an exactly singular
    /// factor; with `warn`, an estimated RCOND below eps (dgecon) warns.
    fn lax_lu_solve(
        &mut self,
        a: &DMatrix<f64>,
        b: &DMatrix<f64>,
        warn: bool,
        out: &mut Execution,
    ) -> Result<Option<DMatrix<f64>>> {
        let lu = a.clone().lu();
        let Some(x) = lu.solve(b) else {
            return Ok(None);
        };
        if warn && a.nrows() > 0 {
            let estimate = self.lax_lu_inverse_norm1(a.nrows(), &lu)?;
            self.lax_condition_warning(lax_norm1(a), estimate, out)?;
        }
        Ok(Some(x))
    }

    /// Reuse the same LU/transpose solves for linsolve and operator warnings.
    fn lax_lu_inverse_norm1(
        &self,
        n: usize,
        lu: &nalgebra::linalg::LU<f64, nalgebra::Dyn, nalgebra::Dyn>,
    ) -> Result<Option<f64>> {
        // P*A = L*U, so A' y = c is U' (L' (P y)) = c.
        let (l, u, p) = (lu.l(), lu.u(), lu.p());
        let solve = |v: &DMatrix<f64>, transpose: bool| {
            if transpose {
                let mut y = l.tr_solve_lower_triangular(&u.tr_solve_upper_triangular(v)?)?;
                p.inv_permute_rows(&mut y);
                Some(y)
            } else {
                lu.solve(v)
            }
        };
        self.lax_inverse_norm1(n, solve)
    }
    /// LAPACK dlacn2 (Hager/Higham) estimate of norm(inv(A),1) from solves
    /// with A (`false`) and A' (`true`), as dgecon and dtrcon use it.
    fn lax_inverse_norm1(
        &self,
        n: usize,
        solve: impl Fn(&DMatrix<f64>, bool) -> Option<DMatrix<f64>>,
    ) -> Result<Option<f64>> {
        if n == 0 {
            return Ok(Some(0.));
        }
        let sign = |x: &DMatrix<f64>| x.map(|v| if v >= 0. { 1. } else { -1. });
        let asum = |x: &DMatrix<f64>| x.iter().map(|v| v.abs()).sum::<f64>();
        let Some(x) = solve(&DMatrix::from_element(n, 1, 1. / n as f64), false) else {
            return Ok(None);
        };
        if n == 1 {
            return Ok(Some(x[0].abs()));
        }
        let mut estimate = asum(&x);
        let mut signs = sign(&x);
        let Some(x) = solve(&signs, true) else {
            return Ok(None);
        };
        let mut j = lax_first_max(0..n, |i| x[i].abs()).unwrap_or(0);
        for iteration in 2.. {
            self.check_cancel()?;
            let mut unit = DMatrix::zeros(n, 1);
            unit[j] = 1.;
            let Some(x) = solve(&unit, false) else {
                return Ok(None);
            };
            let old = estimate;
            estimate = asum(&x);
            let next = sign(&x);
            // A repeated sign vector has converged; a smaller estimate cycles.
            if next == signs || estimate <= old {
                break;
            }
            signs = next;
            let Some(x) = solve(&signs, true) else {
                return Ok(None);
            };
            let last = j;
            j = lax_first_max(0..n, |i| x[i].abs()).unwrap_or(0);
            if x[last] == x[j].abs() || iteration >= 5 {
                break;
            }
        }
        let alternating = DMatrix::from_fn(n, 1, |i, _| {
            (1. + i as f64 / (n - 1) as f64) * if i % 2 == 0 { 1. } else { -1. }
        });
        let Some(x) = solve(&alternating, false) else {
            return Ok(None);
        };
        Ok(Some(estimate.max(2. * asum(&x) / (3 * n) as f64)))
    }
    fn lax_condition_warning(
        &mut self,
        norm: f64,
        inverse_norm: Option<f64>,
        out: &mut Execution,
    ) -> Result<()> {
        let rcond = lax_rcond(norm, inverse_norm);
        if rcond < f64::EPSILON {
            self.emit(out, format!("Warning: {}", lax_close_message(rcond)))?;
        }
        Ok(())
    }
    /// mldivide's rank warning for a rectangular least-squares solve.
    fn lax_rank_warning(
        &mut self,
        solution: &LaxLeastSquares,
        full: usize,
        out: &mut Execution,
    ) -> Result<()> {
        if solution.rank < full {
            self.emit(
                out,
                format!(
                    "Warning: Rank deficient, rank = {}, tol = {:>13}.",
                    solution.rank,
                    crate::display_format::exponent(solution.tol, 6)
                ),
            )?;
        }
        Ok(())
    }
    /// `A\b` for dense real A as MATLAB chooses: LU for a square A, otherwise
    /// QR with column pivoting. Warnings go through the output budget.
    fn lax_mldivide(
        &mut self,
        a: &Matrix,
        b: &Matrix,
        out: &mut Execution,
    ) -> Result<DMatrix<f64>> {
        if a.rows() == a.cols() {
            let x = self.lax_lu_solve(&lax_dm(a), &lax_dm(b), true, out)?;
            return Ok(match x {
                Some(x) => x,
                None => {
                    self.emit(
                        out,
                        "Warning: Matrix is singular to working precision.".into(),
                    )?;
                    DMatrix::from_element(a.cols(), b.cols(), f64::NAN)
                }
            });
        }
        let solution = self.lax_least_squares(a, b)?;
        self.lax_rank_warning(&solution, a.rows().min(a.cols()), out)?;
        Ok(solution.x)
    }
    /// Rectangular least squares as MATLAB's mldivide/linsolve solve it: QR
    /// with column pivoting (the engine's 3-output economy qr), rank r from
    /// |R(i,i)| > max(m,n)*eps*|R(1,1)|, and the basic solution
    /// x(p(1:r),:) = R(1:r,1:r) \ (Q(:,1:r)'*b) with the other rows zero.
    fn lax_least_squares(&self, a: &Matrix, b: &Matrix) -> Result<LaxLeastSquares> {
        if a.rows() != b.rows() {
            return Err("Matrix dimensions must agree.".into());
        }
        let (m, n) = (a.rows(), a.cols());
        self.lax_work(m, n)?;
        self.lax_budget(b.rows().max(n), b.cols(), 4)?;
        let mut solution = LaxLeastSquares {
            x: DMatrix::zeros(n, b.cols()),
            rank: 0,
            tol: 0.,
            r: DMatrix::zeros(0, 0),
            perm: Vec::new(),
        };
        if a.is_empty() {
            return Ok(solution);
        }
        let qr = self
            .linalg2_call("qr", &[Value::Matrix(a.clone()), lax_scalar(0.)], 3)?
            .ok_or("qr dispatch failed")?;
        let q = self.lax_real(&qr[0])?;
        let r = lax_dm(&self.lax_real(&qr[1])?);
        let perm: Vec<usize> = self
            .lax_real(&qr[2])?
            .data()
            .iter()
            .map(|&p| p as usize - 1)
            .collect();
        let tol = m.max(n) as f64 * f64::EPSILON * r[(0, 0)].abs();
        let rank = (0..m.min(n)).take_while(|&i| r[(i, i)].abs() > tol).count();
        solution.tol = tol;
        solution.rank = rank;
        if rank > 0 {
            let r11 = r.view((0, 0), (rank, rank)).into_owned();
            let qtb = lax_dm(&q.transpose().mul(b)?);
            let y = r11
                .solve_upper_triangular(&qtb.rows(0, rank).into_owned())
                .ok_or("Singular triangular factor.")?;
            for (i, &p) in perm.iter().take(rank).enumerate() {
                solution.x.row_mut(p).copy_from(&y.row(i));
            }
            solution.r = r11;
        }
        solution.perm = perm;
        Ok(solution)
    }
    fn lax_recip_condition(&self, a: &DMatrix<f64>) -> Result<(f64, DMatrix<f64>)> {
        let n = a.nrows();
        if n == 0 {
            return Ok((f64::INFINITY, DMatrix::zeros(0, 1)));
        }
        if a.iter().any(|x| !x.is_finite()) {
            return Ok((f64::NAN, DMatrix::from_element(n, 1, f64::NAN)));
        }
        let lu = a.clone().lu();
        let mut best = 0.;
        let mut v = DMatrix::zeros(n, 1);
        // Exact inverse 1-norm using vector solves: no explicit inverse is materialized.
        for j in 0..n {
            self.check_cancel()?;
            let mut e = DMatrix::zeros(n, 1);
            e[(j, 0)] = 1.;
            let Some(y) = lu.solve(&e) else {
                return Ok((0., v));
            };
            let norm = y.iter().map(|x| x.abs()).sum::<f64>();
            if norm > best {
                best = norm;
                v = y / norm;
            }
        }
        Ok((1. / (lax_norm1(a) * best), v))
    }
    fn lax_condition(&self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let a = self.lax_real(&args[0])?;
        lax_square(
            &a,
            if name == "rcond" {
                "Input must be a square matrix."
            } else {
                "Matrix must be square."
            },
        )?;
        self.lax_work(a.rows(), a.cols())?;
        if let Some(v) = args.get(1) {
            let t = self.lax_real(v)?.as_scalar()?;
            if !t.is_finite() || t < 1. || t.fract() != 0. {
                return Err("Block size must be a positive integer.".into());
            }
        }
        let (r, mut v) = self.lax_recip_condition(&lax_dm(&a))?;
        if name == "condest" && count == 2 && r == 0. {
            let basis = self.linalg2_call("null", &[Value::Matrix(a)], 1)?.unwrap();
            let basis = self.lax_real(&basis[0])?;
            if basis.cols() > 0 {
                v = lax_dm(&basis).columns(0, 1).into_owned();
                let norm = v.iter().map(|x| x.abs()).sum::<f64>();
                v /= norm;
            }
        }
        Ok(vec![
            lax_scalar(if name == "rcond" { r } else { 1. / r }),
            lax_val(&v)?,
        ])
    }
    fn lax_normest(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        let a = self.lax_real(&args[0])?;
        self.lax_finite(&a)?;
        self.lax_work(a.rows(), a.cols())?;
        let tol = if let Some(v) = args.get(1) {
            self.lax_real(v)?.as_scalar()?
        } else {
            1e-6
        };
        if !tol.is_finite() || tol <= 0. {
            return Err("Tolerance must be positive and finite.".into());
        }
        let a = lax_dm(&a);
        let mut x = DMatrix::from_fn(a.ncols(), 1, |j, _| {
            a.column(j).iter().map(|v| v.abs()).sum::<f64>()
        });
        let mut e = x.norm();
        let mut count = 0;
        if e != 0. {
            x /= e;
            let mut old = 0.;
            // After 100 iterations, warn and return the current norm estimate.
            while (e - old).abs() > tol * e {
                self.check_cancel()?;
                old = e;
                let mut y = &a * &x;
                let mut yn = y.norm();
                if yn == 0. {
                    // The initial positive vector can lie in the nullspace
                    // (for example A=[1 -1]). Restart from a nonzero column.
                    let j = lax_first_max(0..a.ncols(), |j| a.column(j).norm_squared())
                        .ok_or("normest could not select a restart column.")?;
                    y = a.columns(j, 1).into_owned();
                    yn = y.norm();
                    if yn == 0. {
                        e = 0.;
                        break;
                    }
                }
                x = a.transpose() * y;
                let xn = x.norm();
                e = xn / yn;
                x /= xn;
                count += 1;
                if count > 100 {
                    self.emit(
                        out,
                        format!("Warning: NORMEST did not converge for 100 iterations with tolerance {}", super::text_format::shortest_g(tol, 6)),
                    )?;
                    break;
                }
            }
        }
        Ok(vec![lax_scalar(e), lax_scalar(count as f64)])
    }
    fn lax_subspace(&self, args: &[Value]) -> Result<Value> {
        let a = self.lax_real(&args[0])?;
        let b = self.lax_real(&args[1])?;
        if a.rows() != b.rows() {
            return Err("Inner matrix dimensions must agree.".into());
        }
        self.lax_work(a.rows(), a.cols().max(b.cols()))?;
        let qa = self.linalg2_call("orth", &[Value::Matrix(a)], 1)?.unwrap();
        let qb = self.linalg2_call("orth", &[Value::Matrix(b)], 1)?.unwrap();
        let mut qa = lax_dm(&self.lax_real(&qa[0])?);
        let mut qb = lax_dm(&self.lax_real(&qb[0])?);
        if qa.ncols() < qb.ncols() {
            std::mem::swap(&mut qa, &mut qb);
        }
        if qb.ncols() == 0 {
            return Ok(lax_scalar(0.));
        }
        let r = &qb - &qa * (qa.transpose() * &qb);
        let s = r.svd(false, false).singular_values.max();
        Ok(lax_scalar(s.min(1.).asin()))
    }
    /// Horner's rule for a matrix argument, Y <- A*Y + p(k)*I from Y = p(1)*I
    /// (Golub & Van Loan, Matrix Computations, 4th ed., 2013, §9.2): n-1
    /// products by the bounded GEMM. For finite A and p(1) the first product
    /// A*(p(1)*I) is p(1)*A exactly, so that step scales instead.
    fn lax_polyvalm(&self, args: &[Value]) -> Result<Value> {
        let p = self.lax_real(&args[0])?;
        let a = self.lax_real(&args[1])?;
        let n = lax_square(&a, "Matrix must be square.")?;
        if p.rows() > 1 && p.cols() > 1 {
            return Err("Polynomial coefficients must be a vector.".into());
        }
        if p.len() > 4097 {
            return Err("polyvalm supports polynomial degree up to 4096".into());
        }
        self.lax_work(n, n)?;
        self.lax_budget(n, n, 3)?;
        let Some((&first, rest)) = p.data().split_first() else {
            return Ok(Value::Matrix(Matrix::filled(n, n, 0.)?));
        };
        let finite = first.is_finite() && a.data().iter().all(|v| v.is_finite());
        let (mut y, rest) = match rest.split_first() {
            Some((&second, tail)) if finite => {
                (lax_add_diagonal(a.map(|v| v * first), second)?, tail)
            }
            // p(1)*I scales the diagonal only: an infinite p(1) must not turn
            // the zero off-diagonal entries into NaN.
            _ => (lax_add_diagonal(Matrix::filled(n, n, 0.)?, first)?, rest),
        };
        for &c in rest {
            self.check_cancel()?;
            y = lax_add_diagonal(a.mul(&y)?, c)?;
        }
        Ok(Value::Matrix(y))
    }
    fn lax_planerot(&self, args: &[Value]) -> Result<Vec<Value>> {
        let x = self.lax_real(&args[0])?;
        if (x.rows(), x.cols()) != (2, 1) {
            return Err("Dimensions of matrices being concatenated are not consistent.".into());
        }
        let (a, b) = (x.data()[0], x.data()[1]);
        if b == 0. {
            return Ok(vec![
                Value::Matrix(Matrix::identity(2, 2)?),
                Value::Matrix(x),
            ]);
        }
        let r = a.hypot(b);
        let c = a / r;
        let s = b / r;
        Ok(vec![
            Value::Matrix(Matrix::from_column_vec(2, 2, vec![c, -s, s, c])?),
            Value::Matrix(Matrix::from_column_vec(2, 1, vec![r, 0.])?),
        ])
    }
    fn lax_ldl(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let a = self.lax_real(&args[0])?;
        let n = lax_square(&a, "Matrix must be square.")?;
        self.lax_work(n, n)?;
        self.lax_finite(&a)?;
        let option = args.get(1).map(lax_text);
        let upper = match option.as_ref().map(|s| s.as_deref()) {
            None | Some(Some("lower")) => false,
            Some(Some("upper")) => true,
            _ => return Err("Unknown LDL option.".into()),
        };
        // Symmetric indefinite factorization with bounded rook pivot search.
        // Only the selected triangle is read, as in MATLAB's dense LDL.
        let mut s = DMatrix::from_fn(n, n, |i, j| {
            if upper {
                a.get(i.min(j), i.max(j))
            } else {
                a.get(i.max(j), i.min(j))
            }
        });
        let mut l = DMatrix::<f64>::identity(n, n);
        let mut d = DMatrix::<f64>::zeros(n, n);
        let mut perm: Vec<usize> = (0..n).collect();
        let alpha = (1. + 17f64.sqrt()) / 8.;
        let mut k = 0;
        while k < n {
            self.check_cancel()?;
            let mut p = k;
            let mut q = k;
            let mut block = 1;
            for _ in k..n + 1 {
                let imax = lax_first_max((k..n).filter(|&i| i != p), |i| s[(i, p)].abs());
                let Some(i) = imax else {
                    break;
                };
                let colmax = s[(i, p)].abs();
                if s[(p, p)].abs() >= alpha * colmax || colmax == 0. {
                    break;
                }
                let j = lax_first_max((k..n).filter(|&j| j != i), |j| s[(i, j)].abs()).unwrap();
                let rowmax = s[(i, j)].abs();
                if s[(i, i)].abs() >= alpha * rowmax {
                    p = i;
                    break;
                }
                if j == p || rowmax <= colmax {
                    q = i;
                    block = 2;
                    break;
                }
                p = i;
            }
            if p != k {
                s.swap_rows(k, p);
                s.swap_columns(k, p);
                perm.swap(k, p);
                for j in 0..k {
                    l.swap((k, j), (p, j));
                }
                if q == k {
                    q = p;
                }
            }
            if block == 2 && k + 1 < n {
                if q != k + 1 {
                    s.swap_rows(k + 1, q);
                    s.swap_columns(k + 1, q);
                    perm.swap(k + 1, q);
                    for j in 0..k {
                        l.swap((k + 1, j), (q, j));
                    }
                }
                let pivot = s.view((k, k), (2, 2)).into_owned();
                d.view_mut((k, k), (2, 2)).copy_from(&pivot);
                let rhs = s.view((k + 2, k), (n - k - 2, 2)).transpose();
                let solved = pivot
                    .lu()
                    .solve(&rhs)
                    .ok_or("Singular LDL pivot.")?
                    .transpose();
                l.view_mut((k + 2, k), (n - k - 2, 2)).copy_from(&solved);
                let update = &solved * d.view((k, k), (2, 2)) * solved.transpose();
                for j in k + 2..n {
                    for i in k + 2..n {
                        s[(i, j)] -= update[(i - k - 2, j - k - 2)];
                    }
                }
                k += 2;
            } else {
                let pivot = s[(k, k)];
                d[(k, k)] = pivot;
                if pivot != 0. {
                    for i in k + 1..n {
                        l[(i, k)] = s[(i, k)] / pivot;
                    }
                    for j in k + 1..n {
                        for i in j..n {
                            s[(i, j)] -= l[(i, k)] * pivot * l[(j, k)];
                            s[(j, i)] = s[(i, j)];
                        }
                    }
                }
                k += 1;
            }
        }
        let p = DMatrix::from_fn(n, n, |i, j| f64::from(i == perm[j]));
        if count < 3 {
            l = &p * l;
        }
        if upper {
            l.transpose_mut();
        }
        Ok(vec![lax_val(&l)?, lax_val(&d)?, lax_val(&p)?])
    }
    /// Weighted and generalized least squares (Golub & Van Loan, Matrix
    /// Computations, 4th ed., 2013, §5.3-5.6 and §6.1). Weights w scale the
    /// rows by sqrt(w); a covariance V = L*L' (Cholesky) whitens the problem
    /// to L\A, L\B. The whitened problem is solved by pivoted QR (the basic
    /// solution when A has rank r < n). With R11 the leading r-by-r factor,
    /// mse = |residual|^2/(m-r) per column and S = mse*inv(R11'*R11) on the
    /// basic columns; the other rows and columns of S and SE are zero.
    fn lax_lscov(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let a = self.lax_real(&args[0])?;
        let b = self.lax_real(&args[1])?;
        let (m, n) = (a.rows(), a.cols());
        if b.rows() != m {
            return Err("B must have the same number of rows as A.".into());
        }
        self.lax_finite(&a)?;
        self.lax_finite(&b)?;
        if let Some(v) = args.get(3) {
            let algorithm = lax_text(v).map(|s| s.to_ascii_lowercase());
            if !matches!(algorithm.as_deref(), Some("chol" | "orth")) {
                return Err("The algorithm must be 'chol' or 'orth'.".into());
            }
        }
        if count >= 2 && a.is_empty() {
            return Err("SE, MSE and S outputs require a nonempty design matrix.".into());
        }
        if count == 4 && b.cols() != 1 {
            return Err("The S output requires B to have one column.".into());
        }
        self.lax_work(m, n)?;
        self.lax_budget(m, n + b.cols(), 2)?;
        let weights = match args.get(2) {
            Some(v) => Some(self.lax_real(v)?).filter(|w| !w.is_empty()),
            None => None,
        };
        let (a, b) = match weights {
            None => (a, b),
            Some(w) if (w.rows() == 1 || w.cols() == 1) && w.len() == m => {
                if w.data().iter().any(|v| !v.is_finite() || *v < 0.) {
                    return Err("Weights must be nonnegative and finite.".into());
                }
                let s: Vec<f64> = w.data().iter().map(|v| v.sqrt()).collect();
                let scale = |x: &Matrix| {
                    let mut x = lax_dm(x);
                    for mut column in x.column_iter_mut() {
                        for (v, s) in column.iter_mut().zip(&s) {
                            *v *= s;
                        }
                    }
                    lax_mx(&x)
                };
                (scale(&a)?, scale(&b)?)
            }
            Some(v) if v.rows() == m && v.cols() == m => {
                self.lax_finite(&v)?;
                self.lax_work(m, m)?;
                let l = lax_dm(&v)
                    .cholesky()
                    .ok_or("The covariance matrix V must be symmetric positive definite.")?
                    .unpack();
                self.check_cancel()?;
                let whiten = |x: &Matrix| {
                    l.solve_lower_triangular(&lax_dm(x))
                        .ok_or_else(|| "Singular covariance factor.".to_string())
                        .and_then(|x| lax_mx(&x))
                };
                (whiten(&a)?, whiten(&b)?)
            }
            Some(_) => {
                return Err(
                    "The third input must be a weight vector with one entry per row of A or a square covariance matrix of that size.".into(),
                );
            }
        };
        let solution = self.lax_least_squares(&a, &b)?;
        if solution.rank < n && !a.is_empty() {
            self.lax_identified_warning(
                "MATLAB:lscov:RankDefDesignMat",
                "A is rank deficient to within machine precision.",
                out,
            )?;
        }
        let mut result = vec![lax_val(&solution.x)?];
        if count < 2 {
            return Ok(result);
        }
        self.check_cancel()?;
        let (k, rank) = (b.cols(), solution.rank);
        let fitted = lax_dm(&a.mul(&lax_mx(&solution.x)?)?);
        let b = lax_dm(&b);
        // An exact fit (m = r) leaves no degrees of freedom: mse is 0.
        let mse: Vec<f64> = (0..k)
            .map(|j| {
                if m == rank {
                    return 0.;
                }
                let ss: f64 = (0..m).map(|i| (b[(i, j)] - fitted[(i, j)]).powi(2)).sum();
                ss / (m - rank) as f64
            })
            .collect();
        // inv(R11'*R11) = inv(R11)*inv(R11)'; inv(R11) from triangular solves.
        let rinv = solution
            .r
            .solve_upper_triangular(&DMatrix::identity(rank, rank))
            .ok_or("Singular triangular factor.")?;
        let basic = &solution.perm[..rank];
        let mut se = DMatrix::zeros(n, k);
        for (i, &p) in basic.iter().enumerate() {
            let d = rinv.row(i).norm_squared();
            for (j, mse) in mse.iter().enumerate() {
                se[(p, j)] = (d * mse).sqrt();
            }
        }
        result.push(lax_val(&se)?);
        result.push(Value::Matrix(Matrix::from_column_vec(1, k, mse.clone())?));
        if count == 4 {
            self.lax_budget(n, n, 2)?;
            let covariance = &rinv * rinv.transpose() * mse[0];
            let mut s = DMatrix::zeros(n, n);
            for (j, &q) in basic.iter().enumerate() {
                for (i, &p) in basic.iter().enumerate() {
                    s[(p, q)] = covariance[(i, j)];
                }
            }
            result.push(lax_val(&s)?);
        }
        Ok(result)
    }
    /// Nonnegative least squares by the active-set algorithm NNLS of Lawson &
    /// Hanson, Solving Least Squares Problems (SIAM, 1995), ch. 23. The
    /// passive set P grows by the variable with the largest dual w = C'*r
    /// above tol; an infeasible subproblem solution (a passive entry <= 0)
    /// moves x towards it until the first passive variable reaches zero (inner
    /// loop). tol bounds the dual only: solution entries are compared with 0,
    /// since their scale has nothing to do with the scale of w. `iterations`
    /// counts the outer loop. tol defaults to 10*eps*norm(C,1)*max(m,n) and
    /// MaxIter to 3*n, the iteration limit Lawson & Hanson suggest.
    fn lax_nnls(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let c = self.lax_real(&args[0])?;
        let d = self.lax_real(&args[1])?;
        let (m, n) = (c.rows(), c.cols());
        if d.rows() != m {
            return Err("Matrix dimensions must agree.".into());
        }
        if d.cols() != 1 && !(m == 0 && d.is_empty()) {
            return Err("d must be a column vector.".into());
        }
        let (mut tol, mut max_iter) = (None, None);
        match args.get(2) {
            None => {}
            Some(Value::Matrix(e)) if e.is_empty() => {}
            Some(Value::Container(Array::Struct(s))) if s.len() == 1 => {
                for (i, key) in s.field_names().iter().enumerate() {
                    let tolerance = key.eq_ignore_ascii_case("TolX");
                    if !tolerance && !key.eq_ignore_ascii_case("MaxIter") {
                        continue;
                    }
                    let value = self.lax_real(&Value::from(s.get(i, 0)?.as_ref().clone()))?;
                    let Some(&v) = value.data().first() else {
                        continue;
                    };
                    if value.len() != 1 {
                        return Err(format!("Option {key} must be a scalar."));
                    }
                    if tolerance {
                        if !(v.is_finite() && v >= 0.) {
                            return Err("TolX must be a finite nonnegative scalar.".into());
                        }
                        tol = Some(v);
                    } else {
                        if !(v >= 1. && (v.fract() == 0. || v.is_infinite())) {
                            return Err("MaxIter must be a positive integer.".into());
                        }
                        max_iter = Some(if v.is_infinite() {
                            usize::MAX
                        } else {
                            v as usize
                        });
                    }
                }
            }
            Some(_) => return Err("Options must be a scalar structure.".into()),
        }
        self.lax_finite(&c)?;
        self.lax_finite(&d)?;
        self.lax_work(m, n)?;
        self.lax_budget(m, n, 4)?;
        let cd = lax_dm(&c);
        let dd = DMatrix::from_column_slice(m, 1, d.data());
        let tol = tol.unwrap_or(10. * f64::EPSILON * lax_norm1(&cd) * m.max(n) as f64);
        let max_iter = max_iter.unwrap_or(3 * n);
        let mut x = DMatrix::<f64>::zeros(n, 1);
        let mut passive = vec![false; n];
        // Variables whose trial solution was nonpositive; they may not enter
        // again until x has moved. w itself stays the true dual.
        let mut rejected = vec![false; n];
        let mut residual = dd.clone();
        let mut w = cd.tr_mul(&residual);
        let mut iterations = 0;
        let mut converged = true;
        loop {
            self.check_cancel()?;
            let Some(t) = lax_first_max(
                (0..n).filter(|&j| !passive[j] && !rejected[j] && w[j] > tol),
                |j| w[j],
            ) else {
                break;
            };
            if iterations >= max_iter {
                converged = false;
                break;
            }
            passive[t] = true;
            let mut z = self.lax_nnls_solve(&cd, &d, &passive, out)?;
            if z[t] <= 0. {
                // Rounding made the entering variable nonpositive: reject it
                // for this x (Lawson & Hanson's safeguard against cycling).
                passive[t] = false;
                rejected[t] = true;
                continue;
            }
            iterations += 1;
            while let Some(q) = lax_first_max((0..n).filter(|&j| passive[j] && z[j] <= 0.), |j| {
                -x[j] / (x[j] - z[j])
            }) {
                self.check_cancel()?;
                let alpha = x[q] / (x[q] - z[q]);
                for j in (0..n).filter(|&j| passive[j]) {
                    x[j] += alpha * (z[j] - x[j]);
                }
                x[q] = 0.;
                for j in 0..n {
                    if passive[j] && x[j] <= 0. {
                        passive[j] = false;
                        x[j] = 0.;
                    }
                }
                z = self.lax_nnls_solve(&cd, &d, &passive, out)?;
            }
            x = z;
            rejected.fill(false);
            residual = &dd - &cd * &x;
            w = cd.tr_mul(&residual);
        }
        let (exitflag, message) = if converged {
            (1., "Optimization terminated.".to_string())
        } else {
            let text = format!(
                "Exiting: the iteration count reached MaxIter = {max_iter}; increase MaxIter or TolX."
            );
            self.emit(out, text.clone())?;
            (0., text)
        };
        let mut result = vec![
            lax_val(&x)?,
            lax_scalar(residual.norm_squared()),
            lax_val(&residual)?,
            lax_scalar(exitflag),
        ];
        if count >= 5 {
            let text = |s: &str| CharacterRow::from_units(s.encode_utf16().collect(), &self.cancel);
            result.push(self.lax_struct(vec![
                (
                    "iterations",
                    Array::Numeric(Matrix::scalar(iterations as f64)),
                ),
                ("algorithm", Array::Character(text("active-set")?)),
                ("message", Array::Character(text(&message)?)),
            ])?);
            result.push(lax_val(&w)?);
        }
        Ok(result)
    }
    /// The least-squares solution on the passive columns, `C(:,P)\d` with
    /// mldivide's choice of solver, scattered into an n-vector.
    fn lax_nnls_solve(
        &mut self,
        c: &DMatrix<f64>,
        d: &Matrix,
        passive: &[bool],
        out: &mut Execution,
    ) -> Result<DMatrix<f64>> {
        let columns: Vec<usize> = (0..c.ncols()).filter(|&j| passive[j]).collect();
        let mut z = DMatrix::zeros(c.ncols(), 1);
        if columns.is_empty() {
            return Ok(z);
        }
        let sub = DMatrix::from_fn(c.nrows(), columns.len(), |i, j| c[(i, columns[j])]);
        let y = self.lax_mldivide(&lax_mx(&sub)?, d, out)?;
        for (i, &j) in columns.iter().enumerate() {
            z[j] = y[(i, 0)];
        }
        Ok(z)
    }
    fn lax_sylvester(&self, args: &[Value]) -> Result<Value> {
        let a = self.lax_real(&args[0])?;
        let b = self.lax_real(&args[1])?;
        let c = self.lax_real(&args[2])?;
        if a.rows() != a.cols()
            || b.rows() != b.cols()
            || c.rows() != a.rows()
            || c.cols() != b.rows()
        {
            return Err("Inputs must have compatible size.".into());
        }
        let (m, n) = (a.rows(), b.rows());
        self.lax_work(m, m)?;
        self.lax_work(n, n)?;
        self.lax_budget(m, n, 8)?;
        self.lax_finite(&a)?;
        self.lax_finite(&b)?;
        self.lax_finite(&c)?;
        if m == 0 || n == 0 {
            return Ok(Value::Matrix(c));
        }
        // A scalar identity reduces the equation to one shifted linear solve.
        // This avoids both redundant factorizations and repeated-eigenvalue Schur iteration.
        let scalar_identity = |a: &Matrix| {
            let shift = a.get(0, 0);
            (0..a.cols())
                .all(|j| (0..a.rows()).all(|i| a.get(i, j) == if i == j { shift } else { 0. }))
                .then_some(shift)
        };
        if let Some(shift) = scalar_identity(&b) {
            let mut shifted = lax_dm(&a);
            for i in 0..m {
                shifted[(i, i)] += shift;
            }
            let x = shifted
                .lu()
                .solve(&lax_dm(&c))
                .ok_or("Sylvester equation has no unique solution.")?;
            return lax_val(&x);
        }
        if let Some(shift) = scalar_identity(&a) {
            let mut shifted = lax_dm(&b).transpose();
            for i in 0..n {
                shifted[(i, i)] += shift;
            }
            let x = shifted
                .lu()
                .solve(&lax_dm(&c).transpose())
                .ok_or("Sylvester equation has no unique solution.")?;
            return lax_val(&x.transpose());
        }
        // Bartels-Stewart: real Schur reduction followed by conversion of its
        // 2x2 blocks to complex triangular form. Avoid complex Schur iteration
        // on repeated real eigenvalues. O(m^3+n^3), never a Kronecker matrix.
        self.preflight_elements(8 * (m * m + n * n + m * n))?;
        let schur = |a: &Matrix| -> Result<(DMatrix<C64>, DMatrix<C64>)> {
            let real = self.lax_decompose("schur", &[Value::Matrix(a.clone())], 2)?;
            let complex = self.lax_rsf2csf(&real)?;
            Ok((
                self.lax_complex(&complex[0])?,
                self.lax_complex(&complex[1])?,
            ))
        };
        let (u, r) = schur(&a)?;
        self.check_cancel()?;
        let (v, s) = schur(&b)?;
        let mut y = u.adjoint() * lax_dm(&c).map(|x| C64::new(x, 0.)) * &v;
        for j in 0..n {
            self.check_cancel()?;
            for i in (0..m).rev() {
                let mut rhs = y[(i, j)];
                for k in i + 1..m {
                    rhs -= r[(i, k)] * y[(k, j)];
                }
                for k in 0..j {
                    rhs -= y[(i, k)] * s[(k, j)];
                }
                let den = r[(i, i)] + s[(j, j)];
                if den.norm() == 0. {
                    return Err("Sylvester equation has no unique solution.".into());
                }
                y[(i, j)] = rhs / den;
            }
        }
        lax_val(&(u * y * v.adjoint()).map(|z| z.re))
    }
    fn lax_balance(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let a = self.lax_real(&args[0])?;
        let n = lax_square(&a, "Matrix must be square.")?;
        self.lax_work(n, n)?;
        self.lax_finite(&a)?;
        let option = args.get(1).map(lax_text);
        let permute = match option.as_ref().map(|s| s.as_deref()) {
            None => true,
            Some(Some("noperm")) => false,
            _ => return Err("Unknown balance option.".into()),
        };
        let mut b = lax_dm(&a);
        let mut p: Vec<usize> = (0..n).collect();
        let mut sc = vec![1.; n];
        let mut low = 0;
        let mut high = n;
        if permute {
            // dgebal: push rows that isolate an eigenvalue down until none is
            // left, then push such columns left; b(i,j) = A(p(i),p(j)).
            while high > 0 {
                self.check_cancel()?;
                let row = (0..high)
                    .rev()
                    .find(|&i| (0..high).all(|j| j == i || b[(i, j)] == 0.));
                let Some(i) = row else {
                    break;
                };
                high -= 1;
                b.swap_rows(i, high);
                b.swap_columns(i, high);
                p.swap(i, high);
            }
            while low < high {
                self.check_cancel()?;
                let col = (low..high).find(|&j| (low..high).all(|i| i == j || b[(i, j)] == 0.));
                let Some(j) = col else {
                    break;
                };
                b.swap_rows(j, low);
                b.swap_columns(j, low);
                p.swap(j, low);
                low += 1;
            }
        }
        for pass in 0..1000 {
            self.check_cancel()?;
            let mut changed = false;
            for i in low..high {
                let mut c = (low..high)
                    .filter(|&j| j != i)
                    .map(|j| b[(j, i)].abs())
                    .sum::<f64>();
                let r = (low..high)
                    .filter(|&j| j != i)
                    .map(|j| b[(i, j)].abs())
                    .sum::<f64>();
                if c == 0. || r == 0. || !c.is_finite() || !r.is_finite() {
                    continue;
                }
                let sum = c + r;
                let mut f = 1.;
                while c < r / 2. && f < 1e150 {
                    f *= 2.;
                    c *= 4.;
                }
                while c >= r * 2. && f > 1e-150 {
                    f /= 2.;
                    c /= 4.;
                }
                if (c + r) / f < 0.95 * sum && (sc[i] * f).is_finite() && sc[i] * f != 0. {
                    changed = true;
                    sc[i] *= f;
                    for j in 0..n {
                        b[(i, j)] /= f;
                    }
                    for j in 0..n {
                        b[(j, i)] *= f;
                    }
                }
            }
            if !changed {
                break;
            }
            if pass == 999 {
                return Err("balance did not converge.".into());
            }
        }
        if count == 1 {
            return Ok(vec![lax_val(&b)?]);
        }
        if count == 2 {
            let t = DMatrix::from_fn(n, n, |i, j| if i == p[j] { sc[j] } else { 0. });
            return Ok(vec![lax_val(&t)?, lax_val(&b)?]);
        }
        // The documented relations T(:,P) = diag(S) and
        // B(P,P) = diag(1./S)*A*diag(S): P is the inverse of p and S is
        // indexed by the original rows, S(p(k)) = sc(k).
        let mut inverse = vec![0.; n];
        let mut scale = vec![0.; n];
        for (k, &i) in p.iter().enumerate() {
            inverse[i] = (k + 1) as f64;
            scale[i] = sc[k];
        }
        Ok(vec![
            Value::Matrix(Matrix::from_column_vec(n, 1, scale)?),
            Value::Matrix(Matrix::from_column_vec(n, 1, inverse)?),
            lax_val(&b)?,
        ])
    }
    fn lax_struct(&self, fields: Vec<(&str, Array)>) -> Result<Value> {
        let (names, slots) = fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), std::sync::Arc::new(value)))
            .unzip();
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?)))
    }
    fn lax_mkpp(&self, args: &[Value]) -> Result<Value> {
        let breaks = self.lax_real(&args[0])?;
        let c = self.lax_real(&args[1])?;
        if breaks.len() < 2 || (breaks.rows() != 1 && breaks.cols() != 1) {
            return Err("Breaks must be a vector with at least two entries.".into());
        }
        if breaks
            .data()
            .windows(2)
            .any(|w| !w[0].is_finite() || w[1] <= w[0])
            || !breaks.data().last().unwrap().is_finite()
        {
            return Err("Breaks must be finite and strictly increasing.".into());
        }
        let dim = if let Some(v) = args.get(2) {
            self.lax_real(v)?.as_scalar()?
        } else {
            1.
        };
        if !dim.is_finite() || dim < 1. || dim.fract() != 0. || dim > 65536. {
            return Err("The dimension must be a positive integer scalar.".into());
        }
        let dim = dim as usize;
        let pieces = breaks.len() - 1;
        let rows = pieces.checked_mul(dim).ok_or("PP size overflow")?;
        // Each piece and output component must have the same positive
        // integer number of coefficients.
        if c.is_empty() || !c.len().is_multiple_of(rows) {
            return Err(format!(
                "The requested number of polynomial pieces, {pieces}, is incompatible with the proposed size, [{dim}], of a coefficient and the number, {}, of scalar coefficients provided.",
                c.len()
            ));
        }
        let order = c.len() / rows;
        self.lax_budget(rows, order, 2)?;
        let breaks = breaks.reshape(1, breaks.len())?;
        // Reshape immutable shared storage in column-major order, including vector-valued pieces.
        let shaped = c.reshape(rows, order)?;
        self.lax_struct(vec![
            (
                "form",
                Array::Character(CharacterRow::from_units(
                    "pp".encode_utf16().collect(),
                    &self.cancel,
                )?),
            ),
            ("breaks", Array::Numeric(breaks)),
            ("coefs", Array::Numeric(shaped)),
            ("pieces", Array::Numeric(Matrix::scalar(pieces as f64))),
            ("order", Array::Numeric(Matrix::scalar(order as f64))),
            ("dim", Array::Numeric(Matrix::scalar(dim as f64))),
        ])
    }
    fn lax_unmkpp(&self, v: &Value) -> Result<Vec<Value>> {
        let error = "The input array does not seem to describe a pp function.";
        let Value::Container(Array::Struct(s)) = v else {
            return Err(error.into());
        };
        if s.len() != 1 {
            return Err(error.into());
        }
        let field = |name: &str| -> Result<Value> {
            Ok(Value::from(
                s.get(s.field_index(name).ok_or(error)?, 0)?
                    .as_ref()
                    .clone(),
            ))
        };
        if lax_text(&field("form")?).as_deref() != Some("pp") {
            return Err(error.into());
        }
        let vals = ["breaks", "coefs", "pieces", "order", "dim"]
            .iter()
            .map(|&key| field(key))
            .collect::<Result<Vec<_>>>()?;
        let breaks = self.lax_real(&vals[0])?;
        let c = self.lax_real(&vals[1])?;
        let pieces = self.lax_real(&vals[2])?.as_scalar()?;
        let order = self.lax_real(&vals[3])?.as_scalar()?;
        let dim = self.lax_real(&vals[4])?.as_scalar()?;
        if [pieces, order, dim]
            .iter()
            .any(|x| !x.is_finite() || *x < 0. || x.fract() != 0.)
            || pieces < 1.
            || order < 1.
            || dim < 1.
            || pieces > breaks.len().saturating_sub(1) as f64
            || breaks.len() != pieces as usize + 1
            || c.rows() as f64 != pieces * dim
            || c.cols() as f64 != order
        {
            return Err(error.into());
        }
        if breaks
            .data()
            .windows(2)
            .any(|w| !w[0].is_finite() || w[1] <= w[0])
            || !breaks.data().last().unwrap().is_finite()
        {
            return Err(error.into());
        }
        Ok(vals)
    }
    fn lax_ppval(&self, args: &[Value]) -> Result<Value> {
        let pp = self.lax_unmkpp(&args[0])?;
        let query = self.lax_real(&args[1])?;
        let breaks = self.lax_real(&pp[0])?;
        let c = self.lax_real(&pp[1])?;
        let dim = self.lax_real(&pp[4])?.as_scalar()? as usize;
        if dim > 1 && query.rows() > 1 && query.cols() > 1 {
            return Err(
                "Vector-valued ppval with a matrix query requires unsupported N-D output.".into(),
            );
        }
        let (rows, cols) = if dim == 1 {
            (query.rows(), query.cols())
        } else {
            (dim, query.len())
        };
        self.lax_budget(rows, cols, 1)?;
        if rows as f64 * cols as f64 * c.cols() as f64 > 4.3e9 {
            return Err("Piecewise polynomial evaluation exceeds the work limit.".into());
        }
        let mut data = Vec::with_capacity(rows * cols);
        for (i, &x) in query.data().iter().enumerate() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let piece = breaks
                .data()
                .partition_point(|&v| v <= x)
                .saturating_sub(1)
                .min(breaks.len() - 2);
            let t = x - breaks.data()[piece];
            for j in 0..dim {
                let mut y = 0.;
                for k in 0..c.cols() {
                    y = y * t + c.get(piece * dim + j, k);
                }
                if x.is_nan() {
                    y = f64::NAN;
                }
                data.push(y);
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
    }
    fn lax_blkdiag(&mut self, args: &[Value]) -> Result<Value> {
        let arrays = args
            .iter()
            .cloned()
            .map(|v| v.array(&self.cancel))
            .collect::<Result<Vec<_>>>()?;
        if arrays.iter().any(|a| {
            !matches!(
                a,
                Array::Numeric(_)
                    | Array::Logical(_)
                    | Array::Complex(_)
                    | Array::Typed(_)
                    | Array::Character(_)
            )
        }) {
            return Err("blkdiag inputs must be numeric, logical or character arrays.".into());
        }
        if arrays.len() == 1 {
            return Ok(Value::from(arrays[0].clone()));
        }
        let rows = arrays.iter().try_fold(0usize, |n, a| {
            n.checked_add(a.rows()).ok_or("Matrix size overflow")
        })?;
        let cols = arrays.iter().try_fold(0usize, |n, a| {
            n.checked_add(a.cols()).ok_or("Matrix size overflow")
        })?;
        self.lax_budget(rows, cols, 2)?;
        if rows == 0 || cols == 0 {
            return Ok(Value::Matrix(Matrix::filled(rows, cols, 0.)?));
        }
        // [A 0 ...; 0 B ...; ...] with double zero padding: the result class
        // and conversions are exactly those of concatenation (int8 with
        // double padding stays int8, char stays char, logical becomes double).
        let mut layout = Vec::with_capacity(arrays.len());
        let mut before = 0;
        for (a, v) in arrays.iter().zip(args) {
            self.check_cancel()?;
            let (m, n) = (a.rows(), a.cols());
            if m > 0 {
                let after = cols - before - n;
                let mut row = Vec::with_capacity(3);
                if before > 0 {
                    row.push(Value::Matrix(Matrix::filled(m, before, 0.)?));
                }
                if n > 0 {
                    row.push(v.clone());
                }
                if after > 0 {
                    row.push(Value::Matrix(Matrix::filled(m, after, 0.)?));
                }
                layout.push(row);
            }
            before += n;
        }
        self.concatenate_values(layout)
    }
    fn lax_rsf2csf(&self, args: &[Value]) -> Result<Vec<Value>> {
        let mut u = self.lax_complex(&args[0])?;
        let mut t = self.lax_complex(&args[1])?;
        let n = t.nrows();
        if t.ncols() != n || u.nrows() != n || u.ncols() != n {
            return Err("Inputs must be square matrices of the same size.".into());
        }
        self.lax_work(n, n)?;
        let mut complex =
            matches!(args[0], Value::Complex(_)) || matches!(args[1], Value::Complex(_));
        for j in 0..n.saturating_sub(1) {
            self.check_cancel()?;
            if t[(j + 1, j)].norm() == 0. {
                continue;
            }
            let (a, b, c, d) = (
                t[(j, j)].re,
                t[(j, j + 1)].re,
                t[(j + 1, j)].re,
                t[(j + 1, j + 1)].re,
            );
            let disc = C64::new((a - d).powi(2) + 4. * b * c, 0.).sqrt();
            let lambda = C64::new((a + d) / 2., 0.) + disc / 2.;
            let mu = lambda - C64::new(d, 0.);
            let norm = mu.norm().hypot(c);
            let g = DMatrix::from_column_slice(
                2,
                2,
                &[
                    mu / norm,
                    C64::new(c / norm, 0.),
                    C64::new(-c / norm, 0.),
                    mu.conj() / norm,
                ],
            );
            let columns = t.columns(j, 2).into_owned() * &g;
            t.columns_mut(j, 2).copy_from(&columns);
            let rows = g.adjoint() * t.rows(j, 2).into_owned();
            t.rows_mut(j, 2).copy_from(&rows);
            let columns = u.columns(j, 2).into_owned() * g;
            u.columns_mut(j, 2).copy_from(&columns);
            t[(j + 1, j)] = C64::new(0., 0.);
            t[(j, j)] = lambda;
            t[(j + 1, j + 1)] = C64::new(a + d, 0.) - lambda;
            if (b + c).abs() <= f64::EPSILON * (b.abs() + c.abs()) && a == d {
                t[(j, j + 1)] = C64::new(0., 0.);
            }
            complex = true;
        }
        Ok(vec![self.lax_cv(&u, complex)?, self.lax_cv(&t, complex)?])
    }
    fn lax_cdf2rdf(&self, args: &[Value]) -> Result<Vec<Value>> {
        let v = self.lax_complex(&args[0])?;
        let d = self.lax_complex(&args[1])?;
        let n = d.nrows();
        if d.ncols() != n || v.nrows() != n || v.ncols() != n {
            return Err("Inputs must be square matrices of the same size.".into());
        }
        self.lax_budget(n, n, 8)?;
        let mut vr = v.map(|z| z.re);
        let mut dr = d.map(|z| z.re);
        let mut j = 0;
        while j < n {
            self.check_cancel()?;
            if d[(j, j)].im == 0. {
                j += 1;
                continue;
            }
            if j + 1 >= n
                || (d[(j + 1, j + 1)] - d[(j, j)].conj()).norm() > 1e-12 * d[(j, j)].norm().max(1.)
            {
                return Err("Complex eigenvalues must occur in adjacent conjugate pairs.".into());
            }
            for i in 0..n {
                vr[(i, j)] = v[(i, j)].re;
                vr[(i, j + 1)] = v[(i, j)].im;
            }
            dr[(j, j + 1)] = d[(j, j)].im;
            dr[(j + 1, j)] = -d[(j, j)].im;
            j += 2;
        }
        Ok(vec![lax_val(&vr)?, lax_val(&dr)?])
    }
    /// Partial fractions b(s)/a(s) = sum_j r_j/(s - p_j)^t_j + k(s) and back
    /// (Oppenheim & Schafer, Discrete-Time Signal Processing, 3rd ed., 2010,
    /// §3.3). Forward: a and b are divided by a's leading coefficient, long
    /// division gives k and the proper remainder; the
    /// poles are the roots of a in decreasing magnitude, and poles within a
    /// relative 1e-3 of each other form one pole of multiplicity m at their
    /// mean. The residues of that pole, r_1..r_m for powers 1..m, are the
    /// Taylor coefficients of (s - p)^m b(s)/a(s) at p from order m-1 down to
    /// 0: the remainder's Taylor coefficients (repeated synthetic division)
    /// divided as power series by those of a's other factors.
    fn lax_residue(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.len() == 3 {
            return self.lax_residue_inverse(args);
        }
        let zero = C64::new(0., 0.);
        let b = self.lax_residue_vector(&args[0])?;
        let a = self.lax_residue_vector(&args[1])?;
        let Some(first) = a.iter().position(|&z| z != zero) else {
            return Err("Index exceeds matrix dimensions.".into());
        };
        // Both polynomials are divided by a's leading coefficient first, so
        // the products below neither overflow nor underflow when a and b
        // are scaled by a large or tiny constant; the residues and k are
        // unchanged by that scale. A quotient that overflows is an error.
        let lead = a[first];
        let monic = |v: &[C64]| -> Result<Vec<C64>> {
            let v: Vec<C64> = v.iter().map(|&z| lax_cdiv(z, lead)).collect();
            if v.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
                return Err("Input to ROOTS must not contain NaN or Inf.".into());
            }
            Ok(v)
        };
        let mut a = monic(&a[first..])?;
        a[0] = C64::new(1., 0.);
        let b = monic(&b[b.iter().position(|&z| z != zero).unwrap_or(b.len())..])?;
        let n = a.len() - 1;
        let mut remainder = b.to_vec();
        let mut k = Vec::new();
        if b.len() > n {
            for i in 0..b.len() - n {
                let q = lax_cdiv(remainder[i], a[0]);
                for (j, &coefficient) in a.iter().enumerate().skip(1) {
                    remainder[i + j] -= lax_cmul(q, coefficient);
                }
                k.push(q);
            }
            remainder.drain(..b.len() - n);
        }
        let a_value = self.lax_cv(&DMatrix::from_row_slice(1, a.len(), &a), false)?;
        // A real denominator takes the real roots path, as `roots` itself does: exact real
        // multiple poles and the public pole order.
        let poles = if matches!(a_value, Value::Complex(_)) {
            self.cxp_call("roots", &[a_value], 1, out)?.remove(0)
        } else {
            self.polyopt_call("roots", &[a_value], out)?
                .ok_or("roots is not available")?
        };
        let roots = match poles {
            Value::Complex(c) => c.data().to_vec(),
            other => self
                .lax_real(&other)?
                .data()
                .iter()
                .map(|&v| C64::new(v, 0.))
                .collect(),
        };
        if roots.len() != n || roots.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
            return Err("Input to ROOTS must not contain NaN or Inf.".into());
        }
        let mut order: Vec<usize> = (0..roots.len()).collect();
        order.sort_by(|&i, &j| roots[j].norm().total_cmp(&roots[i].norm()));
        let mut grouped = vec![false; roots.len()];
        let mut groups: Vec<(C64, usize)> = Vec::new();
        for (position, &i) in order.iter().enumerate() {
            if grouped[i] {
                continue;
            }
            let members: Vec<usize> = std::iter::once(i)
                .chain(
                    order[position + 1..]
                        .iter()
                        .copied()
                        .filter(|&j| !grouped[j] && lax_close_poles(roots[i], roots[j])),
                )
                .collect();
            for &j in &members {
                grouped[j] = true;
            }
            groups.push((lax_pole_mean(&roots, &members), members.len()));
        }
        let mut residues = Vec::with_capacity(n);
        let mut poles = Vec::with_capacity(n);
        for (g, &(pole, m)) in groups.iter().enumerate() {
            self.check_cancel()?;
            // Taylor coefficients at h = 0 of a(1) * prod (h + pole - q)^m_q
            // over the other poles q, to order m-1.
            let mut den = vec![zero; m];
            den[0] = a[0];
            for (h, &(other, multiplicity)) in groups.iter().enumerate() {
                if h == g {
                    continue;
                }
                let shift = pole - other;
                for _ in 0..multiplicity {
                    for t in (1..m).rev() {
                        den[t] = lax_cmul(den[t], shift) + den[t - 1];
                    }
                    den[0] = lax_cmul(den[0], shift);
                }
            }
            // Taylor coefficients of the remainder at the pole, by Horner.
            let mut num = vec![zero; m];
            let mut poly = remainder.clone();
            for value in num.iter_mut() {
                let Some((&lead, rest)) = poly.split_first() else {
                    break;
                };
                let mut quotient = Vec::with_capacity(rest.len());
                let mut acc = lead;
                for &coefficient in rest {
                    quotient.push(acc);
                    acc = lax_cmul(acc, pole) + coefficient;
                }
                *value = acc;
                poly = quotient;
            }
            let mut series = vec![zero; m];
            for t in 0..m {
                let mut s = num[t];
                for l in 1..=t {
                    s -= lax_cmul(den[l], series[t - l]);
                }
                series[t] = lax_cdiv(s, den[0]);
            }
            residues.extend(series.iter().rev());
            poles.extend(std::iter::repeat_n(pole, m));
        }
        Ok(vec![
            self.lax_cv(&DMatrix::from_column_slice(n, 1, &residues), false)?,
            self.lax_cv(&DMatrix::from_column_slice(n, 1, &poles), false)?,
            if k.is_empty() {
                lax_val(&DMatrix::zeros(0, 0))?
            } else {
                self.lax_cv(&DMatrix::from_row_slice(1, k.len(), &k), false)?
            },
        ])
    }
    /// [b, a] = residue(r, p, k): a = prod (s - p_j); each group of equal
    /// poles adds sum_t r_t * a(s)/(s - p)^t, reusing the previous quotient
    /// for the next power, and k(s)*a(s) is the direct part. Conjugate-closed
    /// poles (and residues, with a real k) give real coefficients.
    fn lax_residue_inverse(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        let zero = C64::new(0., 0.);
        let r = self.lax_residue_vector(&args[0])?;
        let p = self.lax_residue_vector(&args[1])?;
        let k = self.lax_residue_vector(&args[2])?;
        if r.len() != p.len() {
            return Err("The residue and pole vectors must have the same length.".into());
        }
        let n = p.len();
        let mut grouped = vec![false; n];
        let mut groups: Vec<(C64, Vec<usize>)> = Vec::new();
        for i in 0..n {
            if grouped[i] {
                continue;
            }
            let members: Vec<usize> = (i..n)
                .filter(|&j| !grouped[j] && (j == i || lax_close_poles(p[i], p[j])))
                .collect();
            for &j in &members {
                grouped[j] = true;
            }
            groups.push((lax_pole_mean(&p, &members), members));
        }
        let roots: Vec<C64> = groups
            .iter()
            .flat_map(|(pole, members)| std::iter::repeat_n(*pole, members.len()))
            .collect();
        let mut a = lax_poly_from_roots(&roots, &self.cancel)?;
        let mut numerator = vec![zero; n];
        for (pole, members) in &groups {
            let mut quotient = a.clone();
            for &j in members {
                self.check_cancel()?;
                quotient = lax_synthetic(&quotient, *pole);
                let offset = n - quotient.len();
                for (v, &q) in numerator[offset..].iter_mut().zip(&quotient) {
                    *v += lax_cmul(r[j], q);
                }
            }
        }
        let mut b = if k.is_empty() {
            numerator
        } else {
            let mut b = lax_convolve(&k, &a, &self.cancel)?;
            let offset = b.len() - n;
            for (v, &q) in b[offset..].iter_mut().zip(&numerator) {
                *v += q;
            }
            b
        };
        if lax_conjugate_closed(&roots, &roots) {
            a.iter_mut().for_each(|z| z.im = 0.);
            // (power, pole, residue) of every term; conjugate terms share a power.
            let terms: Vec<(usize, C64, C64)> = groups
                .iter()
                .flat_map(|(pole, members)| {
                    members.iter().enumerate().map(|(t, &j)| (t, *pole, r[j]))
                })
                .collect();
            let levels = groups.iter().map(|(_, m)| m.len()).max().unwrap_or(0);
            let real_b = k.iter().all(|z| z.im == 0.)
                && (0..levels).all(|t| {
                    let (poles, values): (Vec<C64>, Vec<C64>) = terms
                        .iter()
                        .filter(|term| term.0 == t)
                        .map(|term| (term.1, term.2))
                        .unzip();
                    lax_conjugate_closed(&poles, &values)
                });
            if real_b {
                b.iter_mut().for_each(|z| z.im = 0.);
            }
        }
        let lead = b
            .iter()
            .position(|&z| z != zero)
            .unwrap_or(b.len().saturating_sub(1));
        let b = &b[lead..];
        Ok(vec![
            self.lax_cv(&DMatrix::from_row_slice(1, b.len(), b), false)?,
            self.lax_cv(&DMatrix::from_row_slice(1, a.len(), &a), false)?,
        ])
    }
    /// A residue argument as a finite vector of at most 4097 entries.
    fn lax_residue_vector(&self, v: &Value) -> Result<Vec<C64>> {
        let x = self.lax_complex(v)?;
        if x.nrows() > 1 && x.ncols() > 1 {
            return Err("Inputs must be vectors.".into());
        }
        if x.len() > 4097 {
            return Err("residue supports vectors of at most 4097 entries".into());
        }
        if x.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
            return Err("Input must not contain NaN or Inf.".into());
        }
        Ok(x.as_slice().to_vec())
    }
}
/// Poles closer than 1e-3 relative to the larger magnitude are one pole.
fn lax_close_poles(p: C64, q: C64) -> bool {
    (p - q).norm() <= 1e-3 * p.norm().max(q.norm())
}
/// The mean of a cluster of poles, as the first pole plus the mean offset
/// (exactly the pole itself for equal members).
fn lax_pole_mean(poles: &[C64], members: &[usize]) -> C64 {
    let first = poles[members[0]];
    let offset: C64 = members.iter().map(|&j| poles[j] - first).sum();
    first + offset / members.len() as f64
}
/// Complex product in which a real factor scales both parts, as for a real
/// operand of `.*` (no 0*Inf terms from a zero imaginary part).
fn lax_cmul(a: C64, b: C64) -> C64 {
    if a.im == 0. {
        C64::new(a.re * b.re, a.re * b.im)
    } else if b.im == 0. {
        C64::new(a.re * b.re, a.im * b.re)
    } else {
        a * b
    }
}
/// Complex quotient; a real divisor divides both parts.
fn lax_cdiv(a: C64, b: C64) -> C64 {
    if b.im == 0. {
        C64::new(a.re / b.re, a.im / b.re)
    } else {
        a / b
    }
}
/// Y + c*I on the diagonal of a square Y, in place when Y is not shared.
fn lax_add_diagonal(mut y: Matrix, c: f64) -> Result<Matrix> {
    let n = y.rows();
    let changes: Vec<(usize, f64)> = (0..n).map(|i| (i * n + i, y.get(i, i) + c)).collect();
    if n <= 8192 && y.write_unique(&changes) {
        return Ok(y);
    }
    let mut data = y.data().to_vec();
    for (i, v) in changes {
        data[i] = v;
    }
    Matrix::from_column_vec(n, n, data)
}
fn lax_convolve(a: &[C64], b: &[C64], cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<C64>> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    let mut c = vec![C64::new(0., 0.); a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        crate::dsp::check_cancel(cancel)?;
        for (j, &y) in b.iter().enumerate() {
            c[i + j] += x * y;
        }
    }
    Ok(c)
}
/// The monic polynomial with the given roots: multiply by (s - root) one
/// root at a time, c(i) <- c(i) - root*c(i-1) from the top down.
fn lax_poly_from_roots(roots: &[C64], cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<C64>> {
    let mut c = vec![C64::new(0., 0.); roots.len() + 1];
    c[0] = C64::new(1., 0.);
    for (j, &root) in roots.iter().enumerate() {
        if j % 64 == 0 {
            crate::dsp::check_cancel(cancel)?;
        }
        for i in (1..=j + 1).rev() {
            let t = lax_cmul(root, c[i - 1]);
            c[i] -= t;
        }
    }
    Ok(c)
}
/// Whether the (pole, value) pairs are closed under complex conjugation as
/// a multiset, so that sums over them are real. Signed zeros compare equal.
fn lax_conjugate_closed(poles: &[C64], values: &[C64]) -> bool {
    let key = |p: C64, v: C64| [p.re + 0., p.im + 0., v.re + 0., v.im + 0.];
    let order = |x: &[f64; 4], y: &[f64; 4]| {
        x.iter()
            .zip(y)
            .map(|(a, b)| a.total_cmp(b))
            .find(|o| o.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    let mut direct: Vec<[f64; 4]> = poles.iter().zip(values).map(|(&p, &v)| key(p, v)).collect();
    if direct.iter().flatten().any(|x| x.is_nan()) {
        return false;
    }
    let mut mirrored: Vec<[f64; 4]> = poles
        .iter()
        .zip(values)
        .map(|(&p, &v)| key(p.conj(), v.conj()))
        .collect();
    direct.sort_by(order);
    mirrored.sort_by(order);
    direct == mirrored
}
fn lax_synthetic(a: &[C64], root: C64) -> Vec<C64> {
    let mut q = vec![C64::new(0., 0.); a.len() - 1];
    if !q.is_empty() {
        q[0] = a[0];
        for j in 1..q.len() {
            q[j] = a[j] + root * q[j - 1];
        }
    }
    q
}
/// LAPACK dlanv2: the standardized Schur factorization of a real 2x2 block,
/// [a b; c d] = [cs -sn; sn cs] [aa bb; cc dd] [cs sn; -sn cs], where either
/// cc = 0 (real eigenvalues) or aa = dd and bb*cc < 0 (a complex pair).
/// Returns [aa, bb, cc, dd, cs, sn].
fn lax_dlanv2(mut a: f64, mut b: f64, mut c: f64, mut d: f64) -> [f64; 6] {
    let eps = f64::EPSILON;
    // BASE**INT(LOG(SAFMIN/EPS)/LOG(BASE)/2) for IEEE double.
    let safmn2 = 2f64.powi(-485);
    let safmx2 = 1. / safmn2;
    let sign = |x: f64| 1f64.copysign(x);
    let (mut cs, mut sn);
    if c == 0. {
        cs = 1.;
        sn = 0.;
    } else if b == 0. {
        // Swap rows and columns.
        cs = 0.;
        sn = 1.;
        std::mem::swap(&mut a, &mut d);
        b = -c;
        c = 0.;
    } else if a - d == 0. && sign(b) != sign(c) {
        cs = 1.;
        sn = 0.;
    } else {
        let mut temp = a - d;
        let mut p = 0.5 * temp;
        let bcmax = b.abs().max(c.abs());
        let bcmis = b.abs().min(c.abs()) * sign(b) * sign(c);
        let scale = p.abs().max(bcmax);
        let mut z = (p / scale) * p + (bcmax / scale) * bcmis;
        if z >= 4. * eps {
            // Real eigenvalues: compute a and d.
            z = p + (scale.sqrt() * z.sqrt()).copysign(p);
            a = d + z;
            d -= (bcmax / z) * bcmis;
            let tau = c.hypot(z);
            cs = z / tau;
            sn = c / tau;
            b -= c;
            c = 0.;
        } else {
            // Complex or almost equal real eigenvalues: make the diagonal equal.
            let mut sigma = b + c;
            for _ in 0..=20 {
                let scale = temp.abs().max(sigma.abs());
                if scale >= safmx2 {
                    sigma *= safmn2;
                    temp *= safmn2;
                } else if scale <= safmn2 {
                    sigma *= safmx2;
                    temp *= safmx2;
                } else {
                    break;
                }
            }
            p = 0.5 * temp;
            let tau = sigma.hypot(temp);
            cs = (0.5 * (1. + sigma.abs() / tau)).sqrt();
            sn = -(p / (tau * cs)) * sign(sigma);
            // [aa bb; cc dd] = [a b; c d] [cs -sn; sn cs]
            let aa = a * cs + b * sn;
            let bb = -a * sn + b * cs;
            let cc = c * cs + d * sn;
            let dd = -c * sn + d * cs;
            // [a b; c d] = [cs sn; -sn cs] [aa bb; cc dd]
            a = aa * cs + cc * sn;
            b = bb * cs + dd * sn;
            c = -aa * sn + cc * cs;
            d = -bb * sn + dd * cs;
            let temp = 0.5 * (a + d);
            a = temp;
            d = temp;
            if c != 0. {
                if b == 0. {
                    b = -c;
                    c = 0.;
                    let temp = cs;
                    cs = -sn;
                    sn = temp;
                } else if sign(b) == sign(c) {
                    // Real eigenvalues: reduce to upper triangular form.
                    let sab = b.abs().sqrt();
                    let sac = c.abs().sqrt();
                    p = (sab * sac).copysign(c);
                    let tau = 1. / (b + c).abs().sqrt();
                    a = temp + p;
                    d = temp - p;
                    b -= c;
                    c = 0.;
                    let cs1 = sab * tau;
                    let sn1 = sac * tau;
                    let temp = cs * cs1 - sn * sn1;
                    sn = cs * sn1 + sn * cs1;
                    cs = temp;
                }
            }
        }
    }
    [a, b, c, d, cs, sn]
}
/// Standardize the 2x2 block of T at (j, j) with dlanv2 and apply the
/// rotation to the rest of T and to Q, as LAPACK dlahqr does (DROT).
fn lax_standardize_block(q: &mut DMatrix<f64>, t: &mut DMatrix<f64>, j: usize) {
    let n = t.nrows();
    let [a, b, c, d, cs, sn] =
        lax_dlanv2(t[(j, j)], t[(j, j + 1)], t[(j + 1, j)], t[(j + 1, j + 1)]);
    t[(j, j)] = a;
    t[(j, j + 1)] = b;
    t[(j + 1, j)] = c;
    t[(j + 1, j + 1)] = d;
    let rotate = |x: f64, y: f64| (cs * x + sn * y, cs * y - sn * x);
    for k in j + 2..n {
        (t[(j, k)], t[(j + 1, k)]) = rotate(t[(j, k)], t[(j + 1, k)]);
    }
    for i in 0..j {
        (t[(i, j)], t[(i, j + 1)]) = rotate(t[(i, j)], t[(i, j + 1)]);
    }
    for i in 0..n {
        (q[(i, j)], q[(i, j + 1)]) = rotate(q[(i, j)], q[(i, j + 1)]);
    }
}
/// Scaled 2-norm of a complex vector (dznrm2).
fn lax_znrm2(x: &[C64]) -> f64 {
    let scale = x
        .iter()
        .fold(0f64, |m, z| m.max(z.re.abs()).max(z.im.abs()));
    if scale == 0. {
        return 0.;
    }
    scale
        * x.iter()
            .map(|z| (z.re / scale).powi(2) + (z.im / scale).powi(2))
            .sum::<f64>()
            .sqrt()
}
/// LAPACK zlarfg on x = [alpha, rest]: H' [alpha; rest] = [beta; 0] with real
/// beta and H = I - tau v v'. Stores v (v0 = 1 implied) in rest and beta in
/// x[0]; returns (tau, beta). A real alpha with a zero rest gives H = I.
fn lax_zlarfg(x: &mut [C64]) -> (C64, f64) {
    let mut alpha = x[0];
    let mut xnorm = lax_znrm2(&x[1..]);
    if xnorm == 0. && alpha.im == 0. {
        return (C64::new(0., 0.), alpha.re);
    }
    let dlapy3 = |x: f64, y: f64, z: f64| {
        let w = x.abs().max(y.abs()).max(z.abs());
        if w == 0. {
            0.
        } else {
            w * ((x / w).powi(2) + (y / w).powi(2) + (z / w).powi(2)).sqrt()
        }
    };
    let mut beta = -dlapy3(alpha.re, alpha.im, xnorm).copysign(alpha.re);
    let safmin = f64::MIN_POSITIVE / (f64::EPSILON / 2.);
    let mut knt = 0;
    if beta.abs() < safmin {
        let rsafmn = 1. / safmin;
        while beta.abs() < safmin && knt < 20 {
            knt += 1;
            for v in &mut x[1..] {
                *v *= rsafmn;
            }
            beta *= rsafmn;
            alpha *= rsafmn;
        }
        xnorm = lax_znrm2(&x[1..]);
        beta = -dlapy3(alpha.re, alpha.im, xnorm).copysign(alpha.re);
    }
    let tau = C64::new((beta - alpha.re) / beta, -alpha.im / beta);
    let scale = C64::new(1., 0.) / (alpha - beta);
    for v in &mut x[1..] {
        *v *= scale;
    }
    for _ in 0..knt {
        beta *= safmin;
    }
    x[0] = C64::new(beta, 0.);
    (tau, beta)
}

impl Engine {
    /// Real double division. Preserve the scalar and rectangular Matrix paths.
    pub(super) fn real_divide(
        &mut self,
        backslash: bool,
        a: &Matrix,
        b: &Matrix,
        out: &mut Execution,
    ) -> Result<Matrix> {
        let divisor = if backslash { a } else { b };
        if divisor.len() == 1 {
            let numerator = if backslash { b } else { a };
            return Ok(numerator.map(|v| v / divisor.data()[0]));
        }
        if divisor.rows() != divisor.cols() {
            return if backslash {
                a.solve(b)
            } else {
                Ok(b.transpose().solve(&a.transpose())?.transpose())
            };
        }
        if backslash {
            self.real_square_solve(a, b, false, out)
        } else {
            Ok(self
                .real_square_solve(&b.transpose(), &a.transpose(), false, out)?
                .transpose())
        }
    }

    pub(super) fn real_inverse(&mut self, a: &Matrix, out: &mut Execution) -> Result<Matrix> {
        lax_square(a, "Matrix must be square.")?;
        self.real_square_solve(a, &Matrix::identity(a.rows(), a.cols())?, true, out)
    }

    /// Square solve/inverse with the complex path's warning and substitution
    /// policy. The dlacn2 estimator and LU warnings are shared with linsolve.
    fn real_square_solve(
        &mut self,
        a: &Matrix,
        b: &Matrix,
        inverse: bool,
        out: &mut Execution,
    ) -> Result<Matrix> {
        let n = a.rows();
        if b.rows() != n {
            return Err("Linear system row dimensions must agree".into());
        }
        // No size cap beyond Matrix::solve's: large well-conditioned systems
        // solved before these warnings existed.
        self.check_cancel()?;
        if n == 0 {
            return Matrix::from_column_vec(0, b.cols(), Vec::new());
        }
        // As the complex inverse: a finite nonzero scalar uses 1/a directly.
        if inverse && n == 1 && a.data()[0].is_finite() && a.data()[0] != 0. {
            return Ok(Matrix::scalar(1. / a.data()[0]));
        }
        let ad = lax_dm(a);
        let bd = lax_dm(b);
        let lower = (0..n).all(|j| (0..j).all(|i| ad[(i, j)] == 0.));
        let upper = (0..n).all(|j| (j + 1..n).all(|i| ad[(i, j)] == 0.));
        if !inverse && (lower || upper) {
            let x = self.real_substitute(&ad, &bd, lower)?;
            if (0..n).any(|i| ad[(i, i)] == 0.) {
                self.lax_singular_warning(out)?;
            } else if ad.iter().all(|v| v.is_finite()) {
                let estimate =
                    self.lax_inverse_norm1(n, |v, transpose| match (lower, transpose) {
                        (true, false) => ad.solve_lower_triangular(v),
                        (true, true) => ad.tr_solve_lower_triangular(v),
                        (false, false) => ad.solve_upper_triangular(v),
                        (false, true) => ad.tr_solve_upper_triangular(v),
                    })?;
                self.real_condition_warning(lax_norm1(&ad), estimate, out)?;
            } else {
                self.lax_rcond_warning(f64::NAN, out)?;
            }
            return lax_mx(&x);
        }
        let lu = ad.clone().lu();
        self.check_cancel()?;
        let x = if let Some(x) = lu.solve(&bd) {
            if ad.iter().all(|v| v.is_finite()) {
                let estimate = self.lax_lu_inverse_norm1(n, &lu)?;
                self.real_condition_warning(lax_norm1(&ad), estimate, out)?;
            } else {
                self.lax_rcond_warning(f64::NAN, out)?;
            }
            x
        } else {
            self.lax_singular_warning(out)?;
            if inverse {
                DMatrix::from_element(n, n, f64::INFINITY)
            } else {
                // As cxl_singular_solve: L has a unit diagonal; an exact zero
                // pivot of U becomes NaN, then substitution continues.
                let mut rhs = bd;
                lu.p().permute_rows(&mut rhs);
                let l = lu.l();
                for c in 0..rhs.ncols() {
                    for k in 0..n {
                        if k % 256 == 0 {
                            self.check_cancel()?;
                        }
                        let v = rhs[(k, c)];
                        for i in k + 1..n {
                            rhs[(i, c)] -= v * l[(i, k)];
                        }
                    }
                }
                let u = lu.u();
                for c in 0..rhs.ncols() {
                    for k in (0..n).rev() {
                        if k % 256 == 0 {
                            self.check_cancel()?;
                        }
                        let v = if u[(k, k)] == 0. {
                            f64::NAN
                        } else {
                            rhs[(k, c)] / u[(k, k)]
                        };
                        rhs[(k, c)] = v;
                        for i in 0..k {
                            rhs[(i, c)] -= v * u[(i, k)];
                        }
                    }
                }
                rhs
            }
        };
        lax_mx(&x)
    }

    /// dtrsm column order, including division by a zero diagonal. Lower is
    /// checked first, as on the complex path (zeros(2)\[1;2] is [Inf;NaN]).
    fn real_substitute(
        &self,
        t: &DMatrix<f64>,
        b: &DMatrix<f64>,
        lower: bool,
    ) -> Result<DMatrix<f64>> {
        let n = t.nrows();
        let mut x = b.clone();
        for c in 0..x.ncols() {
            for step in 0..n {
                if step % 256 == 0 {
                    self.check_cancel()?;
                }
                let k = if lower { step } else { n - 1 - step };
                let v = x[(k, c)] / t[(k, k)];
                x[(k, c)] = v;
                let rows = if lower { k + 1..n } else { 0..k };
                for i in rows {
                    x[(i, c)] -= v * t[(i, k)];
                }
            }
        }
        Ok(x)
    }

    /// Match cxl_condition_warning's NaN policy without changing linsolve.
    fn real_condition_warning(
        &mut self,
        norm: f64,
        inverse_norm: Option<f64>,
        out: &mut Execution,
    ) -> Result<()> {
        if norm.is_nan() || inverse_norm.is_some_and(f64::is_nan) {
            self.lax_rcond_warning(f64::NAN, out)
        } else {
            self.lax_rcond_warning(lax_rcond(norm, inverse_norm), out)
        }
    }

    /// A^-k (k >= 1) as inv(A)^k, starting from the inverse itself: an
    /// identity start would turn a singular all-Inf inverse into NaN (0*Inf).
    pub(super) fn real_negative_power(
        &mut self,
        a: &Matrix,
        k: u64,
        out: &mut Execution,
    ) -> Result<Matrix> {
        let mut base = self.real_inverse(a, out)?;
        let mut result: Option<Matrix> = None;
        let mut n = k;
        loop {
            if n & 1 == 1 {
                result = Some(match result {
                    Some(r) => r.mul(&base)?,
                    None => base.clone(),
                });
            }
            n >>= 1;
            if n == 0 {
                break;
            }
            self.check_cancel()?;
            base = base.mul(&base)?;
        }
        Ok(result.unwrap_or(base))
    }

    /// The square-solve warnings with the reference's identifiers, issued through
    /// warning() so warning('off', id) and lastwarn apply (real and complex).
    pub(super) fn lax_singular_warning(&mut self, out: &mut Execution) -> Result<()> {
        self.lax_identified_warning(
            "MATLAB:singularMatrix",
            "Matrix is singular to working precision.",
            out,
        )
    }

    /// NaN: the non-finite wording; below eps: the close-to-singular one.
    pub(super) fn lax_rcond_warning(&mut self, rcond: f64, out: &mut Execution) -> Result<()> {
        if rcond.is_nan() {
            self.lax_identified_warning(
                "MATLAB:illConditionedMatrix",
                "Matrix is singular, close to singular or badly scaled. Results may be inaccurate. RCOND = NaN.",
                out,
            )
        } else if rcond < f64::EPSILON {
            self.lax_identified_warning(
                "MATLAB:nearlySingularMatrix",
                &lax_close_message(rcond),
                out,
            )
        } else {
            Ok(())
        }
    }

    fn lax_identified_warning(
        &mut self,
        id: &str,
        message: &str,
        out: &mut Execution,
    ) -> Result<()> {
        // With an identifier, warning formats its message: pass it as data.
        let args = [id, "%s", message].map(|s| Value::Text(s.into()));
        self.lang_utils_call("warning", &args, out)?;
        Ok(())
    }
}
