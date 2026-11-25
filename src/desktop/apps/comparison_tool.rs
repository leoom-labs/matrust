//! Clean-room file comparison, using the engine's bounded comparison API.
//! Public behavior reference: https://www.mathworks.com/help/matlab/ref/visdiff.html
//! The sole script builder is used by preview, workspace export and script save.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot, field};
use crate::toolkits::Action;
use eframe::egui;
use matrust::engine::compare::{
    self, ArrayDifference, Comparison, ComparisonType, DiffStatus, DifferenceReason, HunkKind,
    TextDiffOptions,
};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) type OpenSlot = Arc<Mutex<Option<(PathBuf, PathBuf, ComparisonType)>>>;

impl crate::Workspace {
    pub(crate) fn receive_comparison_tool(&mut self) {
        let paths = self
            .worker
            .comparison_paths
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some((left, right, kind)) = paths
            && !self.worker.cancel.load(Ordering::Relaxed)
        {
            self.toolkits.apps.comparison.app.choose(left, right, kind);
            self.open_app(crate::toolkits::Tool::App(AppKind::ComparisonTool));
        }
    }
}

const MAX_PATH: usize = 4096;
const ROW_HEIGHT: f32 = 24.;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    pub left: String,
    pub right: String,
    pub kind: ComparisonType,
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub ignore_line_endings: bool,
}

/// Quote a path as data and validate the resulting literal with the real parser.
/// Path fields are not MATLAB expressions; apostrophes are doubled before parse.
pub fn path_literal(text: &str) -> Result<String, String> {
    if text.is_empty() || text.len() > MAX_PATH || text.chars().any(char::is_control) {
        return Err(format!(
            "Choose a file or folder path (1–{MAX_PATH} characters, no control characters)"
        ));
    }
    let literal = format!("'{}'", text.replace('\'', "''"));
    super::single_expression(&literal)?;
    Ok(literal)
}

impl Spec {
    fn resolved(&self, ws: &Snapshot) -> Result<Self, String> {
        let resolve = |text: &str| -> Result<String, String> {
            path_literal(text)?;
            let p = Path::new(text);
            let p = if p.is_relative() {
                ws.folder()
                    .map_or_else(|| p.to_owned(), |folder| folder.join(p))
            } else {
                p.to_owned()
            };
            let value = p.to_str().ok_or("Paths must be valid Unicode")?.to_owned();
            path_literal(&value)?;
            Ok(value)
        };
        Ok(Self {
            left: resolve(&self.left)?,
            right: resolve(&self.right)?,
            ..self.clone()
        })
    }
    fn unfiltered_script(&self) -> Result<String, String> {
        let a = path_literal(&self.left)?;
        let b = path_literal(&self.right)?;
        let kind = self.type_argument();
        Ok(format!("visdiff({a}, {b}{kind});\n"))
    }
    fn type_argument(&self) -> &'static str {
        match self.kind {
            ComparisonType::Auto => "",
            ComparisonType::Text => ", 'text'",
            ComparisonType::Binary => ", 'binary'",
        }
    }
    pub fn script(&self) -> Result<String, String> {
        let mat = |path: &str| {
            Path::new(path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mat"))
        };
        if !(self.ignore_whitespace || self.ignore_case || self.ignore_line_endings)
            || self.kind == ComparisonType::Binary
            || (self.kind == ComparisonType::Auto && (mat(&self.left) || mat(&self.right)))
        {
            return self.unfiltered_script();
        }
        let a = path_literal(&self.left)?;
        let b = path_literal(&self.right)?;
        let kind = self.type_argument();
        let mut normalize = String::new();
        for text in ["left_text", "right_text"] {
            if self.ignore_line_endings {
                normalize += &format!("{text} = strrep({text}, char([13 10]), char(10));\n");
            }
            if self.ignore_whitespace {
                // Match compare::text_diff: collapse non-LF whitespace and
                // trim each line's edges, retaining every line terminator.
                // Suffix unterminated final lines with one space so an
                // empty normalized final line still occupies a file line.
                // LF-terminated lines cannot match these keys either way.
                normalize += &format!(
                    "has_tail = ~isempty({text}) && {text}(end) ~= char(10);\n\
                     {text} = regexprep({text}, '[^\\S\\n]+', ' ');\n\
                     {text} = regexprep({text}, '(^|\\n) +', '$1');\n\
                     {text} = regexprep({text}, ' +(\\n|$)', '$1');\n\
                     if has_tail, {text} = [{text} ' ']; end\n"
                );
            }
            if self.ignore_case {
                // The app lowercases individual scalars. Prevent lower's
                // contextual final-sigma mapping from changing that rule.
                normalize += &format!(
                    "{text} = strrep({text}, char(931), char(963));\n{text} = lower({text});\n"
                );
            }
        }
        // visdiff has no filter arguments. Normalize copies and compare those
        // with its public syntax. Local functions isolate temporary variables;
        // cleanup objects remove files and close writers even on errors.
        Ok(format!(
            "matrust_compare_filtered({a}, {b});\n\
             function matrust_compare_filtered(left, right)\n\
             if exist(left, 'dir') == 7 || exist(right, 'dir') == 7\n\
               visdiff(left, right{kind});\n\
               return;\n\
             end\n\
             left_text = fileread(left);\n\
             right_text = fileread(right);\n\
             {normalize}\
             left_file = tempname;\n\
             right_file = tempname;\n\
             cleanup = onCleanup(@() matrust_compare_cleanup(left_file, right_file));\n\
             matrust_compare_write(left_file, left_text);\n\
             matrust_compare_write(right_file, right_text);\n\
             visdiff(left_file, right_file, 'text');\n\
             delete(left_file, right_file);\n\
             end\n\
             function matrust_compare_write(path, text)\n\
             fid = fopen(path, 'wb', 'native', 'UTF-8');\n\
             if fid < 0, error('Cannot write temporary comparison file'); end\n\
             cleanup = onCleanup(@() fclose(fid));\n\
             fprintf(fid, '%s', text);\n\
             end\n\
             function done = matrust_compare_cleanup(left, right)\n\
             done = 0;\n\
             if exist(left, 'file'), delete(left); end\n\
             if exist(right, 'file'), delete(right); end\n\
             end\n"
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub changed: usize,
    pub inserted: usize,
    pub deleted: usize,
    pub same: usize,
}
impl Counts {
    fn record(&mut self, status: DiffStatus) {
        match status {
            DiffStatus::OnlyLeft => self.deleted += 1,
            DiffStatus::OnlyRight => self.inserted += 1,
            DiffStatus::Differs => self.changed += 1,
            DiffStatus::Same => self.same += 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextRow {
    /// One-based line numbers, absent for a gap in the alignment.
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub kind: HunkKind,
    pub inline: [Vec<Range<usize>>; 2],
}

pub struct View {
    pub spec: Spec,
    pub comparison: Comparison,
    pub rows: Vec<TextRow>,
    pub changes: Vec<usize>,
    /// Precomputed row indices: three lines of context either side of changes.
    pub context: Vec<usize>,
    pub counts: Counts,
    pub report: String,
    pub metadata: [BTreeMap<String, String>; 2],
    pub details: Vec<Vec<String>>,
    pub max_columns: usize,
}
impl View {
    pub fn build(
        spec: Spec,
        mut comparison: Comparison,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        if let Comparison::Text(diff) = &comparison
            && (spec.ignore_whitespace || spec.ignore_case || spec.ignore_line_endings)
        {
            comparison = Comparison::Text(compare::text_diff(
                &diff.left_lines.concat(),
                &diff.right_lines.concat(),
                TextDiffOptions {
                    ignore_whitespace: spec.ignore_whitespace,
                    ignore_case: spec.ignore_case,
                    ignore_crlf: spec.ignore_line_endings,
                    cancel: Some(cancel),
                },
            )?);
        }
        let report = compare::render_report(&comparison, &spec.left, &spec.right, cancel)?;
        let mut view = Self {
            spec,
            comparison,
            rows: Vec::new(),
            changes: Vec::new(),
            context: Vec::new(),
            counts: Counts::default(),
            report,
            metadata: Default::default(),
            details: Vec::new(),
            max_columns: 0,
        };
        match &view.comparison {
            Comparison::Text(diff) => {
                view.max_columns = diff
                    .left_lines
                    .iter()
                    .chain(&diff.right_lines)
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0);
                for hunk in &diff.hunks {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Comparison cancelled".into());
                    }
                    if hunk.kind != HunkKind::Equal {
                        view.changes.push(view.rows.len());
                    }
                    for offset in 0..hunk.left.len().max(hunk.right.len()) {
                        let left = (offset < hunk.left.len()).then_some(hunk.left.start + offset);
                        let right =
                            (offset < hunk.right.len()).then_some(hunk.right.start + offset);
                        let kind = match (left, right, hunk.kind) {
                            (_, _, HunkKind::Equal) => HunkKind::Equal,
                            (Some(_), Some(_), _) => HunkKind::Replace,
                            (Some(_), None, _) => HunkKind::Delete,
                            _ => HunkKind::Insert,
                        };
                        match kind {
                            HunkKind::Equal => view.counts.same += 1,
                            HunkKind::Replace => view.counts.changed += 1,
                            HunkKind::Insert => view.counts.inserted += 1,
                            HunkKind::Delete => view.counts.deleted += 1,
                        }
                        let inline = hunk
                            .inline
                            .get(offset)
                            .filter(|d| Some(d.left_line) == left && Some(d.right_line) == right)
                            .map_or_else(
                                || [Vec::new(), Vec::new()],
                                |d| [d.left.clone(), d.right.clone()],
                            );
                        view.rows.push(TextRow {
                            left,
                            right,
                            kind,
                            inline,
                        });
                    }
                }
                let mut end = 0;
                for (i, row) in view.rows.iter().enumerate() {
                    if row.kind != HunkKind::Equal {
                        let start = i.saturating_sub(3).max(end);
                        end = (i + 4).min(view.rows.len());
                        view.context.extend(start..end);
                    }
                }
            }
            Comparison::Mat(diff) => {
                let mut detail_bytes = 0;
                for row in &diff.variables {
                    view.counts.record(row.status);
                    let mut lines = Vec::new();
                    detail_lines(row, "", cancel, &mut lines, &mut detail_bytes)?;
                    view.details.push(lines);
                }
                for (side, path) in [&view.spec.left, &view.spec.right].into_iter().enumerate() {
                    let catalog = matrust::files::inspect_workspace(Path::new(path), cancel)?;
                    for variable in catalog.variables() {
                        view.metadata[side].insert(
                            variable.name.clone(),
                            format!("{} {:?}", variable.class, variable.dimensions),
                        );
                    }
                }
            }
            Comparison::Folder(diff) => {
                for row in &diff.files {
                    view.counts.record(row.status);
                }
            }
            Comparison::Binary(diff) => {
                view.counts.record(if diff.first_offset.is_some() {
                    DiffStatus::Differs
                } else {
                    DiffStatus::Same
                });
            }
        }
        Ok(view)
    }
}

pub struct ComparisonTool {
    pub draft: Spec,
    pub current: Option<Arc<View>>,
    pub output_name: String,
    pub output_edited: bool,
    pub selected_change: Option<usize>,
    pub show_only_changes: bool,
    pub selected_variable: Option<usize>,
    pub laid_out_rows: usize,
    pub laid_out_range: Range<usize>,
    active: Option<Spec>,
    history: Vec<(Spec, Arc<View>)>,
    payload: Arc<Mutex<Option<View>>>,
    compare_requested: bool,
    stop_requested: bool,
    report_requested: bool,
    pick_requested: Option<(usize, bool)>,
    pub report_path: Option<PathBuf>,
    running: bool,
    serial: u64,
    scroll_to: Option<usize>,
    text_column: usize,
    input_error: Option<String>,
}
impl Default for ComparisonTool {
    fn default() -> Self {
        Self {
            draft: Spec::default(),
            current: None,
            output_name: "comparisonCounts".into(),
            output_edited: false,
            selected_change: None,
            show_only_changes: false,
            selected_variable: None,
            laid_out_rows: 0,
            laid_out_range: 0..0,
            active: None,
            history: Vec::new(),
            payload: Default::default(),
            compare_requested: false,
            stop_requested: false,
            report_requested: false,
            pick_requested: None,
            report_path: None,
            running: false,
            serial: 0,
            scroll_to: None,
            text_column: 0,
            input_error: None,
        }
    }
}
impl ComparisonTool {
    pub fn compare(&mut self) {
        self.compare_requested = true;
    }
    pub fn choose(&mut self, left: PathBuf, right: PathBuf, kind: ComparisonType) {
        let left = left.to_string_lossy().into_owned();
        let right = right.to_string_lossy().into_owned();
        // Re-executing this viewer's Export visdiff call must preserve the
        // applied text filters and the folder Back path. Explicitly comparing
        // another pair starts with the standard defaults.
        let same = self.current.as_ref().is_some_and(|view| {
            let actual = match &view.comparison {
                Comparison::Text(_) => ComparisonType::Text,
                Comparison::Binary(_) => ComparisonType::Binary,
                _ => ComparisonType::Auto,
            };
            view.spec.left == left && view.spec.right == right && actual == kind
        });
        self.draft = if same {
            self.current.as_ref().unwrap().spec.clone()
        } else {
            self.history.clear();
            Spec {
                left,
                right,
                kind,
                ..Default::default()
            }
        };
        self.compare();
    }
    pub fn output(&self, ws: &Snapshot) -> Result<String, String> {
        let name = if self.output_edited {
            self.output_name.clone()
        } else {
            super::unused_name("comparisonCounts", |n| ws.contains(n))
        };
        super::single_expression(&name)?;
        if name.len() > 63 || !matrust::files::valid_name(&name) {
            return Err("Use a variable name of at most 63 characters".into());
        }
        super::check_export_name(&name)?;
        // `end` parses as an indexing expression, but is not an assignment
        // target. Validate the actual generated assignment context as well.
        matrust::syntax::parse(&format!("{name} = struct();"))
            .map_err(|e| format!("Invalid output name: {e}"))?;
        if ws.contains(&name) {
            return Err(format!(
                "Variable {name} already exists; choose an unused name"
            ));
        }
        Ok(name)
    }
    fn counts_script(&self, ws: &Snapshot) -> Result<String, String> {
        let name = self.output(ws)?;
        let view = self.current.as_ref().ok_or("Compare two items first")?;
        if self.active.as_ref() != Some(&view.spec) {
            return Err("Compare the current paths before exporting their counts".into());
        }
        let c = view.counts;
        let comparison = self.script(ws)?;
        let (body, functions) = super::split_local_functions(&comparison);
        let script = format!(
            "{body}{name} = struct('changed', {}, 'inserted', {}, 'deleted', {}, 'same', {});\n{functions}",
            c.changed, c.inserted, c.deleted, c.same
        );
        Ok(script)
    }
    pub fn navigate(&mut self, next: bool) -> Option<usize> {
        let view = self.current.as_ref()?;
        let n = view.changes.len();
        if n == 0 {
            self.selected_change = None;
            return None;
        }
        let index = match self.selected_change {
            None => {
                if next {
                    0
                } else {
                    n - 1
                }
            }
            Some(i) => {
                if next {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                }
            }
        };
        self.selected_change = Some(index);
        let row = view.changes[index];
        self.scroll_to = Some(row);
        self.text_column = 0;
        Some(row)
    }
    pub fn drill_down(&mut self, index: usize) -> bool {
        let Some(view) = self.current.as_ref() else {
            return false;
        };
        let Comparison::Folder(folder) = &view.comparison else {
            return false;
        };
        let Some(file) = folder.files.get(index) else {
            return false;
        };
        if file.status != DiffStatus::Differs {
            return false;
        }
        let draft = Spec {
            left: Path::new(&view.spec.left)
                .join(&file.name)
                .to_string_lossy()
                .into_owned(),
            right: Path::new(&view.spec.right)
                .join(&file.name)
                .to_string_lossy()
                .into_owned(),
            ..view.spec.clone()
        };
        if self.history.len() >= 16 {
            return false;
        }
        let old = self.current.as_ref().unwrap().clone();
        self.history.push((self.draft.clone(), old));
        self.draft = draft;
        self.compare();
        true
    }
    pub fn back(&mut self) {
        if let Some((draft, view)) = self.history.pop() {
            self.active = Some(view.spec.clone());
            self.draft = draft;
            self.current = Some(view);
            self.selected_change = None;
            self.selected_variable = None;
            // Rerun through Runner after Back, so Export is tied to this view.
            self.compare();
        }
    }
}

impl App for ComparisonTool {
    fn kind(&self) -> AppKind {
        AppKind::ComparisonTool
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        self.output(ws)?;
        if let Some(error) = &self.input_error {
            return Err(error.clone());
        }
        self.active
            .as_ref()
            .ok_or("Choose two paths and press Compare")?
            .script()
    }
    fn execution_script(&self, ws: &Snapshot) -> Result<String, String> {
        // The private view needs original lines and character ranges. Apply
        // filters in View::build; saved/exported scripts normalize file copies.
        self.script(ws)?;
        self.active.as_ref().unwrap().unfiltered_script()
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.counts_script(ws)
    }
    fn inputs(&self) -> Vec<String> {
        Vec::new()
    }
    fn probe(&self, _ws: &Snapshot) -> (String, Vec<String>) {
        (String::new(), Vec::new())
    }
    fn prelude(&self, _ws: &Snapshot) -> String {
        format!("% Comparison request {}\n", self.serial)
    }
    fn prepare(&mut self, ws: &Snapshot) {
        if std::mem::take(&mut self.compare_requested) {
            self.serial = self.serial.wrapping_add(1);
            match self.draft.resolved(ws) {
                Ok(spec) => {
                    self.active = Some(spec);
                    self.input_error = None;
                }
                Err(error) => self.input_error = Some(error),
            }
        }
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, request: Request) {
        if runner.key() == Some(request.key) {
            runner.poll(ctx);
            return;
        }
        let Some(spec) = self.active.clone() else {
            return;
        };
        let payload = self.payload.clone();
        let key = request.key;
        runner.submit_task(
            ctx,
            key,
            Box::new(move |engine| {
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                let _presentation = matrust::engine::compare_builtin::present_comparisons(
                    move |_, _, comparison| {
                        send.try_send(comparison.clone()).map_err(|e| e.to_string())
                    },
                );
                // The UI builds MATLAB code, and the Runner executes that exact
                // visdiff call. Its engine-only hook supplies the comparison data.
                let outputs = super::run(engine, request)?;
                let comparison = receive
                    .try_recv()
                    .map_err(|_| "visdiff did not produce a comparison")?;
                let view = View::build(spec, comparison, &engine.cancellation())?;
                if engine.cancellation().load(Ordering::Relaxed) {
                    return Err("Comparison cancelled".into());
                }
                *payload
                    .lock()
                    .map_err(|_| "Comparison result lock failed")? = Some(view);
                Ok(outputs)
            }),
        );
    }
    fn receive(&mut self, _outputs: &Outputs) -> Result<(), String> {
        let view = self
            .payload
            .lock()
            .map_err(|_| "Comparison result lock failed")?
            .take()
            .ok_or("Comparison produced no view")?;
        self.current = Some(Arc::new(view));
        self.text_column = 0;
        self.selected_change = None;
        self.selected_variable = None;
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        let mut browse = None;
        for (side, (label, text)) in [
            ("Left", &mut self.draft.left),
            ("Right", &mut self.draft.right),
        ]
        .into_iter()
        .enumerate()
        {
            field(ui, label, |ui| {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(text).char_limit(MAX_PATH))
                        .desired_width(ui.available_width()),
                );
                ui.horizontal(|ui| {
                    if ui.button("Browse file…").clicked() {
                        browse = Some((side, false));
                    }
                    if ui.button("Browse folder…").clicked() {
                        browse = Some((side, true));
                    }
                });
            });
        }
        if browse.is_some() {
            self.pick_requested = browse;
        }
        ui.horizontal(|ui| {
            if ui.button("Swap").clicked() {
                std::mem::swap(&mut self.draft.left, &mut self.draft.right);
                self.compare();
            }
            if ui.button("Compare").clicked() {
                self.compare();
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop_requested = true;
            }
        });
        ui.add_space(crate::tokens::S);
        field(ui, "Type", |ui| {
            egui::ComboBox::from_id_salt("comparison_type")
                .selected_text(type_label(self.draft.kind))
                .show_ui(ui, |ui| {
                    for kind in [
                        ComparisonType::Auto,
                        ComparisonType::Text,
                        ComparisonType::Binary,
                    ] {
                        ui.selectable_value(&mut self.draft.kind, kind, type_label(kind));
                    }
                });
        });
        ui.checkbox(&mut self.draft.ignore_whitespace, "Ignore whitespace");
        ui.checkbox(&mut self.draft.ignore_case, "Ignore case");
        ui.checkbox(&mut self.draft.ignore_line_endings, "Ignore line endings");
        crate::tokens::helper(ui, "Press Compare to apply paths and options.");
        if !self.history.is_empty() && ui.button("Back to folders").clicked() {
            self.back();
        }
    }
    fn main_caption(&self) -> &'static str {
        "COMPARISON"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.laid_out_rows = 0;
        self.laid_out_range = 0..0;
        ui.horizontal(|ui| {
            let has_changes = self.current.as_ref().is_some_and(|v| !v.changes.is_empty());
            if ui
                .add_enabled(has_changes, egui::Button::new("Previous change"))
                .clicked()
            {
                self.navigate(false);
            }
            if ui
                .add_enabled(has_changes, egui::Button::new("Next change"))
                .clicked()
            {
                self.navigate(true);
            }
            if ui
                .checkbox(&mut self.show_only_changes, "Show only changes")
                .changed()
            {
                self.scroll_to = self
                    .selected_change
                    .and_then(|i| self.current.as_ref()?.changes.get(i).copied());
            }
            if let Some(i) = self.selected_change
                && let Some(v) = &self.current
            {
                ui.label(format!("{} / {}", i + 1, v.changes.len()));
            }
        });
        let Some(view) = &self.current else {
            crate::tokens::helper(ui, "Compare files or folders to see their differences.");
            return;
        };
        let palette = crate::tokens::palette(ui.ctx());
        match &view.comparison {
            Comparison::Text(diff) => {
                if view.max_columns > 80 {
                    ui.add(
                        egui::Slider::new(
                            &mut self.text_column,
                            0..=view.max_columns.saturating_sub(1),
                        )
                        .text("Horizontal offset (both sides)")
                        .clamping(egui::SliderClamping::Always),
                    );
                }
                let shown = |i: usize| {
                    if self.show_only_changes {
                        view.context[i]
                    } else {
                        i
                    }
                };
                let count = if self.show_only_changes {
                    view.context.len()
                } else {
                    view.rows.len()
                };
                let mut area = egui::ScrollArea::both()
                    .id_salt("comparison_text")
                    .auto_shrink([false, false]);
                if let Some(target) = self.scroll_to.take() {
                    let index = if self.show_only_changes {
                        view.context.binary_search(&target).unwrap_or(0)
                    } else {
                        target
                    };
                    area = area.vertical_scroll_offset(
                        index as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y),
                    );
                }
                // One shared scroll area synchronizes both sides. show_rows
                // visits visible rows only; text is laid out only for those rows.
                area.show_rows(ui, ROW_HEIGHT, count, |ui, range| {
                    self.laid_out_rows = range.len();
                    self.laid_out_range = range.clone();
                    let width = (ui.available_width() - crate::tokens::S).max(320.) / 2.;
                    for index in range {
                        let row = &view.rows[shown(index)];
                        ui.horizontal(|ui| {
                            for side in 0..2 {
                                let (number, lines) = if side == 0 {
                                    (row.left, &diff.left_lines)
                                } else {
                                    (row.right, &diff.right_lines)
                                };
                                let text = number
                                    .and_then(|n| lines.get(n - 1))
                                    .map_or("", String::as_str);
                                let fill = row_tint(row.kind, side, palette);
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(width, ROW_HEIGHT),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(rect, 0., fill);
                                if self
                                    .selected_change
                                    .is_some_and(|i| view.changes[i] == shown(index))
                                {
                                    ui.painter().line_segment(
                                        [rect.left_top(), rect.left_bottom()],
                                        egui::Stroke::new(2., palette.accent),
                                    );
                                }
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                ui.painter().text(
                                    rect.left_center() + egui::vec2(crate::tokens::S, 0.),
                                    egui::Align2::LEFT_CENTER,
                                    number.map_or(String::new(), |n| n.to_string()),
                                    font.clone(),
                                    palette.text_muted,
                                );
                                let mut job = line_job(
                                    text,
                                    &row.inline[side],
                                    self.text_column,
                                    font,
                                    palette,
                                );
                                job.wrap.max_width = (width - 64.).max(16.);
                                job.wrap.max_rows = 1;
                                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                                ui.painter().with_clip_rect(rect).galley(
                                    rect.left_top() + egui::vec2(64., 4.),
                                    galley,
                                    palette.text,
                                );
                            }
                        });
                    }
                });
            }
            Comparison::Mat(diff) => {
                ui.columns(4, |columns| {
                    for (column, title) in columns.iter_mut().zip([
                        "Variable",
                        "Left class / size",
                        "Right class / size",
                        "Status",
                    ]) {
                        column.label(title);
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("comparison_mat")
                    .max_height((ui.available_height() * 0.6).max(96.))
                    .show_rows(ui, ROW_HEIGHT, diff.variables.len(), |ui, range| {
                        self.laid_out_rows = range.len();
                        for i in range {
                            let row = &diff.variables[i];
                            ui.columns(4, |columns| {
                                if columns[0]
                                    .add(
                                        egui::Button::selectable(
                                            self.selected_variable == Some(i),
                                            &row.name,
                                        )
                                        .truncate(),
                                    )
                                    .clicked()
                                {
                                    self.selected_variable = Some(i);
                                }
                                for (side, column) in columns[1..3].iter_mut().enumerate() {
                                    let text = view.metadata[side]
                                        .get(&row.name)
                                        .map_or("—", String::as_str);
                                    column
                                        .add(egui::Label::new(text).truncate())
                                        .on_hover_text(text);
                                }
                                columns[3].label(status_label(row.status));
                            });
                        }
                    });
                if let Some(index) = self.selected_variable
                    && let Some(lines) = view.details.get(index)
                {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("comparison_details")
                        .show_rows(ui, ROW_HEIGHT, lines.len(), |ui, range| {
                            for i in range {
                                ui.add_sized(
                                    [ui.available_width(), ROW_HEIGHT],
                                    egui::Label::new(&lines[i]).truncate(),
                                )
                                .on_hover_text(&lines[i]);
                            }
                        });
                }
            }
            Comparison::Folder(diff) => {
                ui.columns(2, |columns| {
                    columns[0].label("File");
                    columns[1].label("Status");
                });
                let mut drill = None;
                egui::ScrollArea::vertical()
                    .id_salt("comparison_folder")
                    .show_rows(ui, ROW_HEIGHT, diff.files.len(), |ui, range| {
                        self.laid_out_rows = range.len();
                        for i in range {
                            let file = &diff.files[i];
                            ui.columns(2, |columns| {
                                if columns[0]
                                    .add(
                                        egui::Label::new(file.name.to_string_lossy())
                                            .truncate()
                                            .sense(egui::Sense::click()),
                                    )
                                    .double_clicked()
                                {
                                    drill = Some(i);
                                }
                                columns[1].label(status_label(file.status));
                            });
                        }
                    });
                crate::tokens::helper(ui, "Double-click a changed file to compare its contents.");
                if let Some(index) = drill {
                    self.drill_down(index);
                }
            }
            Comparison::Binary(_) => {
                ui.label(&view.report);
            }
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        self.current.as_ref().map_or_else(Vec::new, |v| {
            vec![
                (
                    "Counting",
                    match &v.comparison {
                        Comparison::Text(_) => "Lines",
                        Comparison::Mat(_) => "Variables",
                        Comparison::Folder(_) | Comparison::Binary(_) => "Files",
                    }
                    .into(),
                ),
                ("Changed", v.counts.changed.to_string()),
                ("Inserted", v.counts.inserted.to_string()),
                ("Deleted", v.counts.deleted.to_string()),
                ("Unchanged", v.counts.same.to_string()),
            ]
        })
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if !self.output_edited {
            self.output_name = super::unused_name("comparisonCounts", |n| ws.contains(n));
        }
        field(ui, "Counts struct", |ui| {
            self.output_edited |= ui
                .add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.output_name).char_limit(63),
                    )
                    .desired_width(ui.available_width()),
                )
                .changed();
        });
        if ui
            .add_enabled(
                self.current.is_some() && !self.running,
                egui::Button::new("Save text report…"),
            )
            .clicked()
        {
            self.report_requested = true;
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.output(ws).into_iter().collect()
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|v| {
            format!(
                "Comparison Tool: {} changed, {} inserted, {} deleted",
                v.counts.changed, v.counts.inserted, v.counts.deleted
            )
        })
    }
    fn export_label(&self) -> &'static str {
        "Export counts to workspace"
    }
}

pub fn row_tint(kind: HunkKind, side: usize, p: &crate::tokens::Palette) -> egui::Color32 {
    match (kind, side) {
        (HunkKind::Insert, 1) => p.success.gamma_multiply(0.12),
        (HunkKind::Delete, 0) => p.error.gamma_multiply(0.12),
        (HunkKind::Replace, _) => p.warning.gamma_multiply(0.12),
        _ => p.surface,
    }
}
pub(crate) fn line_job(
    text: &str,
    changed: &[Range<usize>],
    column: usize,
    font: egui::FontId,
    p: &crate::tokens::Palette,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    // A shared column control reveals long lines without laying out 65k glyphs
    // per visible row. 512 scalars exceed the pane's visible width on the grid.
    let start = text
        .char_indices()
        .nth(column)
        .map_or(text.len(), |(i, _)| i);
    let tail = &text[start..];
    let end = tail.char_indices().nth(512).map_or(tail.len(), |(i, _)| i);
    let text = &tail[..end];
    let clipped = changed
        .iter()
        .filter_map(|r| {
            let start = r.start.saturating_sub(column).min(512);
            let end = r.end.saturating_sub(column).min(512);
            (start < end).then_some(start..end)
        })
        .collect::<Vec<_>>();
    // Ranges use Unicode scalar indices. Group runs to avoid per-character
    // sections, even when a visible input line contains 65k characters.
    let mut bytes = text.char_indices().map(|(i, _)| i).collect::<Vec<_>>();
    bytes.push(text.len());
    let mut cursor = 0;
    for range in &clipped {
        let start = bytes.get(range.start).copied().unwrap_or(text.len());
        let end = bytes.get(range.end).copied().unwrap_or(text.len());
        if start > cursor {
            job.append(
                &text[cursor..start],
                0.,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: p.text,
                    ..Default::default()
                },
            );
        }
        if end > start {
            job.append(
                &text[start..end],
                0.,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: p.text,
                    background: p.warning.gamma_multiply(0.3),
                    underline: egui::Stroke::new(1., p.warning),
                    ..Default::default()
                },
            );
        }
        cursor = end;
    }
    if cursor < text.len() {
        job.append(
            text[cursor..].trim_end_matches(['\r', '\n']),
            0.,
            egui::TextFormat {
                font_id: font,
                color: p.text,
                ..Default::default()
            },
        );
    }
    job
}
fn type_label(kind: ComparisonType) -> &'static str {
    match kind {
        ComparisonType::Auto => "Auto by extension",
        ComparisonType::Text => "Text",
        ComparisonType::Binary => "Binary",
    }
}
fn status_label(status: DiffStatus) -> &'static str {
    match status {
        DiffStatus::OnlyLeft => "Deleted",
        DiffStatus::OnlyRight => "Inserted",
        DiffStatus::Same => "Unchanged",
        DiffStatus::Differs => "Changed",
    }
}
pub(crate) fn detail_lines(
    row: &ArrayDifference,
    prefix: &str,
    cancel: &AtomicBool,
    lines: &mut Vec<String>,
    bytes: &mut usize,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Comparison cancelled".into());
    }
    let name = if prefix.is_empty() {
        row.name.clone()
    } else {
        format!("{prefix}.{}", row.name)
    };
    let mut append = |text: String| -> Result<(), String> {
        *bytes = bytes.saturating_add(text.len());
        if *bytes > 16 * 1024 * 1024 || lines.len() >= 100_000 {
            return Err("MAT detail view exceeds its output limit".into());
        }
        lines.push(text);
        Ok(())
    };
    append(format!("{name}: {}", status_label(row.status)))?;
    for reason in &row.reasons {
        append(match reason {
            DifferenceReason::Class { left, right } => format!("Class: {left} → {right}"),
            DifferenceReason::Size { left, right } => format!("Size: {left:?} → {right:?}"),
            DifferenceReason::Complexity => "Real / complex storage differs".into(),
            DifferenceReason::Sparsity => "Sparse / full storage differs".into(),
            DifferenceReason::Values(v) => format!(
                "{} differing values; first differing index: {} (column-major, 1-based); max abs diff: {}",
                v.count,
                v.first_index,
                v.max_abs_difference.map_or("—".into(), super::number)
            ),
        })?;
    }
    for child in &row.children {
        detail_lines(child, &name, cancel, lines, bytes)?;
    }
    Ok(())
}

pub(crate) fn show(
    host: &mut Host<ComparisonTool>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.stop_requested) {
        host.runner.stop();
        host.script_error = Some("Comparison stopped. Press Compare to run again.".into());
        host.app.input_error = host.script_error.clone();
    }
    if ui.ctx().will_discard() {
        return;
    }
    if let Some((side, folder)) = host.app.pick_requested.take()
        && let Some(path) = pick_path(ws, folder)
    {
        if side == 0 {
            host.app.draft.left = path;
        } else {
            host.app.draft.right = path;
        }
    }
    if std::mem::take(&mut host.app.report_requested)
        && host.ready(ws, busy)
        && !host.app.compare_requested
        && let Some(view) = &host.app.current
        && let Some(path) = pick_report(ws)
    {
        // SaveDesign already uses the workspace worker and atomic_write for
        // arbitrary UTF-8 text. It also carries completion errors to status.
        *action = Some(Action::SaveAppScript(
            AppKind::ComparisonTool,
            view.report.clone(),
        ));
        // Queue the chosen report destination through the standard save path.
        host.app.report_path = Some(path);
    }
}

fn pick_path(ws: &Snapshot, folder: bool) -> Option<String> {
    #[cfg(test)]
    {
        let _ = (ws, folder);
        None
    }
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(path) = ws.folder() {
            dialog = dialog.set_directory(path);
        }
        (if folder {
            dialog.pick_folder()
        } else {
            dialog.pick_file()
        })
        .map(|p| p.to_string_lossy().into_owned())
    }
}
fn pick_report(ws: &Snapshot) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = ws;
        None
    }
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new()
            .set_file_name("comparison.txt")
            .add_filter("Text report", &["txt", "diff"]);
        if let Some(path) = ws.folder() {
            dialog = dialog.set_directory(path);
        }
        dialog.save_file()
    }
}
