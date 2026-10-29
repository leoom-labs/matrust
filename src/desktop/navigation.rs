//! Recoverable panel intent with session-local focus/frame admission state.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Left {
    Files,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct State {
    pub left: Option<Left>,
    pub workspace: bool,
    pub command: bool,
    focus_request: Option<(u64, FocusPane, u64)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            left: Some(Left::Files),
            workspace: true,
            command: true,
            focus_request: None,
        }
    }
}
impl State {
    /// Files, Workspace and Command Window are always-visible cards; a layout
    /// saved with a hidden panel (rails were removed) shows them again.
    pub(super) fn from_panels(_panels: Option<session::PanelLayout>) -> Self {
        Self::default()
    }
    pub(super) fn panels(&self) -> Option<session::PanelLayout> {
        let panels = session::PanelLayout {
            sidebar: match self.left {
                Some(Left::Files) => session::SidebarPanel::Files,
                None => session::SidebarPanel::Hidden,
            },
            workspace: self.workspace,
            command: self.command,
        };
        (panels != session::PanelLayout::default()).then_some(panels)
    }
    pub(super) fn command_focus_requested(&self, frame: u64) -> bool {
        self.focus_request
            .is_some_and(|(queued, pane, _)| queued == frame && pane == FocusPane::Command)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FocusPane {
    Command,
    Editor,
}

impl Workspace {
    fn can_focus_pane(&self, ctx: &egui::Context, pane: FocusPane) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && !self.file_action_pending()
            && !self.recovering_session
            && !self.allow_close
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && (pane == FocusPane::Editor || !self.busy)
    }

    pub(super) fn queue_pane_focus(&mut self, ctx: &egui::Context, pane: FocusPane) {
        if self.editor_window.in_callback && pane == FocusPane::Editor {
            self.queue_editor_window_focus(ctx);
            return;
        }
        if self.can_focus_pane(ctx, pane) {
            // One transient intent; no delayed queue behind a worker or dialog.
            self.navigation.focus_request = Some((
                ctx.cumulative_frame_nr(),
                pane,
                self.documents[self.active].id,
            ));
        }
    }

    pub(super) fn pane_focus_shortcuts(&mut self, ctx: &egui::Context) {
        if egui::Popup::is_any_open(ctx) {
            return;
        }
        // Check the specific chord first. Ctrl+Alt+0 remains scale reset.
        let pane = ctx.input_mut(|input| {
            if input.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                egui::Key::Num0,
            ) {
                Some(FocusPane::Editor)
            } else if input.consume_key(egui::Modifiers::CTRL, egui::Key::Num0) {
                Some(FocusPane::Command)
            } else {
                None
            }
        });
        if let Some(pane) = pane {
            self.queue_pane_focus(ctx, pane);
        }
    }

    pub(super) fn pane_focus_menu(&mut self, ui: &mut egui::Ui) {
        self.navigation_help_menu(ui, true);
        for (pane, label, shortcut) in [
            (FocusPane::Command, "Focus Command Window", "Ctrl+0"),
            (FocusPane::Editor, "Focus Editor", "Ctrl+Shift+0"),
        ] {
            if ui
                .add_enabled(
                    self.can_focus_pane(ui.ctx(), pane),
                    egui::Button::new(label).shortcut_text(shortcut),
                )
                .clicked()
            {
                self.queue_pane_focus(ui.ctx(), pane);
                ui.close();
            }
        }
    }

    pub(super) fn finish_pane_focus(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some((frame, pane, document)) = self.navigation.focus_request.take() else {
            return;
        };
        if frame != ctx.cumulative_frame_nr()
            || !self.can_focus_pane(ctx, pane)
            || egui::Popup::is_any_open(ctx)
            || self.documents[self.active].id != document
        {
            return;
        }
        let id = match pane {
            FocusPane::Command => {
                self.editor_window.clear_focus();
                if self.layout.center.is_some() {
                    self.show_command_center();
                }
                self.navigation.command = true;
                egui::Id::new("command_input")
            }
            FocusPane::Editor => {
                if self.editor_window.detached {
                    self.queue_editor_window_focus(ctx);
                    ctx.send_viewport_cmd_to(
                        editor_window::viewport(),
                        egui::ViewportCommand::Focus,
                    );
                    return;
                }
                self.reveal_editor();
                editor::id(document)
            }
        };
        // egui retains newly requested focus until the next frame even when the
        // just-revealed field was absent this pass. Preserve its cursor/selection.
        ctx.memory_mut(|memory| memory.request_focus(id));
        ctx.request_repaint();
    }
}
