//! Probability Distributions: one script behind the preview, export and save.
//! Parameter defaults follow the public makedist function reference. No numerical
//! distribution kernels live here: the private Engine computes every plotted value.
use super::{App, AppKind, Host, Outputs, Snapshot, field, name_field, number, short};
use crate::toolkits::Action;
use eframe::egui;

const MAX_POINTS: usize = 4096;
pub const MAX_SAMPLES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Normal,
    Uniform,
    Exponential,
    Gamma,
    Beta,
    Chisquare,
    T,
    F,
    Lognormal,
    Weibull,
    Rayleigh,
    ExtremeValue,
    Logistic,
    Loglogistic,
    Rician,
    Nakagami,
    GeneralizedExtremeValue,
    GeneralizedPareto,
    TLocationScale,
    InverseGaussian,
    BirnbaumSaunders,
    HalfNormal,
    Binomial,
    Poisson,
    Geometric,
    NegativeBinomial,
}
#[derive(Clone, Copy)]
pub enum Domain {
    Real,
    Positive,
    Nonnegative,
    Probability,
    Success,
    Integer,
}
#[derive(Clone, Copy)]
pub struct Parameter {
    pub name: &'static str,
    pub default: f64,
    pub domain: Domain,
}
macro_rules! param {
    ($name:expr, $default:expr, $domain:expr) => {
        Parameter {
            name: $name,
            default: $default,
            domain: $domain,
        }
    };
}
impl Family {
    pub const ALL: [Self; 26] = [
        Self::Normal,
        Self::Uniform,
        Self::Exponential,
        Self::Gamma,
        Self::Beta,
        Self::Chisquare,
        Self::T,
        Self::F,
        Self::Lognormal,
        Self::Weibull,
        Self::Rayleigh,
        Self::ExtremeValue,
        Self::Logistic,
        Self::Loglogistic,
        Self::Rician,
        Self::Nakagami,
        Self::GeneralizedExtremeValue,
        Self::GeneralizedPareto,
        Self::TLocationScale,
        Self::InverseGaussian,
        Self::BirnbaumSaunders,
        Self::HalfNormal,
        Self::Binomial,
        Self::Poisson,
        Self::Geometric,
        Self::NegativeBinomial,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Uniform => "Uniform",
            Self::Exponential => "Exponential",
            Self::Gamma => "Gamma",
            Self::Beta => "Beta",
            Self::Chisquare => "Chisquare",
            Self::T => "T",
            Self::F => "F",
            Self::Lognormal => "Lognormal",
            Self::Weibull => "Weibull",
            Self::Rayleigh => "Rayleigh",
            Self::ExtremeValue => "ExtremeValue",
            Self::Logistic => "Logistic",
            Self::Loglogistic => "Loglogistic",
            Self::Rician => "Rician",
            Self::Nakagami => "Nakagami",
            Self::GeneralizedExtremeValue => "GeneralizedExtremeValue",
            Self::GeneralizedPareto => "GeneralizedPareto",
            Self::TLocationScale => "tLocationScale",
            Self::InverseGaussian => "InverseGaussian",
            Self::BirnbaumSaunders => "BirnbaumSaunders",
            Self::HalfNormal => "HalfNormal",
            Self::Binomial => "Binomial",
            Self::Poisson => "Poisson",
            Self::Geometric => "Geometric",
            Self::NegativeBinomial => "NegativeBinomial",
        }
    }
    pub fn discrete(self) -> bool {
        matches!(
            self,
            Self::Binomial | Self::Poisson | Self::Geometric | Self::NegativeBinomial
        )
    }
    pub fn parameters(self) -> &'static [Parameter] {
        use Domain::*;
        match self {
            Self::Normal
            | Self::Lognormal
            | Self::ExtremeValue
            | Self::Logistic
            | Self::Loglogistic
            | Self::HalfNormal => &[param!("mu", 0., Real), param!("sigma", 1., Positive)],
            Self::Uniform => &[param!("Lower", 0., Real), param!("Upper", 1., Real)],
            Self::Exponential => &[param!("mu", 1., Positive)],
            Self::Gamma | Self::Beta => &[param!("a", 1., Positive), param!("b", 1., Positive)],
            Self::Chisquare | Self::T => &[param!("nu", 1., Positive)],
            Self::F => &[param!("nu1", 1., Positive), param!("nu2", 1., Positive)],
            Self::Weibull => &[param!("A", 1., Positive), param!("B", 1., Positive)],
            Self::Rayleigh => &[param!("B", 1., Positive)],
            Self::Rician => &[param!("s", 1., Nonnegative), param!("sigma", 1., Positive)],
            Self::Nakagami => &[param!("mu", 1., Positive), param!("omega", 1., Positive)],
            Self::GeneralizedExtremeValue => &[
                param!("k", 0., Real),
                param!("sigma", 1., Positive),
                param!("mu", 0., Real),
            ],
            Self::GeneralizedPareto => &[
                param!("k", 1., Real),
                param!("sigma", 1., Positive),
                param!("theta", 1., Real),
            ],
            Self::TLocationScale => &[
                param!("mu", 0., Real),
                param!("sigma", 1., Positive),
                param!("nu", 1., Positive),
            ],
            Self::InverseGaussian => &[param!("mu", 1., Positive), param!("lambda", 1., Positive)],
            Self::BirnbaumSaunders => {
                &[param!("beta", 1., Positive), param!("gamma", 1., Positive)]
            }
            Self::Binomial => &[param!("N", 1., Integer), param!("p", 0.5, Probability)],
            Self::Poisson => &[param!("lambda", 1., Nonnegative)],
            Self::Geometric => &[param!("p", 0.5, Success)],
            Self::NegativeBinomial => &[param!("R", 1., Positive), param!("P", 0.5, Success)],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Explore,
    Samples,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    Pdf,
    Cdf,
}
#[derive(Clone, Debug)]
pub struct Control {
    pub value: String,
    pub lower: f64,
    pub upper: f64,
}
#[derive(Clone, Debug)]
pub struct Shown {
    pub family: Family,
    pub mode: Mode,
    pub function: Function,
    pub curve: Vec<[f64; 2]>,
    pub stats: Vec<f64>,
    pub edges: Vec<f64>,
    pub density: Vec<f64>,
    pub probe: f64,
}
pub struct DistributionExplorer {
    pub family: Family,
    pub parameters: Vec<Control>,
    pub mode: Mode,
    pub function: Function,
    pub probe_x: String,
    pub probe_p: String,
    pub n: String,
    pub seed: String,
    /// Empty means automatic.
    pub bins: String,
    pub object: String,
    pub samples: String,
    pub shown: Option<Shown>,
    stopped: bool,
    stop_requested: bool,
    running: bool,
    dragging_probe: bool,
    #[cfg(test)]
    pub probe_screen: Option<egui::Pos2>,
}
impl Default for DistributionExplorer {
    fn default() -> Self {
        let mut app = Self {
            family: Family::Normal,
            parameters: Vec::new(),
            mode: Mode::Explore,
            function: Function::Pdf,
            probe_x: "0".into(),
            probe_p: "0.5".into(),
            n: "1000".into(),
            seed: "0".into(),
            bins: String::new(),
            object: "pd".into(),
            samples: "x".into(),
            shown: None,
            stopped: false,
            stop_requested: false,
            running: false,
            dragging_probe: false,
            #[cfg(test)]
            probe_screen: None,
        };
        app.set_family(Family::Normal);
        app
    }
}
/// Numeric fields accept finite decimal/scientific literals. Parse the real
/// assignment context first, even for fields subsequently restricted to numbers.
fn numeric(text: &str, label: &str) -> Result<f64, String> {
    if text.len() > 64 {
        return Err(format!("{label}: use at most 64 characters"));
    }
    super::single_expression(text).map_err(|e| format!("{label}: {e}"))?;
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{label}: enter a finite number"))
}
fn integer(text: &str, label: &str, low: usize, high: usize) -> Result<usize, String> {
    let v = numeric(text, label)?;
    if v.fract() != 0. || v < low as f64 || v > high as f64 {
        return Err(format!("{label}: enter an integer from {low} to {high}"));
    }
    Ok(v as usize)
}
fn valid_parameter(p: Parameter, text: &str) -> Result<f64, String> {
    let v = numeric(text, p.name)?;
    let (ok, rule) = match p.domain {
        Domain::Real => (true, "finite"),
        Domain::Positive => (v > 0., "greater than zero"),
        Domain::Nonnegative => (v >= 0., "zero or greater"),
        Domain::Probability => ((0. ..=1.).contains(&v), "between 0 and 1"),
        Domain::Success => (v > 0. && v <= 1., "greater than 0 and at most 1"),
        Domain::Integer => (
            v >= 0. && v.fract() == 0. && v < 9_007_199_254_740_992.,
            "a nonnegative integer below 2^53",
        ),
    };
    if !ok {
        return Err(format!("{} must be {rule}", p.name));
    }
    Ok(v)
}
impl DistributionExplorer {
    pub fn set_family(&mut self, family: Family) {
        self.family = family;
        self.parameters = family
            .parameters()
            .iter()
            .map(|p| {
                let (lower, upper) = match p.domain {
                    Domain::Real => (-5., 5.),
                    Domain::Positive => (0.01, 10.),
                    Domain::Nonnegative | Domain::Integer => (0., 10.),
                    Domain::Probability => (0., 1.),
                    Domain::Success => (0.01, 1.),
                };
                Control {
                    value: number(p.default),
                    lower,
                    upper,
                }
            })
            .collect();
        self.stopped = false;
    }
    fn names(&self) -> Result<(&str, &str), String> {
        let pd = self.object.trim();
        let x = self.samples.trim();
        for name in std::iter::once(pd).chain((self.mode == Mode::Samples).then_some(x)) {
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) || name.starts_with("app_") {
                return Err("Use valid export names outside the app_ prefix".into());
            }
            let statements = matrust::syntax::parse(&format!("{name} = 0;"))
                .map_err(|_| "Use a variable name, not a language keyword".to_owned())?;
            if !matches!(statements.as_slice(), [statement] if matches!(
                &statement.kind, matrust::syntax::StatementKind::Evaluate(e)
                    if e.targets == [Some(name.to_owned())] && e.quiet))
            {
                return Err("Use a variable name, not a language keyword".into());
            }
            // Avoid shadowing any builtin called by the generated script.
            if matrust::session::FUNCTIONS.contains(&name) {
                return Err(format!(
                    "{name} is a function name; choose another export name"
                ));
            }
        }
        if self.mode == Mode::Samples && pd == x {
            return Err("Distribution and sample names must differ".into());
        }
        Ok((pd, x))
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.stop_requested = true;
    }
    pub fn resume(&mut self) {
        self.stopped = false;
    }
    /// The sole script builder. Probe lines only read these results.
    fn build_script(&self) -> Result<String, String> {
        if self.stopped {
            return Err("Stopped. Press Run to compute again.".into());
        }
        let (pd, x) = self.names()?;
        if self.parameters.len() != self.family.parameters().len() {
            return Err("Reset the distribution parameters".into());
        }
        let mut params = Vec::new();
        let mut args = String::new();
        for (p, c) in self.family.parameters().iter().zip(&self.parameters) {
            let v = valid_parameter(*p, &c.value)?;
            if !c.lower.is_finite() || !c.upper.is_finite() || c.lower >= c.upper {
                return Err(format!(
                    "{}: slider lower limit must be below upper limit",
                    p.name
                ));
            }
            params.push(v);
            args += &format!(", '{}', {}", p.name, number(v));
        }
        if self.family == Family::Uniform && params[0] >= params[1] {
            return Err("Uniform: Lower must be less than Upper".into());
        }
        let px = numeric(&self.probe_x, "Probe x")?;
        let pp = numeric(&self.probe_p, "Probe p")?;
        if !(0. ..=1.).contains(&pp) {
            return Err("Probe p must be between 0 and 1".into());
        }
        let mut s = format!("{pd} = makedist('{}'{args});\n", self.family.name());
        s += &format!("app_lo = icdf({pd}, 0.001); app_hi = icdf({pd}, 0.999);\n");
        if self.family == Family::Binomial {
            s += &format!("app_lo = 0; app_hi = {};\n", number(params[0]));
        } else if self.family.discrete() {
            s += "app_lo = 0;\n";
        }
        s += "if ~isfinite(app_lo) || ~isfinite(app_hi), error('No finite plotting range for these parameters'); end\n\
              if app_hi <= app_lo, app_lo = app_lo - 0.5; app_hi = app_hi + 0.5; end\n";
        if self.family.discrete() {
            s += &format!(
                "if ceil(app_hi)-floor(app_lo)+1 > {MAX_POINTS}, error('Discrete support exceeds the 4096-point plot limit'); end\napp_grid = floor(app_lo):ceil(app_hi);\n"
            );
        } else {
            s += "app_grid = linspace(app_lo, app_hi, 401);\n";
        }
        let function = if self.mode == Mode::Explore && self.function == Function::Cdf {
            "cdf"
        } else {
            "pdf"
        };
        s += &format!(
            "app_curve = {function}({pd}, app_grid);\n\
             app_stats = [mean({pd}), var({pd}), std({pd}), median({pd}), pdf({pd}, {}), cdf({pd}, {}), icdf({pd}, {})];\n",
            number(px),
            number(px),
            number(pp)
        );
        s += "figure;\n";
        if self.mode == Mode::Samples {
            let n = integer(&self.n, "Sample size", 1, MAX_SAMPLES)?;
            let seed = integer(&self.seed, "Seed", 0, u32::MAX as usize)?;
            let bins = if self.bins.trim().is_empty() {
                String::new()
            } else {
                format!(", {}", integer(&self.bins, "Bins", 1, 1024)?)
            };
            s += &format!(
                "rng({seed}); {x} = random({pd}, {n}, 1);\n\
                histogram({x}{bins}, 'Normalization', 'pdf'); hold on;\n\
                [app_density, app_edges] = histcounts({x}{bins}, 'Normalization', 'pdf');\n\
                app_stats = [mean({x}), std({x}), min({x}), max({x}), mean({pd}), std({pd})];\n"
            );
        } else {
            s += "app_edges = []; app_density = [];\n";
        }
        let plot = if self.family.discrete() {
            if function == "cdf" { "stairs" } else { "stem" }
        } else {
            "plot"
        };
        s += &format!("{plot}(app_grid, app_curve);\nxlabel('x'); ylabel('{function}');\n");
        if self.mode == Mode::Explore {
            s += &format!(
                "hold on; plot([{} {}], [0 max(app_curve)]); hold off;\n",
                number(px),
                number(px)
            );
        } else {
            s += "hold off;\n";
        }
        Ok(s)
    }
}
fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(matrust::Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Missing numeric result {name}")),
    }
}
impl App for DistributionExplorer {
    fn kind(&self) -> AppKind {
        AppKind::DistributionExplorer
    }
    fn script(&self, _ws: &Snapshot) -> Result<String, String> {
        self.build_script()
    }
    fn inputs(&self) -> Vec<String> {
        Vec::new()
    }
    fn probe(&self, _ws: &Snapshot) -> (String, Vec<String>) {
        (
            String::new(),
            [
                "app_grid",
                "app_curve",
                "app_stats",
                "app_edges",
                "app_density",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let grid = values(outputs, "app_grid")?;
        let curve = values(outputs, "app_curve")?;
        if grid.len() != curve.len() || grid.len() > MAX_POINTS {
            return Err("Invalid plot size".into());
        }
        let shown = Shown {
            family: self.family,
            mode: self.mode,
            function: self.function,
            curve: grid.into_iter().zip(curve).map(|(x, y)| [x, y]).collect(),
            stats: values(outputs, "app_stats")?,
            edges: values(outputs, "app_edges")?,
            density: values(outputs, "app_density")?,
            probe: numeric(&self.probe_x, "Probe x")?,
        };
        self.shown = Some(shown);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        let mut family = self.family;
        field(ui, "Distribution family", |ui| {
            egui::ComboBox::from_id_salt("distribution_explorer_family")
                .selected_text(family.name())
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for f in Family::ALL {
                        ui.selectable_value(&mut family, f, f.name());
                    }
                });
        });
        if family != self.family {
            self.set_family(family);
        }
        for (i, (p, c)) in self
            .family
            .parameters()
            .iter()
            .zip(&mut self.parameters)
            .enumerate()
        {
            ui.push_id(i, |ui| {
                field(ui, p.name, |ui| {
                    text(ui, &mut c.value, "number");
                    if let Ok(mut v) = valid_parameter(*p, &c.value) {
                        ui.spacing_mut().slider_width = ui.available_width();
                        if c.lower.is_finite() && c.upper.is_finite() && c.lower < c.upper {
                            let mut slider = egui::Slider::new(&mut v, c.lower..=c.upper)
                                .show_value(false)
                                .clamping(egui::SliderClamping::Never);
                            if matches!(p.domain, Domain::Integer) {
                                slider = slider.integer();
                            }
                            if ui.add(slider).changed() {
                                c.value = number(v);
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Lower");
                        ui.add(egui::DragValue::new(&mut c.lower).speed(0.1));
                        ui.label("Upper");
                        ui.add(egui::DragValue::new(&mut c.upper).speed(0.1));
                    });
                    super::expression_message(ui, valid_parameter(*p, &c.value).map(|_| ()));
                })
            });
        }
        crate::tokens::helper(ui, "Truncation is unavailable in this engine.");
        ui.add_space(crate::tokens::S);
        field(ui, "Mode", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, Mode::Explore, "Explore");
                ui.selectable_value(&mut self.mode, Mode::Samples, "Random samples");
            });
        });
        match self.mode {
            Mode::Explore => {
                field(ui, "Function", |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.function, Function::Pdf, "pdf");
                        ui.selectable_value(&mut self.function, Function::Cdf, "cdf");
                    });
                });
                field(ui, "Probe x", |ui| text(ui, &mut self.probe_x, "number"));
                if self.function == Function::Cdf {
                    field(ui, "Inverse probability p", |ui| {
                        text(ui, &mut self.probe_p, "0 to 1")
                    });
                }
            }
            Mode::Samples => {
                field(ui, "Sample size n (1 to 1,000,000)", |ui| {
                    text(ui, &mut self.n, "integer")
                });
                field(ui, "Seed", |ui| {
                    text(ui, &mut self.seed, "integer");
                    if ui.button("New seed").clicked() {
                        let seed = self.seed.trim().parse::<u32>().unwrap_or(0);
                        self.seed = seed.wrapping_add(1).to_string();
                    }
                });
                field(ui, "Histogram bins", |ui| {
                    text(ui, &mut self.bins, "automatic")
                });
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop();
            }
            if ui
                .add_enabled(self.stopped, egui::Button::new("Run"))
                .clicked()
            {
                self.resume();
            }
        });
    }
    fn main_caption(&self) -> &'static str {
        "DISTRIBUTION"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let Some(shown) = &self.shown else {
            crate::tokens::helper(ui, "Choose a distribution to explore.");
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        ui.label(format!(
            "{} · {}",
            shown.family.name(),
            if shown.mode == Mode::Samples {
                "Random samples"
            } else if shown.function == Function::Pdf {
                "pdf"
            } else {
                "cdf"
            }
        ));
        if shown.mode == Mode::Explore {
            let value = shown.stats[if shown.function == Function::Pdf {
                4
            } else {
                5
            }];
            crate::tokens::helper(
                ui,
                format!(
                    "At x = {}: {} · Drag the probe line",
                    short(shown.probe),
                    short(value)
                ),
            );
        }
        let response = egui_plot::Plot::new("distribution_explorer_plot")
            .height(ui.available_height().max(160.))
            .allow_drag(false)
            .allow_scroll(false)
            .allow_zoom(false)
            .allow_boxed_zoom(false)
            .allow_axis_zoom_drag(false)
            .allow_double_click_reset(false)
            .auto_bounds(egui::Vec2b::TRUE)
            .show_background(false)
            .legend(egui_plot::Legend::default())
            .x_axis_label("x")
            .y_axis_label(
                if shown.mode == Mode::Explore && shown.function == Function::Cdf {
                    "cdf"
                } else {
                    "pdf"
                },
            )
            .show(ui, |plot_ui| {
                if shown.mode == Mode::Samples {
                    let bars = shown
                        .edges
                        .windows(2)
                        .zip(&shown.density)
                        .map(|(e, y)| {
                            egui_plot::Bar::new((e[0] + e[1]) / 2., *y).width(e[1] - e[0])
                        })
                        .collect();
                    plot_ui
                        .bar_chart(egui_plot::BarChart::new("Samples", bars).color(p.text_muted));
                }
                if shown.family.discrete() {
                    let mut points = Vec::new();
                    for (i, &[x, y]) in shown.curve.iter().enumerate() {
                        if shown.mode == Mode::Explore && shown.function == Function::Cdf {
                            if i > 0 {
                                points.push([x, shown.curve[i - 1][1]]);
                            }
                            points.push([x, y]);
                        } else {
                            points.extend([[x, 0.], [x, y], [f64::NAN, f64::NAN]]);
                        }
                    }
                    plot_ui.line(egui_plot::Line::new("Distribution", points).color(p.accent));
                } else {
                    plot_ui.line(
                        egui_plot::Line::new("Distribution", shown.curve.clone()).color(p.accent),
                    );
                }
                if shown.mode == Mode::Explore {
                    plot_ui.vline(egui_plot::VLine::new("Probe", shown.probe).color(p.text_muted));
                }
            });
        #[cfg(test)]
        {
            let mut point = response
                .transform
                .position_from_point(&egui_plot::PlotPoint::new(shown.probe, 0.));
            point.y = response.transform.frame().center().y;
            self.probe_screen = Some(point);
        }
        if response.response.drag_started() && shown.mode == Mode::Explore {
            let line = response
                .transform
                .position_from_point(&egui_plot::PlotPoint::new(shown.probe, 0.));
            self.dragging_probe = ui
                .input(|i| i.pointer.press_origin())
                .is_some_and(|pos| (pos.x - line.x).abs() <= 12.);
        }
        if self.dragging_probe
            && response.response.dragged()
            && let Some(pos) = response.response.interact_pointer_pos()
        {
            let at = response.transform.value_from_position(pos);
            if at.x.is_finite() {
                self.probe_x = number(at.x);
            }
        }
        if response.response.drag_stopped() {
            self.dragging_probe = false;
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(s) = &self.shown else {
            return Vec::new();
        };
        let labels: &[&str] = if s.mode == Mode::Explore {
            &[
                "Mean",
                "Variance",
                "Standard deviation",
                "Median",
                "pdf(x)",
                "cdf(x)",
                "icdf(p)",
            ]
        } else {
            &[
                "Sample mean",
                "Sample std",
                "Sample min",
                "Sample max",
                "Theoretical mean",
                "Theoretical std",
            ]
        };
        labels
            .iter()
            .zip(&s.stats)
            .map(|(label, v)| (*label, short(*v)))
            .collect()
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Distribution object", &mut self.object, ws);
        if self.mode == Mode::Samples {
            name_field(ui, "Sample vector", &mut self.samples, ws);
        }
        super::expression_message(ui, self.names().map(|_| ()));
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        let Ok((pd, x)) = self.names() else {
            return Vec::new();
        };
        std::iter::once(pd)
            .chain((self.mode == Mode::Samples).then_some(x))
            .map(str::to_owned)
            .collect()
    }
    fn summary(&self) -> Option<String> {
        self.shown
            .as_ref()
            .map(|s| format!("{}: {} plotted values", s.family.name(), s.curve.len()))
    }
}
fn text(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.add(
        crate::tokens::input(egui::TextEdit::singleline(text).char_limit(64))
            .hint_text(hint)
            .desired_width(ui.available_width()),
    );
}
pub(super) fn show(
    host: &mut Host<DistributionExplorer>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if host.app.stop_requested {
        host.runner.stop();
        host.app.stop_requested = false;
    }
}
