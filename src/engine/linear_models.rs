//! Immutable numeric regression models. The shared pivoted QR kernel serves
//! OLS and Fisher scoring / IRLS; no normal-equation inverse is formed.
use super::stats_dist::families::Family;
use super::stats_models::regression::{LeastSquares, least_squares};
use super::{Engine, Execution, Value};
use crate::table_array::{TableArray, TableParts};
use crate::{Array, CellArray, CharacterRow, Matrix, Result, StructArray};
use nalgebra::{DMatrix, DVector};
use std::sync::Arc;

pub(super) const CLASSES: [&str; 2] = ["LinearModel", "GeneralizedLinearModel"];

type Term = Vec<(usize, u8)>;
const MAX_TERMS: usize = 256;
const MAX_OBSERVATIONS: usize = 1_000_000;
const MAX_DESIGN: usize = 2_000_000;
const MAX_QR_WORK: usize = 100_000_000;
const MAX_STEP_FITS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Distribution {
    Normal,
    Binomial,
    Poisson,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Link {
    Identity,
    Logit,
    Log,
    Probit,
}
impl Link {
    fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Logit => "logit",
            Self::Log => "log",
            Self::Probit => "probit",
        }
    }
    fn inverse(self, eta: f64) -> f64 {
        match self {
            Self::Identity => eta,
            Self::Log => eta.exp(),
            Self::Logit => {
                if eta >= 0. {
                    1. / (1. + (-eta).exp())
                } else {
                    let z = eta.exp();
                    z / (1. + z)
                }
            }
            Self::Probit => Family::Norm.cdf(eta, &[0., 1.], false),
        }
    }
    fn forward(self, mu: f64) -> f64 {
        match self {
            Self::Identity => mu,
            Self::Log => mu.ln(),
            Self::Logit => mu.ln() - (-mu).ln_1p(),
            Self::Probit => Family::Norm.inv(mu, &[0., 1.]),
        }
    }
    fn derivative(self, eta: f64, mu: f64) -> f64 {
        match self {
            Self::Identity => 1.,
            Self::Log => mu,
            Self::Logit => mu * (1. - mu),
            Self::Probit => (-eta * eta / 2.).exp() / (2. * std::f64::consts::PI).sqrt(),
        }
    }
}

#[derive(Debug)]
pub(super) struct Model {
    predictor_names: Vec<String>,
    terms: Vec<Term>,
    formula: String,
    coefficient_names: Vec<String>,
    coefficients: TableArray,
    beta: DVector<f64>,
    covariance: DMatrix<f64>,
    fitted: Matrix,
    residuals: TableArray,
    n: usize,
    rank: usize,
    dfe: usize,
    sse: f64,
    sst: f64,
    r2: f64,
    adjusted: f64,
    rmse: f64,
    fstat: f64,
    fprob: f64,
    log_likelihood: f64,
    null_log_likelihood: f64,
    null_deviance: f64,
    deviance: f64,
    dispersion: f64,
    distribution: Distribution,
    link: Link,
    generalized: bool,
}
impl Model {
    pub fn class(&self) -> &'static str {
        CLASSES[usize::from(self.generalized)]
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.beta.len() * 8
            + self.covariance.len() * 8
            + self.fitted.len() * 8
            + self.coefficients.bytes()
            + self.residuals.bytes()
            + self.formula.len()
            + self.predictor_names.iter().map(String::len).sum::<usize>()
            + self
                .coefficient_names
                .iter()
                .map(String::len)
                .sum::<usize>()
            + self
                .terms
                .iter()
                .map(|t| t.len() * std::mem::size_of::<(usize, u8)>())
                .sum::<usize>()
    }
    pub fn property_names(&self) -> Vec<String> {
        let mut names = [
            "Coefficients",
            "Rsquared",
            "RMSE",
            "NumObservations",
            "DFE",
            "SSE",
            "SST",
            "SSR",
            "Residuals",
            "Fitted",
            "CoefficientNames",
            "Formula",
            "LogLikelihood",
            "ModelCriterion",
        ]
        .map(String::from)
        .to_vec();
        if self.generalized {
            names.extend(["Deviance".into(), "Dispersion".into()]);
        }
        names
    }
    pub fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        let number = |x| Ok(Value::Matrix(Matrix::scalar(x)));
        match name {
            "Coefficients" => Ok(Value::Container(Array::Table(self.coefficients.clone()))),
            "Residuals" => Ok(Value::Container(Array::Table(self.residuals.clone()))),
            "Fitted" => Ok(Value::Matrix(self.fitted.clone())),
            "CoefficientNames" => names_value(e, &self.coefficient_names),
            "Formula" => Ok(Value::Text(self.formula.clone())),
            "Rsquared" if self.generalized => scalar_struct(
                e,
                &["Ordinary", "Adjusted", "LLR", "Deviance", "AdjGeneralized"],
                &[
                    self.r2,
                    self.adjusted,
                    1. - self.log_likelihood / self.null_log_likelihood,
                    1. - self.deviance / self.null_deviance,
                    // Nagelkerke's normalization of Cox-Snell; exp_m1 avoids
                    // cancellation when the fitted and null likelihoods agree.
                    (2. * (self.null_log_likelihood - self.log_likelihood) / self.n as f64)
                        .exp_m1()
                        / (2. * self.null_log_likelihood / self.n as f64).exp_m1(),
                ],
            ),
            "Rsquared" => scalar_struct(e, &["Ordinary", "Adjusted"], &[self.r2, self.adjusted]),
            "ModelCriterion" => {
                let k = self.rank as f64;
                scalar_struct(
                    e,
                    &["AIC", "BIC"],
                    &[
                        -2. * self.log_likelihood + 2. * k,
                        -2. * self.log_likelihood + (self.n as f64).ln() * k,
                    ],
                )
            }
            "RMSE" => number(self.rmse),
            "NumObservations" => number(self.n as f64),
            "DFE" => number(self.dfe as f64),
            "SSE" => number(self.sse),
            "SST" => number(self.sst),
            "SSR" => number(self.sst - self.sse),
            "LogLikelihood" => number(self.log_likelihood),
            "Deviance" if self.generalized => number(self.deviance),
            "Dispersion" if self.generalized => number(self.dispersion),
            _ => Err(format!("{}: unknown property '{name}'", self.class())),
        }
    }
    pub fn body(&self) -> String {
        let mut s = if self.generalized {
            format!(
                "Generalized linear regression model:\n    {}\n    Distribution = {:?}, Link = {}\n",
                self.formula,
                self.distribution,
                self.link.name()
            )
        } else {
            format!("Linear regression model:\n    {}\n", self.formula)
        };
        s.push_str("\nEstimated Coefficients:\n                      Estimate           SE        tStat       pValue\n");
        for (i, name) in self.coefficient_names.iter().enumerate() {
            s.push_str(&format!("    {name:<18}"));
            for j in 0..4 {
                if let Array::Numeric(m) = self.coefficients.column(j) {
                    s.push_str(&format!(" {:>12.6}", m.data()[i]));
                }
            }
            s.push('\n');
        }
        s.push_str(&format!(
            "\nNumber of observations: {}, Error degrees of freedom: {}\n",
            self.n, self.dfe
        ));
        if self.generalized {
            s.push_str(&format!(
                "Deviance: {:.6}\nDispersion: {:.6}\n",
                self.deviance, self.dispersion
            ));
        } else {
            s.push_str(&format!("Root Mean Squared Error: {:.6}\nR-squared: {:.6}, Adjusted R-Squared: {:.6}\nF-statistic vs. constant model: {:.6}, p-value = {:.6e}\n", self.rmse, self.r2, self.adjusted, self.fstat, self.fprob));
        }
        s
    }
}
fn names_value(e: &Engine, names: &[String]) -> Result<Value> {
    let items = names
        .iter()
        .map(|s| CharacterRow::from_text(s, &e.cancel).map(|c| Arc::new(Array::Character(c))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        1,
        items.len(),
        items,
        &e.cancel,
    )?)))
}
fn scalar_struct(e: &Engine, names: &[&str], numbers: &[f64]) -> Result<Value> {
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        names.iter().map(|s| (*s).into()).collect(),
        numbers
            .iter()
            .map(|&x| Arc::new(Array::Numeric(Matrix::scalar(x))))
            .collect(),
        &e.cancel,
    )?)))
}
fn real(e: &Engine, name: &str, v: &Value) -> Result<Matrix> {
    if v.is_character() || v.is_nd() || matches!(v, Value::Text(_) | Value::QuotedString(_)) {
        return Err(format!(
            "{name}: expected a real two-dimensional numeric array"
        ));
    }
    // Reject oversized sparse/typed inputs before any dense conversion.
    let (rows, cols) = v.shape(&e.cancel)?;
    let len = rows
        .checked_mul(cols)
        .ok_or_else(|| format!("{name}: input too large"))?;
    if rows.max(cols) > MAX_OBSERVATIONS || len > MAX_DESIGN {
        return Err(format!("{name}: input too large"));
    }
    e.preflight_elements(len)?;
    v.clone().matrix(&e.cancel)
}
fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}
fn scalar(e: &Engine, name: &str, v: &Value) -> Result<f64> {
    let x = real(e, name, v)?.as_scalar()?;
    if !x.is_finite() {
        return Err(format!("{name}: option must be a finite scalar"));
    }
    Ok(x)
}
fn table(
    names: &[&str],
    columns: Vec<Vec<f64>>,
    row_names: Option<Vec<String>>,
) -> Result<TableArray> {
    let n = columns.first().map_or(0, Vec::len);
    let columns = columns
        .into_iter()
        .map(|v| Matrix::from_column_vec(n, 1, v).map(Array::Numeric))
        .collect::<Result<Vec<_>>>()?;
    let mut parts = TableParts::new(names.iter().map(|s| (*s).into()).collect(), columns, n);
    parts.row_names = row_names;
    TableArray::new(parts)
}
fn bounds(e: &Engine, name: &str, n: usize, p: usize) -> Result<()> {
    let len = n
        .checked_mul(p)
        .ok_or_else(|| format!("{name}: design too large"))?;
    if n > MAX_OBSERVATIONS
        || p > MAX_TERMS
        || len > MAX_DESIGN
        || len.saturating_mul(p) > MAX_QR_WORK
    {
        return Err(format!(
            "{name}: design too large (limits: {MAX_OBSERVATIONS} rows, {MAX_TERMS} terms, {MAX_DESIGN} elements, {MAX_QR_WORK} QR work units)"
        ));
    }
    e.preflight_elements(
        len.saturating_mul(8)
            .saturating_add(p.saturating_mul(p).saturating_mul(8))
            .saturating_add(n.saturating_mul(12)),
    )?;
    e.check_cancel()
}
fn term_name(t: &Term, names: &[String]) -> String {
    if t.is_empty() {
        return "(Intercept)".into();
    }
    t.iter()
        .map(|&(j, p)| {
            if p == 1 {
                names[j].clone()
            } else {
                format!("{}^{p}", names[j])
            }
        })
        .collect::<Vec<_>>()
        .join(":")
}
fn normalize_terms(terms: &mut Vec<Term>) -> Result<()> {
    terms.sort_by(|a, b| {
        let degree = |t: &Term| t.iter().map(|(_, p)| *p as usize).sum::<usize>();
        degree(a).cmp(&degree(b)).then_with(|| a.cmp(b))
    });
    terms.dedup();
    if terms.len() > MAX_TERMS {
        return Err("model formula: too many expanded terms".into());
    }
    Ok(())
}
fn specification(spec: &str, names: &[String], response: &str) -> Result<Vec<Term>> {
    if spec.len() > 8192 {
        return Err("model formula: expression too large".into());
    }
    let mut terms = vec![vec![]];
    match spec.to_ascii_lowercase().as_str() {
        "constant" => return Ok(terms),
        "linear" | "interactions" | "quadratic" | "purequadratic" => {
            let kind = spec.to_ascii_lowercase();
            for j in 0..names.len() {
                terms.push(vec![(j, 1)]);
            }
            if kind == "interactions" || kind == "quadratic" {
                for j in 0..names.len() {
                    for k in j + 1..names.len() {
                        terms.push(vec![(j, 1), (k, 1)]);
                        if terms.len() > MAX_TERMS {
                            return Err("model formula: too many expanded terms".into());
                        }
                    }
                }
            }
            if kind == "quadratic" || kind == "purequadratic" {
                for j in 0..names.len() {
                    terms.push(vec![(j, 2)]);
                }
            }
        }
        _ => {
            let (lhs, rhs) = spec
                .split_once('~')
                .ok_or("model formula: expected a supported model specification or 'y ~ terms'")?;
            if lhs.trim() != response || rhs.contains('~') {
                return Err("model formula: invalid response or unsupported formula syntax".into());
            }
            // Do not remove whitespace inside identifiers: 'x 1' is invalid.
            let mut start = 0;
            let mut subtract = false;
            for (pos, ch) in rhs.char_indices().chain(std::iter::once((rhs.len(), '+'))) {
                if ch != '+' && ch != '-' {
                    continue;
                }
                let piece = rhs[start..pos].trim();
                if piece.is_empty() {
                    if start == 0 && ch == '-' {
                        subtract = true;
                        start = pos + 1;
                        continue;
                    }
                    return Err("model formula: empty term or unsupported syntax".into());
                }
                if piece == "1" || piece == "0" {
                    if subtract && piece != "1" {
                        return Err("model formula: only '-1' subtraction is supported".into());
                    }
                    if subtract || piece == "0" {
                        terms.retain(|t| !t.is_empty());
                    } else {
                        terms.push(vec![]);
                    }
                } else {
                    if subtract {
                        return Err("model formula: only '-1' subtraction is supported".into());
                    }
                    let variable = |s: &str| {
                        names.iter().position(|n| n == s.trim()).ok_or_else(|| {
                            format!("model formula: unsupported term or unknown predictor '{s}'")
                        })
                    };
                    if piece.contains('*') {
                        let factors: Vec<_> = piece.split('*').collect();
                        if factors.len() != 2 {
                            return Err("model formula: '*' supports two predictor names".into());
                        }
                        let (a, b) = (variable(factors[0])?, variable(factors[1])?);
                        if a == b {
                            return Err("model formula: use x^2 for a squared predictor".into());
                        }
                        terms.push(vec![(a, 1)]);
                        terms.push(vec![(b, 1)]);
                        let mut t = vec![(a, 1), (b, 1)];
                        t.sort();
                        terms.push(t);
                    } else if piece.contains('^') {
                        let (var, power) = piece.split_once('^').unwrap();
                        if power.trim() != "2" {
                            return Err("model formula: only power ^2 is supported".into());
                        }
                        let j = variable(var)?;
                        terms.push(vec![(j, 1)]);
                        terms.push(vec![(j, 2)]);
                    } else if piece.contains(':') {
                        let mut t = piece
                            .split(':')
                            .map(|s| variable(s).map(|j| (j, 1)))
                            .collect::<Result<Term>>()?;
                        t.sort();
                        if t.windows(2).any(|w| w[0].0 == w[1].0) {
                            return Err(
                                "model formula: repeated interaction factor is unsupported".into(),
                            );
                        }
                        terms.push(t);
                    } else {
                        terms.push(vec![(variable(piece)?, 1)]);
                    }
                }
                normalize_terms(&mut terms)?;
                subtract = ch == '-';
                start = pos + 1;
            }
        }
    }
    normalize_terms(&mut terms)?;
    if terms.is_empty() {
        return Err("model formula: at least one term is required".into());
    }
    Ok(terms)
}
struct Input {
    x: Matrix,
    y: Matrix,
    names: Vec<String>,
    response: String,
    terms: Vec<Term>,
    distribution: Distribution,
    link: Link,
    generalized: bool,
    p_enter: f64,
    p_remove: f64,
    upper: Vec<Term>,
}
fn parse(e: &Engine, name: &str, a: &[Value]) -> Result<Input> {
    let first = a
        .first()
        .ok_or_else(|| format!("{name}: expected predictors and response"))?;
    let is_table = matches!(first, Value::Container(Array::Table(_)));
    let mut tail = if is_table { 1 } else { 2 };
    let default = if name == "stepwiselm" {
        "constant"
    } else {
        "linear"
    };
    let spec = if a.get(tail).and_then(text).is_some_and(|s| {
        s.contains('~')
            || [
                "linear",
                "interactions",
                "quadratic",
                "purequadratic",
                "constant",
            ]
            .contains(&s.to_ascii_lowercase().as_str())
    }) {
        tail += 1;
        text(&a[tail - 1]).unwrap()
    } else {
        default
    };
    let (x, y, names, response) = if let Value::Container(Array::Table(t)) = first {
        if t.width() == 0 {
            return Err(format!("{name}: table needs a response variable"));
        }
        let response_index = if let Some((lhs, _)) = spec.split_once('~') {
            t.index_of(lhs.trim())
                .ok_or("model formula: response is not a table variable")?
        } else {
            t.width() - 1
        };
        if t.width() > MAX_TERMS || t.height() > MAX_OBSERVATIONS {
            return Err(format!("{name}: input table too large"));
        }
        let mut cols = Vec::new();
        let mut names = Vec::new();
        for j in 0..t.width() {
            if j == response_index {
                continue;
            }
            let col = real(e, name, &Value::from(t.column(j).clone()))?;
            if col.cols() != 1 {
                return Err(format!(
                    "{name}: table variables must be numeric scalar columns"
                ));
            }
            names.push(t.names()[j].clone());
            cols.push(col);
        }
        bounds(e, name, t.height(), cols.len().max(1))?;
        let mut data = Vec::with_capacity(t.height() * cols.len());
        for col in cols {
            data.extend_from_slice(col.data());
        }
        (
            Matrix::from_column_vec(t.height(), names.len(), data)?,
            real(e, name, &Value::from(t.column(response_index).clone()))?,
            names,
            t.names()[response_index].clone(),
        )
    } else {
        if a.len() < 2 {
            return Err(format!("{name}: expected X and y"));
        }
        let x = real(e, name, first)?;
        let y = real(e, name, &a[1])?;
        bounds(e, name, x.rows(), x.cols().max(1))?;
        let names = (1..=x.cols()).map(|j| format!("x{j}")).collect();
        (x, y, names, "y".into())
    };
    if y.len() != x.rows() || (y.rows() != 1 && y.cols() != 1) || y.is_empty() {
        return Err(format!(
            "{name}: response must be a vector with one entry per predictor row"
        ));
    }
    let mut terms = specification(spec, &names, &response)?;
    let mut intercept = None;
    let mut distribution = Distribution::Normal;
    let mut link_name = None;
    let mut p_enter = 0.05;
    let mut p_remove = 0.10;
    let mut upper_spec = "interactions";
    if !(a.len() - tail).is_multiple_of(2) {
        return Err(format!("{name}: options must be name/value pairs"));
    }
    for pair in a[tail..].as_chunks::<2>().0 {
        let key = text(&pair[0])
            .ok_or_else(|| format!("{name}: expected an option name"))?
            .to_ascii_lowercase();
        match key.as_str() {
            "intercept" => { let v = scalar(e,name,&pair[1])?; if v != 0. && v != 1. { return Err(format!("{name}: Intercept must be logical")); } intercept = Some(v == 1.); }
            "distribution" if name == "fitglm" => distribution = match text(&pair[1]).unwrap_or("").to_ascii_lowercase().as_str() {
                "normal" => Distribution::Normal, "binomial" => Distribution::Binomial, "poisson" => Distribution::Poisson,
                _ => return Err("fitglm: unsupported distribution; supported distributions are normal, binomial and poisson".into()),
            },
            "link" if name == "fitglm" => link_name = Some(text(&pair[1]).ok_or("fitglm: Link must be text")?.to_ascii_lowercase()),
            "penter" if name == "stepwiselm" => p_enter = scalar(e,name,&pair[1])?,
            "premove" if name == "stepwiselm" => p_remove = scalar(e,name,&pair[1])?,
            "upper" if name == "stepwiselm" => upper_spec = text(&pair[1]).ok_or("stepwiselm: Upper must be a model specification")?,
            _ => return Err(format!("{name}: unsupported option '{key}'")),
        }
    }
    if let Some(on) = intercept {
        terms.retain(|t| !t.is_empty());
        if on {
            terms.push(vec![]);
        }
        normalize_terms(&mut terms)?;
    }
    if terms.is_empty() {
        return Err(format!("{name}: model must contain at least one term"));
    }
    let link = match link_name.as_deref() {
        None => match distribution { Distribution::Normal => Link::Identity, Distribution::Binomial => Link::Logit, Distribution::Poisson => Link::Log },
        Some("identity") => Link::Identity, Some("logit") => Link::Logit, Some("log") => Link::Log, Some("probit") => Link::Probit,
        _ => return Err("fitglm: unsupported Link; supported links are identity, log, logit and probit with compatible distributions".into()),
    };
    if !matches!(
        (distribution, link),
        (Distribution::Normal, Link::Identity | Link::Log)
            | (Distribution::Binomial, Link::Logit | Link::Probit)
            | (Distribution::Poisson, Link::Log | Link::Identity)
    ) {
        return Err("fitglm: unsupported distribution/link combination".into());
    }
    let upper = if name == "stepwiselm" {
        if !(0. < p_enter && p_enter < p_remove && p_remove < 1.) {
            return Err("stepwiselm: require 0 < PEnter < PRemove < 1".into());
        }
        if names.len() > 16 {
            return Err("stepwiselm: at most 16 numeric predictors are supported".into());
        }
        let upper = specification(upper_spec, &names, &response)?;
        if !terms.iter().all(|t| upper.contains(t)) {
            return Err("stepwiselm: initial model must be within Upper".into());
        }
        if !terms.iter().any(Vec::is_empty) {
            return Err("stepwiselm: an intercept is required".into());
        }
        upper
    } else {
        Vec::new()
    };
    Ok(Input {
        x,
        y,
        names,
        response,
        terms,
        distribution,
        link,
        generalized: name == "fitglm",
        p_enter,
        p_remove,
        upper,
    })
}
fn complete_rows(e: &Engine, input: &Input, terms: &[Term]) -> Result<Vec<usize>> {
    let mut used = vec![false; input.x.cols()];
    for t in terms {
        for &(j, _) in t {
            used[j] = true;
        }
    }
    let mut keep = Vec::new();
    for i in 0..input.x.rows() {
        if i.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let y = input.y.data()[i];
        if y.is_infinite()
            || used
                .iter()
                .enumerate()
                .any(|(j, &use_it)| use_it && input.x.data()[i + j * input.x.rows()].is_infinite())
        {
            return Err("model fit: infinite data are not supported".into());
        }
        if y.is_nan()
            || used
                .iter()
                .enumerate()
                .any(|(j, &use_it)| use_it && input.x.data()[i + j * input.x.rows()].is_nan())
        {
            continue;
        }
        keep.push(i);
    }
    if keep.is_empty() {
        return Err("model fit: no complete observations".into());
    }
    Ok(keep)
}
fn design(e: &Engine, x: &Matrix, terms: &[Term], keep: &[usize]) -> Result<DMatrix<f64>> {
    bounds(e, "model", keep.len(), terms.len())?;
    let mut data = Vec::with_capacity(keep.len() * terms.len());
    for t in terms {
        for (r, &i) in keep.iter().enumerate() {
            if r.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let v = t
                .iter()
                .map(|&(j, p)| x.data()[i + j * x.rows()].powi(p as i32))
                .product::<f64>();
            if v.is_infinite() {
                return Err("model: predictor term overflow".into());
            }
            data.push(v);
        }
    }
    Ok(DMatrix::from_vec(keep.len(), terms.len(), data))
}
fn deviance(d: Distribution, y: &DVector<f64>, mu: &DVector<f64>) -> f64 {
    y.iter()
        .zip(mu.iter())
        .map(|(&y, &m)| match d {
            Distribution::Normal => (y - m).powi(2),
            Distribution::Poisson => {
                2. * (if y == 0. {
                    m
                } else {
                    y * (y / m).ln() - (y - m)
                })
            }
            Distribution::Binomial => {
                2. * ((if y == 0. { 0. } else { y * (y / m).ln() })
                    + (if y == 1. {
                        0.
                    } else {
                        (1. - y) * ((1. - y) / (1. - m)).ln()
                    }))
            }
        })
        .sum()
}
fn valid_means(d: Distribution, mu: &DVector<f64>) -> bool {
    mu.iter().all(|&m| {
        m.is_finite()
            && match d {
                Distribution::Normal => true,
                Distribution::Binomial => m > 0. && m < 1.,
                Distribution::Poisson => m > 0.,
            }
    })
}
fn weighted(
    e: &Engine,
    x: &DMatrix<f64>,
    y: &DVector<f64>,
    beta: &DVector<f64>,
    d: Distribution,
    link: Link,
) -> Result<LeastSquares> {
    let eta = x * beta;
    let mu = eta.map(|z| link.inverse(z));
    if !valid_means(d, &mu) {
        return Err(
            "fitglm: IRLS left the supported mean-response domain (possibly separated data)".into(),
        );
    }
    let mut wx = x.clone();
    let mut z = DVector::zeros(y.len());
    for i in 0..y.len() {
        if i.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let derivative = link.derivative(eta[i], mu[i]);
        let variance = match d {
            Distribution::Normal => 1.,
            Distribution::Binomial => mu[i] * (1. - mu[i]),
            Distribution::Poisson => mu[i],
        };
        let weight = derivative / variance.sqrt();
        if !weight.is_finite() || weight <= 1e-15 {
            return Err("fitglm: singular IRLS weights (possibly separated data)".into());
        }
        z[i] = (eta[i] + (y[i] - mu[i]) / derivative) * weight;
        for j in 0..x.ncols() {
            wx[(i, j)] *= weight;
        }
    }
    least_squares(e, &wx, &z)
}
fn irls(
    e: &Engine,
    x: &DMatrix<f64>,
    y: &DVector<f64>,
    d: Distribution,
    link: Link,
) -> Result<LeastSquares> {
    if (d == Distribution::Binomial && y.iter().any(|&v| v < 0. || v > 1.))
        || (d == Distribution::Poisson && y.iter().any(|&v| v < 0. || v.fract() != 0.))
    {
        return Err("fitglm: binomial responses must be in [0,1]; poisson responses must be nonnegative integer counts".into());
    }
    if d == Distribution::Normal && link == Link::Identity {
        return least_squares(e, x, y);
    }
    // A data-based starting mean avoids zero starting weights and permits log
    // links without taking log(0). The first QR projects it onto the design.
    let mean = y.sum() / y.len() as f64;
    let initial = y.map(|v| {
        link.forward(match d {
            Distribution::Binomial => (v + 0.5) / 2.,
            Distribution::Poisson => (v + mean + 0.1) / 2.,
            Distribution::Normal => v.max(0.1),
        })
    });
    let mut beta = least_squares(e, x, &initial)?.b;
    let mut mu = (x * &beta).map(|z| link.inverse(z));
    if !valid_means(d, &mu) {
        return Err("fitglm: could not initialize the requested link".into());
    }
    let mut old = deviance(d, y, &mu);
    for _ in 0..100 {
        e.check_cancel()?;
        let fit = weighted(e, x, y, &beta, d, link)?;
        if fit.rank < x.ncols() {
            return Err("fitglm: rank deficient weighted design".into());
        }
        let delta = &fit.b - &beta;
        let mut fraction = 1.;
        let mut candidate = fit.b;
        let mut next;
        loop {
            mu = (x * &candidate).map(|z| link.inverse(z));
            next = deviance(d, y, &mu);
            if valid_means(d, &mu) && next.is_finite() && next <= old + 1e-10 * (1. + old.abs()) {
                break;
            }
            fraction *= 0.5;
            if fraction < 1e-12 {
                return Err("fitglm: IRLS failed to find a decreasing deviance".into());
            }
            candidate = &beta + fraction * &delta;
        }
        let relative = (&candidate - &beta).norm() / (1. + beta.norm());
        let change = (next - old).abs() / (1. + old.abs());
        beta = candidate;
        old = next;
        if relative < 1e-9 && change < 1e-10 {
            let mut fit = weighted(e, x, y, &beta, d, link)?;
            fit.b = beta;
            return Ok(fit);
        }
    }
    Err("fitglm: IRLS did not converge within 100 iterations (possibly separated data)".into())
}
// Evaluate fitted and constant-only likelihoods with the same dispersion,
// matching statsmodels llf_scaled(scale=dispersion) / llnull.
fn model_log_likelihood(
    d: Distribution,
    y: &DVector<f64>,
    mu: &DVector<f64>,
    dispersion: f64,
) -> f64 {
    match d {
        Distribution::Normal => {
            let sse = y
                .iter()
                .zip(mu.iter())
                .map(|(&v, &m)| (v - m).powi(2))
                .sum::<f64>();
            if dispersion == 0. {
                if sse == 0. {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                }
            } else {
                -0.5 * (y.len() as f64 * (2. * std::f64::consts::PI * dispersion).ln()
                    + sse / dispersion)
            }
        }
        Distribution::Poisson => y
            .iter()
            .zip(mu.iter())
            .map(|(&v, &m)| (if v == 0. { 0. } else { v * m.ln() }) - m - libm::lgamma(v + 1.))
            .sum(),
        Distribution::Binomial => y
            .iter()
            .zip(mu.iter())
            .map(|(&v, &m)| {
                (if v == 0. { 0. } else { v * m.ln() })
                    + (if v == 1. { 0. } else { (1. - v) * (-m).ln_1p() })
            })
            .sum(),
    }
}
fn fit_model(e: &Engine, input: &Input, keep: &[usize]) -> Result<Model> {
    let x = design(e, &input.x, &input.terms, keep)?;
    let y = DVector::from_iterator(keep.len(), keep.iter().map(|&i| input.y.data()[i]));
    let fit = irls(e, &x, &y, input.distribution, input.link)?;
    let eta = &x * &fit.b;
    let mu = eta.map(|z| input.link.inverse(z));
    if mu.iter().any(|v| !v.is_finite()) {
        return Err("model fit: nonfinite fitted response".into());
    }
    let (n, p) = x.shape();
    let dfe = n.saturating_sub(fit.rank);
    if dfe == 0 {
        return Err("model fit: positive residual degrees of freedom required".into());
    }
    let residual = &y - &mu;
    let sse = residual.norm_squared();
    let intercept = input.terms.iter().any(Vec::is_empty);
    let center = if intercept || input.generalized {
        y.sum() / n as f64
    } else {
        0.
    };
    let sst = y.iter().map(|v| (v - center).powi(2)).sum::<f64>();
    let r2 = 1. - sse / sst;
    let adjusted =
        1. - (1. - r2) * (n - usize::from(intercept || input.generalized)) as f64 / dfe as f64;
    let dispersion = if input.distribution == Distribution::Normal {
        sse / dfe as f64
    } else {
        1.
    };
    let covariance = (&fit.covariance_factor * fit.covariance_factor.transpose()) * dispersion;
    let se: Vec<_> = (0..p)
        .map(|j| {
            if covariance[(j, j)] == 0. && fit.rank < p {
                f64::NAN
            } else {
                covariance[(j, j)].sqrt()
            }
        })
        .collect();
    let tstat: Vec<_> = (0..p).map(|j| fit.b[j] / se[j]).collect();
    let pval: Vec<_> = tstat
        .iter()
        .map(|t| {
            2. * if input.distribution == Distribution::Normal {
                Family::T.cdf(t.abs(), &[dfe as f64], true)
            } else {
                Family::Norm.cdf(t.abs(), &[0., 1.], true)
            }
        })
        .collect();
    let coefficient_names: Vec<_> = input
        .terms
        .iter()
        .map(|t| term_name(t, &input.names))
        .collect();
    let coefficients = table(
        &["Estimate", "SE", "tStat", "pValue"],
        vec![fit.b.as_slice().to_vec(), se, tstat, pval],
        Some(coefficient_names.clone()),
    )?;
    let mut fitted = vec![f64::NAN; input.x.rows()];
    let mut raw = fitted.clone();
    for (i, &r) in keep.iter().enumerate() {
        fitted[r] = mu[i];
        raw[r] = residual[i];
    }
    let rmse = (sse / dfe as f64).sqrt();
    let model_df = fit.rank.saturating_sub(usize::from(intercept));
    let fstat = (sst - sse).max(0.) / model_df as f64 / (sse / dfe as f64);
    let fprob = Family::F.cdf(fstat, &[model_df as f64, dfe as f64], true);
    let log_likelihood = model_log_likelihood(input.distribution, &y, &mu, dispersion);
    let (null_log_likelihood, null_deviance) = if input.generalized {
        e.check_cancel()?;
        let null_mu = DVector::from_element(n, y.sum() / n as f64);
        (
            model_log_likelihood(input.distribution, &y, &null_mu, dispersion),
            deviance(input.distribution, &y, &null_mu),
        )
    } else {
        (f64::NAN, f64::NAN)
    };
    let rhs = input
        .terms
        .iter()
        .map(|t| {
            if t.is_empty() {
                "1".into()
            } else {
                term_name(t, &input.names)
            }
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let formula = format!(
        "{} ~ {}{}",
        input.response,
        rhs,
        if intercept { "" } else { " - 1" }
    );
    Ok(Model {
        predictor_names: input.names.clone(),
        terms: input.terms.clone(),
        formula,
        coefficient_names,
        coefficients,
        beta: fit.b,
        covariance,
        fitted: Matrix::from_column_vec(input.x.rows(), 1, fitted)?,
        residuals: table(&["Raw"], vec![raw], None)?,
        n,
        rank: fit.rank,
        dfe,
        sse,
        sst,
        r2,
        adjusted,
        rmse,
        fstat,
        fprob,
        log_likelihood,
        null_log_likelihood,
        null_deviance,
        deviance: deviance(input.distribution, &y, &mu),
        dispersion,
        distribution: input.distribution,
        link: input.link,
        generalized: input.generalized,
    })
}
fn subset(t: &Term, larger: &Term) -> bool {
    t.iter()
        .all(|&(j, p)| larger.iter().any(|&(k, q)| j == k && p <= q))
}
fn partial_f(small: &Model, large: &Model) -> Option<(f64, f64)> {
    let df = large.rank.checked_sub(small.rank)?;
    if df == 0 {
        return None;
    }
    let f = ((small.sse - large.sse).max(0.) / df as f64) / (large.sse / large.dfe as f64);
    Some((f, Family::F.cdf(f, &[df as f64, large.dfe as f64], true)))
}
fn stepwise(
    e: &mut Engine,
    input: &mut Input,
    keep: &[usize],
    out: &mut Execution,
) -> Result<Model> {
    let mut current = fit_model(e, input, keep)?;
    let mut fits = 1;
    let mut step = 0;
    let mut visited = vec![input.terms.clone()];
    loop {
        e.check_cancel()?;
        let mut addition: Option<(f64, f64, Term, Model)> = None;
        for t in input.upper.clone() {
            if input.terms.contains(&t) {
                continue;
            }
            // Strong hierarchy: all lower terms in Upper must be present.
            if input
                .upper
                .iter()
                .any(|lower| lower != &t && subset(lower, &t) && !input.terms.contains(lower))
            {
                continue;
            }
            let old = input.terms.clone();
            input.terms.push(t.clone());
            normalize_terms(&mut input.terms)?;
            // A partial F test needs residual degrees of freedom. A saturated
            // candidate cannot be tested and must not abort an otherwise valid fit.
            if input.terms.len() >= keep.len() {
                input.terms = old;
                continue;
            }
            if fits >= MAX_STEP_FITS {
                input.terms = old;
                return Err("stepwiselm: exceeded 1024 candidate fits".into());
            }
            fits += 1;
            let candidate = fit_model(e, input, keep)?;
            input.terms = old;
            if let Some((f, p)) = partial_f(&current, &candidate)
                && p < input.p_enter
                && addition.as_ref().is_none_or(|best| p < best.1)
            {
                addition = Some((f, p, t, candidate));
            }
        }
        if let Some((f, p, t, model)) = addition {
            step += 1;
            e.emit(
                out,
                format!(
                    "{step}. Adding {}, FStat = {f:.6}, pValue = {p:.6e}",
                    term_name(&t, &input.names)
                ),
            )?;
            input.terms.push(t);
            normalize_terms(&mut input.terms)?;
            current = model;
            if visited.contains(&input.terms) {
                return Err("stepwiselm: selection cycled; change thresholds".into());
            }
            visited.push(input.terms.clone());
            // Published stepwiselm scheduling exhausts forward additions
            // before considering a removal, then resumes forward selection.
            continue;
        }
        let mut removal: Option<(f64, f64, Term, Model)> = None;
        for t in input.terms.clone() {
            if t.is_empty()
                || input
                    .terms
                    .iter()
                    .any(|higher| higher != &t && subset(&t, higher))
            {
                continue;
            }
            let old = input.terms.clone();
            input.terms.retain(|v| v != &t);
            if fits >= MAX_STEP_FITS {
                input.terms = old;
                return Err("stepwiselm: exceeded 1024 candidate fits".into());
            }
            fits += 1;
            let candidate = fit_model(e, input, keep)?;
            input.terms = old;
            if let Some((f, p)) = partial_f(&candidate, &current)
                && p > input.p_remove
                && removal.as_ref().is_none_or(|best| p > best.1)
            {
                removal = Some((f, p, t, candidate));
            }
        }
        let Some((f, p, t, model)) = removal else {
            return Ok(current);
        };
        step += 1;
        e.emit(
            out,
            format!(
                "{step}. Removing {}, FStat = {f:.6}, pValue = {p:.6e}",
                term_name(&t, &input.names)
            ),
        )?;
        input.terms.retain(|v| v != &t);
        current = model;
        if visited.contains(&input.terms) {
            return Err("stepwiselm: selection cycled; change thresholds".into());
        }
        visited.push(input.terms.clone());
    }
}
pub(super) fn handles(name: &str) -> bool {
    matches!(name, "fitlm" | "fitglm" | "stepwiselm")
}
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "predict" => model_predict(e, args, count),
        _ => {
            if count > 1 {
                return Err(format!("{name}: too many outputs"));
            }
            let args = e.text_arguments(args.to_vec())?;
            let mut input = parse(e, name, &args)?;
            let keep = complete_rows(
                e,
                &input,
                if name == "stepwiselm" {
                    &input.upper
                } else {
                    &input.terms
                },
            )?;
            bounds(
                e,
                name,
                keep.len(),
                if name == "stepwiselm" {
                    input.upper.len()
                } else {
                    input.terms.len()
                },
            )?;
            let model = if name == "stepwiselm" {
                stepwise(e, &mut input, &keep, out)?
            } else {
                fit_model(e, &input, &keep)?
            };
            if model.rank < model.terms.len() {
                e.lang_utils_call("warning",&[Value::Text(format!("{name}: design is rank deficient; dependent coefficients are zero and confidence intervals are NaN"))],out)?;
            }
            Ok(vec![e.lmo_new(model)?])
        }
    }
}
/// One prediction dispatcher, shared by single- and multiple-output calls.
/// Additional model families can dispatch here before the unsupported-type error.
pub(super) fn model_predict(e: &mut Engine, a: &[Value], count: usize) -> Result<Vec<Value>> {
    if a.len() != 2 {
        return Err("predict: expected a model and new predictor matrix or table".into());
    }
    if count > 2 {
        return Err("predict: too many outputs".into());
    }
    let model = e.lmo_get(&a[0])?;
    let x = if let Value::Container(Array::Table(t)) = &a[1] {
        bounds(e, "predict", t.height(), model.predictor_names.len().max(1))?;
        let mut data = Vec::with_capacity(t.height() * model.predictor_names.len());
        for (j, name) in model.predictor_names.iter().enumerate() {
            let used = model
                .terms
                .iter()
                .any(|term| term.iter().any(|&(k, _)| j == k));
            if !used {
                data.resize(data.len() + t.height(), 0.);
                continue;
            }
            let k = t
                .index_of(name)
                .ok_or_else(|| format!("predict: missing table predictor '{name}'"))?;
            let c = real(e, "predict", &Value::from(t.column(k).clone()))?;
            if c.cols() != 1 {
                return Err("predict: table predictors must be numeric scalar columns".into());
            }
            data.extend_from_slice(c.data());
        }
        Matrix::from_column_vec(t.height(), model.predictor_names.len(), data)?
    } else {
        real(e, "predict", &a[1])?
    };
    if x.cols() != model.predictor_names.len() {
        return Err(format!(
            "predict: expected {} predictor columns",
            model.predictor_names.len()
        ));
    }
    let rows: Vec<_> = (0..x.rows()).collect();
    let design = design(e, &x, &model.terms, &rows)?;
    let eta = &design * &model.beta;
    let mean = eta.map(|z| model.link.inverse(z));
    let mut result = vec![Value::Matrix(Matrix::from_column_vec(
        x.rows(),
        1,
        mean.as_slice().to_vec(),
    )?)];
    if count == 2 {
        let quantile = if model.distribution == Distribution::Normal {
            Family::T.inv(0.975, &[model.dfe as f64])
        } else {
            Family::Norm.inv(0.975, &[0., 1.])
        };
        e.preflight_elements(x.rows().saturating_mul(2))?;
        let mut ci = vec![f64::NAN; x.rows() * 2];
        for i in 0..x.rows() {
            if i.is_multiple_of(1024) {
                e.check_cancel()?;
            }
            if model.rank < model.terms.len() {
                continue;
            }
            let row = design.row(i).transpose();
            let variance = row.dot(&(&model.covariance * &row));
            let half = quantile
                * if variance.is_nan() {
                    f64::NAN
                } else {
                    variance.max(0.).sqrt()
                };
            ci[i] = model.link.inverse(eta[i] - half);
            ci[i + x.rows()] = model.link.inverse(eta[i] + half);
        }
        result.push(Value::Matrix(Matrix::from_column_vec(x.rows(), 2, ci)?));
    }
    Ok(result)
}
