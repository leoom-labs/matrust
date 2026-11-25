//! System Identification: estimates ARX, ARMAX, OE, BJ, transfer-function,
//! state-space and process models from input-output data. INPUTS chooses the
//! data (two vectors and a sample time, or an iddata object), the estimation
//! and validation ranges, the detrending step and the models. One script
//! builds the iddata objects, estimates the models marked for export and
//! compares them with the validation range; the private probe estimates the
//! other models and reads back what the views and the Output column show.
//! Every number shown comes from engine builtins (`iddata`, `detrend`, `arx`,
//! `armax`, `oe`, `bj`, `tfest`, `n4sid`, `ssest`, `procest`, `compare`,
//! `resid`, `step`, `bode`); Rust only validates the inputs and draws.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

/// Most models a session keeps.
pub const MAX_MODELS: usize = 12;
/// Highest polynomial, transfer-function or state-space order a field takes.
pub const MAX_ORDER: usize = 20;
/// Most points a drawn series keeps; a longer one keeps the lowest and the
/// highest sample of each of `DISPLAY_POINTS / 2` equal stretches.
pub const DISPLAY_POINTS: usize = 4000;
const APP: &str = "System Identification";
/// Functions the script and the probe call: no input variable may hide one.
const CALLED: [&str; 20] = [
    "iddata", "detrend", "arx", "armax", "oe", "bj", "tfest", "ssest", "n4sid", "procest",
    "compare", "resid", "zeros", "size", "step", "bode", "tf", "ss", "log10", "isfinite",
];

fn fail<T>(text: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("{APP}: {text}"))
}

// ---------------------------------------------------------------------------
// Choices

/// Where the data comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// An output vector, an input vector and a sample time.
    Vectors,
    /// An iddata object of the workspace.
    Object,
}

/// The trend removed from each range before estimation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Detrend {
    None,
    Mean,
    Linear,
}
impl Detrend {
    pub const ALL: [Self; 3] = [Self::None, Self::Mean, Self::Linear];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Mean => "Remove mean",
            Self::Linear => "Remove linear trend",
        }
    }
}

/// The model structures, in dropdown order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Arx,
    Armax,
    Oe,
    Bj,
    TransferFunction,
    StateSpace,
    Process,
}
impl Kind {
    pub const ALL: [Self; 7] = [
        Self::Arx,
        Self::Armax,
        Self::Oe,
        Self::Bj,
        Self::TransferFunction,
        Self::StateSpace,
        Self::Process,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Arx => "ARX",
            Self::Armax => "ARMAX",
            Self::Oe => "OE",
            Self::Bj => "BJ",
            Self::TransferFunction => "Transfer function",
            Self::StateSpace => "State space",
            Self::Process => "Process model",
        }
    }
    /// Polynomial models: the estimator, the layout of its order vector and
    /// the default orders.
    pub fn polynomial(self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            Self::Arx => Some(("arx", "[na nb nk]", "[2 2 1]")),
            Self::Armax => Some(("armax", "[na nb nc nk]", "[2 2 2 1]")),
            Self::Oe => Some(("oe", "[nb nf nk]", "[2 2 1]")),
            Self::Bj => Some(("bj", "[nb nc nd nf nk]", "[2 2 2 2 1]")),
            _ => None,
        }
    }
    /// Start of a new model's variable name.
    fn prefix(self) -> &'static str {
        match self {
            Self::Arx => "arx",
            Self::Armax => "armax",
            Self::Oe => "oe",
            Self::Bj => "bj",
            Self::TransferFunction => "tf",
            Self::StateSpace => "ss",
            Self::Process => "proc",
        }
    }
    /// Labels of the coefficient rows the probe reads, in its order.
    fn coefficients(self) -> &'static [&'static str] {
        match self {
            Self::Arx | Self::Armax | Self::Oe | Self::Bj => &["A", "B", "C", "D", "F"],
            Self::TransferFunction => &["Numerator", "Denominator", "IO delay"],
            Self::StateSpace => &["A", "B", "C", "D"],
            Self::Process => &["Kp", "Tp1", "Tp2", "Tp3", "Tw", "Zeta", "Td", "Tz"],
        }
    }
}

/// What the main view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum View {
    Time,
    Output,
    Residuals,
    Step,
    Bode,
}
impl View {
    pub const ALL: [Self; 5] = [
        Self::Time,
        Self::Output,
        Self::Residuals,
        Self::Step,
        Self::Bode,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "Time plot",
            Self::Output => "Model output",
            Self::Residuals => "Residuals",
            Self::Step => "Step",
            Self::Bode => "Bode",
        }
    }
}

// ---------------------------------------------------------------------------
// Typed fields

/// One field: at most 64 characters and exactly one expression.
fn one_expression(text: &str, what: &str) -> Result<(), String> {
    if text.len() > 64 {
        return fail(format!("{what}: at most 64 characters"));
    }
    super::single_expression(text).map_err(|e| format!("{APP}: {what}: {e}"))
}
fn digits(text: &str) -> Option<usize> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
/// A typed whole number from `lo` to `hi`.
fn whole(text: &str, what: &str, lo: usize, hi: usize) -> Result<usize, String> {
    one_expression(text, what)?;
    match digits(text).filter(|v| (lo..=hi).contains(v)) {
        Some(v) => Ok(v),
        None => fail(format!("{what} must be a whole number from {lo} to {hi}")),
    }
}
/// A typed finite number of at least zero (above zero when `positive`).
fn amount(text: &str, what: &str, positive: bool) -> Result<f64, String> {
    one_expression(text, what)?;
    let value = text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    match value {
        Some(v) if v > 0. || (!positive && v == 0.) => Ok(v),
        _ if positive => fail(format!("{what} must be a positive number")),
        _ => fail(format!("{what} must be a number of at least 0")),
    }
}
/// A polynomial model's orders, typed as one vector: `[2 2 1]` or `[2, 2, 1]`.
fn order_vector(text: &str, what: &str, layout: &str) -> Result<Vec<usize>, String> {
    one_expression(text, what)?;
    let count = layout.split_whitespace().count();
    let orders = text
        .trim()
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .and_then(|inner| {
            inner
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|t| !t.is_empty())
                .map(|t| digits(t).filter(|v| *v <= MAX_ORDER))
                .collect::<Option<Vec<usize>>>()
        })
        .filter(|v| v.len() == count);
    match orders {
        Some(orders) => Ok(orders),
        None => fail(format!(
            "{what}: enter {count} whole numbers {layout}, each from 0 to {MAX_ORDER}"
        )),
    }
}
/// A state-space order `n` or a range `a:b`.
fn order_range(text: &str, what: &str) -> Result<(usize, usize), String> {
    one_expression(text, what)?;
    let order = |t: &str| digits(t).filter(|v| (1..=MAX_ORDER).contains(v));
    let range = match text.split_once(':') {
        Some((a, b)) => order(a).zip(order(b)).filter(|(a, b)| a <= b),
        None => order(text).map(|n| (n, n)),
    };
    match range {
        Some(range) => Ok(range),
        None => fail(format!(
            "{what}: enter an order from 1 to {MAX_ORDER} or a range such as 1:10"
        )),
    }
}
/// Typed text without line breaks: a comment it enters stays one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn bracketed(values: &[usize]) -> String {
    let items: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("[{}]", items.join(" "))
}

// ---------------------------------------------------------------------------
// Models

/// One model of the list: a structure and its orders.
#[derive(Clone, Debug)]
pub struct Model {
    pub id: u32,
    /// The legend's name and the exported variable.
    pub name: String,
    /// Drawn in the views.
    pub shown: bool,
    /// Export and Generate script include this model.
    pub export: bool,
    pub color: usize,
    pub kind: Kind,
    /// ARX, ARMAX, OE and BJ orders as typed.
    pub orders: String,
    /// Transfer function: poles, zeros and IO delay (blank: none).
    pub poles: String,
    pub zeros: String,
    pub delay: String,
    /// Transfer function and ssest: discrete at the data's sample time.
    pub discrete: bool,
    /// State space: one order or a range `a:b` (n4sid picks from a range).
    pub order: String,
    /// State space: refine by ssest; off estimates by n4sid alone.
    pub refine: bool,
    pub feedthrough: bool,
    /// Process model: poles (0 to 3), delay, zero and underdamped pair.
    pub process_poles: usize,
    pub process_delay: bool,
    pub process_zero: bool,
    pub process_underdamped: bool,
    /// Last good result; errors keep it on screen.
    pub result: Option<Estimate>,
}
impl Model {
    fn new(id: u32, kind: Kind, name: String) -> Self {
        Self {
            id,
            name,
            shown: true,
            export: true,
            color: id as usize - 1,
            kind,
            orders: kind.polynomial().map_or("[2 2 1]", |p| p.2).into(),
            poles: "2".into(),
            zeros: "1".into(),
            delay: String::new(),
            discrete: false,
            order: "1:10".into(),
            refine: true,
            feedthrough: false,
            process_poles: 1,
            process_delay: true,
            process_zero: false,
            process_underdamped: false,
            result: None,
        }
    }
    pub fn set_kind(&mut self, kind: Kind) {
        if let Some((_, _, default)) = kind.polynomial()
            && self.kind != kind
        {
            self.orders = default.into();
        }
        self.kind = kind;
    }
    /// The process model type, e.g. `P2DZ`.
    pub fn process_type(&self) -> String {
        let mut text = format!("P{}", self.process_poles);
        for (on, letter) in [
            (self.process_delay, 'D'),
            (self.process_zero, 'Z'),
            (self.process_underdamped, 'U'),
        ] {
            if on {
                text.push(letter);
            }
        }
        text
    }
    /// The orders as typed on one line, for the script's comment (never
    /// the results: the script must not change when a run finishes).
    fn orders_text(&self) -> String {
        match self.kind {
            Kind::Arx | Kind::Armax | Kind::Oe | Kind::Bj => one_line(&self.orders),
            Kind::TransferFunction => {
                let mut text = format!(
                    "poles {}, zeros {}",
                    one_line(&self.poles),
                    one_line(&self.zeros)
                );
                if !self.delay.trim().is_empty() {
                    text.push_str(&format!(", delay {}", one_line(&self.delay)));
                }
                if self.discrete {
                    text.push_str(", discrete");
                }
                text
            }
            Kind::StateSpace => format!(
                "order {}, {}",
                one_line(&self.order),
                if self.refine { "ssest" } else { "n4sid" }
            ),
            Kind::Process => self.process_type(),
        }
    }
    /// The orders as the model list shows them: a state-space model
    /// estimated from a range also names the order n4sid chose.
    pub fn describe(&self) -> String {
        let text = self.orders_text();
        match self.result.as_ref().and_then(|r| r.states) {
            Some(n) if self.kind == Kind::StateSpace && self.order.contains(':') => {
                format!("{text}, chose {n}")
            }
            _ => text,
        }
    }
    /// The script's comment above the estimation.
    fn comment(&self) -> String {
        match self.kind.polynomial() {
            Some((_, layout, _)) => format!("{} {layout}", self.kind.label()),
            None => format!("{}, {}", self.kind.label(), self.orders_text()),
        }
    }
    /// The statements that estimate this model from `data` into `name`.
    fn estimation(&self, name: &str, data: &str) -> Result<String, String> {
        let what = |part: &str| format!("{}: {part}", self.title());
        if let Some((function, layout, _)) = self.kind.polynomial() {
            let orders = order_vector(&self.orders, &what("orders"), layout)?;
            // [na nb ...] for ARX and ARMAX, [nb ...] for OE and BJ.
            let nb = match self.kind {
                Kind::Arx | Kind::Armax => orders[1],
                _ => orders[0],
            };
            if self.kind == Kind::Arx && orders[0] + nb == 0 {
                return fail(what("na and nb cannot both be 0"));
            }
            if self.kind != Kind::Arx && nb == 0 {
                return fail(what("nb must be at least 1"));
            }
            return Ok(format!(
                "{name} = {function}({data}, {});\n",
                bracketed(&orders)
            ));
        }
        let sample_time = if self.discrete {
            format!(", 'Ts', {data}.Ts")
        } else {
            String::new()
        };
        let feedthrough = if self.feedthrough {
            ", 'Feedthrough', true"
        } else {
            ""
        };
        match self.kind {
            Kind::TransferFunction => {
                let poles = whole(&self.poles, &what("poles"), 1, MAX_ORDER)?;
                let zeros = whole(&self.zeros, &what("zeros"), 0, poles)?;
                let delay = if self.delay.trim().is_empty() {
                    String::new()
                } else if self.discrete {
                    let samples = whole(&self.delay, &what("IO delay in samples"), 0, 1000)?;
                    format!(", {samples}")
                } else {
                    let seconds = amount(&self.delay, &what("IO delay"), false)?;
                    format!(", {}", number(seconds))
                };
                Ok(format!(
                    "{name} = tfest({data}, {poles}, {zeros}{delay}{sample_time});\n"
                ))
            }
            Kind::StateSpace => {
                let (low, high) = order_range(&self.order, &what("order"))?;
                let order = if low == high {
                    low.to_string()
                } else {
                    format!("{low}:{high}")
                };
                if !self.refine {
                    return Ok(format!("{name} = n4sid({data}, {order}{feedthrough});\n"));
                }
                if low == high {
                    return Ok(format!(
                        "{name} = ssest({data}, {order}{sample_time}{feedthrough});\n"
                    ));
                }
                // n4sid picks the order at the largest singular-value gap.
                Ok(format!(
                    "{name} = n4sid({data}, {order}{feedthrough});\n\
                     {name} = ssest({data}, size({name}.A, 1){sample_time}{feedthrough});\n"
                ))
            }
            _ => {
                if self.process_poles > 3 {
                    return fail(what("a process model has 0 to 3 poles"));
                }
                if self.process_zero && self.process_poles == 0 {
                    return fail(what("a zero needs at least 1 pole"));
                }
                if self.process_underdamped && self.process_poles < 2 {
                    return fail(what("an underdamped pair needs at least 2 poles"));
                }
                Ok(format!(
                    "{name} = procest({data}, '{}');\n",
                    self.process_type()
                ))
            }
        }
    }
    /// Shown in lists, legends and messages.
    pub fn title(&self) -> String {
        let name = one_line(&self.name);
        if name.is_empty() {
            format!("Model {}", self.id)
        } else {
            name
        }
    }
    /// Everything the estimation depends on, for Stop.
    fn key(&self, h: &mut impl Hasher) {
        (self.id, &self.name, self.export, self.kind, &self.orders).hash(h);
        (&self.poles, &self.zeros, &self.delay, self.discrete).hash(h);
        (&self.order, self.refine, self.feedthrough).hash(h);
        (
            self.process_poles,
            self.process_delay,
            self.process_zero,
            self.process_underdamped,
        )
            .hash(h);
    }
}

/// One model as the private run estimated it.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    /// Fit to the validation range, percent.
    pub fit: f64,
    /// Fit of the one-step prediction to the estimation range, percent.
    pub estimation_fit: f64,
    pub fpe: f64,
    pub loss: f64,
    /// The model's coefficient rows by label (column-major for matrices).
    pub coefficients: Vec<(&'static str, Vec<f64>)>,
    /// A state-space model's order.
    pub states: Option<usize>,
    /// Simulated output over the validation range: (time, value).
    pub output: Vec<[f64; 2]>,
    /// Autocorrelation of the residuals: (lag, value), and its 99 % bound.
    pub auto: Vec<[f64; 2]>,
    pub auto_bound: f64,
    /// Cross-correlation of input and residuals, and its 99 % bound.
    pub cross: Vec<[f64; 2]>,
    pub cross_bound: f64,
    /// Unit step response: (time, value).
    pub step: Vec<[f64; 2]>,
    /// Frequency response: (rad/s, dB) and (rad/s, degrees).
    pub magnitude: Vec<[f64; 2]>,
    pub phase: Vec<[f64; 2]>,
}
impl Estimate {
    /// The coefficient row called `label` (tests and scripted setups).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn coefficient(&self, label: &str) -> Option<&[f64]> {
        self.coefficients
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, v)| v.as_slice())
    }
}

/// The data of the last good run, as the views draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct Data {
    pub samples: usize,
    pub ts: f64,
    /// Estimation and validation ranges, 1-based and inclusive.
    pub estimation: (usize, usize),
    pub validation: (usize, usize),
    /// The same two ranges in time.
    pub spans: [(f64, f64); 2],
    /// The whole record: (time, value).
    pub output: Vec<[f64; 2]>,
    pub input: Vec<[f64; 2]>,
    /// The validation output the models are compared with.
    pub measured: Vec<[f64; 2]>,
}

// ---------------------------------------------------------------------------
// Reading the run

fn numeric<'a>(outputs: &'a Outputs, name: &str) -> Result<&'a [f64], String> {
    match outputs.get(name) {
        Some(a @ Array::Numeric(m)) if !a.is_complex() => Ok(m.data()),
        _ => fail(format!("{name} is not a real array")),
    }
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    match numeric(outputs, name)?.first() {
        Some(v) => Ok(*v),
        None => fail(format!("{name} is empty")),
    }
}
/// (start + k step, values[k]) for drawing: every finite sample, or the
/// extremes of each stretch when there are more than `DISPLAY_POINTS`.
pub fn series(values: &[f64], start: f64, step: f64) -> Vec<[f64; 2]> {
    let point = |k: usize| [start + k as f64 * step, values[k]];
    if values.len() <= DISPLAY_POINTS {
        return (0..values.len())
            .map(point)
            .filter(|p| p[1].is_finite())
            .collect();
    }
    let stretches = DISPLAY_POINTS / 2;
    let mut out = Vec::with_capacity(DISPLAY_POINTS);
    for s in 0..stretches {
        let from = s * values.len() / stretches;
        let to = ((s + 1) * values.len() / stretches).max(from + 1);
        let mut low: Option<usize> = None;
        let mut high: Option<usize> = None;
        for (k, v) in values.iter().enumerate().take(to).skip(from) {
            if !v.is_finite() {
                continue;
            }
            if low.is_none_or(|i| *v < values[i]) {
                low = Some(k);
            }
            if high.is_none_or(|i| *v > values[i]) {
                high = Some(k);
            }
        }
        if let (Some(low), Some(high)) = (low, high) {
            out.push(point(low.min(high)));
            if low != high {
                out.push(point(low.max(high)));
            }
        }
    }
    out
}
/// At most `DISPLAY_POINTS` evenly spaced points of a smooth curve.
fn thinned(points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let stride = points.len().div_ceil(DISPLAY_POINTS).max(1);
    if stride == 1 {
        return points;
    }
    let last = points[points.len() - 1];
    let mut out: Vec<[f64; 2]> = points.into_iter().step_by(stride).collect();
    if out.last() != Some(&last) {
        out.push(last);
    }
    out
}
/// The finite rows of a column-major n-by-2 array.
fn pairs(values: &[f64]) -> Vec<[f64; 2]> {
    let n = values.len() / 2;
    (0..n)
        .map(|k| [values[k], values[n + k]])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}

impl Estimate {
    fn from_outputs(
        outputs: &Outputs,
        model: &Model,
        validation_start: f64,
        ts: f64,
    ) -> Result<Self, String> {
        let name = |part: &str| format!("sysid_m{}_{part}", model.id);
        let report = numeric(outputs, &name("rp"))?;
        let bounds = numeric(outputs, &name("bd"))?;
        if report.len() != 3 || bounds.len() != 2 {
            return fail(format!("incomplete results of {}", model.title()));
        }
        let coefficients = model
            .kind
            .coefficients()
            .iter()
            .enumerate()
            .map(|(k, label)| Ok((*label, numeric(outputs, &name(&format!("c{k}")))?.to_vec())))
            .collect::<Result<Vec<_>, String>>()?;
        let states = match model.kind {
            Kind::StateSpace => Some(scalar(outputs, &name("nx"))? as usize),
            _ => None,
        };
        // A continuous model's delay shifts its step response.
        let delay = scalar(outputs, &name("dl"))?.max(0.);
        let mut step = Vec::new();
        if delay > 0. {
            step.extend([[0., 0.], [delay, 0.]]);
        }
        step.extend(thinned(pairs(numeric(outputs, &name("st"))?)));
        let bode = numeric(outputs, &name("bo"))?;
        let n = bode.len() / 3;
        let response = |column: usize| -> Vec<[f64; 2]> {
            (0..n)
                .map(|k| [bode[k], bode[column * n + k]])
                .filter(|p| p[0] > 0. && p[0].is_finite() && p[1].is_finite())
                .collect()
        };
        Ok(Self {
            fit: scalar(outputs, &name("fit"))?,
            estimation_fit: report[0],
            fpe: report[1],
            loss: report[2],
            coefficients,
            states,
            output: series(numeric(outputs, &name("yh"))?, validation_start, ts),
            auto: pairs(numeric(outputs, &name("ac"))?),
            auto_bound: bounds[0],
            cross: pairs(numeric(outputs, &name("cc"))?),
            cross_bound: bounds[1],
            step,
            magnitude: response(1),
            phase: response(2),
        })
    }
}

// ---------------------------------------------------------------------------
// The app

pub struct SystemIdentification {
    pub source: Source,
    /// Vectors: the output and input variables and the sample time as typed.
    pub output: String,
    pub input: String,
    pub ts: String,
    /// The iddata variable.
    pub object: String,
    /// First and last sample of each range; blank is the first half
    /// (estimation) or the second half (validation).
    pub estimation: [String; 2],
    pub validation: [String; 2],
    pub detrend: Detrend,
    pub models: Vec<Model>,
    /// The selected model: INPUTS edits it, OUTPUT details it.
    pub selected: usize,
    /// The structure Add creates.
    pub add_kind: Kind,
    pub view: View,
    /// Exported names: the iddata object, the estimation and validation
    /// data and the vector of validation fits.
    pub names: [String; 4],
    /// Last good run's data; errors keep it on screen.
    pub data: Option<Data>,
    /// The host's runner state, once per frame (shows Stop).
    pub running: bool,
    stopped: Option<u64>,
    /// The default names were checked against the workspace.
    named: bool,
    next_id: u32,
}
impl Default for SystemIdentification {
    fn default() -> Self {
        Self {
            source: Source::Vectors,
            output: String::new(),
            input: String::new(),
            ts: "1".into(),
            object: String::new(),
            estimation: [String::new(), String::new()],
            validation: [String::new(), String::new()],
            detrend: Detrend::None,
            models: vec![Model::new(1, Kind::Arx, "arx1".into())],
            selected: 0,
            add_kind: Kind::Arx,
            view: View::Time,
            names: ["data", "ze", "zv", "fit_percent"].map(String::from),
            data: None,
            running: false,
            stopped: None,
            named: false,
            next_id: 2,
        }
    }
}

/// The data the script starts from, checked.
enum Origin {
    Vectors {
        output: String,
        input: String,
        ts: f64,
    },
    Object(String),
}

struct Plan {
    script: String,
    /// The iddata object the ranges index.
    full: String,
    estimation: String,
    validation: String,
    /// Exported models: id and variable.
    models: Vec<(u32, String)>,
    /// Every name Export writes.
    names: Vec<String>,
    samples: usize,
    ranges: [(usize, usize); 2],
}

/// A workspace variable's name, checked before it enters a script.
fn variable<'a>(ws: &'a Snapshot, name: &str, what: &str) -> Result<&'a Array, String> {
    if name.is_empty() {
        return fail(format!("choose {what} in INPUTS"));
    }
    one_expression(name, what)?;
    if !matrust::files::valid_name(name) {
        return fail(format!("{what} must be a workspace variable"));
    }
    if CALLED.contains(&name) {
        return fail(format!(
            "the variable {name} hides the function {name} the app calls; rename it"
        ));
    }
    match ws.get(name) {
        Some(array) => Ok(array),
        None => fail(format!("workspace variable {name} does not exist")),
    }
}
fn real_vector(array: &Array) -> bool {
    let dims = array.dims();
    matches!(array, Array::Numeric(_))
        && !array.is_complex()
        && dims.len() == 2
        && (dims[0] == 1 || dims[1] == 1)
        && array.len() >= 2
}
/// Samples, outputs and inputs of an iddata value.
pub fn iddata_size(array: &Array) -> Option<(usize, usize, usize)> {
    let Array::Struct(s) = array else {
        return None;
    };
    if s.len() != 1 {
        return None;
    }
    let slot = |name: &str| s.get(s.field_index(name)?, 0).ok();
    let Array::Character(class) = slot("MatRustSysId")?.as_ref() else {
        return None;
    };
    if String::from_utf16_lossy(class.units()) != "iddata" {
        return None;
    }
    let y = slot("OutputData")?.dims();
    let u = slot("InputData")?.dims();
    Some((*y.first()?, *y.get(1)?, *u.get(1)?))
}
/// One range's first and last sample; a blank bound is its default.
fn range(
    texts: &[String; 2],
    default: (usize, usize),
    samples: usize,
    what: &str,
) -> Result<(usize, usize), String> {
    let bound = |text: &str, default: usize, end: &str| {
        if text.trim().is_empty() {
            Ok(default)
        } else {
            whole(
                text,
                &format!("the {what} range's {end} sample"),
                1,
                samples,
            )
        }
    };
    let first = bound(&texts[0], default.0, "first")?;
    let last = bound(&texts[1], default.1, "last")?;
    if first > last {
        return fail(format!("the {what} range ends before it starts"));
    }
    Ok((first, last))
}
/// A name Export writes, checked; `used` collects them.
fn export_name(
    name: &str,
    what: &str,
    inputs: &[String],
    used: &mut BTreeSet<String>,
) -> Result<(), String> {
    one_expression(name, what)?;
    if !matrust::files::valid_name(name) {
        return fail(format!(
            "{what} must be a valid variable name (at most 63 characters)"
        ));
    }
    super::check_export_name(name)?;
    if name.starts_with("sysid_") {
        return fail(format!(
            "{what}: names starting with sysid_ are reserved by the app"
        ));
    }
    if matrust::session::FUNCTIONS.contains(&name) {
        return fail(format!(
            "{name} is a function name; choose another export name"
        ));
    }
    if inputs.iter().any(|input| input == name) {
        return fail(format!(
            "{what}: {name} is an input variable; choose another name"
        ));
    }
    if !used.insert(name.to_owned()) {
        return fail(format!("duplicate export name {name}"));
    }
    Ok(())
}

impl SystemIdentification {
    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn model(&self) -> Option<&Model> {
        self.models.get(self.selected)
    }
    /// `arx1`, `arx2`, ...: the first the workspace and the app do not use.
    fn model_name(&self, kind: Kind, ws: &Snapshot) -> String {
        (1..)
            .map(|n| format!("{}{n}", kind.prefix()))
            .find(|name| {
                !ws.contains(name)
                    && !self.names.contains(name)
                    && self.models.iter().all(|m| m.name != *name)
            })
            .unwrap()
    }
    /// New model of `kind` with its default orders; it becomes the selected one.
    pub fn add(&mut self, kind: Kind, ws: &Snapshot) -> Result<(), String> {
        if self.models.len() >= MAX_MODELS {
            return fail(format!("at most {MAX_MODELS} models"));
        }
        let name = self.model_name(kind, ws);
        let id = self.id();
        self.models.push(Model::new(id, kind, name));
        self.selected = self.models.len() - 1;
        Ok(())
    }
    /// A copy of the selected model under a new name.
    pub fn duplicate(&mut self, ws: &Snapshot) -> Result<(), String> {
        let Some(original) = self.model().cloned() else {
            return Ok(());
        };
        self.add(original.kind, ws)?;
        let new = &mut self.models[self.selected];
        *new = Model {
            id: new.id,
            name: new.name.clone(),
            color: new.color,
            result: None,
            ..original
        };
        Ok(())
    }
    pub fn remove(&mut self) {
        if self.selected < self.models.len() {
            self.models.remove(self.selected);
        }
        self.selected = self.selected.min(self.models.len().saturating_sub(1));
    }
    /// Everything the script and the probe depend on, for Stop.
    fn settings_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.source, &self.output, &self.input, &self.ts).hash(&mut h);
        (&self.object, &self.estimation, &self.validation).hash(&mut h);
        (self.detrend, &self.names, self.models.len()).hash(&mut h);
        for model in &self.models {
            model.key(&mut h);
        }
        h.finish()
    }
    /// Stop: cancels the running estimation; none starts until an input
    /// changes or Run is pressed.
    pub fn stop(&mut self) {
        self.stopped = Some(self.settings_key());
    }
    pub fn run(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped == Some(self.settings_key())
    }

    /// The data source and its number of samples, checked.
    fn origin(&self, ws: &Snapshot) -> Result<(Origin, usize), String> {
        match self.source {
            Source::Vectors => {
                let y = variable(ws, &self.output, "the output y")?;
                let u = variable(ws, &self.input, "the input u")?;
                for (name, array) in [(&self.output, y), (&self.input, u)] {
                    if !real_vector(array) {
                        return fail(format!("{name} must be a real numeric vector"));
                    }
                }
                if u.len() != y.len() {
                    return fail(format!(
                        "the input {} has {} samples and the output {} has {}; \
                         they must have the same length",
                        self.input,
                        u.len(),
                        self.output,
                        y.len()
                    ));
                }
                let ts = amount(&self.ts, "the sample time Ts", true)?;
                Ok((
                    Origin::Vectors {
                        output: self.output.clone(),
                        input: self.input.clone(),
                        ts,
                    },
                    y.len(),
                ))
            }
            Source::Object => {
                let array = variable(ws, &self.object, "the iddata object")?;
                match iddata_size(array) {
                    Some((samples, 1, 1)) => Ok((Origin::Object(self.object.clone()), samples)),
                    Some(_) => fail(format!(
                        "{} must have one output and one input channel",
                        self.object
                    )),
                    None => fail(format!("{} is not an iddata object", self.object)),
                }
            }
        }
    }
    /// The estimation and validation ranges for `samples` samples.
    fn ranges(&self, samples: usize) -> Result<[(usize, usize); 2], String> {
        if samples < 4 {
            return fail("the data needs at least 4 samples");
        }
        let half = samples / 2;
        Ok([
            range(&self.estimation, (1, half), samples, "estimation")?,
            range(&self.validation, (half + 1, samples), samples, "validation")?,
        ])
    }

    /// Everything the script, the probe and Export share, checked.
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.models.len() > MAX_MODELS {
            return fail(format!("at most {MAX_MODELS} models"));
        }
        let (origin, samples) = self.origin(ws)?;
        let ranges = self.ranges(samples)?;
        let inputs = self.inputs();
        let [data, estimation, validation, fit] = &self.names;
        let mut used = BTreeSet::new();
        let mut names = Vec::new();
        let (full, mut script) = match &origin {
            Origin::Vectors { output, input, ts } => {
                export_name(data, "the iddata name", &inputs, &mut used)?;
                names.push(data.clone());
                let script = format!(
                    "% {APP}: output {output}, input {input}, {samples} samples, \
                     sample time {} s\n{data} = iddata({output}, {input}, {});\n",
                    short(*ts),
                    number(*ts)
                );
                (data.clone(), script)
            }
            Origin::Object(name) => (
                name.clone(),
                format!("% {APP}: iddata object {name}, {samples} samples\n"),
            ),
        };
        export_name(estimation, "the estimation data name", &inputs, &mut used)?;
        export_name(validation, "the validation data name", &inputs, &mut used)?;
        names.extend([estimation.clone(), validation.clone()]);
        // Every model is checked, exported or not, so errors show at once.
        for model in &self.models {
            model.estimation("sysid_m", estimation)?;
        }
        let exported: Vec<&Model> = self.models.iter().filter(|m| m.export).collect();
        for model in &exported {
            let what = format!("the name of {}", model.title());
            export_name(&model.name, &what, &inputs, &mut used)?;
            names.push(model.name.clone());
        }
        if !exported.is_empty() {
            export_name(fit, "the fit vector name", &inputs, &mut used)?;
            names.push(fit.clone());
        }
        let [(e1, e2), (v1, v2)] = ranges;
        script.push_str(&format!(
            "% Estimation and validation ranges\n\
             {estimation} = {full}({e1}:{e2});\n\
             {validation} = {full}({v1}:{v2});\n"
        ));
        let detrend = match self.detrend {
            Detrend::None => None,
            Detrend::Mean => Some(("mean", "")),
            Detrend::Linear => Some(("linear trend", ", 1")),
        };
        if let Some((what, order)) = detrend {
            script.push_str(&format!(
                "% Detrend: remove the {what} of each range\n\
                 {estimation} = detrend({estimation}{order});\n\
                 {validation} = detrend({validation}{order});\n"
            ));
        }
        for model in &exported {
            script.push_str(&format!("% {}: {}\n", model.name, model.comment()));
            script.push_str(&model.estimation(&model.name, estimation)?);
        }
        if !exported.is_empty() {
            script.push_str(&format!(
                "% Fit to the validation data, percent: \
                 100*(1 - norm(y - yhat)/norm(y - mean(y)))\n\
                 {fit} = zeros(1, {});\n",
                exported.len()
            ));
            for (k, model) in exported.iter().enumerate() {
                script.push_str(&format!(
                    "[~, {fit}({})] = compare({validation}, {});\n",
                    k + 1,
                    model.name
                ));
            }
        }
        Ok(Plan {
            script,
            full,
            estimation: estimation.clone(),
            validation: validation.clone(),
            models: exported.iter().map(|m| (m.id, m.name.clone())).collect(),
            names,
            samples,
            ranges,
        })
    }

    /// The probe lines of one model: its estimate when the script has none,
    /// then the numbers of the list and the curves of the views.
    fn model_probe(&self, model: &Model, plan: &Plan) -> Result<String, String> {
        let id = model.id;
        let m = format!("sysid_m{id}");
        let (ze, zv) = (&plan.estimation, &plan.validation);
        let mut code = match plan.models.iter().find(|(i, _)| *i == id) {
            Some((_, name)) => format!("{m} = {name};\n"),
            None => model.estimation(&m, ze)?,
        };
        code.push_str(&format!(
            "[sysid_h, {m}_fit] = compare({zv}, {m});\n\
             {m}_yh = sysid_h.OutputData;\n\
             [~, ~, sysid_s] = resid({zv}, {m});\n\
             {m}_ac = [sysid_s.Lags sysid_s.AutoCorr];\n\
             {m}_cc = [sysid_s.CrossLags sysid_s.CrossCorr];\n\
             {m}_bd = [sysid_s.AutoBound sysid_s.CrossBound];\n\
             {m}_rp = [{m}.Report.Fit.FitPercent {m}.Report.Fit.FPE {m}.Report.Fit.LossFcn];\n"
        ));
        let fields: &[&str] = match model.kind {
            Kind::TransferFunction => &["Numerator", "Denominator", "IODelay"],
            _ => model.kind.coefficients(),
        };
        for (k, field) in fields.iter().enumerate() {
            code.push_str(&format!("{m}_c{k} = {m}.{field};\n"));
        }
        // The LTI values hold no delay: a continuous model's delay is taken
        // out, then shifts the step response and the phase.
        code.push_str(&match model.kind {
            Kind::StateSpace => {
                format!("{m}_nx = size({m}.A, 1);\nsysid_d = 0;\nsysid_g = ss({m});\n")
            }
            Kind::TransferFunction if !model.discrete => format!(
                "sysid_c = {m};\nsysid_d = sysid_c.IODelay;\nsysid_c.IODelay = 0;\n\
                 sysid_g = tf(sysid_c);\n"
            ),
            Kind::Process => format!(
                "sysid_c = {m};\nsysid_d = sysid_c.Td;\n\
                 if ~isfinite(sysid_d), sysid_d = 0; end\n\
                 sysid_c.Td = 0;\nsysid_g = tf(sysid_c);\n"
            ),
            _ => format!("sysid_d = 0;\nsysid_g = tf({m});\n"),
        });
        code.push_str(&format!(
            "{m}_dl = sysid_d;\n\
             [sysid_y, sysid_t] = step(sysid_g);\n\
             {m}_st = [sysid_t(:) + sysid_d, sysid_y(:)];\n\
             [sysid_a, sysid_p, sysid_w] = bode(sysid_g);\n\
             {m}_bo = [sysid_w(:), 20*log10(sysid_a(:)), \
             sysid_p(:) - sysid_w(:)*sysid_d*180/pi];\n"
        ));
        Ok(code)
    }
    /// The names `model_probe` leaves.
    fn model_outputs(model: &Model) -> Vec<String> {
        let mut parts: Vec<String> = ["fit", "yh", "ac", "cc", "bd", "rp", "dl", "st", "bo"]
            .map(String::from)
            .to_vec();
        parts.extend((0..model.kind.coefficients().len()).map(|k| format!("c{k}")));
        if model.kind == Kind::StateSpace {
            parts.push("nx".into());
        }
        parts
            .into_iter()
            .map(|part| format!("sysid_m{}_{part}", model.id))
            .collect()
    }
    /// The preparation steps the script takes, for INPUTS.
    pub fn steps(&self, ws: &Snapshot) -> Vec<String> {
        let Ok((origin, samples)) = self.origin(ws) else {
            return Vec::new();
        };
        let Ok([(e1, e2), (v1, v2)]) = self.ranges(samples) else {
            return Vec::new();
        };
        let mut steps = vec![match origin {
            Origin::Vectors { output, input, ts } => format!(
                "iddata({output}, {input}, {}): {samples} samples",
                short(ts)
            ),
            Origin::Object(name) => format!("{name}: {samples} samples"),
        }];
        steps.push(format!("Split: estimation {e1}:{e2}, validation {v1}:{v2}"));
        match self.detrend {
            Detrend::None => {}
            Detrend::Mean => steps.push("Detrend: remove the mean of each range".into()),
            Detrend::Linear => steps.push("Detrend: remove the linear trend of each range".into()),
        }
        steps.push(format!(
            "Estimate {} model{} and compare on validation",
            self.models.len(),
            if self.models.len() == 1 { "" } else { "s" }
        ));
        steps
    }
}

impl App for SystemIdentification {
    fn kind(&self) -> AppKind {
        AppKind::SystemIdentification
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.is_stopped() {
            return fail("stopped. Change an input or press Run.");
        }
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        let names = match self.source {
            Source::Vectors => vec![&self.output, &self.input],
            Source::Object => vec![&self.object],
        };
        let mut out: Vec<String> = Vec::new();
        for name in names {
            if !name.is_empty() && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let (full, zv) = (&plan.full, &plan.validation);
        let [(e1, e2), (v1, v2)] = plan.ranges;
        let mut code = format!(
            "sysid_n = {};\nsysid_rg = [{e1} {e2} {v1} {v2}];\n\
             sysid_ts = {full}.Ts;\nsysid_t0 = {full}.Tstart;\n\
             sysid_yy = {full}.OutputData;\nsysid_uu = {full}.InputData;\n\
             sysid_tv = {zv}.Tstart;\nsysid_yv = {zv}.OutputData;\n",
            plan.samples
        );
        let mut names: Vec<String> = [
            "sysid_n", "sysid_rg", "sysid_ts", "sysid_t0", "sysid_yy", "sysid_uu", "sysid_tv",
            "sysid_yv",
        ]
        .map(String::from)
        .to_vec();
        for model in &self.models {
            let Ok(lines) = self.model_probe(model, &plan) else {
                return (String::new(), Vec::new());
            };
            code.push_str(&lines);
            names.extend(Self::model_outputs(model));
        }
        (code, names)
    }
    fn prepare(&mut self, ws: &Snapshot) {
        self.selected = self.selected.min(self.models.len().saturating_sub(1));
        if self.stopped.is_some() && !self.is_stopped() {
            self.stopped = None;
        }
        if !self.named {
            // Default names avoid the workspace once; later edits are the user's.
            self.named = true;
            for k in 0..self.names.len() {
                let taken = |name: &str| {
                    ws.contains(name)
                        || self.names[..k].iter().any(|n| n == name)
                        || self.models.iter().any(|m| m.name == name)
                };
                let name = unused_name(&self.names[k], taken);
                self.names[k] = name;
            }
            for k in 0..self.models.len() {
                if ws.contains(&self.models[k].name) {
                    let name = self.model_name(self.models[k].kind, ws);
                    self.models[k].name = name;
                }
            }
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let ranges = numeric(outputs, "sysid_rg")?;
        if ranges.len() != 4 {
            return fail("incomplete ranges");
        }
        let ts = scalar(outputs, "sysid_ts")?;
        let start = scalar(outputs, "sysid_t0")?;
        let validation_start = scalar(outputs, "sysid_tv")?;
        let time = |sample: f64| start + (sample - 1.) * ts;
        let data = Data {
            samples: scalar(outputs, "sysid_n")? as usize,
            ts,
            estimation: (ranges[0] as usize, ranges[1] as usize),
            validation: (ranges[2] as usize, ranges[3] as usize),
            spans: [
                (time(ranges[0]), time(ranges[1])),
                (time(ranges[2]), time(ranges[3])),
            ],
            output: series(numeric(outputs, "sysid_yy")?, start, ts),
            input: series(numeric(outputs, "sysid_uu")?, start, ts),
            measured: series(numeric(outputs, "sysid_yv")?, validation_start, ts),
        };
        let results = self
            .models
            .iter()
            .map(|model| Estimate::from_outputs(outputs, model, validation_start, ts))
            .collect::<Result<Vec<_>, String>>()?;
        for (model, result) in self.models.iter_mut().zip(results) {
            model.result = Some(result);
        }
        self.data = Some(data);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.data_ui(ui, ws);
        section(ui);
        self.models_ui(ui, ws);
    }
    fn main_caption(&self) -> &'static str {
        "IDENTIFICATION"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                if crate::tokens::tab(ui, self.view == view, view.label()).clicked() {
                    self.view = view;
                }
            }
            ui.add_space(crate::tokens::S);
            if self.is_stopped() {
                crate::tokens::helper(ui, "Stopped");
                if ui.button("Run").clicked() {
                    self.run();
                }
            } else if self.running {
                crate::tokens::helper(ui, "Estimating…");
                if ui.button("Stop").clicked() {
                    self.stop();
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(data) = &self.data else {
            crate::tokens::helper(
                ui,
                "No data yet: choose the output and input in INPUTS. \
                 The status line says what is missing.",
            );
            return;
        };
        let shown: Vec<&Model> = self
            .models
            .iter()
            .filter(|m| m.shown && m.result.is_some())
            .collect();
        if self.view != View::Time && shown.is_empty() {
            crate::tokens::helper(
                ui,
                "No model to draw: add a model in INPUTS or tick Show in the model list.",
            );
        }
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        match self.view {
            View::Time => time_view(ui, data, height),
            View::Output => output_view(ui, data, &shown, height),
            View::Residuals => residual_view(ui, &shown, height),
            View::Step => step_view(ui, &shown, height),
            View::Bode => bode_view(ui, &shown, height),
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(model) = self.model() else {
            return vec![("Model", "—".into())];
        };
        let mut rows = vec![
            ("Model", model.title()),
            ("Type", model.kind.label().to_owned()),
            ("Orders", model.describe()),
        ];
        let Some(r) = &model.result else {
            rows.extend(["Fit (validation)", "FPE", "Loss"].map(|l| (l, "—".to_owned())));
            return rows;
        };
        rows.extend([
            ("Fit (validation)", format!("{} %", short(r.fit))),
            ("Fit (estimation)", format!("{} %", short(r.estimation_fit))),
            ("FPE", short(r.fpe)),
            ("Loss", short(r.loss)),
        ]);
        if let Some(n) = r.states {
            rows.push(("States", n.to_string()));
        }
        if model.kind != Kind::StateSpace {
            let used = process_parameters(model);
            for (label, values) in &r.coefficients {
                // A unit polynomial and a zero delay say nothing.
                let hidden = match model.kind {
                    Kind::Process => !used.contains(label),
                    Kind::TransferFunction => *label == "IO delay" && values.as_slice() == [0.],
                    _ => values.as_slice() == [1.],
                };
                if !hidden {
                    rows.push((*label, row_text(values)));
                }
            }
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.list_ui(ui);
        ui.add_space(crate::tokens::M);
        self.export_ui(ui, ws);
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.plan(ws).map(|plan| plan.names).unwrap_or_default()
    }
    fn summary(&self) -> Option<String> {
        let data = self.data.as_ref()?;
        let mut text = format!(
            "{APP}: {} samples, Ts {} s · {} model{}",
            data.samples,
            short(data.ts),
            self.models.len(),
            if self.models.len() == 1 { "" } else { "s" }
        );
        let best = self
            .models
            .iter()
            .filter_map(|m| Some((m, m.result.as_ref()?.fit)))
            .filter(|(_, fit)| fit.is_finite())
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((model, fit)) = best {
            text.push_str(&format!(
                " · best validation fit {} {} %",
                model.title(),
                short(fit)
            ));
        }
        Some(text)
    }
}

/// The parameters a process model of this type has.
fn process_parameters(model: &Model) -> Vec<&'static str> {
    let mut names = vec!["Kp"];
    if model.process_underdamped {
        names.extend(["Tw", "Zeta"]);
        if model.process_poles == 3 {
            names.push("Tp3");
        }
    } else {
        names.extend(["Tp1", "Tp2", "Tp3"].iter().take(model.process_poles));
    }
    if model.process_delay {
        names.push("Td");
    }
    if model.process_zero {
        names.push("Tz");
    }
    names
}
/// `[1 -1.5 0.7]`, the first eight values.
fn row_text(values: &[f64]) -> String {
    let mut items: Vec<String> = values.iter().take(8).map(|v| short(*v)).collect();
    if values.len() > 8 {
        items.push("…".into());
    }
    if let [one] = items.as_slice() {
        return one.clone();
    }
    format!("[{}]", items.join(" "))
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
fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label(egui::RichText::new(text.into()).color(p.text_muted));
}
fn text_input(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(
        crate::tokens::input(egui::TextEdit::singleline(text).char_limit(64))
            .hint_text(hint)
            .desired_width(width),
    )
}
fn text_field(ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str) {
    field(ui, label, |ui| {
        let width = ui.available_width();
        text_input(ui, text, hint, width);
    });
}
/// A field's own message, without the app's name.
fn error_line(ui: &mut egui::Ui, result: Result<(), String>) {
    let local = result.map_err(|e| e.trim_start_matches("System Identification: ").to_owned());
    super::expression_message(ui, local);
}
/// What `name_field` says under a name: invalid, reserved or replacing.
fn name_message(ui: &mut egui::Ui, name: &str, ws: &Snapshot) {
    let p = crate::tokens::palette(ui.ctx());
    if !matrust::files::valid_name(name) {
        super::expression_message(ui, Err("Use a valid variable name".into()));
    } else if let Err(error) = super::check_export_name(name) {
        super::expression_message(ui, Err(error));
    } else if ws.contains(name) {
        ui.label(
            egui::RichText::new(format!("Replaces the workspace variable {name}"))
                .size(crate::tokens::CAPTION)
                .color(p.warning),
        );
    }
}
/// Real vectors of at least two values: name and length.
pub fn vectors(ws: &Snapshot) -> Vec<(String, usize)> {
    ws.variables()
        .filter(|(_, array)| real_vector(array))
        .map(|(name, array)| (name.to_owned(), array.len()))
        .collect()
}
/// iddata objects with one output and one input: name and samples.
pub fn objects(ws: &Snapshot) -> Vec<(String, usize)> {
    ws.variables()
        .filter_map(|(name, array)| match iddata_size(array) {
            Some((samples, 1, 1)) => Some((name.to_owned(), samples)),
            _ => None,
        })
        .collect()
}
/// A variable dropdown listing each choice with its length.
fn picker(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut String,
    options: &[(String, usize)],
) {
    field(ui, label, |ui| {
        let shown = if value.is_empty() {
            "Choose a variable".to_owned()
        } else {
            value.clone()
        };
        egui::ComboBox::from_id_salt(id)
            .width(ui.available_width())
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (name, length) in options {
                    ui.selectable_value(value, name.clone(), format!("{name} ({length})"));
                }
            });
    });
}

impl SystemIdentification {
    fn data_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        heading(ui, "Data");
        ui.horizontal(|ui| {
            let vectors = ui.radio_value(&mut self.source, Source::Vectors, "Vectors");
            crate::focus_areas::register(
                ui,
                &vectors,
                crate::focus_areas::Area::App(AppKind::SystemIdentification),
            );
            ui.radio_value(&mut self.source, Source::Object, "iddata object");
        });
        ui.add_space(crate::tokens::S);
        match self.source {
            Source::Vectors => {
                let options = vectors(ws);
                picker(ui, "sysid_output", "Output y", &mut self.output, &options);
                picker(ui, "sysid_input", "Input u", &mut self.input, &options);
                if options.is_empty() {
                    crate::tokens::helper(
                        ui,
                        "No numeric vectors in the workspace: create u and y first",
                    );
                }
                text_field(ui, "Sample time Ts (s)", &mut self.ts, "1");
            }
            Source::Object => {
                let options = objects(ws);
                picker(
                    ui,
                    "sysid_object",
                    "iddata object",
                    &mut self.object,
                    &options,
                );
                if options.is_empty() {
                    crate::tokens::helper(
                        ui,
                        "No iddata object with one output and one input in the workspace",
                    );
                }
            }
        }
        let samples = self.origin(ws).ok().map(|(_, samples)| samples);
        let defaults = samples.filter(|n| *n >= 4).map(|n| (n / 2, n));
        let hint = |v: Option<usize>| v.map(|v| v.to_string()).unwrap_or_default();
        type Bounds = [Option<usize>; 2];
        let range_row = |ui: &mut egui::Ui, label: &str, texts: &mut [String; 2], d: Bounds| {
            field(ui, label, |ui| {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 2. * crate::tokens::L) / 2.;
                    text_input(ui, &mut texts[0], &hint(d[0]), width);
                    ui.label("to");
                    let width = ui.available_width();
                    text_input(ui, &mut texts[1], &hint(d[1]), width);
                });
            });
        };
        range_row(
            ui,
            "Estimation samples",
            &mut self.estimation,
            [defaults.map(|_| 1), defaults.map(|d| d.0)],
        );
        range_row(
            ui,
            "Validation samples",
            &mut self.validation,
            [defaults.map(|d| d.0 + 1), defaults.map(|d| d.1)],
        );
        field(ui, "Detrend", |ui| {
            egui::ComboBox::from_id_salt("sysid_detrend")
                .width(ui.available_width())
                .selected_text(self.detrend.label())
                .show_ui(ui, |ui| {
                    for choice in Detrend::ALL {
                        ui.selectable_value(&mut self.detrend, choice, choice.label());
                    }
                });
        });
        let steps = self.steps(ws);
        if !steps.is_empty() {
            heading(ui, "Steps");
            for (k, step) in steps.iter().enumerate() {
                crate::tokens::helper(ui, format!("{}. {step}", k + 1));
            }
        }
    }

    fn models_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        heading(ui, "Models");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("sysid_add_kind")
                .selected_text(self.add_kind.label())
                .show_ui(ui, |ui| {
                    for kind in Kind::ALL {
                        ui.selectable_value(&mut self.add_kind, kind, kind.label());
                    }
                });
            if ui
                .add_enabled(self.models.len() < MAX_MODELS, egui::Button::new("Add"))
                .clicked()
            {
                let _ = self.add(self.add_kind, ws);
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.models.is_empty() && self.models.len() < MAX_MODELS,
                    egui::Button::new("Duplicate"),
                )
                .clicked()
            {
                let _ = self.duplicate(ws);
            }
            if ui
                .add_enabled(!self.models.is_empty(), egui::Button::new("Remove"))
                .clicked()
            {
                self.remove();
            }
        });
        ui.add_space(crate::tokens::S);
        for (k, model) in self.models.iter().enumerate() {
            let text = format!("{} · {}", model.title(), model.kind.label());
            if ui.selectable_label(self.selected == k, text).clicked() {
                self.selected = k;
            }
        }
        let Some(model) = self.models.get_mut(self.selected) else {
            crate::tokens::helper(ui, "No models: choose a type and press Add.");
            return;
        };
        ui.add_space(crate::tokens::S);
        let mut kind = model.kind;
        field(ui, "Type", |ui| {
            egui::ComboBox::from_id_salt("sysid_kind")
                .width(ui.available_width())
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for choice in Kind::ALL {
                        ui.selectable_value(&mut kind, choice, choice.label());
                    }
                });
        });
        model.set_kind(kind);
        match model.kind {
            Kind::TransferFunction => {
                text_field(ui, "Poles", &mut model.poles, "2");
                text_field(ui, "Zeros", &mut model.zeros, "1");
                let label = if model.discrete {
                    "IO delay (samples, blank: none)"
                } else {
                    "IO delay (s, blank: none)"
                };
                text_field(ui, label, &mut model.delay, "0");
                ui.checkbox(&mut model.discrete, "Discrete time (data sample time)");
            }
            Kind::StateSpace => {
                text_field(ui, "Order (n, or a range a:b)", &mut model.order, "1:10");
                ui.checkbox(&mut model.refine, "Refine with ssest (prediction error)");
                ui.add_enabled(
                    model.refine,
                    egui::Checkbox::new(&mut model.discrete, "Discrete time (data sample time)"),
                );
                ui.checkbox(&mut model.feedthrough, "Feedthrough");
                crate::tokens::helper(
                    ui,
                    "n4sid picks the order of a range at the largest singular-value gap.",
                );
            }
            Kind::Process => {
                field(ui, "Poles", |ui| {
                    ui.horizontal(|ui| {
                        for poles in 0..=3 {
                            ui.radio_value(&mut model.process_poles, poles, poles.to_string());
                        }
                    });
                });
                ui.checkbox(&mut model.process_delay, "Delay (D)");
                ui.add_enabled(
                    model.process_poles >= 1,
                    egui::Checkbox::new(&mut model.process_zero, "Zero (Z)"),
                );
                ui.add_enabled(
                    model.process_poles >= 2,
                    egui::Checkbox::new(&mut model.process_underdamped, "Underdamped pair (U)"),
                );
                model.process_zero &= model.process_poles >= 1;
                model.process_underdamped &= model.process_poles >= 2;
                crate::tokens::helper(ui, format!("Type {}", model.process_type()));
            }
            _ => {
                let layout = model.kind.polynomial().map_or("", |p| p.1);
                text_field(ui, &format!("Orders {layout}"), &mut model.orders, layout);
            }
        }
        error_line(ui, model.estimation("m", "ze").map(|_| ()));
        crate::tokens::helper(
            ui,
            format!(
                "Order fields accept whole numbers up to {MAX_ORDER}. Up to {MAX_MODELS} models."
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Views

fn colour(index: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[index % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}
fn chart(id: &str, height: f32, x: &str, y: &str) -> egui_plot::Plot<'static> {
    egui_plot::Plot::new(id.to_owned())
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x.to_owned())
        .y_axis_label(y.to_owned())
        .y_axis_min_width(crate::tokens::XL * 2.)
        .legend(egui_plot::Legend::default().follow_insertion_order(true))
        .show_background(false)
}
/// Heights of two charts stacked with a gap.
fn halves(height: f32) -> f32 {
    ((height - crate::tokens::S) / 2.).max(80.)
}

/// Output and input over time; the two ranges are shaded.
fn time_view(ui: &mut egui::Ui, data: &Data, height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    let half = halves(height);
    for (k, (label, points)) in [("Output y", &data.output), ("Input u", &data.input)]
        .into_iter()
        .enumerate()
    {
        if k == 1 {
            ui.add_space(crate::tokens::S);
        }
        chart(&format!("sysid_time_{k}"), half, "Time (s)", label)
            .link_axis(egui::Id::new("sysid_time_axes"), [true, false])
            .show(ui, |plot| {
                for (what, samples, (from, to), fill) in [
                    ("Estimation", data.estimation, data.spans[0], p.accent_soft),
                    ("Validation", data.validation, data.spans[1], p.hairline),
                ] {
                    let name = format!("{what} {}:{}", samples.0, samples.1);
                    plot.span(egui_plot::Span::new(name, from..=to).fill(fill));
                }
                plot.line(
                    egui_plot::Line::new(label, points.clone())
                        .color(colour(k))
                        .width(1.),
                );
            });
    }
}

/// The validation output and each shown model's simulated output.
fn output_view(ui: &mut egui::Ui, data: &Data, shown: &[&Model], height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    chart("sysid_output", height, "Time (s)", "Output y").show(ui, |plot| {
        plot.line(
            egui_plot::Line::new("Validation data", data.measured.clone())
                .color(p.text_muted)
                .width(1.),
        );
        for model in shown {
            let Some(r) = &model.result else { continue };
            plot.line(
                egui_plot::Line::new(
                    format!("{} ({} %)", model.title(), short(r.fit)),
                    r.output.clone(),
                )
                .color(colour(model.color))
                .width(1.5),
            );
        }
    });
}

/// Correlation tests of the residuals on the validation range, with the
/// 99 % bounds dashed.
fn residual_view(ui: &mut egui::Ui, shown: &[&Model], height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    let half = halves(height);
    for (k, label) in [
        "Autocorrelation of residuals",
        "Cross-correlation of input and residuals",
    ]
    .into_iter()
    .enumerate()
    {
        if k == 1 {
            ui.add_space(crate::tokens::S);
        }
        chart(&format!("sysid_residual_{k}"), half, "Lag (samples)", label).show(ui, |plot| {
            let mut bounds = Vec::new();
            for model in shown {
                let Some(r) = &model.result else { continue };
                let (points, bound) = if k == 0 {
                    (&r.auto, r.auto_bound)
                } else {
                    (&r.cross, r.cross_bound)
                };
                let ink = colour(model.color);
                plot.line(
                    egui_plot::Line::new(model.title(), points.clone())
                        .color(ink)
                        .width(1.5),
                );
                plot.points(
                    egui_plot::Points::new(model.title(), points.clone())
                        .color(ink)
                        .radius(2.),
                );
                bounds.push((bound, ink));
            }
            // One bound serves every model's autocorrelation.
            if k == 0 {
                bounds.truncate(1);
            }
            for (bound, ink) in bounds {
                let ink = if k == 0 { p.text_muted } else { ink };
                for side in [bound, -bound].into_iter().filter(|v| v.is_finite()) {
                    plot.hline(
                        egui_plot::HLine::new("99 % bound", side)
                            .color(ink)
                            .style(egui_plot::LineStyle::dashed_dense())
                            .width(1.),
                    );
                }
            }
        });
    }
}

fn step_view(ui: &mut egui::Ui, shown: &[&Model], height: f32) {
    chart("sysid_step", height, "Time (s)", "Step response").show(ui, |plot| {
        for model in shown {
            let Some(r) = &model.result else { continue };
            plot.line(
                egui_plot::Line::new(model.title(), r.step.clone())
                    .color(colour(model.color))
                    .width(1.5),
            );
        }
    });
}

/// Magnitude and phase over a logarithmic frequency axis.
fn bode_view(ui: &mut egui::Ui, shown: &[&Model], height: f32) {
    let half = halves(height);
    for (k, label) in ["Magnitude (dB)", "Phase (deg)"].into_iter().enumerate() {
        if k == 1 {
            ui.add_space(crate::tokens::S);
        }
        // x is log10 of the frequency; the marks read in rad/s.
        chart(&format!("sysid_bode_{k}"), half, "Frequency (rad/s)", label)
            .link_axis(egui::Id::new("sysid_bode_axes"), [true, false])
            .x_axis_formatter(|mark, _| short(10_f64.powf(mark.value)))
            .show(ui, |plot| {
                for model in shown {
                    let Some(r) = &model.result else { continue };
                    let response = if k == 0 { &r.magnitude } else { &r.phase };
                    let points: Vec<[f64; 2]> =
                        response.iter().map(|p| [p[0].log10(), p[1]]).collect();
                    plot.line(
                        egui_plot::Line::new(model.title(), points)
                            .color(colour(model.color))
                            .width(1.5),
                    );
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Output UI

impl SystemIdentification {
    /// The model list: one row per model with its fit, FPE and loss.
    fn list_ui(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Models");
        if self.models.is_empty() {
            crate::tokens::helper(ui, "No models yet.");
            return;
        }
        let mut chosen = None;
        egui::ScrollArea::horizontal()
            .id_salt("sysid_models_scroll")
            .auto_shrink([false, true])
            .show_themed(ui, |ui| {
                egui::Grid::new("sysid_models_table")
                    .num_columns(7)
                    .spacing(egui::vec2(crate::tokens::M, crate::tokens::XS))
                    .show(ui, |ui| {
                        for title in ["Show", "Model", "Type", "Orders", "Fit %", "FPE", "Loss"] {
                            muted(ui, title);
                        }
                        ui.end_row();
                        for (k, model) in self.models.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut model.shown, "")
                                    .on_hover_text("Show in the views");
                                let response = crate::tokens::color_swatch(
                                    ui,
                                    crate::tokens::M,
                                    colour(model.color),
                                    egui::Sense::click(),
                                );
                                if response.on_hover_text("Change series colour").clicked() {
                                    model.color = (model.color + 1) % matrust::plot::COLORS.len();
                                }
                            });
                            if ui
                                .selectable_label(k == self.selected, model.title())
                                .clicked()
                            {
                                chosen = Some(k);
                            }
                            ui.label(model.kind.label());
                            ui.label(model.describe());
                            match &model.result {
                                Some(r) => {
                                    ui.label(short(r.fit));
                                    ui.label(short(r.fpe));
                                    ui.label(short(r.loss));
                                }
                                None => {
                                    for _ in 0..3 {
                                        ui.label("—");
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(k) = chosen {
            self.selected = k;
        }
        crate::tokens::helper(
            ui,
            "Fit % is on the validation range; FPE and loss are from the estimation range.",
        );
    }

    /// The exported names: the iddata objects and the ticked models.
    fn export_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        heading(ui, "Export");
        let [data, estimation, validation, fit] = &mut self.names;
        if self.source == Source::Vectors {
            name_field(ui, "iddata object", data, ws);
        }
        name_field(ui, "Estimation data", estimation, ws);
        name_field(ui, "Validation data", validation, ws);
        if !self.models.is_empty() {
            ui.label("Models (tick to export)");
        }
        for model in &mut self.models {
            ui.horizontal(|ui| {
                ui.checkbox(&mut model.export, "")
                    .on_hover_text("Export and Generate script include this model");
                let width = ui.available_width();
                text_input(ui, &mut model.name, "Variable name", width);
            });
            if model.export {
                name_message(ui, &model.name, ws);
            }
        }
        ui.add_space(crate::tokens::XS);
        if self.models.iter().any(|m| m.export) {
            name_field(ui, "Validation fits (percent)", fit, ws);
        }
        crate::tokens::helper(
            ui,
            "Unticked models stay in the app; the script estimates and compares the ticked ones.",
        );
    }
}
