//! Immutable probability objects backed by the bounded builtin object registry.
//! makedist builds them from parameters; fitdist (src/engine/dist_fit) adds
//! the fit record (covariance, input data, NLogL) or a kernel estimate.
use super::dist_fit::{FitInfo, kernel::KernelFit};
use super::stats_dist::families::Family;
use super::{Engine, Execution, Fma, Value};
use crate::{Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Distribution {
    pub family: Family,
    pub params: Vec<f64>,
    /// Present for objects made by fitdist.
    pub fit: Option<Arc<FitInfo>>,
    /// Present for nonparametric kernel objects; `family` is then unused.
    pub kernel: Option<Arc<KernelFit>>,
}

fn class_of(f: Family) -> &'static str {
    match f {
        Family::Norm => "prob.NormalDistribution",
        Family::Unif => "prob.UniformDistribution",
        Family::Exp => "prob.ExponentialDistribution",
        Family::Gam => "prob.GammaDistribution",
        Family::Beta => "prob.BetaDistribution",
        Family::Chi2 => "prob.ChiSquareDistribution",
        Family::T => "prob.TDistribution",
        Family::F => "prob.FDistribution",
        Family::Logn => "prob.LognormalDistribution",
        Family::Wbl => "prob.WeibullDistribution",
        Family::Rayl => "prob.RayleighDistribution",
        Family::Ev => "prob.ExtremeValueDistribution",
        Family::Bino => "prob.BinomialDistribution",
        Family::Poiss => "prob.PoissonDistribution",
        Family::Geo => "prob.GeometricDistribution",
        Family::Nbin => "prob.NegativeBinomialDistribution",
        Family::Logistic => "prob.LogisticDistribution",
        Family::Loglogistic => "prob.LoglogisticDistribution",
        Family::Rician => "prob.RicianDistribution",
        Family::Nakagami => "prob.NakagamiDistribution",
        Family::Gev => "prob.GeneralizedExtremeValueDistribution",
        Family::Gp => "prob.GeneralizedParetoDistribution",
        Family::Tls => "prob.tLocationScaleDistribution",
        Family::InvGauss => "prob.InverseGaussianDistribution",
        Family::Bisa => "prob.BirnbaumSaundersDistribution",
        Family::HalfNorm => "prob.HalfNormalDistribution",
    }
}

const COMMON: &[&str] = &[
    "DistributionName",
    "ParameterNames",
    "ParameterDescription",
    "ParameterValues",
    "ParameterCovariance",
    "ParameterIsFixed",
    "NLogL",
    "InputData",
    "Truncation",
    "IsTruncated",
];
const KERNEL_PROPERTIES: &[&str] = &[
    "Kernel",
    "BandWidth",
    "Support",
    "DistributionName",
    "NLogL",
    "InputData",
    "Truncation",
    "IsTruncated",
];

impl Distribution {
    pub fn new(family: Family, params: Vec<f64>) -> Self {
        Self {
            family,
            params,
            fit: None,
            kernel: None,
        }
    }
    pub fn names(&self) -> &'static [&'static str] {
        if self.kernel.is_some() {
            return &[];
        }
        parameter_names(self.family)
    }
    pub fn title(&self) -> &'static str {
        if self.kernel.is_some() {
            return "Kernel";
        }
        match self.family {
            Family::Ev => "ExtremeValue",
            Family::Nbin => "NegativeBinomial",
            Family::Chi2 => "ChiSquare",
            _ => self.family.title(),
        }
    }
    pub fn class(&self) -> &'static str {
        if self.kernel.is_some() {
            return "prob.KernelDistribution";
        }
        class_of(self.family)
    }
    pub fn bytes(&self) -> usize {
        32 + self.params.len() * 8
            + self.fit.as_ref().map_or(0, |f| f.bytes())
            + self.kernel.as_ref().map_or(0, |k| k.bytes())
    }
    pub fn property_names(&self) -> Vec<String> {
        if self.kernel.is_some() {
            return KERNEL_PROPERTIES.iter().map(|s| (*s).into()).collect();
        }
        let mut names: Vec<String> = self.names().iter().map(|s| (*s).into()).collect();
        names.extend(COMMON.iter().map(|s| (*s).into()));
        names
    }
    pub fn body(&self) -> String {
        if let Some(k) = &self.kernel {
            return k.body();
        }
        if let Some(fit) = &self.fit {
            return super::dist_fit::fitted_body(self, fit);
        }
        let mut s = format!("  {} distribution with parameters:\n", self.title());
        for (name, value) in self.names().iter().zip(&self.params) {
            s.push_str(&format!("    {name}: {value}\n"));
        }
        s
    }
    pub fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        let row = |v: Vec<f64>| -> Result<Value> {
            Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
        };
        let empty = || Value::Matrix(Matrix::from_column_vec(0, 0, vec![]).expect("empty"));
        match name {
            "DistributionName" => return Ok(Value::Text(self.title().into())),
            "Truncation" => return Ok(empty()),
            "IsTruncated" => return Ok(Value::Logical(LogicalMatrix::scalar(false))),
            "InputData" | "NLogL" => {
                let data = self
                    .fit
                    .as_ref()
                    .map(|f| f.as_ref())
                    .or(self.kernel.as_ref().map(|k| &k.input));
                let Some(fit) = data else {
                    return Ok(empty());
                };
                if name == "NLogL" {
                    return Ok(Value::Matrix(Matrix::scalar(super::dist_fit::negloglik(
                        e, self, fit,
                    )?)));
                }
                return super::dist_fit::input_data(e, fit);
            }
            _ => {}
        }
        if let Some(k) = &self.kernel {
            return k.property(e, name);
        }
        let k = self.params.len();
        match name {
            "ParameterValues" => row(self.params.clone()),
            "ParameterNames" => cellstr_row(e, self.names()),
            "ParameterDescription" => cellstr_row(e, descriptions(self.family)),
            "ParameterCovariance" => {
                let cov = self
                    .fit
                    .as_ref()
                    .map_or_else(|| vec![0.; k * k], |f| f.cov.clone());
                Ok(Value::Matrix(Matrix::from_column_vec(k, k, cov)?))
            }
            "ParameterIsFixed" => {
                let fixed: Vec<u8> = match &self.fit {
                    Some(f) => f.fixed.iter().map(|&b| u8::from(b)).collect(),
                    None => vec![1; k],
                };
                Ok(Value::Logical(LogicalMatrix::from_column_vec(
                    1, k, fixed, &e.cancel,
                )?))
            }
            _ => self
                .names()
                .iter()
                .position(|n| *n == name)
                .map(|i| Value::Matrix(Matrix::scalar(self.params[i])))
                .ok_or_else(|| format!("{}: unknown property '{name}'", self.class())),
        }
    }
}

pub(super) fn cellstr_row(e: &Engine, names: &[&str]) -> Result<Value> {
    let items = names
        .iter()
        .map(|n| CharacterRow::from_text(n, &e.cancel).map(|c| Arc::new(Array::Character(c))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        1,
        items.len(),
        items,
        &e.cancel,
    )?)))
}

pub(super) fn parameter_names(f: Family) -> &'static [&'static str] {
    match f {
        Family::Norm | Family::Logn | Family::Ev => &["mu", "sigma"],
        Family::Logistic | Family::Loglogistic | Family::HalfNorm => &["mu", "sigma"],
        Family::Unif => &["Lower", "Upper"],
        Family::Exp => &["mu"],
        Family::Gam | Family::Beta => &["a", "b"],
        Family::Chi2 | Family::T => &["nu"],
        Family::F => &["nu1", "nu2"],
        Family::Wbl => &["A", "B"],
        Family::Rayl => &["B"],
        Family::Bino => &["N", "p"],
        Family::Poiss => &["lambda"],
        Family::Geo => &["p"],
        Family::Nbin => &["R", "P"],
        Family::Rician => &["s", "sigma"],
        Family::Nakagami => &["mu", "omega"],
        Family::Gev => &["k", "sigma", "mu"],
        Family::Gp => &["k", "sigma", "theta"],
        Family::Tls => &["mu", "sigma", "nu"],
        Family::InvGauss => &["mu", "lambda"],
        Family::Bisa => &["beta", "gamma"],
    }
}

/// Short descriptions of the parameters (own wording).
fn descriptions(f: Family) -> &'static [&'static str] {
    match f {
        Family::Norm => &["mean", "standard deviation"],
        Family::Logn => &["mean of log(x)", "standard deviation of log(x)"],
        Family::Ev | Family::Logistic => &["location", "scale"],
        Family::Loglogistic => &["location of log(x)", "scale of log(x)"],
        Family::HalfNorm => &["location (lower bound)", "scale"],
        Family::Unif => &["lower bound", "upper bound"],
        Family::Exp => &["mean"],
        Family::Gam => &["shape", "scale"],
        Family::Beta => &["first shape", "second shape"],
        Family::Chi2 | Family::T => &["degrees of freedom"],
        Family::F => &[
            "numerator degrees of freedom",
            "denominator degrees of freedom",
        ],
        Family::Wbl => &["scale", "shape"],
        Family::Rayl => &["scale"],
        Family::Bino => &["number of trials", "probability of success"],
        Family::Poiss => &["mean"],
        Family::Geo => &["probability of success"],
        Family::Nbin => &["number of successes", "probability of success"],
        Family::Rician => &["noncentrality", "scale"],
        Family::Nakagami => &["shape", "spread"],
        Family::Gev | Family::Gp => &["shape", "scale", "location"],
        Family::Tls => &["location", "scale", "degrees of freedom"],
        Family::InvGauss => &["mean", "shape"],
        Family::Bisa => &["scale", "shape"],
    }
}

fn text(v: &Value) -> Result<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s),
        _ => Err("distribution: expected text".into()),
    }
}
fn scalar(e: &Engine, v: &Value) -> Result<f64> {
    if v.is_character() {
        return Err("distribution parameters must be finite real scalars".into());
    }
    let x = e.real_arg("distribution", v)?.as_scalar()?;
    if !x.is_finite() {
        return Err("distribution parameters must be finite real scalars".into());
    }
    Ok(x)
}
fn checked(e: &mut Engine, f: Family, params: Vec<f64>) -> Result<Value> {
    if !f.valid(&params) || params.iter().any(|v| !v.is_finite()) {
        return Err(format!("{}: invalid distribution parameters", f.title()));
    }
    e.dso_new(Distribution::new(f, params))
}
pub(super) fn call(e: &mut Engine, name: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(name, "makedist" | "fitdist") {
        return Ok(None);
    }
    if name == "fitdist" {
        let mut out = Execution::default();
        return super::dist_fit::call(e, name, args, 1, &mut out).map(|mut v| Some(v.remove(0)));
    }
    let args = e.text_arguments(args.to_vec())?;
    let Some(first) = args.first() else {
        return Err("makedist: expected a distribution name".into());
    };
    let f = super::stats_dist::family_by_name(name, text(first)?)?;
    if !args.len().is_multiple_of(2) {
        // one family followed by complete pairs
        let mut params: Vec<f64> = f.defaults().iter().map(|d| d.unwrap_or(1.)).collect();
        if matches!(f, Family::Bino | Family::Nbin) {
            params[1] = 0.5;
        }
        if f == Family::Geo {
            params[0] = 0.5;
        }
        let names = parameter_names(f);
        for pair in args[1..].as_chunks::<2>().0 {
            let key = text(&pair[0])?;
            let i = names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(key))
                .ok_or_else(|| {
                    format!(
                        "makedist: unknown {} parameter '{key}'; expected {}",
                        f.title(),
                        names.join(", ")
                    )
                })?;
            params[i] = scalar(e, &pair[1])?;
        }
        return checked(e, f, params).map(Some);
    }
    Err("makedist: parameters must be name/value pairs".into())
}

pub(super) const METHODS: &[&str] = &[
    "pdf",
    "cdf",
    "icdf",
    "random",
    "mean",
    "std",
    "var",
    "median",
    "disp",
    "paramci",
    "negloglik",
];
pub(super) fn method(
    e: &mut Engine,
    d: &Distribution,
    name: &str,
    rest: &[Value],
    _out: &mut Execution,
) -> Result<Option<Value>> {
    if matches!(name, "paramci" | "negloglik") {
        return super::dist_fit::object_method(e, d, name, rest).map(Some);
    }
    if let Some(k) = &d.kernel {
        return k.method(e, name, rest);
    }
    if matches!(name, "mean" | "std" | "var" | "median") {
        if !rest.is_empty() {
            return Err(format!(
                "{name}: distribution objects take no additional arguments"
            ));
        }
        let (m, v) = moments(d);
        let x = match name {
            "mean" => m,
            "var" => v,
            "std" => v.sqrt(),
            _ => d.family.inv(0.5, &d.params),
        };
        return Ok(Some(Value::Matrix(Matrix::scalar(x))));
    }
    if matches!(name, "pdf" | "cdf" | "icdf" | "random") {
        let mut args = vec![Value::Text(d.family.title().into())];
        if name != "random" {
            let Some(x) = rest.first() else {
                return Err(format!("{name}: expected evaluation points"));
            };
            args.push(x.clone());
        }
        args.extend(d.params.iter().map(|&p| Value::Matrix(Matrix::scalar(p))));
        args.extend_from_slice(if name == "random" { rest } else { &rest[1..] });
        return e.stats_dist_call(name, &args);
    }
    if matches!(name, "delete" | "isvalid") {
        return Err(format!(
            "{name}: probability distributions are value objects"
        ));
    }
    Ok(None)
}
fn moments(d: &Distribution) -> (f64, f64) {
    use Family::*;
    let a = d.params[0];
    let b = d.params.get(1).copied().unwrap_or(0.);
    match d.family {
        Norm => (a, b * b),
        Unif => (a / 2. + b / 2., (b - a).powi(2) / 12.),
        Exp => (a, a * a),
        Gam => (a * b, a * b * b),
        Beta => beta_moments(a, b),
        Chi2 => (a, 2. * a),
        T => (
            if a > 1. { 0. } else { f64::NAN },
            if a > 2. {
                a / (a - 2.)
            } else if a > 1. {
                f64::INFINITY
            } else {
                f64::NAN
            },
        ),
        F => (
            if b > 2. { b / (b - 2.) } else { f64::INFINITY },
            if b > 4. {
                2. * b * b * (a + b - 2.) / (a * (b - 2.).powi(2) * (b - 4.))
            } else {
                f64::INFINITY
            },
        ),
        Logn => lognormal_moments(a, b),
        Wbl => weibull_moments(a, b),
        Rayl => (
            a * (std::f64::consts::PI / 2.).sqrt(),
            (2. - std::f64::consts::PI / 2.) * a * a,
        ),
        Ev => (
            a - std::f64::consts::EULER_GAMMA * b,
            std::f64::consts::PI.powi(2) / 6. * b * b,
        ),
        Bino => (a * b, a * b * (1. - b)),
        Poiss => (a, a),
        Geo => ((1. - a) / a, (1. - a) / (a * a)),
        Nbin => (a * (1. - b) / b, a * (1. - b) / (b * b)),
        f => super::dist_fit::families::moments(f, &d.params),
    }
}

/// Mean `a/(a+b)` and variance `r(1-r)/(a+b+1)`. The ratio put in the
/// denominator is at most 1, so `b/a` cannot overflow, and the smaller share
/// is `ratio/(1+ratio)` so a tiny share is not rounded away.
fn beta_moments(a: f64, b: f64) -> (f64, f64) {
    let (p, q, a_is_large) = if a >= b { (a, b, true) } else { (b, a, false) };
    let ratio = q / p;
    let mean_big = 1. / (1. + ratio);
    let mean_small = if ratio == 0. {
        0.
    } else {
        ratio / (1. + ratio)
    };
    let mean = if a_is_large { mean_big } else { mean_small };
    let inv_p = 1. / p;
    let var = mean_big * mean_small * inv_p / (1. + ratio + inv_p);
    (mean, var)
}

/// Variance `(exp(σ²)-1) exp(2μ+σ²)` in log space. `log(expm1(s))` is
/// `s + log1p(-exp(-s))` once `s` is large enough that `expm1(s)` overflows.
fn lognormal_moments(mu: f64, sigma: f64) -> (f64, f64) {
    let s2 = sigma * sigma;
    let mean = (mu + s2 / 2.).exp();
    let log_expm1 = if s2 >= 1. {
        // s + log1p(-exp(-s)). Negating exp(s) overflows and yields NaN.
        s2 + (-(-s2).exp()).ln_1p()
    } else {
        s2.exp_m1().ln()
    };
    (mean, (2. * mu + s2 + log_expm1).exp())
}

/// `A² (Γ(1+2/B) - Γ(1+1/B)²)`. For `B > 1000` the subtraction cancels; the
/// series below is the Taylor expansion in `μ = 1/B` (mpmath, 50 digits).
/// Shapes `1..=1000` stay on the direct gamma product when that product is
/// finite. `Γ(1+2/B)` can overflow while `A²` times the gamma difference
/// stays finite; those cases multiply in log space (`lgamma`).
fn weibull_moments(a: f64, b: f64) -> (f64, f64) {
    let inv = 1. / b;
    let g1 = libm::tgamma(1. + inv);
    let g2 = libm::tgamma(1. + 2. * inv);
    let mean = a * g1;
    if b > 1e3 {
        return (mean, a * a * weibull_large_shape_variance(b));
    }
    if g1.is_finite() && g2.is_finite() {
        let var = a * a * (g2 - g1 * g1);
        if var.is_finite() {
            return (mean, var);
        }
    }
    (mean, weibull_variance_log(a, b))
}

/// `log(A² (Γ(1+2/B) - Γ(1+1/B)²))` then `exp`. `lgamma` stays finite after
/// `tgamma` has overflowed.
fn weibull_variance_log(a: f64, b: f64) -> f64 {
    if a == 0. {
        return 0.;
    }
    let lg1 = libm::lgamma(1. + 1. / b);
    let lg2 = libm::lgamma(1. + 2. / b);
    if !lg1.is_finite() || !lg2.is_finite() {
        return f64::INFINITY;
    }
    // log(Γ2 - Γ1²) = lg2 + log1p(-exp(2 lg1 - lg2)).
    let delta = 2. * lg1 - lg2;
    let log_factor = lg2 + libm::log1p(-libm::exp(delta));
    if !log_factor.is_finite() {
        return f64::INFINITY;
    }
    libm::exp(2. * a.abs().ln() + log_factor)
}

fn weibull_large_shape_variance(b: f64) -> f64 {
    // Coefficients of μ^2 .. μ^8. Leading term is π²/6.
    const C: [f64; 7] = [
        1.6449340668482264,
        -4.303077228549152,
        11.718339177218881,
        -26.53141916464012,
        57.67611285960975,
        -120.6254077476928,
        247.65840041981136,
    ];
    let mu = 1. / b;
    let mut acc: f64 = 0.;
    for &c in C.iter().rev() {
        acc = acc.fma(mu, c);
    }
    acc * mu * mu
}
