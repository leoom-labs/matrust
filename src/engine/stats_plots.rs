//! Statistics plots built from ordinary plot/bar calls: gplot, histfit,
//! boxplot, plus perfcurve (ROC and other performance curves, no plot).
use super::{Engine, Execution, Value};
use crate::{Array, Matrix, Result};
use std::collections::HashMap;

const NAMES: &[&str] = &["gplot", "histfit", "boxplot", "perfcurve"];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn column(m: Vec<f64>) -> Value {
    let n = m.len();
    Value::Matrix(Matrix::from_column_vec(n, 1, m).expect("column"))
}

fn row(m: Vec<f64>) -> Value {
    let n = m.len();
    Value::Matrix(Matrix::from_column_vec(1, n, m).expect("row"))
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

/// Runs `f` with hold on, then restores the previous hold state.
fn held<T>(
    e: &mut Engine,
    out: &mut Execution,
    f: impl FnOnce(&mut Engine, &mut Execution) -> Result<T>,
) -> Result<T> {
    let was = e.figure.as_ref().is_some_and(|f| f.hold);
    let result = f(e, out);
    if !was {
        e.call("hold", vec![text("off")], out)?;
    }
    result
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "gplot" => gplot(e, args, nargout, out),
        "histfit" => histfit(e, args, out),
        "boxplot" => boxplot(e, args, out),
        _ => perfcurve(e, args, nargout),
    }
}

fn gplot(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let (a, xy, spec) = match args {
        [a, xy] => (a, xy, None),
        [a, xy, s] => (a, xy, Some(s.clone())),
        _ => return Err("gplot requires an adjacency matrix and coordinates".into()),
    };
    let full = e.call("full", vec![a.clone()], out)?;
    let a = e.real_arg("gplot", &full)?;
    let xy = e.real_arg("gplot", xy)?;
    let n = a.rows();
    if a.cols() != n || xy.rows() != n || xy.cols() < 2 {
        return Err("gplot: A must be n-by-n and xy n-by-2".into());
    }
    // Count before allocating either the edge list or the segment arrays.
    let mut edges = 0usize;
    for chunk in a.data().chunks(4096) {
        e.check_cancel()?;
        edges += chunk.iter().filter(|&&v| v != 0.).count();
    }
    let len = edges
        .checked_mul(3)
        .ok_or("gplot: output dimensions overflow")?;
    Matrix::checked_len(len, 1).map_err(|s| format!("gplot: output size: {s}"))?;
    // Edge pairs, stable-sort scratch and both output coordinate arrays.
    e.preflight_elements(edges.saturating_mul(10))?;
    // [i,j] = find(A) in column-major order, then a stable sort by max(i,j).
    let mut pairs = Vec::with_capacity(edges);
    for j in 0..n {
        e.check_cancel()?;
        for i in 0..n {
            if a.data()[i + j * n] != 0. {
                pairs.push((i, j));
            }
        }
    }
    pairs.sort_by_key(|&(i, j)| i.max(j));
    e.check_cancel()?;
    let (mut x, mut y) = (Vec::with_capacity(len), Vec::with_capacity(len));
    let at = |r: usize, c: usize| xy.data()[r + c * n];
    for (k, (i, j)) in pairs.into_iter().enumerate() {
        if k % 4096 == 0 {
            e.check_cancel()?;
        }
        x.extend([at(i, 0), at(j, 0), f64::NAN]);
        y.extend([at(i, 1), at(j, 1), f64::NAN]);
    }
    if nargout > 0 {
        return Ok(vec![column(x), column(y)]);
    }
    let mut plot_args = vec![column(x), column(y)];
    plot_args.extend(spec);
    e.call("plot", plot_args, out)?;
    Ok(Vec::new())
}

fn histfit(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let Some(data) = args.first() else {
        return Err("histfit requires data".into());
    };
    let data: Vec<f64> = e
        .real_arg("histfit", data)?
        .data()
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .collect();
    if data.is_empty() {
        return Err("histfit: the data must contain at least one non-NaN value".into());
    }
    let nbins = match args.get(1) {
        Some(v) if !matches!(v, Value::Matrix(m) if m.data().is_empty()) => {
            let b = e.real_arg("histfit", v)?.as_scalar()?;
            if !(b.is_finite() && b >= 1. && b.fract() == 0.) {
                return Err("histfit: nbins must be a positive integer".into());
            }
            // The bar renderer accepts at most 10000 bars. Check in floating
            // point before casting, so huge finite integers cannot saturate usize.
            if b > 10000. {
                return Err("histfit: nbins exceeds the plotting limit of 10000".into());
            }
            b as usize
        }
        _ => (data.len() as f64).sqrt().ceil() as usize,
    };
    e.preflight_elements(nbins.saturating_mul(2))?;
    let dist = match args.get(2) {
        Some(v) => e.text_value(v.clone())?,
        None => "normal".into(),
    };
    let lo = data.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (lo_edge, width) = if hi > lo {
        (lo, (hi - lo) / nbins as f64)
    } else {
        (lo - 0.5, 1. / nbins as f64)
    };
    let mut counts = vec![0.; nbins];
    for v in &data {
        let k = (((v - lo_edge) / width).floor() as usize).min(nbins - 1);
        counts[k] += 1.;
    }
    let centers: Vec<f64> = (0..nbins)
        .map(|k| lo_edge + (k as f64 + 0.5) * width)
        .collect();
    let pd = e.call("fitdist", vec![column(data.clone()), text(&dist)], out)?;
    let (a, b) = (lo_edge - width, lo_edge + (nbins as f64 + 1.) * width);
    let xs: Vec<f64> = (0..100).map(|k| a + (b - a) * k as f64 / 99.).collect();
    let density = e.call("pdf", vec![pd, row(xs.clone())], out)?;
    let scale = data.len() as f64 * width;
    let ys: Vec<f64> = e
        .real_arg("histfit", &density)?
        .data()
        .iter()
        .map(|d| d * scale)
        .collect();
    let bars = e.call(
        "bar",
        vec![row(centers), row(counts), Value::Matrix(Matrix::scalar(1.))],
        out,
    )?;
    let line = held(e, out, |e, out| {
        e.call("hold", vec![text("on")], out)?;
        e.call(
            "plot",
            vec![
                row(xs),
                row(ys),
                text("r"),
                text("LineWidth"),
                Value::Matrix(Matrix::scalar(2.)),
            ],
            out,
        )
    })?;
    Ok(vec![e.call("vertcat", vec![bars, line], out)?])
}

/// Percentile of sorted data the way prctile does it (midpoint rule).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let pos = n as f64 * p / 100. + 0.5;
    if pos <= 1. {
        return sorted[0];
    }
    if pos >= n as f64 {
        return sorted[n - 1];
    }
    let k = pos.floor() as usize;
    let t = pos - k as f64;
    sorted[k - 1] + t * (sorted[k] - sorted[k - 1])
}

fn group_labels(e: &mut Engine, g: &Value, n: usize) -> Result<Vec<String>> {
    // Numeric observations can exceed the string-array/cell-array slot limit.
    // Format labels directly with string's formatter instead of materializing
    // either bounded container solely as intermediate workspace.
    if let Value::Matrix(m) = g {
        if m.len() != n {
            return Err("boxplot: the group variable must have one entry per observation".into());
        }
        e.preflight_elements(n.saturating_mul(4))?;
        return m
            .data()
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                if i.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                Ok(super::string_class::number(v))
            })
            .collect();
    }
    let mut out = Execution::default();
    let strings = e.call("string", vec![g.clone()], &mut out)?;
    let cell = e.call("cellstr", vec![strings], &mut out)?;
    let Value::Container(Array::Cell(c)) = cell else {
        return Err("boxplot: unsupported group variable".into());
    };
    if c.len() != n {
        return Err("boxplot: the group variable must have one entry per observation".into());
    }
    (0..n)
        .map(|k| {
            if k % 4096 == 0 {
                e.check_cancel()?;
            }
            e.text_value(Value::from(c.get_linear(k)?.as_ref().clone()))
        })
        .collect()
}

fn boxplot(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let Some(x) = args.first() else {
        return Err("boxplot requires data".into());
    };
    let x = e.real_arg("boxplot", x)?;
    let mut rest = &args[1..];
    let mut groups: Vec<(String, Vec<f64>)> = Vec::new();
    if let Some(g) = rest.first()
        && !(matches!(g, Value::Text(_)) && rest.len().is_multiple_of(2))
    {
        let labels = group_labels(e, g, x.len())?;
        // Preserve first-observation order while looking up groups in O(1).
        let mut index = HashMap::new();
        for (i, (label, v)) in labels.into_iter().zip(x.data().iter().copied()).enumerate() {
            if i % 4096 == 0 {
                e.check_cancel()?;
            }
            let next = groups.len();
            let k = *index.entry(label.clone()).or_insert(next);
            if k == next {
                if next >= crate::plot::MAX_SERIES / 7 {
                    return Err("boxplot: group count exceeds the axes series limit".into());
                }
                groups.push((label, Vec::new()));
            }
            groups[k].1.push(v);
        }
        rest = &rest[1..];
    } else if x.rows() == 1 || x.cols() == 1 {
        groups.push(("1".into(), x.data().to_vec()));
    } else {
        for c in 0..x.cols() {
            let col = x.data()[c * x.rows()..(c + 1) * x.rows()].to_vec();
            groups.push(((c + 1).to_string(), col));
        }
    }
    let mut labels: Option<Vec<String>> = None;
    let mut whisker = 1.5;
    for pair in rest.chunks(2) {
        let key = e.text_value(pair[0].clone())?.to_ascii_lowercase();
        let Some(value) = pair.get(1) else {
            return Err(format!("boxplot: option '{key}' needs a value"));
        };
        match key.as_str() {
            "labels" => {
                let cell = e.call("cellstr", vec![value.clone()], out)?;
                let Value::Container(Array::Cell(c)) = cell else {
                    return Err("boxplot: Labels must be text".into());
                };
                labels = Some(
                    (0..c.len())
                        .map(|k| e.text_value(Value::from(c.get_linear(k)?.as_ref().clone())))
                        .collect::<Result<_>>()?,
                );
            }
            "whisker" => whisker = e.real_arg("boxplot", value)?.as_scalar()?,
            _ => return Err(format!("boxplot: option '{key}' is not supported yet")),
        }
    }
    let k = groups.len();
    if k > crate::plot::MAX_SERIES / 7 {
        return Err("boxplot: group count exceeds the axes series limit".into());
    }
    let mut cols = Vec::new();
    let ticks: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    held(e, out, |e, out| {
        // The first line replaces the axes contents unless hold is on.
        let mut first = true;
        for (i, (_, values)) in groups.iter().enumerate() {
            e.check_cancel()?;
            let pos = i as f64 + 1.;
            let mut v: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
            v.sort_by(f64::total_cmp);
            let (q1, med, q3) = if v.is_empty() {
                (f64::NAN, f64::NAN, f64::NAN)
            } else {
                (
                    percentile(&v, 25.),
                    percentile(&v, 50.),
                    percentile(&v, 75.),
                )
            };
            let iqr = q3 - q1;
            let (lo, hi) = (q1 - whisker * iqr, q3 + whisker * iqr);
            let inside: Vec<f64> = v.iter().copied().filter(|x| *x >= lo && *x <= hi).collect();
            let lower = inside.first().copied().unwrap_or(q1).min(q1);
            let upper = inside.last().copied().unwrap_or(q3).max(q3);
            let outliers: Vec<f64> = v.iter().copied().filter(|x| *x < lo || *x > hi).collect();
            let w = 0.25;
            let mut line = |xs: Vec<f64>, ys: Vec<f64>, spec: &str| {
                let h = e.call("plot", vec![row(xs), row(ys), text(spec)], out)?;
                if std::mem::take(&mut first) {
                    e.call("hold", vec![text("on")], out)?;
                }
                Ok::<Value, String>(h)
            };
            let handles = vec![
                line(vec![pos, pos], vec![q3, upper], "k--")?,
                line(vec![pos, pos], vec![lower, q1], "k--")?,
                line(vec![pos - w / 2., pos + w / 2.], vec![upper, upper], "k-")?,
                line(vec![pos - w / 2., pos + w / 2.], vec![lower, lower], "k-")?,
                line(
                    vec![pos - w, pos - w, pos + w, pos + w, pos - w],
                    vec![q1, q3, q3, q1, q1],
                    "b-",
                )?,
                line(vec![pos - w, pos + w], vec![med, med], "r-")?,
                if outliers.is_empty() {
                    line(vec![f64::NAN], vec![f64::NAN], "r+")?
                } else {
                    line(vec![pos; outliers.len()], outliers, "r+")?
                },
            ];
            cols.push(e.call("vertcat", handles, out)?);
        }
        Ok(())
    })?;
    // Group labels are validated but not drawn: axes tick labels are not supported yet.
    let _ = labels;
    e.call("xlim", vec![row(vec![0.5, k as f64 + 0.5])], out)?;
    let _ = ticks;
    Ok(vec![e.call("horzcat", cols, out)?])
}

/// The criterion `name` from confusion counts at one threshold.
fn criterion(name: &str, tp: f64, fp: f64, tn: f64, fn_: f64) -> Result<f64> {
    let div = |a: f64, b: f64| if b == 0. { f64::NAN } else { a / b };
    Ok(match name {
        "fpr" | "fall" => div(fp, fp + tn),
        "tpr" | "reca" | "sens" => div(tp, tp + fn_),
        "tnr" | "spec" => div(tn, fp + tn),
        "fnr" | "miss" => div(fn_, tp + fn_),
        "ppv" | "prec" => div(tp, tp + fp),
        "npv" => div(tn, tn + fn_),
        "accu" => div(tp + tn, tp + fp + tn + fn_),
        "tp" => tp,
        "fp" => fp,
        "tn" => tn,
        "fn" => fn_,
        _ => return Err(format!("perfcurve: criterion '{name}' is not supported")),
    })
}

fn perfcurve(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    let [labels, scores, pos, options @ ..] = args else {
        return Err("perfcurve requires labels, scores and the positive class".into());
    };
    let mut out = Execution::default();
    let is_pos = match labels {
        Value::Container(Array::Categorical(c)) => {
            let positive = group_labels(e, pos, 1)?.remove(0);
            let mut matches = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                if i.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                matches.push(f64::from(c.codes()[i] != 0 && c.text(i) == positive));
            }
            Value::Matrix(Matrix::from_column_vec(c.rows(), c.cols(), matches)?)
        }
        Value::Container(Array::Cell(_)) => {
            e.call("strcmp", vec![labels.clone(), pos.clone()], &mut out)?
        }
        _ => e.call("eq", vec![labels.clone(), pos.clone()], &mut out)?,
    };
    let is_pos: Vec<bool> = e
        .real_arg("perfcurve", &is_pos)?
        .data()
        .iter()
        .map(|v| *v != 0.)
        .collect();
    let scores = e.real_arg("perfcurve", scores)?.data().to_vec();
    if scores.len() != is_pos.len() {
        return Err("perfcurve: labels and scores must have the same length".into());
    }
    let (mut xcrit, mut ycrit) = ("fpr".to_string(), "tpr".to_string());
    for pair in options.chunks(2) {
        let key = e.text_value(pair[0].clone())?.to_ascii_lowercase();
        let Some(v) = pair.get(1) else {
            return Err(format!("perfcurve: option '{key}' needs a value"));
        };
        let v = e.text_value(v.clone())?.to_ascii_lowercase();
        match key.as_str() {
            "xcrit" => xcrit = v,
            "ycrit" => ycrit = v,
            _ => return Err(format!("perfcurve: option '{key}' is not supported yet")),
        }
    }
    let mut order: Vec<usize> = (0..scores.len()).filter(|&k| !scores[k].is_nan()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let p = order.iter().filter(|&&k| is_pos[k]).count() as f64;
    let n = order.len() as f64 - p;
    let (mut tp, mut fp) = (0., 0.);
    let first = order.first().map_or(f64::NAN, |&k| scores[k]);
    let mut points = vec![(0., 0., first)];
    let mut k = 0;
    while k < order.len() {
        let t = scores[order[k]];
        while k < order.len() && scores[order[k]] == t {
            if is_pos[order[k]] {
                tp += 1.;
            } else {
                fp += 1.;
            }
            k += 1;
        }
        points.push((tp, fp, t));
    }
    let (mut xs, mut ys, mut ts) = (Vec::new(), Vec::new(), Vec::new());
    for (tp, fp, t) in &points {
        let (tn, fn_) = (n - fp, p - tp);
        xs.push(criterion(&xcrit, *tp, *fp, tn, fn_)?);
        ys.push(criterion(&ycrit, *tp, *fp, tn, fn_)?);
        ts.push(*t);
    }
    // The (0,0) start shares the largest score with the first real threshold.
    let auc: f64 = xs
        .windows(2)
        .zip(ys.windows(2))
        .filter(|(x, y)| x.iter().chain(y.iter()).all(|v| v.is_finite()))
        .map(|(x, y)| (x[1] - x[0]) * (y[0] + y[1]) / 2.)
        .sum();
    let mut values = vec![
        column(xs),
        column(ys),
        column(ts),
        Value::Matrix(Matrix::scalar(auc)),
    ];
    values.truncate(nargout.max(1));
    Ok(values)
}
