//! Intensity-based registration configurations (`imregconfig`): the optimizer
//! objects `registration.optimizer.RegularStepGradientDescent` and
//! `registration.optimizer.OnePlusOneEvolutionary` and the metric objects
//! `registration.metric.MeanSquares` and
//! `registration.metric.MattesMutualInformation`.
//!
//! Defaults, the documented validation (positive numbers, positive integers,
//! RelaxationFactor in (0, 1), at least 5 histogram bins, a logical
//! UseAllPixels) and the display layout follow the public MathWorks reference
//! pages of the four classes and of imregconfig. No MathWorks code.
//!
//! The objects are values: they live in the engine object table
//! (objects.rs, `ObjState::RegConfig`) and every property change makes a new
//! object, so `o2 = o; o2.MaximumIterations = 300` leaves `o` unchanged.
//! imregister/imregtform read the settings through [`optimizer`] and
//! [`metric`].
use super::super::{Engine, Value};
use crate::{Array, LogicalMatrix, Matrix, Result};

/// Class names, in the order of [`RegConfig::index`].
pub(in crate::engine) const CLASSES: [&str; 4] = [
    "registration.optimizer.RegularStepGradientDescent",
    "registration.optimizer.OnePlusOneEvolutionary",
    "registration.metric.MeanSquares",
    "registration.metric.MattesMutualInformation",
];

/// Regular step gradient descent optimizer settings (reference defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) struct RegularStepGradientDescent {
    pub gradient_magnitude_tolerance: f64,
    pub minimum_step_length: f64,
    pub maximum_step_length: f64,
    /// A positive integer stored as a double, as the property is.
    pub maximum_iterations: f64,
    pub relaxation_factor: f64,
}

impl Default for RegularStepGradientDescent {
    fn default() -> Self {
        Self {
            gradient_magnitude_tolerance: 1e-4,
            minimum_step_length: 1e-5,
            maximum_step_length: 0.0625,
            maximum_iterations: 100.,
            relaxation_factor: 0.5,
        }
    }
}

/// One-plus-one evolutionary optimizer settings (reference defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) struct OnePlusOneEvolutionary {
    pub growth_factor: f64,
    pub epsilon: f64,
    pub initial_radius: f64,
    /// A positive integer stored as a double, as the property is.
    pub maximum_iterations: f64,
}

impl Default for OnePlusOneEvolutionary {
    fn default() -> Self {
        Self {
            growth_factor: 1.05,
            epsilon: 1.5e-6,
            initial_radius: 6.25e-3,
            maximum_iterations: 100.,
        }
    }
}

/// Mattes mutual information metric settings (reference defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) struct MattesMutualInformation {
    /// Positive integer; used only when `use_all_pixels` is false.
    pub number_of_spatial_samples: f64,
    /// Integer of at least 5.
    pub number_of_histogram_bins: f64,
    pub use_all_pixels: bool,
}

impl Default for MattesMutualInformation {
    fn default() -> Self {
        Self {
            number_of_spatial_samples: 500.,
            number_of_histogram_bins: 50.,
            use_all_pixels: true,
        }
    }
}

/// One configuration object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) enum RegConfig {
    StepGradient(RegularStepGradientDescent),
    Evolutionary(OnePlusOneEvolutionary),
    MeanSquares,
    Mattes(MattesMutualInformation),
}

/// The optimizer an imregister/imregtform argument holds.
#[allow(dead_code)] // read by imregister/imregtform (step 7c3)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) enum Optimizer {
    RegularStepGradientDescent(RegularStepGradientDescent),
    OnePlusOneEvolutionary(OnePlusOneEvolutionary),
}

/// The metric an imregister/imregtform argument holds.
#[allow(dead_code)] // read by imregister/imregtform (step 7c3)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) enum Metric {
    MeanSquares,
    MattesMutualInformation(MattesMutualInformation),
}

/// The optimizer settings `v` holds, if it is a registration optimizer object.
#[allow(dead_code)] // read by imregister/imregtform (step 7c3)
pub(in crate::engine) fn optimizer(e: &Engine, v: &Value) -> Option<Optimizer> {
    match super::super::objects::reg_config_value(e, v)? {
        RegConfig::StepGradient(o) => Some(Optimizer::RegularStepGradientDescent(o)),
        RegConfig::Evolutionary(o) => Some(Optimizer::OnePlusOneEvolutionary(o)),
        _ => None,
    }
}

/// The metric settings `v` holds, if it is a registration metric object.
#[allow(dead_code)] // read by imregister/imregtform (step 7c3)
pub(in crate::engine) fn metric(e: &Engine, v: &Value) -> Option<Metric> {
    match super::super::objects::reg_config_value(e, v)? {
        RegConfig::MeanSquares => Some(Metric::MeanSquares),
        RegConfig::Mattes(m) => Some(Metric::MattesMutualInformation(m)),
        _ => None,
    }
}

/// What a property accepts.
#[derive(Clone, Copy)]
enum Rule {
    /// A positive finite real number.
    Positive,
    /// A finite integer of at least the given value.
    Integer(f64),
    /// A real number strictly between 0 and 1.
    Ratio,
    /// Logical true/false or numeric 1/0.
    Flag,
}

const STEP_PROPS: [(&str, Rule); 5] = [
    ("GradientMagnitudeTolerance", Rule::Positive),
    ("MinimumStepLength", Rule::Positive),
    ("MaximumStepLength", Rule::Positive),
    ("MaximumIterations", Rule::Integer(1.)),
    ("RelaxationFactor", Rule::Ratio),
];
const EVOLUTIONARY_PROPS: [(&str, Rule); 4] = [
    ("GrowthFactor", Rule::Positive),
    ("Epsilon", Rule::Positive),
    ("InitialRadius", Rule::Positive),
    ("MaximumIterations", Rule::Integer(1.)),
];
const MATTES_PROPS: [(&str, Rule); 3] = [
    ("NumberOfSpatialSamples", Rule::Integer(1.)),
    ("NumberOfHistogramBins", Rule::Integer(5.)),
    ("UseAllPixels", Rule::Flag),
];

/// C `%e`: six decimals and a signed exponent of at least two digits.
fn scientific(v: f64) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    let text = format!("{v:.6e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}

/// A real scalar of a numeric (or, when `logical`, logical) class.
fn real_scalar(a: &Array, logical: bool) -> Option<f64> {
    if a.is_complex() {
        return None;
    }
    match a {
        Array::Numeric(m) if m.len() == 1 => Some(m.data()[0]),
        Array::Typed(t) if t.len() == 1 => Some(t.real(0)),
        Array::Logical(l) if logical && l.len() == 1 => Some(f64::from(l.data()[0])),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_ref().map(|s| s.to_string())
        }
        _ => None,
    }
}

impl RegConfig {
    /// The object of class `CLASSES[index]` with the reference defaults.
    pub(in crate::engine) fn default_of(index: usize) -> Self {
        match index {
            0 => Self::StepGradient(RegularStepGradientDescent::default()),
            1 => Self::Evolutionary(OnePlusOneEvolutionary::default()),
            2 => Self::MeanSquares,
            _ => Self::Mattes(MattesMutualInformation::default()),
        }
    }
    pub(in crate::engine) fn index(&self) -> usize {
        match self {
            Self::StepGradient(_) => 0,
            Self::Evolutionary(_) => 1,
            Self::MeanSquares => 2,
            Self::Mattes(_) => 3,
        }
    }
    pub(in crate::engine) fn class(&self) -> &'static str {
        CLASSES[self.index()]
    }
    fn rules(&self) -> &'static [(&'static str, Rule)] {
        match self {
            Self::StepGradient(_) => &STEP_PROPS,
            Self::Evolutionary(_) => &EVOLUTIONARY_PROPS,
            Self::MeanSquares => &[],
            Self::Mattes(_) => &MATTES_PROPS,
        }
    }
    /// Property values in the order of `rules` (flags as 0/1).
    fn values(&self) -> Vec<f64> {
        match self {
            Self::StepGradient(o) => vec![
                o.gradient_magnitude_tolerance,
                o.minimum_step_length,
                o.maximum_step_length,
                o.maximum_iterations,
                o.relaxation_factor,
            ],
            Self::Evolutionary(o) => vec![
                o.growth_factor,
                o.epsilon,
                o.initial_radius,
                o.maximum_iterations,
            ],
            Self::MeanSquares => Vec::new(),
            Self::Mattes(m) => vec![
                m.number_of_spatial_samples,
                m.number_of_histogram_bins,
                f64::from(u8::from(m.use_all_pixels)),
            ],
        }
    }
    /// The same class with property `k` (order of `rules`) set to `v`.
    fn with_value(&self, k: usize, v: f64) -> Self {
        let mut c = *self;
        match &mut c {
            Self::StepGradient(o) => match k {
                0 => o.gradient_magnitude_tolerance = v,
                1 => o.minimum_step_length = v,
                2 => o.maximum_step_length = v,
                3 => o.maximum_iterations = v,
                _ => o.relaxation_factor = v,
            },
            Self::Evolutionary(o) => match k {
                0 => o.growth_factor = v,
                1 => o.epsilon = v,
                2 => o.initial_radius = v,
                _ => o.maximum_iterations = v,
            },
            Self::MeanSquares => {}
            Self::Mattes(m) => match k {
                0 => m.number_of_spatial_samples = v,
                1 => m.number_of_histogram_bins = v,
                _ => m.use_all_pixels = v != 0.,
            },
        }
        c
    }
    pub(in crate::engine) fn property_names(&self) -> Vec<String> {
        self.rules().iter().map(|(n, _)| (*n).to_owned()).collect()
    }
    /// `object.Name`.
    pub(in crate::engine) fn property(&self, name: &str) -> Result<Value> {
        let Some(k) = self.rules().iter().position(|(n, _)| *n == name) else {
            return Err(format!(
                "Unrecognized method, property, or field '{name}' for class '{}'.",
                self.class()
            ));
        };
        let v = self.values()[k];
        Ok(match self.rules()[k].1 {
            Rule::Flag => Value::Logical(LogicalMatrix::scalar(v != 0.)),
            _ => Value::Matrix(Matrix::scalar(v)),
        })
    }
    /// `object.Name = value`: a changed copy, after the documented checks.
    pub(in crate::engine) fn assign(&self, name: &str, value: &Array) -> Result<Self> {
        let Some(k) = self.rules().iter().position(|(n, _)| *n == name) else {
            return Err(format!(
                "Unrecognized property '{name}' for class '{}'.",
                self.class()
            ));
        };
        let (name, rule) = self.rules()[k];
        let fail = |what: &str| -> String { format!("{}: {name} must be {what}.", self.class()) };
        let v = match rule {
            Rule::Positive => match real_scalar(value, false) {
                Some(v) if v.is_finite() && v > 0. => v,
                _ => return Err(fail("a positive finite numeric scalar")),
            },
            Rule::Integer(min) => match real_scalar(value, false) {
                Some(v) if v.is_finite() && v.fract() == 0. && v >= min => v,
                _ if min == 1. => return Err(fail("a positive integer scalar")),
                _ => return Err(fail(&format!("an integer scalar of at least {min}"))),
            },
            Rule::Ratio => match real_scalar(value, false) {
                Some(v) if v > 0. && v < 1. => v,
                _ => return Err(fail("a numeric scalar between 0 and 1 (exclusive)")),
            },
            Rule::Flag => match real_scalar(value, true) {
                Some(v) if v == 0. || v == 1. => v,
                _ => return Err(fail("a logical scalar (true or false, 1 or 0)")),
            },
        };
        Ok(self.with_value(k, v))
    }
    /// Display body, laid out as on the reference pages.
    pub(in crate::engine) fn body(&self) -> String {
        let mut s = format!("  {}\n\n", self.class());
        let rules = self.rules();
        if rules.is_empty() {
            s.push_str("  This class has no properties.\n");
            return s;
        }
        s.push_str("  Properties:\n");
        let width = rules.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        for ((name, rule), v) in rules.iter().zip(self.values()) {
            let shown = match rule {
                Rule::Positive | Rule::Ratio => scientific(v),
                Rule::Integer(_) => format!("{v:.0}"),
                Rule::Flag => format!("{}", u8::from(v != 0.)),
            };
            s.push_str(&format!("    {name:>width$}: {shown}\n"));
        }
        s
    }
}

/// `registration.optimizer.<Class>` / `registration.metric.<Class>`: the
/// class index and the call arguments, if `expr` names one of the classes.
pub(in crate::engine) fn constructor_expr(
    expr: &crate::syntax::Expr,
) -> Option<(usize, Option<&[crate::syntax::Expr]>)> {
    use crate::syntax::Expr;
    let (path, args) = match expr {
        Expr::Index(base, args) => (base.as_ref(), Some(args.as_slice())),
        Expr::Field(..) => (expr, None),
        _ => return None,
    };
    let Expr::Field(inner, class) = path else {
        return None;
    };
    let Expr::Field(root, package) = inner.as_ref() else {
        return None;
    };
    if !matches!(root.as_ref(), Expr::Name(n) if n == "registration") {
        return None;
    }
    let full = format!("registration.{package}.{class}");
    let index = CLASSES.iter().position(|c| *c == full)?;
    Some((index, args))
}

/// A new object of class `CLASSES[index]` from name-value arguments (names
/// matched without regard to case).
pub(in crate::engine) fn construct(e: &mut Engine, index: usize, args: &[Value]) -> Result<Value> {
    let mut c = RegConfig::default_of(index);
    let class = c.class();
    if !args.len().is_multiple_of(2) {
        return Err(format!("{class}: name-value arguments must come in pairs."));
    }
    for pair in args.chunks(2) {
        let Some(name) = text(&pair[0]) else {
            return Err(format!("{class}: property names must be text."));
        };
        let Some(exact) = c
            .rules()
            .iter()
            .map(|(n, _)| *n)
            .find(|n| n.eq_ignore_ascii_case(&name))
        else {
            return Err(format!(
                "Unrecognized property '{name}' for class '{class}'."
            ));
        };
        let value = pair[1].clone().array(&e.cancel)?;
        c = c.assign(exact, &value)?;
    }
    super::super::objects::reg_config_new(e, c)
}

/// `[optimizer, metric] = imregconfig(modality)`.
pub(in crate::engine) fn imregconfig(
    e: &mut Engine,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    if count > 2 {
        return Err(
            "imregconfig: too many output arguments (at most optimizer and metric).".into(),
        );
    }
    let [modality] = args else {
        return Err(format!(
            "imregconfig: expected one input, the modality 'monomodal' or 'multimodal' (got {}).",
            args.len()
        ));
    };
    let Some(text) = text(modality) else {
        return Err("imregconfig: modality must be 'monomodal' or 'multimodal'.".into());
    };
    let lower = text.to_ascii_lowercase();
    // validatestring-style: a case-insensitive unique prefix is enough.
    let matches: Vec<&str> = ["monomodal", "multimodal"]
        .into_iter()
        .filter(|m| !lower.is_empty() && m.starts_with(&lower))
        .collect();
    let [chosen] = matches.as_slice() else {
        return Err(format!(
            "imregconfig: modality must be 'monomodal' or 'multimodal', not '{text}'."
        ));
    };
    let (optimizer, metric) = if *chosen == "monomodal" {
        (0, 2)
    } else {
        (1, 3)
    };
    let mut outputs = vec![super::super::objects::reg_config_new(
        e,
        RegConfig::default_of(optimizer),
    )?];
    if count >= 2 {
        outputs.push(super::super::objects::reg_config_new(
            e,
            RegConfig::default_of(metric),
        )?);
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_matches_c_percent_e() {
        assert_eq!(scientific(1e-4), "1.000000e-04");
        assert_eq!(scientific(0.0625), "6.250000e-02");
        assert_eq!(scientific(1.05), "1.050000e+00");
        assert_eq!(scientific(1.5e-6), "1.500000e-06");
        assert_eq!(scientific(2.5e123), "2.500000e+123");
    }

    #[test]
    fn accessors_read_the_settings_of_objects() {
        let mut e = Engine::default();
        e.execute(
            "[o, m] = imregconfig('multimodal'); o.MaximumIterations = 300; m.UseAllPixels = false; [g, s] = imregconfig('monomodal'); x = 1;",
        )
        .unwrap();
        let var = |e: &Engine, name: &str| Value::Container(e.array(name).unwrap());
        let Some(Optimizer::OnePlusOneEvolutionary(o)) = optimizer(&e, &var(&e, "o")) else {
            panic!("o is not a OnePlusOneEvolutionary optimizer");
        };
        assert_eq!(o.maximum_iterations, 300.);
        assert_eq!(o.growth_factor, 1.05);
        let Some(Metric::MattesMutualInformation(m)) = metric(&e, &var(&e, "m")) else {
            panic!("m is not a MattesMutualInformation metric");
        };
        assert!(!m.use_all_pixels);
        assert_eq!(m.number_of_histogram_bins, 50.);
        assert_eq!(
            optimizer(&e, &var(&e, "g")),
            Some(Optimizer::RegularStepGradientDescent(
                RegularStepGradientDescent::default()
            ))
        );
        assert_eq!(metric(&e, &var(&e, "s")), Some(Metric::MeanSquares));
        // The wrong kind of object, or no object, is None.
        assert_eq!(metric(&e, &var(&e, "o")), None);
        assert_eq!(optimizer(&e, &var(&e, "m")), None);
        assert_eq!(optimizer(&e, &Value::Matrix(Matrix::scalar(1.))), None);
    }

    #[test]
    fn display_bodies_follow_the_reference_layout() {
        assert_eq!(
            RegConfig::default_of(0).body(),
            "  registration.optimizer.RegularStepGradientDescent\n\n  Properties:\n    GradientMagnitudeTolerance: 1.000000e-04\n             MinimumStepLength: 1.000000e-05\n             MaximumStepLength: 6.250000e-02\n             MaximumIterations: 100\n              RelaxationFactor: 5.000000e-01\n"
        );
        assert_eq!(
            RegConfig::default_of(2).body(),
            "  registration.metric.MeanSquares\n\n  This class has no properties.\n"
        );
    }
}
