use super::*;
use crate::syntax::PathStep;
#[derive(Clone)]
pub(in crate::engine) struct Model {
    pub c: f64,
    pub ar: Vec<f64>,
    pub sar: Vec<f64>,
    pub ma: Vec<f64>,
    pub sma: Vec<f64>,
    pub d: usize,
    pub s: usize,
    pub beta: Vec<f64>,
    pub variance: f64,
    pub conditional: Option<composite::Variance>,
    pub dof: Option<f64>,
    pub description: Option<String>,
    pub series: String,
    pub fit: Option<Arc<Fit>>,
}
#[derive(Clone)]
pub struct Fit {
    pub n: usize,
    pub free: usize,
    pub logl: f64,
    pub covariance: Vec<f64>,
    pub params: Vec<f64>,
    pub names: Vec<String>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            c: f64::NAN,
            ar: vec![],
            sar: vec![],
            ma: vec![],
            sma: vec![],
            d: 0,
            s: 0,
            beta: vec![],
            variance: f64::NAN,
            conditional: None,
            dof: None,
            description: None,
            series: "Y".into(),
            fit: None,
        }
    }
}
impl Model {
    pub fn p(&self) -> usize {
        self.ar.len() + self.sar.len() + self.d + self.s
    }
    pub fn q(&self) -> usize {
        self.ma.len() + self.sma.len()
    }
    pub fn bytes(&self) -> usize {
        512 + 8
            * (self.ar.len() + self.sar.len() + self.ma.len() + self.sma.len() + self.beta.len())
            + self.conditional.as_ref().map_or(0, |v| v.model.bytes())
            + self.series.len()
            + self.description.as_ref().map_or(0, String::len)
            + self.fit.as_ref().map_or(0, |f| {
                8 * (f.params.len() + f.covariance.len())
                    + f.names.iter().map(String::len).sum::<usize>()
            })
    }
    pub fn description(&self) -> String {
        self.description.clone().unwrap_or_else(|| {
            let lags_str = |c: &[f64]| -> String {
                c.iter()
                    .enumerate()
                    .filter(|&(_, x)| *x != 0.)
                    .map(|(i, _)| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let sar = lags_str(&self.sar);
            let sma = lags_str(&self.sma);
            let mut seasonal = String::new();
            if self.s > 0 {
                seasonal.push_str(" Seasonally Integrated");
            }
            match (sar.is_empty(), sma.is_empty()) {
                (false, false) => {
                    seasonal.push_str(&format!(" with Seasonal AR({sar}) and MA({sma})"))
                }
                (false, true) => seasonal.push_str(&format!(" with Seasonal AR({sar})")),
                (true, false) => seasonal.push_str(&format!(" with Seasonal MA({sma})")),
                (true, true) => {}
            }
            format!(
                "{}({},{},{}) Model{}{} ({} Distribution)",
                if self.beta.is_empty() {
                    "ARIMA"
                } else {
                    "ARIMAX"
                },
                self.ar.len(),
                self.d,
                self.ma.len(),
                seasonal,
                self.conditional
                    .as_ref()
                    .map_or_else(String::new, |v| format!(
                        " with {}({},{}) Conditional Variance",
                        v.model.class().to_uppercase(),
                        v.model.p(),
                        v.model.q()
                    )),
                if self.dof.is_some() { "t" } else { "Gaussian" }
            )
        })
    }
    pub fn fully_specified(&self) -> Result<()> {
        if self.parameters().iter().any(|x| !x.is_finite()) {
            return Err(err(
                "arima",
                "UnspecifiedParameter",
                "Model must be fully specified; estimate NaN parameters first.",
            ));
        }
        if let Some(v) = &self.conditional {
            v.model.complete()?;
        }
        self.validate()
    }
    pub fn parameters(&self) -> Vec<f64> {
        let mut p = vec![self.c];
        for a in [&self.ar, &self.sar, &self.ma, &self.sma] {
            p.extend(a.iter().filter(|x| **x != 0.).copied());
        }
        p.extend(&self.beta);
        if let Some(v) = &self.conditional {
            let mut parameters = v.model.parameters();
            // The innovation distribution belongs to the composition; count
            // its DoF only once, below, even if the source variance used t.
            if v.model.dof.is_some() {
                parameters.pop();
            }
            p.extend(parameters);
        } else {
            p.push(self.variance);
        }
        if let Some(v) = self.dof {
            p.push(v);
        }
        p
    }
    pub fn parameter_names(&self) -> Vec<String> {
        let mut n = vec!["Constant".into()];
        for (name, a) in [
            ("AR", &self.ar),
            ("SAR", &self.sar),
            ("MA", &self.ma),
            ("SMA", &self.sma),
        ] {
            n.extend(
                a.iter()
                    .enumerate()
                    .filter(|(_, v)| **v != 0.)
                    .map(|(i, _)| format!("{name}{{{}}}", i + 1)),
            );
        }
        n.extend((1..=self.beta.len()).map(|i| format!("Beta({i})")));
        if let Some(v) = &self.conditional {
            n.extend(
                v.model
                    .parameter_names()
                    .into_iter()
                    .filter(|s| s != "DoF")
                    .map(|s| format!("Variance.{s}")),
            );
        } else {
            n.push("Variance".into());
        }
        if self.dof.is_some() {
            n.push("DoF".into());
        }
        n
    }
    // Keep the structural zero mask from the template even when an estimate is zero.
    pub fn with_parameters(&self, p: &[f64]) -> Self {
        let mut m = self.clone();
        let mut k = 1;
        m.c = p[0];
        for a in [&mut m.ar, &mut m.sar, &mut m.ma, &mut m.sma] {
            for v in a {
                if *v != 0. {
                    *v = p[k];
                    k += 1;
                }
            }
        }
        for v in &mut m.beta {
            *v = p[k];
            k += 1;
        }
        if let Some(v) = &mut m.conditional {
            let vm = Arc::make_mut(&mut v.model);
            vm.fit = None;
            vm.constant = p[k];
            k += 1;
            for a in [&mut vm.garch, &mut vm.arch, &mut vm.leverage] {
                for x in a {
                    if *x != 0. {
                        *x = p[k];
                        k += 1;
                    }
                }
            }
            vm.dof = self.dof.map(|_| *p.last().unwrap());
        } else {
            m.variance = p[k];
            k += 1;
        }
        if m.dof.is_some() {
            m.dof = Some(p[k]);
        }
        m
    }
    pub fn validate(&self) -> Result<()> {
        if self.p() > 4 * MAX_LAG
            || self.q() > 2 * MAX_LAG
            || self.parameters().len() > MAX_PARAMETERS
        {
            return Err(err(
                "arima",
                "SizeLimit",
                "Model degree or number of parameters exceeds the supported limit.",
            ));
        }
        if self.c.is_infinite()
            || self.beta.iter().any(|x| x.is_infinite())
            || (self.conditional.is_none()
                && !self.variance.is_nan()
                && (!self.variance.is_finite() || self.variance <= 0.))
        {
            return Err(err(
                "arima",
                "InvalidParameter",
                "Constant and Beta must be finite or NaN; Variance must be positive or NaN.",
            ));
        }
        if self
            .dof
            .is_some_and(|v| !v.is_nan() && (!v.is_finite() || v <= 2.))
        {
            return Err(err(
                "arima",
                "InvalidDoF",
                "Student t degrees of freedom must exceed 2 or be NaN.",
            ));
        }
        for (name, a, sign) in [
            ("AR", &self.ar, -1.),
            ("SAR", &self.sar, -1.),
            ("MA", &self.ma, 1.),
            ("SMA", &self.sma, 1.),
        ] {
            if a.iter().any(|x| x.is_infinite()) {
                return Err(err(
                    "arima",
                    "InvalidCoefficients",
                    "Coefficients must be finite or NaN.",
                ));
            }
            if a.iter().all(|x| x.is_finite()) && !stable(a, sign) {
                return Err(err(
                    "arima",
                    if sign < 0. {
                        "NonStationary"
                    } else {
                        "NonInvertible"
                    },
                    &format!(
                        "{name} polynomial must be {}.",
                        if sign < 0. {
                            "stationary"
                        } else {
                            "invertible"
                        }
                    ),
                ));
            }
        }
        Ok(())
    }
    pub fn polynomials(&self) -> (Vec<f64>, Vec<f64>) {
        let poly = |a: &[f64], sign: f64| {
            std::iter::once(1.)
                .chain(a.iter().map(|x| sign * x))
                .collect::<Vec<_>>()
        };
        let mut a = convolve(&poly(&self.ar, -1.), &poly(&self.sar, -1.));
        for _ in 0..self.d {
            a = convolve(&a, &[1., -1.]);
        }
        if self.s > 0 {
            let mut s = vec![0.; self.s + 1];
            s[0] = 1.;
            s[self.s] = -1.;
            a = convolve(&a, &s);
        }
        (a, convolve(&poly(&self.ma, 1.), &poly(&self.sma, 1.)))
    }
}
// Schur-Cohn step-down test. Roots of 1+a1 L+... lie outside the unit circle
// iff every reflection coefficient has modulus <1. No eigenproblem or inversion.
fn stable(c: &[f64], sign: f64) -> bool {
    let mut a: Vec<_> = std::iter::once(1.)
        .chain(c.iter().map(|x| sign * x))
        .collect();
    for k in (1..a.len()).rev() {
        let r = a[k];
        if !r.is_finite() || r.abs() >= 1. - 1e-12 {
            return false;
        }
        let den = 1. - r * r;
        let mut next = vec![1.; k];
        for i in 1..k {
            next[i] = (a[i] - r * a[k - i]) / den;
        }
        a = next;
    }
    true
}
pub(super) fn convolve(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut c = vec![0.; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            c[i + j] += x * y;
        }
    }
    c
}
const PROPS: &[&str] = &[
    "Constant",
    "AR",
    "SAR",
    "MA",
    "SMA",
    "D",
    "Seasonality",
    "Beta",
    "Variance",
    "Distribution",
    "Description",
    "SeriesName",
    "ARLags",
    "SARLags",
    "MALags",
    "SMALags",
];
pub(super) fn construct(e: &Engine, a: &[Value]) -> Result<Model> {
    let mut m = Model::default();
    if a.len() == 3 && text(&a[0]).is_err() {
        m.ar = vec![f64::NAN; integer(e, &a[0], 0, MAX_LAG)?];
        m.d = integer(e, &a[1], 0, MAX_LAG)?;
        m.ma = vec![f64::NAN; integer(e, &a[2], 0, MAX_LAG)?];
        m.validate()?;
        return Ok(m);
    }
    let o = options(e, "arima", a, PROPS)?;
    for name in ["AR", "SAR", "MA", "SMA"] {
        let lags = o
            .get(&format!("{name}Lags"))
            .map(|v| lag_vector(e, v))
            .transpose()?;
        let vals = o.get(name).map(|v| coefficients(e, v)).transpose()?;
        let mut c = match (lags, vals) {
            (Some(l), v) => {
                let v = v.unwrap_or_else(|| vec![f64::NAN; l.len()]);
                if v.len() != l.len() {
                    return Err(err(
                        "arima",
                        "LagSizeMismatch",
                        "Coefficient and lag vectors must have equal lengths.",
                    ));
                }
                let mut c = vec![0.; l.iter().copied().max().unwrap_or(0)];
                for (lag, x) in l.into_iter().zip(v) {
                    c[lag - 1] = x;
                }
                c
            }
            (None, v) => v.unwrap_or_default(),
        };
        trim(&mut c);
        match name {
            "AR" => m.ar = c,
            "SAR" => m.sar = c,
            "MA" => m.ma = c,
            _ => m.sma = c,
        }
    }
    for (k, v) in &o {
        if !matches!(k.as_str(), "AR" | "SAR" | "MA" | "SMA") && !k.ends_with("Lags") {
            set(e, &mut m, k, v)?;
        }
    }
    m.validate()?;
    Ok(m)
}
fn lag_vector(e: &Engine, v: &Value) -> Result<Vec<usize>> {
    let m = matrix(e, v)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(err("arima", "InvalidLags", "Lags must be a vector."));
    }
    let mut l = vec![];
    for &x in m.data() {
        if !x.is_finite()
            || x.fract() != 0.
            || x < 1.
            || x > MAX_LAG as f64
            || l.contains(&(x as usize))
        {
            return Err(err(
                "arima",
                "InvalidLags",
                "Lags must be unique positive integers no greater than 256.",
            ));
        }
        l.push(x as usize);
    }
    Ok(l)
}
fn trim(a: &mut Vec<f64>) {
    for x in a.iter_mut() {
        if x.abs() <= 1e-12 {
            *x = 0.;
        }
    }
    while a.last() == Some(&0.) {
        a.pop();
    }
}
fn set(e: &Engine, m: &mut Model, k: &str, v: &Value) -> Result<()> {
    match k {
        "P" | "Q" => {
            return Err(err(
                "arima",
                "ReadOnlyProperty",
                "P and Q are read-only derived polynomial degrees.",
            ));
        }
        "Constant" => m.c = scalar(e, v)?,
        "Variance" => {
            if let Some(model) = super::super::econ_vol::model_value(e, v) {
                if model.offset != 0. {
                    return Err(err(
                        "arima",
                        "InvalidOffset",
                        "The conditional variance model Offset must be 0.",
                    ));
                }
                m.conditional = Some(composite::Variance {
                    model,
                    object: v.clone().array(&e.cancel)?,
                });
            } else {
                m.variance = scalar(e, v).map_err(|_| {
                    err(
                        "arima",
                        "InvalidVariance",
                        "Variance must be a positive scalar, NaN, or a garch, egarch or gjr model.",
                    )
                })?;
                m.conditional = None;
            }
        }
        "D" => m.d = integer(e, v, 0, MAX_LAG)?,
        "Seasonality" => m.s = integer(e, v, 0, MAX_LAG)?,
        "Beta" => m.beta = coefficients(e, v)?,
        "Description" => m.description = Some(text(v)?),
        "SeriesName" => m.series = text(v)?,
        "AR" | "SAR" | "MA" | "SMA" => {
            let mut a = coefficients(e, v)?;
            trim(&mut a);
            match k {
                "AR" => m.ar = a,
                "SAR" => m.sar = a,
                "MA" => m.ma = a,
                _ => m.sma = a,
            }
        }
        "Distribution" => {
            let a = v.clone().array(&e.cancel)?;
            let (name, dof) = if let Array::Struct(s) = a {
                if s.len() != 1 {
                    return Err(err(
                        "arima",
                        "InvalidDistribution",
                        "Distribution must be a scalar structure.",
                    ));
                }
                let n = text(&Value::from(
                    s.get(
                        s.field_index("Name")
                            .ok_or("arima: Distribution requires Name")?,
                        0,
                    )?
                    .as_ref()
                    .clone(),
                ))?;
                let df = if n.eq_ignore_ascii_case("t") {
                    Some(scalar(
                        e,
                        &Value::from(
                            s.get(
                                s.field_index("DoF")
                                    .ok_or("arima: t Distribution requires DoF")?,
                                0,
                            )?
                            .as_ref()
                            .clone(),
                        ),
                    )?)
                } else {
                    None
                };
                (n, df)
            } else {
                (text(v)?, Some(f64::NAN))
            };
            m.dof = match name.to_ascii_lowercase().as_str() {
                "gaussian" => None,
                "t" => dof,
                _ => {
                    return Err(err(
                        "arima",
                        "InvalidDistribution",
                        "Distribution must be Gaussian or t.",
                    ));
                }
            };
        }
        _ => {
            return Err(err(
                "arima",
                "UnknownProperty",
                &format!("Unrecognized property '{k}' for class 'arima'."),
            ));
        }
    }
    Ok(())
}
pub(in crate::engine) fn assign(
    e: &mut Engine,
    m: &Model,
    steps: &[PathStep],
    v: Array,
    out: &mut Execution,
) -> Result<Model> {
    let Some((first, rest)) = steps.split_first() else {
        return Err(err(
            "arima",
            "InvalidAssignment",
            "Expected property assignment.",
        ));
    };
    let name = match first {
        PathStep::Field(s) => s.clone(),
        PathStep::Dynamic(x) => e.dynamic_field_name(x, out)?,
        _ => {
            return Err(err(
                "arima",
                "InvalidAssignment",
                "Expected property assignment.",
            ));
        }
    };
    let v = if rest.is_empty() {
        v
    } else if name == "Variance"
        && let Some(variance) = &m.conditional
    {
        composite::assign_variance(e, variance, rest, v, out)?
    } else {
        let old = property(e, m, &name)?.array(&e.cancel)?;
        e.assign_step(Some(old), rest, v, false, out)?
    };
    let mut changed = m.clone();
    changed.fit = None;
    set(e, &mut changed, &name, &Value::from(v))?;
    changed.validate()?;
    Ok(changed)
}
