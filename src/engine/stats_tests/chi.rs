//! Pearson goodness-of-fit statistic, with tail-only expected-count pooling.
use super::*;
pub(super) fn test(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let (end, o) = Opts::parse(
        args,
        1,
        &[
            "NBins",
            "Ctrs",
            "Edges",
            "CDF",
            "Expected",
            "Frequency",
            "NParams",
            "EMin",
            "Alpha",
        ],
    )?;
    if end != 1 {
        return Err("chi2gof: invalid name-value options".into());
    }
    let a = alpha(o.num("alpha", 0.05, e)?)?;
    let emin = o.num("emin", 5., e)?;
    if !emin.is_finite() || emin < 0. {
        return Err("chi2gof: EMin must be finite and nonnegative".into());
    }
    let raw = mat(&args[0], e)?;
    if raw.rows() > 1 && raw.cols() > 1 {
        return Err("chi2gof: input must be a vector".into());
    }
    let freq = if let Some(v) = o.get("frequency") {
        let f = mat(v, e)?;
        if f.len() != raw.len() {
            return Err("chi2gof: Frequency must match x".into());
        }
        f.data().to_vec()
    } else {
        vec![1.; raw.len()]
    };
    if freq.iter().any(|v| !v.is_finite() || *v < 0.) {
        return Err("chi2gof: Frequency must be finite and nonnegative".into());
    }
    let mut x = Vec::new();
    let mut ws = Vec::new();
    for (i, (&v, &w)) in raw.data().iter().zip(&freq).enumerate() {
        poll(e, i)?;
        if !v.is_nan() && w > 0. {
            if !v.is_finite() {
                return Err("chi2gof: observations must be finite".into());
            }
            x.push(v);
            ws.push(w)
        }
    }
    if x.is_empty() {
        return Err("chi2gof: no observations".into());
    }
    let n = ws.iter().sum::<f64>();
    if o.get("ctrs").is_some() && o.get("edges").is_some() {
        return Err("chi2gof: Ctrs and Edges cannot be combined".into());
    }
    // Edges, centers and expected counts are a specification, not data:
    // a NaN is an error, not a value to drop the way vector() drops missing
    // observations.
    let expected = o
        .get("expected")
        .map(|v| specification(v, e, "Expected"))
        .transpose()?;
    let mut edges = if let Some(v) = o.get("edges") {
        specification(v, e, "Edges")?
    } else if let Some(v) = o.get("ctrs") {
        let c = specification(v, e, "Ctrs")?;
        if c.len() < 2 {
            return Err("chi2gof: at least two centers required".into());
        }
        let mut edges = vec![c[0] - (c[1] - c[0]) / 2.];
        edges.extend(c.windows(2).map(|w| (w[0] + w[1]) / 2.));
        edges.push(c[c.len() - 1] + (c[c.len() - 1] - c[c.len() - 2]) / 2.);
        edges
    } else {
        let nb = if let Some(v) = o.get("nbins") {
            integer(v, e)?
        } else {
            expected.as_ref().map_or(10, Vec::len)
        };
        if nb == 0 || nb > crate::matrix::MAX_ELEMENTS {
            return Err("chi2gof: NBins must be positive and bounded".into());
        }
        let lo = x.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let (lo, hi) = if lo == hi {
            (lo - 0.5, hi + 0.5)
        } else {
            (lo, hi)
        };
        (0..=nb)
            .map(|i| lo + (hi - lo) * i as f64 / nb as f64)
            .collect()
    };
    if edges.len() < 2
        || edges
            .windows(2)
            .any(|w| w[0].is_nan() || w[1].is_nan() || w[0] >= w[1])
    {
        return Err("chi2gof: bin edges must increase".into());
    }
    let bins = edges.len() - 1;
    let mut observed = vec![0.; bins];
    for (i, (&v, &w)) in x.iter().zip(&ws).enumerate() {
        poll(e, i)?;
        // The outer bins reach -Inf/Inf, as in the expected counts below.
        let b = edges
            .partition_point(|x| *x <= v)
            .saturating_sub(1)
            .min(bins - 1);
        observed[b] += w;
    }
    let mut default_np = 2.;
    let mut exp = if let Some(expected) = expected {
        if expected.len() != bins || expected.iter().any(|v| !v.is_finite() || *v < 0.) {
            return Err("chi2gof: Expected must match bins and be nonnegative".into());
        }
        default_np = 0.;
        expected
    } else {
        let mut bounds = edges.clone();
        bounds[0] = f64::NEG_INFINITY;
        bounds[bins] = f64::INFINITY;
        let probs = if let Some(v) = o.get("cdf") {
            let (h, params) = match v {
                Value::Container(Array::Handle(h)) => (h.clone(), Vec::new()),
                Value::Container(Array::Cell(c)) if !c.is_empty() => {
                    let Array::Handle(h) = c.get_linear(0)?.as_ref() else {
                        return Err("chi2gof: CDF cell must start with a handle".into());
                    };
                    let mut params = Vec::new();
                    for i in 1..c.len() {
                        params.push(match c.get_linear(i)?.as_ref() {
                            Array::Numeric(m) => Value::Matrix(m.clone()),
                            Array::Logical(m) => Value::Logical(m.clone()),
                            a => Value::Container(a.clone()),
                        })
                    }
                    (h.clone(), params)
                }
                _ => return Err("chi2gof: distribution objects are not supported yet".into()),
            };
            default_np = params
                .iter()
                .map(|v| v.dims().iter().product::<usize>())
                .sum::<usize>() as f64;
            let mut call = vec![row(bounds)?];
            call.extend(params);
            let v = e
                .call_handle(&h, call, 1, None, out)?
                .into_iter()
                .next()
                .ok_or("chi2gof: the function returned no value")?;
            let m = mat(&v, e)?;
            if m.len() != bins + 1 {
                return Err("chi2gof: CDF handle must preserve input size".into());
            }
            m.data().to_vec()
        } else {
            let mean = x.iter().zip(&ws).map(|(x, w)| x * w).sum::<f64>() / n;
            let sd = (x
                .iter()
                .zip(&ws)
                .map(|(x, w)| w * (x - mean).powi(2))
                .sum::<f64>()
                / (n - 1.))
                .sqrt();
            bounds
                .iter()
                .map(|v| Family::Norm.cdf(*v, &[mean, sd], false))
                .collect()
        };
        if probs
            .iter()
            .any(|p| !p.is_finite() || !(0. ..=1.).contains(p))
            || probs.windows(2).any(|w| w[0] > w[1])
        {
            return Err("chi2gof: invalid CDF probabilities".into());
        }
        probs
            .windows(2)
            .map(|w| n * (w[1] - w[0]))
            .collect::<Vec<_>>()
    };
    let np = o.num("nparams", default_np, e)?;
    if !np.is_finite() || np < 0. || np.fract() != 0. {
        return Err("chi2gof: NParams must be a nonnegative integer".into());
    }
    // Only outer bins are pooled, moving inward. Keep a retained range so
    // pooling many tiny tail bins is linear rather than shifting vectors.
    let mut left = 0;
    let mut right = exp.len();
    while right - left > 1 && exp[left] < emin {
        e.check_cancel()?;
        observed[left + 1] += observed[left];
        exp[left + 1] += exp[left];
        left += 1;
    }
    while right - left > 1 && exp[right - 1] < emin {
        e.check_cancel()?;
        observed[right - 2] += observed[right - 1];
        exp[right - 2] += exp[right - 1];
        right -= 1;
    }
    if left != 0 || right != exp.len() {
        observed = observed[left..right].to_vec();
        exp = exp[left..right].to_vec();
        let mut pooled = Vec::with_capacity(exp.len() + 1);
        pooled.push(edges[0]);
        for (i, &edge) in edges.iter().enumerate().take(right).skip(left + 1) {
            poll(e, i)?;
            pooled.push(edge);
        }
        pooled.push(*edges.last().unwrap());
        edges = pooled;
    }
    let statistic = observed
        .iter()
        .zip(&exp)
        .map(|(o, e)| {
            if *e == 0. {
                if *o == 0. { 0. } else { f64::INFINITY }
            } else {
                (o - e).powi(2) / e
            }
        })
        .sum::<f64>();
    let df = exp.len() as f64 - 1. - np;
    let p = if df <= 0. {
        f64::NAN
    } else {
        Family::Chi2.cdf(statistic, &[df], true)
    };
    Ok(vec![
        val(decision(p, a)),
        val(p),
        structure(
            e,
            vec![
                ("chi2stat", val(statistic)),
                ("df", val(df)),
                ("edges", row(edges)?),
                ("O", row(observed)?),
                ("E", row(exp)?),
            ],
        )?,
    ])
}

/// Bin specification: keep every entry. Edges may be ±Inf (outer bins); only
/// NaN is rejected. Ctrs and Expected stay finite.
fn specification(v: &Value, e: &Engine, what: &str) -> Result<Vec<f64>> {
    let m = mat(v, e)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err("Input must be a vector".into());
    }
    let rejected = |x: f64| {
        if what == "Edges" {
            x.is_nan()
        } else {
            !x.is_finite()
        }
    };
    if m.data().iter().copied().any(rejected) {
        return Err(format!("chi2gof: {what} must be finite"));
    }
    Ok(m.data().to_vec())
}
