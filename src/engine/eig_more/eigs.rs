//! eigs, svds and condeig. eigs and svds compute every eigen/singular value
//! with the dense kernels and select the requested ones; MATLAB uses ARPACK,
//! so the last digits differ (the corpus compares these at relative 1e-12).
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cancel_svd::Svd as SVD;
use super::{CMat, Mat, Num, cdense, diag_complex, diag_real, text_arg};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, Complex64, Result};
use nalgebra::DMatrix;

const SIGMA_ERROR: &str = "Eigenvalue range sigma must be a valid 2-element string.\nFor real symmetric A, the choices are 'LM', 'SM', 'LA', 'SA' or 'BE'. \nFor non-symmetric or complex A, the choices are 'LM', 'SM', 'LR', 'SR', 'LI' or 'SI'.\n";

#[derive(Clone, Copy, PartialEq)]
enum Sigma {
    Lm,
    Sm,
    La,
    Sa,
    Be,
    Lr,
    Sr,
    Li,
    Si,
    Shift(f64),
}

fn parse_sigma(value: &Value) -> Result<Sigma> {
    if let Some(t) = text_arg(value) {
        return Ok(match t.to_ascii_lowercase().as_str() {
            "lm" => Sigma::Lm,
            "sm" => Sigma::Sm,
            "la" => Sigma::La,
            "sa" => Sigma::Sa,
            "be" => Sigma::Be,
            "lr" => Sigma::Lr,
            "sr" => Sigma::Sr,
            "li" => Sigma::Li,
            "si" => Sigma::Si,
            _ => return Err(SIGMA_ERROR.into()),
        });
    }
    match value {
        Value::Matrix(m) if m.len() == 1 => Ok(Sigma::Shift(m.data()[0])),
        _ => Err(SIGMA_ERROR.into()),
    }
}

fn scalar_of(value: &Value) -> Option<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        Value::Logical(m) if m.len() == 1 => Some(f64::from(m.data()[0])),
        _ => None,
    }
}

fn struct_flag(value: &Value, field: &str) -> Option<f64> {
    let Value::Container(Array::Struct(s)) = value else {
        return None;
    };
    let index = s.field_index(field)?;
    match &**s.get(index, 0).ok()? {
        Array::Numeric(m) if m.len() == 1 => Some(m.data()[0]),
        Array::Logical(m) if m.len() == 1 => Some(f64::from(m.data()[0])),
        _ => None,
    }
}

impl Engine {
    fn lx3_warn(&mut self, id: &str, message: &str, out: &mut Execution) -> Result<()> {
        let args = [
            Value::Text(id.into()),
            Value::Text("%s".into()),
            Value::Text(message.into()),
        ];
        self.lang_utils_call("warning", &args, out)?;
        Ok(())
    }

    /// The dense operator of a function handle: [f(e1) ... f(en)].
    fn lx3_operator(
        &mut self,
        handle: &crate::FunctionHandle,
        n: usize,
        out: &mut Execution,
    ) -> Result<Num> {
        self.preflight_elements(n.saturating_mul(n))?;
        let mut cols: Vec<Num> = Vec::with_capacity(n);
        for j in 0..n {
            self.check_cancel()?;
            let mut e = vec![0.; n];
            e[j] = 1.;
            let arg = Value::Matrix(crate::Matrix::from_column_vec(n, 1, e)?);
            let mut r = self.call_handle(handle, vec![arg], 1, None, out)?;
            if r.is_empty() {
                return Err("The function handle must return a value.".into());
            }
            let v = self.lx3_operand("eigs", &r.remove(0))?.num;
            if v.rows() * v.cols() != n {
                return Err(format!("The function must return a vector of length {n}."));
            }
            cols.push(v);
        }
        if cols.iter().all(|c| matches!(c, Num::R(_))) {
            let mut m = Mat::zeros(n, n);
            for (j, c) in cols.iter().enumerate() {
                if let Num::R(c) = c {
                    for i in 0..n {
                        m.set(i, j, c.data[i]);
                    }
                }
            }
            return Ok(Num::R(m));
        }
        let mut m = CMat::zeros(n, n);
        for (j, c) in cols.iter().enumerate() {
            let c = c.complex();
            for i in 0..n {
                m.set(i, j, c.data[i]);
            }
        }
        Ok(Num::C(m))
    }

    pub(super) fn lx3_eigs(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        let mut rest = &args[1..];
        // The operator: a matrix, or a function handle followed by n.
        let mut from_handle = None;
        let a = match &args[0] {
            Value::Container(Array::Handle(h)) => {
                let n = rest
                    .first()
                    .and_then(scalar_of)
                    .ok_or("Size of problem, n, must be a positive integer.")?;
                if !(n >= 0. && n.fract() == 0. && n <= 4096.) {
                    return Err("Size of problem, n, must be a positive integer.".into());
                }
                rest = &rest[1..];
                from_handle = Some((h.clone(), n as usize));
                None
            }
            v => {
                let a = self
                    .lx3_operand("eigs", v)
                    .map_err(|_| "A must be a square matrix or a function.".to_string())?
                    .num;
                if a.rows() != a.cols() {
                    return Err("A must be a square matrix or a function.".into());
                }
                Some(a)
            }
        };
        // Optional B (a matrix with the size of A, before k).
        let mut b = None;
        let n = a
            .as_ref()
            .map_or_else(|| from_handle.as_ref().map_or(0, |h| h.1), Num::rows);
        if n > 1
            && let Some(first) = rest.first()
            && let Ok(operand) = self.lx3_operand("eigs", first)
            && operand.num.rows() == n
            && operand.num.cols() == n
        {
            b = Some(operand.num);
            rest = &rest[1..];
        }
        let k = match rest.first() {
            Some(v) => {
                let k = scalar_of(v).unwrap_or(-1.);
                if !(k >= 1. && k.fract() == 0. && k <= n as f64) {
                    return Err(
                        "Number of eigenvalues requested, k, must be a positive integer <= n."
                            .into(),
                    );
                }
                k as usize
            }
            None => n.min(6),
        };
        let sigma = match rest.get(1) {
            Some(v) => parse_sigma(v)?,
            None => Sigma::Lm,
        };
        let opts = rest.get(2);
        if rest.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let handle = from_handle.is_some();
        let a = match (a, from_handle) {
            (Some(a), _) => a,
            (None, Some((h, n))) => {
                let m = self.lx3_operator(&h, n, out)?;
                // For 'SM' and a numeric sigma the function applies (A - sigma*I)\x.
                match sigma {
                    Sigma::Sm | Sigma::Shift(_) => {
                        let shift = if let Sigma::Shift(s) = sigma { s } else { 0. };
                        return self.lx3_eigs_inverse(m, shift, k, sigma, count, opts);
                    }
                    _ => m,
                }
            }
            (None, None) => unreachable!(),
        };
        if !a.finite() || b.as_ref().is_some_and(|b| !b.finite()) {
            return Err("Error with ARPACK routine dneupd:\n dnaupd did not find any eigenvalues to sufficient accuracy.".into());
        }
        if n == 0 {
            let empty = Value::Matrix(crate::Matrix::filled(0, usize::from(count < 2), 0.)?);
            return Ok(if count < 2 {
                vec![empty]
            } else {
                vec![
                    Value::Matrix(crate::Matrix::filled(0, 0, 0.)?),
                    Value::Matrix(crate::Matrix::filled(0, 0, 0.)?),
                    Value::Matrix(crate::Matrix::scalar(0.)),
                ]
            });
        }
        let issym = if handle {
            opts.and_then(|o| struct_flag(o, "issym"))
                .is_some_and(|v| v != 0.)
                && matches!(a, Num::R(_))
        } else {
            match (&a, &b) {
                (Num::R(m), None) => super::symmetric(m),
                (Num::R(m), Some(Num::R(bm))) => super::symmetric(m) && super::symmetric(bm),
                _ => false,
            }
        };
        if issym && k == n {
            self.lx3_warn(
                "MATLAB:eigs:TooManyRequestedEigsForRealSym",
                "For real symmetric problems, must have number of eigenvalues k < n.\n Using eig instead.",
                out,
            )?;
            return self.lx3_eigs_full(&a, b.as_ref(), count);
        }
        if !issym && k + 1 >= n {
            self.lx3_warn(
                "MATLAB:eigs:TooManyRequestedEigsForComplexNonsym",
                "For nonsymmetric and complex problems, must have number of eigenvalues k < n-1.\n Using eig instead.",
                out,
            )?;
            return self.lx3_eigs_full(&a, b.as_ref(), count);
        }
        let want_v = count >= 2;
        let (values, vectors) = self.lx3_eigs_dense(&a, b.as_ref(), want_v)?;
        let picked = pick(
            &values,
            k,
            sigma,
            issym,
            matches!(a, Num::C(_)) || matches!(b, Some(Num::C(_))),
        )?;
        self.lx3_eigs_outputs(&values, vectors.as_ref(), &picked, count)
    }

    /// eigs on an operator that applies inv(A - shift*I): eig(A) = shift + 1/mu.
    fn lx3_eigs_inverse(
        &mut self,
        m: Num,
        shift: f64,
        k: usize,
        sigma: Sigma,
        count: usize,
        opts: Option<&Value>,
    ) -> Result<Vec<Value>> {
        let issym = opts
            .and_then(|o| struct_flag(o, "issym"))
            .is_some_and(|v| v != 0.);
        let (mu, vectors) = self.lx3_eigs_dense(&m, None, count >= 2)?;
        let values: Vec<Complex64> = mu
            .iter()
            .map(|&z| {
                let inv = cdense::zladiv(cdense::c(1., 0.), z);
                cdense::c(shift + inv.re, inv.im)
            })
            .collect();
        let picked = pick(&values, k, sigma, issym, matches!(m, Num::C(_)))?;
        self.lx3_eigs_outputs(&values, vectors.as_ref(), &picked, count)
    }

    /// All eigenvalues (as complex) and optional vectors of A or (A, B).
    fn lx3_eigs_dense(
        &self,
        a: &Num,
        b: Option<&Num>,
        want_v: bool,
    ) -> Result<(Vec<Complex64>, Option<CMat>)> {
        let count = if want_v { 2 } else { 1 };
        let (values, v, _) = match b {
            Some(b) => self.lx3_eig_pencil(a, b, count, true)?,
            None => self.lx3_eig_standard(a, count, true)?,
        };
        let values = match values {
            Num::R(m) => m.data.iter().map(|&x| cdense::c(x, 0.)).collect(),
            Num::C(m) => m.data,
        };
        Ok((values, v.map(|v| v.complex())))
    }

    fn lx3_eigs_full(&self, a: &Num, b: Option<&Num>, count: usize) -> Result<Vec<Value>> {
        let c = count.min(2);
        let (values, v, _) = match b {
            Some(b) => self.lx3_eig_pencil(a, b, c, true)?,
            None => self.lx3_eig_standard(a, c, true)?,
        };
        let mut out = self.lx3_eig_outputs(values, v, None, c, count < 2, false)?;
        if count >= 3 {
            out.push(Value::Matrix(crate::Matrix::scalar(0.)));
        }
        Ok(out)
    }

    fn lx3_eigs_outputs(
        &self,
        values: &[Complex64],
        vectors: Option<&CMat>,
        picked: &[usize],
        count: usize,
    ) -> Result<Vec<Value>> {
        let k = picked.len();
        let d: Vec<Complex64> = picked.iter().map(|&i| values[i]).collect();
        let real = d.iter().all(|z| z.im == 0.);
        let as_num = |m: CMat| {
            if m.data.iter().all(|z| z.im == 0.) {
                Num::R(m.re())
            } else {
                Num::C(m)
            }
        };
        if count < 2 {
            let col = CMat {
                rows: k,
                cols: 1,
                data: d,
            };
            return Ok(vec![self.lx3_num_value(as_num(col), false)?]);
        }
        let v = vectors.ok_or("eigs: vectors were not computed")?;
        let mut vm = CMat::zeros(v.rows, k);
        for (j, &i) in picked.iter().enumerate() {
            for r in 0..v.rows {
                vm.set(r, j, v.at(r, i));
            }
        }
        let dm = if real {
            Num::R(diag_real(&d.iter().map(|z| z.re).collect::<Vec<_>>()))
        } else {
            Num::C(diag_complex(&d))
        };
        let mut out = vec![
            self.lx3_num_value(as_num(vm), false)?,
            self.lx3_num_value(dm, false)?,
        ];
        if count >= 3 {
            out.push(Value::Matrix(crate::Matrix::scalar(0.)));
        }
        Ok(out)
    }

    pub(super) fn lx3_svds(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        let first_error = "First input must be either a double matrix or a function handle.";
        if !matches!(args[0], Value::Matrix(_) | Value::Complex(_)) {
            return Err(first_error.into());
        }
        let a = self.lx3_operand("svds", &args[0])?.num;
        let (m, n) = (a.rows(), a.cols());
        let p = m.min(n);
        let k = match args.get(1) {
            Some(v) => {
                let k = scalar_of(v).unwrap_or(-1.);
                if !(k >= 0. && k.fract() == 0.) {
                    return Err("K must be a nonnegative integer.".into());
                }
                (k as usize).min(p)
            }
            None => p.min(6),
        };
        #[derive(PartialEq)]
        enum Want {
            Largest,
            Smallest,
            Near(f64),
        }
        let want = match args.get(2) {
            None => Want::Largest,
            Some(v) => match text_arg(v).map(|t| t.to_ascii_lowercase()) {
                Some(t) if t == "l" || t == "largest" => Want::Largest,
                Some(t) if t == "s" || t == "smallest" => Want::Smallest,
                Some(_) => {
                    return Err(
                        "Third input must be a numeric scalar, 'largest' or 'smallest'.".into(),
                    );
                }
                None => match scalar_of(v) {
                    Some(0.) => {
                        self.lx3_warn(
                            "MATLAB:svds:SigmaZero",
                            "Use SIGMA = 'smallest' to find the smallest singular values of a matrix.",
                            out,
                        )?;
                        Want::Smallest
                    }
                    Some(s) => Want::Near(s),
                    None => {
                        return Err(
                            "Third input must be a numeric scalar, 'largest' or 'smallest'.".into(),
                        );
                    }
                },
            },
        };
        if !a.finite() {
            return Err("Input matrix is badly conditioned. It may contain Inf or NaN.".into());
        }
        let want_vectors = count >= 2;
        let empty = |r: usize, c: usize| -> Result<Value> {
            Ok(Value::Matrix(crate::Matrix::filled(r, c, 0.)?))
        };
        if p == 0 || k == 0 {
            return Ok(if count < 2 {
                vec![empty(0, 1)?]
            } else {
                vec![
                    empty(m, 0)?,
                    empty(0, 0)?,
                    empty(n, 0)?,
                    Value::Matrix(crate::Matrix::scalar(0.)),
                ]
            });
        }
        self.check_cancel()?;
        // Singular values (descending) and, when asked, the vectors.
        let (s, u, v): (Vec<f64>, Option<CMat>, Option<CMat>) = match &a {
            Num::R(x) => {
                let svd = SVD::try_new(
                    DMatrix::from_column_slice(m, n, &x.data),
                    want_vectors,
                    want_vectors,
                    f64::EPSILON,
                    500 * m.max(n),
                    &self.lx3_cancelled(),
                )
                .ok_or("svds: the iteration did not converge")?;
                let u = svd.u.map(|u| CMat {
                    rows: m,
                    cols: p,
                    data: u.as_slice().iter().map(|&x| cdense::c(x, 0.)).collect(),
                });
                let v = svd.v_t.map(|vt| {
                    let vt = vt.transpose();
                    CMat {
                        rows: n,
                        cols: p,
                        data: vt.as_slice().iter().map(|&x| cdense::c(x, 0.)).collect(),
                    }
                });
                (svd.singular_values.as_slice().to_vec(), u, v)
            }
            Num::C(x) => {
                let svd = SVD::try_new(
                    DMatrix::from_column_slice(m, n, &x.data),
                    want_vectors,
                    want_vectors,
                    f64::EPSILON,
                    500 * m.max(n),
                    &self.lx3_cancelled(),
                )
                .ok_or("svds: the iteration did not converge")?;
                let u = svd.u.map(|u| CMat {
                    rows: m,
                    cols: p,
                    data: u.as_slice().to_vec(),
                });
                let v = svd.v_t.map(|vt| {
                    let vt = vt.adjoint();
                    CMat {
                        rows: n,
                        cols: p,
                        data: vt.as_slice().to_vec(),
                    }
                });
                (svd.singular_values.as_slice().to_vec(), u, v)
            }
        };
        self.check_cancel()?;
        let mut order: Vec<usize> = (0..p).collect();
        order.sort_by(|&i, &j| s[j].total_cmp(&s[i]));
        let mut picked: Vec<usize> = match want {
            Want::Largest => order[..k].to_vec(),
            Want::Smallest => order[p - k..].to_vec(),
            Want::Near(sg) => {
                let mut o = order.clone();
                o.sort_by(|&i, &j| (s[i] - sg).abs().total_cmp(&(s[j] - sg).abs()));
                o.truncate(k);
                o
            }
        };
        picked.sort_by(|&i, &j| s[j].total_cmp(&s[i]));
        let sv: Vec<f64> = picked.iter().map(|&i| s[i]).collect();
        if count < 2 {
            return Ok(vec![self.lx3_real_value(Mat::from_vec(k, 1, sv), false)?]);
        }
        let take = |x: &CMat| {
            let mut out = CMat::zeros(x.rows, k);
            for (j, &i) in picked.iter().enumerate() {
                for r in 0..x.rows {
                    out.set(r, j, x.at(r, i));
                }
            }
            if out.data.iter().all(|z| z.im == 0.) {
                Num::R(out.re())
            } else {
                Num::C(out)
            }
        };
        let u = u.ok_or("svds: vectors were not computed")?;
        let v = v.ok_or("svds: vectors were not computed")?;
        let mut out = vec![
            self.lx3_num_value(take(&u), false)?,
            self.lx3_real_value(diag_real(&sv), false)?,
            self.lx3_num_value(take(&v), false)?,
        ];
        if count >= 4 {
            out.push(Value::Matrix(crate::Matrix::scalar(0.)));
        }
        Ok(out)
    }

    pub(super) fn lx3_condeig(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 1 {
            return Err("Too many input arguments.".into());
        }
        let a = self.lx3_operand("condeig", &args[0])?;
        if a.num.rows() != a.num.cols() {
            return Err("For standard eigenproblem EIG(A), A must be square.".into());
        }
        if !a.num.finite() {
            return Err("Input to EIG must not contain NaN or Inf.".into());
        }
        let (values, v, w) = self.lx3_eig_standard(&a.num, 3, true)?;
        let v = v.ok_or("condeig: vectors were not computed")?.complex();
        let w = w.ok_or("condeig: vectors were not computed")?.complex();
        let n = v.rows;
        let s: Vec<f64> = (0..n)
            .map(|j| {
                let mut d = cdense::c(0., 0.);
                for i in 0..n {
                    d += w.at(i, j).conj() * v.at(i, j);
                }
                1. / cdense::cabs(d)
            })
            .collect();
        let s = self.lx3_real_value(Mat::from_vec(n, 1, s), a.single)?;
        if count < 2 {
            return Ok(vec![s]);
        }
        let vv = if v.data.iter().all(|z| z.im == 0.) {
            Num::R(v.re())
        } else {
            Num::C(v)
        };
        let mut out = self.lx3_eig_outputs(values, Some(vv), None, 2, false, a.single)?;
        out.push(s);
        Ok(out)
    }
}

/// Indices of the k wanted eigenvalues in MATLAB's output order.
fn pick(
    values: &[Complex64],
    k: usize,
    sigma: Sigma,
    issym: bool,
    complex: bool,
) -> Result<Vec<usize>> {
    let n = values.len();
    let mut idx: Vec<usize> = (0..n).collect();
    let re = |i: usize| values[i].re;
    let im = |i: usize| values[i].im;
    let abs = |i: usize| cdense::cabs(values[i]);
    if issym {
        let chosen: Vec<usize> = match sigma {
            Sigma::Lm => top(&mut idx, k, abs),
            Sigma::Sm => top(&mut idx, k, |i| -abs(i)),
            Sigma::La | Sigma::Lr => top(&mut idx, k, re),
            Sigma::Sa | Sigma::Sr => top(&mut idx, k, |i| -re(i)),
            Sigma::Be => {
                let mut sorted = idx.clone();
                sorted.sort_by(|&i, &j| re(i).total_cmp(&re(j)));
                let low = k / 2;
                let high = k - low;
                let mut c: Vec<usize> = sorted[..low].to_vec();
                c.extend_from_slice(&sorted[n - high..]);
                c
            }
            Sigma::Shift(s) => top(&mut idx, k, |i| -(re(i) - s).abs()),
            Sigma::Li | Sigma::Si => return Err(SIGMA_ERROR.into()),
        };
        let mut chosen = chosen;
        chosen.sort_by(|&i, &j| re(i).total_cmp(&re(j)));
        if !matches!(sigma, Sigma::Sa | Sigma::Be) {
            chosen.reverse();
        }
        return Ok(chosen);
    }
    if matches!(sigma, Sigma::La | Sigma::Sa | Sigma::Be) {
        return Err(SIGMA_ERROR.into());
    }
    // Conjugate pairs: negative imaginary part first.
    let mut i = 0;
    while i + 1 < n {
        if values[i].im > 0. && values[i + 1] == values[i].conj() {
            idx.swap(i, i + 1);
            i += 2;
        } else {
            i += 1;
        }
    }
    let key = |i: usize| match sigma {
        Sigma::Lm => abs(i),
        Sigma::Sm => -abs(i),
        Sigma::Lr => re(i),
        Sigma::Sr => -re(i),
        Sigma::Li => im(i).abs(),
        Sigma::Si => -im(i).abs(),
        Sigma::Shift(s) => -cdense::cabs(cdense::c(re(i) - s, im(i))),
        _ => 0.,
    };
    let mut chosen = top(&mut idx, k, key);
    if complex {
        chosen.reverse();
    }
    Ok(chosen)
}

/// The k entries with the largest key, most wanted first (stable).
fn top(idx: &mut [usize], k: usize, key: impl Fn(usize) -> f64) -> Vec<usize> {
    idx.sort_by(|&i, &j| key(j).total_cmp(&key(i)));
    idx[..k].to_vec()
}
