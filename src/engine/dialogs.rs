//! Dialog builtins: `uigetfile`, `uiputfile`, `uigetdir`, `inputdlg`,
//! `questdlg`, `msgbox` (with `errordlg`, `warndlg`, `helpdlg`), `waitbar`
//! and `uiwait`. The engine parses the MATLAB arguments into a
//! [`DialogRequest`] and asks its host ([`super::HostIo::dialog`]); the desktop
//! app shows the window, tests answer from a script. Hosts without windows
//! (`-batch`, the terminal, the browser) answer [`DialogReply::Unsupported`]
//! and blocking calls fail at once instead of waiting for input.
//!
//! `msgbox` and `waitbar` return numeric handles in the desktop host and
//! hidden figure handles in headless hosts. `close(h)`, `delete(h)` and
//! `uiwait(h)` accept either kind.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix,
    ObjectIdentity, Result, syntax::Expr,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

/// One file-type choice of a file picker: `*.csv` style patterns and a label.
#[derive(Clone, Debug, PartialEq)]
pub struct FileFilter {
    pub patterns: Vec<String>,
    pub label: String,
}

/// The icon of a message box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageIcon {
    None,
    Error,
    Warn,
    Help,
}

/// What a script asks its host to show.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogRequest {
    /// `uigetfile`: pick one (or with `multiple`, several) existing files.
    OpenFile {
        title: String,
        filters: Vec<FileFilter>,
        folder: Option<PathBuf>,
        name: String,
        multiple: bool,
    },
    /// `uiputfile`: choose a file name to write.
    SaveFile {
        title: String,
        filters: Vec<FileFilter>,
        folder: Option<PathBuf>,
        name: String,
    },
    /// `uigetdir`: pick a folder.
    Folder {
        title: String,
        folder: Option<PathBuf>,
    },
    /// `inputdlg`: one text field per prompt; `lines` is each field's height.
    Input {
        title: String,
        prompts: Vec<String>,
        defaults: Vec<String>,
        lines: Vec<usize>,
    },
    /// `questdlg`: a question with up to three buttons.
    Question {
        title: String,
        text: Vec<String>,
        buttons: Vec<String>,
        default: String,
    },
    /// `listdlg`: pick one or more items from a list.
    List {
        title: String,
        prompts: Vec<String>,
        items: Vec<String>,
        multiple: bool,
        initial: Vec<usize>,
        ok_label: String,
        cancel_label: String,
        list_size: [f64; 2],
    },
    /// `msgbox` family: show a message window and return at once.
    Message {
        id: u32,
        title: String,
        text: Vec<String>,
        icon: MessageIcon,
    },
    /// `waitbar`: create the progress window `id` or update it.
    Progress {
        id: u32,
        title: String,
        text: Vec<String>,
        fraction: f64,
    },
    /// `close(h)` / `delete(h)` of a message or progress window.
    Close { id: u32 },
    /// `uiwait(h)`: block until window `id` is closed (or `timeout` seconds pass).
    WaitClosed { id: u32, timeout: Option<f64> },
    /// `uialert(fig, ...)`: non-blocking modal alert sheet over `parent_fig`.
    UiAlert {
        id: u32,
        parent_fig: usize,
        title: String,
        text: Vec<String>,
        icon: MessageIcon,
    },
    /// `uiconfirm(fig, ...)`: modal confirmation sheet over `parent_fig`.
    UiConfirm {
        parent_fig: usize,
        title: String,
        text: Vec<String>,
        options: Vec<String>,
        default: String,
        icon: MessageIcon,
    },
}

impl DialogRequest {
    /// Whether the script waits for the user's answer.
    pub fn blocks(&self) -> bool {
        !matches!(
            self,
            Self::Message { .. }
                | Self::Progress { .. }
                | Self::Close { .. }
                | Self::UiAlert { .. }
        )
    }
}

/// The host's answer to a [`DialogRequest`].
#[derive(Clone, Debug, PartialEq)]
pub enum DialogReply {
    /// This host cannot show windows (`-batch`, terminal, browser, no host).
    Unsupported,
    /// The user cancelled or closed the dialog.
    Cancelled,
    /// Files picked in `folder`. `filter` is the zero-based chosen filter.
    /// `usize::MAX` means no filter matched the name; the engine reports 0.
    Files {
        folder: PathBuf,
        names: Vec<String>,
        filter: usize,
    },
    Folder(PathBuf),
    /// `inputdlg` answers, one per prompt.
    Answers(Vec<String>),
    /// The `questdlg` button text.
    Button(String),
    /// `listdlg` chosen zero-based item indices.
    Selection(Vec<usize>),
    /// A window was shown, updated or closed.
    Done,
    /// The host could not show the dialog; the text says why (for example a
    /// missing Linux file-picker package).
    Failed(String),
}

/// Handles of message and progress windows this engine created.
#[derive(Clone, Debug, Default)]
pub(super) struct DialogState {
    next: u32,
    handles: BTreeSet<u32>,
    /// waitbar messages, kept between updates.
    progress: BTreeMap<u32, Vec<String>>,
    // Hidden dialog figures have no plot document or active axes.
    headless: BTreeMap<u32, HeadlessDialog>,
    headless_ids: BTreeMap<usize, u32>,
    desktop: BTreeMap<u32, HeadlessDialog>,
}

#[derive(Clone, Debug)]
struct HeadlessDialog {
    identity: ObjectIdentity,
    name: String,
    tag: String,
    properties: BTreeMap<String, Value>,
}

impl DialogState {
    pub(super) fn headless_id(&self, handle: &GraphicsHandle) -> Option<u32> {
        if handle.kind() != GraphicsKind::Figure {
            return None;
        }
        self.headless_ids
            .get(&handle.figure.recovery_key())
            .copied()
    }

    pub(super) fn headless_property(
        &self,
        id: u32,
        property: &Value,
        cancel: &AtomicBool,
    ) -> Option<Result<Value>> {
        let (dialog, visible) = self
            .headless
            .get(&id)
            .map(|dialog| (dialog, "off"))
            .or_else(|| self.desktop.get(&id).map(|dialog| (dialog, "on")))?;
        Some(dlg_get_property(dialog, visible, property, cancel))
    }

    pub(super) fn hidden_handles(&self) -> Vec<GraphicsHandle> {
        self.headless
            .values()
            .chain(self.desktop.values())
            .map(|d| GraphicsHandle::figure(&d.identity))
            .collect()
    }

    pub(super) fn close_headless(&mut self, id: u32) -> bool {
        let Some(dialog) = self.headless.remove(&id) else {
            return false;
        };
        self.headless_ids.remove(&dialog.identity.recovery_key());
        self.progress.remove(&id);
        true
    }

    pub(super) fn close_all_headless(&mut self) {
        for (id, dialog) in &self.headless {
            self.progress.remove(id);
            self.headless_ids.remove(&dialog.identity.recovery_key());
        }
        self.headless.clear();
    }
}

fn dlg_get_property(
    dialog: &HeadlessDialog,
    visible: &str,
    property: &Value,
    cancel: &AtomicBool,
) -> Result<Value> {
    let name =
        dlg_property_text(property, cancel)?.ok_or("Graphics property requires character text")?;
    match name.to_ascii_lowercase().as_str() {
            "type" => Ok(text("figure")),
            "name" => Ok(text(dialog.name.clone())),
            "tag" => Ok(text(dialog.tag.clone())),
            "visible" => Ok(text(visible)),
            "children" | "currentaxes" | "number" => {
                Matrix::from_column_vec(0, 0, Vec::new()).map(Value::Matrix)
            }
            key if dialog.properties.contains_key(key) => Ok(dialog.properties[key].clone()),
            "handlevisibility" => Ok(text("callback")),
            _ => Err("Supported headless dialog properties: Type, Name, Tag, Visible, Children, CurrentAxes and Number".into()),
    }
}

/// First dialog handle; plot figures are numbered 1..=1000000.
const FIRST_HANDLE: u32 = 1_000_001;
const NAMES: &[&str] = &[
    "uigetfile",
    "uiputfile",
    "uigetdir",
    "inputdlg",
    "questdlg",
    "listdlg",
    "msgbox",
    "errordlg",
    "warndlg",
    "helpdlg",
    "waitbar",
    "uiwait",
    "waitfor",
];
const MAX_PROMPTS: usize = 64;
const MAX_LINES: usize = 64;

pub(super) fn dlg_handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

fn zero() -> Value {
    Value::Matrix(Matrix::scalar(0.))
}

/// MATLAB returns folders from file pickers with a trailing separator.
fn folder_text(folder: &std::path::Path) -> Result<String> {
    let mut text = super::env_files::envx_reply_path(folder)?;
    if !text.ends_with(std::path::MAIN_SEPARATOR) && !text.ends_with('/') {
        text.push(std::path::MAIN_SEPARATOR);
    }
    Ok(text)
}

impl Engine {
    fn dlg_one_text(&self, value: &Value, what: &str) -> Result<String> {
        match super::table_fns::text_list(self, value, what)? {
            Some(list) if list.len() == 1 => Ok(list.into_iter().next().unwrap_or_default()),
            Some(list) if list.is_empty() => Ok(String::new()),
            _ => Err(format!(
                "{what} must be a character vector or string scalar."
            )),
        }
    }

    /// Text that may span lines: char row or matrix, cellstr or string array.
    fn dlg_lines(&self, value: &Value, what: &str) -> Result<Vec<String>> {
        if let Value::Matrix(m) = value
            && m.is_empty()
        {
            return Ok(Vec::new());
        }
        let lines = super::table_fns::text_list(self, value, what)?.ok_or_else(|| {
            format!(
                "{what} must be a character array, cell array of character vectors or string array."
            )
        })?;
        if lines.len() > MAX_LINES * 16 {
            return Err(format!("{what} has too many lines."));
        }
        Ok(lines
            .iter()
            .flat_map(|line| line.split('\n').map(str::to_owned).collect::<Vec<_>>())
            .collect())
    }

    /// A handle created by msgbox or waitbar.
    pub(super) fn dlg_handle(&self, value: &Value) -> Option<u32> {
        if let Value::Container(Array::Graphics(handle)) = value {
            return self.dialogs.headless_id(handle.scalar_handle().ok()?);
        }
        let Value::Matrix(m) = value else {
            return None;
        };
        if m.len() != 1 {
            return None;
        }
        let n = m.data()[0];
        if n.fract() != 0. || n < f64::from(FIRST_HANDLE) || n > f64::from(u32::MAX) {
            return None;
        }
        let id = n as u32;
        self.dialogs.handles.contains(&id).then_some(id)
    }

    pub(super) fn dlg_new_handle(&mut self) -> Result<u32> {
        let id = FIRST_HANDLE
            .checked_add(self.dialogs.next)
            .ok_or("Too many dialog windows")?;
        self.dialogs.next += 1;
        Ok(id)
    }

    /// Route a [`DialogRequest`] to the host without turning `Unsupported` into
    /// an error (used by `uialert`, `uiconfirm`, and `uiprogressdlg` so `-batch`
    /// can apply non-blocking defaults while desktop hosts answer interactively).
    pub(super) fn dlg_ask_raw(
        &mut self,
        request: DialogRequest,
        out: &mut Execution,
    ) -> Result<DialogReply> {
        self.check_cancel()?;
        let host = self.host_io.clone();
        if request.blocks() && host.is_some() {
            self.hostio_show_output(out)?;
        }
        let reply = if let Some(host) = host {
            host.lock()
                .map_err(|_| "The console host failed".to_owned())?
                .dialog(request)
        } else {
            DialogReply::Unsupported
        };
        self.check_cancel()?;
        Ok(reply)
    }

    fn dlg_ask(
        &mut self,
        name: &str,
        request: DialogRequest,
        out: &mut Execution,
    ) -> Result<DialogReply> {
        self.check_cancel()?;
        let host = self.host_io.clone();
        if request.blocks() && host.is_some() {
            // Output printed before the dialog appears before it.
            self.hostio_show_output(out)?;
        }
        let allows_headless = matches!(
            request,
            DialogRequest::Message { .. } | DialogRequest::Progress { .. }
        );
        let reply = if let Some(host) = host {
            host.lock()
                .map_err(|_| "The console host failed".to_owned())?
                .dialog(request)
        } else {
            DialogReply::Unsupported
        };
        self.check_cancel()?;
        match reply {
            DialogReply::Unsupported if !allows_headless => Err(unsupported(name)),
            DialogReply::Failed(why) => Err(format!("{name}: {why}")),
            reply => Ok(reply),
        }
    }

    /// `[a, b] = uigetfile(...)` and the other multi-output forms.
    pub(super) fn dlg_eval_outputs(
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
        if !dlg_handles(name) || self.contains_array(name) {
            return Ok(None);
        }
        let args = super::string_class::arguments(self, name, expressions, out)?;
        if let Some(function) = self.resolve_function(name)? {
            return self
                .invoke_function(function, args, count, None, out)
                .map(Some);
        }
        let values = self.dlg_run(name, &args, count, out)?;
        if values.len() < count {
            return Err("Too many output arguments.".into());
        }
        Ok(Some(values))
    }

    /// Statement form: the pickers and question dialogs set `ans`; msgbox,
    /// waitbar and uiwait return nothing.
    pub(super) fn dlg_statement(
        &mut self,
        name: &str,
        expressions: &[Expr],
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        if name == "close"
            && expressions.len() == 2
            && matches!(&expressions[0], Expr::String(s) if s.eq_ignore_ascii_case("all"))
        {
            let args = super::string_class::arguments(self, name, expressions, out)?;
            if matches!(&args[..], [Value::Text(a), Value::Text(b)] if a.eq_ignore_ascii_case("all") && (b.eq_ignore_ascii_case("hidden") || b.eq_ignore_ascii_case("force")))
            {
                let ids: Vec<_> = self.dialogs.handles.iter().copied().collect();
                for id in ids {
                    self.dlg_close(name, id, out)?;
                }
            }
            self.call(name, args, out)?;
            return Ok(true);
        }
        if !dlg_handles(name) {
            return Ok(false);
        }
        let args = super::string_class::arguments(self, name, expressions, out)?;
        let mut values = self.dlg_run(name, &args, 0, out)?;
        if !values.is_empty() {
            let value = values.remove(0).array(&self.cancel)?;
            self.assign_workspace_array("ans", value, quiet, out)?;
        }
        Ok(true)
    }

    /// One-output call form, plus `close(h)` / `delete(h)` of a dialog handle.
    pub(super) fn dlg_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if matches!(name, "get" | "set")
            && let Some(id) = args.first().and_then(|v| self.dlg_handle(v))
        {
            if name == "get"
                && args.len() == 2
                && let Some(result) = self.dialogs.headless_property(id, &args[1], &self.cancel)
            {
                return result.map(Some);
            }
            if name == "set"
                && (self.dialogs.headless.contains_key(&id)
                    || self.dialogs.desktop.contains_key(&id))
            {
                dlg_set_properties(self, id, &args[1..])?;
                return Ok(Some(Value::Void));
            }
        }
        if name == "ishandle"
            && let [Value::Matrix(m)] = args
        {
            let ids = self.figure_ids();
            let values = m
                .data()
                .iter()
                .map(|&v| {
                    u8::from(
                        v == 0.
                            || (v.is_finite()
                                && v.fract() == 0.
                                && v >= 1.
                                && v <= f64::from(u32::MAX)
                                && (ids.contains(&(v as u32))
                                    || self.dialogs.handles.contains(&(v as u32)))),
                    )
                })
                .collect();
            return Ok(Some(Value::Logical(crate::LogicalMatrix::from_column_vec(
                m.rows(),
                m.cols(),
                values,
                &self.cancel,
            )?)));
        }
        if name == "isvalid"
            && let [value] = args
            && matches!(value, Value::Container(Array::Graphics(_)))
        {
            return self.graphics_handle_call("isgraphics", args, out);
        }
        if name == "close"
            && let [value, Value::Text(option)] = args
            && option.eq_ignore_ascii_case("force")
            && let Some(id) = self.dlg_handle(value)
        {
            self.dlg_close(name, id, out)?;
            return Ok(Some(Value::Void));
        }
        if matches!(name, "close" | "delete")
            && let [value] = args
            && let Some(id) = self.dlg_handle(value)
        {
            self.dlg_close(name, id, out)?;
            return Ok(Some(Value::Void));
        }
        if !dlg_handles(name) {
            return Ok(None);
        }
        let mut values = self.dlg_run(name, args, 1, out)?;
        if values.is_empty() {
            return Ok(Some(Value::Void));
        }
        Ok(Some(values.remove(0)))
    }

    fn dlg_close(&mut self, name: &str, id: u32, out: &mut Execution) -> Result<()> {
        self.check_cancel()?;
        if !self.dialogs.close_headless(id) {
            self.dlg_ask(name, DialogRequest::Close { id }, out)?;
            self.dialogs.handles.remove(&id);
            if let Some(dialog) = self.dialogs.desktop.remove(&id) {
                self.dialogs
                    .headless_ids
                    .remove(&dialog.identity.recovery_key());
            }
            self.dialogs.progress.remove(&id);
        }
        Ok(())
    }

    fn dlg_run(
        &mut self,
        name: &str,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = match name {
            "uigetfile" | "uiputfile" => 3,
            "listdlg" => 2,
            "uiwait" | "waitfor" => 0,
            _ => 1,
        };
        if nargout > max {
            return Err("Too many output arguments.".into());
        }
        match name {
            "uigetfile" | "uiputfile" => self.dlg_file(name, args, nargout, out),
            "uigetdir" => self.dlg_folder(args, out),
            "inputdlg" => self.dlg_input(args, out),
            "questdlg" => self.dlg_question(args, out),
            "listdlg" => self.dlg_list(args, nargout, out),
            "msgbox" | "errordlg" | "warndlg" | "helpdlg" => {
                self.dlg_message(name, args, nargout, out)
            }
            "waitbar" => self.dlg_waitbar(args, nargout, out),
            "uiwait" | "waitfor" => self.dlg_uiwait(args, out),
            _ => unreachable!("dialog name {name}"),
        }
    }

    /// `uigetfile(filter, title, defname, 'MultiSelect', 'on')` and
    /// `uiputfile(filter, title, defname)`.
    fn dlg_file(
        &mut self,
        name: &str,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let open = name == "uigetfile";
        let mut positional = args;
        let mut multiple = false;
        if open {
            // 'MultiSelect', value ends the call after at least the filter;
            // the same text earlier is a positional title or file name.
            let at = args.len().saturating_sub(2);
            let option = args.len() >= 3
                && matches!(self.dlg_one_text(&args[at], "Option").as_deref(),
                    Ok(t) if t.eq_ignore_ascii_case("multiselect"));
            if option {
                let rest = &args[at..];
                multiple = match self
                    .dlg_one_text(&rest[1], "MultiSelect")?
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "on" => true,
                    "off" => false,
                    _ => return Err("MultiSelect must be 'on' or 'off'.".into()),
                };
                positional = &args[..at];
            }
        }
        if positional.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let (filters, mut folder, mut file_name) = match positional.first() {
            Some(value) => self.dlg_filters(value)?,
            None => (Vec::new(), None, String::new()),
        };
        let filters = if filters.is_empty() {
            default_filters()
        } else {
            filters
        };
        let title = match positional.get(1) {
            Some(value) => self.dlg_one_text(value, "The title")?,
            None => String::new(),
        };
        let title = if title.is_empty() {
            if open {
                "Select File to Open"
            } else {
                "Select File to Write"
            }
            .to_owned()
        } else {
            title
        };
        if let Some(value) = positional.get(2) {
            let default = self.dlg_one_text(value, "The default file name")?;
            let (dir, base) = split_path(&default);
            if let Some(dir) = dir {
                folder = Some(dir);
            }
            file_name = base;
        }
        // The picker starts in the engine's current folder (cd does not
        // change the process folder).
        let folder = match folder {
            Some(f) => Some(self.dlg_resolve(f)),
            None => self.current_folder().map(std::path::Path::to_owned),
        };
        let request = if open {
            DialogRequest::OpenFile {
                title,
                filters,
                folder,
                name: file_name,
                multiple,
            }
        } else {
            DialogRequest::SaveFile {
                title,
                filters,
                folder,
                name: file_name,
            }
        };
        let reply = self.dlg_ask(name, request, out)?;
        let mut values = match reply {
            DialogReply::Files {
                folder,
                names,
                filter,
            } if !names.is_empty() => {
                let file = if multiple && names.len() > 1 {
                    let slots = names
                        .iter()
                        .map(|n| {
                            Ok(Arc::new(Array::Character(CharacterRow::from_text(
                                n,
                                &self.cancel,
                            )?)))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Value::Container(Array::Cell(CellArray::from_shared(
                        1,
                        slots.len(),
                        slots,
                        &self.cancel,
                    )?))
                } else {
                    text(names[0].clone())
                };
                let index = if filter == usize::MAX {
                    0.
                } else {
                    filter as f64 + 1.
                };
                vec![
                    file,
                    text(folder_text(&folder).map_err(|e| format!("{name}: {e}"))?),
                    Value::Matrix(Matrix::scalar(index)),
                ]
            }
            DialogReply::Cancelled | DialogReply::Files { .. } => vec![zero(), zero(), zero()],
            other => return Err(format!("{name}: unexpected reply {other:?}")),
        };
        values.truncate(nargout.max(1));
        Ok(values)
    }

    /// A relative start folder is relative to the current folder.
    fn dlg_resolve(&self, folder: PathBuf) -> PathBuf {
        if folder.is_absolute() {
            return folder;
        }
        match self.current_folder() {
            Some(current) => current.join(folder),
            None => folder,
        }
    }

    /// The filter argument: `'*.csv'`, `'*.m;*.txt'`, a folder or path with a
    /// pattern, a file name (prefilled; its extension filters), or a cell
    /// `{'*.csv'; '*.txt'}` / `{'*.csv', 'CSV files'; ...}`. Returns the
    /// filters, a start folder and a file name.
    fn dlg_filters(&self, value: &Value) -> Result<(Vec<FileFilter>, Option<PathBuf>, String)> {
        if let Value::Container(Array::Cell(cell)) = value {
            let (rows, cols) = (cell.rows(), cell.cols());
            if cell.is_empty() {
                return Ok((Vec::new(), None, String::new()));
            }
            let texts = super::table_fns::text_list(self, value, "The filter")?
                .ok_or("The filter must be a cell array of character vectors.")?;
            // Two columns are patterns and labels, also as a single 1-by-2 row.
            let (count, labelled) = match (rows, cols) {
                (n, 2) => (n, true),
                (1, n) | (n, 1) => (n, false),
                _ => return Err("The filter cell must have one or two columns.".into()),
            };
            let filters = (0..count)
                .map(|r| {
                    // Column-major: column 2 starts at `count`.
                    let patterns = split_patterns(&texts[r]);
                    let label = if labelled {
                        texts[count + r].clone()
                    } else {
                        texts[r].clone()
                    };
                    FileFilter { patterns, label }
                })
                .collect();
            return Ok((filters, None, String::new()));
        }
        let spec = self.dlg_one_text(value, "The filter")?;
        if spec.is_empty() {
            return Ok((Vec::new(), None, String::new()));
        }
        let (folder, pattern) = split_path(&spec);
        // A folder name alone (or one that exists) lists that folder.
        let candidate = self.dlg_resolve(PathBuf::from(&spec));
        if candidate.is_dir() {
            return Ok((Vec::new(), Some(PathBuf::from(spec)), String::new()));
        }
        if pattern.is_empty() {
            return Ok((Vec::new(), folder, String::new()));
        }
        if !pattern.contains(['*', '?', ';']) {
            // A file name: prefilled, and its extension is the filter.
            let filters = match pattern.rsplit_once('.') {
                Some((_, ext)) if !ext.is_empty() => vec![FileFilter {
                    patterns: vec![format!("*.{ext}")],
                    label: format!("*.{ext}"),
                }],
                _ => Vec::new(),
            };
            return Ok((filters, folder, pattern));
        }
        let filter = FileFilter {
            patterns: split_patterns(&pattern),
            label: pattern,
        };
        Ok((vec![filter], folder, String::new()))
    }

    /// `uigetdir(start, title)`: folder text, or 0 when cancelled.
    fn dlg_folder(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let folder = match args.first() {
            Some(value) => {
                let start = self.dlg_one_text(value, "The start folder")?;
                (!start.is_empty()).then(|| self.dlg_resolve(PathBuf::from(start)))
            }
            None => self.current_folder().map(std::path::Path::to_owned),
        };
        let title = match args.get(1) {
            Some(value) => self.dlg_one_text(value, "The title")?,
            None => "Select a Folder".to_owned(),
        };
        match self.dlg_ask("uigetdir", DialogRequest::Folder { title, folder }, out)? {
            DialogReply::Folder(path) => {
                let path = super::env_files::envx_reply_path(&path)
                    .map_err(|e| format!("uigetdir: {e}"))?;
                Ok(vec![text(path)])
            }
            DialogReply::Cancelled => Ok(vec![zero()]),
            other => Err(format!("uigetdir: unexpected reply {other:?}")),
        }
    }

    /// `inputdlg(prompt, title, dims, definput, opts)`: a cell column of
    /// answers, or a 0x0 cell when cancelled.
    fn dlg_input(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.len() > 5 {
            return Err("Too many input arguments.".into());
        }
        let prompts = match args.first() {
            Some(value) => super::table_fns::text_list(self, value, "The prompt")?.ok_or(
                "The prompt must be a character vector or cell array of character vectors.",
            )?,
            None => vec!["Input:".to_owned()],
        };
        if prompts.is_empty() || prompts.len() > MAX_PROMPTS {
            return Err(format!("inputdlg needs 1 to {MAX_PROMPTS} prompts."));
        }
        let title = match args.get(1) {
            Some(value) => self.dlg_one_text(value, "The title")?,
            None => String::new(),
        };
        let lines = match args.get(2) {
            Some(value) => dims_lines(value, prompts.len(), &self.cancel)?,
            None => vec![1; prompts.len()],
        };
        let defaults = match args.get(3) {
            Some(value) => {
                let list = super::table_fns::text_list(self, value, "The default answers")?
                    .ok_or("The default answers must be a cell array of character vectors.")?;
                if list.is_empty() {
                    vec![String::new(); prompts.len()]
                } else if list.len() != prompts.len() {
                    return Err("The default answers need one entry per prompt.".into());
                } else {
                    list
                }
            }
            None => vec![String::new(); prompts.len()],
        };
        let request = DialogRequest::Input {
            title,
            prompts,
            defaults,
            lines: lines.clone(),
        };
        match self.dlg_ask("inputdlg", request, out)? {
            DialogReply::Answers(answers) => {
                let slots = answers
                    .iter()
                    .zip(lines.iter().chain(std::iter::repeat(&1)))
                    .map(|(answer, &height)| Ok(Arc::new(self.dlg_answer(answer, height)?)))
                    .collect::<Result<Vec<_>>>()?;
                let n = slots.len();
                Ok(vec![Value::Container(Array::Cell(CellArray::from_shared(
                    n,
                    1,
                    slots,
                    &self.cancel,
                )?))])
            }
            DialogReply::Cancelled => Ok(vec![Value::Container(Array::Cell(CellArray::empty(
                0,
                0,
                &self.cancel,
            )?))]),
            other => Err(format!("inputdlg: unexpected reply {other:?}")),
        }
    }

    /// A multi-line field's answer is a char matrix, one padded row per line.
    fn dlg_answer(&self, answer: &str, height: usize) -> Result<Array> {
        let rows: Vec<Vec<u16>> = if height > 1 {
            answer
                .split('\n')
                .map(|l| l.trim_end_matches('\r').encode_utf16().collect())
                .collect()
        } else {
            vec![answer.encode_utf16().collect()]
        };
        if rows.len() <= 1 {
            return Ok(Array::Character(CharacterRow::from_text(
                answer,
                &self.cancel,
            )?));
        }
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut units = vec![u16::from(b' '); rows.len() * cols];
        for (r, row) in rows.iter().enumerate() {
            for (c, &unit) in row.iter().enumerate() {
                units[c * rows.len() + r] = unit;
            }
        }
        Ok(Array::Character(CharacterRow::from_column_major(
            rows.len(),
            cols,
            units,
            &self.cancel,
        )?))
    }

    /// `questdlg(quest, title, defbtn)`, `questdlg(quest, title, btn1, btn2,
    /// defbtn)`, `questdlg(quest, title, btn1, btn2, btn3, defbtn)`.
    fn dlg_question(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 6 {
            return Err("Too many input arguments.".into());
        }
        let text_lines = self.dlg_lines(&args[0], "The question")?;
        let title = match args.get(1) {
            Some(value) => self.dlg_one_text(value, "The title")?,
            None => String::new(),
        };
        let extra = args.get(2..).unwrap_or(&[]);
        // A trailing options struct names the default in its Default field
        // (the first button when it has none).
        let (extra, from_struct) = match extra.last() {
            Some(Value::Container(Array::Struct(options))) => {
                let default = match options.field_index("Default") {
                    Some(field) if options.len() == 1 => match options.get(field, 0)?.as_ref() {
                        Array::Character(c) => Some(c.to_utf8(&self.cancel)?),
                        Array::Str(s) if s.len() == 1 => s.data()[0].as_deref().map(str::to_owned),
                        _ => return Err("The Default option must be text.".into()),
                    },
                    _ => None,
                };
                (&extra[..extra.len() - 1], Some(default))
            }
            _ => (extra, None),
        };
        let texts = extra
            .iter()
            .map(|v| self.dlg_one_text(v, "A button"))
            .collect::<Result<Vec<_>>>()?;
        let yes_no = || vec!["Yes".to_owned(), "No".to_owned(), "Cancel".to_owned()];
        let (buttons, default) = match (texts.len(), from_struct) {
            (0, None) => (yes_no(), "Yes".to_owned()),
            (1, None) => (yes_no(), texts[0].clone()),
            (n, None) => (texts[..n - 1].to_vec(), texts[n - 1].clone()),
            (n, Some(default)) => {
                let buttons = if n == 0 { yes_no() } else { texts.clone() };
                let default = default.unwrap_or_else(|| buttons[0].clone());
                (buttons, default)
            }
        };
        if buttons.len() > 3 {
            return Err("questdlg shows at most three buttons.".into());
        }
        let request = DialogRequest::Question {
            title,
            text: text_lines,
            buttons,
            default,
        };
        match self.dlg_ask("questdlg", request, out)? {
            DialogReply::Button(button) => Ok(vec![text(button)]),
            DialogReply::Cancelled => Ok(vec![text("")]),
            other => Err(format!("questdlg: unexpected reply {other:?}")),
        }
    }

    /// `listdlg('ListString', list, ...)`: returns `[indx, tf]`.
    fn dlg_list(
        &mut self,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        const MAX_LIST_ITEMS: usize = 10_000;
        if args.is_empty() || !args.len().is_multiple_of(2) {
            return Err("listdlg: name-value arguments must come in pairs.".into());
        }
        let mut items: Option<Vec<String>> = None;
        let mut multiple = true;
        let mut initial_raw: Option<Vec<f64>> = None;
        let mut prompts = Vec::new();
        let mut title = String::new();
        let mut ok_label = "OK".to_owned();
        let mut cancel_label = "Cancel".to_owned();
        let mut list_size = [160., 300.];

        for pair in args.as_chunks::<2>().0 {
            let raw = self.dlg_one_text(&pair[0], "Parameter name")?;
            match raw.to_ascii_lowercase().as_str() {
                "liststring" => {
                    let texts = super::table_fns::text_list(self, &pair[1], "ListString")?
                        .ok_or(
                            "ListString must be a character array, cell array of character vectors or string array.",
                        )?;
                    // The MathWorks ListString example treats embedded newlines as extra rows.
                    let list: Vec<_> = texts
                        .iter()
                        .flat_map(|item| item.split('\n'))
                        .take(MAX_LIST_ITEMS + 1)
                        .map(|row| row.strip_suffix('\r').unwrap_or(row).to_owned())
                        .collect();
                    if list.is_empty() {
                        return Err("listdlg: ListString must not be empty.".into());
                    }
                    if list.len() > MAX_LIST_ITEMS {
                        return Err(format!(
                            "listdlg: ListString exceeds {MAX_LIST_ITEMS} items."
                        ));
                    }
                    items = Some(list);
                }
                "selectionmode" => {
                    multiple = match self
                        .dlg_one_text(&pair[1], "SelectionMode")?
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "single" => false,
                        "multiple" => true,
                        _ => return Err("SelectionMode must be 'single' or 'multiple'.".into()),
                    };
                }
                "initialvalue" => {
                    let m = pair[1].clone().matrix(&self.cancel)?;
                    initial_raw = Some(m.data().to_vec());
                }
                "promptstring" => prompts = self.dlg_lines(&pair[1], "PromptString")?,
                "name" => title = self.dlg_one_text(&pair[1], "Name")?,
                "okstring" => ok_label = self.dlg_one_text(&pair[1], "OKString")?,
                "cancelstring" => cancel_label = self.dlg_one_text(&pair[1], "CancelString")?,
                "listsize" => {
                    let m = pair[1].clone().matrix(&self.cancel)?;
                    let [w, h] = match m.data() {
                        [w, h] if w.is_finite() && h.is_finite() && *w > 0. && *h > 0. => [*w, *h],
                        _ => return Err("ListSize must be a positive 2-element vector.".into()),
                    };
                    list_size = [w, h];
                }
                _ => return Err(format!("listdlg: unsupported parameter '{raw}'.")),
            }
        }
        let items = items.ok_or("listdlg: ListString is required.")?;
        let initial = match initial_raw {
            None => vec![0],
            Some(raw) => {
                let mut indices = Vec::with_capacity(raw.len());
                for v in raw {
                    if !v.is_finite() || v.fract() != 0. || v < 1. || v > items.len() as f64 {
                        return Err(format!(
                            "InitialValue entries must be integers from 1 to {}.",
                            items.len()
                        ));
                    }
                    let zero = v as usize - 1;
                    if !indices.contains(&zero) {
                        indices.push(zero);
                    }
                }
                if !multiple && indices.len() > 1 {
                    return Err(
                        "InitialValue must be a scalar when SelectionMode is 'single'.".into(),
                    );
                }
                indices
            }
        };
        let request = DialogRequest::List {
            title,
            prompts,
            items,
            multiple,
            initial,
            ok_label,
            cancel_label,
            list_size,
        };
        let (indices, accepted) = match self.dlg_ask("listdlg", request, out)? {
            DialogReply::Selection(indices) => (indices, 1.),
            DialogReply::Cancelled => (Vec::new(), 0.),
            other => return Err(format!("listdlg: unexpected reply {other:?}")),
        };
        let one_based: Vec<f64> = indices.into_iter().map(|i| (i + 1) as f64).collect();
        let rows = usize::from(!one_based.is_empty());
        let mut values = vec![
            Value::Matrix(Matrix::from_column_vec(rows, one_based.len(), one_based)?),
            Value::Matrix(Matrix::scalar(accepted)),
        ];
        values.truncate(nargout.max(1));
        Ok(values)
    }

    /// `msgbox(message, title, icon, ..., createmode)` and errordlg/warndlg/helpdlg.
    fn dlg_message(
        &mut self,
        name: &str,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.len() > 6 {
            return Err("Too many input arguments.".into());
        }
        let (default_text, default_title, mut icon) = match name {
            "errordlg" => (
                "This is the default error string.",
                "Error Dialog",
                MessageIcon::Error,
            ),
            "warndlg" => (
                "This is the default warning string.",
                "Warning Dialog",
                MessageIcon::Warn,
            ),
            "helpdlg" => (
                "This is the default help string.",
                "Help Dialog",
                MessageIcon::Help,
            ),
            _ => ("", "", MessageIcon::None),
        };
        let text_lines = match args.first() {
            Some(value) => self.dlg_lines(value, "The message")?,
            None => vec![default_text.to_owned()],
        };
        let title = match args.get(1) {
            Some(value) => self.dlg_one_text(value, "The title")?,
            None => default_title.to_owned(),
        };
        if name == "msgbox"
            && let Some(value) = args.get(2)
        {
            let choice = self.dlg_one_text(value, "The icon")?.to_ascii_lowercase();
            icon = match choice.as_str() {
                "none" | "custom" | "modal" | "non-modal" | "replace" => MessageIcon::None,
                "error" => MessageIcon::Error,
                "warn" => MessageIcon::Warn,
                "help" => MessageIcon::Help,
                _ => {
                    return Err(
                        "The icon must be 'none', 'error', 'help', 'warn' or 'custom'.".into(),
                    );
                }
            };
        }
        let id = self.dlg_new_handle()?;
        let request = DialogRequest::Message {
            id,
            title: title.clone(),
            text: text_lines,
            icon,
        };
        let reply = self.dlg_ask(name, request, out)?;
        if reply == DialogReply::Unsupported && nargout == 0 {
            return Ok(Vec::new());
        }
        let tag = format!("Msgbox_{title}");
        let dialog = dlg_figure_state(self, title, tag)?;
        if reply == DialogReply::Unsupported {
            return dlg_create_hidden(self, id, dialog);
        }
        self.dialogs
            .headless_ids
            .insert(dialog.identity.recovery_key(), id);
        self.dialogs.desktop.insert(id, dialog);
        self.dialogs.handles.insert(id);
        Ok(handle_output(id, nargout))
    }

    /// `h = waitbar(x, msg, 'Name', title)`, `waitbar(x, h)`, `waitbar(x, h, msg)`.
    fn dlg_waitbar(
        &mut self,
        args: &[Value],
        nargout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let Some(first) = args.first() else {
            return Err("Not enough input arguments.".into());
        };
        let fraction = match first {
            Value::Matrix(m) if m.len() == 1 && m.data()[0].is_finite() => {
                m.data()[0].clamp(0., 1.)
            }
            Value::Logical(m) if m.len() == 1 => {
                Value::Logical(m.clone()).matrix(&self.cancel)?.data()[0]
            }
            _ => return Err("The waitbar fraction must be a finite real scalar.".into()),
        };
        let existing = args.get(1).and_then(|v| self.dlg_handle(v));
        if existing.is_some_and(|id| !self.dialogs.progress.contains_key(&id)) {
            return Err("Invalid waitbar handle: expected a waitbar, not another dialog.".into());
        }
        let rest = if existing.is_some() {
            &args[2..]
        } else {
            &args[1..]
        };
        if args
            .get(1)
            .is_some_and(|v| matches!(v, Value::Matrix(m) if m.len() == 1))
            && existing.is_none()
        {
            return Err("Invalid waitbar handle: create it with h = waitbar(x, message).".into());
        }
        let (message, options) = match rest.first() {
            Some(value) if rest.len() % 2 == 1 => {
                (Some(self.dlg_lines(value, "The message")?), &rest[1..])
            }
            _ => (None, rest),
        };
        let mut dialog = match existing {
            Some(id) => self
                .dialogs
                .headless
                .get(&id)
                .or_else(|| self.dialogs.desktop.get(&id))
                .ok_or("Invalid waitbar handle")?
                .clone(),
            None => dlg_figure_state(self, String::new(), "TMWWaitbar".into())?,
        };
        dlg_apply_properties(self, &mut dialog, options, true)?;

        let id = match existing {
            Some(id) => id,
            None => self.dlg_new_handle()?,
        };
        if let Some(current) = self.dialogs.headless.get_mut(&id) {
            *current = dialog;
            if let Some(message) = message {
                self.dialogs.progress.insert(id, message);
            }
            return Ok(if nargout == 0 {
                Vec::new()
            } else {
                vec![args[1].clone()]
            });
        }
        let text_lines =
            message.unwrap_or_else(|| self.dialogs.progress.get(&id).cloned().unwrap_or_default());
        let request = DialogRequest::Progress {
            id,
            title: dialog.name.clone(),
            text: text_lines.clone(),
            fraction,
        };
        let reply = self.dlg_ask("waitbar", request, out)?;
        if reply == DialogReply::Unsupported && existing.is_none() {
            // With no output there is no way to refer to the headless waitbar.
            if nargout == 0 {
                return Ok(Vec::new());
            }
            let result = dlg_create_hidden(self, id, dialog)?;
            self.dialogs.progress.insert(id, text_lines);
            return Ok(result);
        }
        if existing.is_none() {
            self.dialogs
                .headless_ids
                .insert(dialog.identity.recovery_key(), id);
            self.dialogs.handles.insert(id);
        }
        self.dialogs.desktop.insert(id, dialog);
        self.dialogs.progress.insert(id, text_lines);
        Ok(handle_output(id, nargout))
    }

    /// `uiwait(h)` / `uiwait(h, timeout)` for a msgbox, waitbar, or uifigure handle.
    fn dlg_uiwait(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if super::ui_components::uiwait(self, args, out)?.is_some() {
            return Ok(Vec::new());
        }
        let id = match args.first() {
            Some(value) => self.dlg_handle(value),
            None => None,
        }
        .ok_or("uiwait needs a msgbox, errordlg, warndlg, helpdlg or waitbar handle.")?;
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let timeout = match args.get(1) {
            Some(Value::Matrix(m)) if m.len() == 1 && m.data()[0] >= 0. => Some(m.data()[0]),
            Some(_) => return Err("The timeout must be a nonnegative scalar.".into()),
            None => None,
        };
        if !self.dialogs.headless.contains_key(&id) {
            self.dlg_ask("uiwait", DialogRequest::WaitClosed { id, timeout }, out)?;
        }
        Ok(Vec::new())
    }
}

fn dlg_create_hidden(engine: &mut Engine, id: u32, dialog: HeadlessDialog) -> Result<Vec<Value>> {
    if engine.dialogs.headless.len() >= crate::script_containers::MAX_CELL_SLOTS {
        return Err("Too many open headless dialogs; close unused dialog handles".into());
    }
    let handle = GraphicsArray::scalar(GraphicsHandle::figure(&dialog.identity), &engine.cancel)?;
    engine.check_cancel()?;
    engine
        .dialogs
        .headless_ids
        .insert(dialog.identity.recovery_key(), id);
    engine.dialogs.headless.insert(id, dialog);
    Ok(vec![Value::Container(handle.into())])
}

fn dlg_figure_state(engine: &Engine, name: String, tag: String) -> Result<HeadlessDialog> {
    let mut properties = BTreeMap::new();
    for prop in ["Color", "Theme", "ThemeMode", "ColorMode", "InvertHardcopy"] {
        properties.insert(
            prop.to_ascii_lowercase(),
            engine.get_root_property(&[Value::Text(format!("DefaultFigure{prop}"))])?,
        );
    }
    Ok(HeadlessDialog {
        identity: ObjectIdentity::default(),
        name,
        tag,
        properties,
    })
}

fn dlg_property_text<'a>(value: &'a Value, cancel: &AtomicBool) -> Result<Option<Cow<'a, str>>> {
    Ok(match value {
        Value::Text(s) | Value::QuotedString(s) => Some(Cow::Borrowed(s)),
        Value::Container(Array::Character(c)) if c.is_text_row() => {
            Some(Cow::Owned(c.to_utf8(cancel)?))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(Cow::Borrowed)
        }
        _ => None,
    })
}

fn dlg_set_properties(engine: &mut Engine, id: u32, args: &[Value]) -> Result<()> {
    if args.is_empty() || args.len() > 32 || !args.len().is_multiple_of(2) {
        return Err("set requires 1..16 property/value pairs".into());
    }
    let mut candidate = engine
        .dialogs
        .headless
        .get(&id)
        .or_else(|| engine.dialogs.desktop.get(&id))
        .ok_or("Invalid dialog handle")?
        .clone();
    dlg_apply_properties(engine, &mut candidate, args, false)?;
    if let Some(slot) = engine.dialogs.headless.get_mut(&id) {
        *slot = candidate;
    } else {
        engine.dialogs.desktop.insert(id, candidate);
    }
    Ok(())
}

fn dlg_apply_properties(
    engine: &Engine,
    dialog: &mut HeadlessDialog,
    args: &[Value],
    waitbar: bool,
) -> Result<()> {
    for [name, value] in args.as_chunks::<2>().0 {
        let raw = if waitbar {
            Cow::Owned(engine.dlg_one_text(name, "A property name")?)
        } else {
            dlg_property_text(name, &engine.cancel)?
                .ok_or("Graphics property requires character text")?
        };
        let key = raw.to_ascii_lowercase();
        match key.as_str() {
            "name" if waitbar => dialog.name = engine.dlg_one_text(value, "The Name")?,
            "name" | "tag" => {
                let value = dlg_property_text(value, &engine.cancel)?
                    .ok_or("Dialog property requires text")?
                    .into_owned();
                if key == "name" {
                    dialog.name = value;
                } else {
                    dialog.tag = value;
                }
            }
            _ => {
                let canon = match key.as_str() {
                    "color" => "Color",
                    "theme" => "Theme",
                    "thememode" => "ThemeMode",
                    "colormode" => "ColorMode",
                    "inverthardcopy" => "InvertHardcopy",
                    // Waitbar has historically accepted extra figure options.
                    _ if waitbar => continue,
                    _ => return Err(format!("Unsupported dialog property '{raw}'")),
                };
                let value = dlg_property_text(value, &engine.cancel)?
                    .map(|s| text(s.into_owned()))
                    .unwrap_or_else(|| value.clone());
                super::graphics_root::validate_default_value(
                    super::graphics_root::DefaultObjectType::Figure,
                    canon,
                    &value,
                    &engine.cancel,
                )?;
                dialog.properties.insert(key, value);
            }
        }
    }
    Ok(())
}

fn handle_output(id: u32, nargout: usize) -> Vec<Value> {
    if nargout == 0 {
        Vec::new()
    } else {
        vec![Value::Matrix(Matrix::scalar(f64::from(id)))]
    }
}

fn unsupported(name: &str) -> String {
    format!(
        "{name}: dialogs need the desktop app (they cannot open in -batch mode, the terminal or the browser)."
    )
}

fn default_filters() -> Vec<FileFilter> {
    vec![FileFilter {
        patterns: vec!["*.*".to_owned()],
        label: "All Files (*.*)".to_owned(),
    }]
}

fn split_patterns(spec: &str) -> Vec<String> {
    spec.split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `C:\data\*.csv` -> (`C:\data`, `*.csv`); a trailing separator leaves no name.
fn split_path(spec: &str) -> (Option<PathBuf>, String) {
    match spec.rfind(['/', '\\']) {
        Some(at) => {
            // Keep the separator of a root (`/`, `C:\`).
            let root = at == 0 || spec[..at].ends_with(':');
            let folder = if root { &spec[..=at] } else { &spec[..at] };
            (Some(PathBuf::from(folder)), spec[at + 1..].to_owned())
        }
        None => (None, spec.to_owned()),
    }
}

/// inputdlg dims: a scalar for every field, one row per prompt, or
/// `[lines width]` rows; only the line counts matter here.
fn dims_lines(
    value: &Value,
    prompts: usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<usize>> {
    let m = value.clone().matrix(cancel)?;
    let count = |x: f64| -> Result<usize> {
        if !x.is_finite() || x < 1. || x.fract() != 0. {
            return Err("The dimensions must be positive whole numbers.".into());
        }
        Ok((x as usize).min(MAX_LINES))
    };
    if m.is_empty() {
        return Ok(vec![1; prompts]);
    }
    if m.len() == 1 || (m.rows() == 1 && m.cols() == 2) {
        return Ok(vec![count(m.data()[0])?; prompts]);
    }
    // A row or column with one height per prompt ([1 30] stays [lines width]).
    if m.len() == prompts && (m.rows() == 1 || m.cols() == 1) {
        return (0..prompts).map(|r| count(m.data()[r])).collect();
    }
    if m.rows() != prompts || m.cols() > 2 {
        return Err("The dimensions need one row per prompt.".into());
    }
    (0..prompts).map(|r| count(m.data()[r])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_into_folder_and_name() {
        assert_eq!(split_path("*.csv"), (None, "*.csv".to_owned()));
        assert_eq!(
            split_path("data/*.csv"),
            (Some("data".into()), "*.csv".to_owned())
        );
        assert_eq!(
            split_path(r"C:\data\out.txt"),
            (Some(r"C:\data".into()), "out.txt".to_owned())
        );
        assert_eq!(
            split_path("/out.txt"),
            (Some("/".into()), "out.txt".to_owned())
        );
    }

    #[test]
    fn patterns_split_on_semicolons() {
        assert_eq!(split_patterns("*.m; *.txt;"), vec!["*.m", "*.txt"]);
    }
}
