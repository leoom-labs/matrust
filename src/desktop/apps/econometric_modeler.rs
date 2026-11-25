//! Clean-room series preparation and diagnostics on the ordinary app framework.
//! The engine runs transforms, diagnostics, estimates and forecasts off the UI thread.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, short};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::{Array, Matrix};
use std::sync::{Arc, Mutex};

#[path = "econometric_modeler/models.rs"]
mod models;
#[path = "econometric_modeler/models_ui.rs"]
mod models_ui;
#[path = "econometric_modeler/script.rs"]
mod script;
pub use models::{ModelKind, ModelSpec, ModelView, parse_lags};
pub use models_ui::ModelControls;
pub use script::{
    Plan, SeriesSpec, Source, TestKind, TestSpec, Transform, build_script, checked_name, integer,
};

pub struct SeriesView {
    pub values: Matrix,
    pub nan_count: usize,
    pub points: Vec<[f64; 2]>,
}
pub struct Diagnostics {
    pub series: usize,
    pub acf: Vec<f64>,
    pub acf_lags: Vec<f64>,
    pub acf_bounds: Vec<f64>,
    pub pacf: Vec<f64>,
    pub pacf_lags: Vec<f64>,
    pub pacf_bounds: Vec<f64>,
}
pub struct ResultView {
    pub series: Vec<SeriesView>,
    pub diagnostics: Option<Diagnostics>,
    /// Each row is h, p, statistic, critical value, straight from the engine.
    pub tests: Vec<[f64; 4]>,
    pub models: Vec<ModelView>,
}
fn struct_field<'a>(a: &'a Array, name: &str) -> Result<&'a Array, String> {
    let Array::Struct(s) = a else {
        return Err("Missing econometric result record".into());
    };
    Ok(s.get(
        s.field_index(name)
            .ok_or_else(|| format!("Missing {name}"))?,
        0,
    )?
    .as_ref())
}
fn numbers(a: &Array) -> Result<Vec<f64>, String> {
    let Array::Numeric(m) = a else {
        return Err("Missing numeric diagnostic result".into());
    };
    Ok(m.data().to_vec())
}
fn decode(a: &Array) -> Result<ResultView, String> {
    let Array::Cell(c) = struct_field(a, "Series")? else {
        return Err("Missing series list".into());
    };
    let mut series = Vec::new();
    let mut total = 0usize;
    for i in 0..c.len() {
        let Array::Numeric(m) = c.get_linear(i)?.as_ref() else {
            return Err("Expected real numeric series".into());
        };
        total = total.saturating_add(m.len());
        if m.len() > script::MAX_POINTS || total > script::MAX_TOTAL {
            return Err("Series exceed observation limits".into());
        }
        let stride = m.len().div_ceil(2000).max(1);
        let points = m
            .data()
            .iter()
            .enumerate()
            .step_by(stride)
            .map(|(i, y)| [(i + 1) as f64, *y])
            .collect();
        series.push(SeriesView {
            values: m.clone(),
            nan_count: m.data().iter().filter(|v| v.is_nan()).count(),
            points,
        });
    }
    let d = struct_field(a, "Diagnostics")?;
    let diagnostics = if let Ok(index) = struct_field(d, "Series") {
        Some(Diagnostics {
            series: numbers(index)?[0] as usize - 1,
            acf: numbers(struct_field(d, "ACF")?)?,
            acf_lags: numbers(struct_field(d, "ACFLags")?)?,
            acf_bounds: numbers(struct_field(d, "ACFBounds")?)?,
            pacf: numbers(struct_field(d, "PACF")?)?,
            pacf_lags: numbers(struct_field(d, "PACFLags")?)?,
            pacf_bounds: numbers(struct_field(d, "PACFBounds")?)?,
        })
    } else {
        None
    };
    let Array::Cell(c) = struct_field(a, "Tests")? else {
        return Err("Missing test list".into());
    };
    let mut tests = Vec::new();
    for i in 0..c.len() {
        let t = c.get_linear(i)?;
        let mut row = [0.; 4];
        for (v, key) in row
            .iter_mut()
            .zip(["H", "PValue", "Statistic", "CriticalValue"])
        {
            let values = numbers(struct_field(t, key)?)?;
            if values.len() != 1 {
                return Err("Expected scalar test result".into());
            }
            *v = values[0];
        }
        tests.push(row);
    }
    Ok(ResultView {
        series,
        diagnostics,
        tests,
        models: models::decode_models(a)?,
    })
}

pub enum Command {
    Import(Source),
    Transform(Transform),
    Diagnostics,
    Test,
    Estimate,
    Forecast,
}
struct Trial {
    plan: Plan,
    inputs: Vec<(String, Array)>,
    versions: Vec<(String, Option<u64>)>,
    selected: usize,
    selected_model: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Diagnostics,
    Models,
}
pub struct EconometricModeler {
    pub source_text: String,
    pub plan: Plan,
    pub current: Option<ResultView>,
    pub selected: usize,
    pub num_lags: String,
    pub test_kind: TestKind,
    pub test_lags: String,
    pub alpha: String,
    pub model: String,
    pub statistic: String,
    pub trend: bool,
    pub dof: String,
    pub transform: usize,
    pub period: String,
    pub name: String,
    pub view: View,
    pub running: bool,
    pub controls: ModelControls,
    pub selected_model: usize,
    pub rename: String,
    pub model_names: [String; 3],
    inputs: Vec<(String, Array)>,
    versions: Vec<(String, Option<u64>)>,
    trial: Option<Trial>,
    serial: u64,
    pending: Arc<Mutex<Option<ResultView>>>,
    command: Option<Result<Command, String>>,
    edit_result: Option<Result<(), String>>,
    clear_requested: bool,
    stop_requested: bool,
}
impl Default for EconometricModeler {
    fn default() -> Self {
        Self {
            source_text: String::new(),
            plan: Plan::default(),
            current: None,
            selected: 0,
            num_lags: "20".into(),
            test_kind: TestKind::Adf,
            test_lags: "1".into(),
            alpha: "0.05".into(),
            model: "ARD".into(),
            statistic: "t1".into(),
            trend: true,
            dof: String::new(),
            transform: 0,
            period: "12".into(),
            name: "econ_results".into(),
            view: View::Diagnostics,
            running: false,
            controls: ModelControls::default(),
            selected_model: 0,
            rename: "Model1".into(),
            model_names: ["econ_models", "econ_residuals", "econ_forecasts"].map(str::to_owned),
            inputs: vec![],
            versions: vec![],
            trial: None,
            serial: 0,
            pending: Arc::default(),
            command: None,
            edit_result: None,
            clear_requested: false,
            stop_requested: false,
        }
    }
}
impl EconometricModeler {
    fn check_versions(&self, ws: &Snapshot) -> Result<(), String> {
        for (name, version) in &self.versions {
            let same = self
                .inputs
                .iter()
                .find(|(n, _)| n == name)
                .is_some_and(|(_, a)| ws.get(name).is_some_and(|b| a.same_snapshot(b)));
            if ws.version(name) != *version || !same {
                return Err(format!(
                    "{name} changed; clear the session and load the current data"
                ));
            }
        }
        Ok(())
    }
    pub fn test_spec(&self) -> Result<TestSpec, String> {
        let min = usize::from(matches!(
            self.test_kind,
            TestKind::LjungBox | TestKind::Arch
        ));
        let lags = integer(&self.test_lags, min, 256, "Test lags")?;
        if self.alpha.len() > 32 {
            return Err("Alpha must be a bounded numeric literal".into());
        }
        super::single_expression(&self.alpha).map_err(|e| format!("Alpha: {e}"))?;
        let alpha = self
            .alpha
            .trim()
            .parse::<f64>()
            .map_err(|_| "Alpha must be a numeric literal")?;
        let dof = if self.test_kind == TestKind::LjungBox && !self.dof.trim().is_empty() {
            Some(integer(&self.dof, 1, lags, "Degrees of freedom")?)
        } else {
            None
        };
        let s = TestSpec {
            kind: self.test_kind,
            series: self.selected,
            lags,
            alpha,
            model: self.model.clone(),
            statistic: if matches!(self.test_kind, TestKind::Adf | TestKind::Pp) {
                self.statistic.clone()
            } else {
                "t1".into()
            },
            trend: self.trend,
            dof,
        };
        s.validate(self.plan.series.len())?;
        Ok(s)
    }
    fn selected_transform(&self) -> Result<Transform, String> {
        match self.transform {
            0 => Ok(Transform::Log),
            1 => Ok(Transform::Difference),
            2 => Ok(Transform::Seasonal(integer(
                &self.period,
                1,
                256,
                "Seasonal period",
            )?)),
            3 => Ok(Transform::Detrend),
            _ => Err("Choose a supported transform".into()),
        }
    }
    /// Construct a candidate before submitting. Accepted data are untouched until
    /// the worker has executed and decoded the entire canonical script.
    pub fn start(&mut self, ws: &Snapshot, command: Command) -> Result<(), String> {
        self.check_versions(ws)?;
        let mut plan = self.plan.clone();
        let mut selected = self.selected;
        let mut selected_model = self.selected_model;
        match command {
            Command::Import(source) => {
                let source = Source::parse(&source.expression, ws)?;
                plan.series.push(SeriesSpec {
                    source,
                    transforms: vec![],
                });
                selected = plan.series.len() - 1;
            }
            Command::Transform(t) => {
                let mut s = plan
                    .series
                    .get(selected)
                    .ok_or("Load a series first")?
                    .clone();
                s.transforms.push(t);
                plan.series.push(s);
                selected = plan.series.len() - 1;
            }
            Command::Diagnostics => {
                if selected >= plan.series.len() {
                    return Err("Load a series first".into());
                }
                plan.diagnostics = Some((selected, integer(&self.num_lags, 1, 256, "NumLags")?));
            }
            Command::Test => plan.tests.push(self.test_spec()?),
            Command::Estimate => {
                let mut spec = self.controls.spec(self.selected, plan.series.len())?;
                spec.name =
                    super::unused_name("Model", |n| plan.models.iter().any(|m| m.name == n));
                selected_model = plan.models.len();
                plan.models.push(spec);
            }
            Command::Forecast => {
                let model = plan
                    .models
                    .get_mut(selected_model)
                    .ok_or("Estimate a model first")?;
                model.horizon = Some(integer(
                    &self.controls.horizon,
                    1,
                    1000,
                    "Forecast horizon",
                )?);
            }
        }
        build_script(&plan, "econ_preview")?;
        let mut bytes = 0usize;
        let inputs = plan
            .roots()
            .into_iter()
            .map(|name| {
                let a = ws
                    .get(&name)
                    .ok_or_else(|| format!("Missing source {name}"))?
                    .clone();
                bytes = bytes.saturating_add(a.bytes());
                if bytes > 128 * 1024 * 1024 {
                    return Err("Source workspace exceeds 128 MiB".into());
                }
                Ok((name, a))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let versions = inputs
            .iter()
            .map(|(n, _)| (n.clone(), ws.version(n)))
            .collect();
        self.trial = Some(Trial {
            plan,
            inputs,
            versions,
            selected,
            selected_model,
        });
        self.serial += 1;
        *self.pending.lock().map_err(|_| "Result lock failed")? = None;
        Ok(())
    }
}

impl App for EconometricModeler {
    fn kind(&self) -> AppKind {
        AppKind::EconometricModeler
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        self.check_versions(ws)?;
        self.output_script()
    }
    fn execution_script(&self, _: &Snapshot) -> Result<String, String> {
        if let Some(t) = &self.trial {
            build_script(&t.plan, "econ_preview")
        } else if !self.plan.series.is_empty() {
            build_script(&self.plan, "econ_preview")
        } else {
            Ok(String::new())
        }
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.script(ws)
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (String::new(), vec!["econ_preview".into()])
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, mut request: Request) {
        let Some(trial) = &self.trial else {
            runner.poll(ctx);
            return;
        };
        if runner.key() == Some(self.serial) {
            runner.poll(ctx);
            return;
        }
        request.inputs = trial.inputs.clone();
        let pending = self.pending.clone();
        runner.submit_task(
            ctx,
            self.serial,
            Box::new(move |engine| {
                let o = super::run(engine, request)?;
                let view = decode(&o["econ_preview"])?;
                *pending.lock().map_err(|_| "Result lock failed")? = Some(view);
                Ok(o)
            }),
        );
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        let view = self
            .pending
            .lock()
            .map_err(|_| "Result lock failed")?
            .take()
            .ok_or("Missing econometric view")?;
        let trial = self.trial.take().ok_or("Missing econometric candidate")?;
        self.plan = trial.plan;
        self.inputs = trial.inputs;
        self.versions = trial.versions;
        self.selected = trial.selected;
        self.selected_model = trial.selected_model;
        if let Some(m) = self.plan.models.get(self.selected_model) {
            self.rename = m.name.clone();
        }
        self.current = Some(view);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.add_enabled_ui(!self.running, |ui| {
            field(ui, "Workspace series", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.source_text).char_limit(256),
                    )
                    .desired_width(ui.available_width()),
                );
                egui::ComboBox::from_id_salt("econ_workspace")
                    .selected_text("Choose a variable")
                    .show_ui(ui, |ui| {
                        for (name, array) in ws.variables() {
                            if script::numeric_source(array) || matches!(array, Array::Table(_)) {
                                ui.selectable_value(&mut self.source_text, name.to_owned(), name);
                            }
                        }
                    });
                crate::tokens::helper(ui, "x, X(:,2), T.variable or TT.variable");
                if !self.source_text.trim().is_empty() {
                    super::expression_message(ui, Source::parse(&self.source_text, ws).map(|_| ()));
                }
            });
            if ui.button("Load series").clicked() {
                self.command = Some(Source::parse(&self.source_text, ws).map(Command::Import));
            }
            ui.add_space(crate::tokens::S);
            ui.label("Retained series");
            egui::ScrollArea::vertical()
                .id_salt("econ_series")
                .max_height(144.)
                .show_themed(ui, |ui| {
                    for (i, s) in self.plan.series.iter().enumerate() {
                        let detail = self
                            .current
                            .as_ref()
                            .and_then(|v| v.series.get(i))
                            .map_or(String::new(), |v| {
                                format!(" · n={} · NaN={}", v.values.len(), v.nan_count)
                            });
                        ui.selectable_value(
                            &mut self.selected,
                            i,
                            format!("{}{}", s.label(), detail),
                        );
                    }
                });
            ui.add_space(crate::tokens::S);
            field(ui, "Create transformed series", |ui| {
                egui::ComboBox::from_id_salt("econ_transform")
                    .selected_text(
                        [
                            "Log",
                            "First difference",
                            "Seasonal difference",
                            "Linear detrend",
                        ][self.transform.min(3)],
                    )
                    .show_ui(ui, |ui| {
                        for (i, label) in [
                            "Log",
                            "First difference",
                            "Seasonal difference",
                            "Linear detrend",
                        ]
                        .iter()
                        .enumerate()
                        {
                            ui.selectable_value(&mut self.transform, i, *label);
                        }
                    });
                if self.transform == 2 {
                    ui.label("Period");
                    ui.add(crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.period).char_limit(32),
                    ));
                    super::expression_message(
                        ui,
                        integer(&self.period, 1, 256, "Seasonal period").map(|_| ()),
                    );
                }
                if ui
                    .add_enabled(
                        !self.plan.series.is_empty(),
                        egui::Button::new("Add transform"),
                    )
                    .clicked()
                {
                    self.command = Some(self.selected_transform().map(Command::Transform));
                }
            });
            if self.view == View::Models {
                self.model_inputs(ui);
            } else {
                field(ui, "NumLags (ACF / PACF)", |ui| {
                    ui.add(crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.num_lags).char_limit(32),
                    ));
                    super::expression_message(
                        ui,
                        integer(&self.num_lags, 1, 256, "NumLags").map(|_| ()),
                    );
                    if ui
                        .add_enabled(
                            !self.plan.series.is_empty(),
                            egui::Button::new("Update diagnostics"),
                        )
                        .clicked()
                    {
                        self.command = Some(Ok(Command::Diagnostics));
                    }
                });
                ui.label("Test");
                ui.horizontal_wrapped(|ui| {
                    for k in TestKind::ALL {
                        if ui
                            .selectable_value(&mut self.test_kind, k, k.label())
                            .changed()
                            && self.statistic == "F"
                            && k != TestKind::Adf
                        {
                            self.statistic = "t1".into();
                        }
                    }
                });
                crate::tokens::helper(
                    ui,
                    match self.test_kind {
                        TestKind::Adf | TestKind::Pp => {
                            "Null hypothesis: the series has a unit root."
                        }
                        TestKind::Kpss => {
                            "Null hypothesis: the series is stationary about the selected trend."
                        }
                        TestKind::LjungBox => {
                            "Null hypothesis: no serial correlation through the selected lags."
                        }
                        TestKind::Arch => {
                            "Null hypothesis: no ARCH effects through the selected lags."
                        }
                    },
                );
                ui.label("Lags");
                ui.add(crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.test_lags).char_limit(32),
                ));
                ui.label("Significance level");
                ui.add(crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.alpha).char_limit(32),
                ));
                if matches!(self.test_kind, TestKind::Adf | TestKind::Pp) {
                    egui::ComboBox::from_id_salt("econ_test_model")
                        .selected_text(format!("Model {}", self.model))
                        .show_ui(ui, |ui| {
                            for m in ["AR", "ARD", "TS"] {
                                if ui.selectable_value(&mut self.model, m.into(), m).changed()
                                    && m == "AR"
                                    && self.statistic == "F"
                                {
                                    self.statistic = "t1".into();
                                }
                            }
                        });
                    egui::ComboBox::from_id_salt("econ_statistic")
                        .selected_text(format!("Statistic {}", self.statistic))
                        .show_ui(ui, |ui| {
                            for s in ["t1", "t2", "F"] {
                                if s != "F"
                                    || (self.test_kind == TestKind::Adf && self.model != "AR")
                                {
                                    ui.selectable_value(&mut self.statistic, s.into(), s);
                                }
                            }
                        });
                }
                if self.test_kind == TestKind::Kpss {
                    ui.checkbox(&mut self.trend, "Include linear trend");
                }
                if self.test_kind == TestKind::LjungBox {
                    ui.label("Degrees of freedom (blank = lags)");
                    ui.add(crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.dof).char_limit(32),
                    ));
                }
                if !self.plan.series.is_empty() {
                    super::expression_message(ui, self.test_spec().map(|_| ()));
                }
                if ui
                    .add_enabled(!self.plan.series.is_empty(), egui::Button::new("Run test"))
                    .clicked()
                {
                    self.command = Some(Ok(Command::Test));
                }
            }
            ui.add_space(crate::tokens::S);
            if ui.button("Clear session").clicked() {
                self.clear_requested = true;
            }
        });
        if self.running && ui.button("Stop").clicked() {
            self.stop_requested = true;
        }
    }
    fn main_caption(&self) -> &'static str {
        "TIME SERIES"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view, View::Diagnostics, "Diagnostics");
            ui.selectable_value(&mut self.view, View::Models, "Models");
        });
        if self.view == View::Models {
            self.models_ui(ui);
            return;
        }
        let Some(v) = &self.current else {
            crate::tokens::helper(
                ui,
                "Load a workspace series to inspect its values and run diagnostics.",
            );
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("econ_main")
            .show_themed(ui, |ui| {
                if let Some(s) = v.series.get(self.selected) {
                    ui.label(self.plan.series[self.selected].label());
                    line_plot(ui, "econ_series_plot", "Observation", &s.points, &[], 180.);
                }
                if let Some(d) = &v.diagnostics {
                    if d.series == self.selected {
                        ui.columns(2, |cols| {
                            for (ui, (label, values, lags, bounds)) in cols.iter_mut().zip([
                                ("ACF", &d.acf, &d.acf_lags, &d.acf_bounds),
                                ("PACF", &d.pacf, &d.pacf_lags, &d.pacf_bounds),
                            ]) {
                                ui.label(label);
                                let points = lags
                                    .iter()
                                    .zip(values)
                                    .map(|(x, y)| [*x, *y])
                                    .collect::<Vec<_>>();
                                line_plot(ui, label, "Lag", &points, bounds, 160.);
                            }
                        });
                    } else {
                        crate::tokens::helper(ui, "Update diagnostics for the selected series.");
                    }
                } else {
                    crate::tokens::helper(
                        ui,
                        "Choose NumLags, then Update diagnostics for ACF and PACF.",
                    );
                }
                ui.add_space(crate::tokens::S);
                ui.label("Retained tests");
                egui::ScrollArea::horizontal()
                    .id_salt("econ_test_table")
                    .show_themed(ui, |ui| {
                        egui::Grid::new("econ_tests")
                            .striped(true)
                            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                            .show(ui, |ui| {
                                for label in [
                                    "Series / test",
                                    "Options",
                                    "h",
                                    "p",
                                    "Statistic",
                                    "Critical value",
                                ] {
                                    ui.label(label);
                                }
                                ui.end_row();
                                for (spec, row) in self.plan.tests.iter().zip(&v.tests) {
                                    ui.label(format!(
                                        "{} / {}",
                                        spec.series + 1,
                                        spec.kind.label()
                                    ));
                                    ui.label(spec.options_label());
                                    for value in row {
                                        ui.label(short(*value));
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Series", self.plan.series.len().to_string()),
            ("Kept tests", self.plan.tests.len().to_string()),
            ("Fitted models", self.plan.models.len().to_string()),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Results struct", &mut self.name, ws);
        super::expression_message(ui, checked_name(&self.name));
        if self.plan.roots().contains(&self.name) {
            super::expression_message(
                ui,
                Err("Results name must differ from source variables".into()),
            );
        }
        super::expression_message(ui, self.check_versions(ws));
        if !self.plan.models.is_empty() {
            for (label, name) in [
                "Estimated models (cell)",
                "Residuals (cell)",
                "Forecast records (cell)",
            ]
            .into_iter()
            .zip(&mut self.model_names)
            {
                name_field(ui, label, name, ws);
            }
            super::expression_message(ui, self.output_script().map(|_| ()));
        }
        crate::tokens::helper(
            ui,
            "Includes series, test options, h / p / statistic / critical value and correlograms. h=1 rejects the test's null hypothesis.",
        );
        ui.collapsing("Script preview", |ui| match self.script(ws) {
            Ok(code) => {
                ui.label(code);
            }
            Err(error) => {
                super::expression_message(ui, Err(error));
            }
        });
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        self.output_names()
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|v| {
            format!(
                "Econometric Modeler: {} series, {} retained tests, {} models",
                v.series.len(),
                v.tests.len(),
                v.models.len()
            )
        })
    }
}

fn line_plot(
    ui: &mut egui::Ui,
    id: &str,
    x: &str,
    points: &[[f64; 2]],
    bounds: &[f64],
    height: f32,
) {
    let p = crate::tokens::palette(ui.ctx());
    egui_plot::Plot::new(id)
        .height(height)
        .x_axis_label(x)
        .show(ui, |plot| {
            // Split at missing observations so the plot preserves gaps.
            for segment in points
                .split(|p| !p[1].is_finite())
                .filter(|s| !s.is_empty())
            {
                plot.line(egui_plot::Line::new(id, segment.to_vec()).color(p.accent));
            }
            for b in bounds.iter().filter(|b| b.is_finite()) {
                plot.hline(
                    egui_plot::HLine::new("Bound", *b)
                        .color(p.text_muted)
                        .style(egui_plot::LineStyle::dashed_dense()),
                );
            }
        });
}

pub(crate) fn start_work(
    host: &mut Host<EconometricModeler>,
    ctx: &egui::Context,
    ws: &Snapshot,
    command: Command,
) {
    match host.app.start(ws, command) {
        Ok(()) => {
            host.run_error = None;
            host.delivered = None;
            host.refresh(ctx, ws);
        }
        Err(error) => host.run_error = Some(error),
    }
}
pub(crate) fn show(
    host: &mut Host<EconometricModeler>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.set_folder(ws.folder());
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if let Some(result) = host.app.edit_result.take() {
        host.run_error = result.err();
    }
    if std::mem::take(&mut host.app.clear_requested) {
        stop(host);
        host.app = EconometricModeler::default();
        host.run_error = None;
        host.script_error = None;
        host.delivered = None;
        host.message.clear();
    } else if host.app.stop_requested {
        host.app.stop_requested = false;
        host.runner.stop();
        host.app.trial = None;
        host.run_error =
            Some("Econometric calculation cancelled; previous results retained".into());
    } else if let Some(command) = host.app.command.take() {
        match command {
            Ok(command) => start_work(host, ui.ctx(), ws, command),
            Err(error) => host.run_error = Some(error),
        }
    }
    if !host.running() && host.run_error.is_some() {
        host.app.trial = None;
    }
}

pub(crate) fn stop(host: &mut Host<EconometricModeler>) {
    host.runner.stop();
    host.app.trial = None;
    host.app.running = false;
}
