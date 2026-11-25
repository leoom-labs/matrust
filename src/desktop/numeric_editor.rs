//! Shared root/nested numeric grid. Payload work is delegated to one worker.
use super::variable_edit_job::{Action, Effect, Finished, Request, Token};
use super::*;
use crate::theme::ThemedScrollArea;
use matrust::{
    Array,
    variable_edit::{self, Rect, Target},
};

enum Intent {
    Submit(Action, Rect),
    RequestPaste,
    Undo,
    Redo,
}

/// Applying an entry publishes `to`; it is valid only while the variable is still `from`.
#[derive(Clone)]
struct Entry {
    from: Array,
    to: Array,
}
const UNDO_DEPTH: usize = 20;
#[derive(Default)]
struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}
/// What the one running worker job means for the history.
#[derive(Default)]
enum Inflight {
    #[default]
    Read,
    Edit(Array),
    Undo(Entry),
    Redo(Entry),
}
#[derive(Clone, Copy)]
enum Menu {
    Copy,
    Paste,
    RowAbove,
    RowBelow,
    ColLeft,
    ColRight,
    DeleteRows,
    DeleteCols,
}
/// Selection bounds `[row0, col0, row1, col1]` (inclusive) a menu acts on.
type Bounds = [usize; 4];
fn context_menu(response: &egui::Response, table: bool) -> Option<Menu> {
    let mut pick = None;
    response.context_menu(|ui| pick = context_items(ui, table));
    pick
}
fn context_items(ui: &mut egui::Ui, table: bool) -> Option<Menu> {
    let mut pick = None;
    for (label, item) in [
        ("Copy", Menu::Copy),
        ("Paste", Menu::Paste),
        ("Insert row above", Menu::RowAbove),
        ("Insert row below", Menu::RowBelow),
        ("Insert column left", Menu::ColLeft),
        ("Insert column right", Menu::ColRight),
        ("Delete row(s)", Menu::DeleteRows),
        ("Delete column(s)", Menu::DeleteCols),
    ] {
        let enabled = !table || !matches!(item, Menu::ColLeft | Menu::ColRight | Menu::DeleteCols);
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            pick = Some(item);
            ui.close();
        }
    }
    pick
}

// Freeze the painted viewport during egui discarded passes. Scroll input is
// admitted once; its resulting offset is resumed on the next actual frame.
struct GridView {
    columns: core::ops::Range<usize>,
    height: f32,
    offset: egui::Vec2,
    next_offset: egui::Vec2,
}
fn rectangle(anchor: [usize; 2], row: usize, col: usize) -> Result<Rect, String> {
    let end = [row - 1, col - 1];
    Rect::new(
        anchor[0].min(end[0]),
        anchor[1].min(end[1]),
        anchor[0].abs_diff(end[0]) + 1,
        anchor[1].abs_diff(end[1]) + 1,
    )
}

#[derive(Default)]
pub(super) struct State {
    revision: Option<Token>,
    editor: Token,
    target: Option<Target>,
    anchor: [usize; 2],
    focus: Option<egui::Id>,
    pending: bool,
    clipboard: Option<String>,
    notice: Option<String>,
    action_frame: Option<u64>,
    scroll_row: Option<usize>,
    format_frame: Option<u64>,
    display: BTreeMap<(usize, usize), String>,
    table_editable: Vec<bool>,
    tooltip: Option<((usize, usize), String)>,
    refresh_literal: bool,
    input_frame: Option<u64>,
    view: Option<GridView>,
    resume_offset: Option<egui::Vec2>,
    defer_frame: Option<u64>,
    intent: Option<(u64, Token, Intent)>,
    /// Cell being edited in place (zero-based) and whether it still needs focus.
    editing: Option<[usize; 2]>,
    edit_focus: bool,
    /// A mouse drag that started on a cell is in progress.
    drag: bool,
    history: BTreeMap<String, History>,
    inflight: Inflight,
    pub(super) formatted: usize,
    #[cfg(test)]
    pub(super) cells: BTreeMap<(usize, usize), egui::Rect>,
    #[cfg(test)]
    pub(super) row_headers: BTreeMap<usize, egui::Rect>,
    #[cfg(test)]
    pub(super) col_headers: BTreeMap<usize, egui::Rect>,
    #[cfg(test)]
    pub(super) grid_clip: Option<egui::Rect>,
    #[cfg(test)]
    pub(super) field_rect: Option<egui::Rect>,
}
impl State {
    pub(super) fn revision(&self) -> Option<Token> {
        self.revision.clone()
    }
    pub(super) fn invalidate(&mut self, explain: bool) {
        let revision = self.revision.clone();
        let format_frame = self.format_frame;
        let formatted = self.formatted;
        let history = std::mem::take(&mut self.history);
        *self = Self { revision, format_frame, formatted, history, notice: explain.then(||
            "Workspace operation completed; selection and edit text were refreshed. Review the value before applying.".into()), ..Default::default() };
    }
    fn admit(&mut self, frame: u64, intent: Intent) {
        // First submit wins within an ordered frame, including discarded passes.
        if self.intent.is_none() && self.action_frame != Some(frame) {
            self.intent = Some((frame, self.editor.clone(), intent));
        }
    }
    /// Selected rectangle as `(row, col, rows, cols)`, for tests.
    #[cfg(test)]
    pub(super) fn selection(&self, end: [usize; 2]) -> (usize, usize, usize, usize) {
        (
            self.anchor[0].min(end[0]),
            self.anchor[1].min(end[1]),
            self.anchor[0].abs_diff(end[0]) + 1,
            self.anchor[1].abs_diff(end[1]) + 1,
        )
    }
    #[cfg(test)]
    pub(super) fn editing(&self) -> Option<[usize; 2]> {
        self.editing
    }
    fn matches(&self, target: &Target) -> bool {
        self.target.as_ref().is_some_and(|old| {
            old.name() == target.name()
                && old.path() == target.path()
                && old.expected_root().same_snapshot(target.expected_root())
        })
    }
}

/// One UTF-16 unit from a keystroke. A supplementary character is a pair, and
/// a lone surrogate is never a Rust scalar, so neither is stored in a char cell.
fn single_utf16_unit(text: &str) -> Option<u16> {
    let mut units = text.encode_utf16();
    let unit = units.next()?;
    if units.next().is_some() || (0xD800..=0xDFFF).contains(&unit) {
        None
    } else {
        Some(unit)
    }
}

const CHAR_CELL_UNIT_ERROR: &str = "one UTF-16 unit per char cell; use \\uXXXX";

impl Workspace {
    pub(super) fn receive_variable_edit(&mut self, revision: Token, finished: Option<Finished>) {
        self.numeric_editor.revision = Some(revision);
        let Some(finished) = finished else {
            if self.editing_variable.is_some() {
                self.numeric_editor.invalidate(true);
                self.typed_inspector.reset_navigation();
            }
            return;
        };
        if !self.numeric_editor.pending
            || !self.numeric_editor.editor.same(&finished.editor)
            || !self.numeric_editor.matches(&finished.target)
            || self.editing_variable.as_deref() != Some(finished.target.name())
        {
            return;
        }
        self.numeric_editor.pending = false;
        let inflight = std::mem::take(&mut self.numeric_editor.inflight);
        match finished.result {
            Err(error) => self.cell_error = Some(error),
            Ok(Effect::Copied(text)) => {
                if self.worker.cancel.load(Ordering::Relaxed) {
                    self.cell_error = Some("Copy cancelled".into());
                } else {
                    self.numeric_editor.clipboard = Some(text);
                }
            }
            Ok(Effect::Edited(committed)) => {
                self.typed_inspector
                    .accept_edit(finished.target.expected_root(), &committed.root);
                let history = self
                    .numeric_editor
                    .history
                    .entry(finished.target.name().into())
                    .or_default();
                match inflight {
                    Inflight::Edit(before) => {
                        history.undo.push(Entry {
                            from: committed.root.clone(),
                            to: before,
                        });
                        if history.undo.len() > UNDO_DEPTH {
                            history.undo.remove(0);
                        }
                        history.redo.clear();
                    }
                    Inflight::Undo(entry) => {
                        history.undo.pop();
                        history.redo.push(Entry {
                            from: committed.root.clone(),
                            to: entry.from,
                        });
                    }
                    Inflight::Redo(entry) => {
                        history.redo.pop();
                        history.undo.push(Entry {
                            from: committed.root.clone(),
                            to: entry.from,
                        });
                    }
                    Inflight::Read => {}
                }
                self.numeric_editor.editing = None;
                self.numeric_editor.target = Target::new(
                    finished.target.name().into(),
                    committed.root,
                    finished.target.path().to_vec(),
                )
                .ok();
                // Fixed-shape edits do not retarget a cursor moved after submission.
                // The request owns its rectangle; current selection remains independent.
                self.numeric_editor.display.clear();
                self.numeric_editor.refresh_literal = true;
                self.cell_error = None;
            }
        }
    }
    fn refresh_numeric_literal(&mut self, leaf: &Array) {
        if self.numeric_editor.formatted < 512 {
            self.edit_value =
                variable_edit::cell_literal(leaf, self.edit_row - 1, self.edit_col - 1)
                    .unwrap_or_default();
            self.numeric_editor.formatted += 1;
            self.numeric_editor.refresh_literal = false;
        } else {
            self.numeric_editor.refresh_literal = true;
        }
    }
    /// Open the in-cell editor on the selection's cursor cell.
    fn begin_cell_edit(&mut self, leaf: &Array, typed: Option<String>, frame: u64) {
        let cell = [self.edit_row - 1, self.edit_col - 1];
        if !variable_edit::cell_editable(leaf, cell[0], cell[1]) {
            if matches!(leaf, Array::Cell(_)) && typed.is_none() {
                self.typed_inspector
                    .open_cell(cell[1] * leaf.rows() + cell[0], frame);
            } else {
                self.cell_error =
                    Some("This cell is read-only; open its contents to inspect it.".into());
            }
            return;
        }
        self.edit_value = match typed {
            Some(text) if matches!(leaf, Array::Character(_)) => {
                let Some(unit) = single_utf16_unit(&text) else {
                    self.cell_error = Some(CHAR_CELL_UNIT_ERROR.into());
                    return;
                };
                self.cell_error = None;
                format!("\\u{unit:04X}")
            }
            Some(text) => text,
            None => variable_edit::cell_literal(leaf, cell[0], cell[1]).unwrap_or_default(),
        };
        self.numeric_editor.editing = Some(cell);
        self.numeric_editor.edit_focus = true;
        self.numeric_editor.refresh_literal = false;
    }
    pub(super) fn numeric_variable_ui(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        root: &Array,
        leaf: &Array,
        path: Vec<usize>,
        enabled: bool,
    ) {
        if leaf.is_nd() && leaf.is_empty() {
            ui.label(matrust::display_format::empty_caption(
                leaf.dims().as_ref(),
                leaf.class(),
            ));
            return;
        }
        if leaf.is_nd() {
            let size = leaf
                .dims()
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("x");
            ui.label(format!("{size} {}", leaf.class()));
            ui.label("N-D array: use indexing, e.g. A(:,:,1)");
            return;
        }
        let target = match Target::new(name.into(), root.clone(), path) {
            Ok(target) => target,
            Err(error) => {
                ui.label(error);
                return;
            }
        };
        let frame = ui.ctx().cumulative_frame_nr();
        let table = matches!(leaf, Array::Table(_));
        let text_limit = if matches!(leaf, Array::Str(_) | Array::Cell(_) | Array::Table(_)) {
            variable_edit::MAX_TEXT_BYTES
        } else {
            256
        };
        if !self.numeric_editor.matches(&target) {
            let defer = self.numeric_editor.format_frame == Some(frame)
                && self.numeric_editor.formatted > 0;
            let notice = self.numeric_editor.notice.take();
            self.numeric_editor.invalidate(false);
            self.numeric_editor.notice = notice;
            self.numeric_editor.target = Some(target.clone());
            if let Array::Table(t) = leaf {
                self.numeric_editor.table_editable = t
                    .columns()
                    .iter()
                    .map(variable_edit::table_column_editable)
                    .collect();
            }
            self.numeric_editor.defer_frame = defer.then_some(frame);
            self.edit_row = 1;
            self.edit_col = 1;
            self.first_col = 1;
            self.numeric_editor.refresh_literal = true;
            self.cell_error = None;
        }
        if self.numeric_editor.format_frame != Some(frame) {
            self.numeric_editor.format_frame = Some(frame);
            self.numeric_editor.formatted = 0;
            self.numeric_editor.display.clear();
            self.numeric_editor.tooltip = None;
            self.numeric_editor.resume_offset =
                self.numeric_editor.view.take().map(|v| v.next_offset);
            #[cfg(test)]
            self.numeric_editor.cells.clear();
        }
        if self.numeric_editor.defer_frame == Some(frame) {
            ui.label("Updating selected array...");
            ui.ctx().request_repaint();
            return;
        }
        if self
            .numeric_editor
            .intent
            .as_ref()
            .is_some_and(|(old, _, _)| *old != frame)
        {
            self.numeric_editor.intent = None;
        }
        if leaf.is_empty() {
            ui.label(matrust::display_format::empty_caption(
                leaf.dims().as_ref(),
                leaf.class(),
            ));
            let enabled =
                enabled && self.numeric_editor.revision.is_some() && !self.numeric_editor.pending;
            if matches!(leaf, Array::Character(text) if text.is_text_row())
                && ui
                    .add_enabled(enabled, egui::Button::new("Copy text row"))
                    .clicked()
            {
                self.numeric_editor.admit(
                    frame,
                    Intent::Submit(Action::CopyText, Rect::new(0, 0, 1, 1).unwrap()),
                );
            }
            let can_undo = self
                .numeric_editor
                .history
                .get(name)
                .is_some_and(|h| !h.undo.is_empty());
            if ui
                .add_enabled(enabled && can_undo, egui::Button::new("Undo"))
                .clicked()
            {
                self.numeric_editor.admit(frame, Intent::Undo);
            }
            for (label, shape, allowed) in [
                (
                    "Insert row",
                    variable_edit::Shape::InsertRows {
                        at: leaf.rows(),
                        count: 1,
                    },
                    true,
                ),
                (
                    "Insert column",
                    variable_edit::Shape::InsertCols {
                        at: leaf.cols(),
                        count: 1,
                    },
                    !table,
                ),
            ] {
                if ui
                    .add_enabled(enabled && allowed, egui::Button::new(label))
                    .clicked()
                {
                    self.numeric_editor.admit(
                        frame,
                        Intent::Submit(Action::Reshape(shape), Rect::new(0, 0, 1, 1).unwrap()),
                    );
                }
            }
            if let Some(error) = &self.cell_error {
                ui.colored_label(tokens::palette(ui.ctx()).error, error);
            }
            let grid = ui.make_persistent_id(("rooted_numeric_grid", name, target.path()));
            let _ = ui.interact(
                ui.available_rect_before_wrap(),
                grid,
                egui::Sense::focusable_noninteractive(),
            );
            if enabled && ui.memory(|m| m.has_focus(grid)) {
                if ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
                        || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                }) {
                    self.numeric_editor.admit(frame, Intent::Undo);
                }
                if ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
                        || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                }) {
                    self.numeric_editor.admit(frame, Intent::Redo);
                }
            }
            self.finish_variable_intent(ui, target, enabled, frame, grid);
            return;
        }
        if matches!(leaf, Array::Character(_)) {
            ui.small("UTF-16 code units: \\uXXXX, 0xXXXX or decimal 0..65535. Copy/paste uses escaped units; shape stays fixed.");
        }
        self.edit_row = self.edit_row.clamp(1, leaf.rows());
        self.edit_col = self.edit_col.clamp(1, leaf.cols());
        if self.numeric_editor.refresh_literal {
            self.refresh_numeric_literal(leaf);
        }
        if let Some(notice) = &self.numeric_editor.notice {
            ui.small(notice);
        }
        let enabled =
            enabled && self.numeric_editor.revision.is_some() && !self.numeric_editor.pending;
        let first_input = self.numeric_editor.input_frame != Some(frame);
        if first_input && !ui.input(|i| i.pointer.primary_down()) {
            self.numeric_editor.drag = false;
        }
        let (can_undo, can_redo) = self
            .numeric_editor
            .history
            .get(name)
            .map_or((false, false), |h| (!h.undo.is_empty(), !h.redo.is_empty()));
        let mut action: Option<(Action, Rect)> = None;
        let mut changed_selection = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Row");
                let row = ui.add(egui::DragValue::new(&mut self.edit_row).range(1..=leaf.rows()));
                ui.label("Column");
                let col = ui.add(egui::DragValue::new(&mut self.edit_col).range(1..=leaf.cols()));
                self.edit_row = self.edit_row.clamp(1, leaf.rows());
                self.edit_col = self.edit_col.clamp(1, leaf.cols());
                if row.changed() || col.changed() {
                    self.numeric_editor.anchor = [self.edit_row - 1, self.edit_col - 1];
                    self.refresh_numeric_literal(leaf);
                    self.cell_error=None;
                }
                let editable = if table { self.numeric_editor.table_editable[self.edit_col-1] }
                    else { variable_edit::cell_editable(leaf, self.edit_row-1, self.edit_col-1) };
                let field=ui.add_enabled(editable, egui::TextEdit::singleline(&mut self.edit_value).desired_width(180.).char_limit(text_limit));
                #[cfg(test)] {self.numeric_editor.field_rect=Some(field.rect);}
                #[cfg(not(test))] let _=field;
                if ui.add_enabled(editable, egui::Button::new("Apply cell")).clicked() && first_input {
                    action=Some((Action::Literal(self.edit_value.clone()),Rect::new(self.edit_row-1,self.edit_col-1,1,1).unwrap()));
                }
                if ui.button("Copy cell").clicked() && first_input {
                    self.numeric_editor.anchor = [self.edit_row - 1, self.edit_col - 1];
                    action = Some((Action::Copy,Rect::new(self.edit_row-1,self.edit_col-1,1,1).unwrap()));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if matches!(leaf, Array::Character(text) if text.is_text_row())
                    && ui.button("Copy text row").clicked() && first_input && action.is_none()
                {
                    action = Some((Action::CopyText, Rect::new(0, 0, 1, 1).unwrap()));
                }
                if ui.button("Copy selection").clicked() && first_input && action.is_none() {
                    match rectangle(self.numeric_editor.anchor,self.edit_row,self.edit_col) {
                        Ok(rect)=>action=Some((Action::Copy,rect)),Err(error)=>self.cell_error=Some(error),
                    }
                }
                if ui.button("Paste selection").clicked() && first_input && action.is_none() {
                    self.numeric_editor.admit(frame,Intent::RequestPaste);
                }
                if ui.add_enabled(can_undo, egui::Button::new("Undo")).clicked() && first_input {
                    self.numeric_editor.admit(frame, Intent::Undo);
                }
                if ui.add_enabled(can_redo, egui::Button::new("Redo")).clicked() && first_input {
                    self.numeric_editor.admit(frame, Intent::Redo);
                }
                ui.small("Click, Shift-click or drag selects; Ctrl+A all; F2 or double-click edits; Ctrl+C/V/Z/Y; right-click for rows and columns.");
            });
        });
        if let Some(error) = &self.cell_error {
            ui.colored_label(tokens::palette(ui.ctx()).error, error);
        }
        ui.horizontal(|ui| {
            ui.label("First visible column");
            ui.add(egui::DragValue::new(&mut self.first_col).range(1..=leaf.cols()));
        });
        self.first_col = self.first_col.clamp(1, leaf.cols());
        self.numeric_editor.anchor = [
            self.numeric_editor.anchor[0].min(leaf.rows() - 1),
            self.numeric_editor.anchor[1].min(leaf.cols() - 1),
        ];
        let grid = ui.make_persistent_id(("rooted_numeric_grid", name, target.path()));
        let _ = ui.interact(
            ui.available_rect_before_wrap(),
            grid,
            egui::Sense::focusable_noninteractive(),
        );
        // egui surrenders focus for plain Escape at the start of the pass.
        // Preserve that one event's prior grid owner, including immediately
        // after a click. A literal field or any other focused widget keeps its
        // own Escape; stale grid focus cannot claim unrelated keyboard input.
        let escape = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, modifiers, .. }
                    if !modifiers.any())
            })
        });
        let owns_focus = ui.memory(|memory| {
            let current = memory.has_focus(grid)
                || self
                    .numeric_editor
                    .focus
                    .is_some_and(|id| memory.has_focus(id));
            if self.numeric_editor.editing.is_some() {
                return false;
            }
            let escaped_grid = escape
                && memory.focused().is_none()
                && (memory.had_focus_last_frame(grid)
                    || self
                        .numeric_editor
                        .focus
                        .is_some_and(|id| memory.had_focus_last_frame(id)));
            current || escaped_grid
        });
        if let Some((action, rect)) = action.take() {
            self.numeric_editor
                .admit(frame, Intent::Submit(action, rect));
        }
        if owns_focus && !self.file_action_pending() && !self.allow_close {
            // Preserve event order. Modified system shortcuts are not navigation.
            let events = ui.input_mut(|input| {
                let mut events = Vec::new();
                let mut index = 0;
                while index < input.events.len() {
                    let take = match &input.events[index] {
                        egui::Event::Copy | egui::Event::Paste(_) | egui::Event::Cut => enabled,
                        egui::Event::Text(text) => {
                            enabled && !text.is_empty() && !text.chars().any(char::is_control)
                        }
                        egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => {
                            let plain = !modifiers.alt
                                && !modifiers.mac_cmd
                                && !modifiers.ctrl
                                && !modifiers.command;
                            let home_end = matches!(key, egui::Key::Home | egui::Key::End)
                                && !modifiers.alt
                                && !modifiers.mac_cmd
                                && (!modifiers.command || modifiers.ctrl);
                            let command =
                                (modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
                                    && !modifiers.alt;
                            let shortcut = enabled
                                && ((command
                                    && matches!(key, egui::Key::A | egui::Key::Z | egui::Key::Y))
                                    || (*key == egui::Key::F2 && !modifiers.any()));
                            (*key == egui::Key::Escape && plain)
                                || shortcut
                                || (enabled
                                    && (home_end
                                        || (plain
                                            && matches!(
                                                key,
                                                egui::Key::ArrowUp
                                                    | egui::Key::ArrowDown
                                                    | egui::Key::ArrowLeft
                                                    | egui::Key::ArrowRight
                                            ))))
                        }
                        _ => false,
                    };
                    if take {
                        events.push(input.events.remove(index));
                    } else {
                        index += 1;
                    }
                }
                events
            });
            if events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::ArrowUp
                            | egui::Key::ArrowDown
                            | egui::Key::ArrowLeft
                            | egui::Key::ArrowRight
                            | egui::Key::Home
                            | egui::Key::End,
                        ..
                    }
                )
            }) {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            }
            if first_input {
                // Escape wins over all edit/copy intents in this frame, including
                // earlier discarded passes; TextEdit-owned Escape is untouched.
                let escape = events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            ..
                        }
                    )
                });
                if escape {
                    if self.numeric_editor.pending {
                        self.worker.cancel.store(true, Ordering::Relaxed);
                    }
                    self.numeric_editor.editor = Token::default();
                    self.numeric_editor.pending = false;
                    self.numeric_editor.intent = None;
                    self.numeric_editor.clipboard = None;
                    self.numeric_editor.anchor = [self.edit_row - 1, self.edit_col - 1];
                    self.numeric_editor.notice = Some(
                        "Selection and edit text reset; pending clipboard results discarded."
                            .into(),
                    );
                    self.numeric_editor.action_frame = Some(frame);
                    changed_selection = true;
                } else {
                    for event in events {
                        match event {
                            egui::Event::Copy | egui::Event::Paste(_) => {
                                let action = match event {
                                    egui::Event::Paste(text) => {
                                        if text.len() > variable_edit::MAX_TEXT_BYTES {
                                            self.cell_error =
                                                Some("Clipboard text exceeds 8 MiB".into());
                                            continue;
                                        }
                                        Action::Paste(text)
                                    }
                                    _ => Action::Copy,
                                };
                                match rectangle(
                                    self.numeric_editor.anchor,
                                    self.edit_row,
                                    self.edit_col,
                                ) {
                                    Ok(rect) => self
                                        .numeric_editor
                                        .admit(frame, Intent::Submit(action, rect)),
                                    Err(error) => self.cell_error = Some(error),
                                }
                            }
                            egui::Event::Cut => {
                                self.cell_error = Some(
                                    "Cut is unavailable; copy and paste preserve array dimensions."
                                        .into(),
                                )
                            }
                            egui::Event::Text(text) => {
                                if matches!(leaf, Array::Character(_))
                                    && single_utf16_unit(&text).is_none()
                                {
                                    self.cell_error = Some(CHAR_CELL_UNIT_ERROR.into());
                                    // The error label is drawn on the next frame and shifts
                                    // auto-ids, so the focused cell button disappears. Keep
                                    // the stable grid id so the next BMP keystroke still edits.
                                    ui.memory_mut(|memory| memory.request_focus(grid));
                                    self.numeric_editor.focus = Some(grid);
                                } else {
                                    self.begin_cell_edit(leaf, Some(text), frame);
                                }
                            }
                            egui::Event::Key {
                                key: egui::Key::A, ..
                            } => {
                                self.numeric_editor.anchor = [0, 0];
                                self.edit_row = leaf.rows();
                                self.edit_col = leaf.cols();
                                ui.memory_mut(|m| m.request_focus(grid));
                                self.numeric_editor.focus = Some(grid);
                                changed_selection = true;
                            }
                            egui::Event::Key {
                                key: egui::Key::Z,
                                modifiers,
                                ..
                            } => self.numeric_editor.admit(
                                frame,
                                if modifiers.shift {
                                    Intent::Redo
                                } else {
                                    Intent::Undo
                                },
                            ),
                            egui::Event::Key {
                                key: egui::Key::Y, ..
                            } => self.numeric_editor.admit(frame, Intent::Redo),
                            egui::Event::Key {
                                key: egui::Key::F2, ..
                            } => self.begin_cell_edit(leaf, None, frame),
                            egui::Event::Key { key, modifiers, .. } => {
                                let mut row = self.edit_row - 1;
                                let mut col = self.edit_col - 1;
                                match key {
                                    egui::Key::ArrowUp => row = row.saturating_sub(1),
                                    egui::Key::ArrowDown => row = (row + 1).min(leaf.rows() - 1),
                                    egui::Key::ArrowLeft => col = col.saturating_sub(1),
                                    egui::Key::ArrowRight => col = (col + 1).min(leaf.cols() - 1),
                                    egui::Key::Home => {
                                        col = 0;
                                        if modifiers.ctrl {
                                            row = 0;
                                        }
                                    }
                                    egui::Key::End => {
                                        col = leaf.cols() - 1;
                                        if modifiers.ctrl {
                                            row = leaf.rows() - 1;
                                        }
                                    }
                                    _ => {}
                                }
                                self.edit_row = row + 1;
                                self.edit_col = col + 1;
                                if !modifiers.shift {
                                    self.numeric_editor.anchor = [row, col];
                                }
                                self.first_col = col.saturating_sub(7) + 1;
                                self.numeric_editor.scroll_row = Some(row);
                                ui.memory_mut(|m| {
                                    m.move_focus(egui::FocusDirection::None);
                                    m.request_focus(grid);
                                });
                                self.numeric_editor.focus = Some(grid);
                                changed_selection = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        self.numeric_editor.input_frame = Some(frame);
        let columns = self.numeric_editor.view.as_ref().map_or_else(
            || self.first_col - 1..(self.first_col + 7).min(leaf.cols()),
            |view| view.columns.clone(),
        );
        let anchor = self.numeric_editor.anchor;
        let end = [self.edit_row - 1, self.edit_col - 1];
        let sel: Bounds = [
            anchor[0].min(end[0]),
            anchor[1].min(end[1]),
            anchor[0].max(end[0]),
            anchor[1].max(end[1]),
        ];
        let (rows_n, cols_n) = (leaf.rows(), leaf.cols());
        let editing = self.numeric_editor.editing;
        let dragging = self.numeric_editor.drag;
        let shift_now = ui.input(|i| i.modifiers.shift);
        let palette = tokens::palette(ui.ctx());
        let editor_id = ui.make_persistent_id(("cell_editor", name, target.path()));
        let mut row_clicked: Option<(usize, bool)> = None;
        let mut col_clicked: Option<(usize, bool)> = None;
        let mut menu_pick: Option<(Menu, Bounds)> = None;
        let mut select_bounds: Option<Bounds> = None;
        let mut pressed: Option<(usize, usize, bool)> = None;
        let mut drag_over: Option<(usize, usize)> = None;
        let mut double_clicked: Option<(usize, usize)> = None;
        let mut editor_drawn = false;
        let selected = |row: usize, col: usize| {
            (anchor[0].min(end[0])..=anchor[0].max(end[0])).contains(&row)
                && (anchor[1].min(end[1])..=anchor[1].max(end[1])).contains(&col)
        };
        // Rows hold monospace cell buttons; derive the stride from the actual
        // button height so the scroll math matches the painted rows.
        let row_height = (ui.text_style_height(&egui::TextStyle::Monospace)
            + 2. * ui.spacing().button_padding.y)
            .max(ui.spacing().interact_size.y)
            .max(22.)
            .ceil();
        ui.horizontal(|ui| {
            ui.add_sized([55., row_height], egui::Label::new("Row"));
            for col in columns.clone() {
                let whole = sel[0] == 0 && sel[2] + 1 == rows_n && (sel[1]..=sel[3]).contains(&col);
                let bounds = if whole {
                    sel
                } else {
                    [0, col, rows_n - 1, col]
                };
                let response = ui
                    .push_id(("col_header", col), |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.add_sized(
                                [90., row_height],
                                egui::Button::new(match leaf {
                                    Array::Table(t) => t.names()[col].clone(),
                                    _ => (col + 1).to_string(),
                                })
                                .selected(whole)
                                .truncate(),
                            )
                        })
                        .inner
                    })
                    .inner;
                #[cfg(test)]
                self.numeric_editor.col_headers.insert(col, response.rect);
                if response.secondary_clicked() && first_input {
                    select_bounds = Some(bounds);
                }
                if let Some(pick) = context_menu(&response, table) {
                    menu_pick = Some((pick, bounds));
                }
                if response.clicked() && first_input {
                    col_clicked = Some((col, shift_now));
                }
            }
        });
        ui.separator();
        let stride = row_height + ui.spacing().item_spacing.y;
        // show_rows uses floor(min), ceil(max)+1: reserve two overscan rows.
        // 480 grid formats leave room for field/tooltip work without truncating
        // a requested visible row. Tall windows retain a bounded scroll viewport.
        let row_capacity = 480 / columns.len();
        let height = self.numeric_editor.view.as_ref().map_or(
            ui.available_height()
                .min((row_capacity - 2) as f32 * stride)
                .max(0.),
            |v| v.height,
        );
        let replay = self.numeric_editor.view.is_some();
        // Dragging over cells selects; it must not scroll the area.
        let mut scroll = egui::ScrollArea::both()
            .id_salt(("rooted_numeric_rows", name, target.path()))
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            )
            .min_scrolled_height(0.)
            .max_height(height);
        if let Some(view) = &self.numeric_editor.view {
            scroll = scroll
                .scroll_offset(view.offset)
                .scroll_source(egui::scroll_area::ScrollSource::NONE);
        } else {
            if let Some(offset) = self.numeric_editor.resume_offset.take() {
                scroll = scroll.scroll_offset(offset);
            }
            if let Some(row) = self.numeric_editor.scroll_row.take() {
                scroll = scroll.vertical_scroll_offset(row as f32 * stride);
            }
        }
        let mut clicked = None;
        let output = scroll.show_viewport_themed(ui, |ui, viewport| {
            if !replay {
                self.numeric_editor.view = Some(GridView {
                    columns: columns.clone(),
                    height,
                    offset: viewport.min.to_vec2(),
                    next_offset: viewport.min.to_vec2(),
                });
            }
            ui.set_height((stride * leaf.rows() as f32 - ui.spacing().item_spacing.y).max(0.));
            let mut start = (viewport.min.y / stride).floor() as usize;
            let mut end = (viewport.max.y / stride).ceil() as usize + 1;
            if end > leaf.rows() {
                let count = end.saturating_sub(start);
                end = leaf.rows();
                start = end.saturating_sub(count);
            }
            let rect = egui::Rect::from_x_y_ranges(
                ui.max_rect().x_range(),
                ui.max_rect().top() + start as f32 * stride
                    ..=ui.max_rect().top() + end as f32 * stride,
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.skip_ahead_auto_ids(start);
                for row in start..end {
                    ui.horizontal(|ui| {
                        let whole =
                            sel[1] == 0 && sel[3] + 1 == cols_n && (sel[0]..=sel[2]).contains(&row);
                        let bounds = if whole {
                            sel
                        } else {
                            [row, 0, row, cols_n - 1]
                        };
                        let header = ui
                            .push_id(("row_header", row), |ui| {
                                ui.add_enabled_ui(enabled, |ui| {
                                    ui.add_sized(
                                        [55., row_height],
                                        egui::Button::new(match leaf {
                                            Array::Table(t) => t.row_names().map_or_else(
                                                || (row + 1).to_string(),
                                                |names| names[row].clone(),
                                            ),
                                            _ => (row + 1).to_string(),
                                        })
                                        .selected(whole)
                                        .truncate(),
                                    )
                                })
                                .inner
                            })
                            .inner;
                        #[cfg(test)]
                        self.numeric_editor.row_headers.insert(row, header.rect);
                        if header.secondary_clicked() && first_input {
                            select_bounds = Some(bounds);
                        }
                        if let Some(pick) = context_menu(&header, table) {
                            menu_pick = Some((pick, bounds));
                        }
                        if header.clicked() && first_input {
                            row_clicked = Some((row, shift_now));
                        }
                        for col in columns.clone() {
                            if editing == Some([row, col]) {
                                editor_drawn = true;
                                if std::mem::take(&mut self.numeric_editor.edit_focus) {
                                    let at = self.edit_value.chars().count();
                                    let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id)
                                        .unwrap_or_default();
                                    state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::one(egui::text::CCursor::new(at)),
                                    ));
                                    egui::TextEdit::store_state(ui.ctx(), editor_id, state);
                                    ui.memory_mut(|m| m.request_focus(editor_id));
                                }
                                let field = ui
                                    .add_enabled_ui(enabled, |ui| {
                                        ui.add_sized(
                                            [90., row_height],
                                            egui::TextEdit::singleline(&mut self.edit_value)
                                                .id(editor_id)
                                                .font(egui::TextStyle::Monospace)
                                                .char_limit(text_limit),
                                        )
                                    })
                                    .inner;
                                #[cfg(test)]
                                self.numeric_editor.cells.insert((row, col), field.rect);
                                if field.lost_focus() && first_input {
                                    let (escape, enter) = ui.input(|i| {
                                        (
                                            i.key_pressed(egui::Key::Escape),
                                            i.key_pressed(egui::Key::Enter),
                                        )
                                    });
                                    self.numeric_editor.editing = None;
                                    let same = variable_edit::cell_literal(leaf, row, col)
                                        .is_ok_and(|old| old == self.edit_value);
                                    if escape || same {
                                        self.numeric_editor.refresh_literal = true;
                                    } else {
                                        self.numeric_editor.admit(
                                            frame,
                                            Intent::Submit(
                                                Action::Literal(self.edit_value.clone()),
                                                Rect::new(row, col, 1, 1).unwrap(),
                                            ),
                                        );
                                    }
                                    if escape || enter {
                                        ui.memory_mut(|m| m.request_focus(grid));
                                        self.numeric_editor.focus = Some(grid);
                                    }
                                }
                                continue;
                            }
                            let text =
                                if let Some(text) = self.numeric_editor.display.get(&(row, col)) {
                                    text.clone()
                                } else {
                                    let text = if matches!(
                                        leaf,
                                        Array::Character(_)
                                            | Array::Str(_)
                                            | Array::Cell(_)
                                            | Array::Table(_)
                                    ) {
                                        variable_edit::cell_preview(leaf, row, col).unwrap()
                                    } else {
                                        format_scalar_cell(leaf.scalar(row, col).unwrap())
                                    };
                                    self.numeric_editor.formatted += 1;
                                    self.numeric_editor.display.insert((row, col), text.clone());
                                    text
                                };
                            let response = ui
                                .push_id((row, col), |ui| {
                                    ui.add_enabled_ui(enabled, |ui| {
                                        ui.add_sized(
                                            [90., row_height],
                                            egui::Button::new(
                                                RichText::new(
                                                    text.chars().take(128).collect::<String>(),
                                                )
                                                .monospace(),
                                            )
                                            // Claim drags so selecting does not move the window.
                                            .sense(egui::Sense::click_and_drag())
                                            .truncate()
                                            .selected(selected(row, col)),
                                        )
                                    })
                                    .inner
                                })
                                .inner;
                            #[cfg(test)]
                            self.numeric_editor.cells.insert((row, col), response.rect);
                            if [row, col] == anchor {
                                ui.painter().rect_stroke(
                                    response.rect.shrink(1.),
                                    0.,
                                    egui::Stroke::new(2., palette.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let bounds = if (sel[0]..=sel[2]).contains(&row)
                                && (sel[1]..=sel[3]).contains(&col)
                            {
                                sel
                            } else {
                                [row, col, row, col]
                            };
                            if response.secondary_clicked() && first_input {
                                select_bounds = Some(bounds);
                            }
                            if let Some(pick) = context_menu(&response, table) {
                                menu_pick = Some((pick, bounds));
                            }
                            if first_input && enabled && response.contains_pointer() {
                                let (down, press) = ui.input(|i| {
                                    (i.pointer.primary_down(), i.pointer.primary_pressed())
                                });
                                if press {
                                    response.request_focus();
                                    self.numeric_editor.focus = Some(response.id);
                                    pressed = Some((row, col, shift_now));
                                } else if down && dragging {
                                    drag_over = Some((row, col));
                                }
                            }
                            if response.double_clicked() && first_input {
                                double_clicked = Some((row, col));
                            }
                            let response = response.on_hover_ui(|ui| {
                                if self
                                    .numeric_editor
                                    .tooltip
                                    .as_ref()
                                    .is_none_or(|(cell, _)| *cell != (row, col))
                                    && self.numeric_editor.formatted < 512
                                {
                                    self.numeric_editor.formatted += 1;
                                    self.numeric_editor.tooltip = Some((
                                        (row, col),
                                        variable_edit::cell_literal(leaf, row, col).unwrap(),
                                    ));
                                }
                                if let Some((cell, text)) = &self.numeric_editor.tooltip
                                    && *cell == (row, col)
                                {
                                    ui.add(
                                        egui::Label::new(RichText::new(text).monospace()).wrap(),
                                    );
                                }
                            });
                            if response.has_focus() {
                                self.numeric_editor.focus = Some(response.id);
                                ui.memory_mut(|m| {
                                    m.set_focus_lock_filter(
                                        response.id,
                                        egui::EventFilter {
                                            horizontal_arrows: true,
                                            vertical_arrows: true,
                                            ..Default::default()
                                        },
                                    )
                                });
                            }
                            if response.clicked() && first_input {
                                response.request_focus();
                                self.numeric_editor.focus = Some(response.id);
                                clicked = Some((row, col, ui.input(|i| i.modifiers.shift)));
                            }
                        }
                    });
                }
            });
        });
        #[cfg(test)]
        {
            self.numeric_editor.grid_clip = Some(output.inner_rect.intersect(ui.clip_rect()));
        }
        if !replay && let Some(view) = &mut self.numeric_editor.view {
            view.next_offset = output.state.offset;
        }
        debug_assert!(self.numeric_editor.formatted <= 512);
        if let Some((row, col, extend)) = clicked {
            self.edit_row = row + 1;
            self.edit_col = col + 1;
            if !extend {
                self.numeric_editor.anchor = [row, col];
            }
            changed_selection = true;
        }
        let set = |this: &mut Self, b: Bounds| {
            this.numeric_editor.anchor = [b[0], b[1]];
            this.edit_row = b[2] + 1;
            this.edit_col = b[3] + 1;
        };
        if let Some((row, col, shift)) = pressed {
            self.numeric_editor.drag = true;
            if shift {
                self.edit_row = row + 1;
                self.edit_col = col + 1;
            } else {
                set(self, [row, col, row, col]);
            }
            changed_selection = true;
        } else if let Some((row, col)) = drag_over
            && (row + 1, col + 1) != (self.edit_row, self.edit_col)
        {
            self.edit_row = row + 1;
            self.edit_col = col + 1;
            changed_selection = true;
        }
        if let Some((row, shift)) = row_clicked {
            let top = if shift {
                self.numeric_editor.anchor[0]
            } else {
                row
            };
            set(self, [top, 0, row, cols_n - 1]);
            self.numeric_editor.anchor[0] = top;
            ui.memory_mut(|m| m.request_focus(grid));
            self.numeric_editor.focus = Some(grid);
            changed_selection = true;
        }
        if let Some((col, shift)) = col_clicked {
            let left = if shift {
                self.numeric_editor.anchor[1]
            } else {
                col
            };
            set(self, [0, left, rows_n - 1, col]);
            self.numeric_editor.anchor[1] = left;
            ui.memory_mut(|m| m.request_focus(grid));
            self.numeric_editor.focus = Some(grid);
            changed_selection = true;
        }
        if let Some(bounds) = select_bounds {
            set(self, bounds);
            changed_selection = true;
        }
        if let Some((row, col)) = double_clicked {
            set(self, [row, col, row, col]);
            self.begin_cell_edit(leaf, None, frame);
        }
        if let Some((item, b)) = menu_pick.filter(|_| first_input) {
            let (count_rows, count_cols) = (b[2] - b[0] + 1, b[3] - b[1] + 1);
            let shape = |shape| Some(Action::Reshape(shape));
            let action = match item {
                Menu::Copy => Rect::new(b[0], b[1], count_rows, count_cols)
                    .map(|rect| (Action::Copy, rect))
                    .map_err(|e| self.cell_error = Some(e))
                    .ok(),
                Menu::Paste => {
                    self.numeric_editor.admit(frame, Intent::RequestPaste);
                    None
                }
                Menu::RowAbove => shape(variable_edit::Shape::InsertRows {
                    at: b[0],
                    count: count_rows,
                })
                .map(|a| (a, Rect::new(0, 0, 1, 1).unwrap())),
                Menu::RowBelow => shape(variable_edit::Shape::InsertRows {
                    at: b[2] + 1,
                    count: count_rows,
                })
                .map(|a| (a, Rect::new(0, 0, 1, 1).unwrap())),
                Menu::ColLeft => shape(variable_edit::Shape::InsertCols {
                    at: b[1],
                    count: count_cols,
                })
                .map(|a| (a, Rect::new(0, 0, 1, 1).unwrap())),
                Menu::ColRight => shape(variable_edit::Shape::InsertCols {
                    at: b[3] + 1,
                    count: count_cols,
                })
                .map(|a| (a, Rect::new(0, 0, 1, 1).unwrap())),
                Menu::DeleteRows => shape(variable_edit::Shape::DeleteRows {
                    at: b[0],
                    count: count_rows,
                })
                .map(|a| (a, Rect::new(0, 0, 1, 1).unwrap())),
                Menu::DeleteCols => shape(variable_edit::Shape::DeleteCols {
                    at: b[1],
                    count: count_cols,
                })
                .map(|a| (a, Rect::new(0, 0, 1, 1).unwrap())),
            };
            if let Some((action, rect)) = action {
                self.numeric_editor
                    .admit(frame, Intent::Submit(action, rect));
            }
        }
        if first_input && editing.is_some() && !editor_drawn {
            self.numeric_editor.editing = None;
            self.numeric_editor.refresh_literal = true;
        }
        if changed_selection && self.numeric_editor.editing.is_none() {
            self.refresh_numeric_literal(leaf);
            self.cell_error = None;
        }
        if self.numeric_editor.refresh_literal {
            ui.ctx().request_repaint();
        }
        self.finish_variable_intent(ui, target, enabled, frame, grid);
    }
    fn finish_variable_intent(
        &mut self,
        ui: &mut egui::Ui,
        target: Target,
        enabled: bool,
        frame: u64,
        grid: egui::Id,
    ) {
        if enabled && !ui.ctx().will_discard() && self.numeric_editor.action_frame != Some(frame) {
            if let Some((intent_frame, generation, intent)) = self.numeric_editor.intent.take()
                && intent_frame == frame
                && generation.same(&self.numeric_editor.editor)
            {
                self.numeric_editor.action_frame = Some(frame);
                match intent {
                    Intent::RequestPaste => {
                        ui.memory_mut(|m| m.request_focus(grid));
                        self.numeric_editor.focus = Some(grid);
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    }
                    Intent::Undo | Intent::Redo => {
                        let undo = matches!(intent, Intent::Undo);
                        let entry = self
                            .numeric_editor
                            .history
                            .get(target.name())
                            .and_then(|h| {
                                if undo { h.undo.last() } else { h.redo.last() }.cloned()
                            });
                        match entry {
                            None => {
                                self.cell_error = Some(format!(
                                    "Nothing to {}",
                                    if undo { "undo" } else { "redo" }
                                ))
                            }
                            Some(entry) if !entry.from.same_snapshot(target.expected_root()) => {
                                self.numeric_editor.history.remove(target.name());
                                self.cell_error = Some(format!(
                                    "{} refused: {} was changed outside the editor",
                                    if undo { "Undo" } else { "Redo" },
                                    target.name()
                                ));
                            }
                            Some(entry) => {
                                let rectangle = Rect::new(0, 0, 1, 1).unwrap();
                                let action = Action::Restore(entry.to.clone());
                                let inflight = if undo {
                                    Inflight::Undo(entry)
                                } else {
                                    Inflight::Redo(entry)
                                };
                                self.submit_variable_request(
                                    ui, target, rectangle, action, inflight, grid,
                                );
                            }
                        }
                    }
                    Intent::Submit(action, rectangle) => {
                        let inflight = match &action {
                            Action::Copy | Action::CopyText => Inflight::Read,
                            _ => Inflight::Edit(target.expected_root().clone()),
                        };
                        self.submit_variable_request(ui, target, rectangle, action, inflight, grid);
                    }
                }
            }
            if !self.busy
                && let Some(text) = self.numeric_editor.clipboard.take()
            {
                if !self.worker.cancel.load(Ordering::Relaxed) {
                    ui.ctx().copy_text(text);
                } else {
                    self.cell_error = Some("Copy cancelled".into());
                }
            }
        }
    }
    fn submit_variable_request(
        &mut self,
        ui: &mut egui::Ui,
        target: Target,
        rectangle: Rect,
        action: Action,
        inflight: Inflight,
        grid: egui::Id,
    ) {
        let request = Request {
            revision: self.numeric_editor.revision.clone().unwrap(),
            editor: self.numeric_editor.editor.clone(),
            target,
            rectangle,
            action,
        };
        self.submit(Job::RootedVariable(Box::new(request)));
        self.numeric_editor.pending = self.busy;
        self.numeric_editor.inflight = inflight;
        if self.busy {
            // Cells have layout-dependent widget IDs and become disabled while a
            // worker runs. Keep keyboard ownership on the persistent grid before
            // either transition. Do not steal focus from the literal field or toolbar.
            let cell_focus = self.numeric_editor.focus;
            let retained = ui.memory_mut(|memory| {
                if memory.has_focus(grid) || cell_focus.is_some_and(|id| memory.has_focus(id)) {
                    memory.request_focus(grid);
                    true
                } else {
                    false
                }
            });
            if retained {
                self.numeric_editor.focus = Some(grid);
            }
            self.cell_error = None;
        } else {
            self.cell_error = Some(self.status.clone());
        }
    }
}
