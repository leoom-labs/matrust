//! Classifier fitting front end; all model state is immutable.
use super::super::classifier_objects::{
    Classifier, FitOptions, Kind, Labels, Node, predictor_matrix,
};
use super::*;
use crate::{Array, Matrix};
use std::sync::Arc;

pub(in crate::engine) fn labels(e: &Engine, v: &Value, name: &str) -> Result<Labels> {
    if !v.is_character() && (v.is_nd() || (v.dims()[0] != 1 && v.dims()[1] != 1)) {
        return Err(format!("{name}: response must be a vector"));
    }
    match v {
        Value::Matrix(m) => {
            if m.data().iter().any(|v| v.is_infinite()) {
                return Err(format!("{name}: responses must be finite"));
            }
            if m.rows() != 1 && m.cols() != 1 {
                return Err(format!("{name}: response must be a vector"));
            }
            Ok(Labels::Numeric(m.data().to_vec()))
        }
        Value::Logical(m) => {
            if m.rows() != 1 && m.cols() != 1 {
                return Err(format!("{name}: response must be a vector"));
            }
            Ok(Labels::Logical(m.data().iter().map(|&x| x != 0).collect()))
        }
        Value::Text(s) => Ok(Labels::Text(vec![s.clone()])),
        Value::QuotedString(s) => Ok(Labels::String(vec![s.clone()])),
        Value::Container(Array::Str(s)) if s.data().iter().any(Option::is_none) => {
            Err(format!("{name}: missing response labels are not supported"))
        }
        Value::Container(Array::Str(s)) => Ok(Labels::String(
            s.data()
                .iter()
                .map(|x| x.as_ref().map_or(String::new(), |x| x.to_string()))
                .collect(),
        )),
        Value::Container(Array::Character(c)) => Ok(Labels::Text(
            (0..c.rows())
                .map(|r| {
                    String::from_utf16_lossy(
                        &(0..c.cols())
                            .map(|j| c.units()[r + j * c.rows()])
                            .collect::<Vec<_>>(),
                    )
                    .trim_end()
                    .into()
                })
                .collect(),
        )),
        Value::Container(Array::Cell(c)) => {
            let mut z = Vec::new();
            for i in 0..c.len() {
                match c.get_linear(i)?.as_ref() {
                    Array::Character(x) => z.push(x.to_utf8(&e.cancel)?),
                    Array::Str(x) if x.len() == 1 => z.push(
                        x.data()[0]
                            .as_ref()
                            .map_or(String::new(), |x| x.to_string()),
                    ),
                    _ => return Err(format!("{name}: cell response must contain text")),
                }
            }
            Ok(Labels::Text(z))
        }
        Value::Container(Array::Categorical(c))
            if (0..c.len()).any(|i| c.text(i) == "<undefined>") =>
        {
            Err(format!(
                "{name}: undefined response labels are not supported"
            ))
        }
        Value::Container(Array::Categorical(c)) => Ok(Labels::Categorical(
            (0..c.len()).map(|i| c.text(i).to_string()).collect(),
            c.categories().clone(),
            c.is_ordinal(),
        )),
        Value::Container(Array::Typed(v)) => {
            if (0..v.len()).any(|i| v.real(i).is_infinite()) {
                return Err(format!("{name}: responses must be finite"));
            }
            Ok(Labels::Typed(v.clone()))
        }
        _ => Err(format!("{name}: unsupported response type")),
    }
}
fn opt_text(e: &Engine, v: &Value, name: &str) -> Result<String> {
    e.text_value(v.clone())
        .map(|s| s.to_ascii_lowercase())
        .map_err(|_| format!("{name}: expected text option"))
}
pub(super) fn fit(e: &mut Engine, name: &str, a: &[Value]) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err(format!("{name}: expected predictors and response"));
    }
    let (x, names, response, y) = if let Value::Container(Array::Table(t)) = &a[0] {
        if a[1].is_character() || matches!(&a[1], Value::QuotedString(_) | Value::Text(_)) {
            let response = e.text_value(a[1].clone())?;
            let index = t
                .names()
                .iter()
                .position(|n| n == &response)
                .ok_or_else(|| format!("{name}: unknown response variable '{response}'"))?;
            let names: Vec<_> = t
                .names()
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != index)
                .map(|(_, n)| n.clone())
                .collect();
            let (x, _) = predictor_matrix(e, &a[0], Some(&names), name)?;
            let y = labels(e, &Value::from(t.column(index).clone()), name)?;
            (x, names, response, y)
        } else {
            let (x, names) = predictor_matrix(e, &a[0], None, name)?;
            (x, names, "Y".into(), labels(e, &a[1], name)?)
        }
    } else {
        let (x, names) = predictor_matrix(e, &a[0], None, name)?;
        (x, names, "Y".into(), labels(e, &a[1], name)?)
    };
    if matches!(&y, Labels::Numeric(v) if v.iter().any(|v| v.is_nan()))
        || matches!(&y,Labels::Typed(v) if (0..v.len()).any(|i|v.real(i).is_nan()))
    {
        return Err(format!("{name}: responses must be finite"));
    }
    let allowed: &[&str] = match name {
        "fitcknn" => &["numneighbors", "distance", "standardize"],
        "fitctree" => &[
            "maxnumsplits",
            "minleafsize",
            "minparentsize",
            "splitcriterion",
        ],
        "fitcsvm" => &[
            "boxconstraint",
            "kernelfunction",
            "kernelscale",
            "standardize",
        ],
        "fitcnb" => &["distributionnames", "prior"],
        _ => return Err(format!("{name}: unsupported classifier")),
    };
    let mut neighbors = 1;
    let mut metric = "euclidean".to_string();
    let mut standardize = false;
    let mut max_splits = x.rows().saturating_sub(1);
    let mut min_leaf = 1;
    let mut min_parent = 10;
    let mut c = 1.;
    let mut kernel = "linear".to_string();
    let mut scale = 1.;
    for (k, v) in options(name, &a[2..], allowed)? {
        match k.as_str() {
            "numneighbors" => neighbors = integer(e, name, v)?,
            "distance" => metric = opt_text(e, v, name)?,
            "standardize" => standardize = boolean(e, name, v)?,
            "maxnumsplits" => {
                let z = scalar(e, name, v)?;
                if z < 0. || z.fract() != 0. || z > 20_000. {
                    return Err(format!(
                        "{name}: MaxNumSplits must be an integer from 0 to 20000"
                    ));
                }
                max_splits = z as usize;
            }
            "minleafsize" => min_leaf = integer(e, name, v)?,
            "minparentsize" => {
                min_parent = integer(e, name, v)?;
                if min_parent < 2 {
                    return Err(format!("{name}: MinParentSize must be at least 2"));
                }
            }
            "splitcriterion" => {
                if opt_text(e, v, name)? != "gdi" {
                    return Err(format!("{name}: only SplitCriterion 'gdi' is supported"));
                }
            }
            "boxconstraint" => {
                c = scalar(e, name, v)?;
                if c <= 0. {
                    return Err(format!("{name}: BoxConstraint must be positive"));
                }
            }
            "kernelfunction" => kernel = opt_text(e, v, name)?,
            "kernelscale" => {
                scale = scalar(e, name, v)?;
                if scale <= 0. || !(scale * scale).is_finite() || scale * scale == 0. {
                    return Err(format!(
                        "{name}: KernelScale must have a finite positive square"
                    ));
                }
            }
            "distributionnames" => {
                if !["normal", "gaussian"].contains(&opt_text(e, v, name)?.as_str()) {
                    return Err(format!(
                        "{name}: only Gaussian ('normal') distributions are supported; kernel and mn are not supported yet"
                    ));
                }
            }
            "prior" => {
                if opt_text(e, v, name)? != "empirical" {
                    return Err(format!("{name}: only Prior 'empirical' is supported"));
                }
            }
            _ => unreachable!(),
        }
    }
    let options = match name {
        "fitcknn" => {
            if !["euclidean", "cityblock", "chebychev", "minkowski"].contains(&metric.as_str()) {
                return Err("fitcknn: unsupported Distance".into());
            }
            FitOptions::Knn {
                neighbors,
                metric,
                standardize,
            }
        }
        "fitctree" => FitOptions::Tree {
            max_splits,
            min_leaf,
            min_parent,
        },
        "fitcnb" => FitOptions::Nb,
        "fitcsvm" => {
            if !["linear", "rbf", "gaussian"].contains(&kernel.as_str()) {
                return Err("fitcsvm: only linear, rbf and gaussian kernels are supported".into());
            }
            FitOptions::Svm {
                c,
                kernel: if kernel == "gaussian" {
                    "rbf".into()
                } else {
                    kernel
                },
                scale,
                standardize,
            }
        }
        _ => unreachable!(),
    };
    let model = train(e, x, y, names, response, options)?;
    e.classifier_new(model).map(|v| vec![v])
}

pub(in crate::engine) fn train(
    e: &mut Engine,
    x: Matrix,
    y: Labels,
    names: Vec<String>,
    response: String,
    options: FitOptions,
) -> Result<Classifier> {
    let (n, p) = (x.rows(), x.cols());
    if n == 0 || p == 0 || x.data().iter().any(|x| !x.is_finite()) {
        return Err("fitc: predictors must be a nonempty finite real matrix".into());
    }
    if n != y.len() {
        return Err("fitc: X and Y must have the same number of observations".into());
    }
    if n > 20_000 || p > 512 || x.len() > 2_000_000 {
        return Err(
            "fitc: training limit is 20000 rows, 512 predictors and 2000000 elements".into(),
        );
    }
    let (class_rows, codes) = y.class_indices();
    let classes = class_rows.len();
    if classes > 256 {
        return Err("fitc: at most 256 response classes are supported".into());
    }
    e.preflight_elements(x.len().saturating_mul(2).saturating_add(n * classes * 2))?;
    let standardize = matches!(
        options,
        FitOptions::Knn {
            standardize: true,
            ..
        } | FitOptions::Svm {
            standardize: true,
            ..
        }
    );
    let mut means = vec![0.; p];
    let mut scales = vec![1.; p];
    if standardize {
        for j in 0..p {
            e.check_cancel()?;
            means[j] = (0..n).map(|i| x.data()[i + j * n]).sum::<f64>() / n as f64;
            scales[j] = ((0..n)
                .map(|i| (x.data()[i + j * n] - means[j]).powi(2))
                .sum::<f64>()
                / n.saturating_sub(1).max(1) as f64)
                .sqrt();
            if scales[j] == 0. {
                scales[j] = 1.;
            }
            if !means[j].is_finite() || !scales[j].is_finite() {
                return Err("fitc: numerical overflow during standardization".into());
            }
        }
    }
    let standardized = if standardize {
        let data = (0..p)
            .flat_map(|j| {
                (0..n).map({
                    let x = &x;
                    let means = &means;
                    let scales = &scales;
                    move |i| (x.data()[i + j * n] - means[j]) / scales[j]
                })
            })
            .collect();
        Matrix::from_column_vec(n, p, data)?
    } else {
        x.clone()
    };
    let kind = match &options {
        FitOptions::Advanced(config) => {
            Kind::Advanced(Arc::new(super::ml::train(e, &x, &codes, classes, config)?))
        }
        FitOptions::Knn {
            neighbors, metric, ..
        } => {
            if *neighbors > n {
                return Err("fitcknn: NumNeighbors cannot exceed observations".into());
            }
            Kind::Knn {
                neighbors: *neighbors,
                metric: metric.clone(),
            }
        }
        FitOptions::Nb => {
            let mut meansc = vec![vec![0.; p]; classes];
            let mut vars = vec![vec![0.; p]; classes];
            let mut counts = vec![0usize; classes];
            // Welford updates give an unbiased per-class sample variance.
            for (i, &k) in codes.iter().enumerate() {
                if i.is_multiple_of(256) {
                    e.check_cancel()?;
                }
                counts[k] += 1;
                for j in 0..p {
                    let z = x.data()[i + j * n];
                    let d = z - meansc[k][j];
                    meansc[k][j] += d / counts[k] as f64;
                    vars[k][j] += d * (z - meansc[k][j]);
                }
            }
            for k in 0..classes {
                for j in 0..p {
                    vars[k][j] =
                        (vars[k][j] / counts[k].saturating_sub(1).max(1) as f64).max(1e-12);
                    if !meansc[k][j].is_finite() || !vars[k][j].is_finite() {
                        return Err("fitcnb: numerical overflow in Gaussian parameters".into());
                    }
                }
            }
            Kind::Nb {
                means: meansc,
                vars,
                priors: counts.iter().map(|&n_k| n_k as f64 / n as f64).collect(),
            }
        }
        FitOptions::Tree {
            max_splits,
            min_leaf,
            min_parent,
        } => Kind::Tree {
            nodes: cart(
                e,
                &standardized,
                &codes,
                classes,
                *max_splits,
                *min_leaf,
                *min_parent,
            )?,
        },
        FitOptions::Svm {
            c, kernel, scale, ..
        } => {
            if classes != 2 {
                return Err("fitcsvm: exactly two classes are required; use fitcecoc for multiclass classification".into());
            }
            let (alpha, bias) = smo(e, &standardized, &codes, *c, kernel, *scale)?;
            Kind::Svm {
                alpha,
                bias,
                kernel: kernel.clone(),
                scale: *scale,
            }
        }
    };
    Ok(Classifier {
        kind,
        options,
        x,
        y,
        codes,
        class_rows,
        names,
        response,
        means,
        scales,
    })
}

struct Split {
    node: usize,
    ids: Vec<usize>,
    feature: usize,
    threshold: f64,
    gain: f64,
}
fn leaf(codes: &[usize], ids: &[usize], classes: usize) -> Node {
    let mut scores = vec![0.; classes];
    for &i in ids {
        scores[codes[i]] += 1. / ids.len() as f64;
    }
    Node {
        split: None,
        scores,
    }
}
struct TreeSearch<'a> {
    x: &'a Matrix,
    codes: &'a [usize],
    classes: usize,
    min_leaf: usize,
    min_parent: usize,
}
fn best_split(
    e: &Engine,
    search: &TreeSearch<'_>,
    node: usize,
    ids: Vec<usize>,
    work: &mut usize,
) -> Result<Option<Split>> {
    let TreeSearch {
        x,
        codes,
        classes,
        min_leaf,
        min_parent,
    } = *search;
    if ids.len() < min_parent || ids.len() < min_leaf.saturating_mul(2) {
        return Ok(None);
    }
    let mut total = vec![0usize; classes];
    for &i in &ids {
        total[codes[i]] += 1;
    }
    if total.iter().filter(|&&n| n > 0).count() < 2 {
        return Ok(None);
    }
    let parent = total.iter().map(|&n| (n as f64).powi(2)).sum::<f64>() / ids.len() as f64;
    let mut best: Option<(f64, usize, f64)> = None;
    for j in 0..x.cols() {
        e.check_cancel()?;
        *work = work.saturating_add(ids.len());
        if *work > 50_000_000 {
            return Err("fitctree: split search work limit exceeded".into());
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable_by(|&a, &b| {
            x.data()[a + j * x.rows()]
                .total_cmp(&x.data()[b + j * x.rows()])
                .then(a.cmp(&b))
        });
        let mut left = vec![0usize; classes];
        let mut right = total.clone();
        let mut left_sq = 0.;
        let mut right_sq = total.iter().map(|&n| (n as f64).powi(2)).sum::<f64>();
        for k in 1..sorted.len() {
            let label = codes[sorted[k - 1]];
            left_sq += 2. * left[label] as f64 + 1.;
            left[label] += 1;
            right_sq += 1. - 2. * right[label] as f64;
            right[label] -= 1;
            let a = x.data()[sorted[k - 1] + j * x.rows()];
            let b = x.data()[sorted[k] + j * x.rows()];
            if k < min_leaf || sorted.len() - k < min_leaf || a == b {
                continue;
            }
            let gain = left_sq / k as f64 + right_sq / (sorted.len() - k) as f64 - parent;
            if best.is_none_or(|(g, _, _)| gain > g + 1e-12) {
                let midpoint = a * 0.5 + b * 0.5;
                best = Some((gain, j, if midpoint < b { midpoint } else { a }));
            }
        }
    }
    Ok(best.map(|(gain, feature, threshold)| Split {
        node,
        ids,
        feature,
        threshold,
        gain,
    }))
}
fn cart(
    e: &Engine,
    x: &Matrix,
    codes: &[usize],
    classes: usize,
    max_splits: usize,
    min_leaf: usize,
    min_parent: usize,
) -> Result<Vec<Node>> {
    let ids: Vec<_> = (0..x.rows()).collect();
    let mut nodes = vec![leaf(codes, &ids, classes)];
    if max_splits == 0 {
        return Ok(nodes);
    }
    let search = TreeSearch {
        x,
        codes,
        classes,
        min_leaf,
        min_parent,
    };
    let mut work = 0;
    let mut pending = Vec::new();
    if let Some(s) = best_split(e, &search, 0, ids, &mut work)? {
        pending.push(s);
    }
    for count in 0..max_splits {
        e.check_cancel()?;
        let Some(best) = (0..pending.len()).max_by(|&a, &b| {
            pending[a]
                .gain
                .total_cmp(&pending[b].gain)
                .then_with(|| pending[b].node.cmp(&pending[a].node))
        }) else {
            break;
        };
        let s = pending.swap_remove(best);
        let (left, right): (Vec<_>, Vec<_>) = s
            .ids
            .into_iter()
            .partition(|&i| x.data()[i + s.feature * x.rows()] <= s.threshold);
        let l = nodes.len();
        let r = l + 1;
        nodes[s.node].split = Some((s.feature, s.threshold, l, r));
        nodes.push(leaf(codes, &left, classes));
        nodes.push(leaf(codes, &right, classes));
        if count + 1 < max_splits {
            if let Some(s) = best_split(e, &search, l, left, &mut work)? {
                pending.push(s);
            }
            if let Some(s) = best_split(e, &search, r, right, &mut work)? {
                pending.push(s);
            }
        }
    }
    Ok(nodes)
}

// Two-variable SMO on the C-SVC dual. Signed pair steps preserve sum(y*alpha)=0.
// The working-set KKT gap provides an explicit convergence check; a bounded run
// that cannot converge errors rather than returning an unfinished model.
pub(super) fn smo(
    e: &Engine,
    x: &Matrix,
    codes: &[usize],
    c: f64,
    kernel: &str,
    scale: f64,
) -> Result<(Vec<f64>, f64)> {
    let n = x.rows();
    if n > 2048 || (n as u128) * (n as u128) * x.cols() as u128 > 100_000_000 {
        return Err(
            "fitcsvm: SMO limit is 2048 observations and 100000000 kernel element products".into(),
        );
    }
    e.preflight_elements(Matrix::checked_len(n, n)?.saturating_add(n * 8))?;
    let mut gram = vec![0.; n * n];
    for i in 0..n {
        e.check_cancel()?;
        for j in 0..=i {
            let mut v = 0.;
            for k in 0..x.cols() {
                let a = x.data()[i + k * n];
                let b = x.data()[j + k * n];
                v += if kernel == "linear" {
                    (a / scale) * (b / scale)
                } else {
                    (a - b).powi(2)
                };
            }
            if kernel != "linear" {
                v = (-v / (scale * scale)).exp();
            }
            if !v.is_finite() {
                return Err("fitcsvm: numerical overflow in kernel; use Standardize".into());
            }
            gram[i * n + j] = v;
            gram[j * n + i] = v;
        }
    }
    let y: Vec<_> = codes
        .iter()
        .map(|&k| if k == 1 { 1. } else { -1. })
        .collect();
    let mut alpha = vec![0.; n];
    let mut gradient = vec![0.0f64; n]; // sum(alpha*y*K), without bias
    for step in 0..200_000 {
        if step % 64 == 0 {
            e.check_cancel()?;
        }
        let f = |i: usize| y[i] - gradient[i];
        let up = |i: usize| {
            if y[i] > 0. {
                alpha[i] < c
            } else {
                alpha[i] > 0.
            }
        };
        let low = |i: usize| {
            if y[i] > 0. {
                alpha[i] > 0.
            } else {
                alpha[i] < c
            }
        };
        let i = (0..n)
            .filter(|&i| up(i))
            .max_by(|&a, &b| f(a).total_cmp(&f(b)))
            .unwrap();
        let min_low = (0..n)
            .filter(|&j| low(j))
            .map(f)
            .fold(f64::INFINITY, f64::min);
        if f(i) - min_low <= 1e-7 {
            let bias = (f(i) + min_low) * 0.5;
            return Ok((alpha, bias));
        }
        let cap_i = if y[i] > 0. { c - alpha[i] } else { alpha[i] };
        let mut best = None;
        for j in 0..n {
            if j == i || !low(j) {
                continue;
            }
            let gap = f(i) - f(j);
            if gap <= 0. {
                continue;
            }
            let cap_j = if y[j] > 0. { alpha[j] } else { c - alpha[j] };
            let eta = (gram[i * n + i] + gram[j * n + j] - 2. * gram[i * n + j]).max(0.);
            let t = if eta > 0. {
                (gap / eta).min(cap_i).min(cap_j)
            } else {
                cap_i.min(cap_j)
            };
            let improvement = t * (gap - 0.5 * eta * t);
            if best.is_none_or(|(_, _, value)| improvement > value) {
                best = Some((j, t, improvement));
            }
        }
        let Some((j, t, improvement)) = best else {
            return Err("fitcsvm: SMO stalled before KKT convergence".into());
        };
        if improvement <= 0. || t <= 0. {
            return Err("fitcsvm: SMO stalled before KKT convergence".into());
        }
        alpha[i] = (alpha[i] + y[i] * t).clamp(0., c);
        alpha[j] = (alpha[j] - y[j] * t).clamp(0., c);
        for k in 0..n {
            gradient[k] += t * (gram[i * n + k] - gram[j * n + k]);
        }
    }
    Err("fitcsvm: SMO iteration limit exceeded before KKT convergence".into())
}
