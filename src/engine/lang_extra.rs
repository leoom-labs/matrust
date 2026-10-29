//! Language helpers: bsxfun, builtin, which/what, inline objects,
//! clearvars, functions/localfunctions/dbstack, diary and environment queries.
//! `inline` objects use the tagged-struct facade of lang_more (kind 3).
use super::{Engine, Execution, Value, lang_more};
use crate::{
    Array, CellArray, FunctionHandle, LogicalMatrix, Matrix, Result, StructArray,
    syntax::{self, Expr},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const FEW: &str = "Not enough input arguments.";
const MANY: &str = "Too many input arguments.";
const STRING: &str = "Argument must contain a string.";
/// Longest list `what`/`clearvars` handle and longest inline expression.
const LIMIT: usize = 65536;
const KEYWORDS: [&str; 20] = [
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "for",
    "function",
    "global",
    "if",
    "otherwise",
    "parfor",
    "persistent",
    "return",
    "spmd",
    "switch",
    "try",
    "while",
];
/// Functions whose `which` result in the reference names a .m file; for
/// them, `builtin` cannot call them and they are not methods of double.
const M_FILES: &[&str] = &[
    "blanks",
    "circshift",
    "clearvars",
    "cross",
    "cumtrapz",
    "datenum",
    "datestr",
    "deal",
    "deblank",
    "dot",
    "factorial",
    "fieldnames",
    "fileparts",
    "flip",
    "fliplr",
    "flipud",
    "fullfile",
    "gcd",
    "genvarname",
    "int2str",
    "interp1",
    "intersect",
    "iscolumn",
    "iskeyword",
    "ismember",
    "ismethod",
    "isprime",
    "isrow",
    "isstudent",
    "kron",
    "lcm",
    "linspace",
    "logspace",
    "mat2str",
    "maxNumCompThreads",
    "mean",
    "median",
    "meshgrid",
    "mode",
    "namelengthmax",
    "nchoosek",
    "ndgrid",
    "null",
    "num2str",
    "orth",
    "polyfit",
    "polyval",
    "primes",
    "rank",
    "repmat",
    "roots",
    "rot90",
    "setdiff",
    "setxor",
    "std",
    "str2num",
    "strjoin",
    "strsplit",
    "trace",
    "trapz",
    "union",
    "unique",
    "usejava",
    "var",
];
const INLINE_METHODS: &[&str] = &[
    "argnames",
    "cat",
    "char",
    "disp",
    "display",
    "exist",
    "feval",
    "formula",
    "horzcat",
    "inline",
    "nargin",
    "nargout",
    "subsref",
    "vectorize",
    "vertcat",
];
const EXCEPTION_METHODS: &[&str] = &[
    "MException",
    "addCause",
    "eq",
    "getReport",
    "isequal",
    "ne",
    "rethrow",
    "throw",
    "throwAsCaller",
];
const MAP_METHODS: &[&str] = &[
    "Map",
    "addlistener",
    "delete",
    "eq",
    "findobj",
    "findprop",
    "ge",
    "gt",
    "isKey",
    "isvalid",
    "keys",
    "le",
    "length",
    "lt",
    "ne",
    "notify",
    "remove",
    "size",
    "values",
];
const WHAT_FIELDS: [&str; 10] = [
    "path", "m", "mlapp", "mat", "mex", "mdl", "slx", "p", "classes", "packages",
];

/// Per-engine state: the diary, the thread-count setting and the call sites
/// `dbstack` reports.
#[derive(Clone, Default)]
pub(super) struct Lg3State {
    diary_on: bool,
    diary_file: Option<PathBuf>,
    threads: Option<usize>,
    /// Byte offset of the statement running in the innermost frame.
    byte: usize,
    /// One entry per entered function frame: its `path::name` and the byte
    /// of the calling statement in the caller.
    calls: Vec<(String, usize)>,
    /// `builtin(name, ...)` in progress: user functions named `name` are
    /// skipped up to this evaluation depth (the dispatch of that call only).
    builtin: Option<(String, usize)>,
    /// Frame depths running `eval` text: its statements do not move the
    /// frame's line (dbstack reports the line of the eval call).
    evals: Vec<usize>,
}

/// Names `lg3_call` answers, so a bare `name` reaches it too.
pub(super) fn lg3_handles(name: &str) -> bool {
    matches!(
        name,
        "bsxfun"
            | "builtin"
            | "which"
            | "what"
            | "isobject"
            | "ismethod"
            | "iskeyword"
            | "genvarname"
            | "namelengthmax"
            | "clearvars"
            | "functions"
            | "localfunctions"
            | "dbstack"
            | "isequalwithequalnans"
            | "bitmax"
            | "inline"
            | "argnames"
            | "formula"
            | "mexext"
            | "maxNumCompThreads"
            | "usejava"
            | "isstudent"
            | "diary"
    )
}
/// The regex crate here lacks Unicode Perl classes: spell \d, \w, \s
/// (and their negations) as the ASCII classes MATLAB means.
fn lg3_regex(pattern: &str) -> Result<regex::Regex> {
    let mut translated = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            translated.push(c);
            continue;
        }
        match chars.next() {
            Some('d') => translated.push_str("[0-9]"),
            Some('D') => translated.push_str("[^0-9]"),
            Some('w') => translated.push_str("[0-9A-Za-z_]"),
            Some('W') => translated.push_str("[^0-9A-Za-z_]"),
            Some('s') => translated.push_str("[\\t\\n\\x0B\\f\\r ]"),
            Some('S') => translated.push_str("[^\\t\\n\\x0B\\f\\r ]"),
            Some(other) => {
                translated.push('\\');
                translated.push(other);
            }
            None => translated.push('\\'),
        }
    }
    regex::Regex::new(&translated).map_err(|e| e.to_string())
}
/// Names from a string or a cell, with the cell's shape.
type Lg3Names = (Vec<String>, Option<(usize, usize)>);

fn lg3_arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err(FEW.into())
    } else if args.len() > max {
        Err(MANY.into())
    } else {
        Ok(())
    }
}
fn lg3_bool(value: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(value))
}
fn lg3_number(value: f64) -> Value {
    Value::Matrix(Matrix::scalar(value))
}
fn lg3_is_inline(value: &Value) -> bool {
    matches!(value, Value::Container(a @ Array::Struct(_)) if lang_more::lng_class(a) == "inline")
}
fn lg3_path_text(path: &Path) -> String {
    path.display().to_string()
}
/// `*` matches any run of characters; the whole name must match.
fn lg3_wildcard(pattern: &str) -> Result<regex::Regex> {
    let body = pattern
        .split('*')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(".*");
    regex::Regex::new(&format!("^{body}$")).map_err(|e| e.to_string())
}
/// genvarname's conversion of one candidate to a valid name.
fn lg3_valid_name(candidate: &str) -> String {
    let valid = |s: &str| {
        s.len() <= 63
            && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !KEYWORDS.contains(&s)
    };
    if valid(candidate) {
        return candidate.into();
    }
    let mut name = String::new();
    let mut capitalize = false;
    for c in candidate.trim().chars() {
        if c.is_whitespace() {
            capitalize = true;
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            if capitalize {
                name.extend(c.to_uppercase());
            } else {
                name.push(c);
            }
        } else {
            for unit in c.to_string().encode_utf16() {
                name.push_str(&format!("0x{unit:02X}"));
            }
        }
        capitalize = false;
    }
    if KEYWORDS.contains(&name.as_str()) {
        let mut chars = name.chars();
        let first = chars.next().map(|c| c.to_ascii_uppercase()).unwrap_or('x');
        name = format!("x{first}{}", chars.as_str());
    } else if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        name.insert(0, 'x');
    }
    name.truncate(63);
    name
}
/// Open (creating) the diary file for appending.
fn lg3_diary_open(path: &Path) -> std::result::Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Could not open diary file '{}' ({e}).", lg3_path_text(path)))
}
/// Line (1-based) of `byte` in the file at `path`, as the parser saw it.
fn lg3_line(path: &Path, byte: usize) -> usize {
    let Ok(source) = std::fs::read(path) else {
        return 1;
    };
    let skip = if source.starts_with(&[0xef, 0xbb, 0xbf]) {
        3
    } else {
        0
    };
    let end = (skip + byte).min(source.len());
    1 + source[skip.min(end)..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

impl Engine {
    fn lg3_text(&self, value: &Value) -> Option<String> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
            Value::Container(Array::Character(c)) if c.is_text_row() || c.is_empty() => {
                c.to_utf8(&self.cancel).ok()
            }
            _ => None,
        }
    }
    fn lg3_string(&self, value: &Value) -> Result<String> {
        self.lg3_text(value).ok_or_else(|| STRING.into())
    }
    fn lg3_char(&self, text: &str) -> Result<Array> {
        Value::Text(text.into()).array(&self.cancel)
    }
    /// `rows x cols` cell of character rows.
    fn lg3_cell(&self, rows: usize, cols: usize, texts: &[String]) -> Result<Array> {
        let slots = texts
            .iter()
            .map(|t| self.lg3_char(t).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(CellArray::from_shared(rows, cols, slots, &self.cancel)?.into())
    }
    fn lg3_struct(&self, fields: Vec<(&str, Array)>) -> Result<Array> {
        let (names, slots): (Vec<_>, Vec<_>) = fields
            .into_iter()
            .map(|(n, a)| (n.to_owned(), Arc::new(a)))
            .unzip();
        Ok(StructArray::from_slots(1, 1, names, slots, &self.cancel)?.into())
    }
    fn lg3_handle(&mut self, text: &str) -> Result<FunctionHandle> {
        let Expr::Anonymous(function) = syntax::parse_expression(text)? else {
            return Err("Invalid anonymous function".into());
        };
        match self.anonymous_handle(&function, false)? {
            Value::Container(Array::Handle(handle)) => Ok(handle),
            _ => unreachable!(),
        }
    }
    /// `@name`: bound to the running file when `name` is one of its local
    /// functions, so `functions` can report the file.
    pub(super) fn lg3_named_handle(&self, name: &str) -> Result<Value> {
        let value = self.named_handle(name)?;
        Ok(match self.lg3_local_file(name) {
            Some(file) => Value::Container(FunctionHandle::local(name, &file).into()),
            None => value,
        })
    }
    fn lg3_folder(&self) -> Option<PathBuf> {
        self.current_folder().map(Path::to_path_buf)
    }
    fn lg3_resolve(&self, text: &str) -> Option<PathBuf> {
        let path = Path::new(text);
        if path.is_absolute() {
            Some(path.to_path_buf())
        } else {
            self.lg3_folder().map(|f| f.join(path))
        }
    }
    /// Installation-relative location reported for a builtin or toolbox file.
    fn lg3_install(&self, name: &str) -> String {
        let root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        lg3_path_text(&root.join("toolbox").join("matlab").join(name))
    }
    /// The .mr/.m file `name` in the function lookup folder.
    fn lg3_user_file(&self, name: &str) -> Option<PathBuf> {
        if !crate::files::valid_name(name) {
            return None;
        }
        let folder = self
            .function_directory()
            .map(Path::to_path_buf)
            .or_else(|| self.file_directory.clone())?;
        ["mr", "m"]
            .iter()
            .map(|e| folder.join(format!("{name}.{e}")))
            .find(|p| p.is_file())
    }
    /// `which name`: what the name resolves to, or None.
    fn lg3_which(&self, name: &str) -> Option<String> {
        if self.contains_array(name) {
            return Some("variable".into());
        }
        if let Some(path) = self.lg3_user_file(name) {
            return Some(lg3_path_text(&path));
        }
        if KEYWORDS.contains(&name) {
            return Some(format!("built-in ({})", self.lg3_install(name)));
        }
        if M_FILES.contains(&name) {
            return Some(self.lg3_install(&format!("{name}.m")));
        }
        if crate::help::is_supported(name) {
            return Some(format!("built-in ({})", self.lg3_install(name)));
        }
        None
    }

    /// Single-output dispatch; `None` when `name` is not ours.
    pub(super) fn lg3_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let inline = args.first().is_some_and(lg3_is_inline);
        let value = match name {
            "bsxfun" => self.lg3_bsxfun(args, out)?,
            "builtin" => self.lg3_builtin(args, 1, None, out)?.remove(0),
            "which" => self.lg3_which_call(args)?,
            "what" => self.lg3_what(args)?,
            "isobject" => {
                lg3_arity(args, 1, 1)?;
                lg3_bool(
                    matches!(&args[0], Value::Container(a @ Array::Struct(_)) if lang_more::lng_class(a) != "struct"),
                )
            }
            "ismethod" => self.lg3_ismethod(args)?,
            "iskeyword" => self.lg3_iskeyword(args)?,
            "genvarname" => self.lg3_genvarname(args)?,
            "namelengthmax" => {
                lg3_arity(args, 0, 0)?;
                lg3_number(63.)
            }
            "clearvars" => {
                self.lg3_clearvars(args)?;
                Value::Void
            }
            "functions" => self.lg3_functions(args)?,
            "localfunctions" => self.lg3_localfunctions(args)?,
            "dbstack" => self.lg3_dbstack(args, 1)?.remove(0),
            "isequalwithequalnans" => self.call("isequaln", args.to_vec(), out)?,
            "bitmax" => {
                lg3_arity(args, 0, 0)?;
                return Err("BITMAX has been removed. Use FLINTMAX instead.".into());
            }
            "inline" => self.lg3_inline(args)?,
            "argnames" | "formula" => {
                lg3_arity(args, 1, 1)?;
                let (expr, names) = self.lg3_inline_parts(&args[0])?;
                if name == "formula" {
                    Value::Text(expr)
                } else {
                    let n = names.len();
                    Value::Container(self.lg3_cell(n, usize::from(n > 0), &names)?)
                }
            }
            "char" | "vectorize" | "nargin" | "feval" | "disp" | "display" if inline => {
                let (expr, names) = self.lg3_inline_parts(&args[0])?;
                match name {
                    "char" => {
                        lg3_arity(args, 1, 1)?;
                        Value::Text(expr)
                    }
                    "vectorize" => {
                        lg3_arity(args, 1, 1)?;
                        let text = self.call("vectorize", vec![Value::Text(expr)], out)?;
                        let text = self.lg3_string(&text)?;
                        Value::Container(self.lg3_inline_object(&text, &names)?)
                    }
                    "nargin" => {
                        lg3_arity(args, 1, 1)?;
                        lg3_number(names.len() as f64)
                    }
                    "feval" => self.lg3_inline_eval(&args[0], args[1..].to_vec(), out)?,
                    _ => {
                        let label = self
                            .functions
                            .lng
                            .call_names
                            .first()
                            .filter(|n| !n.is_empty())
                            .cloned()
                            .unwrap_or_else(|| "ans".into());
                        let text = self.lg3_inline_text(&label, &expr, &names, false);
                        self.emit(out, text)?;
                        Value::Void
                    }
                }
            }
            "mexext" => self.lg3_mexext(args)?,
            "maxNumCompThreads" => self.lg3_threads(args, out)?,
            "usejava" => {
                lg3_arity(args, 1, 1)?;
                // No Java virtual machine, AWT, Swing or Java desktop.
                lg3_bool(false)
            }
            "isstudent" => {
                lg3_arity(args, 0, 0)?;
                lg3_bool(false)
            }
            "diary" => {
                self.lg3_diary(args)?;
                Value::Void
            }
            "get"
                if args.len() == 2
                    && matches!(&args[0], Value::Matrix(m) if m.len() == 1 && m.data()[0] == 0.)
                    && self.lg3_text(&args[1]).is_some_and(|p| {
                        p.eq_ignore_ascii_case("Diary") || p.eq_ignore_ascii_case("DiaryFile")
                    }) =>
            {
                let property = self.lg3_text(&args[1]).unwrap_or_default();
                if property.eq_ignore_ascii_case("Diary") {
                    Value::Text(if self.lg3.diary_on { "on" } else { "off" }.into())
                } else {
                    Value::Text(lg3_path_text(&self.lg3_diary_file()))
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// `[a, b] = builtin(...)`, `[st, k] = dbstack` and `[p, n, e] = fileparts(f)`.
    pub(super) fn lg3_outputs(
        &mut self,
        expression: &Expr,
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let (name, args) = match expression {
            Expr::Call(name, args) => (name.as_str(), args.as_slice()),
            Expr::Name(name) => (name.as_str(), &[][..]),
            _ => return Ok(None),
        };
        if targets.len() < 2
            || !matches!(name, "builtin" | "dbstack" | "fileparts")
            || self.contains_array(name)
            || self.resolve_function(name)?.is_some()
        {
            return Ok(None);
        }
        let values = self.function_arguments(args, None, out)?;
        let outputs = match name {
            "builtin" => self.lg3_builtin(&values, targets.len(), Some(targets), out)?,
            "dbstack" => self.lg3_dbstack(&values, targets.len())?,
            _ => self.lg3_fileparts(&values, targets.len())?,
        };
        self.charge_function_outputs(&outputs)?;
        Ok(Some(outputs))
    }

    /// Statements without outputs that print: `which name`, `what`, `dbstack`.
    pub(super) fn lg3_statement(
        &mut self,
        e: &syntax::Evaluation,
        out: &mut Execution,
    ) -> Result<bool> {
        if !e.targets.is_empty()
            || e.path_target.is_some()
            || e.indexed_target.is_some()
            || e.list_target.is_some()
        {
            return Ok(false);
        }
        let (name, args) = match &e.expr {
            Expr::Name(n) => (n.as_str(), &[][..]),
            Expr::Call(n, a) => (n.as_str(), a.as_slice()),
            _ => return Ok(false),
        };
        if !matches!(name, "which" | "what" | "dbstack")
            || self.contains_array(name)
            || self.resolve_function(name)?.is_some()
        {
            return Ok(false);
        }
        #[cfg(target_arch = "wasm32")]
        if super::browser::file_command(name, None) {
            return Err(super::browser::NO_FILES.into());
        }
        let values = self.function_arguments(args, None, out)?;
        let text = match name {
            "which" => {
                lg3_arity(&values, 1, 2)?;
                let target = self.lg3_string(&values[0])?;
                match self.lg3_which(&target) {
                    Some(v) if v == "variable" => format!("{target} is a variable."),
                    Some(path) => path,
                    None => format!("'{target}' not found."),
                }
            }
            "what" => {
                let Value::Container(Array::Struct(s)) = self.lg3_what(&values)? else {
                    unreachable!()
                };
                if s.is_empty() {
                    return Ok(true);
                }
                let mut text = String::new();
                let path = self.lg3_field_strings(&s, "path")?.concat();
                for (field, title) in [
                    ("m", "MATLAB Code files"),
                    ("mat", "MAT-files"),
                    ("mex", "MEX-files"),
                    ("p", "P-files"),
                    ("classes", "Classes"),
                    ("packages", "Packages"),
                ] {
                    let names = self.lg3_field_strings(&s, field)?;
                    if names.is_empty() {
                        continue;
                    }
                    let names: Vec<_> = names
                        .iter()
                        .map(|n| match field {
                            "m" | "mat" | "p" => n.rsplit_once('.').map_or(n.as_str(), |p| p.0),
                            _ => n,
                        })
                        .collect();
                    text.push_str(&format!(
                        "\n{title} in folder {path}\n\n{}  \n\n",
                        names.join("  ")
                    ));
                }
                if text.is_empty() {
                    return Ok(true);
                }
                text
            }
            _ => {
                let Value::Container(Array::Struct(s)) = self.lg3_dbstack(&values, 1)?.remove(0)
                else {
                    unreachable!()
                };
                let names = self.lg3_field_strings(&s, "name")?;
                if names.is_empty() {
                    return Ok(true);
                }
                let line = s.field_index("line").ok_or("Missing line field")?;
                let mut lines = Vec::new();
                for (i, n) in names.iter().enumerate() {
                    let Array::Numeric(l) = s.get(line, i)?.as_ref() else {
                        unreachable!()
                    };
                    let mark = if i == 0 { ">" } else { " " };
                    lines.push(format!("{mark} In {n} (line {})", l.data()[0]));
                }
                lines.join("\n")
            }
        };
        self.emit(out, text)?;
        Ok(true)
    }
    fn lg3_field_strings(&self, s: &StructArray, field: &str) -> Result<Vec<String>> {
        let index = s.field_index(field).ok_or("Missing field")?;
        let mut names = Vec::new();
        for element in 0..s.len() {
            match s.get(index, element)?.as_ref() {
                Array::Cell(c) => {
                    for i in 0..c.len() {
                        let v = Value::from(c.get_linear(i)?.as_ref().clone());
                        names.push(self.lg3_string(&v)?);
                    }
                }
                a => names.push(self.lg3_string(&Value::from(a.clone()))?),
            }
        }
        Ok(names)
    }

    fn lg3_bsxfun(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        lg3_arity(args, 3, 3)?;
        let Value::Container(Array::Handle(function)) = &args[0] else {
            return Err("First Argument must be a function handle.".into());
        };
        let (ra, ca) = args[1].shape(&self.cancel)?;
        let (rb, cb) = args[2].shape(&self.cancel)?;
        let dim = |a: usize, b: usize| {
            if a == b || b == 1 {
                Ok(a)
            } else if a == 1 {
                Ok(b)
            } else {
                Err("Non-singleton dimensions of the two input arrays must match each other.")
            }
        };
        let rows = dim(ra, rb)?;
        let cols = dim(ca, cb)?;
        if rows.saturating_mul(cols) > crate::script_containers::MAX_CELL_SLOTS * 4096 {
            return Err("bsxfun result exceeds the element limit".into());
        }
        // Builtins work on whole arrays: expand the singleton dimensions once.
        // User functions keep the documented column contract below.
        // Only the elementwise builtins the reference lists for bsxfun take whole
        // expanded arrays; `@mtimes` and the rest follow the column contract.
        const ELEMENTWISE: &[&str] = &[
            "plus", "minus", "times", "rdivide", "ldivide", "power", "max", "min", "rem", "mod",
            "atan2", "atan2d", "hypot", "eq", "ne", "lt", "le", "gt", "ge", "and", "or", "xor",
            "bitand", "bitor", "bitxor",
        ];
        let builtin = match function.name() {
            Some(name) => {
                function.file().is_none()
                    && ELEMENTWISE.contains(&name)
                    && self.resolve_function(name)?.is_none()
            }
            None => false,
        };
        if builtin || rows == 0 || cols == 0 {
            let mut expanded = Vec::with_capacity(2);
            for (value, r, c) in [(&args[1], ra, ca), (&args[2], rb, cb)] {
                let (fr, fc) = (
                    if r == rows { 1 } else { rows },
                    if c == cols { 1 } else { cols },
                );
                expanded.push(if fr == 1 && fc == 1 {
                    value.clone()
                } else {
                    let repeat = vec![value.clone(), lg3_number(fr as f64), lg3_number(fc as f64)];
                    // The builtin repmat, never a user overload of it.
                    let depth = self.evaluation_depth;
                    let saved = self.lg3.builtin.replace(("repmat".to_owned(), depth));
                    let result = self.call("repmat", repeat, out);
                    self.lg3.builtin = saved;
                    result?
                });
            }
            let mut results = self.call_handle(function, expanded, 1, None, out)?;
            if results.is_empty() {
                return Err("Too many output arguments.".into());
            }
            return Ok(results.remove(0));
        }
        // Other functions see one column pair at a time (a column and a
        // scalar when one input has a single row), as the reference documents.
        let column = self.lg3_handle("@(a,k) a(:,k)")?;
        let mut parts = Vec::with_capacity(cols);
        for j in 0..cols {
            self.check_cancel()?;
            let mut inputs = Vec::with_capacity(2);
            for (value, c) in [(&args[1], ca), (&args[2], cb)] {
                let k = lg3_number((if c == 1 { 0 } else { j } + 1) as f64);
                let mut part = self.call_handle(&column, vec![value.clone(), k], 1, None, out)?;
                inputs.push(part.remove(0));
            }
            let mut result = self.call_handle(function, inputs, 1, None, out)?;
            if result.is_empty() {
                return Err("Too many output arguments.".into());
            }
            let result = result.remove(0);
            if result.shape(&self.cancel)? != (rows, 1) {
                return Err("Invalid output dimensions.".into());
            }
            parts.push(result);
        }
        self.concatenate_values(vec![parts])
    }

    fn lg3_builtin(
        &mut self,
        args: &[Value],
        count: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let Some((name, rest)) = args.split_first() else {
            return Err(FEW.into());
        };
        let name = self.lg3_string(name)?;
        if !crate::files::valid_name(&name)
            || M_FILES.contains(&name.as_str())
            || !crate::help::is_supported(&name)
        {
            return Err(format!("Cannot find builtin function '{name}'"));
        }
        // A user function or local function of that name (often the caller
        // itself, overloading it) is skipped while this call is dispatched:
        // at this depth, or two levels deeper inside the multi-output wrapper.
        // Callbacks the builtin runs are deeper and resolve normally.
        let depth = self.evaluation_depth + if count <= 1 { 0 } else { 2 };
        let saved = self.lg3.builtin.replace((name.clone(), depth));
        let result = if count <= 1 {
            self.call(&name, rest.to_vec(), out)
                .map(|value| vec![value])
        } else {
            // Several outputs: the ordinary multi-output dispatch of `name`.
            self.lg3_handle(&format!("@(varargin) {name}(varargin{{:}})"))
                .and_then(|handle| self.call_handle(&handle, rest.to_vec(), count, targets, out))
        };
        self.lg3.builtin = saved;
        let values = result?;
        if values.len() < count {
            return Err("Too many output arguments.".into());
        }
        Ok(values)
    }
    /// True while `builtin(name, ...)` dispatches `name`: user functions of
    /// that name are not candidates.
    pub(super) fn lg3_builtin_only(&self, name: &str) -> bool {
        self.lg3
            .builtin
            .as_ref()
            .is_some_and(|(n, depth)| n == name && self.evaluation_depth <= *depth)
    }

    /// fileparts as in the reference: retain drive-relative prefixes on Windows; the name
    /// follows the last separator and the extension starts at its last dot.
    fn lg3_fileparts(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if count > 3 {
            return Err("Too many output arguments.".into());
        }
        let Some(mut folder) = self.script_path_call("fileparts", args)? else {
            unreachable!()
        };
        let text = self.lg3_string(&args[0])?;
        let cut = if cfg!(windows) {
            text.rfind(['/', '\\']).or_else(|| text.rfind(':'))
        } else {
            text.rfind('/')
        };
        if cfg!(windows)
            && let Some(i) = cut
            && text.as_bytes()[i] == b':'
        {
            folder = Value::Text(text[..=i].into());
        }
        let name = match cut {
            // `\\server`: the whole text is the folder.
            Some(1) if cfg!(windows) && text.starts_with(['/', '\\']) => "",
            Some(i) => &text[i + 1..],
            None => &text,
        };
        let (name, ext) = name.split_at(name.rfind('.').unwrap_or(name.len()));
        let parts = [folder, Value::Text(name.into()), Value::Text(ext.into())];
        Ok(parts.into_iter().take(count).collect())
    }

    fn lg3_which_call(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 1, 2)?;
        let name = self.lg3_string(&args[0])?;
        let all = match args.get(1) {
            Some(v) => self.lg3_string(v)? == "-all",
            None => false,
        };
        let found = self.lg3_which(&name);
        Ok(Value::Container(if all {
            let list: Vec<_> = found.into_iter().collect();
            self.lg3_cell(list.len(), usize::from(!list.is_empty()), &list)?
        } else {
            match found {
                Some(text) => self.lg3_char(&text)?,
                None => self.lg3_char("")?,
            }
        }))
    }

    fn lg3_what(&mut self, args: &[Value]) -> Result<Value> {
        // A `what` statement arrives here before the engine's browser file guard.
        #[cfg(target_arch = "wasm32")]
        if super::browser::file_command("what", Some(args)) {
            return Err(super::browser::NO_FILES.into());
        }
        let empty = || -> Result<Value> {
            let names = WHAT_FIELDS.iter().map(|s| s.to_string()).collect();
            Ok(Value::Container(
                StructArray::filled_empty(0, 1, names, &self.cancel)?.into(),
            ))
        };
        let folder = match args {
            [] => self.lg3_folder().ok_or("No current folder is available")?,
            [dir] => {
                let text = self.lg3_string(dir)?;
                match self.lg3_resolve(&text) {
                    Some(path) if path.is_dir() => path,
                    _ => return empty(),
                }
            }
            _ => return empty(),
        };
        let mut lists: [Vec<String>; 9] = Default::default();
        let mex = format!(".{}", self.lg3_mex_extension());
        let entries = std::fs::read_dir(&folder).map_err(|e| format!("what: {e}"))?;
        for (count, entry) in entries.enumerate() {
            self.check_cancel()?;
            if count >= LIMIT {
                return Err(format!("what lists at most {LIMIT} entries"));
            }
            let entry = entry.map_err(|e| format!("what: {e}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let directory = entry.file_type().is_ok_and(|t| t.is_dir());
            let slot = if directory {
                if let Some(class) = name.strip_prefix('@') {
                    lists[7].push(class.into());
                } else if let Some(package) = name.strip_prefix('+') {
                    lists[8].push(package.into());
                }
                continue;
            } else {
                match Path::new(&name).extension().and_then(|e| e.to_str()) {
                    Some("m") => 0,
                    Some("mlapp") => 1,
                    Some("mat") => 2,
                    _ if name.ends_with(&mex) => 3,
                    Some("mdl") => 4,
                    Some("slx") => 5,
                    Some("p") => 6,
                    _ => continue,
                }
            };
            lists[slot].push(name);
        }
        let mut fields = vec![("path", self.lg3_char(&lg3_path_text(&folder))?)];
        for (field, mut list) in WHAT_FIELDS[1..].iter().zip(lists) {
            list.sort();
            fields.push((field, self.lg3_cell(list.len(), 1, &list)?));
        }
        Ok(Value::Container(self.lg3_struct(fields)?))
    }

    fn lg3_ismethod(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 2, 2)?;
        let Some(method) = self.lg3_text(&args[1]) else {
            return Err("Requires character array or cell array of strings as inputs.".into());
        };
        let class = match self.lg3_text(&args[0]) {
            Some(class) => class,
            None => lang_more::lng_class(&args[0].clone().array(&self.cancel)?).to_owned(),
        };
        let found = match class.as_str() {
            "inline" => INLINE_METHODS.contains(&method.as_str()),
            "MException" => EXCEPTION_METHODS.contains(&method.as_str()),
            "containers.Map" => MAP_METHODS.contains(&method.as_str()),
            "double" | "single" | "logical" | "char" | "int8" | "uint8" | "int16" | "uint16"
            | "int32" | "uint32" | "int64" | "uint64" => {
                crate::help::is_supported(&method) && !M_FILES.contains(&method.as_str())
            }
            _ => false,
        };
        Ok(lg3_bool(found))
    }

    fn lg3_iskeyword(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 0, 1)?;
        let Some(value) = args.first() else {
            let names: Vec<_> = KEYWORDS.iter().map(|s| s.to_string()).collect();
            return Ok(Value::Container(self.lg3_cell(20, 1, &names)?));
        };
        let text = match value {
            Value::Container(Array::Cell(c)) => {
                if c.len() != 1 {
                    return Err(
                        "Inputs must be the same size or either one can be a scalar.".into(),
                    );
                }
                self.lg3_text(&Value::from(c.get_linear(0)?.as_ref().clone()))
            }
            v => self.lg3_text(v),
        };
        Ok(lg3_bool(
            text.is_some_and(|t| KEYWORDS.contains(&t.as_str())),
        ))
    }

    fn lg3_genvarname(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 1, 2)?;
        const BAD: &str =
            "First input argument, VARNAME, must be either a string or a cell array of strings.";
        let strings = |engine: &Self, value: &Value| -> Option<Lg3Names> {
            if let Some(text) = engine.lg3_text(value) {
                return Some((vec![text], None));
            }
            let Value::Container(Array::Cell(c)) = value else {
                return None;
            };
            let mut list = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                list.push(engine.lg3_text(&Value::from(c.get_linear(i).ok()?.as_ref().clone()))?);
            }
            Some((list, Some((c.rows(), c.cols()))))
        };
        let (candidates, shape) = strings(self, &args[0]).ok_or(BAD)?;
        let protected = match args.get(1) {
            Some(v) => {
                strings(self, v)
                    .ok_or("Second input argument, PROTECTED, must be either a string or a cell array of strings.")?
                    .0
            }
            None => Vec::new(),
        };
        if candidates.len() > LIMIT || protected.len() > LIMIT {
            return Err(format!("genvarname accepts at most {LIMIT} names"));
        }
        let mut taken: std::collections::HashSet<String> = protected.into_iter().collect();
        // Next suffix to try per base name: every earlier suffix is taken and
        // stays taken, so the search for a repeated name resumes there.
        let mut next = std::collections::HashMap::<String, usize>::new();
        let mut names = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            self.check_cancel()?;
            let base = lg3_valid_name(candidate);
            let mut n = next.get(&base).copied().unwrap_or(0);
            let mut name = base.clone();
            loop {
                if n > 0 {
                    let suffix = n.to_string();
                    let mut stem = base.clone();
                    stem.truncate(63 - suffix.len());
                    name = stem + &suffix;
                }
                if !taken.contains(&name) {
                    break;
                }
                n += 1;
                if n % 4096 == 0 {
                    self.check_cancel()?;
                }
            }
            next.insert(base, n + 1);
            taken.insert(name.clone());
            names.push(name);
        }
        Ok(match shape {
            None => Value::Text(names.remove(0)),
            Some((rows, cols)) => Value::Container(self.lg3_cell(rows, cols, &names)?),
        })
    }

    fn lg3_clearvars(&mut self, args: &[Value]) -> Result<()> {
        let mut words = Vec::with_capacity(args.len());
        for (i, value) in args.iter().enumerate() {
            let Some(text) = self.lg3_text(value) else {
                let class = lang_more::lng_class(&value.clone().array(&self.cancel)?);
                return Err(format!(
                    "Expected input number {} to be one of these types:\n\nchar\n\nInstead its type was {class}.",
                    i + 1
                ));
            };
            words.push(text);
        }
        let (mut clear, mut keep) = (Vec::new(), Vec::new());
        let (mut except, mut regexp, mut global) = (false, false, false);
        for word in &words {
            match word.as_str() {
                "-except" => {
                    except = true;
                    regexp = false;
                }
                "-regexp" => regexp = true,
                "-global" => global = true,
                pattern => {
                    let compiled = if regexp {
                        lg3_regex(pattern)?
                    } else {
                        lg3_wildcard(pattern)?
                    };
                    if except {
                        keep.push(compiled);
                    } else {
                        clear.push(compiled);
                    }
                }
            }
        }
        // -global selects from the global store, so globals declared only in
        // functions (or no longer linked here) are cleared too.
        let candidates: Vec<String> = if global {
            self.lg3_global_names()
        } else {
            self.variables
                .keys()
                .chain(self.logical_variables.keys())
                .chain(self.complex_variables.keys())
                .chain(self.container_variables.keys())
                .cloned()
                .collect()
        };
        let names: Vec<String> = candidates
            .into_iter()
            .filter(|n| clear.is_empty() || clear.iter().any(|r| r.is_match(n)))
            .filter(|n| !keep.iter().any(|r| r.is_match(n)))
            .collect();
        self.lg3_clear_variables(&names, global)
    }

    fn lg3_functions(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 1, 1)?;
        let Value::Container(Array::Handle(handle)) = &args[0] else {
            let class = lang_more::lng_class(&args[0].clone().array(&self.cancel)?);
            return Err(format!(
                "Undefined function 'functions' for input arguments of type '{class}'."
            ));
        };
        let Some((function, captured)) = handle.anonymous_parts() else {
            let name = handle.func2str().to_owned();
            if let Some(path) = handle.file() {
                // A local function: its file and the chain of parents.
                let parent = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let fields = vec![
                    ("function", self.lg3_char(&name)?),
                    ("type", self.lg3_char("scopedfunction")?),
                    ("file", self.lg3_char(&lg3_path_text(path))?),
                    ("parentage", self.lg3_cell(1, 2, &[name.clone(), parent])?),
                ];
                return Ok(Value::Container(self.lg3_struct(fields)?));
            }
            let file = match self.lg3_user_file(&name) {
                Some(path) => lg3_path_text(&path),
                None if M_FILES.contains(&name.as_str()) => self.lg3_install(&format!("{name}.m")),
                None => String::new(),
            };
            let fields = vec![
                ("function", self.lg3_char(&name)?),
                ("type", self.lg3_char("simple")?),
                ("file", self.lg3_char(&file)?),
            ];
            return Ok(Value::Container(self.lg3_struct(fields)?));
        };
        let (names, slots): (Vec<_>, Vec<_>) = captured
            .iter()
            .map(|(n, a)| (n.clone(), Arc::new(a.clone())))
            .unzip();
        let workspace = StructArray::from_slots(1, 1, names, slots, &self.cancel)?;
        let workspace =
            CellArray::from_shared(1, 1, vec![Arc::new(workspace.into())], &self.cancel)?;
        let fields = vec![
            ("function", self.lg3_char(&function.text)?),
            ("type", self.lg3_char("anonymous")?),
            ("file", self.lg3_char("")?),
            ("workspace", workspace.into()),
            ("within_file_path", self.lg3_char("__base_function")?),
        ];
        Ok(Value::Container(self.lg3_struct(fields)?))
    }

    fn lg3_localfunctions(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 0, 0)?;
        let names = self.functions.lg3_local_names();
        let file = self
            .functions
            .active_path()
            .or(self.document_path.as_deref());
        let slots: Vec<_> = names
            .iter()
            .map(|n| {
                let handle = match file {
                    Some(path) => FunctionHandle::local(n, path),
                    None => FunctionHandle::named(n),
                };
                Arc::new(Array::from(handle))
            })
            .collect();
        let rows = slots.len();
        Ok(Value::Container(
            CellArray::from_shared(rows, usize::from(rows > 0), slots, &self.cancel)?.into(),
        ))
    }

    /// The running function frames, innermost first: (path, name, line).
    fn lg3_frames(&self) -> Vec<(String, String, usize)> {
        let calls = &self.lg3.calls;
        let mut frames = Vec::new();
        for (depth, (function, _)) in calls.iter().enumerate().rev() {
            let (path, name) = function.rsplit_once("::").unwrap_or(("", function));
            if name.starts_with('<') || name.is_empty() {
                continue;
            }
            let byte = calls.get(depth + 1).map_or(self.lg3.byte, |c| c.1);
            let line = if path.is_empty() {
                1
            } else {
                lg3_line(Path::new(path), byte)
            };
            frames.push((path.to_owned(), name.to_owned(), line));
        }
        frames
    }

    fn lg3_dbstack(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if count > 2 {
            return Err("Too many output arguments.".into());
        }
        const LEGAL: &str = "At present, the only legal arguments to dbstack are the string '-completenames' and non-negative integers.";
        let (mut complete, mut omit) = (false, 0usize);
        for value in args {
            if let Some(text) = self.lg3_text(value) {
                if text != "-completenames" {
                    return Err(LEGAL.into());
                }
                complete = true;
            } else {
                let n = value.clone().matrix(&self.cancel).map_err(|_| LEGAL)?;
                if n.len() != 1
                    || !(0. ..=f64::MAX).contains(&n.data()[0])
                    || n.data()[0].fract() != 0.
                {
                    return Err(LEGAL.into());
                }
                omit = n.data()[0].min(1e6) as usize;
            }
        }
        let frames: Vec<_> = self.lg3_frames().into_iter().skip(omit).collect();
        let mut slots = Vec::with_capacity(frames.len() * 3);
        for (path, name, line) in &frames {
            let file = if complete {
                path.clone()
            } else {
                Path::new(path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            slots.push(Arc::new(self.lg3_char(&file)?));
            slots.push(Arc::new(self.lg3_char(name)?));
            slots.push(Arc::new(Matrix::scalar(*line as f64).into()));
        }
        let names = vec!["file".into(), "name".into(), "line".into()];
        let stack = if frames.is_empty() {
            StructArray::filled_empty(0, 1, names, &self.cancel)?
        } else {
            StructArray::from_slots(frames.len(), 1, names, slots, &self.cancel)?
        };
        let mut values = vec![Value::Container(stack.into())];
        if count == 2 {
            values.push(lg3_number(1.));
        }
        Ok(values)
    }

    // inline objects: a tagged struct holding the expression and argument names.
    fn lg3_inline_object(&self, expr: &str, names: &[String]) -> Result<Array> {
        let n = names.len();
        let fields = vec![
            ("expr", self.lg3_char(expr)?),
            ("args", self.lg3_cell(n, usize::from(n > 0), names)?),
            (lang_more::TAG, Matrix::scalar(3.).into()),
        ];
        self.lg3_struct(fields)
    }
    fn lg3_inline_parts(&self, value: &Value) -> Result<(String, Vec<String>)> {
        let Value::Container(Array::Struct(s)) = value else {
            return Err("Input must be an inline object.".into());
        };
        if !lg3_is_inline(value) {
            return Err("Input must be an inline object.".into());
        }
        let expr = self.lg3_field_strings(s, "expr")?.concat();
        let names = self.lg3_field_strings(s, "args")?;
        Ok((expr, names))
    }
    fn lg3_inline(&mut self, args: &[Value]) -> Result<Value> {
        let Some((expr, rest)) = args.split_first() else {
            return Ok(Value::Container(self.lg3_inline_object("", &[])?));
        };
        let expr = self.lg3_text(expr).ok_or("Input must be a string.")?;
        if expr.len() > LIMIT {
            return Err(format!(
                "inline expressions are limited to {LIMIT} characters"
            ));
        }
        let names = match rest {
            [] => {
                let mut found = std::collections::BTreeSet::new();
                let bytes = expr.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i].is_ascii_alphabetic()
                        && (i == 0
                            || !(bytes[i - 1].is_ascii_alphanumeric()
                                || bytes[i - 1] == b'_'
                                || bytes[i - 1] == b'.'))
                    {
                        let start = i;
                        while i < bytes.len()
                            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                        {
                            i += 1;
                        }
                        let word = &expr[start..i];
                        let call = expr[i..].trim_start().starts_with('(');
                        if !call
                            && !matches!(
                                word,
                                "i" | "j" | "pi" | "Inf" | "inf" | "NaN" | "nan" | "eps"
                            )
                        {
                            found.insert(word.to_owned());
                        }
                    } else {
                        i += 1;
                    }
                }
                if found.is_empty() {
                    vec!["x".to_owned()]
                } else {
                    found.into_iter().collect()
                }
            }
            [n] if !n.is_character() => {
                let n = n.clone().matrix(&self.cancel)?;
                let count = n.data().first().copied().unwrap_or(0.);
                if n.len() != 1 || !(0. ..=1000.).contains(&count) || count.fract() != 0. {
                    return Err("Number of parameters must be a non-negative integer.".into());
                }
                std::iter::once("x".to_owned())
                    .chain((1..=count as usize).map(|k| format!("P{k}")))
                    .collect()
            }
            names => names
                .iter()
                .map(|v| {
                    self.lg3_text(v)
                        .ok_or_else(|| "Input must be a string.".into())
                })
                .collect::<Result<Vec<_>>>()?,
        };
        if names.len() > 255 || names.iter().any(|n| !crate::files::valid_name(n)) {
            return Err("Argument names must be valid variable names.".into());
        }
        Ok(Value::Container(self.lg3_inline_object(&expr, &names)?))
    }
    fn lg3_inline_text(&self, label: &str, expr: &str, names: &[String], echo: bool) -> String {
        let body = if expr.is_empty() && names.is_empty() {
            "     Inline function (empty)".to_owned()
        } else {
            format!(
                "     Inline function:\n     {label}({}) = {expr}",
                names.join(",")
            )
        };
        match (echo, self.display.compact) {
            (false, _) => body,
            (true, true) => format!("{label} =\n{body}"),
            (true, false) => format!("\n{label} =\n\n{body}\n"),
        }
    }
    /// Echo text of an inline object, for the object display of lang_more.
    pub(super) fn lg3_inline_display(&self, label: Option<&str>, a: &Array) -> Result<String> {
        let (expr, names) = self.lg3_inline_parts(&Value::Container(a.clone()))?;
        Ok(match label {
            Some(label) => self.lg3_inline_text(label, &expr, &names, true),
            None => self.lg3_inline_text("f", &expr, &names, false),
        })
    }
    fn lg3_inline_eval(
        &mut self,
        object: &Value,
        args: Vec<Value>,
        out: &mut Execution,
    ) -> Result<Value> {
        let (expr, names) = self.lg3_inline_parts(object)?;
        if args.len() > names.len() {
            return Err("Too many inputs to inline function.".into());
        }
        if args.len() < names.len() {
            return Err("Not enough inputs to inline function.".into());
        }
        let wrap = |message: String| format!("Error in inline expression ==> {expr}\n {message}");
        let handle = self
            .lg3_handle(&format!("@({}) {expr}", names.join(",")))
            .map_err(wrap)?;
        let mut results = self
            .call_handle(&handle, args, 1, None, out)
            .map_err(wrap)?;
        if results.is_empty() {
            return Err(wrap("Too many output arguments.".into()));
        }
        Ok(results.remove(0))
    }
    /// `feval(f, ...)` as a statement when `f` is an inline object.
    pub(super) fn lg3_feval(
        &mut self,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        match args.split_first() {
            Some((object, rest)) if lg3_is_inline(object) => {
                self.lg3_inline_eval(object, rest.to_vec(), out).map(Some)
            }
            _ => Ok(None),
        }
    }
    /// `f(args)` where `f` holds an inline object; None for other values.
    pub(super) fn lg3_object_index(
        &mut self,
        a: &Array,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !matches!(a, Array::Struct(_)) || lang_more::lng_class(a) != "inline" {
            return Ok(None);
        }
        let object = Value::Container(a.clone());
        let values = self.function_arguments(args, None, out)?;
        self.lg3_inline_eval(&object, values, out).map(Some)
    }

    fn lg3_mex_extension(&self) -> &'static str {
        if cfg!(windows) {
            "mexw64"
        } else if cfg!(target_os = "macos") {
            "mexmaci64"
        } else {
            "mexa64"
        }
    }
    fn lg3_mexext(&mut self, args: &[Value]) -> Result<Value> {
        lg3_arity(args, 0, 1)?;
        let Some(option) = args.first() else {
            return Ok(Value::Text(self.lg3_mex_extension().into()));
        };
        if self.lg3_text(option).as_deref() != Some("all") {
            return Err("Input must be 'all'.".into());
        }
        let table = [
            ("glnxa64", "mexa64"),
            ("maci64", "mexmaci64"),
            ("win32", "mexw32"),
            ("win64", "mexw64"),
        ];
        let mut slots = Vec::with_capacity(8);
        for (arch, ext) in table {
            slots.push(Arc::new(self.lg3_char(arch)?));
            slots.push(Arc::new(self.lg3_char(ext)?));
        }
        let names = vec!["arch".into(), "ext".into()];
        Ok(Value::Container(
            StructArray::from_slots(1, 4, names, slots, &self.cancel)?.into(),
        ))
    }

    fn lg3_threads(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        lg3_arity(args, 0, 1)?;
        const BAD: &str = "Input to maxNumCompThreads must be a positive numeric integer-valued scalar or 'automatic'.";
        let automatic = std::thread::available_parallelism().map_or(1, |n| n.get());
        let current = self.lg3.threads.unwrap_or(automatic);
        let update = match args.first() {
            None => None,
            Some(v) if v.is_character() => {
                if !self
                    .lg3_text(v)
                    .is_some_and(|t| t.eq_ignore_ascii_case("automatic"))
                {
                    return Err(BAD.into());
                }
                Some(None)
            }
            Some(v) => {
                let n = v.clone().matrix(&self.cancel).map_err(|_| BAD)?;
                let count = n.data().first().copied().unwrap_or(0.);
                if n.len() != 1 || !(1. ..=1e9).contains(&count) || count.fract() != 0. {
                    return Err(BAD.into());
                }
                Some(Some(count as usize))
            }
        };
        self.lang_utils_call(
            "warning",
            &[
                Value::Text("MATLAB:maxNumCompThreads:Deprecated".into()),
                Value::Text("maxNumCompThreads will be removed in a future release. Please remove any instances of this\nfunction from your code.".into()),
            ],
            out,
        )?;
        if let Some(setting) = update {
            self.lg3.threads = setting;
        }
        Ok(lg3_number(current as f64))
    }

    fn lg3_diary_file(&self) -> PathBuf {
        self.lg3.diary_file.clone().unwrap_or_else(|| {
            self.lg3_folder()
                .map_or_else(|| PathBuf::from("diary"), |f| f.join("diary"))
        })
    }
    fn lg3_diary(&mut self, args: &[Value]) -> Result<()> {
        lg3_arity(args, 0, 1)?;
        let on = match args.first() {
            None => !self.lg3.diary_on,
            Some(value) => match self.lg3_string(value)?.as_str() {
                "on" => true,
                "off" => false,
                text => {
                    let path = self
                        .lg3_resolve(text)
                        .ok_or("No current folder is available")?;
                    self.lg3.diary_on = false;
                    lg3_diary_open(&path)?;
                    self.lg3.diary_file = Some(path);
                    true
                }
            },
        };
        if on && !self.lg3.diary_on {
            lg3_diary_open(&self.lg3_diary_file())?;
        }
        self.lg3.diary_on = on;
        Ok(())
    }
    /// Append console text to the diary file while it is on: each emitted
    /// entry as a line, held `fprintf` text (no newline yet) as it is printed.
    /// A failed write turns the diary off and reports it.
    pub(super) fn lg3_diary_write(&mut self, text: &str, line: bool) -> Result<()> {
        if !self.lg3.diary_on {
            return Ok(());
        }
        use std::io::Write;
        let path = self.lg3_diary_file();
        let written = lg3_diary_open(&path).and_then(|mut file| {
            if line {
                writeln!(file, "{text}")
            } else {
                file.write_all(text.as_bytes())
            }
            .map_err(|e| e.to_string())
        });
        written.map_err(|e| {
            self.lg3.diary_on = false;
            format!(
                "Diary file '{}' could not be written ({e}); diary is now off.",
                lg3_path_text(&path)
            )
        })
    }

    /// Frame bookkeeping for dbstack: a function frame was entered or left.
    pub(super) fn lg3_enter(&mut self, function: &str) {
        let byte = self.lg3.byte;
        self.lg3.calls.push((function.to_owned(), byte));
    }
    pub(super) fn lg3_leave(&mut self) {
        if let Some((_, byte)) = self.lg3.calls.pop() {
            self.lg3.byte = byte;
        }
    }
    pub(super) fn lg3_step(&mut self, byte: usize) {
        if self.lg3.evals.last() != Some(&self.lg3.calls.len()) {
            self.lg3.byte = byte;
        }
    }
    pub(super) fn lg3_eval_begin(&mut self) {
        self.lg3.evals.push(self.lg3.calls.len());
    }
    pub(super) fn lg3_eval_end(&mut self) {
        self.lg3.evals.pop();
    }
}
