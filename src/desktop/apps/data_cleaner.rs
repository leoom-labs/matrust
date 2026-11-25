//! Data Cleaner: cleans a table, timetable or numeric matrix with an ordered
//! list of steps (missing data, outliers, smoothing, normalizing, retiming,
//! removing, renaming and sorting). One function builds the script: a copy
//! of the data on the output name, then one statement per enabled step. The
//! private run reruns the steps on a private copy to learn what each step
//! did (rows removed, values filled, outlier thresholds) and the before and
//! after statistics. Every number shown comes from engine builtins.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::collections::BTreeMap;

/// Most variables the app works with.
pub const MAX_VARIABLES: usize = 1000;
/// Most steps in the list.
pub const MAX_STEPS: usize = 64;
/// Rows the Data view shows.
pub const GRID_ROWS: usize = 10_000;
/// Most plotted points per line; longer data draw every k-th point.
const MAX_POINTS: usize = 20_000;

// ---------------------------------------------------------------------------
// Step parameters

/// Ways to fill missing values, named as fillmissing's method argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingFill {
    Constant,
    Previous,
    Next,
    Nearest,
    Linear,
    Spline,
    Pchip,
    Makima,
    MovMean,
    MovMedian,
}
impl MissingFill {
    pub const ALL: [Self; 10] = [
        Self::Constant,
        Self::Previous,
        Self::Next,
        Self::Nearest,
        Self::Linear,
        Self::Spline,
        Self::Pchip,
        Self::Makima,
        Self::MovMean,
        Self::MovMedian,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Previous => "previous",
            Self::Next => "next",
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Spline => "spline",
            Self::Pchip => "pchip",
            Self::Makima => "makima",
            Self::MovMean => "movmean",
            Self::MovMedian => "movmedian",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Constant => "A constant",
            Self::Previous => "Previous value",
            Self::Next => "Next value",
            Self::Nearest => "Nearest value",
            Self::Linear => "Linear interpolation",
            Self::Spline => "Spline interpolation",
            Self::Pchip => "Shape-preserving cubic (pchip)",
            Self::Makima => "Modified Akima (makima)",
            Self::MovMean => "Moving mean",
            Self::MovMedian => "Moving median",
        }
    }
    fn windowed(self) -> bool {
        matches!(self, Self::MovMean | Self::MovMedian)
    }
}

/// Outlier detection methods, named as isoutlier's method argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindMethod {
    Median,
    Mean,
    Quartiles,
    Grubbs,
    Gesd,
    MovMedian,
    MovMean,
}
impl FindMethod {
    pub const ALL: [Self; 7] = [
        Self::Median,
        Self::Mean,
        Self::Quartiles,
        Self::Grubbs,
        Self::Gesd,
        Self::MovMedian,
        Self::MovMean,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Median => "median",
            Self::Mean => "mean",
            Self::Quartiles => "quartiles",
            Self::Grubbs => "grubbs",
            Self::Gesd => "gesd",
            Self::MovMedian => "movmedian",
            Self::MovMean => "movmean",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Median => "Median (scaled MAD)",
            Self::Mean => "Mean (standard deviations)",
            Self::Quartiles => "Quartiles (interquartile range)",
            Self::Grubbs => "Grubbs test",
            Self::Gesd => "Generalized ESD test",
            Self::MovMedian => "Moving median",
            Self::MovMean => "Moving mean",
        }
    }
    /// The ThresholdFactor the function uses when none is given.
    pub fn default_threshold(self) -> f64 {
        match self {
            Self::Quartiles => 1.5,
            Self::Grubbs | Self::Gesd => 0.05,
            _ => 3.,
        }
    }
    pub fn moving(self) -> bool {
        matches!(self, Self::MovMedian | Self::MovMean)
    }
    /// The threshold is a significance level rather than a multiple.
    fn alpha(self) -> bool {
        matches!(self, Self::Grubbs | Self::Gesd)
    }
}

/// Replacements for outliers, named as filloutliers' fill argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlierFill {
    Center,
    Clip,
    Previous,
    Next,
    Nearest,
    Linear,
    Spline,
    Pchip,
    Makima,
}
impl OutlierFill {
    pub const ALL: [Self; 9] = [
        Self::Center,
        Self::Clip,
        Self::Previous,
        Self::Next,
        Self::Nearest,
        Self::Linear,
        Self::Spline,
        Self::Pchip,
        Self::Makima,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Clip => "clip",
            Self::Previous => "previous",
            Self::Next => "next",
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Spline => "spline",
            Self::Pchip => "pchip",
            Self::Makima => "makima",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Center => "Center value",
            Self::Clip => "Clip to the threshold",
            Self::Previous => "Previous value",
            Self::Next => "Next value",
            Self::Nearest => "Nearest value",
            Self::Linear => "Linear interpolation",
            Self::Spline => "Spline interpolation",
            Self::Pchip => "Shape-preserving cubic (pchip)",
            Self::Makima => "Modified Akima (makima)",
        }
    }
}

/// smoothdata methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smoother {
    MovMean,
    MovMedian,
    Gaussian,
    Lowess,
    Loess,
    RLowess,
    RLoess,
    Sgolay,
}
impl Smoother {
    pub const ALL: [Self; 8] = [
        Self::MovMean,
        Self::MovMedian,
        Self::Gaussian,
        Self::Lowess,
        Self::Loess,
        Self::RLowess,
        Self::RLoess,
        Self::Sgolay,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::MovMean => "movmean",
            Self::MovMedian => "movmedian",
            Self::Gaussian => "gaussian",
            Self::Lowess => "lowess",
            Self::Loess => "loess",
            Self::RLowess => "rlowess",
            Self::RLoess => "rloess",
            Self::Sgolay => "sgolay",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::MovMean => "Moving mean",
            Self::MovMedian => "Moving median",
            Self::Gaussian => "Gaussian filter",
            Self::Lowess => "Local linear regression (lowess)",
            Self::Loess => "Local quadratic regression (loess)",
            Self::RLowess => "Robust lowess",
            Self::RLoess => "Robust loess",
            Self::Sgolay => "Savitzky-Golay",
        }
    }
}

/// smoothdata's SmoothingFactor when none is given.
pub const DEFAULT_FACTOR: f64 = 0.25;

/// normalize methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormMethod {
    ZScore,
    Range,
    Center,
    Scale,
    Norm,
}
impl NormMethod {
    pub const ALL: [Self; 5] = [
        Self::ZScore,
        Self::Range,
        Self::Center,
        Self::Scale,
        Self::Norm,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::ZScore => "zscore",
            Self::Range => "range",
            Self::Center => "center",
            Self::Scale => "scale",
            Self::Norm => "norm",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::ZScore => "Z-score",
            Self::Range => "Rescale to a range",
            Self::Center => "Center",
            Self::Scale => "Scale",
            Self::Norm => "Unit norm",
        }
    }
}

/// What 'scale' divides by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleBy {
    Std,
    Mad,
    First,
    Iqr,
}
impl ScaleBy {
    pub const ALL: [Self; 4] = [Self::Std, Self::Mad, Self::First, Self::Iqr];
    pub fn name(self) -> &'static str {
        match self {
            Self::Std => "std",
            Self::Mad => "mad",
            Self::First => "first",
            Self::Iqr => "iqr",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Std => "Standard deviation",
            Self::Mad => "Median absolute deviation",
            Self::First => "First element",
            Self::Iqr => "Interquartile range",
        }
    }
}

/// Units of a time window or time step, named as the duration functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Seconds,
    Minutes,
    Hours,
    Days,
}
impl TimeUnit {
    pub const ALL: [Self; 4] = [Self::Seconds, Self::Minutes, Self::Hours, Self::Days];
    pub fn name(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
        }
    }
}

/// New row times of a Retime step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeStep {
    Secondly,
    Minutely,
    Hourly,
    Daily,
    Monthly,
    Yearly,
    /// Every `every` `unit` from the first row time to the last.
    Custom,
}
impl TimeStep {
    pub const ALL: [Self; 7] = [
        Self::Secondly,
        Self::Minutely,
        Self::Hourly,
        Self::Daily,
        Self::Monthly,
        Self::Yearly,
        Self::Custom,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Secondly => "secondly",
            Self::Minutely => "minutely",
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
            Self::Custom => "custom",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Secondly => "Every second",
            Self::Minutely => "Every minute",
            Self::Hourly => "Every hour",
            Self::Daily => "Every day",
            Self::Monthly => "Every month",
            Self::Yearly => "Every year",
            Self::Custom => "Custom step",
        }
    }
}

/// retime methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetimeMethod {
    FillWithMissing,
    Previous,
    Next,
    Nearest,
    Linear,
    Spline,
    Mean,
    Sum,
    Min,
    Max,
    Count,
    FirstValue,
    LastValue,
}
impl RetimeMethod {
    pub const ALL: [Self; 13] = [
        Self::FillWithMissing,
        Self::Previous,
        Self::Next,
        Self::Nearest,
        Self::Linear,
        Self::Spline,
        Self::Mean,
        Self::Sum,
        Self::Min,
        Self::Max,
        Self::Count,
        Self::FirstValue,
        Self::LastValue,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::FillWithMissing => "fillwithmissing",
            Self::Previous => "previous",
            Self::Next => "next",
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Spline => "spline",
            Self::Mean => "mean",
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Count => "count",
            Self::FirstValue => "firstvalue",
            Self::LastValue => "lastvalue",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::FillWithMissing => "Leave new rows missing",
            Self::Previous => "Previous value",
            Self::Next => "Next value",
            Self::Nearest => "Nearest value",
            Self::Linear => "Linear interpolation",
            Self::Spline => "Spline interpolation",
            Self::Mean => "Mean of each bin",
            Self::Sum => "Sum of each bin",
            Self::Min => "Minimum of each bin",
            Self::Max => "Maximum of each bin",
            Self::Count => "Count of each bin",
            Self::FirstValue => "First value of each bin",
            Self::LastValue => "Last value of each bin",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MissingStep {
    /// Remove the rows rather than fill.
    pub remove: bool,
    pub fill: MissingFill,
    pub constant: f64,
    /// Window of the moving fills, in samples.
    pub window: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct OutlierStep {
    pub find: FindMethod,
    pub threshold: f64,
    /// Window of the moving methods: samples, or `unit` for a timetable.
    pub window: f64,
    pub unit: TimeUnit,
    /// Remove the rows rather than fill.
    pub remove: bool,
    pub fill: OutlierFill,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SmoothStep {
    pub method: Smoother,
    /// A window in samples rather than a smoothing factor.
    pub by_window: bool,
    pub window: u32,
    pub factor: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizeStep {
    pub method: NormMethod,
    /// z-score with median and MAD.
    pub robust: bool,
    pub low: f64,
    pub high: f64,
    /// Center on the median rather than the mean.
    pub median: bool,
    pub scale: ScaleBy,
    /// p of the p-norm (Inf allowed).
    pub p: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RetimeStep {
    pub step: TimeStep,
    pub every: f64,
    pub unit: TimeUnit,
    pub method: RetimeMethod,
}

/// What a step does.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Missing(MissingStep),
    Outliers(OutlierStep),
    Smooth(SmoothStep),
    Normalize(NormalizeStep),
    Retime(RetimeStep),
    Remove,
    /// New name by old name (blank or unchanged: kept).
    Rename(BTreeMap<String, String>),
    Sort {
        descend: bool,
    },
}

/// The step kinds of the Add step menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Missing,
    Outliers,
    Smooth,
    Normalize,
    Retime,
    Remove,
    Rename,
    Sort,
}
impl StepKind {
    pub const ALL: [Self; 8] = [
        Self::Missing,
        Self::Outliers,
        Self::Smooth,
        Self::Normalize,
        Self::Retime,
        Self::Remove,
        Self::Rename,
        Self::Sort,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "Clean missing data",
            Self::Outliers => "Clean outlier data",
            Self::Smooth => "Smooth data",
            Self::Normalize => "Normalize data",
            Self::Retime => "Retime timetable",
            Self::Remove => "Remove variables",
            Self::Rename => "Rename variables",
            Self::Sort => "Sort rows",
        }
    }
    pub fn op(self) -> Op {
        match self {
            Self::Missing => Op::Missing(MissingStep {
                remove: false,
                fill: MissingFill::Linear,
                constant: 0.,
                window: 5.,
            }),
            Self::Outliers => Op::Outliers(OutlierStep {
                find: FindMethod::Median,
                threshold: FindMethod::Median.default_threshold(),
                window: 5.,
                unit: TimeUnit::Hours,
                remove: false,
                fill: OutlierFill::Linear,
            }),
            Self::Smooth => Op::Smooth(SmoothStep {
                method: Smoother::MovMean,
                by_window: true,
                window: 5,
                factor: DEFAULT_FACTOR,
            }),
            Self::Normalize => Op::Normalize(NormalizeStep {
                method: NormMethod::ZScore,
                robust: false,
                low: 0.,
                high: 1.,
                median: false,
                scale: ScaleBy::Std,
                p: 2.,
            }),
            Self::Retime => Op::Retime(RetimeStep {
                step: TimeStep::Hourly,
                every: 30.,
                unit: TimeUnit::Minutes,
                method: RetimeMethod::Linear,
            }),
            Self::Remove => Op::Remove,
            Self::Rename => Op::Rename(BTreeMap::new()),
            Self::Sort => Op::Sort { descend: false },
        }
    }
    /// Acts on numeric variables only.
    fn numeric(self) -> bool {
        matches!(self, Self::Outliers | Self::Smooth | Self::Normalize)
    }
}

/// One step of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub enabled: bool,
    /// Variables the step acts on (its DataVariables), by their names at
    /// the step's input.
    pub vars: Vec<String>,
    pub op: Op,
}
impl Step {
    pub fn new(kind: StepKind, vars: Vec<String>) -> Self {
        Self {
            enabled: true,
            vars,
            op: kind.op(),
        }
    }
    pub fn kind(&self) -> StepKind {
        match self.op {
            Op::Missing(_) => StepKind::Missing,
            Op::Outliers(_) => StepKind::Outliers,
            Op::Smooth(_) => StepKind::Smooth,
            Op::Normalize(_) => StepKind::Normalize,
            Op::Retime(_) => StepKind::Retime,
            Op::Remove => StepKind::Remove,
            Op::Rename(_) => StepKind::Rename,
            Op::Sort { .. } => StepKind::Sort,
        }
    }
    /// One line for the steps list.
    pub fn summary(&self) -> String {
        match &self.op {
            Op::Missing(m) if m.remove => "Remove rows with missing values".into(),
            Op::Missing(m) => format!("Fill missing: {}", m.fill.name()),
            Op::Outliers(o) if o.remove => format!("Remove outliers ({})", o.find.name()),
            Op::Outliers(o) => format!("Fill outliers: {} ({})", o.fill.name(), o.find.name()),
            Op::Smooth(s) => format!("Smooth: {}", s.method.name()),
            Op::Normalize(n) => format!("Normalize: {}", n.method.name()),
            Op::Retime(r) => format!("Retime: {}, {}", r.step.name(), r.method.name()),
            Op::Remove => "Remove variables".into(),
            Op::Rename(_) => "Rename variables".into(),
            Op::Sort { descend } => format!(
                "Sort rows {}",
                if *descend { "descending" } else { "ascending" }
            ),
        }
    }
    /// Names after this step, from the names before it: (current, original).
    fn rename(&self, names: &mut Vec<(String, String)>) {
        match &self.op {
            Op::Remove => names.retain(|(n, _)| !self.vars.contains(n)),
            Op::Rename(map) => {
                for (n, _) in names.iter_mut() {
                    if self.vars.contains(n)
                        && let Some(new) = map.get(n.as_str()).map(|s| s.trim())
                        && !new.is_empty()
                    {
                        *n = new.to_owned();
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The data

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Table,
    Timetable,
    Matrix,
}

/// One variable of the data.
#[derive(Clone, Debug, PartialEq)]
pub struct VarInfo {
    pub name: String,
    pub class: &'static str,
    /// A real numeric single-column variable: every step can use it.
    pub numeric: bool,
}

/// The chosen workspace variable as the app sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub name: String,
    pub kind: SourceKind,
    pub rows: usize,
    pub vars: Vec<VarInfo>,
}
impl Source {
    fn names(&self) -> Vec<String> {
        self.vars.iter().map(|v| v.name.clone()).collect()
    }
    fn info(&self, original: &str) -> Option<&VarInfo> {
        self.vars.iter().find(|v| v.name == original)
    }
    fn kind_label(&self) -> &'static str {
        match self.kind {
            SourceKind::Table => "table",
            SourceKind::Timetable => "timetable",
            SourceKind::Matrix => "matrix",
        }
    }
}

fn numeric_column(a: &Array) -> bool {
    matches!(a, Array::Numeric(_) | Array::Typed(_)) && !a.is_complex() && a.cols() == 1
}

/// A table, timetable or real numeric matrix: what the app can clean.
pub fn source_of(name: &str, array: &Array) -> Option<Source> {
    match array {
        Array::Table(t) => Some(Source {
            name: name.to_owned(),
            kind: if t.is_timetable() {
                SourceKind::Timetable
            } else {
                SourceKind::Table
            },
            rows: t.height(),
            vars: t
                .names()
                .iter()
                .zip(t.columns())
                .map(|(n, c)| VarInfo {
                    name: n.clone(),
                    class: c.class(),
                    numeric: numeric_column(c),
                })
                .collect(),
        }),
        Array::Numeric(_) | Array::Typed(_) if !array.is_complex() && array.dims().len() == 2 => {
            Some(Source {
                name: name.to_owned(),
                kind: SourceKind::Matrix,
                rows: array.rows(),
                vars: (1..=array.cols())
                    .map(|k| VarInfo {
                        name: format!("Var{k}"),
                        class: array.class(),
                        numeric: true,
                    })
                    .collect(),
            })
        }
        _ => None,
    }
}

/// Workspace variables the app can clean: name and a size/type label.
pub fn candidates(ws: &Snapshot) -> Vec<(String, String)> {
    ws.variables()
        .filter_map(|(name, array)| {
            source_of(name, array).map(|s| {
                (
                    name.to_owned(),
                    format!("{}×{} {}", s.rows, s.vars.len(), s.kind_label()),
                )
            })
        })
        .collect()
}

/// Above zero (NaN is not).
fn positive(v: f64) -> bool {
    v > 0.
}
/// a below b (NaN is not).
fn increasing(a: f64, b: f64) -> bool {
    a < b
}

/// A MATLAB char vector literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
/// A cell array of names: `{'a', 'b'}`.
fn name_list<S: AsRef<str>>(names: &[S]) -> String {
    let items: Vec<String> = names.iter().map(|n| quote(n.as_ref())).collect();
    format!("{{{}}}", items.join(", "))
}
/// A number the engine reads back exactly, Inf included.
fn literal(v: f64) -> String {
    if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else {
        number(v)
    }
}

// ---------------------------------------------------------------------------
// What a run computed

/// One column of the Data view.
#[derive(Clone, Debug, PartialEq)]
pub enum Column {
    Numbers(Vec<f64>),
    /// Text; None is a missing value.
    Text(Vec<Option<String>>),
    /// A multicolumn or unshowable variable.
    Other,
}

/// Statistics of one variable: count, missing, outliers, min, median, max,
/// mean, std.
pub type Stats = [f64; 8];

/// A point mark of the Plot view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Outlier,
    Filled,
    Removed,
}

/// The cleaned data and what each step did, as the private run computed it.
#[derive(Clone, Debug)]
pub struct Cleaned {
    /// The table the script leaves (as exported).
    #[cfg_attr(not(test), allow(dead_code))]
    pub table: Array,
    pub names: Vec<String>,
    pub rows: usize,
    /// Row times as text (first GRID_ROWS rows).
    pub times: Option<Vec<Option<String>>>,
    pub time_class: &'static str,
    pub columns: Vec<Column>,
    /// Column classes for the missing-value text.
    pub classes: Vec<&'static str>,
    /// Cells a step changed (rows x variables, column-major); empty when
    /// the rows were replaced (Retime).
    pub changed: Vec<bool>,
    /// Table height before the first step and after each enabled step.
    pub heights: Vec<usize>,
    /// Indices (into the step list) of the enabled steps of this run.
    pub steps: Vec<usize>,
    /// Plotted variable before and after, (x, y) points.
    pub plot: Option<String>,
    pub original: Vec<[f64; 2]>,
    pub cleaned: Vec<[f64; 2]>,
    pub marks: Vec<(usize, Mark, [f64; 2])>,
    /// Outlier thresholds per outlier step: (step index, x, L, U, C).
    pub thresholds: Vec<(usize, [f64; 4])>,
    /// x is time in seconds from the first row time; else the row index.
    pub time_axis: bool,
    pub before: BTreeMap<String, Stats>,
    pub after: BTreeMap<String, Stats>,
    /// Rows with a missing value, per source variable.
    pub missing: BTreeMap<String, f64>,
}
impl Cleaned {
    pub fn width(&self) -> usize {
        self.names.len()
    }
    /// Whether a step changed the cell (row, variable).
    pub fn is_changed(&self, row: usize, var: usize) -> bool {
        self.changed
            .get(var * self.rows + row)
            .copied()
            .unwrap_or(false)
    }
    pub fn changed_count(&self) -> usize {
        self.changed.iter().filter(|c| **c).count()
    }
    /// Text of a cell as the Data view shows it.
    pub fn cell(&self, row: usize, var: usize) -> String {
        match self.columns.get(var) {
            Some(Column::Numbers(v)) => match v.get(row) {
                Some(x) if x.is_nan() => "NaN".into(),
                Some(x) => short(*x),
                None => String::new(),
            },
            Some(Column::Text(v)) => match v.get(row) {
                Some(Some(text)) => text.clone(),
                Some(None) => missing_text(self.classes[var]).into(),
                None => "…".into(),
            },
            _ => "…".into(),
        }
    }
    pub fn time(&self, row: usize) -> String {
        match self.times.as_ref().and_then(|t| t.get(row)) {
            Some(Some(text)) => text.clone(),
            Some(None) => missing_text(self.time_class).into(),
            None => "…".into(),
        }
    }
}

/// How a missing value of a class reads.
fn missing_text(class: &str) -> &'static str {
    match class {
        "datetime" => "NaT",
        "duration" | "double" | "single" => "NaN",
        "categorical" => "<undefined>",
        "cell" => "''",
        _ => "<missing>",
    }
}

fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        Some(Array::Logical(m)) => Ok(m.data().iter().map(|v| f64::from(*v)).collect()),
        _ => Err(format!("Data Cleaner: {name} is not a real array")),
    }
}
/// A numeric output as rows of `width` columns (column-major source).
fn rows_of(outputs: &Outputs, name: &str, width: usize) -> Result<Vec<Vec<f64>>, String> {
    let array = outputs
        .get(name)
        .ok_or_else(|| format!("Data Cleaner: {name} is missing"))?;
    let data = values(outputs, name)?;
    let rows = array.rows();
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if array.cols() != width {
        return Err(format!("Data Cleaner: {name} has the wrong size"));
    }
    Ok((0..rows)
        .map(|r| (0..width).map(|c| data[c * rows + r]).collect())
        .collect())
}
fn text_column(array: &Array, k: usize, n: usize) -> Option<Vec<Option<String>>> {
    match array {
        Array::Str(s) if s.data().len() >= (k + 1) * n => Some(
            s.data()[k * n..(k + 1) * n]
                .iter()
                .map(|v| v.as_ref().map(|t| t.to_string()))
                .collect(),
        ),
        _ => None,
    }
}
fn number_column(array: &Array) -> Option<Vec<f64>> {
    if array.cols() != 1 {
        return None;
    }
    match array {
        Array::Numeric(m) => Some(m.data().to_vec()),
        Array::Typed(t) => Some((0..t.len()).map(|i| t.real(i)).collect()),
        Array::Logical(l) => Some(l.data().iter().map(|v| f64::from(*v)).collect()),
        _ => None,
    }
}
/// (x, y) pairs with both finite, thinned to MAX_POINTS.
fn points(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    let step = x.len().div_ceil(MAX_POINTS).max(1);
    let mut out: Vec<[f64; 2]> = x
        .iter()
        .zip(y)
        .step_by(step)
        .map(|(x, y)| [*x, *y])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect();
    out.sort_by(|a, b| a[0].total_cmp(&b[0]));
    out
}

// ---------------------------------------------------------------------------
// The app

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Data,
    Plot,
    Summary,
}
impl View {
    pub const ALL: [Self; 3] = [Self::Data, Self::Plot, Self::Summary];
    pub fn label(self) -> &'static str {
        match self {
            Self::Data => "Data",
            Self::Plot => "Plot",
            Self::Summary => "Summary",
        }
    }
}

pub struct DataCleaner {
    /// The workspace variable to clean.
    pub data: String,
    /// Variables new steps act on.
    pub ticks: Vec<String>,
    pub steps: Vec<Step>,
    /// The step the editor and the variable ticks show.
    pub selected: Option<usize>,
    pub view: View,
    /// Source variable the Plot view draws.
    pub plot: String,
    pub output_name: String,
    pub output_edited: bool,
    /// Also export the masks of removed rows and filled values.
    pub masks: bool,
    /// Last good result; errors keep it on screen.
    pub result: Option<Cleaned>,
    identity: Option<String>,
}
impl Default for DataCleaner {
    fn default() -> Self {
        Self {
            data: String::new(),
            ticks: Vec::new(),
            steps: Vec::new(),
            selected: None,
            view: View::Data,
            plot: String::new(),
            output_name: String::new(),
            output_edited: false,
            masks: false,
            result: None,
            identity: None,
        }
    }
}

/// Everything the script and the probe share, checked.
struct Plan {
    source: Source,
    out: String,
    /// Mask output per step index, when asked for.
    masks: BTreeMap<usize, String>,
    /// Indices of the enabled steps, in order.
    enabled: Vec<usize>,
    /// Names at the input of each enabled step: (current, original).
    names: Vec<Vec<(String, String)>>,
    /// Names after the last step.
    last: Vec<(String, String)>,
}

/// Step `k`'s label for messages: "Step 2 (Smooth data)".
fn step_label(k: usize, step: &Step) -> String {
    format!("Step {} ({})", k + 1, step.kind().label())
}

impl DataCleaner {
    /// The chosen data, checked.
    pub fn source(&self, ws: &Snapshot) -> Result<Source, String> {
        if self.data.is_empty() {
            return Err(
                "Data Cleaner: choose a table, timetable or numeric matrix in INPUTS".into(),
            );
        }
        let array = ws.get(&self.data).ok_or_else(|| {
            format!(
                "Data Cleaner: workspace variable {} does not exist",
                self.data
            )
        })?;
        let source = source_of(&self.data, array).ok_or_else(|| {
            format!(
                "Data Cleaner: {} is not a table, timetable or real numeric matrix",
                self.data
            )
        })?;
        if source.vars.is_empty() {
            return Err(format!("Data Cleaner: {} has no variables", self.data));
        }
        if source.rows == 0 {
            return Err(format!("Data Cleaner: {} has no rows to clean", self.data));
        }
        if source.vars.len() > MAX_VARIABLES {
            return Err(format!(
                "Data Cleaner: {} has more than {MAX_VARIABLES} variables",
                self.data
            ));
        }
        Ok(source)
    }
    /// The default output name: `cleanedData`, else numbered.
    fn default_output(&self, ws: &Snapshot) -> String {
        unused_name("cleanedData", |n| ws.contains(n) || n == self.data)
    }
    /// The output name, checked.
    pub fn output(&self, ws: &Snapshot) -> Result<String, String> {
        let name = if self.output_edited {
            self.output_name.trim().to_owned()
        } else {
            self.default_output(ws)
        };
        super::check_export_name(&name)?;
        if !matrust::files::valid_name(&name) {
            return Err("Data Cleaner: the output name is not a valid variable name".into());
        }
        if name == self.data {
            return Err(format!(
                "Data Cleaner: the output needs a name other than the data's ({name})"
            ));
        }
        Ok(name)
    }
    /// Source names with the enabled Remove and Rename steps before step
    /// `k` applied: (current, original).
    pub fn names_at(&self, source: &[String], k: usize) -> Vec<(String, String)> {
        let mut names: Vec<(String, String)> =
            source.iter().map(|n| (n.clone(), n.clone())).collect();
        for step in self.steps.iter().take(k).filter(|s| s.enabled) {
            step.rename(&mut names);
        }
        names
    }
    /// Adds a step after the selected one (else at the end), acting on the
    /// ticked variables, and selects it.
    pub fn add(&mut self, kind: StepKind) {
        if self.steps.len() >= MAX_STEPS {
            return;
        }
        let at = self.selected.map_or(self.steps.len(), |k| k + 1);
        self.steps.insert(at, Step::new(kind, self.ticks.clone()));
        self.selected = Some(at);
    }
    pub fn move_up(&mut self, k: usize) {
        if k > 0 && k < self.steps.len() {
            self.steps.swap(k - 1, k);
            self.selected = Some(k - 1);
        }
    }
    pub fn move_down(&mut self, k: usize) {
        if k + 1 < self.steps.len() {
            self.steps.swap(k, k + 1);
            self.selected = Some(k + 1);
        }
    }
    pub fn delete(&mut self, k: usize) {
        if k < self.steps.len() {
            self.steps.remove(k);
            self.selected = if self.steps.is_empty() {
                None
            } else {
                Some(k.min(self.steps.len() - 1))
            };
        }
    }

    /// The checked plan of the current inputs.
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let source = self.source(ws)?;
        let out = self.output(ws)?;
        let source_names = source.names();
        let mut names: Vec<(String, String)> = source_names
            .iter()
            .map(|n| (n.clone(), n.clone()))
            .collect();
        let mut enabled = Vec::new();
        let mut inputs = Vec::new();
        let mut masks = BTreeMap::new();
        let mut taken: Vec<String> = vec![out.clone(), self.data.clone()];
        for (k, step) in self.steps.iter().enumerate() {
            if !step.enabled {
                continue;
            }
            self.check_step(k, step, &source, &names)?;
            if self.masks
                && let Some(base) = mask_base(step)
            {
                let name = unused_name(base, |n| ws.contains(n) || taken.iter().any(|t| t == n));
                taken.push(name.clone());
                masks.insert(k, name);
            }
            enabled.push(k);
            inputs.push(names.clone());
            step.rename(&mut names);
        }
        Ok(Plan {
            source,
            out,
            masks,
            enabled,
            names: inputs,
            last: names,
        })
    }
    /// A step's inputs, against the names it sees.
    fn check_step(
        &self,
        k: usize,
        step: &Step,
        source: &Source,
        names: &[(String, String)],
    ) -> Result<(), String> {
        let label = step_label(k, step);
        let kind = step.kind();
        if kind == StepKind::Retime {
            if source.kind != SourceKind::Timetable {
                return Err(format!("Data Cleaner: {label} needs a timetable"));
            }
            if let Op::Retime(r) = &step.op
                && r.step == TimeStep::Custom
                && !(r.every.is_finite() && r.every > 0.)
            {
                return Err(format!(
                    "Data Cleaner: {label}: the time step must be positive"
                ));
            }
            return Ok(());
        }
        if step.vars.is_empty() {
            return Err(format!(
                "Data Cleaner: {label}: tick the variables it acts on"
            ));
        }
        for var in &step.vars {
            let Some((_, original)) = names.iter().find(|(n, _)| n == var) else {
                return Err(format!(
                    "Data Cleaner: {label}: there is no variable {var} at this step"
                ));
            };
            let info = source.info(original);
            if kind.numeric() && !info.is_some_and(|i| i.numeric) {
                return Err(format!(
                    "Data Cleaner: {label}: {var} is a {} variable; this step needs numeric \
                     single-column variables",
                    info.map_or("non-numeric", |i| i.class)
                ));
            }
        }
        match &step.op {
            Op::Missing(m) if !m.remove && m.fill.windowed() && !positive(m.window) => Err(
                format!("Data Cleaner: {label}: the window must be positive"),
            ),
            Op::Outliers(o) => {
                if !(o.threshold.is_finite() && o.threshold > 0.)
                    || (o.find.alpha() && o.threshold >= 1.)
                {
                    return Err(format!(
                        "Data Cleaner: {label}: the threshold must be {}",
                        if o.find.alpha() {
                            "between 0 and 1"
                        } else {
                            "positive"
                        }
                    ));
                }
                if o.find.moving() && !(o.window.is_finite() && o.window > 0.) {
                    return Err(format!(
                        "Data Cleaner: {label}: the window must be positive"
                    ));
                }
                Ok(())
            }
            Op::Smooth(s) if !s.by_window && !(0. ..=1.).contains(&s.factor) => Err(format!(
                "Data Cleaner: {label}: the smoothing factor is from 0 to 1"
            )),
            Op::Smooth(s) if s.by_window && s.window == 0 => Err(format!(
                "Data Cleaner: {label}: the window must be positive"
            )),
            Op::Normalize(n) if n.method == NormMethod::Range && !increasing(n.low, n.high) => {
                Err(format!("Data Cleaner: {label}: the range needs low < high"))
            }
            Op::Normalize(n) if n.method == NormMethod::Norm && !positive(n.p) => Err(format!(
                "Data Cleaner: {label}: the norm's p must be positive"
            )),
            Op::Rename(map) => {
                let mut after: Vec<(String, String)> = names.to_vec();
                step.rename(&mut after);
                if after == names {
                    return Err(format!(
                        "Data Cleaner: {label}: type a new name for a ticked variable"
                    ));
                }
                for var in &step.vars {
                    if let Some(new) = map.get(var).map(|s| s.trim())
                        && !new.is_empty()
                        && !matrust::files::valid_name(new)
                    {
                        return Err(format!(
                            "Data Cleaner: {label}: {new} is not a valid variable name"
                        ));
                    }
                }
                for (i, (name, _)) in after.iter().enumerate() {
                    if after[..i].iter().any(|(n, _)| n == name) {
                        return Err(format!(
                            "Data Cleaner: {label}: two variables would be named {name}"
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Step `k`'s function call on `table`: the statement's right side and,
    /// for numeric-only steps, the braces it reads and writes.
    fn call(&self, step: &Step, table: &str, names: &[(String, String)], timetable: bool) -> Call {
        let all =
            names.len() == step.vars.len() && names.iter().all(|(n, _)| step.vars.contains(n));
        // Ticked variables in table order.
        let vars: Vec<&str> = names
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| step.vars.iter().any(|v| v == n))
            .collect();
        let data_vars = if all {
            String::new()
        } else {
            format!(", 'DataVariables', {}", name_list(&vars))
        };
        let braces = format!("{table}{{:, {}}}", name_list(&vars));
        match &step.op {
            Op::Missing(m) if m.remove => Call::plain("rmmissing", format!("{table}{data_vars}")),
            Op::Missing(m) => {
                let value = match m.fill {
                    MissingFill::Constant => format!(", {}", literal(m.constant)),
                    f if f.windowed() => format!(", {}", number(m.window)),
                    _ => String::new(),
                };
                Call::plain(
                    "fillmissing",
                    format!("{table}, {}{value}{data_vars}", quote(m.fill.name())),
                )
            }
            Op::Outliers(o) => {
                let mut args = String::new();
                if !o.remove {
                    args.push_str(&format!(", {}", quote(o.fill.name())));
                }
                args.push_str(&format!(", {}", quote(o.find.name())));
                if o.find.moving() {
                    if timetable {
                        args.push_str(&format!(", {}({})", o.unit.name(), number(o.window)));
                    } else {
                        args.push_str(&format!(", {}", number(o.window)));
                    }
                }
                if o.threshold != o.find.default_threshold() {
                    args.push_str(&format!(", 'ThresholdFactor', {}", number(o.threshold)));
                }
                Call::plain(
                    if o.remove {
                        "rmoutliers"
                    } else {
                        "filloutliers"
                    },
                    format!("{table}{args}{data_vars}"),
                )
            }
            Op::Smooth(s) => {
                let mut args = format!("{braces}, 1, {}", quote(s.method.name()));
                if s.by_window {
                    args.push_str(&format!(", {}", s.window));
                } else if s.factor != DEFAULT_FACTOR {
                    args.push_str(&format!(", 'SmoothingFactor', {}", number(s.factor)));
                }
                Call::braced("smoothdata", args, braces)
            }
            Op::Normalize(n) => {
                let method = quote(n.method.name());
                let rest = match n.method {
                    NormMethod::ZScore if n.robust => ", 'robust'".to_owned(),
                    NormMethod::Range if (n.low, n.high) != (0., 1.) => {
                        format!(", [{} {}]", number(n.low), number(n.high))
                    }
                    NormMethod::Center if n.median => ", 'median'".to_owned(),
                    NormMethod::Scale if n.scale != ScaleBy::Std => {
                        format!(", {}", quote(n.scale.name()))
                    }
                    NormMethod::Norm if n.p != 2. => format!(", {}", literal(n.p)),
                    _ => String::new(),
                };
                Call::braced("normalize", format!("{braces}, 1, {method}{rest}"), braces)
            }
            Op::Retime(r) => {
                let target = if r.step == TimeStep::Custom {
                    format!(
                        "({table}.Properties.RowTimes(1):{}({}):{table}.Properties.RowTimes(end))'",
                        r.unit.name(),
                        number(r.every)
                    )
                } else {
                    quote(r.step.name())
                };
                let method = if r.method == RetimeMethod::FillWithMissing {
                    String::new()
                } else {
                    format!(", {}", quote(r.method.name()))
                };
                Call::plain("retime", format!("{table}, {target}{method}"))
            }
            Op::Remove => Call::plain("removevars", format!("{table}, {}", name_list(&vars))),
            Op::Rename(map) => {
                let (old, new): (Vec<&str>, Vec<&str>) = vars
                    .iter()
                    .filter_map(|v| {
                        let new = map.get(*v).map(|s| s.trim()).filter(|s| !s.is_empty())?;
                        (new != *v).then_some((*v, new))
                    })
                    .unzip();
                Call::plain(
                    "renamevars",
                    format!("{table}, {}, {}", name_list(&old), name_list(&new)),
                )
            }
            Op::Sort { descend } => Call::plain(
                "sortrows",
                format!(
                    "{table}, {}{}",
                    name_list(&vars),
                    if *descend { ", 'descend'" } else { "" }
                ),
            ),
        }
    }

    /// The script: the data on the output name, then one statement per
    /// enabled step.
    fn script_of(&self, plan: &Plan) -> String {
        let out = &plan.out;
        let source = &plan.source;
        let mut script = format!(
            "% Clean {} ({} rows, {} variables), one statement per step.\n",
            source.name,
            source.rows,
            source.vars.len()
        );
        script.push_str(&match source.kind {
            SourceKind::Matrix => format!(
                "{out} = array2table({}, 'VariableNames', \"Var\" + (1:{}));\n",
                source.name,
                source.vars.len()
            ),
            _ => format!("{out} = {};\n", source.name),
        });
        let timetable = source.kind == SourceKind::Timetable;
        for (i, &k) in plan.enabled.iter().enumerate() {
            let step = &self.steps[k];
            let call = self.call(step, out, &plan.names[i], timetable);
            let line = match (&call.braces, plan.masks.get(&k)) {
                (Some(braces), _) => format!("{braces} = {}({});\n", call.func, call.args),
                (None, Some(mask)) => {
                    format!("[{out}, {mask}] = {}({});\n", call.func, call.args)
                }
                (None, None) => format!("{out} = {}({});\n", call.func, call.args),
            };
            script.push_str(&line);
        }
        script
    }

    /// The source variable the Plot view draws: the chosen one if numeric,
    /// else the first numeric variable.
    fn plot_variable(&self, source: &Source) -> Option<String> {
        source
            .vars
            .iter()
            .find(|v| v.numeric && v.name == self.plot)
            .or_else(|| source.vars.iter().find(|v| v.numeric))
            .map(|v| v.name.clone())
    }

    /// Private lines after the script: the steps again on `app_T`, with
    /// what each did, then the plot data, the changed cells and statistics.
    fn probe_of(&self, plan: &Plan) -> String {
        let source = &plan.source;
        let timetable = source.kind == SourceKind::Timetable;
        let x_of = |table: &str| {
            if timetable {
                format!("seconds({table}.Properties.RowTimes - app_t0)")
            } else {
                "app_id".to_owned()
            }
        };
        // Private copies first, so no probe name can shadow the user's names.
        let mut code = format!(
            "[app_out, app_src] = deal({}, {});\n",
            plan.out, source.name
        );
        code.push_str(&match source.kind {
            SourceKind::Matrix => format!(
                "app_S = array2table(app_src, 'VariableNames', \"Var\" + (1:{}));\n",
                source.vars.len()
            ),
            _ => "app_S = app_src;\n".to_owned(),
        });
        code.push_str(
            "app_T = app_S; app_id = (1:height(app_S))'; app_h = height(app_S);\n\
             app_mk = zeros(0, 4); app_th = zeros(0, 5);\n",
        );
        if timetable {
            code.push_str("app_t0 = app_S.Properties.RowTimes(1);\n");
        }
        let plot = self.plot_variable(source);
        let mut tracked = plot.clone();
        for (i, &k) in plan.enabled.iter().enumerate() {
            let step = &self.steps[k];
            let names = &plan.names[i];
            let call = self.call(step, "app_T", names, timetable);
            let p = tracked.as_deref().map(quote);
            let n = k + 1;
            match (&step.op, &p) {
                (Op::Missing(m), Some(p)) if !m.remove => {
                    code.push_str(&format!(
                        "app_m = ismissing(app_T.({p}));\napp_T = {}({});\n\
                         app_f = app_m & ~ismissing(app_T.({p})); app_v = double(app_T.({p})); \
                         app_x = {x};\n\
                         app_mk = [app_mk; repmat([{n} 2], nnz(app_f), 1), app_x(app_f), app_v(app_f)];\n",
                        call.func,
                        call.args,
                        x = x_of("app_T")
                    ));
                }
                (Op::Outliers(o), Some(p)) if step.vars.iter().any(|v| quote(v) == *p) => {
                    let removed = if o.remove { "app_r, " } else { "" };
                    code.push_str(&format!(
                        "app_x = {x}; app_v = double(app_T.({p})); \
                         app_j = find(strcmp(app_T.Properties.VariableNames, {p}));\n\
                         [app_T, {removed}app_o, app_L, app_U, app_C] = {}({});\n\
                         app_o = app_o(:, app_j); app_L = double(app_L.({p})); \
                         app_U = double(app_U.({p})); app_C = double(app_C.({p}));\n\
                         app_mk = [app_mk; repmat([{n} 1], nnz(app_o), 1), app_x(app_o), app_v(app_o)];\n\
                         app_e = ones(numel(app_x), 1);\n\
                         app_th = [app_th; {n} * app_e, app_x, app_L .* app_e, app_U .* app_e, app_C .* app_e];\n",
                        call.func,
                        call.args,
                        x = x_of("app_T")
                    ));
                    if o.remove {
                        code.push_str(&format!(
                            "app_q = app_r & ~app_o;\n\
                             app_mk = [app_mk; repmat([{n} 3], nnz(app_q), 1), app_x(app_q), app_v(app_q)];\n\
                             app_id = app_id(~app_r);\n"
                        ));
                    } else {
                        code.push_str(&format!(
                            "app_w = double(app_T.({p}));\n\
                             app_mk = [app_mk; repmat([{n} 2], nnz(app_o), 1), app_x(app_o), app_w(app_o)];\n"
                        ));
                    }
                }
                (Op::Missing(m), Some(p)) if m.remove => {
                    code.push_str(&format!(
                        "app_x = {x}; app_v = double(app_T.({p}));\n\
                         [app_T, app_r] = {}({});\n\
                         app_mk = [app_mk; repmat([{n} 3], nnz(app_r), 1), app_x(app_r), app_v(app_r)];\n\
                         app_id = app_id(~app_r);\n",
                        call.func,
                        call.args,
                        x = x_of("app_T")
                    ));
                }
                (Op::Missing(m), None) if m.remove => {
                    code.push_str(&format!(
                        "[app_T, app_r] = {}({});\napp_id = app_id(~app_r);\n",
                        call.func, call.args
                    ));
                }
                (Op::Outliers(o), _) if o.remove => {
                    code.push_str(&format!(
                        "[app_T, app_r] = {}({});\napp_id = app_id(~app_r);\n",
                        call.func, call.args
                    ));
                }
                (Op::Retime(_), _) => {
                    code.push_str(&format!(
                        "app_T = {}({});\napp_id = NaN * ones(height(app_T), 1);\n",
                        call.func, call.args
                    ));
                }
                (Op::Sort { .. }, _) => {
                    code.push_str(&format!(
                        "[app_T, app_i] = {}({});\napp_id = app_id(app_i);\n",
                        call.func, call.args
                    ));
                }
                _ => match &call.braces {
                    Some(braces) => {
                        code.push_str(&format!("{braces} = {}({});\n", call.func, call.args))
                    }
                    None => code.push_str(&format!("app_T = {}({});\n", call.func, call.args)),
                },
            }
            code.push_str("app_h(end + 1) = height(app_T);\n");
            // Follow the plotted variable through removals and renames.
            let mut follow: Vec<(String, String)> = names.clone();
            step.rename(&mut follow);
            tracked = tracked.and_then(|current| {
                let original = names.iter().find(|(n, _)| *n == current)?.1.clone();
                follow
                    .iter()
                    .find(|(_, o)| *o == original)
                    .map(|(n, _)| n.clone())
            });
        }
        code.push_str(&format!(
            "app_same = double(isequaln(app_T, app_out));\n\
             app_x0 = {}; app_x1 = {};\n",
            if timetable {
                x_of("app_S")
            } else {
                "(1:height(app_S))'".to_owned()
            },
            x_of("app_T")
        ));
        match (&plot, &tracked) {
            (Some(p0), Some(p1)) => code.push_str(&format!(
                "app_y0 = double(app_S.({})); app_y1 = double(app_T.({}));\n",
                quote(p0),
                quote(p1)
            )),
            (Some(p0), None) => code.push_str(&format!(
                "app_y0 = double(app_S.({})); app_x1 = []; app_y1 = [];\n",
                quote(p0)
            )),
            _ => code.push_str("app_x0 = []; app_y0 = []; app_x1 = []; app_y1 = [];\n"),
        }
        // Cells a step changed: the cleaned value against the source row it
        // came from (sorting and removals follow app_id; Retime makes new rows).
        code.push_str(&format!(
            "app_chg = [];\nif ~any(isnan(app_id))\napp_chg = false(height(app_T), {});\n",
            plan.last.len()
        ));
        for (j, (current, original)) in plan.last.iter().enumerate() {
            let Some(info) = source.info(original) else {
                continue;
            };
            let pick = format!(
                "app_a = app_T.({}); app_b = app_S.({}); app_b = app_b(app_id, :);\n",
                quote(current),
                quote(original)
            );
            code.push_str(&if info.numeric {
                format!(
                    "{pick}app_chg(:, {}) = ~((app_a == app_b) | (isnan(app_a) & isnan(app_b)));\n",
                    j + 1
                )
            } else {
                // Other types change only where a fill replaced a missing
                // value; a type without missing values never changes.
                format!(
                    "try\n{pick}app_chg(:, {}) = any(ismissing(app_a) ~= ismissing(app_b), 2);\n\
                     catch\nend\n",
                    j + 1
                )
            });
        }
        code.push_str("end\n");
        // Statistics before and after, outliers by the first outlier step's rule.
        let find = plan
            .enabled
            .iter()
            .find_map(|k| match &self.steps[*k].op {
                Op::Outliers(o) => Some(self.find_arguments(o, timetable)),
                _ => None,
            })
            .unwrap_or_default();
        let numeric0: Vec<&str> = source
            .vars
            .iter()
            .filter(|v| v.numeric)
            .map(|v| v.name.as_str())
            .collect();
        let (numeric1, original1): (Vec<&str>, Vec<&str>) = plan
            .last
            .iter()
            .filter(|(_, o)| source.info(o).is_some_and(|i| i.numeric))
            .map(|(n, o)| (n.as_str(), o.as_str()))
            .unzip();
        code.push_str(&format!(
            "app_n0 = {}; app_n1 = {}; app_o1 = {}; app_pv = {};\n",
            name_list(&numeric0),
            name_list(&numeric1),
            name_list(&original1),
            quote(plot.as_deref().unwrap_or(""))
        ));
        code.push_str(&stats_code("app_q0", "app_S", "app_n0", &find));
        code.push_str(&stats_code("app_q1", "app_T", "app_n1", &find));
        code.push_str(&format!(
            "app_a0 = {}; app_m0 = NaN * ones(1, numel(app_a0));\n\
             for app_k = 1:numel(app_a0)\n\
             try\n\
             app_m0(app_k) = sum(any(ismissing(app_S.(app_a0{{app_k}})), 2));\n\
             catch\n\
             end\n\
             end\n",
            name_list(&source.names())
        ));
        // Text of the non-numeric columns and the row times for the Data view.
        code.push_str(&format!(
            "app_n = min(height(app_out), {GRID_ROWS});\n\
             app_vn = app_out.Properties.VariableNames; app_tx = strings(app_n, 0);\n\
             for app_k = 1:numel(app_vn)\n\
             app_c = app_out.(app_vn{{app_k}});\n\
             if ~(isa(app_c, 'numeric') || isa(app_c, 'logical')) && size(app_c, 2) == 1\n\
             try\n\
             app_tx = [app_tx, string(app_c(1:app_n))];\n\
             catch\n\
             app_tx = [app_tx, repmat(\"…\", app_n, 1)];\n\
             end\n\
             end\n\
             end\n"
        ));
        code.push_str(if timetable {
            "app_rt = string(app_out.Properties.RowTimes(1:app_n));\n"
        } else {
            "app_rt = strings(0, 1);\n"
        });
        code
    }
    /// isoutlier's method arguments of an outlier step (with the window).
    fn find_arguments(&self, o: &OutlierStep, timetable: bool) -> String {
        let mut args = format!(", {}", quote(o.find.name()));
        if o.find.moving() {
            if timetable {
                args.push_str(&format!(", {}({})", o.unit.name(), number(o.window)));
            } else {
                args.push_str(&format!(", {}", number(o.window)));
            }
        }
        if o.threshold != o.find.default_threshold() {
            args.push_str(&format!(", 'ThresholdFactor', {}", number(o.threshold)));
        }
        args
    }
}

/// The function call of a step.
struct Call {
    func: &'static str,
    args: String,
    /// `T{:, {...}}`: the numeric-only steps read and write through braces.
    braces: Option<String>,
}
impl Call {
    fn plain(func: &'static str, args: String) -> Self {
        Self {
            func,
            args,
            braces: None,
        }
    }
    fn braced(func: &'static str, args: String, braces: String) -> Self {
        Self {
            func,
            args,
            braces: Some(braces),
        }
    }
}

/// The mask output a step can export: removed rows or filled values.
fn mask_base(step: &Step) -> Option<&'static str> {
    match &step.op {
        Op::Missing(m) if m.remove => Some("removedMissing"),
        Op::Missing(_) => Some("filledMissing"),
        Op::Outliers(o) if o.remove => Some("removedOutliers"),
        Op::Outliers(_) => Some("filledOutliers"),
        _ => None,
    }
}

/// Probe lines: per numeric variable of `table`, count, missing, outliers
/// (isoutlier with `find`, else its default), min, median, max, mean, std.
fn stats_code(out: &str, table: &str, names: &str, find: &str) -> String {
    format!(
        "{out} = NaN * ones(8, numel({names})); app_O = [];\n\
         if ~isempty({names}) && height({table}) > 0\n\
         try\n\
         app_O = isoutlier({table}{find}, 'DataVariables', {names});\n\
         catch\n\
         app_O = [];\n\
         end\n\
         end\n\
         for app_k = 1:numel({names})\n\
         app_v = double({table}.({names}{{app_k}})); app_g = app_v(~isnan(app_v));\n\
         {out}(1:2, app_k) = [numel(app_v); numel(app_v) - numel(app_g)];\n\
         if ~isempty(app_O)\n\
         {out}(3, app_k) = sum(app_O(:, find(strcmp({table}.Properties.VariableNames, {names}{{app_k}}))));\n\
         end\n\
         if ~isempty(app_g)\n\
         {out}(4:8, app_k) = [min(app_g); median(app_g); max(app_g); mean(app_g); std(app_g)];\n\
         end\n\
         end\n"
    )
}

/// Names in a cell array of char vectors.
fn cell_names(outputs: &Outputs, name: &str) -> Result<Vec<String>, String> {
    let bad = || format!("Data Cleaner: {name} is not a list of names");
    match outputs.get(name) {
        Some(Array::Cell(c)) => (0..c.len())
            .map(|i| match c.get_linear(i).map_err(|_| bad())?.as_ref() {
                Array::Character(s) => Ok(String::from_utf16_lossy(s.units())),
                _ => Err(bad()),
            })
            .collect(),
        _ => Err(bad()),
    }
}
fn char_text(outputs: &Outputs, name: &str) -> String {
    match outputs.get(name) {
        Some(Array::Character(s)) => String::from_utf16_lossy(s.units()),
        _ => String::new(),
    }
}

const PROBE: [&str; 20] = [
    "app_out", "app_same", "app_h", "app_mk", "app_th", "app_x0", "app_y0", "app_x1", "app_y1",
    "app_chg", "app_q0", "app_q1", "app_m0", "app_tx", "app_rt", "app_a0", "app_n0", "app_n1",
    "app_o1", "app_pv",
];

impl DataCleaner {
    /// The probe's outputs as what the views draw.
    fn cleaned(&self, outputs: &Outputs) -> Result<Cleaned, String> {
        if values(outputs, "app_same")?.first() != Some(&1.) {
            return Err(
                "Data Cleaner: the private rerun of the steps differs from the script's result"
                    .into(),
            );
        }
        let table = outputs
            .get("app_out")
            .cloned()
            .ok_or("Data Cleaner: the cleaned table is missing")?;
        let Array::Table(t) = &table else {
            return Err("Data Cleaner: the cleaned data is not a table".into());
        };
        let rows = t.height();
        let shown = rows.min(GRID_ROWS);
        let text = outputs.get("app_tx");
        let mut text_k = 0;
        let mut columns = Vec::with_capacity(t.width());
        let mut classes = Vec::with_capacity(t.width());
        for column in t.columns() {
            classes.push(column.class());
            if let Some(numbers) = number_column(column) {
                columns.push(Column::Numbers(numbers));
            } else if matches!(
                column,
                Array::Numeric(_) | Array::Typed(_) | Array::Logical(_) | Array::Complex(_)
            ) || column.cols() != 1
            {
                columns.push(Column::Other);
            } else {
                columns.push(
                    text.and_then(|a| text_column(a, text_k, shown))
                        .map_or(Column::Other, Column::Text),
                );
                text_k += 1;
            }
        }
        let times = t
            .row_times()
            .and_then(|_| outputs.get("app_rt"))
            .and_then(|a| text_column(a, 0, shown));
        let changed = values(outputs, "app_chg")?
            .into_iter()
            .map(|v| v != 0.)
            .collect::<Vec<bool>>();
        let changed = if changed.len() == rows * t.width() {
            changed
        } else {
            Vec::new()
        };
        let heights = values(outputs, "app_h")?
            .into_iter()
            .map(|h| h.max(0.) as usize)
            .collect();
        let steps: Vec<usize> = (0..self.steps.len())
            .filter(|k| self.steps[*k].enabled)
            .collect();
        let marks = rows_of(outputs, "app_mk", 4)?
            .into_iter()
            .filter_map(|r| {
                let mark = match r[1] as i64 {
                    1 => Mark::Outlier,
                    2 => Mark::Filled,
                    3 => Mark::Removed,
                    _ => return None,
                };
                (r[2].is_finite() && r[3].is_finite())
                    .then(|| ((r[0] as usize).saturating_sub(1), mark, [r[2], r[3]]))
            })
            .collect();
        let thresholds = rows_of(outputs, "app_th", 5)?
            .into_iter()
            .map(|r| ((r[0] as usize).saturating_sub(1), [r[1], r[2], r[3], r[4]]))
            .collect();
        let stats = |name: &str, names: &[String]| -> Result<BTreeMap<String, Stats>, String> {
            let q = values(outputs, name)?;
            Ok(names
                .iter()
                .enumerate()
                .filter_map(|(k, n)| {
                    let column = q.get(8 * k..8 * k + 8)?;
                    let mut s = [f64::NAN; 8];
                    s.copy_from_slice(column);
                    Some((n.clone(), s))
                })
                .collect())
        };
        let missing = cell_names(outputs, "app_a0")?
            .into_iter()
            .zip(values(outputs, "app_m0")?)
            .collect();
        let plot = Some(char_text(outputs, "app_pv")).filter(|p| !p.is_empty());
        Ok(Cleaned {
            names: t.names().to_vec(),
            rows,
            times,
            time_class: t.row_times().map_or("", |a| a.class()),
            columns,
            classes,
            changed,
            heights,
            steps,
            original: points(&values(outputs, "app_x0")?, &values(outputs, "app_y0")?),
            cleaned: points(&values(outputs, "app_x1")?, &values(outputs, "app_y1")?),
            plot,
            marks,
            thresholds,
            time_axis: t.is_timetable(),
            before: stats("app_q0", &cell_names(outputs, "app_n0")?)?,
            // Keyed by the source name, so renamed variables line up.
            after: stats("app_q1", &cell_names(outputs, "app_o1")?)?,
            missing,
            table,
        })
    }
}

impl App for DataCleaner {
    fn kind(&self) -> AppKind {
        AppKind::DataCleaner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.plan(ws)?;
        Ok(self.script_of(&plan))
    }
    fn inputs(&self) -> Vec<String> {
        if self.data.is_empty() {
            Vec::new()
        } else {
            vec![self.data.clone()]
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        match self.plan(ws) {
            Ok(plan) => (
                self.probe_of(&plan),
                PROBE.iter().map(|s| (*s).to_owned()).collect(),
            ),
            Err(_) => (String::new(), Vec::new()),
        }
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let source = self.source(ws).ok();
        let identity = source
            .as_ref()
            .map(|s| format!("{}|{:?}", s.name, s.names()));
        if self.identity != identity {
            // New data: tick its numeric variables and plot the first one.
            if let Some(source) = &source {
                self.ticks = source
                    .vars
                    .iter()
                    .filter(|v| v.numeric)
                    .map(|v| v.name.clone())
                    .collect();
                if self.plot_variable(source).as_deref() != Some(self.plot.as_str()) {
                    self.plot = self.plot_variable(source).unwrap_or_default();
                }
            }
            self.identity = identity;
        }
        if self.selected.is_some_and(|k| k >= self.steps.len()) {
            self.selected = self.steps.len().checked_sub(1);
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        self.result = Some(self.cleaned(outputs)?);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let options = candidates(ws);
        if self.data.is_empty()
            && let Some((name, _)) = options.first()
        {
            self.data = name.clone();
        }
        let before = self.data.clone();
        field(ui, "Data", |ui| {
            let shown = if self.data.is_empty() {
                "Choose a variable".to_owned()
            } else {
                self.data.clone()
            };
            egui::ComboBox::from_id_salt("data_cleaner_data")
                .width(ui.available_width())
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (name, label) in &options {
                        ui.selectable_value(
                            &mut self.data,
                            name.clone(),
                            format!("{name} ({label})"),
                        );
                    }
                });
            if options.is_empty() {
                crate::tokens::helper(
                    ui,
                    "No tables, timetables or numeric matrices in the workspace",
                );
            }
        });
        if self.data != before {
            self.prepare(ws);
        }
        let source = self.source(ws).ok();
        section(ui);
        self.variables_ui(ui, source.as_ref());
        section(ui);
        self.steps_ui(ui, source.as_ref());
        if let Some(k) = self.selected.filter(|k| *k < self.steps.len()) {
            section(ui);
            self.editor_ui(ui, k, source.as_ref());
        }
    }
    fn main_caption(&self) -> &'static str {
        "CLEANED DATA"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                let tab = crate::tokens::tab(ui, self.view == view, view.label());
                if view == View::Data {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::DataCleaner),
                    );
                }
                if tab.clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(result) = &self.result else {
            crate::tokens::helper(
                ui,
                "Nothing cleaned yet: choose the data in INPUTS. The status line says what is \
                 missing.",
            );
            return;
        };
        match self.view {
            View::Data => data_view(ui, result),
            View::Plot => {
                let mut plot = self.plot.clone();
                plot_view(ui, result, &mut plot);
                self.plot = plot;
            }
            View::Summary => summary_view(ui, result),
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(r) = &self.result else {
            return ["Rows", "Variables", "Rows removed", "Changed cells"]
                .into_iter()
                .map(|label| (label, "—".to_owned()))
                .collect();
        };
        let rows0 = r.heights.first().copied().unwrap_or(r.rows);
        let removed: Vec<String> = r
            .steps
            .iter()
            .zip(r.heights.windows(2))
            .filter(|(_, h)| h[1] < h[0])
            .map(|(k, h)| format!("{} by step {}", h[0] - h[1], k + 1))
            .collect();
        let added: Vec<String> = r
            .steps
            .iter()
            .zip(r.heights.windows(2))
            .filter(|(_, h)| h[1] > h[0])
            .map(|(k, h)| format!("{} by step {}", h[1] - h[0], k + 1))
            .collect();
        let mut rows = vec![
            ("Rows", arrow(rows0, r.rows)),
            ("Variables", arrow(r.missing.len(), r.width())),
            (
                "Rows removed",
                if removed.is_empty() {
                    "none".to_owned()
                } else {
                    removed.join(" · ")
                },
            ),
        ];
        if !added.is_empty() {
            rows.push(("Rows added", added.join(" · ")));
        }
        rows.push((
            "Changed cells",
            if r.changed.is_empty() && r.rows > 0 {
                "new rows (retimed)".to_owned()
            } else {
                r.changed_count().to_string()
            },
        ));
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if !self.output_edited {
            self.output_name = self.default_output(ws);
        }
        let before = self.output_name.clone();
        name_field(ui, "Cleaned table name", &mut self.output_name, ws);
        self.output_edited |= self.output_name != before;
        ui.checkbox(
            &mut self.masks,
            "Also export the masks of removed rows and filled values",
        );
        if self.masks {
            let masks: Vec<String> = self
                .plan(ws)
                .map(|plan| plan.masks.into_values().collect())
                .unwrap_or_default();
            crate::tokens::helper(
                ui,
                if masks.is_empty() {
                    "No enabled step removes rows or fills values".to_owned()
                } else {
                    format!("Masks: {}", masks.join(", "))
                },
            );
        }
        ui.add_space(crate::tokens::XS);
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        match self.plan(ws) {
            Ok(plan) => std::iter::once(plan.out)
                .chain(plan.masks.into_values())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        let r = self.result.as_ref()?;
        let rows0 = r.heights.first().copied().unwrap_or(r.rows);
        Some(format!(
            "{}: {} rows · {} enabled steps · {} cells changed",
            self.data,
            arrow(rows0, r.rows),
            r.steps.len(),
            r.changed_count()
        ))
    }
}

/// `a → b`, or `a` when unchanged.
fn arrow(a: usize, b: usize) -> String {
    if a == b {
        a.to_string()
    } else {
        format!("{a} → {b}")
    }
}

// ---------------------------------------------------------------------------
// Inputs

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    all: &[T],
    label: impl Fn(T) -> &'static str,
) {
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for item in all {
                ui.selectable_value(value, *item, label(*item));
            }
        });
}

fn number_input(ui: &mut egui::Ui, value: &mut f64, range: std::ops::RangeInclusive<f64>) {
    ui.add(
        egui::DragValue::new(value)
            .range(range)
            .speed(0.1)
            .max_decimals(6),
    );
}

impl DataCleaner {
    /// The variable list: the ticks are the selected step's variables (else
    /// those new steps get).
    fn variables_ui(&mut self, ui: &mut egui::Ui, source: Option<&Source>) {
        let p = crate::tokens::palette(ui.ctx());
        let Some(source) = source else {
            field(ui, "Variables", |ui| {
                crate::tokens::helper(ui, "Choose the data first");
            });
            return;
        };
        let step = self.selected.filter(|k| *k < self.steps.len());
        let names = self.names_at(&source.names(), step.unwrap_or(self.steps.len()));
        let retime = step.is_some_and(|k| self.steps[k].kind() == StepKind::Retime);
        ui.label("Variables");
        crate::tokens::helper(
            ui,
            match step {
                _ if retime => "Retime resamples every variable".to_owned(),
                Some(k) => format!("Ticked: what step {} acts on", k + 1),
                None => "Ticked: what new steps act on".to_owned(),
            },
        );
        ui.add_space(crate::tokens::XS);
        let mut ticks = match step {
            Some(k) => self.steps[k].vars.clone(),
            None => self.ticks.clone(),
        };
        let mut changed = false;
        ui.add_enabled_ui(!retime, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("All").clicked() {
                    ticks = names.iter().map(|(n, _)| n.clone()).collect();
                    changed = true;
                }
                if ui.small_button("None").clicked() {
                    ticks.clear();
                    changed = true;
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("data_cleaner_variables")
                .max_height(8. * crate::tokens::ROW)
                .auto_shrink([false, true])
                .show_themed(ui, |ui| {
                    for (name, original) in &names {
                        let info = source.info(original);
                        let mut on = ticks.contains(name);
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut on, name.as_str()).changed() {
                                changed = true;
                                if on {
                                    ticks.push(name.clone());
                                } else {
                                    ticks.retain(|t| t != name);
                                }
                            }
                            let mut detail = info.map_or("", |i| i.class).to_owned();
                            if let Some(r) = &self.result {
                                if let Some(m) = r.missing.get(original).filter(|m| m.is_finite()) {
                                    detail.push_str(&format!(" · {} missing", short(*m)));
                                }
                                if let Some(s) = r.before.get(original).filter(|s| s[2].is_finite())
                                {
                                    detail.push_str(&format!(" · {} outliers", short(s[2])));
                                }
                            }
                            ui.label(
                                egui::RichText::new(detail)
                                    .size(crate::tokens::CAPTION)
                                    .color(p.text_muted),
                            );
                        });
                    }
                });
        });
        if changed {
            // Keep the table order.
            let ordered: Vec<String> = names
                .iter()
                .map(|(n, _)| n.clone())
                .filter(|n| ticks.contains(n))
                .collect();
            if let Some(k) = step {
                self.steps[k].vars = ordered.clone();
            }
            self.ticks = ordered;
        }
    }

    /// The ordered step list with Add, Move up, Move down and Delete.
    fn steps_ui(&mut self, ui: &mut egui::Ui, source: Option<&Source>) {
        ui.label("Cleaning steps");
        if self.steps.is_empty() {
            crate::tokens::helper(ui, "No steps yet: the output is a copy of the data");
        }
        let mut select = None;
        for (k, step) in self.steps.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut step.enabled, "")
                    .on_hover_text("Run this step");
                let text = format!("{}. {}", k + 1, step.summary());
                if ui
                    .selectable_label(self.selected == Some(k), text)
                    .clicked()
                {
                    select = Some(k);
                }
            });
        }
        if let Some(k) = select {
            self.selected = if self.selected == Some(k) {
                None
            } else {
                Some(k)
            };
        }
        ui.add_space(crate::tokens::XS);
        let timetable = source.is_some_and(|s| s.kind == SourceKind::Timetable);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(self.steps.len() < MAX_STEPS, |ui| {
                ui.menu_button("Add step", |ui| {
                    for kind in StepKind::ALL {
                        let enabled = kind != StepKind::Retime || timetable;
                        let button = ui
                            .add_enabled(enabled, egui::Button::new(kind.label()))
                            .on_disabled_hover_text("Retime needs a timetable");
                        if button.clicked() {
                            self.add(kind);
                            ui.close();
                        }
                    }
                });
            });
            let k = self.selected.filter(|k| *k < self.steps.len());
            if ui
                .add_enabled(k.is_some_and(|k| k > 0), egui::Button::new("Up"))
                .clicked()
                && let Some(k) = k
            {
                self.move_up(k);
            }
            if ui
                .add_enabled(
                    k.is_some_and(|k| k + 1 < self.steps.len()),
                    egui::Button::new("Down"),
                )
                .clicked()
                && let Some(k) = k
            {
                self.move_down(k);
            }
            if ui
                .add_enabled(k.is_some(), egui::Button::new("Delete"))
                .clicked()
                && let Some(k) = k
            {
                self.delete(k);
            }
        });
    }

    /// The selected step's parameters.
    fn editor_ui(&mut self, ui: &mut egui::Ui, k: usize, source: Option<&Source>) {
        let timetable = source.is_some_and(|s| s.kind == SourceKind::Timetable);
        let kind = self.steps[k].kind();
        ui.label(
            egui::RichText::new(format!("Step {} · {}", k + 1, kind.label()))
                .font(crate::tokens::semibold(crate::tokens::BODY)),
        );
        ui.add_space(crate::tokens::XS);
        let step = &mut self.steps[k];
        let id = |part: &str| format!("data_cleaner_{part}");
        match &mut step.op {
            Op::Missing(m) => {
                fill_or_remove(ui, &mut m.remove);
                if !m.remove {
                    field(ui, "Fill with", |ui| {
                        combo(
                            ui,
                            &id("missing_fill"),
                            &mut m.fill,
                            &MissingFill::ALL,
                            |f| f.label(),
                        );
                    });
                    if m.fill == MissingFill::Constant {
                        field(ui, "Value", |ui| {
                            number_input(ui, &mut m.constant, f64::MIN..=f64::MAX)
                        });
                    }
                    if m.fill.windowed() {
                        field(ui, "Window (samples)", |ui| {
                            number_input(ui, &mut m.window, 1.0..=1e6)
                        });
                    }
                } else {
                    crate::tokens::helper(ui, "Removes rows missing a ticked variable");
                }
            }
            Op::Outliers(o) => {
                field(ui, "Find outliers by", |ui| {
                    let before = o.find;
                    combo(ui, &id("find"), &mut o.find, &FindMethod::ALL, |f| {
                        f.label()
                    });
                    if o.find != before {
                        o.threshold = o.find.default_threshold();
                    }
                });
                field(
                    ui,
                    if o.find.alpha() {
                        "Significance level"
                    } else {
                        "Threshold factor"
                    },
                    |ui| {
                        let range = if o.find.alpha() {
                            1e-6..=0.5
                        } else {
                            0.01..=100.0
                        };
                        ui.add(
                            egui::DragValue::new(&mut o.threshold)
                                .range(range)
                                .speed(0.01)
                                .max_decimals(4),
                        );
                    },
                );
                if o.find.moving() {
                    field(
                        ui,
                        if timetable {
                            "Window (time)"
                        } else {
                            "Window (samples)"
                        },
                        |ui| {
                            ui.horizontal(|ui| {
                                number_input(ui, &mut o.window, 1e-6..=1e9);
                                if timetable {
                                    egui::ComboBox::from_id_salt(id("window_unit"))
                                        .selected_text(o.unit.name())
                                        .show_ui(ui, |ui| {
                                            for unit in TimeUnit::ALL {
                                                ui.selectable_value(&mut o.unit, unit, unit.name());
                                            }
                                        });
                                }
                            });
                        },
                    );
                }
                fill_or_remove(ui, &mut o.remove);
                if !o.remove {
                    field(ui, "Fill with", |ui| {
                        combo(
                            ui,
                            &id("outlier_fill"),
                            &mut o.fill,
                            &OutlierFill::ALL,
                            |f| f.label(),
                        );
                    });
                }
            }
            Op::Smooth(s) => {
                field(ui, "Method", |ui| {
                    combo(ui, &id("smooth"), &mut s.method, &Smoother::ALL, |m| {
                        m.label()
                    });
                });
                field(ui, "Amount", |ui| {
                    ui.horizontal(|ui| {
                        if crate::tokens::toggle(ui, s.by_window, "Window").clicked() {
                            s.by_window = true;
                        }
                        if crate::tokens::toggle(ui, !s.by_window, "Smoothing factor").clicked() {
                            s.by_window = false;
                        }
                    });
                });
                if s.by_window {
                    field(ui, "Window (samples)", |ui| {
                        ui.add(egui::DragValue::new(&mut s.window).range(1..=1_000_000));
                    });
                } else {
                    field(ui, "Smoothing factor", |ui| {
                        ui.spacing_mut().slider_width = ui.available_width() - 64.;
                        ui.add(egui::Slider::new(&mut s.factor, 0.0..=1.0).max_decimals(2));
                        crate::tokens::helper(ui, "0: least smoothing, 1: most");
                    });
                }
            }
            Op::Normalize(n) => {
                field(ui, "Method", |ui| {
                    combo(ui, &id("normalize"), &mut n.method, &NormMethod::ALL, |m| {
                        m.label()
                    });
                });
                match n.method {
                    NormMethod::ZScore => {
                        ui.checkbox(&mut n.robust, "Robust (median and MAD)");
                    }
                    NormMethod::Range => field(ui, "Range", |ui| {
                        ui.horizontal(|ui| {
                            number_input(ui, &mut n.low, f64::MIN..=f64::MAX);
                            ui.label("to");
                            number_input(ui, &mut n.high, f64::MIN..=f64::MAX);
                        });
                    }),
                    NormMethod::Center => {
                        ui.checkbox(&mut n.median, "Center on the median");
                    }
                    NormMethod::Scale => field(ui, "Divide by", |ui| {
                        combo(ui, &id("scale"), &mut n.scale, &ScaleBy::ALL, |s| s.label());
                    }),
                    NormMethod::Norm => field(ui, "p", |ui| {
                        number_input(ui, &mut n.p, 1e-6..=f64::INFINITY);
                    }),
                }
            }
            Op::Retime(r) => {
                field(ui, "New time step", |ui| {
                    combo(ui, &id("time_step"), &mut r.step, &TimeStep::ALL, |s| {
                        s.label()
                    });
                });
                if r.step == TimeStep::Custom {
                    field(ui, "Every", |ui| {
                        ui.horizontal(|ui| {
                            number_input(ui, &mut r.every, 1e-6..=1e9);
                            egui::ComboBox::from_id_salt(id("step_unit"))
                                .selected_text(r.unit.name())
                                .show_ui(ui, |ui| {
                                    for unit in TimeUnit::ALL {
                                        ui.selectable_value(&mut r.unit, unit, unit.name());
                                    }
                                });
                        });
                    });
                }
                field(ui, "Values at the new times", |ui| {
                    combo(
                        ui,
                        &id("retime_method"),
                        &mut r.method,
                        &RetimeMethod::ALL,
                        |m| m.label(),
                    );
                });
            }
            Op::Remove => {
                crate::tokens::helper(ui, "Removes the ticked variables");
            }
            Op::Rename(map) => {
                if step.vars.is_empty() {
                    crate::tokens::helper(ui, "Tick the variables to rename");
                }
                for var in &step.vars {
                    let text = map.entry(var.clone()).or_insert_with(|| var.clone());
                    field(ui, &format!("New name for {var}"), |ui| {
                        ui.add(
                            crate::tokens::input(egui::TextEdit::singleline(text).char_limit(63))
                                .desired_width(ui.available_width()),
                        );
                    });
                }
            }
            Op::Sort { descend } => {
                ui.horizontal(|ui| {
                    if crate::tokens::toggle(ui, !*descend, "Ascending").clicked() {
                        *descend = false;
                    }
                    if crate::tokens::toggle(ui, *descend, "Descending").clicked() {
                        *descend = true;
                    }
                });
                crate::tokens::helper(ui, "Sorts by the ticked variables, in table order");
            }
        }
    }
}

fn fill_or_remove(ui: &mut egui::Ui, remove: &mut bool) {
    field(ui, "Then", |ui| {
        ui.horizontal(|ui| {
            if crate::tokens::toggle(ui, !*remove, "Fill").clicked() {
                *remove = false;
            }
            if crate::tokens::toggle(ui, *remove, "Remove rows").clicked() {
                *remove = true;
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Main view

/// The cleaned table: index, row times and variables; changed cells marked.
fn data_view(ui: &mut egui::Ui, r: &Cleaned) {
    let p = crate::tokens::palette(ui.ctx());
    let shown = r.rows.min(GRID_ROWS);
    let mut note = format!(
        "{} rows × {} variables. Highlighted: cells a step changed.",
        r.rows,
        r.width()
    );
    if shown < r.rows {
        note.push_str(&format!(" Showing the first {shown} rows."));
    }
    crate::tokens::helper(ui, note);
    ui.add_space(crate::tokens::XS);
    let row = crate::tokens::ROW;
    let index_w = 56.;
    let time_w = if r.times.is_some() { 168. } else { 0. };
    let cell_w = 104.;
    let total = index_w + time_w + cell_w * r.width() as f32;
    let font = egui::FontId::proportional(crate::tokens::BODY);
    egui::ScrollArea::horizontal()
        .id_salt("data_cleaner_grid_x")
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            ui.set_min_width(total);
            // Header.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(total, row), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            let mut x = rect.left() + index_w;
            if r.times.is_some() {
                painter.text(
                    egui::pos2(x + crate::tokens::XS, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Time",
                    font.clone(),
                    p.text_muted,
                );
                x += time_w;
            }
            for name in &r.names {
                let cell =
                    egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(cell_w, row));
                painter.with_clip_rect(cell.shrink(2.)).text(
                    egui::pos2(cell.right() - crate::tokens::XS, cell.center().y),
                    egui::Align2::RIGHT_CENTER,
                    name,
                    font.clone(),
                    p.text_muted,
                );
                x += cell_w;
            }
            painter.hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                egui::Stroke::new(1., p.hairline),
            );
            egui::ScrollArea::vertical()
                .id_salt("data_cleaner_grid_y")
                .auto_shrink([true, false])
                .show_rows_themed(ui, row, shown, |ui, rows| {
                    for i in rows {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(total, row), egui::Sense::hover());
                        let painter = ui.painter_at(rect);
                        painter.text(
                            egui::pos2(rect.left() + index_w - crate::tokens::S, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            (i + 1).to_string(),
                            font.clone(),
                            p.text_muted,
                        );
                        let mut x = rect.left() + index_w;
                        if r.times.is_some() {
                            painter.text(
                                egui::pos2(x + crate::tokens::XS, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                r.time(i),
                                font.clone(),
                                p.text,
                            );
                            x += time_w;
                        }
                        for (j, column) in r.columns.iter().enumerate() {
                            let cell = egui::Rect::from_min_size(
                                egui::pos2(x, rect.top()),
                                egui::vec2(cell_w, row),
                            );
                            let changed = r.is_changed(i, j);
                            if changed {
                                painter.rect_filled(cell.shrink(1.), 2., p.accent_soft);
                            }
                            let text = r.cell(i, j);
                            let missing = matches!(
                                text.as_str(),
                                "NaN" | "NaT" | "<missing>" | "<undefined>"
                            );
                            let color = if changed {
                                p.accent_text
                            } else if missing {
                                p.text_muted
                            } else {
                                p.text
                            };
                            let (pos, align) = if matches!(column, Column::Numbers(_)) {
                                (
                                    egui::pos2(cell.right() - crate::tokens::XS, cell.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                )
                            } else {
                                (
                                    egui::pos2(cell.left() + crate::tokens::XS, cell.center().y),
                                    egui::Align2::LEFT_CENTER,
                                )
                            };
                            painter.with_clip_rect(cell.shrink(2.)).text(
                                pos,
                                align,
                                text,
                                font.clone(),
                                color,
                            );
                            x += cell_w;
                        }
                    }
                });
        });
}

fn trace() -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[0];
    egui::Color32::from_rgb(r, g, b)
}

/// x unit of a time axis in seconds: the divisor and its name.
fn time_unit(span: f64) -> (f64, &'static str) {
    if span > 2. * 86_400. {
        (86_400., "days")
    } else if span > 2. * 3_600. {
        (3_600., "h")
    } else if span > 120. {
        (60., "min")
    } else {
        (1., "s")
    }
}

/// One variable before (muted) and after (accent) with the marks and the
/// outlier thresholds; the variable picker above.
fn plot_view(ui: &mut egui::Ui, r: &Cleaned, plot: &mut String) {
    let p = crate::tokens::palette(ui.ctx());
    let numeric: Vec<&String> = r.before.keys().collect();
    if numeric.is_empty() {
        crate::tokens::helper(ui, "The data has no numeric variables to plot.");
        return;
    }
    ui.horizontal(|ui| {
        ui.label("Variable");
        egui::ComboBox::from_id_salt("data_cleaner_plot_variable")
            .selected_text(r.plot.clone().unwrap_or_default())
            .show_ui(ui, |ui| {
                for name in &numeric {
                    ui.selectable_value(plot, (*name).clone(), name.as_str());
                }
            });
    });
    crate::tokens::helper(
        ui,
        "Grey: original. Line: cleaned. Circles: outliers found; dots: filled values; \
         crosses: removed rows; dashed: outlier thresholds.",
    );
    if r.cleaned.is_empty() && r.original.is_empty() {
        return;
    }
    let all_x = r.original.iter().chain(&r.cleaned).map(|p| p[0]);
    let (lo, hi) = all_x.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
        (a.min(x), b.max(x))
    });
    let (scale, x_label) = if r.time_axis {
        let (div, unit) = time_unit(hi - lo);
        (div, format!("Time from the first row ({unit})"))
    } else {
        (1., "Row of the original data".to_owned())
    };
    let scaled =
        |pts: &[[f64; 2]]| -> Vec<[f64; 2]> { pts.iter().map(|q| [q[0] / scale, q[1]]).collect() };
    let height = (ui.available_height() - crate::tokens::S).max(160.);
    let y_label = r.plot.clone().unwrap_or_default();
    egui_plot::Plot::new("data_cleaner_plot")
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x_label)
        .y_axis_label(y_label)
        .show_background(false)
        .legend(egui_plot::Legend::default())
        .show(ui, |plot_ui| {
            plot_ui.line(
                egui_plot::Line::new("Original", scaled(&r.original))
                    .color(p.text_muted)
                    .width(1.),
            );
            let mut outlier_steps: Vec<usize> = r.thresholds.iter().map(|t| t.0).collect();
            outlier_steps.dedup();
            for k in outlier_steps {
                let mut rows: Vec<[f64; 4]> = r
                    .thresholds
                    .iter()
                    .filter(|t| t.0 == k)
                    .map(|t| t.1)
                    .collect();
                rows.sort_by(|a, b| a[0].total_cmp(&b[0]));
                let label = format!("step {}", k + 1);
                for (column, name, style) in [
                    (1, "Lower threshold", egui_plot::LineStyle::dashed_dense()),
                    (2, "Upper threshold", egui_plot::LineStyle::dashed_dense()),
                    (3, "Center", egui_plot::LineStyle::dotted_dense()),
                ] {
                    let line: Vec<[f64; 2]> = rows
                        .iter()
                        .filter(|t| t[0].is_finite() && t[column].is_finite())
                        .map(|t| [t[0] / scale, t[column]])
                        .collect();
                    plot_ui.line(
                        egui_plot::Line::new(format!("{name} ({label})"), line)
                            .color(p.text_muted)
                            .style(style)
                            .width(1.),
                    );
                }
            }
            plot_ui.line(
                egui_plot::Line::new("Cleaned", scaled(&r.cleaned))
                    .color(trace())
                    .width(1.5),
            );
            for (mark, name) in [
                (Mark::Outlier, "Outliers found"),
                (Mark::Filled, "Filled values"),
                (Mark::Removed, "Removed rows"),
            ] {
                let points: Vec<[f64; 2]> = r
                    .marks
                    .iter()
                    .filter(|m| m.1 == mark)
                    .map(|m| [m.2[0] / scale, m.2[1]])
                    .collect();
                if points.is_empty() {
                    continue;
                }
                let shown = egui_plot::Points::new(name, points);
                plot_ui.points(match mark {
                    Mark::Outlier => shown
                        .shape(egui_plot::MarkerShape::Circle)
                        .filled(false)
                        .radius(5.)
                        .color(p.error),
                    Mark::Filled => shown
                        .shape(egui_plot::MarkerShape::Circle)
                        .filled(true)
                        .radius(3.)
                        .color(trace()),
                    Mark::Removed => shown
                        .shape(egui_plot::MarkerShape::Cross)
                        .radius(4.)
                        .color(p.text_muted),
                });
            }
        });
}

/// Per numeric variable: statistics before → after.
fn summary_view(ui: &mut egui::Ui, r: &Cleaned) {
    let p = crate::tokens::palette(ui.ctx());
    if r.before.is_empty() {
        crate::tokens::helper(ui, "The data has no numeric variables.");
        return;
    }
    crate::tokens::helper(
        ui,
        "Before → after the steps. Outliers by the first outlier step's rule, else by the \
         median rule.",
    );
    ui.add_space(crate::tokens::XS);
    const HEADINGS: [&str; 9] = [
        "Variable", "Count", "Missing", "Outliers", "Min", "Median", "Max", "Mean", "Std",
    ];
    egui::ScrollArea::both()
        .id_salt("data_cleaner_summary")
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            egui::Grid::new("data_cleaner_summary_grid")
                .num_columns(HEADINGS.len())
                .spacing(egui::vec2(crate::tokens::M, crate::tokens::XS))
                .striped(false)
                .show(ui, |ui| {
                    for heading in HEADINGS {
                        ui.label(egui::RichText::new(heading).color(p.text_muted));
                    }
                    ui.end_row();
                    for (name, before) in &r.before {
                        ui.label(name.as_str());
                        let after = r.after.get(name);
                        for i in 0..8 {
                            let a = short(before[i]);
                            let text = match after {
                                Some(after) if short(after[i]) != a => {
                                    format!("{a} → {}", short(after[i]))
                                }
                                Some(_) => a,
                                None => format!("{a} → removed"),
                            };
                            ui.label(text);
                        }
                        ui.end_row();
                    }
                });
        });
}
