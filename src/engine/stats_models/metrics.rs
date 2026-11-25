//! Shared classification loss, margin, edge, and impurity importance.
use super::*;
use crate::engine::classifier_objects::{Classifier, Kind, predict_rows, predictor_matrix};
pub(in crate::engine) fn loss_name(e: &Engine, a: &[Value], name: &str) -> Result<String> {
    let a = e.text_arguments(a.to_vec())?;
    let mut result = "classiferror".to_string();
    for (_, v) in options(name, &a, &["lossfun"])? {
        result = super::ml::string(e, v, name)?;
    }
    if ![
        "classiferror",
        "binodeviance",
        "exponential",
        "hinge",
        "mincost",
    ]
    .contains(&result.as_str())
    {
        return Err(format!("{name}: unsupported LossFun"));
    }
    Ok(result)
}
pub(in crate::engine) fn evaluate(
    e: &Engine,
    c: &Classifier,
    s: &Matrix,
    y: &crate::engine::classifier_objects::Labels,
    name: &str,
    loss: &str,
) -> Result<Value> {
    if y.len() != s.rows() {
        return Err(format!(
            "{name}: X and Y must have the same number of observations"
        ));
    }
    let map: std::collections::BTreeMap<_, _> = c
        .class_rows
        .iter()
        .enumerate()
        .map(|(k, &i)| (c.y.key(i), k))
        .collect();
    let mut priors = vec![0.; c.class_rows.len()];
    for &code in &c.codes {
        priors[code] += 1. / c.codes.len() as f64;
    }
    if let Kind::Advanced(m) = &c.kind
        && let super::ml::Model::Discriminant(m) = m.as_ref()
    {
        priors = m.priors().to_vec();
    }
    let mut valid = vec![0usize; priors.len()];
    for i in 0..s.rows() {
        if (0..s.cols()).all(|k| s.data()[i + k * s.rows()].is_finite())
            && let Some(&k) = map.get(&y.key(i))
        {
            valid[k] += 1;
        }
    }
    let normalizer = priors
        .iter()
        .zip(&valid)
        .filter(|(_, n)| **n > 0)
        .map(|(p, _)| p)
        .sum::<f64>();
    let mut margins = Vec::new();
    let mut edge = 0.;
    let mut sum = 0.;

    for i in 0..s.rows() {
        if i.is_multiple_of(256) {
            e.check_cancel()?;
        }
        if (0..s.cols()).any(|k| !s.data()[i + k * s.rows()].is_finite()) {
            margins.push(f64::NAN);
            continue;
        }
        let key = y.key(i);
        let Some(&k) = map.get(&key) else {
            return Err(format!("{name}: response contains an unknown class"));
        };
        let true_score = s.data()[i + k * s.rows()];
        let other = (0..s.cols())
            .filter(|&j| j != k)
            .map(|j| s.data()[i + j * s.rows()])
            .fold(f64::NEG_INFINITY, f64::max);
        let margin = true_score - other;
        margins.push(margin);
        let best = (0..s.cols())
            .max_by(|&a, &b| {
                s.data()[i + a * s.rows()]
                    .total_cmp(&s.data()[i + b * s.rows()])
                    .then_with(|| b.cmp(&a))
            })
            .unwrap();
        let weight = priors[k] / valid[k] as f64;
        edge += weight * margin;
        // Public loss documentation uses y*f_positive for binary problems,
        // and the true-class score for multiclass problems. This differs from
        // the score difference returned by the margin method.
        let loss_score = if s.cols() == 2 {
            if k == 1 {
                s.data()[i + s.rows()]
            } else {
                -s.data()[i + s.rows()]
            }
        } else {
            true_score
        };
        sum += weight
            * match loss {
                "classiferror" | "mincost" => f64::from(best != k),
                "hinge" => (1. - loss_score).max(0.),
                "binodeviance" => super::ml::softplus(-2. * loss_score),
                "exponential" => (-loss_score).exp(),
                _ => unreachable!(),
            };
    }
    match name {
        "margin" => value(margins.len(), 1, margins),
        "edge" => Ok(Value::Matrix(Matrix::scalar(edge / normalizer))),
        _ => Ok(Value::Matrix(Matrix::scalar(sum / normalizer))),
    }
}
pub(in crate::engine) fn call(
    e: &mut Engine,
    c: &Classifier,
    name: &str,
    a: &[Value],
) -> Result<Value> {
    if name == "predictorImportance" {
        if !a.is_empty() {
            return Err("predictorImportance: expected one tree model".into());
        }
        let importance = match &c.kind {
            Kind::Advanced(m) => {
                if let super::ml::Model::Ensemble(m) = m.as_ref() {
                    m.importance()
                } else {
                    return Err("predictorImportance: expected a tree or tree ensemble".into());
                }
            }
            Kind::Tree { nodes } => {
                let mut mass = vec![0usize; nodes.len()];
                for i in 0..c.x.rows() {
                    if i.is_multiple_of(256) {
                        e.check_cancel()?;
                    }
                    let mut at = 0;
                    loop {
                        mass[at] += 1;
                        if let Some((j, t, l, r)) = nodes[at].split {
                            at = if c.x.data()[i + j * c.x.rows()] <= t {
                                l
                            } else {
                                r
                            };
                        } else {
                            break;
                        }
                    }
                }
                let mut result = vec![0.; c.x.cols()];
                let mut branches = 0;
                for (i, node) in nodes.iter().enumerate() {
                    if let Some((j, _, l, r)) = node.split {
                        let gini =
                            |id: usize| 1. - nodes[id].scores.iter().map(|z| z * z).sum::<f64>();
                        result[j] += (mass[i] as f64 * gini(i)
                            - mass[l] as f64 * gini(l)
                            - mass[r] as f64 * gini(r))
                            / c.x.rows() as f64;
                        branches += 1;
                    }
                }
                if branches > 0 {
                    for v in &mut result {
                        *v /= branches as f64;
                    }
                }
                result
            }
            _ => return Err("predictorImportance: expected a tree or tree ensemble".into()),
        };
        return value(1, importance.len(), importance);
    }
    if name == "oobLoss" {
        let loss = loss_name(e, a, name)?;
        let Kind::Advanced(m) = &c.kind else {
            return Err("oobLoss: expected a Bag ensemble".into());
        };
        let super::ml::Model::Ensemble(m) = m.as_ref() else {
            return Err("oobLoss: expected a Bag ensemble".into());
        };
        let scores = m.oob_scores(e, &c.x)?;
        return evaluate(e, c, &scores, &c.y, name, &loss);
    }
    if name == "resubLoss" {
        let loss = loss_name(e, a, name)?;
        let (_, scores) = predict_rows(e, c, &c.x)?;
        return evaluate(e, c, &scores, &c.y, name, &loss);
    }
    if a.len() < 2 {
        return Err(format!("{name}: expected model, X and Y"));
    }
    let (x, _) = predictor_matrix(e, &a[0], Some(&c.names), name)?;
    let response = if let Value::Container(Array::Table(t)) = &a[0] {
        if a[1].is_character() || matches!(&a[1], Value::Text(_) | Value::QuotedString(_)) {
            let column = e.text_value(a[1].clone())?;
            let index = t
                .names()
                .iter()
                .position(|n| n == &column)
                .ok_or_else(|| format!("{name}: unknown response variable"))?;
            Value::from(t.column(index).clone())
        } else {
            a[1].clone()
        }
    } else {
        a[1].clone()
    };
    let y = super::classifiers::labels(e, &response, name)?;
    let loss = loss_name(e, &a[2..], name)?;
    let (_, scores) = predict_rows(e, c, &x)?;
    evaluate(e, c, &scores, &y, name, &loss)
}
