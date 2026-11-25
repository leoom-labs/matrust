//! Linear System Analyzer: compares the time and frequency responses of LTI
//! models (tf, zpk, ss; continuous or discrete; MIMO ss) in one to four
//! plots. A system is a workspace model, used by name, or an expression of
//! models (`feedback(G*C, 1)`) that the script assigns to a new name. One
//! script draws the figure, prints the characteristics and assigns the
//! active plot's response data; its private run and a probe give every curve
//! and number the app shows, all from engine builtins (`step`, `impulse`,
//! `initial`, `lsim`, `bode`, `nyquist`, `nichols`, `sigma`, `pzmap`,
//! `stepinfo`, `allmargin`, `margin`, `pole`, `zero`, `dcgain`, `damp`,
//! `isstable`).
//!
//! The engine has no `bodemag`, `iopzmap` or `sys(i, j)` indexing, so the
//! script uses plain MATLAB in their place: Bode magnitude is `sigma` of
//! one-input one-output channels (their one singular value is the gain), I/O
//! pole-zero is `pzmap` per channel, and a channel of a MIMO model P is
//! `ss(P.A, P.B(:, j), P.C(i, :), P.D(i, j), P.Ts)` (MIMO models are ss).
use super::{
    App, AppKind, Outputs, Request, Runner, Snapshot, field, name_field, number, short, unused_name,
};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// Most plots of the layout.
pub const MAX_PLOTS: usize = 4;
/// Most systems shown at once (the response builtins overlay up to 16).
pub const MAX_SHOWN: usize = 8;
/// Most expression systems in the list.
pub const MAX_EXPRESSIONS: usize = 16;
const MAX_EXPRESSION_LEN: usize = 400;
/// Frequencies of a typed range (as many as the automatic grid).
pub const FREQUENCIES: usize = 1000;
/// Most initial-state values.
const MAX_STATES: usize = 256;
/// Height of the Output column's system tables before they scroll.
const TABLES_HEIGHT: f32 = 240.;

// ---------------------------------------------------------------------------
// Plot types and options

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlotType {
    Step,
    Impulse,
    Initial,
    Bode,
    BodeMagnitude,
    Nyquist,
    Nichols,
    Sigma,
    PoleZero,
    IoPoleZero,
    Lsim,
}
impl PlotType {
    pub const ALL: [Self; 11] = [
        Self::Step,
        Self::Impulse,
        Self::Initial,
        Self::Bode,
        Self::BodeMagnitude,
        Self::Nyquist,
        Self::Nichols,
        Self::Sigma,
        Self::PoleZero,
        Self::IoPoleZero,
        Self::Lsim,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Step => "Step",
            Self::Impulse => "Impulse",
            Self::Initial => "Initial condition",
            Self::Bode => "Bode",
            Self::BodeMagnitude => "Bode magnitude",
            Self::Nyquist => "Nyquist",
            Self::Nichols => "Nichols",
            Self::Sigma => "Singular values",
            Self::PoleZero => "Pole-zero",
            Self::IoPoleZero => "I/O pole-zero",
            Self::Lsim => "Linear simulation",
        }
    }
    fn code(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap()
    }
    fn from_code(code: f64) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }
    /// The characteristics this plot can mark.
    pub fn characteristics(self) -> &'static [Char] {
        match self {
            Self::Step => &[Char::Peak, Char::Settling, Char::Rise, Char::Steady],
            Self::Impulse => &[Char::Peak, Char::Settling],
            Self::Bode | Self::Nyquist | Self::Nichols => &[Char::AllMargins, Char::MinMargins],
            Self::BodeMagnitude | Self::Sigma => &[Char::PeakGain],
            _ => &[],
        }
    }
    /// Export: the response data's output names (None: an output left out)
    /// and the function that returns them.
    pub fn outputs(self) -> (&'static [Option<&'static str>], &'static str) {
        match self {
            Self::Step => (&[Some("y"), Some("t")], "step"),
            Self::Impulse => (&[Some("y"), Some("t")], "impulse"),
            Self::Initial => (&[Some("y"), Some("t")], "initial"),
            Self::Lsim => (&[Some("y"), Some("t")], "lsim"),
            Self::Bode => (&[Some("mag"), Some("phase"), Some("w")], "bode"),
            Self::BodeMagnitude => (&[Some("mag"), None, Some("w")], "bode"),
            Self::Nyquist => (&[Some("re"), Some("im"), Some("w")], "nyquist"),
            Self::Nichols => (&[Some("mag"), Some("phase"), Some("w")], "nichols"),
            Self::Sigma => (&[Some("sv"), Some("w")], "sigma"),
            Self::PoleZero | Self::IoPoleZero => (&[Some("p"), Some("z")], "pzmap"),
        }
    }
}

/// Characteristics a plot marks on its curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Char {
    Peak,
    Settling,
    Rise,
    Steady,
    AllMargins,
    MinMargins,
    PeakGain,
}
impl Char {
    pub fn label(self) -> &'static str {
        match self {
            Self::Peak => "Peak response",
            Self::Settling => "Settling time",
            Self::Rise => "Rise time",
            Self::Steady => "Steady state",
            Self::AllMargins => "All stability margins",
            Self::MinMargins => "Minimum stability margins",
            Self::PeakGain => "Peak gain",
        }
    }
}

/// Line styles of the app's plots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Solid,
    Dashed,
    Dotted,
    LongDash,
}
impl Style {
    pub const ALL: [Self; 4] = [Self::Solid, Self::Dashed, Self::Dotted, Self::LongDash];
    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Dashed => "Dashed",
            Self::Dotted => "Dotted",
            Self::LongDash => "Long dash",
        }
    }
    fn line(self) -> egui_plot::LineStyle {
        match self {
            Self::Solid => egui_plot::LineStyle::Solid,
            Self::Dashed => egui_plot::LineStyle::dashed_dense(),
            Self::Dotted => egui_plot::LineStyle::dotted_dense(),
            Self::LongDash => egui_plot::LineStyle::dashed_loose(),
        }
    }
}

/// sigma's type argument: the singular values of H, inv(H), I + H, I + inv(H).
pub const SIGMA_TYPES: [&str; 4] = ["H", "inv(H)", "I + H", "I + inv(H)"];

/// One plot of the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotSpec {
    pub kind: PlotType,
    /// Characteristics switched on (those of other plot types are kept for
    /// when the type comes back).
    pub chars: Vec<Char>,
    /// Initial condition x0, typed.
    pub x0: String,
    /// Nichols: draw ngrid.
    pub ngrid: bool,
    pub sigma_type: u8,
    /// Linear simulation: workspace input signal and time vector.
    pub u: String,
    pub t: String,
}
impl PlotSpec {
    pub fn new(kind: PlotType) -> Self {
        Self {
            kind,
            chars: Vec::new(),
            x0: String::new(),
            ngrid: false,
            sigma_type: 0,
            u: String::new(),
            t: String::new(),
        }
    }
    pub fn has(&self, c: Char) -> bool {
        self.kind.characteristics().contains(&c) && self.chars.contains(&c)
    }
    pub fn set(&mut self, c: Char, on: bool) {
        self.chars.retain(|k| *k != c);
        if on {
            self.chars.push(c);
            self.chars.sort();
        }
    }
    fn any(&self) -> bool {
        self.kind.characteristics().iter().any(|c| self.has(*c))
    }
}

/// One system of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct System {
    pub name: String,
    /// None: a workspace model; else the expression the script assigns.
    pub expression: Option<String>,
    /// An expression system's name was typed (else the first free sysN).
    pub name_edited: bool,
    pub shown: bool,
    /// Index into the colour order.
    pub color: usize,
    pub style: Style,
}

/// Sizes of a model, from the systems query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub outputs: usize,
    pub inputs: usize,
    /// States of an ss model (0 for tf and zpk).
    pub states: usize,
    /// Sample time; 0 for continuous models.
    pub ts: f64,
    pub ss: bool,
}
impl Shape {
    pub fn siso(&self) -> bool {
        self.outputs == 1 && self.inputs == 1
    }
}

// ---------------------------------------------------------------------------
// Text helpers

fn parse_positive(text: &str, what: &str) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0. => Ok(v),
        _ => Err(format!(
            "Linear System Analyzer: {what} must be a positive number"
        )),
    }
}

/// Numbers typed as `1 0`, `[1; 0]` or `1, 0`.
pub fn parse_vector(text: &str, what: &str) -> Result<Vec<f64>, String> {
    let inner = text.trim();
    let inner = inner
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(inner);
    let values = inner
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<f64>>>()
        .ok_or_else(|| {
            format!("Linear System Analyzer: type {what} as numbers, for example 1 0")
        })?;
    if values.is_empty() {
        return Err(format!(
            "Linear System Analyzer: type {what}, for example 1 0"
        ));
    }
    if values.len() > MAX_STATES {
        return Err(format!(
            "Linear System Analyzer: {what} has at most {MAX_STATES} values"
        ));
    }
    Ok(values)
}

fn column(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(|v| number(*v)).collect();
    format!("[{}]", items.join("; "))
}

/// A MATLAB char vector literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Checks a typed expression: one expression on one line, no statement
/// separators or comments outside brackets and strings.
pub fn check_expression(text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type an expression, for example feedback(G*C, 1)".into());
    }
    if text.len() > MAX_EXPRESSION_LEN {
        return Err(format!(
            "An expression has at most {MAX_EXPRESSION_LEN} characters"
        ));
    }
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        if let Some(q) = quote {
            if c == '\n' || c == '\r' {
                return Err("An expression is one line".into());
            }
            if c == q {
                if chars.get(k + 1) == Some(&q) {
                    k += 2;
                    continue;
                }
                quote = None;
                previous = 'x';
            }
            k += 1;
            continue;
        }
        match c {
            '\n' | '\r' => return Err("An expression is one line".into()),
            '"' => quote = Some('"'),
            '\'' if !(previous.is_alphanumeric()
                || matches!(previous, '_' | ')' | ']' | '}' | '\'' | '.')) =>
            {
                quote = Some('\'')
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return Err("The expression's brackets do not match".into());
                }
            }
            ';' | ',' if depth == 0 => {
                return Err(format!("Type one expression: no {c} outside brackets"));
            }
            '%' | '#' => return Err("An expression has no comments".into()),
            '.' if chars[k..].starts_with(&['.', '.', '.']) => {
                return Err("An expression is one line".into());
            }
            '=' if depth == 0 => {
                let before = if k > 0 { chars[k - 1] } else { ' ' };
                let after = chars.get(k + 1).copied().unwrap_or(' ');
                if !(matches!(before, '=' | '~' | '<' | '>') || after == '=') {
                    return Err("Type the expression only: the app names the new system".into());
                }
            }
            _ => {}
        }
        if !c.is_whitespace() {
            previous = c;
        }
        k += 1;
    }
    if quote.is_some() {
        return Err("The expression has an unclosed string".into());
    }
    if depth != 0 {
        return Err("The expression's brackets do not match".into());
    }
    Ok(())
}

/// Names an expression reads: identifiers outside strings that do not
/// follow a dot (field names).
pub fn identifiers(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        if let Some(q) = quote {
            if c == q {
                quote = None;
                previous = 'x';
            }
            k += 1;
            continue;
        }
        if c == '"'
            || (c == '\''
                && !(previous.is_alphanumeric()
                    || matches!(previous, '_' | ')' | ']' | '}' | '\'' | '.')))
        {
            quote = Some(c);
            k += 1;
            continue;
        }
        if c.is_ascii_digit() {
            // A number, its exponent and an imaginary unit.
            while k < chars.len() && (chars[k].is_ascii_alphanumeric() || chars[k] == '.') {
                if matches!(chars[k], 'e' | 'E')
                    && matches!(chars.get(k + 1), Some('+') | Some('-'))
                {
                    k += 1;
                }
                k += 1;
            }
            previous = '0';
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = k;
            while k < chars.len() && (chars[k].is_ascii_alphanumeric() || chars[k] == '_') {
                k += 1;
            }
            let name: String = chars[start..k].iter().collect();
            if previous != '.' && !out.contains(&name) {
                out.push(name);
            }
            previous = 'x';
            continue;
        }
        if !c.is_whitespace() {
            previous = c;
        }
        k += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Layout grid

/// A rectangle of a rows-by-cols grid, rows from the top, ends exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub rows: usize,
    pub cols: usize,
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
}
impl Cell {
    pub const WHOLE: Self = Self {
        rows: 1,
        cols: 1,
        top: 0,
        bottom: 1,
        left: 0,
        right: 1,
    };
    /// Plot k (0-based) of an n-plot layout: two side by side, four in a
    /// 2-by-2 grid, three as two above one.
    pub fn of_layout(n: usize, k: usize) -> Self {
        match (n, k) {
            (0 | 1, _) => Self::WHOLE,
            (2, k) => Self {
                rows: 1,
                cols: 2,
                top: 0,
                bottom: 1,
                left: k,
                right: k + 1,
            },
            (3, 2) => Self {
                rows: 2,
                cols: 2,
                top: 1,
                bottom: 2,
                left: 0,
                right: 2,
            },
            (_, k) => Self {
                rows: 2,
                cols: 2,
                top: k / 2,
                bottom: k / 2 + 1,
                left: k % 2,
                right: k % 2 + 1,
            },
        }
    }
    /// Panel (i, j) of a p-by-m split of this cell, on the grid refined p by
    /// m times (the cell a response grid draws in).
    pub fn split(self, p: usize, m: usize, i: usize, j: usize) -> Self {
        let (h, w) = (self.bottom - self.top, self.right - self.left);
        Self {
            rows: self.rows * p,
            cols: self.cols * m,
            top: self.top * p + i * h,
            bottom: self.top * p + (i + 1) * h,
            left: self.left * m + j * w,
            right: self.left * m + (j + 1) * w,
        }
    }
    /// The `subplot` call that selects this cell.
    pub fn subplot(self) -> String {
        let cells: Vec<String> = (self.top..self.bottom)
            .flat_map(|r| (self.left..self.right).map(move |c| (r * self.cols + c + 1).to_string()))
            .collect();
        let index = if cells.len() == 1 {
            cells[0].clone()
        } else {
            format!("[{}]", cells.join(" "))
        };
        format!("subplot({}, {}, {index})", self.rows, self.cols)
    }
    /// The cell's part of `area`.
    fn rect(self, area: egui::Rect, gap: f32) -> egui::Rect {
        let w = area.width() / self.cols as f32;
        let h = area.height() / self.rows as f32;
        egui::Rect::from_min_max(
            egui::pos2(
                area.left() + self.left as f32 * w,
                area.top() + self.top as f32 * h,
            ),
            egui::pos2(
                area.left() + self.right as f32 * w,
                area.top() + self.bottom as f32 * h,
            ),
        )
        .shrink(gap / 2.)
    }
}

// ---------------------------------------------------------------------------
// Results of a private run

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepInfo {
    pub rise: f64,
    pub settling: f64,
    pub overshoot: f64,
    pub peak: f64,
    pub peak_time: f64,
    pub undershoot: f64,
}

/// `allmargin` (every crossing) and `margin` (the minimum ones).
#[derive(Clone, Debug, PartialEq)]
pub struct Margins {
    pub gain: Vec<f64>,
    pub gain_w: Vec<f64>,
    pub phase: Vec<f64>,
    pub phase_w: Vec<f64>,
    pub delay: Vec<f64>,
    pub delay_w: Vec<f64>,
    /// margin: [Gm, Pm, Wcg, Wcp].
    pub min: [f64; 4],
}

/// The Output table of one shown system.
#[derive(Clone, Debug)]
pub struct SystemResult {
    /// The list name (the colour and style follow it).
    pub base: String,
    /// The name, with the I/O pair when one is chosen.
    pub label: String,
    /// Sizes of the analysed system (a channel is 1-by-1).
    pub shape: Shape,
    pub poles: Vec<[f64; 2]>,
    /// None: the engine has no zeros for this (MIMO) system.
    pub zeros: Option<Vec<[f64; 2]>>,
    pub dc: Vec<f64>,
    pub wn: Vec<f64>,
    pub zeta: Vec<f64>,
    pub stable: bool,
    pub stepinfo: Option<StepInfo>,
    pub margins: Option<Margins>,
}

/// What one plot draws for one system.
#[derive(Clone, Debug)]
pub enum Curves {
    /// Time responses: `traces` columns of `y` against `t`.
    Time {
        t: Vec<f64>,
        y: Vec<f64>,
        traces: usize,
        /// Impulse: peak (t, y) and settling time.
        peak: Option<[f64; 2]>,
        settling: Option<f64>,
    },
    /// Frequency responses: `traces` columns of `a` (magnitude, real part)
    /// and `b` (phase, imaginary part; empty for Bode magnitude) against w.
    Frequency {
        w: Vec<f64>,
        a: Vec<f64>,
        b: Vec<f64>,
        traces: usize,
        /// Bode magnitude: peak (w, gain).
        peak: Option<[f64; 2]>,
    },
    /// Singular values: `count` rows of `sv` per frequency.
    Sigma {
        w: Vec<f64>,
        sv: Vec<f64>,
        count: usize,
        peak: Option<[f64; 2]>,
    },
    /// Poles and zeros per channel, channels row by row.
    PoleZero { channels: Vec<[Vec<[f64; 2]>; 2]> },
}

#[derive(Clone, Debug)]
pub struct TileResult {
    pub kind: PlotType,
    /// The curves of each system the plot shows (index into `systems`).
    pub curves: Vec<(usize, Curves)>,
}

/// Everything one successful run computed.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub systems: Vec<SystemResult>,
    pub tiles: Vec<TileResult>,
}

fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        Some(Array::Logical(m)) => Ok(m.data().iter().map(|b| f64::from(*b)).collect()),
        _ => Err(format!(
            "Linear System Analyzer: {name} is not a real array"
        )),
    }
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("Linear System Analyzer: {name} is empty"))
}
fn text(outputs: &Outputs, name: &str) -> Result<String, String> {
    match outputs.get(name) {
        Some(Array::Character(c)) => Ok(String::from_utf16_lossy(c.units())),
        Some(Array::Numeric(m)) if m.data().is_empty() => Ok(String::new()),
        _ => Err(format!("Linear System Analyzer: {name} is not text")),
    }
}
fn pairs(re: &[f64], im: &[f64]) -> Vec<[f64; 2]> {
    re.iter().zip(im).map(|(r, i)| [*r, *i]).collect()
}
/// A [first, second] pair, or None for an empty or non-finite result.
fn point(v: &[f64]) -> Option<[f64; 2]> {
    (v.len() == 2 && v.iter().all(|x| x.is_finite())).then(|| [v[0], v[1]])
}

impl Analysis {
    pub fn from_outputs(outputs: &Outputs) -> Result<Self, String> {
        let n = values(outputs, "app_n")?;
        let (systems, tiles) = match n.as_slice() {
            [s, t] => (*s as usize, *t as usize),
            _ => return Err("Linear System Analyzer: app_n has the wrong size".into()),
        };
        let mut out = Self {
            systems: Vec::with_capacity(systems),
            tiles: Vec::with_capacity(tiles),
        };
        for s in 1..=systems {
            let name = |what: &str| format!("app_s{s}_{what}");
            let shape = values(outputs, &name("shape"))?;
            let [outputs_, inputs, states, ts, ss] = shape[..] else {
                return Err("Linear System Analyzer: a system shape has the wrong size".into());
            };
            let si = values(outputs, &name("si"))?;
            let mm = values(outputs, &name("mm"))?;
            let margins = if mm.len() == 4 {
                Some(Margins {
                    gain: values(outputs, &name("gm"))?,
                    gain_w: values(outputs, &name("gmw"))?,
                    phase: values(outputs, &name("pm"))?,
                    phase_w: values(outputs, &name("pmw"))?,
                    delay: values(outputs, &name("dm"))?,
                    delay_w: values(outputs, &name("dmw"))?,
                    min: [mm[0], mm[1], mm[2], mm[3]],
                })
            } else {
                None
            };
            out.systems.push(SystemResult {
                base: text(outputs, &name("base"))?,
                label: text(outputs, &name("label"))?,
                shape: Shape {
                    outputs: outputs_ as usize,
                    inputs: inputs as usize,
                    states: states as usize,
                    ts,
                    ss: ss != 0.,
                },
                poles: pairs(
                    &values(outputs, &name("pr"))?,
                    &values(outputs, &name("pi"))?,
                ),
                zeros: (scalar(outputs, &name("zok"))? != 0.).then_some(pairs(
                    &values(outputs, &name("zr"))?,
                    &values(outputs, &name("zi"))?,
                )),
                dc: values(outputs, &name("dc"))?,
                wn: values(outputs, &name("wn"))?,
                zeta: values(outputs, &name("zeta"))?,
                stable: scalar(outputs, &name("st"))? != 0.,
                stepinfo: (si.len() == 6).then(|| StepInfo {
                    rise: si[0],
                    settling: si[1],
                    overshoot: si[2],
                    peak: si[3],
                    peak_time: si[4],
                    undershoot: si[5],
                }),
                margins,
            });
        }
        for k in 1..=tiles {
            let kind = PlotType::from_code(scalar(outputs, &format!("app_t{k}_kind"))?)
                .ok_or("Linear System Analyzer: unknown plot type")?;
            let mut curves = Vec::new();
            for s in values(outputs, &format!("app_t{k}_sys"))? {
                let s = s as usize;
                let shape = out
                    .systems
                    .get(s.wrapping_sub(1))
                    .ok_or("Linear System Analyzer: a plot names an unknown system")?
                    .shape;
                let name = |what: &str| format!("app_t{k}_{s}_{what}");
                let curve = match kind {
                    PlotType::Step | PlotType::Impulse | PlotType::Initial | PlotType::Lsim => {
                        let t = values(outputs, &name("x"))?;
                        let y = values(outputs, &name("y"))?;
                        let traces = if t.is_empty() { 0 } else { y.len() / t.len() };
                        let (peak, settling) = if kind == PlotType::Impulse && shape.siso() {
                            (
                                point(&values(outputs, &name("pk"))?),
                                values(outputs, &name("ts"))?
                                    .first()
                                    .copied()
                                    .filter(|v| v.is_finite()),
                            )
                        } else {
                            (None, None)
                        };
                        Curves::Time {
                            t,
                            y,
                            traces,
                            peak,
                            settling,
                        }
                    }
                    PlotType::Bode
                    | PlotType::BodeMagnitude
                    | PlotType::Nyquist
                    | PlotType::Nichols => {
                        let w = values(outputs, &name("x"))?;
                        let a = values(outputs, &name("y"))?;
                        let b = if kind == PlotType::BodeMagnitude {
                            Vec::new()
                        } else {
                            values(outputs, &name("z"))?
                        };
                        let traces = if w.is_empty() { 0 } else { a.len() / w.len() };
                        let peak = if kind == PlotType::BodeMagnitude && shape.siso() {
                            point(&values(outputs, &name("pk"))?)
                        } else {
                            None
                        };
                        Curves::Frequency {
                            w,
                            a,
                            b,
                            traces,
                            peak,
                        }
                    }
                    PlotType::Sigma => {
                        let w = values(outputs, &name("x"))?;
                        let sv = values(outputs, &name("y"))?;
                        let count = if w.is_empty() { 0 } else { sv.len() / w.len() };
                        Curves::Sigma {
                            w,
                            sv,
                            count,
                            peak: point(&values(outputs, &name("pk"))?),
                        }
                    }
                    PlotType::PoleZero | PlotType::IoPoleZero => {
                        let (p, m) = if kind == PlotType::IoPoleZero {
                            (shape.outputs, shape.inputs)
                        } else {
                            (1, 1)
                        };
                        let mut channels = Vec::with_capacity(p * m);
                        for i in 1..=p {
                            for j in 1..=m {
                                let c = |what: &str| name(&format!("{i}_{j}_{what}"));
                                channels.push([
                                    pairs(&values(outputs, &c("pr"))?, &values(outputs, &c("pi"))?),
                                    pairs(&values(outputs, &c("zr"))?, &values(outputs, &c("zi"))?),
                                ]);
                            }
                        }
                        Curves::PoleZero { channels }
                    }
                };
                curves.push((s - 1, curve));
            }
            out.tiles.push(TileResult { kind, curves });
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// The systems query: class and sizes of every system, off the UI thread

/// What the systems query learnt, for the request it answered.
#[derive(Default)]
struct ShapeQuery {
    runner: Runner,
    key: Option<u64>,
    /// The whole query failed (no per-system result).
    error: Option<String>,
    known: BTreeMap<String, Result<Shape, String>>,
}

/// The pieces the script, the probe and the export share, checked.
struct Plan {
    /// Expression systems in list order: name and expression.
    definitions: Vec<(String, String)>,
    /// Channels of MIMO models: variable, model, output, input (1-based).
    channels: Vec<(String, String, usize, usize)>,
    shown: Vec<Shown>,
    tiles: Vec<Tile>,
    /// The frequency argument of a typed range.
    frequency: Option<String>,
    final_time: Option<String>,
    /// The export assignment: output names and the line.
    export: Option<(Vec<String>, String)>,
}
/// A shown system as the calls use it.
struct Shown {
    /// The variable the calls name (a channel variable for an I/O pair).
    var: String,
    base: String,
    label: String,
    shape: Shape,
}
struct Tile {
    spec: PlotSpec,
    cell: Cell,
    /// Shown systems this plot applies to (Initial condition: ss only).
    systems: Vec<usize>,
    x0: Option<String>,
}

pub struct LinearSystemAnalyzer {
    pub systems: Vec<System>,
    /// Always MAX_PLOTS specs; the first `count` are shown.
    pub plots: Vec<PlotSpec>,
    pub count: usize,
    /// The plot Export reads.
    pub active: usize,
    pub freq_auto: bool,
    pub wmin: String,
    pub wmax: String,
    pub time_auto: bool,
    pub final_time: String,
    /// Frequencies in Hz (else rad/s) and magnitudes in dB (else absolute).
    pub hz: bool,
    pub db: bool,
    /// One I/O pair (output, input), 1-based; None: every pair.
    pub io: Option<(usize, usize)>,
    /// The Add expression field and its last error.
    pub expression: String,
    pub expression_error: Option<String>,
    /// The system whose data Export assigns.
    pub export_system: String,
    /// Export output names by base name: the name and whether it was typed.
    pub export_names: BTreeMap<String, (String, bool)>,
    /// Last good run; errors keep it on screen.
    pub current: Option<Analysis>,
    /// Workspace variables the script reads (from `prepare`).
    reads: Vec<String>,
    /// Real numeric workspace arrays for Linear simulation: name and size.
    numeric: Vec<(String, String)>,
    shapes: ShapeQuery,
    next_color: usize,
    /// The first workspace model was ticked once.
    ticked: bool,
}
impl Default for LinearSystemAnalyzer {
    fn default() -> Self {
        Self {
            systems: Vec::new(),
            plots: [
                PlotType::Step,
                PlotType::Bode,
                PlotType::Impulse,
                PlotType::PoleZero,
            ]
            .map(PlotSpec::new)
            .to_vec(),
            count: 2,
            active: 0,
            freq_auto: true,
            wmin: "0.1".into(),
            wmax: "100".into(),
            time_auto: true,
            final_time: "10".into(),
            hz: false,
            db: true,
            io: None,
            expression: String::new(),
            expression_error: None,
            export_system: String::new(),
            export_names: BTreeMap::new(),
            current: None,
            reads: Vec::new(),
            numeric: Vec::new(),
            shapes: ShapeQuery::default(),
            next_color: 0,
            ticked: false,
        }
    }
}

const NOT_SHOWN: &str = "Linear System Analyzer: tick a system in INPUTS, or add an expression";

impl LinearSystemAnalyzer {
    /// Adds the expression typed in the Add expression field as a new system
    /// (shown), named the first free sysN.
    pub fn add_expression(&mut self, ws: &Snapshot) -> Result<(), String> {
        let text = self.expression.trim().to_owned();
        let checked = check_expression(&text).and_then(|()| {
            if self
                .systems
                .iter()
                .filter(|s| s.expression.is_some())
                .count()
                >= MAX_EXPRESSIONS
            {
                Err(format!("At most {MAX_EXPRESSIONS} expression systems"))
            } else {
                Ok(())
            }
        });
        if let Err(error) = checked {
            self.expression_error = Some(error.clone());
            return Err(error);
        }
        let name = self.free_name(ws, self.systems.len());
        let shown = self.systems.iter().filter(|s| s.shown).count() < MAX_SHOWN;
        let color = self.take_color();
        self.systems.push(System {
            name,
            expression: Some(text),
            name_edited: false,
            shown,
            color,
            style: Style::Solid,
        });
        self.expression.clear();
        self.expression_error = None;
        Ok(())
    }
    /// Removes an expression system.
    pub fn remove(&mut self, index: usize) {
        if self
            .systems
            .get(index)
            .is_some_and(|s| s.expression.is_some())
        {
            self.systems.remove(index);
        }
    }
    fn take_color(&mut self) -> usize {
        let color = self.next_color % matrust::plot::COLORS.len();
        self.next_color += 1;
        color
    }
    /// The first sysN that no workspace variable and no system before
    /// `index` uses.
    fn free_name(&self, ws: &Snapshot, index: usize) -> String {
        (1..)
            .map(|n| format!("sys{n}"))
            .find(|n| !ws.contains(n) && !self.systems[..index].iter().any(|s| s.name == *n))
            .unwrap()
    }
    /// Lists the workspace models (new ones unticked, the first one ever
    /// ticked), drops vanished ones and renames default-named expression
    /// systems whose name a workspace variable took (after an Export).
    fn sync(&mut self, ws: &Snapshot) {
        let models: Vec<String> = ws.lti_models().map(|(name, _)| name.to_owned()).collect();
        self.systems
            .retain(|s| s.expression.is_some() || models.contains(&s.name));
        for name in models {
            if !self
                .systems
                .iter()
                .any(|s| s.expression.is_none() && s.name == name)
            {
                let color = self.take_color();
                self.systems.push(System {
                    name,
                    expression: None,
                    name_edited: false,
                    shown: false,
                    color,
                    style: Style::Solid,
                });
            }
        }
        if !self.ticked
            && let Some(first) = self.systems.iter_mut().find(|s| s.expression.is_none())
        {
            first.shown = true;
            self.ticked = true;
        }
        for k in 0..self.systems.len() {
            let s = &self.systems[k];
            if s.expression.is_some()
                && !s.name_edited
                && (ws.contains(&s.name) || self.systems[..k].iter().any(|t| t.name == s.name))
            {
                self.systems[k].name = self.free_name(ws, k);
            }
        }
    }
    fn used_plots(&self) -> &[PlotSpec] {
        &self.plots[..self.count.clamp(1, MAX_PLOTS)]
    }

    // --- systems query

    /// Systems the script defines or shows, in list order.
    fn needed(&self) -> Vec<&System> {
        self.systems
            .iter()
            .filter(|s| s.shown || s.expression.is_some())
            .collect()
    }
    /// The query for the class and sizes of every needed system; each
    /// system is tried on its own, so one bad expression names itself.
    pub fn shape_request(&self) -> Option<(Request, Vec<String>)> {
        let needed = self.needed();
        if needed.is_empty() {
            return None;
        }
        let mut code = String::new();
        let mut names = Vec::new();
        for (k, s) in needed.iter().enumerate() {
            let k = k + 1;
            code.push_str(&format!("app_q{k} = []; app_m{k} = '';\ntry\n"));
            if let Some(expr) = &s.expression {
                code.push_str(&format!("{} = eval({});\n", s.name, quote(expr)));
            }
            code.push_str(&format!(
                "app_X = {};\n\
                 app_c = class(app_X);\n\
                 if ~any(strcmp(app_c, {{'tf', 'zpk', 'ss'}}))\n\
                 error('is a %s, not a tf, zpk or ss model', app_c);\n\
                 end\n\
                 if strcmp(app_c, 'ss')\n\
                 app_q{k} = [size(app_X.D), size(app_X.A, 1), app_X.Ts, 1];\n\
                 else\n\
                 app_q{k} = [1, 1, 0, app_X.Ts, 0];\n\
                 end\n\
                 catch app_err\n\
                 app_m{k} = app_err.message;\n\
                 end\n",
                s.name
            ));
            names.push(s.name.clone());
        }
        let outputs = (1..=needed.len())
            .flat_map(|k| [format!("app_q{k}"), format!("app_m{k}")])
            .collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        // The inputs' values are part of the question.
        for name in &self.reads {
            name.hash(&mut hasher);
        }
        Some((
            Request {
                key: hasher.finish(),
                inputs: Vec::new(),
                code,
                outputs,
            },
            names,
        ))
    }
    /// The query with the workspace values it reads, keyed by their versions.
    pub fn shape_request_for(&self, ws: &Snapshot) -> Option<(Request, Vec<String>)> {
        let (mut request, names) = self.shape_request()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        request.key.hash(&mut hasher);
        for name in &self.reads {
            ws.version(name).hash(&mut hasher);
            if let Some(array) = ws.get(name) {
                request.inputs.push((name.clone(), array.clone()));
            }
        }
        request.key = hasher.finish();
        Some((request, names))
    }
    /// Stores a finished systems query.
    pub fn receive_shapes(&mut self, key: u64, names: &[String], result: &Result<Outputs, String>) {
        self.shapes.key = Some(key);
        self.shapes.known.clear();
        self.shapes.error = None;
        let outputs = match result {
            Ok(outputs) => outputs,
            Err(error) => {
                self.shapes.error = Some(error.clone());
                return;
            }
        };
        for (k, name) in names.iter().enumerate() {
            let k = k + 1;
            let shape = text(outputs, &format!("app_m{k}")).and_then(|message| {
                if !message.is_empty() {
                    return Err(message);
                }
                match values(outputs, &format!("app_q{k}"))?[..] {
                    [outputs_, inputs, states, ts, ss] => Ok(Shape {
                        outputs: outputs_ as usize,
                        inputs: inputs as usize,
                        states: states as usize,
                        ts,
                        ss: ss != 0.,
                    }),
                    _ => Err("the systems query gave no sizes".into()),
                }
            });
            self.shapes.known.insert(name.clone(), shape);
        }
    }
    fn query_shapes(&mut self, ctx: &egui::Context, ws: &Snapshot) {
        if let Some((request, names)) = self.shape_request_for(ws) {
            let key = request.key;
            if self.shapes.key == Some(key) {
                return;
            }
            self.shapes.runner.submit(ctx, request);
            if let Some(result) = self.shapes.runner.result() {
                let result = result.clone();
                self.receive_shapes(key, &names, &result);
            }
        }
    }
    /// Stops the systems query (the app closed).
    pub fn stop(&mut self) {
        self.shapes.runner.stop();
    }
    /// The sizes of every needed system, once the query for the current
    /// systems finished; the error names the system that failed.
    fn shapes(&self, ws: &Snapshot) -> Result<BTreeMap<String, Shape>, String> {
        let Some((request, names)) = self.shape_request_for(ws) else {
            return Ok(BTreeMap::new());
        };
        if self.shapes.key != Some(request.key) {
            return Err("Linear System Analyzer: reading the systems…".into());
        }
        if let Some(error) = &self.shapes.error {
            return Err(format!("Linear System Analyzer: {error}"));
        }
        let mut out = BTreeMap::new();
        for name in names {
            match self.shapes.known.get(&name) {
                Some(Ok(shape)) => {
                    out.insert(name, *shape);
                }
                Some(Err(error)) => {
                    let what = match self.systems.iter().find(|s| s.name == name) {
                        Some(System {
                            expression: Some(expr),
                            ..
                        }) => format!("{name} = {expr}"),
                        _ => name.clone(),
                    };
                    return Err(format!("Linear System Analyzer: {what}: {error}"));
                }
                None => return Err("Linear System Analyzer: reading the systems…".into()),
            }
        }
        Ok(out)
    }
    /// The last query's sizes by name (for the I/O pair choices).
    fn known_shapes(&self) -> impl Iterator<Item = (&String, &Shape)> {
        self.shapes
            .known
            .iter()
            .filter_map(|(name, shape)| shape.as_ref().ok().map(|s| (name, s)))
    }
    /// The last query's error for a system, if it failed.
    pub fn system_error(&self, name: &str) -> Option<&str> {
        match self.shapes.known.get(name) {
            Some(Err(error)) => Some(error),
            _ => None,
        }
    }

    // --- the plan

    fn frequency_argument(&self) -> Result<Option<String>, String> {
        if self.freq_auto {
            return Ok(None);
        }
        let unit = if self.hz { "Hz" } else { "rad/s" };
        let lo = parse_positive(&self.wmin, &format!("the lowest frequency ({unit})"))?;
        let hi = parse_positive(&self.wmax, &format!("the highest frequency ({unit})"))?;
        if lo >= hi {
            return Err(
                "Linear System Analyzer: the lowest frequency must be below the highest".into(),
            );
        }
        let rad = |v: f64| {
            if self.hz {
                format!("2*pi*{}", number(v))
            } else {
                number(v)
            }
        };
        Ok(Some(format!(
            "logspace(log10({}), log10({}), {FREQUENCIES})",
            rad(lo),
            rad(hi)
        )))
    }
    fn final_time_argument(&self) -> Result<Option<String>, String> {
        if self.time_auto {
            return Ok(None);
        }
        parse_positive(&self.final_time, "the final time").map(|v| Some(number(v)))
    }
    /// Names the script assigns or reads, apart from the export outputs.
    fn script_names(&self, plan_channels: &[(String, String, usize, usize)]) -> Vec<String> {
        let mut names: Vec<String> = self.systems.iter().map(|s| s.name.clone()).collect();
        names.extend(plan_channels.iter().map(|c| c.0.clone()));
        names.extend(self.reads.iter().cloned());
        names
    }
    /// Expression system names: valid, distinct, and not a workspace model.
    fn check_names(&self, ws: &Snapshot) -> Result<(), String> {
        for (k, s) in self.systems.iter().enumerate() {
            if s.expression.is_none() {
                continue;
            }
            super::check_export_name(&s.name)?;
            if !matrust::files::valid_name(&s.name) {
                return Err(format!(
                    "Linear System Analyzer: the expression system name {} is not a valid variable name",
                    s.name
                ));
            }
            if self
                .systems
                .iter()
                .enumerate()
                .any(|(j, t)| j != k && t.name == s.name)
            {
                return Err(format!(
                    "Linear System Analyzer: two systems are named {}",
                    s.name
                ));
            }
            if ws.contains(&s.name)
                && matrust::engine::lti_class(ws.get(&s.name).unwrap()).is_some()
            {
                return Err(format!(
                    "Linear System Analyzer: {} is the name of a workspace model; rename the expression system",
                    s.name
                ));
            }
            if self.reads.contains(&s.name) {
                return Err(format!(
                    "Linear System Analyzer: the expression system name {} is a variable the script reads",
                    s.name
                ));
            }
        }
        Ok(())
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if !self.systems.iter().any(|s| s.shown) {
            return Err(NOT_SHOWN.into());
        }
        self.check_names(ws)?;
        for s in &self.systems {
            if s.shown && s.expression.is_none() && !ws.contains(&s.name) {
                return Err(format!(
                    "Linear System Analyzer: workspace variable {} does not exist",
                    s.name
                ));
            }
        }
        let frequency = self.frequency_argument()?;
        let final_time = self.final_time_argument()?;
        let shapes = self.shapes(ws)?;
        let definitions: Vec<(String, String)> = self
            .systems
            .iter()
            .filter_map(|s| s.expression.as_ref().map(|e| (s.name.clone(), e.clone())))
            .collect();
        let mut channels: Vec<(String, String, usize, usize)> = Vec::new();
        let mut shown = Vec::new();
        for s in self.systems.iter().filter(|s| s.shown) {
            let shape = *shapes
                .get(&s.name)
                .ok_or("Linear System Analyzer: reading the systems…")?;
            match self.io {
                Some((i, j)) if !shape.siso() => {
                    if i > shape.outputs || j > shape.inputs {
                        return Err(format!(
                            "Linear System Analyzer: {} has {} outputs and {} inputs: it has no output {i}, input {j}",
                            s.name, shape.outputs, shape.inputs
                        ));
                    }
                    let base: String = s.name.chars().take(50).collect();
                    let var = unused_name(&format!("{base}_{i}_{j}"), |n| {
                        ws.contains(n)
                            || self.systems.iter().any(|t| t.name == n)
                            || channels.iter().any(|c| c.0 == n)
                    });
                    channels.push((var.clone(), s.name.clone(), i, j));
                    shown.push(Shown {
                        var,
                        base: s.name.clone(),
                        label: format!("{} (output {i}, input {j})", s.name),
                        shape: Shape {
                            outputs: 1,
                            inputs: 1,
                            ..shape
                        },
                    });
                }
                _ => shown.push(Shown {
                    var: s.name.clone(),
                    base: s.name.clone(),
                    label: s.name.clone(),
                    shape,
                }),
            }
        }
        let mut tiles = Vec::new();
        let count = self.used_plots().len();
        for (k, spec) in self.used_plots().iter().enumerate() {
            let what = format!(
                "Linear System Analyzer: plot {} ({})",
                k + 1,
                spec.kind.label()
            );
            let systems: Vec<usize> = (0..shown.len())
                .filter(|&s| spec.kind != PlotType::Initial || shown[s].shape.ss)
                .collect();
            let mut x0 = None;
            let mimo: Vec<&Shown> = systems
                .iter()
                .map(|&s| &shown[s])
                .filter(|s| !s.shape.siso())
                .collect();
            match spec.kind {
                PlotType::Initial if !systems.is_empty() => {
                    let v = parse_vector(&spec.x0, "the initial state x0")?;
                    for &s in &systems {
                        let states = shown[s].shape.states;
                        if v.len() != states {
                            return Err(format!(
                                "{what}: x0 has {} values but {} has {states} states",
                                v.len(),
                                shown[s].label
                            ));
                        }
                    }
                    x0 = Some(column(&v));
                }
                PlotType::Lsim => {
                    for (name, which) in [(&spec.u, "input signal u"), (&spec.t, "time vector t")] {
                        if name.is_empty() {
                            return Err(format!("{what}: choose the {which}"));
                        }
                        if !ws.contains(name) {
                            return Err(format!(
                                "{what}: workspace variable {name} does not exist"
                            ));
                        }
                    }
                }
                PlotType::PoleZero if !mimo.is_empty() => {
                    return Err(format!(
                        "{what}: {} has several inputs or outputs; choose one I/O pair, or use I/O pole-zero",
                        mimo[0].label
                    ));
                }
                PlotType::BodeMagnitude | PlotType::IoPoleZero
                    if !mimo.is_empty()
                        && systems.iter().any(|&s| {
                            (shown[s].shape.outputs, shown[s].shape.inputs)
                                != (mimo[0].shape.outputs, mimo[0].shape.inputs)
                        }) =>
                {
                    return Err(format!(
                        "{what}: the systems need the same numbers of inputs and outputs; choose one I/O pair"
                    ));
                }
                _ => {}
            }
            tiles.push(Tile {
                spec: spec.clone(),
                cell: Cell::of_layout(count, k),
                systems,
                x0,
            });
        }
        let mut plan = Plan {
            definitions,
            channels,
            shown,
            tiles,
            frequency,
            final_time,
            export: None,
        };
        plan.export = self.export_line(ws, &plan)?;
        Ok(plan)
    }
    /// The plot Export reads (clamped to the layout).
    pub fn active_plot(&self) -> usize {
        self.active.min(self.used_plots().len() - 1)
    }
    /// The shown system Export reads in the active plot.
    fn export_index(&self, plan: &Plan) -> Option<usize> {
        let tile = plan.tiles.get(self.active_plot())?;
        tile.systems
            .iter()
            .copied()
            .find(|&s| plan.shown[s].base == self.export_system)
            .or_else(|| tile.systems.first().copied())
    }
    /// Default export names: the base names, numbered past every name the
    /// workspace or the script uses.
    fn default_export_names(&self, ws: &Snapshot, plan: &Plan) -> Vec<(String, String)> {
        let (outputs, _) = self.used_plots()[self.active_plot()].kind.outputs();
        let script = self.script_names(&plan.channels);
        let mut taken: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for base in outputs.iter().flatten() {
            let typed = self
                .export_names
                .get(*base)
                .filter(|(_, edited)| *edited)
                .map(|(n, _)| n.trim().to_owned());
            let name = typed.unwrap_or_else(|| {
                unused_name(base, |n| {
                    ws.contains(n) || script.iter().any(|s| s == n) || taken.iter().any(|t| t == n)
                })
            });
            taken.push(name.clone());
            out.push(((*base).to_owned(), name));
        }
        out
    }
    fn export_line(
        &self,
        ws: &Snapshot,
        plan: &Plan,
    ) -> Result<Option<(Vec<String>, String)>, String> {
        let Some(s) = self.export_index(plan) else {
            return Ok(None);
        };
        let tile = &plan.tiles[self.active_plot()];
        let shown = &plan.shown[s];
        let names = self.default_export_names(ws, plan);
        let script = self.script_names(&plan.channels);
        for (k, (_, name)) in names.iter().enumerate() {
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Linear System Analyzer: the export name {name:?} is not a valid variable name"
                ));
            }
            if names[..k].iter().any(|(_, n)| n == name) {
                return Err(format!(
                    "Linear System Analyzer: two export outputs are named {name}"
                ));
            }
            if script.contains(name) {
                return Err(format!(
                    "Linear System Analyzer: the export name {name} is a system or input the script uses"
                ));
            }
        }
        let (outputs, function) = tile.spec.kind.outputs();
        let mut given = names.iter().map(|(_, n)| n.clone());
        let mut targets: Vec<String> = outputs
            .iter()
            .map(|o| match o {
                Some(_) => given.next().unwrap_or_default(),
                None => "~".into(),
            })
            .collect();
        let mut exported: Vec<String> = names.into_iter().map(|(_, n)| n).collect();
        let pole_zero = matches!(tile.spec.kind, PlotType::PoleZero | PlotType::IoPoleZero);
        let line = if pole_zero && !shown.shape.siso() {
            // The engine has no zeros of MIMO models: poles only.
            targets.truncate(1);
            exported.truncate(1);
            format!("{} = pole({});", targets[0], shown.var)
        } else {
            let args = self.call_arguments(tile, plan, std::slice::from_ref(&shown.var));
            format!("[{}] = {function}({args});", targets.join(", "))
        };
        Ok(Some((exported, line)))
    }
    /// A plot call's arguments: the systems, then the plot's options.
    fn call_arguments(&self, tile: &Tile, plan: &Plan, vars: &[String]) -> String {
        let mut args: Vec<String> = vars.to_vec();
        match tile.spec.kind {
            PlotType::Step | PlotType::Impulse => args.extend(plan.final_time.clone()),
            PlotType::Initial => {
                args.extend(tile.x0.clone());
                args.extend(plan.final_time.clone());
            }
            PlotType::Lsim => {
                args.push(tile.spec.u.clone());
                args.push(tile.spec.t.clone());
            }
            PlotType::Bode | PlotType::BodeMagnitude | PlotType::Nyquist | PlotType::Nichols => {
                args.extend(plan.frequency.clone())
            }
            PlotType::Sigma => match (&plan.frequency, tile.spec.sigma_type) {
                (None, 0) => {}
                (Some(w), 0) => args.push(w.clone()),
                (w, kind) => {
                    args.push(w.clone().unwrap_or_else(|| "[]".into()));
                    args.push(kind.to_string());
                }
            },
            PlotType::PoleZero | PlotType::IoPoleZero => {}
        }
        args.join(", ")
    }

    // --- the script

    fn units_note(&self) -> Option<&'static str> {
        match (self.hz, self.db) {
            (false, true) => None,
            (true, true) => Some("% The app shows frequency in Hz; the plots below use rad/s.\n"),
            (false, false) => {
                Some("% The app shows magnitude as absolute values; the plots below use dB.\n")
            }
            (true, false) => Some(
                "% The app shows frequency in Hz and absolute magnitudes; the plots below use rad/s and dB.\n",
            ),
        }
    }
    /// The lines that draw one plot in its cell.
    fn tile_lines(&self, tile: &Tile, plan: &Plan, layout: bool) -> String {
        let mut out = String::new();
        let vars: Vec<String> = tile
            .systems
            .iter()
            .map(|&s| plan.shown[s].var.clone())
            .collect();
        if vars.is_empty() {
            if layout {
                out.push_str(&format!("{}\n", tile.cell.subplot()));
            }
            out.push_str("% Initial condition: no state-space (ss) system is shown.\n");
            return out;
        }
        let mimo = tile
            .systems
            .iter()
            .map(|&s| plan.shown[s].shape)
            .find(|shape| !shape.siso());
        let per_channel = matches!(
            tile.spec.kind,
            PlotType::BodeMagnitude | PlotType::IoPoleZero
        );
        if let (true, Some(shape)) = (per_channel, mimo) {
            // One panel per channel, each a one-input one-output plot.
            let (function, what) = if tile.spec.kind == PlotType::BodeMagnitude {
                ("sigma", "Bode magnitude")
            } else {
                ("pzmap", "Pole-zero")
            };
            for i in 1..=shape.outputs {
                for j in 1..=shape.inputs {
                    let cell = tile.cell.split(shape.outputs, shape.inputs, i - 1, j - 1);
                    let channels: Vec<String> = vars
                        .iter()
                        .map(|v| {
                            format!(
                                "ss({v}.A, {v}.B(:, {j}), {v}.C({i}, :), {v}.D({i}, {j}), {v}.Ts)"
                            )
                        })
                        .collect();
                    out.push_str(&format!(
                        "{}\n{function}({})\ntitle('{what}: output {i}, input {j}')\n",
                        cell.subplot(),
                        self.call_arguments(tile, plan, &channels)
                    ));
                    if tile.spec.kind == PlotType::BodeMagnitude {
                        out.push_str("ylabel('Magnitude (dB)')\n");
                    }
                }
            }
            return out;
        }
        if layout {
            out.push_str(&format!("{}\n", tile.cell.subplot()));
        }
        let args = self.call_arguments(tile, plan, &vars);
        match tile.spec.kind {
            PlotType::Step => out.push_str(&format!("step({args})\n")),
            PlotType::Impulse => out.push_str(&format!("impulse({args})\n")),
            PlotType::Initial => out.push_str(&format!("initial({args})\n")),
            PlotType::Lsim => out.push_str(&format!("lsim({args})\n")),
            PlotType::Bode => out.push_str(&format!("bode({args})\n")),
            PlotType::BodeMagnitude => out.push_str(&format!(
                "% One input and one output: the singular value is the Bode magnitude.\n\
                 sigma({args})\ntitle('Bode magnitude')\nylabel('Magnitude (dB)')\n"
            )),
            PlotType::Nyquist => out.push_str(&format!("nyquist({args})\n")),
            PlotType::Nichols => {
                out.push_str(&format!("nichols({args})\n"));
                if tile.spec.ngrid {
                    out.push_str("ngrid\n");
                }
            }
            PlotType::Sigma => out.push_str(&format!("sigma({args})\n")),
            PlotType::PoleZero => out.push_str(&format!("pzmap({args})\n")),
            PlotType::IoPoleZero => {
                out.push_str(&format!("pzmap({args})\ntitle('I/O pole-zero map')\n"))
            }
        }
        out
    }
    /// Characteristic lines: stepinfo, allmargin and peak gains of the
    /// one-input one-output systems of plots that mark them.
    fn characteristic_lines(&self, plan: &Plan) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for tile in &plan.tiles {
            if !tile.spec.any() {
                continue;
            }
            for &s in &tile.systems {
                let shown = &plan.shown[s];
                if !shown.shape.siso() && tile.spec.kind != PlotType::Sigma {
                    continue;
                }
                let line = match tile.spec.kind {
                    PlotType::Step => format!("stepinfo({})", shown.var),
                    PlotType::Bode | PlotType::Nyquist | PlotType::Nichols => {
                        format!("allmargin({})", shown.var)
                    }
                    PlotType::Sigma | PlotType::BodeMagnitude => {
                        let mut sigma_tile = Tile {
                            spec: tile.spec.clone(),
                            cell: tile.cell,
                            systems: Vec::new(),
                            x0: None,
                        };
                        sigma_tile.spec.kind = PlotType::Sigma;
                        if tile.spec.kind == PlotType::BodeMagnitude {
                            sigma_tile.spec.sigma_type = 0;
                        }
                        format!(
                            "max(max(sigma({})))",
                            self.call_arguments(
                                &sigma_tile,
                                plan,
                                std::slice::from_ref(&shown.var)
                            )
                        )
                    }
                    _ => continue,
                };
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
        }
        lines
    }
    fn script_text(&self, plan: &Plan) -> String {
        let names: Vec<&str> = plan.shown.iter().map(|s| s.label.as_str()).collect();
        let kinds: Vec<&str> = plan.tiles.iter().map(|t| t.spec.kind.label()).collect();
        let mut s = format!("% {}: {}\n", names.join(", "), kinds.join(", "));
        for (name, expr) in &plan.definitions {
            s.push_str(&format!("{name} = {expr};\n"));
        }
        for (var, base, i, j) in &plan.channels {
            s.push_str(&format!(
                "% Output {i}, input {j} of {base}.\n\
                 {var} = ss({base}.A, {base}.B(:, {j}), {base}.C({i}, :), {base}.D({i}, {j}), {base}.Ts);\n"
            ));
        }
        if let Some(note) = self.units_note() {
            s.push_str(note);
        }
        s.push_str("figure\n");
        let layout = plan.tiles.len() > 1;
        for tile in &plan.tiles {
            s.push_str(&self.tile_lines(tile, plan, layout));
        }
        let lines = self.characteristic_lines(plan);
        if !lines.is_empty() {
            s.push_str("% Characteristics (peak gains are absolute; 20*log10 gives dB)\n");
            for line in lines {
                s.push_str(&line);
                s.push('\n');
            }
        }
        if let Some((_, line)) = &plan.export {
            let k = self.active_plot();
            let s_index = self.export_index(plan).unwrap_or(0);
            s.push_str(&format!(
                "% Response data of plot {} ({}) for {}\n{line}\n",
                k + 1,
                plan.tiles[k].spec.kind.label(),
                plan.shown[s_index].label
            ));
        }
        s
    }

    // --- the probe

    fn probe_text(&self, plan: &Plan) -> (String, Vec<String>) {
        let mut code = String::new();
        let mut names: Vec<String> = vec!["app_n".into()];
        // Private copies first, so no probe name can shadow the user's names.
        let vars: Vec<&str> = plan.shown.iter().map(|s| s.var.as_str()).collect();
        code.push_str(&format!(
            "app_S = {{{}}};\napp_n = [{}, {}];\n",
            vars.join(", "),
            plan.shown.len(),
            plan.tiles.len()
        ));
        for (s, shown) in plan.shown.iter().enumerate() {
            let s = s + 1;
            let shape = shown.shape;
            code.push_str(&format!(
                "app_X = app_S{{{s}}};\n\
                 app_s{s}_base = {base};\n\
                 app_s{s}_label = {label};\n\
                 app_s{s}_shape = [{o}, {i}, {x}, {ts}, {ss}];\n\
                 app_p = pole(app_X); app_s{s}_pr = real(app_p(:)); app_s{s}_pi = imag(app_p(:));\n\
                 try\n\
                 app_z = zero(app_X); app_s{s}_zr = real(app_z(:)); app_s{s}_zi = imag(app_z(:)); app_s{s}_zok = 1;\n\
                 catch\n\
                 app_s{s}_zr = []; app_s{s}_zi = []; app_s{s}_zok = 0;\n\
                 end\n\
                 try\n\
                 app_s{s}_dc = dcgain(app_X); app_s{s}_dc = app_s{s}_dc(:);\n\
                 catch\n\
                 app_s{s}_dc = [];\n\
                 end\n\
                 [app_wn, app_zeta] = damp(app_X); app_s{s}_wn = app_wn(:); app_s{s}_zeta = app_zeta(:);\n\
                 app_s{s}_st = double(isstable(app_X));\n\
                 app_s{s}_si = []; app_s{s}_mm = [];\n\
                 app_s{s}_gm = []; app_s{s}_gmw = []; app_s{s}_pm = []; app_s{s}_pmw = []; app_s{s}_dm = []; app_s{s}_dmw = [];\n",
                base = quote(&shown.base),
                label = quote(&shown.label),
                o = shape.outputs,
                i = shape.inputs,
                x = shape.states,
                ts = number(shape.ts),
                ss = u8::from(shape.ss),
            ));
            if shape.siso() {
                code.push_str(&format!(
                    "try\n\
                     app_i = stepinfo(app_X);\n\
                     app_s{s}_si = [app_i.RiseTime, app_i.SettlingTime, app_i.Overshoot, app_i.Peak, app_i.PeakTime, app_i.Undershoot];\n\
                     catch\n\
                     end\n\
                     try\n\
                     app_a = allmargin(app_X);\n\
                     [app_g, app_ph, app_wg, app_wp] = margin(app_X);\n\
                     app_s{s}_gm = app_a.GainMargin; app_s{s}_gmw = app_a.GMFrequency;\n\
                     app_s{s}_pm = app_a.PhaseMargin; app_s{s}_pmw = app_a.PMFrequency;\n\
                     app_s{s}_dm = app_a.DelayMargin; app_s{s}_dmw = app_a.DMFrequency;\n\
                     app_s{s}_mm = [app_g, app_ph, app_wg, app_wp];\n\
                     catch\n\
                     end\n"
                ));
            }
            for what in [
                "base", "label", "shape", "pr", "pi", "zr", "zi", "zok", "dc", "wn", "zeta", "st",
                "si", "mm", "gm", "gmw", "pm", "pmw", "dm", "dmw",
            ] {
                names.push(format!("app_s{s}_{what}"));
            }
        }
        for (k, tile) in plan.tiles.iter().enumerate() {
            let k = k + 1;
            let systems: Vec<String> = tile.systems.iter().map(|s| (s + 1).to_string()).collect();
            code.push_str(&format!(
                "app_t{k}_kind = {};\napp_t{k}_sys = [{}];\n",
                tile.spec.kind.code(),
                systems.join(" ")
            ));
            names.push(format!("app_t{k}_kind"));
            names.push(format!("app_t{k}_sys"));
            for &s in &tile.systems {
                let shown = &plan.shown[s];
                let s = s + 1;
                let p = format!("app_t{k}_{s}");
                let args = self.call_arguments(tile, plan, &[format!("app_S{{{s}}}")]);
                let siso = shown.shape.siso();
                let mut out = |what: &str| names.push(format!("{p}_{what}"));
                match tile.spec.kind {
                    PlotType::Step | PlotType::Impulse | PlotType::Initial | PlotType::Lsim => {
                        let (_, function) = tile.spec.kind.outputs();
                        code.push_str(&format!("[{p}_y, {p}_x] = {function}({args});\n"));
                        out("y");
                        out("x");
                        if tile.spec.kind == PlotType::Impulse && siso {
                            // Peak of |y|; settling: the last time |y - y(end)|
                            // is 2% of its largest value, one sample on.
                            code.push_str(&format!(
                                "[app_v, app_i] = max(abs({p}_y(:, 1)));\n\
                                 {p}_pk = [{p}_x(app_i), {p}_y(app_i, 1)];\n\
                                 app_e = abs({p}_y(:, 1) - {p}_y(end, 1));\n\
                                 app_i = find(app_e >= 0.02*max(app_e), 1, 'last');\n\
                                 if isempty(app_i) || max(app_e) == 0\n\
                                 {p}_ts = NaN;\n\
                                 else\n\
                                 {p}_ts = {p}_x(min(app_i + 1, numel({p}_x)));\n\
                                 end\n"
                            ));
                            out("pk");
                            out("ts");
                        }
                    }
                    PlotType::Bode | PlotType::Nyquist | PlotType::Nichols => {
                        let (_, function) = tile.spec.kind.outputs();
                        code.push_str(&format!("[{p}_y, {p}_z, {p}_x] = {function}({args});\n"));
                        out("y");
                        out("z");
                        out("x");
                    }
                    PlotType::BodeMagnitude => {
                        code.push_str(&format!("[{p}_y, ~, {p}_x] = bode({args});\n"));
                        out("y");
                        out("x");
                        if siso {
                            code.push_str(&format!(
                                "[app_v, app_i] = max({p}_y(:, 1)); {p}_pk = [{p}_x(app_i), app_v];\n"
                            ));
                            out("pk");
                        }
                    }
                    PlotType::Sigma => {
                        code.push_str(&format!(
                            "[{p}_y, {p}_x] = sigma({args});\n\
                             [app_v, app_i] = max({p}_y(1, :)); {p}_pk = [{p}_x(app_i), app_v];\n"
                        ));
                        out("y");
                        out("x");
                        out("pk");
                    }
                    PlotType::PoleZero | PlotType::IoPoleZero => {
                        let (rows, cols) = if tile.spec.kind == PlotType::IoPoleZero {
                            (shown.shape.outputs, shown.shape.inputs)
                        } else {
                            (1, 1)
                        };
                        for i in 1..=rows {
                            for j in 1..=cols {
                                let system = if siso {
                                    format!("app_S{{{s}}}")
                                } else {
                                    format!(
                                        "ss(app_S{{{s}}}.A, app_S{{{s}}}.B(:, {j}), app_S{{{s}}}.C({i}, :), app_S{{{s}}}.D({i}, {j}), app_S{{{s}}}.Ts)"
                                    )
                                };
                                let c = format!("{p}_{i}_{j}");
                                code.push_str(&format!(
                                    "[app_p, app_z] = pzmap({system});\n\
                                     {c}_pr = real(app_p(:)); {c}_pi = imag(app_p(:));\n\
                                     {c}_zr = real(app_z(:)); {c}_zi = imag(app_z(:));\n"
                                ));
                                for what in ["pr", "pi", "zr", "zi"] {
                                    out(&format!("{i}_{j}_{what}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        (code, names)
    }
}

impl App for LinearSystemAnalyzer {
    fn kind(&self) -> AppKind {
        AppKind::LinearSystemAnalyzer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.plan(ws)?;
        Ok(self.script_text(&plan))
    }
    fn inputs(&self) -> Vec<String> {
        self.reads.clone()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        match self.plan(ws) {
            Ok(plan) => self.probe_text(&plan),
            Err(_) => (String::new(), Vec::new()),
        }
    }
    fn prepare(&mut self, ws: &Snapshot) {
        self.sync(ws);
        self.count = self.count.clamp(1, MAX_PLOTS);
        self.active = self.active.min(self.count - 1);
        // Workspace variables the script reads.
        let mut reads: Vec<String> = Vec::new();
        let mut add = |name: &str| {
            if ws.contains(name) && !reads.iter().any(|r| r == name) {
                reads.push(name.to_owned());
            }
        };
        let expression_names: Vec<String> = self
            .systems
            .iter()
            .filter(|s| s.expression.is_some())
            .map(|s| s.name.clone())
            .collect();
        for s in &self.systems {
            match &s.expression {
                None if s.shown => add(&s.name),
                None => {}
                Some(expr) => {
                    for name in identifiers(expr) {
                        if !expression_names.contains(&name) {
                            add(&name);
                        }
                    }
                }
            }
        }
        for spec in &self.plots[..self.count] {
            if spec.kind == PlotType::Lsim {
                add(&spec.u);
                add(&spec.t);
            }
        }
        self.reads = reads;
        self.numeric = ws
            .variables()
            .filter(|(_, a)| matches!(a, Array::Numeric(_)) && !a.is_complex() && a.len() >= 2)
            .map(|(name, a)| {
                let dims: Vec<String> = a.dims().iter().map(usize::to_string).collect();
                (name.to_owned(), dims.join("×"))
            })
            .collect();
        if !self
            .systems
            .iter()
            .any(|s| s.shown && s.name == self.export_system)
        {
            self.export_system = self
                .systems
                .iter()
                .find(|s| s.shown)
                .map(|s| s.name.clone())
                .unwrap_or_default();
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        self.current = Some(Analysis::from_outputs(outputs)?);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.query_shapes(ui.ctx(), ws);
        self.systems_ui(ui, ws);
        section(ui);
        self.options_ui(ui);
    }
    fn main_caption(&self) -> &'static str {
        "PLOTS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            ui.label("Layout");
            for n in 1..=MAX_PLOTS {
                let label = if n == 1 {
                    "1 plot".to_owned()
                } else {
                    format!("{n} plots")
                };
                let toggle = crate::tokens::toggle(ui, self.count == n, &label);
                if n == 1 {
                    crate::focus_areas::register(
                        ui,
                        &toggle,
                        crate::focus_areas::Area::App(AppKind::LinearSystemAnalyzer),
                    );
                }
                if toggle.clicked() {
                    self.count = n;
                    self.active = self.active.min(n - 1);
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let area = ui.available_rect_before_wrap();
        let area =
            egui::Rect::from_min_size(area.min, egui::vec2(area.width(), area.height().max(240.)));
        for k in 0..self.count {
            let rect = Cell::of_layout(self.count, k).rect(area, crate::tokens::S);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .id_salt(("lsa_tile", k))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| self.tile_ui(ui, k),
            );
        }
        ui.allocate_rect(area, egui::Sense::hover());
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let shown: Vec<&str> = self
            .systems
            .iter()
            .filter(|s| s.shown)
            .map(|s| s.name.as_str())
            .collect();
        vec![
            (
                "Systems",
                if shown.is_empty() {
                    "—".into()
                } else {
                    shown.join(", ")
                },
            ),
            (
                "Export reads",
                format!(
                    "plot {} ({})",
                    self.active_plot() + 1,
                    self.used_plots()[self.active_plot()].kind.label()
                ),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.tables_ui(ui);
        self.export_ui(ui, ws);
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        let Ok(plan) = self.plan(ws) else {
            return Vec::new();
        };
        let mut names: Vec<String> = plan.definitions.iter().map(|(n, _)| n.clone()).collect();
        names.extend(plan.channels.iter().map(|c| c.0.clone()));
        if let Some((outputs, _)) = &plan.export {
            names.extend(outputs.iter().cloned());
        }
        names
    }
    fn summary(&self) -> Option<String> {
        let a = self.current.as_ref()?;
        let names: Vec<&str> = a.systems.iter().map(|s| s.label.as_str()).collect();
        let kinds: Vec<&str> = a.tiles.iter().map(|t| t.kind.label()).collect();
        let unstable: Vec<&str> = a
            .systems
            .iter()
            .filter(|s| !s.stable)
            .map(|s| s.label.as_str())
            .collect();
        Some(format!(
            "{} · {} · {}",
            names.join(", "),
            kinds.join(", "),
            if unstable.is_empty() {
                "all stable".to_owned()
            } else {
                format!("unstable: {}", unstable.join(", "))
            }
        ))
    }
}

// ---------------------------------------------------------------------------
// Inputs

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}

fn colour(index: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[index % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}

/// A colour swatch; a click moves to the next colour.
fn swatch(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    crate::tokens::color_swatch(ui, 14., color, egui::Sense::click())
        .on_hover_text("Colour: click for the next one")
}

impl LinearSystemAnalyzer {
    fn systems_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        ui.label("Systems");
        if self.systems.is_empty() {
            crate::tokens::helper(
                ui,
                "No tf, zpk or ss models in the workspace: create one, for example G = tf(1, [1 0.6 1])",
            );
        }
        let shown = self.systems.iter().filter(|s| s.shown).count();
        let mut remove = None;
        let errors: Vec<Option<String>> = self
            .systems
            .iter()
            .map(|s| self.system_error(&s.name).map(str::to_owned))
            .collect();
        let classes: BTreeMap<String, &'static str> = ws
            .lti_models()
            .map(|(name, class)| (name.to_owned(), class))
            .collect();
        for (k, system) in self.systems.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = crate::tokens::XS;
                let can_tick = system.shown || shown < MAX_SHOWN;
                ui.add_enabled(can_tick, egui::Checkbox::without_text(&mut system.shown))
                    .on_disabled_hover_text(format!("At most {MAX_SHOWN} systems are shown"));
                if swatch(ui, colour(system.color)).clicked() {
                    system.color = (system.color + 1) % matrust::plot::COLORS.len();
                }
                let class = match &system.expression {
                    Some(_) => "expression",
                    None => classes.get(&system.name).copied().unwrap_or("model"),
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{} ({class})", system.name)).color(p.text),
                    )
                    .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if system.expression.is_some()
                        && ui
                            .small_button("Remove")
                            .on_hover_text("Remove this expression system")
                            .clicked()
                    {
                        remove = Some(k);
                    }
                    egui::ComboBox::from_id_salt(("lsa_style", k))
                        .width(84.)
                        .selected_text(system.style.label())
                        .show_ui(ui, |ui| {
                            for style in Style::ALL {
                                ui.selectable_value(&mut system.style, style, style.label());
                            }
                        });
                });
            });
            if let Some(expr) = &system.expression {
                crate::tokens::helper(ui, format!("= {expr}"));
            }
            if let Some(error) = &errors[k] {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(error)
                            .size(crate::tokens::CAPTION)
                            .color(p.error),
                    )
                    .wrap(),
                );
            }
        }
        if let Some(k) = remove {
            self.remove(k);
        }
        ui.add_space(crate::tokens::S);
        field(ui, "Add expression", |ui| {
            let mut add = false;
            ui.horizontal(|ui| {
                let width = ui.available_width() - 48. - crate::tokens::XS;
                let edit = ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.expression)
                            .char_limit(MAX_EXPRESSION_LEN)
                            .hint_text("feedback(G*C, 1)"),
                    )
                    .desired_width(width),
                );
                add |= edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                add |= ui
                    .add(egui::Button::new("Add").min_size(egui::vec2(48., crate::tokens::CONTROL)))
                    .clicked();
            });
            if add && !self.expression.trim().is_empty() {
                let _ = self.add_expression(ws);
            }
            match &self.expression_error {
                Some(error) => {
                    ui.label(
                        egui::RichText::new(error.as_str())
                            .size(crate::tokens::CAPTION)
                            .color(p.error),
                    );
                }
                None => {
                    crate::tokens::helper(ui, "Uses workspace models; the new system gets a name");
                }
            }
        });
    }
    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        let unit = if self.hz { "Hz" } else { "rad/s" };
        field(ui, "Frequency range", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, self.freq_auto, "Auto").clicked() {
                    self.freq_auto = true;
                }
                if crate::tokens::toggle(ui, !self.freq_auto, "From … to").clicked() {
                    self.freq_auto = false;
                }
            });
            if !self.freq_auto {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 48. - 2. * crate::tokens::XS) / 2.;
                    for text in [&mut self.wmin, &mut self.wmax] {
                        ui.add(
                            crate::tokens::input(egui::TextEdit::singleline(text).char_limit(24))
                                .desired_width(width),
                        );
                    }
                    ui.label(egui::RichText::new(unit).color(p.text_muted));
                });
            }
        });
        field(ui, "Time range", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, self.time_auto, "Auto").clicked() {
                    self.time_auto = true;
                }
                if crate::tokens::toggle(ui, !self.time_auto, "Final time").clicked() {
                    self.time_auto = false;
                }
            });
            if !self.time_auto {
                ui.horizontal(|ui| {
                    ui.add(
                        crate::tokens::input(
                            egui::TextEdit::singleline(&mut self.final_time).char_limit(24),
                        )
                        .desired_width(ui.available_width() - 48.),
                    );
                    ui.label(egui::RichText::new("s").color(p.text_muted));
                });
            }
        });
        field(ui, "Units", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, !self.hz, "rad/s").clicked() {
                    self.hz = false;
                }
                if crate::tokens::toggle(ui, self.hz, "Hz").clicked() {
                    self.hz = true;
                }
                ui.add_space(crate::tokens::S);
                if crate::tokens::toggle(ui, self.db, "dB").clicked() {
                    self.db = true;
                }
                if crate::tokens::toggle(ui, !self.db, "Absolute").clicked() {
                    self.db = false;
                }
            });
        });
        let (outputs, inputs) = self
            .known_shapes()
            .filter(|(name, _)| self.systems.iter().any(|s| s.shown && s.name == **name))
            .fold((1, 1), |(o, i), (_, s)| (o.max(s.outputs), i.max(s.inputs)));
        field(ui, "I/O pair", |ui| {
            if outputs == 1 && inputs == 1 && self.io.is_none() {
                crate::tokens::helper(ui, "The shown systems have one input and one output");
                return;
            }
            let label = |io: Option<(usize, usize)>| match io {
                None => "All pairs".to_owned(),
                Some((i, j)) => format!("Output {i}, input {j}"),
            };
            egui::ComboBox::from_id_salt("lsa_io")
                .width(ui.available_width())
                .selected_text(label(self.io))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.io, None, label(None));
                    for i in 1..=outputs {
                        for j in 1..=inputs {
                            ui.selectable_value(&mut self.io, Some((i, j)), label(Some((i, j))));
                        }
                    }
                });
            crate::tokens::helper(ui, "Applies to systems with several inputs or outputs");
        });
    }
}

// ---------------------------------------------------------------------------
// Output

/// "a ± bi" for complex pairs (the negative one left out), "a" for reals.
fn roots(values: &[[f64; 2]], most: usize) -> String {
    if values.is_empty() {
        return "none".into();
    }
    let mut items: Vec<String> = Vec::new();
    for [re, im] in values {
        if *im < 0. && values.iter().any(|[r, i]| r == re && *i == -im) {
            continue;
        }
        items.push(if *im == 0. {
            short(*re)
        } else {
            format!("{} ± {}i", short(*re), short(im.abs()))
        });
    }
    let total = items.len();
    items.truncate(most);
    let mut text = items.join(", ");
    if total > most {
        text.push_str(&format!(", … ({} in all)", values.len()));
    }
    text
}

impl LinearSystemAnalyzer {
    fn frequency_value(&self, w: f64) -> String {
        if self.hz {
            format!("{} Hz", short(w / std::f64::consts::TAU))
        } else {
            format!("{} rad/s", short(w))
        }
    }
    /// The table rows of one system.
    pub fn table(&self, s: &SystemResult) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("Poles", roots(&s.poles, 6)),
            (
                "Zeros",
                match &s.zeros {
                    Some(z) => roots(z, 6),
                    None => "— (several inputs or outputs)".into(),
                },
            ),
            (
                "DC gain",
                if s.dc.is_empty() {
                    "—".into()
                } else {
                    s.dc.iter()
                        .map(|v| short(*v))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            ),
        ];
        let mut damping: Vec<String> = Vec::new();
        for (wn, zeta) in s.wn.iter().zip(&s.zeta) {
            let item = format!("ζ {} at ωn {}", short(*zeta), self.frequency_value(*wn));
            if !damping.contains(&item) {
                damping.push(item);
            }
        }
        damping.truncate(4);
        rows.push((
            "Damping",
            if damping.is_empty() {
                "none".into()
            } else {
                damping.join("; ")
            },
        ));
        rows.push(("Stable", if s.stable { "yes" } else { "no" }.into()));
        if let Some(i) = &s.stepinfo {
            rows.push(("Rise time", format!("{} s", short(i.rise))));
            rows.push(("Settling time", format!("{} s", short(i.settling))));
            rows.push(("Overshoot", format!("{} %", short(i.overshoot))));
            rows.push((
                "Peak",
                format!("{} at {} s", short(i.peak), short(i.peak_time)),
            ));
        }
        if let Some(m) = &s.margins {
            let [gm, pm, wcg, wcp] = m.min;
            rows.push((
                "Gain margin",
                if gm.is_finite() && gm > 0. {
                    format!(
                        "{} dB at {}",
                        short(20. * gm.log10()),
                        self.frequency_value(wcg)
                    )
                } else {
                    short(gm)
                },
            ));
            rows.push((
                "Phase margin",
                if pm.is_finite() {
                    format!("{}° at {}", short(pm), self.frequency_value(wcp))
                } else {
                    short(pm)
                },
            ));
            let delay = m
                .delay
                .iter()
                .zip(&m.delay_w)
                .filter(|(d, _)| d.is_finite())
                .min_by(|a, b| a.0.total_cmp(b.0));
            rows.push((
                "Delay margin",
                match delay {
                    Some((d, w)) => format!(
                        "{} {} at {}",
                        short(*d),
                        if s.shape.ts > 0. { "samples" } else { "s" },
                        self.frequency_value(*w)
                    ),
                    None => "Inf".into(),
                },
            ));
        }
        rows
    }
    /// The per-system tables, in a scroll area of bounded height so the
    /// export names and buttons stay in view.
    fn tables_ui(&self, ui: &mut egui::Ui) {
        let Some(a) = &self.current else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("lsa_tables")
            .max_height(TABLES_HEIGHT)
            .auto_shrink([false, true])
            .show_themed(ui, |ui| self.tables(ui, a));
        ui.add_space(crate::tokens::M);
    }
    fn tables(&self, ui: &mut egui::Ui, a: &Analysis) {
        let p = crate::tokens::palette(ui.ctx());
        for (k, s) in a.systems.iter().enumerate() {
            if k > 0 {
                ui.add_space(crate::tokens::S);
            }
            ui.horizontal(|ui| {
                let color = self
                    .systems
                    .iter()
                    .find(|t| t.name == s.base)
                    .map_or(colour(k), |t| colour(t.color));
                crate::tokens::color_swatch(ui, 10., color, egui::Sense::hover());
                ui.label(egui::RichText::new(&s.label).strong().color(p.text));
            });
            egui::Grid::new(("lsa_table", k))
                .num_columns(2)
                .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                .show(ui, |ui| {
                    for (label, value) in self.table(s) {
                        ui.label(egui::RichText::new(label).color(p.text_muted));
                        ui.add(egui::Label::new(egui::RichText::new(value).color(p.text)).wrap());
                        ui.end_row();
                    }
                });
        }
    }
    fn export_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let active = self.active_plot();
        let kind = self.used_plots()[active].kind;
        ui.label(format!("Export: plot {} ({})", active + 1, kind.label()));
        let shown: Vec<String> = self
            .systems
            .iter()
            .filter(|s| s.shown)
            .map(|s| s.name.clone())
            .collect();
        if shown.len() > 1 {
            egui::ComboBox::from_id_salt("lsa_export_system")
                .width(ui.available_width())
                .selected_text(format!("Data of {}", self.export_system))
                .show_ui(ui, |ui| {
                    for name in &shown {
                        ui.selectable_value(&mut self.export_system, name.clone(), name);
                    }
                });
        }
        ui.add_space(crate::tokens::XS);
        if let Ok(plan) = self.plan(ws) {
            for (base, name) in self.default_export_names(ws, &plan) {
                let entry = self
                    .export_names
                    .entry(base.clone())
                    .or_insert_with(|| (name.clone(), false));
                if !entry.1 {
                    entry.0 = name;
                }
                let before = entry.0.clone();
                name_field(ui, &format!("Output {base}"), &mut entry.0, ws);
                entry.1 |= entry.0 != before;
            }
        }
        for k in 0..self.systems.len() {
            if self.systems[k].expression.is_none() {
                continue;
            }
            let system = &mut self.systems[k];
            let before = system.name.clone();
            let expr = system.expression.clone().unwrap_or_default();
            name_field(ui, &format!("Name of {expr}"), &mut system.name, ws);
            if system.name != before {
                system.name_edited = true;
                if self.export_system == before {
                    self.export_system = self.systems[k].name.clone();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main view

/// Display units of the app's plots.
#[derive(Clone, Copy)]
struct Units {
    hz: bool,
    db: bool,
}
impl Units {
    /// Plot x of a frequency: log10 of rad/s or Hz.
    fn x(self, w: f64) -> f64 {
        (if self.hz {
            w / std::f64::consts::TAU
        } else {
            w
        })
        .log10()
    }
    /// Plot y of a gain: dB, or log10 of the absolute value.
    fn y(self, gain: f64) -> f64 {
        if self.db {
            20. * gain.log10()
        } else {
            gain.log10()
        }
    }
    fn frequency_label(self) -> &'static str {
        if self.hz {
            "Frequency (Hz)"
        } else {
            "Frequency (rad/s)"
        }
    }
    fn gain_label(self) -> &'static str {
        if self.db {
            "Magnitude (dB)"
        } else {
            "Magnitude (abs)"
        }
    }
}

/// Linear interpolation of y(x) on increasing x.
fn interpolate(x: &[f64], y: &[f64], at: f64) -> Option<f64> {
    let k = x.iter().position(|v| *v >= at)?;
    if k == 0 {
        return (x[0] == at).then(|| y[0]);
    }
    let (x0, x1, y0, y1) = (x[k - 1], x[k], y[k - 1], y[k]);
    Some(if x1 == x0 {
        y1
    } else {
        y0 + (y1 - y0) * (at - x0) / (x1 - x0)
    })
}

fn finite(points: impl Iterator<Item = [f64; 2]>) -> Vec<[f64; 2]> {
    points
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}

/// A marker with its value: a point named for the hover and a label beside it.
struct Marker {
    at: [f64; 2],
    text: String,
    color: egui::Color32,
}

/// One panel's lines and markers.
#[derive(Default)]
struct Panel {
    title: String,
    lines: Vec<(String, Vec<[f64; 2]>, egui::Color32, egui_plot::LineStyle)>,
    markers: Vec<Marker>,
    /// Horizontal guides (steady state) and the Nyquist critical point.
    guides: Vec<(String, f64)>,
    critical: bool,
    /// Pole-zero map: poles and zeros per system, unit circle for discrete.
    roots: Vec<PoleZeroSet>,
    unit_circle: bool,
    /// Nichols grid.
    ngrid: bool,
}

/// Axes of a panel: labels and which coordinates are log10 values.
#[derive(Clone, Copy)]
struct Axes<'a> {
    x: &'a str,
    y: &'a str,
    log_x: bool,
    log_y: bool,
}

impl LinearSystemAnalyzer {
    fn tile_ui(&mut self, ui: &mut egui::Ui, k: usize) {
        let p = crate::tokens::palette(ui.ctx());
        let active = self.active_plot() == k;
        let rect = ui.max_rect();
        ui.painter().rect_stroke(
            rect,
            crate::tokens::CONTROL_RADIUS,
            egui::Stroke::new(1., if active { p.accent } else { p.border }),
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink(crate::tokens::S);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing =
                        egui::vec2(crate::tokens::XS, crate::tokens::XS);
                    if ui
                        .selectable_label(active, format!("Plot {}", k + 1))
                        .on_hover_text("Export reads the selected plot")
                        .clicked()
                    {
                        self.active = k;
                    }
                    let spec = &mut self.plots[k];
                    egui::ComboBox::from_id_salt(("lsa_type", k))
                        .width(140.)
                        .selected_text(spec.kind.label())
                        .show_ui(ui, |ui| {
                            for kind in PlotType::ALL {
                                ui.selectable_value(&mut spec.kind, kind, kind.label());
                            }
                        });
                    if !spec.kind.characteristics().is_empty() {
                        ui.menu_button("Characteristics", |ui| {
                            for c in spec.kind.characteristics() {
                                let mut on = spec.has(*c);
                                if ui.checkbox(&mut on, c.label()).changed() {
                                    spec.set(*c, on);
                                }
                            }
                        });
                    }
                    match spec.kind {
                        PlotType::Initial => {
                            ui.label("x0");
                            ui.add(
                                crate::tokens::input(
                                    egui::TextEdit::singleline(&mut spec.x0)
                                        .char_limit(400)
                                        .hint_text("1 0"),
                                )
                                .desired_width(96.),
                            );
                        }
                        PlotType::Nichols => {
                            ui.checkbox(&mut spec.ngrid, "Grid");
                        }
                        PlotType::Sigma => {
                            egui::ComboBox::from_id_salt(("lsa_sigma", k))
                                .width(96.)
                                .selected_text(SIGMA_TYPES[usize::from(spec.sigma_type.min(3))])
                                .show_ui(ui, |ui| {
                                    for (t, label) in SIGMA_TYPES.iter().enumerate() {
                                        ui.selectable_value(&mut spec.sigma_type, t as u8, *label);
                                    }
                                });
                        }
                        PlotType::Lsim => {
                            for (which, value) in [("u", &mut spec.u), ("t", &mut spec.t)] {
                                egui::ComboBox::from_id_salt(("lsa_lsim", k, which))
                                    .width(88.)
                                    .selected_text(if value.is_empty() {
                                        format!("{which}: choose")
                                    } else {
                                        format!("{which}: {value}")
                                    })
                                    .show_ui(ui, |ui| {
                                        for (name, size) in &self.numeric {
                                            ui.selectable_value(
                                                value,
                                                name.clone(),
                                                format!("{name} ({size})"),
                                            );
                                        }
                                    });
                            }
                        }
                        _ => {}
                    }
                });
                ui.add_space(crate::tokens::XS);
                self.plot_area(ui, k);
            },
        );
    }
    /// The plot's panels, from the last good run.
    fn plot_area(&self, ui: &mut egui::Ui, k: usize) {
        let Some(a) = &self.current else {
            crate::tokens::helper(ui, "No result yet: the status line says why.");
            return;
        };
        let Some(tile) = a.tiles.get(k) else {
            crate::tokens::helper(ui, "Waiting for this plot's first result.");
            return;
        };
        let spec = &self.plots[k];
        if tile.kind != spec.kind {
            crate::tokens::helper(
                ui,
                format!("Showing the last result ({}).", tile.kind.label()),
            );
        }
        if tile.curves.is_empty() {
            crate::tokens::helper(
                ui,
                "Initial condition plots need a state-space (ss) system: tick one in INPUTS.",
            );
            return;
        }
        let units = Units {
            hz: self.hz,
            db: self.db,
        };
        let marks = |c: Char| tile.kind == spec.kind && spec.has(c);
        let (rows, cols, panels) = self.panels(a, tile, units, &marks, spec.ngrid);
        let area = ui.available_rect_before_wrap();
        let area =
            egui::Rect::from_min_size(area.min, egui::vec2(area.width(), area.height().max(120.)));
        let single = rows * cols == 1;
        for (index, (axes, panel)) in panels.into_iter().enumerate() {
            let cell = Cell {
                rows,
                cols,
                top: index / cols,
                bottom: index / cols + 1,
                left: index % cols,
                right: index % cols + 1,
            };
            let rect = cell.rect(area, crate::tokens::XS);
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                let id = egui::Id::new(("lsa_panel", k, index));
                draw_panel(ui, id, rect.size(), axes, &panel, single);
            });
        }
        ui.allocate_rect(area, egui::Sense::hover());
    }
    /// The panels of a plot, row by row, with their axes.
    fn panels<'a>(
        &self,
        a: &Analysis,
        tile: &TileResult,
        units: Units,
        marks: &dyn Fn(Char) -> bool,
        ngrid: bool,
    ) -> (usize, usize, Vec<(Axes<'a>, Panel)>) {
        let first = a.systems[tile.curves[0].0].shape;
        let (p, m) = (first.outputs, first.inputs);
        let style_of = |base: &str, k: usize| {
            self.systems
                .iter()
                .find(|s| s.name == base)
                .map_or((colour(k), egui_plot::LineStyle::Solid), |s| {
                    (colour(s.color), s.style.line())
                })
        };
        let channel_title = |i: usize, j: usize, rows: usize, cols: usize| {
            if rows * cols > 1 {
                format!("Output {}, input {}", i + 1, j + 1)
            } else {
                String::new()
            }
        };
        let frequency = units.frequency_label();
        let gain = units.gain_label();
        match tile.kind {
            PlotType::Step | PlotType::Impulse | PlotType::Initial | PlotType::Lsim => {
                let (rows, cols) = if matches!(tile.kind, PlotType::Step | PlotType::Impulse) {
                    (p, m)
                } else {
                    (p, 1)
                };
                let mut panels = Vec::new();
                for i in 0..rows {
                    for j in 0..cols {
                        let mut panel = Panel {
                            title: channel_title(i, j, rows, cols),
                            ..Panel::default()
                        };
                        for (s, curves) in &tile.curves {
                            let Curves::Time {
                                t,
                                y,
                                traces,
                                peak,
                                settling,
                            } = curves
                            else {
                                continue;
                            };
                            let system = &a.systems[*s];
                            let trace = i + rows * j;
                            if trace >= *traces {
                                continue;
                            }
                            let n = t.len();
                            let ys = &y[trace * n..(trace + 1) * n];
                            let (color, style) = style_of(&system.base, *s);
                            panel.lines.push((
                                system.label.clone(),
                                finite(t.iter().zip(ys).map(|(t, y)| [*t, *y])),
                                color,
                                style,
                            ));
                            if !system.shape.siso() {
                                continue;
                            }
                            let at = |time: f64| interpolate(t, ys, time).map(|v| [time, v]);
                            if tile.kind == PlotType::Step
                                && let Some(info) = &system.stepinfo
                            {
                                let dc = system.dc.first().copied().unwrap_or(f64::NAN);
                                if marks(Char::Peak)
                                    && let Some(at) = at(info.peak_time)
                                {
                                    panel.markers.push(Marker {
                                        at,
                                        text: format!(
                                            "{}: peak {} at {} s ({} % overshoot)",
                                            system.label,
                                            short(info.peak),
                                            short(info.peak_time),
                                            short(info.overshoot)
                                        ),
                                        color,
                                    });
                                }
                                if marks(Char::Settling)
                                    && let Some(at) = at(info.settling)
                                {
                                    panel.markers.push(Marker {
                                        at,
                                        text: format!(
                                            "{}: settles at {} s",
                                            system.label,
                                            short(info.settling)
                                        ),
                                        color,
                                    });
                                }
                                if marks(Char::Rise)
                                    && dc.is_finite()
                                    && let Some(k) =
                                        ys.iter().position(|v| dc.signum() * v >= 0.9 * dc.abs())
                                {
                                    panel.markers.push(Marker {
                                        at: [t[k], ys[k]],
                                        text: format!(
                                            "{}: rise time {} s",
                                            system.label,
                                            short(info.rise)
                                        ),
                                        color,
                                    });
                                }
                                if marks(Char::Steady) && dc.is_finite() {
                                    panel.guides.push((
                                        format!("{}: steady state {}", system.label, short(dc)),
                                        dc,
                                    ));
                                }
                            }
                            if tile.kind == PlotType::Impulse {
                                if marks(Char::Peak)
                                    && let Some(at) = peak
                                {
                                    panel.markers.push(Marker {
                                        at: *at,
                                        text: format!(
                                            "{}: peak {} at {} s",
                                            system.label,
                                            short(at[1]),
                                            short(at[0])
                                        ),
                                        color,
                                    });
                                }
                                if marks(Char::Settling)
                                    && let Some(ts) = settling
                                    && let Some(at) = at(*ts)
                                {
                                    panel.markers.push(Marker {
                                        at,
                                        text: format!(
                                            "{}: settles at {} s",
                                            system.label,
                                            short(*ts)
                                        ),
                                        color,
                                    });
                                }
                            }
                        }
                        let axes = Axes {
                            x: "Time (s)",
                            y: "Amplitude",
                            log_x: false,
                            log_y: false,
                        };
                        panels.push((axes, panel));
                    }
                }
                (rows, cols, panels)
            }
            PlotType::Bode | PlotType::BodeMagnitude => {
                let bode = tile.kind == PlotType::Bode;
                let rows = if bode { 2 * p } else { p };
                let mut panels = Vec::new();
                for row in 0..rows {
                    let (i, phase) = if bode {
                        (row / 2, row % 2 == 1)
                    } else {
                        (row, false)
                    };
                    for j in 0..m {
                        let mut panel = Panel {
                            title: if phase {
                                String::new()
                            } else {
                                channel_title(i, j, p, m)
                            },
                            ..Panel::default()
                        };
                        for (s, curves) in &tile.curves {
                            let Curves::Frequency {
                                w,
                                a: mag,
                                b: ph,
                                traces,
                                peak,
                            } = curves
                            else {
                                continue;
                            };
                            let system = &a.systems[*s];
                            let trace = i + p * j;
                            if trace >= *traces {
                                continue;
                            }
                            let n = w.len();
                            let (color, style) = style_of(&system.base, *s);
                            let range = trace * n..(trace + 1) * n;
                            let x: Vec<f64> = w.iter().map(|v| units.x(*v)).collect();
                            let values: Vec<f64> = if phase {
                                ph[range].to_vec()
                            } else {
                                mag[range].iter().map(|v| units.y(*v)).collect()
                            };
                            panel.lines.push((
                                system.label.clone(),
                                finite(x.iter().zip(&values).map(|(x, y)| [*x, *y])),
                                color,
                                style,
                            ));
                            if !system.shape.siso() {
                                continue;
                            }
                            if !bode {
                                if marks(Char::PeakGain)
                                    && let Some([wp, g]) = peak
                                {
                                    panel.markers.push(Marker {
                                        at: [units.x(*wp), units.y(*g)],
                                        text: format!(
                                            "{}: peak gain {} ({} dB) at {}",
                                            system.label,
                                            short(*g),
                                            short(20. * g.log10()),
                                            self.frequency_value(*wp)
                                        ),
                                        color,
                                    });
                                }
                                continue;
                            }
                            let Some(margins) = &system.margins else {
                                continue;
                            };
                            for (gm, wg, pm, wp) in margin_sets(margins, marks) {
                                if !phase && let (Some(gm), Some(wg)) = (gm, wg) {
                                    panel.markers.push(Marker {
                                        at: [units.x(wg), units.y(1. / gm)],
                                        text: format!(
                                            "{}: gain margin {} dB at {}",
                                            system.label,
                                            short(20. * gm.log10()),
                                            self.frequency_value(wg)
                                        ),
                                        color,
                                    });
                                }
                                if phase
                                    && let (Some(pm), Some(wp)) = (pm, wp)
                                    && let Some(y) =
                                        interpolate(w, &ph[trace * n..(trace + 1) * n], wp)
                                {
                                    panel.markers.push(Marker {
                                        at: [units.x(wp), y],
                                        text: format!(
                                            "{}: phase margin {}° at {}",
                                            system.label,
                                            short(pm),
                                            self.frequency_value(wp)
                                        ),
                                        color,
                                    });
                                }
                            }
                        }
                        let axes = Axes {
                            x: frequency,
                            y: if phase { "Phase (deg)" } else { gain },
                            log_x: true,
                            log_y: !phase && !units.db,
                        };
                        panels.push((axes, panel));
                    }
                }
                (rows, m, panels)
            }
            PlotType::Nyquist | PlotType::Nichols => {
                let nyquist = tile.kind == PlotType::Nyquist;
                let mut panels = Vec::new();
                for i in 0..p {
                    for j in 0..m {
                        let mut panel = Panel {
                            title: channel_title(i, j, p, m),
                            critical: nyquist,
                            ngrid: !nyquist && ngrid && units.db,
                            ..Panel::default()
                        };
                        for (s, curves) in &tile.curves {
                            let Curves::Frequency {
                                w,
                                a: first,
                                b: second,
                                traces,
                                ..
                            } = curves
                            else {
                                continue;
                            };
                            let system = &a.systems[*s];
                            let trace = i + p * j;
                            if trace >= *traces {
                                continue;
                            }
                            let n = w.len();
                            let (color, style) = style_of(&system.base, *s);
                            let range = trace * n..(trace + 1) * n;
                            let (one, two) = (&first[range.clone()], &second[range]);
                            if nyquist {
                                panel.lines.push((
                                    system.label.clone(),
                                    finite(one.iter().zip(two).map(|(r, i)| [*r, *i])),
                                    color,
                                    style,
                                ));
                                panel.lines.push((
                                    format!("{} (negative frequencies)", system.label),
                                    finite(one.iter().zip(two).map(|(r, i)| [*r, -*i])),
                                    color,
                                    egui_plot::LineStyle::dashed_dense(),
                                ));
                            } else {
                                panel.lines.push((
                                    system.label.clone(),
                                    finite(two.iter().zip(one).map(|(ph, g)| [*ph, units.y(*g)])),
                                    color,
                                    style,
                                ));
                            }
                            if !system.shape.siso() {
                                continue;
                            }
                            let Some(margins) = &system.margins else {
                                continue;
                            };
                            for (gm, wg, pm, wp) in margin_sets(margins, marks) {
                                if let (Some(gm), Some(wg)) = (gm, wg) {
                                    let at = if nyquist {
                                        Some([-1. / gm, 0.])
                                    } else {
                                        interpolate(w, two, wg).map(|ph| [ph, units.y(1. / gm)])
                                    };
                                    if let Some(at) = at {
                                        panel.markers.push(Marker {
                                            at,
                                            text: format!(
                                                "{}: gain margin {} dB at {}",
                                                system.label,
                                                short(20. * gm.log10()),
                                                self.frequency_value(wg)
                                            ),
                                            color,
                                        });
                                    }
                                }
                                if let (Some(pm), Some(wp)) = (pm, wp) {
                                    let at = if nyquist {
                                        let angle = (pm - 180.).to_radians();
                                        Some([angle.cos(), angle.sin()])
                                    } else {
                                        interpolate(w, two, wp).map(|ph| [ph, units.y(1.)])
                                    };
                                    if let Some(at) = at {
                                        panel.markers.push(Marker {
                                            at,
                                            text: format!(
                                                "{}: phase margin {}° at {}",
                                                system.label,
                                                short(pm),
                                                self.frequency_value(wp)
                                            ),
                                            color,
                                        });
                                    }
                                }
                            }
                        }
                        let axes = if nyquist {
                            Axes {
                                x: "Real",
                                y: "Imaginary",
                                log_x: false,
                                log_y: false,
                            }
                        } else {
                            Axes {
                                x: "Open-loop phase (deg)",
                                y: if units.db {
                                    "Open-loop gain (dB)"
                                } else {
                                    "Open-loop gain (abs)"
                                },
                                log_x: false,
                                log_y: !units.db,
                            }
                        };
                        panels.push((axes, panel));
                    }
                }
                (p, m, panels)
            }
            PlotType::Sigma => {
                let mut panel = Panel::default();
                for (s, curves) in &tile.curves {
                    let Curves::Sigma { w, sv, count, peak } = curves else {
                        continue;
                    };
                    let system = &a.systems[*s];
                    let (color, style) = style_of(&system.base, *s);
                    for r in 0..*count {
                        panel.lines.push((
                            if *count > 1 {
                                format!("{} σ{}", system.label, r + 1)
                            } else {
                                system.label.clone()
                            },
                            finite(
                                w.iter()
                                    .enumerate()
                                    .map(|(f, w)| [units.x(*w), units.y(sv[r + f * count])]),
                            ),
                            color,
                            style,
                        ));
                    }
                    if marks(Char::PeakGain)
                        && let Some([wp, g]) = peak
                    {
                        panel.markers.push(Marker {
                            at: [units.x(*wp), units.y(*g)],
                            text: format!(
                                "{}: peak gain {} ({} dB) at {}",
                                system.label,
                                short(*g),
                                short(20. * g.log10()),
                                self.frequency_value(*wp)
                            ),
                            color,
                        });
                    }
                }
                let axes = Axes {
                    x: frequency,
                    y: if units.db {
                        "Singular values (dB)"
                    } else {
                        "Singular values (abs)"
                    },
                    log_x: true,
                    log_y: !units.db,
                };
                (1, 1, vec![(axes, panel)])
            }
            PlotType::PoleZero | PlotType::IoPoleZero => {
                let (rows, cols) = if tile.kind == PlotType::IoPoleZero {
                    (p, m)
                } else {
                    (1, 1)
                };
                let mut panels = Vec::new();
                for c in 0..rows * cols {
                    let mut panel = Panel {
                        title: channel_title(c / cols, c % cols, rows, cols),
                        ..Panel::default()
                    };
                    for (s, curves) in &tile.curves {
                        let Curves::PoleZero { channels } = curves else {
                            continue;
                        };
                        let system = &a.systems[*s];
                        let Some([poles, zeros]) = channels.get(c) else {
                            continue;
                        };
                        let (color, _) = style_of(&system.base, *s);
                        panel.unit_circle |= system.shape.ts > 0.;
                        panel.roots.push((
                            system.label.clone(),
                            poles.clone(),
                            zeros.clone(),
                            color,
                        ));
                    }
                    let axes = Axes {
                        x: "Real",
                        y: "Imaginary",
                        log_x: false,
                        log_y: false,
                    };
                    panels.push((axes, panel));
                }
                (rows, cols, panels)
            }
        }
    }
}

/// (name, poles, zeros, colour) for the pole-zero view.
type PoleZeroSet = (String, Vec<[f64; 2]>, Vec<[f64; 2]>, egui::Color32);
/// (gain margin, its frequency, phase margin, its frequency).
type MarginSet = (Option<f64>, Option<f64>, Option<f64>, Option<f64>);

/// The margins a plot marks: every crossing (`allmargin`) or the minimum
/// ones (`margin`), as (gain margin, its frequency, phase margin, its
/// frequency).
fn margin_sets(m: &Margins, marks: &dyn Fn(Char) -> bool) -> Vec<MarginSet> {
    let ok = |v: f64| v.is_finite().then_some(v);
    let mut out = Vec::new();
    if marks(Char::AllMargins) {
        for (g, w) in m.gain.iter().zip(&m.gain_w) {
            out.push((ok(*g).filter(|g| *g > 0.), ok(*w), None, None));
        }
        for (p, w) in m.phase.iter().zip(&m.phase_w) {
            out.push((None, None, ok(*p), ok(*w)));
        }
    } else if marks(Char::MinMargins) {
        let [gm, pm, wg, wp] = m.min;
        out.push((ok(gm).filter(|g| *g > 0.), ok(wg), ok(pm), ok(wp)));
    }
    out
}

/// Nichols grid: closed-loop gain contours (dB) and phase contours (deg) of
/// T = G / (1 + G), traced through G = T / (1 - T) over one turn.
fn nichols_grid() -> Vec<(String, Vec<[f64; 2]>)> {
    use std::f64::consts::PI;
    let point = |t: (f64, f64)| {
        // G = T / (1 - T) with T = (re, im).
        let (re, im) = t;
        let (dr, di) = (1. - re, -im);
        let d = dr * dr + di * di;
        let (gr, gi) = ((re * dr + im * di) / d, (im * dr - re * di) / d);
        let mut phase = gi.atan2(gr).to_degrees();
        if phase > 0. {
            phase -= 360.;
        }
        [phase, 10. * (gr * gr + gi * gi).log10()]
    };
    let mut out = Vec::new();
    for db in [6., 3., 1., 0.5, 0.25, -1., -3., -6., -12., -20.] {
        let mag = 10f64.powf(db / 20.);
        let points: Vec<[f64; 2]> = (1..360)
            .map(|k| {
                let a = -(k as f64).to_radians();
                point((mag * a.cos(), mag * a.sin()))
            })
            .filter(|p| p[1].is_finite() && p[1].abs() < 60.)
            .collect();
        out.push((format!("{db} dB"), points));
    }
    for deg in [-1., -5., -10., -20., -30., -50., -90., -120., -150.] {
        let a = f64::to_radians(deg);
        let points: Vec<[f64; 2]> = (1..400)
            .map(|k| {
                let mag = 10f64.powf(-3. + 6. * k as f64 / 400.);
                point((mag * a.cos(), mag * a.sin()))
            })
            .filter(|p| {
                p[1].is_finite() && p[1].abs() < 60. && p[0] > -2. * 180. + 1e-9 && p[0] < PI
            })
            .collect();
        out.push((format!("{deg}°"), points));
    }
    out
}

fn draw_panel(
    ui: &mut egui::Ui,
    id: egui::Id,
    size: egui::Vec2,
    axes: Axes<'_>,
    panel: &Panel,
    labels: bool,
) {
    let p = crate::tokens::palette(ui.ctx());
    let mut height = size.y;
    if !panel.title.is_empty() {
        ui.label(
            egui::RichText::new(&panel.title)
                .size(crate::tokens::CAPTION)
                .color(p.text_muted),
        );
        height -= crate::tokens::ROW;
    }
    let (log_x, log_y) = (axes.log_x, axes.log_y);
    let shown = |v: f64, log: bool| short(if log { 10_f64.powf(v) } else { v });
    let mut chart = egui_plot::Plot::new(id)
        .width(size.x)
        .height(height.max(60.))
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .show_background(false)
        .x_axis_formatter(move |mark, _| shown(mark.value, log_x))
        .y_axis_formatter(move |mark, _| shown(mark.value, log_y))
        .label_formatter(move |position| {
            let (name, point) = match position {
                egui_plot::HoverPosition::NearDataPoint {
                    plot_name,
                    position,
                    ..
                } => (*plot_name, *position),
                egui_plot::HoverPosition::Elsewhere { position } => ("", *position),
            };
            let values = format!(
                "x = {}\ny = {}",
                shown(point.x, log_x),
                shown(point.y, log_y)
            );
            Some(if name.is_empty() {
                values
            } else {
                format!("{name}\n{values}")
            })
        });
    if labels {
        chart = chart
            .x_axis_label(axes.x.to_owned())
            .y_axis_label(axes.y.to_owned());
    }
    chart.show(ui, |plot_ui| {
        if panel.ngrid {
            for (name, points) in nichols_grid() {
                plot_ui.line(
                    egui_plot::Line::new(format!("Grid {name}"), points)
                        .color(p.border)
                        .width(1.),
                );
            }
        }
        for (name, y) in &panel.guides {
            plot_ui.hline(
                egui_plot::HLine::new(name.clone(), *y)
                    .color(p.text_muted)
                    .style(egui_plot::LineStyle::dashed_dense())
                    .width(1.),
            );
        }
        if panel.critical {
            plot_ui.points(
                egui_plot::Points::new("Critical point -1", vec![[-1., 0.]])
                    .shape(egui_plot::MarkerShape::Plus)
                    .radius(5.)
                    .color(p.error),
            );
        }
        for (name, points, color, style) in &panel.lines {
            plot_ui.line(
                egui_plot::Line::new(name.clone(), points.clone())
                    .color(*color)
                    .style(*style)
                    .width(1.5),
            );
        }
        if !panel.roots.is_empty() {
            let extent = panel
                .roots
                .iter()
                .flat_map(|(_, poles, zeros, _)| poles.iter().chain(zeros))
                .flat_map(|[r, i]| [r.abs(), i.abs()])
                .filter(|v| v.is_finite())
                .fold(if panel.unit_circle { 1. } else { 0. }, f64::max);
            let extent = if extent > 0. { 1.2 * extent } else { 1. };
            for (name, line) in [
                ("Real axis", vec![[-extent, 0.], [extent, 0.]]),
                ("Imaginary axis", vec![[0., -extent], [0., extent]]),
            ] {
                plot_ui.line(
                    egui_plot::Line::new(name, line)
                        .color(p.border)
                        .style(egui_plot::LineStyle::dashed_dense())
                        .width(1.),
                );
            }
            if panel.unit_circle {
                let circle: Vec<[f64; 2]> = (0..=180)
                    .map(|k| {
                        let a = std::f64::consts::TAU * k as f64 / 180.;
                        [a.cos(), a.sin()]
                    })
                    .collect();
                plot_ui.line(
                    egui_plot::Line::new("Unit circle", circle)
                        .color(p.border)
                        .style(egui_plot::LineStyle::dotted_dense())
                        .width(1.),
                );
            }
            for (name, poles, zeros, color) in &panel.roots {
                plot_ui.points(
                    egui_plot::Points::new(format!("{name} poles"), poles.clone())
                        .shape(egui_plot::MarkerShape::Cross)
                        .radius(5.)
                        .color(*color),
                );
                plot_ui.points(
                    egui_plot::Points::new(format!("{name} zeros"), zeros.clone())
                        .shape(egui_plot::MarkerShape::Circle)
                        .filled(false)
                        .radius(5.)
                        .color(*color),
                );
            }
        }
        for marker in &panel.markers {
            plot_ui.points(
                egui_plot::Points::new(marker.text.clone(), vec![marker.at])
                    .shape(egui_plot::MarkerShape::Circle)
                    .filled(true)
                    .radius(4.)
                    .color(marker.color),
            );
            plot_ui.text(
                egui_plot::Text::new(
                    format!("{} label", marker.text),
                    egui_plot::PlotPoint::new(marker.at[0], marker.at[1]),
                    format!("  {}", marker.text.split(": ").last().unwrap_or("")),
                )
                .anchor(egui::Align2::LEFT_BOTTOM)
                .color(p.text),
            );
        }
    });
}
