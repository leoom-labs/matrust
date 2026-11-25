use super::*;
#[derive(Clone)]
pub(super) struct Sample {
    pub y: Matrix,
    pub x: Option<Matrix>,
    pub y0: Option<Matrix>,
    pub e0: Option<Matrix>,
}
pub(super) fn clean_aligned(e: &Engine, a: Vec<Option<Matrix>>) -> Result<Vec<Option<Matrix>>> {
    let n = a.iter().flatten().map(Matrix::rows).max().unwrap_or(0);
    preflight(
        e,
        n,
        a.iter().flatten().map(Matrix::cols).sum::<usize>().max(1),
        1,
    )?;
    let mut drop = vec![false; n];
    for m in a.iter().flatten() {
        for (r, d) in drop.iter_mut().enumerate().skip(n - m.rows()) {
            let row = r - (n - m.rows());
            for j in 0..m.cols() {
                let x = m.data()[j * m.rows() + row];
                if x.is_infinite() {
                    return Err(err(
                        "arima",
                        "InvalidData",
                        "Time series must contain finite values or NaN.",
                    ));
                }
                if x.is_nan() {
                    *d = true;
                }
            }
        }
    }
    a.into_iter()
        .map(|m| {
            m.map(|m| {
                let off = n - m.rows();
                let keep: Vec<_> = (0..m.rows()).filter(|r| !drop[r + off]).collect();
                if keep.len() == m.rows() {
                    return Ok(m);
                }
                let mut v = Vec::with_capacity(keep.len() * m.cols());
                for j in 0..m.cols() {
                    for r in &keep {
                        v.push(m.data()[j * m.rows() + r]);
                    }
                }
                Matrix::from_column_vec(keep.len(), m.cols(), v)
            })
            .transpose()
        })
        .collect()
}
pub(super) fn optional_matrix(
    e: &Engine,
    o: &BTreeMap<String, Value>,
    n: &str,
) -> Result<Option<Matrix>> {
    o.get(n)
        .map(|v| matrix(e, v))
        .transpose()
        .map(|v| v.filter(|m| !m.is_empty()))
}
pub(super) fn presamples(
    e: &Engine,
    o: &BTreeMap<String, Value>,
) -> Result<(Option<Matrix>, Option<Matrix>)> {
    let mut p = clean_aligned(
        e,
        vec![
            optional_matrix(e, o, "Y0")?,
            optional_matrix(e, o, "E0")?,
            optional_matrix(e, o, "V0")?,
        ],
    )?;
    Ok((p.remove(0), p.remove(0)))
}
pub(super) fn sample(
    e: &Engine,
    m: &Model,
    y: &Value,
    o: &BTreeMap<String, Value>,
) -> Result<Sample> {
    let y = matrix(e, y)?;
    if y.is_empty() || y.cols() == 0 {
        return Err(err(
            "arima",
            "InsufficientData",
            "Response data must not be empty.",
        ));
    }
    let (mut y0, e0) = presamples(e, o)?;
    let x = optional_matrix(e, o, "X")?;
    if let Some(x) = &x {
        if x.cols() != m.beta.len() {
            return Err(err(
                "arima",
                "PredictorSizeMismatch",
                "X must have one column for each Beta coefficient.",
            ));
        }
        let min = if y0.is_none() {
            y.rows().saturating_sub(m.p())
        } else {
            y.rows()
        };
        if x.rows() < min.max(1) {
            return Err(err(
                "arima",
                "InsufficientPredictors",
                "X has too few observations for the response and presample.",
            ));
        }
    }
    let mut data = clean_aligned(e, vec![Some(y), x])?;
    let mut y = data.remove(0).unwrap();
    let mut x = data.remove(0);
    // With regression, responses cannot be backcast from unavailable predictors.
    // The earliest P responses become presample; retain the latest aligned X.
    if x.is_some() && y0.is_none() && m.p() > 0 {
        if y.rows() <= m.p() {
            return Err(err(
                "arima",
                "InsufficientData",
                "Not enough responses after removing the presample.",
            ));
        }
        let p = m.p();
        let n = y.rows();
        let cols = y.cols();
        let mut pre = Vec::with_capacity(p * cols);
        let mut rem = Vec::with_capacity((n - p) * cols);
        for j in 0..cols {
            pre.extend_from_slice(&y.data()[j * n..j * n + p]);
            rem.extend_from_slice(&y.data()[j * n + p..(j + 1) * n]);
        }
        y0 = Some(Matrix::from_column_vec(p, cols, pre)?);
        y = Matrix::from_column_vec(n - p, cols, rem)?;
    }
    if let Some(xv) = &x {
        x = Some(tail_matrix(xv, y.rows())?);
    }
    if y.rows() == 0 {
        return Err(err(
            "arima",
            "InsufficientData",
            "No observations remain after missing-value deletion.",
        ));
    }
    validate_pre(&y0, m.p(), y.cols(), "Y0")?;
    validate_pre(&e0, m.q(), y.cols(), "E0")?;
    Ok(Sample { y, x, y0, e0 })
}
pub(super) fn tail_matrix(m: &Matrix, n: usize) -> Result<Matrix> {
    if m.rows() < n {
        return Err(err("arima", "InsufficientData", "Not enough rows."));
    }
    if m.rows() == n {
        return Ok(m.clone());
    }
    let mut v = Vec::with_capacity(n * m.cols());
    for j in 0..m.cols() {
        v.extend_from_slice(&m.data()[(j + 1) * m.rows() - n..(j + 1) * m.rows()]);
    }
    Matrix::from_column_vec(n, m.cols(), v)
}
pub(super) fn validate_pre(m: &Option<Matrix>, lag: usize, paths: usize, name: &str) -> Result<()> {
    if let Some(m) = m
        && (m.rows() < lag || (m.cols() != 1 && m.cols() < paths))
    {
        return Err(err(
            "arima",
            "InvalidPresample",
            &format!("{name} has too few rows or paths; at least {lag} rows are required."),
        ));
    }
    Ok(())
}
pub(super) fn presample_column(
    m: &Option<Matrix>,
    path: usize,
    n: usize,
    default: f64,
) -> Vec<f64> {
    match m {
        Some(m) => {
            let j = if m.cols() == 1 { 0 } else { path };
            let end = (j + 1) * m.rows();
            m.data()[end - n..end].to_vec()
        }
        None => vec![default; n],
    }
}
pub(super) fn sparse(a: &[f64]) -> Vec<(usize, f64)> {
    a.iter()
        .enumerate()
        .skip(1)
        .filter(|(_, x)| **x != 0.)
        .map(|(i, &x)| (i, x))
        .collect()
}
// Backward forecasts on a reversed series, with zero innovations before its
// far end. Odd total differencing changes the sign of the backward drift.
// Explicit Y0 always bypasses this initialization. This is a conditional
// likelihood, not a stationary-state/Kalman exact likelihood.
pub(super) fn backcast(e: &Engine, m: &Model, y: &[f64]) -> Result<Vec<f64>> {
    let (a, b) = m.polynomials();
    let p = m.p();
    let q = m.q();
    if y.len() < p {
        return Err(err(
            "arima",
            "InsufficientData",
            "Too few responses to backcast the required presample.",
        ));
    }
    if p == 0 {
        return Ok(vec![]);
    }
    let ar = sparse(&a);
    let ma = sparse(&b);
    let parity = (m.d + usize::from(m.s > 0)) % 2;
    let c = if parity == 0 { m.c } else { -m.c };
    let initial = if m.d == 0 && m.s == 0 {
        m.c / a.iter().sum::<f64>()
    } else {
        *y.last().unwrap()
    };
    let mut r = vec![initial; p];
    r.extend(y.iter().rev());
    let mut z = vec![0.; q];
    for t in 0..y.len() {
        if t % 4096 == 0 {
            e.check_cancel()?;
        }
        let mut mean = c;
        for &(lag, v) in &ar {
            mean -= v * r[p + t - lag];
        }
        for &(lag, v) in &ma {
            mean += v * z[q + t - lag];
        }
        z.push(r[p + t] - mean);
    }
    let mut pre = Vec::with_capacity(p);
    for t in 0..p {
        let i = p + y.len() + t;
        let j = q + y.len() + t;
        let mut v = c;
        for &(lag, x) in &ar {
            v -= x * r[i - lag];
        }
        for &(lag, x) in &ma {
            v += x * z[j - lag];
        }
        r.push(v);
        z.push(0.);
        pre.push(v);
    }
    pre.reverse();
    Ok(pre)
}
pub(super) fn contributions(m: &Model, residuals: &[f64]) -> Vec<f64> {
    let v = m.variance;
    if let Some(nu) = m.dof {
        let c = libm::lgamma((nu + 1.) / 2.)
            - libm::lgamma(nu / 2.)
            - 0.5 * (std::f64::consts::PI * (nu - 2.) * v).ln();
        residuals
            .iter()
            .map(|x| c - 0.5 * (nu + 1.) * (x * x / (v * (nu - 2.))).ln_1p())
            .collect()
    } else {
        let c = -0.5 * (std::f64::consts::TAU * v).ln();
        residuals.iter().map(|x| c - 0.5 * x * x / v).collect()
    }
}
pub(super) fn residuals(e: &Engine, m: &Model, s: &Sample, path: usize) -> Result<Vec<f64>> {
    let (a, b) = m.polynomials();
    let ar = sparse(&a);
    let ma = sparse(&b);
    let p = m.p();
    let q = m.q();
    let n = s.y.rows();
    let y = &s.y.data()[path * n..(path + 1) * n];
    let mut hist = if s.y0.is_some() {
        presample_column(&s.y0, path, p, 0.)
    } else {
        backcast(e, m, y)?
    };
    hist.extend_from_slice(y);
    let mut z = presample_column(&s.e0, path, q, 0.);
    for t in 0..n {
        if t % 4096 == 0 {
            e.check_cancel()?;
        }
        let mut v = hist[p + t] - m.c;
        for &(lag, x) in &ar {
            v += x * hist[p + t - lag];
        }
        for &(lag, x) in &ma {
            v -= x * z[q + t - lag];
        }
        if let Some(x) = &s.x {
            for (j, beta) in m.beta.iter().enumerate() {
                v -= beta * x.data()[j * n + t];
            }
        }
        if !v.is_finite() {
            return Err(err(
                "arima",
                "NumericalFailure",
                "Residual recursion overflowed.",
            ));
        }
        z.push(v);
    }
    Ok(z.split_off(q))
}
pub(super) fn infer(e: &Engine, m: &Model, a: &[Value]) -> Result<Vec<Value>> {
    if m.conditional.is_some() {
        return composite::infer(e, m, a);
    }
    if a.is_empty() {
        return Err(err(
            "infer",
            "InvalidNumInputs",
            "Not enough input arguments.",
        ));
    }
    let o = options(e, "infer", &a[1..], &["Y0", "E0", "V0", "X"])?;
    let mut model = m.clone();
    if !o.contains_key("X") {
        model.beta.clear();
    }
    model.fully_specified()?;
    let s = sample(e, &model, &a[0], &o)?;
    let n = s.y.rows();
    let paths = s.y.cols();
    preflight(e, n, paths, 3)?;
    let mut z = Vec::with_capacity(n * paths);
    let mut logl = vec![];
    for j in 0..paths {
        let r = residuals(e, &model, &s, j)?;
        logl.push(contributions(&model, &r).iter().sum());
        z.extend(r);
    }
    Ok(vec![
        value_matrix(n, paths, z)?,
        value_matrix(n, paths, vec![model.variance; n * paths])?,
        value_matrix(1, paths, logl)?,
    ])
}
