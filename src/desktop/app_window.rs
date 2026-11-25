//! Window layout (owner 2026-11-09): by default the Editor, every figure and
//! the apps open as their own native windows; the main window keeps the
//! Command Window, Workspace and Files. Settings › Window layout › Docked
//! brings back the single-window desktop.
use super::*;
use matrust::session::{WindowLayout, WindowRect};
use std::collections::BTreeSet;

pub(super) fn apps_viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("matrust_apps_window")
}

#[derive(Default)]
pub(super) struct State {
    /// Figures the separate layout already popped out; one docked back by
    /// the user stays docked until it closes.
    popped: BTreeSet<u32>,
    /// "Dock back" on the Editor window keeps it docked for this session.
    pub(super) editor_docked_by_user: bool,
    /// Placement passed to the window builder while a window is open. It is
    /// fixed at open time: a builder that followed the observed placement
    /// would fight the user's own moves.
    editor_open: Option<Option<WindowRect>>,
    apps_open: Option<Option<WindowRect>>,
}

/// Outer position and inner size of the current viewport, when known and normal.
fn observed(ctx: &egui::Context) -> Option<WindowRect> {
    ctx.input(|i| {
        let info = i.viewport();
        if info.minimized == Some(true) || info.maximized == Some(true) {
            return None;
        }
        let outer = info.outer_rect?;
        let inner = info.inner_rect?;
        let rect = WindowRect {
            x: outer.min.x,
            y: outer.min.y,
            width: inner.width(),
            height: inner.height(),
        };
        rect.valid().then_some(rect)
    })
}

pub(super) fn builder(
    title: &str,
    placement: Option<WindowRect>,
    default: [f32; 2],
    min: [f32; 2],
) -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_min_inner_size(min);
    match placement {
        Some(r) => builder
            .with_position([r.x, r.y])
            .with_inner_size([r.width, r.height]),
        None => builder.with_inner_size(default),
    }
}

impl Workspace {
    pub(super) fn separate_windows(&self) -> bool {
        match self.layout.window_layout {
            Some(WindowLayout::Docked) => false,
            Some(WindowLayout::Separate) => true,
            // The product default is Separate. Unit tests that build a bare
            // Workspace predate it and assert the docked panes.
            None => !cfg!(test),
        }
    }

    /// The app shown in the main window's work area (none when apps have their own window).
    pub(super) fn docked_tool(&self) -> Option<toolkits::Tool> {
        if self.separate_windows() {
            None
        } else {
            self.toolkits.selected_tool()
        }
    }

    pub(super) fn set_window_layout(&mut self, ctx: &egui::Context, layout: WindowLayout) {
        if self.separate_windows() == (layout == WindowLayout::Separate)
            && self.layout.window_layout.is_some()
        {
            return;
        }
        self.layout.window_layout = Some(layout);
        self.recovery_dirty = true;
        self.windows.editor_docked_by_user = false;
        self.windows.popped.clear();
        if layout == WindowLayout::Docked {
            if self.editor_window.detached {
                self.dock_editor_window(ctx);
            }
            let ids: Vec<u32> = self.detached.ids();
            for id in ids {
                self.dock_figure(id, ctx);
            }
            if self.toolkits.selected_tool().is_some() {
                self.apps_home = false;
            }
        }
        ctx.request_repaint();
    }

    /// Start of a root frame: what the separate layout keeps in its own
    /// window leaves the main window.
    pub(super) fn apply_window_layout(&mut self, ctx: &egui::Context) {
        let live: BTreeSet<u32> = self.all_figures.keys().copied().collect();
        self.windows.popped.retain(|id| live.contains(id));
        if !self.separate_windows() || ctx.will_discard() || self.allow_close {
            return;
        }
        if !self.editor_window.detached
            && !self.windows.editor_docked_by_user
            && !self.has_startup_placeholder()
            && !self.busy
            && self.mode() == modes::Mode::Editor
        {
            self.editor_window.cancel();
            self.editor_window.detached = true;
            self.reset_editor_host();
            self.reset_script_focus_host();
            // The main window returns to the Command Window.
            self.layout
                .center
                .get_or_insert(session::CenterLayout {
                    command: true,
                    editor_revealed: true,
                })
                .command = true;
            self.navigation.command = true;
            self.recovery_dirty = true;
            self.queue_editor_window_focus(ctx);
            ctx.send_viewport_cmd_to(editor_window::viewport(), egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
        if self.busy || self.file_action_pending() {
            return;
        }
        for id in live {
            if self.windows.popped.contains(&id) {
                continue;
            }
            self.undock_figure(id, ctx);
            if self.detached.contains(id) {
                self.windows.popped.insert(id);
            }
        }
    }

    /// Placement for the Editor window builder; remembers the window's moves.
    pub(super) fn editor_window_placement(&mut self) -> Option<WindowRect> {
        *self
            .windows
            .editor_open
            .get_or_insert(self.layout.editor_window)
    }
    pub(super) fn editor_window_closed(&mut self) {
        self.windows.editor_open = None;
    }
    pub(super) fn observe_editor_window(&mut self, ctx: &egui::Context) {
        if let Some(rect) = observed(ctx)
            && self.layout.editor_window != Some(rect)
        {
            self.layout.editor_window = Some(rect);
            self.recovery_dirty = true;
        }
    }

    /// Root-pass file commands (open, model I/O, design scripts) accept the
    /// focused main window or, in Separate mode, the focused Apps window that
    /// hosts the buttons.
    pub(super) fn command_window_focused(&self, ctx: &egui::Context) -> bool {
        let usable = |i: &egui::InputState| i.focused && !i.viewport().close_requested();
        ctx.input(usable)
            || (self.separate_windows()
                && self.toolkits.selected_tool().is_some()
                && ctx.input_for(apps_viewport(), usable))
    }

    /// The open apps in one native window: tabs, commands, the app, its status.
    pub(super) fn apps_window_frame(&mut self, ctx: &egui::Context, pass: &mut toolkits::Pass) {
        let tool = self.toolkits.selected_tool();
        let (true, Some(mut tool), false) = (self.separate_windows(), tool, self.allow_close)
        else {
            self.windows.apps_open = None;
            return;
        };
        let placement = *self
            .windows
            .apps_open
            .get_or_insert(self.layout.apps_window);
        let title = format!("{} — MatRust", self.toolkits.tab_label(tool));
        let builder = builder(&title, placement, [1200., 800.], [640., 420.]);
        let busy = self.busy;
        let mut closed = false;
        ctx.show_viewport_immediate(apps_viewport(), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            theme::window_root(ui);
            if ctx.input(|i| i.viewport().close_requested()) {
                closed = true;
                return;
            }
            if let Some(rect) = observed(&ctx)
                && self.layout.apps_window != Some(rect)
            {
                self.layout.apps_window = Some(rect);
                self.recovery_dirty = true;
            }
            let p = tokens::palette(&ctx);
            egui::Panel::top("apps_window_tabs")
                .exact_size(panels::header_height(&ctx))
                .frame(
                    egui::Frame::new()
                        .fill(p.canvas)
                        .inner_margin(egui::Margin::symmetric(8, 4)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let tabs: Vec<toolkits::Tool> = self.toolkits.tabs().collect();
                        for t in tabs {
                            let label = self.toolkits.tab_label(t);
                            if tokens::toggle(ui, t == tool, &label).clicked() {
                                self.select_tool_tab(t);
                                tool = t;
                            }
                            if ui
                                .button(format!("Close {label}"))
                                .on_hover_text(format!("Close {label}"))
                                .clicked()
                            {
                                self.close_tool_tab(t);
                            }
                            ui.add_space(tokens::S);
                        }
                    });
                });
            let Some(tool) = self.toolkits.selected_tool() else {
                return;
            };
            if let Some((text, status)) = self.toolkits.app_status(tool, busy) {
                egui::Panel::bottom("apps_window_status")
                    .exact_size(24.)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(text).color(status.color(ui.ctx())));
                    });
            }
            if tool != toolkits::Tool::App(apps::AppKind::AppDesigner) {
                egui::Panel::top("apps_window_commands")
                    .exact_size(40.)
                    .frame(
                        egui::Frame::new()
                            .fill(p.canvas)
                            .inner_margin(egui::Margin::symmetric(8, 2)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| self.toolkits.app_commands(ui, tool, busy));
                    });
            }
            egui::CentralPanel::default()
                .frame(tokens::card(&ctx))
                .show(ui, |ui| self.toolkits.tab_ui(ui, tool, busy, pass));
        });
        if closed {
            // The window closes; the apps keep their state for the next open.
            self.toolkits.deselect_tool();
            self.windows.apps_open = None;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    }
}

#[cfg(test)]
#[path = "app_window_tests.rs"]
mod tests;
