//! lu, qr, chol, pinv, null, orth, expm, logm, sqrtm, mldivide, mrdivide, cond and norm. Real double inputs only (logical accepted); complex inputs are rejected explicitly.
//! Dense kernels: nalgebra LU/SVD/Schur; Householder QR and Cholesky follow the LAPACK
//! conventions (dlarfg signs, dlaqp2 column-norm pivoting) so Q/R signs agree with MATLAB.
use super::{Engine, Execution, Value};
use crate::{Array, ComplexMatrix, Matrix, Result};
use nalgebra::{Complex, DMatrix, linalg::SVD};

type C64 = Complex<f64>;

const NAMES: &[&str] = &[
    "lu", "qr", "chol", "pinv", "null", "orth", "expm", "logm", "sqrtm", "mldivide", "mrdivide",
    "cond", "norm",
];
/// Bound on m*n*min(m,n): about a 1600-square factorization.
const MAX_WORK: f64 = 4.3e9;
const BUDGET: usize = 32 * 1024 * 1024;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn eps_of(x: f64) -> f64 {
    let x = x.abs();
    f64::from_bits(x.to_bits() + 1) - x
}
fn dm(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}
fn mx(d: &DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_vec(d.nrows(), d.ncols(), d.as_slice().to_vec())
}
fn val(d: &DMatrix<f64>) -> Result<Value> {
    Ok(Value::Matrix(mx(d)?))
}
fn norm1(d: &DMatrix<f64>) -> f64 {
    d.column_iter()
        .map(|c| c.iter().map(|x| x.abs()).sum::<f64>())
        .fold(0.0, |a, b| if b.is_nan() || b > a { b } else { a })
}
fn square(m: &Matrix) -> Result<usize> {
    if m.rows() == m.cols() {
        Ok(m.rows())
    } else {
        Err("Matrix must be square.".into())
    }
}
fn finite(m: &Matrix) -> bool {
    m.data().iter().all(|v| v.is_finite())
}

impl Engine {
    /// Division needs the execution output for numerical warnings; the other
    /// factorizations remain usable by immutable internal callers.
    pub(super) fn linalg2_call_with_output(
        &mut self,
        name: &str,
        args: &[Value],
        outputs: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        if !matches!(name, "mldivide" | "mrdivide") {
            return self.linalg2_call(name, args, outputs);
        }
        self.check_cancel()?;
        if outputs > 1 {
            return Err("Too many output arguments.".into());
        }
        if args.len() != 2 {
            return Err(if args.len() < 2 {
                "Not enough input arguments."
            } else {
                "Too many input arguments."
            }
            .into());
        }
        let a = self.linalg2_real(name, &args[0])?;
        let b = self.linalg2_real(name, &args[1])?;
        self.linalg2_reserve(&[(a.rows(), a.cols()), (b.rows(), b.cols())])?;
        Ok(Some(vec![Value::Matrix(self.real_divide(
            name == "mldivide",
            &a,
            &b,
            out,
        )?)]))
    }

    /// `outputs` is the requested count (at least 1). Returns None for names of other families.
    pub(super) fn linalg2_call(
        &self,
        name: &str,
        args: &[Value],
        outputs: usize,
    ) -> Result<Option<Vec<Value>>> {
        if !handles(name) {
            return Ok(None);
        }
        self.check_cancel()?;
        let max_outputs = match name {
            "lu" | "qr" => 3,
            "chol" => 2,
            _ => 1,
        };
        if outputs > max_outputs {
            return Err("Too many output arguments.".into());
        }
        let (min_args, max_args) = match name {
            "lu" | "pinv" | "cond" | "norm" | "chol" | "qr" => (1, 2),
            "null" | "orth" | "expm" | "logm" | "sqrtm" => (1, 1),
            _ => (2, 2),
        };
        if args.len() < min_args {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > max_args {
            return Err("Too many input arguments.".into());
        }
        if name == "lu" && matches!(args[0], Value::Complex(_)) {
            return self.cxl_lu(args, outputs).map(Some);
        }
        if matches!(name, "qr" | "chol") && matches!(args[0], Value::Complex(_)) {
            return self.cxl_factor_call(name, args, outputs).map(Some);
        }
        if matches!(name, "pinv" | "null" | "orth" | "expm" | "cond" | "norm")
            && matches!(args[0], Value::Complex(_))
        {
            return Ok(Some(vec![self.cxl_linalg2_call(name, args)?]));
        }
        let first = self.linalg2_real(name, &args[0])?;
        let mut inputs = vec![first];
        if matches!(name, "mldivide" | "mrdivide") {
            for v in &args[1..] {
                let m = self.linalg2_real(name, v)?;
                inputs.push(m);
            }
        }
        // Inputs were charged when evaluated; only new work buffers are checked here.
        // Outputs of the other functions are reserved in their own routines.
        let (ar, ac) = (inputs[0].rows(), inputs[0].cols());
        match name {
            "expm" => self.linalg2_reserve(&[(ar, ac); 8])?,
            "logm" | "sqrtm" => self.linalg2_reserve(&[(ar, ac); 5])?,
            "mldivide" => {
                self.linalg2_reserve(&[(ar, ac), (inputs[1].rows(), inputs[1].cols())])?
            }
            "mrdivide" => {
                self.linalg2_reserve(&[(ar, ac), (inputs[1].rows(), inputs[1].cols())])?
            }
            _ => {}
        }
        let a = &inputs[0];
        let option = |i: usize| -> Result<Option<String>> {
            args.get(i).map(|v| self.linalg2_text(name, v)).transpose()
        };
        let one = |v: Value| Ok(Some(vec![v]));
        match name {
            "lu" => {
                let vector = match option(1)? {
                    Some(s) if s == "vector" => true,
                    Some(s) if s == "matrix" => false,
                    Some(_) => return Err("Unknown lu option: use 'matrix' or 'vector'.".into()),
                    None if args.len() == 2 => {
                        return Err("Unknown lu option: use 'matrix' or 'vector'.".into());
                    }
                    None => false,
                };
                self.linalg2_lu(a, outputs, vector).map(Some)
            }
            "qr" => {
                let (economy, vector) = match args.get(1) {
                    None => (false, false),
                    Some(
                        v @ (Value::Text(_)
                        | Value::QuotedString(_)
                        | Value::Container(Array::Character(_))),
                    ) => match self.linalg2_text(name, v)?.as_str() {
                        "vector" => (false, true),
                        "matrix" => (false, false),
                        _ => return Err("Unknown qr option: use 'matrix' or 'vector'.".into()),
                    },
                    Some(v) => {
                        let z = self.linalg2_real(name, v)?;
                        if z.len() != 1 || z.data()[0] != 0.0 {
                            return Err("Second argument must be 0 (economy-size).".into());
                        }
                        (true, true)
                    }
                };
                self.linalg2_qr(a, outputs, economy, vector).map(Some)
            }
            "chol" => {
                let lower = match option(1)? {
                    None => false,
                    Some(s) if s.eq_ignore_ascii_case("lower") => true,
                    Some(s) if s.eq_ignore_ascii_case("upper") => false,
                    Some(_) => return Err("Unknown option: use 'lower' or 'upper'.".into()),
                };
                self.linalg2_chol(a, outputs, lower).map(Some)
            }
            "pinv" => {
                let tol = match args.get(1) {
                    None => None,
                    Some(v) => {
                        let t = self.linalg2_real(name, v)?;
                        if t.len() != 1 {
                            return Err("Tolerance must be a scalar.".into());
                        }
                        Some(t.data()[0])
                    }
                };
                one(Value::Matrix(self.linalg2_pinv(a, tol)?))
            }
            "null" | "orth" => one(Value::Matrix(self.linalg2_basis(a, name == "null")?)),
            "expm" => one(Value::Matrix(self.linalg2_expm(a)?)),
            "sqrtm" | "logm" => one(self.linalg2_schur_function(a, name == "logm")?),
            "mldivide" => one(Value::Matrix(inputs[0].solve(&inputs[1])?)),
            "mrdivide" => {
                let (a, b) = (&inputs[0], &inputs[1]);
                one(Value::Matrix(if b.len() == 1 {
                    a.map(|x| x / b.data()[0])
                } else {
                    b.transpose().solve(&a.transpose())?.transpose()
                }))
            }
            "cond" => {
                let p = self.linalg2_norm_kind(args.get(1), name)?;
                one(Value::Matrix(Matrix::scalar(self.linalg2_cond(a, p)?)))
            }
            _ => {
                let p = self.linalg2_norm_kind(args.get(1), name)?;
                one(Value::Matrix(Matrix::scalar(self.linalg2_norm(a, p)?)))
            }
        }
    }

    fn linalg2_real(&self, name: &str, v: &Value) -> Result<Matrix> {
        match v {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(m) => m.to_numeric(&self.cancel),
            Value::Complex(_) => Err(format!("{name}: complex inputs are not supported yet")),
            Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)) => Err(
                format!("Undefined function '{name}' for input arguments of type 'char'."),
            ),
            _ => Err(format!(
                "Undefined function '{name}' for input arguments of this type."
            )),
        }
    }
    fn linalg2_text(&self, name: &str, v: &Value) -> Result<String> {
        match v {
            Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
            Value::Container(Array::Character(a)) => a.to_utf8(&self.cancel),
            _ => Err(format!("{name}: the option must be a character vector")),
        }
    }
    fn linalg2_work(&self, m: usize, n: usize) -> Result<()> {
        let work = (m as f64) * (n as f64) * (m.min(n) as f64);
        if work > MAX_WORK {
            return Err("The factorization exceeds the dense work limit of this version".into());
        }
        Ok(())
    }
    /// Checks result shapes against the Matrix element limit and the statement budget before
    /// anything of that size is allocated.
    fn linalg2_reserve(&self, shapes: &[(usize, usize)]) -> Result<()> {
        let mut total = self.evaluated_elements;
        for &(rows, cols) in shapes {
            total = total.saturating_add(Matrix::checked_len(rows, cols)?);
        }
        if total > BUDGET {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    // ---- lu -------------------------------------------------------------
    fn linalg2_lu(&self, a: &Matrix, outputs: usize, vector: bool) -> Result<Vec<Value>> {
        let (m, n) = (a.rows(), a.cols());
        let k = m.min(n);
        self.linalg2_work(m, n)?;
        let p_shape = if vector { (1, m) } else { (m, m) };
        self.linalg2_reserve(&match outputs {
            1 => vec![(m, n)],
            2 => vec![(m, k), (k, n)],
            _ => vec![(m, k), (k, n), p_shape],
        })?;
        if k == 0 {
            let l = DMatrix::zeros(m, k);
            let u = DMatrix::zeros(k, n);
            return Ok(match outputs {
                1 => vec![val(&DMatrix::zeros(m, n))?],
                2 => vec![val(&l)?, val(&u)?],
                _ => vec![val(&l)?, val(&u)?, self.linalg2_perm(m, &[], vector)?],
            });
        }
        let lu = dm(a).lu();
        let (mut l, u) = (lu.l(), lu.u());
        // order[i] = original row now at row i, so that P*A = L*U with P = I(order,:)
        let mut idx = DMatrix::from_fn(m, 1, |i, _| i as f64);
        lu.p().permute_rows(&mut idx);
        let order: Vec<usize> = idx.iter().map(|&v| v as usize).collect();
        Ok(match outputs {
            1 => {
                let y = DMatrix::from_fn(m, n, |i, j| if i <= j { u[(i, j)] } else { l[(i, j)] });
                vec![val(&y)?]
            }
            2 => {
                lu.p().inv_permute_rows(&mut l);
                vec![val(&l)?, val(&u)?]
            }
            _ => vec![val(&l)?, val(&u)?, self.linalg2_perm(m, &order, vector)?],
        })
    }
    fn linalg2_perm(&self, m: usize, order: &[usize], vector: bool) -> Result<Value> {
        let order: Vec<usize> = if order.is_empty() {
            (0..m).collect()
        } else {
            order.to_vec()
        };
        if vector {
            let v: Vec<f64> = order.iter().map(|&i| i as f64 + 1.0).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(1, m, v)?));
        }
        let mut p = DMatrix::zeros(m, m);
        for (i, &j) in order.iter().enumerate() {
            p[(i, j)] = 1.0;
        }
        val(&p)
    }

    // ---- qr -------------------------------------------------------------
    fn linalg2_qr(
        &self,
        a: &Matrix,
        outputs: usize,
        economy: bool,
        vector: bool,
    ) -> Result<Vec<Value>> {
        let (m, n) = (a.rows(), a.cols());
        self.linalg2_work(m, n.max(1))?;
        let pivot = outputs == 3;
        let k = m.min(n);
        let (r_rows, qc) = if economy { (k, k) } else { (m, m) };
        let e_shape = if vector { (1, n) } else { (n, n) };
        self.linalg2_reserve(&match outputs {
            1 => vec![(m, n)],
            2 => vec![(r_rows, n), (m, qc)],
            _ => vec![(r_rows, n), (m, qc), e_shape],
        })?;
        let mut h = a.data().to_vec();
        let mut tau = vec![0.0; k];
        let mut jpvt: Vec<usize> = (0..n).collect();
        let mut vn1: Vec<f64> = (0..n).map(|j| nrm2(&h[j * m..(j + 1) * m])).collect();
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
                    for r in 0..m {
                        h.swap(best * m + r, i * m + r);
                    }
                    jpvt.swap(best, i);
                    vn1[best] = vn1[i];
                    vn2[best] = vn2[i];
                }
            }
            tau[i] = larfg(&mut h[i * m + i..(i + 1) * m]);
            if tau[i] != 0.0 {
                for j in i + 1..n {
                    apply_reflector(&mut h, m, i, tau[i], j);
                }
            }
            if pivot {
                for j in i + 1..n {
                    if vn1[j] != 0.0 {
                        let t = (h[j * m + i].abs() / vn1[j]).powi(2);
                        let t = (1.0 - t).max(0.0);
                        let t2 = t * (vn1[j] / vn2[j]).powi(2);
                        if t2 <= tol3z {
                            vn1[j] = nrm2(&h[j * m + i + 1..(j + 1) * m]);
                            vn2[j] = vn1[j];
                        } else {
                            vn1[j] *= t.sqrt();
                        }
                    }
                }
            }
        }
        // One output retains the packed DGEQRF form, including the Householder vectors,
        // even for qr(A,0). Move the existing buffer instead of allocating a triangle.
        if outputs == 1 {
            return Ok(vec![Value::Matrix(Matrix::from_column_vec(m, n, h)?)]);
        }
        // R: upper triangle of h, economy trims to k rows.
        let r = DMatrix::from_fn(r_rows, n, |i, j| if i <= j { h[j * m + i] } else { 0.0 });
        let mut q = DMatrix::from_fn(m, qc, |i, j| if i == j { 1.0 } else { 0.0 });
        {
            let qs = q.as_mut_slice();
            for i in (0..k).rev() {
                self.check_cancel()?;
                if tau[i] == 0.0 {
                    continue;
                }
                let v: Vec<f64> = std::iter::once(1.0)
                    .chain(h[i * m + i + 1..(i + 1) * m].iter().copied())
                    .collect();
                for j in i..qc {
                    let col = &mut qs[j * m + i..(j + 1) * m];
                    let w: f64 = col.iter().zip(&v).map(|(c, x)| c * x).sum::<f64>() * tau[i];
                    for (c, x) in col.iter_mut().zip(&v) {
                        *c -= w * x;
                    }
                }
            }
        }
        let mut out = vec![val(&q)?, val(&r)?];
        if pivot {
            out.push(if vector {
                let v: Vec<f64> = jpvt.iter().map(|&j| j as f64 + 1.0).collect();
                Value::Matrix(Matrix::from_column_vec(1, n, v)?)
            } else {
                let mut e = DMatrix::zeros(n, n);
                for (c, &j) in jpvt.iter().enumerate() {
                    e[(j, c)] = 1.0;
                }
                val(&e)?
            });
        }
        Ok(out)
    }

    // ---- chol -----------------------------------------------------------
    fn linalg2_chol(&self, a: &Matrix, outputs: usize, lower: bool) -> Result<Vec<Value>> {
        let n = square(a)?;
        self.linalg2_work(n, n)?;
        self.linalg2_reserve(&[(n, n)])?;
        // Upper factor of the triangle MATLAB reads: upper for 'upper', transposed lower for 'lower'.
        let src = if lower { a.transpose() } else { a.clone() };
        let s = src.data();
        let mut r = vec![0.0; n * n];
        let mut fail = 0usize;
        'cols: for j in 0..n {
            self.check_cancel()?;
            for i in 0..j {
                let dot: f64 = (0..i).map(|k| r[i * n + k] * r[j * n + k]).sum();
                r[j * n + i] = (s[j * n + i] - dot) / r[i * n + i];
            }
            let d = s[j * n + j] - (0..j).map(|k| r[j * n + k].powi(2)).sum::<f64>();
            if d.is_nan() || d <= 0.0 {
                fail = j + 1;
                break 'cols;
            }
            r[j * n + j] = d.sqrt();
        }
        if fail > 0 && outputs < 2 {
            return Err("Matrix must be positive definite.".into());
        }
        let size = if fail > 0 { fail - 1 } else { n };
        let mut out = DMatrix::from_fn(size, size, |i, j| if i <= j { r[j * n + i] } else { 0.0 });
        if lower {
            out = out.transpose();
        }
        let mut v = vec![val(&out)?];
        if outputs == 2 {
            v.push(Value::Matrix(Matrix::scalar(fail as f64)));
        }
        Ok(v)
    }

    // ---- SVD based ------------------------------------------------------
    fn linalg2_svd(
        &self,
        a: &DMatrix<f64>,
        vectors: bool,
    ) -> Result<SVD<f64, nalgebra::Dyn, nalgebra::Dyn>> {
        if a.iter().any(|v| !v.is_finite()) {
            return Err("Input to SVD must not contain NaN or Inf.".into());
        }
        self.linalg2_work(a.nrows(), a.ncols())?;
        self.check_cancel()?;
        let mut svd = SVD::try_new(a.clone(), vectors, vectors, f64::EPSILON, 0)
            .ok_or("SVD did not converge.")?;
        svd.sort_by_singular_values();
        self.check_cancel()?;
        Ok(svd)
    }
    fn linalg2_pinv(&self, a: &Matrix, tol: Option<f64>) -> Result<Matrix> {
        let (m, n) = (a.rows(), a.cols());
        if m == 0 || n == 0 {
            return Matrix::filled(n, m, 0.0);
        }
        self.linalg2_reserve(&[(m, n), (n, m)])?;
        let svd = self.linalg2_svd(&dm(a), true)?;
        let s = &svd.singular_values;
        let smax = s.iter().copied().fold(0.0, f64::max);
        let tol = tol.unwrap_or(m.max(n) as f64 * eps_of(smax));
        let (u, vt) = (svd.u.as_ref().unwrap(), svd.v_t.as_ref().unwrap());
        let r = s.iter().filter(|&&x| x > tol).count();
        if r == 0 {
            return Matrix::filled(n, m, 0.0);
        }
        let vs = DMatrix::from_fn(n, r, |i, j| vt[(j, i)] / s[j]);
        mx(&(vs * u.columns(0, r).transpose()))
    }
    fn linalg2_basis(&self, a: &Matrix, null: bool) -> Result<Matrix> {
        let (m, n) = (a.rows(), a.cols());
        if m == 0 || n == 0 {
            return if null {
                Matrix::identity(n, n)
            } else {
                Matrix::filled(if n == 0 { m } else { 0 }, 0, 0.0)
            };
        }
        // Pad wide matrices with zero rows so that V is the full n-by-n factor. Bound the padded
        // copy, its U and V and the basis before any of them is allocated.
        let rows = m.max(n);
        self.linalg2_work(rows, n)?;
        self.linalg2_reserve(&[(rows, n), (rows, n), (n, n), (n, n)])?;
        let mut padded = dm(a);
        if m < n {
            padded = padded.insert_rows(m, n - m, 0.0);
        }
        let svd = self.linalg2_svd(&padded, true)?;
        let s = &svd.singular_values;
        let smax = s.iter().copied().fold(0.0, f64::max);
        let tol = rows as f64 * smax * f64::EPSILON;
        let r = s.iter().filter(|&&x| x > tol).count();
        if null {
            let vt = svd.v_t.as_ref().unwrap();
            let z = DMatrix::from_fn(n, n - r, |i, j| {
                let x = vt[(r + j, i)];
                if x.abs() < f64::EPSILON { 0.0 } else { x }
            });
            mx(&z)
        } else {
            let u = svd.u.as_ref().unwrap();
            mx(&u.columns(0, r).into_owned().resize_vertically(m, 0.0))
        }
    }
    fn linalg2_cond(&self, a: &Matrix, p: NormKind) -> Result<f64> {
        if a.is_empty() {
            return Ok(0.0);
        }
        if let NormKind::P(x) = p
            && x == 2.0
        {
            if a.data().iter().any(|v| !v.is_finite()) {
                return Err("Input to SVD must not contain NaN or Inf.".into());
            }
            let svd = self.linalg2_svd(&dm(a), false)?;
            let s = &svd.singular_values;
            let (hi, lo) = (
                s.iter().copied().fold(0.0, f64::max),
                s.iter().copied().fold(f64::INFINITY, f64::min),
            );
            return Ok(if lo == 0.0 { f64::INFINITY } else { hi / lo });
        }
        if let NormKind::P(x) = p
            && !(x == 1.0 || x == f64::INFINITY)
        {
            return Err("The only matrix norms available are 1, 2, inf, and 'fro'.".into());
        }
        if a.rows() != a.cols() {
            return Err("A is rectangular.  Use the 2 norm.".into());
        }
        if !finite(a) {
            return Ok(f64::NAN);
        }
        // norm(A,p)*norm(inv(A),p); the inverse comes from an LU solve against the identity.
        self.linalg2_reserve(&[(a.rows(), a.rows()), (a.rows(), a.rows())])?;
        let inverse = match dm(a).lu().solve(&DMatrix::identity(a.rows(), a.rows())) {
            Some(i) => i,
            None => return Ok(f64::INFINITY),
        };
        let inv = mx(&inverse)?;
        Ok(self.linalg2_norm(a, p)? * self.linalg2_norm(&inv, p)?)
    }

    // ---- norm -----------------------------------------------------------
    fn linalg2_norm_kind(&self, v: Option<&Value>, name: &str) -> Result<NormKind> {
        let Some(v) = v else {
            return Ok(NormKind::P(2.0));
        };
        if let Ok(s) = self.linalg2_text(name, v) {
            return match s.to_ascii_lowercase().as_str() {
                "fro" => Ok(NormKind::Fro),
                "inf" => Ok(NormKind::P(f64::INFINITY)),
                _ => Err("The only matrix norms available are 1, 2, inf, and 'fro'.".into()),
            };
        }
        let p = self.linalg2_real(name, v)?;
        if p.len() != 1 {
            return Err("Unknown norm option: p must be a scalar.".into());
        }
        Ok(NormKind::P(p.data()[0]))
    }
    fn linalg2_norm(&self, a: &Matrix, p: NormKind) -> Result<f64> {
        if a.is_empty() {
            return Ok(0.0);
        }
        let d = a.data();
        let vector = a.rows() == 1 || a.cols() == 1;
        let two = |d: &[f64]| nrm2(d);
        if vector {
            return Ok(match p {
                NormKind::Fro | NormKind::P(2.0) => two(d),
                // f64::max/min drop a NaN operand, so any NaN decides the result first.
                NormKind::P(x) if x.is_infinite() && d.iter().any(|v| v.is_nan()) => f64::NAN,
                NormKind::P(x) if x == f64::INFINITY => {
                    d.iter().fold(0.0, |m, v| f64::max(m, v.abs()))
                }
                NormKind::P(x) if x == f64::NEG_INFINITY => {
                    d.iter().fold(f64::INFINITY, |m, v| f64::min(m, v.abs()))
                }
                NormKind::P(1.0) => d.iter().map(|v| v.abs()).sum(),
                // Any other real p, zero and negative included: norm([3 4],0) is Inf.
                NormKind::P(x) => scaled_p_norm(d, x),
            });
        }
        if d.iter().any(|v| v.is_nan()) {
            return Ok(f64::NAN);
        }
        Ok(match p {
            NormKind::Fro => two(d),
            NormKind::P(1.0) => (0..a.cols())
                .map(|c| {
                    d[c * a.rows()..(c + 1) * a.rows()]
                        .iter()
                        .map(|v| v.abs())
                        .sum::<f64>()
                })
                .fold(0.0, f64::max),
            NormKind::P(x) if x == f64::INFINITY => (0..a.rows())
                .map(|r| {
                    (0..a.cols())
                        .map(|c| d[c * a.rows() + r].abs())
                        .sum::<f64>()
                })
                .fold(0.0, f64::max),
            NormKind::P(2.0) => {
                if d.iter().any(|v| v.is_infinite()) {
                    return Ok(f64::INFINITY);
                }
                let svd = self.linalg2_svd(&dm(a), false)?;
                svd.singular_values.iter().copied().fold(0.0, f64::max)
            }
            NormKind::P(_) => {
                return Err("The only matrix norms available are 1, 2, inf, and 'fro'.".into());
            }
        })
    }

    // ---- matrix functions -----------------------------------------------
    fn linalg2_gemm(&self, a: &DMatrix<f64>, b: &DMatrix<f64>) -> Result<DMatrix<f64>> {
        self.check_cancel()?;
        Ok(dm(&mx(a)?.mul(&mx(b)?)?))
    }
    fn linalg2_expm(&self, a: &Matrix) -> Result<Matrix> {
        let n = square(a)?;
        if n == 0 {
            return Ok(a.clone());
        }
        if n == 1 {
            return Ok(Matrix::scalar(a.data()[0].exp()));
        }
        self.linalg2_work(n, n)?;
        let a = dm(a);
        let nrm = norm1(&a);
        if !nrm.is_finite() {
            return Matrix::filled(n, n, f64::NAN);
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
        let a = if scale > 0 { &a / 2f64.powi(scale) } else { a };
        let id = DMatrix::<f64>::identity(n, n);
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
            let a2 = self.linalg2_gemm(&a, &a)?;
            let a4 = self.linalg2_gemm(&a2, &a2)?;
            let a6 = self.linalg2_gemm(&a2, &a4)?;
            let t1 = &a6 * B[13] + &a4 * B[11] + &a2 * B[9];
            let u0 =
                self.linalg2_gemm(&a6, &t1)? + &a6 * B[7] + &a4 * B[5] + &a2 * B[3] + &id * B[1];
            let u = self.linalg2_gemm(&a, &u0)?;
            let t2 = &a6 * B[12] + &a4 * B[10] + &a2 * B[8];
            let v =
                self.linalg2_gemm(&a6, &t2)? + &a6 * B[6] + &a4 * B[4] + &a2 * B[2] + &id * B[0];
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
            let a2 = self.linalg2_gemm(&a, &a)?;
            let mut pows = vec![id.clone(), a2.clone()];
            for _ in 2..=order / 2 {
                let next = self.linalg2_gemm(pows.last().unwrap(), &a2)?;
                pows.push(next);
            }
            let mut u0 = DMatrix::zeros(n, n);
            let mut v = DMatrix::zeros(n, n);
            for (j, p) in pows.iter().enumerate() {
                v += p * b[2 * j];
                if 2 * j + 1 < b.len() {
                    u0 += p * b[2 * j + 1];
                }
            }
            (self.linalg2_gemm(&a, &u0)?, v)
        };
        let mut r = (&v - &u)
            .lu()
            .solve(&(&v + &u))
            .ok_or("expm: Pade denominator is singular")?;
        for _ in 0..scale {
            r = self.linalg2_gemm(&r, &r)?;
        }
        mx(&r)
    }

    /// sqrtm / logm through the complex Schur form A = Q T Q^H.
    fn linalg2_schur_function(&self, a: &Matrix, log: bool) -> Result<Value> {
        let n = square(a)?;
        if n == 0 {
            return Ok(Value::Matrix(a.clone()));
        }
        self.linalg2_work(n, n)?;
        let cancel = &self.cancel;
        if !finite(a) {
            return Ok(Value::Matrix(Matrix::filled(n, n, f64::NAN)?));
        }
        let ac = dm(a).map(|x| C64::new(x, 0.0));
        let anorm = norm1(&dm(a));
        // An upper triangular A is its own Schur form (Q = I), as in MATLAB; skipping Q also keeps
        // Inf entries of a singular sqrtm out of a Q*F*Q' product that would turn them into NaN.
        let upper = (0..n).all(|j| (j + 1..n).all(|i| a.get(i, j) == 0.0));
        let (q, mut t) = if upper {
            (None, ac)
        } else {
            let schur = nalgebra::linalg::Schur::try_new(ac, f64::EPSILON, 1000 * n + 10000)
                .ok_or("Schur decomposition did not converge.")?;
            self.check_cancel()?;
            let (q, t) = schur.unpack();
            (Some(q), t)
        };
        // A is real: snap numerically real eigenvalues exactly onto the real axis, as real Schur does.
        let snap = 100.0 * f64::EPSILON * anorm.max(f64::MIN_POSITIVE);
        let mut complex_result = false;
        for i in 0..n {
            if t[(i, i)].im.abs() <= snap {
                t[(i, i)].im = 0.0;
                if t[(i, i)].re < 0.0 {
                    complex_result = true;
                }
            }
        }
        for j in 0..n {
            for i in j + 1..n {
                t[(i, j)] = C64::new(0.0, 0.0);
            }
        }
        let f = if log {
            if (0..n).any(|i| t[(i, i)].norm() == 0.0) {
                return Err(
                    "logm: the principal logarithm is not defined for a singular matrix.".into(),
                );
            }
            self.linalg2_tri_log(&t)?
        } else {
            self.linalg2_tri_sqrt(&t)?
        };
        let x = match &q {
            Some(q) => q * f * q.adjoint(),
            None => f,
        };
        if complex_result {
            let data: Vec<C64> = x.as_slice().to_vec();
            return Ok(Value::Complex(ComplexMatrix::from_column_vec(
                n, n, data, cancel,
            )?));
        }
        let re: Vec<f64> = x.iter().map(|z| z.re).collect();
        Ok(Value::Matrix(Matrix::from_column_vec(n, n, re)?))
    }
    fn linalg2_tri_sqrt(&self, t: &DMatrix<C64>) -> Result<DMatrix<C64>> {
        let n = t.nrows();
        let diagonal = (0..n).all(|j| (0..j).all(|i| t[(i, j)] == C64::new(0.0, 0.0)));
        let mut r = DMatrix::from_element(n, n, C64::new(0.0, 0.0));
        for j in 0..n {
            self.check_cancel()?;
            r[(j, j)] = t[(j, j)].sqrt();
            // MATLAB handles diagonal matrices directly, including zeros(n).
            if diagonal {
                continue;
            }
            for i in (0..j).rev() {
                let mut s = t[(i, j)];
                for k in i + 1..j {
                    s -= r[(i, k)] * r[(k, j)];
                }
                let d = r[(i, i)] + r[(j, j)];
                // For a nondiagonal real triangular matrix retain IEEE division:
                // 0/0 is NaN and nonzero/0 is Inf, as in the reference recurrence.
                r[(i, j)] = if d.re == 0.0 && d.im == 0.0 && s.im == 0.0 {
                    C64::new(s.re / d.re, 0.0)
                } else {
                    s / d
                };
            }
        }
        Ok(r)
    }
    /// Inverse scaling and squaring with a [7/7] Pade (Gauss-Legendre) log of a triangular matrix.
    fn linalg2_tri_log(&self, t: &DMatrix<C64>) -> Result<DMatrix<C64>> {
        let n = t.nrows();
        let id = DMatrix::<C64>::identity(n, n);
        let mut s = t.clone();
        let mut k = 0;
        let cnorm1 = |m: &DMatrix<C64>| {
            m.column_iter()
                .map(|c| c.iter().map(|z| z.norm()).sum::<f64>())
                .fold(0.0, f64::max)
        };
        while cnorm1(&(&s - &id)) > 0.25 && k < 64 {
            s = self.linalg2_tri_sqrt(&s)?;
            k += 1;
        }
        let x = &s - &id;
        const NODES: [(f64, f64); 7] = [
            (0.0, 0.4179591836734694),
            (0.4058451513773972, 0.3818300505051189),
            (-0.4058451513773972, 0.3818300505051189),
            (0.7415311855993945, 0.27970539148927664),
            (-0.7415311855993945, 0.27970539148927664),
            (0.9491079123427585, 0.1294849661688697),
            (-0.9491079123427585, 0.1294849661688697),
        ];
        let mut sum = DMatrix::from_element(n, n, C64::new(0.0, 0.0));
        for (node, weight) in NODES {
            self.check_cancel()?;
            let denominator = &id + &x * C64::new((1.0 + node) / 2.0, 0.0);
            let y = denominator
                .solve_upper_triangular(&x)
                .ok_or("logm: triangular solve failed")?;
            sum += y * C64::new(weight / 2.0, 0.0);
        }
        let mut l = sum * C64::new(2f64.powi(k), 0.0);
        for i in 0..n {
            l[(i, i)] = t[(i, i)].ln();
        }
        Ok(l)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NormKind {
    P(f64),
    Fro,
}

fn nrm2(d: &[f64]) -> f64 {
    let scale = d.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if scale == 0.0 || !scale.is_finite() {
        return if d.iter().any(|v| v.is_nan()) {
            f64::NAN
        } else {
            scale
        };
    }
    scale * d.iter().map(|v| (v / scale).powi(2)).sum::<f64>().sqrt()
}

/// Normalize before powering so representable results do not overflow or
/// underflow in the intermediate sum. Negative p uses the smallest magnitude
/// instead: its normalized powers must also be at most one.
fn scaled_p_norm(d: &[f64], p: f64) -> f64 {
    if p == 0.0 {
        return (d.len() as f64).powf(1.0 / p);
    }
    if p.is_nan() || d.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    let scale = if p < 0.0 {
        d.iter().fold(f64::INFINITY, |m, v| m.min(v.abs()))
    } else {
        d.iter().fold(0.0_f64, |m, v| m.max(v.abs()))
    };
    if scale == 0.0 || scale.is_infinite() {
        return scale;
    }
    scale
        * d.iter()
            .map(|v| (v.abs() / scale).powf(p))
            .sum::<f64>()
            .powf(1.0 / p)
}

/// LAPACK dlarfg on x = [alpha, rest]: stores beta in x[0], v (v0 = 1 implied) in rest; returns tau.
/// beta comes from a scaled norm and hypot; a beta below safmin is rescaled as dlarfg does, and
/// where |alpha - beta| = |alpha| + |beta| overflows, tau and v are formed from alpha/beta.
fn larfg(x: &mut [f64]) -> f64 {
    let mut alpha = x[0];
    let mut xnorm = nrm2(&x[1..]);
    if xnorm == 0.0 {
        return 0.0;
    }
    let mut beta = -alpha.hypot(xnorm).copysign(alpha);
    let safmin = f64::MIN_POSITIVE / (f64::EPSILON / 2.0);
    let mut knt = 0;
    if beta.abs() < safmin {
        let rsafmn = 1.0 / safmin;
        while beta.abs() < safmin && knt < 20 {
            knt += 1;
            for v in &mut x[1..] {
                *v *= rsafmn;
            }
            beta *= rsafmn;
            alpha *= rsafmn;
        }
        xnorm = nrm2(&x[1..]);
        beta = -alpha.hypot(xnorm).copysign(alpha);
    }
    let tau = if (alpha - beta).is_finite() {
        let scale = 1.0 / (alpha - beta);
        for v in &mut x[1..] {
            *v *= scale;
        }
        (beta - alpha) / beta
    } else {
        // alpha and beta have opposite signs, so ratio is in [-1, 0] and ratio - 1 in [-2, -1].
        let ratio = alpha / beta;
        for v in &mut x[1..] {
            *v = (*v / beta) / (ratio - 1.0);
        }
        1.0 - ratio
    };
    for _ in 0..knt {
        beta *= safmin;
    }
    x[0] = beta;
    tau
}

/// Apply H = I - tau v v' (v stored below the diagonal of column i, v0 = 1) to column j.
fn apply_reflector(h: &mut [f64], m: usize, i: usize, tau: f64, j: usize) {
    let (left, right) = h.split_at_mut(j * m);
    let v = &left[i * m + i + 1..(i + 1) * m];
    let col = &mut right[i..m];
    let w = (col[0] + v.iter().zip(&col[1..]).map(|(a, b)| a * b).sum::<f64>()) * tau;
    col[0] -= w;
    for (c, a) in col[1..].iter_mut().zip(v) {
        *c -= w * a;
    }
}
