//! Value-semantic volatility models. Dense lag vectors retain interior zero lags;
//! public coefficient cells and lag lists expose the nonzero/unknown terms.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Garch,
    Egarch,
    Gjr,
}
#[derive(Clone, Debug)]
pub struct Model {
    pub kind: Kind,
    pub constant: f64,
    pub garch: Vec<f64>,
    pub arch: Vec<f64>,
    pub leverage: Vec<f64>,
    pub offset: f64,
    /// None is Gaussian; Some(NaN) is Student t with estimable DoF.
    pub dof: Option<f64>,
    pub description: Option<String>,
    pub series_name: String,
    pub(crate) fit: Option<Arc<Fit>>,
}
#[derive(Clone, Debug)]
pub(crate) struct Fit {
    pub values: Vec<f64>,
    pub names: Vec<String>,
    pub covariance: Matrix,
    pub sample_size: usize,
    pub free: usize,
    pub log_likelihood: f64,
}
impl Model {
    pub fn class(&self) -> &'static str {
        match self.kind {
            Kind::Garch => "garch",
            Kind::Egarch => "egarch",
            Kind::Gjr => "gjr",
        }
    }
    pub fn p(&self) -> usize {
        self.garch.len()
    }
    pub fn q(&self) -> usize {
        self.arch.len().max(self.leverage.len())
    }
    pub fn variance_presample(&self) -> usize {
        if self.kind == Kind::Egarch {
            self.p().max(self.q())
        } else {
            self.p()
        }
    }
    pub fn unconditional_variance(&self) -> f64 {
        let garch_sum: f64 = self.garch.iter().sum();
        match self.kind {
            Kind::Garch => self.constant / (1. - self.arch.iter().sum::<f64>() - garch_sum),
            Kind::Gjr => {
                self.constant
                    / (1.
                        - self.arch.iter().sum::<f64>()
                        - garch_sum
                        - 0.5 * self.leverage.iter().sum::<f64>())
            }
            Kind::Egarch => (self.constant / (1. - garch_sum)).exp(),
        }
    }
    pub fn bytes(&self) -> usize {
        512 + (self.garch.len() + self.arch.len() + self.leverage.len()) * 8
            + self.description.as_ref().map_or(0, String::len)
            + self.series_name.len()
            + self.fit.as_ref().map_or(0, |f| {
                f.covariance.len() * 8
                    + f.values.len() * 8
                    + f.names.iter().map(String::len).sum::<usize>()
            })
    }
    pub fn description(&self) -> String {
        self.description.clone().unwrap_or_else(|| {
            format!(
                "{}({},{}) Conditional Variance Model{} ({} Distribution)",
                self.class().to_uppercase(),
                self.p(),
                self.q(),
                if self.offset != 0. {
                    " with Offset"
                } else {
                    ""
                },
                if self.dof.is_some() { "t" } else { "Gaussian" }
            )
        })
    }
    pub fn complete(&self) -> Result<()> {
        if self.parameters().iter().any(|x| !x.is_finite()) {
            return Err(err(
                self.class(),
                "UnspecifiedParameters",
                "Model must be fully specified without NaN parameters.",
            ));
        }
        self.validate()
    }
    /// Documentation parameter order, including fixed coefficients.
    pub fn parameters(&self) -> Vec<f64> {
        let mut v = vec![self.constant];
        for c in [&self.garch, &self.arch, &self.leverage] {
            v.extend(c.iter().filter(|&&x| x != 0.).copied());
        }
        if let Some(nu) = self.dof {
            v.push(nu);
        }
        if self.offset != 0. {
            v.push(self.offset);
        }
        v
    }
    pub fn parameter_names(&self) -> Vec<String> {
        let mut v = vec!["Constant".into()];
        for (n, c) in [
            ("GARCH", &self.garch),
            ("ARCH", &self.arch),
            ("Leverage", &self.leverage),
        ] {
            for (j, &x) in c.iter().enumerate() {
                if x != 0. {
                    v.push(format!("{n}{{{}}}", j + 1));
                }
            }
        }
        if self.dof.is_some() {
            v.push("DoF".into());
        }
        if self.offset != 0. {
            v.push("Offset".into());
        }
        v
    }
    pub fn validate(&self) -> Result<()> {
        let bad = |message| err(self.class(), "InvalidParameter", message);
        if self.kind == Kind::Garch && !self.leverage.is_empty() {
            return Err(bad("GARCH models have no leverage polynomial."));
        }
        if self.p() > MAX_LAG || self.q() > MAX_LAG {
            return Err(bad("Polynomial degree exceeds the supported limit of 128."));
        }
        if self.p() > 0 && self.q() == 0 {
            return Err(bad(
                "A positive GARCH degree requires a positive ARCH or leverage degree.",
            ));
        }
        if self.constant.is_infinite() || (self.kind != Kind::Egarch && self.constant <= 0.) {
            return Err(bad(
                "Constant must be a positive scalar or NaN (any real scalar for EGARCH).",
            ));
        }
        if self.offset.is_infinite() {
            return Err(bad("Offset must be a finite scalar or NaN."));
        }
        if let Some(nu) = self.dof
            && (nu.is_infinite() || nu <= 2.)
        {
            return Err(bad("Student t DoF must exceed 2 or be NaN."));
        }
        for c in [&self.garch, &self.arch, &self.leverage] {
            if c.iter().any(|x| x.is_infinite()) {
                return Err(bad("Coefficients must be finite or NaN."));
            }
        }
        if self.kind == Kind::Egarch {
            if self.garch.iter().all(|x| x.is_finite()) && !stationary(&self.garch) {
                return Err(err(
                    self.class(),
                    "NonStationary",
                    "All roots of the GARCH polynomial must lie outside the unit circle.",
                ));
            }
        } else {
            if self.garch.iter().chain(&self.arch).any(|x| *x < 0.) {
                return Err(bad(
                    "GARCH and ARCH coefficients must be nonnegative or NaN.",
                ));
            }
            for j in 0..self.q() {
                let a = *self.arch.get(j).unwrap_or(&0.);
                let l = *self.leverage.get(j).unwrap_or(&0.);
                if a + l < 0. {
                    return Err(bad("ARCH plus Leverage must be nonnegative at every lag."));
                }
            }
            // The minimum contribution of a partially specified GJR lag is half
            // its known ARCH or negative-shock coefficient, not NaN-as-zero.
            let mut persistence = self.garch.iter().filter(|x| x.is_finite()).sum::<f64>();
            for j in 0..self.q() {
                let a = *self.arch.get(j).unwrap_or(&0.);
                let l = *self.leverage.get(j).unwrap_or(&0.);
                persistence += if a.is_nan() {
                    if l.is_nan() { 0. } else { l.abs() / 2. }
                } else if l.is_nan() {
                    a / 2.
                } else {
                    a + l / 2.
                };
            }
            if persistence >= 1. {
                return Err(err(
                    self.class(),
                    "NonStationary",
                    "Covariance stationarity requires sum(GARCH)+sum(ARCH)+sum(Leverage)/2 < 1.",
                ));
            }
        }
        Ok(())
    }
    pub fn unconditional_scale(&self) -> Result<f64> {
        self.complete()?;
        let g = self.garch.iter().sum::<f64>();
        let v = if self.kind == Kind::Egarch {
            (self.constant / (1. - g)).exp()
        } else {
            self.constant
                / (1. - g - self.arch.iter().sum::<f64>() - 0.5 * self.leverage.iter().sum::<f64>())
        };
        if v > 0. && v.is_finite() {
            Ok(v)
        } else {
            Err(err(
                self.class(),
                "InvalidVariance",
                "Unconditional variance scale is not finite and positive.",
            ))
        }
    }
}
/// Schur step-down recursion for the real AR polynomial 1-sum(phi_j L^j).
fn stationary(phi: &[f64]) -> bool {
    let mut a = phi.to_vec();
    while let Some(&k) = a.last() {
        if k.abs() >= 1. {
            return false;
        }
        let n = a.len() - 1;
        let b = (0..n)
            .map(|i| (a[i] + k * a[n - 1 - i]) / (1. - k * k))
            .collect();
        a = b;
    }
    true
}
pub(super) fn construct(e: &Engine, name: &str, args: &[Value]) -> Result<Model> {
    let mut m = Model {
        kind: match name {
            "egarch" => Kind::Egarch,
            "gjr" => Kind::Gjr,
            _ => Kind::Garch,
        },
        constant: f64::NAN,
        garch: vec![],
        arch: vec![],
        leverage: vec![],
        offset: 0.,
        dof: None,
        description: None,
        series_name: "Y".into(),
        fit: None,
    };
    let mut start = 0;
    if args.first().is_some_and(|v| text(v).is_err()) {
        if args.len() < 2 {
            return Err(err(name, "InvalidOrder", "Specify both P and Q."));
        }
        let p = order(e, &args[0], "P", true, MAX_LAG)?;
        let q = order(e, &args[1], "Q", true, MAX_LAG)?;
        m.garch = vec![f64::NAN; p];
        m.arch = vec![f64::NAN; q];
        if m.kind != Kind::Garch {
            m.leverage = vec![f64::NAN; q];
        }
        start = 2;
    }
    let opts = pairs(
        &args[start..],
        &[
            "Constant",
            "GARCH",
            "ARCH",
            "Leverage",
            "GARCHLags",
            "ARCHLags",
            "LeverageLags",
            "Offset",
            "Distribution",
            "Description",
            "SeriesName",
        ],
    )?;
    for (base, target) in [
        ("GARCH", &mut m.garch),
        ("ARCH", &mut m.arch),
        ("Leverage", &mut m.leverage),
    ] {
        if base == "Leverage"
            && m.kind == Kind::Garch
            && (opts.contains_key("leverage") || opts.contains_key("leveragelags"))
        {
            return Err(err(
                name,
                "InvalidProperty",
                "GARCH models have no Leverage property.",
            ));
        }
        let key = base.to_lowercase();
        let lagkey = format!("{key}lags");
        let coeff = opts
            .get(key.as_str())
            .map(|v| coefficients(e, v))
            .transpose()?;
        let lags = opts
            .get(lagkey.as_str())
            .map(|v| lag_vector(e, v))
            .transpose()?;
        if let Some(lags) = lags {
            if coeff.as_ref().is_some_and(|c| c.len() != lags.len()) {
                return Err(err(
                    name,
                    "InvalidLags",
                    "Coefficients and lag vectors must have equal lengths.",
                ));
            }
            *target = vec![0.; lags.iter().copied().max().unwrap_or(0)];
            for (i, &lag) in lags.iter().enumerate() {
                target[lag - 1] = coeff.as_ref().map_or(f64::NAN, |c| c[i]);
            }
        } else if let Some(c) = coeff {
            *target = c;
        }
        clean(target);
    }
    for (name, v) in opts {
        if !matches!(
            name.as_str(),
            "garch" | "arch" | "leverage" | "garchlags" | "archlags" | "leveragelags"
        ) {
            set(e, &mut m, &name, v)?;
        }
    }
    m.validate()?;
    Ok(m)
}
fn clean(c: &mut Vec<f64>) {
    for x in c.iter_mut() {
        if x.abs() <= 1e-12 {
            *x = 0.;
        }
    }
    while c.last() == Some(&0.) {
        c.pop();
    }
}
fn lag_vector(e: &Engine, v: &Value) -> Result<Vec<usize>> {
    let m = matrix(e, v)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(err(
            "model",
            "InvalidLags",
            "Lags must be a vector of unique positive integers.",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    m.data()
        .iter()
        .map(|&x| {
            if !x.is_finite()
                || x < 1.
                || x.fract() != 0.
                || x > MAX_LAG as f64
                || !seen.insert(x as usize)
            {
                Err(err(
                    "model",
                    "InvalidLags",
                    "Lags must be unique positive integers at most 128.",
                ))
            } else {
                Ok(x as usize)
            }
        })
        .collect()
}
fn coefficients(e: &Engine, v: &Value) -> Result<Vec<f64>> {
    let a = v.clone().array(&e.cancel)?;
    let c = if let Array::Cell(c) = a {
        if c.rows() > 1 && c.cols() > 1 {
            return Err(err(
                "model",
                "InvalidCoefficient",
                "Coefficients must be a cell vector.",
            ));
        }
        (0..c.len())
            .map(|i| scalar(e, &Value::from(c.get_linear(i)?.as_ref().clone())))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![scalar(e, v)?]
    };
    if c.len() > MAX_LAG {
        return Err(err(
            "model",
            "InvalidOrder",
            "At most 128 lags are supported.",
        ));
    }
    Ok(c)
}
fn set(e: &Engine, m: &mut Model, n: &str, v: &Value) -> Result<()> {
    match n.to_lowercase().as_str() {
        "constant" => m.constant = scalar(e, v)?,
        "offset" => m.offset = scalar(e, v)?,
        "garch" => {
            m.garch = coefficients(e, v)?;
            clean(&mut m.garch);
        }
        "arch" => {
            m.arch = coefficients(e, v)?;
            clean(&mut m.arch);
        }
        "leverage" if m.kind != Kind::Garch => {
            m.leverage = coefficients(e, v)?;
            clean(&mut m.leverage);
        }
        "description" => {
            let t = text(v)?;
            StringArray::scalar(t.as_str())?;
            m.description = Some(t);
        }
        "seriesname" => {
            let t = text(v)?;
            StringArray::scalar(t.as_str())?;
            m.series_name = t;
        }
        "distribution" => {
            let (n, d) = if let Ok(t) = text(v) {
                (t, None)
            } else {
                let a = v.clone().array(&e.cancel)?;
                let Array::Struct(s) = a else {
                    return Err(err(
                        m.class(),
                        "InvalidDistribution",
                        "Distribution must be Gaussian, t, or a scalar structure.",
                    ));
                };
                if s.len() != 1 {
                    return Err(err(
                        m.class(),
                        "InvalidDistribution",
                        "Distribution must be a scalar structure.",
                    ));
                }
                let n = s
                    .field_names()
                    .iter()
                    .position(|n| n == "Name")
                    .ok_or("Distribution requires Name")?;
                let t = text(&Value::from(s.get(n, 0)?.as_ref().clone()))?;
                let d = s
                    .field_names()
                    .iter()
                    .position(|n| n == "DoF")
                    .map(|i| scalar(e, &Value::from(s.get(i, 0)?.as_ref().clone())))
                    .transpose()?;
                if t.eq_ignore_ascii_case("t") && d.is_none() {
                    return Err(err(
                        m.class(),
                        "InvalidDistribution",
                        "A t distribution structure requires DoF.",
                    ));
                }
                (t, d)
            };
            m.dof = if n.eq_ignore_ascii_case("gaussian") {
                None
            } else if n.eq_ignore_ascii_case("t") {
                Some(d.unwrap_or(f64::NAN))
            } else {
                return Err(err(
                    m.class(),
                    "InvalidDistribution",
                    "Distribution must be Gaussian or t.",
                ));
            };
        }
        _ => {
            return Err(err(
                m.class(),
                "InvalidProperty",
                "Unrecognized or read-only model property.",
            ));
        }
    }
    Ok(())
}
pub(in crate::engine) fn property(m: &Model, n: &str) -> Result<Value> {
    let coeff = |c: &[f64]| -> Result<Value> {
        Ok(Value::from(Array::Cell(CellArray::from_shared(
            1,
            c.len(),
            // Lag-indexed like MATLAB (GARCH{i} is lag i); display stays compact.
            c.iter()
                .map(|&x| Arc::new(Array::Numeric(Matrix::scalar(x))))
                .collect(),
            &AtomicBool::new(false),
        )?)))
    };
    let lags = |c: &[f64]| -> Result<Value> {
        let v = c
            .iter()
            .enumerate()
            .filter(|&(_, x)| *x != 0.)
            .map(|(j, _)| (j + 1) as f64)
            .collect::<Vec<_>>();
        Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
    };
    match n {
        "P" => Ok(number(m.p() as f64)),
        "Q" => Ok(number(m.q() as f64)),
        "Constant" => Ok(number(m.constant)),
        "Offset" => Ok(number(m.offset)),
        "UnconditionalVariance" => Ok(number(m.unconditional_variance())),
        "Description" => string(&m.description()),
        "SeriesName" => string(&m.series_name),
        "GARCH" => coeff(&m.garch),
        "ARCH" => coeff(&m.arch),
        "Leverage" if m.kind != Kind::Garch => coeff(&m.leverage),
        "GARCHLags" => lags(&m.garch),
        "ARCHLags" => lags(&m.arch),
        "LeverageLags" if m.kind != Kind::Garch => lags(&m.leverage),
        "Distribution" => {
            let mut f = vec![(
                "Name",
                string_array(if m.dof.is_some() { "t" } else { "Gaussian" })?,
            )];
            if let Some(d) = m.dof {
                f.push(("DoF", Array::Numeric(Matrix::scalar(d))));
            }
            structure(f)
        }
        _ => Err(err(
            m.class(),
            "InvalidProperty",
            "Unrecognized model property.",
        )),
    }
}
pub(in crate::engine) fn assign(
    e: &mut Engine,
    m: &Model,
    steps: &[crate::syntax::PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<Model> {
    use crate::syntax::PathStep;
    let Some((first, rest)) = steps.split_first() else {
        return Err(err(
            m.class(),
            "InvalidAssignment",
            "Expected a model property.",
        ));
    };
    let n = match first {
        PathStep::Field(n) => n.clone(),
        PathStep::Dynamic(x) => e.dynamic_field_name(x, out)?,
        _ => {
            return Err(err(
                m.class(),
                "InvalidAssignment",
                "Expected a model property.",
            ));
        }
    };
    if matches!(
        n.as_str(),
        "P" | "Q" | "GARCHLags" | "ARCHLags" | "LeverageLags" | "UnconditionalVariance"
    ) {
        return Err(err(
            m.class(),
            "ReadOnlyProperty",
            if n == "UnconditionalVariance" {
                "UnconditionalVariance is read-only."
            } else {
                "Polynomial degrees and lag lists are read-only."
            },
        ));
    }
    let value = if rest.is_empty() {
        value
    } else {
        e.assign_step(
            Some(property(m, &n)?.array(&e.cancel)?),
            rest,
            value,
            false,
            out,
        )?
    };
    let mut changed = m.clone();
    set(e, &mut changed, &n, &Value::from(value))?;
    changed.fit = None;
    changed.validate()?;
    Ok(changed)
}
