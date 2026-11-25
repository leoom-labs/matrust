//! Wavelet Time-Frequency Analyzer: the scalogram of a workspace vector with
//! cwt or wsst, and the reconstruction of a time-frequency region with icwt or
//! iwsst. One function builds the script; the private run, Export and the
//! saved script all use it, and Rust only validates fields and draws.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;

/// Fewest and most samples of the analysed signal.
pub const MIN_SAMPLES: usize = 8;
pub const MAX_SAMPLES: usize = 131_072;
/// Most image columns the view draws; longer signals are max-pooled.
pub const VIEW_COLUMNS: usize = 1024;
/// Most points of a drawn line (min/max envelope).
const LINE_POINTS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transform {
    Cwt,
    Wsst,
}
impl Transform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cwt => "Continuous (cwt)",
            Self::Wsst => "Synchrosqueezed (wsst)",
        }
    }
    fn default_voices(self) -> &'static str {
        match self {
            Self::Cwt => "10",
            Self::Wsst => "32",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wavelet {
    Morse,
    Amor,
    Bump,
}
impl Wavelet {
    pub fn label(self) -> &'static str {
        match self {
            Self::Morse => "morse",
            Self::Amor => "amor",
            Self::Bump => "bump",
        }
    }
    fn allowed(self, transform: Transform) -> bool {
        transform == Transform::Cwt || self != Self::Morse
    }
}

/// The selected time-frequency box: seconds (samples when normalised) and
/// frequency in the transform's unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub t: [f64; 2],
    pub f: [f64; 2],
}

/// Every input; a changed value reruns the transform.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Settings {
    pub signal: String,
    /// Empty: normalised frequency (cycles/sample, time in samples).
    pub rate: String,
    pub transform: Transform,
    pub wavelet: Wavelet,
    pub voices: String,
    /// CWT frequency limits; both empty is the full range.
    pub f_min: String,
    pub f_max: String,
    /// Morse symmetry and time-bandwidth product.
    pub gamma: String,
    pub time_bandwidth: String,
    /// Region as written into the script (`number` text), None: whole transform.
    pub region: Option<[String; 4]>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            signal: String::new(),
            rate: String::new(),
            transform: Transform::Cwt,
            wavelet: Wavelet::Morse,
            voices: "10".into(),
            f_min: String::new(),
            f_max: String::new(),
            gamma: "3".into(),
            time_bandwidth: "60".into(),
            region: None,
        }
    }
}

/// One output name with its Export check box.
#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub export: bool,
}
impl Output {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            export: true,
        }
    }
}

/// What the view draws: the last good run.
#[derive(Clone, Debug)]
pub struct View {
    pub settings: Settings,
    pub fs: f64,
    pub x: Vec<f64>,
    /// |coefficients| pooled to at most VIEW_COLUMNS columns, column-major.
    pub image: Vec<f64>,
    pub rows: usize,
    pub cols: usize,
    /// Row frequencies (CWT decreasing, WSST increasing).
    pub f: Vec<f64>,
    /// CWT cone of influence per sample; empty for WSST.
    pub coi: Vec<f64>,
    pub rec: Vec<f64>,
    pub ratio: f64,
    pub generation: u64,
}
impl View {
    pub fn samples(&self) -> usize {
        self.x.len()
    }
    fn time(&self, k: usize) -> f64 {
        k as f64 / self.fs
    }
    /// Lowest and highest row frequency.
    pub fn frequency_range(&self) -> [f64; 2] {
        let lo = self.f.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = self.f.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        [lo, hi]
    }
    /// Magnitude at time `t` and frequency `f` (the pooled image cell).
    pub fn magnitude(&self, t: f64, f: f64) -> Option<f64> {
        if self.rows == 0 || self.cols == 0 || f <= 0. {
            return None;
        }
        let duration = self.samples() as f64 / self.fs;
        if !(0. ..duration).contains(&t) {
            return None;
        }
        let col = ((t / duration * self.cols as f64) as usize).min(self.cols - 1);
        let target = f.log10();
        let row = (0..self.rows).min_by(|a, b| {
            let da = (self.f[*a].log10() - target).abs();
            let db = (self.f[*b].log10() - target).abs();
            da.total_cmp(&db)
        })?;
        self.image.get(col * self.rows + row).copied()
    }
}

pub struct WaveletTfAnalyzer {
    pub settings: Settings,
    pub coefficients: Output,
    pub frequencies: Output,
    pub reconstruction: Output,
    pub current: Option<View>,
    /// Stop pressed for these settings; nothing runs until an input changes.
    stopped: Option<Settings>,
    pub running: bool,
    /// A Stop click for the host to apply to its runner.
    pub stop_requested: bool,
    generation: u64,
    /// Drag start of a region box (time, log10 frequency).
    drag: Option<[f64; 2]>,
    drag_end: Option<[f64; 2]>,
    /// Hover readout: time, frequency, magnitude.
    pub hover: Option<[f64; 3]>,
    texture: Option<(u64, egui::TextureHandle)>,
}
impl Default for WaveletTfAnalyzer {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            coefficients: Output::new("wt"),
            frequencies: Output::new("f"),
            reconstruction: Output::new("xrec"),
            current: None,
            stopped: None,
            running: false,
            stop_requested: false,
            generation: 0,
            drag: None,
            drag_end: None,
            hover: None,
            texture: None,
        }
    }
}

/// Script variables the probe reads.
struct Plan {
    script: String,
    x: String,
    fs: String,
    wt: String,
    f: String,
    coi: Option<String>,
    rec: String,
    ratio: String,
    samples: usize,
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

/// A real, finite numeric vector the transform accepts.
pub fn signal_vector(a: &Array) -> bool {
    matches!(a, Array::Numeric(m) if !a.is_complex()
        && (MIN_SAMPLES..=MAX_SAMPLES).contains(&a.len())
        && a.dims().iter().filter(|d| **d > 1).count() == 1
        && m.data().iter().all(|v| v.is_finite()))
}

/// Six significant digits, so a dragged box writes short numbers.
pub fn round6(v: f64) -> f64 {
    format!("{v:.5e}").parse().unwrap_or(v)
}

impl WaveletTfAnalyzer {
    /// The sample rate, None when normalised.
    fn rate(&self) -> Result<Option<f64>, String> {
        let text = self.settings.rate.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let fs = scalar(text, "Sample rate")?;
        if !(fs > 0. && fs <= 1e12) {
            return Err("Sample rate must be positive (at most 1e12 Hz)".into());
        }
        Ok(Some(fs))
    }
    /// Sets the region box (rounded) or clears it.
    pub fn set_region(&mut self, region: Option<Region>) {
        self.settings.region = region.map(|r| {
            let [t0, t1] = [round6(r.t[0].min(r.t[1])), round6(r.t[0].max(r.t[1]))];
            let [f0, f1] = [round6(r.f[0].min(r.f[1])), round6(r.f[0].max(r.f[1]))];
            [number(t0), number(t1), number(f0), number(f1)]
        });
    }
    pub fn region(&self) -> Option<Region> {
        let [t0, t1, f0, f1] = self.settings.region.as_ref()?;
        let v = |s: &String| s.parse::<f64>().unwrap_or(f64::NAN);
        Some(Region {
            t: [v(t0), v(t1)],
            f: [v(f0), v(f1)],
        })
    }
    pub fn set_transform(&mut self, transform: Transform) {
        if transform != self.settings.transform {
            self.settings.transform = transform;
            self.settings.voices = transform.default_voices().into();
            if !self.settings.wavelet.allowed(transform) {
                self.settings.wavelet = Wavelet::Amor;
            }
            self.settings.region = None;
        }
    }
    /// Stop: cancels the run; none starts until an input changes.
    pub fn stop(&mut self) {
        self.stopped = Some(self.settings.clone());
        self.stop_requested = true;
    }
    fn outputs(&self) -> [(&Output, &'static str); 3] {
        [
            (&self.coefficients, "Coefficients"),
            (&self.frequencies, "Frequency vector"),
            (&self.reconstruction, "Reconstructed signal"),
        ]
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let s = &self.settings;
        let fail = |e: String| format!("Wavelet Time-Frequency Analyzer: {e}");
        if s.signal.is_empty() {
            return Err(fail("choose a workspace vector".into()));
        }
        super::single_expression(&s.signal).map_err(fail)?;
        if s.signal.len() > 63 || !matrust::files::valid_name(&s.signal) {
            return Err(fail("choose a workspace vector".into()));
        }
        let source = ws
            .get(&s.signal)
            .ok_or_else(|| fail(format!("workspace variable {} does not exist", s.signal)))?;
        if !signal_vector(source) {
            return Err(fail(format!(
                "the signal must be a finite real numeric vector of {MIN_SAMPLES} to {MAX_SAMPLES} samples"
            )));
        }
        let samples = source.len();
        let fs = self.rate().map_err(fail)?;
        let nyquist = fs.unwrap_or(1.) / 2.;
        let voices = scalar(&s.voices, "Voices per octave").map_err(fail)?;
        let wavelet = s.wavelet;
        if !wavelet.allowed(s.transform) {
            return Err(fail("wsst uses the amor or bump wavelet".into()));
        }
        let voices_ok = match s.transform {
            Transform::Cwt => (1. ..=48.).contains(&voices) && voices.fract() == 0.,
            Transform::Wsst => {
                (10. ..=48.).contains(&voices) && voices.fract() == 0. && voices % 2. == 0.
            }
        };
        if !voices_ok {
            return Err(fail(match s.transform {
                Transform::Cwt => "voices per octave must be an integer from 1 to 48".into(),
                Transform::Wsst => "voices per octave must be an even integer from 10 to 48".into(),
            }));
        }
        let mut options = format!(", 'VoicesPerOctave', {}", number(voices));
        if s.transform == Transform::Cwt && wavelet == Wavelet::Morse {
            let gamma = scalar(&s.gamma, "Morse gamma").map_err(fail)?;
            let tb = scalar(&s.time_bandwidth, "Time-bandwidth").map_err(fail)?;
            if !(1. ..=20.).contains(&gamma) {
                return Err(fail("Morse gamma must be from 1 to 20".into()));
            }
            if !(3. ..=120.).contains(&tb) || tb < gamma {
                return Err(fail(
                    "time-bandwidth must be from 3 to 120 and at least gamma".into(),
                ));
            }
            if gamma != 3. || tb != 60. {
                options.push_str(&format!(
                    ", 'WaveletParameters', [{} {}]",
                    number(gamma),
                    number(tb)
                ));
            }
        }
        let mut limits = String::new();
        if s.transform == Transform::Cwt
            && !(s.f_min.trim().is_empty() && s.f_max.trim().is_empty())
        {
            let lo = scalar(&s.f_min, "Lowest frequency").map_err(fail)?;
            let hi = scalar(&s.f_max, "Highest frequency").map_err(fail)?;
            if !(lo > 0. && lo < hi && hi <= nyquist) {
                return Err(fail(format!(
                    "frequency limits need 0 < low < high <= {} (Nyquist)",
                    short(nyquist)
                )));
            }
            limits = format!(", 'FrequencyLimits', [{} {}]", number(lo), number(hi));
        }
        let region = match self.region() {
            None => None,
            Some(r) => {
                if r.t.iter().chain(&r.f).any(|v| !v.is_finite())
                    || r.t[0] >= r.t[1]
                    || r.f[0] >= r.f[1]
                    || r.f[0] <= 0.
                {
                    return Err(fail(
                        "the region needs a positive time and frequency span".into(),
                    ));
                }
                Some(r)
            }
        };
        // Export names: valid, not reserved, distinct, at least one.
        let mut names = BTreeSet::new();
        for (output, what) in self.outputs() {
            if !output.export {
                continue;
            }
            super::single_expression(&output.name).map_err(fail)?;
            if output.name.len() > 63 || !matrust::files::valid_name(&output.name) {
                return Err(fail(format!("{what}: use a valid variable name")));
            }
            super::check_export_name(&output.name).map_err(fail)?;
            if !names.insert(output.name.clone()) {
                return Err(fail(format!("duplicate export name {}", output.name)));
            }
        }
        if names.is_empty() {
            return Err(fail("tick at least one output to export".into()));
        }
        // Temporaries avoid workspace and output names; exports are
        // assigned last so names such as `plot` stay usable.
        let mut taken = names.clone();
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || taken.contains(n));
            taken.insert(name.clone());
            name
        };
        let [x, fsv, wt, f, coi, t, keep, rec, ratio] = [
            "wtf_x",
            "wtf_fs",
            "wtf_wt",
            "wtf_f",
            "wtf_coi",
            "wtf_t",
            "wtf_keep",
            "wtf_rec",
            "wtf_ratio",
        ]
        .map(&mut fresh);
        let signal = &s.signal;
        let name = wavelet.label();
        let unit = if fs.is_some() { "Hz" } else { "cycles/sample" };
        let mut script = format!(
            "% Wavelet Time-Frequency Analyzer: {} of {signal}, '{name}' wavelet.\n",
            match s.transform {
                Transform::Cwt => "continuous wavelet transform",
                Transform::Wsst => "synchrosqueezed transform",
            }
        );
        script.push_str(&format!(
            "if ~isnumeric({signal}) || ~isreal({signal}) || ~isvector({signal}) || numel({signal}) < {MIN_SAMPLES} || numel({signal}) > {MAX_SAMPLES} || any(~isfinite({signal}(:))), error('Wavelet Time-Frequency Analyzer: the signal must be a finite real numeric vector of {MIN_SAMPLES} to {MAX_SAMPLES} samples'); end\n"
        ));
        script.push_str(&format!("{x} = double(reshape({signal}, 1, []));\n"));
        match fs {
            Some(v) => script.push_str(&format!("{fsv} = {};   % sample rate (Hz)\n", number(v))),
            None => script.push_str(&format!(
                "{fsv} = 1;   % normalised: time in samples, frequency in cycles/sample\n"
            )),
        }
        script.push_str(&format!("{t} = (0:numel({x})-1)/{fsv};\n"));
        let rate_arg = if fs.is_some() {
            format!(", {fsv}")
        } else {
            String::new()
        };
        match s.transform {
            Transform::Cwt => script.push_str(&format!(
                "[{wt}, {f}, {coi}] = cwt({x}, '{name}'{rate_arg}{options}{limits});\n"
            )),
            Transform::Wsst => script.push_str(&format!(
                "[{wt}, {f}] = wsst({x}, {fsv}, '{name}'{options});\n"
            )),
        }
        let inverse_options = match s.transform {
            Transform::Cwt => options.clone(),
            Transform::Wsst => String::new(),
        };
        match (region, s.transform) {
            (Some(_), transform) => {
                let [t0, t1, f0, f1] = self.settings.region.clone().unwrap();
                script.push_str(&format!(
                    "% Region: {t0} to {t1} {}, {f0} to {f1} {unit}.\n{keep} = {wt};\n{keep}(:, {t} < {t0} | {t} > {t1}) = 0;\n",
                    if fs.is_some() { "s" } else { "samples" }
                ));
                match transform {
                    Transform::Cwt => script.push_str(&format!(
                        "{rec} = icwt({keep}, '{name}', {f}, [{f0} {f1}]{inverse_options});\n"
                    )),
                    Transform::Wsst => script.push_str(&format!(
                        "{rec} = iwsst({keep}, {f}, [{f0} {f1}], '{name}');\n"
                    )),
                }
            }
            (None, Transform::Cwt) => script.push_str(&format!(
                "{rec} = icwt({wt}, '{name}'{inverse_options}, 'SignalMean', mean({x}));\n"
            )),
            (None, Transform::Wsst) => {
                script.push_str(&format!("{rec} = iwsst({wt}, '{name}');\n"))
            }
        }
        script.push_str(&format!(
            "{rec} = reshape({rec}, 1, []);\n{ratio} = sum({rec}.^2)/sum({x}.^2);   % reconstruction energy ratio\n"
        ));
        let time_label = if fs.is_some() {
            "Time (s)"
        } else {
            "Time (samples)"
        };
        script.push_str(&format!(
            "figure;\nsubplot(2,1,1);\nplot({t}, {x}, 'DisplayName', 'Signal');\nhold on;\nplot({t}, {rec}, 'DisplayName', 'Reconstruction');\nhold off;\nxlabel('{time_label}'); ylabel('Amplitude'); title('{signal}'); legend('show');\n\
             subplot(2,1,2);\nimagesc({t}, log2({f}), abs({wt}));\naxis xy;\n"
        ));
        if s.transform == Transform::Cwt {
            script.push_str(&format!(
                "hold on;\nplot({t}, log2({coi}), 'w--', 'DisplayName', 'Cone of influence');\nhold off;\n"
            ));
        }
        script.push_str(&format!(
            "xlabel('{time_label}'); ylabel('log_2 frequency ({unit})'); title('Scalogram |coefficients|'); colorbar;\n"
        ));
        for (output, source) in [
            (&self.coefficients, &wt),
            (&self.frequencies, &f),
            (&self.reconstruction, &rec),
        ] {
            if output.export {
                script.push_str(&format!("{} = {source};\n", output.name));
            }
        }
        Ok(Plan {
            script,
            x,
            fs: fsv,
            wt,
            f,
            coi: (s.transform == Transform::Cwt).then_some(coi),
            rec,
            ratio,
            samples,
        })
    }
}

impl App for WaveletTfAnalyzer {
    fn kind(&self) -> AppKind {
        AppKind::WaveletTfAnalyzer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.stopped.as_ref() == Some(&self.settings) {
            return Err(
                "Wavelet Time-Frequency Analyzer: stopped. Change an input to compute again."
                    .into(),
            );
        }
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        if self.settings.signal.is_empty() {
            Vec::new()
        } else {
            vec![self.settings.signal.clone()]
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let Plan {
            x,
            fs,
            wt,
            f,
            coi,
            rec,
            ratio,
            samples,
            ..
        } = plan;
        let mut code = format!(
            "wtf_probe_x = {x};\nwtf_probe_fs = {fs};\nwtf_probe_f = {f}(:);\nwtf_probe_rec = {rec};\nwtf_probe_ratio = {ratio};\n"
        );
        code.push_str(&match coi {
            Some(coi) => format!("wtf_probe_coi = {coi}(:);\n"),
            None => "wtf_probe_coi = zeros(0, 1);\n".into(),
        });
        if samples <= VIEW_COLUMNS {
            code.push_str(&format!("wtf_probe_image = abs({wt});\n"));
        } else {
            // Max over blocks of k samples: rows x ceil(N/k) columns.
            let k = samples.div_ceil(VIEW_COLUMNS);
            let cols = samples.div_ceil(k);
            let pad = k * cols - samples;
            code.push_str(&format!("wtf_probe_image = abs({wt}).';\n"));
            if pad > 0 {
                code.push_str(&format!(
                    "wtf_probe_image = [wtf_probe_image; zeros({pad}, size(wtf_probe_image, 2))];\n"
                ));
            }
            code.push_str(&format!(
                "wtf_probe_image = reshape(max(reshape(wtf_probe_image, {k}, []), [], 1), {cols}, []).';\n"
            ));
        }
        let names = ["x", "fs", "f", "rec", "ratio", "coi", "image"]
            .map(|n| format!("wtf_probe_{n}"))
            .to_vec();
        (code, names)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let read = |name: &str| -> Result<(Vec<f64>, Vec<usize>), String> {
            match outputs.get(&format!("wtf_probe_{name}")) {
                Some(a @ Array::Numeric(m)) => Ok((m.data().to_vec(), a.dims().to_vec())),
                _ => Err(format!(
                    "Wavelet Time-Frequency Analyzer: missing real result {name}"
                )),
            }
        };
        let (x, _) = read("x")?;
        let (fs, _) = read("fs")?;
        let (f, _) = read("f")?;
        let (rec, _) = read("rec")?;
        let (ratio, _) = read("ratio")?;
        let (coi, _) = read("coi")?;
        let (image, dims) = read("image")?;
        let rows = dims.first().copied().unwrap_or(0);
        let cols = image.len().checked_div(rows).unwrap_or(0);
        if f.len() != rows || rec.len() != x.len() || fs.len() != 1 || ratio.len() != 1 {
            return Err("Wavelet Time-Frequency Analyzer: incomplete transform".into());
        }
        if !coi.is_empty() && coi.len() != x.len() {
            return Err("Wavelet Time-Frequency Analyzer: incomplete cone of influence".into());
        }
        self.generation += 1;
        self.current = Some(View {
            settings: self.settings.clone(),
            fs: fs[0],
            x,
            image,
            rows,
            cols,
            f,
            coi,
            rec,
            ratio: ratio[0],
            generation: self.generation,
        });
        Ok(())
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        if self.stopped.is_some() && self.stopped.as_ref() != Some(&self.settings) {
            self.stopped = None;
        }
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let s = &mut self.settings;
        field(ui, "Signal (workspace vector)", |ui| {
            let combo = egui::ComboBox::from_id_salt("wtf_signal")
                .width(ui.available_width())
                .selected_text(if s.signal.is_empty() {
                    "Choose vector"
                } else {
                    &s.signal
                })
                .show_ui(ui, |ui| {
                    for (name, _) in ws.variables().filter(|(_, a)| signal_vector(a)) {
                        ui.selectable_value(&mut s.signal, name.to_owned(), name);
                    }
                });
            crate::focus_areas::register(
                ui,
                &combo.response,
                crate::focus_areas::Area::App(AppKind::WaveletTfAnalyzer),
            );
        });
        let mut normalised = s.rate.trim().is_empty();
        if ui
            .checkbox(&mut normalised, "Normalised frequency (cycles/sample)")
            .changed()
        {
            s.rate = if normalised {
                String::new()
            } else {
                "1000".into()
            };
            s.region = None;
        }
        if !normalised {
            text_field(ui, "Sample rate (Hz)", &mut s.rate);
        }
        let mut transform = s.transform;
        field(ui, "Transform", |ui| {
            egui::ComboBox::from_id_salt("wtf_transform")
                .width(ui.available_width())
                .selected_text(transform.label())
                .show_ui(ui, |ui| {
                    for v in [Transform::Cwt, Transform::Wsst] {
                        ui.selectable_value(&mut transform, v, v.label());
                    }
                });
        });
        self.set_transform(transform);
        let s = &mut self.settings;
        field(ui, "Wavelet", |ui| {
            egui::ComboBox::from_id_salt("wtf_wavelet")
                .width(ui.available_width())
                .selected_text(s.wavelet.label())
                .show_ui(ui, |ui| {
                    for v in [Wavelet::Morse, Wavelet::Amor, Wavelet::Bump] {
                        if v.allowed(s.transform) {
                            ui.selectable_value(&mut s.wavelet, v, v.label());
                        }
                    }
                });
        });
        text_field(
            ui,
            match s.transform {
                Transform::Cwt => "Voices per octave (1–48)",
                Transform::Wsst => "Voices per octave (even, 10–48)",
            },
            &mut s.voices,
        );
        if s.transform == Transform::Cwt {
            ui.label("Frequency limits (empty: full range)");
            ui.horizontal(|ui| {
                let width = (ui.available_width() - crate::tokens::S) / 2.;
                for value in [&mut s.f_min, &mut s.f_max] {
                    ui.add(
                        crate::tokens::input(egui::TextEdit::singleline(value).char_limit(64))
                            .desired_width(width),
                    );
                }
            });
            ui.add_space(crate::tokens::S);
            if s.wavelet == Wavelet::Morse {
                text_field(ui, "Morse gamma (1–20)", &mut s.gamma);
                text_field(ui, "Time-bandwidth P² (3–120)", &mut s.time_bandwidth);
            }
        }
        super::expression_message(ui, self.plan(ws).map(|_| ()));
        ui.add_space(crate::tokens::S);
        if self.running && ui.button("Stop").clicked() {
            self.stop();
        }
        crate::tokens::helper(
            ui,
            format!(
                "Real vectors of {MIN_SAMPLES} to {MAX_SAMPLES} samples. Numeric fields accept finite literals."
            ),
        );
    }
    fn main_caption(&self) -> &'static str {
        "SCALOGRAM"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.current.clone() else {
            crate::tokens::helper(
                ui,
                "Choose a workspace vector to see its scalogram. Drag a box on the scalogram to reconstruct that region.",
            );
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        let hz = !view.settings.rate.trim().is_empty();
        let duration = view.samples() as f64 / view.fs;
        let time_label = if hz { "Time (s)" } else { "Time (samples)" };
        let unit = if hz { "Hz" } else { "cycles/sample" };
        let height = ui.available_height().max(320.);
        let readout = crate::tokens::CONTROL;
        let signal_height = ((height - readout - crate::tokens::S) * 0.3).max(80.);
        let scalogram_height = (height - readout - signal_height - crate::tokens::S).max(160.);
        let axes = egui::Id::new("wavelet_tf_time_axes");
        egui_plot::Plot::new("wavelet_tf_signal")
            .height(signal_height)
            .show_background(false)
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .y_axis_label("Amplitude")
            .link_axis(axes, [true, false])
            .legend(egui_plot::Legend::default())
            .show(ui, |plot| {
                plot.line(
                    egui_plot::Line::new("Signal", envelope(&view.x, view.fs))
                        .color(p.text_muted)
                        .width(1.),
                );
                plot.line(
                    egui_plot::Line::new(
                        if view.settings.region.is_some() {
                            "Region reconstruction"
                        } else {
                            "Reconstruction"
                        },
                        envelope(&view.rec, view.fs),
                    )
                    .color(p.accent)
                    .width(1.),
                );
            });
        ui.add_space(crate::tokens::S);
        let texture = self.texture(ui, &view, p.surface_sunken);
        let [lo, hi] = view.frequency_range();
        let half = if view.rows > 1 {
            (hi.log10() - lo.log10()) / (view.rows - 1) as f64 / 2.
        } else {
            0.05
        };
        let [y0, y1] = [lo.log10() - half, hi.log10() + half];
        let region = self.region();
        let mut drag = self.drag;
        let mut drag_end = self.drag_end;
        let mut finished = None;
        let mut hover = None;
        let mut clear = false;
        egui_plot::Plot::new("wavelet_tf_scalogram")
            .height(scalogram_height)
            .show_background(false)
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .x_axis_label(time_label)
            .y_axis_label(format!("Frequency ({unit}, log)"))
            .y_axis_formatter(|mark, _| short(10f64.powf(mark.value)))
            .link_axis(axes, [true, false])
            .include_x(0.)
            .include_x(duration)
            .include_y(y0)
            .include_y(y1)
            .show(ui, |plot| {
                if let Some(texture) = &texture {
                    plot.image(
                        egui_plot::PlotImage::new(
                            "Scalogram",
                            texture.id(),
                            egui_plot::PlotPoint::new(duration / 2., (y0 + y1) / 2.),
                            egui::vec2(duration as f32, (y1 - y0) as f32),
                        )
                        .allow_hover(false),
                    );
                }
                if !view.coi.is_empty() {
                    let step = (view.coi.len() / LINE_POINTS).max(1);
                    let points: Vec<[f64; 2]> = (0..view.coi.len())
                        .step_by(step)
                        .map(|k| [view.time(k), view.coi[k].max(lo).log10()])
                        .collect();
                    plot.line(
                        egui_plot::Line::new("Cone of influence", points)
                            .color(p.text)
                            .width(1.)
                            .style(egui_plot::LineStyle::dashed_dense()),
                    );
                }
                let response = plot.response().clone();
                let pointer = plot.pointer_coordinate();
                if let Some(pointer) = pointer {
                    let f = 10f64.powf(pointer.y);
                    if let Some(m) = view.magnitude(pointer.x, f) {
                        hover = Some([pointer.x, f, m]);
                    }
                    if response.drag_started_by(egui::PointerButton::Primary) {
                        drag = Some([pointer.x, pointer.y]);
                    }
                    if drag.is_some() && response.dragged_by(egui::PointerButton::Primary) {
                        drag_end = Some([pointer.x, pointer.y]);
                    }
                }
                if response.drag_stopped() {
                    if let (Some(a), Some(b)) = (drag, drag_end) {
                        finished = Some((a, b));
                    }
                    drag = None;
                    drag_end = None;
                }
                if response.double_clicked() {
                    clear = true;
                }
                let boxed = match (drag, drag_end) {
                    (Some(a), Some(b)) => Some([a[0], b[0], a[1], b[1]]),
                    _ => region.map(|r| [r.t[0], r.t[1], r.f[0].log10(), r.f[1].log10()]),
                };
                if let Some([ta, tb, ya, yb]) = boxed {
                    let points = vec![[ta, ya], [tb, ya], [tb, yb], [ta, yb], [ta, ya]];
                    plot.line(
                        egui_plot::Line::new("Region", points)
                            .color(p.accent)
                            .width(2.),
                    );
                }
            });
        self.drag = drag;
        self.drag_end = drag_end;
        self.hover = hover;
        if clear {
            self.set_region(None);
        }
        if let Some((a, b)) = finished {
            let t = [a[0].clamp(0., duration), b[0].clamp(0., duration)];
            let f = [
                10f64.powf(a[1]).clamp(lo, hi),
                10f64.powf(b[1]).clamp(lo, hi),
            ];
            if (t[1] - t[0]).abs() > 0. && (f[1] - f[0]).abs() > 0. {
                self.set_region(Some(Region { t, f }));
            }
        }
        ui.horizontal(|ui| {
            let text = match self.hover {
                Some([t, f, m]) => format!(
                    "t = {} {}, f = {} {unit}, |c| = {}",
                    short(t),
                    if hz { "s" } else { "samples" },
                    short(f),
                    short(m)
                ),
                None => "Hover for time, frequency and magnitude · drag a box to select a region · double-click clears it".into(),
            };
            crate::tokens::helper(ui, text);
            if self.settings.region.is_some() && ui.button("Clear region").clicked() {
                self.set_region(None);
            }
        });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(v) = &self.current else {
            return vec![("Settings", "No transform yet".into())];
        };
        let s = &v.settings;
        let hz = !s.rate.trim().is_empty();
        let unit = if hz { "Hz" } else { "cycles/sample" };
        let mut setting = format!(
            "{} · {} · {} voices",
            match s.transform {
                Transform::Cwt => "cwt",
                Transform::Wsst => "wsst",
            },
            s.wavelet.label(),
            s.voices.trim()
        );
        if s.transform == Transform::Cwt && s.wavelet == Wavelet::Morse {
            setting.push_str(&format!(
                " · γ {} · P² {}",
                s.gamma.trim(),
                s.time_bandwidth.trim()
            ));
        }
        let [lo, hi] = v.frequency_range();
        let mut rows = vec![
            ("Settings", setting),
            (
                "Signal",
                format!(
                    "{} · {} samples{}",
                    s.signal,
                    v.samples(),
                    if hz {
                        format!(" · {} Hz", short(v.fs))
                    } else {
                        String::new()
                    }
                ),
            ),
            ("Scalogram", format!("{} x {} ", v.rows, v.samples())),
            (
                "Frequency range shown",
                format!("{} – {} {unit}", short(lo), short(hi)),
            ),
        ];
        rows.push((
            "Region",
            match &s.region {
                Some([t0, t1, f0, f1]) => format!(
                    "{t0}–{t1} {}, {f0}–{f1} {unit}",
                    if hz { "s" } else { "samples" }
                ),
                None => "Whole transform".into(),
            },
        ));
        rows.push(("Reconstruction energy ratio", short(v.ratio)));
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        for (output, label) in [
            (&mut self.coefficients, "Coefficients"),
            (&mut self.frequencies, "Frequency vector"),
            (&mut self.reconstruction, "Reconstructed signal"),
        ] {
            ui.checkbox(
                &mut output.export,
                format!("Export {}", label.to_lowercase()),
            );
            if output.export {
                name_field(ui, label, &mut output.name, ws);
            }
        }
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        self.outputs()
            .into_iter()
            .filter(|(o, _)| o.export)
            .map(|(o, _)| o.name.clone())
            .collect()
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|v| {
            format!(
                "Wavelet Time-Frequency Analyzer: {} scales x {} samples · energy ratio {}",
                v.rows,
                v.samples(),
                short(v.ratio)
            )
        })
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

/// At most LINE_POINTS points: a min/max envelope of long signals.
fn envelope(v: &[f64], fs: f64) -> Vec<[f64; 2]> {
    if v.len() <= LINE_POINTS {
        return v
            .iter()
            .enumerate()
            .map(|(k, y)| [k as f64 / fs, *y])
            .collect();
    }
    let block = v.len().div_ceil(LINE_POINTS / 2);
    let mut out = Vec::with_capacity(LINE_POINTS);
    for (b, chunk) in v.chunks(block).enumerate() {
        let start = b * block;
        let (lo, hi) = chunk.iter().enumerate().fold(
            ((0, f64::INFINITY), (0, f64::NEG_INFINITY)),
            |(lo, hi), (k, y)| {
                (
                    if *y < lo.1 { (k, *y) } else { lo },
                    if *y > hi.1 { (k, *y) } else { hi },
                )
            },
        );
        let (first, second) = if lo.0 <= hi.0 { (lo, hi) } else { (hi, lo) };
        out.push([(start + first.0) as f64 / fs, first.1]);
        out.push([(start + second.0) as f64 / fs, second.1]);
    }
    out
}

impl WaveletTfAnalyzer {
    /// The scalogram texture (cached per run). Cells under the cone of
    /// influence are blended toward the surface colour.
    fn texture(
        &mut self,
        ui: &egui::Ui,
        view: &View,
        shade: egui::Color32,
    ) -> Option<egui::TextureHandle> {
        if let Some((generation, texture)) = &self.texture
            && *generation == view.generation
        {
            return Some(texture.clone());
        }
        if view.rows == 0 || view.cols == 0 {
            return None;
        }
        let top = view.image.iter().copied().fold(0f64, f64::max);
        let limits = [0., if top > 0. { top } else { 1. }];
        let map = matrust::plot::colormap::default_map();
        let increasing = view.f.first() < view.f.last();
        let n = view.samples();
        let mut rgb = Vec::with_capacity(view.rows * view.cols * 3);
        // Texture row 0 is the top: the highest frequency.
        for tr in 0..view.rows {
            let r = if increasing { view.rows - 1 - tr } else { tr };
            for c in 0..view.cols {
                let mut pixel = matrust::plot::colormap::scaled_rgb(
                    &map,
                    limits,
                    view.image[c * view.rows + r],
                );
                if !view.coi.is_empty() {
                    let k = ((c as f64 + 0.5) / view.cols as f64 * n as f64) as usize;
                    if view.f[r] < view.coi[k.min(n - 1)] {
                        let s = [shade.r(), shade.g(), shade.b()];
                        for (channel, s) in pixel.iter_mut().zip(s) {
                            *channel = ((*channel as u16 + s as u16) / 2) as u8;
                        }
                    }
                }
                rgb.extend(pixel);
            }
        }
        let texture = ui.ctx().load_texture(
            "wavelet-tf-scalogram",
            egui::ColorImage::from_rgb([view.cols, view.rows], &rgb),
            egui::TextureOptions::NEAREST,
        );
        self.texture = Some((view.generation, texture.clone()));
        Some(texture)
    }
}
