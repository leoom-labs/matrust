//! Probability distributions: `<family>pdf`, `<family>cdf`, `<family>inv` and
//! `<family>rnd` for sixteen families, the generic `pdf`, `cdf`, `icdf` and
//! `random`, and the fits `normfit` and `expfit`.
//!
//! The kernels are in `stats_dist/families.rs`; this file parses arguments,
//! expands scalars to a common size and loops. Random draws come from the
//! session's MT19937 stream (`rng` seeds them), consumed in column-major
//! output order.
use super::{Engine, Value};
use crate::{Matrix, Result};

pub(super) mod families;

use families::{ALL, Family, WalkCache};

const CHUNK: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Pdf,
    Cdf,
    Inv,
    Rnd,
}

impl Op {
    fn suffix(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Cdf => "cdf",
            Self::Inv => "inv",
            Self::Rnd => "rnd",
        }
    }
}

/// `normpdf` -> (Norm, Pdf), and so on for every family and operation.
fn split_name(name: &str) -> Option<(Family, Op)> {
    for op in [Op::Pdf, Op::Cdf, Op::Inv, Op::Rnd] {
        if let Some(prefix) = name.strip_suffix(op.suffix())
            && let Some(family) = ALL.iter().find(|f| f.prefix() == prefix)
        {
            return Some((*family, op));
        }
    }
    None
}

const GENERIC: &[&str] = &["pdf", "cdf", "icdf", "random"];
const FITS: &[&str] = &["normfit", "expfit"];

/// Every name this module serves (for completion and `exist`).
pub(super) fn stats_dist_handles(name: &str) -> bool {
    split_name(name).is_some() || GENERIC.contains(&name) || FITS.contains(&name)
}

/// Functions with more than one output.
pub(super) fn stats_dist_has_outputs(name: &str) -> bool {
    FITS.contains(&name)
}

/// The family named in pdf/cdf/icdf/random, case-insensitively, by its
/// long or short name.
pub(super) fn family_by_name(caller: &str, text: &str) -> Result<Family> {
    let key: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect();
    let family = match key.as_str() {
        "normal" | "norm" | "gaussian" => Family::Norm,
        "uniform" | "unif" => Family::Unif,
        "exponential" | "exp" => Family::Exp,
        "gamma" | "gam" => Family::Gam,
        "beta" => Family::Beta,
        "chisquare" | "chi2" | "chisquared" => Family::Chi2,
        "t" | "studentt" | "student" => Family::T,
        "f" => Family::F,
        "lognormal" | "logn" => Family::Logn,
        "weibull" | "wbl" => Family::Wbl,
        "rayleigh" | "rayl" => Family::Rayl,
        "extremevalue" | "ev" => Family::Ev,
        "binomial" | "bino" => Family::Bino,
        "poisson" | "poiss" => Family::Poiss,
        "geometric" | "geo" => Family::Geo,
        "negativebinomial" | "nbin" => Family::Nbin,
        "logistic" => Family::Logistic,
        "loglogistic" => Family::Loglogistic,
        "rician" => Family::Rician,
        "nakagami" => Family::Nakagami,
        "generalizedextremevalue" | "gev" => Family::Gev,
        "generalizedpareto" | "gp" => Family::Gp,
        "tlocationscale" => Family::Tls,
        "inversegaussian" => Family::InvGauss,
        "birnbaumsaunders" => Family::Bisa,
        "halfnormal" => Family::HalfNorm,
        _ => {
            let mut names: Vec<_> = ALL
                .iter()
                .map(|f| format!("'{}' ('{}')", f.title(), f.prefix()))
                .collect();
            names.extend(
                families::OBJECT_ONLY
                    .iter()
                    .map(|f| format!("'{}'", f.title())),
            );
            return Err(format!(
                "{caller}: unknown distribution '{text}'. Supported names: {}",
                names.join(", ")
            ));
        }
    };
    Ok(family)
}

fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Text(text) | Value::QuotedString(text) => Some(text),
        _ => None,
    }
}

/// Common size of the arguments under scalar expansion.
fn common_dims(name: &str, mats: &[Matrix]) -> Result<Vec<usize>> {
    let mut dims: Option<Vec<usize>> = None;
    for m in mats.iter().filter(|m| m.len() != 1) {
        let d = m.dims().into_owned();
        match &dims {
            None => dims = Some(d),
            Some(prev) if *prev == d => {}
            Some(_) => {
                return Err(format!(
                    "{name}: non-scalar arguments must have the same size"
                ));
            }
        }
    }
    Ok(dims.unwrap_or_else(|| vec![1, 1]))
}

fn element(m: &Matrix, i: usize) -> f64 {
    if m.len() == 1 {
        m.data()[0]
    } else {
        m.data()[i]
    }
}

fn build(dims: &[usize], data: Vec<f64>) -> Result<Matrix> {
    let rows = dims.first().copied().unwrap_or(1);
    let cols = dims[1..].iter().product::<usize>();
    Matrix::from_column_vec(rows, cols, data)?.with_dims(dims)
}

impl Engine {
    /// Single-output entry: `Ok(None)` when the name is not this module's.
    pub(super) fn stats_dist_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !stats_dist_handles(name) {
            return Ok(None);
        }
        Ok(Some(self.stats_dist_outputs(name, args, 1)?.remove(0)))
    }

    pub(super) fn stats_dist_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let args = self.text_arguments(args.to_vec())?;
        if FITS.contains(&name) {
            return self.stats_dist_fit(name, &args, count.max(1));
        }
        if count > 1 {
            return Err(format!("{name} does not support {count} outputs"));
        }
        let value = if let Some((family, op)) = split_name(name) {
            self.stats_dist_family(name, family, op, &args)?
        } else {
            let op = match name {
                "pdf" => Op::Pdf,
                "cdf" => Op::Cdf,
                "icdf" => Op::Inv,
                _ => Op::Rnd,
            };
            let Some(first) = args.first() else {
                return Err(format!("{name} requires a distribution name"));
            };
            let Some(text) = text_of(first) else {
                return Err(format!(
                    "{name}: the first argument must be a distribution name such as 'Normal' or a probability distribution object"
                ));
            };
            let family = family_by_name(name, text)?;
            self.stats_dist_family(name, family, op, &args[1..])?
        };
        Ok(vec![Value::Matrix(value)])
    }

    fn stats_dist_real(&self, name: &str, value: &Value) -> Result<Matrix> {
        if text_of(value).is_some() {
            return Err(format!("{name}: unexpected text argument"));
        }
        self.real_arg(name, value)
    }

    /// The parameters of `family` from `args` (defaults filled in), erroring
    /// on a missing required one.
    fn stats_dist_params(&self, name: &str, family: Family, args: &[Value]) -> Result<Vec<Matrix>> {
        let defaults = family.defaults();
        let mut out = Vec::with_capacity(defaults.len());
        for (i, default) in defaults.iter().enumerate() {
            match (args.get(i), default) {
                (Some(v), _) => out.push(self.stats_dist_real(name, v)?),
                (None, Some(d)) => out.push(Matrix::scalar(*d)),
                (None, None) => {
                    return Err(format!(
                        "{name} requires {} distribution parameter{}",
                        defaults.len(),
                        if defaults.len() == 1 { "" } else { "s" }
                    ));
                }
            }
        }
        Ok(out)
    }

    fn stats_dist_family(
        &mut self,
        name: &str,
        family: Family,
        op: Op,
        args: &[Value],
    ) -> Result<Matrix> {
        let nparams = family.defaults().len();
        if op == Op::Rnd {
            return self.stats_dist_random(name, family, args);
        }
        let mut args = args;
        let mut upper = false;
        if op == Op::Cdf
            && let Some(last) = args.last()
            && let Some(flag) = text_of(last)
        {
            if !flag.eq_ignore_ascii_case("upper") {
                return Err(format!("{name}: the only option is 'upper' (got '{flag}')"));
            }
            upper = true;
            args = &args[..args.len() - 1];
        }
        let Some(first) = args.first() else {
            let what = if op == Op::Inv {
                "probabilities"
            } else {
                "values"
            };
            return Err(format!("{name} requires the {what} as its first argument"));
        };
        if args.len() > 1 + nparams {
            return Err(format!(
                "{name}: too many input arguments ({} distribution parameter{} expected)",
                nparams,
                if nparams == 1 { "" } else { "s" }
            ));
        }
        let x = self.stats_dist_real(name, first)?;
        let params = self.stats_dist_params(name, family, &args[1..])?;
        let mut all = vec![x];
        all.extend(params);
        let dims = common_dims(name, &all)?;
        let total = dims.iter().product::<usize>();
        self.preflight_elements(total)?;
        let mut data = Vec::with_capacity(total);
        let mut q = vec![0.; nparams];
        while data.len() < total {
            self.check_cancel()?;
            let end = (data.len() + CHUNK).min(total);
            for i in data.len()..end {
                for (slot, m) in q.iter_mut().zip(&all[1..]) {
                    *slot = element(m, i);
                }
                let v = element(&all[0], i);
                data.push(match op {
                    Op::Pdf => family.pdf(v, &q),
                    Op::Cdf => family.cdf(v, &q, upper),
                    _ => family.inv(v, &q),
                });
            }
        }
        build(&dims, data)
    }

    /// `<family>rnd(params..., sizes...)`: the first arguments are the
    /// parameters, the rest a size (`m, n, ...` or `[m n ...]`).
    fn stats_dist_random(&mut self, name: &str, family: Family, args: &[Value]) -> Result<Matrix> {
        let nparams = family.defaults().len();
        let (param_args, size_args) = args.split_at(args.len().min(nparams));
        let params = self.stats_dist_params(name, family, param_args)?;
        let param_dims = common_dims(name, &params)?;
        let dims = if size_args.is_empty() {
            param_dims
        } else {
            if size_args.iter().any(|a| text_of(a).is_some()) {
                return Err(format!(
                    "{name}: text options are not supported; give the size as numbers"
                ));
            }
            let dims = super::shape_arguments::shape_dims(size_args, None)?;
            if params.iter().any(|m| m.len() != 1 && *m.dims() != *dims) {
                return Err(format!(
                    "{name}: the size must match the size of non-scalar parameters"
                ));
            }
            dims
        };
        let total = dims.iter().product::<usize>();
        self.preflight_elements(total)?;
        let mut data = Vec::with_capacity(total);
        let mut q = vec![0.; nparams];
        let mut cache = WalkCache::default();
        while data.len() < total {
            self.check_cancel()?;
            let end = (data.len() + CHUNK).min(total);
            for i in data.len()..end {
                for (slot, m) in q.iter_mut().zip(&params) {
                    *slot = element(m, i);
                }
                data.push(family.rnd(&q, &mut self.random, &mut cache));
            }
        }
        build(&dims, data)
    }

    /// `[muhat, sigmahat, muci, sigmaci] = normfit(x, alpha)` and
    /// `[muhat, muci] = expfit(x, alpha)`, column-wise for a matrix.
    fn stats_dist_fit(&mut self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let maximum = if name == "normfit" { 4 } else { 2 };
        if count > maximum {
            return Err(format!("{name} does not support {count} outputs"));
        }
        let Some(first) = args.first() else {
            return Err(format!("{name} requires the data as its first argument"));
        };
        if args.len() > 2 {
            return Err(format!(
                "{name}: censoring and frequency arguments are not supported yet"
            ));
        }
        let x = self.stats_dist_real(name, first)?;
        if x.is_nd() {
            return Err(format!("{name}: N-D data is not supported yet"));
        }
        let alpha = match args.get(1) {
            None => 0.05,
            Some(v) => {
                let m = self.stats_dist_real(name, v)?;
                if m.is_empty() {
                    0.05
                } else {
                    let a = m
                        .as_scalar()
                        .map_err(|_| format!("{name}: alpha must be a scalar"))?;
                    if a.is_nan() || a <= 0. || a >= 1. {
                        return Err(format!("{name}: alpha must be between 0 and 1"));
                    }
                    a
                }
            }
        };
        // A row vector is one sample, as is a column; a matrix gives one per column.
        let (n, columns) = if x.is_empty() {
            (0, 1)
        } else if x.rows() == 1 {
            (x.cols(), 1)
        } else {
            (x.rows(), x.cols())
        };
        let mut est = Vec::with_capacity(columns);
        let mut spread = Vec::with_capacity(columns);
        let mut ci_mu = Vec::with_capacity(2 * columns);
        let mut ci_sigma = Vec::with_capacity(2 * columns);
        let nf = n as f64;
        for c in 0..columns {
            self.check_cancel()?;
            let sample = &x.data()[c * n..(c + 1) * n];
            let mean = if n == 0 {
                f64::NAN
            } else {
                sample.iter().sum::<f64>() / nf
            };
            // A second pass removes the rounding of the first mean.
            let mean = if n == 0 {
                mean
            } else {
                mean + sample.iter().map(|v| v - mean).sum::<f64>() / nf
            };
            if name == "expfit" {
                let chi = |p: f64| Family::Chi2.inv(p, &[2. * nf]);
                est.push(mean);
                ci_mu.push(2. * nf * mean / chi(1. - alpha / 2.));
                ci_mu.push(2. * nf * mean / chi(alpha / 2.));
                continue;
            }
            let ss: f64 = sample.iter().map(|v| (v - mean) * (v - mean)).sum();
            let sigma = if n == 0 {
                f64::NAN
            } else if n == 1 {
                0.
            } else {
                (ss / (nf - 1.)).sqrt()
            };
            let nu = nf - 1.;
            let t = Family::T.inv(1. - alpha / 2., &[nu]);
            let half = t * sigma / nf.sqrt();
            est.push(mean);
            spread.push(sigma);
            ci_mu.push(mean - half);
            ci_mu.push(mean + half);
            let chi = |p: f64| Family::Chi2.inv(p, &[nu]);
            ci_sigma.push(sigma * (nu / chi(1. - alpha / 2.)).sqrt());
            ci_sigma.push(sigma * (nu / chi(alpha / 2.)).sqrt());
        }
        let row = |v: Vec<f64>| Matrix::from_column_vec(1, columns, v).map(Value::Matrix);
        let pair = |v: Vec<f64>| Matrix::from_column_vec(2, columns, v).map(Value::Matrix);
        let outputs = if name == "expfit" {
            vec![row(est)?, pair(ci_mu)?]
        } else {
            vec![row(est)?, row(spread)?, pair(ci_mu)?, pair(ci_sigma)?]
        };
        Ok(outputs.into_iter().take(count).collect())
    }
}
