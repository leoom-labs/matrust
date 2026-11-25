//! Clean-room type-1 fuzzy editor. Public MathWorks contracts are linked in the
//! task notes; the UI uses MatRust's ordinary app layout and plot widgets.
//! One canonical builder serves preview, workspace export and saved scripts.
use super::{
    App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, number, short,
};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::{
    Array,
    engine::fuzzy_app_hooks::{self, System, Variable},
};
use std::sync::{Arc, Mutex};

pub(crate) type OpenSlot = Arc<Mutex<Option<System>>>;
const METHODS: [&str; 5] = [
    "AndMethod",
    "OrMethod",
    "ImplicationMethod",
    "AggregationMethod",
    "DefuzzificationMethod",
];
const MFS: [&str; 13] = [
    "trimf", "trapmf", "gaussmf", "gauss2mf", "gbellmf", "sigmf", "dsigmf", "psigmf", "zmf", "smf",
    "pimf", "linsmf", "linzmf",
];
const INTERNAL: &str = "app_fuzzy_";
const MAX_SCRIPT: usize = 900 * 1024;

pub fn empty(sugeno: bool) -> System {
    System {
        sugeno,
        name: "controller".into(),
        methods: if sugeno {
            ["prod", "probor", "prod", "sum", "wtaver"]
        } else {
            ["min", "max", "min", "max", "centroid"]
        }
        .map(str::to_owned),
        inputs: vec![],
        outputs: vec![],
        rules: vec![],
        structural_checks_disabled: false,
    }
}
fn quote(s: &str) -> Result<String, String> {
    if s.len() > 16384 || s.chars().any(char::is_control) {
        return Err("Fuzzy designer: text must be one line, at most 16384 bytes".into());
    }
    let q = format!("'{}'", s.replace('\'', "''"));
    super::single_expression(&q)?;
    Ok(q)
}
pub fn checked_name(s: &str) -> Result<(), String> {
    if !matrust::files::valid_name(s) || s.starts_with(INTERNAL) {
        return Err("Fuzzy designer: use a valid export name outside app_fuzzy_".into());
    }
    super::single_expression(s)?;
    matrust::syntax::parse(&format!("{s} = 0;"))
        .map_err(|e| format!("Invalid export name: {e}"))?;
    super::check_export_name(s)
}
fn vector(v: &[f64]) -> Result<String, String> {
    if v.len() > 128 || v.iter().any(|v| !v.is_finite()) {
        return Err("Fuzzy designer: expected at most 128 finite numbers".into());
    }
    Ok(format!(
        "[{}]",
        v.iter().map(|v| number(*v)).collect::<Vec<_>>().join(" ")
    ))
}
/// Numeric text is parsed in its assignment context, then restricted to a real
/// literal vector. Function calls, comments and additional statements cannot run.
pub fn numeric_text(s: &str) -> Result<Vec<f64>, String> {
    let s = s.trim();
    super::single_expression(s)?;
    let s = s
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(s);
    let values = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<f64>()
                .map_err(|_| "Use a finite numeric literal vector".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    vector(&values)?;
    Ok(values)
}
fn allowed_methods(sugeno: bool, index: usize) -> &'static [&'static str] {
    match index {
        0 => &["min", "prod"],
        1 => &["max", "probor"],
        2 if sugeno => &["prod"],
        2 => &["min", "prod"],
        3 if sugeno => &["sum"],
        3 => &["max", "probor", "sum"],
        _ if sugeno => &["wtaver", "wtsum"],
        _ => &["centroid", "bisector", "mom", "som", "lom"],
    }
}
/// The exact accepted FIS, including numeric NOT/don't-care rules. No file or
/// workspace dependencies survive in generated scripts. Temporaries are local.
pub fn build_script(system: &System, name: &str, inputs: &[f64]) -> Result<String, String> {
    checked_name(name)?;
    if system.inputs.len() > 64 || system.outputs.len() > 64 || system.rules.len() > 65536 {
        return Err("Fuzzy designer: system exceeds engine dimensions".into());
    }
    if inputs.len() != system.inputs.len() {
        return Err("Fuzzy designer: provide one value per input".into());
    }
    let mut code = format!(
        "{name} = app_fuzzy_build();\nfunction fis = app_fuzzy_build()\nfis = {}('Name', {}",
        if system.sugeno { "sugfis" } else { "mamfis" },
        quote(&system.name)?
    );
    for (i, key) in METHODS.iter().enumerate() {
        if !allowed_methods(system.sugeno, i).contains(&system.methods[i].as_str()) {
            return Err(format!(
                "Fuzzy designer: unsupported {key} '{}'",
                system.methods[i]
            ));
        }
        code.push_str(&format!(", '{key}', {}", quote(&system.methods[i])?));
    }
    code.push_str(");\n");
    for (output, variables) in [(false, &system.inputs), (true, &system.outputs)] {
        for v in variables {
            if v.members.len() > 256 {
                return Err("Fuzzy designer: at most 256 MFs per variable".into());
            }
            code.push_str(&format!(
                "fis = {}(fis, {}, 'Name', {});\n",
                if output { "addOutput" } else { "addInput" },
                vector(&v.range)?,
                quote(&v.name)?
            ));
            for m in &v.members {
                let types: &[&str] = if output && system.sugeno {
                    &["constant", "linear"]
                } else {
                    &MFS
                };
                if !types.contains(&m.kind.as_str()) {
                    return Err(format!(
                        "Fuzzy designer: unsupported membership type '{}'",
                        m.kind
                    ));
                }
                code.push_str(&format!(
                    "fis = addMF(fis, {}, {}, {}, 'Name', {}, 'VariableType', '{}');\n",
                    quote(&v.name)?,
                    quote(&m.kind)?,
                    vector(&m.parameters)?,
                    quote(&m.name)?,
                    if output { "output" } else { "input" }
                ));
                if code.len() > MAX_SCRIPT {
                    return Err("Fuzzy designer: generated script exceeds 900 KiB".into());
                }
            }
        }
    }
    for r in &system.rules {
        let mut row = r.antecedent.clone();
        row.extend(&r.consequent);
        row.extend([r.weight, r.connection]);
        // Each row is small even when the rule base is large.
        code.push_str(&format!("fis = addRule(fis, {});\n", vector(&row)?));
        if code.len() > MAX_SCRIPT {
            return Err("Fuzzy designer: generated script exceeds 900 KiB".into());
        }
    }
    if system.structural_checks_disabled {
        code.push_str("fis.DisableStructuralChecks = true;\n");
    }
    if !system.inputs.is_empty() && !system.outputs.is_empty() {
        code.push_str(&format!(
            "app_fuzzy_value = evalfis(fis, {});\n",
            vector(inputs)?
        ));
    }
    code.push_str("end\n");
    if code.len() > MAX_SCRIPT {
        return Err("Fuzzy designer: generated script exceeds 900 KiB".into());
    }
    Ok(code)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Memberships,
    Inference,
    Surface,
    System,
}
#[derive(Clone)]
enum Pending {
    Edit(String),
    Open(String),
    Import(System),
}
#[derive(Clone, Debug)]
pub struct ResultView {
    pub values: Vec<f64>,
    pub firing: Vec<f64>,
    pub aggregate: Vec<f64>,
    pub mf_x: Vec<f64>,
    pub mf_y: Vec<f64>,
    pub mf_rows: usize,
    pub surface_x: Vec<f64>,
    pub surface_y: Vec<f64>,
    pub surface_z: Vec<f64>,
    pub surface_rows: usize,
    pub warning: String,
    pub rules: Vec<String>,
}
struct Prepared {
    system: System,
    result: ResultView,
}
pub struct FuzzyDesigner {
    pub system: System,
    pub current: Option<ResultView>,
    pub name: String,
    pub input_values: Vec<f64>,
    pub view: View,
    pub output_variable: bool,
    pub variable: usize,
    pub member: usize,
    pub surface_inputs: [usize; 2],
    pub surface_output: usize,
    pub grid: u32,
    pub error: Option<String>,
    pub rule_text: String,
    pub mf_parameters: String,
    pub variable_range: String,
    pub variable_name: String,
    pub mf_name: String,
    pub mf_kind: String,
    pub system_name: String,
    pub methods: [String; 5],
    rule: Option<usize>,
    picks: Vec<(usize, bool)>,
    weight: f64,
    or_rule: bool,
    workspace_name: String,
    pub catalogue: Vec<(String, System)>,
    pending: Option<Pending>,
    prepared: Arc<Mutex<Option<Prepared>>>,
    stopped: bool,
    running: bool,
    stop_requested: bool,
    save_requested: bool,
    save_path: Option<std::path::PathBuf>,
    save_runner: Runner,
    save_delivered: Option<u64>,
    serial: u64,
}
impl Default for FuzzyDesigner {
    fn default() -> Self {
        let system = empty(false);
        Self {
            system_name: system.name.clone(),
            methods: system.methods.clone(),
            system,
            current: None,
            name: "fuzzySystem".into(),
            input_values: vec![],
            view: View::Memberships,
            output_variable: false,
            variable: 0,
            member: 0,
            surface_inputs: [0, 1],
            surface_output: 0,
            grid: 21,
            error: None,
            rule_text: String::new(),
            mf_parameters: "[0 0.5 1]".into(),
            variable_range: "[0 1]".into(),
            variable_name: "input1".into(),
            mf_name: "mf1".into(),
            mf_kind: "trimf".into(),
            rule: None,
            picks: vec![],
            weight: 1.,
            or_rule: false,
            workspace_name: String::new(),
            catalogue: vec![],
            pending: None,
            prepared: Default::default(),
            stopped: false,
            running: false,
            stop_requested: false,
            save_requested: false,
            save_path: None,
            save_runner: Runner::default(),
            save_delivered: None,
            serial: 0,
        }
    }
}
impl FuzzyDesigner {
    fn selected(&self) -> Option<&Variable> {
        (if self.output_variable {
            &self.system.outputs
        } else {
            &self.system.inputs
        })
        .get(self.variable)
    }
    fn key(&self) -> &'static str {
        if self.output_variable {
            "Outputs"
        } else {
            "Inputs"
        }
    }
    fn kind(&self) -> &'static str {
        if self.output_variable {
            "output"
        } else {
            "input"
        }
    }
    fn sync_fields(&mut self) {
        self.system_name = self.system.name.clone();
        self.methods = self.system.methods.clone();
        if let Some(v) = self.selected().cloned() {
            self.variable_name = v.name;
            self.variable_range = vector(&v.range).unwrap();
            if let Some(m) = v.members.get(self.member) {
                self.mf_name = m.name.clone();
                self.mf_kind = m.kind.clone();
                self.mf_parameters = vector(&m.parameters).unwrap_or_default();
            } else {
                let sugeno_output = self.output_variable && self.system.sugeno;
                self.mf_name = "mf1".into();
                self.mf_kind = if sugeno_output { "constant" } else { "trimf" }.into();
                self.mf_parameters = if sugeno_output {
                    "[0]".into()
                } else {
                    vector(&[v.range[0], (v.range[0] + v.range[1]) / 2., v.range[1]]).unwrap()
                };
            }
        }
        self.picks.resize(
            self.system.inputs.len() + self.system.outputs.len(),
            (0, false),
        );
    }
    fn queue(&mut self, operation: Result<String, String>) -> Result<(), String> {
        match operation {
            Ok(code) => {
                self.pending = Some(Pending::Edit(code));
                self.error = None;
                self.stopped = false;
                Ok(())
            }
            Err(e) => {
                self.error = Some(e.clone());
                Err(e)
            }
        }
    }
    pub fn import(&mut self, system: System) -> Result<(), String> {
        let inputs = system
            .inputs
            .iter()
            .map(|v| (v.range[0] + v.range[1]) / 2.)
            .collect::<Vec<_>>();
        if let Err(error) = build_script(&system, &self.name, &inputs) {
            self.error = Some(error.clone());
            return Err(error);
        }
        self.input_values = inputs;
        self.pending = Some(Pending::Import(system));
        self.stopped = false;
        self.error = None;
        Ok(())
    }
    pub fn open_file(&mut self, path: &str) -> Result<(), String> {
        let q = quote(path)?;
        self.pending = Some(Pending::Open(q));
        self.stopped = false;
        self.error = None;
        Ok(())
    }
    pub fn add_variable(&mut self, output: bool, name: &str, range: &str) -> Result<(), String> {
        let operation = (|| {
            let r = numeric_text(range)?;
            Ok(format!(
                "{} = {}({}, {}, 'Name', {});\n",
                self.name,
                if output { "addOutput" } else { "addInput" },
                self.name,
                vector(&r)?,
                quote(name)?
            ))
        })();
        self.queue(operation)
    }
    pub fn remove_variable(&mut self, output: bool, index: usize) -> Result<(), String> {
        let operation = (|| {
            let v = (if output {
                &self.system.outputs
            } else {
                &self.system.inputs
            })
            .get(index)
            .ok_or("Select a variable")?;
            Ok(format!(
                "{} = {}({}, {});\n",
                self.name,
                if output {
                    "removeOutput"
                } else {
                    "removeInput"
                },
                self.name,
                quote(&v.name)?
            ))
        })();
        self.queue(operation)
    }
    pub fn add_member(
        &mut self,
        name: &str,
        kind: &str,
        parameters: &str,
        replace: bool,
    ) -> Result<(), String> {
        let operation = (|| {
            let v = self.selected().ok_or("Select a variable")?;
            let types: &[&str] = if self.output_variable && self.system.sugeno {
                &["constant", "linear"]
            } else {
                &MFS
            };
            if !types.contains(&kind) {
                return Err("Choose a supported MF type".into());
            }
            let params = numeric_text(parameters)?;
            if replace {
                let path = format!(
                    "{}.{}({}).MembershipFunctions({})",
                    self.name,
                    self.key(),
                    self.variable + 1,
                    self.member + 1
                );
                // Replace as a single fismf so a type/parameter change is atomic.
                Ok(format!(
                    "{path} = fismf({}, {}, 'Name', {});\n",
                    quote(kind)?,
                    vector(&params)?,
                    quote(name)?
                ))
            } else {
                Ok(format!(
                    "{} = addMF({}, {}, {}, {}, 'Name', {}, 'VariableType', '{}');\n",
                    self.name,
                    self.name,
                    quote(&v.name)?,
                    quote(kind)?,
                    vector(&params)?,
                    quote(name)?,
                    self.kind()
                ))
            }
        })();
        self.queue(operation)
    }
    pub fn set_rule(&mut self, text: &str, replace: Option<usize>) -> Result<(), String> {
        let operation = (|| {
            let q = quote(text)?;
            if text.trim().is_empty() {
                return Err("Enter a rule".into());
            }
            Ok(if let Some(i) = replace {
                if i >= self.system.rules.len() {
                    return Err("Select an existing rule".into());
                }
                let name = &self.name;
                format!(
                    "{name} = addRule({name}, {q});\n{name}.Rules({}) = {name}.Rules(end);\n{name}.Rules(end) = [];\n",
                    i + 1
                )
            } else {
                format!("{} = addRule({}, {q});\n", self.name, self.name)
            })
        })();
        self.queue(operation)
    }
    pub fn save_request(&self, path: &std::path::Path, key: u64) -> Result<Request, String> {
        let script = build_script(&self.system, &self.name, &self.input_values)?;
        let (body, functions) = super::split_local_functions(&script);
        let path = quote(&path.to_string_lossy())?;
        Ok(Request {
            key,
            inputs: vec![],
            code: format!("{body}writefis({}, {path});\n{functions}", self.name),
            outputs: vec![],
        })
    }
    fn edit_system(&mut self) -> Result<(), String> {
        let operation = (|| {
            let mut code = format!("{}.Name = {};\n", self.name, quote(&self.system_name)?);
            for (i, key) in METHODS.iter().enumerate() {
                if !allowed_methods(self.system.sugeno, i).contains(&self.methods[i].as_str()) {
                    return Err(format!("Invalid {key}"));
                }
                code.push_str(&format!(
                    "{}.{key} = {};\n",
                    self.name,
                    quote(&self.methods[i])?
                ));
            }
            Ok(code)
        })();
        self.queue(operation)
    }
    fn picked_rule(&self) -> Result<String, String> {
        let mut sides = [Vec::new(), Vec::new()];
        for (i, v) in self
            .system
            .inputs
            .iter()
            .chain(&self.system.outputs)
            .enumerate()
        {
            let (choice, neg) = self.picks.get(i).copied().unwrap_or_default();
            if choice == 0 {
                continue;
            }
            let m = v.members.get(choice - 1).ok_or("Choose an existing MF")?;
            let side = usize::from(i >= self.system.inputs.len());
            sides[side].push(format!(
                "{} is {}{}",
                v.name,
                if neg { "not " } else { "" },
                m.name
            ));
        }
        if sides.iter().any(Vec::is_empty) {
            return Err("Choose an antecedent and a consequent".into());
        }
        Ok(format!(
            "If {} then {} ({})",
            sides[0].join(if self.or_rule { " or " } else { " and " }),
            sides[1].join(", "),
            number(self.weight)
        ))
    }
}
fn values(o: &Outputs, key: &str) -> Result<Vec<f64>, String> {
    match o.get(key) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Fuzzy designer: missing {key}")),
    }
}
fn text(a: &Array) -> Result<String, String> {
    match a {
        Array::Character(c) => Ok(String::from_utf16_lossy(c.units())),
        _ => Err("Expected text".into()),
    }
}
fn text_rows(a: &Array) -> Result<Vec<String>, String> {
    let Array::Character(c) = a else {
        return Err("Expected rule text".into());
    };
    Ok((0..c.rows())
        .map(|r| {
            String::from_utf16_lossy(
                &(0..c.cols())
                    .map(|k| c.units()[r + k * c.rows()])
                    .collect::<Vec<_>>(),
            )
            .trim_end()
            .to_owned()
        })
        .collect())
}
fn decode(o: &Outputs) -> Result<ResultView, String> {
    Ok(ResultView {
        values: values(o, "app_fuzzy_y")?,
        firing: values(o, "app_fuzzy_firing")?,
        aggregate: values(o, "app_fuzzy_aggregate")?,
        mf_x: values(o, "app_fuzzy_mx")?,
        mf_y: values(o, "app_fuzzy_my")?,
        mf_rows: o["app_fuzzy_mx"].rows(),
        surface_x: values(o, "app_fuzzy_sx")?,
        surface_y: values(o, "app_fuzzy_sy")?,
        surface_z: values(o, "app_fuzzy_sz")?,
        surface_rows: o["app_fuzzy_sz"].rows(),
        warning: text(&o["app_fuzzy_warning"])?,
        rules: text_rows(&o["app_fuzzy_rules"])?,
    })
}
impl App for FuzzyDesigner {
    fn kind(&self) -> AppKind {
        AppKind::FuzzyLogicDesigner
    }
    fn script(&self, _: &Snapshot) -> Result<String, String> {
        build_script(&self.system, &self.name, &self.input_values)
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.script(ws)
    }
    fn execution_script(&self, _: &Snapshot) -> Result<String, String> {
        checked_name(&self.name)?;
        match &self.pending {
            Some(Pending::Open(path)) => Ok(format!("{} = readfis({path});\n", self.name)),
            Some(Pending::Import(s)) => build_script(s, &self.name, &self.input_values),
            pending => {
                let script = build_script(&self.system, &self.name, &self.input_values)?;
                let (body, functions) = super::split_local_functions(&script);
                let edit = match pending {
                    Some(Pending::Edit(edit)) => edit.as_str(),
                    _ => "",
                };
                Ok(format!("{body}{edit}{functions}"))
            }
        }
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        // Preparation depends on the accepted candidate, so it happens on the
        // Runner after engine edit validation (submit), never on the UI thread.
        (String::new(), vec![])
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, request: Request) {
        if self.stopped {
            runner.poll(ctx);
            return;
        }
        let prepared = self.prepared.clone();
        let name = self.name.clone();
        let input_values = self.input_values.clone();
        let selected = (self.output_variable, self.variable);
        let surface = (
            self.view == View::Surface,
            self.surface_inputs,
            self.surface_output,
            self.grid,
        );
        // Host hashes the script; view options are part of the task identity too.
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        request.key.hash(&mut hash);
        selected.hash(&mut hash);
        surface.hash(&mut hash);
        runner.submit_task(ctx, hash.finish(), Box::new(move |engine| {
            super::run(engine, request)?;
            let system = fuzzy_app_hooks::snapshot(engine, &name)?;
            let mut input = input_values;
            if input.len() != system.inputs.len() { input = system.inputs.iter().map(|v| (v.range[0] + v.range[1]) / 2.).collect(); }
            let mut code = format!("app_fuzzy_rules=showrule({name});\napp_fuzzy_y=[]; app_fuzzy_firing=[]; app_fuzzy_aggregate=[]; app_fuzzy_mx=[]; app_fuzzy_my=[]; app_fuzzy_sx=[]; app_fuzzy_sy=[]; app_fuzzy_sz=[]; app_fuzzy_warning='';\n");
            let vars = if selected.0 { &system.outputs } else { &system.inputs };
            if let Some(v) = vars.get(selected.1) && !v.members.is_empty() && !(selected.0 && system.sugeno) {
                code.push_str(&format!("[app_fuzzy_mx,app_fuzzy_my]=plotmf({name},'{}',{});\n", if selected.0 { "output" } else { "input" }, selected.1 + 1));
            }
            if !system.inputs.is_empty() && !system.outputs.is_empty() {
                code.push_str(&format!("lastwarn('');\n[app_fuzzy_y,app_fuzzy_im,app_fuzzy_ro,app_fuzzy_aggregate,app_fuzzy_firing]=evalfis({name},{});\napp_fuzzy_warning=lastwarn;\n", vector(&input)?));
                if surface.0 {
                    if !(2..=201).contains(&surface.3) { return Err("Surface grid must contain 2–201 points per axis".into()); }
                    let ids = if system.inputs.len() == 1 { vec![1.] } else { vec![(surface.1[0] + 1) as f64, (surface.1[1] + 1) as f64] };
                    code.push_str(&format!("[app_fuzzy_sx,app_fuzzy_sy,app_fuzzy_sz]=gensurf({name},gensurfOptions('InputIndex',{},'OutputIndex',{},'NumGridPoints',{},'ReferenceInputs',{}));\n", vector(&ids)?, surface.2 + 1, surface.3, vector(&input)?));
                }
            }
            let execution = engine.execute(&code)?;
            let outputs = ["app_fuzzy_y", "app_fuzzy_firing", "app_fuzzy_aggregate", "app_fuzzy_mx", "app_fuzzy_my", "app_fuzzy_sx", "app_fuzzy_sy", "app_fuzzy_sz", "app_fuzzy_warning", "app_fuzzy_rules"].into_iter().map(|key| (key.into(), engine.array(key).unwrap())).collect::<Outputs>();
            let mut result = decode(&outputs)?;
            if !execution.output.is_empty() { result.warning = execution.output.join("\n"); }
            *prepared.lock().map_err(|_| "Fuzzy designer result lock failed")? = Some(Prepared { system, result });
            Ok(outputs)
        }));
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        let p = self
            .prepared
            .lock()
            .map_err(|_| "Fuzzy designer result lock failed")?
            .take()
            .ok_or("Missing fuzzy view")?;
        let changed = self.system != p.system || self.pending.is_some();
        self.system = p.system;
        self.current = Some(p.result);
        self.pending = None;
        self.error = None;
        if self.input_values.len() != self.system.inputs.len() {
            self.input_values = self
                .system
                .inputs
                .iter()
                .map(|v| (v.range[0] + v.range[1]) / 2.)
                .collect();
        }
        self.surface_inputs[0] =
            self.surface_inputs[0].min(self.system.inputs.len().saturating_sub(1));
        self.surface_inputs[1] =
            self.surface_inputs[1].min(self.system.inputs.len().saturating_sub(1));
        if self.system.inputs.len() > 1 && self.surface_inputs[0] == self.surface_inputs[1] {
            self.surface_inputs[1] = (self.surface_inputs[0] + 1) % self.system.inputs.len();
        }
        self.surface_output = self
            .surface_output
            .min(self.system.outputs.len().saturating_sub(1));
        let len = if self.output_variable {
            self.system.outputs.len()
        } else {
            self.system.inputs.len()
        };
        self.variable = self.variable.min(len.saturating_sub(1));
        self.member = self.member.min(
            self.selected()
                .map_or(0, |v| v.members.len().saturating_sub(1)),
        );
        if changed {
            self.sync_fields();
        }
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _: &Snapshot) {
        self.editor(ui);
    }
    fn main_caption(&self) -> &'static str {
        "FUZZY SYSTEM"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.views(ui);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![("Rules", self.system.rules.len().to_string())];
        if let Some(r) = &self.current {
            rows.push((
                "Outputs",
                self.system
                    .outputs
                    .iter()
                    .zip(&r.values)
                    .map(|(v, x)| format!("{} = {}", v.name, short(*x)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
            if !r.warning.is_empty() {
                rows.push(("Warning", r.warning.clone()));
            }
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.add_enabled_ui(!self.running, |ui| {
            name_field(ui, "Workspace FIS", &mut self.name, ws);
        });
        super::expression_message(ui, checked_name(&self.name));
        if ui
            .add_enabled(!self.running, egui::Button::new("Save .fis"))
            .clicked()
        {
            self.save_requested = true;
        }
        if self.running && ui.button("Stop").clicked() {
            self.stop_requested = true;
        }
        if self.stopped && ui.button("Resume").clicked() {
            self.stopped = false;
        }
        if ui
            .collapsing("Script preview", |ui| match self.script(ws) {
                Ok(code) => {
                    ui.label(code);
                }
                Err(e) => {
                    ui.label(e);
                }
            })
            .header_response
            .clicked()
        {
            ui.ctx().request_repaint();
        }
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        vec![self.name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|_| {
            format!(
                "Fuzzy Logic Designer: {} inputs, {} outputs, {} rules",
                self.system.inputs.len(),
                self.system.outputs.len(),
                self.system.rules.len()
            )
        })
    }
}
fn edit_text(ui: &mut egui::Ui, label: &str, text: &mut String) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(text).char_limit(16384))
                .desired_width(ui.available_width()),
        );
    });
}
fn combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    choices: &[&str],
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.as_str())
        .show_ui(ui, |ui| {
            for &choice in choices {
                ui.selectable_value(value, choice.into(), choice);
            }
        });
}
impl FuzzyDesigner {
    fn editor(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.error {
            super::expression_message(ui, Err(error.clone()));
        }
        ui.add_enabled_ui(!self.running, |ui| {
            ui.label("System");
            ui.horizontal(|ui| {
                if ui.button("New Mamdani").clicked() {
                    let _ = self.import(empty(false));
                }
                if ui.button("New Sugeno").clicked() {
                    let _ = self.import(empty(true));
                }
            });
            egui::ComboBox::from_id_salt("fuzzy_workspace")
                .selected_text(if self.workspace_name.is_empty() {
                    "Workspace FIS"
                } else {
                    &self.workspace_name
                })
                .show_ui(ui, |ui| {
                    for (name, s) in &self.catalogue {
                        ui.selectable_value(
                            &mut self.workspace_name,
                            name.clone(),
                            format!("{name} ({})", if s.sugeno { "sugfis" } else { "mamfis" }),
                        );
                    }
                });
            if ui.button("Import workspace FIS").clicked()
                && let Some((_, system)) = self
                    .catalogue
                    .iter()
                    .find(|(n, _)| n == &self.workspace_name)
            {
                let _ = self.import(system.clone());
            }
            if ui.button("Open .fis").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Fuzzy system", &["fis"])
                    .pick_file()
                && let Err(e) = self.open_file(&path.to_string_lossy())
            {
                self.error = Some(e);
            }
            edit_text(ui, "System name", &mut self.system_name);
            for (i, label) in METHODS.iter().enumerate() {
                ui.label(*label);
                combo(
                    ui,
                    ("fuzzy_method", i),
                    &mut self.methods[i],
                    allowed_methods(self.system.sugeno, i),
                );
            }
            if ui.button("Apply system settings").clicked() {
                let _ = self.edit_system();
            }
            ui.add_space(crate::tokens::M);
            ui.label("Variables");
            for output in [false, true] {
                let vars = if output {
                    &self.system.outputs
                } else {
                    &self.system.inputs
                };
                let mut select = None;
                for (i, v) in vars.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.output_variable == output && self.variable == i,
                            format!("{}: {}", if output { "Output" } else { "Input" }, v.name),
                        )
                        .clicked()
                    {
                        select = Some(i);
                    }
                }
                if let Some(i) = select {
                    self.output_variable = output;
                    self.variable = i;
                    self.member = 0;
                    self.sync_fields();
                }
            }
            ui.horizontal(|ui| {
                for output in [false, true] {
                    if ui
                        .button(if output { "+ Output" } else { "+ Input" })
                        .clicked()
                    {
                        let vars = if output {
                            &self.system.outputs
                        } else {
                            &self.system.inputs
                        };
                        let name =
                            super::unused_name(if output { "output" } else { "input" }, |n| {
                                vars.iter().any(|v| v.name == n)
                            });
                        let _ = self.add_variable(output, &name, "[0 1]");
                    }
                }
            });
            if self.selected().is_some() {
                edit_text(ui, "Variable name", &mut self.variable_name);
                edit_text(ui, "Range", &mut self.variable_range);
                super::expression_message(ui, numeric_text(&self.variable_range).map(|_| ()));
                ui.horizontal(|ui| {
                    if ui.button("Apply variable").clicked() {
                        let operation = (|| {
                            let range = numeric_text(&self.variable_range)?;
                            let path =
                                format!("{}.{}({})", self.name, self.key(), self.variable + 1);
                            Ok(format!(
                                "{path}.Name = {};\n{path}.Range = {};\n",
                                quote(&self.variable_name)?,
                                vector(&range)?
                            ))
                        })();
                        let _ = self.queue(operation);
                    }
                    if ui.button("Remove variable").clicked() {
                        let _ = self.remove_variable(self.output_variable, self.variable);
                    }
                });
                ui.add_space(crate::tokens::M);
                ui.label("Membership functions");
                let v = self.selected().unwrap().clone();
                egui::ScrollArea::vertical()
                    .id_salt("fuzzy_mfs")
                    .max_height(160.)
                    .show_themed(ui, |ui| {
                        for (i, m) in v.members.iter().enumerate() {
                            if ui.selectable_label(self.member == i, &m.name).clicked() {
                                self.member = i;
                                self.sync_fields();
                            }
                        }
                    });
                edit_text(ui, "MF name", &mut self.mf_name);
                let types: &[&str] = if self.output_variable && self.system.sugeno {
                    &["constant", "linear"]
                } else {
                    &MFS
                };
                combo(ui, "fuzzy_mf_type", &mut self.mf_kind, types);
                edit_text(ui, "Parameters", &mut self.mf_parameters);
                let count = match self.mf_kind.as_str() {
                    "trimf" | "gbellmf" => 3,
                    "trapmf" | "gauss2mf" | "dsigmf" | "psigmf" | "pimf" => 4,
                    "constant" => 1,
                    "linear" => self.system.inputs.len() + 1,
                    _ => 2,
                };
                crate::tokens::helper(ui, format!("{count} finite parameters required"));
                super::expression_message(
                    ui,
                    numeric_text(&self.mf_parameters).and_then(|p| {
                        if p.len() == count {
                            Ok(())
                        } else {
                            Err(format!("Expected {count} parameters"))
                        }
                    }),
                );
                ui.horizontal(|ui| {
                    for (label, replace) in [("Add MF", false), ("Apply MF", true)] {
                        if ui.button(label).clicked() {
                            let _ = self.add_member(
                                &self.mf_name.clone(),
                                &self.mf_kind.clone(),
                                &self.mf_parameters.clone(),
                                replace,
                            );
                        }
                    }
                    if ui.button("Remove MF").clicked() {
                        let op = v
                            .members
                            .get(self.member)
                            .ok_or_else(|| "Select an MF".to_owned())
                            .and_then(|m| {
                                Ok(format!(
                                    "{} = removeMF({}, {}, {}, 'VariableType', '{}');\n",
                                    self.name,
                                    self.name,
                                    quote(&v.name)?,
                                    quote(&m.name)?,
                                    self.kind()
                                ))
                            });
                        let _ = self.queue(op);
                    }
                });
            }
            ui.add_space(crate::tokens::M);
            ui.label("Rules");
            let rules = self
                .current
                .as_ref()
                .map_or_else(Vec::new, |r| r.rules.clone());
            egui::ScrollArea::vertical()
                .id_salt("fuzzy_rules")
                .max_height(160.)
                .show_rows(ui, crate::tokens::ROW, rules.len(), |ui, range| {
                    for i in range {
                        if ui
                            .selectable_label(self.rule == Some(i), &rules[i])
                            .clicked()
                        {
                            self.rule = Some(i);
                            self.rule_text = rules[i].clone();
                        }
                    }
                });
            field(ui, "Rule text", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::multiline(&mut self.rule_text).char_limit(16384),
                    )
                    .desired_rows(3)
                    .desired_width(ui.available_width()),
                );
            });
            ui.horizontal(|ui| {
                if ui.button("Add rule").clicked() {
                    let _ = self.set_rule(&self.rule_text.clone(), None);
                }
                if ui
                    .add_enabled(self.rule.is_some(), egui::Button::new("Apply rule"))
                    .clicked()
                {
                    let _ = self.set_rule(&self.rule_text.clone(), self.rule);
                }
                if ui
                    .add_enabled(self.rule.is_some(), egui::Button::new("Remove rule"))
                    .clicked()
                {
                    let _ = self.queue(Ok(format!(
                        "{}.Rules({}) = [];\n",
                        self.name,
                        self.rule.unwrap() + 1
                    )));
                    self.rule = None;
                }
            });
            ui.collapsing("Build a rule", |ui| {
                self.picks.resize(
                    self.system.inputs.len() + self.system.outputs.len(),
                    (0, false),
                );
                for (i, v) in self
                    .system
                    .inputs
                    .iter()
                    .chain(&self.system.outputs)
                    .enumerate()
                {
                    ui.label(&v.name);
                    let (choice, neg) = &mut self.picks[i];
                    egui::ComboBox::from_id_salt(("fuzzy_pick", i))
                        .selected_text(if *choice == 0 {
                            "Any / unused"
                        } else {
                            v.members
                                .get(*choice - 1)
                                .map_or("Missing MF", |m| m.name.as_str())
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(choice, 0, "Any / unused");
                            for (j, m) in v.members.iter().enumerate() {
                                ui.selectable_value(choice, j + 1, &m.name);
                            }
                        });
                    ui.add_enabled_ui(i < self.system.inputs.len() || !self.system.sugeno, |ui| {
                        ui.checkbox(neg, "NOT");
                    });
                }
                ui.checkbox(&mut self.or_rule, "Connect inputs with OR");
                ui.add(egui::Slider::new(&mut self.weight, 0. ..=1.).text("Weight"));
                if ui.button("Use picked rule").clicked() {
                    match self.picked_rule() {
                        Ok(s) => {
                            self.rule_text = s;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            });
        });
    }
    fn views(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (view, label) in [
                (View::Memberships, "Memberships"),
                (View::Inference, "Rule inference"),
                (View::Surface, "Surface"),
                (View::System, "System"),
            ] {
                ui.selectable_value(&mut self.view, view, label);
            }
        });
        ui.add_space(crate::tokens::S);
        egui::ScrollArea::vertical()
            .id_salt("fuzzy_view")
            .show_themed(ui, |ui| {
                if self.view == View::Inference || self.view == View::Surface {
                    for (i, v) in self.system.inputs.iter().enumerate() {
                        if let Some(x) = self.input_values.get_mut(i) {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Slider::new(x, v.range[0]..=v.range[1])
                                        .text(&v.name)
                                        .clamping(egui::SliderClamping::Never),
                                );
                            });
                        }
                    }
                }
                if self.view == View::Surface {
                    ui.horizontal_wrapped(|ui| {
                        for (axis, label) in ["Horizontal input", "Vertical input"]
                            .into_iter()
                            .enumerate()
                        {
                            if axis == 1 && self.system.inputs.len() < 2 {
                                continue;
                            }
                            egui::ComboBox::from_id_salt(("fuzzy_surface_input", axis))
                                .selected_text(format!(
                                    "{label}: {}",
                                    self.system
                                        .inputs
                                        .get(self.surface_inputs[axis])
                                        .map_or("—", |v| v.name.as_str())
                                ))
                                .show_ui(ui, |ui| {
                                    for (i, v) in self.system.inputs.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.surface_inputs[axis],
                                            i,
                                            &v.name,
                                        );
                                    }
                                });
                        }
                        egui::ComboBox::from_id_salt("fuzzy_surface_output")
                            .selected_text(
                                self.system
                                    .outputs
                                    .get(self.surface_output)
                                    .map_or("Output", |v| v.name.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for (i, v) in self.system.outputs.iter().enumerate() {
                                    ui.selectable_value(&mut self.surface_output, i, &v.name);
                                }
                            });
                        ui.label("Grid points");
                        ui.add(egui::DragValue::new(&mut self.grid).range(2..=201));
                    });
                    crate::tokens::helper(
                        ui,
                        "Other inputs use the reference values above. Stop cancels a running grid.",
                    );
                }
                let Some(r) = &self.current else {
                    ui.label("Create or open a system to inspect it.");
                    return;
                };
                match self.view {
                    View::Memberships => {
                        if self.output_variable && self.system.sugeno {
                            ui.label("Sugeno consequents");
                            if let Some(v) = self.selected().cloned() {
                                for (i, m) in v.members.iter().enumerate() {
                                    ui.selectable_label(
                                        i == self.member,
                                        format!(
                                            "{}: {} {}",
                                            m.name,
                                            m.kind,
                                            vector(&m.parameters).unwrap_or_default()
                                        ),
                                    )
                                    .clicked()
                                    .then(|| self.member = i);
                                }
                            }
                        } else if r.mf_rows > 0 {
                            let palette = crate::tokens::palette(ui.ctx());
                            plot("fuzzy_memberships", "Variable value", "Membership").show(
                                ui,
                                |p| {
                                    for (i, (xs, ys)) in r
                                        .mf_x
                                        .chunks(r.mf_rows)
                                        .zip(r.mf_y.chunks(r.mf_rows))
                                        .enumerate()
                                    {
                                        let label = self
                                            .selected()
                                            .and_then(|v| v.members.get(i))
                                            .map_or("MF", |m| m.name.as_str());
                                        p.line(
                                            egui_plot::Line::new(label, pairs(xs, ys))
                                                .color(if i == self.member {
                                                    palette.accent
                                                } else {
                                                    palette.text_muted
                                                })
                                                .width(if i == self.member { 3. } else { 1. }),
                                        );
                                    }
                                },
                            );
                        } else {
                            ui.label("Select a variable and add membership functions.");
                        }
                    }
                    View::Inference => {
                        let palette = crate::tokens::palette(ui.ctx());
                        for (i, (v, value)) in self.system.outputs.iter().zip(&r.values).enumerate()
                        {
                            ui.label(format!("{} = {}", v.name, short(*value)));
                            if !self.system.sugeno {
                                let n = r.aggregate.len() / self.system.outputs.len().max(1);
                                if n > 0 {
                                    let ys = &r.aggregate[i * n..(i + 1) * n];
                                    let xs = (0..n)
                                        .map(|k| {
                                            v.range[0]
                                                + (v.range[1] - v.range[0]) * k as f64
                                                    / (n - 1).max(1) as f64
                                        })
                                        .collect::<Vec<_>>();
                                    plot(
                                        &format!("fuzzy_aggregate_{i}"),
                                        &v.name,
                                        "Aggregated membership",
                                    )
                                    .show(ui, |p| {
                                        p.line(
                                            egui_plot::Line::new("Aggregate", pairs(&xs, ys))
                                                .color(palette.accent),
                                        );
                                        p.vline(
                                            egui_plot::VLine::new("Defuzzified", *value)
                                                .color(palette.text),
                                        );
                                    });
                                }
                            }
                        }
                        ui.label("Rule firing strengths");
                        egui::ScrollArea::vertical()
                            .id_salt("fuzzy_firing")
                            .max_height(192.)
                            .show_rows(ui, crate::tokens::ROW, r.firing.len(), |ui, range| {
                                for i in range {
                                    ui.label(format!("Rule {}: {}", i + 1, short(r.firing[i])));
                                }
                            });
                    }
                    View::Surface => {
                        if r.surface_y.is_empty() {
                            let accent = crate::tokens::palette(ui.ctx()).accent;
                            plot("fuzzy_surface_curve", "Input", "Output").show(ui, |p| {
                                p.line(
                                    egui_plot::Line::new(
                                        "Response",
                                        pairs(&r.surface_x, &r.surface_z),
                                    )
                                    .color(accent),
                                );
                            });
                        } else {
                            surface_plot(ui, r, &self.system, self.surface_inputs);
                        }
                    }
                    View::System => diagram(ui, &self.system),
                }
            });
    }
}
fn pairs(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    x.iter().zip(y).map(|(x, y)| [*x, *y]).collect()
}
fn plot<'a>(id: &str, x: &str, y: &str) -> egui_plot::Plot<'a> {
    egui_plot::Plot::new(id.to_owned())
        .height(240.)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .show_background(false)
        .legend(egui_plot::Legend::default())
        .x_axis_label(x.to_owned())
        .y_axis_label(y.to_owned())
}
fn surface_plot(ui: &mut egui::Ui, r: &ResultView, system: &System, inputs: [usize; 2]) {
    let n = r.surface_rows;
    if n < 2 || r.surface_x.len() < n * n {
        return;
    }
    let xmin = r.surface_x[0];
    let xmax = r.surface_x[n * (n - 1)];
    let ymin = r.surface_y[0];
    let ymax = r.surface_y[n - 1];
    let dx = (xmax - xmin) / (n - 1) as f64;
    let dy = (ymax - ymin) / (n - 1) as f64;
    let lo = r.surface_z.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = r
        .surface_z
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let data = (0..n * n)
        .map(|k| r.surface_z[k / n + (k % n) * n])
        .collect();
    let palette = crate::tokens::palette(ui.ctx());
    crate::tokens::helper(ui, format!("Output range: {} to {}", short(lo), short(hi)));
    let labels = inputs.map(|i| system.inputs.get(i).map_or("Input", |v| v.name.as_str()));
    plot("fuzzy_surface", labels[0], labels[1]).show(ui, |p| {
        p.heatmap(
            egui_plot::Heatmap::new(data, n)
                .at(egui_plot::PlotPoint::new(xmin - dx / 2., ymin - dy / 2.))
                .tile_size(dx as f32, dy as f32)
                .show_labels(false)
                .custom_mapping(Box::new(move |v| {
                    let t = if hi > lo {
                        ((v - lo) / (hi - lo)).clamp(0., 1.) as f32
                    } else {
                        0.5
                    };
                    egui::ecolor::tint_color_towards(
                        palette.surface,
                        palette.accent.gamma_multiply(0.2 + 0.8 * t),
                    )
                })),
        );
    });
}
fn diagram(ui: &mut egui::Ui, s: &System) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            (s.inputs.len().max(s.outputs.len()) as f32 * 48.).max(240.),
        ),
        egui::Sense::hover(),
    );
    let palette = crate::tokens::palette(ui.ctx());
    let painter = ui.painter_at(rect);
    let font = egui::FontId::proportional(crate::tokens::CAPTION);
    let center = rect.center();
    let w = (rect.width() / 4.).max(48.);
    let block = egui::Rect::from_center_size(center, egui::vec2(w, 96.));
    painter.rect_stroke(
        block,
        0.,
        egui::Stroke::new(1., palette.accent),
        egui::StrokeKind::Inside,
    );
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        format!(
            "{}\n{}\n{} rules",
            s.name,
            if s.sugeno { "Sugeno" } else { "Mamdani" },
            s.rules.len()
        ),
        font.clone(),
        palette.text,
    );
    for (left, vars) in [(true, &s.inputs), (false, &s.outputs)] {
        for (i, v) in vars.iter().enumerate() {
            let y = rect.top() + (i + 1) as f32 * rect.height() / (vars.len() + 1) as f32;
            let x = if left {
                rect.left() + w / 2.
            } else {
                rect.right() - w / 2.
            };
            let box_rect = egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(w, 40.));
            painter.rect_stroke(
                box_rect,
                0.,
                egui::Stroke::new(1., palette.text_muted),
                egui::StrokeKind::Inside,
            );
            painter.text(
                box_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{}\n{} MFs", v.name, v.members.len()),
                font.clone(),
                palette.text,
            );
            painter.line_segment(
                if left {
                    [box_rect.right_center(), block.left_center()]
                } else {
                    [block.right_center(), box_rect.left_center()]
                },
                egui::Stroke::new(1., palette.text_muted),
            );
        }
    }
}

pub(crate) fn show(
    host: &mut Host<FuzzyDesigner>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.set_folder(ws.folder());
    host.refresh(ui.ctx(), ws);
    if let Some(e) = host.script_error.as_ref().or(host.run_error.as_ref()) {
        host.app.error = Some(e.clone());
    }
    host.app.running = host.running() || host.app.save_runner.running();
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.stop_requested) {
        stop(host);
    }
    if let Some(e) = host.script_error.as_ref().or(host.run_error.as_ref()) {
        host.app.error = Some(e.clone());
    }
    if std::mem::take(&mut host.app.save_requested) {
        let path = host.app.save_path.take().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Fuzzy system", &["fis"])
                .set_file_name("controller.fis")
                .save_file()
        });
        if let Some(path) = path {
            host.app.serial += 1;
            let request = host.app.save_request(&path, host.app.serial);
            match request {
                Ok(r) => {
                    host.app.save_runner.set_folder(ws.folder());
                    host.app.save_runner.submit(ui.ctx(), r);
                }
                Err(e) => host.run_error = Some(e),
            }
        }
    }
    host.app.save_runner.poll(ui.ctx());
    if host.app.save_runner.key() != host.app.save_delivered
        && let Some(result) = host.app.save_runner.result()
    {
        host.app.save_delivered = host.app.save_runner.key();
        match result {
            Ok(_) => {
                host.message = "Saved fuzzy system".into();
                host.run_error = None;
                host.app.error = None;
            }
            Err(e) => host.run_error = Some(e.clone()),
        }
    }
    if let Some(e) = &host.app.error {
        host.run_error = Some(e.clone());
    }
}
pub(crate) fn stop(host: &mut Host<FuzzyDesigner>) {
    host.app.stopped = true;
    host.runner.stop();
    host.app.save_runner.stop();
    host.run_error = Some("Fuzzy designer: stopped; last good result retained".into());
}
pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .fuzzy_designer
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if workspace
        .worker
        .cancel
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    if let Some(system) = launch {
        let host = &mut workspace.toolkits.apps.fuzzy;
        host.runner.stop();
        host.delivered = None;
        if let Err(e) = host.app.import(system) {
            host.run_error = Some(e);
        }
        workspace.open_app(crate::toolkits::Tool::App(AppKind::FuzzyLogicDesigner));
    }
}
