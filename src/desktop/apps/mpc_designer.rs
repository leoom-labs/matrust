//! Controller inputs on the ordinary app framework. All controller construction
//! and validation runs as MATLAB code on the cancellable private Runner.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::{
    Array, Engine,
    engine::mpc_app_hooks::{self, Source},
};
use std::sync::{Arc, Mutex};
#[path = "mpc_designer/script.rs"]
mod script;
pub use script::{build_scenario_script, build_script, checked_name};
#[path = "mpc_designer/scenario.rs"]
mod scenario;
use scenario::RunSpec;
pub use scenario::{Scenario, ScenarioResult};
const RUN_PREFIX: &str = "scenario";

pub(crate) type OpenSlot = Arc<Mutex<Option<Option<Source>>>>;
pub const BOUNDS: [&str; 6] = ["Min", "Max", "RateMin", "RateMax", "MinECR", "MaxECR"];
#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub channel: usize,
    pub name: String,
    pub units: String,
    pub bounds: [String; 6],
}
impl Constraint {
    fn new(channel: usize, output: bool) -> Self {
        Self {
            channel,
            name: String::new(),
            units: String::new(),
            bounds: [
                "-Inf",
                "Inf",
                "-Inf",
                "Inf",
                if output { "1" } else { "0" },
                if output { "1" } else { "0" },
            ]
            .map(str::to_owned),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub name: String,
    pub source: String,
    pub imported: bool,
    pub initial: bool,
    pub original_roles: (Vec<usize>, Vec<usize>),
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    pub ts: String,
    pub prediction: String,
    pub control: String,
    /// OV in physical output order; MV and MVRate in MV-channel order.
    pub weights: [String; 3],
    pub ecr: String,
    pub mv: Vec<Constraint>,
    pub ov: Vec<Constraint>,
}
impl Spec {
    fn new(source: &Source, name: String) -> Self {
        let ts = property(&source.plant, "Ts")
            .ok()
            .and_then(|a| {
                if let Array::Numeric(m) = a {
                    m.data().first().copied()
                } else {
                    None
                }
            })
            .filter(|x| *x > 0.)
            .unwrap_or(1.);
        Self {
            name,
            source: source.name.clone(),
            imported: source.is_controller(),
            initial: true,
            original_roles: source.roles(),
            inputs: vec![],
            outputs: vec![],
            ts: script::num(ts),
            prediction: "10".into(),
            control: "2".into(),
            weights: ["1", "0", "0.1"].map(str::to_owned),
            ecr: "100000".into(),
            mv: vec![],
            ov: vec![],
        }
    }
    pub fn sync_roles(&mut self) {
        let remap = |old: &[Constraint], ids: Vec<usize>, output| {
            ids.into_iter()
                .map(|i| {
                    old.iter()
                        .find(|r| r.channel == i)
                        .cloned()
                        .unwrap_or_else(|| Constraint::new(i, output))
                })
                .collect()
        };
        self.mv = remap(
            &self.mv,
            self.inputs
                .iter()
                .enumerate()
                .filter(|(_, r)| **r == 0)
                .map(|(i, _)| i)
                .collect(),
            false,
        );
        self.ov = remap(&self.ov, (0..self.outputs.len()).collect(), true);
        self.weights = ["1", "0", "0.1"].map(str::to_owned);
    }
}
#[derive(Clone)]
pub struct Controller {
    pub spec: Spec,
    pub source: Source,
    pub value: Source,
    pub summary: String,
    pub size: [usize; 3],
    pub plant_size: [usize; 3],
    version: Option<u64>,
    source_value: Array,
}
#[derive(Clone, Copy)]
pub enum Command {
    Create,
    Apply,
    Run,
}
struct Trial {
    run: Option<RunSpec>,
    controllers: Vec<Controller>,
    selected: usize,
}
struct Prepared {
    result: Option<ScenarioResult>,
    controllers: Vec<Controller>,
    selected: usize,
}
#[derive(Default)]
pub struct MpcDesigner {
    pub source_text: String,
    pub catalogue: Vec<Source>,
    pub controllers: Vec<Controller>,
    pub selected: usize,
    pub editor: Option<Spec>,
    pub rename: String,
    pub export_all: bool,
    pub scenario: Scenario,
    pub compare: Option<usize>,
    pub result: Option<ScenarioResult>,
    pub controller_only: bool,
    pub scenario_name: String,
    pub view: usize,
    pub output_channel: usize,
    pub mv_channel: usize,
    pub data_mv: bool,
    pub running: bool,
    pub error: Option<String>,
    trial: Option<Trial>,
    serial: u64,
    prepared: Arc<Mutex<Option<Prepared>>>,
    command: Option<Command>,
    edit_result: Option<Result<(), String>>,
    stop_requested: bool,
}
fn property<'a>(a: &'a Array, name: &str) -> Result<&'a Array, String> {
    let Array::Struct(s) = a else {
        return Err(format!("Missing {name}"));
    };
    Ok(s.get(
        s.field_index(name)
            .ok_or_else(|| format!("Missing {name}"))?,
        0,
    )?
    .as_ref())
}
fn text(a: &Array) -> String {
    if let Array::Character(c) = a {
        c.to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap_or_default()
    } else {
        String::new()
    }
}
fn metadata(plant: &Array, key: &str, i: usize) -> String {
    match property(plant, key) {
        Ok(Array::Cell(c)) => c
            .get_linear(i)
            .map(|a| text(a.as_ref()))
            .unwrap_or_default(),
        Ok(a) if i == 0 => text(a),
        _ => String::new(),
    }
}
fn decode(engine: &Engine, mut spec: Spec, value: &Source) -> Result<Spec, String> {
    let props = value.properties(engine)?;
    spec.initial = false;
    (spec.inputs, spec.outputs) = value.roles();
    for (target, key) in [
        (&mut spec.ts, "Ts"),
        (&mut spec.prediction, "PredictionHorizon"),
        (&mut spec.control, "ControlHorizon"),
    ] {
        *target = script::matrix(&props[key])?;
    }
    let w = &props["Weights"];
    for (target, key) in spec.weights.iter_mut().zip(["OV", "MV", "MVRate"]) {
        *target = script::matrix(property(w, key)?)?;
    }
    spec.ecr = script::matrix(property(w, "ECR")?)?;
    let rows = |key: &str, ids: Vec<usize>, output| -> Result<Vec<Constraint>, String> {
        let Array::Struct(s) = &props[key] else {
            return Err("Missing constraints".into());
        };
        ids.into_iter()
            .enumerate()
            .map(|(i, channel)| {
                let mut row = Constraint::new(channel, output);
                for (j, key) in BOUNDS.iter().enumerate() {
                    if let Some(f) = s.field_index(key) {
                        row.bounds[j] = script::matrix(s.get(f, i)?.as_ref())?;
                    }
                }
                for (target, key) in [(&mut row.name, "Name"), (&mut row.units, "Units")] {
                    *target = text(
                        s.get(s.field_index(key).ok_or("Missing signal metadata")?, i)?
                            .as_ref(),
                    );
                }
                // Fill missing names/units from plant metadata, preserving
                // signal metadata explicitly tuned on an imported controller.
                for (target, key) in [
                    (
                        &mut row.name,
                        if output { "OutputName" } else { "InputName" },
                    ),
                    (
                        &mut row.units,
                        if output { "OutputUnit" } else { "InputUnit" },
                    ),
                ] {
                    let name = metadata(&value.plant, key, channel);
                    if target.is_empty() && !name.is_empty() {
                        *target = name;
                    }
                }
                Ok(row)
            })
            .collect()
    };
    spec.mv = rows(
        "ManipulatedVariables",
        spec.inputs
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == 0)
            .map(|(i, _)| i)
            .collect(),
        false,
    )?;
    spec.ov = rows("OutputVariables", (0..spec.outputs.len()).collect(), true)?;
    Ok(spec)
}
impl MpcDesigner {
    fn result_prefix(&self) -> &str {
        if self.scenario_name.is_empty() {
            RUN_PREFIX
        } else {
            &self.scenario_name
        }
    }
    fn select(&mut self, i: usize) {
        self.selected = i;
        self.editor = self.controllers.get(i).map(|c| c.spec.clone());
        self.rename = self
            .editor
            .as_ref()
            .map_or(String::new(), |s| s.name.clone());
    }
    fn source(&self, ws: &Snapshot) -> Result<Source, String> {
        checked_name(&self.source_text)?;
        if let Some(s) = self.catalogue.iter().find(|s| s.name == self.source_text) {
            return Ok(s.clone());
        }
        Source::plant(
            self.source_text.clone(),
            ws.get(&self.source_text)
                .ok_or("Choose a workspace source")?
                .clone(),
        )
    }
    fn check_sources(&self, ws: &Snapshot) -> Result<(), String> {
        for c in &self.controllers {
            if ws.version(&c.source.name) != c.version
                || !ws
                    .get(&c.source.name)
                    .is_some_and(|a| a.same_snapshot(&c.source_value))
            {
                return Err(format!(
                    "{} changed; delete its retained controllers before loading the current value",
                    c.source.name
                ));
            }
        }
        Ok(())
    }
    pub fn start(&mut self, ws: &Snapshot, command: Command) -> Result<(), String> {
        self.check_sources(ws)?;
        let mut controllers = self.controllers.clone();
        let mut run = None;
        let selected = match command {
            Command::Create => {
                if controllers.len() >= 16 {
                    return Err("Keep at most 16 controllers".into());
                }
                let source = self.source(ws)?;
                if source.plant.bytes() > 128 * 1024 * 1024 {
                    return Err("Plant exceeds 128 MiB".into());
                }
                let name = super::unused_name("controller", |n| {
                    ws.contains(n) || controllers.iter().any(|c| c.spec.name == n)
                });
                let spec = Spec::new(&source, name);
                controllers.push(Controller {
                    spec,
                    source: source.clone(),
                    value: source.clone(),
                    summary: String::new(),
                    size: [0; 3],
                    plant_size: [0; 3],
                    version: ws.version(&source.name),
                    source_value: ws.get(&source.name).ok_or("Missing source")?.clone(),
                });
                controllers.len() - 1
            }
            Command::Run => {
                let mut alternatives = vec![
                    controllers
                        .get(self.selected)
                        .ok_or("Create a controller first")?
                        .clone(),
                ];
                if let Some(i) = self.compare {
                    alternatives.push(
                        controllers
                            .get(i)
                            .ok_or("Choose an accepted comparison controller")?
                            .clone(),
                    );
                }
                let specs = alternatives
                    .iter()
                    .map(|c| c.spec.clone())
                    .collect::<Vec<_>>();
                self.scenario.validate(&specs)?;
                if let Some(rows) = scenario::finite_literal(&self.scenario.initial)? {
                    let len = rows.iter().map(Vec::len).sum::<usize>();
                    if alternatives.iter().any(|c| c.plant_size[0] != len) {
                        return Err(
                            "Plant initial state length must match each simulation plant".into(),
                        );
                    }
                }
                build_scenario_script(&specs, &self.scenario, RUN_PREFIX)?;
                run = Some(RunSpec {
                    scenario: self.scenario.clone(),
                    controllers: alternatives,
                });
                self.selected
            }
            Command::Apply => {
                let c = controllers
                    .get_mut(self.selected)
                    .ok_or("Create a controller first")?;
                c.spec = self.editor.clone().ok_or("Missing controller draft")?;
                self.selected
            }
        };
        build_script(
            &controllers
                .iter()
                .map(|c| c.spec.clone())
                .collect::<Vec<_>>(),
        )?;
        self.trial = Some(Trial {
            run,
            controllers,
            selected,
        });
        self.serial += 1;
        *self.prepared.lock().map_err(|_| "MPC result lock failed")? = None;
        self.error = None;
        Ok(())
    }
    fn output_specs(&self) -> Vec<Spec> {
        self.controllers
            .iter()
            .enumerate()
            .filter(|(i, _)| self.export_all || *i == self.selected)
            .map(|(_, c)| c.spec.clone())
            .collect()
    }
    fn rename_selected(&mut self) -> Result<(), String> {
        checked_name(&self.rename)?;
        if self.controllers.iter().enumerate().any(|(i, c)| {
            c.source.name == self.rename || (i != self.selected && c.spec.name == self.rename)
        }) {
            return Err("Choose a unique name different from source variables".into());
        }
        let c = self
            .controllers
            .get_mut(self.selected)
            .ok_or("Create a controller first")?;
        c.spec.name = self.rename.clone();
        if let Some(e) = &mut self.editor {
            e.name = self.rename.clone();
        }
        Ok(())
    }
}
impl App for MpcDesigner {
    fn kind(&self) -> AppKind {
        AppKind::MpcDesigner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        self.check_sources(ws)?;
        if !self.controller_only
            && let Some(result) = &self.result
        {
            for c in &result.run.controllers {
                if ws.version(&c.source.name) != c.version
                    || !ws
                        .get(&c.source.name)
                        .is_some_and(|a| a.same_snapshot(&c.source_value))
                {
                    return Err(format!(
                        "{} changed since the displayed run; rerun the scenario before export",
                        c.source.name
                    ));
                }
            }
            return build_scenario_script(
                &result
                    .run
                    .controllers
                    .iter()
                    .map(|c| c.spec.clone())
                    .collect::<Vec<_>>(),
                &result.run.scenario,
                self.result_prefix(),
            );
        }
        build_script(&self.output_specs())
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.script(ws)
    }
    fn execution_script(&self, ws: &Snapshot) -> Result<String, String> {
        self.check_sources(ws)?;
        match &self.trial {
            Some(t) if t.run.is_some() => {
                let run = t.run.as_ref().unwrap();
                build_scenario_script(
                    &run.controllers
                        .iter()
                        .map(|c| c.spec.clone())
                        .collect::<Vec<_>>(),
                    &run.scenario,
                    RUN_PREFIX,
                )
            }
            Some(t) => build_script(
                &t.controllers
                    .iter()
                    .map(|c| c.spec.clone())
                    .collect::<Vec<_>>(),
            ),
            None => Ok(String::new()),
        }
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (
            String::new(),
            if self.trial.as_ref().is_some_and(|t| t.run.is_some()) {
                scenario::output_names(
                    RUN_PREFIX,
                    self.trial
                        .as_ref()
                        .unwrap()
                        .run
                        .as_ref()
                        .unwrap()
                        .controllers
                        .len(),
                )
            } else {
                vec![]
            },
        )
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, request: Request) {
        let Some(trial) = &self.trial else {
            runner.poll(ctx);
            return;
        };
        if runner.key() == Some(self.serial) {
            runner.poll(ctx);
            return;
        }
        let mut controllers = trial.controllers.clone();
        let selected = trial.selected;
        let prepared = self.prepared.clone();
        let run = trial.run.clone();
        runner.submit_task(
            ctx,
            self.serial,
            Box::new(move |engine| {
                for c in &controllers {
                    c.source.install(engine)?;
                }
                let outputs = super::run(engine, request)?;
                if let Some(run) = run {
                    let result = scenario::collect(&outputs, run, RUN_PREFIX)?;
                    *prepared.lock().map_err(|_| "MPC result lock failed")? = Some(Prepared {
                        controllers,
                        selected,
                        result: Some(result),
                    });
                    return Ok(outputs);
                }
                let initializing = controllers.iter().any(|c| c.spec.initial);
                for c in &mut controllers {
                    c.value = mpc_app_hooks::snapshot(engine, &c.spec.name)?;
                    c.spec = decode(engine, c.spec.clone(), &c.value)?;
                }
                // Canonicalize initial defaults too: the first accepted value
                // has the exact explicit metadata and groups in the saved script.
                if initializing {
                    engine.execute(&build_script(
                        &controllers
                            .iter()
                            .map(|c| c.spec.clone())
                            .collect::<Vec<_>>(),
                    )?)?;
                }
                for c in &mut controllers {
                    c.value = mpc_app_hooks::snapshot(engine, &c.spec.name)?;
                    let (a, b, cc, _, _) = matrust::engine::mpc_prediction_ss(engine, &c.spec.name)
                        .ok_or("Missing discrete prediction model")?;
                    if a.rows() > 128 || b.cols() > 128 || cc.rows() > 128 {
                        return Err("Prediction model dimensions exceed 128".into());
                    }
                    c.plant_size = [a.rows(), b.cols(), cc.rows()];
                    let model = matrust::engine::mpc_prediction_model(engine, &c.spec.name)?;
                    c.size = [model.a.rows(), model.bu.cols(), model.c.rows()];
                    c.summary = engine
                        .execute(&format!("disp({});", c.spec.name))?
                        .output
                        .join("\n");
                }
                *prepared.lock().map_err(|_| "MPC result lock failed")? = Some(Prepared {
                    result: None,
                    controllers,
                    selected,
                });
                Ok(Outputs::new())
            }),
        );
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        let prepared = self
            .prepared
            .lock()
            .map_err(|_| "MPC result lock failed")?
            .take()
            .ok_or("Missing MPC result")?;
        let scenario_run = prepared.result.is_some();
        if let Some(result) = prepared.result {
            self.result = Some(result);
        }
        self.controllers = prepared.controllers;
        if scenario_run {
            self.selected = prepared.selected;
        } else {
            self.select(prepared.selected);
        }
        self.trial = None;
        self.error = None;
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.editor_ui(ui, ws);
    }
    fn main_caption(&self) -> &'static str {
        "SCENARIO"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.scenario_results_ui(ui);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Controllers", self.controllers.len().to_string()),
            (
                "Scenario",
                self.result.as_ref().map_or("Not run".into(), |r| {
                    format!("{} samples · completed", r.run.scenario.samples)
                }),
            ),
            (
                "Selected",
                self.controllers
                    .get(self.selected)
                    .map_or(String::new(), |c| c.spec.name.clone()),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.checkbox(&mut self.controller_only, "Controller only");
        if self.result.is_some() && !self.controller_only {
            ui.label("Exports the displayed run snapshot, including its controllers and scenario.");
            ui.label("Scenario variable prefix");
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.scenario_name)
                        .hint_text("scenario")
                        .char_limit(55),
                )
                .desired_width(ui.available_width()),
            );
        } else {
            ui.checkbox(&mut self.export_all, "All controllers");
        }
        if let Some(result) = &self.result {
            ui.label(format!(
                "Displayed: {} stored samples",
                result.run.scenario.samples
            ));
        }
        for name in self.exported(ws) {
            ui.label(format!("Workspace name: {name}"));
            if ws.contains(&name) {
                crate::tokens::helper(ui, "Replaces an existing workspace variable");
            }
        }
        super::expression_message(ui, self.script(ws).map(|_| ()));
        if let Some(error) = &self.error {
            super::expression_message(ui, Err(error.clone()));
        }
        ui.collapsing("Script preview", |ui| {
            if let Ok(code) = self.script(ws) {
                egui::ScrollArea::both()
                    .max_height(240.)
                    .show_themed(ui, |ui| {
                        ui.label(code);
                    });
            }
        });
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        if !self.controller_only
            && let Some(result) = &self.result
        {
            let mut names = result
                .run
                .controllers
                .iter()
                .map(|c| c.spec.name.clone())
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names.extend(scenario::output_names(
                self.result_prefix(),
                result.trajectories.len(),
            ));
            return names;
        }
        self.output_specs().into_iter().map(|s| s.name).collect()
    }
    fn summary(&self) -> Option<String> {
        self.controllers
            .get(self.selected)
            .map(|c| format!("MPC Designer: {} ready", c.spec.name))
    }
}
#[path = "mpc_designer/results.rs"]
mod results;
#[path = "mpc_designer/ui.rs"]
mod ui;

pub(crate) fn start_work(
    host: &mut Host<MpcDesigner>,
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
        Err(e) => {
            host.app.error = Some(e.clone());
            host.run_error = Some(e);
        }
    }
}
pub(crate) fn show(
    host: &mut Host<MpcDesigner>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.set_folder(ws.folder());
    host.app.running = host.running();
    if ui.available_width() < 2. * super::COLUMN + 360. {
        ui::show_compact(host, ui, ws, busy, action);
    } else {
        host.show(ui, ws, busy, action);
    }
    if let Some(result) = host.app.edit_result.take() {
        host.run_error = result.err();
        host.app.error = host.run_error.clone();
    }
    if std::mem::take(&mut host.app.stop_requested) {
        stop(host);
        host.run_error =
            Some("MPC calculation cancelled; last good controller and scenario retained".into());
    } else if let Some(command) = host.app.command.take() {
        start_work(host, ui.ctx(), ws, command);
    }
    if !host.running() && (host.run_error.is_some() || host.script_error.is_some()) {
        host.app.trial = None;
        host.app.error = host.script_error.clone().or_else(|| host.run_error.clone());
    }
}
pub(crate) fn stop(host: &mut Host<MpcDesigner>) {
    host.runner.stop();
    host.app.trial = None;
    host.app.running = false;
}
pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .mpc_designer
        .lock()
        .ok()
        .and_then(|mut s| s.take());
    if workspace
        .worker
        .cancel
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    if let Some(source) = launch {
        if let Some(source) = source {
            let host = &mut workspace.toolkits.apps.mpc;
            host.app.source_text = source.name.clone();
            if source.is_controller() && !host.app.catalogue.iter().any(|s| s.name == source.name) {
                host.app.catalogue.push(source);
            }
            host.app.command = Some(Command::Create);
        }
        workspace.open_app(crate::toolkits::Tool::App(AppKind::MpcDesigner));
    }
}

/// A deterministic running task lets the pointer test exercise Stop even on
/// machines where constructing a small controller finishes within one frame.
#[cfg(test)]
pub(crate) fn hold_runner_for_stop_test(host: &mut Host<MpcDesigner>, ctx: &egui::Context) {
    host.runner.submit_task(
        ctx,
        u64::MAX,
        Box::new(|engine| {
            let cancel = engine.cancellation();
            while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            Err("Cancelled".into())
        }),
    );
}
