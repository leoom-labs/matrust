//! Signal Multiresolution Analyzer: split a workspace signal into components
//! with modwtmra, emd, vmd or ewt and rebuild it from the chosen ones. One
//! script (the decomposition call and the sum of the included columns) is the
//! preview, the export and the saved script; private probe lines after it
//! compute energies, centre frequencies and spectra. Rust only validates
//! inputs, decimates for drawing and draws.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

/// Longest signal the app accepts.
pub const MAX_SAMPLES: usize = 1 << 20;
/// Bounds of the method settings (components per decomposition stay small).
pub const MAX_LEVEL: usize = 20;
pub const MAX_IMFS: usize = 32;
pub const MAX_MODES: usize = 16;
pub const MAX_PEAKS: usize = 32;
/// Drawn points per line: longer lines are drawn as min/max per bucket.
pub const BUCKETS: usize = 1024;

/// Orthogonal wavelets offered for the MODWT.
pub const WAVELETS: [&str; 12] = [
    "haar", "db2", "db4", "db6", "db8", "sym2", "sym4", "sym6", "sym8", "coif1", "coif3", "coif5",
];

/// Functions the script and its probe call: output names must not shadow them.
const CALLED: [&str; 19] = [
    "modwt", "modwtmra", "emd", "vmd", "ewt", "sum", "setdiff", "size", "abs", "fft", "floor",
    "max", "numel", "isempty", "zeros", "true", "false", "clear", "rng",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Modwt,
    Emd,
    Vmd,
    Ewt,
}
impl Method {
    pub const ALL: [Self; 4] = [Self::Modwt, Self::Emd, Self::Vmd, Self::Ewt];
    pub fn label(self) -> &'static str {
        match self {
            Self::Modwt => "MODWT MRA",
            Self::Emd => "EMD",
            Self::Vmd => "VMD",
            Self::Ewt => "EWT",
        }
    }
    /// Name of component `k` (0-based) of `count`.
    pub fn component(self, k: usize, count: usize) -> String {
        let last = k + 1 == count;
        match self {
            Self::Modwt if last => format!("S{}", count - 1),
            Self::Modwt => format!("D{}", k + 1),
            Self::Emd | Self::Vmd if last => "Residual".into(),
            Self::Emd | Self::Vmd => format!("IMF {}", k + 1),
            Self::Ewt => format!("Mode {}", k + 1),
        }
    }
}

/// What the last good run produced (the view keeps it on errors).
#[derive(Clone, Debug)]
pub struct Decomposition {
    pub method: Method,
    pub fs: f64,
    pub samples: usize,
    pub labels: Vec<String>,
    /// N-by-K component matrix and N-by-1 reconstruction (shared storage;
    /// the view draws their decimated lines, tests read them whole).
    #[cfg_attr(not(test), allow(dead_code))]
    pub components: Array,
    #[cfg_attr(not(test), allow(dead_code))]
    pub reconstruction: Array,
    pub energy: Vec<f64>,
    pub centre: Vec<f64>,
    /// Max abs of the sum of all components minus the signal.
    pub error: f64,
    /// Drawn lines: signal, reconstruction, one per component, spectra (dB).
    pub signal_line: Vec<[f64; 2]>,
    pub reconstruction_line: Vec<[f64; 2]>,
    pub component_lines: Vec<Vec<[f64; 2]>>,
    pub spectrum_lines: Vec<Vec<[f64; 2]>>,
}

pub struct MultiresAnalyzer {
    pub source: String,
    pub fs: String,
    pub method: Method,
    pub wavelet: &'static str,
    pub level: String,
    pub max_imfs: String,
    pub sift_tolerance: String,
    pub modes: String,
    pub penalty: String,
    pub max_peaks: String,
    /// Excluded components, 1-based (indices past the last one are ignored).
    pub excluded: BTreeSet<usize>,
    pub spectra: bool,
    pub components_name: String,
    pub reconstruction_name: String,
    pub current: Option<Decomposition>,
    /// The host's runner state, once per frame (shows Stop).
    pub running: bool,
    stopped: Option<u64>,
}
impl Default for MultiresAnalyzer {
    fn default() -> Self {
        Self {
            source: String::new(),
            fs: "1".into(),
            method: Method::Modwt,
            wavelet: "sym4",
            level: "4".into(),
            max_imfs: "10".into(),
            sift_tolerance: "0.2".into(),
            modes: "5".into(),
            penalty: "1000".into(),
            max_peaks: "5".into(),
            excluded: BTreeSet::new(),
            spectra: false,
            components_name: "components".into(),
            reconstruction_name: "reconstruction".into(),
            current: None,
            running: false,
            stopped: None,
        }
    }
}

/// The script and the private names its probe and prelude use.
struct Plan {
    script: String,
    signal: String,
    probe: [String; 7],
    fs: f64,
}
const PROBE_PARTS: [&str; 7] = [
    "components",
    "reconstruction",
    "energy",
    "centre",
    "frequency",
    "spectrum",
    "error",
];

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
fn count(text: &str, what: &str, max: usize) -> Result<usize, String> {
    let v = scalar(text, what)?;
    if v < 1. || v > max as f64 || v.fract() != 0. {
        return Err(format!("{what} must be an integer from 1 to {max}"));
    }
    Ok(v as usize)
}
fn positive(text: &str, what: &str) -> Result<f64, String> {
    let v = scalar(text, what)?;
    if v <= 0. {
        return Err(format!("{what} must be positive"));
    }
    Ok(v)
}
/// A finite real numeric vector of 2 to MAX_SAMPLES samples.
pub fn signal_vector(a: &Array) -> bool {
    matches!(a, Array::Numeric(m) if !a.is_complex() && a.len() >= 2
        && a.len() <= MAX_SAMPLES && a.dims().iter().filter(|d| **d > 1).count() <= 1
        && m.data().iter().all(|v| v.is_finite()))
}
fn fail<T>(text: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("Signal Multiresolution Analyzer: {text}"))
}

impl MultiresAnalyzer {
    pub fn set_method(&mut self, method: Method) {
        if method != self.method {
            self.method = method;
            self.excluded.clear();
        }
    }
    /// Include or exclude component `k` (0-based).
    pub fn set_included(&mut self, k: usize, include: bool) {
        if include {
            self.excluded.remove(&(k + 1));
        } else {
            self.excluded.insert(k + 1);
        }
    }
    pub fn included(&self, k: usize) -> bool {
        !self.excluded.contains(&(k + 1))
    }
    /// Everything the script depends on, for Stop.
    fn settings_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            &self.source,
            &self.fs,
            self.method,
            self.wavelet,
            &self.level,
        )
            .hash(&mut h);
        (
            &self.max_imfs,
            &self.sift_tolerance,
            &self.modes,
            &self.penalty,
        )
            .hash(&mut h);
        (&self.max_peaks, &self.excluded).hash(&mut h);
        (&self.components_name, &self.reconstruction_name).hash(&mut h);
        h.finish()
    }
    /// Stop: cancels the run; none starts until an input changes or Run.
    pub fn stop(&mut self) {
        self.stopped = Some(self.settings_key());
    }
    pub fn run(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped == Some(self.settings_key())
    }
    /// The decomposition call's arguments after the signal, checked.
    fn arguments(&self, n: usize) -> Result<String, String> {
        Ok(match self.method {
            Method::Modwt => {
                let top = (usize::BITS - 1 - n.leading_zeros()) as usize;
                let max = top.min(MAX_LEVEL);
                let level = count(&self.level, "Level", max).map_err(|e| {
                    format!("{e} (floor(log2(N)) = {top} for this signal of {n} samples)")
                })?;
                format!("'{}', {level}", self.wavelet)
            }
            Method::Emd => {
                let k = count(&self.max_imfs, "Max IMFs", MAX_IMFS)?;
                let tol = positive(&self.sift_tolerance, "Sifting stop")?;
                if tol > 1. {
                    return Err("Sifting stop must be at most 1".into());
                }
                format!("'MaxNumIMF', {k}, 'SiftRelativeTolerance', {}", number(tol))
            }
            Method::Vmd => {
                let k = count(&self.modes, "Number of modes", MAX_MODES)?;
                let alpha = positive(&self.penalty, "Penalty")?;
                if alpha > 1e8 {
                    return Err("Penalty must be at most 1e8".into());
                }
                format!("'NumIMFs', {k}, 'PenaltyFactor', {}", number(alpha))
            }
            Method::Ewt => {
                let k = count(&self.max_peaks, "Max peaks", MAX_PEAKS)?;
                format!("'MaxNumPeaks', {k}")
            }
        })
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.is_stopped() {
            return fail("stopped. Change an input or press Run.");
        }
        if self.source.is_empty() {
            return fail("choose a signal vector in INPUTS");
        }
        super::single_expression(&self.source).map_err(|e| format!("Signal: {e}"))?;
        if self.source.len() > 63 || !matrust::files::valid_name(&self.source) {
            return fail("choose a workspace vector as the signal");
        }
        let Some(x) = ws.get(&self.source) else {
            return fail(format!("{} no longer exists", self.source));
        };
        if !signal_vector(x) {
            return fail(format!(
                "{} must be a finite real numeric vector of 2 to {MAX_SAMPLES} samples",
                self.source
            ));
        }
        if CALLED.contains(&self.source.as_str()) {
            return fail(format!(
                "a variable named {} hides the function of the same name; rename it",
                self.source
            ));
        }
        let n = x.len();
        let fs = positive(&self.fs, "Sample rate").map_err(|e| format!("Signal: {e}"))?;
        let args = self
            .arguments(n)
            .map_err(|e| format!("{}: {e}", self.method.label()))?;
        let mut names = BTreeSet::new();
        for name in [&self.components_name, &self.reconstruction_name] {
            super::single_expression(name)?;
            if name.len() > 63 || !matrust::files::valid_name(name) {
                return fail("use valid variable names (at most 63 characters)");
            }
            super::check_export_name(name)?;
            if CALLED.contains(&name.as_str()) {
                return fail(format!(
                    "{name} is a function the script calls; choose another name"
                ));
            }
            if !names.insert(name.clone()) {
                return fail(format!("duplicate export name {name}"));
            }
        }
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || names.contains(n));
            names.insert(name.clone());
            name
        };
        let source = &self.source;
        let c = &self.components_name;
        let r = &self.reconstruction_name;
        let mut script = format!(
            "% Signal Multiresolution Analyzer: {} of {source} ({n} samples, fs = {} Hz).\n",
            self.method.label(),
            number(fs)
        );
        match self.method {
            Method::Modwt => {
                let wavelet = self.wavelet;
                script.push_str(
                    "% Columns of the component matrix: details D1..DL, then the smooth SL.\n",
                );
                script.push_str(&format!(
                    "{c} = modwtmra(modwt({source}(:), {args}), '{wavelet}').';\n"
                ));
            }
            Method::Emd | Method::Vmd => {
                let residual = fresh("mra_residual");
                let call = if self.method == Method::Emd {
                    "emd"
                } else {
                    "vmd"
                };
                script
                    .push_str("% Columns of the component matrix: the IMFs, then the residual.\n");
                if self.method == Method::Vmd {
                    // Default peak initialization draws the missing centres from
                    // the engine RNG. The preview engine starts at seed 0; the
                    // script sets that same seed so Export matches the preview.
                    script.push_str("rng(0,'twister');\n");
                }
                script.push_str(&format!(
                    "[{c}, {residual}] = {call}({source}(:), {args});\n{c} = [{c} {residual}];\nclear {residual}\n"
                ));
            }
            Method::Ewt => {
                script.push_str(
                    "% Columns of the component matrix: the modes, highest frequency first.\n",
                );
                script.push_str(&format!("{c} = ewt({source}(:), {args});\n"));
            }
        }
        if self.excluded.is_empty() {
            script.push_str(&format!("{r} = sum({c}, 2);\n"));
        } else {
            let list: Vec<String> = self.excluded.iter().map(|k| k.to_string()).collect();
            script.push_str(&format!(
                "% Sum of the included components (columns {} left out).\n{r} = sum({c}(:, setdiff(1:size({c}, 2), [{}])), 2);\n",
                list.join(", "),
                list.join(" ")
            ));
        }
        let signal = fresh("mra_probe_signal");
        let probe = PROBE_PARTS.map(|part| fresh(&format!("mra_probe_{part}")));
        Ok(Plan {
            script,
            signal,
            probe,
            fs,
        })
    }
}

/// `y` against sample times `k / fs`; long lines become min/max per bucket.
pub fn decimate(y: &[f64], step: f64) -> Vec<[f64; 2]> {
    let n = y.len();
    if n <= 2 * BUCKETS {
        return y
            .iter()
            .enumerate()
            .map(|(k, v)| [k as f64 * step, *v])
            .collect();
    }
    let mut out = Vec::with_capacity(2 * BUCKETS);
    for b in 0..BUCKETS {
        let (from, to) = (b * n / BUCKETS, (b + 1) * n / BUCKETS);
        let (mut lo, mut hi) = (from, from);
        for k in from..to {
            if y[k] < y[lo] {
                lo = k;
            }
            if y[k] > y[hi] {
                hi = k;
            }
        }
        let (a, b) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        out.push([a as f64 * step, y[a]]);
        if b != a {
            out.push([b as f64 * step, y[b]]);
        }
    }
    out
}

fn numeric(outputs: &Outputs, name: &str) -> Result<(Array, Vec<usize>), String> {
    match outputs.get(name) {
        Some(a @ Array::Numeric(_)) if !a.is_complex() => Ok((a.clone(), a.dims().to_vec())),
        _ => fail(format!("missing numeric result {name}")),
    }
}
fn data(a: &Array) -> &[f64] {
    match a {
        Array::Numeric(m) => m.data(),
        _ => &[],
    }
}

impl App for MultiresAnalyzer {
    fn kind(&self) -> AppKind {
        AppKind::MultiresAnalyzer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        if self.source.is_empty() {
            Vec::new()
        } else {
            vec![self.source.clone()]
        }
    }
    fn prelude(&self, ws: &Snapshot) -> String {
        match self.plan(ws) {
            Ok(plan) => format!("{} = {}(:);\n", plan.signal, self.source),
            Err(_) => String::new(),
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let [c, r, e, fc, f, s, err] = &plan.probe;
        let x = &plan.signal;
        let fs = number(plan.fs);
        let code = format!(
            "{c} = {};\n{r} = {};\n\
             {e} = sum({c}.^2, 1); {e} = 100*{e}/sum({e});\n\
             {f} = (0:floor(size({c}, 1)/2))'*{fs}/size({c}, 1);\n\
             {s} = abs(fft({c})); {s} = {s}(1:numel({f}), :);\n\
             {fc} = ({f}'*{s}.^2)./sum({s}.^2, 1);\n\
             {err} = max(abs(sum({c}, 2) - {x}));\n",
            self.components_name, self.reconstruction_name
        );
        let mut outputs = vec![plan.signal.clone()];
        outputs.extend(plan.probe.iter().cloned());
        (code, outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let [c, r, e, fc, f, s, err] = PROBE_PARTS.map(|part| {
            outputs
                .keys()
                .find(|k| {
                    k.strip_prefix(&format!("mra_probe_{part}"))
                        .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
                })
                .cloned()
                .unwrap_or_default()
        });
        let (components, dims) = numeric(outputs, &c)?;
        let (reconstruction, _) = numeric(outputs, &r)?;
        let (energy, _) = numeric(outputs, &e)?;
        let (centre, _) = numeric(outputs, &fc)?;
        let (frequency, _) = numeric(outputs, &f)?;
        let (spectrum, _) = numeric(outputs, &s)?;
        let (error, _) = numeric(outputs, &err)?;
        let (n, k) = (dims[0], dims.get(1).copied().unwrap_or(1));
        if dims.len() != 2
            || k == 0
            || reconstruction.len() != n
            || energy.len() != k
            || centre.len() != k
            || spectrum.len() != frequency.len() * k
        {
            return fail("the decomposition returned no components");
        }
        let fs = positive(&self.fs, "Sample rate")?;
        let step = 1. / fs;
        let cd = data(&components);
        let component_lines = (0..k)
            .map(|j| decimate(&cd[j * n..(j + 1) * n], step))
            .collect();
        let rd = data(&reconstruction);
        let m = frequency.len();
        let fd = data(&frequency);
        let sd = data(&spectrum);
        let spectrum_lines = (0..k)
            .map(|j| {
                let col = &sd[j * m..(j + 1) * m];
                let peak = col.iter().fold(0f64, |a, b| a.max(*b));
                let floor = if peak > 0. { peak * 1e-8 } else { 1e-300 };
                let db: Vec<f64> = col.iter().map(|v| 20. * v.max(floor).log10()).collect();
                let df = if m > 1 { fd[1] - fd[0] } else { 0. };
                decimate(&db, df)
            })
            .collect();
        let Some(signal) = self.signal_from(outputs).filter(|x| x.len() == n) else {
            return fail("missing the signal copy");
        };
        self.current = Some(Decomposition {
            method: self.method,
            fs,
            samples: n,
            labels: (0..k).map(|j| self.method.component(j, k)).collect(),
            energy: data(&energy).to_vec(),
            centre: data(&centre).to_vec(),
            error: data(&error).first().copied().unwrap_or(f64::NAN),
            signal_line: decimate(&signal, step),
            reconstruction_line: decimate(rd, step),
            component_lines,
            spectrum_lines,
            components,
            reconstruction,
        });
        Ok(())
    }
    fn prepare(&mut self, ws: &Snapshot) {
        if self.stopped.is_some() && !self.is_stopped() {
            self.stopped = None;
        }
        if self.source.is_empty()
            && let Some((name, _)) = ws
                .variables()
                .find(|(n, a)| signal_vector(a) && !CALLED.contains(n))
        {
            self.source = name.to_owned();
        }
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Signal (workspace vector)", |ui| {
            let combo = egui::ComboBox::from_id_salt("mra_source")
                .width(ui.available_width())
                .selected_text(if self.source.is_empty() {
                    "Choose vector"
                } else {
                    &self.source
                })
                .show_ui(ui, |ui| {
                    for (name, _) in ws.variables().filter(|(_, a)| signal_vector(a)) {
                        ui.selectable_value(&mut self.source, name.to_owned(), name);
                    }
                });
            crate::focus_areas::register(
                ui,
                &combo.response,
                crate::focus_areas::Area::App(AppKind::MultiresAnalyzer),
            );
        });
        text_field(ui, "Sample rate (Hz)", &mut self.fs);
        let mut method = self.method;
        field(ui, "Method", |ui| {
            egui::ComboBox::from_id_salt("mra_method")
                .width(ui.available_width())
                .selected_text(method.label())
                .show_ui(ui, |ui| {
                    for m in Method::ALL {
                        ui.selectable_value(&mut method, m, m.label());
                    }
                });
        });
        self.set_method(method);
        match self.method {
            Method::Modwt => {
                field(ui, "Wavelet", |ui| {
                    egui::ComboBox::from_id_salt("mra_wavelet")
                        .width(ui.available_width())
                        .selected_text(self.wavelet)
                        .show_ui(ui, |ui| {
                            for w in WAVELETS {
                                ui.selectable_value(&mut self.wavelet, w, w);
                            }
                        });
                });
                text_field(ui, "Level (1 to floor(log2 N))", &mut self.level);
            }
            Method::Emd => {
                text_field(ui, "Max IMFs (1–32)", &mut self.max_imfs);
                text_field(
                    ui,
                    "Sifting stop (relative tolerance, 0–1)",
                    &mut self.sift_tolerance,
                );
            }
            Method::Vmd => {
                text_field(ui, "Number of modes (1–16)", &mut self.modes);
                text_field(ui, "Penalty (bandwidth, > 0)", &mut self.penalty);
            }
            Method::Ewt => {
                text_field(ui, "Max peaks (1–32)", &mut self.max_peaks);
            }
        }
        let n = ws.get(&self.source).map_or(0, Array::len);
        let check = if n >= 2 {
            self.arguments(n).map(|_| ())
        } else {
            Ok(())
        };
        super::expression_message(ui, check);
        crate::tokens::helper(
            ui,
            "Numeric fields accept finite literals. EMD and VMD add the residual as the last component.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "COMPONENTS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            if crate::tokens::toggle(ui, self.spectra, "Spectra").clicked() {
                self.spectra = !self.spectra;
            }
            ui.add_space(crate::tokens::S);
            if self.is_stopped() {
                crate::tokens::helper(ui, "Stopped");
                if ui.button("Run").clicked() {
                    self.run();
                }
            } else if self.running {
                crate::tokens::helper(ui, "Decomposing…");
                if ui.button("Stop").clicked() {
                    self.stop();
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(d) = self.current.take() else {
            crate::tokens::helper(
                ui,
                "Choose a signal vector in INPUTS. The status line says what is missing.",
            );
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        let width = ui.available_width();
        let side = self.spectra && width >= 480.;
        let time_width = if side {
            (width - crate::tokens::S) * 0.6
        } else {
            width
        };
        let row = 96.;
        egui::ScrollArea::vertical()
            .id_salt("mra_rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label("Signal and reconstruction");
                chart("mra_signal", time_width, row + crate::tokens::XL, true).show(ui, |plot| {
                    plot.line(
                        egui_plot::Line::new("Signal", d.signal_line.clone()).color(p.text_muted),
                    );
                    plot.line(
                        egui_plot::Line::new("Reconstruction", d.reconstruction_line.clone())
                            .color(p.accent),
                    );
                });
                for (k, label) in d.labels.iter().enumerate() {
                    ui.add_space(crate::tokens::S);
                    let mut include = self.included(k);
                    if ui
                        .checkbox(&mut include, label.as_str())
                        .on_hover_text("Include in the reconstruction")
                        .changed()
                    {
                        self.set_included(k, include);
                    }
                    let draw_time =
                        |ui: &mut egui::Ui| {
                            chart(&format!("mra_component_{k}"), time_width, row, true).show(
                                ui,
                                |plot| {
                                    plot.line(
                                        egui_plot::Line::new(
                                            label.as_str(),
                                            d.component_lines[k].clone(),
                                        )
                                        .color(if include { p.accent } else { p.text_disabled }),
                                    );
                                },
                            );
                        };
                    let draw_spectrum = |ui: &mut egui::Ui, w: f32| {
                        chart(&format!("mra_spectrum_{k}"), w, row, false).show(ui, |plot| {
                            plot.line(
                                egui_plot::Line::new(
                                    format!("{label} (dB)"),
                                    d.spectrum_lines[k].clone(),
                                )
                                .color(if include {
                                    p.accent
                                } else {
                                    p.text_disabled
                                }),
                            );
                        });
                    };
                    if side {
                        ui.horizontal(|ui| {
                            draw_time(ui);
                            draw_spectrum(ui, width - time_width - crate::tokens::S);
                        });
                    } else {
                        draw_time(ui);
                        if self.spectra {
                            ui.add_space(crate::tokens::XS);
                            draw_spectrum(ui, width);
                        }
                    }
                }
                crate::tokens::helper(
                    ui,
                    "Time in seconds; spectra are magnitude in dB against frequency in Hz.",
                );
            });
        self.current = Some(d);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(d) = &self.current else {
            return vec![("Components", "—".into())];
        };
        let k = d.labels.len();
        let all = (1..=k).all(|j| !self.excluded.contains(&j));
        vec![
            ("Method", d.method.label().into()),
            ("Samples", d.samples.to_string()),
            ("Sample rate (Hz)", short(d.fs)),
            (
                "Components",
                format!(
                    "{} of {k} included",
                    (0..k).filter(|j| self.included(*j)).count()
                ),
            ),
            (
                "Reconstruction error (max abs)",
                if all {
                    short(d.error)
                } else {
                    "include all components".into()
                },
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if let Some(d) = &self.current {
            let p = crate::tokens::palette(ui.ctx());
            egui::Grid::new("mra_component_table")
                .num_columns(3)
                .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                .show(ui, |ui| {
                    for head in ["Component", "Energy (%)", "Centre (Hz)"] {
                        ui.label(egui::RichText::new(head).color(p.text_muted));
                    }
                    ui.end_row();
                    for (k, label) in d.labels.iter().enumerate() {
                        ui.label(label);
                        ui.label(short(d.energy.get(k).copied().unwrap_or(f64::NAN)));
                        ui.label(short(d.centre.get(k).copied().unwrap_or(f64::NAN)));
                        ui.end_row();
                    }
                });
            crate::tokens::helper(
                ui,
                "Energy: share of the summed component energies. Centre: power-weighted mean frequency.",
            );
            ui.add_space(crate::tokens::S);
        }
        name_field(
            ui,
            "Component matrix (N-by-K)",
            &mut self.components_name,
            ws,
        );
        name_field(
            ui,
            "Reconstruction (N-by-1)",
            &mut self.reconstruction_name,
            ws,
        );
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        vec![
            self.components_name.clone(),
            self.reconstruction_name.clone(),
        ]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|d| {
            format!(
                "Signal Multiresolution Analyzer: {} · {} components · {} samples",
                d.method.label(),
                d.labels.len(),
                d.samples
            )
        })
    }
}

impl MultiresAnalyzer {
    /// The run's copy of the signal (the prelude's private name).
    fn signal_from(&self, outputs: &Outputs) -> Option<Vec<f64>> {
        outputs
            .iter()
            .find(|(k, _)| {
                k.strip_prefix("mra_probe_signal")
                    .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
            })
            .map(|(_, a)| data(a).to_vec())
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
fn chart(id: &str, width: f32, height: f32, time: bool) -> egui_plot::Plot<'static> {
    let plot = egui_plot::Plot::new(id.to_owned())
        .width(width.max(120.))
        .height(height)
        .allow_scroll(false)
        .show_background(false)
        .y_axis_min_width(crate::tokens::XL * 2.)
        .x_axis_label(if time { "Time (s)" } else { "Frequency (Hz)" });
    if time {
        plot.link_axis(egui::Id::new("mra_time_axes"), [true, false])
    } else {
        plot.link_axis(egui::Id::new("mra_frequency_axes"), [true, false])
    }
}
