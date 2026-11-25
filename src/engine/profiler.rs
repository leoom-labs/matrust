//! Engine-only profiler, based on the public profile/profsave reference pages:
//! https://www.mathworks.com/help/matlab/ref/profile.html
//! https://www.mathworks.com/help/matlab/ref/profsave.html
//! No MATLAB execution or private implementation is used. The requested legacy
//! contract uses 2-row history (off by default), with one million event slots.
use super::{Engine, Execution, Value, user_functions::Program};
use crate::{Array, CharacterRow, Matrix, Result, StructArray, clock::Instant};
use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path, sync::Arc};

// Selected builtin dispatchers bypass Engine::call (notably multiple outputs).
// Keep their disabled path to the same single branch, with no clock or storage.
#[inline]
pub(super) fn dispatch<T>(
    e: &mut Engine,
    name: &str,
    run: impl FnOnce(&mut Engine) -> Result<T>,
) -> Result<T> {
    if e.profiler.enabled {
        builtin(e, name, run)
    } else {
        run(e)
    }
}

const MAX_FUNCTIONS: usize = 1024;
const MAX_LINES: usize = 65536;
const MAX_EDGES: usize = 4096;
const MAX_SOURCE: usize = 16 * 1024 * 1024;
const MAX_HISTORY: usize = 1_000_000;
const FIELDS: &[&str] = &[
    "CompleteName",
    "FunctionName",
    "FileName",
    "Type",
    "Children",
    "Parents",
    "ExecutedLines",
    "IsRecursive",
    "TotalRecursiveTime",
    "PartialData",
    "NumCalls",
    "TotalTime",
];

#[derive(Clone, Debug, Default)]
struct CountTime {
    calls: usize,
    time: f64,
}
#[derive(Clone, Debug)]
struct Function {
    name: String,
    file: String,
    kind: &'static str,
    source: Arc<str>,
    line_starts: Vec<usize>,
    calls: usize,
    total: f64,
    recursive: bool,
    partial: bool,
    children: BTreeMap<usize, CountTime>,
    parents: BTreeMap<usize, usize>,
    lines: BTreeMap<usize, CountTime>,
}
#[derive(Clone, Debug)]
struct Frame {
    index: usize,
    start: Instant,
    recursive: bool,
    line: Option<(usize, Instant)>,
    suppress_lines: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Profiler {
    viewer_snapshot: Option<Array>,
    pub(super) enabled: bool,
    timer: String,
    history_on: bool,
    history_size: usize,
    history: Vec<f64>,
    functions: Vec<Function>,
    indices: BTreeMap<(String, String, &'static str), usize>,
    stack: Vec<Frame>,
    generation: u64,
    source_bytes: usize,
    line_count: usize,
    edge_count: usize,
}
impl Default for Profiler {
    fn default() -> Self {
        Self {
            viewer_snapshot: None,
            enabled: false,
            timer: "performance".into(),
            history_on: false,
            history_size: MAX_HISTORY,
            history: Vec::new(),
            functions: Vec::new(),
            indices: BTreeMap::new(),
            stack: Vec::new(),
            generation: 0,
            source_bytes: 0,
            line_count: 0,
            edge_count: 0,
        }
    }
}
impl Profiler {
    fn event(&mut self, exit: bool, index: usize) {
        if self.history_on && self.history.len() / 2 < self.history_size {
            self.history.extend([f64::from(exit), (index + 1) as f64]);
        }
    }
    fn enter(
        &mut self,
        name: &str,
        file: String,
        kind: &'static str,
        source: Arc<str>,
    ) -> Result<(u64, usize)> {
        let key = (file.clone(), name.to_owned(), kind);
        let index = if let Some(&index) = self.indices.get(&key) {
            self.functions[index].partial |= self.functions[index].source != source;
            index
        } else {
            let retained = source
                .len()
                .saturating_add(name.len().saturating_mul(2))
                .saturating_add(file.len().saturating_mul(2));
            if self.functions.len() >= MAX_FUNCTIONS
                || self.source_bytes.saturating_add(retained) > MAX_SOURCE
            {
                return Err("profile: function/source storage limit exceeded".into());
            }
            self.source_bytes += retained;
            let mut line_starts = vec![0];
            line_starts.extend(
                source
                    .bytes()
                    .enumerate()
                    .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
            );
            let index = self.functions.len();
            self.functions.push(Function {
                name: name.into(),
                file,
                kind,
                source,
                line_starts,
                calls: 0,
                total: 0.,
                recursive: false,
                partial: false,
                children: BTreeMap::new(),
                parents: BTreeMap::new(),
                lines: BTreeMap::new(),
            });
            self.indices.insert(key, index);
            index
        };
        if let Some(parent) = self.stack.last().map(|f| f.index) {
            if !self.functions[parent].children.contains_key(&index) {
                if self.edge_count >= MAX_EDGES {
                    return Err("profile: call graph storage limit exceeded".into());
                }
                self.edge_count += 1;
            }
            self.functions[parent]
                .children
                .entry(index)
                .or_default()
                .calls += 1;
            *self.functions[index].parents.entry(parent).or_default() += 1;
        }
        let recursion_start = self.stack.iter().position(|f| f.index == index);
        let recursive = recursion_start.is_some();
        if let Some(start) = recursion_start {
            for frame in &self.stack[start..] {
                self.functions[frame.index].recursive = true;
            }
        }
        self.functions[index].calls += 1;
        self.event(false, index);
        self.stack.push(Frame {
            index,
            start: Instant::now(),
            recursive,
            line: None,
            suppress_lines: false,
        });
        Ok((self.generation, self.stack.len()))
    }
    fn flush_line(&mut self, now: Instant) {
        if let Some(frame) = self.stack.last_mut()
            && let Some((line, start)) = frame.line.take()
        {
            self.functions[frame.index]
                .lines
                .get_mut(&line)
                .unwrap()
                .time += now.duration_since(start).as_secs_f64();
        }
    }
    pub(super) fn line(&mut self, byte: usize) -> Result<()> {
        // Eval spans belong to different source text. Leave the calling line
        // active, while functions called by eval get their own normal frames.
        if self.stack.last().is_some_and(|f| f.suppress_lines) {
            return Ok(());
        }
        let now = Instant::now();
        self.flush_line(now);
        let Some(frame) = self.stack.last_mut() else {
            return Ok(());
        };
        let function = &mut self.functions[frame.index];
        if function.kind == "Builtin-function" || function.source.is_empty() {
            return Ok(());
        }
        let line = function.line_starts.partition_point(|&start| start <= byte);
        if !function.lines.contains_key(&line) {
            if self.line_count >= MAX_LINES {
                return Err("profile: executed-line storage limit exceeded".into());
            }
            self.line_count += 1;
        }
        function.lines.entry(line).or_default().calls += 1;
        frame.line = Some((line, now));
        Ok(())
    }
    fn leave(&mut self, token: (u64, usize)) {
        // off/info/clear inside a function can already have closed this frame.
        if token != (self.generation, self.stack.len()) {
            return;
        }
        let now = Instant::now();
        self.flush_line(now);
        let frame = self.stack.pop().unwrap();
        let elapsed = now.duration_since(frame.start).as_secs_f64();
        // Public docs: direct recursive entries count calls, but total includes
        // only the outermost invocation, avoiding repeated inclusive time.
        if !frame.recursive {
            self.functions[frame.index].total += elapsed;
        }
        // A recursive parent's entire interval is already inside a child
        // interval of its outermost frame. Charging its outgoing calls again
        // would overlap that interval (even for distinct children like pause).
        // Keep every edge's call count, but time only the outermost parent.
        if let Some(parent) = self.stack.last()
            && !parent.recursive
        {
            self.functions[parent.index]
                .children
                .get_mut(&frame.index)
                .unwrap()
                .time += elapsed;
        }
        self.event(true, frame.index);
    }
    fn stop(&mut self) {
        while !self.stack.is_empty() {
            self.leave((self.generation, self.stack.len()));
        }
        self.enabled = false;
        self.generation = self.generation.wrapping_add(1);
    }
    fn clear(&mut self) {
        self.stop();
        self.functions.clear();
        self.indices.clear();
        self.history.clear();
        self.source_bytes = 0;
        self.line_count = 0;
        self.edge_count = 0;
    }
}

/// Run eval text without interpreting its byte offsets as caller source.
/// Keep the frame so calls made by the text retain their parent relationship.
pub(super) fn eval_text<T>(
    e: &mut Engine,
    run: impl FnOnce(&mut Engine) -> Result<T>,
) -> Result<T> {
    if !e.profiler.enabled {
        return run(e);
    }
    let generation = e.profiler.generation;
    let depth = e.profiler.stack.len();
    let previous = e
        .profiler
        .stack
        .last_mut()
        .map(|frame| std::mem::replace(&mut frame.suppress_lines, true));
    let result = run(e);
    // profile off/clear/on in the evaluated text may have replaced the stack.
    if e.profiler.generation == generation
        && e.profiler.stack.len() == depth
        && let Some(previous) = previous
        && let Some(frame) = e.profiler.stack.last_mut()
    {
        frame.suppress_lines = previous;
    }
    result
}

pub(super) fn program_call<T>(
    e: &mut Engine,
    program: &Program,
    name: &str,
    kind: &'static str,
    run: impl FnOnce(&mut Engine) -> Result<T>,
) -> Result<T> {
    let file = program
        .path
        .as_ref()
        .map_or_else(String::new, |p| super::env_files::envx_path_text(p));
    let token = e.profiler.enter(name, file, kind, program.source.clone())?;
    let result = run(e);
    e.profiler.leave(token);
    result
}
pub(super) fn anonymous<T>(
    e: &mut Engine,
    name: &str,
    run: impl FnOnce(&mut Engine) -> Result<T>,
) -> Result<T> {
    let file = e
        .functions
        .active_path()
        .map_or_else(String::new, super::env_files::envx_path_text);
    let token = e
        .profiler
        .enter(name, file, "M-anonymous-function", Arc::from(""))?;
    let result = run(e);
    e.profiler.leave(token);
    result
}
pub(super) fn builtin<T>(
    e: &mut Engine,
    name: &str,
    run: impl FnOnce(&mut Engine) -> Result<T>,
) -> Result<T> {
    if matches!(name, "profile" | "profsave") {
        return run(e);
    }
    let token = e
        .profiler
        .enter(name, String::new(), "Builtin-function", Arc::from(""))?;
    let result = run(e);
    e.profiler.leave(token);
    result
}
fn text(value: &Value) -> Result<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Ok(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => s.data()[0]
            .as_ref()
            .map(|s| s.to_string())
            .ok_or("profile: missing string".into()),
        _ => Err("profile: expected a character vector or string scalar".into()),
    }
}
fn number(n: f64) -> Array {
    Matrix::scalar(n).into()
}
fn chars(e: &Engine, s: &str) -> Result<Array> {
    Ok(CharacterRow::from_text(s, &e.cancel)?.into())
}
fn structure(e: &Engine, fields: &[&str], rows: Vec<Vec<Array>>) -> Result<Array> {
    Ok(StructArray::from_slots(
        rows.len(),
        1,
        fields.iter().map(|s| (*s).into()).collect(),
        rows.into_iter().flatten().map(Arc::new).collect(),
        &e.cancel,
    )?
    .into())
}
fn status(e: &Engine) -> Result<Value> {
    let p = &e.profiler;
    Ok(structure(
        e,
        &[
            "ProfilerStatus",
            "DetailLevel",
            "Timer",
            "HistoryTracking",
            "HistorySize",
        ],
        vec![vec![
            chars(e, if p.enabled { "on" } else { "off" })?,
            chars(e, "mmex")?,
            chars(e, &p.timer)?,
            chars(e, if p.history_on { "on" } else { "off" })?,
            number(p.history_size as f64),
        ]],
    )?
    .into())
}
fn info(e: &Engine) -> Result<Value> {
    let mut rows = Vec::new();
    for f in &e.profiler.functions {
        e.check_cancel()?;
        let children = structure(
            e,
            &["FunctionTableIndex", "Index", "NumCalls", "TotalTime"],
            f.children
                .iter()
                .map(|(&index, c)| {
                    vec![
                        number((index + 1) as f64),
                        number((index + 1) as f64),
                        number(c.calls as f64),
                        number(c.time),
                    ]
                })
                .collect(),
        )?;
        let parents = structure(
            e,
            &["FunctionTableIndex", "Index", "NumCalls"],
            f.parents
                .iter()
                .map(|(&index, &calls)| {
                    vec![
                        number((index + 1) as f64),
                        number((index + 1) as f64),
                        number(calls as f64),
                    ]
                })
                .collect(),
        )?;
        let mut lines = Vec::with_capacity(f.lines.len() * 3);
        lines.extend(f.lines.keys().map(|&line| line as f64));
        lines.extend(f.lines.values().map(|c| c.calls as f64));
        lines.extend(f.lines.values().map(|c| c.time));
        let complete = if f.file.is_empty() {
            f.name.clone()
        } else {
            Path::new(&f.file)
                .parent()
                .unwrap_or(Path::new(""))
                .join(&f.name)
                .display()
                .to_string()
        };
        rows.push(vec![
            chars(e, &complete)?,
            chars(e, &f.name)?,
            chars(e, &f.file)?,
            chars(e, f.kind)?,
            children,
            parents,
            Matrix::from_column_vec(f.lines.len(), 3, lines)?.into(),
            number(f64::from(f.recursive)),
            number(0.),
            number(f64::from(f.partial)),
            number(f.calls as f64),
            number(f.total),
        ]);
    }
    Ok(structure(
        e,
        &[
            "FunctionTable",
            "FunctionHistory",
            "ClockPrecision",
            "ClockSpeed",
            "Name",
            "Overhead",
        ],
        vec![vec![
            structure(e, FIELDS, rows)?,
            Matrix::from_column_vec(2, e.profiler.history.len() / 2, e.profiler.history.clone())?
                .into(),
            number(1e-9),
            number(0.),
            chars(e, "MATLAB")?,
            number(0.),
        ]],
    )?
    .into())
}

impl Engine {
    /// Consume the desktop viewer event without affecting CLI text output.
    /// Hosts call this after a job, including indirect calls from function files.
    pub fn take_profiler_viewer(&mut self) -> Option<Result<Array>> {
        self.profiler.viewer_snapshot.take().map(Ok)
    }

    /// Stop collection and return the profiler snapshot for a host Profiler app.
    /// The returned immutable arrays are shared with workspace assignments.
    pub fn profiler_info(&mut self) -> Result<Array> {
        self.profiler.stop();
        info(self)?.array(&self.cancel)
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    e.check_cancel()?;
    if name == "profsave" {
        return save(e, args);
    }
    if args.is_empty() {
        return Err("profile: Not enough input arguments. Specify an action or option.".into());
    }
    let first = text(&args[0])?;
    let (action, mut i) = if first.starts_with('-') {
        ("", 0)
    } else {
        (first.as_str(), 1)
    };
    if !matches!(
        action,
        "" | "on" | "resume" | "off" | "clear" | "viewer" | "info" | "status"
    ) {
        return Err(format!("profile: Invalid action '{action}'."));
    }
    if i < args.len() && !matches!(action, "" | "on" | "resume") {
        return Err("profile: Too many input arguments.".into());
    }
    if action.is_empty() && e.profiler.enabled {
        return Err("profile: Turn profiling off before changing options.".into());
    }
    let mut timer = e.profiler.timer.clone();
    let mut history = e.profiler.history_on;
    let mut size = e.profiler.history_size;
    while i < args.len() {
        let option = text(&args[i])?;
        i += 1;
        match option.as_str() {
            "-history" => history = true,
            "-nohistory" => history = false,
            "-timer" => {
                timer = text(args.get(i).ok_or("profile: Missing timer value.")?)?;
                i += 1;
                if !matches!(timer.as_str(), "real" | "cpu" | "performance") {
                    return Err("profile: Invalid timer option.".into());
                }
            }
            "-historysize" => {
                let value = args.get(i).ok_or("profile: Missing history size.")?;
                i += 1;
                let n = match value {
                    Value::Matrix(m) => m.as_scalar()?,
                    _ => text(value)?
                        .parse::<f64>()
                        .map_err(|_| "profile: Invalid history size.")?,
                };
                if !n.is_finite() || n < 1. || n.fract() != 0. || n > MAX_HISTORY as f64 {
                    return Err(format!(
                        "profile: History size must be an integer from 1 to {MAX_HISTORY}."
                    ));
                }
                size = n as usize;
            }
            _ => return Err(format!("profile: Invalid option '{option}'.")),
        }
    }
    e.profiler.timer = timer;
    e.profiler.history_on = history;
    e.profiler.history_size = size;
    e.profiler.history.truncate(size * 2);
    match action {
        "on" => {
            e.profiler.clear();
            e.profiler.enabled = true;
        }
        "resume" => {
            e.profiler.enabled = true;
        }
        "clear" => e.profiler.clear(),
        "off" => e.profiler.stop(),
        "info" => return Ok(e.profiler_info()?.into()),
        "status" => return status(e),
        "viewer" => {
            e.profiler.stop();
            e.profiler.viewer_snapshot = Some(e.profiler_info()?);
            super::app_launchers::Launch::App(crate::session::ToolTab::Profiler).record();
            let table = viewer(e);
            e.emit(out, table)?;
        }
        _ => (),
    }
    Ok(Value::Void)
}

/// Desktop integration hook: replace/present this engine-owned snapshot in the
/// host UI. No UI or browser dependency is needed for profiling or HTML export.
pub(super) fn viewer(e: &Engine) -> String {
    let mut functions: Vec<_> = e.profiler.functions.iter().collect();
    functions.sort_by(|a, b| b.total.total_cmp(&a.total));
    let mut table = String::from("Function\tCalls\tTotal time\tSelf time\n");
    for f in functions {
        let children: f64 = f.children.values().map(|c| c.time).sum();
        let _ = writeln!(
            table,
            "{}\t{}\t{:.6}\t{:.6}",
            f.name,
            f.calls,
            f.total,
            f.total - children
        );
    }
    table
}
fn field<'a>(s: &'a StructArray, index: usize, name: &str) -> Result<&'a Array> {
    Ok(s.get(
        s.field_index(name)
            .ok_or_else(|| format!("profsave: Missing {name} field."))?,
        index,
    )?
    .as_ref())
}
fn scalar(a: &Array) -> Result<f64> {
    if let Array::Numeric(m) = a {
        let n = m.as_scalar()?;
        if n.is_finite() && n >= 0. {
            return Ok(n);
        }
    }
    Err("profsave: Expected a finite nonnegative scalar.".into())
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn save(e: &mut Engine, args: &[Value]) -> Result<Value> {
    Engine::arity_check("profsave", args, &[0, 1, 2])?;
    let snapshot;
    let value = if let Some(value) = args.first() {
        value
    } else {
        e.profiler.stop();
        snapshot = info(e)?;
        &snapshot
    };
    let Value::Container(Array::Struct(p)) = value else {
        return Err("profsave: Expected profile info structure.".into());
    };
    if p.len() != 1 {
        return Err("profsave: Expected scalar profile info.".into());
    }
    let Array::Struct(functions) = field(p, 0, "FunctionTable")? else {
        return Err("profsave: Invalid FunctionTable.".into());
    };
    if functions.len() > MAX_FUNCTIONS {
        return Err("profsave: Too many functions.".into());
    }
    let directory = args
        .get(1)
        .map(text)
        .transpose()?
        .unwrap_or_else(|| "profile_results".into());
    let directory = e
        .file_directory
        .as_deref()
        .unwrap_or(Path::new("."))
        .join(directory);
    // Build and validate bounded documents before writing any files.
    let mut documents = Vec::new();
    let mut summary = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>Profile results</title><h1>Profile results</h1><table><tr><th>Function</th><th>Calls</th><th>Total time</th><th>Self time</th></tr>",
    );
    let mut total_bytes = 0;
    for i in 0..functions.len() {
        e.check_cancel()?;
        let name = text(&Value::from(field(functions, i, "FunctionName")?.clone()))?;
        let file = text(&Value::from(field(functions, i, "FileName")?.clone()))?;
        if name.len().saturating_add(file.len()) > MAX_SOURCE {
            return Err("profsave: Function metadata is too large.".into());
        }
        let calls = scalar(field(functions, i, "NumCalls")?)?;
        let time = scalar(field(functions, i, "TotalTime")?)?;
        let Array::Numeric(lines) = field(functions, i, "ExecutedLines")? else {
            return Err("profsave: Invalid ExecutedLines.".into());
        };
        if lines.cols() != 3
            || lines.rows() > MAX_LINES
            || lines.data().iter().any(|n| !n.is_finite() || *n < 0.)
        {
            return Err("profsave: Invalid ExecutedLines.".into());
        }
        let Array::Struct(children) = field(functions, i, "Children")? else {
            return Err("profsave: Invalid Children.".into());
        };
        let mut child_time = 0.;
        for j in 0..children.len() {
            child_time += scalar(field(children, j, "TotalTime")?)?;
        }
        let escaped = escape(&name);
        let _ = write!(
            summary,
            "<tr><td><a href=\"file{}.html\">{escaped}</a></td><td>{calls}</td><td>{time:.9}</td><td>{:.9}</td></tr>",
            i + 1,
            time - child_time
        );
        let source = if let Some(f) = e
            .profiler
            .functions
            .iter()
            .find(|f| f.name == name && f.file == file)
        {
            f.source.to_string()
        } else if file.is_empty() {
            String::new()
        } else {
            use std::io::Read;
            let mut bytes = Vec::new();
            fs::File::open(&file)
                .map_err(|err| format!("profsave: {err}"))?
                .take((MAX_SOURCE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|err| format!("profsave: {err}"))?;
            if bytes.len() > MAX_SOURCE {
                return Err("profsave: Source file is too large.".into());
            }
            let source = crate::files::decode_script_source(std::path::Path::new(&file), bytes)
                .map_err(|err| format!("profsave: {err}"))?;
            if source.len() > MAX_SOURCE {
                return Err("profsave: Source file is too large.".into());
            }
            source
        };
        let mut document = format!(
            "<!doctype html><meta charset=\"utf-8\"><title>{escaped}</title><h1>{escaped}</h1><table><tr><th>Line</th><th>Calls</th><th>Time</th><th>Source</th></tr>"
        );
        let n = lines.rows();
        let mut stats = BTreeMap::new();
        for j in 0..n {
            let line = lines.data()[j];
            if line < 1. || line.fract() != 0. || line > MAX_SOURCE as f64 {
                return Err("profsave: Invalid source line.".into());
            }
            stats.insert(
                line as usize,
                (lines.data()[n + j], lines.data()[2 * n + j]),
            );
        }
        for (j, line) in source.lines().enumerate() {
            if j.is_multiple_of(1024) {
                e.check_cancel()?;
            }
            let (calls, time) = stats.get(&(j + 1)).copied().unwrap_or_default();
            let _ = write!(
                document,
                "<tr><td>{}</td><td>{calls}</td><td>{time:.9}</td><td><pre>{}</pre></td></tr>",
                j + 1,
                escape(line)
            );
            if document.len() > MAX_SOURCE * 4 {
                return Err("profsave: HTML output limit exceeded.".into());
            }
        }
        document.push_str("</table>");
        total_bytes += document.len();
        if total_bytes.saturating_add(summary.len()) > MAX_SOURCE * 4 {
            return Err("profsave: HTML output limit exceeded.".into());
        }
        documents.push(document);
    }
    summary.push_str("</table>");
    fs::create_dir_all(&directory).map_err(|err| format!("profsave: {err}"))?;
    for (i, document) in std::iter::once(summary).chain(documents).enumerate() {
        e.check_cancel()?;
        fs::write(directory.join(format!("file{i}.html")), document)
            .map_err(|err| format!("profsave: {err}"))?;
    }
    Ok(Value::Void)
}
