use super::super::stats_dist::families::Family;
use super::regression::Fit;
use super::*;
use nalgebra::DMatrix;
use std::sync::Arc;

fn model(s: &str, n: &str) -> Result<usize> {
    match s.to_ascii_uppercase().as_str() {
        "AR" => Ok(0),
        "ARD" => Ok(1),
        "TS" => Ok(2),
        _ => Err(err(
            n,
            "InvalidModel",
            "Model must be 'AR', 'ARD', or 'TS'.",
        )),
    }
}
fn struct_value(e: &Engine, fields: Vec<(&str, Value)>) -> Result<Value> {
    let (names, slots): (Vec<_>, Vec<_>) = fields
        .into_iter()
        .map(|(n, v)| Ok((n.into(), Arc::new(v.array(&e.cancel)?))))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .unzip();
    Ok(Array::Struct(crate::StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?)
    .into())
}
fn regression_record(
    e: &Engine,
    f: &Fit,
    num: usize,
    names: Vec<String>,
    hac: Option<(f64, Vec<f64>)>,
    nw_se: Option<Vec<f64>>,
) -> Result<Value> {
    let t = f.n as f64;
    let k = f.k as f64;
    let df = t - k;
    let r2 = 1. - f.sse / f.sst;
    let adjusted = 1. - (1. - r2) * (t - f64::from(f.intercept)) / df;
    let ll = -t / 2. * ((2. * std::f64::consts::PI).ln() + 1. + (f.sse / t).ln());
    let dw = f.res.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f64>() / f.sse;
    let tt =
        f.b.iter()
            .zip(&f.se)
            .map(|(b, s)| b / s)
            .collect::<Vec<_>>();
    let pp = tt
        .iter()
        .map(|v| 2. * Family::T.cdf(v.abs(), &[df], true))
        .collect::<Vec<_>>();
    let ft = if f.k > usize::from(f.intercept) {
        (f.sst - f.sse) / (k - f64::from(f.intercept)) / f.mse
    } else {
        f64::NAN
    };
    let fp = Family::F.cdf(ft, &[k - f64::from(f.intercept), df], true);
    let labels = crate::StringArray::new(
        names.len(),
        1,
        names.into_iter().map(|s| Some(Arc::from(s))).collect(),
    )?;
    let mut fields = vec![
        ("num", scalar(num as f64)),
        ("size", scalar(t)),
        ("names", Array::Str(labels).into()),
        ("coeff", vecval(false, f.b.clone())?),
        ("se", vecval(false, f.se.clone())?),
        ("Cov", value(f.k, f.k, f.cov.as_slice().to_vec())?),
        (
            "tStats",
            struct_value(
                e,
                vec![("t", vecval(false, tt)?), ("pVal", vecval(false, pp)?)],
            )?,
        ),
        (
            "FStat",
            struct_value(e, vec![("F", scalar(ft)), ("pVal", scalar(fp))])?,
        ),
        ("yMu", scalar(f.mean)),
        (
            "ySigma",
            scalar(
                ((0..f.n)
                    .map(|i| (f.fitted[i] + f.res[i] - f.mean).powi(2))
                    .sum::<f64>()
                    / (t - 1.))
                    .sqrt(),
            ),
        ),
        ("yHat", vecval(false, f.fitted.clone())?),
        ("res", vecval(false, f.res.clone())?),
        ("DWStat", scalar(dw)),
        ("SSR", scalar(f.sst - f.sse)),
        ("SSE", scalar(f.sse)),
        ("SST", scalar(f.sst)),
        ("MSE", scalar(f.mse)),
        ("RMSE", scalar(f.mse.sqrt())),
        ("RSq", scalar(r2)),
        ("aRSq", scalar(adjusted)),
        ("LL", scalar(ll)),
        ("AIC", scalar(-2. * ll + 2. * k)),
        ("BIC", scalar(-2. * ll + k * t.ln())),
        ("HQC", scalar(-2. * ll + 2. * k * t.ln().ln())),
    ];
    if let Some((var, ac)) = hac {
        fields.push((
            "NWEst",
            if let Some(se) = nw_se {
                vecval(false, se)?
            } else {
                scalar(var)
            },
        ));
        fields.push(("autoCov", vecval(false, ac)?));
    }
    struct_value(e, fields)
}
fn records(e: &Engine, row: bool, records: Vec<Value>) -> Result<Value> {
    let mut names = vec![];
    let mut slots = vec![];
    for v in &records {
        let Array::Struct(s) = v.clone().array(&e.cancel)? else {
            unreachable!()
        };
        if names.is_empty() {
            names = s.field_names().to_vec();
        }
        for name in &names {
            slots.push(s.get(s.field_index(name).unwrap(), 0)?.clone());
        }
    }
    let (r, c) = shape(row, records.len());
    Ok(Array::Struct(crate::StructArray::from_slots(
        r, c, names, slots, &e.cancel,
    )?)
    .into())
}
pub(super) fn call(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(err(n, "NotEnoughInputs", "Not enough input arguments."));
    }
    let input = vector(e, n, &a[0])?;
    // Unit-root tests, unlike correlograms, explicitly remove missing observations.
    let y = input
        .data()
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .collect::<Vec<_>>();
    if y.len() < 4 {
        return Err(err(
            n,
            "TooFewObservations",
            "At least four nonmissing observations are required.",
        ));
    }
    let allowed = if n == "kpsstest" {
        vec!["lags", "alpha", "trend"]
    } else {
        vec!["lags", "alpha", "model", "test"]
    };
    let mut o = Options::new(n, &a[1..], &allowed)?;
    let lags = o.nums(e, n, "lags", 0.)?;
    let alpha = o.nums(e, n, "alpha", 0.05)?;
    let trends = if n == "kpsstest" {
        o.nums(e, n, "trend", 1.)?
    } else {
        vec![0.]
    };
    let models = if n != "kpsstest" {
        o.strings(e, n, "model", "AR")?
    } else {
        vec!["AR".into()]
    };
    let tests = if n != "kpsstest" {
        o.strings(e, n, "test", "t1")?
    } else {
        vec!["t1".into()]
    };
    let mut h = vec![];
    let mut p = vec![];
    let mut stats = vec![];
    let mut critical = vec![];
    let mut reg = vec![];
    if nout == 5 {
        let max_params = if n == "adftest" {
            lags.iter()
                .copied()
                .fold(0.0, f64::max)
                .clamp(0., MAX_LAGS as f64) as usize
                + 3
        } else if n == "pptest" {
            3
        } else {
            2
        };
        let per_record = y
            .len()
            .saturating_mul(5)
            .saturating_add(max_params.saturating_mul(max_params).saturating_mul(2))
            .saturating_add(2 * MAX_LAGS);
        reserve(e, n, per_record.saturating_mul(o.count))?;
    }
    for i in 0..o.count {
        e.check_cancel()?;
        let lag = integer(n, *at(&lags, i), 0, (y.len() - 2).min(MAX_LAGS))?;
        let alpha = *at(&alpha, i);
        let range = if n == "kpsstest" {
            0.01..=0.1
        } else {
            0.001..=0.999
        };
        if !alpha.is_finite() || !range.contains(&alpha) {
            return Err(err(
                n,
                "InvalidAlpha",
                "Alpha is outside the supported significance range.",
            ));
        }
        let (s, prob, cv, record) = if n == "kpsstest" {
            let trend = *at(&trends, i);
            if trend != 0. && trend != 1. {
                return Err(err(
                    n,
                    "InvalidTrend",
                    "Trend must be a logical scalar or vector.",
                ));
            }
            let k = if trend == 1. { 2 } else { 1 };
            design(e, n, y.len(), k)?;
            let x = DMatrix::from_fn(y.len(), k, |r, c| if c == 0 { 1. } else { (r + 1) as f64 });
            let f = regression::fit(e, n, &x, &y, true)?;
            let hac = regression::hac(e, n, &f.res, lag)?;
            let mut sum = 0.;
            let mut partial = 0.;
            for r in &f.res {
                partial += r;
                sum += partial * partial;
            }
            let s = sum / (y.len() as f64).powi(2) / hac.0;
            let (prob, cv) = tables::kpss(e, trend == 1., s, alpha, out)?;
            let names = if trend == 1. {
                vec!["c".into(), "d".into()]
            } else {
                vec!["c".into()]
            };
            let rec = if nout == 5 {
                let nw_se = regression::hac_se(e, n, &x, &f, lag)?;
                Some(regression_record(
                    e,
                    &f,
                    y.len(),
                    names,
                    Some(hac),
                    Some(nw_se),
                )?)
            } else {
                None
            };
            (s, prob, cv, rec)
        } else {
            let m = model(at(&models, i), n)?;
            let test = at(&tests, i).to_ascii_lowercase();
            if !matches!(test.as_str(), "t1" | "t2" | "f")
                || (test == "f" && (m == 0 || n == "pptest"))
            {
                return Err(err(
                    n,
                    "InvalidTest",
                    "Test must be t1 or t2; ADF also supports F for ARD and TS.",
                ));
            }
            let augmentation = if n == "adftest" { lag } else { 0 };
            let start = augmentation + 1;
            let rows = y.len() - start;
            let k = m + 1 + augmentation;
            if rows <= k {
                return Err(err(
                    n,
                    "TooFewObservations",
                    "Too few observations for the requested regression.",
                ));
            }
            if rows.saturating_mul(k) > MAX_DESIGN {
                return Err(err(
                    n,
                    "WorkspaceLimit",
                    "Regression design exceeds 2000000 elements.",
                ));
            }
            work(e, n, rows.saturating_mul(k).saturating_mul(k))?;
            // Regress differences to avoid cancellation in phi-1. Coefficient
            // order matches MATLAB: c, d, a, b1,... with absent terms omitted.
            design(e, n, rows, k)?;
            let x = DMatrix::from_fn(rows, k, |r, c| {
                let t = r + start;
                if c < m {
                    if c == 0 { 1. } else { (r + 1) as f64 }
                } else if c == m {
                    y[t - 1]
                } else {
                    let j = c - m;
                    y[t - j] - y[t - j - 1]
                }
            });
            let dy = (start..y.len())
                .map(|t| y[t] - y[t - 1])
                .collect::<Vec<_>>();
            let mut f = regression::fit(e, n, &x, &dy, m > 0)?;
            let gamma = f.b[m];
            let se = f.se[m];
            let mut hac = None;
            let s = if n == "pptest" {
                let (var, ac) = regression::hac(e, n, &f.res, lag)?;
                let gamma0 = f.sse / rows as f64;
                let t = rows as f64;
                let s = if test == "t1" {
                    (gamma0 / var).sqrt() * gamma / se
                        - 0.5 * (var - gamma0) / var.sqrt() * (t * se / f.mse.sqrt())
                } else {
                    t * gamma - 0.5 * t * t * se * se / f.mse * (var - gamma0)
                };
                hac = Some((var, ac));
                s
            } else if test == "t1" {
                gamma / se
            } else if test == "t2" {
                rows as f64 * gamma / (1. - f.b[m + 1..].iter().sum::<f64>())
            } else {
                let keep = (0..k)
                    .filter(|&c| c != m && !(c == 0 && m == 1) && !(c == 1 && m == 2))
                    .collect::<Vec<_>>();
                let restricted = if keep.is_empty() {
                    dy.iter().map(|v| v * v).sum::<f64>()
                } else {
                    let xr = DMatrix::from_fn(rows, keep.len(), |r, c| x[(r, keep[c])]);
                    regression::fit(e, n, &xr, &dy, m == 2)?.sse
                };
                ((restricted - f.sse) / 2. / f.mse).max(0.)
            };
            if !s.is_finite() {
                return Err(err(n, "DegenerateData", "Test statistic is not finite."));
            }
            let (prob, cv) = tables::df(e, n, m, &test, rows, s, alpha, out)?;
            let record = if nout == 5 {
                f.b[m] += 1.;
                for (r, v) in f.fitted.iter_mut().enumerate() {
                    *v += y[r + start - 1];
                }
                f.mean = y[start..].iter().sum::<f64>() / rows as f64;
                f.sst = y[start..]
                    .iter()
                    .map(|v| (v - if m > 0 { f.mean } else { 0. }).powi(2))
                    .sum::<f64>();
                let mut names = vec![];
                if m > 0 {
                    names.push("c".into());
                }
                if m > 1 {
                    names.push("d".into());
                }
                names.push("a".into());
                names.extend((1..=augmentation).map(|j| format!("b{j}")));
                Some(regression_record(e, &f, y.len(), names, hac, None)?)
            } else {
                None
            };
            (s, prob, cv, record)
        };
        h.push(
            if n == "kpsstest" || at(&tests, i).eq_ignore_ascii_case("f") {
                s > cv
            } else {
                s < cv
            },
        );
        p.push(prob);
        stats.push(s);
        critical.push(cv);
        if let Some(r) = record {
            reg.push(r);
        }
    }
    let mut v = test_values(o.row, h, p, stats, critical)?;
    if nout == 5 {
        v.push(records(e, o.row, reg)?);
    }
    Ok(v)
}
