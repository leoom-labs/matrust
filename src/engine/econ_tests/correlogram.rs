use super::*;
use nalgebra::{DMatrix, DVector};

/// Missing observations retain their time positions. With missing values each
/// lag uses its available cross products; complete data use the biased 1/T ACF.
pub(super) fn acf(e: &mut Engine, n: &str, y: &[f64], lags: usize) -> Result<Vec<f64>> {
    work(e, n, y.len().saturating_mul(lags + 1))?;
    let count = y.iter().filter(|v| !v.is_nan()).count();
    let mean = y.iter().filter(|v| !v.is_nan()).sum::<f64>() / count as f64;
    let ss = y
        .iter()
        .filter(|v| !v.is_nan())
        .map(|v| (v - mean).powi(2))
        .sum::<f64>();
    if count < 2 || !ss.is_finite() || ss <= 0. {
        return Err(err(
            n,
            "DegenerateData",
            "At least two nonmissing observations with positive finite variance are required.",
        ));
    }
    let missing = count != y.len();
    let mut out = vec![1.];
    for lag in 1..=lags {
        e.check_cancel()?;
        let mut sum = 0.;
        let mut pairs = 0;
        for (&a, &b) in y[lag..].iter().zip(y) {
            if !a.is_nan() && !b.is_nan() {
                sum += (a - mean) * (b - mean);
                pairs += 1;
            }
        }
        out.push(if missing {
            if pairs == 0 {
                f64::NAN
            } else {
                sum / ss * count as f64 / pairs as f64
            }
        } else {
            sum / ss
        });
    }
    Ok(out)
}
fn pacf(e: &mut Engine, y: &[f64], lags: usize, method: &str) -> Result<Vec<f64>> {
    let n = "parcorr";
    if method.eq_ignore_ascii_case("ols") {
        if y.iter().any(|v| v.is_nan()) {
            return Err(err(
                n,
                "MissingData",
                "Method 'ols' does not support NaN observations.",
            ));
        }
        let mean = y.iter().sum::<f64>() / y.len() as f64;
        let ss = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
        if y.len() < 2 || !ss.is_finite() || ss <= 0. {
            return Err(err(
                n,
                "DegenerateData",
                "At least two nonmissing observations with positive finite variance are required.",
            ));
        }
        // Bound the sum of n*p^2 QR costs before allocating any designs.
        work(
            e,
            n,
            y.len()
                .saturating_mul(lags.saturating_mul(lags + 1).saturating_mul(2 * lags + 1) / 6),
        )?;
        let mut p = vec![1.];
        for lag in 1..=lags {
            let rows = y.len() - lag;
            if rows.saturating_mul(lag + 1) > MAX_DESIGN {
                return Err(err(
                    n,
                    "WorkspaceLimit",
                    "PACF regression design exceeds 2000000 elements.",
                ));
            }
            design(e, n, rows, lag + 1)?;
            let x = DMatrix::from_fn(
                rows,
                lag + 1,
                |r, c| if c == 0 { 1. } else { y[r + lag - c] },
            );
            let ls = super::super::stats_models::regression::least_squares(
                e,
                &x,
                &DVector::from_column_slice(&y[lag..]),
            )?;
            p.push(ls.b[lag]);
        }
        return Ok(p);
    }
    if !method.eq_ignore_ascii_case("yule-walker") {
        return Err(err(
            n,
            "InvalidMethod",
            "Method must be 'ols' or 'yule-walker'.",
        ));
    }
    let r = acf(e, n, y, lags)?;
    let mut coeff = vec![0.; lags];
    let mut p = vec![1.];
    let mut variance = 1.;
    for k in 1..=lags {
        e.check_cancel()?;
        let reflection = (r[k] - (1..k).map(|j| coeff[j - 1] * r[k - j]).sum::<f64>()) / variance;
        let previous = coeff[..k - 1].to_vec();
        for j in 1..k {
            coeff[j - 1] = previous[j - 1] - reflection * previous[k - j - 1];
        }
        coeff[k - 1] = reflection;
        p.push(reflection);
        variance *= 1. - reflection * reflection;
    }
    Ok(p)
}
fn xcf(e: &mut Engine, x: &[f64], y: &[f64], lags: usize) -> Result<Vec<f64>> {
    let n = "crosscorr";
    let len = x.len().max(y.len());
    work(e, n, len.saturating_mul(2 * lags + 1))?;
    // Zero-pad the shorter series to the longer length before computing means and covariances.
    let pad = |v: &[f64]| {
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(v);
        out.resize(len, 0.);
        out
    };
    let x_pad;
    let y_pad;
    let x = if x.len() < len {
        x_pad = pad(x);
        &x_pad[..]
    } else {
        x
    };
    let y = if y.len() < len {
        y_pad = pad(y);
        &y_pad[..]
    } else {
        y
    };
    let center = |v: &[f64]| {
        let k = v.iter().filter(|v| !v.is_nan()).count();
        let mean = v.iter().filter(|v| !v.is_nan()).sum::<f64>() / k as f64;
        v.iter().map(|v| v - mean).collect::<Vec<_>>()
    };
    let nx = x.iter().filter(|v| !v.is_nan()).count();
    let ny = y.iter().filter(|v| !v.is_nan()).count();
    let missing = nx != len || ny != len;
    let x = center(x);
    let y = center(y);
    let ss = |v: &[f64]| v.iter().filter(|v| !v.is_nan()).map(|v| v * v).sum::<f64>();
    let denom = (ss(&x) * ss(&y)).sqrt();
    if !denom.is_finite() || denom <= 0. {
        return Err(err(
            n,
            "DegenerateData",
            "Both series must have positive finite variance.",
        ));
    }
    let mut result = Vec::with_capacity(2 * lags + 1);
    for lag in -(lags as isize)..=lags as isize {
        e.check_cancel()?;
        let mut sum = 0.;
        let mut pairs = 0;
        for (i, &a) in x.iter().enumerate() {
            let j = i as isize + lag;
            if j >= 0
                && let Some(&b) = y.get(j as usize)
                && !a.is_nan()
                && !b.is_nan()
            {
                sum += a * b;
                pairs += 1;
            }
        }
        result.push(if missing {
            if pairs == 0 {
                f64::NAN
            } else {
                sum / denom * (nx as f64 * ny as f64).sqrt() / pairs as f64
            }
        } else {
            sum / denom
        });
    }
    Ok(result)
}
pub(super) fn call(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let required = if n == "crosscorr" { 2 } else { 1 };
    if a.len() < required {
        return Err(err(n, "NotEnoughInputs", "Not enough input arguments."));
    }
    let y = vector(e, n, &a[0])?;
    let z = if n == "crosscorr" {
        Some(vector(e, n, &a[1])?)
    } else {
        None
    };
    let observed = y.data().iter().filter(|v| !v.is_nan()).count();
    let effective = observed.min(z.as_ref().map_or(observed, |z| {
        z.data().iter().filter(|v| !v.is_nan()).count()
    }));
    if effective < 2 {
        return Err(err(
            n,
            "TooFewObservations",
            "At least two nonmissing observations are required.",
        ));
    }
    let mut nl = 20.min(effective - 1);
    let mut order = 0;
    let mut std = 2.;
    let rest = &a[required..];
    let names = if n == "crosscorr" {
        vec!["numlags", "numstd"]
    } else if n == "autocorr" {
        vec!["numlags", "numma", "numstd"]
    } else {
        vec!["numlags", "numar", "numstd", "method"]
    };
    let mut positional = 0;
    while positional < rest.len() && text(&rest[positional]).is_none() {
        positional += 1;
    }
    let positional_max = if n == "crosscorr" { 2 } else { 3 };
    if positional > positional_max {
        return Err(err(n, "InvalidNumInputs", "Too many positional inputs."));
    }
    let mut opts = Options::new(n, &rest[positional..], &names)?;
    for (i, v) in rest[..positional].iter().enumerate() {
        let m = real(e, n, v)?;
        if m.is_empty() {
            continue;
        }
        if m.len() != 1 {
            return Err(err(
                n,
                "InvalidOptions",
                "Positional options must be scalars.",
            ));
        }
        let x = m.data()[0];
        match i {
            0 => nl = integer(n, x, 1, (effective - 1).min(MAX_LAGS))?,
            1 if n != "crosscorr" => order = integer(n, x, 0, MAX_LAGS)?,
            _ => std = x,
        }
    }
    if opts.values.contains_key("numlags") {
        nl = integer(
            n,
            opts.nums(e, n, "numlags", nl as f64)?[0],
            1,
            (effective - 1).min(MAX_LAGS),
        )?;
    }
    let key = if n == "autocorr" { "numma" } else { "numar" };
    if opts.values.contains_key(key) {
        order = integer(n, opts.nums(e, n, key, 0.)?[0], 0, MAX_LAGS)?;
    }
    if opts.values.contains_key("numstd") {
        std = opts.nums(e, n, "numstd", std)?[0];
    }
    if opts.count != 1 || !std.is_finite() || std < 0. || order >= nl {
        return Err(err(
            n,
            "InvalidOptions",
            "Options must be scalars, NumSTD nonnegative, and NumMA/NumAR less than NumLags.",
        ));
    }
    let method = opts.strings(
        e,
        n,
        "method",
        if observed == y.len() {
            "ols"
        } else {
            "yule-walker"
        },
    )?;
    if method.len() != 1 {
        return Err(err(n, "InvalidMethod", "Method must be scalar text."));
    }
    reserve(
        e,
        n,
        y.len()
            .saturating_mul(4)
            .saturating_add(z.as_ref().map_or(0, |z| z.len()).saturating_mul(4)),
    )?;
    let values = match n {
        "autocorr" => acf(e, n, y.data(), nl)?,
        "parcorr" => pacf(e, y.data(), nl, &method[0])?,
        _ => xcf(e, y.data(), z.as_ref().unwrap().data(), nl)?,
    };
    let lags = if n == "crosscorr" {
        (-(nl as i64)..=nl as i64)
            .map(|i| i as f64)
            .collect::<Vec<_>>()
    } else {
        (0..=nl).map(|i| i as f64).collect()
    };
    let bartlett = if n == "autocorr" {
        1. + 2. * values[1..=order].iter().map(|v| v * v).sum::<f64>()
    } else {
        1.
    };
    let bound = std * (bartlett / effective as f64).sqrt();
    let row = y.rows() == 1;
    let mut result = vec![
        vecval(row, values.clone())?,
        vecval(row, lags.clone())?,
        value(2, 1, vec![bound, -bound])?,
    ];
    if nout == 0 || nout == 4 {
        let stem = e.call(
            "stem",
            vec![vecval(row, lags.clone())?, vecval(row, values)?],
            out,
        )?;
        let ax = e.call("gca", vec![], out)?;
        let previous = e.call("get", vec![ax.clone(), Value::Text("NextPlot".into())], out)?;
        e.call("hold", vec![Value::Text("on".into())], out)?;
        let mut handles = vec![];
        if let Array::Graphics(g) = stem.array(&e.cancel)? {
            for i in 0..g.len() {
                handles.push(g.get_linear(i)?.clone());
            }
        }
        let plot_result = (|| -> Result<()> {
            if std > 0. {
                let start = if n == "crosscorr" {
                    lags[0]
                } else {
                    (order + 1) as f64
                };
                for b in [bound, -bound] {
                    let h = e.call(
                        "plot",
                        vec![
                            value(1, 2, vec![start, *lags.last().unwrap()])?,
                            value(1, 2, vec![b, b])?,
                            Value::Text("r--".into()),
                        ],
                        out,
                    )?;
                    if let Array::Graphics(g) = h.array(&e.cancel)? {
                        for i in 0..g.len() {
                            handles.push(g.get_linear(i)?.clone());
                        }
                    }
                }
            }
            e.call("xlabel", vec![Value::Text("Lag".into())], out)?;
            e.call("ylabel", vec![Value::Text(n.into())], out)?;
            Ok(())
        })();
        e.call(
            "set",
            vec![ax, Value::Text("NextPlot".into()), previous],
            out,
        )?;
        plot_result?;
        result.push(
            Array::Graphics(crate::GraphicsArray::from_shared(
                handles.len(),
                1,
                handles,
                &e.cancel,
            )?)
            .into(),
        );
    }
    Ok(result)
}
