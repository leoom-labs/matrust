use super::*;
use std::f64::consts::PI;
/// E|z| for unit-variance Gaussian or Student t innovations.
pub(super) fn absolute_moment(m: &Model) -> f64 {
    m.dof.map_or((2. / PI).sqrt(), |nu| {
        ((nu - 2.) / PI).sqrt() * (libm::lgamma((nu - 1.) / 2.) - libm::lgamma(nu / 2.)).exp()
    })
}
fn lag(data: &[f64], pre: &[f64], t: usize, j: usize) -> f64 {
    if t >= j {
        data[t - j]
    } else {
        pre[pre.len() - (j - t)]
    }
}
#[allow(clippy::too_many_arguments)]
fn step(
    m: &Model,
    t: usize,
    innov: &[f64],
    v: &[f64],
    e0: &[f64],
    v0: &[f64],
    moment: f64,
) -> Result<f64> {
    let mut h = m.constant;
    for (i, &b) in m.garch.iter().enumerate() {
        if b != 0. {
            let p = lag(v, v0, t, i + 1);
            h += b * if m.kind == Kind::Egarch { p.ln() } else { p };
        }
    }
    for (i, &a) in m.arch.iter().enumerate() {
        if a != 0. {
            let e = lag(innov, e0, t, i + 1);
            h += a * if m.kind == Kind::Egarch {
                e.abs() / lag(v, v0, t, i + 1).sqrt() - moment
            } else {
                e * e
            };
        }
    }
    for (i, &l) in m.leverage.iter().enumerate() {
        if l != 0. {
            let e = lag(innov, e0, t, i + 1);
            h += l * if m.kind == Kind::Egarch {
                e / lag(v, v0, t, i + 1).sqrt()
            } else if e < 0. {
                e * e
            } else {
                0.
            };
        }
    }
    if m.kind == Kind::Egarch {
        h = h.exp();
    }
    if h > 0. && h.is_finite() {
        Ok(h)
    } else {
        Err(err(
            m.class(),
            "InvalidVariance",
            "Conditional variance must be finite and positive.",
        ))
    }
}
fn validate_pre(m: &Model, e0: &[f64], v0: &[f64]) -> Result<()> {
    if e0.len() < m.q() || v0.len() < m.variance_presample() {
        return Err(err(
            m.class(),
            "InsufficientPresample",
            "Insufficient presample innovations or variances for the model degrees.",
        ));
    }
    if e0.iter().any(|x| !x.is_finite()) || v0.iter().any(|x| !x.is_finite() || *x <= 0.) {
        return Err(err(
            m.class(),
            "InvalidPresample",
            "Presample innovations must be finite and variances positive.",
        ));
    }
    Ok(())
}
/// Variance recursion for a single centered innovation path; no offset subtraction.
/// Explicit presamples must supply at least Q innovations and P variances (max(P,Q)
/// variances for EGARCH). Extra leading entries are ignored. Returns one variance
/// for every innovation, using the MathWorks infer/estimate Algorithms defaults.
pub fn conditional_variance(
    m: &Model,
    innovations: &[f64],
    e0: Option<&[f64]>,
    v0: Option<&[f64]>,
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    m.complete()?;
    estimation_variance(m, innovations, e0, v0, c)
}
/// Finite-sample recursion for native likelihood trials. Model feasibility is
/// handled by SQP, so derivative probes may cross a stationarity/positivity row
/// provided every conditional variance still exists. Public calls validate first.
pub(in crate::engine) fn estimation_variance(
    m: &Model,
    innovations: &[f64],
    e0: Option<&[f64]>,
    v0: Option<&[f64]>,
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    cancel(c)?;
    if innovations.len() > MAX_POINTS || innovations.iter().any(|x| !x.is_finite()) {
        return Err(err(
            m.class(),
            "InvalidData",
            "Innovations must be finite and contain at most 10000000 observations.",
        ));
    }
    let s = if innovations.is_empty() {
        m.unconditional_scale()?
    } else {
        innovations.iter().map(|x| x * x).sum::<f64>() / innovations.len() as f64
    };
    let ep = vec![if m.kind == Kind::Egarch { 0. } else { s.sqrt() }; m.q()];
    let vp = vec![s; m.variance_presample()];
    let ep = e0.unwrap_or(&ep);
    let vp = v0.unwrap_or(&vp);
    if e0.is_some() || v0.is_some() {
        if ep.len() < m.q() || vp.len() < m.variance_presample() {
            return Err(err(
                m.class(),
                "InsufficientPresample",
                "Insufficient presample innovations or variances for the model degrees.",
            ));
        }
        if ep.iter().any(|x| !x.is_finite())
            || v0.is_some_and(|v| v.iter().any(|x| !x.is_finite() || *x <= 0.))
        {
            return Err(err(
                m.class(),
                "InvalidPresample",
                "Presample innovations must be finite and variances positive.",
            ));
        }
    }
    if m.kind == Kind::Egarch && vp.iter().any(|x| *x <= 0.) {
        return Err(err(
            m.class(),
            "InvalidVariance",
            "EGARCH initialization requires a positive second moment or V0.",
        ));
    }
    let mut v = Vec::with_capacity(innovations.len());
    let moment = absolute_moment(m);
    for t in 0..innovations.len() {
        if t.is_multiple_of(1024) {
            cancel(c)?;
        }
        v.push(step(m, t, innovations, &v, ep, vp, moment)?);
    }
    Ok(v)
}
pub(in crate::engine) fn likelihood(
    m: &Model,
    e: &[f64],
    v: &[f64],
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    let normal = -0.5 * (2. * PI).ln();
    let t_constant = m.dof.map(|nu| {
        libm::lgamma((nu + 1.) / 2.) - libm::lgamma(nu / 2.) - 0.5 * ((nu - 2.) * PI).ln()
    });
    let mut ll = Vec::with_capacity(e.len());
    for (i, (&x, &h)) in e.iter().zip(v).enumerate() {
        if i.is_multiple_of(4096) {
            cancel(c)?;
        }
        let value = if let Some(nu) = m.dof {
            t_constant.unwrap() - 0.5 * h.ln() - 0.5 * (nu + 1.) * (x * x / ((nu - 2.) * h)).ln_1p()
        } else {
            normal - 0.5 * (h.ln() + x * x / h)
        };
        if !value.is_finite() {
            return Err(err(
                m.class(),
                "UndefinedLikelihood",
                "Likelihood is undefined at these parameters.",
            ));
        }
        ll.push(value);
    }
    Ok(ll)
}
/// One-step forecast given aligned chronological histories of centered innovations
/// and conditional variances. Requires Q innovations and variance_presample() values.
pub fn one_step_forecast(m: &Model, innovations: &[f64], variances: &[f64]) -> Result<f64> {
    m.complete()?;
    validate_pre(m, innovations, variances)?;
    step(m, 0, &[], &[], innovations, variances, absolute_moment(m))
}
/// Conditional variance forecasts from known aligned histories. GARCH/GJR replace
/// future squared innovations by their variance, negative squares by variance/2.
/// EGARCH forecasts the log variance then exponentiates, without a Jensen correction.
pub fn forecast_path(
    m: &Model,
    n: usize,
    innovations: &[f64],
    variances: &[f64],
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    m.complete()?;
    validate_pre(m, innovations, variances)?;
    if n > MAX_POINTS {
        return Err(err(
            m.class(),
            "InvalidOrder",
            "Forecast horizon exceeds 10000000.",
        ));
    }
    let mut f: Vec<f64> = Vec::with_capacity(n);
    let moment = absolute_moment(m);
    for t in 0..n {
        if t.is_multiple_of(1024) {
            cancel(c)?;
        }
        let mut h = m.constant;
        for (i, &b) in m.garch.iter().enumerate() {
            if b != 0. {
                let v = lag(&f, variances, t, i + 1);
                h += b * if m.kind == Kind::Egarch { v.ln() } else { v };
            }
        }
        for (coeff, is_leverage) in [(&m.arch, false), (&m.leverage, true)] {
            for (i, &a) in coeff.iter().enumerate() {
                if a == 0. {
                    continue;
                }
                let j = i + 1;
                let x = if t >= j {
                    if m.kind == Kind::Egarch {
                        0.
                    } else {
                        f[t - j] * if is_leverage { 0.5 } else { 1. }
                    }
                } else {
                    let e = innovations[innovations.len() - (j - t)];
                    if m.kind == Kind::Egarch {
                        let z = e / variances[variances.len() - (j - t)].sqrt();
                        if is_leverage { z } else { z.abs() - moment }
                    } else if is_leverage && e >= 0. {
                        0.
                    } else {
                        e * e
                    }
                };
                h += a * x;
            }
        }
        if m.kind == Kind::Egarch {
            h = h.exp();
        }
        if !h.is_finite() || h <= 0. {
            return Err(err(
                m.class(),
                "InvalidVariance",
                "Forecast variance must be finite and positive.",
            ));
        }
        f.push(h);
    }
    Ok(f)
}
/// List-wise deletion across paths, preserving time order. Inf is always invalid.
pub(super) fn clean_data(e: &Engine, m: &Matrix) -> Result<Matrix> {
    if m.rows() == 0 || m.cols() == 0 {
        return Ok(m.clone());
    }
    e.preflight_elements(m.len().saturating_mul(2))?;
    if m.len() > MAX_POINTS {
        return Err(err(
            "model",
            "InvalidData",
            "At most 10000000 data elements are supported.",
        ));
    }
    let mut rows = Vec::with_capacity(m.rows());
    for r in 0..m.rows() {
        if r.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let mut missing = false;
        for j in 0..m.cols() {
            let x = m.data()[j * m.rows() + r];
            if x.is_infinite() {
                return Err(err(
                    "model",
                    "InvalidData",
                    "Data must be finite (NaN marks missing observations).",
                ));
            }
            missing |= x.is_nan();
        }
        if !missing {
            rows.push(r);
        }
    }
    if rows.len() == m.rows() {
        return Ok(m.clone());
    }
    let mut data = Vec::with_capacity(rows.len() * m.cols());
    for j in 0..m.cols() {
        for (i, &r) in rows.iter().enumerate() {
            if i.is_multiple_of(1024) {
                e.check_cancel()?;
            }
            data.push(m.data()[j * m.rows() + r]);
        }
    }
    Matrix::from_column_vec(rows.len(), m.cols(), data)
}
/// Synchronize jagged presamples at their last row before list-wise deletion.
pub(super) fn sync_pre(e: &Engine, a: &mut Option<Matrix>, b: &mut Option<Matrix>) -> Result<()> {
    for m in [a.as_ref(), b.as_ref()].into_iter().flatten() {
        if m.rows() == 0 || m.cols() == 0 {
            return Err(err(
                "model",
                "InsufficientPresample",
                "Insufficient presample rows or paths for the model degrees.",
            ));
        }
    }
    let height = a
        .as_ref()
        .map_or(0, Matrix::rows)
        .max(b.as_ref().map_or(0, Matrix::rows));
    let elements = a
        .as_ref()
        .map_or(0, Matrix::len)
        .saturating_add(b.as_ref().map_or(0, Matrix::len));
    e.preflight_elements(elements.saturating_mul(2))?;
    if elements > MAX_POINTS {
        return Err(err(
            "model",
            "InvalidPresample",
            "At most 10000000 presample elements are supported.",
        ));
    }
    let mut keep = Vec::with_capacity(height);
    for r in 0..height {
        if r.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let mut missing = false;
        for m in [a.as_ref(), b.as_ref()].into_iter().flatten() {
            if r < height - m.rows() {
                continue;
            }
            for j in 0..m.cols() {
                let x = m.data()[j * m.rows() + r - (height - m.rows())];
                if x.is_infinite() {
                    return Err(err(
                        "model",
                        "InvalidPresample",
                        "Presample data must be finite.",
                    ));
                }
                missing |= x.is_nan();
            }
        }
        if !missing {
            keep.push(r);
        }
    }
    if keep.len() == height {
        return Ok(());
    }
    for m in [a, b].into_iter().flatten() {
        let rows = keep
            .iter()
            .filter_map(|&r| r.checked_sub(height - m.rows()))
            .collect::<Vec<_>>();
        let mut data = Vec::with_capacity(rows.len() * m.cols());
        for j in 0..m.cols() {
            for (i, &r) in rows.iter().enumerate() {
                if i.is_multiple_of(1024) {
                    e.check_cancel()?;
                }
                data.push(m.data()[j * m.rows() + r]);
            }
        }
        *m = Matrix::from_column_vec(rows.len(), m.cols(), data)?;
    }
    Ok(())
}
pub(super) fn presamples(
    e: &Engine,
    opts: &BTreeMap<String, &Value>,
) -> Result<(Option<Matrix>, Option<Matrix>)> {
    let mut ep = opts.get("e0").map(|v| matrix(e, v)).transpose()?;
    let mut vp = opts.get("v0").map(|v| matrix(e, v)).transpose()?;
    sync_pre(e, &mut ep, &mut vp)?;
    Ok((ep, vp))
}
fn pre_column<'a>(
    m: &Model,
    p: Option<&'a Matrix>,
    j: usize,
    paths: usize,
    variance: bool,
) -> Result<Option<&'a [f64]>> {
    let Some(p) = p else {
        return Ok(None);
    };
    let need = if variance {
        m.variance_presample()
    } else {
        m.q()
    };
    if p.rows() < need || p.cols() == 0 || (p.cols() != 1 && p.cols() < paths) {
        return Err(err(
            m.class(),
            "InsufficientPresample",
            "Insufficient presample rows or paths for the model degrees.",
        ));
    }
    let col = if p.cols() == 1 { 0 } else { j };
    let v = &p.data()[col * p.rows()..(col + 1) * p.rows()];
    if variance && v.iter().any(|x| *x <= 0.) {
        return Err(err(
            m.class(),
            "InvalidPresample",
            "Presample variances must be positive.",
        ));
    }
    Ok(Some(v))
}
pub(super) fn call(e: &mut Engine, n: &str, m: &Model, a: &[Value]) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err("Not enough input arguments.".into());
    }
    if n == "infer" {
        let y = clean_data(e, &matrix(e, &a[0])?)?;
        if y.rows() == 0 || y.cols() == 0 {
            return Err(err(n, "InvalidData", "Response data must be nonempty."));
        }
        e.preflight_elements(y.len().saturating_mul(5))?;
        let opts = pairs(&a[1..], &["E0", "V0"])?;
        let (ep, vp) = presamples(e, &opts)?;
        let mut all = Vec::with_capacity(y.len());
        let mut ll = Vec::with_capacity(y.cols());
        for j in 0..y.cols() {
            e.check_cancel()?;
            let inn = y.data()[j * y.rows()..(j + 1) * y.rows()]
                .iter()
                .map(|x| x - m.offset)
                .collect::<Vec<_>>();
            let v = conditional_variance(
                m,
                &inn,
                pre_column(m, ep.as_ref(), j, y.cols(), false)?,
                pre_column(m, vp.as_ref(), j, y.cols(), true)?,
                &e.cancel,
            )?;
            ll.push(likelihood(m, &inn, &v, &e.cancel)?.iter().sum());
            all.extend(v);
        }
        return Ok(vec![
            Value::Matrix(Matrix::from_column_vec(y.rows(), y.cols(), all)?),
            Value::Matrix(Matrix::from_column_vec(1, y.cols(), ll)?),
        ]);
    }
    let count = order(
        e,
        &a[0],
        if n == "simulate" {
            "numObs"
        } else {
            "numPeriods"
        },
        false,
        MAX_POINTS,
    )?;
    let start = if n == "forecast" && a.get(1).is_some_and(|v| text(v).is_err()) {
        2
    } else {
        1
    };
    let opts = pairs(
        &a[start..],
        if n == "simulate" {
            &["NumPaths", "E0", "V0"]
        } else {
            &["Y0", "V0"]
        },
    )?;
    if n == "simulate" {
        let paths = opts
            .get("numpaths")
            .map(|v| order(e, v, "NumPaths", false, MAX_POINTS))
            .transpose()?
            .unwrap_or(1);
        let total = Matrix::checked_len(count, paths)?;
        if total > MAX_POINTS {
            return Err(err(
                n,
                "InvalidSize",
                "Simulation exceeds 10000000 observations.",
            ));
        }
        e.preflight_elements(
            total.saturating_mul(2)
                + count.saturating_mul(2)
                + paths * (m.q() + m.variance_presample()),
        )?;
        let (ep, vp) = presamples(e, &opts)?;
        let scale = m.unconditional_scale()?;
        let mut vs = Vec::with_capacity(total);
        let mut ys = Vec::with_capacity(total);
        let moment = absolute_moment(m);
        for j in 0..paths {
            e.check_cancel()?;
            let explicit_e = pre_column(m, ep.as_ref(), j, paths, false)?;
            let explicit_v = pre_column(m, vp.as_ref(), j, paths, true)?;
            let e0 = explicit_e
                .map(|x| x[x.len() - m.q()..].to_vec())
                .unwrap_or_else(|| (0..m.q()).map(|_| draw(e, m.dof) * scale.sqrt()).collect());
            let v0 = explicit_v
                .map(|x| x[x.len() - m.variance_presample()..].to_vec())
                .unwrap_or_else(|| vec![scale; m.variance_presample()]);
            let mut inn = Vec::with_capacity(count);
            let mut v = Vec::with_capacity(count);
            for t in 0..count {
                if t.is_multiple_of(1024) {
                    e.check_cancel()?;
                }
                let h = step(m, t, &inn, &v, &e0, &v0, moment)?;
                v.push(h);
                inn.push(draw(e, m.dof) * h.sqrt());
            }
            vs.extend(v);
            ys.extend(inn.into_iter().map(|x| x + m.offset));
        }
        return Ok(vec![
            Value::Matrix(Matrix::from_column_vec(count, paths, vs)?),
            Value::Matrix(Matrix::from_column_vec(count, paths, ys)?),
        ]);
    }
    let mut y0 = if start == 2 {
        if opts.contains_key("y0") {
            return Err(err(n, "InvalidNameValue", "Y0 was specified twice."));
        }
        Some(matrix(e, &a[1])?)
    } else {
        opts.get("y0").map(|v| matrix(e, v)).transpose()?
    };
    let mut v0 = opts.get("v0").map(|v| matrix(e, v)).transpose()?;
    sync_pre(e, &mut y0, &mut v0)?;
    let paths = y0
        .as_ref()
        .map_or(1, Matrix::cols)
        .max(v0.as_ref().map_or(1, Matrix::cols));
    let total = Matrix::checked_len(count, paths)?;
    if total > MAX_POINTS {
        return Err(err(
            n,
            "InvalidSize",
            "Forecast exceeds 10000000 observations.",
        ));
    }
    e.preflight_elements(total.saturating_mul(3) + y0.as_ref().map_or(0, Matrix::len) * 3)?;
    for p in [y0.as_ref(), v0.as_ref()].into_iter().flatten() {
        if p.cols() != 1 && p.cols() != paths {
            return Err(err(
                n,
                "InvalidPresample",
                "Presamples must have one column or the same number of paths.",
            ));
        }
    }
    let mut all = Vec::with_capacity(total);
    for j in 0..paths {
        let scale = m.unconditional_scale()?;
        let inn = pre_column(m, y0.as_ref(), j, paths, false)?
            .map(|x| x.iter().map(|x| x - m.offset).collect::<Vec<_>>())
            .unwrap_or_else(|| {
                vec![
                    if m.kind == Kind::Egarch {
                        0.
                    } else {
                        scale.sqrt()
                    };
                    m.q().max(m.p())
                ]
            });
        let variances = if let Some(v) = pre_column(m, v0.as_ref(), j, paths, true)? {
            v.to_vec()
        } else if y0
            .as_ref()
            .is_some_and(|y| y.rows() >= m.p().max(m.q()) + m.p())
        {
            // garch.forecast infers V0 only from max(P,Q)+P presample responses;
            // shorter Y0 starts from the unconditional variance.
            let mut v = conditional_variance(m, &inn, None, None, &e.cancel)?;
            if v.len() < m.variance_presample() {
                let mut pre = vec![scale; m.variance_presample() - v.len()];
                pre.extend(v);
                v = pre;
            }
            v
        } else {
            vec![scale; m.variance_presample()]
        };
        all.extend(forecast_path(m, count, &inn, &variances, &e.cancel)?);
    }
    Ok(vec![Value::Matrix(Matrix::from_column_vec(
        count, paths, all,
    )?)])
}
/// Standardized t: N(0,1)*sqrt((nu-2)/chi2_nu), using the session MT stream.
fn draw(e: &mut Engine, nu: Option<f64>) -> f64 {
    let z = e.random.next_normal();
    nu.map_or(z, |nu| z * ((nu - 2.) / (2. * gamma(e, nu / 2.))).sqrt())
}
fn gamma(e: &mut Engine, shape: f64) -> f64 {
    // Marsaglia & Tsang (2000), ACM TOMS 26(3), 363-372; shape=nu/2>1.
    let d = shape - 1. / 3.;
    let c = (9. * d).sqrt().recip();
    loop {
        let x = e.random.next_normal();
        let v = 1. + c * x;
        if v <= 0. {
            continue;
        }
        let v = v * v * v;
        let u = e.random.next_f64();
        if u < 1. - 0.0331 * x.powi(4) || u.ln() < 0.5 * x * x + d * (1. - v + v.ln()) {
            return d * v;
        }
    }
}
