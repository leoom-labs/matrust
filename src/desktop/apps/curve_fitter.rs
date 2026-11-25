//! Curve Fitter: fits a curve y(x) or a surface z(x, y) to workspace data
//! with `fit`. The app keeps a list of fits, each with its own inputs; the
//! selected fit's script runs privately and its probe reads back what the
//! view draws. Every number shown comes from engine builtins (`fit`,
//! `fittype`, `fitoptions`, `formula`, `coeffnames`, `coeffvalues`,
//! `confint`, `predint`).
use super::{
    App, AppKind, Outputs, Request, Runner, Snapshot, field, name_field, number, short, unused_name,
};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;

/// Model families of the Fit type dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Polynomial,
    Exponential,
    Fourier,
    Gaussian,
    Power,
    Rational,
    Sine,
    Weibull,
    Interpolant,
    SmoothingSpline,
    Custom,
}
impl Family {
    pub const ALL: [Self; 11] = [
        Self::Polynomial,
        Self::Exponential,
        Self::Fourier,
        Self::Gaussian,
        Self::Power,
        Self::Rational,
        Self::Sine,
        Self::Weibull,
        Self::Interpolant,
        Self::SmoothingSpline,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Polynomial => "Polynomial",
            Self::Exponential => "Exponential",
            Self::Fourier => "Fourier",
            Self::Gaussian => "Gaussian",
            Self::Power => "Power",
            Self::Rational => "Rational",
            Self::Sine => "Sum of sines",
            Self::Weibull => "Weibull",
            Self::Interpolant => "Interpolant",
            Self::SmoothingSpline => "Smoothing spline",
            Self::Custom => "Custom equation",
        }
    }
    /// The one number the family takes (curve degree or terms) and its range.
    pub fn terms(self) -> Option<(&'static str, RangeInclusive<u8>)> {
        match self {
            Self::Polynomial => Some(("Degree", 1..=9)),
            Self::Exponential | Self::Power => Some(("Terms", 1..=2)),
            Self::Fourier | Self::Gaussian | Self::Sine => Some(("Terms", 1..=8)),
            _ => None,
        }
    }
    /// Fitted by nonlinear least squares: StartPoint, Lower and Upper apply.
    pub fn nonlinear(self) -> bool {
        !matches!(
            self,
            Self::Polynomial | Self::Interpolant | Self::SmoothingSpline
        )
    }
    /// Fitted by least squares: Robust applies and coefficients have bounds.
    pub fn least_squares(self) -> bool {
        !matches!(self, Self::Interpolant | Self::SmoothingSpline)
    }
    /// Heading of the dropdown group this family starts.
    fn group(self) -> Option<&'static str> {
        match self {
            Self::Polynomial => Some("Library models"),
            Self::Interpolant => Some("Nonparametric"),
            Self::Custom => Some("Your model"),
            _ => None,
        }
    }
}

/// Interpolant methods, named as fit's library interpolants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interp {
    Linear,
    Nearest,
    Pchip,
    Cubic,
}
impl Interp {
    pub const ALL: [Self; 4] = [Self::Linear, Self::Nearest, Self::Pchip, Self::Cubic];
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Nearest => "Nearest neighbor",
            Self::Pchip => "Shape-preserving (PCHIP)",
            Self::Cubic => "Cubic spline",
        }
    }
    pub fn model(self) -> &'static str {
        match self {
            Self::Linear => "linearinterp",
            Self::Nearest => "nearestinterp",
            Self::Pchip => "pchipinterp",
            Self::Cubic => "cubicinterp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Robust {
    Off,
    Lar,
    Bisquare,
}
impl Robust {
    pub const ALL: [Self; 3] = [Self::Off, Self::Lar, Self::Bisquare];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Lar => "LAR",
            Self::Bisquare => "Bisquare",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Fit,
    Residuals,
}
impl View {
    pub const ALL: [Self; 2] = [Self::Fit, Self::Residuals];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit",
            Self::Residuals => "Residuals",
        }
    }
}

/// Smoothing slider: position t = -log10(1 - p), so p = 0 (a straight line)
/// sits at 0 and the useful values near 1 get most of the travel.
pub const SMOOTHING_SLIDER: RangeInclusive<f64> = 0.0..=8.0;
/// Most fits a session keeps.
pub const MAX_FITS: usize = 32;
/// Points of the fitted curve over the data range.
const CURVE_POINTS: usize = 400;
/// Grid points per side of the fitted surface image.
pub const GRID: usize = 41;

pub fn smoothing_position(p: f64) -> f64 {
    if p >= 1. {
        *SMOOTHING_SLIDER.end()
    } else {
        (-(1. - p).log10()).clamp(*SMOOTHING_SLIDER.start(), *SMOOTHING_SLIDER.end())
    }
}
/// The p of a slider position, with 1 - p kept to two significant digits so
/// the script reads 0.99 or 0.9985 rather than a long fraction.
pub fn smoothing_from_position(t: f64) -> f64 {
    let rest = 10_f64.powf(-t.clamp(*SMOOTHING_SLIDER.start(), *SMOOTHING_SLIDER.end()));
    let scale = 10_f64.powf(1. - rest.log10().floor());
    let rest = ((rest * scale).round() / scale).min(1.);
    let p = 1. - rest;
    // 1 - 0.012 is 0.988 within an ulp; print and read back the short form.
    number(p).parse().unwrap_or(p)
}

/// The inputs of one fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// Data are columns of a table variable rather than workspace vectors.
    pub use_table: bool,
    pub table: String,
    pub x: String,
    pub y: String,
    /// Empty: a curve y(x); else a surface z(x, y).
    pub z: String,
    /// Empty: no weights.
    pub weights: String,
    pub family: Family,
    /// Degree of a curve polynomial, or the number of terms.
    pub terms: u8,
    pub x_degree: u8,
    pub y_degree: u8,
    pub numerator: u8,
    pub denominator: u8,
    pub interp: Interp,
    pub smoothing: f64,
    pub default_smoothing: bool,
    pub equation: String,
    pub robust: Robust,
    pub normalize: bool,
    /// StartPoint, Lower and Upper text by coefficient name; blank = default.
    pub bounds: BTreeMap<String, [String; 3]>,
    /// Data points left out by clicking, 0-based.
    pub excluded: BTreeSet<usize>,
    /// Keep only x in [x_min, x_max]; blank = no limit.
    pub x_min: String,
    pub x_max: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            use_table: false,
            table: String::new(),
            x: String::new(),
            y: String::new(),
            z: String::new(),
            weights: String::new(),
            family: Family::Polynomial,
            terms: 1,
            x_degree: 1,
            y_degree: 1,
            numerator: 1,
            denominator: 1,
            interp: Interp::Linear,
            smoothing: 0.99,
            default_smoothing: true,
            equation: "a*exp(-b*x)+c".into(),
            robust: Robust::Off,
            normalize: false,
            bounds: BTreeMap::new(),
            excluded: BTreeSet::new(),
            x_min: String::new(),
            x_max: String::new(),
        }
    }
}

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
fn vector(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(|v| literal(*v)).collect();
    format!("[{}]", items.join(" "))
}
/// A typed number: blank is None; Inf and -Inf are allowed, NaN is not.
fn parse_number(text: &str, what: &str) -> Result<Option<f64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(v) if !v.is_nan() => Ok(Some(v)),
        _ => Err(format!("Curve Fitter: {what} must be a number")),
    }
}

impl Spec {
    pub fn surface(&self) -> bool {
        !self.z.is_empty()
    }
    /// The model as fittype names it: a library name or the custom equation.
    pub fn model(&self) -> String {
        match self.family {
            Family::Polynomial if self.surface() => {
                format!("poly{}{}", self.x_degree, self.y_degree)
            }
            Family::Polynomial => format!("poly{}", self.terms),
            Family::Exponential => format!("exp{}", self.terms),
            Family::Fourier => format!("fourier{}", self.terms),
            Family::Gaussian => format!("gauss{}", self.terms),
            Family::Power => format!("power{}", self.terms),
            Family::Rational => format!("rat{}{}", self.numerator, self.denominator),
            Family::Sine => format!("sin{}", self.terms),
            Family::Weibull => "weibull".into(),
            Family::Interpolant => self.interp.model().into(),
            Family::SmoothingSpline => "smoothingspline".into(),
            Family::Custom => self.equation.trim().to_owned(),
        }
    }
    /// fittype's argument: the model as a char vector literal.
    pub fn fittype_argument(&self) -> String {
        quote(&self.model())
    }
    /// Selects a family; its number restarts at the family's first value.
    pub fn set_family(&mut self, family: Family) {
        if self.family != family {
            self.family = family;
            if let Some((_, range)) = family.terms() {
                self.terms = *range.start();
            }
        }
    }
    /// How the script names a data variable: `x`, or `T.x` for a table column.
    pub fn data(&self, name: &str) -> String {
        if !self.use_table {
            name.to_owned()
        } else if matrust::files::valid_name(name) {
            format!("{}.{name}", self.table)
        } else {
            format!("{}.({})", self.table, quote(name))
        }
    }
    /// Workspace variables the script reads.
    pub fn variables(&self) -> Vec<String> {
        let names: Vec<&String> = if self.use_table {
            vec![&self.table]
        } else {
            vec![&self.x, &self.y, &self.z, &self.weights]
        };
        let mut out: Vec<String> = Vec::new();
        for name in names {
            if !name.is_empty() && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }
    /// The x limits of the Exclude range.
    pub fn x_range(&self) -> Result<(Option<f64>, Option<f64>), String> {
        let min = parse_number(&self.x_min, "the lower x limit")?;
        let max = parse_number(&self.x_max, "the upper x limit")?;
        if let (Some(a), Some(b)) = (min, max)
            && a > b
        {
            return Err("Curve Fitter: the lower x limit is above the upper one".into());
        }
        Ok((min, max))
    }
    /// Which data points the fit leaves out, from the clicked points and the
    /// x range (`x` is the first independent variable, in data order).
    pub fn excluded_mask(&self, x: &[f64]) -> Vec<bool> {
        let (min, max) = self.x_range().unwrap_or((None, None));
        x.iter()
            .enumerate()
            .map(|(i, v)| {
                self.excluded.contains(&i)
                    || min.is_some_and(|m| *v < m)
                    || max.is_some_and(|m| *v > m)
            })
            .collect()
    }
    /// Names and lengths that fix what a point index means: a change
    /// forgets the clicked points.
    fn data_identity(&self, ws: &Snapshot) -> String {
        let length = ws
            .get(if self.use_table { &self.table } else { &self.x })
            .map(|a| if self.use_table { a.rows() } else { a.len() });
        format!(
            "{}|{}|{}|{}|{}|{length:?}",
            self.use_table, self.table, self.x, self.y, self.z
        )
    }
}

/// What the coefficient query learnt about a model.
#[derive(Clone, Debug, PartialEq)]
pub struct CoefInfo {
    pub names: Vec<String>,
    /// fitoptions' default bounds; empty means unbounded.
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub nonlinear: bool,
}

/// One coefficient with its 95% confidence bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct Coefficient {
    pub name: String,
    pub value: f64,
    pub lower: f64,
    pub upper: f64,
}

/// The fitted surface sampled on a GRID x GRID mesh, rows from the lowest y.
#[derive(Clone, Debug)]
pub struct Grid {
    pub x: (f64, f64),
    pub y: (f64, f64),
    pub values: Vec<f64>,
}

/// One fit as the private run computed it.
#[derive(Clone, Debug)]
pub struct FitResult {
    pub type_name: String,
    pub formula: String,
    pub coefficients: Vec<Coefficient>,
    pub sse: f64,
    pub rsquare: f64,
    pub adjrsquare: f64,
    pub rmse: f64,
    pub dfe: f64,
    /// The smoothing spline's p.
    pub smoothing: Option<f64>,
    /// The engine's last warning of the fit.
    pub warning: Option<String>,
    /// Data in data order: x, then y; z for a surface.
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Option<Vec<f64>>,
    /// The fit at the data points and the response minus it.
    pub fitted: Vec<f64>,
    pub residuals: Vec<f64>,
    /// The fitted curve over the data range.
    pub curve: Vec<[f64; 2]>,
    /// Prediction bounds over the same points.
    pub lower: Vec<[f64; 2]>,
    pub upper: Vec<[f64; 2]>,
    pub grid: Option<Grid>,
}

fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Curve Fitter: {name} is not a real array")),
    }
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("Curve Fitter: {name} is empty"))
}
fn text(outputs: &Outputs, name: &str) -> Result<String, String> {
    match outputs.get(name) {
        Some(Array::Character(c)) => Ok(String::from_utf16_lossy(c.units())),
        Some(Array::Numeric(m)) if m.data().is_empty() => Ok(String::new()),
        _ => Err(format!("Curve Fitter: {name} is not text")),
    }
}
fn finite_pairs(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    x.iter()
        .zip(y)
        .map(|(x, y)| [*x, *y])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}
fn finite_range(values: &[f64]) -> (f64, f64) {
    let (lo, hi) = values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    if lo > hi {
        (0., 1.)
    } else if lo == hi {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    }
}

impl FitResult {
    pub fn from_outputs(outputs: &Outputs) -> Result<Self, String> {
        let surface = outputs.contains_key("app_z");
        let names: Vec<String> = text(outputs, "app_names")?
            .split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect();
        let c = values(outputs, "app_c")?;
        let ci = values(outputs, "app_ci")?;
        let coefficients = if !c.is_empty() && c.len() == names.len() && ci.len() == 2 * c.len() {
            names
                .iter()
                .enumerate()
                .map(|(k, name)| Coefficient {
                    name: name.clone(),
                    value: c[k],
                    lower: ci[2 * k],
                    upper: ci[2 * k + 1],
                })
                .collect()
        } else {
            Vec::new()
        };
        let warning = text(outputs, "app_warn")?;
        let x = values(outputs, "app_x")?;
        let y = values(outputs, "app_y")?;
        let z = surface.then(|| values(outputs, "app_z")).transpose()?;
        let (curve, lower, upper, grid) = if surface {
            let gz = values(outputs, "app_GZ")?;
            if gz.len() != GRID * GRID {
                return Err("Curve Fitter: the surface grid has the wrong size".into());
            }
            // Column-major meshgrid: rows follow y, columns follow x.
            let values = (0..GRID * GRID)
                .map(|k| gz[(k / GRID) + (k % GRID) * GRID])
                .collect();
            let grid = Grid {
                x: finite_range(&x),
                y: finite_range(&y),
                values,
            };
            (Vec::new(), Vec::new(), Vec::new(), Some(grid))
        } else {
            let xq = values(outputs, "app_xq")?;
            let bounds = values(outputs, "app_pi")?;
            let (lower, upper) = if bounds.len() == 2 * xq.len() {
                let (lo, hi) = bounds.split_at(xq.len());
                (finite_pairs(&xq, lo), finite_pairs(&xq, hi))
            } else {
                (Vec::new(), Vec::new())
            };
            (
                finite_pairs(&xq, &values(outputs, "app_yq")?),
                lower,
                upper,
                None,
            )
        };
        let smoothing = values(outputs, "app_p")?.first().copied();
        Ok(Self {
            type_name: text(outputs, "app_type")?,
            formula: text(outputs, "app_formula")?,
            coefficients,
            sse: scalar(outputs, "app_sse")?,
            rsquare: scalar(outputs, "app_r2")?,
            adjrsquare: scalar(outputs, "app_adj")?,
            rmse: scalar(outputs, "app_rmse")?,
            dfe: scalar(outputs, "app_dfe")?,
            smoothing,
            warning: (!warning.is_empty()).then_some(warning),
            fitted: values(outputs, "app_fx")?,
            residuals: values(outputs, "app_res")?,
            x,
            y,
            z,
            curve,
            lower,
            upper,
            grid,
        })
    }
    /// The points the fit view draws: (x, y) of a curve, (x, y) of a surface.
    pub fn points(&self) -> Vec<[f64; 2]> {
        self.x.iter().zip(&self.y).map(|(x, y)| [*x, *y]).collect()
    }
}

const CURVE_PROBE: [&str; 18] = [
    "app_warn",
    "app_x",
    "app_y",
    "app_type",
    "app_formula",
    "app_names",
    "app_c",
    "app_ci",
    "app_sse",
    "app_r2",
    "app_adj",
    "app_rmse",
    "app_dfe",
    "app_p",
    "app_fx",
    "app_res",
    "app_xq",
    "app_yq",
];
const SURFACE_PROBE: [&str; 2] = ["app_z", "app_GZ"];

/// One fit of the fits list.
#[derive(Clone, Debug)]
pub struct FitEntry {
    pub name: String,
    /// The inputs as edited.
    pub spec: Spec,
    /// The inputs as of the last Fit click (or the last frame of auto-fit):
    /// what the script uses while auto-fit is off.
    pub applied: Spec,
    pub fit_name: String,
    pub fit_edited: bool,
    pub gof_name: String,
    pub gof_edited: bool,
    /// Last good result; errors keep it on screen.
    pub result: Option<FitResult>,
    data_identity: Option<String>,
}
impl FitEntry {
    fn new(name: String, spec: Spec) -> Self {
        Self {
            name,
            applied: spec.clone(),
            spec,
            fit_name: String::new(),
            fit_edited: false,
            gof_name: String::new(),
            gof_edited: false,
            result: None,
            data_identity: None,
        }
    }
}

/// Coefficient names and default bounds per model, read with
/// `coeffnames(fittype(model))` and `fitoptions` in a private run.
#[derive(Default)]
struct Coefficients {
    runner: Runner,
    cache: BTreeMap<String, Result<CoefInfo, String>>,
}
const COEFFICIENT_CACHE: usize = 64;

/// The pieces the script and the probe share.
struct Call {
    comment: String,
    ft: String,
    opts: Option<String>,
    /// Option assignments, each after `opts.`.
    options: Vec<String>,
    /// fit's first argument: `x`, or `[x, y]` for a surface.
    predictors: String,
    response: String,
    /// Data expressions read back by the probe: x, y (and z).
    columns: Vec<String>,
    fit: String,
    gof: String,
}
impl Call {
    fn arguments(&self) -> String {
        let mut arguments = format!("{}, {}, {}", self.predictors, self.response, self.ft);
        if let Some(opts) = &self.opts {
            arguments.push_str(", ");
            arguments.push_str(opts);
        }
        arguments
    }
}

pub struct CurveFitter {
    pub fits: Vec<FitEntry>,
    pub selected: usize,
    /// Fit on every change; off, the Fit button applies the inputs.
    pub auto: bool,
    pub view: View,
    pub show_bounds: bool,
    /// Prediction bound level, percent.
    pub level: f64,
    /// Functional (curve) rather than observation bounds.
    pub functional: bool,
    next: usize,
    coefficients: Coefficients,
}
impl Default for CurveFitter {
    fn default() -> Self {
        Self {
            fits: vec![FitEntry::new("Fit 1".into(), Spec::default())],
            selected: 0,
            auto: true,
            view: View::Fit,
            show_bounds: false,
            level: 95.,
            functional: false,
            next: 2,
            coefficients: Coefficients::default(),
        }
    }
}

impl CurveFitter {
    pub fn fit(&self) -> &FitEntry {
        &self.fits[self.selected]
    }
    pub fn fit_mut(&mut self) -> &mut FitEntry {
        &mut self.fits[self.selected]
    }
    /// The inputs the script uses: as edited while auto-fit is on, else as
    /// of the last Fit click.
    pub fn active(&self) -> &Spec {
        let entry = self.fit();
        if self.auto {
            &entry.spec
        } else {
            &entry.applied
        }
    }
    /// New fit: the selected fit's data with a default model.
    pub fn new_fit(&mut self) {
        if self.fits.len() >= MAX_FITS {
            return;
        }
        let data = &self.fit().spec;
        let spec = Spec {
            use_table: data.use_table,
            table: data.table.clone(),
            x: data.x.clone(),
            y: data.y.clone(),
            z: data.z.clone(),
            weights: data.weights.clone(),
            ..Spec::default()
        };
        let name = self.next_name();
        self.fits.push(FitEntry::new(name, spec));
        self.selected = self.fits.len() - 1;
    }
    /// Duplicate: the selected fit's inputs and result under a new name.
    pub fn duplicate(&mut self) {
        if self.fits.len() >= MAX_FITS {
            return;
        }
        let mut copy = self.fit().clone();
        copy.name = self.next_name();
        copy.fit_edited = false;
        copy.gof_edited = false;
        self.fits.push(copy);
        self.selected = self.fits.len() - 1;
    }
    /// Delete the selected fit; the last one stays.
    pub fn delete(&mut self) {
        if self.fits.len() > 1 {
            self.fits.remove(self.selected);
            self.selected = self.selected.min(self.fits.len() - 1);
        }
    }
    pub fn select(&mut self, index: usize) {
        if index < self.fits.len() {
            self.selected = index;
        }
    }
    fn next_name(&mut self) -> String {
        let name = format!("Fit {}", self.next);
        self.next += 1;
        name
    }
    /// Click on a data point: excluded points come back, others leave.
    pub fn toggle_point(&mut self, index: usize) {
        let excluded = &mut self.fit_mut().spec.excluded;
        if !excluded.remove(&index) {
            excluded.insert(index);
        }
    }
    /// Fit button: the edited inputs become the fitted ones.
    pub fn apply(&mut self) {
        let entry = self.fit_mut();
        entry.applied = entry.spec.clone();
    }

    /// Default output names: `fittedmodel` and `goodness`, else numbered.
    fn default_names(&self, ws: &Snapshot) -> (String, String) {
        let entry = self.fit();
        let data = self.active().variables();
        let fit = unused_name("fittedmodel", |n| ws.contains(n));
        let fit_used = if entry.fit_edited {
            entry.fit_name.trim().to_owned()
        } else {
            fit.clone()
        };
        let gof = unused_name("goodness", |n| {
            ws.contains(n) || n == fit_used || data.iter().any(|d| d == n)
        });
        (fit, gof)
    }
    /// The fit object's and goodness struct's names, checked.
    pub fn output_names(&self, ws: &Snapshot) -> Result<(String, String), String> {
        let entry = self.fit();
        let (fit_default, gof_default) = self.default_names(ws);
        let fit = if entry.fit_edited {
            entry.fit_name.trim().to_owned()
        } else {
            fit_default
        };
        let gof = if entry.gof_edited {
            entry.gof_name.trim().to_owned()
        } else {
            gof_default
        };
        for (name, what) in [(&fit, "fit"), (&gof, "goodness")] {
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Curve Fitter: the {what} name is not a valid variable name"
                ));
            }
            if self.active().variables().contains(name) {
                return Err(format!(
                    "Curve Fitter: the {what} name {name} is a data variable of this fit"
                ));
            }
        }
        if fit == gof {
            return Err("Curve Fitter: the fit and goodness need different names".into());
        }
        Ok((fit, gof))
    }

    /// The coefficient query of the selected fit's edited model, if it has bounds.
    pub fn coefficient_request(&self) -> Option<(String, Request)> {
        let spec = &self.fit().spec;
        if !spec.family.nonlinear() || spec.model().is_empty() {
            return None;
        }
        let model = spec.fittype_argument();
        if self.coefficients.cache.contains_key(&model) {
            return None;
        }
        let code = format!(
            "app_ft = fittype({model});\n\
             app_names = strjoin(coeffnames(app_ft)', ',');\n\
             app_o = fitoptions(app_ft);\n\
             app_nl = double(strcmp(app_o.Method, 'NonlinearLeastSquares'));\n\
             app_lo = [];\n\
             app_up = [];\n\
             if app_nl\n\
             app_lo = app_o.Lower;\n\
             app_up = app_o.Upper;\n\
             end\n"
        );
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        let request = Request {
            key: hasher.finish(),
            inputs: Vec::new(),
            code,
            outputs: ["app_names", "app_nl", "app_lo", "app_up"]
                .map(str::to_owned)
                .to_vec(),
        };
        Some((model, request))
    }
    /// Stores a finished coefficient query.
    pub fn receive_coefficients(&mut self, model: String, result: &Result<Outputs, String>) {
        let info = result.clone().and_then(|outputs| {
            Ok(CoefInfo {
                names: text(&outputs, "app_names")?
                    .split(',')
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                    .collect(),
                lower: values(&outputs, "app_lo")?,
                upper: values(&outputs, "app_up")?,
                nonlinear: scalar(&outputs, "app_nl")? != 0.,
            })
        });
        if self.coefficients.cache.len() >= COEFFICIENT_CACHE {
            self.coefficients.cache.clear();
        }
        self.coefficients.cache.insert(model, info);
    }
    /// Asks for the edited model's coefficients off the UI thread.
    fn query_coefficients(&mut self, ctx: &egui::Context) {
        if let Some((model, request)) = self.coefficient_request() {
            self.coefficients.runner.submit(ctx, request);
            if let Some(result) = self.coefficients.runner.result() {
                let result = result.clone();
                self.receive_coefficients(model, &result);
            }
        }
    }
    /// What the coefficient query said about a model, once it finished.
    pub fn coefficient_info(&self, spec: &Spec) -> Option<&Result<CoefInfo, String>> {
        self.coefficients.cache.get(&spec.fittype_argument())
    }
    /// Stops the coefficient query (the app closed).
    pub fn stop(&mut self) {
        self.coefficients.runner.stop();
    }

    /// StartPoint, Lower and Upper assignments of a nonlinear model.
    fn bound_options(&self, spec: &Spec) -> Result<Vec<String>, String> {
        let filled = spec
            .bounds
            .values()
            .any(|b| b.iter().any(|t| !t.trim().is_empty()));
        if !spec.family.nonlinear() || !filled {
            return Ok(Vec::new());
        }
        let info = match self.coefficient_info(spec) {
            Some(Ok(info)) => info,
            // The fit reports the model's own error.
            Some(Err(_)) => return Ok(Vec::new()),
            None => return Err("Curve Fitter: reading the model's coefficients…".into()),
        };
        let cell =
            |name: &String, k: usize| spec.bounds.get(name).map_or("", |b| b[k].trim()).to_owned();
        let mut lines = Vec::new();
        let starts: Vec<String> = info.names.iter().map(|n| cell(n, 0)).collect();
        if starts.iter().any(|s| !s.is_empty()) {
            if starts.iter().any(String::is_empty) {
                return Err(
                    "Curve Fitter: give a start value for every coefficient, or for none".into(),
                );
            }
            let start = starts
                .iter()
                .zip(&info.names)
                .map(|(s, name)| {
                    parse_number(s, &format!("the start value of {name}"))
                        .map(|v| v.unwrap_or(0.))
                        .and_then(|v| {
                            if v.is_finite() {
                                Ok(v)
                            } else {
                                Err(format!(
                                    "Curve Fitter: the start value of {name} must be finite"
                                ))
                            }
                        })
                })
                .collect::<Result<Vec<f64>, String>>()?;
            lines.push(format!("StartPoint = {}", vector(&start)));
        }
        for (k, field, defaults, fill, what) in [
            (1, "Lower", &info.lower, f64::NEG_INFINITY, "lower bound"),
            (2, "Upper", &info.upper, f64::INFINITY, "upper bound"),
        ] {
            if info.names.iter().all(|n| cell(n, k).is_empty()) {
                continue;
            }
            let bound = info
                .names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    parse_number(&cell(name, k), &format!("the {what} of {name}"))
                        .map(|v| v.unwrap_or_else(|| defaults.get(i).copied().unwrap_or(fill)))
                })
                .collect::<Result<Vec<f64>, String>>()?;
            lines.push(format!("{field} = {}", vector(&bound)));
        }
        Ok(lines)
    }
    /// The non-default fit options of `spec`, as `opts.` assignments.
    fn options(&self, spec: &Spec, x: &str) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        if spec.normalize {
            lines.push("Normalize = 'on'".to_owned());
        }
        if spec.family.least_squares() && spec.robust != Robust::Off {
            lines.push(format!("Robust = '{}'", spec.robust.label()));
        }
        if spec.family == Family::SmoothingSpline && !spec.default_smoothing {
            if !(0. ..=1.).contains(&spec.smoothing) {
                return Err("Curve Fitter: the smoothing parameter is from 0 to 1".into());
            }
            lines.push(format!("SmoothingParam = {}", number(spec.smoothing)));
        }
        lines.extend(self.bound_options(spec)?);
        let range = match spec.x_range()? {
            (Some(a), Some(b)) => Some(format!("{x} < {} | {x} > {}", literal(a), literal(b))),
            (Some(a), None) => Some(format!("{x} < {}", literal(a))),
            (None, Some(b)) => Some(format!("{x} > {}", literal(b))),
            (None, None) => None,
        };
        let points: Vec<f64> = spec.excluded.iter().map(|i| (*i + 1) as f64).collect();
        match (range, points.is_empty()) {
            (None, true) => {}
            (None, false) => lines.push(format!("Exclude = {}", vector(&points))),
            (Some(range), true) => lines.push(format!("Exclude = {range}")),
            (Some(range), false) => {
                lines.push(format!("Exclude = {range}"));
                lines.push(format!("Exclude({}) = true", vector(&points)));
            }
        }
        if !spec.weights.is_empty() {
            lines.push(format!("Weights = {}", spec.data(&spec.weights)));
        }
        Ok(lines)
    }
    /// Everything the selected fit's script and probe need, checked.
    fn call(&self, ws: &Snapshot) -> Result<Call, String> {
        let spec = self.active();
        if spec.use_table {
            if spec.table.is_empty() {
                return Err("Curve Fitter: choose a table in INPUTS".into());
            }
            if !ws.contains(&spec.table) {
                return Err(format!(
                    "Curve Fitter: workspace variable {} does not exist",
                    spec.table
                ));
            }
        }
        if spec.x.is_empty() || spec.y.is_empty() {
            return Err("Curve Fitter: choose X and Y data in INPUTS".into());
        }
        if spec.surface() && spec.family != Family::Polynomial {
            return Err("Curve Fitter: surfaces fit polynomial models only".into());
        }
        if spec.model().is_empty() {
            return Err("Curve Fitter: type a custom equation, for example a*exp(-b*x)+c".into());
        }
        let (fit, gof) = self.output_names(ws)?;
        let taken = |n: &str| ws.contains(n) || n == fit || n == gof;
        let ft = unused_name("ft", taken);
        let x = spec.data(&spec.x);
        let y = spec.data(&spec.y);
        let options = self.options(spec, &x)?;
        let opts = (!options.is_empty()).then(|| unused_name("opts", |n| taken(n) || n == ft));
        // A surface's predictors are two columns side by side.
        let column = |name: &str| {
            let expr = spec.data(name);
            if spec.use_table || ws.get(name).is_some_and(|a| a.cols() == 1) {
                expr
            } else {
                format!("{expr}(:)")
            }
        };
        let (predictors, response, columns, against) = if spec.surface() {
            let z = spec.data(&spec.z);
            (
                format!("[{}, {}]", column(&spec.x), column(&spec.y)),
                column(&spec.z),
                vec![x.clone(), y.clone(), z.clone()],
                format!("{z} against {x} and {y}"),
            )
        } else {
            // fit wants a predictor column and a response column.
            (
                column(&spec.x),
                column(&spec.y),
                vec![x.clone(), y.clone()],
                format!("{y} against {x}"),
            )
        };
        let weighted = if spec.weights.is_empty() {
            String::new()
        } else {
            format!(", weighted by {}", spec.data(&spec.weights))
        };
        Ok(Call {
            comment: format!("% {}: {against}{weighted}", self.fit().name),
            ft,
            opts,
            options,
            predictors,
            response,
            columns,
            fit,
            gof,
        })
    }
}

impl App for CurveFitter {
    fn kind(&self) -> AppKind {
        AppKind::CurveFitter
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let call = self.call(ws)?;
        let spec = self.active();
        let mut script = format!(
            "{}\n{} = fittype({});\n",
            call.comment,
            call.ft,
            spec.fittype_argument()
        );
        if let Some(opts) = &call.opts {
            script.push_str(&format!("{opts} = fitoptions({});\n", call.ft));
            for line in &call.options {
                script.push_str(&format!("{opts}.{line};\n"));
            }
        }
        script.push_str(&format!(
            "[{}, {}] = fit({});\nplot({}, {}, {})\n",
            call.fit,
            call.gof,
            call.arguments(),
            call.fit,
            call.predictors,
            call.response
        ));
        Ok(script)
    }
    fn inputs(&self) -> Vec<String> {
        self.active().variables()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(call) = self.call(ws) else {
            return (String::new(), Vec::new());
        };
        let spec = self.active();
        let surface = spec.surface();
        // Private copies first, so no probe name can shadow the user's names;
        // the warning first, before any probe line can warn.
        let mut code = format!(
            "[app_f, app_g] = deal({}, {});\napp_warn = lastwarn;\n",
            call.fit, call.gof
        );
        for (var, expr) in ["app_x", "app_y", "app_z"].iter().zip(&call.columns) {
            code.push_str(&format!("{var} = double({expr}); {var} = {var}(:);\n"));
        }
        code.push_str(
            "app_type = type(app_f);\n\
             app_formula = formula(app_f);\n\
             app_names = strjoin(coeffnames(app_f)', ',');\n",
        );
        code.push_str(if spec.family.least_squares() {
            "app_c = coeffvalues(app_f);\napp_ci = confint(app_f);\n"
        } else {
            "app_c = [];\napp_ci = [];\n"
        });
        code.push_str(
            "app_sse = app_g.sse; app_r2 = app_g.rsquare; app_adj = app_g.adjrsquare;\n\
             app_rmse = app_g.rmse; app_dfe = app_g.dfe;\n",
        );
        if spec.family == Family::SmoothingSpline {
            // The smoothing parameter fit chose is in its third output.
            code.push_str(&format!(
                "[~, ~, app_out] = fit({});\napp_p = app_out.p;\n",
                call.arguments()
            ));
        } else {
            code.push_str("app_p = [];\n");
        }
        let mut names: Vec<String> = CURVE_PROBE.iter().map(|s| (*s).to_owned()).collect();
        if surface {
            names.retain(|n| n != "app_xq" && n != "app_yq");
            names.extend(SURFACE_PROBE.iter().map(|s| (*s).to_owned()));
            code.push_str(&format!(
                "app_fx = app_f(app_x, app_y);\napp_res = app_z - app_fx;\n\
                 [app_GX, app_GY] = meshgrid(linspace(min(app_x), max(app_x), {GRID}), \
                 linspace(min(app_y), max(app_y), {GRID}));\n\
                 app_GZ = app_f(app_GX, app_GY);\n"
            ));
        } else {
            names.push("app_pi".into());
            code.push_str(&format!(
                "app_fx = app_f(app_x);\napp_res = app_y - app_fx;\n\
                 app_xq = linspace(min(app_x), max(app_x), {CURVE_POINTS})';\n\
                 app_yq = app_f(app_xq);\n"
            ));
            if self.show_bounds {
                code.push_str(&format!(
                    "try\napp_pi = predint(app_f, app_xq, {}, '{}', 'off');\ncatch\napp_pi = [];\nend\n",
                    number(self.level / 100.),
                    if self.functional {
                        "functional"
                    } else {
                        "observation"
                    }
                ));
            } else {
                code.push_str("app_pi = [];\n");
            }
        }
        (code, names)
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let auto = self.auto;
        let entry = self.fit_mut();
        if entry.spec.surface() && entry.spec.family != Family::Polynomial {
            entry.spec.set_family(Family::Polynomial);
        }
        let identity = entry.spec.data_identity(ws);
        if entry.data_identity.as_ref() != Some(&identity) {
            if entry.data_identity.is_some() {
                entry.spec.excluded.clear();
            }
            entry.data_identity = Some(identity);
        }
        if auto && entry.applied != entry.spec {
            entry.applied = entry.spec.clone();
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let result = FitResult::from_outputs(outputs)?;
        self.fit_mut().result = Some(result);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.query_coefficients(ui.ctx());
        let info = self.coefficient_info(&self.fit().spec).cloned();
        let default_p = self.fit().result.as_ref().and_then(|r| r.smoothing);
        let spec = &mut self.fits[self.selected].spec;
        data_ui(ui, spec, ws);
        section(ui);
        model_ui(ui, spec, info.as_ref(), default_p);
        section(ui);
        options_ui(ui, spec, info.as_ref());
        section(ui);
        exclude_ui(ui, spec);
        section(ui);
        ui.checkbox(&mut self.auto, "Fit on every change");
        if !self.auto {
            ui.add_space(crate::tokens::XS);
            let pending = self.fit().applied != self.fit().spec;
            if ui
                .add(
                    egui::Button::new("Fit")
                        .min_size(egui::vec2(ui.available_width(), crate::tokens::CONTROL)),
                )
                .clicked()
            {
                self.apply();
            }
            if pending {
                crate::tokens::helper(ui, "The inputs changed since the last fit.");
            }
        }
    }
    fn main_caption(&self) -> &'static str {
        "FITS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.fits_ui(ui);
        ui.add_space(crate::tokens::S);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                let tab = crate::tokens::tab(ui, self.view == view, view.label());
                if view == View::Fit {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::CurveFitter),
                    );
                }
                if tab.clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let surface = self.fit().spec.surface();
        if self.view == View::Fit && !surface {
            self.bounds_ui(ui);
            ui.add_space(crate::tokens::XS);
        }
        let entry = &self.fits[self.selected];
        let Some(result) = &entry.result else {
            crate::tokens::helper(
                ui,
                "No fit yet: choose the data in INPUTS. The status line says what is missing.",
            );
            return;
        };
        let spec = &entry.spec;
        let mask = spec.excluded_mask(&result.x);
        // Axis labels: the variable or column names.
        let names = (spec.x.clone(), spec.y.clone());
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        let clicked = match (self.view, &result.grid) {
            (View::Fit, Some(grid)) => {
                surface_plot(ui, height, (&names.0, &names.1), result, grid, &mask)
            }
            (View::Fit, None) => {
                let shown = self.show_bounds && !result.lower.is_empty();
                if self.show_bounds && result.lower.is_empty() {
                    crate::tokens::helper(
                        ui,
                        "Interpolants and smoothing splines have no prediction bounds.",
                    );
                }
                curve_plot(ui, height, (&names.0, &names.1), result, &mask, shown)
            }
            (View::Residuals, _) => {
                // A surface's residuals are drawn against the fitted value.
                let surface = result.z.is_some();
                let x_label = if surface { "Fitted value" } else { &names.0 };
                residual_plot(ui, height, x_label, result, &mask, surface)
            }
        };
        if let Some(index) = clicked {
            self.toggle_point(index);
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let entry = self.fit();
        let Some(r) = &entry.result else {
            return ["Fit", "SSE", "R-square", "Adj R-square", "RMSE"]
                .into_iter()
                .map(|label| {
                    (
                        label,
                        if label == "Fit" {
                            entry.name.clone()
                        } else {
                            "—".to_owned()
                        },
                    )
                })
                .collect();
        };
        let mut rows = vec![
            ("Fit", entry.name.clone()),
            ("Type", r.type_name.clone()),
            ("SSE", short(r.sse)),
            ("R-square", short(r.rsquare)),
            ("Adj R-square", short(r.adjrsquare)),
            ("RMSE", short(r.rmse)),
            ("DFE", short(r.dfe)),
        ];
        if let Some(p) = r.smoothing {
            rows.push(("Smoothing p", short(p)));
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        if let Some(r) = &self.fit().result {
            ui.label(egui::RichText::new("Formula").color(p.text_muted));
            ui.add(egui::Label::new(egui::RichText::new(&r.formula).color(p.text)).wrap());
            if !r.coefficients.is_empty() {
                ui.add_space(crate::tokens::S);
                ui.label(
                    egui::RichText::new("Coefficients (95% confidence bounds)").color(p.text_muted),
                );
                egui::Grid::new("curve_fitter_coefficients")
                    .num_columns(2)
                    .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                    .show(ui, |ui| {
                        for c in &r.coefficients {
                            ui.label(egui::RichText::new(&c.name).color(p.text_muted));
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}  ({}, {})",
                                    short(c.value),
                                    short(c.lower),
                                    short(c.upper)
                                ))
                                .color(p.text),
                            );
                            ui.end_row();
                        }
                    });
            }
            if let Some(warning) = &r.warning {
                ui.add_space(crate::tokens::S);
                ui.add(egui::Label::new(egui::RichText::new(warning).color(p.warning)).wrap());
            }
            ui.add_space(crate::tokens::M);
        }
        let (fit_default, gof_default) = self.default_names(ws);
        let entry = self.fit_mut();
        if !entry.fit_edited {
            entry.fit_name = fit_default;
        }
        let before = entry.fit_name.clone();
        name_field(ui, "Fit object name", &mut entry.fit_name, ws);
        entry.fit_edited |= entry.fit_name != before;
        if !entry.gof_edited {
            entry.gof_name = gof_default;
        }
        let before = entry.gof_name.clone();
        name_field(ui, "Goodness name", &mut entry.gof_name, ws);
        entry.gof_edited |= entry.gof_name != before;
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        match self.call(ws) {
            Ok(call) => [Some(call.fit), Some(call.gof), Some(call.ft), call.opts]
                .into_iter()
                .flatten()
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        let entry = self.fit();
        let r = entry.result.as_ref()?;
        let mut text = format!(
            "{}: {} · R² {} · RMSE {}",
            entry.name,
            r.type_name,
            short(r.rsquare),
            short(r.rmse)
        );
        if let Some(warning) = &r.warning {
            text.push_str(" · Warning: ");
            text.push_str(warning);
        }
        Some(text)
    }
}

// ---------------------------------------------------------------------------
// Inputs

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}

/// Real numeric vectors of at least two values: name and length.
pub fn vectors(ws: &Snapshot) -> Vec<(String, usize)> {
    ws.variables()
        .filter_map(|(name, array)| vector_length(array).map(|n| (name.to_owned(), n)))
        .collect()
}
fn vector_length(array: &Array) -> Option<usize> {
    let real = matches!(array, Array::Numeric(_) | Array::Typed(_)) && !array.is_complex();
    let dims = array.dims();
    (real && dims.len() == 2 && (dims[0] == 1 || dims[1] == 1) && array.len() >= 2)
        .then(|| array.len())
}
/// Table variables: name and height.
pub fn tables(ws: &Snapshot) -> Vec<(String, usize)> {
    ws.variables()
        .filter_map(|(name, array)| match array {
            Array::Table(t) => Some((name.to_owned(), t.height())),
            _ => None,
        })
        .collect()
}
/// A table's numeric vector columns: name and height.
pub fn table_columns(ws: &Snapshot, table: &str) -> Vec<(String, usize)> {
    match ws.get(table) {
        Some(Array::Table(t)) => t
            .names()
            .iter()
            .zip(t.columns())
            .filter(|(_, column)| column.cols() == 1)
            .filter_map(|(name, column)| vector_length(column).map(|n| (name.clone(), n)))
            .collect(),
        _ => Vec::new(),
    }
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

fn data_ui(ui: &mut egui::Ui, spec: &mut Spec, ws: &Snapshot) {
    field(ui, "Data", |ui| {
        ui.horizontal(|ui| {
            let mut use_table = spec.use_table;
            if crate::tokens::toggle(ui, !use_table, "Vectors").clicked() {
                use_table = false;
            }
            if crate::tokens::toggle(ui, use_table, "Table").clicked() {
                use_table = true;
            }
            if use_table != spec.use_table {
                spec.use_table = use_table;
                spec.table.clear();
                spec.x.clear();
                spec.y.clear();
                spec.z.clear();
                spec.weights.clear();
            }
        });
    });
    let options = if spec.use_table {
        let tables = tables(ws);
        let before = spec.table.clone();
        picker(
            ui,
            "curve_fitter_table",
            "Table",
            &mut spec.table,
            &tables,
            None,
        );
        if spec.table != before {
            spec.x.clear();
            spec.y.clear();
            spec.z.clear();
            spec.weights.clear();
        }
        if tables.is_empty() {
            crate::tokens::helper(ui, "No table variables in the workspace");
        }
        table_columns(ws, &spec.table)
    } else {
        vectors(ws)
    };
    let before = spec.x.clone();
    picker(ui, "curve_fitter_x", "X data", &mut spec.x, &options, None);
    let length = options
        .iter()
        .find(|(name, _)| *name == spec.x)
        .map(|(_, n)| *n);
    let matching: Vec<(String, usize)> = options
        .iter()
        .filter(|(_, n)| length.is_none_or(|l| *n == l))
        .cloned()
        .collect();
    if spec.x != before {
        // A new X keeps only the other inputs of its length.
        for name in [&mut spec.y, &mut spec.z, &mut spec.weights] {
            if !matching.iter().any(|(m, _)| m == name) {
                name.clear();
            }
        }
    }
    picker(ui, "curve_fitter_y", "Y data", &mut spec.y, &matching, None);
    picker(
        ui,
        "curve_fitter_z",
        "Z data",
        &mut spec.z,
        &matching,
        Some("None: fit a curve"),
    );
    picker(
        ui,
        "curve_fitter_weights",
        "Weights",
        &mut spec.weights,
        &matching,
        Some("None"),
    );
    if options.is_empty() && (!spec.use_table || !spec.table.is_empty()) {
        crate::tokens::helper(ui, "No numeric vectors to fit: create x and y first");
    }
}

fn number_picker(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut u8,
    range: RangeInclusive<u8>,
) {
    field(ui, label, |ui| {
        egui::ComboBox::from_id_salt(id)
            .width(ui.available_width())
            .selected_text(value.to_string())
            .show_ui(ui, |ui| {
                for n in range {
                    ui.selectable_value(value, n, n.to_string());
                }
            });
    });
}

fn model_ui(
    ui: &mut egui::Ui,
    spec: &mut Spec,
    info: Option<&Result<CoefInfo, String>>,
    default_p: Option<f64>,
) {
    let p = crate::tokens::palette(ui.ctx());
    let surface = spec.surface();
    field(ui, "Fit type", |ui| {
        let mut chosen = spec.family;
        egui::ComboBox::from_id_salt("curve_fitter_family")
            .width(ui.available_width())
            .selected_text(spec.family.label())
            .show_ui(ui, |ui| {
                for family in Family::ALL {
                    if surface && family != Family::Polynomial {
                        continue;
                    }
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
        spec.set_family(chosen);
        if surface {
            crate::tokens::helper(ui, "Surfaces fit polynomials in x and y");
        }
    });
    match spec.family {
        Family::Polynomial if surface => {
            number_picker(
                ui,
                "curve_fitter_x_degree",
                "Degree in x",
                &mut spec.x_degree,
                0..=5,
            );
            number_picker(
                ui,
                "curve_fitter_y_degree",
                "Degree in y",
                &mut spec.y_degree,
                0..=5,
            );
        }
        Family::Rational => {
            number_picker(
                ui,
                "curve_fitter_numerator",
                "Numerator degree",
                &mut spec.numerator,
                0..=5,
            );
            number_picker(
                ui,
                "curve_fitter_denominator",
                "Denominator degree",
                &mut spec.denominator,
                1..=5,
            );
        }
        Family::Weibull => {
            crate::tokens::helper(ui, "y = a*b*x^(b-1)*exp(-a*x^b)");
        }
        Family::Interpolant => {
            field(ui, "Method", |ui| {
                egui::ComboBox::from_id_salt("curve_fitter_interp")
                    .width(ui.available_width())
                    .selected_text(spec.interp.label())
                    .show_ui(ui, |ui| {
                        for interp in Interp::ALL {
                            ui.selectable_value(&mut spec.interp, interp, interp.label());
                        }
                    });
            });
        }
        Family::SmoothingSpline => smoothing_ui(ui, spec, default_p),
        Family::Custom => {
            field(ui, "Equation y =", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut spec.equation).char_limit(400),
                    )
                    .desired_width(ui.available_width()),
                );
                match info {
                    Some(Ok(info)) => {
                        crate::tokens::helper(
                            ui,
                            format!("Coefficients: {}", info.names.join(", ")),
                        );
                    }
                    Some(Err(error)) => {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(error.as_str())
                                    .size(crate::tokens::CAPTION)
                                    .color(p.error),
                            )
                            .wrap(),
                        );
                    }
                    None => {
                        crate::tokens::helper(ui, "Uses x; other names are coefficients");
                    }
                }
            });
        }
        family => {
            if let Some((label, range)) = family.terms() {
                number_picker(ui, "curve_fitter_terms", label, &mut spec.terms, range);
            }
        }
    }
}

fn smoothing_ui(ui: &mut egui::Ui, spec: &mut Spec, default_p: Option<f64>) {
    let p = crate::tokens::palette(ui.ctx());
    field(ui, "Smoothing parameter", |ui| {
        let was_default = spec.default_smoothing;
        ui.checkbox(
            &mut spec.default_smoothing,
            "Default (chosen from the data)",
        );
        if was_default
            && !spec.default_smoothing
            && let Some(p) = default_p
        {
            // Start the slider where the default fit is.
            spec.smoothing = smoothing_from_position(smoothing_position(p));
        }
        let shown = if spec.default_smoothing {
            default_p.unwrap_or(spec.smoothing)
        } else {
            spec.smoothing
        };
        let mut position = smoothing_position(shown);
        ui.spacing_mut().slider_width = ui.available_width();
        let response = ui.add_enabled(
            !spec.default_smoothing,
            egui::Slider::new(&mut position, SMOOTHING_SLIDER).show_value(false),
        );
        if response.changed() {
            spec.smoothing = smoothing_from_position(position);
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Smoother").color(p.text_muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Closer to data").color(p.text_muted));
            });
        });
        crate::tokens::helper(
            ui,
            match (spec.default_smoothing, default_p) {
                (true, Some(p)) => format!("p = {} (default)", short(p)),
                (true, None) => "p is chosen when the fit runs".to_owned(),
                (false, _) => format!("p = {}", number(spec.smoothing)),
            },
        );
    });
}

fn options_ui(ui: &mut egui::Ui, spec: &mut Spec, info: Option<&Result<CoefInfo, String>>) {
    if spec.family.least_squares() {
        field(ui, "Robust", |ui| {
            ui.horizontal(|ui| {
                for robust in Robust::ALL {
                    if crate::tokens::toggle(ui, spec.robust == robust, robust.label()).clicked() {
                        spec.robust = robust;
                    }
                }
            });
        });
    }
    ui.checkbox(
        &mut spec.normalize,
        "Normalize (center and scale the inputs)",
    );
    if !spec.family.nonlinear() {
        return;
    }
    ui.add_space(crate::tokens::S);
    field(ui, "Coefficients", |ui| match info {
        Some(Ok(info)) if !info.names.is_empty() => {
            let cell = ((ui.available_width() - 48.) / 3. - crate::tokens::XS).max(40.);
            egui::Grid::new("curve_fitter_bounds")
                .num_columns(4)
                .spacing(egui::vec2(crate::tokens::XS, crate::tokens::XS))
                .show(ui, |ui| {
                    ui.label("");
                    for heading in ["Start", "Lower", "Upper"] {
                        crate::tokens::helper(ui, heading);
                    }
                    ui.end_row();
                    for name in &info.names {
                        ui.label(name.as_str());
                        let entry = spec.bounds.entry(name.clone()).or_default();
                        for text in entry.iter_mut() {
                            ui.add(
                                crate::tokens::input(
                                    egui::TextEdit::singleline(text).char_limit(32),
                                )
                                .desired_width(cell),
                            );
                        }
                        ui.end_row();
                    }
                });
            crate::tokens::helper(ui, "Blank: the default. Start needs all or none.");
        }
        Some(Ok(_)) => {
            crate::tokens::helper(ui, "The model has no coefficients");
        }
        Some(Err(_)) => {
            crate::tokens::helper(ui, "Fix the model to set start points and bounds");
        }
        None => {
            crate::tokens::helper(ui, "Reading the coefficients…");
        }
    });
}

fn exclude_ui(ui: &mut egui::Ui, spec: &mut Spec) {
    field(ui, "Exclude", |ui| {
        crate::tokens::helper(
            ui,
            "Click a point in the plot to leave it out; click again to bring it back.",
        );
        ui.horizontal(|ui| {
            crate::tokens::helper(ui, format!("{} points clicked out", spec.excluded.len()));
            if !spec.excluded.is_empty() && ui.button("Bring back").clicked() {
                spec.excluded.clear();
            }
        });
        ui.add_space(crate::tokens::XS);
        ui.label("Keep x from … to");
        ui.horizontal(|ui| {
            let width = (ui.available_width() - crate::tokens::S) / 2.;
            for text in [&mut spec.x_min, &mut spec.x_max] {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(text).char_limit(32))
                        .hint_text("no limit")
                        .desired_width(width),
                );
            }
        });
        if let Err(error) = spec.x_range() {
            let p = crate::tokens::palette(ui.ctx());
            ui.label(
                egui::RichText::new(error.trim_start_matches("Curve Fitter: "))
                    .size(crate::tokens::CAPTION)
                    .color(p.error),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Main view

impl CurveFitter {
    /// New fit / Duplicate / Delete and the table that compares the fits.
    fn fits_ui(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        ui.horizontal(|ui| {
            let room = self.fits.len() < MAX_FITS;
            if ui.add_enabled(room, egui::Button::new("New fit")).clicked() {
                self.new_fit();
            }
            if ui
                .add_enabled(room, egui::Button::new("Duplicate"))
                .clicked()
            {
                self.duplicate();
            }
            if ui
                .add_enabled(self.fits.len() > 1, egui::Button::new("Delete"))
                .clicked()
            {
                self.delete();
            }
        });
        ui.add_space(crate::tokens::XS);
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .id_salt("curve_fitter_fits")
            .max_height(5. * crate::tokens::ROW + crate::tokens::S)
            .auto_shrink([false, true])
            .show_themed(ui, |ui| {
                egui::Grid::new("curve_fitter_fit_table")
                    .num_columns(6)
                    .spacing(egui::vec2(crate::tokens::M, crate::tokens::XS))
                    .show(ui, |ui| {
                        for heading in ["Fit", "Type", "SSE", "R²", "Adj R²", "RMSE"] {
                            ui.label(egui::RichText::new(heading).color(p.text_muted));
                        }
                        ui.end_row();
                        for (i, entry) in self.fits.iter().enumerate() {
                            if ui
                                .selectable_label(i == self.selected, &entry.name)
                                .clicked()
                            {
                                chosen = Some(i);
                            }
                            match &entry.result {
                                Some(r) => {
                                    ui.label(&r.type_name);
                                    for v in [r.sse, r.rsquare, r.adjrsquare, r.rmse] {
                                        ui.label(short(v));
                                    }
                                }
                                None => {
                                    ui.label(entry.spec.model());
                                    for _ in 0..4 {
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
    /// Prediction bounds: on/off, level and kind.
    fn bounds_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_bounds, "Prediction bounds");
            ui.add_enabled(
                self.show_bounds,
                egui::DragValue::new(&mut self.level)
                    .range(50.0..=99.9)
                    .speed(0.1)
                    .max_decimals(1)
                    .suffix(" %"),
            );
            for (functional, label) in [(false, "Observation"), (true, "Functional")] {
                let response = ui.add_enabled_ui(self.show_bounds, |ui| {
                    crate::tokens::toggle(ui, self.functional == functional, label)
                });
                if response.inner.clicked() {
                    self.functional = functional;
                }
            }
        });
    }
}

fn chart<'a>(id: &str, height: f32, (x_label, y_label): (&str, &str)) -> egui_plot::Plot<'a> {
    egui_plot::Plot::new(id.to_owned())
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
        .show_background(false)
}
fn trace() -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[0];
    egui::Color32::from_rgb(r, g, b)
}
/// Most data dots a plot draws; larger data sets draw every k-th kept point
/// (excluded points always show).
const MAX_MARKERS: usize = 10_000;
/// Kept points as dots, excluded ones as crosses.
fn markers(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    points: &[[f64; 2]],
    mask: &[bool],
    ink: egui::Color32,
    muted: egui::Color32,
) {
    let (mut kept, mut left) = (Vec::new(), Vec::new());
    let step = points.len().div_ceil(MAX_MARKERS).max(1);
    for (i, point) in points.iter().enumerate() {
        if !(point[0].is_finite() && point[1].is_finite()) {
            continue;
        }
        if mask.get(i).copied().unwrap_or(false) {
            left.push(*point);
        } else if i % step == 0 {
            kept.push(*point);
        }
    }
    plot_ui.points(
        egui_plot::Points::new("Data", kept)
            .shape(egui_plot::MarkerShape::Circle)
            .filled(true)
            .radius(2.5)
            .color(ink),
    );
    if !left.is_empty() {
        plot_ui.points(
            egui_plot::Points::new("Excluded", left)
                .shape(egui_plot::MarkerShape::Cross)
                .radius(4.)
                .color(muted),
        );
    }
}
/// The data point nearest to `pos` within `radius` screen pixels.
pub fn nearest(
    points: impl Iterator<Item = egui::Pos2>,
    pos: egui::Pos2,
    radius: f32,
) -> Option<usize> {
    points
        .enumerate()
        .map(|(i, p)| (i, p.distance(pos)))
        .filter(|(_, d)| d.is_finite() && *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}
/// The data point a click on the plot hit.
fn clicked_point(response: &egui_plot::PlotResponse<()>, points: &[[f64; 2]]) -> Option<usize> {
    if !response.response.clicked() {
        return None;
    }
    let pos = response.response.interact_pointer_pos()?;
    nearest(
        points.iter().map(|p| {
            response
                .transform
                .position_from_point(&egui_plot::PlotPoint::new(p[0], p[1]))
        }),
        pos,
        10.,
    )
}

fn curve_plot(
    ui: &mut egui::Ui,
    height: f32,
    labels: (&str, &str),
    result: &FitResult,
    mask: &[bool],
    bounds: bool,
) -> Option<usize> {
    let p = crate::tokens::palette(ui.ctx());
    let points = result.points();
    let response = chart("curve_fitter_fit", height, labels).show(ui, |plot_ui| {
        markers(plot_ui, &points, mask, p.text, p.text_muted);
        if bounds {
            for (name, line) in [
                ("Lower bound", &result.lower),
                ("Upper bound", &result.upper),
            ] {
                plot_ui.line(
                    egui_plot::Line::new(name, line.clone())
                        .color(trace())
                        .style(egui_plot::LineStyle::dashed_dense())
                        .width(1.),
                );
            }
        }
        plot_ui.line(
            egui_plot::Line::new("Fit", result.curve.clone())
                .color(trace())
                .width(1.5),
        );
    });
    clicked_point(&response, &points)
}

/// Fixed perceptual ramp, dark (low z) to light (high z).
const RAMP: [[u8; 3]; 5] = [
    [68, 1, 84],
    [59, 82, 139],
    [33, 145, 140],
    [94, 201, 98],
    [253, 231, 37],
];
pub fn ramp(t: f64) -> egui::Color32 {
    if !t.is_finite() {
        return egui::Color32::TRANSPARENT;
    }
    let t = t.clamp(0., 1.) * (RAMP.len() - 1) as f64;
    let k = (t.floor() as usize).min(RAMP.len() - 2);
    let f = t - k as f64;
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    let (a, b) = (RAMP[k], RAMP[k + 1]);
    egui::Color32::from_rgb(mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]))
}

fn surface_plot(
    ui: &mut egui::Ui,
    height: f32,
    labels: (&str, &str),
    result: &FitResult,
    grid: &Grid,
    mask: &[bool],
) -> Option<usize> {
    let p = crate::tokens::palette(ui.ctx());
    let (lo, hi) = finite_range(&grid.values);
    crate::tokens::helper(
        ui,
        format!(
            "Colour: the fitted surface, dark at {} to light at {}.",
            short(lo),
            short(hi)
        ),
    );
    let height = (height - crate::tokens::ROW).max(160.);
    let n = GRID as f64 - 1.;
    let dx = (grid.x.1 - grid.x.0) / n;
    let dy = (grid.y.1 - grid.y.0) / n;
    let points = result.points();
    let response = chart("curve_fitter_surface", height, labels).show(ui, |plot_ui| {
        plot_ui.heatmap(
            egui_plot::Heatmap::new(grid.values.clone(), GRID)
                .at(egui_plot::PlotPoint::new(
                    grid.x.0 - dx / 2.,
                    grid.y.0 - dy / 2.,
                ))
                .tile_size(dx as f32, dy as f32)
                .show_labels(false)
                .custom_mapping(Box::new(move |v| ramp((v - lo) / (hi - lo)))),
        );
        markers(plot_ui, &points, mask, p.text, p.surface);
    });
    clicked_point(&response, &points)
}

fn residual_plot(
    ui: &mut egui::Ui,
    height: f32,
    x_label: &str,
    result: &FitResult,
    mask: &[bool],
    surface: bool,
) -> Option<usize> {
    let p = crate::tokens::palette(ui.ctx());
    let against = if surface { &result.fitted } else { &result.x };
    let points: Vec<[f64; 2]> = against
        .iter()
        .zip(&result.residuals)
        .map(|(x, r)| [*x, *r])
        .collect();
    let response =
        chart("curve_fitter_residuals", height, (x_label, "Residual")).show(ui, |plot_ui| {
            plot_ui.hline(
                egui_plot::HLine::new("Zero", 0.)
                    .color(p.text_muted)
                    .width(1.),
            );
            markers(plot_ui, &points, mask, trace(), p.text_muted);
        });
    clicked_point(&response, &points)
}
