//! Spectral windows. The exported script is also the private preview's source
//! of vectors, spectra and measurements; Rust only validates and draws them.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;

pub const MAX_WINDOWS: usize = 8;
pub const MAX_LENGTH: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hamming,
    Hann,
    Blackman,
    BlackmanHarris,
    Nuttall,
    FlatTop,
    Bartlett,
    Triangular,
    Rectangular,
    Kaiser,
    Chebyshev,
    Gaussian,
    Tukey,
    Taylor,
    BartlettHann,
    Bohman,
    Parzen,
    User,
}
impl Kind {
    pub const ALL: [Self; 18] = [
        Self::Hamming,
        Self::Hann,
        Self::Blackman,
        Self::BlackmanHarris,
        Self::Nuttall,
        Self::FlatTop,
        Self::Bartlett,
        Self::Triangular,
        Self::Rectangular,
        Self::Kaiser,
        Self::Chebyshev,
        Self::Gaussian,
        Self::Tukey,
        Self::Taylor,
        Self::BartlettHann,
        Self::Bohman,
        Self::Parzen,
        Self::User,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Hamming => "hamming",
            Self::Hann => "hann",
            Self::Blackman => "blackman",
            Self::BlackmanHarris => "blackmanharris",
            Self::Nuttall => "nuttallwin",
            Self::FlatTop => "flattopwin",
            Self::Bartlett => "bartlett",
            Self::Triangular => "triang",
            Self::Rectangular => "rectwin",
            Self::Kaiser => "kaiser",
            Self::Chebyshev => "chebwin",
            Self::Gaussian => "gausswin",
            Self::Tukey => "tukeywin",
            Self::Taylor => "taylorwin",
            Self::BartlettHann => "barthannwin",
            Self::Bohman => "bohmanwin",
            Self::Parzen => "parzenwin",
            Self::User => "Workspace vector",
        }
    }
    pub fn periodic(self) -> bool {
        matches!(
            self,
            Self::Hamming
                | Self::Hann
                | Self::Blackman
                | Self::BlackmanHarris
                | Self::Nuttall
                | Self::FlatTop
        )
    }
    fn parameter(self) -> Option<&'static str> {
        match self {
            Self::Kaiser => Some("Beta (0–100)"),
            Self::Chebyshev => Some("Sidelobe attenuation dB (0–300, exclusive 0)"),
            Self::Gaussian => Some("Alpha (0–100, exclusive 0)"),
            Self::Tukey => Some("Taper ratio r (0–1)"),
            Self::Taylor => Some("nbar (integer 1–64)"),
            _ => None,
        }
    }
    fn default_parameter(self) -> &'static str {
        match self {
            Self::Kaiser => "8",
            Self::Chebyshev => "80",
            Self::Gaussian => "2.5",
            Self::Tukey => "0.5",
            Self::Taylor => "4",
            _ => "",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Window {
    pub id: u32,
    pub name: String,
    pub shown: bool,
    pub color: usize,
    pub kind: Kind,
    pub length: String,
    pub periodic: bool,
    pub parameter: String,
    pub sidelobe: String,
    pub source: String,
}
impl Window {
    fn new(id: u32, kind: Kind, name: String) -> Self {
        Self {
            id,
            name,
            shown: true,
            color: id as usize - 1,
            kind,
            length: "64".into(),
            periodic: false,
            parameter: kind.default_parameter().into(),
            sidelobe: "-30".into(),
            source: String::new(),
        }
    }
    pub fn set_kind(&mut self, kind: Kind) {
        self.kind = kind;
        self.parameter = kind.default_parameter().into();
    }
    fn expression(&self, ws: &Snapshot) -> Result<String, String> {
        if self.kind == Kind::User {
            super::single_expression(&self.source)?;
            if self.source.len() > 63 || !matrust::files::valid_name(&self.source) {
                return Err("Choose a workspace vector".into());
            }
            let a = ws
                .get(&self.source)
                .ok_or("Workspace vector no longer exists")?;
            if !user_vector(a) {
                return Err(format!(
                    "User window must be a finite real numeric vector of 1–{MAX_LENGTH} samples"
                ));
            }
            return Ok(format!("{}(:)", self.source));
        }
        let n = scalar(&self.length, "Length N")?;
        if !(1. ..=MAX_LENGTH as f64).contains(&n) || n.fract() != 0. {
            return Err(format!(
                "Length N must be an integer from 1 to {MAX_LENGTH}"
            ));
        }
        let mut args = number(n);
        if let Some(label) = self.kind.parameter() {
            let p = scalar(&self.parameter, label)?;
            let valid = match self.kind {
                Kind::Kaiser => (0. ..=100.).contains(&p),
                Kind::Chebyshev => p > 0. && p <= 300.,
                Kind::Gaussian => p > 0. && p <= 100.,
                Kind::Tukey => (0. ..=1.).contains(&p),
                Kind::Taylor => (1. ..=64.).contains(&p) && p.fract() == 0.,
                _ => true,
            };
            if !valid {
                return Err(format!("{label} is outside its allowed range"));
            }
            args.push_str(&format!(", {}", number(p)));
            if self.kind == Kind::Taylor {
                let s = scalar(&self.sidelobe, "Taylor sidelobe level")?;
                if !(-300. ..0.).contains(&s) {
                    return Err(
                        "Taylor sidelobe level must be negative and at least -300 dB".into(),
                    );
                }
                args.push_str(&format!(", {}", number(s)));
            }
        }
        if self.kind.periodic() {
            args.push_str(if self.periodic {
                ", 'periodic'"
            } else {
                ", 'symmetric'"
            });
        }
        Ok(format!("{}({args})", self.kind.label()))
    }
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
fn user_vector(a: &Array) -> bool {
    matches!(a, Array::Numeric(m) if !a.is_complex() && !a.is_empty()
        && a.len() <= MAX_LENGTH && a.dims().iter().filter(|d| **d > 1).count() <= 1
        && m.data().iter().all(|v| v.is_finite()))
}

#[derive(Clone, Debug)]
pub struct Curve {
    pub id: u32,
    pub name: String,
    pub color: usize,
    pub samples: Vec<f64>,
    pub frequency: Vec<f64>,
    pub db: Vec<f64>,
    /// Leakage %, positive attenuation dB, full -3 dB width relative to Nyquist,
    /// ENBW bins, coherent gain. Unresolved measurements are NaN.
    pub measurements: Vec<f64>,
}
pub struct WindowDesigner {
    pub windows: Vec<Window>,
    pub selected: usize,
    pub current: Vec<Curve>,
    next_id: u32,
}
impl Default for WindowDesigner {
    fn default() -> Self {
        Self {
            windows: vec![Window::new(1, Kind::Hamming, "window1".into())],
            selected: 0,
            current: Vec::new(),
            next_id: 2,
        }
    }
}
struct Plan {
    script: String,
    /// Read-only private probes refer to these script variables.
    records: Vec<(u32, String, usize, [String; 4])>,
}
impl WindowDesigner {
    pub fn add(&mut self, kind: Kind, ws: &Snapshot) -> Result<(), String> {
        if self.windows.len() >= MAX_WINDOWS {
            return Err(format!("At most {MAX_WINDOWS} windows"));
        }
        let name = unused_name("window", |n| {
            ws.contains(n) || self.windows.iter().any(|w| w.name == n)
        });
        self.windows.push(Window::new(self.next_id, kind, name));
        self.next_id += 1;
        self.selected = self.windows.len() - 1;
        Ok(())
    }
    pub fn duplicate(&mut self, ws: &Snapshot) -> Result<(), String> {
        let Some(original) = self.windows.get(self.selected).cloned() else {
            return Ok(());
        };
        self.add(original.kind, ws)?;
        let new = &mut self.windows[self.selected];
        let (id, name, color) = (new.id, new.name.clone(), new.color);
        *new = Window {
            id,
            name,
            color,
            ..original
        };
        Ok(())
    }
    pub fn remove(&mut self) {
        if self.selected < self.windows.len() {
            self.windows.remove(self.selected);
        }
        self.selected = self.selected.min(self.windows.len().saturating_sub(1));
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.windows.len() > MAX_WINDOWS {
            return Err(format!("At most {MAX_WINDOWS} windows"));
        }
        let shown: Vec<_> = self.windows.iter().filter(|w| w.shown).collect();
        if shown.is_empty() {
            return Err("Window Designer: add a window and tick Show".into());
        }
        let mut names = BTreeSet::new();
        for w in &shown {
            super::single_expression(&w.name)?;
            if w.name.len() > 63 || !matrust::files::valid_name(&w.name) {
                return Err(
                    "Window Designer: use valid variable names (at most 63 characters)".into(),
                );
            }
            super::check_export_name(&w.name)?;
            if !names.insert(w.name.clone()) {
                return Err(format!("Window Designer: duplicate export name {}", w.name));
            }
        }
        // Every temporary and the local function avoid existing workspace names
        // and output names. Stage all sources before assigning any exports.
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || names.contains(n));
            names.insert(name.clone());
            name
        };
        let helper = fresh("wd_measure");
        let mut script = String::from(
            "% Window Designer: frequency is relative to Nyquist; width spans both sides.\n% Mainlobe ends at the nearest spectral minima around its peak.\n% Attenuation is positive dB below the mainlobe; NaN means unresolved.\n",
        );
        let mut records = Vec::new();
        for w in &shown {
            let vars = [
                fresh("wd_samples"),
                fresh("wd_frequency"),
                fresh("wd_db"),
                fresh("wd_metrics"),
            ];
            let expression = w
                .expression(ws)
                .map_err(|e| format!("Window Designer: {}: {e}", w.name))?;
            script.push_str(&format!(
                "% {}: metrics = [leakage_percent attenuation_dB width_over_Nyquist ENBW_bins coherent_gain]\n",
                w.name
            ));
            if w.kind == Kind::User {
                let source = &w.source;
                script.push_str(&format!("if ~isnumeric({source}) || ~isreal({source}) || ~isvector({source}) || isempty({source}) || numel({source}) > {MAX_LENGTH}, error('Window Designer: user window must be a finite real numeric vector of 1 to {MAX_LENGTH} samples'); end\n"));
            }
            script.push_str(&format!(
                "{} = {expression};\n[{}, {}, {}] = {helper}({});\n",
                vars[0], vars[1], vars[2], vars[3], vars[0]
            ));
            records.push((w.id, w.name.clone(), w.color, vars));
        }
        script.push_str("figure;\nsubplot(1,2,1);\nhold on;\n");
        for (_, name, color, vars) in &records {
            script.push_str(&format!(
                "plot((0:numel({})-1)', {}, 'Color', {}, 'DisplayName', '{name}');\n",
                vars[0],
                vars[0],
                rgb(*color)
            ));
        }
        script.push_str("hold off; xlabel('Samples'); ylabel('Amplitude'); title('Time domain'); legend('show');\nsubplot(1,2,2);\nhold on;\n");
        for (_, name, color, vars) in &records {
            script.push_str(&format!(
                "plot({}, {}, 'Color', {}, 'DisplayName', '{name}');\n",
                vars[1],
                vars[2],
                rgb(*color)
            ));
        }
        script.push_str("hold off; xlabel('Frequency / Nyquist'); ylabel('Magnitude (dB)'); title('Frequency domain'); legend('show');\n");
        // Assign after all function calls, so names such as `plot` are safe.
        for (_, name, _, vars) in &records {
            script.push_str(&format!("{name} = {}(:);\n", vars[0]));
        }
        script.push_str(&MEASURE.replace("WD_MEASURE", &helper));
        Ok(Plan { script, records })
    }
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

// FFT sampling: >= 16384, >= 32*N (max 131072). First local minima
// delimit the mainlobe, including the flat-top's off-DC peak. Trapezoidal
// power weights account for DC and Nyquist. Crossing positions interpolate
// amplitude at exactly -3 dB; the two sides give the full width.
const MEASURE: &str = "\nfunction [f, db, metrics] = WD_MEASURE(w)\n\
w = w(:); n = numel(w);\n\
if n < 1 || n > 4096 || ~isreal(w), error('Window Designer: expected 1 to 4096 real samples'); end\n\
if any(~isfinite(w)), error('Window Designer: window contains NaN or Inf; check Taylor nbar and sidelobe level'); end\n\
gain = sum(w)/n; energy = sum(w.^2);\n\
enbw_bins = NaN;\n\
if sum(w) ~= 0, enbw_bins = n*energy/sum(w)^2; elseif energy > 0, enbw_bins = Inf; end\n\
nfft = max(16384, 2^nextpow2(32*n));\n\
f = 2*(0:nfft/2)'/nfft;\n\
a = abs(fft(w, nfft)); a = a(1:nfft/2+1);\n\
[peak, at] = max(a); db = -150*ones(size(a));\n\
leakage = NaN; attenuation = NaN; width = NaN;\n\
if peak > 0\n\
    a = a/peak; db = 20*log10(max(a, 10^(-150/20)));\n\
    minima = find(a(2:end-1) <= a(1:end-2) & a(2:end-1) < a(3:end)) + 1;\n\
    if max(a)-min(a) <= 1e-12, minima = []; end\n\
    left = 1; right = numel(a);\n\
    before = minima(minima < at); after = minima(minima > at);\n\
    if ~isempty(before), left = before(end); end\n\
    if ~isempty(after), right = after(1); end\n\
    power = a.^2; power(1) = power(1)/2; power(end) = power(end)/2;\n\
    leakage = 100*(sum(power(1:left-1)) + sum(power(right+1:end)))/sum(power);\n\
    side = [a(1:left-1); a(right+1:end)];\n\
    if ~isempty(side) && max(side) > 0, attenuation = -20*log10(max(side)); end\n\
    level = 10^(-3/20);\n\
    below = find(a(at:right) <= level);\n\
    if ~isempty(below)\n\
        k = at + below(1)-1;\n\
        hi = (k-2 + (a(k-1)-level)/(a(k-1)-a(k)))/nfft;\n\
        below_left = find(a(left:at) <= level);\n\
        if ~isempty(below_left)\n\
            k = left + below_left(end)-1;\n\
            lo = (k-1 + (level-a(k))/(a(k+1)-a(k)))/nfft;\n\
            width = 2*(hi-lo);\n\
        elseif left == 1\n\
            width = 4*hi;\n\
        end\n\
    elseif right == numel(a)\n\
        below_left = find(a(left:at) <= level);\n\
        if ~isempty(below_left)\n\
            k = left + below_left(end)-1;\n\
            lo = (k-1 + (level-a(k))/(a(k+1)-a(k)))/nfft;\n\
            width = 4*(0.5-lo);\n\
        end\n\
    end\n\
end\n\
metrics = [leakage attenuation width enbw_bins gain];\n\
end\n";

impl App for WindowDesigner {
    fn kind(&self) -> AppKind {
        AppKind::WindowDesigner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        self.windows
            .iter()
            .filter(|w| w.shown && w.kind == Kind::User)
            .map(|w| w.source.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let mut code = String::new();
        let mut outputs = Vec::new();
        for (id, _, _, vars) in plan.records {
            for (part, source) in ["samples", "frequency", "db", "metrics"].iter().zip(vars) {
                let name = format!("wd_probe_{id}_{part}");
                code.push_str(&format!("{name} = {source};\n"));
                outputs.push(name);
            }
        }
        (code, outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let read = |name: &str| -> Result<Vec<f64>, String> {
            match outputs.get(name) {
                Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
                _ => Err(format!("Window Designer: missing numeric result {name}")),
            }
        };
        let mut next = Vec::new();
        for w in self.windows.iter().filter(|w| w.shown) {
            let value = |part: &str| read(&format!("wd_probe_{}_{part}", w.id));
            let samples = value("samples")?;
            if samples.iter().any(|v| !v.is_finite()) {
                return Err(
                    "Window Designer: window contains NaN or Inf; check Taylor nbar".into(),
                );
            }
            let frequency = value("frequency")?;
            let db = value("db")?;
            let measurements = value("metrics")?;
            if frequency.len() != db.len() || measurements.len() != 5 {
                return Err("Window Designer: incomplete measurements".into());
            }
            next.push(Curve {
                id: w.id,
                name: w.name.clone(),
                color: w.color,
                samples,
                frequency,
                db,
                measurements,
            });
        }
        self.current = next;
        Ok(())
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        self.selected = self.selected.min(self.windows.len().saturating_sub(1));
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal(|ui| {
            let add = ui.add_enabled(self.windows.len() < MAX_WINDOWS, egui::Button::new("Add"));
            crate::focus_areas::register(
                ui,
                &add,
                crate::focus_areas::Area::App(AppKind::WindowDesigner),
            );
            if add.clicked() {
                let _ = self.add(Kind::Hamming, ws);
            }
            if ui
                .add_enabled(
                    !self.windows.is_empty() && self.windows.len() < MAX_WINDOWS,
                    egui::Button::new("Duplicate"),
                )
                .clicked()
            {
                let _ = self.duplicate(ws);
            }
            if ui
                .add_enabled(!self.windows.is_empty(), egui::Button::new("Remove"))
                .clicked()
            {
                self.remove();
            }
        });
        ui.add_space(crate::tokens::S);
        for (k, w) in self.windows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut w.shown, "").on_hover_text("Show window");
                let response = crate::tokens::color_swatch(
                    ui,
                    crate::tokens::M,
                    colour(w.color),
                    egui::Sense::click(),
                );
                if response.on_hover_text("Change series colour").clicked() {
                    w.color = (w.color + 1) % matrust::plot::COLORS.len();
                }
                if ui.selectable_label(self.selected == k, &w.name).clicked() {
                    self.selected = k;
                }
            });
        }
        let Some(w) = self.windows.get_mut(self.selected) else {
            return;
        };
        ui.add_space(crate::tokens::S);
        name_field(ui, "Window name", &mut w.name, ws);
        let mut kind = w.kind;
        field(ui, "Type", |ui| {
            egui::ComboBox::from_id_salt("window_kind")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for v in Kind::ALL {
                        ui.selectable_value(&mut kind, v, v.label());
                    }
                });
        });
        if kind != w.kind {
            w.set_kind(kind);
        }
        if w.kind == Kind::User {
            field(ui, "Workspace vector", |ui| {
                egui::ComboBox::from_id_salt("window_source")
                    .selected_text(if w.source.is_empty() {
                        "Choose vector"
                    } else {
                        &w.source
                    })
                    .show_ui(ui, |ui| {
                        for (name, _) in ws.variables().filter(|(_, a)| user_vector(a)) {
                            ui.selectable_value(&mut w.source, name.to_owned(), name);
                        }
                    });
            });
        } else {
            text_field(ui, "Length N (1–4096)", &mut w.length);
            if w.kind.periodic() {
                field(ui, "Sampling", |ui| {
                    egui::ComboBox::from_id_salt("window_sampling")
                        .selected_text(if w.periodic { "periodic" } else { "symmetric" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut w.periodic, false, "symmetric");
                            ui.selectable_value(&mut w.periodic, true, "periodic");
                        });
                });
            }
            if let Some(label) = w.kind.parameter() {
                text_field(ui, label, &mut w.parameter);
            }
            if w.kind == Kind::Taylor {
                text_field(ui, "Sidelobe level dB (-300 to <0)", &mut w.sidelobe);
            }
        }
        super::expression_message(ui, w.expression(ws).map(|_| ()));
        crate::tokens::helper(
            ui,
            "Numeric fields accept finite literals. Up to 8 windows, 4096 samples each.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "WINDOWS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        if self.current.is_empty() {
            crate::tokens::helper(ui, "Add a window to compare its samples and spectrum.");
            return;
        }
        let width = ui.available_width();
        let height = ui.available_height().max(320.);
        let gap = crate::tokens::S;
        if width >= 560. {
            ui.horizontal(|ui| {
                self.chart(ui, false, (width - gap) / 2., height);
                self.chart(ui, true, (width - gap) / 2., height);
            });
        } else {
            self.chart(ui, false, width, (height - gap) / 2.);
            ui.add_space(gap);
            self.chart(ui, true, width, (height - gap) / 2.);
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(w) = self.windows.get(self.selected) else {
            return Vec::new();
        };
        let Some(c) = self.current.iter().find(|c| c.id == w.id) else {
            return vec![
                ("Selected window", w.name.clone()),
                ("Measurements", "Show this window to measure it".into()),
            ];
        };
        let v = &c.measurements;
        vec![
            ("Selected window", c.name.clone()),
            ("Leakage (%)", measurement(v[0])),
            ("Relative sidelobe attenuation (dB)", measurement(v[1])),
            ("Mainlobe width (-3 dB)", measurement(v[2])),
            ("ENBW (bins)", measurement(v[3])),
            ("Coherent gain", measurement(v[4])),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        crate::tokens::helper(
            ui,
            "Frequency and full width are relative to Nyquist (1 = pi rad/sample). Attenuation is positive dB below the peak. Mainlobe boundaries are the nearest spectral minima. n/a: no resolved sidelobe or crossing (including zero windows).",
        );
        ui.add_space(crate::tokens::S);
        for w in self.windows.iter_mut().filter(|w| w.shown) {
            name_field(ui, "Export column vector", &mut w.name, ws);
        }
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        self.windows
            .iter()
            .filter(|w| w.shown)
            .map(|w| w.name.clone())
            .collect()
    }
    fn summary(&self) -> Option<String> {
        (!self.current.is_empty()).then(|| {
            format!(
                "Window Designer: {} windows · FFT floor -150 dB",
                self.current.len()
            )
        })
    }
}
fn measurement(v: f64) -> String {
    if v.is_nan() { "n/a".into() } else { short(v) }
}
fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(value).char_limit(64))
                .desired_width(ui.available_width()),
        );
    });
}
fn colour(index: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[index % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}
impl WindowDesigner {
    fn chart(&self, ui: &mut egui::Ui, spectrum: bool, width: f32, height: f32) {
        egui_plot::Plot::new(("window_designer", spectrum))
            .width(width)
            .height(height.max(120.))
            .show_background(false)
            .allow_scroll(false)
            .legend(egui_plot::Legend::default())
            .x_axis_label(if spectrum {
                "Frequency / Nyquist"
            } else {
                "Samples"
            })
            .y_axis_label(if spectrum {
                "Magnitude (dB)"
            } else {
                "Amplitude"
            })
            .show(ui, |p| {
                for c in &self.current {
                    let points: Vec<[f64; 2]> = if spectrum {
                        c.frequency
                            .iter()
                            .zip(&c.db)
                            .map(|(x, y)| [*x, *y])
                            .collect()
                    } else {
                        c.samples
                            .iter()
                            .enumerate()
                            .map(|(k, y)| [k as f64, *y])
                            .collect()
                    };
                    p.line(egui_plot::Line::new(&c.name, points).color(colour(c.color)));
                }
            });
    }
}
