//! Data, censoring and frequency vectors of a fit, and statset options.
use super::super::{Engine, Value};
use super::optimize::Limits;
use crate::{Array, Result};

/// Most observations a fit accepts (each likelihood pass is O(n)).
pub(super) const MAX_OBSERVATIONS: usize = 1_000_000;

#[derive(Clone, Debug, Default)]
pub(super) struct Sample {
    pub x: Vec<f64>,
    /// Right-censored flags, one per observation.
    pub cens: Vec<bool>,
    /// Frequencies, one per observation (1 when none were given).
    pub freq: Vec<f64>,
    pub cens_given: bool,
    pub freq_given: bool,
}

impl Sample {
    pub fn from_parts(x: Vec<f64>, cens: Vec<bool>, freq: Vec<f64>) -> Self {
        let n = x.len();
        let cens_given = !cens.is_empty();
        let freq_given = !freq.is_empty();
        Self {
            cens: if cens_given { cens } else { vec![false; n] },
            freq: if freq_given { freq } else { vec![1.; n] },
            x,
            cens_given,
            freq_given,
        }
    }
    pub fn plain(x: Vec<f64>) -> Self {
        Self::from_parts(x, Vec::new(), Vec::new())
    }
    pub fn has_cens(&self) -> bool {
        self.cens.iter().any(|&c| c)
    }
    pub fn has_freq(&self) -> bool {
        self.freq_given
    }
    /// Total frequency.
    pub fn weight(&self) -> f64 {
        self.freq.iter().sum()
    }
    /// Total frequency of the exactly observed values.
    pub fn uncensored_weight(&self) -> f64 {
        self.iter().filter(|o| !o.1).map(|o| o.2).sum()
    }
    /// (x, censored, frequency) per observation.
    pub fn iter(&self) -> impl Iterator<Item = (f64, bool, f64)> + '_ {
        self.x
            .iter()
            .zip(&self.cens)
            .zip(&self.freq)
            .map(|((&x, &c), &w)| (x, c, w))
    }
    /// Weighted mean of the values (all of them, censored or not).
    pub fn mean(&self) -> f64 {
        let w = self.weight();
        let m = self.iter().map(|o| o.0 * o.2).sum::<f64>() / w;
        m + self.iter().map(|o| (o.0 - m) * o.2).sum::<f64>() / w
    }
    /// Weighted standard deviation (divided by W - 1, or W when W <= 1).
    pub fn std(&self) -> f64 {
        let w = self.weight();
        let m = self.mean();
        let ss: f64 = self.iter().map(|o| o.2 * (o.0 - m) * (o.0 - m)).sum();
        (ss / if w > 1. { w - 1. } else { w }).sqrt()
    }
    /// A copy with every value mapped (for the log-scale families).
    pub fn mapped(&self, f: impl Fn(f64) -> f64) -> Self {
        let mut s = self.clone();
        for v in &mut s.x {
            *v = f(*v);
        }
        s
    }
    /// Number of distinct exactly observed values.
    pub fn distinct_uncensored(&self) -> usize {
        let mut v: Vec<f64> = self.iter().filter(|o| !o.1).map(|o| o.0).collect();
        v.sort_by(f64::total_cmp);
        v.dedup();
        v.len()
    }
}

/// A real vector argument as its values (empty for []).
pub(super) fn vector(e: &Engine, who: &str, what: &str, v: &Value) -> Result<Vec<f64>> {
    if v.is_character() {
        return Err(format!("{who}: {what} must be numeric"));
    }
    let m = e.real_arg(who, v)?;
    if m.is_empty() {
        return Ok(Vec::new());
    }
    if m.is_nd() || (m.rows() != 1 && m.cols() != 1) {
        return Err(format!("{who}: {what} must be a real row or column vector"));
    }
    Ok(m.data().to_vec())
}

/// The data, censoring and frequency vectors of a fit with NaN rows and zero
/// frequencies removed.
pub(super) fn sample(
    e: &Engine,
    who: &str,
    data: &Value,
    cens: Option<&Value>,
    freq: Option<&Value>,
) -> Result<Sample> {
    let raw = e.real_arg(who, data)?;
    if raw.is_nd() || (raw.rows() != 1 && raw.cols() != 1) || data.is_character() {
        return Err(format!("{who}: data must be a real row or column vector"));
    }
    let n = raw.len();
    if n > MAX_OBSERVATIONS {
        return Err(format!(
            "{who}: at most {MAX_OBSERVATIONS} observations are supported"
        ));
    }
    e.preflight_elements(n.saturating_mul(3))?;
    let cens = match cens {
        Some(c) => vector(e, who, "the censoring vector", c)?,
        None => Vec::new(),
    };
    let freq = match freq {
        Some(f) => vector(e, who, "the frequency vector", f)?,
        None => Vec::new(),
    };
    for (what, v) in [("censoring", &cens), ("frequency", &freq)] {
        if !v.is_empty() && v.len() != n {
            return Err(format!(
                "{who}: the {what} vector must have one entry per observation"
            ));
        }
    }
    let mut s = Sample {
        cens_given: !cens.is_empty(),
        freq_given: !freq.is_empty(),
        ..Sample::default()
    };
    for (i, &x) in raw.data().iter().enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let c = cens.get(i).copied().unwrap_or(0.);
        let w = freq.get(i).copied().unwrap_or(1.);
        if x.is_nan() || c.is_nan() || w.is_nan() {
            continue;
        }
        if !x.is_finite() {
            return Err(format!(
                "{who}: observations must be finite (NaN is ignored)"
            ));
        }
        if c != 0. && c != 1. {
            return Err(format!(
                "{who}: censoring must be 0 (observed) or 1 (right-censored); left censoring is not supported"
            ));
        }
        if w < 0. || !w.is_finite() {
            return Err(format!(
                "{who}: frequencies must be nonnegative finite numbers"
            ));
        }
        if w == 0. {
            continue;
        }
        s.x.push(x);
        s.cens.push(c == 1.);
        s.freq.push(w);
    }
    if s.x.is_empty() {
        return Err(format!("{who}: no observations remain after ignoring NaN"));
    }
    Ok(s)
}

/// Iteration limits from a statset structure (MaxIter, MaxFunEvals, TolX,
/// TolFun); empty fields keep the defaults.
pub(super) fn limits(e: &Engine, who: &str, v: Option<&Value>, mut base: Limits) -> Result<Limits> {
    let Some(v) = v else {
        return Ok(base);
    };
    if let Value::Matrix(m) = v
        && m.is_empty()
    {
        return Ok(base);
    }
    let Value::Container(Array::Struct(s)) = v else {
        return Err(format!(
            "{who}: 'Options' must be a structure made by statset"
        ));
    };
    for (i, name) in s.field_names().iter().enumerate() {
        let value = Value::from(s.get(i, 0)?.as_ref().clone());
        let number = || -> Result<Option<f64>> {
            if value.is_character() {
                return Ok(None);
            }
            let m = e.real_arg(who, &value)?;
            if m.is_empty() {
                return Ok(None);
            }
            let x = m.as_scalar()?;
            if x.is_nan() || x <= 0. {
                return Err(format!("{who}: option {name} must be a positive number"));
            }
            Ok(Some(x))
        };
        match name.to_ascii_lowercase().as_str() {
            "maxiter" => {
                if let Some(x) = number()? {
                    base.max_iter = x.min(1e9) as usize;
                }
            }
            "maxfunevals" => {
                if let Some(x) = number()? {
                    base.max_evals = x.min(1e9) as usize;
                }
            }
            "tolx" => {
                if let Some(x) = number()? {
                    base.tol_x = x;
                }
            }
            "tolfun" => {
                if let Some(x) = number()? {
                    base.tol_fun = x;
                }
            }
            _ => {}
        }
    }
    Ok(base)
}
