//! Clean-room profiler viewer; public contracts: MathWorks profile/profsave.
//! The script builder drives the Runner, workspace export and saved scripts.
//! Timing and counts come only from the engine. Source I/O runs in the Runner.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, short};
use crate::toolkits::Action;
use eframe::egui;
use matrust::{Array, StructArray};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

const INFO: &str = "app_profiler_info";
const SOURCES: &str = "app_profiler_sources";
const ELAPSED: &str = "app_profiler_elapsed";
const MAX_CODE: usize = 16 * 1024;
const MAX_SCRIPT: usize = 900 * 1024;
const MAX_FUNCTIONS: usize = 1024;
const MAX_SOURCE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Type,
    Calls,
    Total,
    SelfTime,
}
#[derive(Clone, Debug)]
pub struct Edge {
    pub index: usize,
    pub calls: usize,
    pub time: f64,
}
#[derive(Clone, Debug)]
pub struct Line {
    pub number: usize,
    pub calls: usize,
    pub time: f64,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub calls: usize,
    pub total: f64,
    pub self_time: f64,
    pub parents: Vec<Edge>,
    pub children: Vec<Edge>,
    pub lines: BTreeMap<usize, Line>,
    pub source: Arc<Vec<String>>,
    pub hottest: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct Profile {
    pub info: Array,
    pub functions: Vec<Function>,
    pub total: f64,
}
#[derive(Clone)]
enum Mode {
    Empty,
    Run {
        code: String,
        history: bool,
        serial: u64,
    },
    Current(String, f64),
    Stopped,
}

pub struct Profiler {
    pub code: String,
    pub history: bool,
    pub name: String,
    pub current: Option<Profile>,
    pub sort: Sort,
    pub descending: bool,
    pub order: Vec<usize>,
    pub selected: Option<usize>,
    back: Vec<Option<usize>>,
    mode: Mode,
    serial: u64,
    inputs: Vec<String>,
    pub running: bool,
    stop_requested: bool,
    current_requested: bool,
    clear_requested: bool,
    pub html_folder: String,
    html_requested: bool,
    html_runner: Runner,
    html_delivered: Option<u64>,
}
impl Default for Profiler {
    fn default() -> Self {
        Self {
            code: "sum(1:1000)".into(),
            history: false,
            name: "profile_info".into(),
            current: None,
            sort: Sort::SelfTime,
            descending: true,
            order: Vec::new(),
            selected: None,
            back: Vec::new(),
            mode: Mode::Empty,
            serial: 0,
            inputs: Vec::new(),
            running: false,
            stop_requested: false,
            current_requested: false,
            clear_requested: false,
            html_folder: String::new(),
            html_requested: false,
            html_runner: Runner::default(),
            html_delivered: None,
        }
    }
}

fn quote(text: &str) -> String {
    if text.chars().any(char::is_control) {
        format!(
            "char([{}])",
            text.encode_utf16()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        format!("'{}'", text.replace('\'', "''"))
    }
}
fn structure(value: &Array) -> Result<&StructArray, String> {
    match value {
        Array::Struct(s) => Ok(s),
        _ => Err("Profiler: expected a profile structure".into()),
    }
}
fn get<'a>(s: &'a StructArray, row: usize, name: &str) -> Result<&'a Array, String> {
    s.get(
        s.field_index(name)
            .ok_or_else(|| format!("Profiler: missing {name}"))?,
        row,
    )
    .map(|v| v.as_ref())
}
fn scalar(value: &Array) -> Result<f64, String> {
    match value {
        Array::Numeric(m) if m.as_scalar().is_ok_and(|n| n.is_finite() && n >= 0.) => {
            Ok(m.as_scalar().unwrap())
        }
        _ => Err("Profiler: expected a finite nonnegative scalar".into()),
    }
}
fn count(value: &Array) -> Result<usize, String> {
    let n = scalar(value)?;
    if n.fract() != 0. || n > usize::MAX as f64 {
        return Err("Profiler: invalid count".into());
    }
    Ok(n as usize)
}
fn text(value: &Array) -> Result<String, String> {
    match value {
        Array::Character(c) => c.to_utf8(&AtomicBool::new(false)),
        _ => Err("Profiler: expected character text".into()),
    }
}

/// Validate an entire statement list, then validate it in the exact wrapper.
/// Newlines isolate trailing comments; definitions belong in a function file.
pub fn validate_code(code: &str) -> Result<(), String> {
    if code.trim().is_empty() || code.len() > MAX_CODE {
        return Err(format!("Profiler: enter 1–{MAX_CODE} bytes of code"));
    }
    let statements = matrust::syntax::parse(code).map_err(|e| format!("Profiler: {e}"))?;
    fn definitions(statements: &[matrust::syntax::Statement]) -> bool {
        use matrust::syntax::StatementKind as S;
        statements.iter().any(|s| match &s.kind {
            S::Function(_) => true,
            S::If {
                branches,
                otherwise,
            } => branches.iter().any(|b| definitions(&b.body)) || definitions(otherwise),
            S::While { body, .. } | S::For { body, .. } | S::SerialBlock { body, .. } => {
                definitions(body)
            }
            S::Try { body, handler, .. } => definitions(body) || definitions(handler),
            S::Switch {
                cases, otherwise, ..
            } => cases.iter().any(|c| definitions(&c.body)) || definitions(otherwise),
            _ => false,
        })
    }
    if statements.is_empty() || definitions(&statements) {
        return Err("Profiler: use executable statements or call a function file".into());
    }
    matrust::syntax::parse(&format!(
        "profile on;\n{code}\n;\nprofile off;\n{INFO} = profile('info');\n"
    ))
    .map_err(|e| format!("Profiler: {e}"))?;
    Ok(())
}

/// Serialize into small assignments: the engine bounds each statement to 256
/// expression nodes. Large line/history arrays are written in 128-value chunks.
/// All values retain their original shape; no engine limit is relaxed.
fn snapshot_script(value: &Array) -> Result<String, String> {
    struct Writer {
        script: String,
        names: Vec<String>,
        cached: BTreeMap<String, String>,
    }
    impl Writer {
        fn emit(&mut self, line: String) -> Result<(), String> {
            if self.script.len().saturating_add(line.len()) > MAX_SCRIPT {
                return Err("Profiler: profile script exceeds its size limit".into());
            }
            self.script.push_str(&line);
            Ok(())
        }
        fn name(&mut self) -> String {
            let name = format!("app_profiler_v{}", self.names.len());
            self.names.push(name.clone());
            name
        }
        fn cached(&mut self, expression: String) -> Result<String, String> {
            if let Some(name) = self.cached.get(&expression) {
                return Ok(name.clone());
            }
            let name = self.name();
            self.emit(format!("{name} = {expression};\n"))?;
            self.cached.insert(expression, name.clone());
            Ok(name)
        }
        fn numeric(&mut self, rows: usize, cols: usize, values: &[f64]) -> Result<String, String> {
            if rows == 1 && cols == 1 {
                return Ok(super::number(values[0]));
            }
            if values.len() <= 128 {
                let values = values
                    .iter()
                    .map(|n| super::number(*n))
                    .collect::<Vec<_>>()
                    .join(" ");
                return self.cached(format!("reshape([{values}], {rows}, {cols})"));
            }
            let name = self.name();
            self.emit(format!("{name} = zeros({rows}, {cols});\n"))?;
            for (i, chunk) in values.chunks(128).enumerate() {
                let start = i * 128 + 1;
                let end = start + chunk.len() - 1;
                let values = chunk
                    .iter()
                    .map(|n| super::number(*n))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.emit(format!("{name}({start}:{end}) = [{values}];\n"))?;
            }
            Ok(name)
        }
        fn value(&mut self, value: &Array, depth: usize) -> Result<String, String> {
            if depth > 8 {
                return Err("Profiler: profile data nesting limit exceeded".into());
            }
            match value {
                Array::Numeric(m) => self.numeric(m.rows(), m.cols(), m.data()),
                Array::Character(_) => {
                    let text = text(value)?;
                    if text.chars().any(char::is_control) {
                        let units: Vec<_> = text.encode_utf16().map(f64::from).collect();
                        let value = self.numeric(1, units.len(), &units)?;
                        self.cached(format!("char({value})"))
                    } else {
                        Ok(quote(&text))
                    }
                }
                Array::Struct(s) => {
                    if s.field_names().len() > 16 {
                        return Err("Profiler: too many structure fields".into());
                    }
                    if s.is_empty() {
                        let fields = s
                            .field_names()
                            .iter()
                            .map(|name| {
                                format!("{}, cell({}, {})", quote(name), s.rows(), s.cols())
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        return self.cached(format!("struct({fields})"));
                    }
                    let target = self.name();
                    for row in 0..s.len() {
                        let mut fields = Vec::new();
                        for (field, name) in s.field_names().iter().enumerate() {
                            let value = self.value(s.get(field, row)?, depth + 1)?;
                            fields.push(format!("{}, {value}", quote(name)));
                        }
                        let destination = if row == 0 {
                            target.clone()
                        } else {
                            format!("{target}({})", row + 1)
                        };
                        self.emit(format!("{destination} = struct({});\n", fields.join(", ")))?;
                    }
                    if s.len() != 1 {
                        self.emit(format!(
                            "{target} = reshape({target}, {}, {});\n",
                            s.rows(),
                            s.cols()
                        ))?;
                    }
                    Ok(target)
                }
                _ => Err("Profiler: unsupported value in profile data".into()),
            }
        }
    }
    let mut writer = Writer {
        script: String::new(),
        names: Vec::new(),
        cached: BTreeMap::new(),
    };
    let value = writer.value(value, 0)?;
    writer.emit(format!("app_profiler_snapshot = {value};\n"))?;
    for names in writer.names.clone().chunks(128) {
        writer.emit(format!("clear {};\n", names.join(" ")))?;
    }
    Ok(writer.script)
}

/// Build the profiling run or a standalone serialization of current data.
fn script_text(mode: &Mode, name: &str) -> Result<String, String> {
    super::single_expression(name)?;
    crate::workspace_edit::validate_name(name)?;
    super::check_export_name(name)?;
    if name.starts_with("app_profiler_") || matches!(name, "profile" | "profsave") {
        return Err("Profiler: choose an unreserved output name".into());
    }
    let body = match mode {
        Mode::Empty => format!("profile clear;\n{name} = profile('info');\n{ELAPSED} = 0;\n"),
        Mode::Stopped => return Err("Profiler: stopped; last good result retained".into()),
        Mode::Current(value, total) => {
            format!(
                "{value}{name} = app_profiler_snapshot;\n{ELAPSED} = {};\nclear app_profiler_snapshot;\n",
                super::number(*total)
            )
        }
        Mode::Run {
            code,
            history,
            serial,
        } => {
            validate_code(code)?;
            let history = if *history { "-history" } else { "-nohistory" };
            format!(
                "% Profiler run {serial}\ntic;\napp_profiler_clock = tic;\nprofile on {history};\n{code}\n;\nprofile off;\nif exist('app_profiler_clock', 'var')\n{ELAPSED} = toc(app_profiler_clock);\nelse\n{ELAPSED} = toc;\nend\n{name} = profile('info');\nclear app_profiler_clock;\n"
            )
        }
    };
    // Free text was parsed above; generated literals use only fixed grammar.
    Ok(body)
}

pub fn decode(info: &Array, sources: Option<&Array>) -> Result<Profile, String> {
    let p = structure(info)?;
    if p.len() != 1 {
        return Err("Profiler: expected scalar profile info".into());
    }
    let table = structure(get(p, 0, "FunctionTable")?)?;
    if table.len() > MAX_FUNCTIONS {
        return Err("Profiler: at most 1024 functions".into());
    }
    let mut functions = Vec::new();
    let mut source_bytes = 0usize;
    let mut source_cache = BTreeMap::<String, Arc<Vec<String>>>::new();
    for i in 0..table.len() {
        let edges = |field: &str| -> Result<Vec<Edge>, String> {
            let values = structure(get(table, i, field)?)?;
            if values.len() > 4096 {
                return Err("Profiler: too many call edges".into());
            }
            (0..values.len())
                .map(|j| {
                    let index = count(get(values, j, "Index")?)?;
                    if index == 0 || index > table.len() {
                        return Err("Profiler: invalid function index".into());
                    }
                    Ok(Edge {
                        index: index - 1,
                        calls: count(get(values, j, "NumCalls")?)?,
                        time: if field == "Children" {
                            scalar(get(values, j, "TotalTime")?)?
                        } else {
                            0.
                        },
                    })
                })
                .collect()
        };
        let parents = edges("Parents")?;
        let children = edges("Children")?;
        let total = scalar(get(table, i, "TotalTime")?)?;
        let self_time = (total - children.iter().map(|e| e.time).sum::<f64>()).max(0.);
        let Array::Numeric(executed) = get(table, i, "ExecutedLines")? else {
            return Err("Profiler: invalid line table".into());
        };
        if executed.cols() != 3 || executed.rows() > 65536 {
            return Err("Profiler: invalid line table size".into());
        }
        let mut lines = BTreeMap::new();
        let n = executed.rows();
        for j in 0..n {
            let number = executed.data()[j];
            let calls = executed.data()[j + n];
            let time = executed.data()[j + 2 * n];
            if !number.is_finite()
                || number < 1.
                || number.fract() != 0.
                || number > 65536.
                || !calls.is_finite()
                || calls < 0.
                || calls.fract() != 0.
                || calls > usize::MAX as f64
                || !time.is_finite()
                || time < 0.
            {
                return Err("Profiler: invalid line measurements".into());
            }
            lines.insert(
                number as usize,
                Line {
                    number: number as usize,
                    calls: calls as usize,
                    time,
                },
            );
        }
        let mut hottest: Vec<_> = lines.values().collect();
        hottest.sort_by(|a, b| b.time.total_cmp(&a.time).then(a.number.cmp(&b.number)));
        let hottest = hottest.into_iter().take(5).map(|l| l.number).collect();
        let file = text(get(table, i, "FileName")?)?;
        let source = if let Some(source) = source_cache.get(&file) {
            source.clone()
        } else {
            let lines = if !file.is_empty() {
                if let Some(Array::Cell(s)) = sources {
                    let value = s.get_linear(i).map_err(|e| format!("Profiler: {e}"))?;
                    let source = text(value)?;
                    source_bytes = source_bytes.saturating_add(source.len());
                    if source.len() > MAX_SOURCE || source_bytes > 16 * MAX_SOURCE {
                        Vec::new()
                    } else {
                        let mut source = source.lines();
                        let lines = source.by_ref().take(65536).map(str::to_owned).collect();
                        if source.next().is_some() {
                            Vec::new()
                        } else {
                            lines
                        }
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let source = Arc::new(lines);
            source_cache.insert(file.clone(), source.clone());
            source
        };
        functions.push(Function {
            name: text(get(table, i, "FunctionName")?)?,
            kind: text(get(table, i, "Type")?)?,
            file,
            calls: count(get(table, i, "NumCalls")?)?,
            total,
            self_time,
            parents,
            children,
            lines,
            source,
            hottest,
        });
    }
    let total = functions
        .iter()
        .filter(|f| f.parents.is_empty())
        .map(|f| f.total)
        .sum();
    Ok(Profile {
        info: info.clone(),
        functions,
        total,
    })
}

impl Profiler {
    pub fn run(&mut self) {
        self.stop_requested = false;
        self.serial += 1;
        self.mode = Mode::Run {
            code: self.code.clone(),
            history: self.history,
            serial: self.serial,
        };
    }
    pub fn stop(&mut self) {
        self.mode = Mode::Stopped;
        self.stop_requested = true;
        self.html_runner.stop();
    }
    pub fn use_snapshot(&mut self, info: Array) -> Result<(), String> {
        let mut next = decode(&info, None)?;
        if let Some(previous) = &self.current
            && previous.info.same_snapshot(&info)
        {
            next.total = previous.total;
        }
        let value = snapshot_script(&info)?;
        self.stop_requested = false;
        self.mode = Mode::Current(value, next.total);
        self.current = Some(next);
        self.reset_view();
        Ok(())
    }
    fn reset_view(&mut self) {
        self.selected = None;
        self.back.clear();
        self.sort_rows();
    }
    pub fn sort_by(&mut self, column: Sort) {
        if self.sort == column {
            self.descending = !self.descending;
        } else {
            self.sort = column;
            self.descending = !matches!(column, Sort::Name | Sort::Type);
        }
        self.sort_rows();
    }
    fn sort_rows(&mut self) {
        let Some(p) = &self.current else {
            self.order.clear();
            return;
        };
        self.order = (0..p.functions.len()).collect();
        self.order.sort_by(|&a, &b| {
            let (a, b) = (&p.functions[a], &p.functions[b]);
            let order = match self.sort {
                Sort::Name => a.name.cmp(&b.name),
                Sort::Type => a.kind.cmp(&b.kind),
                Sort::Calls => a.calls.cmp(&b.calls),
                Sort::Total => a.total.total_cmp(&b.total),
                Sort::SelfTime => a.self_time.total_cmp(&b.self_time),
            };
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then(a.name.cmp(&b.name))
        });
    }
    pub fn navigate(&mut self, index: usize) {
        if self
            .current
            .as_ref()
            .is_some_and(|p| index < p.functions.len())
            && self.selected != Some(index)
        {
            self.back.push(self.selected);
            self.selected = Some(index);
        }
    }
    pub fn back(&mut self) {
        self.selected = self.back.pop().flatten();
    }
    pub fn html_request(&self, folder: &str, key: u64) -> Result<Request, String> {
        if folder.trim().is_empty() || folder.len() > 4096 || folder.chars().any(char::is_control) {
            return Err("Profiler: choose a report folder (at most 4096 characters)".into());
        }
        let folder = quote(folder);
        super::single_expression(&folder)?;
        let p = self
            .current
            .as_ref()
            .ok_or("Profiler: run code before saving a report")?;
        let value = snapshot_script(&p.info)?;
        let script = script_text(&Mode::Current(value, p.total), "profile_report")?;
        Ok(Request {
            key,
            inputs: Vec::new(),
            code: format!("{script}profsave(profile_report, {folder});\n"),
            outputs: Vec::new(),
        })
    }
}

impl App for Profiler {
    fn kind(&self) -> AppKind {
        AppKind::Profiler
    }
    fn script(&self, _ws: &Snapshot) -> Result<String, String> {
        script_text(&self.mode, &self.name)
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        let profile = self
            .current
            .as_ref()
            .ok_or("Profiler: no profile to export")?;
        let snapshot = snapshot_script(&profile.info)?;
        let body = script_text(&Mode::Current(snapshot, profile.total), &self.name)?;
        let helper = super::unused_name("app_profiler_export_snapshot", |name| ws.contains(name));
        // A local function confines serialization helpers (including elapsed
        // time) to its own workspace. Export never executes the profiled code.
        Ok(format!(
            "{name} = {helper}();\nfunction {name} = {helper}()\n{body}end\n",
            name = self.name
        ))
    }
    fn inputs(&self) -> Vec<String> {
        self.inputs.clone()
    }
    fn prepare(&mut self, ws: &Snapshot) {
        self.inputs = if matches!(self.mode, Mode::Run { .. }) {
            ws.variables().map(|(n, _)| n.to_owned()).collect()
        } else {
            Vec::new()
        };
    }
    fn probe(&self, _ws: &Snapshot) -> (String, Vec<String>) {
        let name = &self.name;
        // File I/O belongs to the worker, after profiling stops. Missing source
        // leaves the measurements usable. The engine bounds fileread allocation.
        (
            format!(
                "{INFO} = {name};\n{SOURCES} = cell(numel({INFO}.FunctionTable), 1);\n\
            for app_profiler_k = 1:numel({INFO}.FunctionTable)\n\
              {SOURCES}{{app_profiler_k}} = '';\n\
              if ~isempty({INFO}.FunctionTable(app_profiler_k).FileName)\n\
                app_profiler_source_index = 0;\n\
                for app_profiler_j = 1:app_profiler_k-1\n\
                  if strcmp({INFO}.FunctionTable(app_profiler_j).FileName, {INFO}.FunctionTable(app_profiler_k).FileName)\n\
                    app_profiler_source_index = app_profiler_j; break;\n\
                  end\n\
                end\n\
                if app_profiler_source_index > 0\n\
                  {SOURCES}{{app_profiler_k}} = {SOURCES}{{app_profiler_source_index}};\n\
                else\n\
                  try\n\
                    {SOURCES}{{app_profiler_k}} = fileread({INFO}.FunctionTable(app_profiler_k).FileName);\n\
                  catch\n\
                    {SOURCES}{{app_profiler_k}} = '';\n\
                  end\n\
                end\n\
              end\n\
            end\n"
            ),
            vec![INFO.into(), SOURCES.into(), ELAPSED.into()],
        )
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let info = outputs.get(INFO).ok_or("Profiler: missing profile info")?;
        let mut next = decode(info, outputs.get(SOURCES))?;
        next.total = scalar(
            outputs
                .get(ELAPSED)
                .ok_or("Profiler: missing elapsed time")?,
        )?;
        self.current = Some(next);
        self.reset_view();
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        field(ui, "Code to run", |ui| {
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::multiline(&mut self.code).char_limit(MAX_CODE),
                )
                .desired_rows(8)
                .desired_width(ui.available_width()),
            );
        });
        ui.checkbox(&mut self.history, "Record call history");
        ui.add_space(crate::tokens::S);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.running, egui::Button::new("Run and Profile"))
                .clicked()
            {
                self.run();
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop();
            }
        });
        ui.add_space(crate::tokens::S);
        if ui.button("Use current profile data").clicked() {
            self.current_requested = true;
        }
        if ui.button("Clear").clicked() {
            self.clear_requested = true;
        }
        crate::tokens::helper(
            ui,
            "Function files are resolved in the current folder. Times include profiling overhead.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "PROFILE"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        if self.current.as_ref().is_none_or(|p| p.functions.is_empty()) {
            crate::tokens::helper(
                ui,
                "Run code or use an existing profile to inspect calls and timings.",
            );
            return;
        }
        if let Some(index) = self.selected {
            if ui.button("Back").clicked() {
                self.back();
                return;
            }
            let p = self.current.as_ref().unwrap();
            let f = &p.functions[index];
            ui.label(&f.name);
            super::result_block(
                ui,
                &[
                    ("Type", f.kind.clone()),
                    ("Calls", f.calls.to_string()),
                    ("Total time (s)", short(f.total)),
                    ("Self time (s)", short(f.self_time)),
                ],
            );
            let mut navigate = None;
            for (title, edges) in [("Parents", &f.parents), ("Children", &f.children)] {
                ui.add_space(crate::tokens::S);
                ui.label(title);
                egui::Grid::new(("profiler_edges", title))
                    .num_columns(3)
                    .show(ui, |ui| {
                        for edge in edges {
                            if ui.link(&p.functions[edge.index].name).clicked() {
                                navigate = Some(edge.index);
                            }
                            ui.label(format!("{} calls", edge.calls));
                            ui.label(if title == "Children" {
                                format!("{} s", short(edge.time))
                            } else {
                                String::new()
                            });
                            ui.end_row();
                        }
                    });
            }
            ui.add_space(crate::tokens::S);
            if f.file.is_empty() {
                crate::tokens::helper(ui, "Built-in function: no source listing.");
            } else if f.source.is_empty() {
                crate::tokens::helper(
                    ui,
                    "Source listing unavailable (file missing or listing limit exceeded).",
                );
            } else {
                ui.label(&f.file);
                crate::tokens::helper(
                    ui,
                    "Mark = five slowest executed lines. Dim lines were not executed by this function.",
                );
                let palette = crate::tokens::palette(ui.ctx());
                egui::ScrollArea::both()
                    .id_salt("profiler_source")
                    .show_rows(ui, crate::tokens::CONTROL, f.source.len(), |ui, range| {
                        for i in range {
                            let line = f.lines.get(&(i + 1));
                            let calls = line.map_or(0, |l| l.calls);
                            let time = line.map_or(0., |l| l.time);
                            let mark = if f.hottest.contains(&(i + 1)) {
                                "•"
                            } else {
                                " "
                            };
                            let color = if calls == 0 {
                                palette.text_disabled
                            } else {
                                palette.text
                            };
                            ui.label(
                                egui::RichText::new(format!(
                                    "{mark} {:4}  {:6}  {:>10} s   {}",
                                    i + 1,
                                    calls,
                                    short(time),
                                    f.source[i]
                                ))
                                .color(color),
                            );
                        }
                    });
            }
            if let Some(index) = navigate {
                self.navigate(index);
            }
        } else {
            ui.horizontal(|ui| {
                for (sort, label) in [
                    (Sort::Name, "Function"),
                    (Sort::Type, "Type"),
                    (Sort::Calls, "Calls"),
                    (Sort::Total, "Total time"),
                    (Sort::SelfTime, "Self time"),
                ] {
                    let arrow = if sort == self.sort {
                        if self.descending { " ↓" } else { " ↑" }
                    } else {
                        ""
                    };
                    if ui.button(format!("{label}{arrow}")).clicked() {
                        self.sort_by(sort);
                    }
                }
            });
            let p = self.current.as_ref().unwrap();
            let max = p.functions.iter().map(|f| f.self_time).fold(0., f64::max);
            let mut navigate = None;
            egui::ScrollArea::both()
                .id_salt("profiler_summary")
                .show_rows(ui, crate::tokens::CONTROL, self.order.len(), |ui, range| {
                    egui::Grid::new("profiler_rows")
                        .num_columns(6)
                        .min_col_width(64.)
                        .show(ui, |ui| {
                            for row in range {
                                let index = self.order[row];
                                let f = &p.functions[index];
                                if ui.selectable_label(false, &f.name).clicked() {
                                    navigate = Some(index);
                                }
                                ui.label(&f.kind);
                                ui.label(f.calls.to_string());
                                ui.label(format!("{} s", short(f.total)));
                                ui.label(format!("{} s", short(f.self_time)));
                                let (rect, _) = ui
                                    .allocate_exact_size(egui::vec2(80., 8.), egui::Sense::hover());
                                if max > 0. {
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_size(
                                            rect.min,
                                            egui::vec2(
                                                rect.width() * (f.self_time / max) as f32,
                                                rect.height(),
                                            ),
                                        ),
                                        0.,
                                        crate::tokens::palette(ui.ctx()).accent,
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
            if let Some(index) = navigate {
                self.navigate(index);
            }
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(p) = &self.current else {
            return Vec::new();
        };
        let slowest = p
            .functions
            .iter()
            .max_by(|a, b| a.self_time.total_cmp(&b.self_time));
        let line = p
            .functions
            .iter()
            .flat_map(|f| f.lines.values().map(move |l| (f, l)))
            .max_by(|a, b| a.1.time.total_cmp(&b.1.time));
        vec![
            (
                if matches!(self.mode, Mode::Run { .. }) {
                    "Total run time (s)"
                } else {
                    "Recorded call time (s)"
                },
                short(p.total),
            ),
            ("Functions", p.functions.len().to_string()),
            (
                "Slowest self time",
                slowest.map_or_else(
                    || "—".into(),
                    |f| format!("{} ({} s)", f.name, short(f.self_time)),
                ),
            ),
            (
                "Slowest line",
                line.map_or_else(
                    || "—".into(),
                    |(f, l)| format!("{}:{} ({} s)", f.name, l.number, short(l.time)),
                ),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Profile info variable", &mut self.name, ws);
        field(ui, "HTML report folder", |ui| {
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.html_folder).char_limit(4096),
                )
                .desired_width(ui.available_width()),
            );
        });
        #[cfg(not(test))]
        if ui.button("Choose folder…").clicked()
            && let Some(folder) = rfd::FileDialog::new().pick_folder()
        {
            self.html_folder = folder.to_string_lossy().into_owned();
        }
        if ui
            .add_enabled(
                !self.running && self.current.is_some(),
                egui::Button::new("Save HTML report"),
            )
            .clicked()
        {
            self.html_requested = true;
        }
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        vec![self.name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|p| {
            format!(
                "Profiler: {} functions, {} s",
                p.functions.len(),
                short(p.total)
            )
        })
    }
}

pub(crate) fn show(
    host: &mut Host<Profiler>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.set_folder(ws.folder());
    host.app.running = host.running() || host.app.html_runner.running();
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.stop_requested) {
        host.runner.stop();
        host.app.html_runner.stop();
        host.run_error = Some("Profiler: stopped; last good result retained".into());
    }
    if !busy && std::mem::take(&mut host.app.current_requested) {
        *action = Some(Action::Script("profile viewer".into()));
    }
    if !busy && std::mem::take(&mut host.app.clear_requested) {
        *action = Some(Action::Script("profile clear; profile viewer".into()));
    }
    if std::mem::take(&mut host.app.html_requested) {
        host.app.html_runner.set_folder(ws.folder());
        host.app.serial += 1;
        match host
            .app
            .html_request(&host.app.html_folder, host.app.serial)
        {
            Ok(request) => {
                host.app.html_runner.submit(ui.ctx(), request);
                host.message = "Saving HTML report…".into();
            }
            Err(error) => host.run_error = Some(error),
        }
    }
    host.app.html_runner.poll(ui.ctx());
    if host.app.html_runner.key() != host.app.html_delivered
        && let Some(result) = host.app.html_runner.result()
    {
        host.app.html_delivered = host.app.html_runner.key();
        match result {
            Ok(_) => {
                host.message = "HTML report saved".into();
                host.run_error = None;
            }
            Err(error) => host.run_error = Some(error.clone()),
        }
    }
}

/// A real engine event, so indirect/script calls to profile viewer also open it.
pub(crate) fn receive_viewer(workspace: &mut crate::Workspace, info: Result<Array, String>) {
    let host = &mut workspace.toolkits.apps.profiler;
    host.runner.stop();
    host.delivered = None;
    match info.and_then(|info| host.app.use_snapshot(info)) {
        Ok(()) => {
            host.run_error = None;
            host.script_error = None;
        }
        Err(error) => host.run_error = Some(error),
    }
    let tool = crate::toolkits::Tool::App(AppKind::Profiler);
    workspace.toolkits.open_tool(tool);
    workspace.select_tool_tab(tool);
    workspace.apps_home = false;
}
