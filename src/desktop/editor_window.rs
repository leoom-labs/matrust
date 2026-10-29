//! One immediate native Editor window borrowing the existing Workspace owner.
use super::*;

pub(super) fn group_id() -> egui::Id {
    egui::Id::new("shared_native_editor_group")
}

pub(super) fn viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("matrust_native_editor")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Undock,
    Dock,
    Save(bool),
    Run(bool),
    Close(u64),
    New,
    Open,
    Cycle(bool),
    Command,
    Focus,
}

#[derive(Clone, Copy)]
struct Ticket {
    viewport: egui::ViewportId,
    frame: u64,
    generation: u64,
    document: u64,
    interaction: u64,
    action: Action,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) detached: bool,
    pub(super) in_callback: bool,
    embedded: bool,
    painted: Option<(egui::ViewportId, u64, bool)>, // viewport, frame, window host
    generation: u64,
    pending: Option<Ticket>,
    close: Option<(egui::ViewportId, u64, u64)>, // viewport, frame, placement generation
    finished: Option<(egui::ViewportId, u64)>,
    focus: Option<(u64, u64, u64)>, // placement generation, document, interaction
    root_command: Option<(u64, u64, u64)>,
    composing: Option<egui::Id>,
    context: Option<egui::Context>,
    reveal: Option<(egui::ViewportId, u64, u64, u64)>, // origin viewport/frame, placement, interaction
    #[cfg(test)]
    dispatched: Vec<(u64, &'static str)>,
}

impl State {
    pub(super) fn clear_focus(&mut self) {
        self.focus = None;
        self.reveal = None;
    }
    pub(super) fn cancel(&mut self) {
        if let Some(ticket) = self.pending.take() {
            self.finished = Some((ticket.viewport, ticket.frame));
        }
        self.focus = None;
        self.reveal = None;
        self.close = None;
        self.root_command = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

impl Workspace {
    fn editor_host_paint_conflicts(&self, ctx: &egui::Context, window: bool) -> bool {
        self.editor_window
            .painted
            .is_some_and(|(viewport, frame, host)| {
                viewport == ctx.viewport_id()
                    && frame == ctx.cumulative_frame_nr()
                    && host != window
            })
    }

    pub(super) fn begin_editor_host_paint(&mut self, ctx: &egui::Context) -> bool {
        let window = self.editor_window.in_callback;
        if self.editor_host_paint_conflicts(ctx, window) {
            ctx.request_repaint();
            return false;
        }
        // Keep this stamp through Dock/Undock/Stop: changing placement cannot
        // erase an already painted shared widget tree in this viewport/frame.
        self.editor_window.painted = Some((ctx.viewport_id(), ctx.cumulative_frame_nr(), window));
        true
    }

    pub(super) fn observe_editor_window_root(&mut self, ctx: &egui::Context) {
        if ctx.viewport_id() != egui::ViewportId::ROOT {
            return;
        }
        self.editor_window.context = Some(ctx.clone());
        if let Some((generation, document, interaction)) = self.editor_window.root_command.take()
            && generation == self.editor_window.generation
            && document == self.documents[self.active].id
            && interaction == self.document_save.interaction_generation()
            && !self.file_action_pending()
            && !self.pending_quit
            && !self.allow_close
        {
            self.queue_pane_focus(ctx, navigation::FocusPane::Command);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    // New calls reveal before its final Document exists. Capture only the
    // origin pass and interaction, then resolve the final active ID below.
    pub(super) fn reveal_editor_window(&mut self) {
        if self.editor_window.detached
            && let Some(ctx) = &self.editor_window.context
        {
            self.editor_window.reveal = Some((
                ctx.viewport_id(),
                ctx.cumulative_frame_nr(),
                self.editor_window.generation,
                self.document_save.interaction_generation(),
            ));
        }
    }

    fn finish_editor_window_reveal(&mut self, ctx: &egui::Context) {
        let Some((origin, frame, generation, interaction)) = self.editor_window.reveal else {
            return;
        };
        if origin != ctx.viewport_id() {
            return;
        }
        if ctx.will_discard() {
            return;
        }
        self.editor_window.reveal = None;
        if frame != ctx.cumulative_frame_nr()
            || generation != self.editor_window.generation
            || interaction != self.document_save.interaction_generation()
            || !self.editor_window.detached
            || !self.editor_window_context_available(ctx)
            || self.busy
        {
            return;
        }
        // A completed foreground Open may arrive while another application is
        // active. Retain the document selection without stealing OS focus.
        let owns_native_focus = ctx.input_for(egui::ViewportId::ROOT, |input| {
            input.viewport().focused != Some(false)
        }) || ctx
            .input_for(viewport(), |input| input.viewport().focused == Some(true));
        if owns_native_focus {
            self.editor_window.focus =
                Some((generation, self.documents[self.active].id, interaction));
            ctx.send_viewport_cmd_to(viewport(), egui::ViewportCommand::Focus);
            ctx.request_repaint_of(viewport());
        }
    }

    pub(super) fn editor_viewport_allowed(&self, ctx: &egui::Context) -> bool {
        if self.editor_window.detached {
            self.editor_window.in_callback
                && (ctx.viewport_id() == viewport()
                    || self.editor_window.embedded && ctx.viewport_id() == egui::ViewportId::ROOT)
        } else {
            ctx.viewport_id() == egui::ViewportId::ROOT
        }
    }

    pub(super) fn editor_focus_viewport(&self) -> egui::ViewportId {
        if self.editor_window.detached && !self.editor_window.embedded {
            viewport()
        } else {
            egui::ViewportId::ROOT
        }
    }

    fn editor_window_context_available(&self, ctx: &egui::Context) -> bool {
        self.editor_window_control_context(ctx) && !egui::Popup::is_any_open(ctx)
    }

    fn editor_window_control_context(&self, ctx: &egui::Context) -> bool {
        !self.file_action_pending()
            && !self.recovering_session
            && !self.pending_quit
            && !self.allow_close
            && (!egui::Popup::is_any_open(ctx)
                || egui::Popup::is_id_open(ctx, egui::Id::new("editor_actions")))
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && ctx.input(|input| !input.viewport().close_requested())
    }

    fn editor_window_available(&self, ctx: &egui::Context) -> bool {
        self.editor_window_context_available(ctx)
            && ctx.input(|input| input.viewport().focused != Some(false))
    }

    pub(super) fn editor_window_input_activity(
        &self,
        ctx: &egui::Context,
        focused: egui::Id,
    ) -> bool {
        let mut composing = self.editor_window.composing == Some(focused);
        ctx.input(|input| document_save::editor_input_events(&input.events, &mut composing))
    }

    fn queue_editor_window_action(&mut self, ctx: &egui::Context, action: Action) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if ctx.viewport_id() != egui::ViewportId::ROOT && !self.editor_viewport_allowed(ctx) {
            return;
        }
        if self.editor_window.finished == Some(stamp)
            || self.editor_window.pending.is_some()
            || !self.editor_window_available(ctx)
            || (self.busy && action != Action::Dock)
            || (action == Action::Undock && self.has_startup_placeholder())
        {
            return;
        }
        self.editor_window.pending = Some(Ticket {
            viewport: stamp.0,
            frame: stamp.1,
            generation: self.editor_window.generation,
            document: self.script_action_document(ctx),
            interaction: self.document_save.interaction_generation(),
            action,
        });
    }

    pub(super) fn queue_editor_tab_close(&mut self, ctx: &egui::Context, index: usize) {
        if let Some(document) = self.documents.get(index).map(|doc| doc.id) {
            self.queue_editor_window_action(ctx, Action::Close(document));
        }
    }

    pub(super) fn queue_editor_window_focus(&mut self, ctx: &egui::Context) {
        if self.editor_window.detached && self.editor_window_available(ctx) {
            self.editor_window.focus = Some((
                self.editor_window.generation,
                self.documents[self.active].id,
                self.document_save.interaction_generation(),
            ));
            ctx.request_repaint_of(viewport());
        }
    }

    // Only changes observed synchronously inside this Editor paint may advance
    // its ticket. Root events between discarded child passes cannot do so.
    fn acknowledge_editor_window_interaction(&mut self, before: u64) {
        if let Some(ticket) = &mut self.editor_window.pending
            && ticket.interaction == before
        {
            ticket.interaction = self.document_save.interaction_generation();
        }
    }

    pub(super) fn dock_editor_window(&mut self, ctx: &egui::Context) {
        self.focus_areas.retire_editor_host();
        self.editor_window.cancel();
        self.editor_window.detached = false;
        self.editor_window.composing = None;
        self.reset_editor_host();
        self.reset_script_focus_host();
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
    }

    pub(super) fn finish_editor_window_action(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(ticket) = self.editor_window.pending else {
            return;
        };
        // Root and immediate child counters can be numerically equal. Never
        // consume or deduplicate another viewport's ticket here.
        if ticket.viewport != ctx.viewport_id() {
            return;
        }
        self.editor_window.pending = None;
        let stamp = (ticket.viewport, ticket.frame);
        if ticket.frame != ctx.cumulative_frame_nr() || self.editor_window.finished == Some(stamp) {
            return;
        }
        self.editor_window.finished = Some(stamp);
        if ticket.generation != self.editor_window.generation
            || ticket.document != self.documents[self.active].id
            || ticket.interaction != self.document_save.interaction_generation()
            || !self.editor_window_available(ctx)
            || (self.busy && ticket.action != Action::Dock)
        {
            return;
        }
        match ticket.action {
            Action::Undock => {
                if self.editor_window.detached || self.has_startup_placeholder() {
                    return;
                }
                self.editor_window.cancel();
                self.editor_window.detached = true;
                self.reset_editor_host();
                self.reset_script_focus_host();
                self.queue_editor_window_focus(ctx);
                ctx.send_viewport_cmd_to(viewport(), egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
            Action::Dock => self.dock_editor_window(ctx),
            action => {
                if !self.editor_viewport_allowed(ctx) || !self.editor_window.in_callback {
                    return;
                }
                #[cfg(test)]
                {
                    assert!(self.editor_window.dispatched.len() < 128);
                    let name = match action {
                        Action::Save(_) => "save",
                        Action::Run(false) => "run",
                        Action::Run(true) => "selection",
                        Action::Close(_) => "close",
                        Action::New => "new",
                        Action::Open => "open",
                        Action::Cycle(_) => "cycle",
                        Action::Command => "command",
                        Action::Focus => "focus",
                        _ => unreachable!(),
                    };
                    self.editor_window.dispatched.push((ticket.document, name));
                }
                match action {
                    Action::Save(save_as) => {
                        self.save_document(self.active, save_as);
                    }
                    Action::Run(true) => self.run_selection(),
                    Action::Run(false) => self.run_document(None),
                    Action::Close(document) => {
                        if let Some(index) =
                            self.documents.iter().position(|doc| doc.id == document)
                        {
                            self.request_script_close(index);
                        }
                    }
                    Action::New => self.new_document(""),
                    Action::Open => self.open_document(),
                    Action::Cycle(backwards) => self.cycle_script(ctx, backwards),
                    Action::Command => {
                        self.editor_window.clear_focus();
                        self.editor_window.root_command = Some((
                            self.editor_window.generation,
                            ticket.document,
                            self.document_save.interaction_generation(),
                        ));
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Focus,
                        );
                    }
                    Action::Focus => self.queue_editor_window_focus(ctx),
                    _ => unreachable!(),
                }
                // Confirmation UI is deliberately owned by the root. The
                // pending worker itself needs no focus transfer.
                if self.pending_close.is_some()
                    || self.script_conflict.is_some()
                    || self.script_reload.is_some()
                {
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                }
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        }
    }

    pub(super) fn editor_window_control(&mut self, ui: &mut egui::Ui) {
        let action = if self.editor_window.in_callback {
            Action::Dock
        } else {
            Action::Undock
        };
        let label = if action == Action::Dock {
            "Dock back"
        } else {
            "Undock Editor"
        };
        if ui
            .add_enabled(
                !self.has_startup_placeholder()
                    && self.editor_window_control_context(ui.ctx())
                    && ui.input(|input| input.viewport().focused != Some(false))
                    && (!self.busy || action == Action::Dock),
                egui::Button::new(label).small(),
            )
            .clicked()
        {
            egui::Popup::close_id(ui.ctx(), egui::Id::new("editor_actions"));
            ui.close();
            self.queue_editor_window_action(ui.ctx(), action);
        }
    }

    pub(super) fn editor_window_placeholder(&mut self, ui: &mut egui::Ui) {
        ui.heading("Editor is in a separate window");
        ui.label("Scripts remain open in the same workspace.");
        if ui.button("Focus Editor").clicked() {
            self.queue_editor_window_focus(ui.ctx());
            ui.ctx()
                .send_viewport_cmd_to(viewport(), egui::ViewportCommand::Focus);
        }
        if ui.button("Dock back").clicked() {
            self.queue_editor_window_action(ui.ctx(), Action::Dock);
        }
    }

    fn editor_window_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.editor_window_available(ctx) {
            return;
        }
        // Find/Replace establishes its own query focus during paint. It must
        // supersede an older generic focus request from the root/placeholder.
        if ctx.input(|input| {
            input.modifiers.ctrl
                && (input.key_pressed(egui::Key::F) || input.key_pressed(egui::Key::H))
        }) {
            self.editor_window.clear_focus();
        }
        self.editor.shortcuts(ctx);
        let action = ctx.input_mut(|input| {
            fn exact(
                input: &mut egui::InputState,
                modifiers: egui::Modifiers,
                key: egui::Key,
            ) -> bool {
                input.events.iter().any(|event| {
                    matches!(event, egui::Event::Key {
                    key: actual, pressed: true, modifiers: actual_modifiers, ..
                } if *actual == key && actual_modifiers.matches_exact(modifiers))
                }) && input.consume_key(modifiers, key)
            }
            let ctrl = egui::Modifiers::CTRL;
            if exact(input, ctrl | egui::Modifiers::SHIFT, egui::Key::S) {
                Some(Action::Save(true))
            } else if exact(input, ctrl, egui::Key::S) {
                Some(Action::Save(false))
            } else if exact(input, ctrl, egui::Key::Enter) {
                Some(Action::Run(false))
            } else if exact(input, egui::Modifiers::NONE, egui::Key::F9) {
                Some(Action::Run(true))
            } else if exact(input, ctrl, egui::Key::W) {
                Some(Action::Close(self.documents[self.active].id))
            } else if exact(input, ctrl, egui::Key::N) {
                Some(Action::New)
            } else if exact(input, ctrl, egui::Key::O) {
                Some(Action::Open)
            } else if exact(input, ctrl | egui::Modifiers::SHIFT, egui::Key::Tab) {
                Some(Action::Cycle(true))
            } else if exact(input, ctrl, egui::Key::Tab) {
                Some(Action::Cycle(false))
            } else if exact(input, ctrl | egui::Modifiers::SHIFT, egui::Key::Num0) {
                Some(Action::Focus)
            } else if exact(input, ctrl, egui::Key::Num0) {
                Some(Action::Command)
            } else {
                None
            }
        });
        if let Some(action) = action {
            self.queue_editor_window_action(ctx, action);
        }
    }

    pub(super) fn editor_window_frame(&mut self, ctx: &egui::Context) {
        if !self.editor_window.detached || self.allow_close {
            return;
        }
        self.finish_editor_window_reveal(ctx);
        self.editor_window.embedded = ctx.embed_viewports();
        ctx.show_viewport_immediate(
            viewport(),
            egui::ViewportBuilder::default()
                .with_title("Editor — MatRust")
                .with_inner_size([1000., 700.])
                .with_min_inner_size([480., 320.]),
            |ui, class| {
                self.editor_window.in_callback = true;
                self.editor_window.embedded = class == egui::ViewportClass::EmbeddedWindow;
                self.editor_window_ui(ui);
                self.editor_window.in_callback = false;
            },
        );
    }

    pub(super) fn editor_window_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.set_style(ctx.global_style());
        let close_stamp = (
            ctx.viewport_id(),
            ctx.cumulative_frame_nr(),
            self.editor_window.generation,
        );
        if self
            .editor_window
            .close
            .is_some_and(|stamp| stamp != close_stamp)
        {
            self.editor_window.close = None;
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            self.editor_window.close = Some(close_stamp);
        }
        if self.editor_window.close == Some(close_stamp) {
            // Viewport events are taken after the first pass. Retain only this
            // exact native X gesture until its final pass, never a later frame.
            // It closes the container, not documents or root confirmations.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !ctx.will_discard() {
                self.dock_editor_window(&ctx);
            }
            return;
        }
        // An immediate viewport can be embedded in ROOT, including a backend
        // fallback. Undock is admitted after the docked tree was already painted.
        // Defer its input and shared tree together; actual native viewports have
        // separate widget maps and are not delayed by a ROOT paint.
        if self.editor_host_paint_conflicts(&ctx, true) {
            ctx.request_repaint();
            return;
        }
        self.begin_focus_areas(&ctx);
        if self.editor_window.pending.is_some_and(|ticket| {
            ticket.viewport == ctx.viewport_id()
                && (ticket.interaction != self.document_save.interaction_generation()
                    || !self.editor_window_available(&ctx))
        }) {
            self.editor_window.cancel();
        }
        if self.editor_window.pending.is_some_and(|ticket| {
            ticket.viewport == ctx.viewport_id() && ticket.frame != ctx.cumulative_frame_nr()
        }) {
            self.editor_window.pending = None;
        }
        let before = self.document_save.interaction_generation();
        self.observe_editor_columns(&ctx);
        self.acknowledge_editor_window_interaction(before);
        self.observe_script_focus(&ctx);
        self.editor_window_shortcuts(&ctx);
        let enabled = self.editor_window_available(&ctx);
        ui.horizontal_wrapped(|ui| {
            for (label, action) in [
                ("Save", Action::Save(false)),
                ("Save as…", Action::Save(true)),
                ("Run", Action::Run(false)),
                ("Run selection", Action::Run(true)),
                ("New script", Action::New),
                ("Open script", Action::Open),
                (
                    "Close script",
                    Action::Close(self.documents[self.active].id),
                ),
            ] {
                if ui
                    .add_enabled(enabled && !self.busy, egui::Button::new(label))
                    .clicked()
                {
                    self.queue_editor_window_action(&ctx, action);
                }
            }
            self.navigation_help_menu(ui, false);
            if self.busy && ui.button("Stop").clicked() {
                self.stop_work();
            }
        });
        if self.file_action_pending() || self.pending_quit || self.recovering_session {
            ui.label("Finish the pending operation or dialog in the desktop.");
            if ui.button("Go to desktop").clicked() {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
        }
        let before = self.document_save.interaction_generation();
        // Native focus controls admission, not appearance. An unfocused window
        // still paints its real Editor and retains document-keyed widget state.
        let paint_enabled = self.editor_window_context_available(&ctx);
        ui.add_enabled_ui(paint_enabled, |ui| self.editor_pane(ui));
        self.acknowledge_editor_window_interaction(before);
        let focused = ctx.memory(|memory| memory.focused());
        let mut composing = focused.is_some() && focused == self.editor_window.composing;
        ctx.input(|input| {
            document_save::editor_input_events(&input.events, &mut composing);
        });
        self.editor_window.composing = if composing { focused } else { None };
        self.finish_editor_columns(&ctx);
        self.finish_editor_window_action(&ctx);
        self.finish_editor_window_reveal(&ctx);
        self.finish_script_focus(&ctx);
        if self.script_conflict.is_some()
            || self.pending_close.is_some()
            || self.script_reload.is_some()
        {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        if !ctx.will_discard()
            && let Some((generation, document, interaction)) = self.editor_window.focus
        {
            if !self.editor_window.detached
                || generation != self.editor_window.generation
                || document != self.documents[self.active].id
                || interaction != self.document_save.interaction_generation()
                || self.file_action_pending()
                || self.pending_quit
                || self.allow_close
            {
                self.editor_window.focus = None;
            } else if self.editor_window_available(&ctx) {
                self.editor_window.focus = None;
                ctx.memory_mut(|memory| memory.request_focus(editor::id(document)));
            }
        }
        self.navigation_help_ui(&ctx);
        self.finish_focus_areas(&ctx);
        if self.busy || self.script_io.pending() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
impl State {
    pub(super) fn last_dispatch(&self) -> Option<(u64, &'static str)> {
        self.dispatched.last().copied()
    }

    pub(super) fn dispatch_count(&self, action: &str) -> usize {
        self.dispatched
            .iter()
            .filter(|(_, name)| *name == action)
            .count()
    }
}
