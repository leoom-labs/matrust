//! Distribution fitting for the Statistics toolbox core: fitdist for 21
//! parametric families and kernel estimates (with censoring, frequencies,
//! statset options and 'By' groups), paramci, negloglik, mle (named and
//! custom), the <family>fit and <family>like functions, probability and
//! quantile plots, ecdf and cdfplot. Built from the public function
//! documentation and published formulas; expected values in tests come from
//! SciPy/NumPy (tool/refs/stats/dist_fit.py).
use super::dist_objects::Distribution;
use super::stats_dist::families::Family;
use super::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, Result};

mod builtins;
pub(in crate::engine) mod families;
pub(in crate::engine) mod kernel;
mod mle;
mod optimize;
mod plots;
mod sample;

pub(super) const NAMES: &[&str] = &[
    "fitdist",
    "paramci",
    "negloglik",
    "mle",
    "statset",
    "normlike",
    "explike",
    "gamlike",
    "wbllike",
    "lognlike",
    "evlike",
    "betalike",
    "gamfit",
    "wblfit",
    "lognfit",
    "betafit",
    "raylfit",
    "evfit",
    "poissfit",
    "binofit",
    "nbinfit",
    "gevfit",
    "gpfit",
    "probplot",
    "normplot",
    "wblplot",
    "qqplot",
    "ecdf",
    "cdfplot",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Names that draw when called as a statement (no outputs).
pub(super) fn statement(name: &str) -> bool {
    matches!(
        name,
        "probplot" | "normplot" | "wblplot" | "qqplot" | "ecdf" | "cdfplot"
    )
}

/// The fit record of an object made by fitdist.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct FitInfo {
    /// Observations used (NaN rows and zero frequencies removed).
    pub data: Vec<f64>,
    /// Right-censoring flags, empty when no censoring was given.
    pub cens: Vec<bool>,
    /// Frequencies, empty when none were given.
    pub freq: Vec<f64>,
    /// Parameter covariance, k-by-k column-major (zero rows for fixed ones).
    pub cov: Vec<f64>,
    pub fixed: Vec<bool>,
}

impl FitInfo {
    pub fn bytes(&self) -> usize {
        64 + self.data.len() * 8 + self.cens.len() + self.freq.len() * 8 + self.cov.len() * 8
    }
    fn sample(&self) -> sample::Sample {
        sample::Sample::from_parts(self.data.clone(), self.cens.clone(), self.freq.clone())
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let args = e.text_arguments(args.to_vec())?;
    match name {
        "probplot" | "normplot" | "wblplot" | "qqplot" | "ecdf" | "cdfplot" => {
            plots::call(e, name, &args, nargout, out)
        }
        _ => builtins::call(e, name, &args, nargout, out),
    }
}

/// `paramci(pd, ...)` and `negloglik(pd)` reached as object methods.
pub(super) fn object_method(
    e: &mut Engine,
    d: &Distribution,
    name: &str,
    rest: &[Value],
) -> Result<Value> {
    let rest = e.text_arguments(rest.to_vec())?;
    if name == "negloglik" {
        if !rest.is_empty() {
            return Err("negloglik: expected one probability distribution object".into());
        }
        let fit = d.fit.as_deref().or(d.kernel.as_ref().map(|k| &k.input));
        return Ok(Value::Matrix(match fit {
            Some(f) => Matrix::scalar(negloglik(e, d, f)?),
            None => Matrix::from_column_vec(0, 0, vec![])?,
        }));
    }
    builtins::paramci(e, d, &rest)
}

/// Negative log-likelihood of the object's own input data.
pub(super) fn negloglik(e: &Engine, d: &Distribution, fit: &FitInfo) -> Result<f64> {
    let s = fit.sample();
    match &d.kernel {
        Some(k) => k.nll(e, &s),
        None => Ok(mle::nll(d.family, &d.params, &s)),
    }
}

/// `InputData`: struct with fields data, cens and freq.
pub(super) fn input_data(e: &Engine, fit: &FitInfo) -> Result<Value> {
    let column = |v: Vec<f64>| -> Result<Array> {
        Ok(Array::Numeric(Matrix::from_column_vec(v.len(), 1, v)?))
    };
    let empty = || Array::Numeric(Matrix::from_column_vec(0, 0, vec![]).expect("empty"));
    let cens = if fit.cens.is_empty() {
        empty()
    } else {
        Array::Logical(LogicalMatrix::from_column_vec(
            fit.cens.len(),
            1,
            fit.cens.iter().map(|&c| u8::from(c)).collect(),
            &e.cancel,
        )?)
    };
    let freq = if fit.freq.is_empty() {
        empty()
    } else {
        column(fit.freq.clone())?
    };
    super::optim_nl::onl_struct(
        e,
        vec![
            ("data", column(fit.data.clone())?),
            ("cens", cens),
            ("freq", freq),
        ],
    )
}

/// `%g`-style text with `digits` significant digits.
pub(super) fn short(v: f64, digits: usize) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0. { "Inf" } else { "-Inf" }.into();
    }
    if v == 0. {
        return "0".into();
    }
    let exponent = v.abs().log10().floor() as i32;
    if !(-5..digits as i32).contains(&exponent) {
        let s = format!("{:.*e}", digits.saturating_sub(1), v);
        let (mantissa, power) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.trim_end_matches('0').trim_end_matches('.')
        } else {
            mantissa
        };
        let power: i32 = power.parse().unwrap_or(0);
        return format!(
            "{mantissa}e{}{:02}",
            if power < 0 { '-' } else { '+' },
            power.abs()
        );
    }
    let decimals = (digits as i32 - 1 - exponent).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Display of a fitted parametric object: the family, the sample size and
/// each parameter with its 95% confidence interval.
pub(super) fn fitted_body(d: &Distribution, fit: &FitInfo) -> String {
    let s = fit.sample();
    let ci = mle::param_ci(d.family, &d.params, &s, &fit.cov, &fit.fixed, 0.05);
    let n = s.weight();
    let mut text = format!(
        "  {} distribution fitted to {} observation{}\n",
        d.title(),
        short(n, 10),
        if n == 1. { "" } else { "s" }
    );
    if s.has_cens() {
        let censored = s.weight() - s.uncensored_weight();
        text.push_str(&format!("  ({} right-censored)\n", short(censored, 10)));
    }
    text.push_str("  Estimates with 95% confidence intervals:\n");
    let width = d.names().iter().map(|n| n.len()).max().unwrap_or(0);
    let values: Vec<String> = d.params.iter().map(|&v| short(v, 6)).collect();
    let value_width = values.iter().map(String::len).max().unwrap_or(0);
    for (i, name) in d.names().iter().enumerate() {
        let interval = if fit.fixed[i] {
            "(fixed)".to_string()
        } else {
            format!("[{}, {}]", short(ci[i][0], 6), short(ci[i][1], 6))
        };
        text.push_str(&format!(
            "    {name:>width$} = {:<value_width$}  {interval}\n",
            values[i]
        ));
    }
    text
}

/// The single family name argument of fitdist/mle/probplot.
pub(super) fn family_name(who: &str, v: &Value) -> Result<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
        _ => Err(format!("{who}: expected a distribution name")),
    }
}

/// Fits `f` (or a kernel when `kernel` is set) to the sample and wraps the
/// result as a distribution object.
fn fitted_object(
    e: &mut Engine,
    f: Family,
    params: Vec<f64>,
    s: &sample::Sample,
    fixed: Vec<bool>,
) -> Result<Distribution> {
    let cov = mle::covariance(e, f, &params, s, &fixed)?;
    let fit = FitInfo {
        data: s.x.clone(),
        cens: if s.cens_given {
            s.cens.clone()
        } else {
            Vec::new()
        },
        freq: if s.has_freq() {
            s.freq.clone()
        } else {
            Vec::new()
        },
        cov,
        fixed,
    };
    Ok(Distribution {
        family: f,
        params,
        fit: Some(std::sync::Arc::new(fit)),
        kernel: None,
    })
}
