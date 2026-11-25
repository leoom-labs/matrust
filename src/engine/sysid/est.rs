//! Estimators. ARX: linear least squares over the regression rows that need
//! no unmeasured past (Ljung, System Identification, 2nd ed., sec. 7.3).
//! ARMAX/OE/BJ: prediction-error minimisation of sum eps^2 with
//! eps = D/C (A y - B/F u) and zero initial conditions (Ljung eqs. 4.9-4.12,
//! 10.40-10.53, analytic gradients), Levenberg-Marquardt. Initial values:
//! ARMAX by the Hannan-Rissanen two-stage regression (high-order ARX
//! residuals as noise regressors), OE by ARX followed by two instrumental-
//! variable steps with simulated instruments (Ljung sec. 7.6), BJ by OE for
//! B/F and Hannan-Rissanen on the OE residuals for C/D. Unstable C, D or F
//! initial estimates are reflected into the unit circle.
//! Continuous models (tfest, ssest, procest) minimise the zero-order-hold
//! simulation error over the continuous parameters (central-difference
//! Jacobian), started from a discrete OE model whose poles are mapped by
//! s = log(z)/Ts and whose numerator is then solved by linear least squares.
//! n4sid: PO-MOESP (Verhaegen 1994, Automatica 30(1); Verhaegen & Verdult,
//! Filtering and System Identification, 2007, ch. 9): RQ of the stacked block
//! Hankel matrices, SVD of the L32 block for the extended observability
//! matrix, A and C from its shift structure, then B, D and x0 by linear least
//! squares over all samples.
use super::super::{Engine, Value};
use super::lti::{Dss, Model, Proc, Structure, realize, trim};
use super::num::{
    C, conv, hurwitz, levenberg_marquardt, lfilter, lstsq, reflect, roots, shift, stable_q, sumsq,
};
use super::obj::{FitStats, IdData, IdModel, MAX_ORDER, report, scalar_of, text_of};
use crate::Result;
use nalgebra::{DMatrix, DVector};
use std::sync::Arc;

const MAX_ITER: usize = 200;
type SsMatrices = (DMatrix<f64>, DMatrix<f64>, DMatrix<f64>, DMatrix<f64>);
/// Bound on samples x parameters (Jacobian entries, 80 MB per copy).
pub(super) const WORK_LIMIT: usize = 10_000_000;

pub(super) fn orders(v: &Value, who: &str, n: usize, what: &str) -> Result<Vec<usize>> {
    let m = super::obj::real_of(v, &format!("{who}: orders"))?;
    if m.len() != n {
        return Err(format!("{who}: the orders must be {what}"));
    }
    m.data()
        .iter()
        .map(|&x| {
            if x >= 0. && x.fract() == 0. && x <= MAX_ORDER as f64 {
                Ok(x as usize)
            } else {
                Err(format!(
                    "{who}: orders must be nonnegative integers of at most {MAX_ORDER}"
                ))
            }
        })
        .collect()
}
/// Name-value options: returns (name lowercased, value) pairs, rejecting
/// names outside `allowed`.
pub(super) fn options(args: &[Value], who: &str, allowed: &[&str]) -> Result<Vec<(String, Value)>> {
    if !args.len().is_multiple_of(2) {
        return Err(format!("{who}: options must be name-value pairs"));
    }
    let mut out = Vec::new();
    for p in args.chunks(2) {
        let name = text_of(&p[0]).ok_or_else(|| format!("{who}: option names must be text"))?;
        let Some(canon) = allowed.iter().find(|a| a.eq_ignore_ascii_case(&name)) else {
            return Err(format!(
                "{who}: unknown option name '{name}'; valid names: {}",
                allowed.join(", ")
            ));
        };
        out.push((canon.to_string(), p[1].clone()));
    }
    Ok(out)
}
fn check_len(n: usize, p: usize, who: &str) -> Result<()> {
    if n < 2 * p + 2 {
        return Err(format!(
            "{who}: the data has too few samples ({n}) for {p} parameters"
        ));
    }
    if n.saturating_mul(p + 1) > WORK_LIMIT {
        return Err(format!(
            "{who}: samples times parameters exceeds the work bound ({WORK_LIMIT})"
        ));
    }
    Ok(())
}

// ------------------------------------------------------- linear regression --
fn regressors(
    y: &[f64],
    u: &[f64],
    na: usize,
    nb: usize,
    nk: usize,
    extra: &[(&[f64], usize)],
    start: usize,
) -> DMatrix<f64> {
    let n = y.len();
    let p = na + nb + extra.iter().map(|x| x.1).sum::<usize>();
    let rows = n.saturating_sub(start);
    let mut m = DMatrix::zeros(rows, p);
    for (r, t) in (start..n).enumerate() {
        let mut c = 0;
        for i in 1..=na {
            m[(r, c)] = if t >= i { -y[t - i] } else { 0. };
            c += 1;
        }
        for i in 0..nb {
            let l = nk + i;
            m[(r, c)] = if t >= l { u[t - l] } else { 0. };
            c += 1;
        }
        for (x, k) in extra {
            for i in 1..=*k {
                m[(r, c)] = if t >= i { x[t - i] } else { 0. };
                c += 1;
            }
        }
    }
    m
}
/// ARX least squares; returns (a, b) with b carrying nk leading zeros.
pub(super) fn arx_ls(
    y: &[f64],
    u: &[f64],
    na: usize,
    nb: usize,
    nk: usize,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let start = na.max(if nb > 0 { nb + nk - 1 } else { 0 });
    if y.len() <= start + na + nb {
        return Err("arx: the data has too few samples for the requested orders".into());
    }
    let phi = regressors(y, u, na, nb, nk, &[], start);
    let rhs = DVector::from_column_slice(&y[start..]);
    let th = lstsq(&phi, &rhs)?;
    Ok(unpack_ab(th.as_slice(), na, nb, nk))
}
fn unpack_ab(th: &[f64], na: usize, nb: usize, nk: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = vec![1.];
    a.extend_from_slice(&th[..na]);
    let mut b = if nb > 0 { vec![0.; nk] } else { Vec::new() };
    b.extend_from_slice(&th[na..na + nb]);
    (a, b)
}

// ---------------------------------------------------- prediction error --
#[derive(Clone, Copy)]
pub(super) struct Orders {
    na: usize,
    nb: usize,
    nc: usize,
    nd: usize,
    nf: usize,
    nk: usize,
}
#[derive(Clone)]
struct Poly {
    a: Vec<f64>,
    b: Vec<f64>,
    c: Vec<f64>,
    d: Vec<f64>,
    f: Vec<f64>,
}
fn pack(p: &Poly, o: &Orders) -> Vec<f64> {
    let mut t = Vec::new();
    t.extend_from_slice(&p.a[1..]);
    t.extend_from_slice(&p.b[o.nk..o.nk + o.nb]);
    t.extend_from_slice(&p.c[1..]);
    t.extend_from_slice(&p.d[1..]);
    t.extend_from_slice(&p.f[1..]);
    t
}
fn unpack(t: &[f64], o: &Orders) -> Poly {
    let mut i = 0;
    let mut take = |n: usize| {
        let s = &t[i..i + n];
        i += n;
        s.to_vec()
    };
    let monic = |v: Vec<f64>| {
        let mut m = vec![1.];
        m.extend(v);
        m
    };
    let a = monic(take(o.na));
    let mut b = vec![0.; o.nk];
    b.extend(take(o.nb));
    let c = monic(take(o.nc));
    let d = monic(take(o.nd));
    let f = monic(take(o.nf));
    Poly { a, b, c, d, f }
}
fn errors(p: &Poly, y: &[f64], u: &[f64]) -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    if !stable_q(&p.c) || !stable_q(&p.d) || !stable_q(&p.f) {
        return None;
    }
    let w = lfilter(&p.b, &p.f, u);
    let ay = lfilter(&p.a, &[1.], y);
    let v: Vec<f64> = ay.iter().zip(&w).map(|(a, b)| a - b).collect();
    let e = lfilter(&p.d, &p.c, &v);
    if e.iter().any(|x| !x.is_finite()) {
        return None;
    }
    Some((e, v, w))
}
fn jacobian(
    p: &Poly,
    o: &Orders,
    y: &[f64],
    u: &[f64],
    e: &[f64],
    v: &[f64],
    w: &[f64],
) -> DMatrix<f64> {
    let n = y.len();
    let cols = o.na + o.nb + o.nc + o.nd + o.nf;
    let mut j = DMatrix::zeros(n, cols);
    let mut c = 0;
    let mut put = |x: &[f64], k: usize, sign: f64, c: &mut usize| {
        let s = shift(x, k);
        for (r, val) in s.iter().enumerate() {
            j[(r, *c)] = sign * val;
        }
        *c += 1;
    };
    if o.na > 0 {
        let gy = lfilter(&p.d, &p.c, y);
        for k in 1..=o.na {
            put(&gy, k, 1., &mut c);
        }
    }
    let cf = conv(&p.c, &p.f);
    if o.nb > 0 {
        let gu = lfilter(&p.d, &cf, u);
        for k in 0..o.nb {
            put(&gu, o.nk + k, -1., &mut c);
        }
    }
    if o.nc > 0 {
        let ge = lfilter(&[1.], &p.c, e);
        for k in 1..=o.nc {
            put(&ge, k, -1., &mut c);
        }
    }
    if o.nd > 0 {
        let gv = lfilter(&[1.], &p.c, v);
        for k in 1..=o.nd {
            put(&gv, k, 1., &mut c);
        }
    }
    if o.nf > 0 {
        let gw = lfilter(&p.d, &cf, w);
        for k in 1..=o.nf {
            put(&gw, k, 1., &mut c);
        }
    }
    j
}
fn pem(e: &Engine, init: Poly, o: &Orders, y: &[f64], u: &[f64]) -> Result<(Poly, usize, String)> {
    pem_n(e, init, o, y, u, MAX_ITER)
}
/// `pem` with an explicit iteration bound (the 'MaxIterations' option).
fn pem_n(
    e: &Engine,
    init: Poly,
    o: &Orders,
    y: &[f64],
    u: &[f64],
    max_iter: usize,
) -> Result<(Poly, usize, String)> {
    let theta = pack(&init, o);
    let cancel = || e.check_cancel();
    let mut r = |t: &[f64]| errors(&unpack(t, o), y, u).map(|x| x.0);
    let mut jac = |t: &[f64], _: &[f64]| {
        let p = unpack(t, o);
        let (ee, v, w) = errors(&p, y, u).expect("admissible");
        jacobian(&p, o, y, u, &ee, &v, &w)
    };
    let lm = levenberg_marquardt(theta, max_iter, &cancel, &mut r, &mut jac)?;
    Ok((unpack(&lm.theta, o), lm.iterations, lm.why.to_string()))
}
/// Hannan-Rissanen residuals: a high-order ARX (or AR when nb = 0) fit.
fn hr_residuals(y: &[f64], u: &[f64], nb: bool, nk: usize, order: usize) -> Result<Vec<f64>> {
    let nh = order.min(y.len() / 4).max(1);
    let (a, b) = arx_ls(y, u, nh, if nb { nh } else { 0 }, nk)?;
    let ay = lfilter(&a, &[1.], y);
    let bu = if nb {
        lfilter(&b, &[1.], u)
    } else {
        vec![0.; y.len()]
    };
    Ok(ay.iter().zip(&bu).map(|(p, q)| p - q).collect())
}
fn oe_init(y: &[f64], u: &[f64], nb: usize, nf: usize, nk: usize) -> Result<(Vec<f64>, Vec<f64>)> {
    let (mut f, mut b) = arx_ls(y, u, nf, nb, nk)?;
    f = reflect(&f);
    let start = nf.max(nb + nk - 1);
    for _ in 0..2 {
        let x = lfilter(&b, &f, u);
        let zeta = regressors(&x, u, nf, nb, nk, &[], start);
        let phi = regressors(y, u, nf, nb, nk, &[], start);
        let lhs = zeta.transpose() * &phi;
        let rhs = zeta.transpose() * DVector::from_column_slice(&y[start..]);
        let th = lstsq(&lhs, &rhs)?;
        let (ff, bb) = unpack_ab(th.as_slice(), nf, nb, nk);
        if ff.iter().chain(&bb).any(|v| !v.is_finite()) {
            break;
        }
        f = reflect(&ff);
        b = bb;
    }
    Ok((b, f))
}
/// ARMA(nd, nc) for v: D v = C e (Hannan-Rissanen), returns (c, d).
fn arma_init(v: &[f64], nc: usize, nd: usize) -> Result<(Vec<f64>, Vec<f64>)> {
    if nc == 0 && nd == 0 {
        return Ok((vec![1.], vec![1.]));
    }
    let eh = hr_residuals(v, v, false, 1, (2 * (nc + nd)).max(8))?;
    let start = nc.max(nd);
    let phi = regressors(v, v, nd, 0, 1, &[(&eh, nc)], start);
    let rhs: Vec<f64> = (start..v.len()).map(|t| v[t] - eh[t]).collect();
    let th = lstsq(&phi, &DVector::from_vec(rhs))?;
    let mut d = vec![1.];
    d.extend_from_slice(&th.as_slice()[..nd]);
    let mut c = vec![1.];
    c.extend_from_slice(&th.as_slice()[nd..]);
    if c.iter().chain(&d).any(|x| !x.is_finite()) {
        let mut c0 = vec![0.; nc + 1];
        let mut d0 = vec![0.; nd + 1];
        c0[0] = 1.;
        d0[0] = 1.;
        return Ok((c0, d0));
    }
    Ok((reflect(&c), reflect(&d)))
}

pub(super) fn fit_stats(
    m: &Model,
    y: &[f64],
    u: &[f64],
    t: f64,
    iterations: usize,
    why: &str,
) -> Result<FitStats> {
    let eps = m.innovations(y, u, t)?;
    let n = y.len();
    let loss = sumsq(&eps) / n as f64;
    let mean = y.iter().sum::<f64>() / n as f64;
    let den = y
        .iter()
        .map(|v| (v - mean) * (v - mean))
        .sum::<f64>()
        .sqrt();
    let fit = 100. * (1. - sumsq(&eps).sqrt() / den);
    Ok(FitStats {
        fit,
        loss,
        mse: loss,
        n,
        np: m.parameters(),
        iterations,
        why: why.into(),
    })
}
fn finish(
    e: &Engine,
    data: &IdData,
    model: Model,
    delays: (f64, f64),
    method: &str,
    run: (usize, &str),
    yu: (&[f64], &[f64]),
) -> Result<Value> {
    let (io_delay, input_delay) = delays;
    let (iterations, why) = run;
    let (y, u) = yu;
    let stats = fit_stats(&model, y, u, data.ts, iterations, why)?;
    let mut model = model;
    model.noise_variance = stats.loss;
    let mut m = IdModel::new(e, model, io_delay, input_delay)?;
    if let Some(a) = data.meta("InputName").filter(|_| data.nu() == 1) {
        m.set_meta("InputName", a.clone());
    }
    if let Some(a) = data.meta("OutputName") {
        m.set_meta("OutputName", a.clone());
    }
    if let Some(a) = data.meta("TimeUnit") {
        m.set_meta("TimeUnit", a.clone());
    }
    let status = format!("Estimated using {method} on time domain data.");
    m.set_meta(
        "Report",
        Arc::new(report(e, method, &status, Some(&stats))?),
    );
    m.encode(e)
}

/// arx, armax, oe, bj.
pub(super) fn polynomial(e: &Engine, who: &str, data: &IdData, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(format!("{who}: expected the orders as the second argument"));
    }
    let opts = options(&args[1..], who, &["InputDelay", "MaxIterations"])?;
    let mut extra_delay = 0usize;
    let mut max_iter = MAX_ITER;
    for (n, v) in &opts {
        let x = scalar_of(v, &format!("{who}: {n}"))?;
        if !(x >= 0. && x.fract() == 0. && x <= 1e6) {
            return Err(format!("{who}: {n} must be a nonnegative integer"));
        }
        if n == "InputDelay" {
            extra_delay = x as usize;
        } else {
            max_iter = x as usize;
        }
    }
    let time_series = data.nu() == 0;
    if time_series && who != "arx" {
        return Err(format!("{who}: the data must contain an input channel"));
    }
    let (y, u0) = data.siso(who, !time_series)?;
    let u0 = if time_series { Vec::new() } else { u0 };
    let u = shift(&u0, extra_delay);
    let o = match who {
        "arx" if time_series => {
            let v = orders(&args[0], who, 1, "na for time-series data")?;
            Orders {
                na: v[0],
                nb: 0,
                nc: 0,
                nd: 0,
                nf: 0,
                nk: 0,
            }
        }
        "arx" => {
            let v = orders(&args[0], who, 3, "[na nb nk]")?;
            Orders {
                na: v[0],
                nb: v[1],
                nc: 0,
                nd: 0,
                nf: 0,
                nk: v[2],
            }
        }
        "armax" => {
            let v = orders(&args[0], who, 4, "[na nb nc nk]")?;
            Orders {
                na: v[0],
                nb: v[1],
                nc: v[2],
                nd: 0,
                nf: 0,
                nk: v[3],
            }
        }
        "oe" => {
            let v = orders(&args[0], who, 3, "[nb nf nk]")?;
            Orders {
                na: 0,
                nb: v[0],
                nc: 0,
                nd: 0,
                nf: v[1],
                nk: v[2],
            }
        }
        _ => {
            let v = orders(&args[0], who, 5, "[nb nc nd nf nk]")?;
            Orders {
                na: 0,
                nb: v[0],
                nc: v[1],
                nd: v[2],
                nf: v[3],
                nk: v[4],
            }
        }
    };
    if who != "arx" && o.nb == 0 {
        return Err(format!("{who}: nb must be at least 1"));
    }
    let p = o.na + o.nb + o.nc + o.nd + o.nf;
    check_len(y.len(), p + o.nk, who)?;
    let (poly, iterations, why) = match who {
        "arx" => {
            let (a, b) = arx_ls(&y, &u, o.na, o.nb, o.nk)?;
            (
                Poly {
                    a,
                    b,
                    c: vec![1.],
                    d: vec![1.],
                    f: vec![1.],
                },
                0,
                String::new(),
            )
        }
        "armax" => {
            let eh = hr_residuals(&y, &u, true, o.nk, (2 * (o.na + o.nb + o.nc)).max(10))?;
            let start = o.na.max(o.nb + o.nk - 1).max(o.nc);
            let phi = regressors(&y, &u, o.na, o.nb, o.nk, &[(&eh, o.nc)], start);
            let rhs: Vec<f64> = (start..y.len()).map(|t| y[t] - eh[t]).collect();
            let th = lstsq(&phi, &DVector::from_vec(rhs))?;
            let (a, b) = unpack_ab(th.as_slice(), o.na, o.nb, o.nk);
            let mut c = vec![1.];
            c.extend_from_slice(&th.as_slice()[o.na + o.nb..]);
            let init = Poly {
                a,
                b,
                c: reflect(&c),
                d: vec![1.],
                f: vec![1.],
            };
            pem_n(e, init, &o, &y, &u, max_iter)?
        }
        "oe" => {
            let (b, f) = oe_init(&y, &u, o.nb, o.nf, o.nk)?;
            pem_n(
                e,
                Poly {
                    a: vec![1.],
                    b,
                    c: vec![1.],
                    d: vec![1.],
                    f,
                },
                &o,
                &y,
                &u,
                max_iter,
            )?
        }
        _ => {
            let oo = Orders { nc: 0, nd: 0, ..o };
            let (b, f) = oe_init(&y, &u, o.nb, o.nf, o.nk)?;
            let (first, ..) = pem_n(
                e,
                Poly {
                    a: vec![1.],
                    b,
                    c: vec![1.],
                    d: vec![1.],
                    f,
                },
                &oo,
                &y,
                &u,
                max_iter,
            )?;
            let w = lfilter(&first.b, &first.f, &u);
            let v: Vec<f64> = y.iter().zip(&w).map(|(a, b)| a - b).collect();
            let (c, d) = arma_init(&v, o.nc, o.nd)?;
            pem_n(e, Poly { c, d, ..first }, &o, &y, &u, max_iter)?
        }
    };
    let model = Model {
        s: Structure::Poly {
            a: poly.a,
            b: poly.b,
            c: poly.c,
            d: poly.d,
            f: poly.f,
        },
        ts: data.ts,
        delay: 0.,
        noise_variance: 0.,
    };
    let method = who.to_ascii_uppercase();
    let model = Model {
        delay: extra_delay as f64,
        ..model
    };
    finish(
        e,
        data,
        model,
        (0., extra_delay as f64),
        &method,
        (iterations, &why),
        (&y, &u0),
    )
}

// ------------------------------------------------------ continuous models --
fn numeric_jacobian(
    t: &[f64],
    r0: &[f64],
    r: &mut dyn FnMut(&[f64]) -> Option<Vec<f64>>,
    scale: &[f64],
) -> DMatrix<f64> {
    let n = r0.len();
    let mut j = DMatrix::zeros(n, t.len());
    for i in 0..t.len() {
        let h = 1e-6 * (t[i].abs().max(scale[i]));
        let mut tp = t.to_vec();
        tp[i] += h;
        let mut tm = t.to_vec();
        tm[i] -= h;
        match (r(&tp), r(&tm)) {
            (Some(a), Some(b)) => {
                for k in 0..n {
                    j[(k, i)] = (a[k] - b[k]) / (2. * h);
                }
            }
            (Some(a), None) => {
                for k in 0..n {
                    j[(k, i)] = (a[k] - r0[k]) / h;
                }
            }
            (None, Some(b)) => {
                for k in 0..n {
                    j[(k, i)] = (r0[k] - b[k]) / h;
                }
            }
            (None, None) => {}
        }
    }
    j
}
fn sim_error(m: &Model, y: &[f64], u: &[f64], t: f64) -> Option<Vec<f64>> {
    let s = m.simulate(u, t).ok()?;
    let e: Vec<f64> = y.iter().zip(&s).map(|(a, b)| a - b).collect();
    if e.iter().all(|x| x.is_finite()) {
        Some(e)
    } else {
        None
    }
}
fn minimise(
    e: &Engine,
    theta: Vec<f64>,
    build: &dyn Fn(&[f64]) -> Option<Model>,
    y: &[f64],
    u: &[f64],
    t: f64,
) -> Result<(Vec<f64>, usize, String)> {
    let cancel = || e.check_cancel();
    let scale: Vec<f64> = theta.iter().map(|x| x.abs().max(1e-3)).collect();
    let mut r = |th: &[f64]| build(th).and_then(|m| sim_error(&m, y, u, t));
    let mut r2 = |th: &[f64]| build(th).and_then(|m| sim_error(&m, y, u, t));
    let mut jac = |th: &[f64], r0: &[f64]| numeric_jacobian(th, r0, &mut r2, &scale);
    let lm = levenberg_marquardt(theta, MAX_ITER, &cancel, &mut r, &mut jac)?;
    Ok((lm.theta, lm.iterations, lm.why.to_string()))
}
/// Continuous poles from discrete ones: s = log(z)/T (real part only for
/// negative real z, which has no continuous zero-order-hold preimage).
fn continuous_poles(f: &[f64], t: f64) -> Vec<C> {
    roots(f)
        .into_iter()
        .map(|z| {
            if z.norm() < 1e-8 {
                return C::new(-1. / t * 5., 0.);
            }
            let l = z.ln() / t;
            if z.im.abs() < 1e-12 && z.re < 0. {
                C::new(l.re, 0.)
            } else {
                l
            }
        })
        .collect()
}
fn tf_model(den_tail: &[f64], num: &[f64], ts: f64, delay: f64) -> Model {
    let mut den = vec![1.];
    den.extend_from_slice(den_tail);
    Model {
        s: Structure::Tf {
            num: num.to_vec(),
            den,
        },
        ts,
        delay,
        noise_variance: 0.,
    }
}
/// Continuous transfer function with np poles and nz zeros: returns
/// (num, den, iterations, why).
fn tf_continuous(
    e: &Engine,
    y: &[f64],
    u: &[f64],
    t: f64,
    np: usize,
    nz: usize,
    delay: f64,
) -> Result<(Vec<f64>, Vec<f64>, usize, String)> {
    let dint = (delay / t + 1e-9).floor() as usize;
    let (nb, nk) = if nz >= np {
        (np + 1, dint)
    } else {
        (np, dint + 1)
    };
    let (b, f) = oe_init(y, u, nb, np, nk)?;
    let o = Orders {
        na: 0,
        nb,
        nc: 0,
        nd: 0,
        nf: np,
        nk,
    };
    let (pd, ..) = pem(
        e,
        Poly {
            a: vec![1.],
            b,
            c: vec![1.],
            d: vec![1.],
            f,
        },
        &o,
        y,
        u,
    )?;
    let poles = continuous_poles(&pd.f, t);
    let mut den = super::num::poly(&poles);
    if !hurwitz(&den) || den.iter().any(|x| !x.is_finite()) {
        den = super::num::poly(&vec![C::new(-1. / (10. * t), 0.); np]);
    }
    // Numerator by linear least squares on basis responses.
    let mut basis = DMatrix::zeros(y.len(), nz + 1);
    for j in 0..=nz {
        let mut num = vec![0.; nz + 1];
        num[j] = 1.;
        let m = tf_model(&den[1..], &num, 0., delay);
        let s = m.simulate(u, t)?;
        basis.column_mut(j).copy_from_slice(&s);
    }
    let num = lstsq(&basis, &DVector::from_column_slice(y))?;
    let mut theta: Vec<f64> = den[1..].to_vec();
    theta.extend(num.iter());
    let build = |th: &[f64]| -> Option<Model> {
        if !hurwitz(&[&[1.], &th[..np]].concat()) {
            return None;
        }
        Some(tf_model(&th[..np], &th[np..], 0., delay))
    };
    let (th, it, why) = minimise(e, theta, &build, y, u, t)?;
    let mut den = vec![1.];
    den.extend_from_slice(&th[..np]);
    Ok((th[np..].to_vec(), den, it, why))
}
fn order_arg(v: &Value, who: &str, what: &str) -> Result<usize> {
    let x = scalar_of(v, &format!("{who}: {what}"))?;
    if !(x >= 0. && x.fract() == 0. && x <= MAX_ORDER as f64) {
        return Err(format!(
            "{who}: {what} must be a nonnegative integer of at most {MAX_ORDER}"
        ));
    }
    Ok(x as usize)
}
fn ts_option(opts: &[(String, Value)], who: &str) -> Result<Option<f64>> {
    for (n, v) in opts {
        if n == "Ts" {
            let x = scalar_of(v, &format!("{who}: Ts"))?;
            if (x.is_nan() || x < 0.) || !x.is_finite() {
                return Err(format!("{who}: Ts must be 0 or a positive sample time"));
            }
            return Ok(Some(x));
        }
    }
    Ok(None)
}
fn flag(opts: &[(String, Value)], name: &str, who: &str) -> Result<bool> {
    for (n, v) in opts {
        if n == name {
            return Ok(scalar_of(v, &format!("{who}: {name}"))? != 0.);
        }
    }
    Ok(false)
}

/// tfest(data, np, nz, iodelay, 'Ts', T)
pub(super) fn tfest(e: &Engine, data: &IdData, args: &[Value]) -> Result<Value> {
    let who = "tfest";
    let (y, u) = data.siso(who, true)?;
    let np = order_arg(
        args.first().ok_or("tfest: expected the number of poles")?,
        who,
        "np",
    )?;
    if np == 0 {
        return Err("tfest: np must be at least 1".into());
    }
    let mut rest = &args[1..];
    let nz = match rest.first() {
        Some(v) if text_of(v).is_none() => {
            rest = &rest[1..];
            order_arg(v, who, "nz")?
        }
        _ => np - 1,
    };
    if nz > np {
        return Err("tfest: nz must not exceed np".into());
    }
    let delay = match rest.first() {
        Some(v) if text_of(v).is_none() => {
            rest = &rest[1..];
            let d = scalar_of(v, "tfest: iodelay")?;
            if (d.is_nan() || d < 0.) || !d.is_finite() {
                return Err("tfest: iodelay must be a nonnegative finite delay (NaN estimation is not supported yet)".into());
            }
            d
        }
        _ => 0.,
    };
    let opts = options(rest, who, &["Ts", "Feedthrough"])?;
    let feed = flag(&opts, "Feedthrough", who)?;
    check_len(y.len(), np + nz + 1, who)?;
    match ts_option(&opts, who)? {
        Some(ts) if ts > 0. => {
            if (ts - data.ts).abs() > 1e-9 * ts {
                return Err("tfest: Ts must be 0 or the data sample time".into());
            }
            if delay.fract() != 0. {
                return Err("tfest: a discrete iodelay must be a whole number of samples".into());
            }
            if delay >= y.len() as f64 {
                return Err("tfest: the iodelay must be shorter than the data".into());
            }
            // Discrete: B(z^-1)/F(z^-1) with nz+1 numerator terms, one sample
            // transport delay (z-domain proper) unless Feedthrough is true,
            // then OE refinement.
            let nk = usize::from(!feed) + delay as usize;
            let (b, f) = oe_init(&y, &u, nz + 1, np, nk)?;
            let o = Orders {
                na: 0,
                nb: nz + 1,
                nc: 0,
                nd: 0,
                nf: np,
                nk,
            };
            let (p, it, why) = pem(
                e,
                Poly {
                    a: vec![1.],
                    b,
                    c: vec![1.],
                    d: vec![1.],
                    f,
                },
                &o,
                &y,
                &u,
            )?;
            // q^-1 (b0 + .. + bnz q^-nz) / F(q^-1) as descending powers of z
            // (without the q^-1 under Feedthrough): both arrays padded to one
            // length (the IO delay stays separate).
            let mut num = if feed { Vec::new() } else { vec![0.] };
            num.extend_from_slice(&p.b[nk..]);
            let mut den = p.f.clone();
            let len = num.len().max(den.len());
            num.resize(len, 0.);
            den.resize(len, 0.);
            let model = Model {
                s: Structure::Tf {
                    num: trim(&num),
                    den,
                },
                ts,
                delay,
                noise_variance: 0.,
            };
            finish(e, data, model, (delay, 0.), "TFEST", (it, &why), (&y, &u))
        }
        _ => {
            if feed && nz < np {
                return Err("tfest: Feedthrough applies to discrete-time models; a continuous model has a direct term when nz equals np".into());
            }
            if delay / data.ts >= y.len() as f64 {
                return Err("tfest: the iodelay must be shorter than the data".into());
            }
            let (num, den, it, why) = tf_continuous(e, &y, &u, data.ts, np, nz, delay)?;
            let model = Model {
                s: Structure::Tf { num, den },
                ts: 0.,
                delay,
                noise_variance: 0.,
            };
            finish(e, data, model, (delay, 0.), "TFEST", (it, &why), (&y, &u))
        }
    }
}

/// ssest(data, nx) and n4sid(data, nx).
pub(super) fn state_space(e: &Engine, who: &str, data: &IdData, args: &[Value]) -> Result<Value> {
    let (y, u) = data.siso(who, true)?;
    let first = args
        .first()
        .ok_or_else(|| format!("{who}: expected the model order nx"))?;
    let range: Vec<usize> = if text_of(first).is_some_and(|t| t.eq_ignore_ascii_case("best")) {
        (1..=10).collect()
    } else {
        let m = super::obj::real_of(first, &format!("{who}: nx"))?;
        if m.is_empty() {
            return Err(format!("{who}: nx must be a positive integer or a range"));
        }
        m.data()
            .iter()
            .map(|&x| {
                if x >= 1. && x.fract() == 0. && x <= MAX_ORDER as f64 {
                    Ok(x as usize)
                } else {
                    Err(format!(
                        "{who}: nx must be positive integers of at most {MAX_ORDER}"
                    ))
                }
            })
            .collect::<Result<_>>()?
    };
    let opts = options(&args[1..], who, &["Ts", "Feedthrough"])?;
    let feed = flag(&opts, "Feedthrough", who)?;
    let ts_opt = ts_option(&opts, who)?;
    let top = *range.iter().max().expect("nonempty order range");
    // Order selection runs at the largest order: bound the work and admit
    // the stacked Hankel matrices before any of them is built.
    check_len(y.len(), 2 * top + 1, who)?;
    if who == "n4sid" || range.len() > 1 {
        moesp_admit(e, y.len(), top)?;
    }
    let nx = if range.len() == 1 {
        range[0]
    } else {
        moesp_order(&y, &u, &range)?
    };
    check_len(y.len(), 2 * nx + 1, who)?;
    let discrete = match who {
        "n4sid" => ts_opt.is_none_or(|t| t > 0.),
        _ => ts_opt.is_some_and(|t| t > 0.),
    };
    if let Some(t) = ts_opt
        && t > 0.
        && (t - data.ts).abs() > 1e-9 * t
    {
        return Err(format!("{who}: Ts must be 0 or the data sample time"));
    }
    let (model, it, why) = if who == "n4sid" && discrete {
        let (a, b, c, d) = moesp(&y, &u, nx, feed)?;
        let k = DMatrix::zeros(nx, 1);
        (
            Model {
                s: Structure::Ss { a, b, c, d, k },
                ts: data.ts,
                delay: 0.,
                noise_variance: 0.,
            },
            0,
            String::new(),
        )
    } else if discrete {
        let nk = usize::from(!feed);
        let nb = nx + usize::from(feed);
        let (b, f) = oe_init(&y, &u, nb, nx, nk)?;
        let o = Orders {
            na: 0,
            nb,
            nc: 0,
            nd: 0,
            nf: nx,
            nk,
        };
        let (p, it, why) = pem(
            e,
            Poly {
                a: vec![1.],
                b,
                c: vec![1.],
                d: vec![1.],
                f,
            },
            &o,
            &y,
            &u,
        )?;
        let mut num = p.b.clone();
        num.resize(nx + 1, 0.);
        let (a, b, c, d) = realize(&num, &p.f)?;
        (ss_model(a, b, c, d, data.ts), it, why)
    } else {
        let nz = if feed { nx } else { nx - 1 };
        let (num, den, it, why) = tf_continuous(e, &y, &u, data.ts, nx, nz, 0.)?;
        let (a, b, c, d) = realize(&num, &den)?;
        (ss_model(a, b, c, d, 0.), it, why)
    };
    let method = if who == "n4sid" { "N4SID" } else { "SSEST" };
    finish(e, data, model, (0., 0.), method, (it, &why), (&y, &u))
}
fn ss_model(a: DMatrix<f64>, b: DVector<f64>, c: DVector<f64>, d: f64, ts: f64) -> Model {
    let n = a.nrows();
    Model {
        s: Structure::Ss {
            a,
            b: DMatrix::from_column_slice(n, 1, b.as_slice()),
            c: DMatrix::from_row_slice(1, n, c.as_slice()),
            d: DMatrix::from_element(1, 1, d),
            k: DMatrix::zeros(n, 1),
        },
        ts,
        delay: 0.,
        noise_variance: 0.,
    }
}
fn hankel(x: &[f64], start: usize, rows: usize, cols: usize) -> DMatrix<f64> {
    DMatrix::from_fn(rows, cols, |r, c| x[start + r + c])
}
/// L32 of the PO-MOESP RQ factorisation and its singular values.
fn moesp_l32(y: &[f64], u: &[f64], i: usize) -> Result<DMatrix<f64>> {
    let n = y.len();
    if n < 2 * i + 2 * i + 2 {
        return Err("n4sid: the data has too few samples for the requested order".into());
    }
    let j = n - 2 * i + 1;
    let uf = hankel(u, i, i, j);
    let up = hankel(u, 0, i, j);
    let yp = hankel(y, 0, i, j);
    let yf = hankel(y, i, i, j);
    let rows = 4 * i;
    let mut m = DMatrix::zeros(rows, j);
    m.view_mut((0, 0), (i, j)).copy_from(&uf);
    m.view_mut((i, 0), (i, j)).copy_from(&up);
    m.view_mut((2 * i, 0), (i, j)).copy_from(&yp);
    m.view_mut((3 * i, 0), (i, j)).copy_from(&yf);
    // M = L Q with L = R^T from the QR factorisation of M^T.
    let r = m.transpose().qr().r();
    let l = r.transpose();
    Ok(l.view((3 * i, i), (i, 2 * i)).into_owned())
}
/// Peak scratch of `moesp_l32`: the 4i x j stacked Hankel, its transpose for
/// the QR factorisation and the four i x j block Hankels.
fn moesp_admit(e: &Engine, n: usize, nx: usize) -> Result<()> {
    let i = moesp_horizon(nx, n);
    let j = n.saturating_sub(2 * i) + 1;
    let elements = i
        .checked_mul(j)
        .and_then(|x| x.checked_mul(12))
        .ok_or("n4sid: the subspace matrices exceed the element limit")?;
    e.preflight_elements(elements)
}
fn moesp_horizon(nx: usize, n: usize) -> usize {
    (2 * nx + 2)
        .max(5)
        .min((n.saturating_sub(2)) / 4)
        .max(nx + 1)
}
fn moesp_order(y: &[f64], u: &[f64], range: &[usize]) -> Result<usize> {
    let top = *range.iter().max().unwrap();
    let i = moesp_horizon(top, y.len());
    let mut s: Vec<f64> = moesp_l32(y, u, i)?
        .svd(false, false)
        .singular_values
        .iter()
        .copied()
        .collect();
    s.sort_by(|a, b| b.total_cmp(a));
    let mut best = range[0];
    let mut gap = f64::NEG_INFINITY;
    for &n in range {
        if n >= s.len() {
            continue;
        }
        let g = (s[n - 1].max(1e-300)).ln() - (s[n].max(1e-300)).ln();
        if g > gap {
            gap = g;
            best = n;
        }
    }
    Ok(best)
}
fn moesp(y: &[f64], u: &[f64], nx: usize, feed: bool) -> Result<SsMatrices> {
    let i = moesp_horizon(nx, y.len());
    if i <= nx {
        return Err("n4sid: the data has too few samples for the requested order".into());
    }
    let l32 = moesp_l32(y, u, i)?;
    let svd = l32.svd(true, false);
    let uu = svd.u.ok_or("n4sid: SVD failed")?;
    // Sort singular values descending (nalgebra does not guarantee order).
    let mut idx: Vec<usize> = (0..svd.singular_values.len()).collect();
    idx.sort_by(|a, b| svd.singular_values[*b].total_cmp(&svd.singular_values[*a]));
    let mut gamma = DMatrix::zeros(i, nx);
    for (k, &c) in idx.iter().take(nx).enumerate() {
        let s = svd.singular_values[c].sqrt();
        gamma.column_mut(k).copy_from(&(uu.column(c) * s));
    }
    let c = gamma.rows(0, 1).into_owned();
    let up = gamma.rows(0, i - 1).into_owned();
    let down = gamma.rows(1, i - 1).into_owned();
    let mut a = DMatrix::zeros(nx, nx);
    for k in 0..nx {
        let col = lstsq(&up, &down.column(k).into_owned())?;
        a.column_mut(k).copy_from(&col);
    }
    // B, D and x0 by least squares: y = C A^t x0 + sum C A^(t-1-s) B u(s) + D u(t).
    let n = y.len();
    let p = 2 * nx + usize::from(feed);
    let mut phi = DMatrix::zeros(n, p);
    let free = Dss {
        a: a.clone(),
        b: DVector::zeros(nx),
        c: c.row(0).transpose(),
        d: 0.,
        shift: 0,
    }
    .free(n);
    phi.view_mut((0, 0), (n, nx)).copy_from(&free);
    for r in 0..nx {
        let mut e = DVector::zeros(nx);
        e[r] = 1.;
        let s = Dss {
            a: a.clone(),
            b: e,
            c: c.row(0).transpose(),
            d: 0.,
            shift: 0,
        }
        .simulate(u, None);
        phi.column_mut(nx + r).copy_from_slice(&s);
    }
    if feed {
        phi.column_mut(2 * nx).copy_from_slice(u);
    }
    let th = lstsq(&phi, &DVector::from_column_slice(y))?;
    let b = DMatrix::from_column_slice(nx, 1, &th.as_slice()[nx..2 * nx]);
    let d = DMatrix::from_element(1, 1, if feed { th[2 * nx] } else { 0. });
    Ok((a, b, c, d))
}

/// procest(data, type)
pub(super) fn procest(e: &Engine, data: &IdData, args: &[Value]) -> Result<Value> {
    let who = "procest";
    let (y, u) = data.siso(who, true)?;
    let t = data.ts;
    let ty = args
        .first()
        .and_then(text_of)
        .ok_or("procest: expected the model type, e.g. 'P1D'")?;
    let mut p = Proc::parse(&ty)?;
    options(&args[1..], who, &[])?;
    if p.integrator {
        return Err("procest: integrating process models (I) are not supported yet".into());
    }
    let names = p.names();
    check_len(y.len(), names.len(), who)?;
    let np = p.poles;
    // Initial delay: integer scan with ARX [np, np+1, d+1] residuals.
    let dmax = if p.delay { (y.len() / 4).min(100) } else { 0 };
    let mut best = (f64::INFINITY, 0usize);
    for d in 0..=dmax {
        let Ok((a, b)) = arx_ls(&y, &u, np, np + 1, d + usize::from(np > 0)) else {
            break;
        };
        let pred: Vec<f64> = {
            let ay = lfilter(&a, &[1.], &y);
            let bu = lfilter(&b, &[1.], &u);
            ay.iter().zip(&bu).map(|(x, z)| x - z).collect()
        };
        let start = (dmax + np + 1).min(y.len());
        let v = sumsq(&pred[start..]);
        if v < best.0 * (1. - 1e-9) {
            best = (v, d);
        }
    }
    let d = best.1;
    let nk = d + usize::from(np > 0);
    if np == 0 {
        let s = shift(&u, nk);
        let k = s.iter().zip(&y).map(|(a, b)| a * b).sum::<f64>() / sumsq(&s).max(1e-300);
        p.kp = k;
    } else {
        let (b, f) = oe_init(&y, &u, np + 1, np, nk)?;
        let o = Orders {
            na: 0,
            nb: np + 1,
            nc: 0,
            nd: 0,
            nf: np,
            nk,
        };
        let (pd, ..) = pem(
            e,
            Poly {
                a: vec![1.],
                b,
                c: vec![1.],
                d: vec![1.],
                f,
            },
            &o,
            &y,
            &u,
        )?;
        let gain = pd.b.iter().sum::<f64>() / pd.f.iter().sum::<f64>();
        p.kp = if gain.is_finite() { gain } else { 1. };
        let poles = continuous_poles(&pd.f, t);
        let fallback = 10. * t;
        if p.under {
            let mut sorted = poles.clone();
            sorted.sort_by(|a, b| b.norm().total_cmp(&a.norm()));
            let (s1, s2) = (sorted[0], sorted[1]);
            let wn = (s1 * s2).norm().sqrt();
            let zeta = -(s1 + s2).re / (2. * wn);
            p.tw = if wn > 0. && wn.is_finite() {
                1. / wn
            } else {
                fallback
            };
            p.zeta = if zeta > 0. && zeta.is_finite() {
                zeta
            } else {
                0.5
            };
            if np == 3 {
                p.tp[2] = sorted.get(2).map_or(fallback, |s| -1. / s.re);
            }
        } else {
            let mut tps: Vec<f64> = poles
                .iter()
                .map(|s| if s.re < 0. { -1. / s.re } else { fallback })
                .collect();
            tps.sort_by(|a, b| b.total_cmp(a));
            for (k, v) in tps.into_iter().enumerate().take(np) {
                p.tp[k] = v;
            }
        }
        if p.delay {
            // Fraction of the delay inside a sample from the numerator split.
            let bb = &pd.b[nk..];
            let total: f64 = bb.iter().sum();
            let frac = if total != 0. {
                (bb[bb.len() - 1] / total).clamp(0., 0.99)
            } else {
                0.5
            };
            p.td = d as f64 * t + frac * t;
        }
    }
    if p.delay && np == 0 {
        p.td = d as f64 * t;
    }
    let theta: Vec<f64> = names.iter().map(|n| p.get(n)).collect();
    let base = p.clone();
    let build = |th: &[f64]| -> Option<Model> {
        let mut q = base.clone();
        for (n, v) in names.iter().zip(th) {
            q.set(n, *v);
        }
        let ok = names.iter().zip(th).all(|(n, v)| match *n {
            "Kp" | "Tz" => v.is_finite(),
            "Td" => *v >= 0.,
            _ => *v > 0.,
        });
        if !ok {
            return None;
        }
        Some(Model {
            s: Structure::Proc(q),
            ts: 0.,
            delay: 0.,
            noise_variance: 0.,
        })
    };
    let (th, it, why) = minimise(e, theta, &build, &y, &u, t)?;
    let model = build(&th).ok_or("procest: the estimate left the admissible region")?;
    finish(e, data, model, (0., 0.), "PROCEST", (it, &why), (&y, &u))
}
