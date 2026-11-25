//! Matrix-only shallow-network evaluation and graphics; no network object needed.
//! Training-record fields read: epoch, perf, vperf, tperf, best_epoch,
//! best_perf, best_vperf, gradient, mu, val_fail. All others are ignored.
//! Contracts: MathWorks public deeplearning/ref/{confusion,roc,plotconfusion,
//! plotroc,plotregression,ploterrhist,plotperform,plottrainstate}.html.
use super::{Engine, Execution, Value};
use crate::{Array, CellArray, Matrix, Result, StructArray};
use std::sync::Arc;

const NAMES: &[&str] = &[
    "plotsomtop",
    "plotsomnc",
    "plotsomnd",
    "plotsomplanes",
    "plotsomhits",
    "plotsompos",
    "confusion",
    "plotconfusion",
    "ploterrhist",
    "plotperform",
    "plotregression",
    "plotroc",
    "plottrainstate",
    "roc",
];
pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
pub(super) fn is_plot(name: &str) -> bool {
    handles(name) && name.starts_with("plot")
}
fn num(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}
fn text(x: &str) -> Value {
    Value::Text(x.into())
}
fn row(x: Vec<f64>) -> Value {
    Value::Matrix(Matrix::from_column_vec(1, x.len(), x).expect("bounded row"))
}
fn cell(e: &Engine, rows: usize, cols: usize, values: Vec<Value>) -> Result<Value> {
    let slots = values
        .into_iter()
        .map(|v| v.array(&e.cancel).map(Arc::new))
        .collect::<Result<_>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        rows, cols, slots, &e.cancel,
    )?)))
}
fn ratio(a: f64, b: f64) -> f64 {
    if b == 0. { f64::NAN } else { a / b }
}
fn finite(e: &Engine, m: &Matrix, name: &str) -> Result<()> {
    for chunk in m.data().chunks(4096) {
        e.check_cancel()?;
        if chunk.iter().any(|v| !v.is_finite()) {
            return Err(format!("{name}: data must be finite"));
        }
    }
    Ok(())
}
fn matrices(e: &Engine, t: &Value, y: &Value, name: &str) -> Result<(Matrix, Matrix)> {
    let (t, y) = (e.real_arg(name, t)?, e.real_arg(name, y)?);
    if t.is_nd() || y.is_nd() || t.is_empty() || t.rows() != y.rows() || t.cols() != y.cols() {
        return Err(format!(
            "{name}: targets and outputs must have equal, nonempty sizes"
        ));
    }
    finite(e, &t, name)?;
    finite(e, &y, name)?;
    Ok((t, y))
}
fn classification_matrices(
    e: &Engine,
    t: &Value,
    y: &Value,
    name: &str,
) -> Result<(Matrix, Matrix)> {
    let (t, y) = matrices(e, t, y, name)?;
    if t.rows() > 256 {
        return Err(format!("{name}: class count exceeds 256"));
    }
    for q in 0..t.cols() {
        if q.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let ts = &t.data()[q * t.rows()..(q + 1) * t.rows()];
        let ys = &y.data()[q * y.rows()..(q + 1) * y.rows()];
        if ts.iter().any(|&v| v != 0. && v != 1.) || (t.rows() > 1 && ts.iter().sum::<f64>() != 1.)
        {
            return Err(format!(
                "{name}: targets must be one-hot (or a 1-by-Q binary row)"
            ));
        }
        if ys.iter().any(|v| !(0. ..=1.).contains(v)) {
            return Err(format!("{name}: outputs must be in [0,1]"));
        }
    }
    Ok((t, y))
}
struct Classes {
    labels: Vec<String>,
    target: Vec<usize>,
    output: Vec<usize>,
}
fn classes(e: &Engine, t: &Value, y: &Value, categorical: bool, name: &str) -> Result<Classes> {
    if categorical
        && let (Value::Container(Array::Categorical(t)), Value::Container(Array::Categorical(y))) =
            (t, y)
    {
        if t.is_empty()
            || t.len() != y.len()
            || (t.rows() != 1 && t.cols() != 1)
            || (y.rows() != 1 && y.cols() != 1)
        {
            return Err(format!(
                "{name}: categorical labels must be vectors of equal nonzero length"
            ));
        }
        if (t.is_ordinal() || y.is_ordinal())
            && (t.is_ordinal() != y.is_ordinal() || t.categories() != y.categories())
        {
            return Err(format!("{name}: ordinal categories must match in order"));
        }
        if t.categories().len() > 256 || y.categories().len() > 256 {
            return Err(format!("{name}: class count exceeds 256"));
        }
        let mut labels = t.categories().as_ref().clone();
        for label in y.categories().iter() {
            if !labels.contains(label) {
                labels.push(label.clone());
            }
        }
        if !t.is_ordinal() {
            labels.sort();
        }
        if labels.len() > 256 {
            return Err(format!("{name}: class count exceeds 256"));
        }
        e.preflight_elements(t.len().saturating_mul(2))?;
        let mut target = Vec::with_capacity(t.len());
        let mut output = Vec::with_capacity(t.len());
        for i in 0..t.len() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            if t.codes()[i] == 0 || y.codes()[i] == 0 {
                return Err(format!(
                    "{name}: undefined categorical labels are unsupported"
                ));
            }
            target.push(
                labels
                    .iter()
                    .position(|s| s == t.text(i))
                    .ok_or("Missing target category")?,
            );
            output.push(
                labels
                    .iter()
                    .position(|s| s == y.text(i))
                    .ok_or("Missing output category")?,
            );
        }
        return Ok(Classes {
            labels,
            target,
            output,
        });
    }
    let (t, y) = classification_matrices(e, t, y, name)?;
    let n = t.rows().max(2);
    if n > 256 {
        return Err(format!("{name}: class count exceeds 256"));
    }
    e.preflight_elements(t.cols().saturating_mul(2))?;
    let mut target = Vec::with_capacity(t.cols());
    let mut output = Vec::with_capacity(t.cols());
    for q in 0..t.cols() {
        if q.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        if t.rows() == 1 {
            target.push(usize::from(t.data()[q] != 0.));
            output.push(usize::from(y.data()[q] >= 0.5));
        } else {
            let start = q * t.rows();
            target.push(
                t.data()[start..start + t.rows()]
                    .iter()
                    .position(|&v| v == 1.)
                    .ok_or("Missing target class")?,
            );
            // Strict comparison preserves the first class when scores tie.
            let mut best = 0;
            for i in 1..t.rows() {
                if y.data()[start + i] > y.data()[start + best] {
                    best = i;
                }
            }
            output.push(best);
        }
    }
    Ok(Classes {
        labels: (1..=n).map(|v| v.to_string()).collect(),
        target,
        output,
    })
}
fn counts(e: &Engine, c: &Classes) -> Result<Vec<f64>> {
    let n = c.labels.len();
    e.preflight_elements(n * n * 6 + c.target.len())?;
    let mut cm = vec![0.; n * n];
    for (i, (&t, &y)) in c.target.iter().zip(&c.output).enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        cm[t + y * n] += 1.;
    }
    Ok(cm)
}
fn confusion(e: &Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    let [t, y] = args else {
        return Err("confusion requires targets and outputs".into());
    };
    let c = classes(e, t, y, false, "confusion")?;
    let n = c.labels.len();
    // Include the two class vectors, growing index vectors and cell metadata.
    e.preflight_elements(
        c.target
            .len()
            .saturating_mul(if nargout >= 3 { 8 } else { 2 })
            .saturating_add(n * n * 10),
    )?;
    if nargout >= 3
        && c.target
            .len()
            .saturating_mul(8)
            .saturating_add(n * n * crate::containers::SLOT)
            > 256 * 1024 * 1024
    {
        return Err("confusion: index cells exceed the 256 MiB output limit".into());
    }
    let cm = counts(e, &c)?;
    let correct = (0..n).map(|i| cm[i + i * n]).sum::<f64>();
    let mut result = vec![num(1. - correct / c.target.len() as f64)];
    if nargout < 2 {
        return Ok(result);
    }
    result.push(Value::Matrix(Matrix::from_column_vec(n, n, cm.clone())?));
    if nargout < 3 {
        return Ok(result);
    }
    let mut indices = vec![Vec::new(); n * n];
    for (q, (&t, &y)) in c.target.iter().zip(&c.output).enumerate() {
        if q.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        indices[t + y * n].push((q + 1) as f64);
    }
    result.push(cell(
        e,
        n,
        n,
        indices
            .into_iter()
            .map(|v| {
                if v.is_empty() {
                    Value::Matrix(Matrix::filled(0, 0, 0.).expect("empty indices"))
                } else {
                    row(v)
                }
            })
            .collect(),
    )?);
    if nargout < 4 {
        return Ok(result);
    }
    let mut per = vec![0.; n * 4];
    for i in 0..n {
        let tp = cm[i + i * n];
        let predicted = (0..n).map(|j| cm[j + i * n]).sum::<f64>();
        let actual = (0..n).map(|j| cm[i + j * n]).sum::<f64>();
        let (fp, fn_) = (predicted - tp, actual - tp);
        let negatives = c.target.len() as f64 - predicted;
        per[i] = ratio(fn_, negatives);
        per[i + n] = ratio(fp, predicted);
        per[i + 2 * n] = ratio(tp, predicted);
        per[i + 3 * n] = ratio(negatives - fn_, negatives);
    }
    result.push(Value::Matrix(Matrix::from_column_vec(n, 4, per)?));
    Ok(result)
}
struct Curve {
    tpr: Vec<f64>,
    fpr: Vec<f64>,
    thresholds: Vec<f64>,
}
fn curves(e: &Engine, t: &Value, y: &Value, name: &str) -> Result<Vec<Curve>> {
    let (t, y) = classification_matrices(e, t, y, name)?;
    // Sort once per class, process tied scores together: O(S Q log Q), O(S Q) output.
    e.preflight_elements(t.len().saturating_mul(8).saturating_add(t.rows() * 6))?;
    Matrix::checked_len(1, t.cols().saturating_add(2))?;
    if t.rows() > 1
        && (t.len().saturating_add(2 * t.rows()))
            .saturating_mul(8)
            .saturating_add(t.rows() * crate::containers::SLOT)
            > 256 * 1024 * 1024
    {
        return Err(format!(
            "{name}: curve cells exceed the 256 MiB output limit"
        ));
    }
    if t.rows() > 256 {
        return Err(format!("{name}: class count exceeds 256"));
    }
    let mut result = Vec::with_capacity(t.rows());
    for i in 0..t.rows() {
        e.check_cancel()?;
        let mut scores = Vec::with_capacity(t.cols());
        let mut positives = 0.;
        for q in 0..t.cols() {
            if q.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let positive = t.data()[i + q * t.rows()] == 1.;
            positives += f64::from(positive);
            scores.push((y.data()[i + q * y.rows()], positive));
        }
        scores.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        e.check_cancel()?;
        let negatives = t.cols() as f64 - positives;
        let mut curve = Curve {
            tpr: Vec::new(),
            fpr: Vec::new(),
            thresholds: Vec::new(),
        };
        let mut k = 0;
        let (mut tp, mut fp) = (0., 0.);
        // Public contract: thresholds in [0,1], output >= threshold is positive.
        let mut threshold = 1.;
        loop {
            e.check_cancel()?;
            while k < scores.len() && scores[k].0 >= threshold {
                if k.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                if scores[k].1 {
                    tp += 1.;
                } else {
                    fp += 1.;
                }
                k += 1;
            }
            curve.thresholds.push(threshold);
            curve.tpr.push(ratio(tp, positives));
            curve.fpr.push(ratio(fp, negatives));
            if threshold == 0. {
                break;
            }
            threshold = if k < scores.len() { scores[k].0 } else { 0. };
        }
        result.push(curve);
    }
    Ok(result)
}
fn roc(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    let [t, y] = args else {
        return Err("roc requires targets and outputs".into());
    };
    let curves = curves(e, t, y, "roc")?;
    let n = curves.len();
    let (mut tp, mut fp, mut th) = (Vec::new(), Vec::new(), Vec::new());
    for c in curves {
        tp.push(row(c.tpr));
        fp.push(row(c.fpr));
        th.push(row(c.thresholds));
    }
    if n == 1 {
        Ok(vec![tp.remove(0), fp.remove(0), th.remove(0)])
    } else {
        Ok(vec![
            cell(e, 1, n, tp)?,
            cell(e, 1, n, fp)?,
            cell(e, 1, n, th)?,
        ])
    }
}

struct Pair<'a> {
    target: &'a Value,
    output: &'a Value,
    name: String,
}
fn pairs<'a>(e: &Engine, args: &'a [Value], name: &str) -> Result<Vec<Pair<'a>>> {
    if args.len() == 2 {
        return Ok(vec![Pair {
            target: &args[0],
            output: &args[1],
            name: String::new(),
        }]);
    }
    if args.is_empty() || !args.len().is_multiple_of(3) || args.len() > 48 {
        return Err(format!(
            "{name}: use targets,outputs or up to 16 targets,outputs,name triples"
        ));
    }
    args.chunks(3)
        .map(|a| {
            Ok(Pair {
                target: &a[0],
                output: &a[1],
                name: e.text_value(a[2].clone())?,
            })
        })
        .collect()
}
fn subplot(e: &mut Engine, out: &mut Execution, total: usize, index: usize) -> Result<()> {
    if total > 1 {
        let cols = (total as f64).sqrt().ceil() as usize;
        e.call(
            "subplot",
            vec![
                num(total.div_ceil(cols) as f64),
                num(cols as f64),
                num((index + 1) as f64),
            ],
            out,
        )?;
    }
    Ok(())
}
fn labels(e: &mut Engine, out: &mut Execution, title: &str, x: &str, y: &str) -> Result<()> {
    for (command, value) in [("title", title), ("xlabel", x), ("ylabel", y)] {
        e.call(command, vec![text(value)], out)?;
    }
    Ok(())
}
fn line(
    e: &mut Engine,
    out: &mut Execution,
    x: Vec<f64>,
    y: Vec<f64>,
    spec: &str,
    name: &str,
    log: bool,
) -> Result<Value> {
    e.call(
        if log { "semilogy" } else { "plot" },
        vec![row(x), row(y), text(spec), text("DisplayName"), text(name)],
        out,
    )
}
fn legend(
    e: &mut Engine,
    out: &mut Execution,
    handles: Vec<Value>,
    names: &[String],
) -> Result<()> {
    let h = e.call("horzcat", handles, out)?;
    let names = cell(e, 1, names.len(), names.iter().map(|s| text(s)).collect())?;
    e.call(
        "legend",
        vec![h, names, text("AutoUpdate"), text("off")],
        out,
    )?;
    Ok(())
}
fn patch(
    e: &mut Engine,
    out: &mut Execution,
    x: Vec<f64>,
    y: Vec<f64>,
    color: [f64; 3],
    name: &str,
    outline: bool,
) -> Result<Value> {
    let n = x.len() / 4;
    e.call(
        "patch",
        vec![
            text("XData"),
            Value::Matrix(Matrix::from_column_vec(4, n, x)?),
            text("YData"),
            Value::Matrix(Matrix::from_column_vec(4, n, y)?),
            text("FaceColor"),
            row(color.to_vec()),
            text("EdgeColor"),
            text(if outline { "black" } else { "none" }),
            text("DisplayName"),
            text(name),
        ],
        out,
    )
}
fn annotation(
    e: &mut Engine,
    out: &mut Execution,
    x: f64,
    y: f64,
    s: &str,
    color: [f64; 3],
) -> Result<()> {
    e.call(
        "text",
        vec![
            num(x),
            num(y),
            text(s),
            text("HorizontalAlignment"),
            text("center"),
            text("Color"),
            row(color.to_vec()),
        ],
        out,
    )?;
    Ok(())
}
fn plotconfusion(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let pairs = pairs(e, args, "plotconfusion")?;
    let mut data = Vec::new();
    let mut retained = 0usize;
    for p in &pairs {
        let samples = match p.target {
            Value::Container(Array::Categorical(c)) => c.len(),
            _ => p.target.shape(&e.cancel)?.1,
        };
        e.preflight_elements(retained.saturating_add(samples.saturating_mul(2)))?;
        let c = classes(e, p.target, p.output, true, "plotconfusion")?;
        // Two strings per grid/margin cell plus category labels fit 256 texts.
        if 2 * (c.labels.len() + 1).pow(2) + 2 * c.labels.len() > crate::plot::MAX_TEXTS {
            return Err("plotconfusion: class count exceeds the 256-text graphics limit".into());
        }
        e.preflight_elements(retained.saturating_add(
            c.target.capacity() + c.output.capacity() + c.labels.len().pow(2) * 6,
        ))?;
        let cm = counts(e, &c)?;
        retained += c.target.capacity() + c.output.capacity() + cm.capacity();
        data.push((c, cm));
    }
    let fig = e.call("figure", vec![], out)?;
    for (k, ((c, cm), p)) in data.iter().zip(&pairs).enumerate() {
        e.check_cancel()?;
        subplot(e, out, pairs.len(), k)?;
        e.call("hold", vec![text("on")], out)?;
        let n = c.labels.len();
        let total = c.target.len() as f64;
        let correct = (0..n).map(|i| cm[i + i * n]).sum::<f64>();
        // axis ij puts output class 1 at the top, keeping class coordinates intact.
        let (mut gx, mut gy, mut rx, mut ry, mut sx, mut sy) =
            (vec![], vec![], vec![], vec![], vec![], vec![]);
        for r in 0..=n {
            for col in 0..=n {
                let (x, y) = (col as f64 + 1., r as f64 + 1.);
                let (xs, ys) = if r == n || col == n {
                    (&mut sx, &mut sy)
                } else if r == col {
                    (&mut gx, &mut gy)
                } else {
                    (&mut rx, &mut ry)
                };
                xs.extend([x - 0.5, x + 0.5, x + 0.5, x - 0.5]);
                ys.extend([y - 0.5, y - 0.5, y + 0.5, y + 0.5]);
            }
        }
        patch(e, out, gx, gy, [0.5, 0.8, 0.5], "", true)?;
        if !rx.is_empty() {
            patch(e, out, rx, ry, [1., 0.6, 0.6], "", true)?;
        }
        patch(e, out, sx, sy, [0.85, 0.85, 0.85], "", true)?;
        for r in 0..=n {
            for col in 0..=n {
                let (x, y) = (col as f64 + 1., r as f64 + 1.);
                if r < n && col < n {
                    let count = cm[col + r * n];
                    annotation(e, out, x, y - 0.12, &format!("{count}"), [0., 0., 0.])?;
                    annotation(
                        e,
                        out,
                        x,
                        y + 0.15,
                        &format!("{:.1}%", 100. * count / total),
                        [0., 0., 0.],
                    )?;
                } else {
                    let accuracy = if r == n && col == n {
                        correct / total
                    } else if r == n {
                        ratio(cm[col + col * n], (0..n).map(|j| cm[col + j * n]).sum())
                    } else {
                        ratio(cm[r + r * n], (0..n).map(|j| cm[j + r * n]).sum())
                    };
                    annotation(
                        e,
                        out,
                        x,
                        y - 0.12,
                        &format!("{:.1}%", 100. * accuracy),
                        [0., 0.5, 0.],
                    )?;
                    annotation(
                        e,
                        out,
                        x,
                        y + 0.15,
                        &format!("{:.1}%", 100. * (1. - accuracy)),
                        [0.8, 0., 0.],
                    )?;
                }
            }
        }
        for (i, label) in c.labels.iter().enumerate() {
            annotation(e, out, i as f64 + 1., n as f64 + 1.75, label, [0., 0., 0.])?;
            annotation(e, out, 0.15, i as f64 + 1., label, [0., 0., 0.])?;
        }
        e.call(
            "axis",
            vec![row(vec![0., n as f64 + 1.5, 0.5, n as f64 + 2.])],
            out,
        )?;
        e.call("axis", vec![text("ij")], out)?;
        labels(
            e,
            out,
            &if p.name.is_empty() {
                "Confusion Matrix".into()
            } else {
                format!("{} Confusion Matrix", p.name)
            },
            "Target Class",
            "Output Class",
        )?;
        e.call("hold", vec![text("off")], out)?;
    }
    Ok(fig)
}
fn plotroc(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let pairs = pairs(e, args, "plotroc")?;
    let mut data = Vec::new();
    let mut retained = 0usize;
    for p in &pairs {
        let t = e.real_arg("plotroc", p.target)?;
        e.preflight_elements(
            retained
                .saturating_add(t.len().saturating_mul(8))
                .saturating_add(t.rows() * 6),
        )?;
        let cs = curves(e, p.target, p.output, "plotroc")?;
        retained += cs
            .iter()
            .map(|c| c.tpr.capacity() + c.fpr.capacity() + c.thresholds.capacity())
            .sum::<usize>();
        data.push(cs);
    }
    if data.iter().any(|c| {
        c.len() >= crate::plot::MAX_SERIES
            || c.iter().map(|v| v.tpr.len()).sum::<usize>() + 2 > crate::plot::MAX_POINTS
    }) {
        return Err("plotroc: curves exceed graphics limits".into());
    }
    let fig = e.call("figure", vec![], out)?;
    for (k, (cs, p)) in data.into_iter().zip(&pairs).enumerate() {
        subplot(e, out, pairs.len(), k)?;
        let mut hs = Vec::new();
        let mut names = Vec::new();
        for (i, c) in cs.into_iter().enumerate() {
            let name = format!("Class {}", i + 1);
            hs.push(line(e, out, c.fpr, c.tpr, "-", &name, false)?);
            names.push(name);
            e.call("hold", vec![text("on")], out)?;
        }
        line(e, out, vec![0., 1.], vec![0., 1.], "k--", "", false)?;
        legend(e, out, hs, &names)?;
        labels(
            e,
            out,
            &if p.name.is_empty() {
                "ROC".into()
            } else {
                format!("{} ROC", p.name)
            },
            "False Positive Rate",
            "True Positive Rate",
        )?;
        e.call("axis", vec![row(vec![0., 1., 0., 1.])], out)?;
        e.call("hold", vec![text("off")], out)?;
    }
    Ok(fig)
}
fn fit_label(value: f64) -> String {
    if value != 0. && !(1e-4..1e6).contains(&value.abs()) {
        format!("{value:.5e}")
    } else {
        format!("{value:.5}")
    }
}
fn plotregression(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let pairs = pairs(e, args, "plotregression")?;
    let mut data = Vec::new();
    for p in &pairs {
        let (t, y) = matrices(e, p.target, p.output, "plotregression")?;
        if t.len() < 2 || t.len() + 4 > crate::plot::MAX_POINTS {
            return Err("plotregression: requires 2 to 999996 observations".into());
        }
        e.preflight_elements(t.len() * 4)?;
        // Scale before centering to avoid overflow in means and covariance.
        let sx = t
            .data()
            .iter()
            .map(|v| v.abs())
            .fold(0., f64::max)
            .max(f64::MIN_POSITIVE);
        let sy = y
            .data()
            .iter()
            .map(|v| v.abs())
            .fold(0., f64::max)
            .max(f64::MIN_POSITIVE);
        let (mut mt, mut my, mut tt, mut yy, mut ty) = (0., 0., 0., 0., 0.);
        // Welford's centered covariance update, in normalized units.
        for (i, (&t, &y)) in t.data().iter().zip(y.data()).enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let (t, y) = (t / sx, y / sy);
            let (dt, dy) = (t - mt, y - my);
            mt += dt / (i + 1) as f64;
            my += dy / (i + 1) as f64;
            tt += dt * (t - mt);
            yy += dy * (y - my);
            ty += dt * (y - my);
        }
        if tt == 0. {
            return Err("plotregression: targets must have nonzero variance".into());
        }
        let slope = ty / tt;
        let a = (slope * sy) / sx;
        let b = sy * (my - slope * mt);
        let r = ratio(ty, tt.sqrt() * yy.sqrt());
        if !a.is_finite() || !b.is_finite() {
            return Err("plotregression: numerical range exceeds finite fit limits".into());
        }
        let lo = t.data().iter().copied().fold(f64::INFINITY, f64::min);
        let hi = t.data().iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let fitted = vec![
            sy * (my + slope * (lo / sx - mt)),
            sy * (my + slope * (hi / sx - mt)),
        ];
        if fitted.iter().any(|v| !v.is_finite()) {
            return Err("plotregression: fitted values exceed finite limits".into());
        }
        data.push((t, y, a, b, r, lo, hi, fitted));
    }
    e.preflight_elements(data.iter().map(|(t, ..)| t.len() * 4).sum())?;
    let fig = e.call("figure", vec![], out)?;
    for (k, ((t, y, a, b, r, lo, hi, fitted), p)) in data.into_iter().zip(&pairs).enumerate() {
        subplot(e, out, pairs.len(), k)?;
        let d = line(
            e,
            out,
            t.data().to_vec(),
            y.data().to_vec(),
            "ko",
            "Data",
            false,
        )?;
        e.call("hold", vec![text("on")], out)?;
        let fit = line(e, out, vec![lo, hi], fitted, "b-", "Fit", false)?;
        let identity = line(e, out, vec![lo, hi], vec![lo, hi], "k--", "Y = T", false)?;
        legend(
            e,
            out,
            vec![d, fit, identity],
            &["Data".into(), "Fit".into(), "Y = T".into()],
        )?;
        let title = if p.name.is_empty() {
            format!("R={r:.5}")
        } else {
            format!("{}: R={r:.5}", p.name)
        };
        labels(
            e,
            out,
            &title,
            "Target",
            &format!("Output ~= {}*Target + {}", fit_label(a), fit_label(b)),
        )?;
        e.call("hold", vec![text("off")], out)?;
    }
    Ok(fig)
}
fn ploterrhist(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let mut end = args.len();
    let mut bins = 20;
    if end >= 2 && matches!(&args[end-2],Value::Text(s) if s.eq_ignore_ascii_case("bins")) {
        let b = e.real_arg("ploterrhist", &args[end - 1])?.as_scalar()?;
        if !b.is_finite() || b < 1. || b.fract() != 0. || b > 10000. {
            return Err("ploterrhist: bins must be an integer in 1..10000".into());
        }
        bins = b as usize;
        end -= 2;
    }
    if end == 0 || (end > 1 && (!end.is_multiple_of(2) || end > 32)) {
        return Err("ploterrhist: use errors or errors,name pairs and optional 'bins',n".into());
    }
    let mut data = Vec::new();
    let mut names = Vec::new();
    for a in args[..end].chunks(if end == 1 { 1 } else { 2 }) {
        let m = e.real_arg("ploterrhist", &a[0])?;
        if m.is_empty() {
            return Err("ploterrhist: errors must be nonempty".into());
        }
        finite(e, &m, "ploterrhist")?;
        data.push(m);
        names.push(if a.len() == 1 {
            "Errors".into()
        } else {
            e.text_value(a[1].clone())?
        });
    }
    e.preflight_elements(bins * (10 * data.len() + 2))?;
    if bins * data.len() > crate::plot::MAX_PATCH_FACES {
        return Err("ploterrhist: histogram exceeds the graphics face limit".into());
    }
    let lo = data
        .iter()
        .flat_map(|m| m.data())
        .copied()
        .fold(f64::INFINITY, f64::min);
    let hi = data
        .iter()
        .flat_map(|m| m.data())
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let (edge, width) = if hi > lo {
        (lo, (hi - lo) / bins as f64)
    } else {
        (lo - 0.5, 1. / bins as f64)
    };
    if !edge.is_finite() || !width.is_finite() || width == 0. {
        return Err("ploterrhist: numerical range exceeds bin limits".into());
    }
    let mut counts = Vec::new();
    for m in &data {
        let mut c = vec![0.; bins];
        for (i, &v) in m.data().iter().enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            c[(((v - edge) / width).floor() as usize).min(bins - 1)] += 1.;
        }
        counts.push(c);
    }
    let fig = e.call("figure", vec![], out)?;
    e.call("hold", vec![text("on")], out)?;
    let mut bottom = vec![0.; bins];
    let mut handles = Vec::new();
    let colors = [
        [0., 0., 1.],
        [0., 0.7, 0.],
        [1., 0., 0.],
        [0., 0.7, 0.7],
        [0.7, 0., 0.7],
    ];
    for (k, c) in counts.iter().enumerate() {
        let (mut x, mut y) = (Vec::with_capacity(bins * 4), Vec::with_capacity(bins * 4));
        for j in 0..bins {
            let l = edge + j as f64 * width;
            let r = l + width;
            let top = bottom[j] + c[j];
            x.extend([l, r, r, l]);
            y.extend([bottom[j], bottom[j], top, top]);
            bottom[j] = top;
        }
        handles.push(patch(
            e,
            out,
            x,
            y,
            colors[k % colors.len()],
            &names[k],
            false,
        )?);
    }
    let top = bottom.into_iter().fold(1., f64::max);
    handles.push(line(
        e,
        out,
        vec![0., 0.],
        vec![0., top],
        "y-",
        "Zero Error",
        false,
    )?);
    names.push("Zero Error".into());
    legend(e, out, handles, &names)?;
    labels(
        e,
        out,
        &format!("Error Histogram with {bins} Bins"),
        "Errors = Targets - Outputs",
        "Instances",
    )?;
    e.call("hold", vec![text("off")], out)?;
    Ok(fig)
}
fn record<'a>(args: &'a [Value], name: &str) -> Result<&'a StructArray> {
    let [Value::Container(Array::Struct(s))] = args else {
        return Err(format!("{name}: requires a scalar training-record struct"));
    };
    if s.len() != 1 {
        return Err(format!("{name}: requires a scalar training-record struct"));
    }
    Ok(s)
}
fn field(e: &Engine, s: &StructArray, name: &str) -> Result<Option<Matrix>> {
    s.field_index(name)
        .map(|i| e.real_arg(name, &Value::from(s.get(i, 0)?.as_ref().clone())))
        .transpose()
}
fn required(e: &Engine, s: &StructArray, name: &str) -> Result<Matrix> {
    field(e, s, name)?.ok_or_else(|| format!("training record: missing {name}"))
}
fn epochs(e: &Engine, s: &StructArray) -> Result<Matrix> {
    let m = required(e, s, "epoch")?;
    finite(e, &m, "epoch")?;
    if m.is_empty()
        || (m.rows() != 1 && m.cols() != 1)
        || m.len() + 4 > crate::plot::MAX_POINTS
        || m.data().iter().any(|v| *v < 0. || v.fract() != 0.)
        || m.data().windows(2).any(|w| w[0] >= w[1])
    {
        return Err("training record: epoch must be a nonempty increasing vector of nonnegative integers within graphics limits".into());
    }
    Ok(m)
}
fn series_field(
    e: &Engine,
    s: &StructArray,
    key: &str,
    epoch: &Matrix,
    required: bool,
) -> Result<Option<Matrix>> {
    let m = field(e, s, key)?;
    let Some(m) = m.filter(|m| !m.is_empty()) else {
        return if required {
            Err(format!("training record: missing or empty {key}"))
        } else {
            Ok(None)
        };
    };
    if m.len() != epoch.len() || (m.rows() != 1 && m.cols() != 1) {
        return Err(format!(
            "training record: {key} must have one value per epoch"
        ));
    }
    for chunk in m.data().chunks(4096) {
        e.check_cancel()?;
        if chunk.iter().any(|v| v.is_infinite() || *v < 0.) {
            return Err(format!(
                "training record: {key} must be nonnegative, finite or NaN"
            ));
        }
    }
    Ok(Some(m))
}
fn plotperform(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let s = record(args, "plotperform")?;
    let epoch = epochs(e, s)?;
    let mut data = Vec::new();
    let mut names = Vec::new();
    for (key, name) in [
        ("perf", "Train"),
        ("vperf", "Validation"),
        ("tperf", "Test"),
    ] {
        if let Some(m) = series_field(e, s, key, &epoch, key == "perf")? {
            data.push(m);
            names.push(name.to_string());
        }
    }
    if data.len() * epoch.len() + 1 > crate::plot::MAX_POINTS {
        return Err("plotperform: curves exceed graphics point limit".into());
    }
    e.preflight_elements(epoch.len() * data.len() * 4)?;
    let best_epoch = field(e, s, "best_epoch")?
        .map(|m| m.as_scalar())
        .transpose()?
        .unwrap_or(*epoch.data().last().unwrap());
    if !epoch.data().contains(&best_epoch) {
        return Err("plotperform: best_epoch must be in epoch".into());
    }
    let validation = names.iter().position(|n| n == "Validation");
    let best_field = if validation.is_some() {
        "best_vperf"
    } else {
        "best_perf"
    };
    let best = field(e, s, best_field)?
        .map(|m| m.as_scalar())
        .transpose()?
        .unwrap_or(
            data[validation.unwrap_or(0)].data()
                [epoch.data().iter().position(|&x| x == best_epoch).unwrap()],
        );
    if !best.is_finite() || best < 0. {
        return Err("plotperform: best performance must be finite and nonnegative".into());
    }
    let fig = e.call("figure", vec![], out)?;
    let mut hs = Vec::new();
    for (m, name) in data.iter().zip(&names) {
        hs.push(line(
            e,
            out,
            epoch.data().to_vec(),
            m.data().to_vec(),
            match name.as_str() {
                "Train" => "b-",
                "Validation" => "g-",
                _ => "r-",
            },
            name,
            true,
        )?);
        e.call("hold", vec![text("on")], out)?;
    }
    hs.push(line(
        e,
        out,
        vec![best_epoch],
        vec![best],
        "ko",
        "Best",
        true,
    )?);
    names.push("Best".into());
    legend(e, out, hs, &names)?;
    labels(
        e,
        out,
        &format!(
            "Best {}Performance is {best} at epoch {best_epoch}",
            if validation.is_some() {
                "Validation "
            } else {
                "Training "
            }
        ),
        &format!("{} Epochs", epoch.data().last().unwrap()),
        "Mean Squared Error (mse)",
    )?;
    e.call("hold", vec![text("off")], out)?;
    Ok(fig)
}
fn plottrainstate(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let s = record(args, "plottrainstate")?;
    let epoch = epochs(e, s)?;
    let mut data = Vec::new();
    for (key, title, log) in [
        ("gradient", "Gradient", true),
        ("mu", "Mu", true),
        ("val_fail", "Validation Checks", false),
    ] {
        if let Some(m) = series_field(e, s, key, &epoch, key != "mu")? {
            data.push((key, title, log, m));
        }
    }
    if data.len() * epoch.len() > crate::plot::MAX_POINTS {
        return Err("plottrainstate: curves exceed graphics point limit".into());
    }
    e.preflight_elements(epoch.len() * data.len() * 4)?;
    let fig = e.call("figure", vec![], out)?;
    for (k, (key, title, log, m)) in data.iter().enumerate() {
        e.call(
            "subplot",
            vec![num(data.len() as f64), num(1.), num((k + 1) as f64)],
            out,
        )?;
        line(
            e,
            out,
            epoch.data().to_vec(),
            m.data().to_vec(),
            if *log { "b-" } else { "bo" },
            title,
            *log,
        )?;
        labels(
            e,
            out,
            &format!(
                "{title} = {}, at epoch {}",
                m.data().last().unwrap(),
                epoch.data().last().unwrap()
            ),
            &if k + 1 == data.len() {
                format!("{} Epochs", epoch.data().last().unwrap())
            } else {
                String::new()
            },
            if *key == "val_fail" { "val fail" } else { key },
        )?;
    }
    Ok(fig)
}
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let max = match name {
        "confusion" => 4,
        "roc" => 3,
        _ => 1,
    };
    if nargout > max {
        return Err("Too many output arguments.".into());
    }
    e.check_cancel()?;
    match name {
        "plotsomtop" | "plotsomnc" | "plotsomnd" | "plotsomplanes" | "plotsomhits"
        | "plotsompos" => Ok(vec![super::nnet::som_plots::call(e, name, args, out)?]),
        "confusion" => confusion(e, args, nargout),
        "roc" => roc(e, args),
        "plotconfusion" => Ok(vec![plotconfusion(e, args, out)?]),
        "plotroc" => Ok(vec![plotroc(e, args, out)?]),
        "plotregression" => Ok(vec![plotregression(e, args, out)?]),
        "ploterrhist" => Ok(vec![ploterrhist(e, args, out)?]),
        "plotperform" => Ok(vec![plotperform(e, args, out)?]),
        "plottrainstate" => Ok(vec![plottrainstate(e, args, out)?]),
        _ => Err(format!("Unsupported network plot {name}")),
    }
}
