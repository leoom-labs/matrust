//! Console input and processes the host provides: `input`, bare `pause`,
//! `system`, `display`, `exit`/`quit` and `fprintf(2, ...)`. The engine asks its host for a line of text
//! through [`HostIo`]; the host decides where the prompt appears (stdout for
//! the CLI, the Command Window for the desktop). Without a host every read
//! answers "no input", which is the browser's and the tests' default.
use super::{Engine, Execution, Value};
use crate::{Array, Matrix, Result, syntax::Expr};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, Ordering},
};

enum EditTarget {
    Local(std::path::PathBuf),
    Added(std::path::PathBuf),
}

/// The host side of console input.
pub trait HostIo: Send {
    /// Whether completed output lines should be shown as the engine emits them.
    /// Hosts that leave this false receive output at the end of a run as before.
    fn streams_output(&self) -> bool {
        false
    }
    /// Show `prompt` and wait for one line of input, without its line ending.
    /// `None` means the wait was cancelled or no input is available.
    /// A host that blocks should return soon after the engine's cancel flag
    /// is set.
    fn read_line(&mut self, prompt: &str) -> Option<String>;
    /// Output produced before a prompt, so it can appear before the prompt.
    /// `clear` is set when `clc` ran before these lines. Returns `true` when
    /// the host showed the lines; the engine then drops them from the run's
    /// output. The default keeps them in the run's output.
    fn show_output(&mut self, _lines: &[String], _clear: bool) -> bool {
        false
    }
    /// `exit`/`quit` asks to end the process with `status`. A host that owns
    /// its process (the CLI) returns `true`; the run then stops and the host
    /// reads the status with [`Engine::take_exit_request`] and exits itself.
    /// The default refuses: the desktop app and the browser keep running and
    /// the builtin reports an error.
    fn request_exit(&mut self, _status: u8) -> bool {
        false
    }
    /// Text written with `fprintf(2, ...)`. Returns `true` when the host wrote
    /// it to its error stream; the default shows it with ordinary output.
    fn write_error(&mut self, _text: &str) -> bool {
        false
    }
    /// `drawnow` (and `pause` with a figure open) flushes graphics: a host
    /// that shows figures while a script runs redraws from this snapshot.
    /// Snapshots share sample storage with the engine. The default (CLI,
    /// `--svg`, browser) ignores it.
    fn draw_frame(&mut self, _frame: GraphicsFrame) {}
    /// Show a dialog (uigetfile, inputdlg, msgbox, waitbar, ...). Requests
    /// that [`super::DialogRequest::blocks`] wait for the user's answer and
    /// should return soon after the engine's cancel flag is set (with
    /// `Cancelled`). The default has no windows: the builtin then fails with
    /// "dialogs need the desktop app".
    fn dialog(&mut self, _request: super::DialogRequest) -> super::DialogReply {
        super::DialogReply::Unsupported
    }
    /// Open `path` (or a new unsaved buffer with `untitled_name`) in the
    /// desktop editor. Returns `false` when the host has no editor.
    fn edit_file(&mut self, _path: Option<&std::path::Path>, _untitled_name: Option<&str>) -> bool {
        false
    }
    /// Show an added-folder file. A script opens with a read-only source so the
    /// user must choose a destination on Save.
    fn edit_file_copy(&mut self, _path: &std::path::Path) -> bool {
        false
    }
    /// Open an http/https `url` in the system browser. Returns `None` when the
    /// host has no desktop browser opener (CLI / `-batch`).
    fn open_url(&mut self, _url: &str) -> Option<std::result::Result<(), String>> {
        None
    }
    /// Ring a short system bell (desktop UI). CLI / `-batch` / tests are no-ops.
    fn beep(&mut self) {}
    /// Whether this host runs interactive UI windows and supports blocking `uiwait(fig)`.
    fn supports_ui_wait(&self) -> bool {
        false
    }
    /// Poll for a pending UI component interaction event while blocked in `uiwait(fig)`.
    fn poll_ui_event(&mut self, _timeout: std::time::Duration) -> Option<super::UiEvent> {
        None
    }
    /// Non-blocking poll for a queued UI component interaction event (used by `drawnow`/`pause`).
    fn try_poll_ui_event(&mut self) -> Option<super::UiEvent> {
        None
    }
}

/// Every open figure at one `drawnow`, plus the current figure number and UI snapshot.
#[derive(Clone, Debug)]
pub struct GraphicsFrame {
    pub figures: Vec<(u32, crate::plot::Figure)>,
    pub figure: Option<crate::plot::Figure>,
    pub current: u32,
    pub ui: super::UiSnapshot,
}

/// 0 = no exit requested, otherwise the status plus one.
pub(super) type ExitRequest = Arc<AtomicU16>;

/// The host shared by an engine and its clones.
pub(super) type SharedHost = Option<Arc<Mutex<Box<dyn HostIo>>>>;

/// Captured `system` output is bounded like other console output.
const MAX_SYSTEM_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_ANSWER: usize = 1_000_000;

pub(super) fn hostio_handles(name: &str) -> bool {
    matches!(
        name,
        "input" | "system" | "dos" | "unix" | "edit" | "web" | "display" | "exit" | "quit"
    )
}

fn hostio_empty() -> Value {
    Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).expect("empty matrix"))
}

impl Engine {
    /// Set the host that answers `input` and `pause`; `None` restores the default.
    pub fn set_host_io(&mut self, host: Option<Box<dyn HostIo>>) {
        self.host_io = host.map(|h| Arc::new(Mutex::new(h)));
    }

    /// Whether the current host consumes each completed output line immediately.
    pub(super) fn hostio_streams_output(&self) -> bool {
        self.host_io
            .as_ref()
            .and_then(|host| host.lock().ok().map(|host| host.streams_output()))
            .unwrap_or(false)
    }

    /// Give buffered completed lines to the host. A host that accepts them owns
    /// them, so the execution result retains only output it did not show.
    pub(super) fn hostio_show_output(&mut self, out: &mut Execution) -> Result<()> {
        let Some(host) = self.host_io.clone() else {
            return Ok(());
        };
        let mut host = host
            .lock()
            .map_err(|_| "The console host failed".to_owned())?;
        if (!out.output.is_empty() || out.clear_console)
            && host.show_output(&out.output, out.clear_console)
        {
            out.output.clear();
            out.clear_console = false;
        }
        Ok(())
    }

    fn hostio_text(&self, value: &Value, what: &str) -> Result<String> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
            Value::Container(Array::Character(c)) if c.is_text_row() => c.to_utf8(&self.cancel),
            _ => Err(format!(
                "{what} must be a character vector or string scalar."
            )),
        }
    }

    /// Ask the host for a line after `prompt`. Text `fprintf` held without a
    /// newline leads the prompt, and earlier output goes to the host first.
    fn hostio_read(&mut self, prompt: &str, out: &mut Execution) -> Result<Option<String>> {
        self.check_cancel()?;
        let Some(host) = self.host_io.clone() else {
            return Ok(None);
        };
        let prompt = std::mem::take(&mut self.pending_line) + prompt;
        self.hostio_show_output(out)?;
        let mut host = host
            .lock()
            .map_err(|_| "The console host failed".to_owned())?;
        let line = host.read_line(&prompt);
        drop(host);
        self.check_cancel()?;
        let Some(mut line) = line else {
            return Ok(None);
        };
        if line.ends_with('\n') {
            line.pop();
        }
        if line.ends_with('\r') {
            line.pop();
        }
        if line.len() > MAX_ANSWER || line.contains('\0') {
            return Err("The answer is longer than 1000000 bytes or contains NUL.".into());
        }
        Ok(Some(line))
    }

    pub(super) fn hostio_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !hostio_handles(name) {
            return Ok(None);
        }
        Ok(Some(self.hostio_run(name, args, 1, out)?.remove(0)))
    }

    /// `[status, out] = system(...)`; other names take the one-output path.
    pub(super) fn hostio_outputs(
        &mut self,
        expression: &Expr,
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let Expr::Call(name, expressions) = expression else {
            return Ok(None);
        };
        if !matches!(name.as_str(), "system" | "dos" | "unix")
            || count < 2
            || self.contains_array(name)
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
        self.hostio_run(name, &args, count, out).map(Some)
    }

    fn hostio_run(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        match name {
            "input" => {
                if count > 1 {
                    return Err("Too many output arguments.".into());
                }
                Ok(vec![self.hostio_input(args, out)?])
            }
            "system" | "dos" | "unix" => self.hostio_system(name, args, count, out),
            "edit" => self.hostio_edit(args),
            "web" => self.hostio_web(args, count, out),
            "exit" | "quit" => self.hostio_exit(name, args),
            _ => {
                if args.len() != 1 {
                    return Err(if args.is_empty() {
                        "Not enough input arguments."
                    } else {
                        "Too many input arguments."
                    }
                    .into());
                }
                // display(x) of a variable shows it as an unsuppressed statement
                // would, under its name. An expression has no name, and MATLAB's
                // display documentation (and GNU Octave) print no "ans =" header
                // for it: the value is shown as disp shows it.
                let Some(label) = self
                    .functions
                    .lng
                    .call_names
                    .first()
                    .filter(|n| !n.is_empty() && self.array(n).is_some())
                    .cloned()
                else {
                    self.call("disp", vec![args[0].clone()], out)?;
                    return Ok(vec![Value::Void]);
                };
                let text = self.hostio_display_text(&label, &args[0])?;
                self.emit(out, text)?;
                Ok(vec![Value::Void])
            }
        }
    }

    /// `exit`, `exit(status)` and the same for `quit`. An accepted request
    /// stops the run like a cancel (no `catch` handles it) and the run then
    /// ends normally with its output; the host ends the process. The engine
    /// never terminates the process itself.
    fn hostio_exit(&mut self, name: &str, args: &[Value]) -> Result<Vec<Value>> {
        let status = match args {
            [] => Some(0),
            [Value::Matrix(m)] if m.len() == 1 => {
                let v = m.data()[0];
                ((0.0..=255.0).contains(&v) && v.fract() == 0.0).then_some(v as u8)
            }
            _ => None,
        }
        .ok_or_else(|| format!("{name}: the status must be one integer from 0 to 255."))?;
        let accepted = match self.host_io.clone() {
            Some(host) => host
                .lock()
                .map_err(|_| "The console host failed".to_owned())?
                .request_exit(status),
            None => false,
        };
        if !accepted {
            return Err(format!(
                "{name} is not supported in this host; it ends scripts run with the matrust command line."
            ));
        }
        self.exit_request
            .store(u16::from(status) + 1, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
        Err(format!("{name} requested"))
    }

    pub(super) fn hostio_clear_exit(&mut self) {
        self.exit_request.store(0, Ordering::Relaxed);
    }

    /// At the end of a run: whether an accepted exit unwound it. The cancel
    /// flag it raised is cleared so held output can still be flushed.
    pub(super) fn hostio_exit_unwind(&mut self) -> bool {
        let exiting = self.exit_request.load(Ordering::Relaxed) != 0;
        if exiting {
            self.cancel.store(false, Ordering::Relaxed);
        }
        exiting
    }

    /// The status of an `exit`/`quit` the host accepted during the last run,
    /// cleared by reading it.
    pub fn take_exit_request(&mut self) -> Option<u8> {
        match self.exit_request.swap(0, Ordering::Relaxed) {
            0 => None,
            n => Some((n - 1) as u8),
        }
    }

    /// `fprintf(2, ...)`: give the text to a host with an error stream.
    /// `false` means the caller shows it as ordinary output.
    pub(super) fn hostio_write_error(&mut self, text: &str) -> Result<bool> {
        let Some(host) = self.host_io.clone() else {
            return Ok(false);
        };
        let mut host = host
            .lock()
            .map_err(|_| "The console host failed".to_owned())?;
        Ok(host.write_error(text))
    }

    /// Render exactly the kinds of value an unsuppressed statement can echo;
    /// a double-quoted string is a string scalar and echoes as one.
    fn hostio_display_text(&self, label: &str, value: &Value) -> Result<String> {
        self.display_text(label, &value.clone().array(&self.cancel)?)
    }

    fn hostio_input(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let prompt = self.hostio_text(&args[0], "The prompt")?;
        // As in MATLAB, \n in the prompt starts a new line.
        let prompt = prompt.replace("\\n", "\n");
        let text = match args.get(1) {
            None => false,
            Some(mode) if self.hostio_text(mode, "The second argument").as_deref() == Ok("s") => {
                true
            }
            Some(_) => return Err("The second argument to input must be 's'.".into()),
        };
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (prompt, text, out);
            Err("input is not available in the browser console".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        loop {
            let Some(line) = self.hostio_read(&prompt, out)? else {
                return Err("input: no answer is available (end of input).".into());
            };
            if text {
                return Ok(Value::Text(line));
            }
            if line.trim().is_empty() {
                return Ok(hostio_empty());
            }
            match self.hostio_evaluate(&line, out) {
                Ok(value) => return Ok(value),
                Err(message) if self.is_catchable(&message) => {
                    self.raised = None;
                    self.functions.lng.thrown_stack = None;
                    self.functions.lng.thrown_object = None;
                    let message = if message.starts_with("Error") {
                        message
                    } else {
                        format!("Error: {message}")
                    };
                    self.emit(out, message)?;
                }
                Err(message) => return Err(message),
            }
        }
    }

    /// Evaluate an `input` answer as one expression in the current workspace.
    fn hostio_evaluate(&mut self, line: &str, out: &mut Execution) -> Result<Value> {
        let statements = match crate::syntax::parse_in_workspace(line, &|n| self.contains_array(n))
        {
            Ok(statements) => statements,
            Err(error) => return Err(format!("Error: {}", error.message)),
        };
        let expression = match statements.as_slice() {
            [statement] => match &statement.kind {
                crate::syntax::StatementKind::Evaluate(e)
                    if e.targets.is_empty()
                        && e.list_target.is_none()
                        && e.indexed_target.is_none()
                        && e.path_target.is_none() =>
                {
                    e.expr.clone()
                }
                _ => return Err("The answer to input must be an expression.".into()),
            },
            _ => return Err("The answer to input must be one expression.".into()),
        };
        let failure = self.functions.failure.clone();
        let file_diagnostic = self.file_diagnostic.clone();
        let value = self.eval(&expression, None, out);
        if value.is_err() {
            self.functions.failure = failure;
            self.file_diagnostic = file_diagnostic;
        }
        match value? {
            Value::Void => Err("The answer to input must produce a value.".into()),
            value => Ok(value),
        }
    }

    /// Ring the host's bell if it has one.
    pub(super) fn hostio_beep(&mut self) {
        if let Some(host) = &self.host_io
            && let Ok(mut host) = host.lock()
        {
            host.beep();
        }
    }

    /// Bare `pause`: wait for the host's next line (Enter in the CLI).
    pub(super) fn hostio_pause(&mut self, out: &mut Execution) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = out;
            Err("pause is not available in the browser console; use pause(seconds).".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // End of input lets the run continue, as an Enter would.
            self.hostio_read("", out)?;
            Ok(())
        }
    }

    fn hostio_system(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if count > 2 {
            return Err("Too many output arguments.".into());
        }
        let command = self.hostio_text(&args[0], "The command")?;
        if let Some(extra) = args.get(1) {
            // system(cmd, '-echo') prints the output even when it is returned.
            if args.len() > 2 || self.hostio_text(extra, "The option").as_deref() != Ok("-echo") {
                return Err(format!("{name} accepts only the '-echo' option."));
            }
        }
        if command.len() > 32768 || command.contains('\0') {
            return Err("The command is longer than 32768 bytes or contains NUL.".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (count, out);
            Err(format!("{name} is not available in the browser console"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let folder = self.current_folder().map(std::path::Path::to_owned);
            let variables = self.envx_child_variables();
            let (status, bytes) =
                hostio_spawn(&command, &self.cancel, folder.as_deref(), variables)?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if count < 2 || args.len() == 2 {
                if let Some(text) = text.strip_suffix('\n') {
                    self.emit(out, text.strip_suffix('\r').unwrap_or(text).to_owned())?;
                } else {
                    self.hold_output(&text)?;
                }
            }
            let mut values = vec![Value::Matrix(Matrix::scalar(f64::from(status)))];
            if count == 2 {
                values.push(Value::Text(text));
            }
            Ok(values)
        }
    }

    fn hostio_edit(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() > 1 {
            return Err("Too many input arguments.".into());
        }
        let (target, untitled) = if let Some(arg) = args.first() {
            let name = self.hostio_text(arg, "The file name")?;
            if name.is_empty() || name.len() > 32768 || name.contains('\0') {
                return Err("edit: file name is empty, oversized or contains NUL".into());
            }
            if name.contains("://") || name.starts_with("\\\\") || name.starts_with("//") {
                return Err("edit: network paths and URLs are not supported".into());
            }
            crate::files::read_fallback::reject_drive_relative(std::path::Path::new(&name))?;
            let resolved = self.hostio_edit_target(&name);
            // Only scripts open from an added folder, and only as a copy whose
            // Save asks for a destination. Other kinds (models, figures, data)
            // would make the search-path file their default Save target.
            if let Some(EditTarget::Added(path)) = &resolved
                && !path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| ["m", "mr", "txt"].iter().any(|s| e.eq_ignore_ascii_case(s)))
            {
                return Err(format!(
                    "edit: \"{name}\" was found only in an added path folder; copy it to the current folder or open it with File > Open"
                ));
            }
            if let Some(found) = resolved {
                (Some(found), None)
            } else {
                let title = if std::path::Path::new(&name).extension().is_none() {
                    format!("{name}.m")
                } else {
                    name
                };
                (None, Some(title))
            }
        } else {
            (None, None)
        };
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (target, untitled);
            Err("edit is not available in the browser console".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let opened = match self.host_io.clone() {
                Some(host) => {
                    let mut host = host
                        .lock()
                        .map_err(|_| "The console host failed".to_owned())?;
                    match target {
                        Some(EditTarget::Added(path)) => host.edit_file_copy(&path),
                        Some(EditTarget::Local(path)) => host.edit_file(Some(&path), None),
                        None => host.edit_file(None, untitled.as_deref()),
                    }
                }
                None => false,
            };
            if !opened {
                return Err("edit needs the MatRust desktop".into());
            }
            Ok(vec![Value::Void])
        }
    }

    fn hostio_edit_target(&self, name: &str) -> Option<EditTarget> {
        let candidate = std::path::Path::new(name);
        if candidate.is_absolute() {
            return candidate
                .is_file()
                .then(|| EditTarget::Local(candidate.to_path_buf()));
        }
        if let Some(folder) = self.current_folder() {
            // Built like the read lookup's local path, so a file in a Windows
            // verbatim current folder is found before any added folder.
            let local = crate::files::read_fallback::local_file(folder, candidate);
            if local.is_file() {
                return Some(EditTarget::Local(local));
            }
        }
        let context = self.help_path();
        for folder in context.folders() {
            let found = if candidate.extension().is_some() {
                folder.existing_file(candidate)
            } else if crate::files::valid_name(name) {
                super::user_functions::spath_existing(folder, std::path::Path::new(""), name).next()
            } else {
                None
            };
            if let Some(file) = found {
                return Some(if folder.added {
                    EditTarget::Added(file)
                } else {
                    EditTarget::Local(file)
                });
            }
        }
        None
    }

    fn hostio_web(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        if count > 1 {
            return Err("Too many output arguments.".into());
        }
        let url = self.hostio_text(&args[0], "The URL")?;
        if let Some(opt) = args.get(1)
            && self.hostio_text(opt, "The option").as_deref() != Ok("-browser")
        {
            return Err("web: unsupported option; only '-browser' is supported".into());
        }
        if url.is_empty() || url.len() > 32768 || url.chars().any(char::is_control) {
            return Err("web: URL is empty, oversized or contains control characters".into());
        }
        // RFC 3986 reserved/unreserved characters and percent escapes only.
        // Windows passes this URL to rundll32 as a raw command-line argument.
        if !url
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&b))
        {
            return Err(
                "web: URL contains characters outside RFC 3986; percent-encode them".into(),
            );
        }
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err("web: only http:// and https:// URLs are supported".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = out;
            Err("web is not available in the browser console".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let reply = match self.host_io.clone() {
                Some(host) => host
                    .lock()
                    .map_err(|_| "The console host failed".to_owned())?
                    .open_url(&url),
                None => None,
            };
            match reply {
                Some(Ok(())) => {}
                Some(Err(e)) => return Err(format!("web: {e}")),
                None => self.emit(out, format!("Open in a browser: {url}"))?,
            }
            if count == 1 {
                Ok(vec![Value::Matrix(Matrix::scalar(0.))])
            } else {
                Ok(Vec::new())
            }
        }
    }
}

/// Run `command` through the platform shell. Both pipes drain on helper
/// threads while this thread polls the child and cancellation; in particular,
/// never use `Child::wait`, because a child can close its pipes before it exits.
#[cfg(not(target_arch = "wasm32"))]
fn hostio_spawn(
    command: &str,
    cancel: &std::sync::atomic::AtomicBool,
    folder: Option<&std::path::Path>,
    variables: &std::collections::BTreeMap<String, String>,
) -> Result<(i32, Vec<u8>)> {
    use std::{
        process::{Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize},
            mpsc,
        },
        time::{Duration, Instant},
    };
    let mut shell = if cfg!(windows) {
        let mut shell = Command::new("cmd");
        shell.arg("/C").arg(command);
        shell
    } else {
        let mut shell = Command::new("sh");
        shell.arg("-c").arg(command);
        shell
    };
    if let Some(folder) = folder {
        shell.current_dir(folder);
    }
    for (name, value) in variables {
        // `setenv(name)` represents an engine-local unset, so do not let an
        // inherited process environment value leak into this child.
        if value.is_empty() {
            shell.env_remove(name);
        } else {
            shell.env(name, value);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // The shell and every descendant get a private group. A Stop can then
        // terminate the whole command tree, rather than leaving its children.
        shell.process_group(0);
    }
    shell
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = shell.spawn().map_err(|e| format!("system: {e}"))?;
    let used = Arc::new(AtomicUsize::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let (stdout_send, stdout_receive) = mpsc::channel();
    let (stderr_send, stderr_receive) = mpsc::channel();
    hostio_read_pipe(
        child.stdout.take().expect("stdout was piped"),
        stdout_send,
        used.clone(),
        overflow.clone(),
    );
    hostio_read_pipe(
        child.stderr.take().expect("stderr was piped"),
        stderr_send,
        used,
        overflow.clone(),
    );
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            hostio_stop_tree(&mut child);
            // Reap promptly when possible without a blocking wait. The group
            // kill above makes the normal path take one polling interval.
            let deadline = Instant::now() + Duration::from_millis(250);
            while Instant::now() < deadline {
                if child
                    .try_wait()
                    .map_err(|e| format!("system: {e}"))?
                    .is_some()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            return Err("Calculation cancelled".into());
        }
        if status.is_none() {
            status = child.try_wait().map_err(|e| format!("system: {e}"))?;
        }
        if stdout.is_none() {
            match stdout_receive.try_recv() {
                Ok(Ok(bytes)) => stdout = Some(bytes),
                Ok(Err(error)) => return Err(error),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("system: the stdout reader failed".into());
                }
            }
        }
        if stderr.is_none() {
            match stderr_receive.try_recv() {
                Ok(Ok(bytes)) => stderr = Some(bytes),
                Ok(Err(error)) => return Err(error),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("system: the stderr reader failed".into());
                }
            }
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            let status = status.take().expect("checked above");
            let mut stdout = stdout.take().expect("checked above");
            let stderr = stderr.take().expect("checked above");
            if overflow.load(Ordering::Relaxed) {
                return Err("system output exceeds 4 MiB.".into());
            }
            // Separate pipes cannot retain a total ordering. Keeping stdout
            // first matches the common shell contract and preserves both.
            stdout.extend_from_slice(&stderr);
            return Ok((hostio_status(status), stdout));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Drain one command pipe without allowing the two pipes together to retain
/// more than `MAX_SYSTEM_OUTPUT` bytes. Reading continues after the cap so a
/// verbose child cannot block on a full pipe.
#[cfg(not(target_arch = "wasm32"))]
fn hostio_read_pipe<R: std::io::Read + Send + 'static>(
    mut reader: R,
    send: std::sync::mpsc::Sender<Result<Vec<u8>>>,
    used: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        let result = loop {
            match reader.read(&mut chunk) {
                Ok(0) => break Ok(bytes),
                Ok(n) => {
                    if used
                        .try_update(Ordering::Relaxed, Ordering::Relaxed, |at| {
                            at.checked_add(n).filter(|next| *next <= MAX_SYSTEM_OUTPUT)
                        })
                        .is_ok()
                    {
                        bytes.extend_from_slice(&chunk[..n]);
                    } else {
                        overflow.store(true, Ordering::Relaxed);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => break Err(format!("system: {error}")),
            }
        };
        let _ = send.send(result);
    });
}

/// End a shell and every child it started. This uses only the PID just
/// returned by our own `Command::spawn`.
#[cfg(all(not(target_arch = "wasm32"), unix))]
fn hostio_stop_tree(child: &mut std::process::Child) {
    use std::ffi::c_int;
    unsafe extern "C" {
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }
    let pid = child.id() as c_int;
    if pid > 0 {
        // SAFETY: `-pid` is the private process group established above, and
        // SIGKILL needs no pointed-to memory or retained Rust references.
        unsafe {
            let _ = kill(-pid, 9);
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn hostio_stop_tree(child: &mut std::process::Child) {
    // taskkill /T follows the owned shell's descendants; /F handles a shell
    // that is blocked in a child command. Waiting for this tiny helper makes
    // the cancellation result meaningful when it returns to the UI.
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .status();
}

#[cfg(not(target_arch = "wasm32"))]
fn hostio_status(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host like the desktop Command Window: it answers input but keeps the
    /// default `request_exit`, which refuses.
    struct WindowHost;
    impl HostIo for WindowHost {
        fn read_line(&mut self, _prompt: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn hosts_without_a_process_refuse_exit_without_terminating() {
        // No host is the browser's default; WindowHost stands for the desktop.
        for host in [false, true] {
            let mut engine = Engine::default();
            if host {
                engine.set_host_io(Some(Box::new(WindowHost)));
            }
            for source in ["exit", "quit", "exit(3)", "quit(0)"] {
                let error = engine.execute(source).unwrap_err();
                assert!(
                    error.contains("is not supported in this host"),
                    "{source}: {error}"
                );
                assert_eq!(engine.take_exit_request(), None);
            }
            // The engine keeps working, and catch sees the refusal.
            let out = engine
                .execute("try, exit(2), catch e, disp('refused'), end, disp('still here')")
                .unwrap();
            assert_eq!(out.output, ["refused", "still here"]);
            // An invalid status is reported before the host is asked.
            let error = engine.execute("exit(256)").unwrap_err();
            assert!(error.contains("0 to 255"), "{error}");
        }
    }

    /// A host that owns its process, like the CLI.
    struct ProcessHost(Arc<Mutex<Vec<u8>>>);
    impl HostIo for ProcessHost {
        fn read_line(&mut self, _prompt: &str) -> Option<String> {
            None
        }
        fn request_exit(&mut self, status: u8) -> bool {
            self.0.lock().unwrap().push(status);
            true
        }
    }

    #[test]
    fn an_accepted_exit_stops_the_run_and_keeps_its_output() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut engine = Engine::default();
        engine.set_host_io(Some(Box::new(ProcessHost(asked.clone()))));
        let out = engine
            .execute("disp('a'); try, quit(9), catch, disp('caught'), end, disp('b')")
            .unwrap();
        assert_eq!(out.output, ["a"]);
        assert_eq!(*asked.lock().unwrap(), [9]);
        assert_eq!(engine.take_exit_request(), Some(9));
        assert_eq!(engine.take_exit_request(), None);
        // The cancel flag the exit raised does not leak into the next run.
        assert_eq!(engine.execute("disp('c')").unwrap().output, ["c"]);
    }
}
