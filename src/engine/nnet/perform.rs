use super::*;
pub const NAMES: &[&str] = &["mse", "mae", "sse", "sae", "crossentropy"];
pub(super) fn call(e: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    let net = args
        .first()
        .and_then(|v| super::super::objects::nnet_value(e, v));
    let ranges = net
        .as_ref()
        .map(|n| n.core.output.range.clone())
        .unwrap_or_default();
    let (kind, t, y, reg, norm, weights) = if let Some(net) = net {
        arity(name, args, 3)?;
        let kind = if name == "perform" {
            text(&net.props["performFcn"])?
        } else {
            name.into()
        };
        let p = &net.props["performParam"];
        let regularization = scalar(field(p, "regularization")?)?;
        (
            kind,
            sequence_matrix(e, &args[1])?,
            sequence_matrix(e, &args[2])?,
            regularization,
            text(field(p, "normalization")?)?,
            if regularization > 0. {
                net.core.getwb()
            } else {
                Vec::new()
            },
        )
    } else {
        if name == "perform" {
            return Err("perform: first argument must be a network".into());
        }
        if name == "crossentropy" {
            arity(name, args, 2)?;
            (
                name.into(),
                sequence_matrix(e, &args[0])?,
                sequence_matrix(e, &args[1])?,
                0.,
                "none".into(),
                Vec::new(),
            )
        } else if args.len() == 1 {
            let error = sequence_matrix(e, &args[0])?;
            (
                name.into(),
                error.clone(),
                Matrix::filled(error.rows(), error.cols(), 0.)?,
                0.,
                "none".into(),
                Vec::new(),
            )
        } else {
            arity(name, args, 2)?;
            (
                name.into(),
                sequence_matrix(e, &args[0])?,
                sequence_matrix(e, &args[1])?,
                0.,
                "none".into(),
                Vec::new(),
            )
        }
    };
    if t.rows() != y.rows() || t.cols() != y.cols() {
        return Err(format!("{kind}: targets and outputs must have equal size"));
    }
    if kind == "crossentropy" && norm != "none" {
        return Err("crossentropy: only normalization none is supported".into());
    }
    let mut scales = vec![1.; t.rows()];
    if norm != "none" && kind != "crossentropy" {
        for (r, s) in scales.iter_mut().enumerate() {
            let min = (0..t.cols())
                .map(|c| t.data()[c * t.rows() + r])
                .filter(|v| v.is_finite())
                .fold(f64::INFINITY, f64::min);
            let max = (0..t.cols())
                .map(|c| t.data()[c * t.rows() + r])
                .filter(|v| v.is_finite())
                .fold(f64::NEG_INFINITY, f64::max);
            let (min, max) = ranges.get(r).copied().unwrap_or((min, max));
            if max > min {
                *s = if norm == "standard" { 2. } else { 1. } / (max - min);
            }
        }
    }
    let mut total = 0.;
    let mut count = 0usize;
    for (i, (&t, &y)) in t.data().iter().zip(y.data()).enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        if t.is_nan() || y.is_nan() {
            continue;
        }
        let loss = if kind == "crossentropy" {
            if !(0. ..=1.).contains(&t) || !(0. ..=1.).contains(&y) {
                return Err("crossentropy: targets and outputs must lie in [0,1]".into());
            }
            let term = |t: f64, y: f64| {
                if t == 0. {
                    0.
                } else {
                    -t * y.max(f64::EPSILON).ln()
                }
            };
            term(t, y)
                + if scales.len() == 1 {
                    term(1. - t, 1. - y)
                } else {
                    0.
                }
        } else {
            let err = (t - y) * scales[i % scales.len()];
            if kind == "mse" || kind == "sse" {
                err * err
            } else {
                err.abs()
            }
        };
        total += loss;
        count += 1;
    }
    let mean = matches!(kind.as_str(), "mse" | "mae" | "crossentropy");
    if mean {
        total /= count as f64;
    }
    if reg > 0. && !weights.is_empty() {
        let mut penalty = weights
            .iter()
            .map(|w| {
                if matches!(kind.as_str(), "mae" | "sae") {
                    w.abs()
                } else {
                    w * w
                }
            })
            .sum::<f64>();
        if mean {
            penalty /= weights.len() as f64;
        }
        total = (1. - reg) * total + reg * penalty;
    }
    Ok(Value::Matrix(Matrix::scalar(total)))
}
