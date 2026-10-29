//! Transient active-document routing. No document data or persistent session state.
use super::detached_figures::{Action, TileAuthority};
use super::{Workspace, egui, layout::PaneMode};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Pane {
    #[default]
    Script,
    Figure,
    Preview,
    Model,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Script { document: u64, save_as: bool },
    Figure { figure: u32, save_as: bool },
    PreviewCopy,
    Model { save_as: bool },
}

#[derive(Clone, Copy)]
enum Intent {
    Save { save_as: bool },
    Close,
}

#[derive(Clone, Copy)]
struct Region {
    pane: Pane,
    rect: egui::Rect,
    layer: egui::LayerId,
}

#[derive(Clone)]
struct TileRegion {
    authority: TileAuthority,
    rect: egui::Rect,
    layer: egui::LayerId,
}
struct Pending {
    frame: u64,
    intent: Intent,
    pane: Pane,
    tiled: bool,
    authority: Option<TileAuthority>,
    script: Option<u64>,
}

#[derive(Clone, Copy)]
struct ObservedScriptPress {
    viewport: egui::ViewportId,
    frame: u64,
    position: egui::Pos2,
    before: u64,
    after: u64,
}

#[derive(Default)]
pub(super) struct State {
    script_document: Option<u64>,
    tiled: bool,
    tile_regions: [Option<TileRegion>; 4],
    active_tile: Option<TileAuthority>,
    active: Pane,
    last_editable: Pane,
    preview_return: Pane,
    model_generation: Option<(u64, bool)>,
    // Exactly one slot per document kind, cleared on every layout pass.
    regions: [Option<Region>; 4],
    last_focus: Option<egui::Id>,
    compact: bool,
    interaction_generation: u64,
    script_press: Option<ObservedScriptPress>,
    opening_generation: Option<u64>,
    figure_open_generation: Option<(u64, bool)>,
    preview_generation: Option<(u64, bool)>,
    composing_editor: Option<egui::Id>,
    overlay_previous: bool,
    overlay_this_pass: bool,
    pending: Option<Pending>,
    dispatched_frame: Option<u64>,
    #[cfg(test)]
    pub(super) intercepted: Option<Vec<Request>>,
}

// Shared input semantics keep retained editor focus from taking over Save.
pub(super) fn editor_input_events(events: &[egui::Event], composing: &mut bool) -> bool {
    let mut editing = false;
    for event in events {
        editing |= match event {
            egui::Event::Text(text) | egui::Event::Paste(text) => !text.is_empty(),
            egui::Event::Cut => true,
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                let changed = !text.is_empty() || *composing;
                *composing = !text.is_empty();
                changed
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                let changed = !text.is_empty() || *composing;
                *composing = false;
                changed
            }
            egui::Event::Ime(egui::ImeEvent::DeleteSurrounding {
                before_chars,
                after_chars,
            }) => *before_chars != 0 || *after_chars != 0,
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                matches!(
                    key,
                    egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                        | egui::Key::Home
                        | egui::Key::End
                        | egui::Key::PageUp
                        | egui::Key::PageDown
                        | egui::Key::Backspace
                        | egui::Key::Delete
                        | egui::Key::Enter
                        | egui::Key::Tab
                ) || ((modifiers.ctrl || modifiers.command)
                    && matches!(
                        key,
                        egui::Key::A
                            | egui::Key::C
                            | egui::Key::X
                            | egui::Key::V
                            | egui::Key::Z
                            | egui::Key::Y
                            | egui::Key::F
                            | egui::Key::H
                    ))
            }
            _ => false,
        };
    }
    editing
}

impl State {
    pub(super) fn set_script_document(&mut self, document: u64) {
        self.script_document = Some(document);
    }
    pub(super) fn set_tiled(&mut self, tiled: bool) {
        self.tiled = tiled;
        if !tiled {
            self.tile_regions = Default::default();
            self.active_tile = None;
        }
    }
    pub(super) fn register_tile(
        &mut self,
        index: usize,
        authority: TileAuthority,
        rect: egui::Rect,
        layer: egui::LayerId,
    ) {
        if let Some(slot) = self.tile_regions.get_mut(index) {
            *slot = Some(TileRegion {
                authority,
                rect,
                layer,
            });
        }
    }
    pub(super) fn default_tile(&mut self, authority: TileAuthority) {
        // Refresh a retained active identity's revision after accepted edits,
        // but never rewrite an already captured Save/Close authority.
        if self
            .active_tile
            .as_ref()
            .is_none_or(|old| old.key.same(&authority.key))
        {
            self.active_tile = Some(authority);
        }
    }
    pub(super) fn activate_tile(&mut self, authority: TileAuthority) {
        self.active_tile = Some(authority);
        self.activate(Pane::Figure);
    }

    pub(super) fn activate(&mut self, pane: Pane) {
        // One scalar generation, never an event log or document-data snapshot.
        self.interaction_generation = self.interaction_generation.wrapping_add(1);
        if pane == Pane::Preview && self.active != Pane::Preview {
            self.preview_return = self.active;
        }
        self.active = pane;
        if matches!(pane, Pane::Script | Pane::Figure) {
            self.last_editable = pane;
        }
    }

    pub(super) fn preview_closed(&mut self) {
        self.preview_generation = None;
        self.regions[2] = None;
        if self.active == Pane::Preview {
            self.activate(self.preview_return);
        }
    }

    pub(super) fn model_closed(&mut self) {
        self.regions[3] = None;
        if self.preview_return == Pane::Model {
            self.preview_return = self.last_editable;
        }
        if self.active == Pane::Model {
            self.activate(self.last_editable);
        }
    }

    pub(super) fn command_selected(&mut self) {
        // Command has no document Save/Close target; retain the last real
        // document while invalidating a pending background activation.
        self.interaction_generation = self.interaction_generation.wrapping_add(1);
    }

    pub(super) fn figure_open_started(&mut self, activate: bool) {
        self.figure_open_generation = Some((self.interaction_generation, activate));
    }

    pub(super) fn figure_open_completed(&mut self) -> Option<bool> {
        self.figure_open_generation
            .take()
            .map(|(generation, activate)| activate && generation == self.interaction_generation)
    }

    pub(super) fn model_read_started(&mut self) {
        self.model_generation
            .get_or_insert((self.interaction_generation, true));
    }

    pub(super) fn suppress_model_activation(&mut self) {
        self.model_generation = Some((self.interaction_generation, false));
    }

    /// True when the opened model became the active document.
    pub(super) fn model_read_completed(&mut self, success: bool) -> bool {
        let activate = self
            .model_generation
            .take()
            .is_some_and(|(generation, activate)| {
                activate && generation == self.interaction_generation
            });
        if success && activate {
            self.activate(Pane::Model);
        }
        success && activate
    }

    pub(super) fn cancel_model_activation(&mut self) {
        self.model_generation = None;
    }

    pub(super) fn begin_pass(&mut self, ctx: &egui::Context, editor_id: egui::Id) {
        self.overlay_this_pass = self.overlay_previous || Self::overlay_open(ctx);
        // Observe presses against the last rendered regions before receiving a
        // background result that could install an overlaid preview this frame.
        self.observe_interaction(ctx);
        self.observe_editor_input(ctx, editor_id);
        self.regions = [None; 4];
        self.tile_regions = Default::default();
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.frame != ctx.cumulative_frame_nr())
        {
            self.pending = None;
        }
    }

    /// Inspect the same ordered IME/edit events before selecting an editor column.
    /// The ordinary observer owns composition updates later in this pass.
    pub(super) fn editor_input_activity(&self, ctx: &egui::Context, focused: egui::Id) -> bool {
        let mut composing = self.composing_editor == Some(focused);
        ctx.input(|input| editor_input_events(&input.events, &mut composing))
    }

    fn observe_editor_input(&mut self, ctx: &egui::Context, editor_id: egui::Id) {
        if self.overlay_this_pass || Self::overlay_open(ctx) {
            return;
        }
        let focused = ctx.memory(|memory| memory.focused());
        let pane = if focused == Some(editor_id) && self.regions[0].is_some() {
            Some(Pane::Script)
        } else if let Some(response) = focused.and_then(|id| ctx.read_response(id))
            && self.regions[3].is_some_and(|region| {
                let visible = region.rect.intersect(response.interact_rect);
                region.layer == response.layer_id && visible.is_finite() && visible.is_positive()
            })
        {
            Some(Pane::Model)
        } else {
            None
        };
        let Some(pane) = pane else {
            self.composing_editor = None;
            return;
        };
        let mut composing = self.composing_editor == focused;
        let editing = ctx.input(|input| editor_input_events(&input.events, &mut composing));
        self.composing_editor = if composing { focused } else { None };
        // Generic Save keys are deliberately absent: merely retaining editor
        // focus after selecting the figure must never reroute Ctrl+S to Script.
        if editing {
            self.activate(pane);
        }
    }

    pub(super) fn open_started(&mut self) {
        self.opening_generation = Some(self.interaction_generation);
    }

    pub(super) fn open_completed(&mut self) -> bool {
        self.opening_generation
            .take()
            .is_none_or(|generation| generation == self.interaction_generation)
    }

    pub(super) fn preview_started(&mut self) {
        self.preview_generation = Some((self.interaction_generation, true));
    }

    pub(super) fn suppress_preview_activation(&mut self) {
        if let Some((_, activate)) = &mut self.preview_generation {
            *activate = false;
        }
    }

    pub(super) fn preview_installed(&mut self) {
        if self
            .preview_generation
            .take()
            .is_none_or(|(generation, activate)| {
                activate && generation == self.interaction_generation
            })
        {
            self.activate(Pane::Preview);
        }
    }

    pub(super) fn interaction_generation(&self) -> u64 {
        self.interaction_generation
    }

    pub(super) fn is_compact(&self) -> bool {
        self.compact
    }

    pub(super) fn set_compact(&mut self, compact: bool) {
        self.compact = compact;
    }

    fn overlay_open(ctx: &egui::Context) -> bool {
        egui::Popup::is_any_open(ctx) || ctx.memory(|memory| memory.top_modal_layer().is_some())
    }

    pub(super) fn register_ui(&mut self, pane: Pane, ui: &egui::Ui) {
        self.register(
            pane,
            ui.available_rect_before_wrap().intersect(ui.clip_rect()),
            ui.layer_id(),
        );
    }

    pub(super) fn register(&mut self, pane: Pane, rect: egui::Rect, layer: egui::LayerId) {
        let index = match pane {
            Pane::Script => 0,
            Pane::Figure => 1,
            Pane::Preview => 2,
            Pane::Model => 3,
        };
        self.regions[index] = Some(Region { pane, rect, layer });
    }

    fn observe_interaction(&mut self, ctx: &egui::Context) {
        let focus = ctx.memory(|memory| memory.focused());
        // A TextEdit can retain keyboard focus after clicking a plot. Continuing
        // text focus must not take the document back on every subsequent frame.
        // Menu navigation/restoration of an earlier text focus is not an
        // explicit document activation. Still record the ID to avoid treating
        // that restored focus as a new transition on the next frame.
        if !self.overlay_this_pass
            && !Self::overlay_open(ctx)
            && focus != self.last_focus
            && let Some(response) = focus.and_then(|id| ctx.read_response(id))
            && let Some(region) = self.regions.iter().flatten().find(|region| {
                let visible = region.rect.intersect(response.interact_rect);
                region.layer == response.layer_id && visible.is_finite() && visible.is_positive()
            })
        {
            self.activate(region.pane);
        }
        if self.tiled
            && !self.overlay_this_pass
            && !Self::overlay_open(ctx)
            && focus != self.last_focus
            && let Some(response) = focus.and_then(|id| ctx.read_response(id))
            && let Some(region) = self.tile_regions.iter().flatten().find(|region| {
                let visible = region.rect.intersect(response.interact_rect);
                region.layer == response.layer_id && visible.is_finite() && visible.is_positive()
            })
        {
            self.activate_tile(region.authority.clone());
        }
        self.last_focus = focus;
        self.observe_pointer(ctx);
    }

    fn observe_pointer(&mut self, ctx: &egui::Context) {
        if self.tiled && !self.overlay_this_pass && !Self::overlay_open(ctx) {
            let position = ctx.input(|input| {
                input.events.iter().rev().find_map(|event| match event {
                    egui::Event::PointerButton {
                        pos, pressed: true, ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            if let Some(position) = position
                && let Some(layer) = ctx.layer_id_at(position)
                && let Some(region) = self
                    .tile_regions
                    .iter()
                    .flatten()
                    .find(|r| r.layer == layer && r.rect.contains(position))
            {
                self.activate_tile(region.authority.clone());
                return;
            }
        }
        // Real presses win over simultaneous focus transitions. Top-layer checks
        // exclude menus, modal windows and preview areas covering another pane.
        if let Some(position) = ctx.input(|input| {
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos, pressed: true, ..
                } => Some(*pos),
                _ => None,
            })
        }) && let Some(layer) = ctx.layer_id_at(position)
            && let Some(region) = self
                .regions
                .iter()
                .flatten()
                .find(|region| region.layer == layer && region.rect.contains(position))
        {
            let pane = region.pane;
            let before = self.interaction_generation;
            self.activate(pane);
            if pane == Pane::Script {
                self.script_press = Some(ObservedScriptPress {
                    viewport: ctx.viewport_id(),
                    frame: ctx.cumulative_frame_nr(),
                    position,
                    before,
                    after: self.interaction_generation,
                });
            }
        }
    }

    /// A tab press is observed again after paint. Acknowledge only that exact
    /// observer transition, never another action or a later frame's generation.
    pub(super) fn observed_script_press_generation(
        &self,
        ctx: &egui::Context,
        position: egui::Pos2,
        before: u64,
    ) -> Option<u64> {
        self.script_press
            .filter(|press| {
                press.viewport == ctx.viewport_id()
                    && press.frame == ctx.cumulative_frame_nr()
                    && press.position == position
                    && press.before == before
                    && press.after == self.interaction_generation
            })
            .map(|press| press.after)
    }

    pub(super) fn target(
        &self,
        mode: PaneMode,
        compact_figure: bool,
        preview_open: bool,
        model_open: bool,
    ) -> Pane {
        if self.active == Pane::Preview && preview_open {
            return Pane::Preview;
        }
        if self.active == Pane::Model && model_open {
            return Pane::Model;
        }
        match mode {
            PaneMode::Editor => Pane::Script,
            PaneMode::Figure => Pane::Figure,
            PaneMode::Split if self.compact => {
                if compact_figure {
                    Pane::Figure
                } else {
                    Pane::Script
                }
            }
            PaneMode::Split => {
                if matches!(self.active, Pane::Preview | Pane::Model) {
                    self.last_editable
                } else {
                    self.active
                }
            }
        }
    }

    pub(super) fn queue(&mut self, frame: u64, save_as: bool, pane: Pane) {
        self.queue_intent(frame, Intent::Save { save_as }, pane);
    }

    pub(super) fn queue_close(&mut self, frame: u64, pane: Pane) {
        self.queue_intent(frame, Intent::Close, pane);
    }

    fn queue_intent(&mut self, frame: u64, intent: Intent, pane: Pane) {
        if self.dispatched_frame != Some(frame) && self.pending.is_none() {
            self.pending = Some(Pending {
                frame,
                intent,
                pane,
                tiled: self.tiled,
                authority: self.active_tile.clone(),
                script: (pane == Pane::Script)
                    .then_some(self.script_document)
                    .flatten(),
            });
        }
    }

    fn take(&mut self, frame: u64) -> Option<Pending> {
        let pending = self.pending.take()?;
        if pending.frame != frame || self.dispatched_frame == Some(frame) {
            return None;
        }
        self.dispatched_frame = Some(frame);
        Some(pending)
    }
}

impl Workspace {
    pub(super) fn show_model_document(&mut self) {
        self.toolkits.open_tool(super::toolkits::Tool::Model);
        self.document_save.activate(Pane::Model);
    }

    pub(super) fn generic_save_target(&self) -> Pane {
        self.document_save.target(
            self.layout.mode,
            self.compact_figure,
            self.image_view.is_some(),
            self.toolkits.model_open,
        )
    }

    fn generic_document_available(&self) -> bool {
        self.generic_save_target() != Pane::Script
            || self
                .layout
                .center
                .is_none_or(|center| center.editor_revealed)
    }

    pub(super) fn generic_save_menu(&mut self, ui: &mut egui::Ui) {
        let target = self.generic_save_target();
        let enabled = self.generic_document_available()
            && !self.busy
            && !self.file_action_pending()
            && (target != Pane::Figure || self.figure.is_some());
        let (save, save_as) = match target {
            Pane::Script => ("Save script", "Save script as…"),
            Pane::Figure => ("Save figure", "Save figure as…"),
            Pane::Preview => ("Save preview copy…", "Save preview copy as…"),
            Pane::Model => ("Save model", "Save model as…"),
        };
        for (label, shortcut, save_as) in [(save, "Ctrl+S", false), (save_as, "Ctrl+Shift+S", true)]
        {
            if ui
                .add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
                .clicked()
            {
                self.document_save
                    .queue(ui.ctx().cumulative_frame_nr(), save_as, target);
                ui.close();
            }
        }
    }

    pub(super) fn generic_close_menu(&mut self, ui: &mut egui::Ui) {
        let target = self.generic_save_target();
        let enabled = self.generic_document_available()
            && !self.busy
            && !self.file_action_pending()
            && !self.allow_close;
        let label = match target {
            Pane::Script => "Close script",
            Pane::Figure => "Close figure",
            Pane::Preview => "Close preview",
            Pane::Model => "Close model view",
        };
        if ui
            .add_enabled(
                enabled && (target != Pane::Figure || self.figure.is_some()),
                egui::Button::new(label).shortcut_text("Ctrl+W"),
            )
            .clicked()
        {
            self.document_save
                .queue_close(ui.ctx().cumulative_frame_nr(), target);
            ui.close();
        }
        // Preserve the explicitly named script command when another document
        // owns Ctrl+W. Script tabs retain their existing individual close buttons.
        if target != Pane::Script
            && ui
                .add_enabled(enabled, egui::Button::new("Close script"))
                .clicked()
        {
            self.request_script_close(self.active);
            ui.close();
        }
    }

    fn close_active_document(&mut self) {
        match self.generic_save_target() {
            Pane::Script => self.request_script_close(self.active),
            Pane::Figure
                if self.figure.is_some() && self.figure_ids.contains(&self.current_figure) =>
            {
                self.request_figure_close(self.current_figure);
            }
            Pane::Figure => {
                self.status = "No editable figure to close in the active plot pane".into();
            }
            Pane::Preview => self.close_image_preview(),
            Pane::Model => {
                self.close_tool_tab(super::toolkits::Tool::Model);
            }
        }
    }

    pub(super) fn finish_document_action(&mut self, ctx: &egui::Context) {
        self.document_save.observe_interaction(ctx);
        self.document_save.overlay_previous = State::overlay_open(ctx);
        // Retain the single intent across an already requested layout retry.
        // A retry requested by the caller after this function is deduplicated
        // by dispatched_frame instead; publication still occurs at most once.
        if ctx.will_discard() {
            return;
        }
        let Some(pending) = self.document_save.take(ctx.cumulative_frame_nr()) else {
            return;
        };
        let intent = pending.intent;
        if pending.pane != self.generic_save_target() {
            self.status =
                "The active document changed; choose the intended document and try again".into();
            return;
        }
        if pending.pane == Pane::Script && pending.script != Some(self.documents[self.active].id) {
            self.status =
                "The active script changed; choose the intended script and try again".into();
            return;
        }
        if self.busy || self.file_action_pending() || self.allow_close {
            self.status = match intent {
                Intent::Save { .. } => {
                    "Save not started during another operation; finish it and save again"
                }
                Intent::Close => {
                    "Close not started during another operation; finish it and close again"
                }
            }
            .into();
            return;
        }
        if !self.generic_document_available() {
            self.status = "Choose New script, Open script or an existing document first".into();
            return;
        }
        if self.generic_save_target() == Pane::Figure
            && (pending.tiled || self.layout.figure_tiling.is_some())
        {
            let Some(authority) = pending
                .authority
                .filter(|authority| pending.tiled && self.detached.valid_tile(self, authority))
            else {
                self.status =
                    "The selected figure tile changed; select the intended tile and try again"
                        .into();
                return;
            };
            #[cfg(test)]
            if let Intent::Save { save_as } = intent
                && let Some(requests) = &mut self.document_save.intercepted
            {
                assert!(requests.len() < 16, "Bounded route-only test probe");
                requests.push(Request::Figure {
                    figure: authority.key.number,
                    save_as,
                });
                return;
            }
            let action = match intent {
                Intent::Save { save_as: true } => Action::SaveAs,
                Intent::Save { save_as: false } => Action::Save,
                Intent::Close => Action::Close,
            };
            if !self.dispatch_tile(ctx, &authority, action) {
                self.status = "The selected figure tile is busy or changed; try again".into();
            }
            return;
        }
        let save_as = match intent {
            Intent::Save { save_as } => save_as,
            Intent::Close => {
                self.close_active_document();
                return;
            }
        };
        let request = match self.generic_save_target() {
            Pane::Script => Request::Script {
                document: self.documents[self.active].id,
                save_as,
            },
            Pane::Figure
                if self.figure.is_some() && self.figure_ids.contains(&self.current_figure) =>
            {
                Request::Figure {
                    figure: self.current_figure,
                    save_as,
                }
            }
            Pane::Figure => {
                self.status = "No editable figure to save in the active plot pane".into();
                return;
            }
            Pane::Preview => Request::PreviewCopy,
            Pane::Model => Request::Model { save_as },
        };
        #[cfg(test)]
        if let Some(requests) = &mut self.document_save.intercepted {
            assert!(requests.len() < 16, "Bounded route-only test probe");
            requests.push(request);
            return;
        }
        match request {
            Request::Script { document, save_as } => {
                if let Some(index) = self.documents.iter().position(|doc| doc.id == document) {
                    self.save_document(index, save_as);
                }
            }
            Request::Figure { figure, save_as } => {
                if self.figure_ids.contains(&figure) {
                    self.save_figure_document(figure, save_as);
                }
            }
            Request::PreviewCopy => self.save_preview_copy(),
            Request::Model { save_as } => {
                if save_as {
                    self.toolkits.save_model_as();
                } else {
                    self.toolkits.save_model();
                }
                self.advance_model_read();
            }
        }
    }
}
