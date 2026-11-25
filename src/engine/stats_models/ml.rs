//! Classification learners derived from public MathWorks function documentation.
//! Numerical references are independently generated in tool/refs/stats/ml_classification.py.
use super::*;
use crate::engine::classifier_objects::{Classifier, FitOptions, Labels, predictor_matrix};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(in crate::engine) enum Config {
    Discriminant {
        kind: String,
        gamma: f64,
        prior: Option<Vec<f64>>,
        uniform: bool,
    },
    Linear {
        logistic: bool,
        lasso: bool,
        lambda: Option<f64>,
    },
    Ecoc {
        ova: bool,
        learner: Box<FitOptions>,
    },
    Ensemble {
        method: String,
        cycles: usize,
        rate: f64,
        splits: usize,
        leaf: usize,
    },
}
#[derive(Clone, Debug)]
pub(in crate::engine) enum Model {
    Discriminant(super::discriminant::Discriminant),
    Linear {
        beta: Vec<f64>,
        bias: f64,
        logistic: bool,
    },
    Ecoc {
        learners: Vec<Arc<Classifier>>,
        coding: Vec<Vec<f64>>,
        probability: bool,
    },
    Ensemble(super::ensemble::Ensemble),
}
impl Model {
    pub(in crate::engine) fn class(&self) -> &'static str {
        match self {
            Self::Discriminant(_) => "ClassificationDiscriminant",
            Self::Linear { .. } => "ClassificationLinear",
            Self::Ecoc { .. } => "ClassificationECOC",
            Self::Ensemble(m) if m.method == "bag" => "ClassificationBaggedEnsemble",
            Self::Ensemble(_) => "ClassificationEnsemble",
        }
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        match self {
            Self::Discriminant(m) => m.bytes(),
            Self::Linear { beta, .. } => 128 + 8 * beta.len(),
            Self::Ecoc {
                learners, coding, ..
            } => {
                128 + learners.iter().map(|m| m.bytes()).sum::<usize>()
                    + coding.iter().map(|v| v.len() * 8).sum::<usize>()
            }
            Self::Ensemble(m) => m.bytes(),
        }
    }
}
pub(in crate::engine) fn string(e: &Engine, v: &Value, name: &str) -> Result<String> {
    e.text_value(v.clone())
        .map(|s| s.to_ascii_lowercase())
        .map_err(|_| format!("{name}: expected text option"))
}
pub(in crate::engine) fn config(e: &Engine, name: &str, tail: &[Value]) -> Result<FitOptions> {
    let mut discrim = "linear".to_string();
    let mut gamma = 0.;
    let mut prior = None;
    let mut uniform = false;
    let mut logistic = false;
    let mut lasso = false;
    let mut lambda = None;
    let mut ova = false;
    let mut learner = FitOptions::Svm {
        c: 1.,
        kernel: "linear".into(),
        scale: 1.,
        standardize: false,
    };
    let mut method = "bag".to_string();
    let mut cycles = 100;
    let mut rate = 1.;
    let mut splits = None;
    let mut leaf = 1;
    let allowed: &[&str] = match name {
        "fitcdiscr" => &["discrimtype", "gamma", "prior"],
        "fitclinear" => &["learner", "regularization", "lambda", "solver"],
        "fitcecoc" => &["coding", "learners"],
        "fitcensemble" => &["method", "numlearningcycles", "learners", "learnrate"],
        _ => return Err(format!("{name}: unsupported classifier")),
    };
    for (key, v) in options(name, tail, allowed)? {
        match key.as_str() {
            "discrimtype" => discrim = string(e, v, name)?,
            "gamma" => {
                gamma = scalar(e, name, v)?;
                if !(0. ..=1.).contains(&gamma) {
                    return Err(format!("{name}: Gamma must be between 0 and 1"));
                }
            }
            "prior" => {
                if text(v).is_some() || v.is_character() {
                    match string(e, v, name)?.as_str() {
                        "empirical" => {}
                        "uniform" => uniform = true,
                        _ => return Err(format!("{name}: invalid Prior")),
                    }
                } else {
                    let m = real(e, name, v)?;
                    if m.rows() != 1 && m.cols() != 1 {
                        return Err(format!("{name}: Prior must be a vector"));
                    }
                    prior = Some(m.data().to_vec());
                }
            }
            "learner" => match string(e, v, name)?.as_str() {
                "svm" => logistic = false,
                "logistic" => logistic = true,
                _ => return Err(format!("{name}: Learner must be svm or logistic")),
            },
            "regularization" => match string(e, v, name)?.as_str() {
                "ridge" => lasso = false,
                "lasso" => lasso = true,
                _ => return Err(format!("{name}: Regularization must be ridge or lasso")),
            },
            "lambda" => {
                let z = scalar(e, name, v)?;
                if z <= 0. {
                    return Err(format!("{name}: Lambda must be positive"));
                }
                lambda = Some(z);
            }
            "solver" => {
                if !["lbfgs", "sparsa"].contains(&string(e, v, name)?.as_str()) {
                    return Err(format!(
                        "{name}: supported Solver names are lbfgs and sparsa"
                    ));
                }
            }
            "coding" => match string(e, v, name)?.as_str() {
                "onevsone" => ova = false,
                "onevsall" => ova = true,
                _ => return Err(format!("{name}: Coding must be onevsone or onevsall")),
            },
            "learners" => {
                learner = template_config(e, v)?;
                if name == "fitcensemble" {
                    if let FitOptions::Tree {
                        max_splits,
                        min_leaf,
                        ..
                    } = learner
                    {
                        splits = if matches!(v, Value::Container(Array::Struct(_))) {
                            Some(max_splits)
                        } else {
                            None
                        };
                        leaf = min_leaf;
                    } else {
                        return Err(format!("{name}: Learners must be tree or templateTree"));
                    }
                }
            }
            "method" => {
                method = string(e, v, name)?;
                if ![
                    "bag",
                    "adaboostm1",
                    "adaboostm2",
                    "gentleboost",
                    "logitboost",
                    "rusboost",
                ]
                .contains(&method.as_str())
                {
                    return Err(format!("{name}: unsupported Method"));
                }
            }
            "numlearningcycles" => {
                cycles = integer(e, name, v)?;
                if cycles > 1000 {
                    return Err(format!("{name}: at most 1000 learning cycles"));
                }
            }
            "learnrate" => {
                rate = scalar(e, name, v)?;
                if rate <= 0. || rate > 1. {
                    return Err(format!("{name}: LearnRate must be in (0,1]"));
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(FitOptions::Advanced(Box::new(match name {
        "fitcdiscr" => {
            if ![
                "linear",
                "quadratic",
                "diaglinear",
                "diagquadratic",
                "pseudolinear",
                "pseudoquadratic",
            ]
            .contains(&discrim.as_str())
            {
                return Err("fitcdiscr: unsupported DiscrimType".into());
            }
            if gamma != 0. && discrim != "linear" {
                return Err("fitcdiscr: Gamma requires linear DiscrimType".into());
            }
            Config::Discriminant {
                kind: discrim,
                gamma,
                prior,
                uniform,
            }
        }
        "fitclinear" => Config::Linear {
            logistic,
            lasso,
            lambda,
        },
        "fitcecoc" => Config::Ecoc {
            ova,
            learner: Box::new(learner),
        },
        _ => Config::Ensemble {
            splits: splits.unwrap_or(if method == "bag" { 20000 } else { 1 }),
            method,
            cycles,
            rate,
            leaf,
        },
    })))
}
// Templates are immutable value structs, carrying only explicitly supplied options.
pub(in crate::engine) fn template(e: &Engine, name: &str, a: &[Value]) -> Result<Value> {
    let fit = match name {
        "templateSVM" => "svm",
        "templateTree" => "tree",
        "templateLinear" => "linear",
        _ => "discriminant",
    };
    let allowed: &[&str] = match fit {
        "svm" => &[
            "boxconstraint",
            "kernelfunction",
            "kernelscale",
            "standardize",
        ],
        "tree" => &[
            "maxnumsplits",
            "minleafsize",
            "minparentsize",
            "predictorselection",
        ],
        "linear" => &["learner", "lambda", "regularization", "solver"],
        _ => &["discrimtype", "gamma", "prior"],
    };
    options(name, a, allowed)?;
    let mut fields = vec!["LearnerType".into()];
    let mut slots = vec![Arc::new(Array::Character(crate::CharacterRow::from_text(
        fit, &e.cancel,
    )?))];
    for pair in a.as_chunks::<2>().0 {
        fields.push(e.text_value(pair[0].clone())?.to_ascii_lowercase());
        slots.push(Arc::new(pair[1].clone().array(&e.cancel)?));
    }
    let result = Value::Container(Array::Struct(crate::StructArray::from_slots(
        1, 1, fields, slots, &e.cancel,
    )?));
    template_config(e, &result)?;
    Ok(result)
}
pub(in crate::engine) fn template_parts(e: &Engine, v: &Value) -> Result<(String, Vec<Value>)> {
    let mut tail = Vec::new();
    let kind = if let Value::Container(Array::Struct(s)) = v {
        if s.len() != 1 {
            return Err("Learners: expected a scalar template".into());
        }
        let i = s
            .field_index("LearnerType")
            .ok_or("Learners: expected a learner template")?;
        let kind = string(e, &Value::from(s.get(i, 0)?.as_ref().clone()), "Learners")?;
        for (j, key) in s.field_names().iter().enumerate() {
            if j != i {
                tail.push(Value::Text(key.clone()));
                tail.push(Value::from(s.get(j, 0)?.as_ref().clone()));
            }
        }
        kind
    } else {
        string(e, v, "Learners")?
    };
    Ok((kind, tail))
}
fn template_config(e: &Engine, v: &Value) -> Result<FitOptions> {
    let (kind, tail) = template_parts(e, v)?;
    match kind.as_str() {
        "linear" => config(e, "fitclinear", &tail),
        "discriminant" => config(e, "fitcdiscr", &tail),
        "naivebayes" => Ok(FitOptions::Nb),
        "knn" => Ok(FitOptions::Knn {
            neighbors: 1,
            metric: "euclidean".into(),
            standardize: false,
        }),
        "tree" => {
            let mut splits = 20000;
            let mut leaf = 1;
            let mut parent = 2;
            for (k, v) in options(
                "templateTree",
                &tail,
                &[
                    "maxnumsplits",
                    "minleafsize",
                    "minparentsize",
                    "predictorselection",
                ],
            )? {
                if k == "predictorselection" {
                    if string(e, v, "templateTree")? != "allsplits" {
                        return Err(
                            "templateTree: only PredictorSelection allsplits is supported".into(),
                        );
                    }
                    continue;
                }
                let z = if k == "maxnumsplits" {
                    let z = scalar(e, "templateTree", v)?;
                    if !(0. ..=20000.).contains(&z) || z.fract() != 0. {
                        return Err(
                            "templateTree: MaxNumSplits must be an integer from 0 to 20000".into(),
                        );
                    }
                    z as usize
                } else {
                    integer(e, "templateTree", v)?
                };
                match k.as_str() {
                    "maxnumsplits" => {
                        if z > 20000 {
                            return Err("templateTree: MaxNumSplits exceeds 20000".into());
                        }
                        splits = z
                    }
                    "minleafsize" => leaf = z,
                    _ => parent = z.max(2),
                }
            }
            Ok(FitOptions::Tree {
                max_splits: splits,
                min_leaf: leaf,
                min_parent: parent,
            })
        }
        "svm" => {
            let mut c = 1.;
            let mut kernel = "linear".to_string();
            let mut scale = 1.;
            let mut standardize = false;
            for (k, v) in options(
                "templateSVM",
                &tail,
                &[
                    "boxconstraint",
                    "kernelfunction",
                    "kernelscale",
                    "standardize",
                ],
            )? {
                match k.as_str() {
                    "boxconstraint" => c = scalar(e, "templateSVM", v)?,
                    "kernelfunction" => kernel = string(e, v, "templateSVM")?,
                    "kernelscale" => scale = scalar(e, "templateSVM", v)?,
                    _ => standardize = boolean(e, "templateSVM", v)?,
                }
            }
            if c <= 0. || scale <= 0. || !["linear", "rbf", "gaussian"].contains(&kernel.as_str()) {
                return Err("templateSVM: invalid kernel or positive scale/constraint".into());
            }
            if kernel == "gaussian" {
                kernel = "rbf".into();
            }
            Ok(FitOptions::Svm {
                c,
                kernel,
                scale,
                standardize,
            })
        }
        _ => Err("Learners: unsupported learner".into()),
    }
}
pub(in crate::engine) fn fit(e: &mut Engine, name: &str, a: &[Value]) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err(format!("{name}: expected predictors and response"));
    }
    let (response_arg, tail) = if matches!(a.first(), Some(Value::Container(Array::Table(_))))
        && string(e, &a[1], name).is_ok_and(|s| s == "responsevarname")
    {
        if a.len() < 3 {
            return Err(format!("{name}: missing ResponseVarName"));
        }
        (&a[2], &a[3..])
    } else {
        (&a[1], &a[2..])
    };
    let (x, names, response, y) = if let Value::Container(Array::Table(t)) = &a[0] {
        if response_arg.is_character()
            || matches!(response_arg, Value::Text(_) | Value::QuotedString(_))
        {
            let spec = e.text_value(response_arg.clone())?;
            let parts: Vec<_> = spec.split('~').collect();
            if parts.len() > 2 {
                return Err(format!("{name}: invalid formula"));
            }
            let response = parts[0].trim().to_string();
            let index = t
                .names()
                .iter()
                .position(|s| s == &response)
                .ok_or_else(|| format!("{name}: unknown response variable '{response}'"))?;
            let names: Vec<String> = if parts.len() == 2 {
                let rhs = parts[1].trim();
                if rhs == "." {
                    t.names()
                        .iter()
                        .filter(|s| *s != &response)
                        .cloned()
                        .collect()
                } else {
                    rhs.split('+').map(|s| s.trim().to_string()).collect()
                }
            } else {
                t.names()
                    .iter()
                    .filter(|s| *s != &response)
                    .cloned()
                    .collect()
            };
            if names.is_empty() || names.iter().any(|s| s == &response) {
                return Err(format!("{name}: invalid predictors in formula"));
            }
            let (x, _) = predictor_matrix(e, &a[0], Some(&names), name)?;
            (
                x,
                names,
                response,
                super::classifiers::labels(e, &Value::from(t.column(index).clone()), name)?,
            )
        } else {
            let (x, names) = predictor_matrix(e, &a[0], None, name)?;
            (
                x,
                names,
                "Y".into(),
                super::classifiers::labels(e, response_arg, name)?,
            )
        }
    } else {
        let (x, names) = predictor_matrix(e, &a[0], None, name)?;
        (
            x,
            names,
            "Y".into(),
            super::classifiers::labels(e, response_arg, name)?,
        )
    };
    if x.rows() != y.len() {
        return Err(format!(
            "{name}: X and Y must have the same number of observations"
        ));
    }
    let keep: Vec<_> = (0..y.len())
        .filter(|&i| match &y {
            Labels::Numeric(v) => !v[i].is_nan(),
            Labels::Typed(v) => !v.real(i).is_nan(),
            _ => true,
        })
        .collect();
    let x = if keep.len() == y.len() {
        x
    } else {
        crate::engine::classifier_objects::take_rows(&x, &keep)?
    };
    let y = if keep.len() == y.len() {
        y
    } else {
        y.subset(&keep)?
    };
    if y.class_indices().0.len() < 2 {
        return Err(format!(
            "{name}: at least two response classes are required"
        ));
    }
    let mut tail = tail.to_vec();
    if name == "fitcensemble"
        && y.class_indices().0.len() == 2
        && !tail.as_chunks::<2>().0.iter().any(|pair| {
            e.text_value(pair[0].clone())
                .is_ok_and(|s| s.eq_ignore_ascii_case("Method"))
        })
    {
        tail.extend([
            Value::Text("Method".into()),
            Value::Text("LogitBoost".into()),
        ]);
    }
    let opts = config(e, name, &tail)?;
    let model = super::classifiers::train(e, x, y, names, response, opts)?;
    Ok(vec![e.classifier_new(model)?])
}
pub(in crate::engine) fn train(
    e: &mut Engine,
    x: &Matrix,
    codes: &[usize],
    classes: usize,
    config: &Config,
) -> Result<Model> {
    match config {
        Config::Discriminant {
            kind,
            gamma,
            prior,
            uniform,
        } => Ok(Model::Discriminant(super::discriminant::fit(
            e,
            x,
            codes,
            classes,
            kind,
            *gamma,
            prior.as_deref(),
            *uniform,
        )?)),
        Config::Linear {
            logistic,
            lasso,
            lambda,
        } => {
            if classes != 2 {
                return Err("fitclinear: exactly two response classes are required".into());
            }
            let (beta, bias) = super::linear_classifier::fit(
                e,
                x,
                codes,
                *logistic,
                *lasso,
                lambda.unwrap_or(1. / x.rows() as f64),
            )?;
            Ok(Model::Linear {
                beta,
                bias,
                logistic: *logistic,
            })
        }
        Config::Ecoc { ova, learner } => super::ecoc::fit(e, x, codes, classes, *ova, learner),
        Config::Ensemble {
            method,
            cycles,
            rate,
            splits,
            leaf,
        } => Ok(Model::Ensemble(super::ensemble::fit(
            e, x, codes, classes, method, *cycles, *rate, *splits, *leaf,
        )?)),
    }
}
pub(in crate::engine) fn scores(e: &Engine, m: &Model, x: &Matrix) -> Result<Matrix> {
    match m {
        Model::Discriminant(m) => m.predict(e, x),
        Model::Linear {
            beta,
            bias,
            logistic,
        } => {
            let mut s = vec![0.; x.rows() * 2];
            for i in 0..x.rows() {
                if i.is_multiple_of(256) {
                    e.check_cancel()?;
                }
                let f = bias
                    + beta
                        .iter()
                        .enumerate()
                        .map(|(j, b)| b * x.data()[i + j * x.rows()])
                        .sum::<f64>();
                let z = if *logistic { sigmoid(f) } else { f };
                s[i] = if *logistic { 1. - z } else { -z };
                s[i + x.rows()] = z;
            }
            Matrix::from_column_vec(x.rows(), 2, s)
        }
        Model::Ecoc {
            learners,
            coding,
            probability,
        } => super::ecoc::scores(e, learners, coding, *probability, x),
        Model::Ensemble(m) => m.predict(e, x, None),
    }
}
pub(in crate::engine) fn sigmoid(x: f64) -> f64 {
    if x >= 0. {
        1. / (1. + (-x).exp())
    } else {
        let z = x.exp();
        z / (1. + z)
    }
}
pub(in crate::engine) fn softplus(x: f64) -> f64 {
    x.max(0.) + (-x.abs()).exp().ln_1p()
}
pub(in crate::engine) fn property(e: &Engine, m: &Model, name: &str) -> Result<Value> {
    match m {
        Model::Discriminant(m) => m.property(e, name),
        Model::Linear { beta, bias, .. } => match name {
            "Beta" => value(beta.len(), 1, beta.clone()),
            "Bias" => Ok(Value::Matrix(Matrix::scalar(*bias))),
            _ => Err(format!("ClassificationLinear: unknown property '{name}'")),
        },
        Model::Ecoc { coding, .. } if name == "CodingMatrix" => {
            let k = coding.len();
            let b = coding[0].len();
            value(
                k,
                b,
                (0..b)
                    .flat_map(|j| coding.iter().map(move |v| v[j]))
                    .collect(),
            )
        }
        Model::Ensemble(m) => m.property(name),
        _ => Err(format!("{}: unknown property '{name}'", m.class())),
    }
}
pub(in crate::engine) fn property_names(m: &Model) -> Vec<&'static str> {
    match m {
        Model::Discriminant(_) => vec!["Mu", "Sigma", "Coeffs", "Prior"],
        Model::Linear { .. } => vec!["Beta", "Bias"],
        Model::Ecoc { .. } => vec!["CodingMatrix"],
        Model::Ensemble(_) => vec!["TrainedWeights", "NumTrained", "Method"],
    }
}
