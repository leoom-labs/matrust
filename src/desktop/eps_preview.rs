use super::*;
#[derive(Clone, Copy)]
pub(super) enum Action {
    Choose,
    Render,
    Cancel,
    Reopen,
    Forget,
}
#[derive(Default)]
pub(super) struct Settings {
    pub renderer: Option<PathBuf>,
    pub pending: Option<PathBuf>,
    pub loading: Option<PathBuf>,
    pub error: Option<String>,
    action: Option<(u64, PathBuf, Action)>,
    configure: Option<(u64, PathBuf)>,
}
pub(super) fn is_eps(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("eps"))
}
impl Settings {
    pub fn configured(isolated: bool) -> Self {
        let renderer = (!isolated)
            .then(|| std::env::var_os("MATRUST_EPS_RENDERER"))
            .flatten()
            .map(PathBuf::from)
            .filter(|p| {
                p.is_absolute()
                    && p.as_os_str().len() <= 32768
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            });
        Self {
            renderer,
            ..Default::default()
        }
    }
}
impl Workspace {
    pub(super) fn begin_eps_preview(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        self.eps_preview.error = None;
        if let Some(renderer) = self.eps_preview.renderer.clone() {
            let restore = self.preview_restore.take().filter(|r| r.state.path == path);
            self.submit(Job::OpenEps(path.clone(), renderer));
            if self.busy {
                self.eps_preview.loading = Some(path);
                self.preview_restore = restore.map(|mut r| {
                    r.started = true;
                    r.error = None;
                    r
                });
            }
        } else {
            self.eps_preview.pending = Some(path);
            self.status = "Choose Ghostscript to preview EPS files".into();
        }
    }
    pub(super) fn eps_preview_ui(&mut self, ctx: &egui::Context) {
        let Some(path) = self.eps_preview.pending.clone() else {
            return;
        };
        let enabled = !self.busy && !self.file_action_pending_without_eps();
        let mut open = true;
        let mut choose = false;
        let mut render = false;
        let mut close = false;
        egui::Window::new("Configure EPS preview").open(&mut open).collapsible(false)
            .default_width(440.).show(ctx, |ui| {
                let filename = path.file_name().map(std::path::Path::new).unwrap_or(path.as_path());
                ui.add(egui::Label::new(format!("File: {}", crate::path_label::display(filename))).truncate())
                    .on_hover_ui(|ui| { ui.label(crate::path_label::display(&path).to_string()); });
                ui.label("Select your installed Ghostscript console executable. EPS previews are read-only fitted images.");
                if let Some(renderer) = &self.eps_preview.renderer {
                    let filename = renderer.file_name().map(std::path::Path::new).unwrap_or(renderer.as_path());
                    ui.add(egui::Label::new(format!("Renderer: {}", crate::path_label::display(filename))).truncate())
                        .on_hover_ui(|ui| { ui.label(crate::path_label::display(renderer).to_string()); });
                } else { ui.label("No renderer selected."); }
                ui.weak("Your renderer choice is saved with workspace recovery. EPS files are rendered only when you open or reopen them.");
                if let Some(error) = &self.eps_preview.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                ui.horizontal(|ui| {
                    choose = ui.add_enabled(enabled, egui::Button::new("Choose Ghostscript...")).clicked();
                    render = ui.add_enabled(enabled && self.eps_preview.renderer.is_some(), egui::Button::new("Open preview")).clicked();
                    close = ui.button("Cancel").clicked();
                });
            });
        let requested = if !open || close {
            Some(Action::Cancel)
        } else if choose {
            Some(Action::Choose)
        } else if render {
            Some(Action::Render)
        } else {
            None
        };
        let Some(action) = self.eps_preview.action(ctx, &path, requested) else {
            return;
        };
        self.apply_eps_action(path, action, enabled);
    }
}

impl Workspace {
    pub(super) fn eps_recovery_ui(&mut self, ctx: &egui::Context, blocked: bool) {
        if self.eps_preview.pending.is_some() {
            return;
        }
        let Some(restore) = &self.preview_restore else {
            return;
        };
        if restore.started && restore.error.is_none() {
            return;
        }
        let path = restore.state.path.clone();
        let mut reopen = false;
        let mut forget = false;
        egui::Window::new("Reopen EPS preview").collapsible(false).default_width(420.)
            .show(ctx, |ui| {
                let filename = path.file_name().map(std::path::Path::new).unwrap_or(path.as_path());
                ui.add(egui::Label::new(format!("File: {}", crate::path_label::display(filename))).truncate())
                    .on_hover_ui(|ui| { ui.label(crate::path_label::display(&path).to_string()); });
                ui.label("Your EPS preview reference was recovered. Open it to render the current file with your configured Ghostscript.");
                if let Some(error) = &restore.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                ui.horizontal(|ui| {
                    reopen = ui.add_enabled(!blocked && !self.busy, egui::Button::new("Reopen EPS")).clicked();
                    forget = ui.add_enabled(!blocked && !self.busy, egui::Button::new("Forget preview")).clicked();
                });
            });
        let requested = if forget {
            Some(Action::Forget)
        } else if reopen {
            Some(Action::Reopen)
        } else {
            None
        };
        let Some(action) = self.eps_preview.action(ctx, &path, requested) else {
            return;
        };
        self.apply_eps_recovery_action(path, action, !blocked && !self.busy);
    }
}

impl Settings {
    fn action(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
        requested: Option<Action>,
    ) -> Option<Action> {
        let frame = ctx.cumulative_frame_nr();
        if self
            .action
            .as_ref()
            .is_some_and(|(old, saved, _)| *old != frame || saved != path)
        {
            self.action = None;
        }
        if let Some(action) = requested {
            self.action = Some((frame, path.to_path_buf(), action));
        }
        if ctx.will_discard() {
            return None;
        }
        self.action.take().map(|(_, _, action)| action)
    }
}

#[cfg(test)]
#[path = "eps_preview_tests.rs"]
mod tests;

impl Settings {
    pub(super) fn configure_from_view(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
        clicked: bool,
        enabled: bool,
    ) {
        let frame = ctx.cumulative_frame_nr();
        if self
            .configure
            .as_ref()
            .is_some_and(|(old, saved)| *old != frame || saved != path)
        {
            self.configure = None;
        }
        if clicked {
            self.configure = Some((frame, path.to_path_buf()));
        }
        if ctx.will_discard() {
            return;
        }
        if let Some((_, path)) = self.configure.take()
            && enabled
        {
            self.pending = Some(path);
            self.error = None;
        }
    }
}

impl Workspace {
    fn apply_eps_recovery_action(&mut self, path: PathBuf, action: Action, enabled: bool) {
        if !enabled {
            return;
        }
        if matches!(action, Action::Forget) {
            self.preview_restore = None;
            self.recovery_dirty = true;
        } else if matches!(action, Action::Reopen) {
            self.begin_eps_preview(path);
        }
    }
    pub(super) fn select_eps_renderer(&mut self, renderer: PathBuf) {
        self.eps_preview.renderer = Some(renderer);
        self.recovery_dirty = true;
        self.eps_preview.error = None;
    }
    pub(super) fn apply_eps_action(&mut self, path: PathBuf, action: Action, enabled: bool) {
        if matches!(action, Action::Cancel) {
            self.eps_preview.pending = None;
            return;
        }
        if !enabled {
            return;
        }
        if matches!(action, Action::Choose)
            && let Some(renderer) = rfd::FileDialog::new()
                .add_filter("Ghostscript console", &["exe"])
                .pick_file()
        {
            self.select_eps_renderer(renderer);
        }
        if matches!(action, Action::Render) {
            self.eps_preview.pending = None;
            self.begin_eps_preview(path);
        }
    }
    pub(super) fn capture_eps_action(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
        action: Action,
    ) {
        if let Some(action) = self.eps_preview.action(ctx, path, Some(action)) {
            let enabled = !self.busy && !self.file_action_pending_without_eps();
            match action {
                Action::Reopen | Action::Forget => {
                    self.apply_eps_recovery_action(path.to_path_buf(), action, enabled)
                }
                _ => self.apply_eps_action(path.to_path_buf(), action, enabled),
            }
        }
    }
}
