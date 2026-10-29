//! polyeig uses companion linearisation (Tisseur & Meerbergen, "The quadratic
//! eigenvalue problem", SIAM Review 43, 2001); gsvd uses the cosine-sine
//! decomposition (Golub & Van Loan, Matrix Computations, 4th ed., 2013;
//! Paige & Saunders, SIAM J. Numer. Anal. 18, 1981).
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cancel_svd::Svd as SVD;
use super::cdense::{self, CMat, c, cabs, cabs1, cdivr, cmulr};
use super::cgen;
use super::{Mat, Num};
use crate::engine::{Engine, Value};
use crate::{Complex64 as C, Result};
use nalgebra::DMatrix;

const ZERO: C = C { re: 0., im: 0. };

/// Solve A X = B (A square) by LU with partial pivoting (zgetf2/zgetrs).
fn lu_solve(a: &CMat, b: &CMat, cancel: &dyn Fn() -> bool) -> CMat {
    let n = a.rows;
    let mut lu = a.clone();
    let mut piv: Vec<usize> = (0..n).collect();
    for k in 0..n {
        if cancel() {
            return b.clone();
        }
        let p = (k..n)
            .max_by(|&i, &j| {
                cabs1(lu.at(i, k))
                    .total_cmp(&cabs1(lu.at(j, k)))
                    .then(j.cmp(&i))
            })
            .unwrap_or(k);
        if p != k {
            lu.swap_rows(p, k, 0, n);
            piv.swap(p, k);
        }
        let d = lu.at(k, k);
        if d != ZERO {
            for i in k + 1..n {
                let l = cdense::cdiv(lu.at(i, k), d);
                lu.set(i, k, l);
                for j in k + 1..n {
                    let v = lu.at(i, j) - l * lu.at(k, j);
                    lu.set(i, j, v);
                }
            }
        }
    }
    let mut x = CMat::zeros(n, b.cols);
    for col in 0..b.cols {
        if cancel() {
            return x;
        }
        let mut y: Vec<C> = piv.iter().map(|&p| b.at(p, col)).collect();
        for i in 0..n {
            for k in 0..i {
                y[i] = y[i] - lu.at(i, k) * y[k];
            }
        }
        for i in (0..n).rev() {
            for k in i + 1..n {
                y[i] = y[i] - lu.at(i, k) * y[k];
            }
            y[i] = cdense::cdiv(y[i], lu.at(i, i));
        }
        for (i, v) in y.into_iter().enumerate() {
            x.set(i, col, v);
        }
    }
    x
}

fn fro(m: &CMat) -> f64 {
    cdense::znrm2(m.data.iter())
}

fn as_num(m: CMat) -> Num {
    if m.data.iter().all(|z| z.im == 0.) {
        Num::R(m.re())
    } else {
        Num::C(m)
    }
}

impl Engine {
    /// polyeig through the companion pencil of `companion`, solved by QZ
    /// (Golub & Van Loan 2013, §7.7); eigenvectors and condition numbers as in
    /// `polynomial_vectors` and `polyeig_condition`.
    pub(super) fn lx3_polyeig(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        // Without a coefficient there is no first block to read.
        if args.is_empty() {
            return Err("Index exceeds matrix dimensions.".into());
        }
        if count > 3 {
            return Err("Too many output arguments.".into());
        }
        // Condition numbers need a polynomial; a lone matrix has none.
        if count > 2 && args.len() == 1 {
            return Err("Must provide at least two matrices.".into());
        }
        let mut single = false;
        let mut coefficients = Vec::with_capacity(args.len());
        for value in args {
            let operand = self.lx3_operand("polyeig", value)?;
            single |= operand.single;
            coefficients.push(operand.num);
        }
        let n = coefficients[0].cols();
        if let Some(a) = coefficients.iter().find(|a| a.rows() != n || a.cols() != n) {
            // Each coefficient fills one n-by-n block of the pencil; report the
            // block assignment that cannot take it.
            let subscripts = if n > 1 || a.rows() > 1 { 2 } else { 0 };
            let given = usize::from(a.rows() != 1) + usize::from(a.cols() != 1);
            return Err(if given < subscripts {
                "Assignment has fewer non-singleton rhs dimensions than non-singleton subscripts"
            } else {
                "Subscripted assignment dimension mismatch."
            }
            .into());
        }
        if coefficients.iter().any(|a| !a.finite()) {
            return Err("Input to EIG must not contain NaN or Inf.".into());
        }
        let complex = coefficients.iter().any(|a| matches!(a, Num::C(_)));
        let p = coefficients.len() - 1;
        self.lx3_qz_budget(n.saturating_mul(p.max(1)), complex, count)?;
        let (a, b) = companion(&coefficients, n);
        let (e, vectors) = match (&a, &b) {
            (Num::C(ac), Num::C(bc)) => {
                // QZ directly, so an infinite eigenvalue (beta = 0) keeps the
                // direction of alpha instead of becoming 0/0.
                let r = cgen::zggev(ac, bc, false, count >= 2, &self.lx3_cancelled())
                    .map_err(|_| "eig: the QZ iteration failed to converge".to_string())?;
                let e: Vec<C> = r
                    .alpha
                    .iter()
                    .zip(&r.beta)
                    .map(|(&x, &y)| {
                        if y == ZERO {
                            infinite(x)
                        } else {
                            cdense::cdiv(x, y)
                        }
                    })
                    .collect();
                (e, r.vr.map(Num::C))
            }
            _ => {
                let (values, vectors, _) = self.lx3_eig_pencil(&a, &b, count.min(2), false)?;
                let e: Vec<C> = match values {
                    Num::R(v) => v.data.iter().map(|&x| c(x, 0.)).collect(),
                    Num::C(v) => v.data,
                };
                (e, vectors)
            }
        };
        drop((a, b));
        let order = e.len();
        let e_value = as_num(CMat {
            rows: order,
            cols: 1,
            data: e.clone(),
        });
        if count < 2 {
            return Ok(vec![self.lx3_num_value(e_value, single)?]);
        }
        let cancel = self.lx3_cancelled();
        // One coefficient is the standard problem A0*x = lambda*x, the
        // degree-one polynomial A0 - lambda*I.
        let mut cm: Vec<CMat> = coefficients.iter().map(Num::complex).collect();
        drop(coefficients);
        if p == 0 {
            let mut minus = CMat::zeros(n, n);
            for i in 0..n {
                minus.set(i, i, c(-1., 0.));
            }
            cm.push(minus);
        }
        let z = match vectors.ok_or("polyeig: vectors were not computed")? {
            Num::R(v) => CMat::from_real(&v),
            Num::C(v) => v,
        };
        let x = polynomial_vectors(&cm, &z, &e, n, p.max(1), &cancel);
        drop(z);
        let mut out = vec![
            self.lx3_num_value(as_num(x.clone()), single)?,
            self.lx3_num_value(e_value, single)?,
        ];
        if count > 2 {
            let s = polyeig_condition(&cm, &x, &e, n, p.max(1), &cancel)?;
            out.push(self.lx3_real_value(s, single)?);
        }
        Ok(out)
    }

    /// gsvd from a QR factorisation [A; B] = [Q1; Q2]*R and the CS
    /// decomposition of [Q1; Q2], so X = R'*Z (Golub & Van Loan 2013, §6.1.6
    /// and §8.7.4; Paige & Saunders 1981 for the shapes of C and S).
    pub(super) fn lx3_gsvd(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let a = self.lx3_operand("gsvd", &args[0])?;
        let b = self.lx3_operand("gsvd", &args[1])?;
        let economy = match args.get(2) {
            None => false,
            Some(v) => {
                let flag = self.lx3_operand("gsvd", v)?.num;
                flag.rows() * flag.cols() == 1 && flag.complex().data[0] == ZERO
            }
        };
        let single = a.single || b.single;
        let (m, p, n) = (a.num.rows(), a.num.cols(), b.num.rows());
        if b.num.cols() != p {
            return Err("Matrices must have the same number of columns.".into());
        }
        if !a.num.finite() || !b.num.finite() {
            return Err("Input to SVD must not contain NaN or Inf.".into());
        }
        let q = m.saturating_add(n).min(p);
        // Complex working storage, two budget elements per entry: the stacked
        // operand and its QR factors, then U, V, Z, X, C and S with the SVD
        // and QR copies made on the way. One output and the economy form
        // reduce a tall A or B first, so U and V are at most p-by-p there
        // (plus the m-by-p and n-by-p reduction factors and the final U, V).
        let reduced = count < 2 || economy;
        let (mu, nv) = if reduced {
            (m.min(p), n.min(p))
        } else {
            (m, n)
        };
        let square = |k: usize| k.saturating_mul(k);
        let reduction = if reduced {
            m.saturating_mul(p.saturating_add(mu))
                .saturating_add(n.saturating_mul(p.saturating_add(nv)))
                .saturating_mul(6)
        } else {
            0
        };
        let elements = mu
            .saturating_add(nv)
            .saturating_mul(p.saturating_add(q))
            .saturating_mul(8)
            .saturating_add(
                square(mu)
                    .saturating_add(square(nv))
                    .saturating_add(square(q))
                    .saturating_add(p.saturating_mul(q))
                    .saturating_mul(10),
            )
            .saturating_add(reduction);
        self.preflight_elements(elements)?;
        let cancel = self.lx3_cancelled();
        let (a, b) = (a.num.complex(), b.num.complex());
        if count < 2 {
            let sigma = gsvd_values(&a, &b, &cancel)?;
            return Ok(vec![self.lx3_real_value(sigma, single)?]);
        }
        // The economy form first reduces a tall A (or B) to its triangular
        // factor, so U (V) keeps at most p columns.
        let (qa, a) = if economy {
            reduce_tall(&a, &cancel)
        } else {
            (None, a)
        };
        let (qb, b) = if economy {
            reduce_tall(&b, &cancel)
        } else {
            (None, b)
        };
        let (mut u, mut v, x, cc, s) = gsvd_full(&a, &b, &cancel)?;
        if let Some(qa) = qa {
            u = qa.mul(&u, &cancel);
        }
        if let Some(qb) = qb {
            v = qb.mul(&v, &cancel);
        }
        Ok(vec![
            self.lx3_num_value(as_num(u), single)?,
            self.lx3_num_value(as_num(v), single)?,
            self.lx3_num_value(as_num(x), single)?,
            self.lx3_real_value(cc.re(), single)?,
            self.lx3_real_value(s.re(), single)?,
        ])
    }
}

/// The companion pencil (A, B) of order n*p for
/// P(lambda) = A0 + lambda*A1 + ... + lambda^p*Ap: A = blkdiag(A0, I) and B
/// holds -A1, ..., -Ap in its first block row with identities below, so
/// A*z = lambda*B*z for z = [x; lambda*x; ...; lambda^(p-1)*x]
/// (Tisseur & Meerbergen 2001, §3.4). One coefficient gives (A0, I).
fn companion(coefficients: &[Num], n: usize) -> (Num, Num) {
    fn blocks<T: Copy + std::ops::Neg<Output = T>>(
        coefficients: &[&[T]],
        n: usize,
        zero: T,
        one: T,
    ) -> (usize, Vec<T>, Vec<T>) {
        let p = coefficients.len() - 1;
        let order = n * p.max(1);
        let mut a = vec![zero; order * order];
        let mut b = vec![zero; order * order];
        for j in 0..n {
            a[j * order..j * order + n].copy_from_slice(&coefficients[0][j * n..(j + 1) * n]);
        }
        if p == 0 {
            for i in 0..n {
                b[i + i * order] = one;
            }
            return (order, a, b);
        }
        for i in n..order {
            a[i + i * order] = one;
            b[i + (i - n) * order] = one;
        }
        for (k, ak) in coefficients.iter().enumerate().skip(1) {
            for j in 0..n {
                let col = ((k - 1) * n + j) * order;
                for i in 0..n {
                    b[col + i] = -ak[i + j * n];
                }
            }
        }
        (order, a, b)
    }
    if coefficients.iter().all(|a| matches!(a, Num::R(_))) {
        let data: Vec<&[f64]> = coefficients
            .iter()
            .map(|a| match a {
                Num::R(m) => &m.data[..],
                Num::C(_) => unreachable!(),
            })
            .collect();
        let (order, a, b) = blocks(&data, n, 0., 1.);
        (
            Num::R(Mat::from_vec(order, order, a)),
            Num::R(Mat::from_vec(order, order, b)),
        )
    } else {
        let cm: Vec<CMat> = coefficients.iter().map(Num::complex).collect();
        let data: Vec<&[C]> = cm.iter().map(|m| &m.data[..]).collect();
        let (order, a, b) = blocks(&data, n, ZERO, c(1., 0.));
        let wrap = |data| CMat {
            rows: order,
            cols: order,
            data,
        };
        (Num::C(wrap(a)), Num::C(wrap(b)))
    }
}

/// Homogeneous coordinates (alpha, beta) of lambda = alpha/beta with
/// |alpha|^2 + |beta|^2 = 1 and beta >= 0; an infinite lambda is (1, 0).
fn homogeneous(lambda: C) -> (C, C) {
    if lambda.re.is_infinite() || lambda.im.is_infinite() {
        return (c(1., 0.), ZERO);
    }
    if lambda.re.is_nan() || lambda.im.is_nan() {
        return (c(f64::NAN, f64::NAN), c(f64::NAN, 0.));
    }
    let t = cabs(lambda);
    let beta = if t <= 1. {
        1. / (1. + t * t).sqrt()
    } else {
        let u = 1. / t;
        u / (1. + u * u).sqrt()
    };
    (cmulr(lambda, beta), c(beta, 0.))
}

/// alpha/0: infinite parts in the directions of alpha's nonzero parts;
/// 0/0 (a singular pencil) stays NaN.
fn infinite(alpha: C) -> C {
    if alpha == ZERO {
        return c(f64::NAN, f64::NAN);
    }
    let part = |v: f64| {
        if v == 0. {
            0.
        } else {
            f64::INFINITY.copysign(v)
        }
    };
    c(part(alpha.re), part(alpha.im))
}

fn cpow(z: C, k: usize) -> C {
    (0..k).fold(c(1., 0.), |acc, _| acc * z)
}

/// sum_k w[k]*A_k*z.
fn combine(cm: &[CMat], w: &[C], z: &[C]) -> Vec<C> {
    let n = z.len();
    let mut out = vec![ZERO; n];
    for (a, &wk) in cm.iter().zip(w) {
        if wk == ZERO {
            continue;
        }
        for (j, &zj) in z.iter().enumerate() {
            let t = wk * zj;
            if t == ZERO {
                continue;
            }
            for (i, o) in out.iter_mut().enumerate() {
                *o += a.at(i, j) * t;
            }
        }
    }
    out
}

/// Eigenvectors of P from those of the companion pencil: block k of z is
/// lambda^k*x, and the block with the smallest residual
/// ||P(alpha,beta)*z_k|| / ||z_k|| (homogeneous form, so infinite
/// eigenvalues use Ap) is scaled to unit 2-norm.
fn polynomial_vectors(
    cm: &[CMat],
    z: &CMat,
    e: &[C],
    n: usize,
    p: usize,
    cancel: &dyn Fn() -> bool,
) -> CMat {
    let blocks = z.rows.checked_div(n).unwrap_or(0);
    let mut x = CMat::zeros(n, z.cols);
    for j in 0..z.cols {
        if cancel() {
            return x;
        }
        let (alpha, beta) = homogeneous(e[j]);
        let w: Vec<C> = (0..=p)
            .map(|k| cpow(alpha, k) * cpow(beta, p - k))
            .collect();
        let column = z.col(j);
        let mut best: Option<(f64, usize, f64)> = None;
        for k in 0..blocks {
            let zk = &column[k * n..(k + 1) * n];
            let norm = cdense::znrm2(zk.iter());
            if norm == 0. || !norm.is_finite() {
                continue;
            }
            let residual = if blocks == 1 {
                0.
            } else {
                let r = cdense::znrm2(combine(cm, &w, zk).iter()) / norm;
                if r.is_nan() { f64::INFINITY } else { r }
            };
            if best.is_none_or(|(r, _, _)| residual < r) {
                best = Some((residual, k, norm));
            }
        }
        if let Some((_, k, norm)) = best {
            for i in 0..n {
                x.set(i, j, cdivr(column[k * n + i], norm));
            }
        }
    }
    x
}

/// Condition numbers of the eigenvalues of P in homogeneous form (Dedieu &
/// Tisseur, LAA 358, 2003; Tisseur & Meerbergen 2001, §4):
/// kappa = (sum_k |alpha|^2k |beta|^2(p-k) w_k^2)^(1/2) ||y|| ||x||
///         / |y'*(conj(beta)*DaP - conj(alpha)*DbP)*x|,
/// with weights w_k = ||A_k||_F. The left eigenvector y comes from one step
/// of inverse iteration, P(alpha,beta)'*y = x, x being the right one.
fn polyeig_condition(
    cm: &[CMat],
    x: &CMat,
    e: &[C],
    n: usize,
    p: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<Mat> {
    let weights: Vec<f64> = cm.iter().map(fro).collect();
    let mut s = Mat::zeros(e.len(), 1);
    for (j, &lambda) in e.iter().enumerate() {
        if cancel() {
            return Err("Calculation cancelled".into());
        }
        let (alpha, beta) = homogeneous(lambda);
        if alpha.re.is_nan() {
            s.set(j, 0, f64::NAN);
            continue;
        }
        let xj = x.col(j);
        let mut pm = CMat::zeros(n, n);
        for (k, a) in cm.iter().enumerate() {
            let w = cpow(alpha, k) * cpow(beta, p - k);
            for (o, &v) in pm.data.iter_mut().zip(&a.data) {
                *o += w * v;
            }
        }
        let rhs = CMat {
            rows: n,
            cols: 1,
            data: xj.to_vec(),
        };
        let mut y = lu_solve(&pm.adjoint(), &rhs, cancel).data;
        let size = cdense::znrm2(y.iter());
        if !size.is_finite() || size == 0. || y.iter().any(|v| !v.is_finite()) {
            // An exactly singular P(alpha,beta) leaves a zero pivot: take the
            // left null vector from the SVD of P' instead. A zero P has every
            // vector as a null vector, so y = x.
            y = if fro(&pm) > 0. {
                let (_, _, v) = svd_jacobi(&pm.adjoint(), cancel)?;
                v.col(n - 1).to_vec()
            } else {
                xj.to_vec()
            };
        } else if !(1e-100..=1e100).contains(&size) {
            // Keep the products with y in range.
            for v in &mut y {
                *v = cdivr(*v, size);
            }
        }
        // conj(beta)*DaP - conj(alpha)*DbP as one combination of the A_k.
        let d: Vec<C> = (0..=p)
            .map(|k| {
                let mut t = ZERO;
                if k > 0 {
                    t += beta.conj() * cpow(alpha, k - 1) * cpow(beta, p - k) * k as f64;
                }
                if k < p {
                    t -= alpha.conj() * cpow(alpha, k) * cpow(beta, p - k - 1) * (p - k) as f64;
                }
                t
            })
            .collect();
        let v = combine(cm, &d, xj);
        let yv = y
            .iter()
            .zip(&v)
            .fold(ZERO, |acc, (&yi, &vi)| acc + yi.conj() * vi);
        let (a2, b2) = (cabs(alpha), cabs(beta));
        let terms: Vec<f64> = weights
            .iter()
            .enumerate()
            .map(|(k, &w)| a2.powi(k as i32) * b2.powi((p - k) as i32) * w)
            .collect();
        let scale = norm2(&terms);
        let norms = cdense::znrm2(y.iter()) * cdense::znrm2(xj.iter());
        s.set(j, 0, scale * norms / cabs(yv));
    }
    Ok(s)
}

/// Euclidean norm of a real vector without squaring entries before scaling.
fn norm2(t: &[f64]) -> f64 {
    let top = t.iter().fold(0f64, |m, v| m.max(v.abs()));
    if top == 0. || !top.is_finite() {
        return top;
    }
    top * t.iter().map(|v| (v / top).powi(2)).sum::<f64>().sqrt()
}

/// [A; B] as one matrix.
fn stack(a: &CMat, b: &CMat) -> CMat {
    let (m, n, p) = (a.rows, b.rows, a.cols);
    let mut out = CMat::zeros(m + n, p);
    for j in 0..p {
        out.data[j * (m + n)..j * (m + n) + m].copy_from_slice(a.col(j));
        out.data[j * (m + n) + m..(j + 1) * (m + n)].copy_from_slice(b.col(j));
    }
    out
}

/// Reduce a tall operand to its triangular factor: x = Qx*R.
fn reduce_tall(x: &CMat, cancel: &dyn Fn() -> bool) -> (Option<CMat>, CMat) {
    if x.rows > x.cols {
        let (qx, r) = qr_econ(x, cancel);
        (Some(qx), r)
    } else {
        (None, x.clone())
    }
}

/// Generalized singular values c_j/s_j, ascending, from the CS
/// decomposition of the full form. A tall A or B is first reduced to its
/// triangular factor, which leaves the values unchanged and keeps U and V
/// at most p-by-p.
fn gsvd_values(a: &CMat, b: &CMat, cancel: &dyn Fn() -> bool) -> Result<Mat> {
    let (_, a) = reduce_tall(a, cancel);
    let (_, b) = reduce_tall(b, cancel);
    let (_, _, _, cc, s) = gsvd_full(&a, &b, cancel)?;
    let q = cc.cols;
    let off = q.saturating_sub(cc.rows);
    let mut sigma = Mat::zeros(q, 1);
    for j in 0..q {
        let cj = if j < off { 0. } else { cc.at(j - off, j).re };
        let sj = if j < s.rows { s.at(j, j).re } else { 0. };
        sigma.set(j, 0, cj / sj);
    }
    Ok(sigma)
}

/// The full decomposition: A = U*C*X', B = V*S*X'.
fn gsvd_full(
    a: &CMat,
    b: &CMat,
    cancel: &dyn Fn() -> bool,
) -> Result<(CMat, CMat, CMat, CMat, CMat)> {
    let (m, n) = (a.rows, b.rows);
    let (qq, r) = qr_econ(&stack(a, b), cancel);
    let q = qq.cols;
    let all: Vec<usize> = (0..q).collect();
    let q1 = block(&qq, &(0..m).collect::<Vec<_>>(), &all);
    let q2 = block(&qq, &(m..m + n).collect::<Vec<_>>(), &all);
    drop(qq);
    let (u, v, z, cc, s) = csd(&q1, &q2, cancel)?;
    let x = r.adjoint().mul(&z, cancel);
    Ok((u, v, x, cc, s))
}

/// Economy QR with LAPACK's Householder (MATLAB's qr(A,0)).
fn qr_econ(a: &CMat, cancel: &dyn Fn() -> bool) -> (CMat, CMat) {
    let (m, n) = (a.rows, a.cols);
    let k = m.min(n);
    let mut h = a.clone();
    let tau = super::cgen::zgeqr2(&mut h, 0, 0, m, n, cancel);
    let mut r = CMat::zeros(k, n);
    for j in 0..n {
        for i in 0..(j + 1).min(k) {
            r.set(i, j, h.at(i, j));
        }
    }
    let mut q = CMat::zeros(m, k);
    for j in 0..k {
        for i in j + 1..m {
            q.set(i, j, h.at(i, j));
        }
    }
    if k > 0 {
        cdense::zung2r(&mut q, k, &tau, 0, 0, m, k, cancel);
    }
    (q, r)
}

/// Full QR (MATLAB's qr(A)).
fn qr_full(a: &CMat, cancel: &dyn Fn() -> bool) -> (CMat, CMat) {
    let (m, n) = (a.rows, a.cols);
    let k = m.min(n);
    let mut h = a.clone();
    let tau = super::cgen::zgeqr2(&mut h, 0, 0, m, n, cancel);
    let mut r = CMat::zeros(m, n);
    for j in 0..n {
        for i in 0..(j + 1).min(m) {
            r.set(i, j, h.at(i, j));
        }
    }
    let mut q = CMat::identity(m);
    for j in 0..k {
        for i in j + 1..m {
            q.set(i, j, h.at(i, j));
        }
    }
    if m > 0 {
        cdense::zung2r(&mut q, k, &tau, 0, 0, m, m, cancel);
    }
    (q, r)
}

/// Start (row, column) of diagonal k.
fn diag_start(k: isize) -> (usize, usize) {
    if k >= 0 {
        (0, k as usize)
    } else {
        (k.unsigned_abs(), 0)
    }
}

/// Diagonal k of x (k > 0 above the main diagonal).
fn diagk(x: &CMat, k: isize) -> Vec<C> {
    let (r0, c0) = diag_start(k);
    (0..)
        .map_while(|t| (r0 + t < x.rows && c0 + t < x.cols).then(|| x.at(r0 + t, c0 + t)))
        .collect()
}

/// Make diagonal k of x real and nonnegative by unimodular scaling of the
/// rows of x and the matching columns of y; y*x is unchanged.
fn diagp(mut y: CMat, mut x: CMat, k: isize) -> (CMat, CMat) {
    let (r0, _) = diag_start(k);
    for (t, d) in diagk(&x, k).into_iter().enumerate() {
        let row = r0 + t;
        if d.im == 0. && d.re >= 0. {
            continue;
        }
        if d.im == 0. {
            for j in 0..x.cols {
                let v = x.at(row, j);
                x.set(row, j, -v);
            }
            for i in 0..y.rows {
                let v = y.at(i, row);
                y.set(i, row, -v);
            }
            continue;
        }
        let phase = cdivr(d, cabs(d));
        for j in 0..x.cols {
            let v = x.at(row, j);
            x.set(row, j, phase.conj() * v);
        }
        for i in 0..y.rows {
            let v = y.at(i, row);
            y.set(i, row, v * phase);
        }
    }
    (y, x)
}

/// svd(A) with full U and V (singular values descending on C's diagonal).
fn svd_full(a: &CMat, cancel: &dyn Fn() -> bool) -> Result<(CMat, CMat, CMat)> {
    let (m, n) = (a.rows, a.cols);
    let k = m.min(n);
    let mut s = CMat::zeros(m, n);
    if k == 0 {
        return Ok((CMat::identity(m), s, CMat::identity(n)));
    }
    let svd = SVD::try_new(
        DMatrix::from_column_slice(m, n, &a.data),
        true,
        true,
        f64::EPSILON,
        500 * m.max(n),
        cancel,
    )
    .ok_or("svd: the iteration did not converge")?;
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by(|&i, &j| svd.singular_values[j].total_cmp(&svd.singular_values[i]));
    let u = svd.u.ok_or("svd: no U")?;
    let vt = svd.v_t.ok_or("svd: no V")?;
    let mut uk = CMat::zeros(m, k);
    let mut vk = CMat::zeros(n, k);
    for (c2, &o) in order.iter().enumerate() {
        if cancel() {
            return Err("Calculation cancelled".into());
        }
        s.set(c2, c2, c(svd.singular_values[o], 0.));
        for i in 0..m {
            uk.set(i, c2, u[(i, o)]);
        }
        for i in 0..n {
            vk.set(i, c2, vt[(o, i)].conj());
        }
    }
    Ok((complete(&uk, cancel), s, complete(&vk, cancel)))
}

/// svd_full for real data; complex data goes through `svd_jacobi`, since
/// the bidiagonal kernel can lose accuracy on rank-deficient complex input
/// (a 2-by-3 Q1 with Frobenius norm 1 gave a singular value 1.003).
fn svd_any(a: &CMat, cancel: &dyn Fn() -> bool) -> Result<(CMat, CMat, CMat)> {
    if a.data.iter().all(|z| z.im == 0.) {
        svd_full(a, cancel)
    } else {
        svd_jacobi(a, cancel)
    }
}

/// One-sided (Hestenes) Jacobi SVD with full U and V, singular values
/// descending on the diagonal (Golub & Van Loan 2013, §8.6.4). Each step
/// turns the phase of column k so the pair's inner product is real, then
/// applies the symmetric Schur rotation of their 2-by-2 Gram matrix.
fn svd_jacobi(a: &CMat, cancel: &dyn Fn() -> bool) -> Result<(CMat, CMat, CMat)> {
    let (m, n) = (a.rows, a.cols);
    let mut g = a.clone();
    // Scale before forming Gram products or squared tolerances. This leaves
    // the singular vectors unchanged even for very large or tiny input.
    let scale = a
        .data
        .iter()
        .fold(0f64, |top, z| top.max(z.re.abs()).max(z.im.abs()));
    if scale > 0. {
        for z in &mut g.data {
            *z = cdivr(*z, scale);
        }
    }
    let mut v = CMat::identity(n);
    // Pairs with a numerically zero column (norm below eps*||A||) or a
    // cosine below sqrt(m)*eps count as orthogonal.
    let floor = (f64::EPSILON * fro(&g)).powi(2);
    let tol = f64::EPSILON * (m.max(1) as f64).sqrt();
    let mut converged = false;
    for _ in 0..60 {
        let mut rotated = false;
        for j in 0..n {
            if cancel() {
                return Err("Calculation cancelled".into());
            }
            for k in j + 1..n {
                let (gj, gk) = (g.col(j), g.col(k));
                let alpha = cdense::znrm2(gj.iter()).powi(2);
                let beta = cdense::znrm2(gk.iter()).powi(2);
                let gamma = gj
                    .iter()
                    .zip(gk)
                    .fold(ZERO, |acc, (&x, &y)| acc + x.conj() * y);
                let size = cabs(gamma);
                if alpha.min(beta) <= floor || size <= tol * (alpha * beta).sqrt() {
                    continue;
                }
                rotated = true;
                let phase = cdivr(gamma, size).conj();
                let zeta = (beta - alpha) / (2. * size);
                let t = 1f64.copysign(zeta) / (zeta.abs() + zeta.hypot(1.));
                let cs = 1. / (1. + t * t).sqrt();
                let sn = cs * t;
                for x in [&mut g, &mut v] {
                    for i in 0..x.rows {
                        let (xj, xk) = (x.at(i, j), x.at(i, k) * phase);
                        x.set(i, j, cmulr(xj, cs) - cmulr(xk, sn));
                        x.set(i, k, cmulr(xj, sn) + cmulr(xk, cs));
                    }
                }
            }
        }
        if !rotated {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err("svd: the iteration did not converge".into());
    }
    let norms: Vec<f64> = (0..n).map(|j| cdense::znrm2(g.col(j).iter())).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| norms[j].total_cmp(&norms[i]));
    let k = m.min(n);
    let largest = norms.iter().fold(0f64, |x, &y| x.max(y));
    let tiny = f64::EPSILON * largest * m.max(n) as f64;
    let mut s = CMat::zeros(m, n);
    let mut u = Vec::new();
    for (t, &o) in order.iter().enumerate().take(k) {
        s.set(t, t, c(norms[o] * scale, 0.));
        if norms[o] > tiny {
            u.push(o);
        }
    }
    // Columns for (numerically) zero singular values complete the basis.
    let mut uk = CMat::zeros(m, u.len());
    for (t, &o) in u.iter().enumerate() {
        for i in 0..m {
            uk.set(i, t, cdivr(g.at(i, o), norms[o]));
        }
    }
    Ok((complete(&uk, cancel), s, cols(&v, &order)))
}

/// Extend orthonormal columns to a square unitary matrix.
fn complete(q: &CMat, cancel: &dyn Fn() -> bool) -> CMat {
    let (m, k) = (q.rows, q.cols);
    if k == m {
        return q.clone();
    }
    let (full, _) = qr_full(q, cancel);
    let mut out = full;
    for j in 0..k {
        for i in 0..m {
            out.set(i, j, q.at(i, j));
        }
    }
    out
}

fn cols(x: &CMat, idx: &[usize]) -> CMat {
    let mut out = CMat::zeros(x.rows, idx.len());
    for (j, &c0) in idx.iter().enumerate() {
        for i in 0..x.rows {
            out.set(i, j, x.at(i, c0));
        }
    }
    out
}

fn block(x: &CMat, rows: &[usize], cs: &[usize]) -> CMat {
    let mut out = CMat::zeros(rows.len(), cs.len());
    for (j, &c0) in cs.iter().enumerate() {
        for (i, &r0) in rows.iter().enumerate() {
            out.set(i, j, x.at(r0, c0));
        }
    }
    out
}

fn put_block(x: &mut CMat, rows: &[usize], cs: &[usize], v: &CMat) {
    for (j, &c0) in cs.iter().enumerate() {
        for (i, &r0) in rows.iter().enumerate() {
            x.set(r0, c0, v.at(i, j));
        }
    }
}

fn put_cols(x: &mut CMat, cs: &[usize], v: &CMat) {
    let rows: Vec<usize> = (0..x.rows).collect();
    put_block(x, &rows, cs, v);
}

/// The main diagonal of x, other entries zero.
fn diagf(x: &CMat) -> CMat {
    let mut out = CMat::zeros(x.rows, x.cols);
    for (t, d) in diagk(x, 0).into_iter().enumerate() {
        out.set(t, t, d);
    }
    out
}

/// CS decomposition of a matrix with orthonormal columns [Q1; Q2]:
/// Q1 = U*C*Z' and Q2 = V*S*Z' with C'*C + S'*S = I, cosines ascending on
/// diagonal max(0, q-m) of C and sines descending on the main diagonal of S.
/// Following Golub & Van Loan 2013 (§8.7.4, after Stewart's and Van Loan's
/// algorithms): an SVD of Q1 gives U, Z and the cosines; a QR factorisation
/// of Q2*Z gives V and the sines where they are at least 1/sqrt(2); the
/// remaining small sines come from an SVD of the trailing block of that
/// triangular factor, and their cosines from a QR factorisation of Q1*Z.
fn csd(q1: &CMat, q2: &CMat, cancel: &dyn Fn() -> bool) -> Result<(CMat, CMat, CMat, CMat, CMat)> {
    let (m, n, q) = (q1.rows, q2.rows, q1.cols);
    let off = q.saturating_sub(m);
    let k1 = m.min(q);
    let (u0, sigma, w0) = svd_any(q1, cancel)?;
    // Ascending cosines: reverse the singular triplets; with m < q the
    // q - m null vectors of Q1 lead Z.
    let order: Vec<usize> = if m >= q {
        (0..q).rev().chain(q..m).collect()
    } else {
        (0..m).rev().collect()
    };
    let mut u = cols(&u0, &order);
    let mut z = cols(&w0, &(0..q).rev().collect::<Vec<_>>());
    drop((u0, w0));
    let mut cosines = vec![0f64; q];
    let singular = diagk(&sigma, 0);
    for i in 0..k1 {
        cosines[off + i] = singular[k1 - 1 - i].re;
    }
    let split = cosines
        .iter()
        .take_while(|&&v| v <= std::f64::consts::FRAC_1_SQRT_2)
        .count();
    let (v, r) = qr_full(&q2.mul(&z, cancel), cancel);
    let (mut v, mut r) = diagp(v, r, 0);
    if split < q {
        let cs: Vec<usize> = (split..q).collect();
        let rows: Vec<usize> = (split..n).collect();
        if !rows.is_empty() {
            let (ur, sr, wr) = svd_any(&block(&r, &rows, &cs), cancel)?;
            let vt = cols(&v, &rows).mul(&ur, cancel);
            put_cols(&mut v, &rows, &vt);
            let zt = cols(&z, &cs).mul(&wr, cancel);
            put_cols(&mut z, &cs, &zt);
            put_block(&mut r, &rows, &cs, &sr);
        }
        let (ut, rt) = qr_econ(&q1.mul(&cols(&z, &cs), cancel), cancel);
        let (ut, rt) = diagp(ut, rt, 0);
        put_cols(&mut u, &(split - off..q - off).collect::<Vec<_>>(), &ut);
        for (t, d) in diagk(&rt, 0).into_iter().enumerate() {
            cosines[split + t] = d.re;
        }
    }
    let mut cc = CMat::zeros(m, q);
    for (j, &v0) in cosines.iter().enumerate().skip(off) {
        cc.set(j - off, j, c(v0, 0.));
    }
    Ok((u, v, z, cc, diagf(&r)))
}
