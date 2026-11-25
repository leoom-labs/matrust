//! Wavelet Signal Denoiser: one workspace signal, up to five `wdenoise`
//! settings. The selected version's script (the `wdenoise` call with
//! name-value options) is the preview, the export and the saved script; the
//! private probe only reads its variables, recomputes the per-level
//! thresholds from the published rules and the other versions' statistics.
use super::signal_analyzer::{Source, source};
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

pub const MAX_VERSIONS: usize = 5;
/// Longest signal the app denoises.
pub const MAX_SAMPLES: usize = 10_000_000;
/// Min/max buckets per drawn line: at most twice this many points.
pub const PLOT_BUCKETS: usize = 2048;

const BIOR: [&str; 15] = [
    "1.1", "1.3", "1.5", "2.2", "2.4", "2.6", "2.8", "3.1", "3.3", "3.5", "3.7", "3.9", "4.4",
    "5.5", "6.8",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Haar,
    Daubechies,
    Symlet,
    Coiflet,
    Biorthogonal,
}
impl Family {
    pub const ALL: [Self; 5] = [
        Self::Haar,
        Self::Daubechies,
        Self::Symlet,
        Self::Coiflet,
        Self::Biorthogonal,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Haar => "Haar (haar)",
            Self::Daubechies => "Daubechies (db)",
            Self::Symlet => "Symlet (sym)",
            Self::Coiflet => "Coiflet (coif)",
            Self::Biorthogonal => "Biorthogonal (bior)",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Self::Haar => "haar",
            Self::Daubechies => "db",
            Self::Symlet => "sym",
            Self::Coiflet => "coif",
            Self::Biorthogonal => "bior",
        }
    }
    /// The numbers the engine has filters for.
    pub fn orders(self) -> Vec<String> {
        let range = |a: u32, b: u32| (a..=b).map(|n| n.to_string()).collect();
        match self {
            Self::Haar => Vec::new(),
            Self::Daubechies => range(1, 20),
            Self::Symlet => range(2, 20),
            Self::Coiflet => range(1, 5),
            Self::Biorthogonal => BIOR.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn default_order(self) -> &'static str {
        match self {
            Self::Haar => "",
            Self::Daubechies => "4",
            Self::Symlet => "4",
            Self::Coiflet => "2",
            Self::Biorthogonal => "3.5",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Bayes,
    Sure,
    Universal,
    Minimax,
}
impl Method {
    pub const ALL: [Self; 4] = [Self::Bayes, Self::Sure, Self::Universal, Self::Minimax];
    /// The `DenoisingMethod` value.
    pub fn option(self) -> &'static str {
        match self {
            Self::Bayes => "Bayes",
            Self::Sure => "SURE",
            Self::Universal => "UniversalThreshold",
            Self::Minimax => "Minimax",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Bayes => "Empirical Bayes",
            Self::Sure => "SURE",
            Self::Universal => "Universal threshold",
            Self::Minimax => "Minimax",
        }
    }
    pub fn rules(self) -> &'static [Rule] {
        match self {
            Self::Bayes => &[Rule::Median, Rule::Mean, Rule::Soft, Rule::Hard],
            _ => &[Rule::Soft, Rule::Hard],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    Median,
    Mean,
    Soft,
    Hard,
}
impl Rule {
    pub fn option(self) -> &'static str {
        match self {
            Self::Median => "Median",
            Self::Mean => "Mean",
            Self::Soft => "Soft",
            Self::Hard => "Hard",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Noise {
    LevelIndependent,
    LevelDependent,
}
impl Noise {
    pub fn option(self) -> &'static str {
        match self {
            Self::LevelIndependent => "LevelIndependent",
            Self::LevelDependent => "LevelDependent",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::LevelIndependent => "Level-independent (finest details)",
            Self::LevelDependent => "Level-dependent (each level)",
        }
    }
}

/// One denoised version's `wdenoise` options.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Settings {
    pub family: Family,
    pub order: String,
    /// Empty: the `wdenoise` default level.
    pub level: String,
    pub method: Method,
    pub rule: Rule,
    pub noise: Noise,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            family: Family::Symlet,
            order: "4".into(),
            level: String::new(),
            method: Method::Bayes,
            rule: Rule::Median,
            noise: Noise::LevelIndependent,
        }
    }
}
impl Settings {
    pub fn set_family(&mut self, family: Family) {
        if family != self.family {
            self.family = family;
            self.order = family.default_order().into();
        }
    }
    pub fn set_method(&mut self, method: Method) {
        self.method = method;
        if !method.rules().contains(&self.rule) {
            self.rule = method.rules()[0];
        }
    }
    /// The wavelet name, checked against the engine's filter list.
    pub fn wavelet(&self) -> Result<String, String> {
        if self.family == Family::Haar {
            return Ok("haar".into());
        }
        if !self.family.orders().contains(&self.order) {
            return Err(format!(
                "Choose a number for the {} family",
                self.family.prefix()
            ));
        }
        Ok(format!("{}{}", self.family.prefix(), self.order))
    }
    /// The explicit level, if typed: an integer from 1 to floor(log2 n).
    pub fn level(&self, n: usize) -> Result<Option<usize>, String> {
        let text = self.level.trim();
        if text.is_empty() {
            return Ok(None);
        }
        if text.len() > 16 {
            return Err("Level: at most 16 characters".into());
        }
        super::single_expression(text).map_err(|e| format!("Level: {e}"))?;
        let top = (n.max(2) as f64).log2().floor() as usize;
        text.parse::<usize>()
            .ok()
            .filter(|l| (1..=top).contains(l))
            .map(Some)
            .ok_or_else(|| format!("Level: enter an integer from 1 to {top} (floor(log2 N))"))
    }
    /// `wdenoise` arguments after the signal.
    fn arguments(&self, n: usize) -> Result<String, String> {
        if !self.method.rules().contains(&self.rule) {
            return Err(format!(
                "Rule {} does not apply to {}",
                self.rule.option(),
                self.method.label()
            ));
        }
        let mut args = String::new();
        if let Some(level) = self.level(n)? {
            args.push_str(&format!("{level}, "));
        }
        args.push_str(&format!(
            "'Wavelet', '{}', 'DenoisingMethod', '{}', 'ThresholdRule', '{}', 'NoiseEstimate', '{}'",
            self.wavelet()?,
            self.method.option(),
            self.rule.option(),
            self.noise.option()
        ));
        Ok(args)
    }
    /// Short description for the versions table.
    pub fn title(&self) -> String {
        let level = if self.level.trim().is_empty() {
            "default level".to_string()
        } else {
            format!("level {}", self.level.trim())
        };
        format!(
            "{} · {level} · {} {} · {}",
            self.wavelet().unwrap_or_else(|_| "?".into()),
            self.method.option(),
            self.rule.option().to_ascii_lowercase(),
            match self.noise {
                Noise::LevelIndependent => "indep.",
                Noise::LevelDependent => "per level",
            }
        )
    }
}

#[derive(Clone, Debug)]
pub struct Version {
    pub id: u32,
    pub settings: Settings,
}

/// What the view draws (decimated) and the run's numbers.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub samples: usize,
    pub hz: bool,
    pub original: Vec<[f64; 2]>,
    pub denoised: Vec<[f64; 2]>,
    /// Finest level first: original and denoised coefficients against the
    /// signal's x axis, and the level's threshold (NaN when none applies).
    pub levels: Vec<LevelView>,
    pub level: usize,
    /// Version id, [SNR dB, removed RMS].
    pub stats: Vec<(u32, [f64; 2])>,
}
#[derive(Clone, Debug)]
pub struct LevelView {
    pub count: usize,
    pub original: Vec<[f64; 2]>,
    pub denoised: Vec<[f64; 2]>,
    pub threshold: f64,
}

pub struct WaveletDenoiser {
    pub source: String,
    /// Sample rate text in Hz; ignored when `samples`.
    pub rate: String,
    pub samples: bool,
    pub versions: Vec<Version>,
    pub selected: usize,
    pub signal_name: String,
    pub settings_name: String,
    pub current: Option<View>,
    /// The Host is computing (set each frame by the Apps window).
    pub running: bool,
    /// Inputs at Stop; nothing runs until they change or Run.
    stopped: Option<u64>,
    next_id: u32,
}
impl Default for WaveletDenoiser {
    fn default() -> Self {
        Self {
            source: String::new(),
            rate: "1".into(),
            samples: true,
            versions: vec![Version {
                id: 1,
                settings: Settings::default(),
            }],
            selected: 0,
            signal_name: "xden".into(),
            settings_name: "denoiseSettings".into(),
            current: None,
            running: false,
            stopped: None,
            next_id: 2,
        }
    }
}

/// Names of the script's temporaries, read by the private probe.
struct Plan {
    script: String,
    signal: String,
    denoised: String,
    coefficients: String,
    original: String,
    samples: usize,
}

impl WaveletDenoiser {
    /// Keeps a copy of the selected version as a new row and selects it.
    pub fn keep(&mut self) -> Result<(), String> {
        if self.versions.len() >= MAX_VERSIONS {
            return Err(format!("At most {MAX_VERSIONS} versions"));
        }
        let settings = self
            .versions
            .get(self.selected)
            .map(|v| v.settings.clone())
            .unwrap_or_default();
        self.versions.push(Version {
            id: self.next_id,
            settings,
        });
        self.next_id += 1;
        self.selected = self.versions.len() - 1;
        Ok(())
    }
    pub fn remove(&mut self) {
        if self.versions.len() > 1 && self.selected < self.versions.len() {
            self.versions.remove(self.selected);
        }
        self.selected = self.selected.min(self.versions.len().saturating_sub(1));
    }
    pub fn settings(&mut self) -> &mut Settings {
        let k = self.selected.min(self.versions.len() - 1);
        &mut self.versions[k].settings
    }
    /// Choose a workspace signal; a timetable's row times set the rate.
    pub fn choose(&mut self, name: &str, ws: &Snapshot) {
        self.source = name.to_owned();
        if let Some(Source::Timetable {
            rate: Some(rate), ..
        }) = ws.get(name).and_then(source)
        {
            self.rate = number(rate);
            self.samples = false;
        }
    }
    /// Stop: cancels the run; nothing runs until an input changes or Run.
    pub fn halt(&mut self) {
        self.stopped = Some(self.fingerprint());
    }
    pub fn resume(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }
    fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&self.source, &self.rate, self.samples, self.selected).hash(&mut hasher);
        (&self.signal_name, &self.settings_name).hash(&mut hasher);
        for v in &self.versions {
            v.settings.hash(&mut hasher);
        }
        hasher.finish()
    }
    fn rate_value(&self) -> Result<Option<f64>, String> {
        if self.samples {
            return Ok(None);
        }
        let text = self.rate.trim();
        if text.len() > 64 {
            return Err("Sample rate: at most 64 characters".into());
        }
        super::single_expression(text).map_err(|e| format!("Sample rate: {e}"))?;
        text.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.)
            .map(Some)
            .ok_or_else(|| "Sample rate: enter one positive finite number (Hz)".into())
    }
    /// The signal expression and its length.
    fn signal(&self, ws: &Snapshot) -> Result<(String, usize), String> {
        if self.source.is_empty() {
            return Err("Wavelet Signal Denoiser: choose a signal in Inputs".into());
        }
        super::single_expression(&self.source)?;
        if self.source.len() > 63 || !matrust::files::valid_name(&self.source) {
            return Err("Wavelet Signal Denoiser: choose a workspace signal".into());
        }
        let array = ws.get(&self.source).ok_or(format!(
            "Wavelet Signal Denoiser: {} is no longer in the workspace",
            self.source
        ))?;
        let (expression, n) = match source(array) {
            Some(Source::Vector(n)) if !array.is_complex() => (self.source.clone(), n),
            Some(Source::Timetable {
                variable, height, ..
            }) if matrust::files::valid_name(&variable) => {
                (format!("{}.{variable}", self.source), height)
            }
            _ => {
                return Err(format!(
                    "Wavelet Signal Denoiser: {} is not a real vector or a one-variable timetable",
                    self.source
                ));
            }
        };
        if n > MAX_SAMPLES {
            return Err(format!(
                "Wavelet Signal Denoiser: at most {MAX_SAMPLES} samples"
            ));
        }
        Ok((expression, n))
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.stopped.is_some() {
            return Err(
                "Wavelet Signal Denoiser: stopped. Press Run or change an input to compute again."
                    .into(),
            );
        }
        let (expression, n) = self.signal(ws)?;
        let fs = self
            .rate_value()
            .map_err(|e| format!("Wavelet Signal Denoiser: {e}"))?;
        let mut names = BTreeSet::new();
        for (what, name) in [
            ("denoised signal", &self.signal_name),
            ("settings", &self.settings_name),
        ] {
            super::single_expression(name)?;
            if name.len() > 63 || !matrust::files::valid_name(name) {
                return Err(format!(
                    "Wavelet Signal Denoiser: the {what} needs a valid variable name (at most 63 characters)"
                ));
            }
            super::check_export_name(name)?;
            if !names.insert(name.clone()) {
                return Err(format!(
                    "Wavelet Signal Denoiser: duplicate export name {name}"
                ));
            }
        }
        let version = self
            .versions
            .get(self.selected)
            .ok_or("Wavelet Signal Denoiser: select a version")?;
        let arguments = version
            .settings
            .arguments(n)
            .map_err(|e| format!("Wavelet Signal Denoiser: {e}"))?;
        // Temporaries avoid workspace and output names; exports come last,
        // so an output named like the source or a builtin is safe.
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || names.contains(n));
            names.insert(name.clone());
            name
        };
        let signal = fresh("wsd_signal");
        let denoised = fresh("wsd_denoised");
        let coefficients = fresh("wsd_coefficients");
        let original = fresh("wsd_original");
        let settings = fresh("wsd_settings");
        let mode = fresh("wsd_dwtmode");
        let s = &version.settings;
        let rate = fs.map_or("[]".to_string(), number);
        let mut script = format!(
            "% Wavelet Signal Denoiser: {}.\n% Coefficient cells hold details d1 (finest) to dL, then the approximation aL.\n",
            match fs {
                Some(fs) => format!("sample rate {} Hz", number(fs)),
                None => "x axis in samples".into(),
            }
        );
        script.push_str(&format!(
            "if ~isnumeric({expression}) || ~isreal({expression}) || ~isvector({expression}) || numel({expression}) < 2 || numel({expression}) > {MAX_SAMPLES}, error('Wavelet Signal Denoiser: the signal must be a real numeric vector of 2 to {MAX_SAMPLES} samples'); end\n"
        ));
        script.push_str(&format!("{signal} = double({expression});\n"));
        // Preview, Export and this script all denoise with symmetric extension,
        // then put the workspace mode back.
        script.push_str(&format!(
            "{mode} = dwtmode('status','nodisp');\ndwtmode('sym','nodisp');\n"
        ));
        script.push_str(&format!(
            "[{denoised}, {coefficients}, {original}] = wdenoise({signal}, {arguments});\n"
        ));
        script.push_str(&format!("dwtmode({mode},'nodisp');\nclear {mode}\n"));
        script.push_str(&format!(
            "{settings} = struct('Wavelet', '{}', 'Level', numel({coefficients}) - 1, 'DenoisingMethod', '{}', 'ThresholdRule', '{}', 'NoiseEstimate', '{}', 'SampleRate', {rate});\n",
            s.wavelet()?,
            s.method.option(),
            s.rule.option(),
            s.noise.option()
        ));
        script.push_str(&format!(
            "{} = {denoised};\n{} = {settings};\n",
            self.signal_name, self.settings_name
        ));
        Ok(Plan {
            script,
            signal,
            denoised,
            coefficients,
            original,
            samples: n,
        })
    }
}

/// Probe lines: the level thresholds `wdenoise` applied, rebuilt from the
/// published rules (Bayes: the largest coefficient it set to zero).
fn threshold_probe(plan: &Plan, settings: &Settings) -> String {
    let n = plan.samples;
    let noise = match settings.noise {
        Noise::LevelIndependent => "wsd_probe_s1",
        Noise::LevelDependent => "median(abs(wsd_probe_d)) / 0.6745",
    };
    let rule = match settings.method {
        Method::Universal => format!("wsd_probe_t = wsd_probe_s * sqrt(2 * log({n}));"),
        Method::Minimax => format!(
            "wsd_probe_t = wsd_probe_s * ({n} > 32) * (0.3936 + 0.1829 * log2({n}));"
        ),
        Method::Sure => "if wsd_probe_s > 0, wsd_probe_t = wsd_probe_s * thselect(wsd_probe_d / wsd_probe_s, 'rigrsure');\nelse\nwsd_probe_t = 0;\nend".into(),
        Method::Bayes => "wsd_probe_z = abs(wsd_probe_d(wsd_probe_c == 0)); if isempty(wsd_probe_z), wsd_probe_t = NaN;\nelse\nwsd_probe_t = max(wsd_probe_z);\nend".into(),
    };
    format!(
        "wsd_probe_thresholds = zeros(1, numel({o}) - 1);\n\
         wsd_probe_d = {o}{{1}}; wsd_probe_s1 = median(abs(wsd_probe_d(:))) / 0.6745;\n\
         for wsd_probe_k = 1:numel(wsd_probe_thresholds)\n\
         wsd_probe_d = {o}{{wsd_probe_k}}; wsd_probe_d = wsd_probe_d(:);\n\
         wsd_probe_c = {c}{{wsd_probe_k}}; wsd_probe_c = wsd_probe_c(:);\n\
         wsd_probe_s = {noise};\n\
         if wsd_probe_s <= 0\nwsd_probe_t = 0;\nelse\n{rule}\nend\n\
         wsd_probe_thresholds(wsd_probe_k) = wsd_probe_t;\n\
         end\n",
        o = plan.original,
        c = plan.coefficients,
    )
}

fn stats_line(name: &str, x: &str, y: &str) -> String {
    format!(
        "{name} = [10 * log10(sum({x}(:) .^ 2) / sum(({x}(:) - {y}(:)) .^ 2)), sqrt(mean(({x}(:) - {y}(:)) .^ 2))];\n"
    )
}

/// Min and max of each bucket, in sample order: the drawn shape of a long
/// signal (spikes survive) with at most 2 * PLOT_BUCKETS points.
pub fn envelope(values: &[f64], x: impl Fn(usize) -> f64) -> Vec<[f64; 2]> {
    if values.len() <= 2 * PLOT_BUCKETS {
        return values.iter().enumerate().map(|(k, v)| [x(k), *v]).collect();
    }
    let size = values.len().div_ceil(PLOT_BUCKETS);
    let mut out = Vec::with_capacity(2 * PLOT_BUCKETS);
    for (b, chunk) in values.chunks(size).enumerate() {
        let start = b * size;
        let (mut lo, mut hi) = (0, 0);
        for (k, v) in chunk.iter().enumerate() {
            if *v < chunk[lo] {
                lo = k;
            }
            if *v > chunk[hi] {
                hi = k;
            }
        }
        for k in if lo <= hi { [lo, hi] } else { [hi, lo] } {
            out.push([x(start + k), chunk[k]]);
        }
    }
    out
}

fn numbers(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(a @ Array::Numeric(m)) if !a.is_complex() => Ok(m.data().to_vec()),
        _ => Err(format!(
            "Wavelet Signal Denoiser: missing numeric result {name}"
        )),
    }
}
fn cells(outputs: &Outputs, name: &str) -> Result<Vec<Vec<f64>>, String> {
    let bad = || format!("Wavelet Signal Denoiser: missing coefficient cell {name}");
    let Some(Array::Cell(c)) = outputs.get(name) else {
        return Err(bad());
    };
    (0..c.len())
        .map(|k| match c.get_linear(k).map(|a| a.as_ref()) {
            Ok(Array::Numeric(m)) => Ok(m.data().to_vec()),
            _ => Err(bad()),
        })
        .collect()
}

impl App for WaveletDenoiser {
    fn kind(&self) -> AppKind {
        AppKind::WaveletDenoiser
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
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let selected = &self.versions[self.selected].settings;
        let mut code = format!(
            "wsd_probe_x = {}(:);\nwsd_probe_y = {}(:);\nwsd_probe_original = {};\nwsd_probe_coefficients = {};\n",
            plan.signal, plan.denoised, plan.original, plan.coefficients
        );
        code.push_str(&threshold_probe(&plan, selected));
        let mut outputs: Vec<String> = [
            "wsd_probe_x",
            "wsd_probe_y",
            "wsd_probe_original",
            "wsd_probe_coefficients",
            "wsd_probe_thresholds",
        ]
        .map(String::from)
        .to_vec();
        for (k, v) in self.versions.iter().enumerate() {
            let name = format!("wsd_probe_stats_{}", v.id);
            if k == self.selected {
                code.push_str(&stats_line(&name, &plan.signal, &plan.denoised));
            } else if let Ok(args) = v.settings.arguments(plan.samples) {
                code.push_str(&format!(
                    "wsd_probe_v = wdenoise({}, {args});\n",
                    plan.signal
                ));
                code.push_str(&stats_line(&name, &plan.signal, "wsd_probe_v"));
            } else {
                continue;
            }
            outputs.push(name);
        }
        (code, outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let x = numbers(outputs, "wsd_probe_x")?;
        let y = numbers(outputs, "wsd_probe_y")?;
        let original = cells(outputs, "wsd_probe_original")?;
        let coefficients = cells(outputs, "wsd_probe_coefficients")?;
        let thresholds = numbers(outputs, "wsd_probe_thresholds")?;
        let level = thresholds.len();
        if x.len() != y.len()
            || x.len() < 2
            || original.len() != level + 1
            || coefficients.len() != level + 1
        {
            return Err("Wavelet Signal Denoiser: incomplete results".into());
        }
        let fs = self.rate_value().ok().flatten();
        let step = fs.map_or(1., |fs| 1. / fs);
        let n = x.len();
        let levels = (0..level)
            .map(|k| {
                let count = original[k].len();
                // Coefficient i of a level covers about n / count samples.
                let at = |i: usize| (i as f64 + 0.5) * n as f64 / count.max(1) as f64 * step;
                LevelView {
                    count,
                    original: envelope(&original[k], at),
                    denoised: envelope(&coefficients[k], at),
                    threshold: thresholds[k],
                }
            })
            .collect();
        let mut stats = Vec::new();
        for v in &self.versions {
            if let Ok(s) = numbers(outputs, &format!("wsd_probe_stats_{}", v.id))
                && s.len() == 2
            {
                stats.push((v.id, [s[0], s[1]]));
            }
        }
        self.current = Some(View {
            samples: n,
            hz: fs.is_some(),
            original: envelope(&x, |k| k as f64 * step),
            denoised: envelope(&y, |k| k as f64 * step),
            levels,
            level,
            stats,
        });
        Ok(())
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        if self.versions.is_empty() {
            self.versions.push(Version {
                id: self.next_id,
                settings: Settings::default(),
            });
            self.next_id += 1;
        }
        self.selected = self.selected.min(self.versions.len() - 1);
        if self.stopped.is_some_and(|f| f != self.fingerprint()) {
            self.stopped = None;
        }
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let signals: Vec<(String, String)> = ws
            .variables()
            .filter_map(|(name, a)| {
                let info = match source(a)? {
                    Source::Vector(n) if !a.is_complex() => format!("{n} samples"),
                    Source::Timetable {
                        variable, height, ..
                    } if matrust::files::valid_name(&variable) => {
                        format!("timetable, {height} rows")
                    }
                    _ => return None,
                };
                Some((name.to_owned(), info))
            })
            .collect();
        let mut chosen = None;
        field(ui, "Signal", |ui| {
            let combo = egui::ComboBox::from_id_salt("wsd_source")
                .width(ui.available_width())
                .selected_text(if self.source.is_empty() {
                    "Choose signal"
                } else {
                    &self.source
                })
                .show_ui(ui, |ui| {
                    for (name, info) in &signals {
                        if ui
                            .selectable_label(self.source == *name, format!("{name} ({info})"))
                            .clicked()
                        {
                            chosen = Some(name.clone());
                        }
                    }
                });
            crate::focus_areas::register(
                ui,
                &combo.response,
                crate::focus_areas::Area::App(AppKind::WaveletDenoiser),
            );
        });
        if let Some(name) = chosen {
            self.choose(&name, ws);
        }
        if signals.is_empty() {
            crate::tokens::helper(
                ui,
                "No real vectors or one-variable timetables in the workspace.",
            );
        }
        field(ui, "Time axis", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.samples, true, "Samples");
                ui.selectable_value(&mut self.samples, false, "Sample rate (Hz)");
            });
            if !self.samples {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(&mut self.rate).char_limit(64))
                        .desired_width(ui.available_width()),
                );
            }
        });
        super::expression_message(ui, self.rate_value().map(|_| ()));
        ui.add_space(crate::tokens::S);
        let n = self.signal(ws).map(|(_, n)| n).unwrap_or(0);
        let s = self.settings();
        let mut family = s.family;
        field(ui, "Wavelet", |ui| {
            egui::ComboBox::from_id_salt("wsd_family")
                .width(ui.available_width())
                .selected_text(family.label())
                .show_ui(ui, |ui| {
                    for f in Family::ALL {
                        ui.selectable_value(&mut family, f, f.label());
                    }
                });
        });
        s.set_family(family);
        if s.family != Family::Haar {
            field(ui, "Number", |ui| {
                egui::ComboBox::from_id_salt("wsd_order")
                    .width(ui.available_width())
                    .selected_text(s.wavelet().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for o in s.family.orders() {
                            let label = format!("{}{o}", s.family.prefix());
                            ui.selectable_value(&mut s.order, o, label);
                        }
                    });
            });
        }
        field(ui, "Level (empty: wdenoise default)", |ui| {
            ui.add(
                crate::tokens::input(egui::TextEdit::singleline(&mut s.level).char_limit(16))
                    .hint_text("default")
                    .desired_width(ui.available_width()),
            );
        });
        if n > 0 {
            super::expression_message(ui, s.level(n).map(|_| ()));
        }
        let mut method = s.method;
        field(ui, "Method", |ui| {
            egui::ComboBox::from_id_salt("wsd_method")
                .width(ui.available_width())
                .selected_text(method.label())
                .show_ui(ui, |ui| {
                    for m in Method::ALL {
                        ui.selectable_value(&mut method, m, m.label());
                    }
                });
        });
        s.set_method(method);
        field(ui, "Threshold rule", |ui| {
            egui::ComboBox::from_id_salt("wsd_rule")
                .width(ui.available_width())
                .selected_text(s.rule.option())
                .show_ui(ui, |ui| {
                    for r in s.method.rules() {
                        ui.selectable_value(&mut s.rule, *r, r.option());
                    }
                });
        });
        field(ui, "Noise estimate", |ui| {
            egui::ComboBox::from_id_salt("wsd_noise")
                .width(ui.available_width())
                .selected_text(s.noise.label())
                .show_ui(ui, |ui| {
                    for v in [Noise::LevelIndependent, Noise::LevelDependent] {
                        ui.selectable_value(&mut s.noise, v, v.label());
                    }
                });
        });
        ui.horizontal(|ui| {
            if self.running && !self.is_stopped() {
                if ui.button("Stop").clicked() {
                    self.halt();
                }
                crate::tokens::helper(ui, "Denoising…");
            } else if self.is_stopped() && ui.button("Run").clicked() {
                self.resume();
            }
        });
    }
    fn main_caption(&self) -> &'static str {
        "SIGNAL AND COEFFICIENTS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let Some(view) = &self.current else {
            crate::tokens::helper(
                ui,
                "Choose a signal to compare it with its denoised version.",
            );
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        let width = ui.available_width();
        let height = ui.available_height().max(320.);
        let top = (height * 0.45).max(160.);
        let axis = if view.hz { "Time (s)" } else { "Samples" };
        let link = egui::Id::new("wsd_time_axis");
        egui_plot::Plot::new("wsd_signal")
            .width(width)
            .height(top)
            .show_background(false)
            .allow_scroll(false)
            .link_axis(link, [true, false])
            .legend(egui_plot::Legend::default().follow_insertion_order(true))
            .x_axis_label(axis)
            .y_axis_label("Amplitude")
            .show(ui, |plot| {
                plot.line(
                    egui_plot::Line::new("Original", view.original.clone())
                        .color(p.text_muted)
                        .width(1.),
                );
                plot.line(
                    egui_plot::Line::new("Denoised", view.denoised.clone())
                        .color(p.accent)
                        .width(1.5),
                );
            });
        ui.add_space(crate::tokens::S);
        crate::tokens::helper(
            ui,
            "Detail coefficients per level (d1 finest), placed at their approximate time; dashed lines: ± threshold.",
        );
        let rows = view.levels.len().max(1) as f32;
        let caption_height = ui.text_style_height(&egui::TextStyle::Body);
        let each =
            (ui.available_height() / rows - caption_height - 2. * ui.spacing().item_spacing.y)
                .clamp(48., 160.)
                .floor();
        egui::ScrollArea::vertical()
            .id_salt("wsd_levels_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (k, level) in view.levels.iter().enumerate() {
                    ui.label(format!("d{} ({} coefficients)", k + 1, level.count));
                    egui_plot::Plot::new(("wsd_level", k))
                        .width(ui.available_width())
                        .height(each)
                        .show_background(false)
                        .allow_scroll(false)
                        .link_axis(link, [true, false])
                        .show(ui, |plot| {
                            plot.line(
                                egui_plot::Line::new("Original", level.original.clone())
                                    .color(p.text_muted)
                                    .width(1.),
                            );
                            plot.line(
                                egui_plot::Line::new("Denoised", level.denoised.clone())
                                    .color(p.accent)
                                    .width(1.),
                            );
                            if level.threshold.is_finite() {
                                for t in [level.threshold, -level.threshold] {
                                    plot.hline(
                                        egui_plot::HLine::new("Threshold", t)
                                            .color(p.text)
                                            .style(egui_plot::LineStyle::dashed_dense())
                                            .width(1.),
                                    );
                                }
                            }
                        });
                }
            });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(view) = &self.current else {
            return Vec::new();
        };
        let mut rows = vec![
            ("Signal", self.source.clone()),
            ("Samples", view.samples.to_string()),
            ("Levels", view.level.to_string()),
        ];
        let id = self.versions.get(self.selected).map(|v| v.id);
        if let Some((_, [snr, rms])) = view.stats.iter().find(|(v, _)| Some(*v) == id) {
            rows.push(("SNR vs original (dB)", short(*snr)));
            rows.push(("RMS of removed part", short(*rms)));
        }
        if let Some(t) = view.levels.first().map(|l| l.threshold) {
            rows.push((
                "Threshold d1",
                if t.is_nan() { "n/a".into() } else { short(t) },
            ));
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.versions.len() < MAX_VERSIONS,
                    egui::Button::new("Keep as new version"),
                )
                .on_hover_text("Copy the selected settings to a new row and edit that row")
                .clicked()
            {
                let _ = self.keep();
            }
            if ui
                .add_enabled(self.versions.len() > 1, egui::Button::new("Remove"))
                .clicked()
            {
                self.remove();
            }
        });
        ui.add_space(crate::tokens::XS);
        let p = crate::tokens::palette(ui.ctx());
        let stats = self
            .current
            .as_ref()
            .map(|v| v.stats.clone())
            .unwrap_or_default();
        let mut chosen = None;
        egui::Grid::new("wsd_versions")
            .num_columns(3)
            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
            .show(ui, |ui| {
                for heading in ["Version", "SNR dB", "Removed RMS"] {
                    ui.label(egui::RichText::new(heading).color(p.text_muted));
                }
                ui.end_row();
                for (k, v) in self.versions.iter().enumerate() {
                    if ui
                        .selectable_label(k == self.selected, format!("{}", k + 1))
                        .on_hover_text(v.settings.title())
                        .clicked()
                    {
                        chosen = Some(k);
                    }
                    let s = stats.iter().find(|(id, _)| *id == v.id).map(|(_, s)| *s);
                    ui.label(s.map_or("—".into(), |s| short(s[0])));
                    ui.label(s.map_or("—".into(), |s| short(s[1])));
                    ui.end_row();
                }
            });
        if let Some(k) = chosen {
            self.selected = k;
        }
        if let Some(v) = self.versions.get(self.selected) {
            crate::tokens::helper(ui, format!("Shown: {}", v.settings.title()));
        }
        ui.add_space(crate::tokens::S);
        name_field(ui, "Denoised signal", &mut self.signal_name, ws);
        name_field(ui, "Settings struct", &mut self.settings_name, ws);
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        vec![self.signal_name.clone(), self.settings_name.clone()]
    }
    fn summary(&self) -> Option<String> {
        let view = self.current.as_ref()?;
        let v = self.versions.get(self.selected)?;
        Some(format!(
            "Wavelet Signal Denoiser: {} · {} samples",
            v.settings.title(),
            view.samples
        ))
    }
}
