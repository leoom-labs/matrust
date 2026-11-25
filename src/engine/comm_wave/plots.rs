//! Uses ordinary figure/axes/line objects so all plotted data remain editable.
use super::*;
fn txt(s: &str) -> Value {
    Value::Text(s.into())
}
fn num(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}
fn row(v: Vec<f64>) -> Result<Value> {
    rv(1, v.len(), v)
}
fn label(e: &mut Engine, out: &mut Execution, name: &str, s: &str) -> Result<()> {
    e.call(name, vec![txt(s)], out)?;
    Ok(())
}
fn iq(e: &Engine, v: &Value, name: &str) -> Result<(Array, usize, bool)> {
    let x = numeric(e, v, name, 2)?;
    let complex = matches!(x, Array::Complex(_));
    if (complex && x.rows() > 1 && x.cols() > 1) || (!complex && x.rows() > 1 && x.cols() > 2) {
        return Err(format!(
            "{name}: expected a vector or a real two-column I/Q matrix"
        ));
    }
    let two = !complex && x.cols() == 2 && x.rows() > 1;
    let n = if two { x.rows() } else { x.len() };
    Ok((x, n, complex || two))
}
fn component(x: &Array, n: usize, i: usize, quad: bool) -> f64 {
    if matches!(x, Array::Complex(_)) {
        let z = at(x, i);
        if quad { z.im } else { z.re }
    } else {
        at(x, i + if quad { n } else { 0 }).re
    }
}
fn figure(e: &mut Engine, target: Option<&Value>, out: &mut Execution) -> Result<Value> {
    if let Some(target) = target {
        let typ = e.call("get", vec![target.clone(), txt("Type")], out)?;
        if text(e, &typ, "comm_wave plot target")? != "figure" {
            return Err("comm_wave: plot target must be an existing figure".into());
        }
    }
    e.call("figure", target.cloned().into_iter().collect(), out)
}
fn line_args(x: Value, y: Value, spec: &str) -> Vec<Value> {
    let mut args = vec![x, y, txt(spec)];
    if !spec.contains(['-', ':'])
        && spec.contains(['o', '.', '+', 'x', '*', 's', 'd', '^', 'v', '<', '>'])
    {
        // Marker-only legacy LineSpecs otherwise select the scatter kind.
        args.extend([txt("LineStyle"), txt("none")]);
    }
    args
}
pub(super) fn scatter(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Value> {
    arity(a, 1, 5, "scatterplot")?;
    let (x, n, quad) = iq(e, &a[0], "scatterplot")?;
    let step = match a.get(1).filter(|v| !empty(v)) {
        Some(v) => integer(e, v, 1, MAX_SAMPLES, "scatterplot n")?,
        None => 1,
    };
    let offset = match a.get(2).filter(|v| !empty(v)) {
        Some(v) => integer(e, v, 0, MAX_SAMPLES, "scatterplot offset")?,
        None => 0,
    };
    let spec = match a.get(3).filter(|v| !empty(v)) {
        Some(v) => text(e, v, "scatterplot")?,
        None => "b.".into(),
    };
    let count = n.saturating_sub(offset).div_ceil(step);
    if count > crate::plot::MAX_POINTS {
        return Err("scatterplot: point limit exceeded".into());
    }
    reserve(e, count.saturating_mul(8))?;
    work(e, count)?;
    let mut xs = zeros::<f64>(e, count)?;
    let mut ys = zeros::<f64>(e, count)?;
    for (j, i) in (offset..n).step_by(step).enumerate() {
        if j % 4096 == 0 {
            e.check_cancel()?;
        }
        xs[j] = component(&x, n, i, false);
        ys[j] = if quad { component(&x, n, i, true) } else { 0. };
    }
    let fig = figure(e, a.get(4).filter(|v| !empty(v)), out)?;
    e.call("plot", line_args(row(xs)?, row(ys)?, &spec), out)?;
    label(e, out, "xlabel", "In-Phase")?;
    label(e, out, "ylabel", "Quadrature")?;
    label(e, out, "title", "Scatter plot")?;
    Ok(fig)
}
pub(super) fn eye(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Value> {
    arity(a, 2, 6, "eyediagram")?;
    let (x, n, quad) = iq(e, &a[0], "eyediagram")?;
    let ns = integer(e, &a[1], 2, MAX_SAMPLES, "eyediagram n")?;
    let period = match a.get(2).filter(|v| !empty(v)) {
        Some(v) => positive(e, v, "eyediagram period")?,
        None => 1.,
    };
    let offset = match a.get(3).filter(|v| !empty(v)) {
        Some(v) => integer(e, v, 0, ns - 1, "eyediagram offset")?,
        None => 0,
    };
    let spec = match a.get(4).filter(|v| !empty(v)) {
        Some(v) => text(e, v, "eyediagram")?,
        None => "b-".into(),
    };
    let half = ns / 2;
    let points = 2 * half + 1;
    let traces = if n > offset {
        (n - 1 - offset + half) / ns + 1
    } else {
        0
    };
    let total = (points + 1)
        .checked_mul(traces)
        .filter(|n| *n <= crate::plot::MAX_POINTS)
        .ok_or("eyediagram: trace size limit exceeded")?;
    reserve(e, total.saturating_mul(if quad { 12 } else { 6 }))?;
    work(e, total)?;
    let mut times = zeros::<f64>(e, total)?;
    for trace in 0..traces {
        e.check_cancel()?;
        for k in 0..=points {
            times[trace * (points + 1) + k] = if k == points {
                f64::NAN
            } else {
                (k as f64 - half as f64) * period / ns as f64
            };
        }
    }
    let fig = figure(e, a.get(5).filter(|v| !empty(v)), out)?;
    for q in 0..if quad { 2 } else { 1 } {
        if quad {
            e.call("subplot", vec![num(2.), num(1.), num((q + 1) as f64)], out)?;
        }
        let mut ys = zeros::<f64>(e, total)?;
        for trace in 0..traces {
            e.check_cancel()?;
            let center = offset + trace * ns;
            for k in 0..points {
                if k % 4096 == 0 {
                    e.check_cancel()?;
                }
                let i = center as i64 + k as i64 - half as i64;
                ys[trace * (points + 1) + k] = if i >= 0 && (i as usize) < n {
                    component(&x, n, i as usize, q == 1)
                } else {
                    f64::NAN
                };
            }
            ys[trace * (points + 1) + points] = f64::NAN;
        }
        e.call("plot", line_args(row(times.clone())?, row(ys)?, &spec), out)?;
        label(e, out, "xlabel", "Time")?;
        label(e, out, "ylabel", "Amplitude")?;
        label(
            e,
            out,
            "title",
            if q == 1 {
                "Eye Diagram for Quadrature Signal"
            } else {
                "Eye Diagram for In-Phase Signal"
            },
        )?;
        e.call("xlim", vec![row(vec![-period / 2., period / 2.])?], out)?;
    }
    Ok(fig)
}
pub(super) fn ber(
    e: &mut Engine,
    x: &[f64],
    y: &[f64],
    fx: &[f64],
    fits: &[(String, Vec<f64>)],
    out: &mut Execution,
) -> Result<()> {
    e.call("figure", vec![], out)?;
    e.call(
        "semilogy",
        vec![
            row(x.to_vec())?,
            row(y.to_vec())?,
            txt("bo"),
            txt("LineStyle"),
            txt("none"),
            txt("DisplayName"),
            txt("Empirical BER"),
        ],
        out,
    )?;
    e.call("hold", vec![txt("on")], out)?;
    for (name, values) in fits {
        e.call(
            "semilogy",
            vec![
                row(fx.to_vec())?,
                row(values.clone())?,
                txt("-"),
                txt("DisplayName"),
                txt(name),
            ],
            out,
        )?;
    }
    e.call("hold", vec![txt("off")], out)?;
    label(e, out, "xlabel", "Eb/No (dB)")?;
    label(e, out, "ylabel", "BER")?;
    label(e, out, "title", "BER vs. Eb/No with Best Curve Fit")?;
    e.call("legend", vec![txt("show")], out)?;
    Ok(())
}
