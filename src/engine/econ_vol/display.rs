use super::*;
use crate::{TableArray, table_array::TableParts};
pub(in crate::engine) fn body(m: &Model) -> String {
    let mut s = format!(
        "  {} with properties:\n\n         Description: \"{}\"\n          SeriesName: \"{}\"\n        Distribution: Name = \"{}\"",
        m.class(),
        m.description(),
        m.series_name,
        if m.dof.is_some() { "t" } else { "Gaussian" }
    );
    if let Some(nu) = m.dof {
        s.push_str(&format!(", DoF = {}", fmt(nu)));
    }
    s.push_str(&format!(
        "\n                   P: {}\n                   Q: {}\n            Constant: {}\n",
        m.p(),
        m.q(),
        fmt(m.constant)
    ));
    for (n, c) in [
        ("GARCH", &m.garch),
        ("ARCH", &m.arch),
        ("Leverage", &m.leverage),
    ] {
        if n == "Leverage" && m.kind == Kind::Garch {
            continue;
        }
        let active = c
            .iter()
            .enumerate()
            .filter(|&(_, x)| *x != 0.)
            .collect::<Vec<_>>();
        let v = active
            .iter()
            .map(|(_, x)| fmt(**x))
            .collect::<Vec<_>>()
            .join(" ");
        let l = active
            .iter()
            .map(|(j, _)| (j + 1).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        s.push_str(&format!(
            "{n:>20}: {{{v}}}{}\n",
            if active.is_empty() {
                String::new()
            } else {
                format!(" at lag{} [{l}]", if active.len() == 1 { "" } else { "s" })
            }
        ));
    }
    s.push_str(&format!(
        "              Offset: {}\nUnconditionalVariance: {}\n",
        fmt(m.offset),
        fmt(m.unconditional_variance())
    ));
    s
}
fn fmt(x: f64) -> String {
    super::super::string_class::number(x)
}
fn table(m: &Model) -> Result<TableArray> {
    let fit = m.fit.as_ref().ok_or("Model has not been estimated")?;
    let x = fit.values.clone();
    let n = x.len();
    let mut se = vec![];
    let mut ts = vec![];
    let mut pv = vec![];
    for (i, &v) in x.iter().enumerate() {
        let s = fit.covariance.data()[i * n + i].sqrt();
        se.push(s);
        let t = if s == 0. { f64::NAN } else { v / s };
        ts.push(t);
        pv.push(libm::erfc(t.abs() / 2_f64.sqrt()));
    }
    let columns = [x, se, ts, pv]
        .into_iter()
        .map(|x| Matrix::from_column_vec(n, 1, x).map(Array::Numeric))
        .collect::<Result<Vec<_>>>()?;
    let mut p = TableParts::new(
        ["Value", "StandardError", "TStatistic", "PValue"]
            .map(String::from)
            .to_vec(),
        columns,
        n,
    );
    p.row_names = Some(fit.names.clone());
    TableArray::new(p)
}
pub(super) fn summary_body(m: &Model) -> Result<String> {
    let Some(f) = &m.fit else {
        return Ok(body(m));
    };
    let k = f.free as f64;
    let n = f.sample_size as f64;
    Ok(format!(
        "  {}\n\n    Effective Sample Size: {}\n    Number of Estimated Parameters: {}\n    LogLikelihood: {}\n    AIC: {}\n    BIC: {}\n\n{}",
        m.description(),
        f.sample_size,
        f.free,
        fmt(f.log_likelihood),
        fmt(-2. * f.log_likelihood + 2. * k),
        fmt(-2. * f.log_likelihood + n.ln() * k),
        table(m)?.preview()
    ))
}
pub(super) fn summarize(
    e: &mut Engine,
    m: &Model,
    original: &Value,
    nout: usize,
    out: &mut Execution,
) -> Result<Value> {
    if nout == 0 {
        e.emit(out, summary_body(m)?)?;
        return Ok(Value::Void);
    }
    let Some(f) = &m.fit else {
        return Ok(original.clone());
    };
    let k = f.free as f64;
    structure(vec![
        ("Description", string_array(&m.description())?),
        (
            "SampleSize",
            Array::Numeric(Matrix::scalar(f.sample_size as f64)),
        ),
        ("NumEstimatedParameters", Array::Numeric(Matrix::scalar(k))),
        (
            "LogLikelihood",
            Array::Numeric(Matrix::scalar(f.log_likelihood)),
        ),
        (
            "AIC",
            Array::Numeric(Matrix::scalar(-2. * f.log_likelihood + 2. * k)),
        ),
        (
            "BIC",
            Array::Numeric(Matrix::scalar(
                -2. * f.log_likelihood + (f.sample_size as f64).ln() * k,
            )),
        ),
        ("Table", Array::Table(table(m)?)),
    ])
}
