//! Native, bounded connection editing; simulator storage and execution stay independent.
use super::{Model, egui};
use crate::theme::ThemedScrollArea;
use crate::tokens;
use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::VecDeque;

const HISTORY_LIMIT: usize = 128;
const HISTORY_BYTES: usize = 4 * 1024 * 1024;
enum Height {
    #[cfg(test)]
    Legacy(f32),
    FillPane,
}

#[derive(Clone, Copy)]
struct Wire {
    target: usize,
    port: usize,
    before: usize,
    after: usize,
}

#[path = "model_clipboard.rs"]
mod clipboard;
#[path = "model_clipboard_ui.rs"]
mod clipboard_ui;
#[path = "model_group_delete.rs"]
mod group_delete;
#[path = "model_keyboard.rs"]
mod keyboard;
#[path = "model_layout.rs"]
mod layout;
#[path = "model_organize.rs"]
mod organize;
#[path = "model_parameters.rs"]
pub(super) mod parameters;
#[path = "model_selection.rs"]
mod selection;
#[path = "model_topology.rs"]
mod topology;
#[path = "model_validation.rs"]
pub(super) mod validation;
#[path = "model_view.rs"]
mod view;
enum Change {
    Wire(Wire),
    Property {
        edit: parameters::Edit,
        epoch: u64,
    },
    Coefficients(Box<parameters::Coefficients>),
    Topology(Box<topology::Change>),
    GroupDelete(Box<group_delete::Change>),
    Paste(Box<clipboard::Change>),
    Layout {
        before: layout::Snapshot,
        after: layout::Snapshot,
    },
}
impl Change {
    fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + match self {
                Self::Wire(_) => 0,
                Self::Property { edit, .. } => edit.heap_bytes(),
                Self::Coefficients(change) => change.retained_bytes(),
                Self::Topology(change) => change.retained_bytes(),
                Self::GroupDelete(change) => change.retained_bytes(),
                Self::Paste(change) => change.retained_bytes(),
                Self::Layout { before, after } => {
                    (before.positions.capacity() + after.positions.capacity())
                        * size_of::<[f32; 2]>()
                }
            }
    }
}

#[derive(Default)]
pub(super) struct Canvas {
    selection: selection::Selection,
    view: view::View,
    last_paste: Option<(blake3::Hash, u32)>,
    changed: bool,
    source: Option<usize>,
    moving: Option<layout::Moving>,
    marquee: Option<organize::Marquee>,
    reveal: Option<usize>,
    reveal_port: Option<usize>,
    dragging: bool,
    keyboard_wiring: bool,
    keyboard_port: usize,
    nudge: Option<(usize, [i8; 2])>,
    history: VecDeque<Change>,
    history_bytes: usize,
    cursor: usize,
    last_input_frame: Option<u64>,
    edit_group: Option<egui::Id>,
    edit_epoch: u64,
    #[cfg(test)]
    ports: Vec<(usize, Option<usize>, Pos2)>,
    #[cfg(test)]
    blocks: Vec<(usize, Rect)>,
}

impl Canvas {
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
    pub fn is_selected(&self, index: usize) -> bool {
        self.selection.contains(index)
    }
    pub fn selection_len(&self) -> usize {
        self.selection.indices().len()
    }
    pub fn select(
        &mut self,
        index: usize,
        modifiers: egui::Modifiers,
        count: usize,
        primary: &mut usize,
    ) {
        if count > 512 {
            return;
        }
        self.nudge = None;
        self.selection.click(index, modifiers, count, primary);
    }
    pub fn delete_selection(
        &mut self,
        model: &mut Model,
        selected: &mut usize,
    ) -> matrust::Result<()> {
        self.selection.sync(model.nodes.len(), selected);
        let indices = self.selection.indices().to_vec();
        let Some(&first) = indices.first() else {
            return Ok(());
        };
        if indices.len() == 1 {
            *selected = self.delete(model, first)?;
        } else {
            let change =
                group_delete::Change::delete(model, &indices, HISTORY_BYTES - size_of::<Change>())?;
            self.cancel_connection();
            self.push(Change::GroupDelete(Box::new(change)));
            *selected = first.min(model.nodes.len().saturating_sub(1));
            self.reveal(*selected);
        }
        self.selection.replace(
            model.nodes.len(),
            if *selected < model.nodes.len() {
                vec![*selected]
            } else {
                vec![]
            },
            selected,
        );
        Ok(())
    }
    pub fn cancel_connection(&mut self) {
        self.source = None;
        self.dragging = false;
        self.moving = None;
        self.marquee = None;
        self.view.pan = None;
        self.keyboard_wiring = false;
        self.keyboard_port = 0;
        self.nudge = None;
    }

    pub fn reveal(&mut self, node: usize) {
        self.reveal = Some(node);
        self.reveal_port = None;
    }

    fn reveal_input(&mut self, node: usize, port: usize) {
        self.reveal(node);
        self.reveal_port = Some(port);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn record(&mut self, target: usize, port: usize, before: usize, after: usize) {
        if before == after {
            return;
        }
        self.push(Change::Wire(Wire {
            target,
            port,
            before,
            after,
        }));
    }
    fn push(&mut self, change: Change) {
        self.changed = true;
        self.nudge = None;
        self.edit_group = None;
        while self.history.len() > self.cursor {
            self.history_bytes -= self.history.pop_back().unwrap().retained_bytes();
        }
        let bytes = change.retained_bytes();
        debug_assert!(bytes <= HISTORY_BYTES);
        while self.history.len() >= HISTORY_LIMIT || self.history_bytes + bytes > HISTORY_BYTES {
            self.history_bytes -= self.history.pop_front().unwrap().retained_bytes();
        }
        self.history_bytes += bytes;
        self.history.push_back(change);
        self.cursor = self.history.len();
    }
    pub fn finish_edit(&mut self, active: Option<egui::Id>) {
        if active.is_some() && active != self.edit_group {
            self.edit_epoch = self.edit_epoch.wrapping_add(1);
        }
        self.edit_group = active;
    }
    pub fn record_edit(&mut self, mut edit: parameters::Edit, group: Option<egui::Id>) {
        self.changed = true;
        self.finish_edit(group);
        if group.is_some()
            && self.cursor == self.history.len()
            && let Some(Change::Property {
                edit: previous,
                epoch,
            }) = self.history.back_mut()
            && *epoch == self.edit_epoch
        {
            let old_bytes = previous.heap_bytes();
            match previous.merge(edit) {
                Some(next) => edit = next,
                None => {
                    self.history_bytes = self.history_bytes - old_bytes + previous.heap_bytes();
                    if previous.unchanged() {
                        self.history_bytes -= self.history.pop_back().unwrap().retained_bytes();
                    }
                    while self.history_bytes > HISTORY_BYTES {
                        self.history_bytes -= self.history.pop_front().unwrap().retained_bytes();
                    }
                    self.cursor = self.history.len();
                    return;
                }
            }
        }
        self.push(Change::Property {
            edit,
            epoch: self.edit_epoch,
        });
        self.edit_group = group;
    }
    pub fn replace_coefficients(
        &mut self,
        model: &mut Model,
        index: usize,
        block: matrust::simulation::Block,
    ) -> matrust::Result<()> {
        if let Some(change) = parameters::Coefficients::replace(model, index, block)? {
            self.push(Change::Coefficients(Box::new(change)));
        }
        Ok(())
    }
    pub fn insert(
        &mut self,
        model: &mut Model,
        node: matrust::simulation::Node,
    ) -> matrust::Result<usize> {
        let change = topology::Change::insert(model, node)?;
        let index = change.index;
        self.cancel_connection();
        self.push(Change::Topology(Box::new(change)));
        self.reveal(index);
        Ok(index)
    }
    pub fn delete(&mut self, model: &mut Model, index: usize) -> matrust::Result<usize> {
        let change = topology::Change::delete(model, index)?;
        self.cancel_connection();
        self.push(Change::Topology(Box::new(change)));
        let selected = index.min(model.nodes.len().saturating_sub(1));
        self.reveal(selected);
        Ok(selected)
    }
    fn arrange(&mut self, model: &mut Model, selected: usize) {
        let before = layout::Snapshot::capture(model);
        if !model.positions.is_empty() && model.set_layout(Vec::new()).is_ok() {
            self.push(Change::Layout {
                before,
                after: layout::Snapshot::capture(model),
            });
            self.cancel_connection();
            self.reveal(selected);
        }
    }

    fn connect(&mut self, model: &mut Model, target: usize, port: usize, source: usize) -> bool {
        if source != usize::MAX && source >= model.nodes.len() {
            return false;
        }
        let Some(input) = model
            .nodes
            .get_mut(target)
            .and_then(|node| node.inputs.get_mut(port))
        else {
            return false;
        };
        let previous = *input;
        *input = source;
        self.record(target, port, previous, source);
        previous != source
    }

    fn travel(
        &mut self,
        model: &mut Model,
        selected: &mut usize,
        redo: bool,
        message: &mut String,
    ) {
        self.edit_group = None;
        let index = if redo {
            if self.cursor >= self.history.len() {
                return;
            }
            self.cursor
        } else {
            if self.cursor == 0 {
                return;
            }
            self.cursor - 1
        };
        let mut reveal = None;
        let mut restored_selection = None;
        let applied = match &mut self.history[index] {
            Change::Property { edit, .. } => {
                let applied = edit.apply(model, redo);
                if applied {
                    reveal = edit.index();
                }
                applied
            }
            Change::Coefficients(change) => {
                let applied = change.apply(model);
                if applied {
                    reveal = Some(change.index);
                }
                applied
            }
            Change::Topology(change) => {
                let applied = change.apply(model, redo);
                if applied {
                    reveal = Some(change.index.min(model.nodes.len().saturating_sub(1)));
                }
                applied
            }
            Change::GroupDelete(change) => {
                let applied = change.apply(model, redo);
                if applied {
                    reveal = Some(change.first().min(model.nodes.len().saturating_sub(1)));
                    if !redo {
                        restored_selection = Some(change.indices());
                    }
                }
                applied
            }
            Change::Paste(change) => {
                let applied = change.apply(model, redo);
                if applied {
                    let indices = if redo {
                        change.indices()
                    } else {
                        change.previous_selection.clone()
                    };
                    reveal = Some(indices.first().copied().unwrap_or(usize::MAX));
                    restored_selection = Some(indices);
                }
                applied
            }
            Change::Wire(change) => {
                let (expected, replacement) = if redo {
                    (change.before, change.after)
                } else {
                    (change.after, change.before)
                };
                let source_exists = replacement == usize::MAX || replacement < model.nodes.len();
                let input = model
                    .nodes
                    .get_mut(change.target)
                    .and_then(|node| node.inputs.get_mut(change.port));
                if let Some(input) = input.filter(|input| **input == expected && source_exists) {
                    *input = replacement;
                    true
                } else {
                    false
                }
            }
            Change::Layout { before, after } => {
                let (expected, replacement) = if redo {
                    (before, after)
                } else {
                    (after, before)
                };
                expected.matches(model) && replacement.apply(model)
            }
        };
        if applied {
            self.changed = true;
            if redo {
                self.cursor += 1;
            } else {
                self.cursor -= 1;
            }
            self.cancel_connection();
            if let Some(index) = reveal {
                *selected = index;
                self.selection.replace(
                    model.nodes.len(),
                    restored_selection.unwrap_or_else(|| {
                        if index < model.nodes.len() {
                            vec![index]
                        } else {
                            vec![]
                        }
                    }),
                    selected,
                );
                self.reveal(index);
            }
            *message = if redo {
                "Diagram edit redone"
            } else {
                "Diagram edit undone"
            }
            .into();
        } else {
            self.reset();
            *message = "Diagram changed; the previous diagram history was cleared".into();
        }
    }

    #[cfg(test)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut Model,
        selected: &mut usize,
        message: &mut String,
        keyboard_allowed: bool,
        inspector_reserve: f32,
    ) {
        self.show_with_height(
            ui,
            model,
            selected,
            message,
            keyboard_allowed,
            Height::Legacy(inspector_reserve),
        );
    }

    pub fn show_pane(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut Model,
        selected: &mut usize,
        message: &mut String,
        keyboard_allowed: bool,
    ) {
        self.show_with_height(
            ui,
            model,
            selected,
            message,
            keyboard_allowed,
            Height::FillPane,
        );
    }

    fn show_with_height(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut Model,
        selected: &mut usize,
        message: &mut String,
        keyboard_allowed: bool,
        height_mode: Height,
    ) {
        if model.nodes.len() > 512 || model.nodes.iter().any(|node| node.inputs.len() > 32) {
            self.cancel_connection();
            ui.label("Diagram exceeds the supported block or port limit");
            return;
        }
        if let Err(error) = model.validate_layout() {
            self.cancel_connection();
            ui.label(error);
            return;
        }
        let enabled = ui.is_enabled();
        self.selection.sync(model.nodes.len(), selected);
        let frame = ui.ctx().cumulative_frame_nr();
        let interactive = enabled && self.last_input_frame != Some(frame);
        self.last_input_frame = Some(frame);
        if !enabled || self.source.is_some_and(|index| index >= model.nodes.len()) {
            self.cancel_connection();
        }
        if interactive && keyboard_allowed {
            self.keyboard(ui, model, selected, message);
        } else if !keyboard_allowed {
            self.nudge = None;
        }
        if interactive {
            self.navigate_pointer(ui, keyboard_allowed);
        }
        let canvas_width = ui.available_width();
        let mut organized = false;
        let mut edit_action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.cursor > 0, egui::Button::new("Undo diagram"))
                .clicked()
                && interactive
            {
                self.travel(model, selected, false, message);
                self.focus(ui.ctx());
            }
            if ui
                .add_enabled(
                    self.cursor < self.history.len(),
                    egui::Button::new("Redo diagram"),
                )
                .clicked()
                && interactive
            {
                self.travel(model, selected, true, message);
                self.focus(ui.ctx());
            }
            if ui
                .add_enabled(
                    !model.positions.is_empty(),
                    egui::Button::new("Auto arrange"),
                )
                .clicked()
                && interactive
            {
                self.arrange(model, *selected);
                self.focus(ui.ctx());
            }
            ui.menu_button("Edit blocks", |ui| {
                edit_action = self.edit_menu(ui, interactive);
            });
            // Keep this single menu in the parent wrapping row. An enabled-UI
            // scope narrows its label to the leftover row width and can make
            // the toolbar hundreds of pixels tall when the label wraps.
            ui.add_enabled(self.selection_len() >= 2, |ui: &mut egui::Ui| {
                ui.menu_button("Align / distribute", |ui| {
                    for (operation, label) in organize::Operation::ALL {
                        if ui
                            .add_enabled(
                                self.selection_len() >= operation.minimum(),
                                egui::Button::new(label),
                            )
                            .clicked()
                            && interactive
                        {
                            *message = match self.organize(model, canvas_width, operation) {
                                Ok(true) => label.into(),
                                Ok(false) => {
                                    "Selected blocks already match this arrangement".into()
                                }
                                Err(error) => error,
                            };
                            ui.close();
                            organized = true;
                        }
                    }
                })
                .response
            });
            ui.menu_button("View", |ui| {
                for (command, label) in [
                    (view::Command::In, "Zoom in"),
                    (view::Command::Out, "Zoom out"),
                    (view::Command::Actual, "Actual size (100%)"),
                    (view::Command::All, "Fit diagram"),
                    (view::Command::Selection, "Fit selection"),
                ] {
                    if ui
                        .add_enabled(
                            !matches!(command, view::Command::Selection)
                                || self.selection_len() > 0,
                            egui::Button::new(label),
                        )
                        .clicked()
                        && interactive
                    {
                        self.cancel_connection();
                        self.view.request = Some(command);
                        ui.close();
                        organized = true;
                    }
                }
            });
            let zoom_label = if self.view.zoom < 0.01 {
                format!("{:.2}%", self.view.zoom * 100.)
            } else {
                format!("{:.0}%", self.view.zoom * 100.)
            };
            // Digit transitions during pointer zoom must not rewrap the toolbar.
            ui.add_sized([48., ui.spacing().interact_size.y], egui::Label::new(egui::RichText::new(zoom_label).small()).truncate());
            ui.small("Controls").on_hover_text("Ctrl+wheel zooms; middle-drag pans. Drag empty space to select; Ctrl adds, Shift extends or unsnaps.\nCtrl+A selects all; arrows/Home/End select; Delete removes; Ctrl+Z/Y undo/redo.\nCtrl+arrows move (Shift: fine); C starts wire; PgUp/PgDn choose input; Enter connects; Backspace disconnects.");
            if ui.add_enabled(self.source.is_some() || self.moving.is_some() || self.marquee.is_some(), egui::Button::new("Cancel gesture")).clicked()
                && interactive
            {
                self.cancel_connection();
            }
        });
        // Keep one fixed guidance row so starting a connection never moves
        // the ports underneath the pointer. Long keyboard instructions remain
        // available in full on hover without changing the canvas geometry.
        let guidance = if self.keyboard_wiring {
            let name = self
                .source
                .and_then(|source| model.nodes.get(source))
                .map_or("missing block", |node| node.name.as_str());
            format!(
                "Wire from {name}: select a target, PgUp/PgDn choose its input, Enter connects; Escape cancels."
            )
        } else if self.source.is_some() {
            "Choose an input port. Escape cancels. Right-click an input to disconnect.".into()
        } else {
            "Select a block to edit; drag an output to an input to connect.".into()
        };
        ui.add(egui::Label::new(egui::RichText::new(&guidance).small()).truncate())
            .on_hover_text(&guidance);
        if self
            .moving
            .as_ref()
            .is_some_and(|moving| !moving.before.matches(model))
        {
            self.cancel_connection();
            *message = "Model layout changed; move cancelled".into();
        }
        if self
            .marquee
            .as_ref()
            .is_some_and(|marquee| !marquee.before.matches(model))
        {
            self.cancel_connection();
            *message = "Model layout changed; selection cancelled".into();
        }
        #[cfg(test)]
        self.ports.clear();
        #[cfg(test)]
        self.blocks.clear();
        let mut boxes = layout::boxes(model, ui.available_width());
        let positions = self
            .moving
            .as_ref()
            .map_or(&model.positions, |moving| &moving.positions);
        for (rect, position) in boxes.iter_mut().zip(positions) {
            *rect = Rect::from_min_size(Pos2::new(position[0], position[1]), rect.size());
        }
        let height = match height_mode {
            Height::FillPane => ui
                .available_height()
                .min(ui.clip_rect().bottom() - ui.cursor().top())
                .max(64.),
            #[cfg(test)]
            Height::Legacy(reserve) => {
                let mut height = (ui.available_height() * 0.32).clamp(100., 200.);
                if !positions.is_empty() {
                    // Fit modest custom layouts without clipping a block at the viewport
                    // edge; larger layouts still use a bounded, scrolling canvas.
                    let needed = boxes
                        .iter()
                        .map(|rect| rect.bottom() + 16.)
                        .fold(0., f32::max);
                    height = height.max(needed.min(200.));
                }
                if reserve > 0. {
                    let visible = (ui.clip_rect().bottom() - ui.cursor().top()).max(0.);
                    height = height.min((visible - reserve).max(64.));
                }
                height
            }
        };
        self.view.prepare(
            &boxes,
            self.selection.indices(),
            Vec2::new(ui.available_width(), height),
        );
        let zoom = self.view.zoom;
        let detail = zoom >= 0.4;
        let extent = boxes
            .iter()
            .fold(Vec2::new(ui.available_width(), height), |extent, rect| {
                extent.max(
                    (rect.max.to_vec2() + Vec2::splat(2. * view::CONTENT_PAD)) * zoom
                        + Vec2::splat(40.),
                )
            });
        let mut inside_canvas = false;
        let mut connection = None;
        let mut release_target = None;
        let mut scroll = egui::ScrollArea::both()
            .animated(false)
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            )
            .id_salt("block_diagram")
            .max_height(height)
            .auto_shrink([false, false]);
        if let Some(offset) = self.view.requested_offset.take() {
            scroll = scroll.scroll_offset(offset);
        }
        let scroll_output = scroll.show_themed(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(extent, Sense::hover());
            let diagram_origin = rect.min + Vec2::splat(view::CONTENT_PAD * zoom);
            self.view.origin = diagram_origin;
            let focus = ui.interact(rect, Self::keyboard_id(), Sense::click_and_drag());
            if interactive {
                super::super::focus_areas::register(
                    ui,
                    &focus,
                    super::super::focus_areas::Area::Model,
                );
            }
            if focus.secondary_clicked() && interactive {
                self.cancel_connection();
                self.focus(ui.ctx());
            }
            focus.context_menu(|ui| {
                if let Some(action) = self.edit_menu(ui, interactive) {
                    edit_action = Some(action);
                }
            });
            if focus.clicked() && interactive {
                self.focus(ui.ctx());
            }
            if focus.drag_started_by(egui::PointerButton::Primary)
                && self.view.pan.is_none()
                && interactive
                && let Some(origin) = ui.input(|input| input.pointer.press_origin())
            {
                self.cancel_connection();
                self.focus(ui.ctx());
                let additive = ui.input(|input| {
                    input.modifiers.ctrl || input.modifiers.command || input.modifiers.shift
                });
                self.marquee = Some(organize::Marquee {
                    before: layout::Snapshot::capture(model),
                    start: self.view.model_point(origin, diagram_origin),
                    end: self.view.model_point(origin, diagram_origin),
                    base: if additive {
                        self.selection.indices().to_vec()
                    } else {
                        Vec::new()
                    },
                });
            }
            if let Some(marquee) = &mut self.marquee
                && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            {
                marquee.end = self.view.model_point(pointer, diagram_origin);
            }
            if focus.has_focus() {
                Self::lock_arrows(ui.ctx());
            }
            inside_canvas = ui.rect_contains_pointer(rect);
            if let Some(moving) = &mut self.moving
                && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            {
                moving.update(
                    self.view.model_point(pointer, diagram_origin),
                    ui.input(|input| input.modifiers.shift),
                );
                for (block, position) in boxes.iter_mut().zip(&moving.positions) {
                    *block = Rect::from_min_size(Pos2::new(position[0], position[1]), block.size());
                }
            }
            for block in &mut boxes {
                *block = self.view.screen_rect(*block, diagram_origin);
            }
            if let Some(bounds) = self.view.focus.take() {
                ui.scroll_to_rect(
                    self.view.screen_rect(bounds, diagram_origin).expand(8.),
                    Some(egui::Align::Center),
                );
            }
            if let Some(node) = self.reveal.take()
                && let Some(block) = boxes.get(node)
            {
                let port = self
                    .reveal_port
                    .take()
                    .filter(|port| *port < model.nodes[node].inputs.len());
                let target = port.map_or(*block, |port| {
                    Rect::from_center_size(
                        input_position(*block, port, model.nodes[node].inputs.len(), zoom),
                        Vec2::splat(24.),
                    )
                });
                ui.scroll_to_rect(target, Some(egui::Align::Center));
            }
            let clip = ui.clip_rect();
            let p = tokens::palette(ui.ctx());
            let painter = ui.painter();
            painter.rect_filled(rect, 0., p.surface);
            // Only visible input endpoints generate wires. A dense offscreen model
            // does not emit thousands of invisible paths or text galleys per frame.
            for (target, node) in model.nodes.iter().enumerate() {
                if !boxes[target].expand(10.).intersects(clip) {
                    continue;
                }
                for (port, source) in node.inputs.iter().enumerate() {
                    if !detail {
                        if !node.inputs[..port].contains(source)
                            && let Some(source_box) = boxes.get(*source)
                        {
                            painter.line_segment(
                                [source_box.right_center(), boxes[target].left_center()],
                                Stroke::new(1_f32, p.text_muted),
                            );
                        }
                        continue;
                    }
                    let end = input_position(boxes[target], port, node.inputs.len(), zoom);
                    if !clip.contains(end) {
                        continue;
                    }
                    if let Some(source_box) = boxes.get(*source) {
                        let start = source_box.right_center();
                        let stroke = Stroke::new(
                            if target == *selected { 1.8_f32 } else { 1_f32 },
                            if target == *selected {
                                p.accent
                            } else {
                                p.text_muted
                            },
                        );
                        let near_input = end - Vec2::new(10. * zoom, 0.);
                        let points = if end.x - start.x > 0. && end.x - start.x <= 80. * zoom {
                            let middle = (start.x + end.x) * 0.5;
                            [
                                start,
                                Pos2::new(middle, start.y),
                                Pos2::new(middle, end.y),
                                near_input,
                                near_input,
                                near_input,
                            ]
                        } else {
                            let lane = if (source_box.top() - boxes[target].top()).abs() < 1. {
                                source_box.top() - (12. + (port % 3) as f32 * 3.) * zoom
                            } else {
                                source_box.bottom().max(boxes[target].bottom())
                                    + (14. + (port % 3) as f32 * 3.) * zoom
                            };
                            [
                                start,
                                Pos2::new(start.x + 12. * zoom, start.y),
                                Pos2::new(start.x + 12. * zoom, lane),
                                Pos2::new(end.x - 12. * zoom, lane),
                                Pos2::new(end.x - 12. * zoom, end.y),
                                near_input,
                            ]
                        };
                        for pair in points.windows(2) {
                            if pair[0] != pair[1] {
                                painter.line_segment([pair[0], pair[1]], stroke);
                            }
                        }
                        painter.arrow(near_input, Vec2::new(10. * zoom, 0.), stroke);
                    }
                }
            }
            for (index, node) in model.nodes.iter().enumerate() {
                let block = boxes[index];
                if !block.expand(10.).intersects(clip) {
                    continue;
                }
                #[cfg(test)]
                self.blocks.push((index, block));
                let painter = ui.painter();
                painter.rect_filled(
                    block,
                    3.,
                    if self.marquee.as_ref().map_or_else(
                        || self.selection.contains(index),
                        |marquee| {
                            marquee.contains(index, self.view.model_rect(block, diagram_origin))
                        },
                    ) {
                        p.accent_soft
                    } else {
                        p.surface
                    },
                );
                painter.rect_stroke(
                    block,
                    3.,
                    if index == *selected {
                        Stroke::new(2_f32, p.accent)
                    } else {
                        Stroke::new(1_f32, p.border)
                    },
                    egui::StrokeKind::Inside,
                );
                if detail {
                    let name: String = node.name.chars().take(16).collect();
                    painter.text(
                        block.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{}. {name}\n{}", index + 1, node.block.label()),
                        egui::FontId::proportional(tokens::BODY * zoom),
                        p.text,
                    );
                }
                let body = ui.interact(
                    block.shrink(9. * zoom),
                    ui.id().with(("block", index)),
                    Sense::click_and_drag(),
                );
                let body = body.on_hover_text(format!(
                    "{}. {} ({})",
                    index + 1,
                    node.name,
                    node.block.label()
                ));
                if body.secondary_clicked() && interactive {
                    self.cancel_connection();
                    if self.selection.contains(index) {
                        self.selection.activate(index, selected);
                    } else {
                        self.select(index, egui::Modifiers::NONE, model.nodes.len(), selected);
                    }
                }
                body.context_menu(|ui| {
                    if let Some(action) = self.edit_menu(ui, interactive) {
                        edit_action = Some(action);
                    }
                });
                if body.clicked() && interactive {
                    self.select(
                        index,
                        ui.input(|input| input.modifiers),
                        model.nodes.len(),
                        selected,
                    );
                    self.keyboard_port = 0;
                    self.focus(ui.ctx());
                }
                if body.drag_started_by(egui::PointerButton::Primary)
                    && interactive
                    && self.view.pan.is_none()
                {
                    self.focus(ui.ctx());
                    self.cancel_connection();
                    if self.selection.contains(index) {
                        self.selection.activate(index, selected);
                    } else {
                        self.select(index, egui::Modifiers::NONE, model.nodes.len(), selected);
                    }
                    if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
                        let positions = if model.positions.is_empty() {
                            layout::boxes(model, canvas_width)
                                .iter()
                                .map(|block| [block.left(), block.top()])
                                .collect()
                        } else {
                            model.positions.clone()
                        };
                        let mut moving = layout::Moving::new(
                            model,
                            positions,
                            index,
                            self.view.model_point(origin, diagram_origin),
                        );
                        moving.set_group(self.selection.indices());
                        if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                            moving.update(
                                self.view.model_point(pointer, diagram_origin),
                                ui.input(|input| input.modifiers.shift),
                            );
                        }
                        self.moving = Some(moving);
                    }
                }
                if !detail {
                    continue;
                }
                let output = block.right_center();
                let output_hit = Rect::from_center_size(output, Vec2::splat(16. * zoom));
                let response = ui.interact(
                    output_hit,
                    ui.id().with(("output", index)),
                    Sense::click_and_drag(),
                );
                if response.clicked() && interactive {
                    self.keyboard_wiring = false;
                    self.focus(ui.ctx());
                    self.source = if self.source == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                    *selected = index;
                    self.selection
                        .replace(model.nodes.len(), vec![index], selected);
                }
                if response.drag_started_by(egui::PointerButton::Primary)
                    && interactive
                    && self.view.pan.is_none()
                {
                    self.keyboard_wiring = false;
                    self.focus(ui.ctx());
                    self.source = Some(index);
                    self.dragging = true;
                    *selected = index;
                    self.selection
                        .replace(model.nodes.len(), vec![index], selected);
                }
                response.on_hover_text("Output: click, then click an input; or drag to an input");
                ui.painter().circle_filled(
                    output,
                    4. * zoom,
                    if self.source == Some(index) {
                        p.warning
                    } else {
                        p.accent
                    },
                );
                #[cfg(test)]
                self.ports.push((index, None, output));
                for (port, source) in node.inputs.iter().enumerate() {
                    let position = input_position(block, port, node.inputs.len(), zoom);
                    if !clip.contains(position) {
                        continue;
                    }
                    let hit = Rect::from_center_size(position, Vec2::splat(14. * zoom));
                    let response =
                        ui.interact(hit, ui.id().with(("input", index, port)), Sense::click());
                    if response.clicked() && interactive {
                        self.keyboard_port = port;
                        self.focus(ui.ctx());
                        *selected = index;
                        self.selection
                            .replace(model.nodes.len(), vec![index], selected);
                        if let Some(source) = self.source.take() {
                            connection = Some((index, port, source));
                        }
                    }
                    response.context_menu(|ui| {
                        if ui
                            .add_enabled(
                                *source != usize::MAX,
                                egui::Button::new("Disconnect input"),
                            )
                            .clicked()
                            && interactive
                        {
                            connection = Some((index, port, usize::MAX));
                            ui.close();
                        }
                    });
                    if response.contains_pointer() {
                        release_target = Some((index, port));
                    }
                    let source_name: String = model
                        .nodes
                        .get(*source)
                        .map_or("not connected", |n| n.name.as_str())
                        .chars()
                        .take(128)
                        .collect();
                    response.on_hover_text(format!(
                        "Input {}: {}\nRight-click to disconnect",
                        port + 1,
                        source_name
                    ));
                    ui.painter().circle_filled(
                        position,
                        4. * zoom,
                        if *source < model.nodes.len() {
                            p.accent
                        } else {
                            p.surface
                        },
                    );
                    ui.painter()
                        .circle_stroke(position, 4. * zoom, Stroke::new(1_f32, p.accent));
                    if index == *selected
                        && port == self.keyboard_port.min(node.inputs.len() - 1)
                        && ui.memory(|memory| memory.has_focus(Self::keyboard_id()))
                    {
                        ui.painter().circle_stroke(
                            position,
                            7. * zoom,
                            Stroke::new(2_f32, p.warning),
                        );
                    }
                    #[cfg(test)]
                    self.ports.push((index, Some(port), position));
                }
            }
            if let Some(marquee) = &self.marquee {
                let area = self.view.screen_rect(
                    Rect::from_two_pos(marquee.start, marquee.end),
                    diagram_origin,
                );
                ui.painter()
                    .rect_filled(area, 0., p.accent.gamma_multiply(0.1));
                ui.painter().rect_stroke(
                    area,
                    0.,
                    Stroke::new(1_f32, p.accent),
                    egui::StrokeKind::Inside,
                );
            }
            if interactive
                && ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
                && let Some(marquee) = self.marquee.take()
                && inside_canvas
                && marquee.before.matches(model)
            {
                let indices = boxes
                    .iter()
                    .enumerate()
                    .filter_map(|(index, block)| {
                        marquee
                            .contains(index, self.view.model_rect(*block, diagram_origin))
                            .then_some(index)
                    })
                    .collect();
                self.selection.replace(model.nodes.len(), indices, selected);
            }
            let endpoint = if self.keyboard_wiring {
                boxes.get(*selected).map(|block| {
                    let count = model.nodes[*selected].inputs.len();
                    if count == 0 {
                        block.center()
                    } else {
                        input_position(*block, self.keyboard_port.min(count - 1), count, zoom)
                    }
                })
            } else {
                ui.input(|input| input.pointer.hover_pos())
            };
            if let Some(source) = self.source.and_then(|source| boxes.get(source))
                && let Some(pointer) = endpoint
            {
                ui.painter().line_segment(
                    [source.right_center(), pointer],
                    Stroke::new(1.5_f32, p.accent),
                );
            }
            if ui.memory(|memory| memory.has_focus(Self::keyboard_id())) {
                ui.painter().rect_stroke(
                    rect.intersect(clip),
                    0.,
                    Stroke::new(1_f32, p.accent),
                    egui::StrokeKind::Inside,
                );
            }
        });
        self.view.offset = scroll_output.state.offset;
        self.view.viewport = scroll_output.inner_rect.intersect(ui.clip_rect());
        if interactive
            && ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
            && let Some(moving) = self.moving.take()
            && inside_canvas
            && moving.before.matches(model)
            && moving.changed()
        {
            match model.set_layout(moving.positions) {
                Ok(()) => {
                    let after = layout::Snapshot::capture(model);
                    self.push(Change::Layout {
                        before: moving.before,
                        after,
                    });
                    *message = "Block moved".into();
                }
                Err(error) => *message = error,
            }
        }
        if interactive
            && self.dragging
            && ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
        {
            if let (Some(source), Some((target, port))) = (self.source.take(), release_target) {
                connection = Some((target, port, source));
            }
            self.dragging = false;
        }
        if interactive && let Some((target, port, source)) = connection {
            self.source = None;
            self.dragging = false;
            self.keyboard_wiring = false;
            if self.connect(model, target, port, source) {
                *selected = target;
                self.selection
                    .replace(model.nodes.len(), vec![target], selected);
                *message = if source == usize::MAX {
                    "Input disconnected"
                } else {
                    "Connection updated"
                }
                .into();
            }
        }
        if let Some(action) = edit_action {
            self.edit_action(action, ui, model, canvas_width, selected, message);
            organized = true;
        }
        if organized {
            self.focus(ui.ctx());
        }
        self.last_input_frame = Some(frame);
    }
}

fn input_position(rect: Rect, port: usize, count: usize, zoom: f32) -> Pos2 {
    Pos2::new(
        rect.left(),
        rect.center().y + (port as f32 - (count.saturating_sub(1)) as f32 * 0.5) * 16. * zoom,
    )
}

#[cfg(test)]
#[path = "model_canvas_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "model_topology_tests.rs"]
mod topology_tests;

#[cfg(test)]
#[path = "model_parameters_tests.rs"]
mod parameter_tests;

#[cfg(test)]
#[path = "model_keyboard_tests.rs"]
mod keyboard_tests;

#[cfg(test)]
#[path = "model_keyboard_edit_tests.rs"]
mod keyboard_edit_tests;

#[cfg(test)]
#[path = "model_selection_tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "model_organize_tests.rs"]
mod organize_tests;

#[cfg(test)]
#[path = "model_clipboard_tests.rs"]
mod clipboard_tests;

#[cfg(test)]
#[path = "model_cut_tests.rs"]
mod cut_tests;

#[cfg(test)]
#[path = "model_view_tests.rs"]
mod view_tests;

#[cfg(test)]
#[path = "model_navigation_tests.rs"]
mod navigation_tests;

#[cfg(test)]
#[path = "model_arithmetic_tests.rs"]
mod arithmetic_tests;

#[cfg(test)]
#[path = "model_routing_tests.rs"]
mod routing_tests;

#[cfg(test)]
#[path = "model_product_scroll_tests.rs"]
mod product_scroll_tests;

#[cfg(test)]
#[path = "model_inspector_layout_tests.rs"]
mod inspector_layout_tests;

#[cfg(test)]
#[path = "model_integrator_tests.rs"]
mod integrator_tests;
