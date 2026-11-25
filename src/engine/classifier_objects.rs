//! Immutable classifier state and the shared prediction dispatch.
use super::{Engine, Value};
use crate::{Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, StringArray};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub(super) enum Labels {
    Numeric(Vec<f64>),
    Typed(crate::TypedArray),
    Logical(Vec<bool>),
    Text(Vec<String>),
    String(Vec<String>),
    Categorical(Vec<String>, Arc<Vec<String>>, bool),
}
impl Labels {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Numeric(v) => v.len(),
            Self::Typed(v) => v.len(),
            Self::Logical(v) => v.len(),
            Self::Text(v) | Self::String(v) | Self::Categorical(v, ..) => v.len(),
        }
    }
    pub(super) fn key(&self, i: usize) -> String {
        match self {
            Self::Numeric(v) => format!("N{:016x}", if v[i] == 0. { 0 } else { v[i].to_bits() }),
            Self::Typed(v) => {
                if v.is_integer() {
                    format!("I{}", v.int(i))
                } else {
                    let z = v.real(i);
                    format!("S{:016x}", if z == 0. { 0 } else { z.to_bits() })
                }
            }
            Self::Logical(v) => format!("L{}", v[i] as u8),
            Self::Text(v) | Self::String(v) | Self::Categorical(v, ..) => format!("T{}", v[i]),
        }
    }
    // ClassNames is sorted (categoricals use category order), independent of row order.
    pub(super) fn class_indices(&self) -> (Vec<usize>, Vec<usize>) {
        let mut ids: Vec<_> = (0..self.len()).collect();
        ids.sort_by(|&a, &b| match self {
            Self::Numeric(v) => v[a].partial_cmp(&v[b]).unwrap(),
            Self::Typed(v) => {
                if v.is_integer() {
                    v.int(a).cmp(&v.int(b))
                } else {
                    v.real(a).partial_cmp(&v.real(b)).unwrap()
                }
            }
            Self::Logical(v) => v[a].cmp(&v[b]),
            Self::Text(v) | Self::String(v) => v[a].cmp(&v[b]),
            Self::Categorical(v, cats, _) => cats
                .iter()
                .position(|x| x == &v[a])
                .cmp(&cats.iter().position(|x| x == &v[b])),
        });
        ids.dedup_by(|a, b| self.key(*a) == self.key(*b));
        let lookup: BTreeMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(k, &i)| (self.key(i), k))
            .collect();
        let codes = (0..self.len()).map(|i| lookup[&self.key(i)]).collect();
        (ids, codes)
    }
    pub(super) fn subset(&self, ids: &[usize]) -> Result<Self> {
        Ok(match self {
            Self::Numeric(v) => Self::Numeric(ids.iter().map(|&i| v[i]).collect()),
            Self::Typed(v) => Self::Typed(v.select(ids.len(), 1, ids)?),
            Self::Logical(v) => Self::Logical(ids.iter().map(|&i| v[i]).collect()),
            Self::Text(v) => Self::Text(ids.iter().map(|&i| v[i].clone()).collect()),
            Self::String(v) => Self::String(ids.iter().map(|&i| v[i].clone()).collect()),
            Self::Categorical(v, cats, ordinal) => Self::Categorical(
                ids.iter().map(|&i| v[i].clone()).collect(),
                cats.clone(),
                *ordinal,
            ),
        })
    }
    pub(super) fn values(&self, ids: &[usize], e: &Engine) -> Result<Value> {
        self.optional_values(&ids.iter().copied().map(Some).collect::<Vec<_>>(), e)
    }
    fn optional_values(&self, ids: &[Option<usize>], e: &Engine) -> Result<Value> {
        match self {
            Self::Typed(v) => {
                let values = if v.is_integer() {
                    crate::TypedArray::from_ints(
                        v.class(),
                        ids.len(),
                        1,
                        ids.iter().map(|i| i.map_or(0, |i| v.int(i))),
                    )?
                } else {
                    crate::TypedArray::from_reals(
                        v.class(),
                        ids.len(),
                        1,
                        ids.iter().map(|i| i.map_or(f64::NAN, |i| v.real(i))),
                    )?
                };
                Ok(Value::Container(Array::Typed(values)))
            }
            Self::Numeric(v) => Ok(Value::Matrix(Matrix::from_column_vec(
                ids.len(),
                1,
                ids.iter().map(|i| i.map_or(f64::NAN, |i| v[i])).collect(),
            )?)),
            Self::Logical(v) => Ok(Value::Logical(LogicalMatrix::from_column_vec(
                ids.len(),
                1,
                ids.iter()
                    .map(|i| u8::from(i.is_some_and(|i| v[i])))
                    .collect(),
                &e.cancel,
            )?)),
            Self::Text(v) => cellstr(
                ids.iter()
                    .map(|i| i.map_or(String::new(), |i| v[i].clone()))
                    .collect(),
                e,
            ),
            Self::String(v) => Ok(Value::Container(Array::Str(StringArray::new(
                ids.len(),
                1,
                ids.iter()
                    .map(|i| i.map(|i| Arc::from(v[i].as_str())))
                    .collect(),
            )?))),
            Self::Categorical(v, cats, ordinal) => {
                let codes = ids
                    .iter()
                    .map(|i| {
                        i.and_then(|i| cats.iter().position(|x| x == &v[i]))
                            .map_or(0, |p| (p + 1) as u32)
                    })
                    .collect();
                Ok(Value::Container(Array::Categorical(
                    crate::CategoricalArray::new(ids.len(), 1, codes, cats.clone(), *ordinal)?,
                )))
            }
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Numeric(v) => v.len() * 8,
            Self::Typed(v) => v.bytes(),
            Self::Logical(v) => v.len(),
            Self::Text(v) | Self::String(v) => {
                v.len() * 24 + v.iter().map(String::len).sum::<usize>()
            }
            Self::Categorical(v, cats, _) => {
                (v.len() + cats.len()) * 24
                    + v.iter().chain(cats.iter()).map(String::len).sum::<usize>()
            }
        }
    }
}
fn cellstr(items: Vec<String>, e: &Engine) -> Result<Value> {
    let a = items
        .into_iter()
        .map(|s| CharacterRow::from_text(&s, &e.cancel).map(|x| Arc::new(Array::Character(x))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        a.len(),
        1,
        a,
        &e.cancel,
    )?)))
}

#[derive(Clone, Debug)]
pub(super) enum FitOptions {
    Advanced(Box<super::stats_models::ml::Config>),
    Knn {
        neighbors: usize,
        metric: String,
        standardize: bool,
    },
    Nb,
    Tree {
        max_splits: usize,
        min_leaf: usize,
        min_parent: usize,
    },
    Svm {
        c: f64,
        kernel: String,
        scale: f64,
        standardize: bool,
    },
}
#[derive(Clone, Debug)]
pub(super) enum Kind {
    Advanced(Arc<super::stats_models::ml::Model>),
    Knn {
        neighbors: usize,
        metric: String,
    },
    Nb {
        means: Vec<Vec<f64>>,
        vars: Vec<Vec<f64>>,
        priors: Vec<f64>,
    },
    Tree {
        nodes: Vec<Node>,
    },
    Svm {
        alpha: Vec<f64>,
        bias: f64,
        kernel: String,
        scale: f64,
    },
    Partitioned {
        trained: Vec<Arc<Classifier>>,
        test_indices: Vec<Vec<usize>>,
    },
}
#[derive(Clone, Debug)]
pub(super) struct Node {
    pub split: Option<(usize, f64, usize, usize)>,
    pub scores: Vec<f64>,
}
#[derive(Clone, Debug)]
pub(super) struct Classifier {
    pub kind: Kind,
    pub options: FitOptions,
    pub x: Matrix,
    pub y: Labels,
    pub codes: Vec<usize>,
    pub class_rows: Vec<usize>,
    pub names: Vec<String>,
    pub response: String,
    pub means: Vec<f64>,
    pub scales: Vec<f64>,
}
impl Classifier {
    pub(super) fn class(&self) -> &'static str {
        match &self.kind {
            Kind::Advanced(m) => m.class(),
            Kind::Knn { .. } => "ClassificationKNN",
            Kind::Tree { .. } => "ClassificationTree",
            Kind::Svm { .. } => "ClassificationSVM",
            Kind::Nb { .. } => "ClassificationNaiveBayes",
            Kind::Partitioned { .. } => "ClassificationPartitionedModel",
        }
    }
    pub(super) fn bytes(&self) -> usize {
        let state = match &self.kind {
            Kind::Advanced(m) => m.bytes(),
            Kind::Knn { metric, .. } => metric.len(),
            Kind::Nb {
                means,
                vars,
                priors,
            } => {
                (means.iter().map(Vec::len).sum::<usize>()
                    + vars.iter().map(Vec::len).sum::<usize>()
                    + priors.len())
                    * 8
            }
            Kind::Tree { nodes } => {
                nodes.len() * std::mem::size_of::<Node>()
                    + nodes.iter().map(|n| n.scores.len() * 8).sum::<usize>()
            }
            Kind::Svm { alpha, .. } => alpha.len() * 8,
            Kind::Partitioned {
                trained,
                test_indices,
            } => {
                trained.iter().map(|c| c.bytes()).sum::<usize>()
                    + test_indices.iter().map(|v| v.len() * 8).sum::<usize>()
            }
        };
        self.x.len() * 8
            + self.y.bytes()
            + (self.codes.len() + self.class_rows.len() + self.means.len() + self.scales.len()) * 8
            + self.names.iter().map(|s| s.len() + 24).sum::<usize>()
            + state
            + 512
    }
    pub(super) fn body(&self) -> String {
        format!(
            "  {} with properties:\n\n{:>23}: {}\n{:>23}: {}\n{:>23}: {}\n{:>23}: {}\n",
            self.class(),
            "NumObservations",
            self.x.rows(),
            "NumPredictors",
            self.x.cols(),
            "ClassNames",
            self.class_rows.len(),
            "ResponseName",
            self.response
        )
    }
}
pub(super) fn property_names(e: &Engine, c: &Classifier) -> Result<Value> {
    let mut names = vec![
        "ClassNames",
        "ResponseName",
        "PredictorNames",
        "NumObservations",
        "X",
        "Y",
    ];
    match &c.kind {
        Kind::Advanced(m) => names.extend(super::stats_models::ml::property_names(m)),
        Kind::Nb { .. } => names.push("DistributionParameters"),
        Kind::Tree { .. } => names.extend(["NumNodes", "Children"]),
        Kind::Svm { kernel, .. } => {
            names.extend(["Alpha", "Bias", "SupportVectors"]);
            if kernel == "linear" {
                names.push("Beta");
            }
        }
        Kind::Partitioned { .. } => names.push("KFold"),
        _ => {}
    }
    cellstr(names.into_iter().map(String::from).collect(), e)
}
pub(super) fn property(e: &Engine, c: &Classifier, name: &str) -> Result<Value> {
    match name {
        "ClassNames" => return c.y.values(&c.class_rows, e),
        "ResponseName" => return Ok(Value::Text(c.response.clone())),
        "PredictorNames" => return cellstr(c.names.clone(), e),
        "NumObservations" => return Ok(Value::Matrix(Matrix::scalar(c.x.rows() as f64))),
        "X" => return Ok(Value::Matrix(c.x.clone())),
        "Y" => return c.y.values(&(0..c.y.len()).collect::<Vec<_>>(), e),
        _ => {}
    }
    match (&c.kind, name) {
        (Kind::Advanced(m), _) => super::stats_models::ml::property(e, m, name),
        (Kind::Nb { means, vars, .. }, "DistributionParameters") => {
            let mut cells = Vec::new();
            for j in 0..c.x.cols() {
                for k in 0..c.class_rows.len() {
                    cells.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                        2,
                        1,
                        vec![means[k][j], vars[k][j].sqrt()],
                    )?)));
                }
            }
            Ok(Value::Container(Array::Cell(CellArray::from_shared(
                c.class_rows.len(),
                c.x.cols(),
                cells,
                &e.cancel,
            )?)))
        }
        (Kind::Tree { nodes }, "NumNodes") => Ok(Value::Matrix(Matrix::scalar(nodes.len() as f64))),
        (Kind::Tree { nodes }, "Children") => {
            let mut data = vec![0.; 2 * nodes.len()];
            for (i, n) in nodes.iter().enumerate() {
                if let Some((_, _, l, r)) = n.split {
                    data[i] = (l + 1) as f64;
                    data[i + nodes.len()] = (r + 1) as f64;
                }
            }
            Ok(Value::Matrix(Matrix::from_column_vec(
                nodes.len(),
                2,
                data,
            )?))
        }
        (
            Kind::Svm {
                alpha,
                bias,
                kernel,
                scale,
            },
            _,
        ) => {
            let ids: Vec<_> = (0..alpha.len()).filter(|&i| alpha[i] > 0.).collect();
            match name {
                "Alpha" => Ok(Value::Matrix(Matrix::from_column_vec(
                    ids.len(),
                    1,
                    ids.iter().map(|&i| alpha[i]).collect(),
                )?)),
                "Bias" => Ok(Value::Matrix(Matrix::scalar(*bias))),
                "SupportVectors" => {
                    let mut data = Vec::with_capacity(ids.len() * c.x.cols());
                    for j in 0..c.x.cols() {
                        for &i in &ids {
                            data.push((c.x.data()[i + j * c.x.rows()] - c.means[j]) / c.scales[j]);
                        }
                    }
                    Ok(Value::Matrix(Matrix::from_column_vec(
                        ids.len(),
                        c.x.cols(),
                        data,
                    )?))
                }
                "Beta" if kernel == "linear" => {
                    let data = (0..c.x.cols())
                        .map(|j| {
                            ids.iter()
                                .map(|&i| {
                                    alpha[i]
                                        * if c.codes[i] == 1 { 1. } else { -1. }
                                        * (c.x.data()[i + j * c.x.rows()] - c.means[j])
                                        / c.scales[j]
                                        / scale
                                })
                                .sum()
                        })
                        .collect();
                    Ok(Value::Matrix(Matrix::from_column_vec(c.x.cols(), 1, data)?))
                }
                _ => Err(format!("{}: unknown property '{name}'", c.class())),
            }
        }
        (Kind::Partitioned { trained, .. }, "KFold") => {
            Ok(Value::Matrix(Matrix::scalar(trained.len() as f64)))
        }
        _ => Err(format!("{}: unknown property '{name}'", c.class())),
    }
}

pub(super) fn take_rows(x: &Matrix, ids: &[usize]) -> Result<Matrix> {
    let mut data = Vec::with_capacity(Matrix::checked_len(ids.len(), x.cols())?);
    for j in 0..x.cols() {
        for &i in ids {
            data.push(x.data()[i + j * x.rows()]);
        }
    }
    Matrix::from_column_vec(ids.len(), x.cols(), data)
}
pub(super) fn predictor_matrix(
    e: &Engine,
    v: &Value,
    names: Option<&[String]>,
    operation: &str,
) -> Result<(Matrix, Vec<String>)> {
    if let Value::Container(Array::Table(t)) = v {
        let columns: Vec<_> = if let Some(names) = names {
            names
                .iter()
                .map(|n| {
                    t.names()
                        .iter()
                        .position(|s| s == n)
                        .ok_or_else(|| format!("{operation}: missing predictor '{n}'"))
                })
                .collect::<Result<_>>()?
        } else {
            (0..t.names().len()).collect()
        };
        let mut data = Vec::new();
        let mut out_names = Vec::new();
        for i in columns {
            e.check_cancel()?;
            let m = Value::from(t.column(i).clone())
                .matrix(&e.cancel)
                .map_err(|_| {
                    format!("{operation}: table predictors must be real numeric columns")
                })?;
            if m.cols() != 1 || matches!(t.column(i), Array::Character(_)) {
                return Err(format!(
                    "{operation}: table predictors must be real numeric single-column variables"
                ));
            }
            e.preflight_elements(Matrix::checked_len(t.height(), out_names.len() + 1)?)?;
            data.extend_from_slice(m.data());
            out_names.push(t.names()[i].clone());
        }
        Ok((
            Matrix::from_column_vec(t.height(), out_names.len(), data)?,
            out_names,
        ))
    } else {
        if v.is_character() || v.is_nd() {
            return Err(format!(
                "{operation}: predictors must be a real numeric matrix"
            ));
        }
        let x = v
            .clone()
            .matrix(&e.cancel)
            .map_err(|_| format!("{operation}: predictors must be a real numeric matrix"))?;
        let names = (1..=x.cols()).map(|j| format!("x{j}")).collect();
        Ok((x, names))
    }
}
fn query_row(c: &Classifier, x: &Matrix, r: usize) -> Vec<f64> {
    (0..x.cols())
        .map(|j| (x.data()[r + j * x.rows()] - c.means[j]) / c.scales[j])
        .collect()
}
fn distance(c: &Classifier, q: &[f64], i: usize, metric: &str) -> f64 {
    let mut d = 0.;
    for (j, &qj) in q.iter().enumerate() {
        let z = (qj - (c.x.data()[i + j * c.x.rows()] - c.means[j]) / c.scales[j]).abs();
        d = match metric {
            "cityblock" => d + z,
            "chebychev" => d.max(z),
            _ => d + z * z,
        };
    }
    d
}
pub(super) fn predict_rows(e: &Engine, c: &Classifier, x: &Matrix) -> Result<(Vec<usize>, Matrix)> {
    let classes = c.class_rows.len();
    let len = Matrix::checked_len(x.rows(), classes)?;
    e.preflight_elements(len.saturating_add(c.x.rows() * 2))?;
    if x.cols() != c.x.cols() || x.data().iter().any(|v| !v.is_finite()) {
        return Err(
            "predict: Xnew must be finite with the same number of columns as the training data"
                .into(),
        );
    }
    if (x.rows() as u128) * (c.x.rows() as u128) * (x.cols() as u128) > 200_000_000 {
        return Err("predict: comparison work limit exceeded; predict in smaller batches".into());
    }
    if let Kind::Advanced(m) = &c.kind {
        let scores = super::stats_models::ml::scores(e, m, x)?;
        if scores.data().iter().any(|v| !v.is_finite()) {
            return Err("predict: numerical overflow in scores".into());
        }
        let chosen = (0..x.rows())
            .map(|r| {
                let best = (0..classes)
                    .max_by(|&a, &b| {
                        scores.data()[r + a * x.rows()]
                            .total_cmp(&scores.data()[r + b * x.rows()])
                            .then_with(|| b.cmp(&a))
                    })
                    .unwrap();
                c.class_rows[best]
            })
            .collect();
        return Ok((chosen, scores));
    }
    let mut chosen = Vec::with_capacity(x.rows());
    let mut scores = vec![0.; len];
    for r in 0..x.rows() {
        e.check_cancel()?;
        let q = query_row(c, x, r);
        let s = match &c.kind {
            Kind::Knn { neighbors, metric } => {
                let mut d: Vec<_> = (0..c.x.rows())
                    .map(|i| (distance(c, &q, i, metric), i))
                    .collect();
                let order =
                    |a: &(f64, usize), b: &(f64, usize)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
                if *neighbors < d.len() {
                    d.select_nth_unstable_by(*neighbors, order);
                }
                let mut s = vec![0.; classes];
                for &(_, i) in d.iter().take(*neighbors) {
                    s[c.codes[i]] += 1. / *neighbors as f64;
                }
                s
            }
            Kind::Nb {
                means,
                vars,
                priors,
            } => {
                let l: Vec<_> = (0..classes)
                    .map(|k| {
                        priors[k].ln()
                            + q.iter()
                                .enumerate()
                                .map(|(j, &z)| {
                                    -0.5 * ((z - means[k][j]).powi(2) / vars[k][j]
                                        + vars[k][j].ln()
                                        + std::f64::consts::TAU.ln())
                                })
                                .sum::<f64>()
                    })
                    .collect();
                let mx = l.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let sum: f64 = l.iter().map(|v| (v - mx).exp()).sum();
                l.into_iter().map(|v| (v - mx).exp() / sum).collect()
            }
            Kind::Tree { nodes } => {
                let mut i = 0;
                while let Some((j, t, l, r)) = nodes[i].split {
                    i = if q[j] <= t { l } else { r };
                }
                nodes[i].scores.clone()
            }
            Kind::Svm {
                alpha,
                bias,
                kernel,
                scale,
            } => {
                let mut f = *bias;
                for (i, &a) in alpha.iter().enumerate() {
                    if a == 0. {
                        continue;
                    }
                    let k = if kernel == "linear" {
                        q.iter()
                            .enumerate()
                            .map(|(j, &z)| {
                                (z / scale)
                                    * ((c.x.data()[i + j * c.x.rows()] - c.means[j])
                                        / c.scales[j]
                                        / scale)
                            })
                            .sum()
                    } else {
                        (-distance(c, &q, i, "euclidean") / (scale * scale)).exp()
                    };
                    f += a * if c.codes[i] == 1 { 1. } else { -1. } * k;
                }
                vec![-f, f]
            }
            Kind::Advanced(_) => unreachable!(),
            Kind::Partitioned { .. } => {
                return Err("predict: use kfoldPredict for a partitioned model".into());
            }
        };
        if s.iter().any(|v| !v.is_finite()) {
            return Err("predict: numerical overflow in scores".into());
        }
        let best = (0..classes)
            .max_by(|&a, &b| s[a].total_cmp(&s[b]).then_with(|| b.cmp(&a)))
            .unwrap();
        chosen.push(c.class_rows[best]);
        for k in 0..classes {
            scores[r + k * x.rows()] = s[k];
        }
    }
    Ok((chosen, Matrix::from_column_vec(x.rows(), classes, scores)?))
}
pub(super) fn classifier_predict(
    e: &mut Engine,
    c: &Classifier,
    args: &[Value],
) -> Result<Vec<Value>> {
    let [input] = args else {
        return Err("predict: expected model and Xnew".into());
    };
    let (x, _) = predictor_matrix(e, input, Some(&c.names), "predict")?;
    let (chosen, scores) = predict_rows(e, c, &x)?;
    let mut outputs = vec![c.y.values(&chosen, e)?, Value::Matrix(scores.clone())];
    if matches!(&c.kind,Kind::Advanced(m) if matches!(m.as_ref(),super::stats_models::ml::Model::Discriminant(_)))
    {
        outputs.push(Value::Matrix(Matrix::from_column_vec(
            scores.rows(),
            scores.cols(),
            scores.data().iter().map(|s| 1. - s).collect(),
        )?));
    }
    Ok(outputs)
}
pub(super) fn resub_loss(e: &mut Engine, c: &Classifier, args: &[Value]) -> Result<Value> {
    super::stats_models::metrics::call(e, c, "resubLoss", args)
}

pub(super) fn crossval(e: &mut Engine, c: &Classifier, args: &[Value]) -> Result<Value> {
    if matches!(c.kind, Kind::Partitioned { .. }) {
        return Err("crossval: model is already partitioned".into());
    }
    let n = c.x.rows();
    let partition = super::stats_models::partition::for_crossval(e, n, c.codes.clone(), args)?;
    if partition.n != n {
        return Err("crossval: CVPartition NumObservations does not match training rows".into());
    }
    let folds = partition.tests.len();
    if folds > 1000 || folds.saturating_mul(c.x.len()) > 8_000_000 {
        return Err("crossval: retained fold workspace limit exceeded".into());
    }
    e.preflight_elements(folds.saturating_mul(c.bytes().div_ceil(8)))?;
    let tests = partition.tests.clone();
    let mut trained = Vec::new();
    let mut retained = c.bytes();
    for test in &tests {
        e.check_cancel()?;
        let mut is_test = vec![false; n];
        for &i in test {
            is_test[i] = true;
        }
        let train: Vec<_> = (0..n).filter(|&i| partition.resub || !is_test[i]).collect();
        let model = super::stats_models::classifiers::train(
            e,
            take_rows(&c.x, &train)?,
            c.y.subset(&train)?,
            c.names.clone(),
            c.response.clone(),
            c.options.clone(),
        )
        .map_err(|s| format!("crossval: fold fitting failed: {s}"))?;
        retained = retained.saturating_add(model.bytes());
        if retained > 128 * 1024 * 1024 {
            return Err("crossval: retained model workspace exceeds 128 MiB".into());
        }
        e.preflight_elements(retained.div_ceil(8))?;
        trained.push(Arc::new(model));
    }
    let mut model = c.clone();
    model.kind = Kind::Partitioned {
        trained,
        test_indices: tests,
    };
    e.classifier_new(model)
}
fn fold_predictions(e: &Engine, c: &Classifier) -> Result<(Vec<Option<usize>>, Matrix)> {
    let Kind::Partitioned {
        trained,
        test_indices,
    } = &c.kind
    else {
        return Err("kfoldPredict: expected a ClassificationPartitionedModel".into());
    };
    let n = c.x.rows();
    let classes = c.class_rows.len();
    e.preflight_elements(Matrix::checked_len(n, classes)?)?;
    let mut chosen = vec![None; n];
    let mut scores = vec![f64::NAN; n * classes];
    let class_map: BTreeMap<_, _> = c
        .class_rows
        .iter()
        .enumerate()
        .map(|(k, &i)| (c.y.key(i), k))
        .collect();
    for (model, test) in trained.iter().zip(test_indices) {
        let (pred, s) = predict_rows(e, model, &take_rows(&c.x, test)?)?;
        for (r, &i) in test.iter().enumerate() {
            let k = class_map[&model.y.key(pred[r])];
            chosen[i] = Some(c.class_rows[k]);
            for j in 0..classes {
                scores[i + j * n] = 0.;
            }
            for (j, &row) in model.class_rows.iter().enumerate() {
                scores[i + class_map[&model.y.key(row)] * n] = s.data()[r + j * test.len()];
            }
        }
    }
    Ok((chosen, Matrix::from_column_vec(n, classes, scores)?))
}
pub(super) fn kfold_predict(e: &Engine, c: &Classifier, args: &[Value]) -> Result<Vec<Value>> {
    if !args.is_empty() {
        return Err("kfoldPredict: expected one partitioned model".into());
    }
    let (chosen, scores) = fold_predictions(e, c)?;
    Ok(vec![
        c.y.optional_values(&chosen, e)?,
        Value::Matrix(scores),
    ])
}
pub(super) fn kfold_loss(e: &Engine, c: &Classifier, args: &[Value]) -> Result<Value> {
    let loss = super::stats_models::metrics::loss_name(e, args, "kfoldLoss")?;
    let (chosen, scores) = fold_predictions(e, c)?;
    if matches!(loss.as_str(), "classiferror" | "mincost") {
        let mut count = 0;
        let mut bad = 0;
        for (i, p) in chosen.iter().enumerate() {
            if let Some(p) = p {
                count += 1;
                bad += usize::from(c.codes[*p] != c.codes[i]);
            }
        }
        return Ok(Value::Matrix(Matrix::scalar(bad as f64 / count as f64)));
    }
    super::stats_models::metrics::evaluate(e, c, &scores, &c.y, "kfoldLoss", &loss)
}
