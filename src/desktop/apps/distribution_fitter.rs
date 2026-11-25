//! Distribution Fitter: fits probability distributions to workspace data
//! with `fitdist`. INPUTS lists data sets (a data vector with optional
//! censoring and frequency vectors), exclusion rules and fits. One script
//! fits every fit marked for export and draws the chosen display; the
//! private probe fits the other fits and reads back what the view and the
//! Output column show. Every number shown comes from engine builtins
//! (`fitdist`, `negloglik`, `paramci`, `pdf`, `cdf`, `icdf`, `mean`, `var`,
//! `histcounts`, `ecdf`, `probplot`); confidence bounds of a fitted cdf are
//! the delta method on the logit scale with the fit's ParameterCovariance.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Choices

/// The distributions `fitdist` fits, in dropdown order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Normal,
    Lognormal,
    Exponential,
    Gamma,
    Weibull,
    Rayleigh,
    ExtremeValue,
    Logistic,
    Loglogistic,
    HalfNormal,
    InverseGaussian,
    BirnbaumSaunders,
    Nakagami,
    Rician,
    Beta,
    GeneralizedExtremeValue,
    GeneralizedPareto,
    TLocationScale,
    Poisson,
    Binomial,
    NegativeBinomial,
    Kernel,
}
impl Family {
    pub const ALL: [Self; 22] = [
        Self::Normal,
        Self::Lognormal,
        Self::Exponential,
        Self::Gamma,
        Self::Weibull,
        Self::Rayleigh,
        Self::ExtremeValue,
        Self::Logistic,
        Self::Loglogistic,
        Self::HalfNormal,
        Self::InverseGaussian,
        Self::BirnbaumSaunders,
        Self::Nakagami,
        Self::Rician,
        Self::Beta,
        Self::GeneralizedExtremeValue,
        Self::GeneralizedPareto,
        Self::TLocationScale,
        Self::Poisson,
        Self::Binomial,
        Self::NegativeBinomial,
        Self::Kernel,
    ];
    /// fitdist's distribution name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Lognormal => "Lognormal",
            Self::Exponential => "Exponential",
            Self::Gamma => "Gamma",
            Self::Weibull => "Weibull",
            Self::Rayleigh => "Rayleigh",
            Self::ExtremeValue => "ExtremeValue",
            Self::Logistic => "Logistic",
            Self::Loglogistic => "Loglogistic",
            Self::HalfNormal => "HalfNormal",
            Self::InverseGaussian => "InverseGaussian",
            Self::BirnbaumSaunders => "BirnbaumSaunders",
            Self::Nakagami => "Nakagami",
            Self::Rician => "Rician",
            Self::Beta => "Beta",
            Self::GeneralizedExtremeValue => "GeneralizedExtremeValue",
            Self::GeneralizedPareto => "GeneralizedPareto",
            Self::TLocationScale => "tLocationScale",
            Self::Poisson => "Poisson",
            Self::Binomial => "Binomial",
            Self::NegativeBinomial => "NegativeBinomial",
            Self::Kernel => "Kernel",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Lognormal => "Lognormal",
            Self::Exponential => "Exponential",
            Self::Gamma => "Gamma",
            Self::Weibull => "Weibull",
            Self::Rayleigh => "Rayleigh",
            Self::ExtremeValue => "Extreme value",
            Self::Logistic => "Logistic",
            Self::Loglogistic => "Log-logistic",
            Self::HalfNormal => "Half-normal",
            Self::InverseGaussian => "Inverse Gaussian",
            Self::BirnbaumSaunders => "Birnbaum-Saunders",
            Self::Nakagami => "Nakagami",
            Self::Rician => "Rician",
            Self::Beta => "Beta",
            Self::GeneralizedExtremeValue => "Generalized extreme value",
            Self::GeneralizedPareto => "Generalized Pareto",
            Self::TLocationScale => "t location-scale",
            Self::Poisson => "Poisson",
            Self::Binomial => "Binomial",
            Self::NegativeBinomial => "Negative binomial",
            Self::Kernel => "Kernel density",
        }
    }
    /// Integer-valued: drawn as steps on the integers.
    pub fn discrete(self) -> bool {
        matches!(
            self,
            Self::Poisson | Self::Binomial | Self::NegativeBinomial
        )
    }
    /// Values above zero only: quantile bounds on the log scale.
    fn positive(self) -> bool {
        matches!(
            self,
            Self::Lognormal
                | Self::Exponential
                | Self::Gamma
                | Self::Weibull
                | Self::Rayleigh
                | Self::Loglogistic
                | Self::InverseGaussian
                | Self::BirnbaumSaunders
                | Self::Nakagami
                | Self::Rician
                | Self::Beta
        )
    }
    /// The parameter fitdist holds at a value the user gives.
    pub fn fixed(self) -> Option<Fixed> {
        let (option, label, noun, required) = match self {
            Self::Binomial => (
                "NTrials",
                "Number of trials N (fixed)",
                "number of trials N",
                true,
            ),
            Self::HalfNormal => ("mu", "Location mu (fixed, default 0)", "location mu", false),
            Self::GeneralizedPareto => (
                "Theta",
                "Threshold theta (fixed, default 0)",
                "threshold theta",
                false,
            ),
            _ => return None,
        };
        Some(Fixed {
            option,
            label,
            noun,
            required,
        })
    }
    /// Heading of the dropdown group this family starts.
    fn group(self) -> Option<&'static str> {
        match self {
            Self::Normal => Some("Continuous"),
            Self::Poisson => Some("Discrete"),
            Self::Kernel => Some("Nonparametric"),
            _ => None,
        }
    }
}

/// A parameter held at a typed value: fitdist's option name, the INPUTS
/// label, the noun of messages, and whether a value is required.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixed {
    pub option: &'static str,
    pub label: &'static str,
    pub noun: &'static str,
    pub required: bool,
}

/// Kernel smoothing functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Smoother {
    Normal,
    Box,
    Triangle,
    Epanechnikov,
}
impl Smoother {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Box, Self::Triangle, Self::Epanechnikov];
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Box => "box",
            Self::Triangle => "triangle",
            Self::Epanechnikov => "epanechnikov",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Box => "Box",
            Self::Triangle => "Triangle",
            Self::Epanechnikov => "Epanechnikov",
        }
    }
}

/// Where a kernel estimate puts its mass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Support {
    Unbounded,
    Positive,
    Bounded,
}
impl Support {
    pub const ALL: [Self; 3] = [Self::Unbounded, Self::Positive, Self::Bounded];
    pub fn label(self) -> &'static str {
        match self {
            Self::Unbounded => "Unbounded",
            Self::Positive => "Positive",
            Self::Bounded => "Bounded",
        }
    }
}

/// What the main view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Display {
    Density,
    Cdf,
    Quantile,
    Probability,
    Survivor,
    CumulativeHazard,
}
impl Display {
    pub const ALL: [Self; 6] = [
        Self::Density,
        Self::Cdf,
        Self::Quantile,
        Self::Probability,
        Self::Survivor,
        Self::CumulativeHazard,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Density => "Density",
            Self::Cdf => "Cumulative probability",
            Self::Quantile => "Quantile",
            Self::Probability => "Probability plot",
            Self::Survivor => "Survivor",
            Self::CumulativeHazard => "Cumulative hazard",
        }
    }
    /// Data and fitted curves can carry confidence bounds.
    pub fn has_bounds(self) -> bool {
        matches!(self, Self::Cdf | Self::Survivor | Self::CumulativeHazard)
    }
    /// ecdf's 'Function' value, when it is not the cdf.
    fn ecdf_function(self) -> Option<&'static str> {
        match self {
            Self::Survivor => Some("survivor"),
            Self::CumulativeHazard => Some("cumulative hazard"),
            _ => None,
        }
    }
    fn axis_labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Density => ("Data", "Density"),
            Self::Cdf => ("Data", "Cumulative probability"),
            Self::Quantile => ("Probability", "Data"),
            Self::Probability => ("Data", "Probability"),
            Self::Survivor => ("Data", "Survival probability"),
            Self::CumulativeHazard => ("Data", "Cumulative hazard"),
        }
    }
}

/// Histogram bin rule of the Density display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bins {
    Auto,
    Scott,
    FreedmanDiaconis,
    Integers,
    Width,
    Count,
}
impl Bins {
    pub const ALL: [Self; 6] = [
        Self::Auto,
        Self::Scott,
        Self::FreedmanDiaconis,
        Self::Integers,
        Self::Width,
        Self::Count,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Scott => "Scott",
            Self::FreedmanDiaconis => "Freedman-Diaconis",
            Self::Integers => "Integers",
            Self::Width => "Bin width",
            Self::Count => "Number of bins",
        }
    }
}

/// Probability paper of the Probability plot display (probplot's names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scale {
    Normal,
    Lognormal,
    Exponential,
    Weibull,
    ExtremeValue,
    Rayleigh,
    HalfNormal,
    Logistic,
    Loglogistic,
}
impl Scale {
    pub const ALL: [Self; 9] = [
        Self::Normal,
        Self::Lognormal,
        Self::Exponential,
        Self::Weibull,
        Self::ExtremeValue,
        Self::Rayleigh,
        Self::HalfNormal,
        Self::Logistic,
        Self::Loglogistic,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Lognormal => "lognormal",
            Self::Exponential => "exponential",
            Self::Weibull => "weibull",
            Self::ExtremeValue => "extreme value",
            Self::Rayleigh => "rayleigh",
            Self::HalfNormal => "half normal",
            Self::Logistic => "logistic",
            Self::Loglogistic => "loglogistic",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Lognormal => "Lognormal",
            Self::Exponential => "Exponential",
            Self::Weibull => "Weibull",
            Self::ExtremeValue => "Extreme value",
            Self::Rayleigh => "Rayleigh",
            Self::HalfNormal => "Half-normal",
            Self::Logistic => "Logistic",
            Self::Loglogistic => "Log-logistic",
        }
    }
    /// The data axis is logarithmic.
    pub fn log_x(self) -> bool {
        matches!(self, Self::Lognormal | Self::Weibull | Self::Loglogistic)
    }
    /// The vertical position of probability `p` (a MATLAB row) on this paper.
    fn position(self, p: &str) -> String {
        match self {
            Self::Normal | Self::Lognormal => format!("norminv({p})"),
            Self::Exponential => format!("-log(1 - {p})"),
            Self::Weibull | Self::ExtremeValue => format!("log(-log(1 - {p}))"),
            Self::Rayleigh => format!("sqrt(-2*log(1 - {p}))"),
            Self::HalfNormal => format!("norminv(0.5 + {p}/2)"),
            Self::Logistic | Self::Loglogistic => format!("log({p} ./ (1 - {p}))"),
        }
    }
}
/// Probabilities marked on the probability axis.
pub const LEVELS: [f64; 15] = [
    0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, 0.98, 0.99, 0.995, 0.999,
];

/// What the Evaluate panel computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    Pdf,
    Cdf,
    Icdf,
    Survivor,
}
impl Function {
    pub const ALL: [Self; 4] = [Self::Pdf, Self::Cdf, Self::Icdf, Self::Survivor];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "Density (pdf)",
            Self::Cdf => "Cumulative probability (cdf)",
            Self::Icdf => "Quantile (inverse cdf)",
            Self::Survivor => "Survivor",
        }
    }
    pub fn has_bounds(self) -> bool {
        self != Self::Pdf
    }
}

/// Order of the fits table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sort {
    Created,
    NLogL,
    Aic,
    Bic,
}
impl Sort {
    pub const ALL: [Self; 4] = [Self::Created, Self::NLogL, Self::Aic, Self::Bic];
    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::NLogL => "NLogL",
            Self::Aic => "AIC",
            Self::Bic => "BIC",
        }
    }
}

/// The Output column's views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    Fits,
    Details,
    Evaluate,
}
impl Pane {
    pub const ALL: [Self; 3] = [Self::Fits, Self::Details, Self::Evaluate];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fits => "Compare",
            Self::Details => "Selected fit",
            Self::Evaluate => "Evaluate",
        }
    }
}

/// Most data sets, rules and fits a session keeps.
pub const MAX_DATA_SETS: usize = 16;
pub const MAX_RULES: usize = 16;
pub const MAX_FITS: usize = 32;
/// Most points the Evaluate panel takes.
pub const MAX_EVALUATE: usize = 50;
/// Points of each fitted curve.
const CURVE_POINTS: usize = 200;

// ---------------------------------------------------------------------------
// Inputs

/// A data vector with its optional censoring and frequency vectors.
#[derive(Clone, Debug, PartialEq)]
pub struct DataSet {
    pub id: u32,
    pub name: String,
    pub name_edited: bool,
    pub data: String,
    /// Empty: no censoring.
    pub censoring: String,
    /// Empty: every value counts once.
    pub frequency: String,
}
impl DataSet {
    fn new(id: u32) -> Self {
        Self {
            id,
            name: String::new(),
            name_edited: false,
            data: String::new(),
            censoring: String::new(),
            frequency: String::new(),
        }
    }
    /// Workspace variables the data set reads.
    pub fn variables(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in [&self.data, &self.censoring, &self.frequency] {
            if !name.is_empty() && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }
    /// Shown in lists, the legend and the fits table.
    pub fn title(&self) -> String {
        let name = one_line(&self.name);
        if !name.is_empty() {
            name
        } else if !self.data.is_empty() {
            self.data.clone()
        } else {
            format!("Data set {}", self.id)
        }
    }
    fn check(&self, ws: &Snapshot) -> Result<(), String> {
        if self.data.is_empty() {
            return Err(format!(
                "Distribution Fitter: choose the data of {} in INPUTS",
                self.title()
            ));
        }
        for name in self.variables() {
            if !ws.contains(&name) {
                return Err(format!(
                    "Distribution Fitter: workspace variable {name} does not exist"
                ));
            }
        }
        Ok(())
    }
}

/// Values a fit keeps: lower ≤ (or <) x ≤ (or <) upper.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: u32,
    pub name: String,
    /// Blank: no limit.
    pub lower: String,
    /// x > lower rather than x >= lower.
    pub lower_strict: bool,
    pub upper: String,
    /// x < upper rather than x <= upper.
    pub upper_strict: bool,
}
impl Rule {
    fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            lower: String::new(),
            lower_strict: false,
            upper: String::new(),
            upper_strict: false,
        }
    }
    pub fn title(&self) -> String {
        let name = one_line(&self.name);
        if name.is_empty() {
            format!("Rule {}", self.id)
        } else {
            name
        }
    }
    /// The limits, checked.
    pub fn limits(&self) -> Result<(Option<f64>, Option<f64>), String> {
        let what = self.title();
        let lower = parse_number(&self.lower, &format!("{what}: the lower limit"))?;
        let upper = parse_number(&self.upper, &format!("{what}: the upper limit"))?;
        match (lower, upper) {
            (None, None) => Err(format!(
                "Distribution Fitter: {what}: give a lower or an upper limit"
            )),
            (Some(a), Some(b)) if a > b || (a == b && (self.lower_strict || self.upper_strict)) => {
                Err(format!(
                    "Distribution Fitter: {what}: no value is inside the limits"
                ))
            }
            limits => Ok(limits),
        }
    }
    /// The logical index of the kept values of `x`.
    pub fn mask(&self, x: &str) -> Result<String, String> {
        let (lower, upper) = self.limits()?;
        let mut parts = Vec::new();
        if let Some(a) = lower {
            let op = if self.lower_strict { ">" } else { ">=" };
            parts.push(format!("{x} {op} {}", literal(a)));
        }
        if let Some(b) = upper {
            let op = if self.upper_strict { "<" } else { "<=" };
            parts.push(format!("{x} {op} {}", literal(b)));
        }
        Ok(parts.join(" & "))
    }
    /// "1 ≤ x < 5" for lists.
    pub fn describe(&self) -> String {
        let lower = self.lower.trim();
        let upper = self.upper.trim();
        let mut text = String::new();
        if !lower.is_empty() {
            text.push_str(&format!(
                "{lower} {} ",
                if self.lower_strict { "<" } else { "≤" }
            ));
        }
        text.push('x');
        if !upper.is_empty() {
            text.push_str(&format!(
                " {} {upper}",
                if self.upper_strict { "<" } else { "≤" }
            ));
        }
        text
    }
}

/// One fit: a distribution, a data set and an optional exclusion rule.
#[derive(Clone, Debug)]
pub struct Fit {
    pub id: u32,
    pub name: String,
    pub data_set: u32,
    pub rule: Option<u32>,
    pub family: Family,
    /// NTrials, mu or theta as typed; blank is the default.
    pub fixed: String,
    pub smoother: Smoother,
    /// Blank: fitdist's bandwidth.
    pub bandwidth: String,
    pub support: Support,
    pub support_lower: String,
    pub support_upper: String,
    /// Export and Generate script include this fit.
    pub export: bool,
    /// The exported object's name.
    pub object: String,
    pub object_edited: bool,
    /// Last good result; errors keep it on screen.
    pub result: Option<FitResult>,
}
impl Fit {
    fn new(id: u32, name: String, data_set: u32) -> Self {
        Self {
            id,
            name,
            data_set,
            rule: None,
            family: Family::Normal,
            fixed: String::new(),
            smoother: Smoother::Normal,
            bandwidth: String::new(),
            support: Support::Unbounded,
            support_lower: String::new(),
            support_upper: String::new(),
            export: true,
            object: String::new(),
            object_edited: false,
            result: None,
        }
    }
    pub fn title(&self) -> String {
        let name = one_line(&self.name);
        if name.is_empty() {
            format!("Fit {}", self.id)
        } else {
            name
        }
    }
    /// fitdist's options after the data's, only the non-default ones.
    pub fn options(&self) -> Result<String, String> {
        let what = self.title();
        let mut out = String::new();
        if let Some(fixed) = self.family.fixed() {
            let noun = fixed.noun;
            match parse_number(&self.fixed, &format!("{what}: the {noun}"))? {
                None if fixed.required => {
                    return Err(format!("Distribution Fitter: {what}: give the {noun}"));
                }
                Some(v) if !v.is_finite() => {
                    return Err(format!(
                        "Distribution Fitter: {what}: the {noun} must be finite"
                    ));
                }
                Some(v) if fixed.required || v != 0. => {
                    out.push_str(&format!(", '{}', {}", fixed.option, number(v)));
                }
                _ => {}
            }
        }
        if self.family == Family::Kernel {
            if self.smoother != Smoother::Normal {
                out.push_str(&format!(", 'Kernel', '{}'", self.smoother.name()));
            }
            if let Some(h) = parse_number(&self.bandwidth, &format!("{what}: the bandwidth"))? {
                if !(h.is_finite() && h > 0.) {
                    return Err(format!(
                        "Distribution Fitter: {what}: the bandwidth must be a positive number"
                    ));
                }
                out.push_str(&format!(", 'Bandwidth', {}", number(h)));
            }
            match self.support {
                Support::Unbounded => {}
                Support::Positive => out.push_str(", 'Support', 'positive'"),
                Support::Bounded => {
                    let a =
                        parse_number(&self.support_lower, &format!("{what}: the lower support"))?;
                    let b =
                        parse_number(&self.support_upper, &format!("{what}: the upper support"))?;
                    match (a, b) {
                        (Some(a), Some(b)) if a.is_finite() && b.is_finite() && a < b => {
                            out.push_str(&format!(", 'Support', [{} {}]", number(a), number(b)));
                        }
                        _ => {
                            return Err(format!(
                                "Distribution Fitter: {what}: the support needs two finite \
                                 limits, lower below upper"
                            ));
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Results

/// One fit as the private run computed it.
#[derive(Clone, Debug, PartialEq)]
pub struct FitResult {
    /// Parameter names, values, whether held fixed, and 95% intervals.
    pub names: Vec<String>,
    pub values: Vec<f64>,
    pub fixed: Vec<bool>,
    pub intervals: Vec<[f64; 2]>,
    pub nlogl: f64,
    /// Estimated parameters (NaN for a kernel estimate).
    pub parameters: f64,
    /// Observations, frequencies counted.
    pub observations: f64,
    pub aic: f64,
    pub bic: f64,
    pub mean: f64,
    pub variance: f64,
    /// ParameterCovariance, column-major.
    pub covariance: Vec<f64>,
    /// A kernel estimate's bandwidth.
    pub bandwidth: Option<f64>,
    /// The fitted curve of the display, finite points.
    pub curve: Vec<[f64; 2]>,
    /// Confidence bounds of the curve; empty when off.
    pub lower: Vec<[f64; 2]>,
    pub upper: Vec<[f64; 2]>,
    /// Evaluate panel: point, value, lower, upper (NaN when no bound).
    pub evaluated: Vec<[f64; 4]>,
}

/// One data set as the private run computed it for the display.
#[derive(Clone, Debug, PartialEq)]
pub struct DataCurve {
    pub id: u32,
    /// Density: histogram bins as (left edge, right edge, height).
    pub bins: Vec<[f64; 3]>,
    /// Steps of the empirical function, or probability plot points.
    pub points: Vec<[f64; 2]>,
    pub lower: Vec<[f64; 2]>,
    pub upper: Vec<[f64; 2]>,
}

/// What the view draws from the last good run.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub display: Display,
    pub scale: Scale,
    pub data: Vec<DataCurve>,
    /// Probability axis marks: position and label.
    pub ticks: Vec<(f64, String)>,
}

fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Distribution Fitter: {name} is not a real array")),
    }
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("Distribution Fitter: {name} is empty"))
}
fn text(outputs: &Outputs, name: &str) -> Result<String, String> {
    match outputs.get(name) {
        Some(Array::Character(c)) => Ok(String::from_utf16_lossy(c.units())),
        Some(Array::Numeric(m)) if m.data().is_empty() => Ok(String::new()),
        _ => Err(format!("Distribution Fitter: {name} is not text")),
    }
}
/// Finite (x, y) pairs.
fn pairs(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    x.iter()
        .zip(y)
        .map(|(x, y)| [*x, *y])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}
/// A step function through (x, y): level y[i] from x[i] to x[i + 1].
fn steps(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(2 * x.len());
    let n = x.len().min(y.len());
    for i in 0..n {
        out.push([x[i], y[i]]);
        if i + 1 < n {
            out.push([x[i + 1], y[i]]);
        }
    }
    out.retain(|p| p[0].is_finite() && p[1].is_finite());
    out
}

impl DataCurve {
    fn from_outputs(outputs: &Outputs, id: u32, display: Display) -> Result<Self, String> {
        let get = |part: &str| values(outputs, &format!("app_d{id}_{part}"));
        let (x, y, lo, up) = (get("x")?, get("y")?, get("lo")?, get("up")?);
        let mut curve = Self {
            id,
            bins: Vec::new(),
            points: Vec::new(),
            lower: Vec::new(),
            upper: Vec::new(),
        };
        match display {
            Display::Density => {
                // x holds the bin edges, y the heights.
                curve.bins = y
                    .iter()
                    .enumerate()
                    .filter_map(|(i, h)| Some([*x.get(i)?, *x.get(i + 1)?, *h]))
                    .filter(|b| b.iter().all(|v| v.is_finite()))
                    .collect();
            }
            Display::Probability => curve.points = pairs(&x, &y),
            Display::Quantile => {
                // ecdf gives (x, F) with x(1) repeated; the quantile at
                // probabilities F(i-1)..F(i) is x(i).
                let shifted: Vec<f64> = x.iter().skip(1).chain(x.last()).copied().collect();
                curve.points = steps(&y, &shifted);
            }
            _ => {
                curve.points = steps(&x, &y);
                if lo.len() == x.len() && up.len() == x.len() {
                    curve.lower = steps(&x, &lo);
                    curve.upper = steps(&x, &up);
                }
            }
        }
        Ok(curve)
    }
}

impl FitResult {
    fn from_outputs(outputs: &Outputs, fit: &Fit, display: Display) -> Result<Self, String> {
        let id = fit.id;
        let get = |part: &str| values(outputs, &format!("app_f{id}_{part}"));
        let one = |part: &str| scalar(outputs, &format!("app_f{id}_{part}"));
        let names: Vec<String> = text(outputs, &format!("app_f{id}_nm"))?
            .split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect();
        let values = get("v")?;
        let fixed: Vec<bool> = get("fx")?.iter().map(|v| *v != 0.).collect();
        let ci = get("ci")?;
        let intervals = (0..values.len())
            .map(|k| {
                [
                    ci.get(2 * k).copied().unwrap_or(f64::NAN),
                    ci.get(2 * k + 1).copied().unwrap_or(f64::NAN),
                ]
            })
            .collect();
        let bandwidth = get("bw")?.first().copied();
        let (x, y, lo, up) = (get("x")?, get("y")?, get("lo")?, get("up")?);
        let stepped =
            fit.family.discrete() && !matches!(display, Display::Density | Display::Probability);
        let line = |y: &[f64]| {
            if stepped { steps(&x, y) } else { pairs(&x, y) }
        };
        let (curve, lower, upper) = if lo.len() == x.len() && up.len() == x.len() && !x.is_empty() {
            (line(&y), line(&lo), line(&up))
        } else {
            (line(&y), Vec::new(), Vec::new())
        };
        let (at, ev, el, eu) = (get("ex")?, get("ev")?, get("el")?, get("eu")?);
        let evaluated = at
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let pick = |v: &[f64]| v.get(i).copied().unwrap_or(f64::NAN);
                [*a, pick(&ev), pick(&el), pick(&eu)]
            })
            .collect();
        Ok(Self {
            names,
            values,
            fixed,
            intervals,
            nlogl: one("nll")?,
            parameters: one("k")?,
            observations: one("n")?,
            aic: one("aic")?,
            bic: one("bic")?,
            mean: one("mean")?,
            variance: one("var")?,
            covariance: get("cov")?,
            bandwidth,
            curve,
            lower,
            upper,
            evaluated,
        })
    }
    /// The value a sort compares.
    pub fn criterion(&self, sort: Sort) -> f64 {
        match sort {
            Sort::Created => 0.,
            Sort::NLogL => self.nlogl,
            Sort::Aic => self.aic,
            Sort::Bic => self.bic,
        }
    }
    /// "a = 1.2, b = 3.4" (a kernel shows its bandwidth).
    pub fn parameter_text(&self) -> String {
        if let Some(h) = self.bandwidth {
            return format!("bandwidth = {}", short(h));
        }
        self.names
            .iter()
            .zip(&self.values)
            .map(|(n, v)| format!("{n} = {}", short(*v)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ---------------------------------------------------------------------------
// Script text

/// A MATLAB char vector literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
/// A number the engine reads back exactly, Inf included.
fn literal(v: f64) -> String {
    if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else {
        number(v)
    }
}
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ").trim().to_owned()
}
/// A typed number: blank is None; Inf and -Inf are allowed, NaN is not.
fn parse_number(text: &str, what: &str) -> Result<Option<f64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(v) if !v.is_nan() => Ok(Some(v)),
        _ => Err(format!("Distribution Fitter: {what} must be a number")),
    }
}
/// Evaluation points typed as `1 2.5 4` or `[1, 2.5; 4]`.
pub fn parse_points(text: &str) -> Result<Vec<f64>, String> {
    let inner = text.trim();
    let inner = inner
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(inner);
    let points = inner
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().ok().filter(|v| !v.is_nan()))
        .collect::<Option<Vec<f64>>>()
        .ok_or("Evaluate: type numbers, for example 1 2.5 4")?;
    if points.len() > MAX_EVALUATE {
        return Err(format!("Evaluate: at most {MAX_EVALUATE} points"));
    }
    Ok(points)
}
fn row(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(|v| literal(*v)).collect();
    format!("[{}]", items.join(" "))
}

/// A data set's columns as a script names them, after an exclusion.
struct Columns {
    data: String,
    cens: Option<String>,
    freq: Option<String>,
}
impl Columns {
    fn new(ds: &DataSet, keep: Option<&str>) -> Self {
        let pick = |v: &str| match keep {
            Some(k) => format!("{v}({k})"),
            None => v.to_owned(),
        };
        let optional = |v: &str| (!v.is_empty()).then(|| pick(v));
        Self {
            data: pick(&ds.data),
            cens: optional(&ds.censoring),
            freq: optional(&ds.frequency),
        }
    }
    /// fitdist's and ecdf's `'Censoring'` and `'Frequency'` options.
    fn options(&self) -> String {
        let mut out = String::new();
        if let Some(c) = &self.cens {
            out.push_str(&format!(", 'Censoring', {c}"));
        }
        if let Some(f) = &self.freq {
            out.push_str(&format!(", 'Frequency', {f}"));
        }
        out
    }
    /// probplot's positional censoring and frequency arguments.
    fn positional(&self) -> String {
        match (&self.cens, &self.freq) {
            (None, None) => String::new(),
            (Some(c), None) => format!(", {c}"),
            (None, Some(f)) => format!(", false(size({})), {f}", self.data),
            (Some(c), Some(f)) => format!(", {c}, {f}"),
        }
    }
    /// The histogram's data: values repeated by their frequencies.
    fn histogram(&self) -> String {
        match &self.freq {
            Some(f) => format!("repelem({}, {f})", self.data),
            None => self.data.clone(),
        }
    }
    /// Lines setting `out` to the observations, frequencies counted.
    fn count(&self, out: &str) -> String {
        match &self.freq {
            None => format!("app_t = {}; {out} = sum(~isnan(app_t));\n", self.data),
            Some(f) => format!(
                "app_t = {}; app_u = {f}; {out} = sum(app_u(~isnan(app_t)));\n",
                self.data
            ),
        }
    }
}

/// Names the script, its probe and Export share.
struct Plan {
    /// Exported fits' object names, by fit id.
    objects: BTreeMap<u32, String>,
    /// Logical indexes by (data set id, rule id).
    keeps: BTreeMap<(u32, u32), String>,
    /// Helper variables of the plot block.
    helpers: Vec<String>,
    /// Probability display: each data set's probplot handle.
    handles: BTreeMap<u32, String>,
    script: String,
}

/// Lines computing the gradient of `function(p, x)` with respect to the
/// free parameters into `app_G` (forward differences, backward where the
/// forward step leaves the parameter space); `value` holds the row of
/// function values.
fn gradient(p: &str, x: &str, function: &str, value: &str) -> String {
    format!(
        "app_v = {p}.ParameterValues; app_C = {p}.ParameterCovariance;\n\
         app_G = zeros(numel(app_v), numel({value}));\n\
         for app_k = find(~{p}.ParameterIsFixed)\n\
         app_h = 1e-6*max(abs(app_v(app_k)), 1);\n\
         app_w = app_v; app_w(app_k) = app_v(app_k) + app_h; app_c = num2cell(app_w);\n\
         app_d = (reshape({function}({p}.DistributionName, {x}, app_c{{:}}), 1, []) - {value}) / app_h;\n\
         if ~all(isfinite(app_d))\n\
         app_w(app_k) = app_v(app_k) - app_h; app_c = num2cell(app_w);\n\
         app_d = ({value} - reshape({function}({p}.DistributionName, {x}, app_c{{:}}), 1, [])) / app_h;\n\
         end\n\
         app_G(app_k, :) = app_d;\n\
         end\n"
    )
}
/// Lines setting the rows `f`, `lo` and `up` to `cdf(p, x)` and its
/// delta-method confidence bounds on the logit scale.
fn cdf_bounds(p: &str, x: &str, (f, lo, up): (&str, &str, &str), alpha: f64) -> String {
    format!(
        "{f} = reshape(cdf({p}, {x}), 1, []);\n{}\
         app_in = {f} > 0 & {f} < 1;\n\
         app_D = app_G ./ repmat({f} .* (1 - {f}), numel(app_v), 1);\n\
         app_s = sqrt(max(sum(app_D .* (app_C * app_D), 1), 0));\n\
         app_z = norminv(1 - {alpha}/2); app_g = log({f} ./ (1 - {f}));\n\
         {lo} = {f}; {up} = {f};\n\
         {lo}(app_in) = 1 ./ (1 + exp(-(app_g(app_in) - app_z*app_s(app_in))));\n\
         {up}(app_in) = 1 ./ (1 + exp(-(app_g(app_in) + app_z*app_s(app_in))));\n",
        gradient(p, x, "cdf", f),
        alpha = number(alpha),
    )
}
/// Lines setting the rows `q`, `lo` and `up` to `icdf(p, x)` and its
/// delta-method bounds (on the log scale for a positive family).
fn icdf_bounds(
    p: &str,
    x: &str,
    (q, lo, up): (&str, &str, &str),
    alpha: f64,
    positive: bool,
) -> String {
    let linear = format!(
        "app_s = sqrt(max(sum(app_G .* (app_C * app_G), 1), 0));\n\
         {lo} = {q} - app_z*app_s; {up} = {q} + app_z*app_s;\n"
    );
    let body = if positive {
        format!(
            "if all({q} > 0)\n\
             app_D = app_G ./ repmat({q}, numel(app_v), 1);\n\
             app_s = sqrt(max(sum(app_D .* (app_C * app_D), 1), 0));\n\
             {lo} = {q} .* exp(-app_z*app_s); {up} = {q} .* exp(app_z*app_s);\n\
             else\n{linear}end\n"
        )
    } else {
        linear
    };
    format!(
        "{q} = reshape(icdf({p}, {x}), 1, []);\n{}app_z = norminv(1 - {}/2);\n{body}",
        gradient(p, x, "icdf", q),
        number(alpha),
    )
}

// ---------------------------------------------------------------------------
// The app

pub struct DistributionFitter {
    pub data_sets: Vec<DataSet>,
    pub rules: Vec<Rule>,
    pub fits: Vec<Fit>,
    /// Data set and rule the INPUTS editors show.
    pub data_index: usize,
    pub rule_index: usize,
    /// The selected fit: INPUTS edits it, OUTPUT details it.
    pub selected: usize,
    pub display: Display,
    pub bins: Bins,
    pub bin_width: String,
    pub bin_count: u32,
    /// Confidence bounds on the Cdf, Survivor and Cumulative hazard displays.
    pub bounds: bool,
    /// Confidence level, percent.
    pub level: f64,
    pub scale: Scale,
    pub sort: Sort,
    pub pane: Pane,
    pub function: Function,
    pub points: String,
    pub point_bounds: bool,
    /// Last good run's view; errors keep it on screen.
    pub shown: Option<Shown>,
    next_id: u32,
    next_fit: usize,
    next_rule: usize,
}
impl Default for DistributionFitter {
    fn default() -> Self {
        Self {
            data_sets: vec![DataSet::new(1)],
            rules: Vec::new(),
            fits: vec![Fit::new(2, "Fit 1".into(), 1)],
            data_index: 0,
            rule_index: 0,
            selected: 0,
            display: Display::Density,
            bins: Bins::Auto,
            bin_width: String::new(),
            bin_count: 10,
            bounds: false,
            level: 95.,
            scale: Scale::Normal,
            sort: Sort::Created,
            pane: Pane::Fits,
            function: Function::Cdf,
            points: String::new(),
            point_bounds: false,
            shown: None,
            next_id: 3,
            next_fit: 2,
            next_rule: 1,
        }
    }
}

impl DistributionFitter {
    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn data_set(&self, id: u32) -> Option<&DataSet> {
        self.data_sets.iter().find(|d| d.id == id)
    }
    pub fn rule(&self, id: u32) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }
    pub fn fit(&self) -> Option<&Fit> {
        self.fits.get(self.selected)
    }
    /// New data set; the editor shows it.
    pub fn new_data_set(&mut self) -> Option<u32> {
        if self.data_sets.len() >= MAX_DATA_SETS {
            return None;
        }
        let id = self.id();
        self.data_sets.push(DataSet::new(id));
        self.data_index = self.data_sets.len() - 1;
        Some(id)
    }
    /// Deletes the edited data set and the fits on it.
    pub fn delete_data_set(&mut self) {
        if self.data_index >= self.data_sets.len() {
            return;
        }
        let id = self.data_sets.remove(self.data_index).id;
        self.fits.retain(|f| f.data_set != id);
        self.data_index = self.data_index.min(self.data_sets.len().saturating_sub(1));
        self.selected = self.selected.min(self.fits.len().saturating_sub(1));
    }
    /// New exclusion rule; the editor shows it.
    pub fn new_rule(&mut self) -> Option<u32> {
        if self.rules.len() >= MAX_RULES {
            return None;
        }
        let id = self.id();
        let name = format!("Rule {}", self.next_rule);
        self.next_rule += 1;
        self.rules.push(Rule::new(id, name));
        self.rule_index = self.rules.len() - 1;
        Some(id)
    }
    /// Deletes the edited rule; its fits keep all values.
    pub fn delete_rule(&mut self) {
        if self.rule_index >= self.rules.len() {
            return;
        }
        let id = self.rules.remove(self.rule_index).id;
        for fit in &mut self.fits {
            if fit.rule == Some(id) {
                fit.rule = None;
            }
        }
        self.rule_index = self.rule_index.min(self.rules.len().saturating_sub(1));
    }
    fn next_fit_name(&mut self) -> String {
        let name = format!("Fit {}", self.next_fit);
        self.next_fit += 1;
        name
    }
    /// New fit of `family` on the selected fit's data set and rule (or the
    /// edited data set); it becomes the selected fit.
    pub fn new_fit(&mut self, family: Family) -> Option<u32> {
        if self.fits.len() >= MAX_FITS || self.data_sets.is_empty() {
            return None;
        }
        let (data_set, rule) = match self.fit() {
            Some(f) => (f.data_set, f.rule),
            None => (
                self.data_sets[self.data_index.min(self.data_sets.len() - 1)].id,
                None,
            ),
        };
        let id = self.id();
        let name = self.next_fit_name();
        let mut fit = Fit::new(id, name, data_set);
        fit.rule = rule;
        fit.family = family;
        self.fits.push(fit);
        self.selected = self.fits.len() - 1;
        Some(id)
    }
    /// A copy of the selected fit under a new name.
    pub fn duplicate(&mut self) {
        if self.fits.len() >= MAX_FITS {
            return;
        }
        let Some(mut copy) = self.fit().cloned() else {
            return;
        };
        copy.id = self.id();
        copy.name = self.next_fit_name();
        copy.object_edited = false;
        copy.result = None;
        self.fits.push(copy);
        self.selected = self.fits.len() - 1;
    }
    pub fn delete_fit(&mut self) {
        if self.selected < self.fits.len() {
            self.fits.remove(self.selected);
            self.selected = self.selected.min(self.fits.len().saturating_sub(1));
        }
    }
    pub fn select(&mut self, index: usize) {
        if index < self.fits.len() {
            self.selected = index;
        }
    }

    /// Fit indices in the table's order; fits without a value go last.
    pub fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.fits.len()).collect();
        if self.sort != Sort::Created {
            let key = |i: &usize| {
                self.fits[*i]
                    .result
                    .as_ref()
                    .map(|r| r.criterion(self.sort))
                    .filter(|v| !v.is_nan())
                    .unwrap_or(f64::INFINITY)
            };
            order.sort_by(|a, b| key(a).total_cmp(&key(b)));
        }
        order
    }
    /// Workspace variables of every data set.
    fn data_variables(&self) -> BTreeSet<String> {
        self.data_sets.iter().flat_map(|d| d.variables()).collect()
    }
    /// The exported objects' names: the user's, else `pd1`, `pd2`, ...
    /// unused by the workspace, the data and the other fits.
    pub fn default_objects(&self, ws: &Snapshot) -> BTreeMap<u32, String> {
        let data = self.data_variables();
        let mut names = BTreeMap::new();
        let mut used = BTreeSet::new();
        for fit in self.fits.iter().filter(|f| f.export && f.object_edited) {
            let name = fit.object.trim().to_owned();
            used.insert(name.clone());
            names.insert(fit.id, name);
        }
        let mut n = 1;
        for fit in self.fits.iter().filter(|f| f.export && !f.object_edited) {
            let name = loop {
                let name = format!("pd{n}");
                n += 1;
                if !ws.contains(&name) && !used.contains(&name) && !data.contains(&name) {
                    break name;
                }
            };
            used.insert(name.clone());
            names.insert(fit.id, name);
        }
        names
    }
    /// The exported objects' names, checked.
    pub fn object_names(&self, ws: &Snapshot) -> Result<BTreeMap<u32, String>, String> {
        let data = self.data_variables();
        let mut used = BTreeSet::new();
        for fit in self.fits.iter().filter(|f| f.export && f.object_edited) {
            let name = fit.object.trim();
            let what = fit.title();
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Distribution Fitter: {what}: the object name is not a valid variable name"
                ));
            }
            if data.contains(name) {
                return Err(format!(
                    "Distribution Fitter: {what}: the object name {name} is a data variable"
                ));
            }
            if !used.insert(name) {
                return Err(format!(
                    "Distribution Fitter: two fits export to {name}: give them different names"
                ));
            }
        }
        Ok(self.default_objects(ws))
    }
    /// The data set and rule of `fit`, checked.
    fn source(&self, fit: &Fit) -> Result<(&DataSet, Option<&Rule>), String> {
        let ds = self.data_set(fit.data_set).ok_or_else(|| {
            format!(
                "Distribution Fitter: {}: choose a data set in INPUTS",
                fit.title()
            )
        })?;
        let rule = match fit.rule {
            Some(id) => Some(self.rule(id).ok_or_else(|| {
                format!(
                    "Distribution Fitter: {}: its exclusion rule is gone",
                    fit.title()
                )
            })?),
            None => None,
        };
        Ok((ds, rule))
    }
    /// The (min, max) expressions over every data set's values.
    fn range(&self) -> (String, String) {
        let data: Vec<&str> = self.data_sets.iter().map(|d| d.data.as_str()).collect();
        if let [one] = data.as_slice() {
            (format!("min({one})"), format!("max({one})"))
        } else {
            let all: Vec<String> = data.iter().map(|d| format!("{d}(:)")).collect();
            let all = all.join("; ");
            (format!("min([{all}])"), format!("max([{all}])"))
        }
    }
    /// histcounts' and histogram's bin options.
    fn bin_options(&self) -> Result<String, String> {
        Ok(match self.bins {
            Bins::Auto => String::new(),
            Bins::Scott => ", 'BinMethod', 'scott'".into(),
            Bins::FreedmanDiaconis => ", 'BinMethod', 'fd'".into(),
            Bins::Integers => ", 'BinMethod', 'integers'".into(),
            Bins::Width => match parse_number(&self.bin_width, "the bin width")? {
                Some(w) if w.is_finite() && w > 0. => format!(", 'BinWidth', {}", number(w)),
                _ => return Err("Distribution Fitter: type a positive bin width".into()),
            },
            Bins::Count => format!(", 'NumBins', {}", self.bin_count.clamp(1, 1000)),
        })
    }
    fn alpha(&self) -> f64 {
        1. - self.level.clamp(50., 99.9) / 100.
    }

    /// Everything the script, the probe and Export share, checked.
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.data_sets.is_empty() {
            return Err("Distribution Fitter: add a data set in INPUTS".into());
        }
        for ds in &self.data_sets {
            ds.check(ws)?;
        }
        let objects = self.object_names(ws)?;
        let data_vars = self.data_variables();
        let mut taken: BTreeSet<String> = objects.values().cloned().collect();
        taken.extend(data_vars);
        let claim = |base: &str, taken: &mut BTreeSet<String>| {
            let name = unused_name(base, |n| ws.contains(n) || taken.contains(n));
            taken.insert(name.clone());
            name
        };
        let mut keeps = BTreeMap::new();
        let mut script = format!(
            "% Distribution Fitter: {} data set{}, {} fit{} exported\n",
            self.data_sets.len(),
            if self.data_sets.len() == 1 { "" } else { "s" },
            objects.len(),
            if objects.len() == 1 { "" } else { "s" },
        );
        // Every fit is checked, exported or not, so errors show at once.
        for fit in &self.fits {
            let (ds, rule) = self.source(fit)?;
            if let Some(rule) = rule {
                rule.mask(&ds.data)?;
            }
            fit.options()?;
        }
        for fit in self.fits.iter().filter(|f| objects.contains_key(&f.id)) {
            let (ds, rule) = self.source(fit)?;
            if let Some(rule) = rule
                && !keeps.contains_key(&(ds.id, rule.id))
            {
                let name = claim("keep", &mut taken);
                script.push_str(&format!(
                    "% {}: {}\n{name} = {};\n",
                    rule.title(),
                    rule.describe().replace('≤', "<="),
                    rule.mask(&ds.data)?
                ));
                keeps.insert((ds.id, rule.id), name);
            }
            let keep = rule.and_then(|r| keeps.get(&(ds.id, r.id)));
            let columns = Columns::new(ds, keep.map(String::as_str));
            script.push_str(&format!(
                "% {}: {} on {}\n{} = fitdist({}, '{}'{}{});\n",
                fit.title(),
                fit.family.label(),
                ds.title(),
                objects[&fit.id],
                columns.data,
                fit.family.name(),
                columns.options(),
                fit.options()?
            ));
        }
        let (plot, helpers, handles) =
            self.plot_block(&objects, &mut |base| claim(base, &mut taken))?;
        script.push_str(&plot);
        Ok(Plan {
            objects,
            keeps,
            helpers,
            handles,
            script,
        })
    }

    /// The chosen display drawn from the data sets and exported fits, with
    /// its helper names and (probability plot) data handles.
    #[allow(clippy::type_complexity)]
    fn plot_block(
        &self,
        objects: &BTreeMap<u32, String>,
        claim: &mut dyn FnMut(&str) -> String,
    ) -> Result<(String, Vec<String>, BTreeMap<u32, String>), String> {
        let display = self.display;
        let mut helpers = Vec::new();
        let mut handles = BTreeMap::new();
        let mut out = format!("% {} display\nfigure\n", display.label());
        // Confidence bounds, drawn after the curves the legend names.
        let mut bound_lines = String::new();
        let mut fit_bounds = false;
        let alpha = self.alpha();
        // With bounds the legend names its curves by handle.
        let bounded = self.bounds && display.has_bounds();
        let mut curve_handles = Vec::new();
        // `hold on` before the second plotting call.
        let mut drawn = 0;
        let mut next = |out: &mut String| {
            drawn += 1;
            if drawn == 2 {
                out.push_str("hold on\n");
            }
        };
        let mut labels = Vec::new();
        let bins = self.bin_options()?;
        for ds in &self.data_sets {
            let columns = Columns::new(ds, None);
            next(&mut out);
            match display {
                Display::Density => out.push_str(&format!(
                    "histogram({}, 'Normalization', 'pdf'{bins})\n",
                    columns.histogram()
                )),
                Display::Cdf | Display::Survivor | Display::CumulativeHazard => {
                    let function = display
                        .ecdf_function()
                        .map(|f| format!(", 'Function', '{f}'"))
                        .unwrap_or_default();
                    if bounded {
                        // Outputs, so the bound lines follow every curve
                        // and the legend labels stay on the curves.
                        let [f, x, lo, up, h] = ["eF", "eX", "eLo", "eUp", "hc"].map(&mut *claim);
                        out.push_str(&format!(
                            "[{f}, {x}, {lo}, {up}] = ecdf({}{}{function}, 'Bounds', 'on', 'Alpha', {});\n\
                             {h} = stairs({x}, {f});\n",
                            columns.data,
                            columns.options(),
                            number(alpha)
                        ));
                        bound_lines.push_str(&format!(
                            "stairs({x}, {lo}, 'k:')\nstairs({x}, {up}, 'k:')\n"
                        ));
                        curve_handles.push(h.clone());
                        helpers.extend([f, x, lo, up, h]);
                    } else {
                        out.push_str(&format!(
                            "ecdf({}{}{function})\n",
                            columns.data,
                            columns.options()
                        ));
                    }
                }
                Display::Quantile => {
                    let (f, x) = (claim("qF"), claim("qX"));
                    // The quantile at probabilities F(i-1)..F(i) is x(i).
                    out.push_str(&format!(
                        "[{f}, {x}] = ecdf({}{});\nstairs({f}, {x}([2:end end]))\n",
                        columns.data,
                        columns.options()
                    ));
                    helpers.extend([f, x]);
                }
                Display::Probability => {
                    let h = claim("hp");
                    out.push_str(&format!(
                        "{h} = probplot('{}', {}{}, 'noref');\n",
                        self.scale.name(),
                        columns.data,
                        columns.positional()
                    ));
                    handles.insert(ds.id, h.clone());
                    helpers.push(h);
                }
            }
            labels.push(ds.title());
        }
        let (lo, hi) = self.range();
        // Evaluation grids, made the first time a fit needs one.
        let mut grids: BTreeMap<&'static str, String> = BTreeMap::new();
        let mut fit_handles = Vec::new();
        for fit in self.fits.iter().filter(|f| objects.contains_key(&f.id)) {
            if display == Display::Probability && fit.family == Family::Kernel {
                continue;
            }
            let pd = &objects[&fit.id];
            next(&mut out);
            let discrete = fit.family.discrete();
            let (base, expr) = match (display, discrete) {
                (Display::Quantile, _) => {
                    ("pg", format!("linspace(0.001, 0.999, {CURVE_POINTS})'"))
                }
                (_, false) => ("xg", format!("linspace({lo}, {hi}, {CURVE_POINTS})'")),
                (_, true) => (
                    "xi",
                    format!("unique(round(linspace({lo}, {hi}, {CURVE_POINTS})))'"),
                ),
            };
            let x = if display == Display::Probability {
                String::new()
            } else if let Some(name) = grids.get(base) {
                name.clone()
            } else {
                let name = claim(base);
                out.push_str(&format!("{name} = {expr};\n"));
                helpers.push(name.clone());
                grids.insert(base, name.clone());
                name
            };
            let line = if discrete { "stairs" } else { "plot" };
            match display {
                Display::Probability => {
                    let h = claim("hf");
                    out.push_str(&format!("{h} = probplot(gca, {pd});\n"));
                    fit_handles.push(h.clone());
                    helpers.push(h);
                }
                Display::Density => {
                    let style = if discrete { ", '.-'" } else { "" };
                    out.push_str(&format!("plot({x}, pdf({pd}, {x}){style})\n"));
                }
                Display::Quantile => out.push_str(&format!("{line}({x}, icdf({pd}, {x}))\n")),
                display => {
                    let call = match display {
                        Display::Cdf => format!("{line}({x}, cdf({pd}, {x}))"),
                        Display::Survivor => format!("{line}({x}, cdf({pd}, {x}, 'upper'))"),
                        _ => format!("{line}({x}, -log(cdf({pd}, {x}, 'upper')))"),
                    };
                    if bounded {
                        let h = claim("hc");
                        out.push_str(&format!("{h} = {call};\n"));
                        curve_handles.push(h.clone());
                        helpers.push(h);
                    } else {
                        out.push_str(&format!("{call}\n"));
                    }
                }
            }
            if bounded && fit.family != Family::Kernel {
                let [lo, up] = ["fLo", "fUp"].map(&mut *claim);
                out.push_str(&cdf_bounds(pd, &x, ("app_F", "app_L", "app_U"), alpha));
                out.push_str(&match display {
                    Display::Cdf => format!("{lo} = app_L; {up} = app_U;\n"),
                    Display::Survivor => format!("{lo} = 1 - app_U; {up} = 1 - app_L;\n"),
                    _ => format!("{lo} = -log(1 - app_L); {up} = -log(1 - app_U);\n"),
                });
                bound_lines.push_str(&format!(
                    "{line}({x}, {lo}, 'k:')\n{line}({x}, {up}, 'k:')\n"
                ));
                helpers.extend([lo, up]);
                fit_bounds = true;
            }
            labels.push(fit.title());
        }
        if !bound_lines.is_empty() {
            if drawn == 1 {
                out.push_str("hold on\n");
            }
            out.push_str(&bound_lines);
            drawn += 1;
        }
        if fit_bounds {
            out.push_str(
                "clear app_v app_C app_G app_k app_h app_w app_c app_d app_in app_D app_s app_z app_g \
                 app_F app_L app_U\n",
            );
        }
        if drawn > 1 {
            out.push_str("hold off\n");
        }
        let labels: Vec<String> = labels.iter().map(|l| quote(l)).collect();
        let labels = format!("{{{}}}", labels.join(", "));
        if display == Display::Probability {
            let members: Vec<String> = self
                .data_sets
                .iter()
                .map(|d| format!("{}(1)", handles[&d.id]))
                .chain(fit_handles)
                .collect();
            out.push_str(&format!("legend([{}], {labels})\n", members.join("; ")));
        } else {
            let (x, y) = display.axis_labels();
            if bounded {
                out.push_str(&format!(
                    "legend([{}], {labels})\n",
                    curve_handles.join("; ")
                ));
            } else {
                out.push_str(&format!("legend({labels})\n"));
            }
            out.push_str(&format!("xlabel('{x}')\nylabel('{y}')\n"));
        }
        Ok((out, helpers, handles))
    }

    /// Lines computing what the view draws for one data set.
    fn data_probe(&self, ds: &DataSet, plan: &Plan) -> Result<String, String> {
        let id = ds.id;
        let columns = Columns::new(ds, None);
        let names = format!("app_d{id}_x, app_d{id}_y");
        Ok(match self.display {
            Display::Density => format!(
                "[app_d{id}_y, app_d{id}_x] = histcounts({}, 'Normalization', 'pdf'{});\n\
                 [app_d{id}_lo, app_d{id}_up] = deal([]);\n",
                columns.histogram(),
                self.bin_options()?
            ),
            Display::Probability => {
                let h = &plan.handles[&id];
                format!(
                    "[{names}] = deal(get({h}(1), 'XData'), get({h}(1), 'YData'));\n\
                     [app_d{id}_lo, app_d{id}_up] = deal([]);\n"
                )
            }
            display => {
                let function = display
                    .ecdf_function()
                    .map(|f| format!(", 'Function', '{f}'"))
                    .unwrap_or_default();
                if display.has_bounds() && self.bounds {
                    format!(
                        "[app_d{id}_y, app_d{id}_x, app_d{id}_lo, app_d{id}_up] = \
                         ecdf({}{}{function}, 'Bounds', 'on', 'Alpha', {});\n",
                        columns.data,
                        columns.options(),
                        number(self.alpha())
                    )
                } else {
                    format!(
                        "[app_d{id}_y, app_d{id}_x] = ecdf({}{}{function});\n\
                         [app_d{id}_lo, app_d{id}_up] = deal([]);\n",
                        columns.data,
                        columns.options()
                    )
                }
            }
        })
    }

    /// Lines computing one fit's numbers, curve and evaluated points from
    /// its private copy `app_p<id>`.
    fn fit_probe(&self, fit: &Fit, columns: &Columns, points: &[f64]) -> String {
        let id = fit.id;
        let p = format!("app_p{id}");
        let f = |part: &str| format!("app_f{id}_{part}");
        let kernel = fit.family == Family::Kernel;
        let mut code = format!("{} = negloglik({p});\n", f("nll"));
        if kernel {
            code.push_str(&format!(
                "[{}, {}, {}, {}] = deal([]); {} = ''; {} = NaN; {} = {p}.BandWidth;\n",
                f("v"),
                f("fx"),
                f("ci"),
                f("cov"),
                f("nm"),
                f("k"),
                f("bw"),
            ));
        } else {
            code.push_str(&format!(
                "{} = {p}.ParameterValues; {} = strjoin({p}.ParameterNames, ',');\n\
                 {} = double({p}.ParameterIsFixed); {} = paramci({p});\n\
                 {} = {p}.ParameterCovariance; {} = sum(~{p}.ParameterIsFixed); {} = [];\n",
                f("v"),
                f("nm"),
                f("fx"),
                f("ci"),
                f("cov"),
                f("k"),
                f("bw"),
            ));
        }
        code.push_str(&columns.count(&f("n")));
        code.push_str(&format!(
            "{aic} = 2*{nll} + 2*{k}; {bic} = 2*{nll} + {k}*log({n});\n\
             {mean} = mean({p}); {var} = var({p});\n",
            aic = f("aic"),
            bic = f("bic"),
            nll = f("nll"),
            k = f("k"),
            n = f("n"),
            mean = f("mean"),
            var = f("var"),
        ));
        // The display's curve.
        let x = match (self.display, fit.family.discrete()) {
            (Display::Quantile, _) => "app_pg",
            (_, true) => "app_xi",
            (_, false) => "app_xg",
        };
        let bounds = self.bounds && self.display.has_bounds() && !kernel;
        let alpha = self.alpha();
        let (fx, fy, flo, fup) = (f("x"), f("y"), f("lo"), f("up"));
        match self.display {
            Display::Density => code.push_str(&format!(
                "{fx} = {x}; {fy} = pdf({p}, {x}); [{flo}, {fup}] = deal([]);\n"
            )),
            Display::Quantile => code.push_str(&format!(
                "{fx} = {x}; {fy} = icdf({p}, {x}); [{flo}, {fup}] = deal([]);\n"
            )),
            Display::Probability => {
                if kernel {
                    code.push_str(&format!("[{fx}, {fy}, {flo}, {fup}] = deal([]);\n"));
                } else {
                    code.push_str(&format!(
                        "app_l = probplot(gca, {p}); {fx} = get(app_l, 'XData'); \
                         {fy} = get(app_l, 'YData'); [{flo}, {fup}] = deal([]);\n"
                    ));
                }
            }
            display => {
                code.push_str(&format!("{fx} = {x};\n"));
                let value = match display {
                    Display::Cdf => format!("cdf({p}, {x})"),
                    Display::Survivor => format!("cdf({p}, {x}, 'upper')"),
                    _ => format!("-log(cdf({p}, {x}, 'upper'))"),
                };
                code.push_str(&format!("{fy} = {value};\n"));
                if bounds {
                    code.push_str(&cdf_bounds(&p, x, ("app_F", "app_L", "app_U"), alpha));
                    code.push_str(&match display {
                        Display::Cdf => format!("{flo} = app_L; {fup} = app_U;\n"),
                        Display::Survivor => format!("{flo} = 1 - app_U; {fup} = 1 - app_L;\n"),
                        _ => format!("{flo} = -log(1 - app_L); {fup} = -log(1 - app_U);\n"),
                    });
                } else {
                    code.push_str(&format!("[{flo}, {fup}] = deal([]);\n"));
                }
            }
        }
        // The Evaluate panel.
        let (ex, ev, el, eu) = (f("ex"), f("ev"), f("el"), f("eu"));
        if points.is_empty() {
            code.push_str(&format!("[{ex}, {ev}, {el}, {eu}] = deal([]);\n"));
            return code;
        }
        code.push_str(&format!("{ex} = {};\n", row(points)));
        let bounded = self.point_bounds && self.function.has_bounds() && !kernel;
        match (self.function, bounded) {
            (Function::Pdf, _) => code.push_str(&format!(
                "{ev} = pdf({p}, {ex}); [{el}, {eu}] = deal([]);\n"
            )),
            (Function::Cdf, false) => code.push_str(&format!(
                "{ev} = cdf({p}, {ex}); [{el}, {eu}] = deal([]);\n"
            )),
            (Function::Survivor, false) => code.push_str(&format!(
                "{ev} = cdf({p}, {ex}, 'upper'); [{el}, {eu}] = deal([]);\n"
            )),
            (Function::Icdf, false) => code.push_str(&format!(
                "{ev} = icdf({p}, {ex}); [{el}, {eu}] = deal([]);\n"
            )),
            (Function::Cdf, true) => {
                code.push_str(&cdf_bounds(&p, &ex, (&ev, &el, &eu), alpha));
            }
            (Function::Survivor, true) => {
                code.push_str(&cdf_bounds(&p, &ex, ("app_F", "app_L", "app_U"), alpha));
                code.push_str(&format!(
                    "{ev} = cdf({p}, {ex}, 'upper'); {el} = 1 - app_U; {eu} = 1 - app_L;\n"
                ));
            }
            (Function::Icdf, true) => {
                code.push_str(&icdf_bounds(
                    &p,
                    &ex,
                    (&ev, &el, &eu),
                    alpha,
                    fit.family.positive(),
                ));
            }
        }
        code
    }

    /// The Evaluate panel's points, checked (empty: nothing to evaluate).
    pub fn evaluate_points(&self) -> Result<Vec<f64>, String> {
        let points = parse_points(&self.points)?;
        if self.function == Function::Icdf && points.iter().any(|p| !(0. ..=1.).contains(p)) {
            return Err("Evaluate: probabilities are from 0 to 1".into());
        }
        Ok(points)
    }
}

impl App for DistributionFitter {
    fn kind(&self) -> AppKind {
        AppKind::DistributionFitter
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        self.data_variables().into_iter().collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let points = self.evaluate_points().unwrap_or_default();
        let mut code = String::new();
        let mut names = Vec::new();
        // Private copies of the exported fits first, so no probe name can
        // shadow the user's names; the other fits are fitted here.
        // (`deal` takes no single distribution object, so plain copies.)
        for (id, name) in &plan.objects {
            code.push_str(&format!("app_p{id} = {name};\n"));
        }
        let mut columns = BTreeMap::new();
        for fit in &self.fits {
            let Ok((ds, rule)) = self.source(fit) else {
                return (String::new(), Vec::new());
            };
            let keep = match (rule, plan.objects.contains_key(&fit.id)) {
                (None, _) => None,
                (Some(rule), true) => plan.keeps.get(&(ds.id, rule.id)).cloned(),
                (Some(rule), false) => {
                    let name = format!("app_k{}", fit.id);
                    let Ok(mask) = rule.mask(&ds.data) else {
                        return (String::new(), Vec::new());
                    };
                    code.push_str(&format!("{name} = {mask};\n"));
                    Some(name)
                }
            };
            let c = Columns::new(ds, keep.as_deref());
            if !plan.objects.contains_key(&fit.id) {
                code.push_str(&format!(
                    "app_p{} = fitdist({}, '{}'{}{});\n",
                    fit.id,
                    c.data,
                    fit.family.name(),
                    c.options(),
                    fit.options().unwrap_or_default()
                ));
            }
            columns.insert(fit.id, c);
        }
        let (lo, hi) = self.range();
        code.push_str(&format!(
            "app_xg = linspace({lo}, {hi}, {CURVE_POINTS})';\n\
             app_xi = unique(round(linspace({lo}, {hi}, {CURVE_POINTS})))';\n\
             app_pg = linspace(0.001, 0.999, {CURVE_POINTS})';\n"
        ));
        for ds in &self.data_sets {
            let Ok(lines) = self.data_probe(ds, &plan) else {
                return (String::new(), Vec::new());
            };
            code.push_str(&lines);
            for part in ["x", "y", "lo", "up"] {
                names.push(format!("app_d{}_{part}", ds.id));
            }
        }
        if self.display == Display::Probability {
            // Fitted lines over every data set's range, and the axis marks.
            let all: Vec<String> = self
                .data_sets
                .iter()
                .map(|d| format!("app_d{}_x(:)", d.id))
                .collect();
            code.push_str(&format!(
                "app_a = [{}]; app_a = app_a(isfinite(app_a));\n\
                 if ~isempty(app_a) && min(app_a) < max(app_a), xlim([min(app_a) max(app_a)]); end\n\
                 app_pl = {}; app_tk = {};\n",
                all.join("; "),
                row(&LEVELS),
                self.scale.position("app_pl")
            ));
            names.push("app_tk".into());
        }
        for fit in &self.fits {
            code.push_str(&self.fit_probe(fit, &columns[&fit.id], &points));
            for part in [
                "nll", "v", "nm", "fx", "ci", "cov", "k", "bw", "n", "aic", "bic", "mean", "var",
                "x", "y", "lo", "up", "ex", "ev", "el", "eu",
            ] {
                names.push(format!("app_f{}_{part}", fit.id));
            }
        }
        (code, names)
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        self.data_index = self.data_index.min(self.data_sets.len().saturating_sub(1));
        self.rule_index = self.rule_index.min(self.rules.len().saturating_sub(1));
        self.selected = self.selected.min(self.fits.len().saturating_sub(1));
        // Default names: the data variable, numbered when it repeats.
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for ds in &mut self.data_sets {
            if ds.name_edited {
                continue;
            }
            let base = if ds.data.is_empty() {
                format!("Data set {}", ds.id)
            } else {
                ds.data.clone()
            };
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            ds.name = if *count == 1 {
                base
            } else {
                format!("{base} ({count})")
            };
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let display = self.display;
        let data = self
            .data_sets
            .iter()
            .map(|ds| DataCurve::from_outputs(outputs, ds.id, display))
            .collect::<Result<Vec<_>, String>>()?;
        let ticks = if display == Display::Probability {
            let positions = values(outputs, "app_tk")?;
            LEVELS
                .iter()
                .zip(positions)
                .filter(|(_, y)| y.is_finite())
                .map(|(p, y)| (y, short(*p)))
                .collect()
        } else {
            Vec::new()
        };
        let results = self
            .fits
            .iter()
            .map(|fit| FitResult::from_outputs(outputs, fit, display))
            .collect::<Result<Vec<_>, String>>()?;
        for (fit, result) in self.fits.iter_mut().zip(results) {
            fit.result = Some(result);
        }
        self.shown = Some(Shown {
            display,
            scale: self.scale,
            data,
            ticks,
        });
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.data_sets_ui(ui, ws);
        section(ui);
        self.rules_ui(ui);
        section(ui);
        self.fits_ui(ui);
    }
    fn main_caption(&self) -> &'static str {
        "DISTRIBUTIONS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for display in Display::ALL {
                let tab = crate::tokens::tab(ui, self.display == display, display.label());
                if display == Display::Density {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::DistributionFitter),
                    );
                }
                if tab.clicked() {
                    self.display = display;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        self.display_options_ui(ui);
        ui.add_space(crate::tokens::XS);
        let Some(shown) = &self.shown else {
            crate::tokens::helper(
                ui,
                "No fits yet: choose data in INPUTS. The status line says what is missing.",
            );
            return;
        };
        if shown.display == Display::Density
            && self.data_sets.iter().any(|d| !d.censoring.is_empty())
        {
            crate::tokens::helper(ui, "The histogram counts censored values as observed.");
        }
        if shown.display == Display::Probability
            && self.fits.iter().any(|f| f.family == Family::Kernel)
        {
            crate::tokens::helper(ui, "Kernel fits have no line on probability paper.");
        }
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        draw(ui, height, shown, &self.data_sets, &self.fits);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(fit) = self.fit() else {
            return vec![("Fit", "—".into())];
        };
        let data = self
            .data_set(fit.data_set)
            .map(DataSet::title)
            .unwrap_or_default();
        let mut rows = vec![
            ("Fit", fit.title()),
            ("Distribution", fit.family.label().to_owned()),
            ("Data set", data),
        ];
        match &fit.result {
            Some(r) => rows.extend([
                ("NLogL", short(r.nlogl)),
                ("AIC", short(r.aic)),
                ("BIC", short(r.bic)),
            ]),
            None => rows.extend(["NLogL", "AIC", "BIC"].map(|l| (l, "—".to_owned()))),
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for pane in Pane::ALL {
                if crate::tokens::tab(ui, self.pane == pane, pane.label()).clicked() {
                    self.pane = pane;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        match self.pane {
            Pane::Fits => self.table_ui(ui),
            Pane::Details => self.details_ui(ui),
            Pane::Evaluate => self.evaluate_ui(ui),
        }
        ui.add_space(crate::tokens::M);
        self.export_ui(ui, ws);
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        match self.plan(ws) {
            Ok(plan) => plan
                .objects
                .into_values()
                .chain(plan.keeps.into_values())
                .chain(plan.helpers)
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        self.shown.as_ref()?;
        let fitted: Vec<&Fit> = self.fits.iter().filter(|f| f.result.is_some()).collect();
        let Some(fit) = self.fit().filter(|f| f.result.is_some()) else {
            return Some(format!(
                "{} data set{} · no fits",
                self.data_sets.len(),
                if self.data_sets.len() == 1 { "" } else { "s" }
            ));
        };
        let r = fit.result.as_ref()?;
        let mut text = format!(
            "{}: {} · NLogL {} · AIC {}",
            fit.title(),
            fit.family.label(),
            short(r.nlogl),
            short(r.aic)
        );
        let best = fitted
            .iter()
            .filter(|f| f.result.as_ref().is_some_and(|r| r.aic.is_finite()))
            .min_by(|a, b| {
                let key = |f: &&&Fit| f.result.as_ref().map_or(f64::INFINITY, |r| r.aic);
                key(a).total_cmp(&key(b))
            });
        if fitted.len() > 1
            && let Some(best) = best
        {
            text.push_str(&format!(" · lowest AIC: {}", best.title()));
        }
        Some(text)
    }
}

// ---------------------------------------------------------------------------
// Inputs UI

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}
fn heading(ui: &mut egui::Ui, text: &str) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(crate::panels::caption_semibold(ui))
            .color(p.text_muted),
    );
    ui.add_space(crate::tokens::XS);
}
fn error_line(ui: &mut egui::Ui, text: &str) {
    let p = crate::tokens::palette(ui.ctx());
    ui.add(
        egui::Label::new(
            egui::RichText::new(text.trim_start_matches("Distribution Fitter: "))
                .size(crate::tokens::CAPTION)
                .color(p.error),
        )
        .wrap(),
    );
}
fn text_input(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(
        crate::tokens::input(egui::TextEdit::singleline(text).char_limit(63))
            .hint_text(hint)
            .desired_width(width),
    )
}

/// Real vectors of at least two values (logical ones when `logical`):
/// name and length.
pub fn vectors(ws: &Snapshot, logical: bool) -> Vec<(String, usize)> {
    ws.variables()
        .filter_map(|(name, array)| {
            let kind = matches!(array, Array::Numeric(_) | Array::Typed(_))
                || (logical && matches!(array, Array::Logical(_)));
            let dims = array.dims();
            (kind
                && !array.is_complex()
                && dims.len() == 2
                && (dims[0] == 1 || dims[1] == 1)
                && array.len() >= 2)
                .then(|| (name.to_owned(), array.len()))
        })
        .collect()
}

/// A variable dropdown; `none` names the empty choice of an optional input.
fn picker(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut String,
    options: &[(String, usize)],
    none: Option<&str>,
) {
    field(ui, label, |ui| {
        let shown = if value.is_empty() {
            none.unwrap_or("Choose a variable").to_owned()
        } else {
            value.clone()
        };
        egui::ComboBox::from_id_salt(id)
            .width(ui.available_width())
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if let Some(none) = none {
                    ui.selectable_value(value, String::new(), none);
                }
                for (name, length) in options {
                    ui.selectable_value(value, name.clone(), format!("{name} ({length})"));
                }
            });
    });
}

/// Selectable rows of a list; returns the clicked index.
fn list(ui: &mut egui::Ui, rows: &[String], selected: usize) -> Option<usize> {
    let mut clicked = None;
    for (i, text) in rows.iter().enumerate() {
        let wide = egui::vec2(ui.available_width(), crate::tokens::ROW);
        if ui
            .add(
                // A growing atom after the text keeps the text on the left.
                egui::Button::selectable(i == selected, text.as_str())
                    .right_text("")
                    .min_size(wide),
            )
            .clicked()
        {
            clicked = Some(i);
        }
    }
    clicked
}

/// Two buttons side by side: (left clicked, right clicked).
fn button_pair(
    ui: &mut egui::Ui,
    (left, left_on): (&str, bool),
    (right, right_on): (&str, bool),
) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        out.0 = ui.add_enabled(left_on, egui::Button::new(left)).clicked();
        out.1 = ui.add_enabled(right_on, egui::Button::new(right)).clicked();
    });
    out
}

impl DistributionFitter {
    fn data_sets_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        heading(ui, "Data sets");
        let rows: Vec<String> = self
            .data_sets
            .iter()
            .map(|d| match ws.get(&d.data) {
                Some(a) => format!("{} ({} values)", d.title(), a.len()),
                None => d.title(),
            })
            .collect();
        if let Some(i) = list(ui, &rows, self.data_index) {
            self.data_index = i;
        }
        let (new, delete) = button_pair(
            ui,
            ("New data set", self.data_sets.len() < MAX_DATA_SETS),
            ("Delete", self.data_sets.len() > 1),
        );
        if new {
            self.new_data_set();
        }
        if delete {
            self.delete_data_set();
        }
        ui.add_space(crate::tokens::S);
        let options = vectors(ws, false);
        let flags = vectors(ws, true);
        let Some(ds) = self.data_sets.get_mut(self.data_index) else {
            return;
        };
        picker(ui, "dfit_data", "Data", &mut ds.data, &options, None);
        if options.is_empty() {
            crate::tokens::helper(ui, "No numeric vectors in the workspace: create one first");
        }
        let length = ws.get(&ds.data).map(Array::len);
        let same = |list: &[(String, usize)]| -> Vec<(String, usize)> {
            list.iter()
                .filter(|(_, n)| length.is_none_or(|l| *n == l))
                .cloned()
                .collect()
        };
        picker(
            ui,
            "dfit_censoring",
            "Censoring",
            &mut ds.censoring,
            &same(&flags),
            Some("None"),
        );
        picker(
            ui,
            "dfit_frequency",
            "Frequency",
            &mut ds.frequency,
            &same(&options),
            Some("None"),
        );
        crate::tokens::helper(ui, "Censoring: 1 marks a right-censored value.");
        ui.add_space(crate::tokens::XS);
        field(ui, "Data set name", |ui| {
            let before = ds.name.clone();
            text_input(ui, &mut ds.name, "name", ui.available_width());
            ds.name_edited |= ds.name != before;
        });
    }

    fn rules_ui(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Exclusion rules");
        let rows: Vec<String> = self
            .rules
            .iter()
            .map(|r| format!("{}: {}", r.title(), r.describe()))
            .collect();
        if rows.is_empty() {
            crate::tokens::helper(ui, "None: fits use every value.");
        }
        if let Some(i) = list(ui, &rows, self.rule_index) {
            self.rule_index = i;
        }
        let (new, delete) = button_pair(
            ui,
            ("New rule", self.rules.len() < MAX_RULES),
            ("Delete", !self.rules.is_empty()),
        );
        if new {
            self.new_rule();
        }
        if delete {
            self.delete_rule();
        }
        let Some(rule) = self.rules.get_mut(self.rule_index) else {
            return;
        };
        ui.add_space(crate::tokens::S);
        field(ui, "Rule name", |ui| {
            text_input(ui, &mut rule.name, "name", ui.available_width());
        });
        for (label, text, strict) in [
            ("Keep values above", &mut rule.lower, &mut rule.lower_strict),
            ("Keep values below", &mut rule.upper, &mut rule.upper_strict),
        ] {
            field(ui, label, |ui| {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 2. * 40. - crate::tokens::S).max(60.);
                    text_input(ui, text, "no limit", width);
                    let (equal, less) = if label.ends_with("above") {
                        ("≥", ">")
                    } else {
                        ("≤", "<")
                    };
                    if crate::tokens::toggle(ui, !*strict, equal).clicked() {
                        *strict = false;
                    }
                    if crate::tokens::toggle(ui, *strict, less).clicked() {
                        *strict = true;
                    }
                });
            });
        }
        crate::tokens::helper(ui, format!("Keeps {}", rule.describe()));
        if let Err(error) = rule.limits() {
            error_line(ui, &error);
        }
    }

    fn fits_ui(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Fits");
        let rows: Vec<String> = self
            .fits
            .iter()
            .map(|f| format!("{} · {}", f.title(), f.family.label()))
            .collect();
        if let Some(i) = list(ui, &rows, self.selected) {
            self.selected = i;
        }
        ui.horizontal(|ui| {
            let room = self.fits.len() < MAX_FITS && !self.data_sets.is_empty();
            if ui.add_enabled(room, egui::Button::new("New fit")).clicked() {
                self.new_fit(Family::Normal);
            }
            if ui
                .add_enabled(
                    room && !self.fits.is_empty(),
                    egui::Button::new("Duplicate"),
                )
                .clicked()
            {
                self.duplicate();
            }
            if ui
                .add_enabled(!self.fits.is_empty(), egui::Button::new("Delete"))
                .clicked()
            {
                self.delete_fit();
            }
        });
        let data: Vec<(u32, String)> = self.data_sets.iter().map(|d| (d.id, d.title())).collect();
        let rules: Vec<(u32, String)> = self.rules.iter().map(|r| (r.id, r.title())).collect();
        let Some(fit) = self.fits.get_mut(self.selected) else {
            crate::tokens::helper(ui, "No fits: New fit adds one.");
            return;
        };
        ui.add_space(crate::tokens::S);
        field(ui, "Fit name", |ui| {
            text_input(ui, &mut fit.name, "name", ui.available_width());
        });
        field(ui, "Data set", |ui| {
            let shown = data
                .iter()
                .find(|(id, _)| *id == fit.data_set)
                .map_or("Choose a data set", |(_, t)| t.as_str())
                .to_owned();
            egui::ComboBox::from_id_salt("dfit_fit_data")
                .width(ui.available_width())
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (id, title) in &data {
                        ui.selectable_value(&mut fit.data_set, *id, title);
                    }
                });
        });
        field(ui, "Exclusion rule", |ui| {
            let shown = fit
                .rule
                .and_then(|id| rules.iter().find(|(r, _)| *r == id))
                .map_or("None", |(_, t)| t.as_str())
                .to_owned();
            egui::ComboBox::from_id_salt("dfit_fit_rule")
                .width(ui.available_width())
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut fit.rule, None, "None");
                    for (id, title) in &rules {
                        ui.selectable_value(&mut fit.rule, Some(*id), title);
                    }
                });
        });
        family_ui(ui, fit);
        if let Err(error) = fit.options() {
            error_line(ui, &error);
        }
    }
}

fn family_ui(ui: &mut egui::Ui, fit: &mut Fit) {
    let p = crate::tokens::palette(ui.ctx());
    field(ui, "Distribution", |ui| {
        let mut chosen = fit.family;
        egui::ComboBox::from_id_salt("dfit_family")
            .width(ui.available_width())
            .height(480.)
            .selected_text(fit.family.label())
            .show_ui(ui, |ui| {
                for family in Family::ALL {
                    if let Some(group) = family.group() {
                        ui.label(
                            egui::RichText::new(group)
                                .size(crate::tokens::CAPTION)
                                .color(p.text_muted),
                        );
                    }
                    ui.selectable_value(&mut chosen, family, family.label());
                }
            });
        if chosen != fit.family {
            fit.family = chosen;
            fit.fixed.clear();
        }
    });
    if let Some(fixed) = fit.family.fixed() {
        field(ui, fixed.label, |ui| {
            text_input(ui, &mut fit.fixed, "", ui.available_width());
        });
    }
    if fit.family != Family::Kernel {
        return;
    }
    field(ui, "Kernel", |ui| {
        egui::ComboBox::from_id_salt("dfit_kernel")
            .width(ui.available_width())
            .selected_text(fit.smoother.label())
            .show_ui(ui, |ui| {
                for smoother in Smoother::ALL {
                    ui.selectable_value(&mut fit.smoother, smoother, smoother.label());
                }
            });
    });
    field(ui, "Bandwidth", |ui| {
        text_input(ui, &mut fit.bandwidth, "default", ui.available_width());
    });
    field(ui, "Support", |ui| {
        ui.horizontal(|ui| {
            for support in Support::ALL {
                if crate::tokens::toggle(ui, fit.support == support, support.label()).clicked() {
                    fit.support = support;
                }
            }
        });
        if fit.support == Support::Bounded {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - crate::tokens::S) / 2.;
                text_input(ui, &mut fit.support_lower, "lower", width);
                text_input(ui, &mut fit.support_upper, "upper", width);
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Main view

impl DistributionFitter {
    fn display_options_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| match self.display {
            Display::Density => {
                ui.label("Bins");
                egui::ComboBox::from_id_salt("dfit_bins")
                    .selected_text(self.bins.label())
                    .show_ui(ui, |ui| {
                        for bins in Bins::ALL {
                            ui.selectable_value(&mut self.bins, bins, bins.label());
                        }
                    });
                match self.bins {
                    Bins::Width => {
                        text_input(ui, &mut self.bin_width, "width", 80.);
                    }
                    Bins::Count => {
                        ui.add(egui::DragValue::new(&mut self.bin_count).range(1..=1000));
                    }
                    _ => {}
                }
            }
            Display::Probability => {
                ui.label("Paper");
                egui::ComboBox::from_id_salt("dfit_scale")
                    .selected_text(self.scale.label())
                    .show_ui(ui, |ui| {
                        for scale in Scale::ALL {
                            ui.selectable_value(&mut self.scale, scale, scale.label());
                        }
                    });
            }
            Display::Quantile => {
                crate::tokens::helper(ui, "Data and fitted quantiles against probability.");
            }
            _ => {
                ui.checkbox(&mut self.bounds, "Confidence bounds");
                ui.add_enabled(
                    self.bounds,
                    egui::DragValue::new(&mut self.level)
                        .range(50.0..=99.9)
                        .speed(0.1)
                        .max_decimals(1)
                        .suffix(" %"),
                );
            }
        });
    }
}

/// Series colour `k`: data sets first, then fits.
fn color(k: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[k % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}

fn draw(ui: &mut egui::Ui, height: f32, shown: &Shown, data_sets: &[DataSet], fits: &[Fit]) {
    let (x_label, y_label) = shown.display.axis_labels();
    let log_x = shown.display == Display::Probability && shown.scale.log_x();
    let tx = move |x: f64| if log_x { x.log10() } else { x };
    let along = |points: &[[f64; 2]]| -> Vec<[f64; 2]> {
        points
            .iter()
            .map(|p| [tx(p[0]), p[1]])
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .collect()
    };
    let mut chart = egui_plot::Plot::new("distribution_fitter_plot")
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x_label.to_owned())
        .y_axis_label(y_label.to_owned())
        .legend(egui_plot::Legend::default().follow_insertion_order(true))
        .show_background(false);
    if log_x {
        chart = chart.x_axis_formatter(|mark, _| short(10_f64.powf(mark.value)));
    }
    if !shown.ticks.is_empty() {
        let marks = shown.ticks.clone();
        let labels = shown.ticks.clone();
        chart = chart
            .y_grid_spacer(move |_| {
                marks
                    .iter()
                    .map(|(v, _)| egui_plot::GridMark {
                        value: *v,
                        step_size: 1.,
                    })
                    .collect()
            })
            .y_axis_formatter(move |mark, _| {
                labels
                    .iter()
                    .find(|(v, _)| (v - mark.value).abs() < 1e-9)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default()
            });
    }
    chart.show(ui, |plot_ui| {
        let mut k = 0;
        for curve in &shown.data {
            let Some(ds) = data_sets.iter().find(|d| d.id == curve.id) else {
                continue;
            };
            let name = ds.title();
            let ink = color(k);
            k += 1;
            if !curve.bins.is_empty() {
                let bars = curve
                    .bins
                    .iter()
                    .map(|[a, b, h]| egui_plot::Bar::new((a + b) / 2., *h).width(b - a))
                    .collect();
                plot_ui.bar_chart(
                    egui_plot::BarChart::new(name.clone(), bars)
                        .color(ink)
                        .allow_hover(false),
                );
            }
            if shown.display == Display::Probability {
                plot_ui.points(
                    egui_plot::Points::new(name.clone(), along(&curve.points))
                        .shape(egui_plot::MarkerShape::Plus)
                        .radius(3.5)
                        .color(ink),
                );
            } else if !curve.points.is_empty() {
                plot_ui.line(
                    egui_plot::Line::new(name.clone(), along(&curve.points))
                        .color(ink)
                        .width(1.5),
                );
            }
            for bound in [&curve.lower, &curve.upper] {
                if !bound.is_empty() {
                    plot_ui.line(
                        egui_plot::Line::new(name.clone(), along(bound))
                            .color(ink)
                            .style(egui_plot::LineStyle::dashed_dense())
                            .width(1.),
                    );
                }
            }
        }
        for fit in fits {
            let ink = color(k);
            k += 1;
            let Some(r) = &fit.result else {
                continue;
            };
            if r.curve.is_empty() {
                continue;
            }
            let name = fit.title();
            plot_ui.line(
                egui_plot::Line::new(name.clone(), along(&r.curve))
                    .color(ink)
                    .width(1.5),
            );
            if fit.family.discrete() && shown.display == Display::Density {
                plot_ui.points(
                    egui_plot::Points::new(name.clone(), along(&r.curve))
                        .shape(egui_plot::MarkerShape::Circle)
                        .filled(true)
                        .radius(2.5)
                        .color(ink),
                );
            }
            for bound in [&r.lower, &r.upper] {
                if !bound.is_empty() {
                    plot_ui.line(
                        egui_plot::Line::new(name.clone(), along(bound))
                            .color(ink)
                            .style(egui_plot::LineStyle::dashed_dense())
                            .width(1.),
                    );
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Output UI

fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label(egui::RichText::new(text.into()).color(p.text_muted));
}

impl DistributionFitter {
    /// The fits compared: sortable by NLogL, AIC or BIC.
    fn table_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            muted(ui, "Sort by");
            for sort in Sort::ALL {
                if crate::tokens::toggle(ui, self.sort == sort, sort.label()).clicked() {
                    self.sort = sort;
                }
            }
        });
        ui.add_space(crate::tokens::XS);
        if self.fits.is_empty() {
            crate::tokens::helper(ui, "No fits yet.");
            return;
        }
        let mut chosen = None;
        egui::ScrollArea::horizontal()
            .id_salt("dfit_fits_scroll")
            .auto_shrink([false, true])
            .show_themed(ui, |ui| {
                egui::Grid::new("dfit_fits_table")
                    .num_columns(8)
                    .spacing(egui::vec2(crate::tokens::M, crate::tokens::XS))
                    .show(ui, |ui| {
                        for heading in [
                            "Fit",
                            "Distribution",
                            "Data set",
                            "NLogL",
                            "Params",
                            "AIC",
                            "BIC",
                            "Parameters",
                        ] {
                            muted(ui, heading);
                        }
                        ui.end_row();
                        for i in self.order() {
                            let fit = &self.fits[i];
                            if ui
                                .selectable_label(i == self.selected, fit.title())
                                .clicked()
                            {
                                chosen = Some(i);
                            }
                            ui.label(fit.family.label());
                            ui.label(
                                self.data_set(fit.data_set)
                                    .map(DataSet::title)
                                    .unwrap_or_default(),
                            );
                            match &fit.result {
                                Some(r) => {
                                    ui.label(short(r.nlogl));
                                    ui.label(short(r.parameters));
                                    ui.label(short(r.aic));
                                    ui.label(short(r.bic));
                                    ui.label(r.parameter_text());
                                }
                                None => {
                                    for _ in 0..5 {
                                        ui.label("—");
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(i) = chosen {
            self.select(i);
        }
    }

    /// Parameters with 95% intervals, moments and covariance.
    fn details_ui(&self, ui: &mut egui::Ui) {
        let Some(fit) = self.fit() else {
            crate::tokens::helper(ui, "No fit selected.");
            return;
        };
        let Some(r) = &fit.result else {
            crate::tokens::helper(ui, "Not fitted yet: the status line says why.");
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        egui::Grid::new("dfit_details")
            .num_columns(2)
            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
            .show(ui, |ui| {
                if let Some(h) = r.bandwidth {
                    muted(ui, "Kernel");
                    ui.label(fit.smoother.label());
                    ui.end_row();
                    muted(ui, "Bandwidth");
                    ui.label(short(h));
                    ui.end_row();
                }
                for (k, name) in r.names.iter().enumerate() {
                    muted(ui, name);
                    let value = short(r.values[k]);
                    let interval = if r.fixed.get(k).copied().unwrap_or(false) {
                        "fixed".to_owned()
                    } else {
                        let [lo, hi] = r.intervals[k];
                        format!("[{}, {}]", short(lo), short(hi))
                    };
                    ui.label(egui::RichText::new(format!("{value}  {interval}")).color(p.text));
                    ui.end_row();
                }
                muted(ui, "Mean");
                ui.label(short(r.mean));
                ui.end_row();
                muted(ui, "Variance");
                ui.label(short(r.variance));
                ui.end_row();
                muted(ui, "Observations");
                ui.label(short(r.observations));
                ui.end_row();
            });
        if !r.names.is_empty() {
            crate::tokens::helper(ui, "Intervals: 95% (paramci).");
        }
        let k = r.names.len();
        if k > 0 && r.covariance.len() == k * k {
            ui.add_space(crate::tokens::S);
            muted(ui, "Parameter covariance");
            egui::Grid::new("dfit_covariance")
                .num_columns(k + 1)
                .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                .show(ui, |ui| {
                    ui.label("");
                    for name in &r.names {
                        muted(ui, name);
                    }
                    ui.end_row();
                    for (i, name) in r.names.iter().enumerate() {
                        muted(ui, name);
                        for j in 0..k {
                            ui.label(short(r.covariance[i + j * k]));
                        }
                        ui.end_row();
                    }
                });
        }
    }

    /// x values (or probabilities) → pdf, cdf, icdf or survivor of the
    /// selected fit, with optional bounds.
    fn evaluate_ui(&mut self, ui: &mut egui::Ui) {
        field(ui, "Compute", |ui| {
            egui::ComboBox::from_id_salt("dfit_function")
                .width(ui.available_width())
                .selected_text(self.function.label())
                .show_ui(ui, |ui| {
                    for function in Function::ALL {
                        ui.selectable_value(&mut self.function, function, function.label());
                    }
                });
        });
        let what = if self.function == Function::Icdf {
            "At probabilities"
        } else {
            "At x values"
        };
        field(ui, what, |ui| {
            text_input(
                ui,
                &mut self.points,
                "for example 1 2.5 4",
                ui.available_width(),
            );
        });
        ui.horizontal(|ui| {
            ui.add_enabled(
                self.function.has_bounds(),
                egui::Checkbox::new(&mut self.point_bounds, "Bounds"),
            );
            ui.add_enabled(
                self.function.has_bounds() && self.point_bounds,
                egui::DragValue::new(&mut self.level)
                    .range(50.0..=99.9)
                    .speed(0.1)
                    .max_decimals(1)
                    .suffix(" %"),
            );
        });
        if let Err(error) = self.evaluate_points() {
            error_line(ui, &error);
            return;
        }
        let Some(fit) = self.fit() else {
            return;
        };
        let Some(r) = &fit.result else {
            return;
        };
        if r.evaluated.is_empty() {
            crate::tokens::helper(ui, "Type the points to evaluate the selected fit at.");
            return;
        }
        if self.point_bounds && fit.family == Family::Kernel {
            crate::tokens::helper(ui, "A kernel estimate has no parameter bounds.");
        }
        egui::Grid::new("dfit_evaluated")
            .num_columns(4)
            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
            .show(ui, |ui| {
                let first = if self.function == Function::Icdf {
                    "P"
                } else {
                    "x"
                };
                for heading in [first, "Value", "Lower", "Upper"] {
                    muted(ui, heading);
                }
                ui.end_row();
                for [at, value, lo, hi] in &r.evaluated {
                    for v in [at, value, lo, hi] {
                        ui.label(short(*v));
                    }
                    ui.end_row();
                }
            });
    }

    /// Which fits Export and Generate script include, and their names.
    fn export_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        heading(ui, "Export");
        if self.fits.is_empty() {
            crate::tokens::helper(ui, "No fits to export.");
            return;
        }
        let defaults = self.default_objects(ws);
        for fit in &mut self.fits {
            if !fit.object_edited
                && let Some(name) = defaults.get(&fit.id)
            {
                fit.object = name.clone();
            }
        }
        for fit in &mut self.fits {
            let title = fit.title();
            ui.checkbox(&mut fit.export, title);
            if fit.export {
                let before = fit.object.clone();
                name_field(ui, "Object name", &mut fit.object, ws);
                fit.object_edited |= fit.object != before;
            }
        }
        crate::tokens::helper(
            ui,
            "Unchecked fits stay in the app; the script fits and plots the checked ones.",
        );
    }
}
