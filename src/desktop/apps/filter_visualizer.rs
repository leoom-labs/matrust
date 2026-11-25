//! Filter Visualization: compares digital filters taken from the workspace.
//! One script holds the analysis calls (`freqz`, `phasez`, `grpdelay`,
//! `impz`, `stepz`, `tf2zp`, `sos2zp`) and the plot commands of the current
//! view; the app runs it privately, Export runs it in the workspace and
//! Generate script saves it. The private probe only derives what the view
//! draws and the Output column's flags from the script's variables and the
//! filters' coefficients. Rust validates the inputs and draws.
use super::{App, AppKind, Outputs, Snapshot, field, number, short, unused_name};
use crate::response_plot::{self, Roots, Series};
use eframe::egui;
use matrust::Array;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

pub const MAX_FILTERS: usize = 8;
pub const MAX_POINTS: usize = 65536;
pub const DEFAULT_POINTS: usize = 8192;
/// Longest impulse or step response asked for.
pub const MAX_SAMPLES: usize = 65536;
/// Longest coefficient vector and most sections of one filter.
pub const MAX_COEFFICIENTS: usize = 8192;
pub const MAX_SECTIONS: usize = 1024;
const AXIS_NORMALIZED: &str = "Normalized frequency (Nyquist = 1)";
const AXIS_HZ: &str = "Frequency (Hz)";

/// How a filter is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Numerator and denominator vectors.
    Tf,
    /// An L-by-6 matrix of second-order sections and a gain.
    Sos,
    /// A numerator vector alone.
    Fir,
    /// A digitalFilter object; its coefficients are read in the workspace.
    Object,
}
impl Form {
    pub const ALL: [Self; 4] = [Self::Tf, Self::Sos, Self::Fir, Self::Object];
    pub fn label(self) -> &'static str {
        match self {
            Self::Tf => "Coefficients b, a",
            Self::Sos => "Sections sos, gain",
            Self::Fir => "FIR coefficients b",
            Self::Object => "digitalFilter object",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Magnitude,
    Phase,
    MagnitudePhase,
    GroupDelay,
    PhaseDelay,
    Impulse,
    Step,
    PoleZero,
}
impl View {
    pub const ALL: [Self; 8] = [
        Self::Magnitude,
        Self::Phase,
        Self::MagnitudePhase,
        Self::GroupDelay,
        Self::PhaseDelay,
        Self::Impulse,
        Self::Step,
        Self::PoleZero,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Magnitude => "Magnitude",
            Self::Phase => "Phase",
            Self::MagnitudePhase => "Mag + phase",
            Self::GroupDelay => "Group delay",
            Self::PhaseDelay => "Phase delay",
            Self::Impulse => "Impulse",
            Self::Step => "Step",
            Self::PoleZero => "Pole-zero",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Magnitude => "Magnitude response",
            Self::Phase => "Phase response",
            Self::MagnitudePhase => "Magnitude and phase response",
            Self::GroupDelay => "Group delay",
            Self::PhaseDelay => "Phase delay",
            Self::Impulse => "Impulse response",
            Self::Step => "Step response",
            Self::PoleZero => "Pole-zero plot",
        }
    }
    /// Drawn over the frequency axis.
    pub fn frequency(self) -> bool {
        matches!(
            self,
            Self::Magnitude
                | Self::Phase
                | Self::MagnitudePhase
                | Self::GroupDelay
                | Self::PhaseDelay
        )
    }
    pub fn time(self) -> bool {
        matches!(self, Self::Impulse | Self::Step)
    }
    fn has_magnitude(self) -> bool {
        matches!(self, Self::Magnitude | Self::MagnitudePhase)
    }
    fn has_phase(self) -> bool {
        matches!(self, Self::Phase | Self::MagnitudePhase)
    }
    /// The arrays the view computes per filter, in export order.
    pub fn parts(self) -> &'static [Part] {
        match self {
            Self::Magnitude => &[Part::Response, Part::Frequency],
            Self::Phase => &[Part::Phase, Part::Frequency],
            Self::MagnitudePhase => &[Part::Response, Part::Phase, Part::Frequency],
            Self::GroupDelay => &[Part::GroupDelay, Part::Frequency],
            Self::PhaseDelay => &[Part::PhaseDelay, Part::Frequency],
            Self::Impulse => &[Part::Impulse, Part::Time],
            Self::Step => &[Part::Step, Part::Time],
            Self::PoleZero => &[Part::Zeros, Part::Poles, Part::Gain],
        }
    }
}

/// One exported array of a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Response,
    Frequency,
    Phase,
    GroupDelay,
    PhaseDelay,
    Impulse,
    Step,
    Time,
    Zeros,
    Poles,
    Gain,
}
impl Part {
    pub const ALL: [Self; 11] = [
        Self::Response,
        Self::Frequency,
        Self::Phase,
        Self::GroupDelay,
        Self::PhaseDelay,
        Self::Impulse,
        Self::Step,
        Self::Time,
        Self::Zeros,
        Self::Poles,
        Self::Gain,
    ];
    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Response => "Complex response",
            Self::Frequency => "Frequency vector",
            Self::Phase => "Phase",
            Self::GroupDelay => "Group delay",
            Self::PhaseDelay => "Phase delay",
            Self::Impulse => "Impulse response",
            Self::Step => "Step response",
            Self::Time => "Time vector",
            Self::Zeros => "Zeros",
            Self::Poles => "Poles",
            Self::Gain => "Gain",
        }
    }
    fn default_name(self) -> &'static str {
        match self {
            Self::Response | Self::Impulse => "h",
            Self::Frequency => "w",
            Self::Phase => "phi",
            Self::GroupDelay => "gd",
            Self::PhaseDelay => "pd",
            Self::Step => "s",
            Self::Time => "t",
            Self::Zeros => "z",
            Self::Poles => "p",
            Self::Gain => "k",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Filter {
    pub id: u32,
    /// Legend label and suffix of the exported names.
    pub name: String,
    pub shown: bool,
    pub color: usize,
    pub form: Form,
    /// Workspace variables: numerator, denominator, sections, object.
    pub b: String,
    pub a: String,
    pub sos: String,
    pub object: String,
    /// Sections gain: blank (1), a number or a workspace scalar.
    pub gain: String,
    /// Sample rate in Hz; blank is normalised frequency.
    pub fs: String,
    /// Names a digitalFilter's coefficients were asked to be copied to
    /// (FIR numerator, sections); the copy that appears decides the form.
    pending: Option<(String, String)>,
}
impl Filter {
    fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            shown: true,
            color: id as usize - 1,
            form: Form::Tf,
            b: String::new(),
            a: String::new(),
            sos: String::new(),
            object: String::new(),
            gain: String::new(),
            fs: String::new(),
            pending: None,
        }
    }
    /// The sample rate, None when normalised.
    fn sample_rate(&self) -> Result<Option<f64>, String> {
        if self.fs.trim().is_empty() {
            return Ok(None);
        }
        let fs = scalar(&self.fs, "Sample rate")?;
        if fs <= 0. || fs > 1e12 {
            return Err("Sample rate must be positive (at most 1e12 Hz) or blank".into());
        }
        Ok(Some(fs))
    }
    /// The gain as script text, and the workspace scalar it names.
    fn gain_expression(&self, ws: &Snapshot) -> Result<Option<(String, Option<String>)>, String> {
        let text = self.gain.trim();
        if text.is_empty() {
            return Ok(None);
        }
        if text.len() > 64 {
            return Err("Gain: at most 64 characters".into());
        }
        super::single_expression(text).map_err(|e| format!("Gain: {e}"))?;
        if let Ok(v) = text.parse::<f64>() {
            if !v.is_finite() {
                return Err("Gain must be finite".into());
            }
            return Ok((v != 1.).then(|| (number(v), None)));
        }
        if !matrust::files::valid_name(text) {
            return Err("Gain: enter a number or a workspace scalar".into());
        }
        variable(text, "Gain")?;
        match ws.get(text) {
            Some(a @ Array::Numeric(m)) if a.len() == 1 && !a.is_complex() => {
                if !m.data()[0].is_finite() {
                    return Err(format!("Gain {text} must be finite"));
                }
                Ok(Some((text.to_owned(), Some(text.to_owned()))))
            }
            Some(_) => Err(format!("Gain {text} must be a real scalar")),
            None => Err(format!("Gain {text} is not a workspace variable")),
        }
    }
    /// The filter as the analysis calls take it.
    fn source(&self, ws: &Snapshot) -> Result<Source, String> {
        let vector = |name: &str, what: &str| -> Result<usize, String> {
            variable(name, what)?;
            let array = ws
                .get(name)
                .ok_or_else(|| format!("{what} {name} is not a workspace variable"))?;
            coefficient_vector(array).ok_or_else(|| {
                format!(
                    "{what} {name} must be a nonempty real vector of at most {MAX_COEFFICIENTS} finite values"
                )
            })
        };
        match self.form {
            Form::Tf => {
                let nb = vector(&self.b, "Numerator b")?;
                let na = vector(&self.a, "Denominator a")?;
                if first_value(ws.get(&self.a)) == Some(0.) {
                    return Err(format!(
                        "Denominator {} must start with a nonzero coefficient",
                        self.a
                    ));
                }
                Ok(Source {
                    system: System::Tf(self.b.clone(), self.a.clone()),
                    gain: None,
                    inputs: vec![self.b.clone(), self.a.clone()],
                    describe: format!(
                        "b = {}, a = {} ({nb} and {na} coefficients)",
                        self.b, self.a
                    ),
                })
            }
            Form::Fir => {
                let nb = vector(&self.b, "Numerator b")?;
                Ok(Source {
                    system: System::Fir(self.b.clone()),
                    gain: None,
                    inputs: vec![self.b.clone()],
                    describe: format!("FIR b = {} ({nb} coefficients)", self.b),
                })
            }
            Form::Sos => {
                variable(&self.sos, "Sections sos")?;
                let array = ws
                    .get(&self.sos)
                    .ok_or_else(|| format!("Sections {} is not a workspace variable", self.sos))?;
                let rows = section_matrix(array).ok_or_else(|| {
                    format!(
                        "Sections {} must be a real L-by-6 matrix (L up to {MAX_SECTIONS}) with nonzero a0 in column 4",
                        self.sos
                    )
                })?;
                let gain = self.gain_expression(ws)?;
                let mut inputs = vec![self.sos.clone()];
                inputs.extend(gain.iter().filter_map(|(_, name)| name.clone()));
                Ok(Source {
                    system: if rows == 1 {
                        System::Row(self.sos.clone())
                    } else {
                        System::Sos(self.sos.clone())
                    },
                    describe: format!(
                        "sos = {} ({rows} section{}){}",
                        self.sos,
                        if rows == 1 { "" } else { "s" },
                        match &gain {
                            Some((g, _)) => format!(", gain {g}"),
                            None => String::new(),
                        }
                    ),
                    gain: gain.map(|(g, _)| g),
                    inputs,
                })
            }
            Form::Object => {
                variable(&self.object, "digitalFilter")?;
                Err(format!(
                    "read the coefficients of {} first (Read coefficients in INPUTS)",
                    self.object
                ))
            }
        }
    }
    /// A digitalFilter cannot leave the workspace engine, so its
    /// coefficients are copied there to plain variables: this is the script
    /// and the two names it may create (FIR numerator, sections).
    pub fn read_object(&mut self, ws: &Snapshot) -> Result<String, String> {
        variable(&self.object, "digitalFilter")?;
        if !ws.contains(&self.object) {
            return Err(format!("{} is not a workspace variable", self.object));
        }
        let d = &self.object;
        let fresh = |suffix: &str| {
            let base: String = format!("{d}_{suffix}").chars().take(60).collect();
            unused_name(&base, |n| ws.contains(n))
        };
        let (b, sos) = (fresh("b"), fresh("sos"));
        let script = format!(
            "if ~isa({d}, 'digitalFilter'), error('Filter Visualization: {d} is not a digitalFilter object'); end\n\
             if strcmp({d}.ImpulseResponse, 'fir'), {b} = {d}.Coefficients; else, {sos} = {d}.Coefficients; end\n"
        );
        self.pending = Some((b.clone(), sos.clone()));
        Ok(super::all_or_nothing(&script, &[b, sos]))
    }
    /// Takes the copy a `read_object` script left.
    fn adopt(&mut self, ws: &Snapshot) {
        if self.form != Form::Object {
            self.pending = None;
            return;
        }
        let Some((b, sos)) = &self.pending else {
            return;
        };
        if ws.get(b).and_then(coefficient_vector).is_some() {
            (self.form, self.b) = (Form::Fir, b.clone());
        } else if ws.get(sos).and_then(section_matrix).is_some() {
            (self.form, self.sos) = (Form::Sos, sos.clone());
            self.gain.clear();
        } else {
            return;
        }
        self.pending = None;
    }
}

/// A filter's arguments in the script.
enum System {
    Tf(String, String),
    Fir(String),
    /// One section: its numerator and denominator are taken as `b, a`
    /// (the analysis calls read a single 1-by-6 row as a numerator).
    Row(String),
    Sos(String),
}
struct Source {
    system: System,
    /// Multiplies the response.
    gain: Option<String>,
    /// Workspace variables read.
    inputs: Vec<String>,
    describe: String,
}
impl Source {
    /// Identifies the form and the coefficient values (not their names):
    /// properties computed for one key hold for any filter with that key.
    fn key(&self, ws: &Snapshot) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.system).hash(&mut hasher);
        for name in &self.inputs {
            if let Some(a @ Array::Numeric(m)) = ws.get(name) {
                a.dims().hash(&mut hasher);
                for v in m.data() {
                    v.to_bits().hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }
    /// The filter arguments of an analysis call.
    fn arguments(&self) -> String {
        match &self.system {
            System::Tf(b, a) => format!("{b}, {a}"),
            System::Fir(b) => format!("{b}, 1"),
            System::Row(s) => format!("{s}(1, 1:3), {s}(1, 4:6)"),
            System::Sos(s) => s.clone(),
        }
    }
    /// `[z, p, k] = ...`: coefficients of z^-1, so the shorter of b and a
    /// is padded with trailing zeros first.
    fn roots_call(&self) -> String {
        match &self.system {
            System::Tf(b, a) => format!(
                "tf2zp([{b}(:).' zeros(1, numel({a}) - numel({b}))], [{a}(:).' zeros(1, numel({b}) - numel({a}))])"
            ),
            System::Fir(b) => format!("tf2zp({b}(:).', [1 zeros(1, numel({b}) - 1)])"),
            System::Row(s) => format!("tf2zp({s}(1, 1:3), {s}(1, 4:6))"),
            System::Sos(s) => format!("sos2zp({s})"),
        }
    }
    /// A check that keeps a saved script honest when the variable changed.
    fn guard(&self) -> Option<String> {
        match &self.system {
            System::Row(s) => Some(format!(
                "if ~isequal(size({s}), [1 6]), error('Filter Visualization: {s} is no longer a single 1-by-6 section'); end\n"
            )),
            _ => None,
        }
    }
}

/// Functions the script calls: a variable of that name would shadow them.
const SCRIPT_FUNCTIONS: [&str; 41] = [
    "freqz", "phasez", "grpdelay", "impz", "stepz", "tf2zp", "sos2zp", "angle", "rad2deg", "abs",
    "log10", "plot", "semilogx", "stem", "subplot", "figure", "hold", "grid", "xlabel", "ylabel",
    "title", "legend", "axis", "real", "imag", "cos", "sin", "linspace", "zeros", "numel", "pi",
    "isequal", "size", "error", "text", "sprintf", "mean", "sum", "max", "true", "isa",
];

fn variable(name: &str, what: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("choose {what}"));
    }
    super::single_expression(name).map_err(|e| format!("{what}: {e}"))?;
    if name.len() > 63 || !matrust::files::valid_name(name) {
        return Err(format!("{what}: choose a workspace variable"));
    }
    if SCRIPT_FUNCTIONS.contains(&name) {
        return Err(format!(
            "{what}: the variable {name} hides a function the script calls; rename it"
        ));
    }
    Ok(())
}
fn scalar(text: &str, what: &str) -> Result<f64, String> {
    if text.len() > 64 {
        return Err(format!("{what}: at most 64 characters"));
    }
    super::single_expression(text).map_err(|e| format!("{what}: {e}"))?;
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what}: enter one finite numeric literal"))
}
fn whole_number(text: &str, what: &str, lo: usize, hi: usize) -> Result<usize, String> {
    let v = scalar(text, what)?;
    if v.fract() != 0. || !(lo as f64..=hi as f64).contains(&v) {
        return Err(format!("{what} must be an integer from {lo} to {hi}"));
    }
    Ok(v as usize)
}
/// Length of a nonempty real double vector of finite values.
fn coefficient_vector(a: &Array) -> Option<usize> {
    match a {
        Array::Numeric(m)
            if !a.is_complex()
                && !a.is_empty()
                && a.len() <= MAX_COEFFICIENTS
                && a.dims().len() == 2
                && a.dims().iter().filter(|d| **d > 1).count() <= 1
                && m.data().iter().all(|v| v.is_finite()) =>
        {
            Some(a.len())
        }
        _ => None,
    }
}
/// Rows of a real L-by-6 matrix of finite values with nonzero a0.
fn section_matrix(a: &Array) -> Option<usize> {
    match a {
        Array::Numeric(m) if !a.is_complex() => {
            let dims = a.dims();
            let rows = dims[0];
            (dims.len() == 2
                && dims[1] == 6
                && (1..=MAX_SECTIONS).contains(&rows)
                && m.data().iter().all(|v| v.is_finite())
                && m.data()[3 * rows..4 * rows].iter().all(|v| *v != 0.))
            .then_some(rows)
        }
        _ => None,
    }
}
fn first_value(a: Option<&Array>) -> Option<f64> {
    match a {
        Some(Array::Numeric(m)) => m.data().first().copied(),
        _ => None,
    }
}
/// An engine-owned object (the workspace holds only its handle).
fn object_handle(a: &Array) -> bool {
    matches!(a, Array::Struct(s) if s.len() == 1 && s.field_count() == 1
        && s.field_names()[0] == "MatRustHandle")
}
fn rgb(index: usize) -> String {
    let [r, g, b] = matrust::plot::COLORS[index % matrust::plot::COLORS.len()];
    format!(
        "[{} {} {}]",
        number(r as f64 / 255.),
        number(g as f64 / 255.),
        number(b as f64 / 255.)
    )
}

/// What the Output column says about one filter. A flag is NaN when the
/// roots are out of the engine's reach (more than 1000 coefficients).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub order: f64,
    pub stable: f64,
    pub minimum_phase: f64,
    pub linear_phase: f64,
    pub fir: bool,
    /// Second-order sections; 0 for b, a.
    pub sections: usize,
    /// Numerator coefficients after leading and trailing zeros.
    pub taps: usize,
}
impl Info {
    pub fn form(&self) -> String {
        let kind = if self.fir { "FIR" } else { "IIR" };
        match (self.sections, self.fir) {
            (0, true) => format!("{kind}, {} taps", self.taps),
            (0, false) => format!("{kind}, direct form"),
            (1, _) => format!("{kind}, 1 section"),
            (n, _) => format!("{kind}, {n} sections"),
        }
    }
}
fn flag(v: f64) -> &'static str {
    if v.is_nan() {
        "n/a"
    } else if v != 0. {
        "Yes"
    } else {
        "No"
    }
}

/// One filter of the last successful run.
#[derive(Clone, Debug)]
pub struct Curve {
    pub id: u32,
    pub name: String,
    pub color: usize,
    /// Display axis: Hz or normalised frequency, seconds or samples.
    pub x: Vec<f64>,
    /// First plot: magnitude, phase, delay or amplitude.
    pub y: Vec<f64>,
    /// Second plot of magnitude + phase: the phase.
    pub y2: Vec<f64>,
    pub zeros: Vec<[f64; 2]>,
    pub poles: Vec<[f64; 2]>,
    /// Locations holding several zeros or poles, with the count.
    pub zero_marks: Vec<([f64; 2], usize)>,
    pub pole_marks: Vec<([f64; 2], usize)>,
    pub info: Info,
}

/// The last successful run and the settings it was computed with.
#[derive(Clone, Debug)]
pub struct Shown {
    pub view: View,
    pub hz: bool,
    pub log: bool,
    pub db: bool,
    pub degrees: bool,
    pub curves: Vec<Curve>,
}
impl Shown {
    fn x_label(&self) -> &'static str {
        match (self.view.time(), self.hz) {
            (true, true) => "Time (s)",
            (true, false) => "Samples",
            (false, true) => AXIS_HZ,
            (false, false) => AXIS_NORMALIZED,
        }
    }
    fn magnitude_label(&self) -> &'static str {
        if self.db {
            "Magnitude (dB)"
        } else {
            "Magnitude"
        }
    }
    fn phase_label(&self) -> &'static str {
        if self.degrees {
            "Phase (degrees)"
        } else {
            "Phase (rad)"
        }
    }
    /// Axis label of the first plot.
    fn y_label(&self) -> &'static str {
        match self.view {
            View::Magnitude | View::MagnitudePhase => self.magnitude_label(),
            View::Phase => self.phase_label(),
            View::GroupDelay => "Group delay (samples)",
            View::PhaseDelay => "Phase delay (samples)",
            View::Impulse | View::Step => "Amplitude",
            View::PoleZero => "Imaginary part",
        }
    }
    /// The hover readout at display position `x`: every filter's value at
    /// its nearest computed point.
    pub fn readout(&self, x: f64) -> Option<String> {
        if self.view == View::PoleZero || !x.is_finite() {
            return None;
        }
        let position = match (self.view.time(), self.hz) {
            (true, true) => format!("t = {} s", short(x)),
            (true, false) => format!("n = {}", short(x)),
            (false, true) => format!("f = {} Hz", short(x)),
            (false, false) => format!("f = {} (Nyquist = 1)", short(x)),
        };
        let unit = |label: &str| match label {
            "Magnitude (dB)" => " dB",
            "Phase (degrees)" => "\u{b0}",
            "Phase (rad)" => " rad",
            "Group delay (samples)" | "Phase delay (samples)" => " samples",
            _ => "",
        };
        let mut parts = vec![position];
        for c in &self.curves {
            let Some(at) = nearest(&c.x, x) else {
                continue;
            };
            let mut text = format!("{}: {}{}", c.name, short(c.y[at]), unit(self.y_label()));
            if let Some(v) = c.y2.get(at) {
                text.push_str(&format!(", {}{}", short(*v), unit(self.phase_label())));
            }
            parts.push(text);
        }
        (parts.len() > 1).then(|| parts.join("  \u{b7}  "))
    }
}
/// Index of the value of ascending `x` nearest to `at`.
fn nearest(x: &[f64], at: f64) -> Option<usize> {
    if x.is_empty() {
        return None;
    }
    let right = x.partition_point(|v| *v < at).min(x.len() - 1);
    let left = right.saturating_sub(1);
    Some(if (at - x[left]).abs() <= (x[right] - at).abs() {
        left
    } else {
        right
    })
}

pub struct FilterVisualizer {
    pub filters: Vec<Filter>,
    pub selected: usize,
    pub view: View,
    /// Magnitude in dB (else linear).
    pub db: bool,
    /// Phase unwrapped (else wrapped to -pi..pi).
    pub unwrapped: bool,
    pub degrees: bool,
    /// The whole circle, 0..fs (else 0..fs/2).
    pub whole: bool,
    /// Logarithmic frequency axis.
    pub log: bool,
    pub points: String,
    /// Impulse and step length in samples; blank is automatic.
    pub length: String,
    /// Export base names, by `Part`.
    pub names: [String; 11],
    pub current: Option<Shown>,
    /// Set by the host: a private run is in flight.
    pub running: bool,
    /// Set by the host: a workspace command is running.
    pub busy: bool,
    /// The script whose run Stop cancelled; it stays stopped until an input
    /// changes or Run is pressed.
    stopped: Option<u64>,
    /// A digitalFilter copy script waiting to run in the workspace.
    convert: Option<String>,
    /// Each filter's properties with the key of the coefficients they were
    /// computed from, so a new view does not find the roots again.
    info: BTreeMap<u32, (u64, Info)>,
    next_id: u32,
}
impl Default for FilterVisualizer {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            selected: 0,
            view: View::Magnitude,
            db: true,
            unwrapped: true,
            degrees: true,
            whole: false,
            log: false,
            points: DEFAULT_POINTS.to_string(),
            length: String::new(),
            names: Part::ALL.map(|p| p.default_name().to_owned()),
            current: None,
            running: false,
            busy: false,
            stopped: None,
            convert: None,
            info: BTreeMap::new(),
            next_id: 1,
        }
    }
}

/// One shown filter of a plan.
struct Record {
    id: u32,
    name: String,
    color: usize,
    source: Source,
    fs: Option<f64>,
    /// Exported names in the order of the view's parts.
    names: Vec<String>,
}
struct Plan {
    script: String,
    records: Vec<Record>,
    hz: bool,
    /// The pole-zero view's local functions: clusters, marks.
    helpers: Option<[String; 2]>,
}

impl FilterVisualizer {
    pub fn add(&mut self, form: Form, ws: &Snapshot) -> Result<&mut Filter, String> {
        if self.filters.len() >= MAX_FILTERS {
            return Err(format!("At most {MAX_FILTERS} filters"));
        }
        let name = unused_name("filter", |n| {
            n == "filter" || ws.contains(n) || self.filters.iter().any(|f| f.name == n)
        });
        let mut filter = Filter::new(self.next_id, name);
        filter.form = form;
        self.next_id += 1;
        self.filters.push(filter);
        self.selected = self.filters.len() - 1;
        Ok(self.filters.last_mut().unwrap())
    }
    pub fn remove(&mut self) {
        if self.selected < self.filters.len() {
            let removed = self.filters.remove(self.selected);
            self.info.remove(&removed.id);
        }
        self.selected = self.selected.min(self.filters.len().saturating_sub(1));
    }
    /// Moves the selected filter one place up (`-1`) or down (`1`).
    pub fn reorder(&mut self, step: isize) {
        let Some(to) = self.selected.checked_add_signed(step) else {
            return;
        };
        if self.selected < self.filters.len() && to < self.filters.len() {
            self.filters.swap(self.selected, to);
            self.selected = to;
        }
    }
    /// Asks the workspace to copy the selected digitalFilter's coefficients.
    pub fn read_selected_object(&mut self, ws: &Snapshot) -> Result<(), String> {
        let filter = self
            .filters
            .get_mut(self.selected)
            .ok_or("Select a filter")?;
        self.convert = Some(filter.read_object(ws)?);
        Ok(())
    }
    /// Stop: cancels the run of the current script; none starts until an
    /// input changes or Run is pressed.
    pub fn stop(&mut self, ws: &Snapshot) {
        self.stopped = self.script_key(ws);
    }
    pub fn run(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self, ws: &Snapshot) -> bool {
        self.stopped.is_some() && self.stopped == self.script_key(ws)
    }
    fn script_key(&self, ws: &Snapshot) -> Option<u64> {
        let plan = self.plan(ws).ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        plan.script.hash(&mut hasher);
        Some(hasher.finish())
    }
    fn name_of(&self, part: Part) -> &str {
        &self.names[part.index()]
    }
    /// The points of a frequency view.
    fn point_count(&self) -> Result<usize, String> {
        whole_number(&self.points, "Filter Visualization: Points", 2, MAX_POINTS)
    }
    /// The samples of a time view; None is the engine's automatic length.
    fn sample_count(&self) -> Result<Option<usize>, String> {
        if self.length.trim().is_empty() {
            return Ok(None);
        }
        whole_number(
            &self.length,
            "Filter Visualization: Response length",
            1,
            MAX_SAMPLES,
        )
        .map(Some)
    }

    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.filters.len() > MAX_FILTERS {
            return Err(format!(
                "Filter Visualization: at most {MAX_FILTERS} filters"
            ));
        }
        let shown: Vec<&Filter> = self.filters.iter().filter(|f| f.shown).collect();
        if shown.is_empty() {
            return Err("Filter Visualization: add a filter and tick Show".into());
        }
        let view = self.view;
        let points = if view.frequency() {
            self.point_count()?
        } else {
            0
        };
        let samples = if view.time() {
            self.sample_count()?
        } else {
            None
        };
        let mut labels = BTreeSet::new();
        let mut sources = Vec::new();
        for f in &shown {
            super::single_expression(&f.name)
                .map_err(|e| format!("Filter Visualization: filter name: {e}"))?;
            if f.name.len() > 40 || !matrust::files::valid_name(&f.name) {
                return Err(format!(
                    "Filter Visualization: name a filter like a variable (letters, digits, _; at most 40 characters), not '{}'",
                    f.name.chars().take(40).collect::<String>()
                ));
            }
            if !labels.insert(f.name.clone()) {
                return Err(format!(
                    "Filter Visualization: two shown filters are named {}",
                    f.name
                ));
            }
            let prefix = |e: String| format!("Filter Visualization: {}: {e}", f.name);
            sources.push((
                f.source(ws).map_err(prefix)?,
                f.sample_rate().map_err(prefix)?,
            ));
        }
        // Hz (and seconds) only when every shown filter has a sample rate.
        let hz = sources.iter().all(|(_, fs)| fs.is_some());
        let inputs: BTreeSet<String> = sources
            .iter()
            .flat_map(|(s, _)| s.inputs.iter().cloned())
            .collect();
        let mut exports = BTreeSet::new();
        let mut records = Vec::new();
        for (f, (source, fs)) in shown.iter().zip(sources) {
            let mut names = Vec::new();
            for part in view.parts() {
                let base = self.name_of(*part);
                super::single_expression(base)
                    .map_err(|e| format!("Filter Visualization: export name: {e}"))?;
                let name = format!("{base}_{}", f.name);
                if !matrust::files::valid_name(base) || name.len() > 63 {
                    return Err(format!(
                        "Filter Visualization: {} needs a valid export name (with the filter name at most 63 characters)",
                        part.label()
                    ));
                }
                super::check_export_name(&name)?;
                // The script reads its inputs after it writes its outputs.
                if inputs.contains(&name) {
                    return Err(format!(
                        "Filter Visualization: export name {name} is a filter's own input; choose another name"
                    ));
                }
                if !exports.insert(name.clone()) {
                    return Err(format!(
                        "Filter Visualization: duplicate export name {name}"
                    ));
                }
                names.push(name);
            }
            records.push(Record {
                id: f.id,
                name: f.name.clone(),
                color: f.color,
                source,
                fs,
                names,
            });
        }
        // The pole-zero view's local functions avoid every name in play.
        let helpers = (view == View::PoleZero).then(|| {
            let mut taken = exports.clone();
            ["fv_multiplicity", "fv_marks"].map(|base| {
                let name = unused_name(base, |n| ws.contains(n) || taken.contains(n));
                taken.insert(name.clone());
                name
            })
        });
        let script = self.script_text(&records, hz, points, samples, helpers.as_ref());
        Ok(Plan {
            script,
            records,
            hz,
            helpers,
        })
    }

    /// `, n[, 'whole'][, fs]` of a frequency call.
    fn frequency_arguments(&self, points: usize, fs: Option<f64>, hz: bool) -> String {
        let mut text = format!(", {points}");
        if self.whole {
            text.push_str(", 'whole'");
        }
        if let (true, Some(fs)) = (hz, fs) {
            text.push_str(&format!(", {}", number(fs)));
        }
        text
    }

    /// The analysis lines of one filter.
    fn analysis(&self, r: &Record, hz: bool, points: usize, samples: Option<usize>) -> String {
        let args = r.source.arguments();
        let freq = self.frequency_arguments(points, r.fs, hz);
        let gain = r.source.gain.as_deref();
        let name = |part: Part| -> &str {
            let at = self.view.parts().iter().position(|p| *p == part).unwrap();
            &r.names[at]
        };
        let mut out = format!("% {}: {}", r.name, r.source.describe);
        match (hz, r.fs) {
            (true, Some(fs)) => out.push_str(&format!(", sample rate {} Hz\n", number(fs))),
            (false, Some(_)) => out.push_str(", sample rate not used\n"),
            _ => out.push('\n'),
        }
        if let Some(guard) = r.source.guard() {
            out.push_str(&guard);
        }
        let scaled = |out: &mut String, what: &str| {
            if let Some(g) = gain {
                out.push_str(&format!("{what} = {g} * {what};\n"));
            }
        };
        let turned = |out: &mut String, what: &str| {
            if let Some(g) = gain {
                out.push_str(&format!("{what} = {what} + angle({g});\n"));
            }
        };
        let degrees = |out: &mut String, what: &str| {
            if self.degrees {
                out.push_str(&format!("{what} = rad2deg({what});\n"));
            }
        };
        match self.view {
            View::Magnitude => {
                let (h, w) = (name(Part::Response), name(Part::Frequency));
                out.push_str(&format!("[{h}, {w}] = freqz({args}{freq});\n"));
                scaled(&mut out, h);
            }
            View::Phase => {
                let (phi, w) = (name(Part::Phase), name(Part::Frequency));
                if self.unwrapped {
                    out.push_str(&format!("[{phi}, {w}] = phasez({args}{freq});\n"));
                    turned(&mut out, phi);
                } else {
                    out.push_str(&format!("[{phi}, {w}] = freqz({args}{freq});\n"));
                    match gain {
                        Some(g) => out.push_str(&format!("{phi} = angle({g} * {phi});\n")),
                        None => out.push_str(&format!("{phi} = angle({phi});\n")),
                    }
                }
                degrees(&mut out, phi);
            }
            View::MagnitudePhase => {
                let (h, phi, w) = (
                    name(Part::Response),
                    name(Part::Phase),
                    name(Part::Frequency),
                );
                out.push_str(&format!("[{h}, {w}] = freqz({args}{freq});\n"));
                scaled(&mut out, h);
                if self.unwrapped {
                    out.push_str(&format!("{phi} = phasez({args}{freq});\n"));
                    turned(&mut out, phi);
                } else {
                    out.push_str(&format!("{phi} = angle({h});\n"));
                }
                degrees(&mut out, phi);
            }
            View::GroupDelay => {
                let (gd, w) = (name(Part::GroupDelay), name(Part::Frequency));
                out.push_str(&format!("[{gd}, {w}] = grpdelay({args}{freq});\n"));
            }
            View::PhaseDelay => {
                let (pd, w) = (name(Part::PhaseDelay), name(Part::Frequency));
                out.push_str(&format!("[{pd}, {w}] = phasez({args}{freq});\n"));
                turned(&mut out, pd);
                // Phase over angular frequency in rad/sample; undefined at 0.
                let radians = match (hz, r.fs) {
                    (true, Some(fs)) => format!("(2*pi*{w}/{})", number(fs)),
                    _ => w.to_owned(),
                };
                out.push_str(&format!("{pd} = -{pd} ./ {radians};\n"));
            }
            View::Impulse | View::Step => {
                let (y, call) = if self.view == View::Impulse {
                    (name(Part::Impulse), "impz")
                } else {
                    (name(Part::Step), "stepz")
                };
                let t = name(Part::Time);
                let rest = match (samples, hz.then_some(r.fs).flatten()) {
                    (None, None) => String::new(),
                    (Some(n), None) => format!(", {n}"),
                    (None, Some(fs)) => format!(", [], {}", number(fs)),
                    (Some(n), Some(fs)) => format!(", {n}, {}", number(fs)),
                };
                out.push_str(&format!("[{y}, {t}] = {call}({args}{rest});\n"));
                scaled(&mut out, y);
            }
            View::PoleZero => {
                let (z, p, k) = (name(Part::Zeros), name(Part::Poles), name(Part::Gain));
                out.push_str(&format!("[{z}, {p}, {k}] = {};\n", r.source.roots_call()));
                scaled(&mut out, k);
            }
        }
        out
    }

    /// The display abscissa of a record in script text.
    fn x_expression(&self, r: &Record, hz: bool) -> String {
        let parts = self.view.parts();
        let at = |part: Part| &r.names[parts.iter().position(|p| *p == part).unwrap()];
        if self.view.time() {
            at(Part::Time).clone()
        } else if hz {
            at(Part::Frequency).clone()
        } else {
            format!("{}/pi", at(Part::Frequency))
        }
    }
    /// The plotted ordinates of a record in script text: one per plot.
    fn y_expressions(&self, r: &Record) -> Vec<String> {
        let parts = self.view.parts();
        let at = |part: Part| &r.names[parts.iter().position(|p| *p == part).unwrap()];
        let magnitude = |h: &str| {
            if self.db {
                format!("20*log10(abs({h}))")
            } else {
                format!("abs({h})")
            }
        };
        match self.view {
            View::Magnitude => vec![magnitude(at(Part::Response))],
            View::Phase => vec![at(Part::Phase).clone()],
            View::MagnitudePhase => {
                vec![magnitude(at(Part::Response)), at(Part::Phase).clone()]
            }
            View::GroupDelay => vec![at(Part::GroupDelay).clone()],
            View::PhaseDelay => vec![at(Part::PhaseDelay).clone()],
            View::Impulse => vec![at(Part::Impulse).clone()],
            View::Step => vec![at(Part::Step).clone()],
            View::PoleZero => Vec::new(),
        }
    }
    fn labels(&self, hz: bool) -> Shown {
        Shown {
            view: self.view,
            hz,
            log: self.log && self.view.frequency(),
            db: self.db,
            degrees: self.degrees,
            curves: Vec::new(),
        }
    }

    fn script_text(
        &self,
        records: &[Record],
        hz: bool,
        points: usize,
        samples: Option<usize>,
        helpers: Option<&[String; 2]>,
    ) -> String {
        let view = self.view;
        let labels = self.labels(hz);
        let mut script = format!("% Filter Visualization: {}", view.title().to_lowercase());
        if view.frequency() {
            script.push_str(&format!(
                ", {points} points over {}.\n",
                match (self.whole, hz) {
                    (false, true) => "0 to fs/2",
                    (true, true) => "0 to fs",
                    (false, false) => "0 to pi rad/sample",
                    (true, false) => "0 to 2 pi rad/sample",
                }
            ));
        } else if view.time() {
            script.push_str(&match samples {
                Some(n) => format!(", {n} samples.\n"),
                None => ", automatic length.\n".to_owned(),
            });
        } else {
            script.push_str(".\n");
        }
        if view == View::PhaseDelay {
            script.push_str("% Phase delay = -phase / angular frequency, in samples.\n");
        }
        for r in records {
            script.push_str(&self.analysis(r, hz, points, samples));
        }
        script.push_str("figure;\n");
        if view == View::PoleZero {
            let [_, marks] = helpers.expect("pole-zero helpers");
            script.push_str(
                "plot(cos(linspace(0, 2*pi, 361)), sin(linspace(0, 2*pi, 361)), ':', 'Color', [0.5 0.5 0.5], 'DisplayName', 'Unit circle');\nhold on;\n",
            );
            for r in records {
                let (z, p) = (&r.names[0], &r.names[1]);
                let color = rgb(r.color);
                script.push_str(&format!(
                    "plot(real({z}), imag({z}), 'o', 'Color', {color}, 'DisplayName', '{} zeros');\n\
                     plot(real({p}), imag({p}), 'x', 'Color', {color}, 'DisplayName', '{} poles');\n\
                     {marks}({z}); {marks}({p});\n",
                    r.name, r.name
                ));
            }
            script.push_str(
                "hold off;\naxis equal; grid on;\nxlabel('Real part'); ylabel('Imaginary part'); title('Pole-zero plot'); legend('show');\n",
            );
            script.push_str(&self.helper_text(helpers.unwrap()));
            return script;
        }
        let command = if view.time() {
            "stem"
        } else if self.log {
            "semilogx"
        } else {
            "plot"
        };
        let panels: Vec<&str> = match view {
            View::MagnitudePhase => vec![labels.magnitude_label(), labels.phase_label()],
            _ => vec![labels.y_label()],
        };
        for (k, y_label) in panels.iter().enumerate() {
            if panels.len() > 1 {
                script.push_str(&format!("subplot({}, 1, {});\n", panels.len(), k + 1));
            }
            for (i, r) in records.iter().enumerate() {
                script.push_str(&format!(
                    "{command}({}, {}, 'Color', {}, 'DisplayName', '{}');\n",
                    self.x_expression(r, hz),
                    self.y_expressions(r)[k],
                    rgb(r.color),
                    r.name
                ));
                // The first plot sets the axis scale; the others join it.
                if i == 0 && records.len() > 1 {
                    script.push_str("hold on;\n");
                }
            }
            if records.len() > 1 {
                script.push_str("hold off;\n");
            }
            script.push_str(&format!(
                "grid on; xlabel('{}'); ylabel('{y_label}');",
                labels.x_label()
            ));
            if k == 0 {
                script.push_str(&format!(" title('{}'); legend('show');", view.title()));
            }
            script.push('\n');
        }
        script
    }

    /// The pole-zero view's local functions. Roots closer than 1e-3
    /// (relative to max(1, |root|)) share a marker on any screen, so they
    /// count as one location; a location with several roots gets its count.
    fn helper_text(&self, [multiplicity, marks]: &[String; 2]) -> String {
        format!(
            "\nfunction [u, m] = {multiplicity}(r)\n\
             % Locations of the roots r and the number of roots at each: roots closer\n\
             % than 1e-3 (relative to max(1, |root|)) count as one location.\n\
             r = r(:); u = zeros(0, 1); m = zeros(0, 1);\n\
             left = true(size(r));\n\
             for i = 1:numel(r)\n\
             \x20   if left(i)\n\
             \x20       near = left & abs(r - r(i)) <= 1e-3 * max(1, abs(r(i)));\n\
             \x20       u = [u; mean(r(near))]; m = [m; sum(near)];\n\
             \x20       left(near) = false;\n\
             \x20   end\n\
             end\n\
             end\n\
             \nfunction {marks}(r)\n\
             % Writes the count beside every location that holds several roots.\n\
             [u, m] = {multiplicity}(r);\n\
             for i = 1:numel(m)\n\
             \x20   if m(i) > 1, text(real(u(i)), imag(u(i)), sprintf('  %d', m(i))); end\n\
             end\n\
             end\n"
        )
    }

    /// Probe lines for the Output column of one record: order, stability,
    /// minimum phase, linear phase, FIR, sections and numerator length.
    /// A zero r of the polynomial c counts as outside the unit circle only
    /// beyond its own numerical uncertainty: the smallest d with
    /// |c^(j)(r)| d^j / j! equal to the rounding level of c at r, over the
    /// derivative orders j. A simple zero is then known to about eps and
    /// an m-fold one to about eps^(1/m), so the repeated zeros of a
    /// Butterworth numerator stay on the circle while (z + 2)^2 is outside.
    fn info_probe(&self, r: &Record, q: &str, out: &str) -> String {
        let mut code = String::new();
        match &r.source.system {
            System::Sos(s) => code.push_str(&format!(
                "{q}_o = filtord({s}); {q}_n = {s}(:, 1:3); {q}_d = {s}(:, 4:6);\n\
                 [{q}_fb, {q}_fa] = sos2tf({s}); {q}_sec = size({s}, 1);\n"
            )),
            system => {
                let (b, a, sections) = match system {
                    System::Tf(b, a) => (b.clone(), a.clone(), 0),
                    System::Fir(b) => (b.clone(), "1".to_owned(), 0),
                    System::Row(s) => (format!("{s}(1, 1:3)"), format!("{s}(1, 4:6)"), 1),
                    System::Sos(_) => unreachable!(),
                };
                code.push_str(&format!(
                    "{q}_n = {b}; {q}_n = {q}_n(:).'; {q}_d = {a}; {q}_d = {q}_d(:).';\n\
                     {q}_o = filtord({q}_n, {q}_d); {q}_fb = {q}_n; {q}_fa = {q}_d; {q}_sec = {sections};\n"
                ));
            }
        }
        // Leading and trailing zeros are delays and padding, not roots.
        let trim = |v: &str| {
            format!(
                "if any({v} ~= 0), {v} = {v}(find({v} ~= 0, 1, 'first'):find({v} ~= 0, 1, 'last')); else, {v} = 0; end\n"
            )
        };
        let symmetric = |v: &str| {
            format!(
                "(max(abs({v} - fliplr({v}))) <= eps^(2/3) * max(abs({v})) || max(abs({v} + fliplr({v}))) <= eps^(2/3) * max(abs({v})))"
            )
        };
        code.push_str(&format!(
            "{q}_st = 1; {q}_mp = 1;\n\
             try\n\
             for {q}_k = 1:size({q}_n, 1)\n\
             {q}_c = {q}_d({q}_k, :);\n{}\
             if numel({q}_c) > 1, {q}_st = {q}_st && all(abs(roots({q}_c)) < 1); end\n\
             {q}_c = {q}_n({q}_k, :);\n{}\
             if numel({q}_c) > 1\n\
             {q}_r = roots({q}_c);\n\
             {q}_s = 1e4 * eps * polyval(abs({q}_c), abs({q}_r));\n\
             {q}_e = Inf(size({q}_r)); {q}_p = {q}_c;\n\
             for {q}_j = 1:min(numel({q}_c) - 1, 170)\n\
             {q}_p = polyder({q}_p) / {q}_j;\n\
             {q}_e = min({q}_e, ({q}_s ./ abs(polyval({q}_p, {q}_r))) .^ (1 / {q}_j));\n\
             end\n\
             {q}_mp = {q}_mp && all(abs({q}_r) - 1 <= max({q}_e, eps^(2/3)));\n\
             end\n\
             end\n\
             catch\n\
             {q}_st = NaN; {q}_mp = NaN;\n\
             end\n",
            trim(&format!("{q}_c")),
            trim(&format!("{q}_c")),
        ));
        code.push_str(&trim(&format!("{q}_fb")));
        code.push_str(&trim(&format!("{q}_fa")));
        code.push_str(&format!(
            "{out} = [{q}_o, {q}_st, {q}_mp, {} && {}, numel({q}_fa) == 1, {q}_sec, numel({q}_fb)];\n",
            symmetric(&format!("{q}_fb")),
            symmetric(&format!("{q}_fa")),
        ));
        code
    }
}

/// Probe output names of one filter: `<prefix>_<part><id>`.
const PROBE_PARTS: [&str; 9] = ["x", "y", "v", "zr", "zi", "pr", "pi", "mz", "mp"];

impl FilterVisualizer {
    /// A prefix no workspace variable or export name starts with, so the
    /// probe's names cannot collide with the script's.
    fn probe_prefix(&self, ws: &Snapshot, plan: &Plan) -> String {
        let used: Vec<&str> = ws
            .variables()
            .map(|(name, _)| name)
            .chain(
                plan.records
                    .iter()
                    .flat_map(|r| r.names.iter().map(String::as_str)),
            )
            .chain(plan.helpers.iter().flatten().map(String::as_str))
            .collect();
        unused_name("fvq", |p| used.iter().any(|n| n.starts_with(p)))
    }
}

impl App for FilterVisualizer {
    fn kind(&self) -> AppKind {
        AppKind::FilterVisualizer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        Ok(self.plan(ws)?.script)
    }
    fn execution_script(&self, ws: &Snapshot) -> Result<String, String> {
        let script = self.script(ws)?;
        if self.is_stopped(ws) {
            return Err("Filter Visualization: stopped. Change an input or press Run.".into());
        }
        Ok(script)
    }
    fn inputs(&self) -> Vec<String> {
        // Without the workspace, a gain that is not a number names a variable.
        let mut names = BTreeSet::new();
        for f in self.filters.iter().filter(|f| f.shown) {
            match f.form {
                Form::Tf => names.extend([f.b.clone(), f.a.clone()]),
                Form::Fir => {
                    names.insert(f.b.clone());
                }
                Form::Sos => {
                    names.insert(f.sos.clone());
                    let gain = f.gain.trim();
                    if !gain.is_empty() && gain.parse::<f64>().is_err() {
                        names.insert(gain.to_owned());
                    }
                }
                Form::Object => {}
            }
        }
        names.into_iter().collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let q = self.probe_prefix(ws, &plan);
        let mut code = String::new();
        let mut outputs = Vec::new();
        for r in &plan.records {
            let id = r.id;
            let out = |part: &str| format!("{q}_{part}{id}");
            let mut lines = Vec::new();
            if self.view == View::PoleZero {
                let [multiplicity, _] = plan.helpers.as_ref().expect("pole-zero helpers");
                let (z, p) = (&r.names[0], &r.names[1]);
                lines.extend([
                    format!("{} = real({z}(:));", out("zr")),
                    format!("{} = imag({z}(:));", out("zi")),
                    format!("{} = real({p}(:));", out("pr")),
                    format!("{} = imag({p}(:));", out("pi")),
                    format!(
                        "[{q}_u, {q}_m] = {multiplicity}({z}); {} = [real({q}_u), imag({q}_u), {q}_m];",
                        out("mz")
                    ),
                    format!(
                        "[{q}_u, {q}_m] = {multiplicity}({p}); {} = [real({q}_u), imag({q}_u), {q}_m];",
                        out("mp")
                    ),
                    format!("{} = zeros(0, 1); {} = zeros(0, 1); {} = zeros(0, 1);", out("x"), out("y"), out("v")),
                ]);
            } else {
                let ys = self.y_expressions(r);
                lines.push(format!("{} = {};", out("x"), self.x_expression(r, plan.hz)));
                lines.push(format!("{} = {};", out("y"), ys[0]));
                lines.push(match ys.get(1) {
                    Some(y) => format!("{} = {y};", out("v")),
                    None => format!("{} = zeros(0, 1);", out("v")),
                });
                for part in &PROBE_PARTS[3..] {
                    lines.push(format!("{} = zeros(0, 1);", out(part)));
                }
            }
            code.push_str(&lines.join("\n"));
            code.push('\n');
            // Properties already known for these coefficients are passed
            // through; the key travels with them (two 32-bit halves).
            let key = r.source.key(ws);
            match self.info.get(&id) {
                Some((known, info)) if *known == key => {
                    let values = [
                        info.order,
                        info.stable,
                        info.minimum_phase,
                        info.linear_phase,
                        f64::from(u8::from(info.fir)),
                        info.sections as f64,
                        info.taps as f64,
                    ];
                    let row: Vec<String> = values.iter().map(|v| number(*v)).collect();
                    code.push_str(&format!("{} = [{}];\n", out("i"), row.join(", ")));
                }
                _ => code.push_str(&self.info_probe(r, &q, &out("i"))),
            }
            code.push_str(&format!(
                "{} = [{}, {}];\n",
                out("key"),
                key >> 32,
                key & 0xffff_ffff
            ));
            outputs.extend(PROBE_PARTS.iter().map(|part| out(part)));
            outputs.extend([out("i"), out("key")]);
        }
        (code, outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let missing = |name: &str| format!("Filter Visualization: missing result {name}");
        // Every probe output is `<prefix>_<part><id>`.
        let q = outputs
            .keys()
            .find_map(|name| name.rsplit_once('_'))
            .map(|(prefix, _)| prefix.to_owned())
            .ok_or_else(|| missing("of the probe"))?;
        let read = |name: String| -> Result<Vec<f64>, String> {
            match outputs.get(&name) {
                Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
                Some(Array::Logical(m)) => Ok(m.data().iter().map(|v| f64::from(*v)).collect()),
                _ => Err(missing(&name)),
            }
        };
        let mut shown = self.labels(false);
        let mut known = Vec::new();
        // Hz when the run used every filter's sample rate (as the plan did).
        shown.hz = self
            .filters
            .iter()
            .filter(|f| f.shown)
            .all(|f| matches!(f.sample_rate(), Ok(Some(_))));
        for f in self.filters.iter().filter(|f| f.shown) {
            let value = |part: &str| read(format!("{q}_{part}{}", f.id));
            let (x, y, y2) = (value("x")?, value("y")?, value("v")?);
            if x.len() != y.len() || (!y2.is_empty() && y2.len() != x.len()) {
                return Err("Filter Visualization: incomplete response".into());
            }
            let points = |re: Vec<f64>, im: Vec<f64>| -> Vec<[f64; 2]> {
                re.into_iter().zip(im).map(|(a, b)| [a, b]).collect()
            };
            let marks = |flat: Vec<f64>| -> Vec<([f64; 2], usize)> {
                let n = flat.len() / 3;
                (0..n)
                    .filter(|k| flat[2 * n + k] > 1.)
                    .map(|k| ([flat[k], flat[n + k]], flat[2 * n + k] as usize))
                    .collect()
            };
            let (info, key) = (value("i")?, value("key")?);
            if info.len() != 7 || key.len() != 2 {
                return Err("Filter Visualization: incomplete filter information".into());
            }
            let info = Info {
                order: info[0],
                stable: info[1],
                minimum_phase: info[2],
                linear_phase: info[3],
                fir: info[4] != 0.,
                sections: info[5] as usize,
                taps: info[6] as usize,
            };
            known.push((f.id, ((key[0] as u64) << 32 | key[1] as u64, info)));
            shown.curves.push(Curve {
                id: f.id,
                name: f.name.clone(),
                color: f.color,
                x,
                y,
                y2,
                zeros: points(value("zr")?, value("zi")?),
                poles: points(value("pr")?, value("pi")?),
                zero_marks: marks(value("mz")?),
                pole_marks: marks(value("mp")?),
                info,
            });
        }
        self.info.extend(known);
        self.current = Some(shown);
        Ok(())
    }
    fn prepare(&mut self, ws: &Snapshot) {
        self.selected = self.selected.min(self.filters.len().saturating_sub(1));
        for f in &mut self.filters {
            f.adopt(ws);
        }
        if self.stopped.is_some() && !self.is_stopped(ws) {
            self.stopped = None;
        }
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.list_ui(ui, ws);
        if self.selected < self.filters.len() {
            ui.add_space(crate::tokens::S);
            self.filter_ui(ui, ws);
        }
        ui.add_space(crate::tokens::S);
        ui.separator();
        ui.add_space(crate::tokens::XS);
        text_field(ui, "Frequency points (2\u{2013}65536)", &mut self.points);
        super::expression_message(ui, self.point_count().map(|_| ()));
        text_field(
            ui,
            "Impulse and step length in samples (blank: automatic)",
            &mut self.length,
        );
        super::expression_message(ui, self.sample_count().map(|_| ()));
        if self.is_stopped(ws) {
            ui.horizontal(|ui| {
                crate::tokens::helper(ui, "Stopped");
                if ui.button("Run").clicked() {
                    self.run();
                }
            });
        } else if self.running {
            ui.horizontal(|ui| {
                crate::tokens::helper(ui, "Computing\u{2026}");
                if ui.button("Stop").clicked() {
                    self.stop(ws);
                }
            });
        }
        crate::tokens::helper(
            ui,
            "Up to 8 filters. The axis is in Hz when every shown filter has a sample rate.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "RESPONSE"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                let tab = crate::tokens::tab(ui, self.view == view, view.label());
                if view == View::Magnitude {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::FilterVisualizer),
                    );
                }
                if tab.clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::XS);
        self.options_ui(ui);
        ui.add_space(crate::tokens::XS);
        let Some(shown) = &self.current else {
            crate::tokens::helper(
                ui,
                "Add a filter in INPUTS: choose its coefficients from the workspace.",
            );
            return;
        };
        let mixed = {
            let rates: Vec<bool> = self
                .filters
                .iter()
                .filter(|f| f.shown)
                .map(|f| !f.fs.trim().is_empty())
                .collect();
            rates.iter().any(|r| *r) && !rates.iter().all(|r| *r)
        };
        if mixed && shown.view != View::PoleZero {
            crate::tokens::helper(
                ui,
                "Not every shown filter has a sample rate: the axis is normalised.",
            );
        }
        // One line under the plot is kept for the hover readout.
        let height = (ui.available_height() - crate::tokens::ROW - crate::tokens::S).max(160.);
        let text = draw(ui, shown, height)
            .and_then(|x| shown.readout(x))
            .unwrap_or_else(|| match shown.view {
                View::PoleZero => "o zeros, x poles; a number counts roots at one place".into(),
                _ => "Point at the plot to read every filter's value".into(),
            });
        ui.add_sized(
            egui::vec2(ui.available_width(), crate::tokens::ROW),
            egui::Label::new(
                egui::RichText::new(text).color(crate::tokens::palette(ui.ctx()).text_muted),
            )
            .truncate(),
        );
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(f) = self.filters.get(self.selected) else {
            return vec![("Filter", "\u{2014}".into())];
        };
        let info = self
            .current
            .as_ref()
            .and_then(|s| s.curves.iter().find(|c| c.id == f.id))
            .map(|c| c.info);
        let Some(info) = info else {
            return vec![
                ("Filter", f.name.clone()),
                (
                    "Properties",
                    if f.shown {
                        "not computed yet"
                    } else {
                        "show this filter to analyse it"
                    }
                    .into(),
                ),
            ];
        };
        vec![
            ("Filter", f.name.clone()),
            ("Order", short(info.order)),
            ("Stable", flag(info.stable).into()),
            ("Minimum phase", flag(info.minimum_phase).into()),
            ("Linear phase", flag(info.linear_phase).into()),
            ("Form", info.form()),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if let Some(shown) = &self.current
            && shown.curves.len() > 1
        {
            let p = crate::tokens::palette(ui.ctx());
            for c in &shown.curves {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "{}: order {}, {}, {}, {}, {}",
                            c.name,
                            short(c.info.order),
                            match flag(c.info.stable) {
                                "Yes" => "stable",
                                "No" => "unstable",
                                _ => "stability n/a",
                            },
                            match flag(c.info.minimum_phase) {
                                "Yes" => "minimum phase",
                                "No" => "not minimum phase",
                                _ => "minimum phase n/a",
                            },
                            match flag(c.info.linear_phase) {
                                "Yes" => "linear phase",
                                _ => "not linear phase",
                            },
                            c.info.form()
                        ))
                        .color(p.text_muted),
                    )
                    .wrap(),
                );
            }
            ui.add_space(crate::tokens::M);
        }
        let p = crate::tokens::palette(ui.ctx());
        for part in self.view.parts() {
            let name = &mut self.names[part.index()];
            field(ui, &format!("{} name", part.label()), |ui| {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(name).char_limit(63))
                        .desired_width(ui.available_width()),
                );
                if !matrust::files::valid_name(name) {
                    super::expression_message(ui, Err("Use a valid variable name".into()));
                }
            });
        }
        let created = self.exported(ws);
        if created.is_empty() {
            crate::tokens::helper(
                ui,
                "Each name gets the filter name as a suffix, for example h_filter1.",
            );
            return;
        }
        crate::tokens::helper(ui, format!("Export creates {}.", created.join(", ")));
        let replaced: Vec<&str> = created
            .iter()
            .filter(|n| ws.contains(n))
            .map(String::as_str)
            .collect();
        if !replaced.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "Replaces the workspace variable{} {}",
                    if replaced.len() == 1 { "" } else { "s" },
                    replaced.join(", ")
                ))
                .size(crate::tokens::CAPTION)
                .color(p.warning),
            );
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        match self.plan(ws) {
            Ok(plan) => plan.records.into_iter().flat_map(|r| r.names).collect(),
            Err(_) => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        let shown = self.current.as_ref()?;
        let n = shown.curves.len();
        Some(format!(
            "Filter Visualization: {n} filter{} \u{b7} {}",
            if n == 1 { "" } else { "s" },
            shown.view.title().to_lowercase()
        ))
    }
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(value).char_limit(64))
                .desired_width(ui.available_width()),
        );
    });
}

/// A workspace variable dropdown over the variables `accept` keeps.
fn picker(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut String,
    ws: &Snapshot,
    accept: impl Fn(&Array) -> Option<String>,
) {
    field(ui, label, |ui| {
        egui::ComboBox::from_id_salt(id)
            .width(ui.available_width())
            .selected_text(if value.is_empty() {
                "Choose a variable"
            } else {
                value.as_str()
            })
            .show_ui(ui, |ui| {
                for (name, array) in ws.variables() {
                    if let Some(detail) = accept(array) {
                        ui.selectable_value(value, name.to_owned(), format!("{name} ({detail})"));
                    }
                }
            });
    });
}

impl FilterVisualizer {
    fn list_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal_wrapped(|ui| {
            let add = ui.add_enabled(self.filters.len() < MAX_FILTERS, egui::Button::new("Add"));
            if add.clicked() {
                let _ = self.add(Form::Tf, ws);
            }
            let some = self.selected < self.filters.len();
            if ui.add_enabled(some, egui::Button::new("Remove")).clicked() {
                self.remove();
            }
            if ui
                .add_enabled(some && self.selected > 0, egui::Button::new("Up"))
                .clicked()
            {
                self.reorder(-1);
            }
            if ui
                .add_enabled(
                    self.selected + 1 < self.filters.len(),
                    egui::Button::new("Down"),
                )
                .clicked()
            {
                self.reorder(1);
            }
        });
        ui.add_space(crate::tokens::S);
        if self.filters.is_empty() {
            crate::tokens::helper(ui, "No filters yet. Add one, then choose its variables.");
        }
        for (k, f) in self.filters.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.shown, "").on_hover_text("Show filter");
                let response = crate::tokens::color_swatch(
                    ui,
                    crate::tokens::M,
                    response_plot::series_color(f.color),
                    egui::Sense::click(),
                );
                if response.on_hover_text("Change series colour").clicked() {
                    f.color = (f.color + 1) % matrust::plot::COLORS.len();
                }
                if ui.selectable_label(self.selected == k, &f.name).clicked() {
                    self.selected = k;
                }
            });
        }
    }

    fn filter_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let busy = self.busy;
        let mut read = false;
        let f = &mut self.filters[self.selected];
        field(ui, "Name (legend and export suffix)", |ui| {
            ui.add(
                crate::tokens::input(egui::TextEdit::singleline(&mut f.name).char_limit(40))
                    .desired_width(ui.available_width()),
            );
        });
        let mut form = f.form;
        field(ui, "Given as", |ui| {
            egui::ComboBox::from_id_salt("filter_visualizer_form")
                .width(ui.available_width())
                .selected_text(form.label())
                .show_ui(ui, |ui| {
                    for v in Form::ALL {
                        ui.selectable_value(&mut form, v, v.label());
                    }
                });
        });
        if form != f.form {
            f.form = form;
            f.pending = None;
        }
        let vector = |a: &Array| coefficient_vector(a).map(|n| format!("{n} values"));
        match f.form {
            Form::Tf => {
                picker(
                    ui,
                    "filter_visualizer_b",
                    "Numerator b",
                    &mut f.b,
                    ws,
                    vector,
                );
                picker(
                    ui,
                    "filter_visualizer_a",
                    "Denominator a",
                    &mut f.a,
                    ws,
                    vector,
                );
            }
            Form::Fir => {
                picker(
                    ui,
                    "filter_visualizer_b",
                    "Numerator b",
                    &mut f.b,
                    ws,
                    vector,
                );
            }
            Form::Sos => {
                picker(
                    ui,
                    "filter_visualizer_sos",
                    "Sections sos",
                    &mut f.sos,
                    ws,
                    |a| section_matrix(a).map(|rows| format!("{rows}\u{d7}6")),
                );
                text_field(
                    ui,
                    "Gain (blank: 1; a number or a scalar variable)",
                    &mut f.gain,
                );
            }
            Form::Object => {
                picker(
                    ui,
                    "filter_visualizer_object",
                    "digitalFilter variable",
                    &mut f.object,
                    ws,
                    |a| object_handle(a).then(|| "object".to_owned()),
                );
                let ready = !busy && !f.object.is_empty() && ws.contains(&f.object);
                if ui
                    .add_enabled(ready, egui::Button::new("Read coefficients"))
                    .on_disabled_hover_text(
                        "Choose a digitalFilter; available when no command is running",
                    )
                    .clicked()
                {
                    read = true;
                }
                crate::tokens::helper(
                    ui,
                    if f.pending.is_some() {
                        "Waiting for the workspace to copy the coefficients\u{2026}"
                    } else {
                        "A digitalFilter stays in the workspace engine: Read coefficients copies them to a new workspace variable, which is then analysed."
                    },
                );
            }
        }
        text_field(ui, "Sample rate in Hz (blank: normalised)", &mut f.fs);
        let check = f.source(ws).and_then(|_| f.sample_rate()).map(|_| ());
        super::expression_message(ui, check);
        if read {
            let result = self.read_selected_object(ws);
            super::expression_message(ui, result);
        }
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let view = self.view;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            let pair = |ui: &mut egui::Ui, value: &mut bool, on: &str, off: &str| {
                if crate::tokens::toggle(ui, *value, on).clicked() {
                    *value = true;
                }
                if crate::tokens::toggle(ui, !*value, off).clicked() {
                    *value = false;
                }
                ui.add_space(crate::tokens::S);
            };
            if view.has_magnitude() {
                pair(ui, &mut self.db, "dB", "Linear");
            }
            if view.has_phase() {
                pair(ui, &mut self.unwrapped, "Unwrapped", "Wrapped");
                pair(ui, &mut self.degrees, "Degrees", "Radians");
            }
            if view.frequency() {
                let mut half = !self.whole;
                pair(ui, &mut half, "0 to fs/2", "0 to fs");
                self.whole = !half;
                let mut linear = !self.log;
                pair(ui, &mut linear, "Linear axis", "Log axis");
                self.log = !linear;
            }
        });
    }
}

/// Draws the last result; returns the hovered display abscissa.
fn draw(ui: &mut egui::Ui, shown: &Shown, height: f32) -> Option<f64> {
    let circle = crate::tokens::palette(ui.ctx()).text_muted;
    let legend = egui_plot::Legend::default().follow_insertion_order(true);
    if shown.view == View::PoleZero {
        let names: Vec<(String, String)> = shown
            .curves
            .iter()
            .map(|c| (format!("{} zeros", c.name), format!("{} poles", c.name)))
            .collect();
        let marks: Vec<Vec<([f64; 2], usize)>> = shown
            .curves
            .iter()
            .map(|c| c.zero_marks.iter().chain(&c.pole_marks).copied().collect())
            .collect();
        let roots: Vec<Roots<'_>> = shown
            .curves
            .iter()
            .zip(names.iter().zip(&marks))
            .map(|(c, ((zeros, poles), marks))| Roots {
                zeros_name: zeros,
                poles_name: poles,
                color: response_plot::series_color(c.color),
                zeros: &c.zeros,
                poles: &c.poles,
                marks,
            })
            .collect();
        response_plot::plot(
            "filter_visualizer_pole_zero",
            height,
            "Real part",
            "Imaginary part",
        )
        .data_aspect(1.)
        .legend(legend)
        .show(ui, |plot_ui| {
            response_plot::pole_zero(plot_ui, circle, &roots)
        });
        return None;
    }
    let log = shown.log;
    // A logarithmic axis has no place for 0 and below.
    let along = |x: &[f64]| -> Vec<f64> {
        x.iter()
            .map(|v| {
                if !log {
                    *v
                } else if *v > 0. {
                    v.log10()
                } else {
                    f64::NAN
                }
            })
            .collect()
    };
    let xs: Vec<Vec<f64>> = shown.curves.iter().map(|c| along(&c.x)).collect();
    let mut hover = None;
    let mut panel = |ui: &mut egui::Ui, id: &str, height: f32, y_label: &str, second: bool| {
        let series: Vec<Series<'_>> = shown
            .curves
            .iter()
            .zip(&xs)
            .map(|(c, x)| Series {
                name: &c.name,
                color: response_plot::series_color(c.color),
                x,
                y: if second { &c.y2 } else { &c.y },
            })
            .collect();
        let mut chart = response_plot::plot(
            ("filter_visualizer", id.to_owned()),
            height,
            shown.x_label(),
            y_label,
        )
        .legend(legend.clone())
        .label_formatter(|_| None);
        if log {
            chart = chart.x_axis_formatter(|mark, _| short(10_f64.powf(mark.value)));
        }
        chart.show(ui, |plot_ui| {
            if shown.view.time() {
                response_plot::stems(plot_ui, &series);
            } else {
                response_plot::curves(plot_ui, &series);
            }
            if let Some(pointer) = plot_ui.pointer_coordinate() {
                hover = Some(if log {
                    10_f64.powf(pointer.x)
                } else {
                    pointer.x
                });
            }
        });
    };
    if shown.view == View::MagnitudePhase {
        let half = (height - crate::tokens::S) / 2.;
        panel(ui, "magnitude", half, shown.magnitude_label(), false);
        ui.add_space(crate::tokens::S);
        panel(ui, "phase", half, shown.phase_label(), true);
    } else {
        panel(ui, "single", height, shown.y_label(), false);
    }
    hover
}

/// The host's frame: Stop state, then the copy of a digitalFilter.
pub(crate) fn show(
    host: &mut super::Host<FilterVisualizer>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<crate::toolkits::Action>,
) {
    host.app.running = host.running();
    host.app.busy = busy;
    host.show(ui, ws, busy, action);
    if let Some(script) = host.app.convert.take() {
        if busy || action.is_some() {
            host.message = "Wait for the running command to finish, then read again".into();
            if let Some(f) = host.app.filters.get_mut(host.app.selected) {
                f.pending = None;
            }
        } else {
            *action = Some(crate::toolkits::Action::ExportScript(script));
            host.message = "Reading the digitalFilter's coefficients in the workspace".into();
        }
    } else if !busy {
        // The workspace is idle again and this frame's `prepare` found no
        // copy: the read failed (its error is in the Command Window).
        for f in &mut host.app.filters {
            if f.pending.take().is_some() {
                host.message = format!(
                    "{} could not be read as a digitalFilter; see the Command Window",
                    f.object
                );
            }
        }
    }
}
