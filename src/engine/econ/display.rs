use super::*;
use crate::table_array::{TableArray, TableParts};
pub(in crate::engine) fn property(e: &Engine, m: &Model, n: &str) -> Result<Value> {
    let value = match n {
        "P" => number(m.p() as f64),
        "Q" => number(m.q() as f64),
        "D" => number(m.d as f64),
        "Seasonality" => number(m.s as f64),
        "Constant" => number(m.c),
        "Variance" => m
            .conditional
            .as_ref()
            .map_or_else(|| number(m.variance), |v| v.object.clone()),
        "AR" => cells(e, &m.ar)?,
        "SAR" => cells(e, &m.sar)?,
        "MA" => cells(e, &m.ma)?,
        "SMA" => cells(e, &m.sma)?,
        "Beta" => row(m.beta.clone())?,
        "Description" => string(e, &m.description())?,
        "SeriesName" => string(e, &m.series)?,
        "Distribution" => {
            let mut p = vec![(
                "Name",
                string(e, if m.dof.is_some() { "t" } else { "Gaussian" })?,
            )];
            if let Some(v) = m.dof {
                p.push(("DoF", number(v)));
            }
            record(e, p)?
        }
        _ => {
            return Err(err(
                "arima",
                "UnknownProperty",
                &format!("Unrecognized property '{n}' for class 'arima'."),
            ));
        }
    };
    Ok(Value::from(value))
}
pub(in crate::engine) fn body(m: &Model) -> String {
    let mut s = format!(
        "  arima with properties:\n\n     Description: \"{}\"\n      SeriesName: \"{}\"\n    Distribution: Name = \"{}\"\n               P: {}\n               D: {}\n               Q: {}\n        Constant: {}\n",
        m.description(),
        m.series,
        if m.dof.is_some() { "t" } else { "Gaussian" },
        m.p(),
        m.d,
        m.q(),
        m.c
    );
    for (name, c) in [
        ("AR", &m.ar),
        ("SAR", &m.sar),
        ("MA", &m.ma),
        ("SMA", &m.sma),
    ] {
        let entries = c
            .iter()
            .filter(|v| **v != 0.)
            .map(|v| format!("{v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let lags = c
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.)
            .map(|(i, _)| (i + 1).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        s.push_str(&format!("{name:>16}: {{{entries}}}"));
        if !c.is_empty() {
            s.push_str(&format!(" at lags [{lags}]"));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "     Seasonality: {}\n            Beta: [{}]\n        Variance: {}\n",
        m.s,
        m.beta
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" "),
        m.conditional.as_ref().map_or_else(
            || m.variance.to_string(),
            |v| format!(
                "[{}({},{}) Model]",
                v.model.class().to_uppercase(),
                v.model.p(),
                v.model.q()
            )
        )
    ));
    if let Some(v) = m.dof {
        s.push_str(&format!("             DoF: {v}\n"));
    }
    s
}
pub(super) fn summary_text(m: &Model) -> String {
    let Some(f) = &m.fit else {
        return body(m);
    };
    let k = f.params.len();
    let mut s = format!(
        "\n   {}\n\n    Effective Sample Size: {}\n    Number of Estimated Parameters: {}\n    LogLikelihood: {:0.10}\n    AIC: {:0.10}\n    BIC: {:0.10}\n\n                  Value      StandardError      TStatistic          PValue\n",
        m.description(),
        f.n,
        f.free,
        f.logl,
        -2. * f.logl + 2. * f.free as f64,
        -2. * f.logl + (f.n as f64).ln() * f.free as f64
    );
    let (mean, variance) = table_indices(f);
    let indices = if m.conditional.is_some() {
        mean
    } else {
        (0..k).collect()
    };
    for i in indices {
        summary_row(&mut s, f, i);
    }
    if let Some(vm) = &m.conditional {
        s.push_str(&format!("\n   {}:\n\n                  Value      StandardError      TStatistic          PValue\n", vm.model.description()));
        for i in variance {
            summary_row(&mut s, f, i);
        }
    }
    s
}
fn summary_row(s: &mut String, f: &arima::Fit, i: usize) {
    let k = f.params.len();
    let name = f.names[i].trim_start_matches("Variance.");
    let v = f.params[i];
    let se = f.covariance[i * k + i].max(0.).sqrt();
    let t = if se > 0. { v / se } else { f64::NAN };
    let p = libm::erfc(t.abs() / std::f64::consts::SQRT_2);
    s.push_str(&format!(
        "{name:>14} {v:14.7} {se:18.7} {t:15.7} {p:15.7}\n"
    ));
}
fn parameter_table(f: &arima::Fit, indices: &[usize]) -> Result<Array> {
    let k = f.params.len();
    let mut cols = vec![vec![]; 4];
    for &i in indices {
        let v = f.params[i];
        let se = f.covariance[i * k + i].max(0.).sqrt();
        let t = if se > 0. { v / se } else { f64::NAN };
        cols[0].push(v);
        cols[1].push(se);
        cols[2].push(t);
        cols[3].push(libm::erfc(t.abs() / std::f64::consts::SQRT_2));
    }
    let columns = cols.into_iter().map(column).collect::<Result<Vec<_>>>()?;
    let mut parts = TableParts::new(
        ["Value", "StandardError", "TStatistic", "PValue"]
            .map(String::from)
            .to_vec(),
        columns,
        indices.len(),
    );
    parts.row_names = Some(
        indices
            .iter()
            .map(|&i| f.names[i].trim_start_matches("Variance.").to_owned())
            .collect(),
    );
    Ok(Array::Table(TableArray::new(parts)?))
}
pub(super) fn summarize(e: &Engine, m: &Model, original: Value) -> Result<Value> {
    let Some(f) = &m.fit else {
        return Ok(original);
    };
    let (mean, variance) = table_indices(f);
    Ok(Value::from(record(
        e,
        vec![
            ("Description", string(e, &m.description())?),
            ("SampleSize", number(f.n as f64)),
            ("NumEstimatedParameters", number(f.free as f64)),
            ("LogLikelihood", number(f.logl)),
            ("AIC", number(-2. * f.logl + 2. * f.free as f64)),
            (
                "BIC",
                number(-2. * f.logl + (f.n as f64).ln() * f.free as f64),
            ),
            ("Table", parameter_table(f, &mean)?),
            ("VarianceTable", parameter_table(f, &variance)?),
        ],
    )?))
}
fn table_indices(f: &arima::Fit) -> (Vec<usize>, Vec<usize>) {
    let mean = f
        .names
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() != "Variance" && !s.starts_with("Variance."))
        .map(|(i, _)| i)
        .collect();
    let variance = f
        .names
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == "Variance" || s.starts_with("Variance."))
        .map(|(i, _)| i)
        .collect();
    (mean, variance)
}
pub(super) fn aicbic(e: &Engine, a: &[Value], nout: usize) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err(err(
            "aicbic",
            "InvalidNumInputs",
            "Not enough input arguments.",
        ));
    }
    let ll = matrix(e, &a[0])?;
    let k = matrix(e, &a[1])?;
    let has_n = a.get(2).is_some_and(|v| text(v).is_err());
    let n = if has_n { Some(matrix(e, &a[2])?) } else { None };
    if nout > 1 && n.is_none() {
        return Err(err(
            "aicbic",
            "MissingNumObs",
            "numObs is required for BIC and other information criteria.",
        ));
    }
    let o = options(e, "aicbic", &a[if has_n { 3 } else { 2 }..], &["Normalize"])?;
    let normalize = o
        .get("Normalize")
        .map(|v| match v.clone().array(&e.cancel)? {
            Array::Logical(m) if m.len() == 1 => Ok(f64::from(m.data()[0])),
            _ => scalar(e, v),
        })
        .transpose()?
        .unwrap_or(0.);
    if !matches!(normalize, 0. | 1.) {
        return Err(err(
            "aicbic",
            "InvalidNormalize",
            "Normalize must be a logical scalar.",
        ));
    }
    if normalize == 1. && n.is_none() {
        return Err(err("aicbic", "MissingNumObs", "Normalize requires numObs."));
    }
    if ll.is_empty()
        || (ll.rows() != 1 && ll.cols() != 1)
        || k.is_empty()
        || (k.len() != 1 && k.len() != ll.len())
        || (k.rows() != 1 && k.cols() != 1)
        || n.as_ref().is_some_and(|v| {
            (v.len() != 1 && v.len() != ll.len()) || (v.rows() != 1 && v.cols() != 1)
        })
    {
        return Err(err(
            "aicbic",
            "InvalidSize",
            "Inputs must be compatible numeric vectors or scalars.",
        ));
    }
    if ll.data().iter().any(|v| !v.is_finite())
        || k.data()
            .iter()
            .any(|v| !v.is_finite() || *v < 0. || v.fract() != 0.)
        || n.as_ref().is_some_and(|v| {
            v.data()
                .iter()
                .any(|v| !v.is_finite() || *v <= 0. || v.fract() != 0.)
        })
    {
        return Err(err(
            "aicbic",
            "InvalidInput",
            "Log likelihoods must be finite, parameter counts nonnegative integers, and sample sizes positive integers.",
        ));
    }
    preflight(e, ll.len(), 1, 7)?;
    let mut ic: Vec<Vec<f64>> = (0..5).map(|_| Vec::with_capacity(ll.len())).collect();
    for (i, &l) in ll.data().iter().enumerate() {
        let k = k.data()[if k.len() == 1 { 0 } else { i }];
        let n = n
            .as_ref()
            .map_or(f64::NAN, |n| n.data()[if n.len() == 1 { 0 } else { i }]);
        let scale = if normalize == 1. { n } else { 1. };
        let aic = -2. * l + 2. * k;
        let bic = -2. * l + k * n.ln();
        let aicc = aic + 2. * k * (k + 1.) / (n - k - 1.);
        let values = [aic, bic, aicc, bic + k, -2. * l + 2. * k * n.ln().ln()];
        for (v, x) in ic.iter_mut().zip(values) {
            v.push(x / scale);
        }
    }
    let arrays = ic
        .into_iter()
        .map(|v| {
            Ok(Array::Numeric(Matrix::from_column_vec(
                ll.rows(),
                ll.cols(),
                v,
            )?))
        })
        .collect::<Result<Vec<_>>>()?;
    let st = record(
        e,
        ["aic", "bic", "aicc", "caic", "hqc"]
            .into_iter()
            .zip(arrays.iter().cloned())
            .collect(),
    )?;
    Ok(vec![
        Value::from(arrays[0].clone()),
        Value::from(arrays[1].clone()),
        Value::from(st),
    ])
}
