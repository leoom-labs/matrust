//! PID Tuner: tunes a P, I, PI, PD, PID, PDF or PIDF controller for a SISO
//! plant with `pidtune`. The Response time slider scales the crossover of the
//! initial design (×0.1..×10); the Robustness slider sets the target phase
//! margin through `pidtuneOptions`. Every number shown comes from engine
//! builtins (`pidtune`, `step`, `bode`, `stepinfo`, `margin`, `isstable`).
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;

/// Controller types, named as pidtune's type argument.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    P,
    I,
    PI,
    PD,
    PID,
    PDF,
    PIDF,
}
impl Kind {
    pub const ALL: [Self; 7] = [
        Self::P,
        Self::I,
        Self::PI,
        Self::PD,
        Self::PID,
        Self::PDF,
        Self::PIDF,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::P => "P",
            Self::I => "I",
            Self::PI => "PI",
            Self::PD => "PD",
            Self::PID => "PID",
            Self::PDF => "PDF",
            Self::PIDF => "PIDF",
        }
    }
    /// One free gain: the crossover alone fixes it, so no margin target.
    pub fn one_gain(self) -> bool {
        matches!(self, Self::P | Self::I)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Reference,
    Disturbance,
    Bode,
    Effort,
}
impl View {
    pub const ALL: [Self; 4] = [Self::Reference, Self::Disturbance, Self::Bode, Self::Effort];
    pub fn label(self) -> &'static str {
        match self {
            Self::Reference => "Step (reference)",
            Self::Disturbance => "Step (disturbance)",
            Self::Bode => "Bode (open loop)",
            Self::Effort => "Controller effort",
        }
    }
}

/// Response time slider: the crossover is the initial one times this.
pub const FACTOR: std::ops::RangeInclusive<f64> = 0.1..=10.;
/// Robustness slider: target phase margin, degrees.
pub const MARGIN: std::ops::RangeInclusive<f64> = 30.0..=90.0;
pub const DEFAULT_MARGIN: f64 = 60.;

/// One tuned design as the private run computed it.
#[derive(Clone, Debug)]
pub struct Design {
    /// Kp, Ki, Kd, Tf.
    pub gains: [f64; 4],
    /// Crossover pidtune used (info.CrossoverFrequency).
    pub wc: f64,
    pub reference: Vec<[f64; 2]>,
    pub disturbance: Vec<[f64; 2]>,
    /// (log10 w, dB) and (log10 w, degrees) of C*G.
    pub magnitude: Vec<[f64; 2]>,
    pub phase: Vec<[f64; 2]>,
    /// Controller output for a reference step; None for an improper controller.
    pub effort: Option<Vec<[f64; 2]>>,
    pub rise: f64,
    pub settling: f64,
    pub overshoot: f64,
    pub peak: f64,
    pub gain_margin: f64,
    pub phase_margin: f64,
    pub phase_crossover: f64,
    pub gain_crossover: f64,
    pub stable: bool,
}

const PROBE: [&str; 23] = [
    "app_wc",
    "app_kp",
    "app_ki",
    "app_kd",
    "app_tf",
    "app_r",
    "app_rt",
    "app_d",
    "app_dt",
    "app_db",
    "app_ph",
    "app_w",
    "app_rise",
    "app_settle",
    "app_over",
    "app_peak",
    "app_gm",
    "app_pm",
    "app_wcg",
    "app_wcp",
    "app_stable",
    "app_u",
    "app_ut",
];

fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(matrust::Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("PID Tuner: {name} is not a real array")),
    }
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("PID Tuner: {name} is empty"))
}
/// Finite (x, y) points; `log_x` plots x on a log10 axis.
fn curve(x: &[f64], y: &[f64], log_x: bool) -> Vec<[f64; 2]> {
    x.iter()
        .zip(y)
        .map(|(x, y)| [if log_x { x.log10() } else { *x }, *y])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}
impl Design {
    /// Same controller (gains and crossover), hence the same responses. The
    /// measured fields may hold NaN (no phase crossover), so no `==`.
    pub fn same_as(&self, other: &Self) -> bool {
        self.gains == other.gains && self.wc == other.wc
    }
    pub fn from_outputs(outputs: &Outputs) -> Result<Self, String> {
        let w = values(outputs, "app_w")?;
        let effort_y = values(outputs, "app_u")?;
        Ok(Self {
            gains: [
                scalar(outputs, "app_kp")?,
                scalar(outputs, "app_ki")?,
                scalar(outputs, "app_kd")?,
                scalar(outputs, "app_tf")?,
            ],
            wc: scalar(outputs, "app_wc")?,
            reference: curve(
                &values(outputs, "app_rt")?,
                &values(outputs, "app_r")?,
                false,
            ),
            disturbance: curve(
                &values(outputs, "app_dt")?,
                &values(outputs, "app_d")?,
                false,
            ),
            magnitude: curve(&w, &values(outputs, "app_db")?, true),
            phase: curve(&w, &values(outputs, "app_ph")?, true),
            effort: (!effort_y.is_empty())
                .then(|| values(outputs, "app_ut").map(|t| curve(&t, &effort_y, false)))
                .transpose()?,
            rise: scalar(outputs, "app_rise")?,
            settling: scalar(outputs, "app_settle")?,
            overshoot: scalar(outputs, "app_over")?,
            peak: scalar(outputs, "app_peak")?,
            gain_margin: scalar(outputs, "app_gm")?,
            phase_margin: scalar(outputs, "app_pm")?,
            phase_crossover: scalar(outputs, "app_wcg")?,
            gain_crossover: scalar(outputs, "app_wcp")?,
            stable: scalar(outputs, "app_stable")? != 0.,
        })
    }
}

/// Coefficients typed as `1 3 3 1`, `[1, 3, 3, 1]` or `[1;3]`.
pub fn parse_coefficients(text: &str, what: &str) -> Result<Vec<f64>, String> {
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
        .ok_or_else(|| format!("{what}: type numbers, for example 1 3 3 1"))?;
    if values.is_empty() || values.iter().all(|v| *v == 0.) {
        return Err(format!("{what}: needs at least one nonzero coefficient"));
    }
    if values.len() > 64 {
        return Err(format!("{what}: at most 64 coefficients"));
    }
    Ok(values)
}
fn vector(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(|v| number(*v)).collect();
    format!("[{}]", items.join(" "))
}

pub struct PidTuner {
    /// The plant is a workspace variable rather than typed coefficients.
    pub use_variable: bool,
    pub numerator: String,
    pub denominator: String,
    pub variable: String,
    pub kind: Kind,
    /// Response time slider: crossover = wc0 * factor.
    pub factor: f64,
    /// Robustness slider: target phase margin.
    pub margin: f64,
    /// Crossover of the initial design (default pidtune) for this plant and type.
    pub wc0: Option<f64>,
    pub view: View,
    pub controller_name: String,
    pub controller_edited: bool,
    pub plant_name: String,
    pub plant_edited: bool,
    /// Last good design; errors keep it on screen.
    pub current: Option<Design>,
    /// The design before the last slider move, drawn dashed.
    pub previous: Option<Design>,
    identity: Option<String>,
}
impl Default for PidTuner {
    fn default() -> Self {
        Self {
            use_variable: false,
            numerator: "1".into(),
            denominator: "1 3 3 1".into(),
            variable: String::new(),
            kind: Kind::PID,
            factor: 1.,
            margin: DEFAULT_MARGIN,
            wc0: None,
            view: View::Reference,
            controller_name: String::new(),
            controller_edited: false,
            plant_name: String::new(),
            plant_edited: false,
            current: None,
            previous: None,
            identity: None,
        }
    }
}

impl PidTuner {
    /// The initial design: the default pidtune call.
    pub fn initial(&self) -> bool {
        self.factor == 1. && self.margin == DEFAULT_MARGIN
    }
    /// The crossover the sliders ask for, once the initial design is known.
    pub fn crossover(&self) -> Option<f64> {
        self.wc0.map(|wc0| wc0 * self.factor)
    }
    /// Reset: back to the initial design; the old one stays for comparison.
    pub fn reset(&mut self) {
        if !self.initial() {
            self.previous = self.current.clone();
        }
        self.factor = 1.;
        self.margin = DEFAULT_MARGIN;
    }
    fn default_plant_name(&self, ws: &Snapshot) -> String {
        unused_name("G", |n| ws.contains(n))
    }
    /// The plant's name in the script and its definition line (typed plants).
    pub fn plant(&self, ws: &Snapshot) -> Result<(String, Option<String>), String> {
        if self.use_variable {
            if self.variable.is_empty() {
                return Err(
                    "PID Tuner: choose a workspace variable holding a tf, zpk or ss model".into(),
                );
            }
            if !ws.contains(&self.variable) {
                return Err(format!(
                    "PID Tuner: workspace variable {} does not exist",
                    self.variable
                ));
            }
            return Ok((self.variable.clone(), None));
        }
        let num = parse_coefficients(&self.numerator, "Numerator")?;
        let den = parse_coefficients(&self.denominator, "Denominator")?;
        let name = if self.plant_edited {
            self.plant_name.trim().to_owned()
        } else {
            self.default_plant_name(ws)
        };
        super::check_export_name(&name)?;
        if !matrust::files::valid_name(&name) {
            return Err("PID Tuner: the plant name is not a valid variable name".into());
        }
        Ok((
            name.clone(),
            Some(format!(
                "{name} = tf({}, {});\n",
                vector(&num),
                vector(&den)
            )),
        ))
    }
    /// The controller's output name: the user's, else `C`, `C1`, ... unused.
    pub fn controller(&self, ws: &Snapshot) -> Result<String, String> {
        let plant = self.plant(ws).map(|(name, _)| name).unwrap_or_default();
        let name = if self.controller_edited {
            self.controller_name.trim().to_owned()
        } else {
            unused_name("C", |n| ws.contains(n) || n == plant)
        };
        super::check_export_name(&name)?;
        if !matrust::files::valid_name(&name) {
            return Err("PID Tuner: the controller name is not a valid variable name".into());
        }
        if name == plant {
            return Err("PID Tuner: the controller and the plant need different names".into());
        }
        Ok(name)
    }
    /// pidtune's arguments after the type.
    fn tune_arguments(&self) -> Result<String, String> {
        if self.initial() {
            return Ok(String::new());
        }
        let wc = self
            .crossover()
            .ok_or("PID Tuner: the initial design is not known yet")?;
        Ok(if self.kind.one_gain() || self.margin == DEFAULT_MARGIN {
            format!(", {}", number(wc))
        } else {
            format!(
                ", {}, pidtuneOptions('PhaseMargin', {})",
                number(wc),
                number(self.margin)
            )
        })
    }
    /// Plant, type and the plant variable's value: a change starts a new
    /// initial design.
    fn identity(&self, ws: &Snapshot) -> String {
        if self.use_variable {
            format!(
                "{}|var|{}|{:?}",
                self.kind.name(),
                self.variable,
                ws.version(&self.variable)
            )
        } else {
            format!(
                "{}|tf|{:?}|{:?}",
                self.kind.name(),
                parse_coefficients(&self.numerator, ""),
                parse_coefficients(&self.denominator, "")
            )
        }
    }
    /// Remember the design a slider move starts from.
    fn slider_moved(&mut self, response: &egui::Response) {
        if response.drag_started() || (response.changed() && !response.dragged()) {
            self.previous = self.current.clone();
        }
    }
}

impl App for PidTuner {
    fn kind(&self) -> AppKind {
        AppKind::PidTuner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let (plant, definition) = self.plant(ws)?;
        let controller = self.controller(ws)?;
        let mut script = match definition {
            Some(line) => line,
            None => format!("% The plant is the workspace variable {plant}.\n"),
        };
        script.push_str(&format!(
            "{controller} = pidtune({plant}, '{}'{});\n\
             step(feedback({controller}*{plant}, 1))\n",
            self.kind.name(),
            self.tune_arguments()?
        ));
        Ok(script)
    }
    fn inputs(&self) -> Vec<String> {
        if self.use_variable && !self.variable.is_empty() {
            vec![self.variable.clone()]
        } else {
            Vec::new()
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let (Ok((plant, _)), Ok(controller), Ok(arguments)) =
            (self.plant(ws), self.controller(ws), self.tune_arguments())
        else {
            return (String::new(), Vec::new());
        };
        // Private copies first, so no probe name can shadow the user's names.
        let code = format!(
            "[app_G, app_C] = deal({plant}, {controller});\n\
             [~, app_info] = pidtune(app_G, '{kind}'{arguments});\n\
             app_wc = app_info.CrossoverFrequency;\n\
             app_kp = app_C.Kp; app_ki = app_C.Ki; app_kd = app_C.Kd; app_tf = app_C.Tf;\n\
             app_T = feedback(app_C*app_G, 1);\n\
             [app_r, app_rt] = step(app_T);\n\
             [app_d, app_dt] = step(feedback(app_G, app_C));\n\
             [app_mag, app_ph, app_w] = bode(app_C*app_G);\n\
             app_db = 20*log10(app_mag(:)); app_ph = app_ph(:); app_w = app_w(:);\n\
             app_S = stepinfo(app_T);\n\
             app_rise = app_S.RiseTime; app_settle = app_S.SettlingTime;\n\
             app_over = app_S.Overshoot; app_peak = app_S.Peak;\n\
             [app_gm, app_pm, app_wcg, app_wcp] = margin(app_C*app_G);\n\
             app_stable = double(isstable(app_T));\n\
             try\n\
             [app_u, app_ut] = step(feedback(app_C, app_G));\n\
             catch\n\
             app_u = []; app_ut = [];\n\
             end\n",
            kind = self.kind.name(),
        );
        (code, PROBE.iter().map(|s| (*s).to_owned()).collect())
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let identity = self.identity(ws);
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.factor = 1.;
            self.margin = DEFAULT_MARGIN;
            self.wc0 = None;
            self.previous = None;
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let design = Design::from_outputs(outputs)?;
        if self.initial() {
            self.wc0 = Some(design.wc);
        }
        self.current = Some(design);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        field(ui, "Plant", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, !self.use_variable, "Transfer function").clicked() {
                    self.use_variable = false;
                }
                if crate::tokens::toggle(ui, self.use_variable, "Workspace").clicked() {
                    self.use_variable = true;
                }
            });
        });
        if self.use_variable {
            let models: Vec<(String, &str)> = ws
                .lti_models()
                .map(|(name, class)| (name.to_owned(), class))
                .collect();
            if self.variable.is_empty()
                && let Some((name, _)) = models.first()
            {
                self.variable = name.clone();
            }
            field(ui, "Model variable", |ui| {
                let shown = if self.variable.is_empty() {
                    "Choose a model".to_owned()
                } else {
                    self.variable.clone()
                };
                egui::ComboBox::from_id_salt("pid_tuner_plant_variable")
                    .width(ui.available_width())
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for (name, class) in &models {
                            ui.selectable_value(
                                &mut self.variable,
                                name.clone(),
                                format!("{name} ({class})"),
                            );
                        }
                    });
                if models.is_empty() {
                    crate::tokens::helper(ui, "No tf, zpk or ss variables in the workspace");
                }
            });
        } else {
            let width = ui.available_width();
            field(ui, "Numerator", |ui| {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(&mut self.numerator))
                        .desired_width(width),
                );
            });
            field(ui, "Denominator", |ui| {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(&mut self.denominator))
                        .desired_width(width),
                );
                crate::tokens::helper(ui, "Coefficients in descending powers of s");
            });
        }
        field(ui, "Controller type", |ui| {
            egui::ComboBox::from_id_salt("pid_tuner_type")
                .width(ui.available_width())
                .selected_text(self.kind.name())
                .show_ui(ui, |ui| {
                    for kind in Kind::ALL {
                        ui.selectable_value(&mut self.kind, kind, kind.name());
                    }
                });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        let known = self.wc0.is_some();
        field(ui, "Response time", |ui| {
            let response = ui.add_enabled(
                known,
                egui::Slider::new(&mut self.factor, FACTOR)
                    .logarithmic(true)
                    .show_value(false),
            );
            self.slider_moved(&response);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Slower").color(p.text_muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new("Faster").color(p.text_muted));
                });
            });
            crate::tokens::helper(
                ui,
                match self.crossover() {
                    Some(wc) => format!("Crossover {} rad/s (×{:.2})", short(wc), self.factor),
                    None => "Waiting for the initial design".into(),
                },
            );
        });
        field(ui, "Robustness", |ui| {
            let response = ui.add_enabled(
                known && !self.kind.one_gain(),
                egui::Slider::new(&mut self.margin, MARGIN)
                    .step_by(1.)
                    .show_value(false),
            );
            self.slider_moved(&response);
            crate::tokens::helper(
                ui,
                if self.kind.one_gain() {
                    format!(
                        "{} control has one gain: only the response time can move",
                        self.kind.name()
                    )
                } else {
                    format!("Phase margin {}°", number(self.margin))
                },
            );
        });
        if ui
            .add_enabled(
                !self.initial(),
                egui::Button::new("Reset")
                    .min_size(egui::vec2(ui.available_width(), crate::tokens::CONTROL)),
            )
            .clicked()
        {
            self.reset();
        }
    }
    fn main_caption(&self) -> &'static str {
        "RESPONSE"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                let tab = crate::tokens::tab(ui, self.view == view, view.label());
                if view == View::Reference {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::PidTuner),
                    );
                }
                if tab.clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(design) = &self.current else {
            crate::tokens::helper(ui, "No design yet: the status line says why.");
            return;
        };
        let previous = self.previous.as_ref().filter(|p| !p.same_as(design));
        crate::tokens::helper(
            ui,
            if previous.is_some() {
                "Solid: this design. Dashed: the design before the last slider move."
            } else {
                "Solid: this design."
            },
        );
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        match self.view {
            View::Reference => plot(
                ui,
                "pid_reference",
                height,
                ("Time (s)", "Output"),
                &design.reference,
                previous.map(|p| p.reference.as_slice()),
                false,
            ),
            View::Disturbance => plot(
                ui,
                "pid_disturbance",
                height,
                ("Time (s)", "Output"),
                &design.disturbance,
                previous.map(|p| p.disturbance.as_slice()),
                false,
            ),
            View::Bode => {
                let half = ((height - crate::tokens::S) / 2.).max(80.);
                plot(
                    ui,
                    "pid_bode_magnitude",
                    half,
                    ("Frequency (rad/s)", "Magnitude (dB)"),
                    &design.magnitude,
                    previous.map(|p| p.magnitude.as_slice()),
                    true,
                );
                ui.add_space(crate::tokens::S);
                plot(
                    ui,
                    "pid_bode_phase",
                    half,
                    ("Frequency (rad/s)", "Phase (deg)"),
                    &design.phase,
                    previous.map(|p| p.phase.as_slice()),
                    true,
                );
            }
            View::Effort => match &design.effort {
                Some(effort) => plot(
                    ui,
                    "pid_effort",
                    height,
                    ("Time (s)", "Controller output"),
                    effort,
                    previous.and_then(|p| p.effort.as_deref()),
                    false,
                ),
                None => {
                    crate::tokens::helper(
                        ui,
                        "A PD or PID controller has a pure derivative, so its output to a step \
                         is not a finite signal. Choose PDF or PIDF to see the effort.",
                    );
                }
            },
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(d) = &self.current else {
            return ["Kp", "Ki", "Kd", "Tf"]
                .into_iter()
                .map(|label| (label, "—".to_owned()))
                .collect();
        };
        // Finite margins name the frequency where the loop phase is -180.
        let gain_margin = if d.gain_margin.is_finite() && d.gain_margin > 0. {
            format!(
                "{} ({} dB) at {} rad/s",
                short(d.gain_margin),
                short(20. * d.gain_margin.log10()),
                short(d.phase_crossover)
            )
        } else {
            short(d.gain_margin)
        };
        vec![
            ("Kp", short(d.gains[0])),
            ("Ki", short(d.gains[1])),
            ("Kd", short(d.gains[2])),
            ("Tf", short(d.gains[3])),
            ("Rise time", format!("{} s", short(d.rise))),
            ("Settling time", format!("{} s", short(d.settling))),
            ("Overshoot", format!("{} %", short(d.overshoot))),
            ("Peak", short(d.peak)),
            ("Gain margin", gain_margin),
            ("Phase margin", format!("{}°", short(d.phase_margin))),
            ("Crossover", format!("{} rad/s", short(d.gain_crossover))),
            (
                "Closed loop stable",
                if d.stable { "yes" } else { "no" }.to_owned(),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if !self.controller_edited {
            self.controller_name = self.controller(ws).unwrap_or_else(|_| "C".into());
        }
        let before = self.controller_name.clone();
        name_field(ui, "Controller name", &mut self.controller_name, ws);
        self.controller_edited |= self.controller_name != before;
        if !self.use_variable {
            if !self.plant_edited {
                self.plant_name = self.default_plant_name(ws);
            }
            let before = self.plant_name.clone();
            name_field(ui, "Plant name", &mut self.plant_name, ws);
            self.plant_edited |= self.plant_name != before;
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        let mut names: Vec<String> = self.controller(ws).into_iter().collect();
        if let Ok((plant, Some(_))) = self.plant(ws) {
            names.push(plant);
        }
        names
    }
    fn summary(&self) -> Option<String> {
        let d = self.current.as_ref()?;
        Some(format!(
            "{} controller · crossover {} rad/s · phase margin {}° · closed loop {}",
            self.kind.name(),
            short(d.wc),
            short(d.phase_margin),
            if d.stable { "stable" } else { "unstable" }
        ))
    }
}

/// One response plot; the previous design is dashed and muted.
fn plot(
    ui: &mut egui::Ui,
    id: &str,
    height: f32,
    (x_label, y_label): (&str, &str),
    current: &[[f64; 2]],
    previous: Option<&[[f64; 2]]>,
    log_x: bool,
) {
    let p = crate::tokens::palette(ui.ctx());
    let [r, g, b] = matrust::plot::COLORS[0];
    let trace = egui::Color32::from_rgb(r, g, b);
    let mut chart = egui_plot::Plot::new(id.to_owned())
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x_label.to_owned())
        .y_axis_label(y_label.to_owned())
        .show_background(false);
    if log_x {
        // x is log10(w); the marks read in rad/s.
        chart = chart.x_axis_formatter(|mark, _| short(10_f64.powf(mark.value)));
    }
    chart.show(ui, |plot_ui| {
        if let Some(previous) = previous {
            plot_ui.line(
                egui_plot::Line::new("Previous design", previous.to_vec())
                    .color(p.text_muted)
                    .style(egui_plot::LineStyle::dashed_dense())
                    .width(1.),
            );
        }
        plot_ui.line(
            egui_plot::Line::new("This design", current.to_vec())
                .color(trace)
                .width(1.5),
        );
    });
}
