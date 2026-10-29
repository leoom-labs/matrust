//! Two editor views over existing documents; no copied document or worker state.
use super::*;

#[derive(Clone, Copy)]
struct Region {
    document: u64,
    rect: egui::Rect,
    layer: egui::LayerId,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) struct HeadingObservation {
    pub document: u64,
    pub active: bool,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
    pub frame: u64,
}

#[derive(Clone, Copy)]
struct TabOrder {
    viewport: egui::ViewportId,
    frame: u64,
    document: u64,
    neighbor: u64,
    left: bool,
    interaction: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) struct TabDragObservation {
    pub pixels_per_point: f32,
    pub document: u64,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
    pub frame: u64,
    pub response_id: egui::Id,
    pub contains_pointer: bool,
    pub enabled: bool,
    pub drag_started: bool,
    pub dragged_id: Option<egui::Id>,
    pub pointer: Option<egui::Pos2>,
    pub primary_down: bool,
    pub interaction: u64,
    pub active: u64,
    pub available: bool,
    pub retained: Option<(u64, bool, u64, bool)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TabDragToken {
    viewport: egui::ViewportId,
    serial: u64,
}

#[derive(Clone, Copy)]
struct TabDrag {
    token: TabDragToken,
    source: u64,
    started: bool,
    press_frame: u64,
    press_position: egui::Pos2,
    active: u64,
    interaction: u64,
    order: [u64; session::MAX_DOCUMENTS],
    len: usize,
}

#[derive(Clone, Copy)]
struct TabDrop {
    token: TabDragToken,
    frame: u64,
    target: u64,
    before: bool,
}

#[cfg(test)]
struct TabScrollObservation {
    clip: egui::Rect,
    content_width: f32,
    offset: f32,
    stable_dt: f32,
    pointer: Option<egui::Pos2>,
    retained: Option<(bool, bool)>,
    drop_pending: bool,
    frame: u64,
}

#[derive(Default)]
pub(super) struct State {
    #[cfg(test)]
    tab_drag_observations: [Option<TabDragObservation>; session::MAX_DOCUMENTS],
    tab_drag: Option<TabDrag>,
    tab_drop: Option<TabDrop>,
    tab_drag_serial: u64,
    #[cfg(test)]
    tab_scroll_observation: Option<TabScrollObservation>,
    tab_drag_scroll: Option<f32>,
    tab_drop_finished: Option<(egui::ViewportId, u64)>,
    tab_order: Option<TabOrder>,
    tab_order_finished: Option<(egui::ViewportId, u64)>,
    #[cfg(test)]
    headings: [Option<HeadingObservation>; 2],
    owner: Option<u64>,
    pub(super) other: Option<u64>,
    active_right: bool,
    secondary: editor::Editor,
    regions: [Option<Region>; 2],
    pending: Option<(u64, bool, u64)>,
    finished_frame: Option<u64>,
    last_focus: Option<egui::Id>,
    painted: Option<(
        session::PaneMode,
        bool,
        Option<session::CenterLayout>,
        egui::Rect,
    )>,
    overlay_previous: bool,
    context: Option<egui::Context>,
    input_frame: Option<u64>,
    input_target: Option<(u64, u64, u64)>, // frame, intended document, prior active document
}

impl Workspace {
    #[cfg(test)]
    pub(super) fn editor_heading_observations(&self) -> [Option<HeadingObservation>; 2] {
        self.editor_columns.headings
    }

    /// Forget viewport-local geometry and interaction tickets; retain both Editor states.
    pub(super) fn reset_editor_host(&mut self) {
        self.cancel_script_tab_order();
        self.editor_columns.regions = [None; 2];
        self.editor_columns.painted = None;
        self.editor_columns.pending = None;
        self.editor_columns.finished_frame = None;
        self.editor_columns.input_frame = None;
        self.editor_columns.input_target = None;
        self.editor_columns.last_focus = None;
        self.editor_columns.context = None;
        self.editor_columns.overlay_previous = false;
    }

    /// Reconcile genuine external document activation, never a temporary paint selection.
    pub(super) fn sync_editor_columns(&mut self) {
        let current = self.documents[self.active].id;
        if self.editor_columns.owner != Some(current) {
            if self.editor_columns.other == Some(current) {
                let previous = self
                    .editor_columns
                    .owner
                    .filter(|id| self.documents.iter().any(|doc| doc.id == *id));
                std::mem::swap(&mut self.editor, &mut self.editor_columns.secondary);
                self.editor_columns.other = previous;
                self.editor_columns.active_right = !self.editor_columns.active_right;
            }
            self.editor_columns.owner = Some(current);
        }
        if self
            .editor_columns
            .other
            .is_some_and(|id| id == current || !self.documents.iter().any(|doc| doc.id == id))
        {
            self.editor_columns.other = None;
            self.editor_columns.secondary = Default::default();
            self.editor_columns.active_right = false;
        }
    }

    pub(super) fn activate_script_index(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.sync_editor_columns();
        if self.active != index {
            let previous = self.documents[self.active].id;
            self.active = index;
            // Explicit activation (including New) must reveal its real tab.
            // Repeated observation of the same active document leaves manual scrolling alone.
            self.script_tab_reveal = Some(self.documents[index].id);
            self.selection = None;
            self.recovery_dirty = true;
            self.sync_editor_columns();
            // An explicit tab/Open/New target replaces only editor-owned focus.
            // Retained focus in a preview, Command, model or modal is not ours.
            if let Some(ctx) = self.editor_columns.context.clone()
                && self.editor_columns_input_available(&ctx)
                && !ctx.will_discard()
                && let Some(focus) = ctx.memory(|m| m.focused())
                && (focus == editor::id(previous)
                    || self
                        .editor_columns
                        .regions
                        .iter()
                        .flatten()
                        .filter(|region| region.document == previous)
                        .any(|region| {
                            ctx.read_response(focus).is_some_and(|response| {
                                response.layer_id == region.layer
                                    && region.rect.contains_rect(response.interact_rect)
                            })
                        }))
            {
                ctx.memory_mut(|memory| memory.request_focus(editor::id(self.documents[index].id)));
            }
        }
        self.document_save
            .set_script_document(self.documents[self.active].id);
    }

    pub(super) fn forget_script_editor(&mut self, document: u64) {
        self.editor.forget(document);
        self.editor_columns.secondary.forget(document);
    }

    pub(super) fn editor_columns_snapshot(&self) -> Option<session::EditorColumns> {
        let other = self.editor_columns.other?;
        let other = self.documents.iter().position(|doc| doc.id == other)?;
        if other == self.active {
            return None;
        }
        Some(if self.editor_columns.active_right {
            session::EditorColumns {
                left: other,
                right: self.active,
            }
        } else {
            session::EditorColumns {
                left: self.active,
                right: other,
            }
        })
    }

    pub(super) fn restore_editor_columns(&mut self) {
        self.editor_columns = Default::default();
        self.editor_columns.owner = Some(self.documents[self.active].id);
        if let Some(columns) = self
            .layout
            .editor_columns
            .filter(|columns| columns.valid_for(self.documents.len(), self.active))
        {
            self.editor_columns.active_right = self.active == columns.right;
            let other = if self.editor_columns.active_right {
                columns.left
            } else {
                columns.right
            };
            self.editor_columns.other = Some(self.documents[other].id);
        } else {
            self.layout.editor_columns = None;
        }
    }

    pub(super) fn collapse_editor_columns(&mut self) {
        self.editor_columns.other = None;
        self.editor_columns.secondary = Default::default();
        self.editor_columns.regions = [None; 2];
        self.editor_columns.active_right = false;
        self.layout.editor_columns = None;
        self.editor_columns.painted = None;
    }

    /// Reusable layout intent never contains document indices or identities.
    pub(super) fn set_named_editor_split(&mut self, split: bool) {
        self.editor_columns.pending = None;
        self.sync_editor_columns();
        if !split {
            self.collapse_editor_columns();
        } else if self.editor_columns.other.is_none() {
            let current = self.documents[self.active].id;
            if let Some(other) = self.documents.iter().find(|doc| doc.id != current) {
                self.editor_columns.other = Some(other.id);
                self.editor_columns.owner = Some(current);
                self.editor_columns.active_right = false;
                self.editor_columns.secondary = Default::default();
            }
        }
        self.editor_columns.regions = [None; 2];
        self.editor_columns.painted = None;
        self.editor_columns.input_target = None;
    }

    /// Called before shortcut and toolbar admission, using retained prior-frame regions.
    pub(super) fn observe_editor_columns(&mut self, ctx: &egui::Context) {
        if !self.editor_viewport_allowed(ctx) {
            return;
        }
        self.editor_columns.context = Some(ctx.clone());
        self.sync_editor_columns();
        let focus = ctx.memory(|m| m.focused());
        let overlay = egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || self.file_action_pending()
            || self.recovering_session
            || self.pending_quit
            || self.allow_close
            || ctx.input(|input| input.viewport().close_requested());
        let frame = ctx.cumulative_frame_nr();
        if self.editor_columns.input_frame != Some(frame) {
            self.editor_columns.input_frame = Some(frame);
            self.editor_columns.input_target = None;
            if !overlay
                && !self.editor_columns.overlay_previous
                && self.editor_columns_input_available(ctx)
            {
                let press = ctx.input(|input| {
                    input.events.iter().rev().find_map(|event| {
                        if let egui::Event::PointerButton {
                            pos, pressed: true, ..
                        } = event
                        {
                            Some(*pos)
                        } else {
                            None
                        }
                    })
                });
                let pointed = press.and_then(|point| {
                    self.editor_columns
                        .regions
                        .iter()
                        .flatten()
                        .find(|region| {
                            region.rect.contains(point)
                                && ctx.layer_id_at(point) == Some(region.layer)
                        })
                        .map(|region| region.document)
                });
                let focused = focus.and_then(|id| {
                    // Only a real text-input interaction revives retained editor focus.
                    let interacting = if self.editor_window.in_callback {
                        self.editor_window_input_activity(ctx, id)
                    } else {
                        self.document_save.editor_input_activity(ctx, id)
                    };
                    if !interacting && focus == self.editor_columns.last_focus {
                        return None;
                    }
                    self.editor_columns
                        .regions
                        .iter()
                        .flatten()
                        .find(|region| {
                            editor::id(region.document) == id
                                || ctx.read_response(id).is_some_and(|response| {
                                    response.layer_id == region.layer
                                        && region.rect.contains_rect(response.interact_rect)
                                })
                        })
                        .map(|region| region.document)
                });
                if let Some(document) = pointed.or(focused)
                    && self.documents.iter().any(|doc| doc.id == document)
                {
                    self.editor_columns.input_target =
                        Some((frame, document, self.documents[self.active].id));
                }
            }
        }
        // A discarded pass can record intent, but cannot activate a document.
        // Keep that exact ID for Save/Run admission; never reinterpret it as the
        // old tab when a later pass cannot validate the intended view.
        if !ctx.will_discard()
            && self.editor_columns_input_available(ctx)
            && let Some((queued, document, previous)) = self.editor_columns.input_target
            && queued == frame
            && [previous, document].contains(&self.documents[self.active].id)
            && let Some(index) = self.documents.iter().position(|doc| doc.id == document)
        {
            self.activate_script_index(index);
            self.document_save.activate(document_save::Pane::Script);
        }
        self.editor_columns.last_focus = focus;
        self.editor_columns.overlay_previous = overlay;
        self.document_save
            .set_script_document(self.script_action_document(ctx));
    }

    pub(super) fn script_action_document(&self, ctx: &egui::Context) -> u64 {
        if !self.editor_viewport_allowed(ctx) {
            return self.documents[self.active].id;
        }
        self.editor_columns
            .input_target
            .filter(|(frame, _, _)| *frame == ctx.cumulative_frame_nr())
            .map_or(self.documents[self.active].id, |(_, document, _)| document)
    }

    fn editor_columns_input_available(&self, ctx: &egui::Context) -> bool {
        self.editor_columns.painted
            == Some((
                self.layout.mode,
                self.compact_figure,
                self.layout.center,
                ctx.content_rect(),
            ))
            && (self.editor_window.in_callback || !self.central_command())
            && !self.file_action_pending()
            && !self.recovering_session
            && !self.pending_quit
            && !self.allow_close
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|m| m.top_modal_layer().is_none())
            && self.editor_viewport_allowed(ctx)
            && ctx.input(|input| {
                input.viewport().focused != Some(false) && !input.viewport().close_requested()
            })
    }

    pub(super) fn editor_columns_control(&mut self, ui: &mut egui::Ui) {
        let split = self.editor_columns.other.is_some();
        let label = if split {
            "Single editor"
        } else {
            "Side by side"
        };
        if ui.add_enabled(!self.file_action_pending() && (split || self.documents.len() > 1),
            egui::Button::new(label).small())
            .on_hover_text("Show two existing scripts. Click a view to target Save, Run, Find and Close. Open or create a second script first.")
            .clicked()
        {
            let frame = ui.ctx().cumulative_frame_nr();
            if self.editor_columns.finished_frame != Some(frame)
                && self.editor_columns.pending.is_none_or(|(queued, _, _)| queued != frame)
            {
                self.editor_columns.pending = Some((frame, !split, self.documents[self.active].id));
                ui.close();
            }
        }
    }

    pub(super) fn cancel_script_tab_order(&mut self) {
        self.cancel_script_tab_drag();
        if let Some(ticket) = self.editor_columns.tab_order.take() {
            self.editor_columns.tab_order_finished = Some((ticket.viewport, ticket.frame));
        }
    }

    fn script_tab_order_available(&self, ctx: &egui::Context) -> bool {
        self.editor_viewport_allowed(ctx)
            && self.documents.len() > 1
            && !self.busy
            && !self.file_action_pending()
            && !self.recovering_session
            && !self.pending_quit
            && !self.allow_close
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && ctx.input(|input| {
                input.viewport().focused != Some(false) && !input.viewport().close_requested()
            })
    }

    #[cfg(test)]
    pub(super) fn script_tab_drag_observations(
        &self,
    ) -> [Option<TabDragObservation>; session::MAX_DOCUMENTS] {
        self.editor_columns.tab_drag_observations
    }

    #[cfg(test)]
    pub(super) fn script_tab_drag_diagnostic(&self, document: u64) -> String {
        let response = self
            .editor_columns
            .tab_drag_observations
            .iter()
            .flatten()
            .find(|tab| tab.document == document)
            .map(|tab| {
                (
                    tab.response_id,
                    tab.contains_pointer,
                    tab.enabled,
                    tab.drag_started,
                    tab.dragged_id,
                    tab.pointer,
                    tab.primary_down,
                    tab.interaction,
                    tab.active,
                    tab.available,
                    tab.retained,
                )
            });
        let retained = self.editor_columns.tab_drag.map(|drag| {
            (
                drag.source,
                drag.started,
                drag.press_frame,
                drag.press_position,
                drag.interaction,
                drag.active,
                drag.token.serial,
            )
        });
        format!(
            "response={response:?}; retained_after_frame={retained:?}; active={}; interaction={}; busy={}; pending={}; serial={}",
            self.documents[self.active].id,
            self.document_save.interaction_generation(),
            self.busy,
            self.file_action_pending(),
            self.editor_columns.tab_drag_serial
        )
    }

    pub(super) fn script_tab_order_ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(test)]
        {
            self.editor_columns.tab_drag_observations = [None; session::MAX_DOCUMENTS];
        }
        self.observe_script_tab_drag(ui.ctx());
        ui.add_enabled_ui(
            self.script_tab_order_available(ui.ctx()) && self.editor_columns.tab_drag.is_none(),
            |ui| {
                ui.menu_button("Tabs", |ui| {
                    for (label, left) in [("Move tab left", true), ("Move tab right", false)] {
                        let neighbor = if left {
                            self.active.checked_sub(1)
                        } else {
                            (self.active + 1 < self.documents.len()).then_some(self.active + 1)
                        };
                        if ui
                            .add_enabled(neighbor.is_some(), egui::Button::new(label))
                            .clicked()
                        {
                            let stamp = (ui.ctx().viewport_id(), ui.ctx().cumulative_frame_nr());
                            if self.script_tab_order_available(ui.ctx())
                                && self.editor_columns.tab_order_finished != Some(stamp)
                                && self.editor_columns.tab_order.is_none()
                                && let Some(neighbor) = neighbor
                            {
                                self.editor_columns.tab_order = Some(TabOrder {
                                    viewport: stamp.0,
                                    frame: stamp.1,
                                    document: self.documents[self.active].id,
                                    neighbor: self.documents[neighbor].id,
                                    left,
                                    interaction: self.document_save.interaction_generation(),
                                });
                            }
                            ui.close();
                        }
                    }
                });
            },
        );
    }

    fn cancel_script_tab_drag(&mut self) {
        if let Some(drop) = self.editor_columns.tab_drop.take() {
            self.editor_columns.tab_drop_finished = Some((drop.token.viewport, drop.frame));
        }
        let owned = self.editor_columns.tab_drag.take().map(|drag| drag.token);
        self.editor_columns.tab_drag_scroll = None;
        self.editor_columns.tab_drag_serial = self.editor_columns.tab_drag_serial.wrapping_add(1);
        if let (Some(ctx), Some(owned)) = (&self.editor_columns.context, owned)
            && egui::DragAndDrop::payload::<TabDragToken>(ctx)
                .is_some_and(|payload| *payload == owned)
        {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }

    fn valid_script_tab_drag(&self, ctx: &egui::Context, drag: TabDrag) -> bool {
        self.script_tab_order_available(ctx)
            && !egui::Popup::is_any_open(ctx)
            && drag.token.viewport == ctx.viewport_id()
            && drag.token.serial == self.editor_columns.tab_drag_serial
            && drag.active == self.documents[self.active].id
            && drag.interaction == self.document_save.interaction_generation()
            && self.documents.len() == drag.len
            && self
                .documents
                .iter()
                .zip(&drag.order)
                .all(|(doc, id)| doc.id == *id)
    }

    fn observe_script_tab_drag(&mut self, ctx: &egui::Context) {
        let Some(drag) = self.editor_columns.tab_drag else {
            return;
        };
        if !self.valid_script_tab_drag(ctx, drag)
            || (drag.started
                && self.editor_columns.tab_drop.is_none()
                && egui::DragAndDrop::payload::<TabDragToken>(ctx)
                    .is_none_or(|payload| *payload != drag.token))
            || self
                .editor_columns
                .tab_drop
                .is_some_and(|drop| drop.frame != ctx.cumulative_frame_nr())
        {
            self.cancel_script_tab_drag();
        }
    }

    pub(super) fn script_tab_drag_response(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        document: u64,
    ) {
        let ctx = ui.ctx();
        #[cfg(test)]
        {
            let retained = self.editor_columns.tab_drag.map(|drag| {
                (
                    drag.source,
                    drag.started,
                    drag.interaction,
                    self.valid_script_tab_drag(ctx, drag),
                )
            });
            let available = self.script_tab_order_available(ctx);
            if let Some(slot) = self
                .editor_columns
                .tab_drag_observations
                .iter_mut()
                .find(|slot| slot.is_none())
            {
                *slot = Some(TabDragObservation {
                    pixels_per_point: ctx.pixels_per_point(),
                    document,
                    rect: response.rect,
                    clip: ui.clip_rect(),
                    frame: ctx.cumulative_frame_nr(),
                    response_id: response.id,
                    contains_pointer: response.contains_pointer(),
                    enabled: response.enabled(),
                    drag_started: response.drag_started_by(egui::PointerButton::Primary),
                    dragged_id: ctx.dragged_id(),
                    pointer: ctx.input(|input| input.pointer.interact_pos()),
                    primary_down: ctx.input(|input| input.pointer.primary_down()),
                    interaction: self.document_save.interaction_generation(),
                    active: self.documents[self.active].id,
                    available,
                    retained,
                });
            }
        }
        if response.contains_pointer()
            && ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
            && self.script_tab_order_available(ctx)
            && !egui::Popup::is_any_open(ctx)
            && self.editor_columns.tab_order.is_none()
            && self.editor_columns.tab_drag.is_none()
            && !egui::DragAndDrop::has_any_payload(ctx)
            && self.documents.len() <= session::MAX_DOCUMENTS
            && ctx.input(|input| input.modifiers.is_none())
            && let Some(press_position) = ctx.input(|input| input.pointer.interact_pos())
        {
            self.editor_columns.tab_drag_serial =
                self.editor_columns.tab_drag_serial.wrapping_add(1);
            let token = TabDragToken {
                viewport: ctx.viewport_id(),
                serial: self.editor_columns.tab_drag_serial,
            };
            let mut order = [0; session::MAX_DOCUMENTS];
            for (slot, doc) in order.iter_mut().zip(&self.documents) {
                *slot = doc.id;
            }
            self.editor_columns.tab_drag = Some(TabDrag {
                token,
                source: document,
                started: false,
                press_frame: ctx.cumulative_frame_nr(),
                press_position,
                active: self.documents[self.active].id,
                interaction: self.document_save.interaction_generation(),
                order,
                len: self.documents.len(),
            });
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(mut drag) = self.editor_columns.tab_drag
            && drag.source == document
            && !drag.started
            && !egui::DragAndDrop::has_any_payload(ctx)
            && self.valid_script_tab_drag(ctx, drag)
        {
            drag.started = true;
            self.editor_columns.tab_drag = Some(drag);
            self.script_tab_reveal = None;
            response.dnd_set_drag_payload(drag.token);
        }
        let Some(drag) = self.editor_columns.tab_drag else {
            return;
        };
        if !self.valid_script_tab_drag(ctx, drag) {
            self.cancel_script_tab_drag();
            return;
        }
        let Some(pointer) = ctx
            .input(|input| input.pointer.hover_pos())
            .filter(|point| ui.clip_rect().contains(*point) && response.rect.contains(*point))
        else {
            return;
        };
        if response
            .dnd_hover_payload::<TabDragToken>()
            .is_some_and(|token| *token == drag.token)
        {
            let before = pointer.x < response.rect.center().x;
            let x = if before {
                response.rect.left()
            } else {
                response.rect.right()
            };
            ui.painter().line_segment(
                [
                    egui::pos2(x, response.rect.top()),
                    egui::pos2(x, response.rect.bottom()),
                ],
                egui::Stroke::new(2., tokens::palette(ui.ctx()).accent),
            );
            if response
                .dnd_release_payload::<TabDragToken>()
                .is_some_and(|token| *token == drag.token)
                && self.editor_columns.tab_drop.is_none()
            {
                self.editor_columns.tab_drop = Some(TabDrop {
                    token: drag.token,
                    frame: ctx.cumulative_frame_nr(),
                    target: document,
                    before,
                });
            }
        }
    }

    #[cfg(test)]
    pub(super) fn script_tab_scroll_diagnostic(&self) -> String {
        let observation = self
            .editor_columns
            .tab_scroll_observation
            .as_ref()
            .map(|value| {
                (
                    value.clip,
                    value.content_width,
                    value.offset,
                    value.stable_dt,
                    value.pointer,
                    value.retained,
                    value.drop_pending,
                    value.frame,
                )
            });
        format!(
            "(inner_clip, content_width, offset, stable_dt, pointer, retained_started_valid, drop_pending, frame)={observation:?}; next_offset={:?}",
            self.editor_columns.tab_drag_scroll
        )
    }

    pub(super) fn script_tab_drag_scroll(&self) -> Option<f32> {
        self.editor_columns.tab_drag_scroll
    }

    pub(super) fn update_script_tab_drag_scroll(
        &mut self,
        ctx: &egui::Context,
        clip: egui::Rect,
        content_width: f32,
        offset: f32,
    ) {
        #[cfg(test)]
        {
            let retained = self
                .editor_columns
                .tab_drag
                .map(|drag| (drag.started, self.valid_script_tab_drag(ctx, drag)));
            self.editor_columns.tab_scroll_observation = Some(TabScrollObservation {
                clip,
                content_width,
                offset,
                stable_dt: ctx.input(|input| input.stable_dt),
                pointer: ctx.input(|input| input.pointer.hover_pos()),
                retained,
                drop_pending: self.editor_columns.tab_drop.is_some(),
                frame: ctx.cumulative_frame_nr(),
            });
        }
        let Some(drag) = self.editor_columns.tab_drag else {
            return;
        };
        if !drag.started
            || self.editor_columns.tab_drop.is_some()
            || !self.valid_script_tab_drag(ctx, drag)
        {
            return;
        }
        let Some(pointer) = ctx
            .input(|input| input.pointer.hover_pos())
            .filter(|point| clip.contains(*point))
        else {
            self.editor_columns.tab_drag_scroll = None;
            return;
        };
        let edge = 18.0_f32.min(clip.width().max(0.) * 0.25);
        let direction = if pointer.x < clip.left() + edge {
            -1.
        } else if pointer.x > clip.right() - edge {
            1.
        } else {
            0.
        };
        let step = ctx.input(|input| input.stable_dt.clamp(0., 0.05)) * 320.;
        let next = (offset + direction * step).clamp(0., (content_width - clip.width()).max(0.));
        self.editor_columns.tab_drag_scroll = (direction != 0.).then_some(next);
        if (next - offset).abs() > 0.01 {
            ctx.request_repaint();
        }
    }

    fn finish_script_tab_drag(&mut self, ctx: &egui::Context) {
        if let Some(drag) = &mut self.editor_columns.tab_drag
            && drag.token.viewport == ctx.viewport_id()
            && drag.press_frame == ctx.cumulative_frame_nr()
            && let Some(generation) = self.document_save.observed_script_press_generation(
                ctx,
                drag.press_position,
                drag.interaction,
            )
        {
            drag.interaction = generation;
        }
        let Some(drop) = self.editor_columns.tab_drop else {
            if self.editor_columns.tab_drag.is_some()
                && ctx.input(|input| input.pointer.any_released())
                && !ctx.will_discard()
            {
                self.cancel_script_tab_drag();
            }
            return;
        };
        if drop.token.viewport != ctx.viewport_id() || ctx.will_discard() {
            return;
        }
        self.editor_columns.tab_drop = None;
        let stamp = (drop.token.viewport, drop.frame);
        let Some(drag) = self.editor_columns.tab_drag.take() else {
            return;
        };
        self.editor_columns.tab_drag_scroll = None;
        if self.editor_columns.tab_drop_finished == Some(stamp) {
            return;
        }
        self.editor_columns.tab_drop_finished = Some(stamp);
        if drop.frame != ctx.cumulative_frame_nr()
            || drop.token != drag.token
            || !self.valid_script_tab_drag(ctx, drag)
        {
            return;
        }
        let Some(from) = self.documents.iter().position(|doc| doc.id == drag.source) else {
            return;
        };
        let Some(target) = self.documents.iter().position(|doc| doc.id == drop.target) else {
            return;
        };
        let boundary = target + usize::from(!drop.before);
        let to = boundary - usize::from(from < boundary);
        if from == to {
            return;
        }
        let selected = self
            .selection
            .as_ref()
            .and_then(|(index, _)| self.documents.get(*index))
            .map(|doc| doc.id);
        let document = self.documents.remove(from);
        self.documents.insert(to, document); // existing capacity; move structs, never copy script buffers
        self.active = self
            .documents
            .iter()
            .position(|doc| doc.id == drag.active)
            .unwrap();
        if let Some((index, _)) = &mut self.selection {
            if let Some(id) = selected {
                *index = self.documents.iter().position(|doc| doc.id == id).unwrap();
            } else {
                self.selection = None;
            }
        }
        self.layout.editor_columns = self.editor_columns_snapshot();
        self.document_save.set_script_document(drag.active);
        self.document_save.activate(document_save::Pane::Script);
        self.script_tab_reveal = Some(drag.source);
        self.recovery_dirty = true;
        ctx.request_repaint();
    }

    fn finish_script_tab_order(&mut self, ctx: &egui::Context) {
        let Some(ticket) = self.editor_columns.tab_order else {
            return;
        };
        if ticket.viewport != ctx.viewport_id() || ctx.will_discard() {
            return;
        }
        self.editor_columns.tab_order = None;
        let stamp = (ticket.viewport, ticket.frame);
        if ticket.frame != ctx.cumulative_frame_nr()
            || self.editor_columns.tab_order_finished == Some(stamp)
        {
            return;
        }
        self.editor_columns.tab_order_finished = Some(stamp);
        if !self.script_tab_order_available(ctx)
            || egui::Popup::is_any_open(ctx)
            || ticket.document != self.documents[self.active].id
            || ticket.interaction != self.document_save.interaction_generation()
        {
            return;
        }
        let neighbor = if ticket.left {
            self.active.checked_sub(1)
        } else {
            (self.active + 1 < self.documents.len()).then_some(self.active + 1)
        };
        let Some(neighbor) = neighbor.filter(|&index| self.documents[index].id == ticket.neighbor)
        else {
            return;
        };
        let selected_document = self
            .selection
            .as_ref()
            .and_then(|(index, _)| self.documents.get(*index))
            .map(|document| document.id);
        self.documents.swap(self.active, neighbor);
        self.active = neighbor; // same active stable ID; do not swap/replace either Editor state
        if let Some((index, _)) = &mut self.selection {
            if let Some(selected) = selected_document {
                *index = self
                    .documents
                    .iter()
                    .position(|document| document.id == selected)
                    .unwrap();
            } else {
                self.selection = None;
            }
        }
        self.layout.editor_columns = self.editor_columns_snapshot();
        self.document_save.set_script_document(ticket.document);
        self.document_save.activate(document_save::Pane::Script);
        self.script_tab_reveal = Some(ticket.document);
        self.recovery_dirty = true;
        // Geometry stays attached to the same two stable Document IDs. Preserve
        // both Editor instances, their cursor/find/scroll state and selections.
        ctx.request_repaint();
    }

    pub(super) fn finish_editor_columns(&mut self, ctx: &egui::Context) {
        self.finish_script_tab_drag(ctx);
        self.finish_script_tab_order(ctx);
        if !self.editor_viewport_allowed(ctx) {
            return;
        }
        if ctx.will_discard() {
            return;
        }
        let Some((frame, split, document)) = self.editor_columns.pending.take() else {
            return;
        };
        if frame != ctx.cumulative_frame_nr() || self.editor_columns.finished_frame == Some(frame) {
            return;
        }
        self.editor_columns.finished_frame = Some(frame);
        if self.file_action_pending()
            || self.recovering_session
            || self.pending_quit
            || self.allow_close
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || self.documents[self.active].id != document
        {
            return;
        }
        if split {
            if let Some(other) = self.documents.iter().find(|doc| doc.id != document) {
                self.editor_columns.other = Some(other.id);
                self.editor_columns.owner = Some(document);
                self.editor_columns.active_right = false;
                self.editor_columns.secondary = Default::default();
                self.select_document_layout(layout::PaneMode::Editor);
            }
        } else {
            self.collapse_editor_columns();
        }
        self.layout.editor_columns = self.editor_columns_snapshot();
        self.recovery_dirty = true;
        ctx.request_repaint();
    }

    pub(super) fn script_editor_views(&mut self, ui: &mut egui::Ui) {
        #[cfg(test)]
        {
            self.editor_columns.headings = [None; 2];
        }
        self.sync_editor_columns();
        let other = self
            .editor_columns
            .other
            .and_then(|id| self.documents.iter().position(|doc| doc.id == id));
        let indices = other.map(|other| {
            if self.editor_columns.active_right {
                [other, self.active]
            } else {
                [self.active, other]
            }
        });
        self.editor_columns.regions = [None; 2];
        self.editor_columns.painted = Some((
            self.layout.mode,
            self.compact_figure,
            self.layout.center,
            ui.ctx().content_rect(),
        ));
        let mut results = [None, None];
        if let Some(indices) = indices {
            let area = ui.available_rect_before_wrap();
            let gap = 8.0_f32.min(area.width().max(0.0));
            let width = ((area.width() - gap) * 0.5).max(1.0);
            ui.allocate_rect(area, egui::Sense::hover());
            for (side, index) in indices.into_iter().enumerate() {
                let rect = egui::Rect::from_min_size(
                    area.min + egui::vec2(side as f32 * (width + gap), 0.0),
                    egui::vec2(width, area.height().max(1.0)),
                );
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt(("script_column", side))
                        .max_rect(rect),
                );
                child.set_clip_rect(rect.intersect(ui.clip_rect()));
                shell::active_header(&child, index == self.active);
                let _heading = child
                    .add(
                        egui::Label::new(egui::RichText::new(self.documents[index].title()).color(
                            if index == self.active {
                                tokens::palette(child.ctx()).accent
                            } else {
                                child.visuals().text_color()
                            },
                        ))
                        .truncate(),
                    )
                    .on_hover_ui(|ui| {
                        ui.label(self.documents[index].path.as_ref().map_or_else(
                            || "Unsaved script".to_owned(),
                            |path| crate::path_label::display(path).to_string(),
                        ));
                    });
                #[cfg(test)]
                {
                    self.editor_columns.headings[side] = Some(HeadingObservation {
                        document: self.documents[index].id,
                        active: index == self.active,
                        rect: _heading.rect,
                        clip: child.clip_rect(),
                        frame: child.ctx().cumulative_frame_nr(),
                    });
                }
                self.editor_columns.regions[side] = Some(Region {
                    document: self.documents[index].id,
                    rect: child.clip_rect(),
                    layer: child.layer_id(),
                });
                results[side] = Some(self.paint_script_view(&mut child, index));
            }
        } else {
            self.editor_columns.regions[0] = Some(Region {
                document: self.documents[self.active].id,
                rect: ui.available_rect_before_wrap().intersect(ui.clip_rect()),
                layer: ui.layer_id(),
            });
            results[0] = Some(self.paint_script_view(ui, self.active));
        }
        if !ui.ctx().will_discard()
            && !self.file_action_pending()
            && !self.recovering_session
            && !self.pending_quit
            && !self.allow_close
            && let Some((index, _, _, _)) = results
                .iter()
                .flatten()
                .find(|(_, _, changed, focused)| *changed || *focused)
        {
            self.activate_script_index(*index);
        }
        for (index, selection, changed, _) in results.into_iter().flatten() {
            if index == self.active
                && let Some(selection) = selection
            {
                self.selection = Some((index, selection));
            }
            if changed {
                self.document_save.activate(document_save::Pane::Script);
                self.recovery_dirty = true;
            }
        }
    }

    fn paint_script_view(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
    ) -> (usize, Option<std::ops::Range<usize>>, bool, bool) {
        let document = &mut self.documents[index];
        let error_line = self
            .editor_issue
            .as_ref()
            .filter(|issue| {
                document.id == issue.origin.document
                    && document.text.as_str() == issue.origin.source.as_ref()
            })
            .map(|issue| issue.diagnostic.line);
        let editor = if index == self.active {
            &mut self.editor
        } else {
            &mut self.editor_columns.secondary
        };
        let before = ui.memory(|m| m.focused());
        let (selection, changed) = editor.show(ui, document, error_line);
        if let Some(response) = ui.ctx().read_response(editor::id(document.id)) {
            focus_areas::register(ui, &response, focus_areas::Area::Editor(document.id));
        }
        let focused = ui.memory(|m| m.has_focus(editor::id(document.id)))
            && before != Some(editor::id(document.id));
        (index, selection, changed, focused)
    }
}
