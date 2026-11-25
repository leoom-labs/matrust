//! Outlier detection, filling and removal: isoutlier, filloutliers and
//! rmoutliers on numeric arrays, tables and timetables.
//!
//! Calling forms follow the public MathWorks reference pages; no MathWorks
//! code. Definitions (published statistics):
//! - median: outside median +- k c MAD with the normal-consistency constant
//!   c = -1/(sqrt(2) erfcinv(3/2)) (Rousseeuw & Croux, JASA 88(424), 1993); k = 3.
//! - mean: outside mean +- k std (n - 1 normalization); k = 3.
//! - quartiles: outside [Q1 - k IQR, Q3 + k IQR], k = 1.5, quartiles by the
//!   midpoint rule of quantile (Hyndman & Fan, The American Statistician
//!   50(4), 1996, definition 5); the center is (Q1 + Q3) / 2.
//! - percentiles: outside [P_lo, P_hi]; the center is their midpoint.
//! - grubbs: two-sided Grubbs test (Technometrics 11(1), 1969) removing one
//!   point per iteration while G = max|x - mean| / s exceeds
//!   (n-1)/sqrt(n) * t / sqrt(n - 2 + t^2), t the upper alpha/(2n) quantile of
//!   Student's t with n - 2 degrees of freedom (NIST/SEMATECH e-Handbook of
//!   Statistical Methods, 1.3.5.17.1); alpha = ThresholdFactor (0.05).
//! - gesd: Rosner's generalized ESD test (Technometrics 25(2), 1983; NIST
//!   1.3.5.17.3) with at most MaxNumOutliers outliers: the same critical value
//!   on the n - i + 1 points left at step i; the number of outliers is the
//!   largest i whose statistic exceeds it.
//!   Grubbs and gesd thresholds are mean +- critical * s of the points kept;
//!   the center is their mean.
//! - movmedian / movmean: the median and mean rules with local statistics over
//!   moving windows that shrink at the ends. A scalar window k covers the sample
//!   points in [t - k/2, t + k/2), a window [kb kf] covers [t - kb, t + kf];
//!   sample indices are the default sample points, the row times of a
//!   timetable its default.
//!
//! NaN is never an outlier and is left out of every statistic. Fills use the
//! non-outlier, non-NaN points; interpolating fills call interp1 with
//! extrapolation at the ends; previous/next/nearest keep an outlier that has no
//! such neighbour.
use super::{
    Engine, Execution, Value,
    stats_gaps::{self, Lanes},
    table_fns::{self, text_scalar},
};
use crate::{
    Array, LogicalMatrix, MATLAB_NAN, Matrix, NumericClass, Result, TableArray, TypedArray,
    syntax::Expr,
};

const NAMES: &[&str] = &["isoutlier", "filloutliers", "rmoutliers"];

/// Functions MATLAB ships as program files: `exist` gives 2 for them.
pub(super) const PROGRAM_FILES: &[&str] = NAMES;

/// -1/(sqrt(2) erfcinv(3/2)) = 1/Phi^-1(3/4), the MAD consistency constant.
const MAD_SCALE: f64 = 1.482_602_218_505_601_8;
/// Window element visits one moving detection may spend.
const MOVING_WORK: u128 = 1_000_000_000;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn max_outputs(name: &str) -> usize {
    match name {
        "isoutlier" => 4,
        "filloutliers" => 5,
        _ => 6,
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = max_outputs(name);
    if count > max {
        return Err(format!("{name}: too many output arguments (at most {max})"));
    }
    if args.is_empty() {
        return Err(format!("{name} requires at least one input argument"));
    }
    let (fill, spec) = parse(name, args)?;
    let mut values = match &args[0] {
        Value::Container(Array::Table(t)) => table_call(e, name, t, fill, &spec, count, out)?,
        data => array_call(e, name, data, fill, &spec, count, out)?,
    };
    values.truncate(count.max(1));
    e.check_cancel()?;
    Ok(values)
}

// ---------------------------------------------------------------- options

#[derive(Clone, Copy, PartialEq, Debug)]
enum Find {
    Median,
    Mean,
    Quartiles,
    Percentiles,
    Grubbs,
    Gesd,
    MovMedian,
    MovMean,
}

impl Find {
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "median" => Self::Median,
            "mean" => Self::Mean,
            "quartiles" => Self::Quartiles,
            "percentiles" => Self::Percentiles,
            "grubbs" => Self::Grubbs,
            "gesd" => Self::Gesd,
            "movmedian" => Self::MovMedian,
            "movmean" => Self::MovMean,
            _ => return None,
        })
    }
    fn moving(self) -> bool {
        matches!(self, Self::MovMedian | Self::MovMean)
    }
}

#[derive(Clone, Copy)]
enum Window {
    Centered(f64),
    Sides(f64, f64),
}

#[derive(Clone, Copy, PartialEq)]
enum Fill {
    Value(f64),
    Center,
    Clip,
    Previous,
    Next,
    Nearest,
    Interp(&'static str),
}

fn fill_of(name: &str) -> Option<Fill> {
    Some(match name {
        "center" => Fill::Center,
        "clip" => Fill::Clip,
        "previous" => Fill::Previous,
        "next" => Fill::Next,
        "nearest" => Fill::Nearest,
        "linear" => Fill::Interp("linear"),
        "spline" => Fill::Interp("spline"),
        "pchip" => Fill::Interp("pchip"),
        "makima" => Fill::Interp("makima"),
        _ => return None,
    })
}

struct Spec {
    find: Find,
    find_given: bool,
    /// Window and whether it is a duration (milliseconds).
    window: Option<(Window, bool)>,
    percent: [f64; 2],
    factor: Option<f64>,
    max_outliers: Option<usize>,
    dim: Option<usize>,
    samples: Option<Value>,
    data_vars: Option<Value>,
    tabular: bool,
    locations: Option<Value>,
    min_outliers: usize,
}

const METHODS: &str =
    "'median', 'mean', 'quartiles', 'grubbs', 'gesd', 'percentiles', 'movmedian' or 'movmean'";

fn option_names(name: &str) -> &'static [&'static str] {
    match name {
        "isoutlier" => &[
            "ThresholdFactor",
            "SamplePoints",
            "DataVariables",
            "OutputFormat",
            "MaxNumOutliers",
        ],
        "filloutliers" => &[
            "ThresholdFactor",
            "SamplePoints",
            "DataVariables",
            "MaxNumOutliers",
            "OutlierLocations",
            "ReplaceValues",
        ],
        _ => &[
            "ThresholdFactor",
            "SamplePoints",
            "DataVariables",
            "MaxNumOutliers",
            "OutlierLocations",
            "MinNumOutliers",
        ],
    }
}

/// The option a (case-insensitive, unambiguous prefix) text names.
fn option_of(name: &str, text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    let names = option_names(name);
    if let Some(exact) = names.iter().find(|o| o.eq_ignore_ascii_case(text)) {
        return Some(exact);
    }
    let mut hits = names
        .iter()
        .filter(|o| !lower.is_empty() && o.to_ascii_lowercase().starts_with(&lower));
    match (hits.next(), hits.next()) {
        (Some(o), None) => Some(o),
        _ => None,
    }
}

fn real_scalar(v: &Value) -> Option<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        Value::Container(Array::Numeric(m)) if m.len() == 1 => Some(m.data()[0]),
        Value::Container(Array::Typed(t)) if t.len() == 1 => {
            t.to_matrix().ok().map(|m| m.data()[0])
        }
        _ => None,
    }
}

fn is_numeric_value(v: &Value) -> bool {
    matches!(
        v,
        Value::Matrix(_) | Value::Container(Array::Numeric(_) | Array::Typed(_))
    )
}

fn window_arg(name: &str, v: &Value) -> Result<(Window, bool)> {
    let (values, duration) = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => (m.data().to_vec(), false),
        Value::Container(Array::Typed(t)) => (t.to_matrix()?.data().to_vec(), false),
        Value::Container(Array::Duration(d)) if !d.is_calendar() => (d.data().to_vec(), true),
        _ => {
            return Err(format!(
                "{name}: window must be a positive scalar or a two-element vector of nonnegative values"
            ));
        }
    };
    match values.as_slice() {
        [k] if k.is_finite() && *k > 0. => Ok((Window::Centered(*k), duration)),
        [b, f] if b.is_finite() && f.is_finite() && *b >= 0. && *f >= 0. => {
            Ok((Window::Sides(*b, *f), duration))
        }
        _ => Err(format!(
            "{name}: window must be a positive scalar or a two-element vector of nonnegative values"
        )),
    }
}

fn parse(name: &str, args: &[Value]) -> Result<(Option<Fill>, Spec)> {
    let mut spec = Spec {
        find: Find::Median,
        find_given: false,
        window: None,
        percent: [0., 100.],
        factor: None,
        max_outliers: None,
        dim: None,
        samples: None,
        data_vars: None,
        tabular: false,
        locations: None,
        min_outliers: 1,
    };
    let mut at = 1;
    let mut fill = None;
    if name == "filloutliers" {
        let Some(v) = args.get(1) else {
            return Err("filloutliers requires a fill method".into());
        };
        fill = Some(if let Some(t) = text_scalar(v) {
            fill_of(&t.to_ascii_lowercase()).ok_or_else(|| {
                format!("filloutliers: unknown fill method '{t}'; use a numeric scalar, 'center', 'clip', 'previous', 'next', 'nearest', 'linear', 'spline', 'pchip' or 'makima'")
            })?
        } else if let Some(x) = real_scalar(v)
            && is_numeric_value(v)
        {
            Fill::Value(x)
        } else {
            return Err(
                "filloutliers: the fill method must be a numeric scalar or a method name".into(),
            );
        });
        at = 2;
    }
    // Detection method (and its threshold or window).
    if let Some(t) = args.get(at).and_then(text_scalar)
        && option_of(name, &t).is_none()
    {
        let lower = t.to_ascii_lowercase();
        let find = Find::of(&lower)
            .ok_or_else(|| format!("{name}: unknown method '{t}'; use {METHODS}"))?;
        spec.find = find;
        spec.find_given = true;
        at += 1;
        if find == Find::Percentiles {
            let p = match args.get(at) {
                Some(Value::Matrix(m)) if m.len() == 2 => [m.data()[0], m.data()[1]],
                _ => {
                    return Err(format!(
                        "{name}: 'percentiles' needs a threshold [lower upper] within 0..100"
                    ));
                }
            };
            if !(p[0] >= 0. && p[1] <= 100. && p[0] <= p[1]) {
                return Err(format!(
                    "{name}: 'percentiles' needs a threshold [lower upper] within 0..100"
                ));
            }
            spec.percent = p;
            at += 1;
        } else if find.moving() {
            let Some(v) = args.get(at) else {
                return Err(format!("{name}: '{lower}' requires a window length"));
            };
            spec.window = Some(window_arg(name, v)?);
            at += 1;
        }
    }
    // Operating dimension.
    if let Some(v) = args.get(at)
        && is_numeric_value(v)
    {
        let d = real_scalar(v).filter(|d| *d >= 1. && d.fract() == 0. && *d <= 1e9);
        spec.dim = Some(
            d.ok_or_else(|| format!("{name}: dimension must be a positive integer scalar"))?
                as usize,
        );
        at += 1;
    }
    // Name-value pairs.
    while at < args.len() {
        let Some(key) = text_scalar(&args[at]) else {
            return Err(format!(
                "{name}: expected a Name,Value pair at argument {}",
                at + 1
            ));
        };
        let Some(option) = option_of(name, &key) else {
            return Err(format!("{name}: unknown option '{key}'"));
        };
        let Some(value) = args.get(at + 1) else {
            return Err(format!("{name}: option '{option}' needs a value"));
        };
        match option {
            "ThresholdFactor" => {
                spec.factor = Some(
                    real_scalar(value)
                        .filter(|x| *x >= 0. && x.is_finite())
                        .ok_or_else(|| {
                            format!("{name}: ThresholdFactor must be a nonnegative real scalar")
                        })?,
                );
            }
            "MaxNumOutliers" => {
                let x = real_scalar(value)
                    .filter(|x| *x >= 1. && x.fract() == 0. && *x <= 1e12)
                    .ok_or_else(|| {
                        format!("{name}: MaxNumOutliers must be a positive integer scalar")
                    })?;
                spec.max_outliers = Some(x as usize);
            }
            "MinNumOutliers" => {
                let x = real_scalar(value)
                    .filter(|x| *x >= 1. && x.fract() == 0. && *x <= 1e12)
                    .ok_or_else(|| {
                        format!("{name}: MinNumOutliers must be a positive integer scalar")
                    })?;
                spec.min_outliers = x as usize;
            }
            "OutputFormat" => {
                spec.tabular = match text_scalar(value)
                    .map(|t| t.to_ascii_lowercase())
                    .as_deref()
                {
                    Some("logical") => false,
                    Some("tabular") => true,
                    _ => {
                        return Err(format!(
                            "{name}: OutputFormat must be 'logical' or 'tabular'"
                        ));
                    }
                };
            }
            "ReplaceValues" => {
                if real_scalar(value) != Some(1.)
                    && !matches!(value, Value::Logical(l) if l.len() == 1 && l.data()[0] != 0)
                {
                    return Err(format!(
                        "{name}: ReplaceValues false (appending filled variables) is not supported yet"
                    ));
                }
            }
            "SamplePoints" => spec.samples = Some(value.clone()),
            "DataVariables" => spec.data_vars = Some(value.clone()),
            _ => spec.locations = Some(value.clone()),
        }
        at += 2;
    }
    // Consistency of the options with the method.
    if let Some(k) = spec.factor {
        match spec.find {
            Find::Percentiles => {
                return Err(format!(
                    "{name}: ThresholdFactor does not apply to the 'percentiles' method"
                ));
            }
            Find::Grubbs | Find::Gesd if k > 1. => {
                return Err(format!(
                    "{name}: ThresholdFactor of 'grubbs' and 'gesd' must be between 0 and 1"
                ));
            }
            _ => {}
        }
    }
    if spec.max_outliers.is_some() && spec.find != Find::Gesd {
        return Err(format!(
            "{name}: MaxNumOutliers applies only to the 'gesd' method"
        ));
    }
    if spec.locations.is_some() {
        if spec.find_given || spec.factor.is_some() || spec.max_outliers.is_some() {
            return Err(format!(
                "{name}: OutlierLocations cannot be combined with a detection method or its options"
            ));
        }
        if matches!(fill, Some(Fill::Center | Fill::Clip)) {
            return Err(
                "filloutliers: 'center' and 'clip' need a detection method, not OutlierLocations"
                    .into(),
            );
        }
    }
    Ok((fill, spec))
}

// ---------------------------------------------------------------- data

/// Real double data of an array argument and its class (None for double).
fn numeric_data(name: &str, v: &Value) -> Result<(Matrix, Option<NumericClass>)> {
    let (m, class) = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => (m.clone(), None),
        Value::Container(Array::Typed(t)) => (t.to_matrix()?, Some(t.class())),
        Value::Complex(_) | Value::Container(Array::Complex(_)) => {
            return Err(format!("{name} of complex data is not supported yet"));
        }
        Value::Container(Array::Sparse(_)) => {
            return Err(format!("{name} of sparse data is not supported yet"));
        }
        _ => {
            return Err(format!(
                "{name}: input data must be a numeric array, a table or a timetable"
            ));
        }
    };
    if m.is_nd() {
        return Err(format!(
            "{name} of arrays with more than two dimensions is not supported yet"
        ));
    }
    Ok((m, class))
}

fn time_values(a: &Array) -> Option<Vec<f64>> {
    match a {
        Array::Datetime(d) => {
            let first = d.data().iter().find_map(|v| v.total_nanos()).unwrap_or(0);
            Some(
                d.data()
                    .iter()
                    .map(|v| {
                        v.total_nanos()
                            .map_or(f64::NAN, |n| (n - first) as f64 / 1e6)
                    })
                    .collect(),
            )
        }
        Array::Duration(d) if !d.is_calendar() => Some(d.data().to_vec()),
        _ => None,
    }
}

/// Sample points (and whether they are times in milliseconds) for `n`
/// samples, validated; None for the default sample indices.
fn sample_points(
    name: &str,
    spec: &Spec,
    n: usize,
    table: Option<&TableArray>,
) -> Result<Option<(Vec<f64>, bool)>> {
    let points = match &spec.samples {
        Some(v) => {
            let v = match (table, text_scalar(v)) {
                (Some(t), Some(var)) => {
                    let index = t.index_of(&var).ok_or_else(|| {
                        format!("{name}: SamplePoints names no table variable '{var}'")
                    })?;
                    Value::Container(t.column(index).clone())
                }
                _ => v.clone(),
            };
            let (values, time, shape) = match &v {
                Value::Matrix(m) | Value::Container(Array::Numeric(m)) => {
                    (m.data().to_vec(), false, (m.rows(), m.cols()))
                }
                Value::Container(a @ (Array::Datetime(_) | Array::Duration(_))) => (
                    time_values(a).ok_or_else(|| {
                        format!("{name}: calendar duration SamplePoints are not supported")
                    })?,
                    true,
                    (a.rows(), a.cols()),
                ),
                _ => {
                    return Err(format!(
                        "{name}: SamplePoints must be a numeric, datetime or duration vector"
                    ));
                }
            };
            if shape.0 != 1 && shape.1 != 1 {
                return Err(format!("{name}: SamplePoints must be a vector"));
            }
            (values, time)
        }
        None => match table.and_then(TableArray::row_times) {
            Some(times) => (
                time_values(times).ok_or_else(|| format!("{name}: unsupported row times"))?,
                true,
            ),
            None => return Ok(None),
        },
    };
    if points.0.len() != n {
        return Err(format!(
            "{name}: SamplePoints must have one value per element along the operating dimension ({n})"
        ));
    }
    if points.0.iter().any(|v| !v.is_finite()) || points.0.windows(2).any(|p| p[1] <= p[0]) {
        return Err(format!(
            "{name}: SamplePoints must be finite, sorted and unique"
        ));
    }
    Ok(Some(points))
}

// ---------------------------------------------------------------- detection

#[derive(Clone, Copy)]
struct Setup {
    find: Find,
    factor: f64,
    percent: [f64; 2],
    window: Option<Window>,
    max_outliers: Option<usize>,
}

fn setup(name: &str, spec: &Spec, time: bool) -> Result<Setup> {
    let window = match spec.window {
        Some((w, duration)) => {
            if duration && !time {
                return Err(format!(
                    "{name}: a duration window needs datetime or duration sample points"
                ));
            }
            if time && !duration {
                return Err(format!(
                    "{name}: the window must be a duration for datetime or duration sample points"
                ));
            }
            Some(w)
        }
        None => None,
    };
    Ok(Setup {
        find: spec.find,
        factor: spec.factor.unwrap_or(match spec.find {
            Find::Quartiles => 1.5,
            Find::Grubbs | Find::Gesd => 0.05,
            _ => 3.,
        }),
        percent: spec.percent,
        window,
        max_outliers: spec.max_outliers,
    })
}

/// Outlier flags of one lane and its thresholds and centers (one value for
/// whole-lane methods, one per element for moving methods).
struct Lane {
    tf: Vec<bool>,
    lo: Vec<f64>,
    hi: Vec<f64>,
    center: Vec<f64>,
}

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_unstable_by(f64::total_cmp);
    v
}

/// Median of NaN-free values by selection (O(n); the slice is reordered).
fn median_of(v: &mut [f64]) -> f64 {
    let n = v.len();
    if n == 0 {
        return MATLAB_NAN;
    }
    let (lower, upper, _) = v.select_nth_unstable_by(n / 2, f64::total_cmp);
    let upper = *upper;
    if n % 2 == 1 {
        return upper;
    }
    let lower = lower.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    stats_gaps::median_sorted(&[lower, upper])
}

/// Median and scaled MAD of NaN-free values (the slice is reused).
fn median_mad(v: &mut [f64]) -> (f64, f64) {
    let m = median_of(v);
    for x in v.iter_mut() {
        *x = (*x - m).abs();
    }
    (m, MAD_SCALE * median_of(v))
}

/// Critical value (n-1)/sqrt(n) * t / sqrt(n - 2 + t^2) of the Grubbs and
/// generalized ESD tests on n points (its limit (n-1)/sqrt(n) below 3 points).
fn critical(n: usize, alpha: f64) -> f64 {
    if n == 0 {
        return MATLAB_NAN;
    }
    let nf = n as f64;
    let bound = (nf - 1.) / nf.sqrt();
    if n < 3 {
        return bound;
    }
    let t = -super::stats_dist::families::t_quantile(alpha / (2. * nf), nf - 2.);
    if t.is_infinite() {
        return bound;
    }
    bound / (1. + (nf - 2.) / (t * t)).sqrt()
}

/// [lo, hi) sample ranges of the moving windows.
fn window_ranges(n: usize, t: Option<&[f64]>, w: Window) -> Vec<(usize, usize)> {
    let at = |j: usize| t.map_or(j as f64, |t| t[j]);
    let mut ranges = Vec::with_capacity(n);
    let (mut lo, mut hi) = (0, 0);
    for i in 0..n {
        let ti = at(i);
        let (start, end_inclusive, end) = match w {
            Window::Centered(k) => (ti - k / 2., false, ti + k / 2.),
            Window::Sides(b, f) => (ti - b, true, ti + f),
        };
        while lo < n && at(lo) < start {
            lo += 1;
        }
        hi = hi.max(lo);
        while hi < n && (at(hi) < end || (end_inclusive && at(hi) == end)) {
            hi += 1;
        }
        ranges.push((lo, hi.max(i + 1).min(n)));
    }
    ranges
}

/// Grubbs (`limit` None: remove while the test rejects) or generalized ESD
/// (`limit` tests; the outliers are the points up to the last rejection) on
/// NaN-free values: outlier positions, then the thresholds mean -+ critical *
/// std and the mean of the points kept.
///
/// The most deviant point is always the smallest or the largest left, so two
/// sorted orders (ties: first position) give each step in O(1) with running
/// sums about a shift, summed again exactly whenever their rounding error
/// could reach about 1e-13 of the variance.
fn esd(
    e: &Engine,
    values: &[f64],
    alpha: f64,
    limit: Option<usize>,
) -> Result<(Vec<usize>, f64, f64, f64)> {
    let n = values.len();
    let mut up: Vec<usize> = (0..n).collect();
    up.sort_unstable_by(|&a, &b| values[a].total_cmp(&values[b]).then(a.cmp(&b)));
    let mut down = up.clone();
    down.sort_unstable_by(|&a, &b| values[b].total_cmp(&values[a]).then(a.cmp(&b)));
    let mut gone = vec![false; n];
    let (mut lo, mut hi) = (0, 0);
    let sums = |gone: &[bool]| {
        let kept = values
            .iter()
            .zip(gone)
            .filter(|(_, g)| !**g)
            .map(|(v, _)| *v);
        let count = gone.iter().filter(|g| !**g).count();
        let shift = kept.clone().sum::<f64>() / count.max(1) as f64;
        let (s1, s2) = kept.fold((0., 0.), |(a, b), v| {
            (a + (v - shift), b + (v - shift) * (v - shift))
        });
        (shift, s1, s2)
    };
    let (mut shift, mut s1, mut s2) = sums(&gone);
    // s2 at the last exact summation: the rounding error of the running
    // variance numerator is about eps times this.
    let mut scale = s2;
    let mut removed = Vec::new();
    let mut outliers = 0;
    let mut m = n;
    while limit.is_none_or(|l| removed.len() < l) && m >= 3 {
        stats_gaps::poll(e, removed.len())?;
        let mf = m as f64;
        let mean = shift + s1 / mf;
        let sd = ((s2 - s1 * s1 / mf).max(0.) / (mf - 1.)).sqrt();
        // Equal (or infinite) values: no point stands out.
        if sd.is_nan() || sd <= 0. {
            break;
        }
        while gone[up[lo]] {
            lo += 1;
        }
        while gone[down[hi]] {
            hi += 1;
        }
        let (a, b) = (up[lo], down[hi]);
        let (da, db) = ((values[a] - mean).abs(), (values[b] - mean).abs());
        let j = if da > db || (da == db && a < b) { a } else { b };
        let exceeds = da.max(db) / sd > critical(m, alpha);
        if limit.is_none() && !exceeds {
            break;
        }
        gone[j] = true;
        removed.push(j);
        m -= 1;
        let d = values[j] - shift;
        s1 -= d;
        s2 -= d * d;
        // Resum exactly once the error bound eps * scale could exceed about
        // 1e-13 of the variance numerator (dominant points removed, or the
        // mean drifted far from the shift).
        let bound = 1e3 * (s2 - s1 * s1 / m as f64);
        if scale > bound || bound.is_nan() {
            e.check_cancel()?;
            (shift, s1, s2) = sums(&gone);
            scale = s2;
        }
        if exceeds {
            outliers = removed.len();
        }
    }
    if limit.is_none() {
        outliers = removed.len();
    }
    removed.truncate(outliers);
    let mut keep = vec![true; n];
    for &j in &removed {
        keep[j] = false;
    }
    let rest: Vec<f64> = (0..n).filter(|&j| keep[j]).map(|j| values[j]).collect();
    let mean = stats_gaps::mean(&rest);
    let sd = stats_gaps::std_dev(&rest, false);
    let c = critical(rest.len(), alpha);
    Ok((removed, mean - c * sd, mean + c * sd, mean))
}

fn detect(e: &Engine, x: &[f64], t: Option<&[f64]>, s: &Setup) -> Result<Lane> {
    let n = x.len();
    let k = s.factor;
    if s.find.moving() {
        let w = s.window.expect("moving methods have a window");
        let ranges = window_ranges(n, t, w);
        let visits: u128 = ranges.iter().map(|(a, b)| (b - a) as u128).sum();
        if visits > MOVING_WORK {
            return Err("Moving outlier detection: the window is too large for this much data; use a smaller window or less data".into());
        }
        let mut lane = Lane {
            tf: vec![false; n],
            lo: vec![MATLAB_NAN; n],
            hi: vec![MATLAB_NAN; n],
            center: vec![MATLAB_NAN; n],
        };
        let mut window = Vec::new();
        for (i, &(a, b)) in ranges.iter().enumerate() {
            stats_gaps::poll(e, i)?;
            window.clear();
            window.extend(x[a..b].iter().copied().filter(|v| !v.is_nan()));
            if window.is_empty() {
                continue;
            }
            let (c, scale) = if s.find == Find::MovMedian {
                median_mad(&mut window)
            } else {
                (
                    stats_gaps::mean(&window),
                    stats_gaps::std_dev(&window, false),
                )
            };
            lane.center[i] = c;
            lane.lo[i] = c - k * scale;
            lane.hi[i] = c + k * scale;
            lane.tf[i] = x[i] < lane.lo[i] || x[i] > lane.hi[i];
        }
        return Ok(lane);
    }
    let valid: Vec<usize> = (0..n).filter(|&i| !x[i].is_nan()).collect();
    let mut values: Vec<f64> = valid.iter().map(|&i| x[i]).collect();
    let mut tf = vec![false; n];
    let (lo, hi, center) = match s.find {
        Find::Median => {
            let (m, scale) = median_mad(&mut values);
            (m - k * scale, m + k * scale, m)
        }
        Find::Mean => {
            let m = stats_gaps::mean(&values);
            let sd = stats_gaps::std_dev(&values, false);
            (m - k * sd, m + k * sd, m)
        }
        Find::Quartiles | Find::Percentiles => {
            let sv = sorted(values);
            let nf = sv.len() as f64;
            let at = |q: f64| stats_gaps::midpoint_at(&sv, nf * q + 0.5);
            let (l, u) = if s.find == Find::Quartiles {
                let (q1, q3) = (at(0.25), at(0.75));
                (q1 - k * (q3 - q1), q3 + k * (q3 - q1))
            } else {
                (at(s.percent[0] / 100.), at(s.percent[1] / 100.))
            };
            (l, u, l / 2. + u / 2.)
        }
        _ => {
            // Grubbs (unbounded iterations) or generalized ESD (bounded).
            let limit = match s.find {
                Find::Grubbs => None,
                _ => Some(
                    s.max_outliers
                        .unwrap_or_else(|| ((0.1 * n as f64).round() as usize).max(1)),
                ),
            };
            let (outliers, lo, hi, center) = esd(e, &values, k, limit)?;
            for j in outliers {
                tf[valid[j]] = true;
            }
            return Ok(Lane {
                tf,
                lo: vec![lo],
                hi: vec![hi],
                center: vec![center],
            });
        }
    };
    for &i in &valid {
        tf[i] = x[i] < lo || x[i] > hi;
    }
    Ok(Lane {
        tf,
        lo: vec![lo],
        hi: vec![hi],
        center: vec![center],
    })
}

/// Detection on every lane of a matrix: flags (size of the matrix) and the
/// lower, upper and center matrices.
struct Detected {
    tf: Vec<bool>,
    lo: Matrix,
    hi: Matrix,
    center: Matrix,
    /// One statistic per element (moving methods) rather than per lane.
    moving: bool,
}

fn detect_matrix(
    e: &Engine,
    m: &Matrix,
    dim: usize,
    t: Option<&[f64]>,
    s: &Setup,
) -> Result<Detected> {
    let lanes = Lanes::new(m, dim);
    let len = lanes.len();
    let stat_len = if s.find.moving() { len } else { 1 };
    let (rows, cols) = lanes.shape(stat_len);
    let total = Matrix::checked_len(rows, cols)?;
    stats_gaps::budget(e, m.len().saturating_add(3 * total))?;
    let out_lanes = Lanes { rows, cols, dim };
    let mut tf = vec![false; m.len()];
    let (mut lo, mut hi, mut center) = (
        vec![MATLAB_NAN; total],
        vec![MATLAB_NAN; total],
        vec![MATLAB_NAN; total],
    );
    let mut lane = Vec::with_capacity(len);
    for l in 0..lanes.count() {
        stats_gaps::poll(e, l)?;
        lanes.lane(m.data(), l, &mut lane);
        let r = detect(e, &lane, t, s)?;
        for (i, &flag) in r.tf.iter().enumerate() {
            tf[lanes.index(l, i, len)] = flag;
        }
        for i in 0..stat_len {
            let at = out_lanes.index(l, i, stat_len);
            lo[at] = r.lo[i];
            hi[at] = r.hi[i];
            center[at] = r.center[i];
        }
    }
    Ok(Detected {
        tf,
        lo: Matrix::from_column_vec(rows, cols, lo)?,
        hi: Matrix::from_column_vec(rows, cols, hi)?,
        center: Matrix::from_column_vec(rows, cols, center)?,
        moving: s.find.moving(),
    })
}

// ---------------------------------------------------------------- filling

/// Fill the flagged elements of one lane in place.
#[allow(clippy::too_many_arguments)]
fn fill_lane(
    e: &mut Engine,
    x: &mut [f64],
    tf: &[bool],
    t: Option<&[f64]>,
    lo: &dyn Fn(usize) -> f64,
    hi: &dyn Fn(usize) -> f64,
    center: &dyn Fn(usize) -> f64,
    fill: Fill,
    out: &mut Execution,
) -> Result<()> {
    let n = x.len();
    let at = |j: usize| t.map_or(j as f64 + 1., |t| t[j]);
    let known: Vec<usize> = (0..n).filter(|&j| !tf[j] && !x[j].is_nan()).collect();
    let targets: Vec<usize> = (0..n).filter(|&j| tf[j]).collect();
    if targets.is_empty() {
        return Ok(());
    }
    match fill {
        Fill::Value(v) => targets.iter().for_each(|&i| x[i] = v),
        Fill::Center => targets.iter().for_each(|&i| x[i] = center(i)),
        Fill::Clip => {
            for &i in &targets {
                if x[i] < lo(i) {
                    x[i] = lo(i);
                } else if x[i] > hi(i) {
                    x[i] = hi(i);
                }
            }
        }
        Fill::Previous | Fill::Next | Fill::Nearest => {
            let original = x.to_vec();
            for &i in &targets {
                // Position of the first known sample after i.
                let p = known.partition_point(|&j| j < i);
                let prev = p.checked_sub(1).map(|q| known[q]);
                let next = known.get(p).copied();
                let from = match fill {
                    Fill::Previous => prev,
                    Fill::Next => next,
                    _ => match (prev, next) {
                        (Some(a), Some(b)) => {
                            Some(if at(b) - at(i) <= at(i) - at(a) { b } else { a })
                        }
                        (a, b) => a.or(b),
                    },
                };
                if let Some(j) = from {
                    x[i] = original[j];
                }
            }
        }
        Fill::Interp(method) => {
            if known.len() < 2 {
                return Ok(());
            }
            let column = |v: Vec<f64>| -> Result<Value> {
                Ok(Value::Matrix(Matrix::from_column_vec(v.len(), 1, v)?))
            };
            let result = e.call(
                "interp1",
                vec![
                    column(known.iter().map(|&j| at(j)).collect())?,
                    column(known.iter().map(|&j| x[j]).collect())?,
                    column(targets.iter().map(|&j| at(j)).collect())?,
                    Value::Text(method.into()),
                    Value::Text("extrap".into()),
                ],
                out,
            )?;
            let Value::Matrix(values) = result else {
                return Err("interp1 returned a non-numeric result".into());
            };
            for (&i, &v) in targets.iter().zip(values.data()) {
                x[i] = v;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn fill_matrix(
    e: &mut Engine,
    m: &Matrix,
    dim: usize,
    t: Option<&[f64]>,
    tf: &[bool],
    d: Option<&Detected>,
    fill: Fill,
    out: &mut Execution,
) -> Result<Matrix> {
    let lanes = Lanes::new(m, dim);
    let len = lanes.len();
    let mut data = m.data().to_vec();
    let mut lane = Vec::with_capacity(len);
    let mut flags = Vec::with_capacity(len);
    for l in 0..lanes.count() {
        stats_gaps::poll(e, l)?;
        lanes.lane(m.data(), l, &mut lane);
        flags.clear();
        flags.extend((0..len).map(|i| tf[lanes.index(l, i, len)]));
        if !flags.iter().any(|&f| f) {
            continue;
        }
        let stat = |which: fn(&Detected) -> &Matrix| {
            move |i: usize| -> f64 {
                let Some(d) = d else {
                    return MATLAB_NAN;
                };
                let s = which(d);
                let (stat_len, at) = if d.moving { (len, i) } else { (1, 0) };
                let index = Lanes {
                    rows: s.rows(),
                    cols: s.cols(),
                    dim,
                }
                .index(l, at, stat_len);
                s.data()[index]
            }
        };
        let (lo, hi, center) = (stat(|d| &d.lo), stat(|d| &d.hi), stat(|d| &d.center));
        fill_lane(e, &mut lane, &flags, t, &lo, &hi, &center, fill, out)?;
        for (i, &v) in lane.iter().enumerate() {
            data[lanes.index(l, i, len)] = v;
        }
    }
    Matrix::from_column_vec(m.rows(), m.cols(), data)
}

// ---------------------------------------------------------------- outputs

fn logical(e: &Engine, rows: usize, cols: usize, flags: &[bool]) -> Result<Value> {
    Ok(Value::Logical(LogicalMatrix::from_column_vec(
        rows,
        cols,
        flags.iter().map(|&b| u8::from(b)).collect(),
        &e.cancel,
    )?))
}

/// A double result in the class of the data (single stays single; integer
/// data keep double statistics).
fn stat_value(m: Matrix, class: Option<NumericClass>) -> Result<Value> {
    Ok(match class {
        Some(NumericClass::Single) => Value::Container(Array::Typed(TypedArray::from_matrix(
            NumericClass::Single,
            &m,
        )?)),
        _ => Value::Matrix(m),
    })
}

fn data_value(m: Matrix, class: Option<NumericClass>) -> Result<Value> {
    Ok(match class {
        Some(c) => Value::Container(Array::Typed(TypedArray::from_matrix(c, &m)?)),
        None => Value::Matrix(m),
    })
}

fn locations_flags(name: &str, v: &Value, rows: usize, cols: usize) -> Result<Vec<bool>> {
    let (r, c, flags): (usize, usize, Vec<bool>) = match v {
        Value::Logical(l) => (
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| b != 0).collect(),
        ),
        Value::Matrix(m) if m.data().iter().all(|&x| x == 0. || x == 1.) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|&x| x != 0.).collect(),
        ),
        _ => {
            return Err(format!(
                "{name}: OutlierLocations must be a logical array the size of the data"
            ));
        }
    };
    if (r, c) != (rows, cols) {
        return Err(format!(
            "{name}: OutlierLocations must be a logical array the size of the data ({rows}x{cols})"
        ));
    }
    Ok(flags)
}

/// Lines (rows for dim 1, columns for dim 2) holding at least `min` flags.
fn removed_lines(tf: &[bool], rows: usize, cols: usize, dim: usize, min: usize) -> Vec<bool> {
    let lines = if dim == 1 { rows } else { cols };
    let mut counts = vec![0usize; lines];
    for (i, &f) in tf.iter().enumerate() {
        if f {
            counts[if dim == 1 { i % rows } else { i / rows }] += 1;
        }
    }
    counts.iter().map(|&c| c >= min).collect()
}

/// `A` without the lines marked `gone` along `dim`, in A's own class.
fn remove_lines(
    e: &mut Engine,
    a: &Value,
    gone: &[bool],
    dim: usize,
    out: &mut Execution,
) -> Result<Value> {
    if !gone.iter().any(|&g| g) {
        return Ok(a.clone());
    }
    let keep: Vec<usize> = (0..gone.len()).filter(|&i| !gone[i]).collect();
    let picked = if keep.is_empty() {
        (0..gone.len()).collect()
    } else {
        keep.clone()
    };
    let subs = if dim == 1 {
        vec![table_fns::row_expr(&picked), Expr::All]
    } else {
        vec![Expr::All, table_fns::row_expr(&picked)]
    };
    if keep.is_empty() {
        // `A(all, :) = []` keeps the class and the other dimension.
        let array = a.clone().array(&e.cancel)?;
        return Ok(Value::from(e.tbl_write(array, subs, None, out)?));
    }
    e.index_value(a.clone(), &subs, out)
}

fn array_call(
    e: &mut Engine,
    name: &str,
    data: &Value,
    fill: Option<Fill>,
    spec: &Spec,
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let (m, class) = numeric_data(name, data)?;
    if spec.data_vars.is_some() {
        return Err(format!("{name}: DataVariables needs a table or timetable"));
    }
    if spec.tabular {
        return Err(format!(
            "{name}: OutputFormat 'tabular' needs a table or timetable"
        ));
    }
    let dim = spec.dim.unwrap_or(if m.rows() != 1 { 1 } else { 2 });
    if name == "rmoutliers" && dim > 2 {
        return Err("rmoutliers: dimension must be 1 or 2".into());
    }
    let len = Lanes::new(&m, dim).len();
    let points = sample_points(name, spec, len, None)?;
    let t = points.as_ref().map(|p| p.0.as_slice());
    let s = setup(name, spec, points.as_ref().is_some_and(|p| p.1))?;
    if spec.find.moving() && s.window.is_none() {
        return Err(format!("{name}: moving methods require a window"));
    }
    let (tf, detected) = match &spec.locations {
        Some(v) => (locations_flags(name, v, m.rows(), m.cols())?, None),
        None => {
            let d = detect_matrix(e, &m, dim, t, &s)?;
            (d.tf.clone(), Some(d))
        }
    };
    let stats = |d: Option<Detected>, wanted: usize| -> Result<Vec<Value>> {
        match d {
            Some(d) => Ok(vec![
                stat_value(d.lo, class)?,
                stat_value(d.hi, class)?,
                stat_value(d.center, class)?,
            ]),
            None if wanted > 0 => Err(format!(
                "{name}: thresholds and centers are not available with OutlierLocations"
            )),
            None => Ok(vec![]),
        }
    };
    match name {
        "isoutlier" => {
            let mut values = vec![logical(e, m.rows(), m.cols(), &tf)?];
            values.extend(stats(detected, count.saturating_sub(1))?);
            Ok(values)
        }
        "filloutliers" => {
            let filled = fill_matrix(
                e,
                &m,
                dim,
                t,
                &tf,
                detected.as_ref(),
                fill.expect("filloutliers has a fill method"),
                out,
            )?;
            let mut values = vec![
                data_value(filled, class)?,
                logical(e, m.rows(), m.cols(), &tf)?,
            ];
            values.extend(stats(detected, count.saturating_sub(2))?);
            Ok(values)
        }
        _ => {
            let gone = removed_lines(&tf, m.rows(), m.cols(), dim, spec.min_outliers);
            let b = remove_lines(e, data, &gone, dim, out)?;
            let (r, c) = if dim == 1 {
                (m.rows(), 1)
            } else {
                (1, m.cols())
            };
            let mut values = vec![
                b,
                logical(e, r, c, &gone)?,
                logical(e, m.rows(), m.cols(), &tf)?,
            ];
            values.extend(stats(detected, count.saturating_sub(3))?);
            Ok(values)
        }
    }
}

// ---------------------------------------------------------------- tables

/// Data variables: DataVariables (names, indices, logical mask) or every
/// numeric variable but a SamplePoints variable.
fn data_variables(e: &Engine, name: &str, t: &TableArray, spec: &Spec) -> Result<Vec<usize>> {
    let numeric = |a: &Array| matches!(a, Array::Numeric(_) | Array::Typed(_));
    let vars: Vec<usize> = match &spec.data_vars {
        None => {
            // A variable named as SamplePoints is not data.
            let points = spec
                .samples
                .as_ref()
                .and_then(text_scalar)
                .and_then(|n| t.index_of(&n));
            (0..t.width())
                .filter(|&v| numeric(t.column(v)) && Some(v) != points)
                .collect()
        }
        Some(v @ (Value::Matrix(_) | Value::Logical(_))) => {
            let flags: Vec<usize> = match v {
                Value::Logical(l) => {
                    if l.len() != t.width() {
                        return Err(format!(
                            "{name}: a logical DataVariables needs one value per table variable"
                        ));
                    }
                    (0..l.len()).filter(|&i| l.data()[i] != 0).collect()
                }
                Value::Matrix(m) => m
                    .data()
                    .iter()
                    .map(|&x| {
                        if x >= 1. && x.fract() == 0. && x <= t.width() as f64 {
                            Ok(x as usize - 1)
                        } else {
                            Err(format!("{name}: DataVariables index {x} is out of range"))
                        }
                    })
                    .collect::<Result<_>>()?,
                _ => unreachable!(),
            };
            flags
        }
        Some(Value::Container(Array::Handle(_))) => {
            return Err(format!(
                "{name}: DataVariables as a function handle is not supported yet"
            ));
        }
        Some(v) => {
            let names = table_fns::text_list(e, v, "DataVariables")?
                .ok_or_else(|| format!("{name}: DataVariables must name table variables"))?;
            names
                .iter()
                .map(|n| {
                    t.index_of(n)
                        .ok_or_else(|| format!("{name}: no table variable named '{n}'"))
                })
                .collect::<Result<_>>()?
        }
    };
    for &v in &vars {
        let column = t.column(v);
        if !numeric(column) {
            return Err(format!(
                "{name}: table variable '{}' is not numeric",
                t.names()[v]
            ));
        }
        if column.cols() != 1 {
            return Err(format!(
                "{name}: multicolumn table variable '{}' is not supported yet",
                t.names()[v]
            ));
        }
    }
    Ok(vars)
}

fn column_matrix(name: &str, a: &Array) -> Result<(Matrix, Option<NumericClass>)> {
    numeric_data(name, &Value::Container(a.clone()))
}

fn table_call(
    e: &mut Engine,
    name: &str,
    t: &TableArray,
    fill: Option<Fill>,
    spec: &Spec,
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if spec.dim.is_some_and(|d| d != 1) {
        return Err(format!("{name}: for tables only dimension 1 is supported"));
    }
    if spec.tabular && name != "isoutlier" {
        return Err(format!("{name}: unknown option 'OutputFormat'"));
    }
    let vars = data_variables(e, name, t, spec)?;
    let h = t.height();
    let points = sample_points(name, spec, h, Some(t))?;
    let tp = points.as_ref().map(|p| p.0.as_slice());
    let s = setup(name, spec, points.as_ref().is_some_and(|p| p.1))?;
    if spec.find.moving() && s.window.is_none() {
        return Err(format!("{name}: moving methods require a window"));
    }
    let w = t.width();
    // Flags h x w (non-data variables false) and per-variable detections.
    let mut tf = vec![false; h * w];
    if let Some(v) = &spec.locations {
        let flags = locations_flags(name, v, h, w)?;
        for &var in &vars {
            tf[var * h..(var + 1) * h].copy_from_slice(&flags[var * h..(var + 1) * h]);
        }
    }
    let mut detections = Vec::with_capacity(vars.len());
    let mut filled_columns = Vec::new();
    for &var in &vars {
        let (m, class) = column_matrix(name, t.column(var))?;
        let d = if spec.locations.is_none() {
            let d = detect_matrix(e, &m, 1, tp, &s)?;
            tf[var * h..(var + 1) * h].copy_from_slice(&d.tf);
            Some(d)
        } else {
            None
        };
        if let Some(fill) = fill {
            let filled = fill_matrix(
                e,
                &m,
                1,
                tp,
                &tf[var * h..(var + 1) * h],
                d.as_ref(),
                fill,
                out,
            )?;
            filled_columns.push((var, data_value(filled, class)?));
        }
        detections.push((d, class));
    }
    let wanted_stats = count.saturating_sub(match name {
        "isoutlier" => 1,
        "filloutliers" => 2,
        _ => 3,
    });
    let stats = if wanted_stats > 0 {
        if spec.locations.is_some() {
            return Err(format!(
                "{name}: thresholds and centers are not available with OutlierLocations"
            ));
        }
        let moving = spec.find.moving();
        let mut tables = Vec::with_capacity(3);
        for which in 0..3 {
            let mut parts = t.parts().select_variables(&vars);
            for (k, (d, class)) in detections.iter().enumerate() {
                let d = d.as_ref().expect("detected");
                let m = [&d.lo, &d.hi, &d.center][which].clone();
                parts.columns[k] = stat_value(m, *class)?.array(&e.cancel)?;
            }
            if !moving {
                parts.height = 1;
                parts.row_names = None;
                parts.row_times = None;
            }
            tables.push(Value::Container(Array::Table(TableArray::new(parts)?)));
        }
        tables
    } else {
        vec![]
    };
    let flags_value = |e: &Engine, tabular: bool| -> Result<Value> {
        if !tabular {
            return logical(e, h, w, &tf);
        }
        let mut parts = t.to_parts();
        for v in 0..w {
            parts.columns[v] = Array::Logical(LogicalMatrix::from_column_vec(
                h,
                1,
                tf[v * h..(v + 1) * h]
                    .iter()
                    .map(|&b| u8::from(b))
                    .collect(),
                &e.cancel,
            )?);
        }
        Ok(Value::Container(Array::Table(TableArray::new(parts)?)))
    };
    let mut values = match name {
        "isoutlier" => vec![flags_value(e, spec.tabular)?],
        "filloutliers" => {
            let mut parts = t.to_parts();
            for (var, column) in filled_columns {
                parts.columns[var] = column.array(&e.cancel)?;
            }
            vec![
                Value::Container(Array::Table(TableArray::new(parts)?)),
                flags_value(e, false)?,
            ]
        }
        _ => {
            let gone = removed_lines(&tf, h, w, 1, spec.min_outliers);
            let keep: Vec<usize> = (0..h).filter(|&i| !gone[i]).collect();
            let b = if keep.len() == h {
                t.clone()
            } else {
                e.tbl_select(t, &keep, None, out)?
            };
            vec![
                Value::Container(Array::Table(b)),
                logical(e, h, 1, &gone)?,
                flags_value(e, false)?,
            ]
        }
    };
    values.extend(stats);
    Ok(values)
}
