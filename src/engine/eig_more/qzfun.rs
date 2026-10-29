//! qz, ordschur, ordeig and ordqz.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cdense::{self, CMat, c};
use super::{Mat, Num, cgen, ordering, pair_vectors, rgen, text_arg};
use crate::engine::{Engine, Value};
use crate::{Complex64 as C, Result};

const SELECT_LENGTH: &str = "Length of SELECT or CLUSTER vector must match number of eigenvalues.";

/// What to move to the top-left.
enum Selection {
    Select(Vec<bool>),
    Clusters(Vec<usize>),
}

fn as_num(m: CMat) -> Num {
    if m.data.iter().all(|z| z.im == 0.) {
        Num::R(m.re())
    } else {
        Num::C(m)
    }
}

/// Eigenvalues on the (quasi-)triangular diagonal of T.
fn quasi_eigs(t: &Mat, cancel: &dyn Fn() -> bool) -> Vec<C> {
    let n = t.rows;
    let mut e = Vec::with_capacity(n);
    let mut k = 0;
    while k < n {
        if k + 1 < n && t.at(k + 1, k) != 0. {
            let block = Mat::from_vec(
                2,
                2,
                vec![
                    t.at(k, k),
                    t.at(k + 1, k),
                    t.at(k, k + 1),
                    t.at(k + 1, k + 1),
                ],
            );
            match super::dense::geevx(&block, true, false, false, cancel) {
                Ok(r) => {
                    e.push(c(r.wr[0], r.wi[0]));
                    e.push(c(r.wr[1], r.wi[1]));
                }
                Err(_) => {
                    e.push(c(f64::NAN, 0.));
                    e.push(c(f64::NAN, 0.));
                }
            }
            k += 2;
        } else {
            e.push(c(t.at(k, k), 0.));
            k += 1;
        }
    }
    e
}

/// Generalized eigenvalues on the diagonal of a (quasi-)triangular pencil.
fn quasi_eigs_pencil(a: &Mat, b: &Mat, cancel: &dyn Fn() -> bool) -> Vec<C> {
    let n = a.rows;
    let mut e = Vec::with_capacity(n);
    let mut k = 0;
    while k < n {
        if k + 1 < n && a.at(k + 1, k) != 0. {
            let pick = |m: &Mat| {
                Mat::from_vec(
                    2,
                    2,
                    vec![
                        m.at(k, k),
                        m.at(k + 1, k),
                        m.at(k, k + 1),
                        m.at(k + 1, k + 1),
                    ],
                )
            };
            match rgen::ggev(&pick(a), &pick(b), false, false, cancel) {
                Ok(r) => {
                    for i in 0..2 {
                        e.push(c(r.alphar[i] / r.beta[i], r.alphai[i] / r.beta[i]));
                    }
                }
                Err(_) => {
                    e.push(c(f64::NAN, 0.));
                    e.push(c(f64::NAN, 0.));
                }
            }
            k += 2;
        } else {
            e.push(c(a.at(k, k) / b.at(k, k), 0.));
            k += 1;
        }
    }
    e
}

impl Engine {
    /// Peak live storage, in eight-byte budget elements. This includes operand
    /// copies, reduction factors, back-transforms and packaged output copies.
    /// Complex storage consumes two elements per entry. Charge the worst path
    /// before creating conversion buffers or entering a LAPACK kernel.
    pub(super) fn lx3_qz_budget(&self, n: usize, complex: bool, count: usize) -> Result<()> {
        let factors = if count >= 5 { 20 } else { 16 };
        self.preflight_elements(
            n.saturating_mul(n)
                .saturating_mul(factors)
                .saturating_mul(if complex { 2 } else { 1 })
                .saturating_add(n.saturating_mul(32)),
        )
    }
    fn lx3_square_operand(&self, fname: &str, v: &Value) -> Result<Num> {
        Ok(self.lx3_operand(fname, v)?.num)
    }

    /// The SELECT / keyword / CLUSTER argument for n eigenvalues `e`.
    fn lx3_selection(&self, v: &Value, e: &[C]) -> Result<Selection> {
        if let Some(t) = text_arg(v) {
            let f: fn(C) -> bool = match t.to_ascii_lowercase().as_str() {
                "lhp" => |z| z.re < 0.,
                "rhp" => |z| z.re >= 0.,
                "udi" => |z| cdense::cabs(z) < 1.,
                "udo" => |z| cdense::cabs(z) >= 1.,
                _ => return Err(
                    "Invalid region specification. Valid keywords are 'lhp', 'rhp', 'udi', 'udo'."
                        .into(),
                ),
            };
            return Ok(Selection::Select(e.iter().map(|&z| f(z)).collect()));
        }
        match v {
            Value::Logical(m) => {
                if m.len() != e.len() {
                    return Err(SELECT_LENGTH.into());
                }
                Ok(Selection::Select(m.data().iter().map(|&b| b != 0).collect()))
            }
            Value::Matrix(m) => {
                if m.len() != e.len() {
                    return Err(SELECT_LENGTH.into());
                }
                if m.data().iter().any(|&x| !(x >= 0. && x.fract() == 0. && x <= 1e9)) {
                    return Err("CLUSTERS must be a vector of nonnegative integers.".into());
                }
                Ok(Selection::Clusters(m.data().iter().map(|&x| x as usize).collect()))
            }
            _ => Err("Third input must be a logical vector, a vector of cluster indices or a region keyword.".into()),
        }
    }

    pub(super) fn lx3_ordeig(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        let a = self.lx3_square_operand("ordeig", &args[0])?;
        if a.rows() != a.cols() {
            return Err("Input must be square.".into());
        }
        let b = match args.get(1) {
            Some(v) => {
                let b = self.lx3_square_operand("ordeig", v)?;
                if b.rows() != a.rows() || b.cols() != a.cols() {
                    return Err("Inputs must be square and of the same size.".into());
                }
                Some(b)
            }
            None => None,
        };
        let n = a.rows();
        let cancel = self.lx3_cancelled();
        let e: Vec<C> = match (&a, &b) {
            (Num::R(a), None) => quasi_eigs(a, &cancel),
            (Num::C(a), None) => (0..n).map(|i| a.at(i, i)).collect(),
            (Num::R(a), Some(Num::R(b))) => quasi_eigs_pencil(a, b, &cancel),
            (a, Some(b)) => {
                let (a, b) = (a.complex(), b.complex());
                (0..n)
                    .map(|i| cdense::cdiv(a.at(i, i), b.at(i, i)))
                    .collect()
            }
        };
        drop(cancel);
        let col = CMat {
            rows: n,
            cols: 1,
            data: e,
        };
        Ok(vec![self.lx3_num_value(as_num(col), false)?])
    }

    pub(super) fn lx3_ordschur(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err("Not enough input arguments.".into());
        }
        let u = self.lx3_square_operand("ordschur", &args[0])?;
        let t = self.lx3_square_operand("ordschur", &args[1])?;
        let n = t.rows();
        if t.cols() != n || u.rows() != u.cols() || u.rows() != n {
            return Err(
                "The Schur matrix and the unitary factor must be square and of the same size."
                    .into(),
            );
        }
        if !u.finite() || !t.finite() {
            return Err("Inputs must not contain NaN or Inf.".into());
        }
        let cancel = self.lx3_cancelled();
        let e = match &t {
            Num::R(m) => quasi_eigs(m, &cancel),
            Num::C(m) => (0..n).map(|i| m.at(i, i)).collect(),
        };
        drop(cancel);
        let sel = self.lx3_selection(&args[2], &e)?;
        let failed = || {
            "ordschur: reordering failed because some eigenvalues are too close to separate (the problem is very ill-conditioned).".to_string()
        };
        let (us, ts) = match (u, t) {
            (Num::R(mut u), Num::R(mut t)) => {
                match sel {
                    Selection::Select(s) => {
                        ordering::trsen(&mut t, &mut u, &s, &self.lx3_cancelled())
                            .map_err(|_| failed())?
                    }
                    Selection::Clusters(cl) => {
                        ordering::ordschur_clusters(&mut u, &mut t, &cl, &self.lx3_cancelled())
                            .map_err(|_| failed())?
                    }
                }
                (Num::R(u), Num::R(t))
            }
            (u, t) => {
                let (mut u, mut t) = (u.complex(), t.complex());
                match sel {
                    Selection::Select(s) => {
                        ordering::ztrsen(&mut t, &mut u, &s, &self.lx3_cancelled())
                    }
                    Selection::Clusters(cl) => {
                        ordering::zordschur_clusters(&mut u, &mut t, &cl, &self.lx3_cancelled())?
                    }
                }
                (Num::C(u), Num::C(t))
            }
        };
        let mut out = vec![self.lx3_num_value(us, false)?];
        if count >= 2 {
            out.push(self.lx3_num_value(ts, false)?);
        }
        Ok(out)
    }

    pub(super) fn lx3_qz(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let a = self.lx3_square_operand("qz", &args[0])?;
        let b = self.lx3_square_operand("qz", &args[1])?;
        let mode = match args.get(2) {
            None => None,
            Some(v) => match text_arg(v).map(|t| t.to_ascii_lowercase()).as_deref() {
                Some("real") => Some(true),
                Some("complex") => Some(false),
                _ => return Err("Third argument must be 'real' or 'complex'.".into()),
            },
        };
        let n = a.rows();
        if a.cols() != n || b.rows() != n || b.cols() != n {
            return Err("Matrix dimensions must agree.".into());
        }
        if !a.finite() || !b.finite() {
            return Err("Input to QZ must not contain NaN or Inf.".into());
        }
        let cancel = self.lx3_cancelled();
        let want_v = count >= 5;
        let failed = |_| "qz: the QZ iteration failed to converge".to_string();
        let real_path = mode == Some(true) && matches!((&a, &b), (Num::R(_), Num::R(_)));
        self.lx3_qz_budget(n, !real_path, count)?;
        let outputs: Vec<Num> = if real_path {
            let (Num::R(a), Num::R(b)) = (a, b) else {
                unreachable!()
            };
            let (mut h, mut t, mut q, mut z) = real_qz(&a, &b, &self.lx3_cancelled());
            let r = rgen::hgeqz_with_rounding(
                true,
                &mut h,
                &mut t,
                0,
                n.saturating_sub(1),
                Some(&mut q),
                Some(&mut z),
                &cancel,
                true,
            )
            .map_err(failed)?;
            let mut outs = vec![
                Num::R(h.clone()),
                Num::R(t.clone()),
                Num::R(q.transpose()),
                Num::R(z.clone()),
            ];
            if want_v {
                let (mut vl, mut vr) = (q.clone(), z.clone());
                rgen::tgevc(&h, &t, Some(&mut vl), Some(&mut vr), &self.lx3_cancelled())
                    .map_err(failed)?;
                let real = r.alphai.iter().all(|&v| v == 0.);
                let wrap = |v: Mat| {
                    if real {
                        Num::R(v)
                    } else {
                        Num::C(pair_vectors(&v, &r.alphai))
                    }
                };
                outs.push(wrap(vr));
                outs.push(wrap(vl));
            }
            outs
        } else {
            let (a, b) = (a.complex(), b.complex());
            let (mut h, mut t, mut q, mut z) = complex_qz(&a, &b, &self.lx3_cancelled());
            cgen::zhgeqz(
                true,
                &mut h,
                &mut t,
                0,
                n.saturating_sub(1),
                Some(&mut q),
                Some(&mut z),
                &cancel,
            )
            .map_err(failed)?;
            let mut outs = vec![h.clone(), t.clone(), q.adjoint(), z.clone()];
            if want_v {
                let (mut vl, mut vr) = (q.clone(), z.clone());
                cgen::ztgevc(&h, &t, Some(&mut vl), Some(&mut vr), &self.lx3_cancelled());
                outs.push(vr);
                outs.push(vl);
            }
            if mode == Some(false) {
                outs.into_iter().map(Num::C).collect()
            } else {
                outs.into_iter().map(as_num).collect()
            }
        };
        drop(cancel);
        outputs
            .into_iter()
            .take(count.max(1))
            .map(|o| self.lx3_num_value(o, false))
            .collect()
    }

    pub(super) fn lx3_ordqz(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 5 {
            return Err("Not enough input arguments.".into());
        }
        let aa = self.lx3_square_operand("ordqz", &args[0])?;
        let bb = self.lx3_square_operand("ordqz", &args[1])?;
        let qm = self.lx3_square_operand("ordqz", &args[2])?;
        let zm = self.lx3_square_operand("ordqz", &args[3])?;
        let n = aa.rows();
        self.lx3_qz_budget(
            n,
            [&aa, &bb, &qm, &zm].iter().any(|m| matches!(m, Num::C(_))),
            count,
        )?;
        for m in [&aa, &bb, &qm, &zm] {
            if m.rows() != n || m.cols() != n {
                return Err(
                    "The first four inputs must be square matrices of the same size.".into(),
                );
            }
            if !m.finite() {
                return Err("Inputs must not contain NaN or Inf.".into());
            }
        }
        let cancel = self.lx3_cancelled();
        let e: Vec<C> = match (&aa, &bb) {
            (Num::R(a), Num::R(b)) => quasi_eigs_pencil(a, b, &cancel),
            (a, b) => {
                let (a, b) = (a.complex(), b.complex());
                (0..n)
                    .map(|i| cdense::cdiv(a.at(i, i), b.at(i, i)))
                    .collect()
            }
        };
        drop(cancel);
        let sel = self.lx3_selection(&args[4], &e)?;
        let failed = || {
            "ordqz: reordering failed because some eigenvalues are too close to separate (the problem is very ill-conditioned).".to_string()
        };
        let all_real = [&aa, &bb, &qm, &zm].iter().all(|m| matches!(m, Num::R(_)));
        let outs: Vec<Num> = if all_real {
            let (Num::R(mut a), Num::R(mut b), Num::R(q), Num::R(mut z)) = (aa, bb, qm, zm) else {
                unreachable!()
            };
            let mut ql = q.transpose();
            let select = match sel {
                Selection::Select(s) => vec![s],
                Selection::Clusters(cl) => {
                    cluster_steps(&cl, |k| k + 1 < n && a.at(k + 1, k) != 0.)
                }
            };
            let mut labels_ok = true;
            for s in select {
                if ordering::tgsen(&mut a, &mut b, &mut ql, &mut z, &s, &self.lx3_cancelled())
                    .is_err()
                {
                    labels_ok = false;
                    break;
                }
            }
            if !labels_ok {
                return Err(failed());
            }
            vec![Num::R(a), Num::R(b), Num::R(ql.transpose()), Num::R(z)]
        } else {
            let (mut a, mut b, q, mut z) = (aa.complex(), bb.complex(), qm.complex(), zm.complex());
            let mut ql = q.adjoint();
            let select = match sel {
                Selection::Select(s) => vec![s],
                Selection::Clusters(cl) => cluster_steps(&cl, |_| false),
            };
            for s in select {
                ordering::ztgsen(&mut a, &mut b, &mut ql, &mut z, &s, &self.lx3_cancelled())
                    .map_err(|_| failed())?;
            }
            vec![Num::C(a), Num::C(b), Num::C(ql.adjoint()), Num::C(z)]
        };
        outs.into_iter()
            .take(count.max(1))
            .map(|o| self.lx3_num_value(o, false))
            .collect()
    }
}

/// The select vectors that realise a cluster ordering step by step.
fn cluster_steps(clusters: &[usize], pair: impl Fn(usize) -> bool) -> Vec<Vec<bool>> {
    let mut labels = clusters.to_vec();
    let mut k = 0;
    while k + 1 < labels.len() {
        if pair(k) {
            let label = labels[k].max(labels[k + 1]);
            labels[k] = label;
            labels[k + 1] = label;
            k += 2;
        } else {
            k += 1;
        }
    }
    let mut values = labels.clone();
    values.sort_unstable();
    values.dedup();
    let mut steps = Vec::new();
    for &cv in values.iter().rev().take(values.len().saturating_sub(1)) {
        steps.push(labels.iter().map(|&l| l >= cv).collect::<Vec<bool>>());
        let (sel, rest): (Vec<usize>, Vec<usize>) = labels.iter().partition(|&&l| l >= cv);
        labels = sel.into_iter().chain(rest).collect();
    }
    steps
}

/// QR of B, Q^T applied to A, and dgghrd: (H, T, Q, Z) with LAPACK's Q.
fn real_qz(a: &Mat, b: &Mat, cancel: &dyn Fn() -> bool) -> (Mat, Mat, Mat, Mat) {
    let n = a.rows;
    let mut h = a.clone();
    let mut t = b.clone();
    let tau = rgen::geqr2(&mut t, 0, 0, n, n, cancel);
    rgen::orm2r_lt(&t, 0, 0, n, n, &tau, &mut h, 0, 0, n, cancel);
    let mut q = Mat::identity(n);
    for j in 0..n.saturating_sub(1) {
        for i in j + 1..n {
            q.set(i, j, t.at(i, j));
        }
    }
    super::dense::org2r(&mut q, n, &tau, 0, 0, n, n, cancel);
    let mut z = Mat::identity(n);
    if n > 0 {
        rgen::gghrd(&mut h, &mut t, 0, n - 1, Some(&mut q), Some(&mut z), cancel);
    }
    (h, t, q, z)
}

fn complex_qz(a: &CMat, b: &CMat, cancel: &dyn Fn() -> bool) -> (CMat, CMat, CMat, CMat) {
    let n = a.rows;
    let mut h = a.clone();
    let mut t = b.clone();
    let tau = cgen::zgeqr2(&mut t, 0, 0, n, n, cancel);
    cgen::zunm2r_lc(&t, 0, 0, n, n, &tau, &mut h, 0, 0, n, cancel);
    let mut q = CMat::identity(n);
    for j in 0..n.saturating_sub(1) {
        for i in j + 1..n {
            q.set(i, j, t.at(i, j));
        }
    }
    cdense::zung2r(&mut q, n, &tau, 0, 0, n, n, cancel);
    let mut z = CMat::identity(n);
    if n > 0 {
        cgen::zgghrd(&mut h, &mut t, 0, n - 1, Some(&mut q), Some(&mut z), cancel);
    }
    (h, t, q, z)
}
