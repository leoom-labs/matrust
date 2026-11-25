//! Numeric learner models, derived from public function contracts and published
//! algorithms. State is immutable and shared by the object table.
use super::*;
use crate::{CellArray, CharacterRow, LogicalMatrix, StructArray};
use nalgebra::{DVector, linalg::Cholesky};
use std::sync::Arc;

mod gp;
mod linear;
mod svm;
mod tree;
mod validation;

#[derive(Clone, Debug)]
struct TreeOptions {
    leaf: usize,
    parent: usize,
    splits: usize,
}
impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            leaf: 1,
            parent: 10,
            splits: 20_000,
        }
    }
}
#[derive(Clone, Debug)]
struct SvmOptions {
    kernel: String,
    scale: Option<f64>,
    order: i32,
    c: Option<f64>,
    epsilon: Option<f64>,
    standardize: bool,
}
#[derive(Clone, Debug)]
struct GpOptions {
    kernel: String,
    basis: String,
    sigma: Option<f64>,
    parameters: Option<Vec<f64>>,
}
#[derive(Clone, Debug)]
enum FitOptions {
    Tree(TreeOptions),
    Svm(SvmOptions),
    Gp(GpOptions),
    Ensemble {
        bag: bool,
        cycles: usize,
        rate: f64,
        tree: TreeOptions,
    },
    Linear {
        svm: bool,
        lasso: bool,
        lambda: Option<f64>,
        epsilon: Option<f64>,
    },
}
#[derive(Clone, Debug)]
enum Kind {
    Tree(tree::Tree),
    Svm(svm::Svm),
    Gp(Box<gp::Gp>),
    Ensemble(tree::Ensemble),
    Linear(linear::Linear),
    Partitioned {
        trained: Vec<Arc<Model>>,
        tests: Vec<Vec<usize>>,
    },
}
#[derive(Clone, Debug)]
pub(in crate::engine) struct Model {
    x: Matrix,
    y: Matrix,
    names: Vec<String>,
    response: String,
    options: FitOptions,
    kind: Kind,
}
impl Model {
    pub(in crate::engine) fn class(&self) -> &'static str {
        match self.kind {
            Kind::Tree(_) => "RegressionTree",
            Kind::Svm(_) => "RegressionSVM",
            Kind::Gp(_) => "RegressionGP",
            Kind::Ensemble(_) => "RegressionEnsemble",
            Kind::Linear(_) => "RegressionLinear",
            Kind::Partitioned { .. } => "RegressionPartitionedModel",
        }
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        let state = match &self.kind {
            Kind::Tree(t) => t.bytes(),
            Kind::Svm(s) => (s.alpha.len() + s.means.len() + s.scales.len()) * 8 + s.x.len() * 8,
            Kind::Gp(g) => g.bytes(),
            Kind::Ensemble(s) => {
                s.trees.iter().map(tree::Tree::bytes).sum::<usize>()
                    + s.inbag.iter().map(Vec::len).sum::<usize>()
            }
            Kind::Linear(s) => s.beta.len() * 8,
            Kind::Partitioned { trained, tests } => {
                trained.iter().map(|m| m.bytes()).sum::<usize>()
                    + tests.iter().map(|v| v.len() * 8).sum::<usize>()
            }
        };
        (self.x.len() + self.y.len()) * 8
            + state
            + 1024
            + self.names.iter().map(|s| s.len() + 24).sum::<usize>()
    }
    pub(in crate::engine) fn body(&self) -> String {
        format!(
            "  {} with properties:\n\n    NumObservations: {}\n    NumPredictors: {}\n    ResponseName: '{}'\n",
            self.class(),
            self.x.rows(),
            self.x.cols(),
            self.response
        )
    }
}

fn strings(e: &Engine, names: &[String]) -> Result<Value> {
    let cells = names
        .iter()
        .map(|s| CharacterRow::from_text(s, &e.cancel).map(|a| Arc::new(Array::Character(a))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        cells.len(),
        1,
        cells,
        &e.cancel,
    )?)))
}
fn structure(e: &Engine, fields: Vec<(&str, Value)>) -> Result<Value> {
    let names = fields.iter().map(|(n, _)| (*n).to_string()).collect();
    let slots = fields
        .into_iter()
        .map(|(_, v)| v.array(&e.cancel).map(Arc::new))
        .collect::<Result<_>>()?;
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?)))
}
fn opt_text(e: &Engine, v: &Value, name: &str) -> Result<String> {
    e.text_value(v.clone())
        .map(|s| s.to_ascii_lowercase())
        .map_err(|_| format!("{name}: expected a text option"))
}
fn nonnegative(e: &Engine, v: &Value, name: &str) -> Result<f64> {
    let v = scalar(e, name, v)?;
    if v < 0. {
        return Err(format!("{name}: value must be nonnegative"));
    }
    Ok(v)
}
fn positive(e: &Engine, v: &Value, name: &str) -> Result<f64> {
    let v = scalar(e, name, v)?;
    if v <= 0. {
        return Err(format!("{name}: value must be positive"));
    }
    Ok(v)
}
fn tree_option(e: &Engine, opts: &mut TreeOptions, key: &str, v: &Value) -> Result<bool> {
    match key {
        "minleafsize" => opts.leaf = integer(e, "MinLeafSize", v)?,
        "minparentsize" => {
            opts.parent = integer(e, "MinParentSize", v)?;
            if opts.parent < 2 {
                return Err("MinParentSize: must be at least 2".into());
            }
        }
        "maxnumsplits" => {
            let z = nonnegative(e, v, "MaxNumSplits")?;
            if z.fract() != 0. || z > 20_000. {
                return Err("MaxNumSplits: expected an integer from 0 to 20000".into());
            }
            opts.splits = z as usize;
        }
        "predictorselection" => {
            if opt_text(e, v, "PredictorSelection")? != "allsplits" {
                return Err("PredictorSelection: only allsplits is supported".into());
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
fn inputs(
    e: &Engine,
    name: &str,
    args: &[Value],
) -> Result<(Matrix, Matrix, Vec<String>, String, usize)> {
    use super::super::classifier_objects::{predictor_matrix, take_rows};
    if args.len() < 2 {
        return Err(format!("{name}: expected predictors and response"));
    }
    let mut used = 2;
    let (x, names, response, y) = if let Value::Container(Array::Table(t)) = &args[0]
        && let Ok(mut spec) = e.text_value(args[1].clone())
    {
        if spec.eq_ignore_ascii_case("ResponseVarName") {
            spec = e.text_value(
                args.get(2)
                    .ok_or_else(|| format!("{name}: missing ResponseVarName"))?
                    .clone(),
            )?;
            used = 3;
        }
        let (response, selected) = if let Some((r, predictors)) = spec.split_once('~') {
            let response = r.trim().to_string();
            if predictors.contains('~') {
                return Err(format!("{name}: unsupported formula"));
            }
            let selected = if predictors.trim() == "." {
                t.names()
                    .iter()
                    .filter(|n| *n != &response)
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                predictors
                    .split('+')
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>()
            };
            if selected.is_empty()
                || selected
                    .iter()
                    .any(|s| !crate::struct_array::valid_field_name(s) || s == &response)
                || selected
                    .iter()
                    .enumerate()
                    .any(|(i, s)| selected[..i].contains(s))
            {
                return Err(format!(
                    "{name}: unsupported formula; use response~predictor1+predictor2"
                ));
            }
            (response, selected)
        } else {
            let selected = t.names().iter().filter(|n| *n != &spec).cloned().collect();
            (spec, selected)
        };
        let i = t
            .names()
            .iter()
            .position(|n| n == &response)
            .ok_or_else(|| format!("{name}: unknown response variable '{response}'"))?;
        let (x, names) = predictor_matrix(e, &args[0], Some(&selected), name)?;
        let y = real(e, name, &Value::from(t.column(i).clone()))?;
        (x, names, response, y)
    } else {
        let (x, names) = predictor_matrix(e, &args[0], None, name)?;
        (x, names, "Y".to_string(), real(e, name, &args[1])?)
    };
    if y.len() != x.rows() || (y.rows() != 1 && y.cols() != 1) {
        return Err(format!(
            "{name}: X and Y must have the same number of rows; Y must be a vector"
        ));
    }
    if x.rows() == 0 || x.cols() == 0 {
        return Err(format!("{name}: expected nonempty data"));
    }
    if x.rows() > 20_000 || x.cols() > 512 || x.len() > 2_000_000 {
        return Err(format!(
            "{name}: training limit is 20000 rows, 512 predictors and 2000000 elements"
        ));
    }
    let keep = clean_rows(e, name, &x, Some(&y))?;
    if keep.is_empty() {
        return Err(format!("{name}: no complete observations"));
    }
    size(e, name, keep.len(), x.cols() + 1)?;
    if keep.len() == x.rows() && y.cols() == 1 {
        return Ok((x, y, names, response, used));
    }
    let yy = Matrix::from_column_vec(keep.len(), 1, keep.iter().map(|&i| y.data()[i]).collect())?;
    let xx = if keep.len() == x.rows() {
        x
    } else {
        take_rows(&x, &keep)?
    };
    Ok((xx, yy, names, response, used))
}

pub(in crate::engine) fn fit(e: &mut Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    let (x, y, names, response, used) = inputs(e, name, args)?;
    let mut opts = match name {
        "fitrtree" => FitOptions::Tree(TreeOptions::default()),
        "fitrsvm" => FitOptions::Svm(SvmOptions {
            kernel: "linear".into(),
            scale: Some(1.),
            order: 3,
            c: None,
            epsilon: None,
            standardize: false,
        }),
        "fitrgp" => FitOptions::Gp(GpOptions {
            kernel: "squaredexponential".into(),
            basis: "constant".into(),
            sigma: None,
            parameters: None,
        }),
        "fitrensemble" => FitOptions::Ensemble {
            bag: false,
            cycles: 100,
            rate: 1.,
            tree: TreeOptions {
                leaf: 5,
                parent: 10,
                splits: 10,
            },
        },
        "fitrlinear" => FitOptions::Linear {
            svm: true,
            lasso: false,
            lambda: None,
            epsilon: None,
        },
        _ => return Err(format!("{name}: unsupported regression learner")),
    };
    let allowed: &[&str] = match name {
        "fitrtree" => &[
            "minleafsize",
            "minparentsize",
            "maxnumsplits",
            "predictorselection",
        ],
        "fitrsvm" => &[
            "kernelfunction",
            "kernelscale",
            "polynomialorder",
            "boxconstraint",
            "epsilon",
            "standardize",
        ],
        "fitrgp" => &[
            "basis",
            "basisfunction",
            "kernelfunction",
            "fitmethod",
            "sigma",
            "kernelparameters",
        ],
        "fitrensemble" => &["method", "numlearningcycles", "learnrate", "learners"],
        _ => &["learner", "regularization", "lambda", "epsilon"],
    };
    let mut template = false;
    for (k, v) in options(name, &args[used..], allowed)? {
        match &mut opts {
            FitOptions::Tree(t) => {
                tree_option(e, t, &k, v)?;
            }
            FitOptions::Svm(s) => match k.as_str() {
                "kernelfunction" => {
                    s.kernel = opt_text(e, v, name)?;
                    if s.kernel == "gaussian" {
                        s.kernel = "rbf".into();
                    }
                    if !["linear", "rbf", "polynomial"].contains(&s.kernel.as_str()) {
                        return Err(format!("{name}: unsupported KernelFunction"));
                    }
                }
                "kernelscale" => {
                    s.scale = if e
                        .text_value(v.clone())
                        .is_ok_and(|s| s.eq_ignore_ascii_case("auto"))
                    {
                        None
                    } else {
                        Some(positive(e, v, "KernelScale")?)
                    };
                    if s.scale
                        .is_some_and(|s| !s.powi(2).is_finite() || s.powi(2) == 0.)
                    {
                        return Err("KernelScale: invalid scale".into());
                    }
                }
                "polynomialorder" => {
                    let n = integer(e, name, v)?;
                    if n > 16 {
                        return Err("PolynomialOrder: maximum supported order is 16".into());
                    }
                    s.order = n as i32;
                }
                "boxconstraint" => s.c = Some(positive(e, v, "BoxConstraint")?),
                "epsilon" => s.epsilon = Some(nonnegative(e, v, "Epsilon")?),
                "standardize" => s.standardize = boolean(e, name, v)?,
                _ => unreachable!(),
            },
            FitOptions::Gp(g) => match k.as_str() {
                "basis" | "basisfunction" => {
                    g.basis = opt_text(e, v, name)?;
                    if !["constant", "none", "linear"].contains(&g.basis.as_str()) {
                        return Err(format!("{name}: unsupported Basis"));
                    }
                }
                "kernelfunction" => {
                    g.kernel = opt_text(e, v, name)?;
                    if ![
                        "squaredexponential",
                        "exponential",
                        "matern32",
                        "matern52",
                        "rationalquadratic",
                    ]
                    .contains(&g.kernel.as_str())
                    {
                        return Err(format!("{name}: unsupported KernelFunction"));
                    }
                }
                "fitmethod" => {
                    if opt_text(e, v, name)? != "exact" {
                        return Err(format!("{name}: only FitMethod exact is supported"));
                    }
                }
                "sigma" => g.sigma = Some(positive(e, v, "Sigma")?),
                "kernelparameters" => {
                    let m = real(e, name, v)?;
                    if (m.rows() != 1 && m.cols() != 1)
                        || m.data().iter().any(|x| !x.is_finite() || *x <= 0.)
                    {
                        return Err("KernelParameters: expected a positive vector".into());
                    }
                    g.parameters = Some(m.data().to_vec());
                }
                _ => unreachable!(),
            },
            FitOptions::Ensemble {
                bag,
                cycles,
                rate,
                tree,
            } => match k.as_str() {
                "method" => {
                    let m = opt_text(e, v, name)?;
                    if !["bag", "lsboost"].contains(&m.as_str()) {
                        return Err(format!("{name}: Method must be LSBoost or Bag"));
                    }
                    *bag = m == "bag";
                }
                "numlearningcycles" => {
                    *cycles = integer(e, name, v)?;
                    if *cycles > 1000 {
                        return Err("NumLearningCycles: limit is 1000".into());
                    }
                }
                "learnrate" => {
                    *rate = positive(e, v, "LearnRate")?;
                    if *rate > 1. {
                        return Err("LearnRate: must be at most 1".into());
                    }
                }
                "learners" => {
                    let (kind, tail) = super::ml::template_parts(e, v)?;
                    if kind != "tree" {
                        return Err("Learners: expected tree or templateTree".into());
                    }
                    if matches!(v, Value::Container(Array::Struct(_))) {
                        *tree = TreeOptions::default();
                        for (key, value) in options(
                            "templateTree",
                            &tail,
                            &[
                                "maxnumsplits",
                                "minleafsize",
                                "minparentsize",
                                "predictorselection",
                            ],
                        )? {
                            tree_option(e, tree, &key, value)?;
                        }
                        template = true;
                    }
                }
                _ => unreachable!(),
            },
            FitOptions::Linear {
                svm,
                lasso,
                lambda,
                epsilon,
            } => match k.as_str() {
                "learner" => {
                    let s = opt_text(e, v, name)?;
                    if !["svm", "leastsquares"].contains(&s.as_str()) {
                        return Err("Learner: expected leastsquares or svm".into());
                    }
                    *svm = s == "svm";
                }
                "regularization" => {
                    let s = opt_text(e, v, name)?;
                    if !["ridge", "lasso"].contains(&s.as_str()) {
                        return Err("Regularization: expected ridge or lasso".into());
                    }
                    *lasso = s == "lasso";
                }
                "lambda" => {
                    *lambda = if e
                        .text_value(v.clone())
                        .is_ok_and(|s| s.eq_ignore_ascii_case("auto"))
                    {
                        None
                    } else {
                        Some(nonnegative(e, v, "Lambda")?)
                    }
                }
                "epsilon" => *epsilon = Some(nonnegative(e, v, "Epsilon")?),
                _ => unreachable!(),
            },
        }
    }
    if let FitOptions::Ensemble {
        bag: true, tree, ..
    } = &mut opts
        && !template
    {
        tree.splits = 20_000;
    }
    let m = train(e, x, y, names, response, opts)?;
    super::super::objects::mlr_new(e, m).map(|m| vec![m])
}
fn train(
    e: &mut Engine,
    x: Matrix,
    y: Matrix,
    names: Vec<String>,
    response: String,
    options: FitOptions,
) -> Result<Model> {
    e.check_cancel()?;
    let kind = match &options {
        FitOptions::Tree(t) => Kind::Tree(tree::train(
            e,
            &x,
            y.data(),
            &(0..x.rows()).collect::<Vec<_>>(),
            t,
        )?),
        FitOptions::Svm(s) => Kind::Svm(svm::train(e, &x, y.data(), s)?),
        FitOptions::Gp(g) => Kind::Gp(Box::new(gp::train(e, &x, y.data(), g)?)),
        FitOptions::Ensemble {
            bag,
            cycles,
            rate,
            tree,
        } => Kind::Ensemble(tree::ensemble(
            e,
            &x,
            y.data(),
            (*bag, *cycles, *rate),
            tree,
        )?),
        FitOptions::Linear {
            svm,
            lasso,
            lambda,
            epsilon,
        } => Kind::Linear(linear::train(
            e,
            &x,
            y.data(),
            *svm,
            *lasso,
            lambda.unwrap_or(1. / x.rows() as f64),
            *epsilon,
        )?),
    };
    let m = Model {
        x,
        y,
        names,
        response,
        options,
        kind,
    };
    e.preflight_elements(m.bytes().div_ceil(8))?;
    Ok(m)
}
fn means_scales(x: &Matrix) -> (Vec<f64>, Vec<f64>) {
    let n = x.rows();
    let means: Vec<_> = (0..x.cols())
        .map(|j| x.data()[j * n..(j + 1) * n].iter().sum::<f64>() / n as f64)
        .collect();
    let scales = (0..x.cols())
        .map(|j| {
            let sd = (x.data()[j * n..(j + 1) * n]
                .iter()
                .map(|v| (v - means[j]).powi(2))
                .sum::<f64>()
                / n.saturating_sub(1).max(1) as f64)
                .sqrt();
            if sd == 0. { 1. } else { sd }
        })
        .collect();
    (means, scales)
}
fn iqr(y: &[f64]) -> f64 {
    let mut v = y.to_vec();
    v.sort_by(f64::total_cmp);
    let q = |p: f64| {
        let i = (p * v.len() as f64 - 0.5).clamp(0., (v.len() - 1) as f64);
        let k = i.floor() as usize;
        v[k] + (i - k as f64) * (v[(k + 1).min(v.len() - 1)] - v[k])
    };
    q(0.75) - q(0.25)
}
fn default_epsilon(y: &[f64]) -> f64 {
    let r = iqr(y);
    if r > 0. { r / 13.49 } else { 0.1 }
}
fn predict_rows(e: &Engine, m: &Model, x: &Matrix) -> Result<Vec<f64>> {
    if x.cols() != m.x.cols() {
        return Err("predict: number of predictors does not match the model".into());
    }
    size(e, "predict", x.rows(), 1)?;
    if matches!(m.kind, Kind::Svm(_) | Kind::Gp(_))
        && x.rows()
            .saturating_mul(m.x.rows())
            .saturating_mul(m.x.cols())
            > 100_000_000
    {
        return Err("predict: kernel prediction work limit exceeded".into());
    }
    let mut result = Vec::with_capacity(x.rows());
    for i in 0..x.rows() {
        if i.is_multiple_of(64) {
            e.check_cancel()?;
        }
        let q = row(x, i);
        if q.iter().any(|v| v.is_infinite()) {
            return Err("predict: infinite predictors are not supported".into());
        }
        let v = if q.iter().any(|v| v.is_nan()) {
            f64::NAN
        } else {
            match &m.kind {
                Kind::Tree(t) => t.predict(&q),
                Kind::Svm(s) => s.predict(&q),
                Kind::Gp(g) => g.mean(&q),
                Kind::Ensemble(s) => s.predict(&q),
                Kind::Linear(s) => s.bias + s.beta.iter().zip(&q).map(|(a, b)| a * b).sum::<f64>(),
                Kind::Partitioned { .. } => {
                    return Err("predict: use kfoldPredict for a partitioned model".into());
                }
            }
        };
        result.push(v);
    }
    Ok(result)
}
pub(in crate::engine) fn predict(e: &Engine, m: &Model, args: &[Value]) -> Result<Vec<Value>> {
    let [query] = args else {
        return Err("predict: expected model and predictors".into());
    };
    let (x, _) =
        super::super::classifier_objects::predictor_matrix(e, query, Some(&m.names), "predict")?;
    if let Kind::Gp(g) = &m.kind {
        if x.cols() != m.x.cols() {
            return Err("predict: number of predictors does not match the model".into());
        }
        size(e, "predict", x.rows(), 4)?;
        if x.rows()
            .saturating_mul(m.x.rows())
            .saturating_mul(m.x.rows() + m.x.cols())
            > 100_000_000
        {
            return Err("predict: GP uncertainty work limit exceeded".into());
        }
        let mut y = Vec::new();
        let mut sd = Vec::new();
        for i in 0..x.rows() {
            e.check_cancel()?;
            let q = row(&x, i);
            if q.iter().any(|x| x.is_infinite()) {
                return Err("predict: infinite predictors are not supported".into());
            }
            let (a, b) = if q.iter().any(|x| x.is_nan()) {
                (f64::NAN, f64::NAN)
            } else {
                g.predict(&q)
            };
            y.push(a);
            sd.push(b);
        }
        let ci = y
            .iter()
            .zip(&sd)
            .map(|(a, b)| a - 1.959963984540054 * b)
            .chain(y.iter().zip(&sd).map(|(a, b)| a + 1.959963984540054 * b))
            .collect();
        return Ok(vec![
            value(x.rows(), 1, y)?,
            value(x.rows(), 1, sd)?,
            value(x.rows(), 2, ci)?,
        ]);
    }
    Ok(vec![value(x.rows(), 1, predict_rows(e, m, &x)?)?])
}
pub(in crate::engine) fn kfold_predict(
    e: &Engine,
    m: &Model,
    args: &[Value],
) -> Result<Vec<Value>> {
    if !args.is_empty() {
        return Err("kfoldPredict: expected one partitioned model".into());
    }
    Ok(vec![value(m.x.rows(), 1, validation::predictions(e, m)?)?])
}
pub(in crate::engine) fn property(e: &Engine, m: &Model, name: &str) -> Result<Value> {
    match name {
        "X" => return Ok(Value::Matrix(m.x.clone())),
        "Y" => return Ok(Value::Matrix(m.y.clone())),
        "NumObservations" => return value(1, 1, vec![m.x.rows() as f64]),
        "NumPredictors" => return value(1, 1, vec![m.x.cols() as f64]),
        "ResponseName" => return Ok(Value::Text(m.response.clone())),
        "PredictorNames" => return strings(e, &m.names),
        _ => {}
    }
    match &m.kind {
        Kind::Tree(t) => t.property(e, name, &m.names),
        Kind::Svm(s) => s.property(e, name),
        Kind::Gp(g) => g.property(e, name),
        Kind::Linear(s) => match name {
            "Beta" => value(s.beta.len(), 1, s.beta.clone()),
            "Bias" => value(1, 1, vec![s.bias]),
            _ => Err(format!("RegressionLinear: unknown property '{name}'")),
        },
        Kind::Ensemble(s) => match name {
            "NumTrained" => value(1, 1, vec![s.trees.len() as f64]),
            "Method" => Ok(Value::Text(if s.bag { "Bag" } else { "LSBoost" }.into())),
            _ => Err(format!("RegressionEnsemble: unknown property '{name}'")),
        },
        Kind::Partitioned { trained, .. } if name == "KFold" => {
            value(1, 1, vec![trained.len() as f64])
        }
        _ => Err(format!("{}: unknown property '{name}'", m.class())),
    }
}
pub(in crate::engine) fn method(
    e: &mut Engine,
    m: &Model,
    name: &str,
    args: &[Value],
) -> Result<Option<Value>> {
    let v = match name {
        "predict" => {
            let [query] = args else {
                return Err("predict: expected model and predictors".into());
            };
            let (x, _) = super::super::classifier_objects::predictor_matrix(
                e,
                query,
                Some(&m.names),
                "predict",
            )?;
            value(x.rows(), 1, predict_rows(e, m, &x)?)?
        }
        "crossval" => return validation::crossval(e, m, args).map(Some),
        "kfoldPredict" => kfold_predict(e, m, args)?.remove(0),
        "loss" | "resubLoss" | "kfoldLoss" => validation::loss(e, m, name, args)?,
        "view" => {
            let Kind::Tree(t) = &m.kind else {
                return Err("view: expected a RegressionTree".into());
            };
            for (k, v) in options(name, args, &["mode"])? {
                if k == "mode" && opt_text(e, v, name)? != "text" {
                    return Err("view: only text mode is supported".into());
                }
            }
            Value::Text(t.view(&m.names))
        }
        "prune" => return tree::prune(e, m, args).map(Some),
        "predictorImportance" => {
            if !args.is_empty() {
                return Err("predictorImportance: expected one model".into());
            }
            let imp = match &m.kind {
                Kind::Tree(t) => t.importance(m.x.cols()),
                Kind::Ensemble(s) => s.importance(m.x.cols()),
                _ => return Err("predictorImportance: expected a tree or ensemble".into()),
            };
            value(1, imp.len(), imp)?
        }
        "oobLoss" => tree::oob_loss(e, m, args)?,
        "properties" | "fieldnames" => {
            let mut fields = vec![
                "X",
                "Y",
                "NumObservations",
                "NumPredictors",
                "ResponseName",
                "PredictorNames",
            ];
            fields.extend(match &m.kind {
                Kind::Tree(_) => vec![
                    "Children",
                    "CutPoint",
                    "CutPredictor",
                    "NodeMean",
                    "NumNodes",
                ],
                Kind::Svm(_) => vec![
                    "Alpha",
                    "Bias",
                    "SupportVectors",
                    "IsSupportVector",
                    "Beta",
                    "Mu",
                    "Sigma",
                    "Epsilon",
                    "KernelParameters",
                ],
                Kind::Gp(_) => vec!["KernelInformation", "Beta", "Sigma", "LogLikelihood"],
                Kind::Linear(_) => vec!["Beta", "Bias"],
                Kind::Ensemble(_) => vec!["NumTrained", "Method"],
                Kind::Partitioned { .. } => vec!["KFold"],
            });
            strings(e, &fields.into_iter().map(String::from).collect::<Vec<_>>())?
        }
        _ => return Ok(None),
    };
    Ok(Some(v))
}
