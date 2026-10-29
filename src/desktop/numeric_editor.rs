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
    tooltip: Option<((usize, usize), String)>,
    refresh_literal: bool,
    input_frame: Option<u64>,
    view: Option<GridView>,
    resume_offset: Option<egui::Vec2>,
    defer_frame: Option<u64>,
    intent: Option<(u64, Token, Intent)>,
    pub(super) formatted: usize,
    #[cfg(test)]
    pub(super) cells: BTreeMap<(usize, usize), egui::Rect>,
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
        *self = Self { revision, format_frame, formatted, notice: explain.then(||
            "Workspace operation completed; selection and edit text were refreshed. Review the value before applying.".into()), ..Default::default() };
    }
    fn admit(&mut self, frame: u64, intent: Intent) {
        // First submit wins within an ordered frame, including discarded passes.
        if self.intent.is_none() && self.action_frame != Some(frame) {
            self.intent = Some((frame, self.editor.clone(), intent));
        }
    }
    fn matches(&self, target: &Target) -> bool {
        self.target.as_ref().is_some_and(|old| {
            old.name() == target.name()
                && old.path() == target.path()
                && old.expected_root().same_snapshot(target.expected_root())
        })
    }
}

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
                variable_edit::cell_literal(leaf, self.edit_row - 1, self.edit_col - 1).unwrap();
            self.numeric_editor.formatted += 1;
            self.numeric_editor.refresh_literal = false;
        } else {
            self.numeric_editor.refresh_literal = true;
        }
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
        let target = match Target::new(name.into(), root.clone(), path) {
            Ok(target) => target,
            Err(error) => {
                ui.label(error);
                return;
            }
        };
        let frame = ui.ctx().cumulative_frame_nr();
        if !self.numeric_editor.matches(&target) {
            let defer = self.numeric_editor.format_frame == Some(frame)
                && self.numeric_editor.formatted > 0;
            let notice = self.numeric_editor.notice.take();
            self.numeric_editor.invalidate(false);
            self.numeric_editor.notice = notice;
            self.numeric_editor.target = Some(target.clone());
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
            ui.label(if matches!(leaf, Array::Character(_)) {
                "Empty character array; no editable code units."
            } else {
                "Empty matrix"
            });
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
            if let Some(error) = &self.cell_error {
                ui.colored_label(tokens::palette(ui.ctx()).error, error);
            }
            let grid = ui.make_persistent_id(("rooted_numeric_grid", name, target.path()));
            self.finish_variable_intent(ui, target, enabled, frame, grid);
            return;
        }
        if matches!(leaf, Array::Character(_)) {
            ui.small("UTF-16 code units: \\uXXXX, 0xXXXX or decimal 0..65535. Copy/paste uses escaped units; shape stays fixed.");
        }
        if self.numeric_editor.refresh_literal {
            self.refresh_numeric_literal(leaf);
        }
        if let Some(notice) = &self.numeric_editor.notice {
            ui.small(notice);
        }
        let enabled =
            enabled && self.numeric_editor.revision.is_some() && !self.numeric_editor.pending;
        let first_input = self.numeric_editor.input_frame != Some(frame);
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
                let field=ui.add(egui::TextEdit::singleline(&mut self.edit_value).desired_width(180.).char_limit(256));
                #[cfg(test)] {self.numeric_editor.field_rect=Some(field.rect);}
                #[cfg(not(test))] let _=field;
                if ui.button("Apply cell").clicked() && first_input {
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
                ui.small("Shift-click or Shift+arrows selects a rectangle. Ctrl+C / Ctrl+V acts on the focused grid.");
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
                            (*key == egui::Key::Escape && plain)
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
                ui.add_sized([90., row_height], egui::Label::new((col + 1).to_string()));
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
        let mut scroll = egui::ScrollArea::both()
            .id_salt(("rooted_numeric_rows", name, target.path()))
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
                        ui.add_sized([55., row_height], egui::Label::new((row + 1).to_string()));
                        for col in columns.clone() {
                            let text =
                                if let Some(text) = self.numeric_editor.display.get(&(row, col)) {
                                    text.clone()
                                } else {
                                    let text = if matches!(leaf, Array::Character(_)) {
                                        variable_edit::cell_literal(leaf, row, col).unwrap()
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
                                            egui::Button::new(RichText::new(text).monospace())
                                                .truncate()
                                                .selected(selected(row, col)),
                                        )
                                    })
                                    .inner
                                })
                                .inner;
                            #[cfg(test)]
                            self.numeric_editor.cells.insert((row, col), response.rect);
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
        if changed_selection {
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
                    Intent::Submit(action, rectangle) => {
                        let request = Request {
                            revision: self.numeric_editor.revision.clone().unwrap(),
                            editor: self.numeric_editor.editor.clone(),
                            target,
                            rectangle,
                            action,
                        };
                        self.submit(Job::RootedVariable(Box::new(request)));
                        self.numeric_editor.pending = self.busy;
                        if self.busy {
                            // Cells have layout-dependent widget IDs and become
                            // disabled while a worker runs. Keep keyboard ownership
                            // on the persistent grid before either transition.
                            // Do not steal focus from the literal field or toolbar.
                            let cell_focus = self.numeric_editor.focus;
                            let retained = ui.memory_mut(|memory| {
                                if memory.has_focus(grid)
                                    || cell_focus.is_some_and(|id| memory.has_focus(id))
                                {
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
}
