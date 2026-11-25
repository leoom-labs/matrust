use super::*;
use likelihood::{
    clean_aligned, optional_matrix, presample_column, presamples, sparse, tail_matrix, validate_pre,
};
pub(super) fn call(e: &mut Engine, n: &str, m: &Model, a: &[Value]) -> Result<Vec<Value>> {
    if m.conditional.is_some() {
        return composite::paths(e, n, m, a);
    }
    if a.is_empty() {
        return Err(err(n, "InvalidNumInputs", "Not enough input arguments."));
    }
    let periods = integer(e, &a[0], 1, MAX_ELEMENTS)?;
    let positional = n == "forecast" && a.get(1).is_some_and(|v| text(v).is_err());
    let mut o = options(
        e,
        n,
        &a[if positional { 2 } else { 1 }..],
        if n == "forecast" {
            &["Y0", "E0", "V0", "X0", "XF"]
        } else {
            &["Y0", "E0", "V0", "X", "NumPaths"]
        },
    )?;
    if positional {
        if o.contains_key("Y0") {
            return Err(err(n, "DuplicateY0", "Specify Y0 only once."));
        }
        o.insert("Y0".into(), a[1].clone());
    }
    let mut model = m.clone();
    let xname = if n == "forecast" { "XF" } else { "X" };
    if !o.contains_key(xname) {
        if o.contains_key("X0") {
            return Err(err(n, "MissingXF", "X0 requires XF."));
        }
        model.beta.clear();
    }
    model.fully_specified()?;
    let m = &model;
    let (y0, mut e0, x0) = if n == "forecast" && optional_matrix(e, &o, "E0")?.is_none() {
        // Synchronize presample deletion across Y0, V0, and X0 when inferring E0.
        let mut p = clean_aligned(
            e,
            vec![
                optional_matrix(e, &o, "Y0")?,
                None,
                optional_matrix(e, &o, "V0")?,
                optional_matrix(e, &o, "X0")?,
            ],
        )?;
        let y0 = p.remove(0);
        let x0 = p.pop().unwrap();
        (y0, None, x0)
    } else {
        let (y0, e0) = presamples(e, &o)?;
        (y0, e0, None)
    };
    if n == "forecast" && y0.is_none() && m.p() > 0 {
        return Err(err(n, "MissingY0", "Presample responses Y0 are required."));
    }
    let paths = if n == "simulate" {
        o.get("NumPaths")
            .map(|v| integer(e, v, 1, MAX_ELEMENTS))
            .transpose()?
            .unwrap_or(1)
    } else {
        y0.iter()
            .chain(e0.iter())
            .map(Matrix::cols)
            .max()
            .unwrap_or(1)
    };
    let p = m.p();
    let q = m.q();
    validate_pre(&y0, p, paths, "Y0")?;
    validate_pre(&e0, q, paths, "E0")?;
    preflight(e, periods, paths, 4)?;
    let x = optional_matrix(e, &o, xname)?;
    let x = clean_aligned(e, vec![x])?.remove(0);
    let x = if let Some(x) = x {
        if x.cols() != m.beta.len() || x.rows() < periods {
            return Err(err(
                n,
                "PredictorSizeMismatch",
                "Predictors have too few rows or the wrong number of columns.",
            ));
        }
        Some(if n == "forecast" {
            // XF begins immediately after the forecast origin; excess rows ignored.
            let mut v = Vec::with_capacity(periods * x.cols());
            for j in 0..x.cols() {
                v.extend_from_slice(&x.data()[j * x.rows()..j * x.rows() + periods]);
            }
            Matrix::from_column_vec(periods, x.cols(), v)?
        } else {
            tail_matrix(&x, periods)?
        })
    } else {
        None
    };
    if n == "forecast"
        && e0.is_none()
        && q > 0
        && let Some(y) = &y0
        && y.rows() >= p + q
        && (x0.is_some() || m.beta.is_empty())
    {
        // ARIMAX without presample predictors: E0 = 0 (forecast documentation).
        let opts = match x0 {
            Some(x0) => BTreeMap::from([("X".into(), Value::Matrix(x0))]),
            None => BTreeMap::new(),
        };
        // Infer from the available history, using its first P observations as
        // explicit presample. This avoids a second, origin-dependent backcast.
        let mut opts = opts;
        let pre = Matrix::from_column_vec(
            p,
            y.cols(),
            (0..y.cols())
                .flat_map(|j| y.data()[j * y.rows()..j * y.rows() + p].iter().copied())
                .collect(),
        )?;
        opts.insert("Y0".into(), Value::Matrix(pre));
        let rest = tail_matrix(y, y.rows() - p)?;
        let s = likelihood::sample(e, m, &Value::Matrix(rest), &opts)?;
        let mut z = Vec::with_capacity(s.y.rows() * paths);
        for j in 0..paths {
            z.extend(likelihood::residuals(e, m, &s, j)?);
        }
        e0 = Some(Matrix::from_column_vec(s.y.rows(), paths, z)?);
        // Listwise NaN deletion can leave fewer than Q inferred residuals.
        likelihood::validate_pre(&e0, q, paths, "E0 inferred from Y0")?;
    }
    let (a, b) = m.polynomials();
    let ar = sparse(&a);
    let ma = sparse(&b);
    let initial = if m.d == 0 && m.s == 0 && m.beta.is_empty() {
        m.c / a.iter().sum::<f64>()
    } else {
        0.
    };
    let mut ys = Vec::with_capacity(periods * paths);
    let mut es = Vec::with_capacity(periods * paths);
    for j in 0..paths {
        let mut y = presample_column(&y0, j, p, initial);
        let mut z = presample_column(&e0, j, q, 0.);
        for t in 0..periods {
            if t % 4096 == 0 {
                e.check_cancel()?;
            }
            let innovation = if n == "simulate" {
                let normal = e.random.next_normal();
                let draw = if let Some(nu) = m.dof {
                    normal * ((nu - 2.) / (2. * gamma(e, nu / 2.)?)).sqrt()
                } else {
                    normal
                };
                draw * m.variance.sqrt()
            } else {
                0.
            };
            let mut v = m.c + innovation;
            for &(lag, x) in &ar {
                v -= x * y[p + t - lag];
            }
            for &(lag, x) in &ma {
                v += x * z[q + t - lag];
            }
            if let Some(x) = &x {
                for (k, beta) in m.beta.iter().enumerate() {
                    v += beta * x.data()[k * periods + t];
                }
            }
            if !v.is_finite() {
                return Err(err(n, "NumericalFailure", "Response recursion overflowed."));
            }
            y.push(v);
            z.push(innovation);
        }
        ys.extend_from_slice(&y[p..]);
        es.extend_from_slice(&z[q..]);
    }
    let variance = vec![m.variance; periods * paths];
    if n == "simulate" {
        Ok(vec![
            value_matrix(periods, paths, ys)?,
            value_matrix(periods, paths, es)?,
            value_matrix(periods, paths, variance)?,
        ])
    } else {
        let mut psi = vec![0.; periods];
        let mut mse = vec![0.; periods];
        let mut sum = 0.;
        for t in 0..periods {
            if t % 4096 == 0 {
                e.check_cancel()?;
            }
            let mut v = b.get(t).copied().unwrap_or(0.);
            for &(lag, x) in &ar {
                if lag <= t {
                    v -= x * psi[t - lag];
                }
            }
            psi[t] = v;
            sum += v * v * m.variance;
            if !sum.is_finite() {
                return Err(err(
                    "forecast",
                    "NumericalFailure",
                    "Forecast error variance recursion overflowed.",
                ));
            }
            mse[t] = sum;
        }
        Ok(vec![
            value_matrix(periods, paths, ys)?,
            value_matrix(
                periods,
                paths,
                (0..paths).flat_map(|_| mse.iter().copied()).collect(),
            )?,
            value_matrix(periods, paths, variance)?,
        ])
    }
}
// Marsaglia & Tsang (2000) gamma rejection sampler, on the session MT19937
// stream. t = N/sqrt(chi2/nu), standardized by sqrt((nu-2)/nu).
pub(super) fn gamma(e: &mut Engine, shape: f64) -> Result<f64> {
    let d = shape - 1. / 3.;
    let c = (9. * d).sqrt().recip();
    for attempt in 0..1_000_000 {
        if attempt % 1024 == 0 {
            e.check_cancel()?;
        }
        let x = e.random.next_normal();
        let t = 1. + c * x;
        if t <= 0. {
            continue;
        }
        let v = t * t * t;
        let u = e.random.next_f64();
        if u < 1. - 0.0331 * x.powi(4) || u.ln() < 0.5 * x * x + d * (1. - v + v.ln()) {
            return Ok(d * v);
        }
    }
    Err(err(
        "simulate",
        "RandomFailure",
        "Gamma rejection sampler exceeded its iteration limit.",
    ))
}
