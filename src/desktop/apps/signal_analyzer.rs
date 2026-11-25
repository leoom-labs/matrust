//! Signal Analyzer: workspace vectors and one-variable timetables in time and
//! frequency. Preprocessing makes NEW derived signals (`x_lp`, `x_mm`, ...)
//! with `detrend`, `lowpass`, `highpass`, `bandpass`, `movmean`, `resample`
//! or an index range, and keeps the original. Spectra come from `pwelch` or
//! `periodogram`, the time-frequency image from `spectrogram`, and every
//! number shown from engine builtins (`numel`, `min`, `max`, `mean`, `rms`,
//! `std`). Drawing alone thins long lines to their min/max envelope.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;
use std::collections::{BTreeMap, BTreeSet};

/// Most points one drawn line gets; longer data is drawn as its min/max
/// envelope over the visible range (the analysis keeps every sample).
const MAX_DRAWN: usize = 4000;
/// Largest spectrogram image drawn (frequency rows, time columns); larger
/// results are max-pooled for display.
const IMAGE_ROWS: usize = 512;
const IMAGE_COLS: usize = 1024;
/// Largest p and q of a `resample(x, p, q)` ratio.
const MAX_RATIO_TERM: u64 = 1000;
/// Spectral lengths the engine accepts.
const MAX_LENGTH: usize = 1 << 20;
/// Width of the spectrogram colour scale with its labels.
const SCALE_WIDTH: f32 = 72.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Time,
    Spectrum,
    Spectrogram,
}
impl View {
    pub const ALL: [Self; 3] = [Self::Time, Self::Spectrum, Self::Spectrogram];
    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Spectrum => "Spectrum",
            Self::Spectrogram => "Spectrogram",
        }
    }
}

/// Spectrum estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Welch,
    Periodogram,
}

/// Periodogram window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taper {
    Rectangular,
    Hann,
    Hamming,
}
impl Taper {
    pub const ALL: [Self; 3] = [Self::Rectangular, Self::Hann, Self::Hamming];
    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangular => "Rectangular",
            Self::Hann => "Hann",
            Self::Hamming => "Hamming",
        }
    }
    /// The window function; rectangular is periodogram's default.
    fn function(self) -> Option<&'static str> {
        match self {
            Self::Rectangular => None,
            Self::Hann => Some("hann"),
            Self::Hamming => Some("hamming"),
        }
    }
}

/// One preprocessing step: the line that makes a derived signal.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// `detrend(x)` removes the best straight line; `detrend(x, 'constant')`
    /// the mean.
    Detrend {
        linear: bool,
    },
    /// Passband edge in Hz, or normalized (×π rad/sample) without a rate.
    Lowpass(f64),
    Highpass(f64),
    Bandpass(f64, f64),
    /// `movmean` window in samples.
    MovingMean(usize),
    /// New rate in Hz, or the rate factor without a rate.
    Resample(f64),
    /// `x(first:last)`, 1-based and inclusive.
    Extract(usize, usize),
}
impl Step {
    fn suffix(&self) -> &'static str {
        match self {
            Self::Detrend { .. } => "dt",
            Self::Lowpass(_) => "lp",
            Self::Highpass(_) => "hp",
            Self::Bandpass(..) => "bp",
            Self::MovingMean(_) => "mm",
            Self::Resample(_) => "rs",
            Self::Extract(..) => "ext",
        }
    }
    /// Plain words for the signal list; `hz` = the input has a rate.
    pub fn describe(&self, hz: bool) -> String {
        let edge = |w: f64| {
            if hz {
                format!("{} Hz", short(w))
            } else {
                format!("{}π rad/sample", short(w))
            }
        };
        match self {
            Self::Detrend { linear: true } => "linear trend removed".into(),
            Self::Detrend { linear: false } => "mean removed".into(),
            Self::Lowpass(w) => format!("lowpass {}", edge(*w)),
            Self::Highpass(w) => format!("highpass {}", edge(*w)),
            Self::Bandpass(a, b) => {
                if hz {
                    format!("bandpass {}–{} Hz", short(*a), short(*b))
                } else {
                    format!("bandpass {}π–{}π rad/sample", short(*a), short(*b))
                }
            }
            Self::MovingMean(k) => format!("moving mean of {k}"),
            Self::Resample(r) if hz => format!("resampled to {} Hz", short(*r)),
            Self::Resample(r) => format!("resampled ×{}", short(*r)),
            Self::Extract(a, b) => format!("samples {a} to {b}"),
        }
    }
}

/// The step kinds the Preprocess form offers (Extract comes from the view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Detrend,
    Lowpass,
    Highpass,
    Bandpass,
    MovingMean,
    Resample,
}
impl StepKind {
    pub const ALL: [Self; 6] = [
        Self::Detrend,
        Self::Lowpass,
        Self::Highpass,
        Self::Bandpass,
        Self::MovingMean,
        Self::Resample,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Detrend => "Detrend",
            Self::Lowpass => "Lowpass",
            Self::Highpass => "Highpass",
            Self::Bandpass => "Bandpass",
            Self::MovingMean => "Moving mean",
            Self::Resample => "Resample",
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Detrend => "dt",
            Self::Lowpass => "lp",
            Self::Highpass => "hp",
            Self::Bandpass => "bp",
            Self::MovingMean => "mm",
            Self::Resample => "rs",
        }
    }
}

/// A signal the app made: `name = step(source)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Derived {
    pub name: String,
    pub source: String,
    pub step: Step,
    /// The step's frequencies and rate are in Hz (the source had a rate);
    /// otherwise normalized frequencies and a rate factor.
    pub hz: bool,
}

/// What a workspace variable offers as a signal.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A real double vector of this length.
    Vector(usize),
    /// A timetable with one real column variable.
    Timetable {
        variable: String,
        height: usize,
        /// Mean rate of the row times, when they increase.
        rate: Option<f64>,
    },
}
impl Source {
    fn info(&self) -> String {
        match self {
            Self::Vector(n) => format!("{n} samples"),
            Self::Timetable { height, .. } => format!("timetable, {height} rows"),
        }
    }
}

/// The variable as a signal, if it is one.
pub fn source(array: &Array) -> Option<Source> {
    match array {
        Array::Numeric(m) if !m.is_nd() && m.len() >= 2 && (m.rows() == 1 || m.cols() == 1) => {
            Some(Source::Vector(m.len()))
        }
        Array::Table(t) if t.is_timetable() && t.width() == 1 && t.height() >= 2 => {
            match t.column(0) {
                Array::Numeric(m) if m.cols() == 1 && !m.is_nd() => Some(Source::Timetable {
                    variable: t.names()[0].clone(),
                    height: t.height(),
                    rate: t.row_times().and_then(row_rate),
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// (n - 1) / (last - first) of a timetable's row times in seconds, to 12
/// significant digits (so 500 Hz reads back as 500, not 499.99999999999994).
fn row_rate(times: &Array) -> Option<f64> {
    let (n, span) = match times {
        Array::Duration(d) if !d.is_calendar() => {
            let data = d.data();
            (data.len(), (data.last()? - data.first()?) / 1000.)
        }
        Array::Datetime(d) => {
            let data = d.data();
            let first = data.first()?.total_nanos()?;
            let last = data.last()?.total_nanos()?;
            (data.len(), (last - first) as f64 / 1e9)
        }
        _ => return None,
    };
    let rate = (n as f64 - 1.) / span;
    (n >= 2 && rate.is_finite() && rate > 0.)
        .then(|| format!("{rate:.11e}").parse().ok())
        .flatten()
}

/// The sample-rate setting of a workspace signal.
#[derive(Clone, Debug, PartialEq)]
pub struct Rate {
    /// The Sample rate field, in Hz.
    pub text: String,
    /// "Samples" units: no rate; frequencies are in rad/sample.
    pub samples: bool,
    /// Typed by the user: a changed variable keeps it.
    pub edited: bool,
    /// Version of the variable the default came from.
    version: Option<u64>,
}
impl Default for Rate {
    fn default() -> Self {
        Self {
            text: "1".into(),
            samples: false,
            edited: false,
            version: None,
        }
    }
}

/// A signal as the script sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub name: String,
    /// Its samples in the script: `x`, `tt.Var1` or `x_lp`.
    pub expr: String,
    /// Hz; None in samples units.
    pub fs: Option<f64>,
    /// Time of the first sample (s, or samples without a rate).
    pub start: f64,
    /// The workspace variable the chain starts from.
    pub root: String,
    /// Steps from the root to this signal.
    pub chain: Vec<Step>,
}
impl Resolved {
    /// Seconds (or samples) between samples.
    fn spacing(&self) -> f64 {
        self.fs.map_or(1., |fs| 1. / fs)
    }
}

/// `p/q = r` with p, q up to MAX_RATIO_TERM, by continued fractions.
pub fn ratio(r: f64) -> Option<(u64, u64)> {
    if !(r.is_finite() && r > 0.) {
        return None;
    }
    let (mut h0, mut h1, mut k0, mut k1) = (0u64, 1u64, 1u64, 0u64);
    let mut x = r;
    for _ in 0..64 {
        let a = x.floor();
        if a > MAX_RATIO_TERM as f64 {
            return None;
        }
        let a = a as u64;
        let (h2, k2) = (a * h1 + h0, a * k1 + k0);
        if h2 > MAX_RATIO_TERM || k2 > MAX_RATIO_TERM {
            return None;
        }
        (h0, h1, k0, k1) = (h1, h2, k1, k2);
        if h1 > 0 && (h1 as f64 / k1 as f64 - r).abs() <= 1e-9 * r {
            return Some((h1, k1));
        }
        let fraction = x - a as f64;
        if fraction < 1e-12 {
            return None;
        }
        x = 1. / fraction;
    }
    None
}

/// The right-hand side that makes a derived signal from `input`; `hz` says
/// the step's numbers are in Hz, so the call passes the input's rate.
fn step_call(step: &Step, input: &Resolved, hz: bool) -> Result<String, String> {
    let x = &input.expr;
    let rate = || match (hz, input.fs) {
        (false, _) => Ok(None),
        (true, Some(fs)) => Ok(Some(fs)),
        (true, None) => Err(format!(
            "{} has no sample rate, so {} cannot run; give {} a rate in Hz",
            input.name,
            step.describe(true),
            input.root
        )),
    };
    let fs = match step {
        Step::Lowpass(_) | Step::Highpass(_) | Step::Bandpass(..) | Step::Resample(_) => rate()?,
        _ => None,
    };
    let fs_argument = fs.map(|fs| format!(", {}", number(fs))).unwrap_or_default();
    let fs = fs_argument.as_str();
    Ok(match step {
        Step::Detrend { linear: true } => format!("detrend({x})"),
        Step::Detrend { linear: false } => format!("detrend({x}, 'constant')"),
        Step::Lowpass(w) => format!("lowpass({x}, {}{fs})", number(*w)),
        Step::Highpass(w) => format!("highpass({x}, {}{fs})", number(*w)),
        Step::Bandpass(a, b) => format!("bandpass({x}, [{} {}]{fs})", number(*a), number(*b)),
        Step::MovingMean(k) => format!("movmean({x}, {k})"),
        Step::Resample(r) => {
            let factor = match rate()? {
                Some(rate) => r / rate,
                None => *r,
            };
            let (p, q) = ratio(factor).filter(|(p, q)| p != q).ok_or_else(|| {
                format!(
                    "resampling {} by {} needs a ratio p/q of whole numbers \
                     up to {MAX_RATIO_TERM} other than 1",
                    input.name,
                    short(factor)
                )
            })?;
            format!("resample({x}, {p}, {q})")
        }
        Step::Extract(a, b) => format!("{x}({a}:{b})"),
    })
}

/// `name(x, a, b, ..., fs)` with `[]` for default arguments; without a rate
/// the trailing defaults go, keeping at least `keep` arguments after x.
fn positional(name: &str, s: &Resolved, args: Vec<Option<String>>, keep: usize) -> String {
    let mut parts: Vec<String> = args
        .into_iter()
        .map(|a| a.unwrap_or_else(|| "[]".into()))
        .collect();
    match s.fs {
        Some(fs) => parts.push(number(fs)),
        None => {
            while parts.len() > keep && parts.last().is_some_and(|p| p == "[]") {
                parts.pop();
            }
        }
    }
    let mut call = format!("{name}({}", s.expr);
    for part in parts {
        call.push_str(", ");
        call.push_str(&part);
    }
    call.push(')');
    call
}

/// An optional whole number field: empty = the function's default.
fn optional_count(text: &str, what: &str, min: usize) -> Result<Option<usize>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(v) if v.fract() == 0. && v >= min as f64 && v <= MAX_LENGTH as f64 => {
            Ok(Some(v as usize))
        }
        _ => Err(format!(
            "Signal Analyzer: {what} must be a whole number from {min} to {MAX_LENGTH}, or empty for the default"
        )),
    }
}

/// Overlap percent of a window of `window` samples, in samples.
fn overlap_samples(text: &str, window: Option<usize>, what: &str) -> Result<Option<usize>, String> {
    let text = text.trim();
    let Some(window) = window else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(pct) if (0. ..100.).contains(&pct) => {
            let samples = (pct / 100. * window as f64).round() as usize;
            Ok(Some(samples.min(window - 1)))
        }
        _ => Err(format!(
            "Signal Analyzer: {what} overlap must be a percent from 0 to below 100"
        )),
    }
}

fn positive(text: &str, what: &str) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0. => Ok(v),
        _ => Err(format!("{what} must be a positive number")),
    }
}

/// Spectrum and spectrogram axis words.
fn frequency_label(hz: bool) -> &'static str {
    if hz {
        "Frequency (Hz)"
    } else {
        "Frequency (rad/sample)"
    }
}
fn power_label(hz: bool) -> &'static str {
    if hz {
        "Power/frequency (dB/Hz)"
    } else {
        "Power/frequency (dB/(rad/sample))"
    }
}
fn time_label(signals: &[Resolved]) -> &'static str {
    let rated = signals.iter().filter(|s| s.fs.is_some()).count();
    if rated == signals.len() {
        "Time (s)"
    } else if rated == 0 {
        "Samples"
    } else {
        "Time (s) or samples"
    }
}
/// The times of a signal's samples in the script.
fn time_expression(s: &Resolved) -> String {
    let index = format!("(0:numel({})-1)", s.expr);
    // The colon binds loosest: an offset keeps the range in parentheses.
    match (s.fs, s.start == 0.) {
        (Some(fs), true) => format!("{index}/{}", number(fs)),
        (Some(fs), false) => format!("{} + {index}/{}", number(s.start), number(fs)),
        (None, true) => format!("0:numel({})-1", s.expr),
        (None, false) => format!("{} + {index}", number(s.start)),
    }
}

/// A MATLAB character vector literal.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Settings of the Spectrum and Spectrogram views (empty = default).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spectral {
    pub window: String,
    pub overlap: String,
    pub nfft: String,
}

/// Signals the private run used: the selected one and those plotted.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub selected: Resolved,
    pub shown: Vec<Resolved>,
}

/// One plotted signal: samples at `start + i * spacing`.
#[derive(Clone, Debug)]
pub struct Trace {
    pub name: String,
    pub start: f64,
    pub spacing: f64,
    /// The run's own storage (shared, not copied).
    pub y: matrust::Matrix,
}
impl Trace {
    fn end(&self) -> f64 {
        self.start + self.spacing * (self.y.len().max(1) - 1) as f64
    }
}

/// One spectrum line in dB.
#[derive(Clone, Debug)]
pub struct Line {
    pub name: String,
    pub f: Vec<f64>,
    pub db: Vec<f64>,
}

/// The spectrogram in dB: `rows` frequencies × `cols` times, column-major
/// (the run's own storage, shared).
#[derive(Clone, Debug)]
pub struct Image {
    pub rows: usize,
    pub cols: usize,
    pub db: matrust::Matrix,
    pub f: Vec<f64>,
    pub t: Vec<f64>,
}
impl Image {
    /// Finite colour limits [min, max] (imagesc's automatic CLim).
    pub fn limits(&self) -> Option<[f64; 2]> {
        let (lo, hi) = self
            .db
            .data()
            .iter()
            .filter(|v| v.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(*v), hi.max(*v))
            });
        (lo <= hi).then_some(if lo == hi {
            [lo - 1., hi + 1.]
        } else {
            [lo, hi]
        })
    }
}

/// What the last good private run computed.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub plan: Plan,
    pub traces: Vec<Trace>,
    pub spectra: Vec<Line>,
    /// The selected signal's spectrum, as exported (the plot draws `spectra`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub pxx: Vec<f64>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub f: Vec<f64>,
    pub samples: f64,
    pub duration: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub rms: f64,
    pub std: f64,
    pub peak_to_peak: f64,
    pub peak_frequency: f64,
    /// pxx at the peak in dB.
    pub peak_db: f64,
    pub image: Image,
    /// Changes with every delivered run (spectrogram texture cache).
    pub generation: u64,
}

fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Signal Analyzer: {name} is not a real array")),
    }
}
/// A real array of the run, sharing its storage.
fn matrix(outputs: &Outputs, name: &str) -> Result<matrust::Matrix, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.clone()),
        _ => Err(format!("Signal Analyzer: {name} is not a real array")),
    }
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("Signal Analyzer: {name} is empty"))
}

/// Points of `(x[i], y[i])` for i in `range`, thinned to the min and max of
/// each of MAX_DRAWN/2 buckets (non-finite values are left out).
fn thinned(x: impl Fn(usize) -> f64, y: &[f64], range: std::ops::Range<usize>) -> Vec<[f64; 2]> {
    let count = range.len();
    let finite = |i: usize| y[i].is_finite().then(|| [x(i), y[i]]);
    if count <= MAX_DRAWN {
        return range.filter_map(finite).collect();
    }
    let per = count.div_ceil(MAX_DRAWN / 2);
    let mut points = Vec::with_capacity(MAX_DRAWN + 2);
    let mut i = range.start;
    while i < range.end {
        let end = (i + per).min(range.end);
        let (mut lo, mut hi) = (None::<usize>, None::<usize>);
        for j in i..end {
            if !y[j].is_finite() {
                continue;
            }
            if lo.is_none_or(|k| y[j] < y[k]) {
                lo = Some(j);
            }
            if hi.is_none_or(|k| y[j] > y[k]) {
                hi = Some(j);
            }
        }
        if let (Some(a), Some(b)) = (lo, hi) {
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            points.push([x(a), y[a]]);
            if b != a {
                points.push([x(b), y[b]]);
            }
        }
        i = end;
    }
    points
}

/// The samples of `trace` inside the x range (one more on each side).
fn trace_points(trace: &Trace, range: Option<[f64; 2]>) -> Vec<[f64; 2]> {
    let y = trace.y.data();
    let n = y.len();
    let (first, last) = match range {
        Some([lo, hi]) if hi > lo => {
            let index = |t: f64| (t - trace.start) / trace.spacing;
            let first = index(lo).floor() - 1.;
            let last = index(hi).ceil() + 2.;
            (
                first.clamp(0., n as f64) as usize,
                last.clamp(0., n as f64) as usize,
            )
        }
        _ => (0, n),
    };
    thinned(
        |i| trace.start + i as f64 * trace.spacing,
        y,
        first..last.max(first),
    )
}

/// Max-pools a column-major rows×cols matrix to at most IMAGE_ROWS×IMAGE_COLS.
fn pooled(image: &Image) -> (usize, usize, Vec<f64>) {
    let (rows, cols) = (image.rows, image.cols);
    let (rs, cs) = (
        rows.div_ceil(IMAGE_ROWS).max(1),
        cols.div_ceil(IMAGE_COLS).max(1),
    );
    let (out_rows, out_cols) = (rows.div_ceil(rs), cols.div_ceil(cs));
    let mut out = vec![f64::NEG_INFINITY; out_rows * out_cols];
    let data = image.db.data();
    for c in 0..cols {
        for r in 0..rows {
            let v = data[c * rows + r];
            let slot = &mut out[(c / cs) * out_rows + r / rs];
            if v > *slot {
                *slot = v;
            }
        }
    }
    (out_rows, out_cols, out)
}

pub struct SignalAnalyzer {
    /// Sample-rate settings of workspace signals, by name.
    pub rates: BTreeMap<String, Rate>,
    /// Derived signals in the order they were made.
    pub derived: Vec<Derived>,
    /// Signals drawn in the Time and Spectrum views.
    pub ticked: BTreeSet<String>,
    /// The signal that preprocessing, the Output column and the spectrogram use.
    pub selected: Option<String>,
    pub view: View,
    /// Time view: one plot per signal instead of one shared axis.
    pub stacked: bool,
    pub method: Method,
    pub welch: Spectral,
    pub taper: Taper,
    pub periodogram_nfft: String,
    pub spectrogram: Spectral,
    /// The Preprocess form.
    pub step_kind: StepKind,
    pub linear: bool,
    pub edge: String,
    pub band: [String; 2],
    pub span: String,
    pub new_rate: String,
    /// The last preprocessing message and whether it is an error.
    pub note: Option<(String, bool)>,
    pub pxx_name: String,
    pub pxx_edited: bool,
    pub f_name: String,
    pub f_edited: bool,
    /// Time view cursors (s, or samples).
    pub cursors: Option<[f64; 2]>,
    dragging: Option<usize>,
    /// Visible time range of the Time view.
    pub view_range: Option<[f64; 2]>,
    reset_view: bool,
    /// Extract region was clicked; done in `prepare`, which sees the workspace.
    pub extract_requested: bool,
    /// The signals of the current script (refreshed in `prepare`).
    pub plan: Option<Plan>,
    /// Last good run; errors keep it on screen.
    pub current: Option<Analysis>,
    generation: u64,
    /// Spectrogram texture and its colour limits, by analysis generation.
    texture: Option<(u64, egui::TextureHandle, [f64; 2])>,
}
impl Default for SignalAnalyzer {
    fn default() -> Self {
        Self {
            rates: BTreeMap::new(),
            derived: Vec::new(),
            ticked: BTreeSet::new(),
            selected: None,
            view: View::Time,
            stacked: false,
            method: Method::Welch,
            welch: Spectral::default(),
            taper: Taper::Rectangular,
            periodogram_nfft: String::new(),
            spectrogram: Spectral::default(),
            step_kind: StepKind::Lowpass,
            linear: true,
            edge: String::new(),
            band: [String::new(), String::new()],
            span: "5".into(),
            new_rate: String::new(),
            note: None,
            pxx_name: String::new(),
            pxx_edited: false,
            f_name: String::new(),
            f_edited: false,
            cursors: None,
            dragging: None,
            view_range: None,
            reset_view: false,
            extract_requested: false,
            plan: None,
            current: None,
            generation: 0,
            texture: None,
        }
    }
}

impl SignalAnalyzer {
    pub fn derived(&self, name: &str) -> Option<&Derived> {
        self.derived.iter().find(|d| d.name == name)
    }
    /// A name a new output or derived signal must avoid.
    fn taken(&self, ws: &Snapshot, name: &str) -> bool {
        ws.contains(name) || self.derived(name).is_some()
    }
    /// The signal named `name`, following derived signals to the workspace.
    pub fn resolve(&self, ws: &Snapshot, name: &str) -> Result<Resolved, String> {
        if let Some(d) = self.derived(name) {
            let input = self.resolve(ws, &d.source)?;
            let mut out = Resolved {
                name: d.name.clone(),
                expr: d.name.clone(),
                ..input.clone()
            };
            match &d.step {
                Step::Resample(r) if d.hz => out.fs = Some(*r),
                Step::Resample(r) => out.fs = input.fs.map(|fs| fs * r),
                Step::Extract(first, _) => {
                    out.start = input.start + (*first as f64 - 1.) * input.spacing();
                }
                _ => {}
            }
            out.chain.push(d.step.clone());
            return Ok(out);
        }
        let Some(array) = ws.get(name) else {
            return Err(format!(
                "Signal Analyzer: {name} is no longer in the workspace"
            ));
        };
        let Some(kind) = source(array) else {
            return Err(format!(
                "Signal Analyzer: {name} is not a numeric vector or a one-variable timetable"
            ));
        };
        let rate = self.rates.get(name).cloned().unwrap_or_default();
        let fs = if rate.samples {
            None
        } else {
            Some(positive(
                &rate.text,
                &format!("Signal Analyzer: the sample rate of {name}"),
            )?)
        };
        let expr = match &kind {
            Source::Vector(_) => name.to_owned(),
            Source::Timetable { variable, .. } if matrust::files::valid_name(variable) => {
                format!("{name}.{variable}")
            }
            Source::Timetable { variable, .. } => format!("{name}.({})", quoted(variable)),
        };
        Ok(Resolved {
            name: name.to_owned(),
            expr,
            fs,
            start: 0.,
            root: name.to_owned(),
            chain: Vec::new(),
        })
    }
    /// The selected signal and the plotted ones (ticked, else the selected).
    pub fn make_plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let name = self
            .selected
            .as_deref()
            .ok_or("Signal Analyzer: choose a workspace vector or timetable in Signals")?;
        let selected = self.resolve(ws, name)?;
        let mut shown = Vec::new();
        for name in self.order(ws) {
            if self.ticked.contains(&name) {
                shown.push(self.resolve(ws, &name)?);
            }
        }
        if shown.is_empty() {
            shown.push(selected.clone());
        }
        Ok(Plan { selected, shown })
    }
    /// Every signal in list order: workspace signals by name, then derived
    /// signals as made. A workspace variable named like a derived signal (an
    /// earlier export) is shown once, as the derived signal.
    pub fn order(&self, ws: &Snapshot) -> Vec<String> {
        let mut names: Vec<String> = ws
            .variables()
            .filter(|(name, array)| {
                self.derived(name).is_none()
                    && (source(array).is_some()
                        || self.ticked.contains(*name)
                        || self.selected.as_deref() == Some(*name))
            })
            .map(|(name, _)| name.to_owned())
            .collect();
        names.extend(self.derived.iter().map(|d| d.name.clone()));
        names
    }
    /// The spectrum's output names: the user's, else `pxx`, `f` unused.
    pub fn spectrum_names(&self, ws: &Snapshot) -> Result<(String, String), String> {
        let pxx = if self.pxx_edited {
            self.pxx_name.trim().to_owned()
        } else {
            unused_name("pxx", |n| self.taken(ws, n))
        };
        let f = if self.f_edited {
            self.f_name.trim().to_owned()
        } else {
            unused_name("f", |n| self.taken(ws, n) || n == pxx)
        };
        for (what, name) in [("power", &pxx), ("frequency", &f)] {
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Signal Analyzer: the spectrum {what} name is not a valid variable name"
                ));
            }
            if self.derived(name).is_some() || self.order(ws).iter().any(|n| n == name) {
                return Err(format!(
                    "Signal Analyzer: the spectrum {what} name {name} is a signal's name"
                ));
            }
        }
        if pxx == f {
            return Err(
                "Signal Analyzer: the spectrum power and frequency need different names".into(),
            );
        }
        Ok((pxx, f))
    }
    /// Spectrogram outputs: frequencies, times and power, unused names.
    pub fn spectrogram_names(&self, ws: &Snapshot, pxx: &str, f: &str) -> [String; 3] {
        let mut names: Vec<String> = Vec::new();
        for base in ["fsg", "tsg", "psg"] {
            let name = unused_name(base, |n| {
                self.taken(ws, n) || n == pxx || n == f || names.iter().any(|m| m == n)
            });
            names.push(name);
        }
        [names[0].clone(), names[1].clone(), names[2].clone()]
    }
    /// The spectrum call for `s` with the current settings.
    pub fn spectrum_call(&self, s: &Resolved) -> Result<String, String> {
        match self.method {
            Method::Welch => {
                let window = optional_count(&self.welch.window, "the pwelch window length", 2)?;
                let overlap = overlap_samples(&self.welch.overlap, window, "The pwelch")?;
                let nfft = optional_count(&self.welch.nfft, "the pwelch nfft", 1)?;
                Ok(positional(
                    "pwelch",
                    s,
                    vec![
                        window.map(|w| w.to_string()),
                        overlap.map(|o| o.to_string()),
                        nfft.map(|n| n.to_string()),
                    ],
                    0,
                ))
            }
            Method::Periodogram => {
                let window = self
                    .taper
                    .function()
                    .map(|w| format!("{w}(numel({}))", s.expr));
                let nfft = optional_count(&self.periodogram_nfft, "the periodogram nfft", 1)?;
                Ok(positional(
                    "periodogram",
                    s,
                    vec![window, nfft.map(|n| n.to_string())],
                    0,
                ))
            }
        }
    }
    /// The spectrogram call (the engine wants window, noverlap and nfft).
    pub fn spectrogram_call(&self, s: &Resolved) -> Result<String, String> {
        let window = optional_count(&self.spectrogram.window, "the spectrogram window length", 2)?;
        let overlap = overlap_samples(&self.spectrogram.overlap, window, "The spectrogram")?;
        let nfft = optional_count(&self.spectrogram.nfft, "the spectrogram nfft", 1)?;
        Ok(positional(
            "spectrogram",
            s,
            vec![
                window.map(|w| w.to_string()),
                overlap.map(|o| o.to_string()),
                nfft.map(|n| n.to_string()),
            ],
            3,
        ))
    }
    /// The Preprocess form as a step for the selected signal.
    pub fn form_step(&self, ws: &Snapshot) -> Result<Step, String> {
        let name = self
            .selected
            .as_deref()
            .ok_or("Choose a signal to preprocess")?;
        let input = self.resolve(ws, name)?;
        let nyquist = input.fs.map_or(1., |fs| fs / 2.);
        let unit = if input.fs.is_some() { " Hz" } else { "" };
        let inside = |w: f64, what: &str| {
            if w < nyquist {
                Ok(w)
            } else {
                Err(format!(
                    "{what} {}{unit} must be below the Nyquist frequency {}{unit}",
                    short(w),
                    short(nyquist)
                ))
            }
        };
        Ok(match self.step_kind {
            StepKind::Detrend => Step::Detrend {
                linear: self.linear,
            },
            StepKind::Lowpass => Step::Lowpass(inside(
                positive(&self.edge, "The passband edge")?,
                "The passband edge",
            )?),
            StepKind::Highpass => Step::Highpass(inside(
                positive(&self.edge, "The passband edge")?,
                "The passband edge",
            )?),
            StepKind::Bandpass => {
                let low = positive(&self.band[0], "The lower passband edge")?;
                let high = inside(
                    positive(&self.band[1], "The upper passband edge")?,
                    "The upper passband edge",
                )?;
                if low >= high {
                    return Err("The lower passband edge must be below the upper one".into());
                }
                Step::Bandpass(low, high)
            }
            StepKind::MovingMean => Step::MovingMean(
                optional_count(&self.span, "The moving mean window", 1)?
                    .ok_or("The moving mean window must be a whole number of samples")?,
            ),
            StepKind::Resample => {
                let what = if input.fs.is_some() {
                    "The new rate"
                } else {
                    "The rate factor"
                };
                let step = Step::Resample(positive(&self.new_rate, what)?);
                step_call(&step, &input, input.fs.is_some())?;
                step
            }
        })
    }
    /// The name a step on `source` gets: `x_lp`, else `x_lp1`, ...
    pub fn derived_name(&self, ws: &Snapshot, source: &str, suffix: &str) -> String {
        let mut base = format!("{source}_{suffix}");
        if base.len() > 59 {
            base = format!("{}_{suffix}", &source[..59 - suffix.len() - 1]);
        }
        unused_name(&base, |n| self.taken(ws, n))
    }
    /// Makes the derived signal `step(selected)`, selects and ticks it.
    pub fn add_step(&mut self, ws: &Snapshot, step: Step) -> Result<String, String> {
        let source = self
            .selected
            .clone()
            .ok_or("Choose a signal to preprocess")?;
        let input = self.resolve(ws, &source)?;
        let hz = input.fs.is_some();
        step_call(&step, &input, hz)?;
        let name = self.derived_name(ws, &source, step.suffix());
        self.derived.push(Derived {
            name: name.clone(),
            source,
            step,
            hz,
        });
        self.ticked.insert(name.clone());
        self.selected = Some(name.clone());
        Ok(name)
    }
    /// Samples of the selected signal inside the visible time range, when
    /// that is less than the whole signal.
    pub fn region(&self) -> Option<(usize, usize)> {
        let a = self.current.as_ref()?;
        let s = &a.plan.selected;
        let [lo, hi] = self.view_range?;
        let n = a.samples as usize;
        // Sample index of a time; the tolerance keeps 0.9 s at 1000 Hz on 900.
        let index = |t: f64| (t - s.start) * s.fs.unwrap_or(1.);
        let first = (index(lo) - 1e-9).ceil().max(0.);
        let last = (index(hi) + 1e-9).floor().min(n as f64 - 1.);
        if !(first.is_finite() && last.is_finite()) || last < first + 1. {
            return None;
        }
        let (first, last) = (first as usize + 1, last as usize + 1);
        (first > 1 || last < n).then_some((first, last))
    }
    /// Extract region: `x(i1:i2)` of the selected signal as a derived signal.
    pub fn extract(&mut self, ws: &Snapshot) -> Result<String, String> {
        let (first, last) = self
            .region()
            .ok_or("Zoom the Time view to part of the selected signal first")?;
        if self.selected.as_deref() != self.current.as_ref().map(|a| a.plan.selected.name.as_str())
        {
            return Err("Wait for the selected signal to compute, then extract".into());
        }
        self.add_step(ws, Step::Extract(first, last))
    }
    /// Deletes a derived signal and the ones made from it; a deleted
    /// selection moves to the signal it was made from.
    pub fn delete(&mut self, name: &str) -> Vec<String> {
        let parent = self.derived(name).map(|d| d.source.clone());
        let mut gone = vec![name.to_owned()];
        let mut i = 0;
        while i < gone.len() {
            let parent = gone[i].clone();
            gone.extend(
                self.derived
                    .iter()
                    .filter(|d| d.source == parent)
                    .map(|d| d.name.clone()),
            );
            i += 1;
        }
        self.derived.retain(|d| !gone.contains(&d.name));
        self.ticked.retain(|n| !gone.contains(n));
        if self.selected.as_ref().is_some_and(|s| gone.contains(s)) {
            self.selected = None;
            if let Some(parent) = parent {
                self.select(&parent);
            }
        }
        gone
    }
    /// Selecting a signal ticks it.
    pub fn select(&mut self, name: &str) {
        self.selected = Some(name.to_owned());
        self.ticked.insert(name.to_owned());
    }
    fn set_note(&mut self, result: Result<String, String>, done: impl Fn(&str) -> String) {
        self.note = Some(match result {
            Ok(name) => (done(&name), false),
            Err(error) => (error, true),
        });
    }
    /// Steps from the root to `name`, in words.
    pub fn history(&self, name: &str) -> String {
        let mut parts = Vec::new();
        let mut current = name.to_owned();
        while let Some(d) = self.derived(&current) {
            parts.push(d.step.describe(d.hz));
            current = d.source.clone();
        }
        parts.push(current);
        parts.reverse();
        parts.join(" → ")
    }
}

impl App for SignalAnalyzer {
    fn kind(&self) -> AppKind {
        AppKind::SignalAnalyzer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.make_plan(ws)?;
        let mut script = String::new();
        for d in &self.derived {
            let input = self.resolve(ws, &d.source)?;
            let call = step_call(&d.step, &input, d.hz)
                .map_err(|e| format!("Signal Analyzer: {}: {e}", d.name))?;
            script.push_str(&format!("{} = {call};\n", d.name));
        }
        let s = &plan.selected;
        let (pxx, f) = self.spectrum_names(ws)?;
        let [fsg, tsg, psg] = self.spectrogram_names(ws, &pxx, &f);
        script.push_str(&format!("[{pxx}, {f}] = {};\n", self.spectrum_call(s)?));
        script.push_str(&format!(
            "[~, {fsg}, {tsg}, {psg}] = {};\n",
            self.spectrogram_call(s)?
        ));
        let pairs: Vec<String> = plan
            .shown
            .iter()
            .map(|s| format!("{}, {}", time_expression(s), s.expr))
            .collect();
        let names: Vec<String> = plan.shown.iter().map(|s| quoted(&s.name)).collect();
        let hz = s.fs.is_some();
        script.push_str(&format!(
            "figure\n\
             subplot(3, 1, 1)\n\
             plot({})\n\
             xlabel('{}')\n\
             legend({{{}}}, 'Interpreter', 'none')\n\
             subplot(3, 1, 2)\n\
             plot({f}, 10*log10({pxx}))\n\
             xlabel('{}')\n\
             ylabel('{}')\n\
             subplot(3, 1, 3)\n\
             imagesc({tsg}, {fsg}, 10*log10({psg} + eps))\n\
             axis xy\n\
             colorbar\n\
             xlabel('{}')\n\
             ylabel('{}')\n",
            pairs.join(", "),
            time_label(&plan.shown),
            names.join(", "),
            frequency_label(hz),
            power_label(hz),
            if hz { "Time (s)" } else { "Samples" },
            frequency_label(hz),
        ));
        Ok(script)
    }
    fn inputs(&self) -> Vec<String> {
        let Some(plan) = &self.plan else {
            return Vec::new();
        };
        let mut roots: BTreeSet<String> = plan.shown.iter().map(|s| s.root.clone()).collect();
        roots.insert(plan.selected.root.clone());
        // Roots of derived signals that are not plotted.
        for d in &self.derived {
            let mut current = d.source.clone();
            while let Some(parent) = self.derived(&current) {
                current = parent.source.clone();
            }
            roots.insert(current);
        }
        roots.into_iter().collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let (Ok(plan), Ok((pxx, f))) = (self.make_plan(ws), self.spectrum_names(ws)) else {
            return (String::new(), Vec::new());
        };
        let [fsg, tsg, psg] = self.spectrogram_names(ws, &pxx, &f);
        let s = &plan.selected;
        // Private copies first, so no probe name can shadow the user's names.
        let mut left = vec![
            "app_s".to_owned(),
            "app_pxx".into(),
            "app_f".into(),
            "app_sgf".into(),
            "app_sgt".into(),
            "app_sgp".into(),
        ];
        let mut right = vec![s.expr.clone(), pxx, f, fsg, tsg, psg];
        for (k, shown) in plan.shown.iter().enumerate() {
            left.push(format!("app_y{k}"));
            right.push(shown.expr.clone());
        }
        let mut code = format!("[{}] = deal({});\n", left.join(", "), right.join(", "));
        code.push_str(&format!(
            "app_n = numel(app_s); app_min = min(app_s); app_max = max(app_s);\n\
             app_mean = mean(app_s); app_rms = rms(app_s); app_std = std(app_s);\n\
             app_p2p = max(app_s) - min(app_s); app_dur = numel(app_s) / {};\n\
             [app_pk, app_ipk] = max(app_pxx); app_fpk = app_f(app_ipk);\n\
             app_pkdb = 10*log10(app_pk);\n\
             app_sgdb = 10*log10(app_sgp + eps);\n",
            s.fs.map_or("1".into(), number)
        ));
        let mut outputs: Vec<String> = [
            "app_pxx", "app_f", "app_n", "app_min", "app_max", "app_mean", "app_rms", "app_std",
            "app_p2p", "app_dur", "app_fpk", "app_pkdb", "app_sgdb", "app_sgf", "app_sgt",
        ]
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
        for (k, shown) in plan.shown.iter().enumerate() {
            outputs.push(format!("app_y{k}"));
            outputs.push(format!("app_p{k}"));
            outputs.push(format!("app_q{k}"));
            if shown.name == s.name {
                code.push_str(&format!("app_p{k} = app_pxx; app_q{k} = app_f;\n"));
                continue;
            }
            // Another signal may be too short for the window: no line then.
            let private = Resolved {
                expr: format!("app_y{k}"),
                ..shown.clone()
            };
            match self.spectrum_call(&private) {
                Ok(call) => code.push_str(&format!(
                    "try\n[app_p{k}, app_q{k}] = {call};\ncatch\napp_p{k} = []; app_q{k} = [];\nend\n"
                )),
                Err(_) => code.push_str(&format!("app_p{k} = []; app_q{k} = [];\n")),
            }
        }
        (code, outputs)
    }
    fn prepare(&mut self, ws: &Snapshot) {
        // Rates: 1 Hz, or the timetable's row times, until the user types one.
        for (name, array) in ws.variables() {
            if let Some(kind) = source(array) {
                let version = ws.version(name);
                let rate = self.rates.entry(name.to_owned()).or_default();
                if !rate.edited && rate.version != version {
                    rate.version = version;
                    let detected = match kind {
                        Source::Timetable { rate, .. } => rate,
                        Source::Vector(_) => None,
                    };
                    rate.text = number(detected.unwrap_or(1.));
                }
            }
        }
        let derived: BTreeSet<String> = self.derived.iter().map(|d| d.name.clone()).collect();
        let present = |n: &String| ws.contains(n) || derived.contains(n);
        self.ticked.retain(present);
        if self.selected.as_ref().is_some_and(|n| !present(n)) {
            self.selected = None;
        }
        if self.selected.is_none() {
            let first = self
                .order(ws)
                .into_iter()
                .find(|n| self.ticked.contains(n))
                .or_else(|| {
                    self.order(ws)
                        .into_iter()
                        .find(|n| ws.get(n).and_then(source).is_some() || derived.contains(n))
                });
            if let Some(name) = first {
                self.select(&name);
            }
        }
        if std::mem::take(&mut self.extract_requested) {
            let result = self.extract(ws);
            self.set_note(result, |name| format!("Made {name}"));
        }
        self.plan = self.make_plan(ws).ok();
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let plan = self
            .plan
            .clone()
            .ok_or("Signal Analyzer: the run finished after its signals changed")?;
        let mut traces = Vec::new();
        let mut spectra = Vec::new();
        for (k, shown) in plan.shown.iter().enumerate() {
            traces.push(Trace {
                name: shown.name.clone(),
                start: shown.start,
                spacing: shown.spacing(),
                y: matrix(outputs, &format!("app_y{k}"))?,
            });
            let p = values(outputs, &format!("app_p{k}"))?;
            if !p.is_empty() {
                spectra.push(Line {
                    name: shown.name.clone(),
                    f: values(outputs, &format!("app_q{k}"))?,
                    db: p.iter().map(|v| 10. * v.log10()).collect(),
                });
            }
        }
        let image = match outputs.get("app_sgdb") {
            Some(Array::Numeric(m)) => Image {
                rows: m.rows(),
                cols: m.cols(),
                db: m.clone(),
                f: values(outputs, "app_sgf")?,
                t: values(outputs, "app_sgt")?,
            },
            _ => return Err("Signal Analyzer: the spectrogram is not a real array".into()),
        };
        self.generation += 1;
        let analysis = Analysis {
            traces,
            spectra,
            pxx: values(outputs, "app_pxx")?,
            f: values(outputs, "app_f")?,
            samples: scalar(outputs, "app_n")?,
            duration: scalar(outputs, "app_dur")?,
            min: scalar(outputs, "app_min")?,
            max: scalar(outputs, "app_max")?,
            mean: scalar(outputs, "app_mean")?,
            rms: scalar(outputs, "app_rms")?,
            std: scalar(outputs, "app_std")?,
            peak_to_peak: scalar(outputs, "app_p2p")?,
            peak_frequency: scalar(outputs, "app_fpk")?,
            peak_db: scalar(outputs, "app_pkdb")?,
            image,
            generation: self.generation,
            plan,
        };
        // New cursors when there are none or both left the plotted span.
        let span = analysis.traces.iter().fold(None::<[f64; 2]>, |span, t| {
            let [lo, hi] = span.unwrap_or([t.start, t.end()]);
            Some([lo.min(t.start), hi.max(t.end())])
        });
        if let Some([a, b]) = span
            && self
                .cursors
                .is_none_or(|c| c.iter().all(|t| !(a..=b).contains(t)))
        {
            self.cursors = Some([a + 0.25 * (b - a), a + 0.75 * (b - a)]);
        }
        self.current = Some(analysis);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        let width = ui.available_width();
        let order = self.order(ws);
        field(ui, "Signals", |ui| {
            if order.is_empty() {
                crate::tokens::helper(
                    ui,
                    "No numeric vectors or one-variable timetables in the workspace",
                );
            }
            for name in &order {
                let (info, problem) = match self.derived(name) {
                    Some(d) => (d.step.describe(d.hz), false),
                    None => match ws.get(name).and_then(source) {
                        Some(kind) => (kind.info(), false),
                        None => ("not a numeric vector".to_owned(), true),
                    },
                };
                ui.horizontal(|ui| {
                    let mut ticked = self.ticked.contains(name);
                    if ui
                        .add(egui::Checkbox::without_text(&mut ticked))
                        .on_hover_text("Plot this signal")
                        .changed()
                    {
                        if ticked {
                            self.ticked.insert(name.clone());
                        } else {
                            self.ticked.remove(name);
                        }
                    }
                    let on = self.selected.as_deref() == Some(name.as_str());
                    if ui
                        .selectable_label(on, name)
                        .on_hover_text("Preprocess, measure and spectrogram this signal")
                        .clicked()
                    {
                        self.select(name);
                    }
                    ui.add(
                        egui::Label::new(egui::RichText::new(info).color(if problem {
                            p.error
                        } else {
                            p.text_muted
                        }))
                        .truncate(),
                    );
                });
            }
        });
        let Some(selected) = self.selected.clone() else {
            return;
        };
        ui.add_space(crate::tokens::S);
        if let Some(d) = self.derived(&selected).cloned() {
            field(ui, &format!("Derived signal {selected}"), |ui| {
                crate::tokens::helper(ui, self.history(&selected));
                if let Ok(s) = self.resolve(ws, &selected) {
                    crate::tokens::helper(
                        ui,
                        match s.fs {
                            Some(fs) => format!("Sample rate {} Hz", short(fs)),
                            None => "In samples (no sample rate)".into(),
                        },
                    );
                }
                let dependents = self.derived.iter().filter(|e| e.source == d.name).count();
                let label = if dependents == 0 {
                    format!("Delete {selected}")
                } else {
                    format!("Delete {selected} and signals made from it")
                };
                if ui
                    .add(
                        egui::Button::new(label)
                            .min_size(egui::vec2(width, crate::tokens::CONTROL)),
                    )
                    .clicked()
                {
                    let gone = self.delete(&selected);
                    self.note = Some((format!("Deleted {}", gone.join(", ")), false));
                }
            });
        } else if ws.get(&selected).and_then(source).is_some() {
            let detected = matches!(
                ws.get(&selected).and_then(source),
                Some(Source::Timetable { rate: Some(_), .. })
            );
            let rate = self.rates.entry(selected.clone()).or_default();
            field(ui, &format!("Sample rate of {selected}"), |ui| {
                ui.horizontal(|ui| {
                    if crate::tokens::toggle(ui, !rate.samples, "Hz").clicked() {
                        rate.samples = false;
                    }
                    if crate::tokens::toggle(ui, rate.samples, "Samples").clicked() {
                        rate.samples = true;
                    }
                    if !rate.samples {
                        let response = ui.add(
                            crate::tokens::input(egui::TextEdit::singleline(&mut rate.text))
                                .desired_width(ui.available_width()),
                        );
                        rate.edited |= response.changed();
                    }
                });
                crate::tokens::helper(
                    ui,
                    if rate.samples {
                        "Times in samples, frequencies in rad/sample"
                    } else if detected && !rate.edited {
                        "From the timetable's row times"
                    } else {
                        "Samples per second"
                    },
                );
            });
        }
        ui.add_space(crate::tokens::S);
        self.preprocess_ui(ui, ws, width);
        ui.add_space(crate::tokens::S);
        match self.view {
            View::Time => {}
            View::Spectrum => self.spectrum_ui(ui, width),
            View::Spectrogram => {
                field(ui, "Spectrogram", |ui| {
                    spectral_fields(ui, "signal_analyzer_spectrogram", &mut self.spectrogram);
                });
            }
        }
    }
    fn main_caption(&self) -> &'static str {
        "SIGNAL"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                let tab = crate::tokens::tab(ui, self.view == view, view.label());
                if view == View::Time {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::SignalAnalyzer),
                    );
                }
                if tab.clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        if self.current.is_none() {
            crate::tokens::helper(ui, "Nothing computed yet: the status line says why.");
            return;
        }
        match self.view {
            View::Time => self.time_ui(ui),
            View::Spectrum => self.spectrum_view(ui),
            View::Spectrogram => self.spectrogram_view(ui),
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(a) = &self.current else {
            return ["Signal", "Samples", "Duration", "RMS"]
                .into_iter()
                .map(|label| (label, "—".to_owned()))
                .collect();
        };
        let hz = a.plan.selected.fs.is_some();
        let mut rows = vec![
            ("Signal", a.plan.selected.name.clone()),
            ("Samples", short(a.samples)),
            (
                "Duration",
                if hz {
                    format!("{} s", short(a.duration))
                } else {
                    format!("{} samples", short(a.duration))
                },
            ),
            ("Min", short(a.min)),
            ("Max", short(a.max)),
            ("Mean", short(a.mean)),
            ("RMS", short(a.rms)),
            ("Std", short(a.std)),
            ("Peak-to-peak", short(a.peak_to_peak)),
        ];
        if self.view == View::Spectrum {
            rows.push((
                "Peak frequency",
                if hz {
                    format!("{} Hz", short(a.peak_frequency))
                } else {
                    format!("{} rad/sample", short(a.peak_frequency))
                },
            ));
            rows.push((
                "Peak power",
                format!(
                    "{} {}",
                    short(a.peak_db),
                    if hz { "dB/Hz" } else { "dB/(rad/sample)" }
                ),
            ));
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let defaults = self.spectrum_names(ws).ok();
        if !self.pxx_edited {
            self.pxx_name = defaults.as_ref().map_or("pxx".into(), |(p, _)| p.clone());
        }
        let before = self.pxx_name.clone();
        name_field(ui, "Spectrum power name", &mut self.pxx_name, ws);
        self.pxx_edited |= self.pxx_name != before;
        if !self.f_edited {
            self.f_name = defaults.as_ref().map_or("f".into(), |(_, f)| f.clone());
        }
        let before = self.f_name.clone();
        name_field(ui, "Frequency name", &mut self.f_name, ws);
        self.f_edited |= self.f_name != before;
        let others = self.exported(ws);
        if !others.is_empty() {
            crate::tokens::helper(ui, format!("Export creates {}", others.join(", ")));
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        let mut names: Vec<String> = self.derived.iter().map(|d| d.name.clone()).collect();
        if let Ok((pxx, f)) = self.spectrum_names(ws) {
            let sg = self.spectrogram_names(ws, &pxx, &f);
            names.extend([pxx, f]);
            names.extend(sg);
        }
        names
    }
    fn summary(&self) -> Option<String> {
        let a = self.current.as_ref()?;
        let s = &a.plan.selected;
        Some(match s.fs {
            Some(fs) => format!(
                "{} · {} samples at {} Hz · spectrum peak {} Hz",
                s.name,
                short(a.samples),
                short(fs),
                short(a.peak_frequency)
            ),
            None => format!(
                "{} · {} samples · spectrum peak {} rad/sample",
                s.name,
                short(a.samples),
                short(a.peak_frequency)
            ),
        })
    }
}

/// Window, overlap and nfft fields; empty means the function's default.
fn spectral_fields(ui: &mut egui::Ui, id: &str, settings: &mut Spectral) {
    let width = ui.available_width();
    let text = |ui: &mut egui::Ui, value: &mut String, hint: &str, enabled: bool| {
        ui.add_enabled(
            enabled,
            crate::tokens::input(egui::TextEdit::singleline(value).hint_text(hint))
                .id_salt((id, hint))
                .desired_width(width),
        );
    };
    ui.label("Window length (samples)");
    text(ui, &mut settings.window, "default: 8 segments", true);
    ui.label("Overlap (%)");
    let windowed = !settings.window.trim().is_empty();
    text(ui, &mut settings.overlap, "default: 50", windowed);
    if !windowed {
        crate::tokens::helper(ui, "Set a window length to choose the overlap");
    }
    ui.label("FFT length (nfft)");
    text(ui, &mut settings.nfft, "default: 256 or more", true);
}

impl SignalAnalyzer {
    fn preprocess_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot, width: f32) {
        let p = crate::tokens::palette(ui.ctx());
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let input = self.resolve(ws, &selected).ok();
        let hz = input.as_ref().is_none_or(|s| s.fs.is_some());
        field(ui, &format!("Preprocess {selected}"), |ui| {
            egui::ComboBox::from_id_salt("signal_analyzer_step")
                .width(width)
                .selected_text(self.step_kind.label())
                .show_ui(ui, |ui| {
                    for kind in StepKind::ALL {
                        ui.selectable_value(&mut self.step_kind, kind, kind.label());
                    }
                });
            ui.add_space(crate::tokens::XS);
            let unit = if hz { "Hz" } else { "×π rad/sample" };
            let text = |ui: &mut egui::Ui, label: &str, value: &mut String| {
                ui.label(label);
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(value))
                        .id_salt(("signal_analyzer_form", label.to_owned()))
                        .desired_width(width),
                );
            };
            match self.step_kind {
                StepKind::Detrend => {
                    ui.horizontal(|ui| {
                        if crate::tokens::toggle(ui, self.linear, "Linear").clicked() {
                            self.linear = true;
                        }
                        if crate::tokens::toggle(ui, !self.linear, "Constant").clicked() {
                            self.linear = false;
                        }
                    });
                }
                StepKind::Lowpass | StepKind::Highpass => {
                    text(ui, &format!("Passband edge ({unit})"), &mut self.edge);
                }
                StepKind::Bandpass => {
                    text(ui, &format!("Lower edge ({unit})"), &mut self.band[0]);
                    text(ui, &format!("Upper edge ({unit})"), &mut self.band[1]);
                }
                StepKind::MovingMean => text(ui, "Window (samples)", &mut self.span),
                StepKind::Resample => {
                    let label = if hz {
                        "New sample rate (Hz)"
                    } else {
                        "Rate factor"
                    };
                    text(ui, label, &mut self.new_rate);
                }
            }
            if let Some(input) = &input {
                match input.fs {
                    Some(fs)
                        if matches!(
                            self.step_kind,
                            StepKind::Lowpass | StepKind::Highpass | StepKind::Bandpass
                        ) =>
                    {
                        crate::tokens::helper(
                            ui,
                            format!("Nyquist frequency {} Hz", short(fs / 2.)),
                        );
                    }
                    None if matches!(
                        self.step_kind,
                        StepKind::Lowpass | StepKind::Highpass | StepKind::Bandpass
                    ) =>
                    {
                        crate::tokens::helper(ui, "Normalized: 1 is the Nyquist frequency");
                    }
                    _ => {}
                }
            }
            ui.add_space(crate::tokens::XS);
            let name = self.derived_name(ws, &selected, self.step_kind.suffix());
            if ui
                .add(
                    egui::Button::new(format!("Make {name}"))
                        .min_size(egui::vec2(width, crate::tokens::CONTROL)),
                )
                .on_hover_text("A new signal; the original stays")
                .clicked()
            {
                let result = self.form_step(ws).and_then(|step| self.add_step(ws, step));
                self.set_note(result, |name| format!("Made {name}"));
            }
            if let Some((note, error)) = &self.note {
                ui.label(
                    egui::RichText::new(note)
                        .size(crate::tokens::CAPTION)
                        .color(if *error { p.error } else { p.text_muted }),
                );
            }
        });
    }
    fn spectrum_ui(&mut self, ui: &mut egui::Ui, width: f32) {
        field(ui, "Spectrum", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, self.method == Method::Welch, "Welch").clicked() {
                    self.method = Method::Welch;
                }
                if crate::tokens::toggle(ui, self.method == Method::Periodogram, "Periodogram")
                    .clicked()
                {
                    self.method = Method::Periodogram;
                }
            });
            ui.add_space(crate::tokens::XS);
            match self.method {
                Method::Welch => {
                    crate::tokens::helper(ui, "pwelch: averaged Hamming-windowed segments");
                    spectral_fields(ui, "signal_analyzer_welch", &mut self.welch);
                }
                Method::Periodogram => {
                    crate::tokens::helper(ui, "periodogram: one window over the whole signal");
                    ui.label("Window");
                    egui::ComboBox::from_id_salt("signal_analyzer_taper")
                        .width(width)
                        .selected_text(self.taper.label())
                        .show_ui(ui, |ui| {
                            for taper in Taper::ALL {
                                ui.selectable_value(&mut self.taper, taper, taper.label());
                            }
                        });
                    ui.label("FFT length (nfft)");
                    ui.add(
                        crate::tokens::input(
                            egui::TextEdit::singleline(&mut self.periodogram_nfft)
                                .hint_text("default: 256 or more"),
                        )
                        .desired_width(width),
                    );
                }
            }
        });
    }
    fn time_ui(&mut self, ui: &mut egui::Ui) {
        let region = self.region();
        let Self {
            current,
            cursors,
            dragging,
            view_range,
            reset_view,
            stacked,
            extract_requested,
            ..
        } = self;
        let Some(a) = current.as_ref() else {
            return;
        };
        let hz = a.plan.shown.iter().all(|s| s.fs.is_some());
        let unit = if hz { "s" } else { "samples" };
        ui.horizontal(|ui| {
            if crate::tokens::toggle(ui, *stacked, "Stacked").clicked() {
                *stacked = !*stacked;
            }
            if ui
                .add_enabled(region.is_some(), egui::Button::new("Extract region"))
                .on_hover_text(format!(
                    "A new signal from the samples of {} in the visible time range",
                    a.plan.selected.name
                ))
                .on_disabled_hover_text("Right-drag on the plot to zoom to a region first")
                .clicked()
            {
                *extract_requested = true;
            }
            if ui.button("Full view").clicked() {
                *reset_view = true;
            }
        });
        if let Some([t1, t2]) = *cursors {
            let dt = t2 - t1;
            let rate = if hz { "Hz" } else { "cycles/sample" };
            crate::tokens::helper(
                ui,
                format!(
                    "t1 {} {unit} · t2 {} {unit} · Δt {} {unit} · 1/Δt {} {rate}",
                    short(t1),
                    short(t2),
                    short(dt),
                    short(1. / dt)
                ),
            );
        }
        crate::tokens::helper(
            ui,
            "Drag: move the nearest cursor · Right-drag: zoom · Double-click: full view",
        );
        let x_label = time_label(&a.plan.shown);
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        let groups: Vec<Vec<usize>> = if *stacked {
            (0..a.traces.len()).map(|k| vec![k]).collect()
        } else {
            vec![(0..a.traces.len()).collect()]
        };
        let each = ((height - crate::tokens::S * (groups.len() as f32 - 1.)) / groups.len() as f32)
            .max(80.);
        let reset = std::mem::take(reset_view);
        // Stacked plots keep 80 px each and scroll when they do not fit.
        let overflow = each * groups.len() as f32 > height;
        let mut draw = |ui: &mut egui::Ui| {
            for (g, group) in groups.iter().enumerate() {
                let mut chart = egui_plot::Plot::new(("signal_analyzer_time", g))
                    .height(each)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .allow_zoom(true)
                    .allow_boxed_zoom(true)
                    .allow_double_click_reset(true)
                    .show_background(false)
                    .y_axis_label(if *stacked {
                        a.traces[group[0]].name.clone()
                    } else {
                        "Amplitude".to_owned()
                    })
                    .link_axis(egui::Id::new("signal_analyzer_time_axes"), [true, false])
                    .link_cursor(egui::Id::new("signal_analyzer_time_cursor"), [true, false]);
                if g + 1 == groups.len() {
                    chart = chart.x_axis_label(x_label);
                }
                if !*stacked && group.len() > 1 {
                    chart = chart.legend(egui_plot::Legend::default());
                }
                let response = chart.show(ui, |plot_ui| {
                    if reset {
                        plot_ui.set_auto_bounds(true);
                    }
                    let bounds = plot_ui.plot_bounds();
                    let range = [bounds.min()[0], bounds.max()[0]];
                    for &k in group {
                        let trace = &a.traces[k];
                        let [r, g, b] = matrust::plot::COLORS[k % matrust::plot::COLORS.len()];
                        plot_ui.line(
                            egui_plot::Line::new(
                                trace.name.clone(),
                                trace_points(trace, Some(range)),
                            )
                            .color(egui::Color32::from_rgb(r, g, b))
                            .width(1.),
                        );
                    }
                    let response = plot_ui.response().clone();
                    let pointer = plot_ui.pointer_coordinate();
                    if let (Some([t1, t2]), Some(pointer)) = (cursors.as_mut(), pointer) {
                        let nearest = || {
                            let x = |t: f64| {
                                plot_ui
                                    .screen_from_plot(egui_plot::PlotPoint::new(t, pointer.y))
                                    .x
                            };
                            let px = x(pointer.x);
                            if (x(*t1) - px).abs() <= (x(*t2) - px).abs() {
                                0
                            } else {
                                1
                            }
                        };
                        if response.drag_started_by(egui::PointerButton::Primary)
                            || response.clicked()
                        {
                            *dragging = Some(nearest());
                        }
                        if response.dragged_by(egui::PointerButton::Primary) || response.clicked() {
                            match *dragging {
                                Some(0) => *t1 = pointer.x,
                                Some(_) => *t2 = pointer.x,
                                None => {}
                            }
                        }
                    }
                    if response.drag_stopped() || response.clicked() {
                        *dragging = None;
                    }
                    if let Some(cursors) = cursors.as_ref() {
                        let ink = crate::tokens::palette(plot_ui.ctx()).text;
                        for (k, t) in cursors.iter().enumerate() {
                            plot_ui.vline(
                                egui_plot::VLine::new(format!("t{}", k + 1), *t)
                                    .color(ink)
                                    .width(1.)
                                    .style(egui_plot::LineStyle::dashed_dense()),
                            );
                        }
                    }
                    range
                });
                *view_range = Some(response.inner);
                if g + 1 < groups.len() {
                    ui.add_space(crate::tokens::S);
                }
            }
        };
        if overflow {
            egui::ScrollArea::vertical()
                .id_salt("signal_analyzer_stacked")
                .auto_shrink([false, false])
                .show(ui, draw);
        } else {
            draw(ui);
        }
    }
    fn spectrum_view(&mut self, ui: &mut egui::Ui) {
        let Some(a) = &self.current else {
            return;
        };
        let hz = a.plan.selected.fs.is_some();
        crate::tokens::helper(
            ui,
            format!(
                "{} · marker: the peak of {}",
                match self.method {
                    Method::Welch => "pwelch",
                    Method::Periodogram => "periodogram",
                },
                a.plan.selected.name
            ),
        );
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        let p = crate::tokens::palette(ui.ctx());
        egui_plot::Plot::new("signal_analyzer_spectrum")
            .height(height)
            .allow_scroll(false)
            .allow_boxed_zoom(true)
            .allow_double_click_reset(true)
            .show_background(false)
            .x_axis_label(frequency_label(hz))
            .y_axis_label(power_label(hz))
            .legend(egui_plot::Legend::default())
            .show(ui, |plot_ui| {
                for (k, line) in a.spectra.iter().enumerate() {
                    let colour = a
                        .plan
                        .shown
                        .iter()
                        .position(|s| s.name == line.name)
                        .unwrap_or(k);
                    let [r, g, b] = matrust::plot::COLORS[colour % matrust::plot::COLORS.len()];
                    plot_ui.line(
                        egui_plot::Line::new(
                            line.name.clone(),
                            thinned(|i| line.f[i], &line.db, 0..line.db.len().min(line.f.len())),
                        )
                        .color(egui::Color32::from_rgb(r, g, b))
                        .width(1.),
                    );
                }
                if a.peak_db.is_finite() {
                    plot_ui.points(
                        egui_plot::Points::new("Peak", vec![[a.peak_frequency, a.peak_db]])
                            .shape(egui_plot::MarkerShape::Circle)
                            .radius(4.)
                            .color(p.text),
                    );
                    plot_ui.text(
                        egui_plot::Text::new(
                            "Peak label",
                            egui_plot::PlotPoint::new(a.peak_frequency, a.peak_db),
                            format!(
                                "  {} {}",
                                short(a.peak_frequency),
                                if hz { "Hz" } else { "rad/sample" }
                            ),
                        )
                        .anchor(egui::Align2::LEFT_BOTTOM)
                        .color(p.text),
                    );
                }
            });
    }
    fn spectrogram_view(&mut self, ui: &mut egui::Ui) {
        let Some(a) = &self.current else {
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        let hz = a.plan.selected.fs.is_some();
        let image = &a.image;
        crate::tokens::helper(
            ui,
            format!(
                "spectrogram of {} · colour: power in dB",
                a.plan.selected.name
            ),
        );
        let map = matrust::plot::colormap::default_map();
        let generation = a.generation;
        let cached = self
            .texture
            .as_ref()
            .filter(|(key, ..)| *key == generation)
            .map(|(_, texture, limits)| (texture.clone(), *limits));
        let (texture, limits) = match cached {
            Some(cached) => cached,
            None => {
                let (Some(limits), true) = (image.limits(), image.rows > 0 && image.cols > 0)
                else {
                    crate::tokens::helper(ui, "The spectrogram is empty");
                    return;
                };
                let (rows, cols, pooled) = pooled(image);
                let mut rgb = Vec::with_capacity(rows * cols * 3);
                // Texture row 0 is the top: the highest frequency.
                for r in (0..rows).rev() {
                    for c in 0..cols {
                        rgb.extend(matrust::plot::colormap::scaled_rgb(
                            &map,
                            limits,
                            pooled[c * rows + r],
                        ));
                    }
                }
                let texture = ui.ctx().load_texture(
                    "signal-analyzer-spectrogram",
                    egui::ColorImage::from_rgb([cols, rows], &rgb),
                    egui::TextureOptions::NEAREST,
                );
                self.texture = Some((generation, texture.clone(), limits));
                (texture, limits)
            }
        };
        let edges = |v: &[f64]| -> [f64; 2] {
            match v {
                [] => [0., 1.],
                [only] => [only - 0.5, only + 0.5],
                [first, .., last] => {
                    let half = (last - first) / (v.len() - 1) as f64 / 2.;
                    [first - half, last + half]
                }
            }
        };
        let [t0, t1] = edges(&image.t);
        let [f0, f1] = edges(&image.f);
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        let plot_width = (ui.available_width() - SCALE_WIDTH - crate::tokens::S).max(120.);
        ui.horizontal(|ui| {
            egui_plot::Plot::new("signal_analyzer_spectrogram")
                .width(plot_width)
                .height(height)
                .allow_scroll(false)
                .allow_boxed_zoom(true)
                .allow_double_click_reset(true)
                .show_background(false)
                .x_axis_label(if hz { "Time (s)" } else { "Samples" })
                .y_axis_label(frequency_label(hz))
                .show(ui, |plot_ui| {
                    plot_ui.image(
                        egui_plot::PlotImage::new(
                            "Spectrogram",
                            texture.id(),
                            egui_plot::PlotPoint::new((t0 + t1) / 2., (f0 + f1) / 2.),
                            egui::vec2((t1 - t0) as f32, (f1 - f0) as f32),
                        )
                        .allow_hover(false),
                    );
                });
            colour_scale(ui, &map, limits, height, p);
        });
    }
}

/// A vertical colour bar with its dB limits.
fn colour_scale(
    ui: &mut egui::Ui,
    map: &[[f64; 3]],
    [lo, hi]: [f64; 2],
    height: f32,
    p: &crate::tokens::Palette,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SCALE_WIDTH, height), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(
        rect.min + egui::vec2(0., crate::tokens::S),
        egui::vec2(crate::tokens::M, height - 2. * crate::tokens::L),
    );
    let painter = ui.painter_at(rect);
    let steps = 64;
    for i in 0..steps {
        let t = (i as f64 + 0.5) / steps as f64;
        let [r, g, b] = matrust::plot::colormap::scaled_rgb(map, [0., 1.], t);
        let y1 = bar.bottom() - bar.height() * i as f32 / steps as f32;
        let y0 = bar.bottom() - bar.height() * (i + 1) as f32 / steps as f32;
        painter.rect_filled(
            egui::Rect::from_x_y_ranges(bar.x_range(), y0..=y1),
            0.,
            egui::Color32::from_rgb(r, g, b),
        );
    }
    painter.rect_stroke(
        bar,
        0.,
        egui::Stroke::new(1., p.border),
        egui::StrokeKind::Outside,
    );
    let font = egui::FontId::proportional(crate::tokens::CAPTION);
    let x = bar.right() + crate::tokens::XS;
    for (value, y) in [
        (hi, bar.top()),
        ((lo + hi) / 2., bar.center().y),
        (lo, bar.bottom()),
    ] {
        painter.text(
            egui::pos2(x, y),
            egui::Align2::LEFT_CENTER,
            short(value),
            font.clone(),
            p.text_muted,
        );
    }
    painter.text(
        egui::pos2(bar.left(), rect.bottom() - crate::tokens::XS),
        egui::Align2::LEFT_BOTTOM,
        "dB",
        font,
        p.text_muted,
    );
}
