//! Environment and local filesystem operations. Folder and environment changes
//! belong to an engine, never to the process shared with desktop/other engines.
use super::{Engine, Execution, Value};
use crate::{
    Array, CharacterRow, LogicalMatrix, Matrix, Result, StructArray,
    clock::{Instant, system_now},
    syntax::Expr,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const NAMES: &[&str] = &[
    "getenv",
    "setenv",
    "ispc",
    "isunix",
    "ismac",
    "computer",
    "version",
    "ver",
    "verLessThan",
    "matlabroot",
    "filesep",
    "pathsep",
    "pwd",
    "cd",
    "dir",
    "ls",
    "mkdir",
    "rmdir",
    "fileread",
    "tempdir",
    "tempname",
    "clock",
    "cputime",
    "date",
    "now",
    "pause",
    "timeit",
    "isdir",
    "copyfile",
    "movefile",
    "delete",
];
pub(super) fn envx_handles(name: &str) -> bool {
    NAMES.contains(&name)
}
const MAX_ENTRIES: usize = 10_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const DIR_FIELDS: &[&str] = &["name", "date", "bytes", "isdir", "datenum"];

#[derive(Clone, Default)]
pub(super) struct Environment {
    variables: BTreeMap<String, String>,
    pause_off: bool,
}

fn envx_arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}
fn envx_bool(value: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(value))
}
fn envx_number(value: f64) -> Value {
    Value::Matrix(Matrix::scalar(value))
}
fn envx_text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}
pub(super) fn envx_path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(network) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{network}");
        }
        if let Some(local) = text.strip_prefix(r"\\?\") {
            return local.into();
        }
    }
    text.into_owned()
}
fn envx_arch() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("PCWIN64", "win64")
    } else if cfg!(target_os = "macos") {
        ("MACI64", "maci64")
    } else {
        ("GLNXA64", "glnxa64")
    }
}
fn envx_wildcard(pattern: &str, name: &str) -> bool {
    // Bounded, iterative glob matching; only * and ? in the final component.
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len()
            && (p[i] == '?' || p[i] == n[j] || cfg!(windows) && p[i].eq_ignore_ascii_case(&n[j]))
        {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

impl Engine {
    /// Set the engine's current folder for both file I/O and script/function
    /// lookup. Validation precedes mutation; the host process CWD is unchanged.
    pub fn set_current_folder(&mut self, directory: &Path) -> Result<()> {
        let path = if directory.is_relative() {
            self.current_folder()
                .ok_or("No current folder is available")?
                .join(directory)
        } else {
            directory.to_owned()
        };
        let path = path
            .canonicalize()
            .map_err(|e| format!("Current folder: {e}"))?;
        if !path.is_dir() {
            return Err("Current folder must be a directory".into());
        }
        self.set_function_directory(&path)?;
        self.file_directory = Some(path);
        Ok(())
    }

    /// The engine's file/current folder, independent of the host process.
    pub fn current_folder(&self) -> Option<&Path> {
        self.file_directory.as_deref()
    }

    pub(super) fn envx_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !envx_handles(name) {
            return Ok(None);
        }
        Ok(Some(self.envx_run(name, args, 1, out)?.remove(0)))
    }

    pub(super) fn envx_statement(
        &mut self,
        name: &str,
        expressions: &[Expr],
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        if !matches!(
            name,
            "setenv"
                | "cd"
                | "dir"
                | "ls"
                | "mkdir"
                | "rmdir"
                | "copyfile"
                | "movefile"
                | "delete"
                | "pause"
                | "ver"
        ) {
            return Ok(false);
        }
        let args = self.function_arguments(expressions, None, out)?;
        if name == "pause"
            && args
                .first()
                .is_some_and(|a| self.envx_string(a, "").is_ok_and(|s| s == "query"))
        {
            // pause('query') is a function value like any other: it sets ans.
            let value = self.envx_run(name, &args, 1, out)?.remove(0);
            self.assign_workspace_array("ans", value.array(&self.cancel)?, quiet, out)?;
            return Ok(true);
        }
        self.envx_run(name, &args, 0, out)?;
        Ok(true)
    }

    pub(super) fn envx_eval_outputs(
        &mut self,
        expression: &Expr,
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let (name, expressions) = match expression {
            Expr::Call(name, args) => (name.as_str(), args.as_slice()),
            Expr::Name(name) => (name.as_str(), &[][..]),
            _ => return Ok(None),
        };
        if !matches!(
            name,
            "computer" | "mkdir" | "rmdir" | "copyfile" | "movefile"
        ) || self.contains_array(name)
        {
            return Ok(None);
        }
        if let Some(function) = self.resolve_function(name)? {
            let args = self.function_arguments(expressions, None, out)?;
            return self
                .invoke_function(function, args, count, None, out)
                .map(Some);
        }
        let args = self.function_arguments(expressions, None, out)?;
        self.envx_run(name, &args, count, out).map(Some)
    }

    fn envx_string(&self, value: &Value, error: &str) -> Result<String> {
        let text = match value {
            Value::Text(s) | Value::QuotedString(s) => s.clone(),
            Value::Container(Array::Character(c)) if c.is_text_row() => c.to_utf8(&self.cancel)?,
            _ => return Err(error.into()),
        };
        if text.len() > 65536 || text.contains('\0') {
            return Err("Text is oversized or contains NUL".into());
        }
        Ok(text)
    }
    fn envx_path(&self, text: &str) -> Result<PathBuf> {
        if text.is_empty() || text.len() > 32768 || text.contains('\0') {
            return Err("Filename is empty, oversized or contains NUL".into());
        }
        if text.contains("://") || text.starts_with("\\\\") || text.starts_with("//") {
            return Err("Network paths and URLs are not supported yet".into());
        }
        self.file_path(text, "")
    }
    fn envx_warning(&mut self, id: &str, message: String, out: &mut Execution) -> Result<()> {
        // With an identifier, warning formats its message: pass it as data.
        let args = [envx_text(id), envx_text("%s"), envx_text(message)];
        self.lang_utils_call("warning", &args, out)?;
        Ok(())
    }
    fn envx_status(
        &mut self,
        status: bool,
        message: String,
        id: &str,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if count == 0 {
            if !status {
                return Err(message);
            }
            if !message.is_empty() {
                self.envx_warning(id, message, out)?;
            }
            return Ok(Vec::new());
        }
        let mut values = vec![envx_bool(status), envx_text(message), envx_text(id)];
        values.truncate(count);
        Ok(values)
    }
    fn envx_run(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        #[cfg(target_arch = "wasm32")]
        if super::browser::file_command(name, Some(args)) {
            return Err(super::browser::NO_FILES.into());
        }
        let multi = matches!(
            name,
            "computer" | "mkdir" | "rmdir" | "copyfile" | "movefile"
        );
        if count > if multi { 3 } else { 1 } {
            return Err("Too many output arguments.".into());
        }
        let value = match name {
            "ispc" | "isunix" | "ismac" | "filesep" | "pathsep" | "pwd" | "matlabroot"
            | "tempdir" | "clock" | "date" | "now" | "cputime" => {
                envx_arity(args, 0, 0)?;
                match name {
                    "ispc" => envx_bool(cfg!(windows)),
                    "isunix" => envx_bool(cfg!(unix)),
                    "ismac" => envx_bool(cfg!(target_os = "macos")),
                    "filesep" => envx_text(std::path::MAIN_SEPARATOR.to_string()),
                    "pathsep" => envx_text(if cfg!(windows) { ";" } else { ":" }),
                    "pwd" => envx_text(envx_path_text(
                        self.current_folder()
                            .ok_or("No current folder is available")?,
                    )),
                    "matlabroot" => envx_text(envx_path_text(
                        std::env::current_exe()
                            .map_err(|e| e.to_string())?
                            .parent()
                            .ok_or("No installation folder")?,
                    )),
                    "tempdir" => {
                        let mut text = envx_path_text(&std::env::temp_dir());
                        if !text.ends_with(std::path::MAIN_SEPARATOR) {
                            text.push(std::path::MAIN_SEPARATOR);
                        }
                        envx_text(text)
                    }
                    "cputime" => envx_number(envx_cpu_seconds()?),
                    _ => {
                        let parts = envx_local_clock(system_now())?;
                        let vector = Value::Matrix(Matrix::from_column_vec(1, 6, parts.to_vec())?);
                        if name == "clock" {
                            vector
                        } else {
                            let serial = self
                                .datex_call("datenum", &[vector])?
                                .ok_or("Date conversion failed")?;
                            if name == "now" {
                                serial
                            } else {
                                self.datex_call("datestr", &[serial, envx_number(1.)])?
                                    .ok_or("Date formatting failed")?
                            }
                        }
                    }
                }
            }
            "getenv" | "setenv" => {
                envx_arity(args, 1, if name == "getenv" { 1 } else { 2 })?;
                let key = self.envx_string(&args[0], "Argument must contain a string.")?;
                if key.is_empty() || key.contains('=') {
                    return Err("Invalid environment variable name.".into());
                }
                let key = if cfg!(windows) {
                    key.to_uppercase()
                } else {
                    key
                };
                if name == "getenv" {
                    envx_text(
                        self.envx
                            .variables
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| std::env::var(&key).unwrap_or_default()),
                    )
                } else {
                    let value = args
                        .get(1)
                        .map(|v| self.envx_string(v, "Argument must contain a string."))
                        .transpose()?
                        .unwrap_or_default();
                    let retained: usize = self
                        .envx
                        .variables
                        .iter()
                        .filter(|(k, _)| *k != &key)
                        .map(|(k, v)| k.len() + v.len())
                        .sum();
                    if retained + key.len() + value.len() > 1024 * 1024
                        || (!self.envx.variables.contains_key(&key)
                            && self.envx.variables.len() >= 1024)
                    {
                        return Err("Environment exceeds 1024 variables or 1 MiB".into());
                    }
                    self.envx.variables.insert(key, value);
                    Value::Void
                }
            }
            "computer" => {
                envx_arity(args, 0, 1)?;
                if let Some(arg) = args.first() {
                    if self.envx_string(arg, "Argument must contain a string.")? != "arch" {
                        return Err("Unknown command option.".into());
                    }
                    if count > 1 {
                        return Err("Too many output arguments.".into());
                    }
                    envx_text(envx_arch().1)
                } else if count > 1 {
                    let mut values = vec![
                        envx_text(envx_arch().0),
                        envx_number(281_474_976_710_655.),
                        envx_text(if cfg!(target_endian = "little") {
                            "L"
                        } else {
                            "B"
                        }),
                    ];
                    values.truncate(count);
                    return Ok(values);
                } else {
                    envx_text(envx_arch().0)
                }
            }
            "version" => {
                envx_arity(args, 0, 1)?;
                let option = args
                    .first()
                    .map(|v| self.envx_string(v, "Argument must contain a string."))
                    .transpose()?
                    .unwrap_or_default();
                envx_text(match option.as_str() {
                    "" => concat!("MatRust ", env!("CARGO_PKG_VERSION")),
                    "-release" => "alpha",
                    "-date" => "not recorded",
                    _ => return Err("Unknown command option.".into()),
                })
            }
            "ver" => {
                envx_arity(args, 0, 1)?;
                let product = args
                    .first()
                    .map(|v| self.envx_string(v, "Argument must contain a string."))
                    .transpose()?;
                let present = product.as_deref().is_none_or(|p| {
                    p.eq_ignore_ascii_case("matlab") || p.eq_ignore_ascii_case("matrust")
                });
                let slots = if present {
                    ["MatRust", env!("CARGO_PKG_VERSION"), "(alpha)", ""]
                        .into_iter()
                        .map(|s| envx_text(s).array(&self.cancel).map(Arc::new))
                        .collect::<Result<Vec<_>>>()?
                } else {
                    Vec::new()
                };
                Value::Container(Array::Struct(StructArray::from_slots(
                    usize::from(present),
                    1,
                    ["Name", "Version", "Release", "Date"]
                        .map(str::to_owned)
                        .to_vec(),
                    slots,
                    &self.cancel,
                )?))
            }
            "verLessThan" => {
                envx_arity(args, 2, 2)?;
                let product = self.envx_string(&args[0], "Argument must contain a string.")?;
                let installed = match product.as_str() {
                    "matlab" => "9.0",
                    "matrust" => env!("CARGO_PKG_VERSION"),
                    _ => return Err(format!("Toolbox '{product}' not found.")),
                };
                let wanted = self.envx_string(&args[1], "Argument must contain a string.")?;
                let parse = |s: &str| -> Result<[u64; 3]> {
                    let mut result = [0; 3];
                    for (i, p) in s.split('.').enumerate() {
                        if i >= 3 || p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()) {
                            return Err("Version must have one to three numeric components.".into());
                        }
                        result[i] = p.parse().map_err(|_| "Invalid version number")?;
                    }
                    Ok(result)
                };
                envx_bool(parse(installed)? < parse(&wanted)?)
            }
            "tempname" => {
                envx_arity(args, 0, 1)?;
                let folder = if let Some(v) = args.first() {
                    self.envx_path(&self.envx_string(v, "Argument must contain a string.")?)?
                } else {
                    std::env::temp_dir()
                };
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let stamp = system_now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos();
                let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
                envx_text(envx_path_text(&folder.join(format!(
                    "tp{:x}_{stamp:x}_{sequence:x}",
                    std::process::id()
                ))))
            }
            "cd" => {
                envx_arity(args, 0, 1)?;
                let old = self
                    .current_folder()
                    .ok_or("No current folder is available")?
                    .to_owned();
                if let Some(v) = args.first() {
                    let text = self.envx_string(v, "Argument must contain a string.")?;
                    let path = self.envx_path(&text)?;
                    self.set_current_folder(&path).map_err(|_| {
                        format!("Cannot CD to {text} (Name is nonexistent or not a directory).")
                    })?;
                    if count == 0 {
                        return Ok(Vec::new());
                    }
                }
                envx_text(envx_path_text(&old))
            }
            "isdir" => {
                envx_arity(args, 1, 1)?;
                let text =
                    self.envx_string(&args[0], "The first input to exist must be a string.")?;
                envx_bool(!text.is_empty() && self.envx_path(&text)?.is_dir())
            }
            "mkdir" | "rmdir" => {
                envx_arity(args, 1, 2)?;
                let first = self.envx_string(&args[0], "Argument must contain a string.")?;
                let mut path = self.envx_path(&first)?;
                let option = args
                    .get(1)
                    .map(|v| self.envx_string(v, "Argument must contain a string."))
                    .transpose()?;
                if name == "mkdir" {
                    if let Some(child) = option {
                        path = path.join(child);
                        self.envx_path(&envx_path_text(&path))?;
                    }
                    if path.is_dir() {
                        return self.envx_status(
                            true,
                            "Directory already exists.".into(),
                            "MATLAB:MKDIR:DirectoryExists",
                            count,
                            out,
                        );
                    }
                    return match fs::create_dir_all(&path) {
                        Ok(()) => self.envx_status(true, String::new(), "", count, out),
                        Err(e) => self.envx_status(
                            false,
                            e.to_string(),
                            "MATLAB:MKDIR:OSError",
                            count,
                            out,
                        ),
                    };
                }
                if option.as_deref().is_some_and(|s| s != "s") {
                    return Err("Unknown command option.".into());
                }
                if !path.is_dir() {
                    return self.envx_status(
                        false,
                        format!("{} is not a directory.", envx_path_text(&path)),
                        "MATLAB:RMDIR:NotADirectory",
                        count,
                        out,
                    );
                }
                let mut removed = 0;
                let result = if option.is_some() {
                    self.envx_remove_tree(&path, 0, &mut 0, &mut removed)
                } else {
                    fs::remove_dir(&path).map_err(|e| e.to_string())
                };
                match result {
                    Ok(()) => return self.envx_status(true, String::new(), "", count, out),
                    Err(e) if super::errors::is_run_limit(&e) => return Err(e),
                    Err(_) if removed == 0 => {
                        return self.envx_status(
                            false,
                            "No directories were removed.".into(),
                            "MATLAB:RMDIR:NoDirectoriesRemoved",
                            count,
                            out,
                        );
                    }
                    Err(e) => {
                        return self.envx_status(
                            false,
                            format!(
                                "{removed} files or folders were removed, but the folder could not be removed completely: {e}"
                            ),
                            "MATLAB:RMDIR:OSError",
                            count,
                            out,
                        );
                    }
                }
            }
            "fileread" => {
                envx_arity(args, 1, 1)?;
                let text = self.envx_string(&args[0], "Filename must be a string.")?;
                let path = self.envx_path(&text)?;
                let mut file = fs::File::open(path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        format!("Could not open file {text}. No such file or directory.")
                    } else {
                        format!("Could not open file {text}. {e}")
                    }
                })?;
                let mut data = Vec::new();
                let mut chunk = [0; 8192];
                loop {
                    self.check_cancel()?;
                    let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    if data.len() + n > 4 * crate::script_containers::MAX_CHARACTER_UNITS {
                        return Err("fileread exceeds character size limit".into());
                    }
                    data.extend_from_slice(&chunk[..n]);
                }
                let text = String::from_utf8(data)
                    .map_err(|_| "fileread: only UTF-8 text is supported yet")?;
                CharacterRow::checked_len(1, text.encode_utf16().count())?;
                envx_text(text)
            }
            "dir" | "ls" => {
                envx_arity(args, 0, 1)?;
                let text = args
                    .first()
                    .map(|v| self.envx_string(v, "Function is not defined for 'double' inputs."))
                    .transpose()?
                    .unwrap_or_else(|| ".".into());
                let entries = self.envx_entries(&text, true)?;
                if count == 0 {
                    let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
                    self.emit(out, format!("\n{}\n", names.join("   ")))?;
                    return Ok(Vec::new());
                }
                if name == "ls" {
                    self.envx_char_rows(entries.iter().map(|(n, _)| n.clone()).collect())?
                } else {
                    self.envx_directory(entries)?
                }
            }
            "delete" => {
                envx_arity(args, 1, 64)?;
                for arg in args {
                    let text = self.envx_string(arg, "Invalid or deleted object.")?;
                    let entries = self.envx_entries(&text, false)?;
                    if entries.is_empty() {
                        self.envx_warning(
                            "MATLAB:DELETE:FileNotFound",
                            format!("File '{text}' not found."),
                            out,
                        )?;
                    }
                    let wildcard = text.contains(['*', '?']);
                    for (_, path) in entries {
                        self.check_cancel()?;
                        if path.is_dir() {
                            if wildcard {
                                continue;
                            }
                            return Err("delete cannot remove directories; use rmdir.".into());
                        }
                        fs::remove_file(path).map_err(|e| e.to_string())?;
                    }
                }
                Value::Void
            }
            "copyfile" | "movefile" => return self.envx_transfer(name, args, count, out),
            "pause" => {
                envx_arity(args, 0, 1)?;
                let Some(arg) = args.first() else {
                    return Err("pause without an argument waits for a key and is not supported yet; use pause(seconds).".into());
                };
                if arg.is_character() || matches!(arg, Value::QuotedString(_)) {
                    match self.envx_string(arg, "Unknown command option.")?.as_str() {
                        "query" => {
                            let state = if self.envx.pause_off { "off" } else { "on" };
                            return Ok(vec![envx_text(state)]);
                        }
                        "on" => self.envx.pause_off = false,
                        "off" => self.envx.pause_off = true,
                        _ => return Err("Unknown command option.".into()),
                    }
                } else {
                    let seconds = self.real_arg(name, arg)?.as_scalar()?;
                    if seconds.is_nan() {
                        return Err("Pause duration must not be NaN.".into());
                    }
                    if !self.envx.pause_off && seconds > 0. {
                        let start = Instant::now();
                        while start.elapsed().as_secs_f64() < seconds {
                            self.check_cancel()?;
                            // wasm32 cannot sleep a thread: the browser worker
                            // waits by polling the clock until the deadline.
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                let remaining =
                                    (seconds - start.elapsed().as_secs_f64()).clamp(0., 0.01);
                                std::thread::sleep(Duration::from_secs_f64(remaining));
                            }
                        }
                    }
                    self.check_cancel()?;
                }
                Value::Void
            }
            "timeit" => {
                envx_arity(args, 1, 2)?;
                let Value::Container(Array::Handle(handle)) = &args[0] else {
                    return Err(
                        "First argument must be a function handle that takes no input argument."
                            .into(),
                    );
                };
                let outputs = if let Some(v) = args.get(1) {
                    let n = self.real_arg(name, v)?.as_scalar()?;
                    if !n.is_finite() || !(0. ..=16.).contains(&n) || n.fract() != 0. {
                        return Err("timeit output count must be an integer from 0 to 16.".into());
                    }
                    n as usize
                } else {
                    1
                };
                // Five independent samples, bounded to 25 ms per batch and 64
                // calls per sample. The ordinary execution budget still applies.
                self.call_handle(handle, Vec::new(), outputs, None, out)?;
                let mut samples = [0.; 5];
                for sample in &mut samples {
                    let start = Instant::now();
                    let mut calls = 0;
                    while calls == 0 || calls < 64 && start.elapsed() < Duration::from_millis(25) {
                        self.check_cancel()?;
                        self.call_handle(handle, Vec::new(), outputs, None, out)?;
                        calls += 1;
                    }
                    *sample = start.elapsed().as_secs_f64() / calls as f64;
                }
                samples.sort_by(f64::total_cmp);
                if samples[2] < 1e-5 {
                    self.envx_warning(
                        "MATLAB:timeit:HighOverhead",
                        "The measured time for F may be inaccurate because it is running too fast. Try measuring something\nthat takes longer.".into(),
                        out,
                    )?;
                }
                envx_number(samples[2])
            }
            _ => return Err("Unknown environment function".into()),
        };
        if count == 0 {
            if !matches!(value, Value::Void) {
                let array = value.array(&self.cancel)?;
                if let Some(text) = crate::display_format::disp(&array, &self.display)? {
                    self.emit(out, text)?;
                }
            }
            Ok(Vec::new())
        } else {
            Ok(vec![value])
        }
    }

    fn envx_entries(&self, text: &str, list_directory: bool) -> Result<Vec<(String, PathBuf)>> {
        let path = self.envx_path(text)?;
        let pattern = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let wildcard = pattern.contains(['*', '?']);
        let directory = if wildcard {
            path.parent().unwrap_or(Path::new("."))
        } else {
            &path
        };
        if wildcard && envx_path_text(directory).contains(['*', '?']) {
            return Err("Wildcards in parent directories are not supported yet".into());
        }
        if !wildcard && (!list_directory || !path.is_dir()) {
            return Ok(if path.exists() {
                vec![(pattern.to_owned(), path.clone())]
            } else {
                Vec::new()
            });
        }
        if pattern.len() > 1024 && wildcard {
            return Err("Wildcard pattern exceeds 1024 bytes".into());
        }
        let mut entries = Vec::new();
        let mut retained_bytes = 0;
        if list_directory {
            for name in [".", ".."] {
                if !wildcard || envx_wildcard(pattern, name) {
                    entries.push((name.into(), directory.join(name)));
                }
            }
        }
        let dots = entries.len();
        let scan = match fs::read_dir(directory) {
            Ok(scan) => scan,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.to_string()),
        };
        for (i, entry) in scan.enumerate() {
            self.check_cancel()?;
            if i >= MAX_ENTRIES {
                return Err("Directory scan exceeds 10000 entries".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Filename is not valid Unicode")?;
            if !wildcard || envx_wildcard(pattern, &name) {
                let path = entry.path();
                retained_bytes += name.len() + path.as_os_str().len();
                if retained_bytes > 8 * 1024 * 1024 {
                    return Err("Directory listing exceeds 8 MiB".into());
                }
                entries.push((name, path));
            }
        }
        // "." and ".." stay first; the rest sorts case-insensitively, ties by bytes.
        entries[dots..].sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then_with(|| a.0.cmp(&b.0))
        });
        Ok(entries)
    }
    fn envx_char_rows(&self, rows: Vec<String>) -> Result<Value> {
        let width = rows
            .iter()
            .map(|r| r.encode_utf16().count())
            .max()
            .unwrap_or(0);
        CharacterRow::checked_len(rows.len(), width)?;
        let mut data = vec![u16::from(b' '); rows.len() * width];
        for (r, row) in rows.iter().enumerate() {
            self.check_cancel()?;
            for (c, ch) in row.encode_utf16().enumerate() {
                data[c * rows.len() + r] = ch;
            }
        }
        Ok(Value::Container(Array::Character(
            CharacterRow::from_column_major(rows.len(), width, data, &self.cancel)?,
        )))
    }
    fn envx_directory(&mut self, entries: Vec<(String, PathBuf)>) -> Result<Value> {
        let count = entries.len();
        let mut slots = Vec::with_capacity(count * 5);
        for (name, path) in entries {
            self.check_cancel()?;
            // A dangling symlink lists with its own metadata, not as an error.
            let metadata = fs::metadata(&path)
                .or_else(|_| fs::symlink_metadata(&path))
                .map_err(|e| e.to_string())?;
            let clock = envx_local_clock(metadata.modified().unwrap_or(UNIX_EPOCH))?;
            let vector = Value::Matrix(Matrix::from_column_vec(1, 6, clock.to_vec())?);
            let serial = self
                .datex_call("datenum", &[vector])?
                .ok_or("Date conversion failed")?;
            let date = self
                .datex_call("datestr", &[serial.clone(), envx_number(0.)])?
                .ok_or("Date formatting failed")?;
            for value in [
                envx_text(name),
                date,
                envx_number(if metadata.is_dir() {
                    0.
                } else {
                    metadata.len() as f64
                }),
                envx_bool(metadata.is_dir()),
                serial,
            ] {
                slots.push(Arc::new(value.array(&self.cancel)?));
            }
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            count,
            1,
            DIR_FIELDS.iter().map(|s| (*s).into()).collect(),
            slots,
            &self.cancel,
        )?)))
    }
    fn envx_remove_tree(
        &self,
        path: &Path,
        depth: usize,
        visited: &mut usize,
        removed: &mut usize,
    ) -> Result<()> {
        self.check_cancel()?;
        *visited += 1;
        if depth > 64 || *visited > MAX_ENTRIES {
            return Err("Recursive removal exceeds 64 levels or 10000 entries".into());
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.is_symlink() || !metadata.is_dir() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
            *removed += 1;
            return Ok(());
        }
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            self.envx_remove_tree(
                &entry.map_err(|e| e.to_string())?.path(),
                depth + 1,
                visited,
                removed,
            )?;
        }
        fs::remove_dir(path).map_err(|e| e.to_string())?;
        *removed += 1;
        Ok(())
    }
    fn envx_transfer(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        envx_arity(args, 1, 3)?;
        let source = self.envx_string(&args[0], "Argument must contain a string.")?;
        let destination = args
            .get(1)
            .map(|v| self.envx_string(v, "Argument must contain a string."))
            .transpose()?
            .unwrap_or_else(|| ".".into());
        let force = args
            .get(2)
            .map(|v| self.envx_string(v, "Argument must contain a string."))
            .transpose()?;
        if force.as_deref().is_some_and(|s| s != "f") {
            return Err("Unknown command option.".into());
        }
        let entries = self.envx_entries(&source, false)?;
        if entries.is_empty() {
            let id = format!("MATLAB:{}:FileDoesNotExist", name.to_ascii_uppercase());
            return self.envx_status(
                false,
                "No matching files were found.".into(),
                &id,
                count,
                out,
            );
        }
        let destination = self.envx_path(&destination)?;
        if envx_path_text(&destination).contains(['*', '?']) {
            return Err("Wildcards are not supported in the destination.".into());
        }
        let wildcard = source.contains(['*', '?']);
        let result = (|| {
            if wildcard && !destination.exists() {
                fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
            }
            if entries.len() > 1 && !destination.is_dir() {
                return Err("Destination must be a directory for multiple sources.".into());
            }
            let mut visited = 0;
            let mut bytes = 0;
            for (leaf, source) in entries {
                self.check_cancel()?;
                let target = if destination.is_dir()
                    && (name == "movefile" || !source.is_dir() || wildcard)
                {
                    destination.join(leaf)
                } else {
                    destination.clone()
                };
                let canonical = source.canonicalize().map_err(|e| e.to_string())?;
                let parent = target
                    .parent()
                    .ok_or("Invalid destination")?
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let normalized = parent.join(target.file_name().ok_or("Invalid destination")?);
                // a.txt -> A.txt on a case-insensitive file system is the same file.
                let case_only = name == "movefile"
                    && normalized != canonical
                    && normalized.to_string_lossy().to_lowercase()
                        == canonical.to_string_lossy().to_lowercase();
                if normalized == canonical
                    || (source.is_dir() && normalized.starts_with(&canonical))
                    || (!case_only && target.canonicalize().is_ok_and(|p| p == canonical))
                {
                    return Err(
                        "Source and destination must be different and cannot contain one another."
                            .into(),
                    );
                }
                if name == "movefile" {
                    if source.symlink_metadata().is_ok_and(|m| m.is_symlink())
                        || target.symlink_metadata().is_ok_and(|m| m.is_symlink())
                    {
                        return Err("Moving symbolic links is not supported yet".into());
                    }
                    self.envx_writable_destination(&target, force.is_some())?;
                    // Same-filesystem moves use the OS rename kernel and do
                    // not read or copy file contents. Fall back only across devices.
                    match fs::rename(&source, &target) {
                        Ok(()) => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {}
                        Err(error) => return Err(error.to_string()),
                    }
                }
                self.envx_copy_tree(
                    &source,
                    &target,
                    force.is_some(),
                    0,
                    &mut visited,
                    &mut bytes,
                )?;
                if name == "movefile" {
                    self.envx_remove_tree(&source, 0, &mut 0, &mut 0)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => self.envx_status(true, String::new(), "", count, out),
            Err(e) if super::errors::is_run_limit(&e) => Err(e),
            Err(e) => self.envx_status(false, e, "MatRust:File:OperationFailed", count, out),
        }
    }
    fn envx_copy_tree(
        &self,
        source: &Path,
        target: &Path,
        force: bool,
        depth: usize,
        visited: &mut usize,
        bytes: &mut u64,
    ) -> Result<()> {
        self.check_cancel()?;
        *visited += 1;
        if depth > 64 || *visited > MAX_ENTRIES {
            return Err("Copy exceeds 64 levels or 10000 entries".into());
        }
        let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            return Err("Copying symbolic links is not supported yet".into());
        }
        if target.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
            return Err("Copying onto symbolic links is not supported yet".into());
        }
        if metadata.is_dir() {
            fs::create_dir_all(target).map_err(|e| e.to_string())?;
            for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                self.envx_copy_tree(
                    &entry.path(),
                    &target.join(entry.file_name()),
                    force,
                    depth + 1,
                    visited,
                    bytes,
                )?;
            }
        } else {
            if !metadata.is_file() {
                return Err("Only regular files and directories are supported".into());
            }
            if *bytes + metadata.len() > MAX_BYTES {
                return Err("Copy is larger than the 268435456-byte limit".into());
            }
            let destination_permissions = target.metadata().ok().map(|m| m.permissions());
            self.envx_writable_destination(target, force)?;
            let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
            let mut output =
                tempfile::NamedTempFile::new_in(target.parent().ok_or("Invalid destination")?)
                    .map_err(|e| e.to_string())?;
            let mut chunk = [0; 65536];
            loop {
                self.check_cancel()?;
                let n = input.read(&mut chunk).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                *bytes += n as u64;
                if *bytes > MAX_BYTES {
                    return Err("Copy is larger than the 268435456-byte limit".into());
                }
                output.write_all(&chunk[..n]).map_err(|e| e.to_string())?;
            }
            self.check_cancel()?;
            let mut times = fs::FileTimes::new();
            if let Ok(modified) = metadata.modified() {
                times = times.set_modified(modified);
            }
            if let Ok(accessed) = metadata.accessed() {
                times = times.set_accessed(accessed);
            }
            output
                .as_file()
                .set_times(times)
                .map_err(|e| e.to_string())?;
            output
                .as_file()
                .set_permissions(destination_permissions.unwrap_or_else(|| metadata.permissions()))
                .map_err(|e| e.to_string())?;
            output.persist(target).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn envx_writable_destination(&self, target: &Path, force: bool) -> Result<()> {
        if let Ok(existing) = target.metadata()
            && existing.permissions().readonly()
        {
            if !force {
                return Err("Destination is read-only; use the 'f' option.".into());
            }
            // Preserve all permission bits except the owner's write bit.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    target,
                    fs::Permissions::from_mode(existing.permissions().mode() | 0o200),
                )
                .map_err(|e| e.to_string())?;
            }
            #[cfg(windows)]
            {
                let mut permissions = existing.permissions();
                // Windows has only the read-only attribute; no mode bits exist.
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                fs::set_permissions(target, permissions).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn envx_local_clock(time: SystemTime) -> Result<[f64; 6]> {
    use std::ffi::{c_char, c_int, c_long};
    #[repr(C)]
    struct Tm {
        sec: c_int,
        min: c_int,
        hour: c_int,
        day: c_int,
        month: c_int,
        year: c_int,
        weekday: c_int,
        yearday: c_int,
        dst: c_int,
        offset: c_long,
        zone: *const c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const c_long, result: *mut Tm) -> *mut Tm;
    }
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(error) => -error.duration().as_secs_f64(),
    };
    if seconds < c_long::MIN as f64 || seconds >= c_long::MAX as f64 {
        return Err("Date is out of range".into());
    }
    let whole = seconds.floor() as c_long;
    let mut result = std::mem::MaybeUninit::<Tm>::uninit();
    // SAFETY: localtime_r writes exactly one platform tm to the supplied aligned
    // storage; null is checked before initialization is assumed. No shared tm.
    let tm = unsafe {
        if localtime_r(&whole, result.as_mut_ptr()).is_null() {
            return Err("Local clock conversion failed".into());
        }
        result.assume_init()
    };
    Ok([
        (tm.year + 1900) as f64,
        (tm.month + 1) as f64,
        tm.day as f64,
        tm.hour as f64,
        tm.min as f64,
        tm.sec as f64 + (seconds - seconds.floor()),
    ])
}
#[cfg(unix)]
fn envx_cpu_seconds() -> Result<f64> {
    use std::ffi::{c_int, c_long};
    #[repr(C)]
    struct Timespec {
        sec: c_long,
        nsec: c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: c_int, time: *mut Timespec) -> c_int;
    }
    let mut time = Timespec { sec: 0, nsec: 0 };
    let id = if cfg!(target_os = "macos") { 12 } else { 2 };
    // SAFETY: a valid process CPU clock and writable timespec, no pointers retained.
    if unsafe { clock_gettime(id, &mut time) } != 0 {
        return Err("Process CPU clock is unavailable".into());
    }
    Ok(time.sec as f64 + time.nsec as f64 * 1e-9)
}

#[cfg(windows)]
mod envx_windows {
    use super::*;
    #[repr(C)]
    #[derive(Default)]
    pub(super) struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    pub(super) struct LocalTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            created: *mut FileTime,
            exited: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn FileTimeToLocalFileTime(file: *const FileTime, local: *mut FileTime) -> i32;
        fn FileTimeToSystemTime(file: *const FileTime, time: *mut LocalTime) -> i32;
    }
    pub(super) fn envx_local_clock(time: SystemTime) -> Result<[f64; 6]> {
        let elapsed = match time.duration_since(UNIX_EPOCH) {
            Ok(elapsed) => elapsed.as_nanos() as i128,
            Err(error) => -(error.duration().as_nanos() as i128),
        };
        let ticks = elapsed.div_euclid(100) + 116_444_736_000_000_000;
        let ticks = u64::try_from(ticks).map_err(|_| "Date is out of range")?;
        let file = FileTime {
            low: ticks as u32,
            high: (ticks >> 32) as u32,
        };
        let mut local = FileTime::default();
        let mut time = LocalTime::default();
        // SAFETY: all arguments point to initialized, correctly sized Windows
        // structures and the functions retain no pointers.
        if unsafe { FileTimeToLocalFileTime(&file, &mut local) } == 0
            || unsafe { FileTimeToSystemTime(&local, &mut time) } == 0
        {
            return Err("Local clock conversion failed".into());
        }
        Ok([
            time.year as f64,
            time.month as f64,
            time.day as f64,
            time.hour as f64,
            time.minute as f64,
            time.second as f64 + elapsed.rem_euclid(1_000_000_000) as f64 * 1e-9,
        ])
    }
    pub(super) fn envx_cpu_seconds() -> Result<f64> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        // SAFETY: current-process pseudo-handle is valid; output pointers refer
        // to four distinct writable FILETIMEs and are not retained.
        if unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            return Err("Process CPU clock is unavailable".into());
        }
        let ticks = |t: FileTime| ((t.high as u64) << 32) | t.low as u64;
        Ok((ticks(kernel) as f64 + ticks(user) as f64) * 1e-7)
    }
}
#[cfg(windows)]
use envx_windows::{envx_cpu_seconds, envx_local_clock};

/// Browser build: the JavaScript Date gives the local calendar time; there is
/// no process CPU clock, so cputime counts wall time since its first use.
#[cfg(target_arch = "wasm32")]
mod envx_browser {
    use super::*;
    pub(super) fn envx_local_clock(time: SystemTime) -> Result<[f64; 6]> {
        let seconds = match time.duration_since(UNIX_EPOCH) {
            Ok(elapsed) => elapsed.as_secs_f64(),
            Err(error) => -error.duration().as_secs_f64(),
        };
        let whole = seconds.floor();
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(whole * 1000.));
        if date.get_time().is_nan() {
            return Err("Date is out of range".into());
        }
        Ok([
            date.get_full_year() as f64,
            (date.get_month() + 1) as f64,
            date.get_date() as f64,
            date.get_hours() as f64,
            date.get_minutes() as f64,
            date.get_seconds() as f64 + (seconds - whole),
        ])
    }
    pub(super) fn envx_cpu_seconds() -> Result<f64> {
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        Ok(ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64())
    }
}
#[cfg(target_arch = "wasm32")]
use envx_browser::{envx_cpu_seconds, envx_local_clock};
