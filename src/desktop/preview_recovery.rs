use super::*;
use matrust::session::{PreviewFormat, PreviewState};

pub struct Restore {
    pub state: PreviewState,
    pub started: bool,
    pub error: Option<String>,
}
impl Restore {
    pub fn new(state: PreviewState) -> Self {
        Self {
            state,
            started: false,
            error: None,
        }
    }
}
impl image_view::Viewer {
    pub(super) fn checkpoint(&self) -> PreviewState {
        PreviewState {
            navigation: self.navigation,
            path: self.path.clone(),
            page: self.page.as_ref().map_or(0, |p| p.index),
            fit: self.fit,
            zoom: if self.fit { 1. } else { self.zoom },
            format: match self.export_format {
                image_export::Format::Png => PreviewFormat::Png,
                image_export::Format::Jpeg => PreviewFormat::Jpeg,
                image_export::Format::Bmp => PreviewFormat::Bmp,
                image_export::Format::Tiff => PreviewFormat::Tiff,
            },
        }
    }
    pub(super) fn restore_view(&mut self, state: PreviewState) {
        if state.path != self.path {
            return;
        }
        self.fit = state.fit;
        self.navigation = state.navigation;
        self.navigation.normalize();
        self.restore_window = self.navigation.window.is_some();
        self.zoom = state.zoom;
        self.export_format = match state.format {
            PreviewFormat::Png => image_export::Format::Png,
            PreviewFormat::Jpeg => image_export::Format::Jpeg,
            PreviewFormat::Bmp => image_export::Format::Bmp,
            PreviewFormat::Tiff => image_export::Format::Tiff,
        };
        self.notice =
            Some("Reopened from the last session. The current file contents are shown.".into());
    }
}
impl Workspace {
    pub(super) fn preview_checkpoint(&self) -> Option<PreviewState> {
        self.preview_restore
            .as_ref()
            .map(|r| r.state.clone())
            .or_else(|| self.image_view.as_ref().map(image_view::Viewer::checkpoint))
    }
    pub(super) fn preview_recovery_ui(&mut self, ctx: &egui::Context, blocked: bool) {
        if self
            .preview_restore
            .as_ref()
            .is_some_and(|r| eps_preview::is_eps(&r.state.path))
        {
            self.eps_recovery_ui(ctx, blocked);
            return;
        }
        if !blocked
            && self.figure_export.is_none()
            && !self.busy
            && self
                .preview_restore
                .as_ref()
                .is_some_and(|r| !r.started && r.error.is_none())
        {
            let mut restore = self.preview_restore.take().unwrap();
            let job = if restore.state.paged() {
                Job::OpenPagedImage(restore.state.path.clone(), restore.state.page)
            } else {
                Job::OpenImage(restore.state.path.clone())
            };
            self.submit(job);
            restore.started = self.busy;
            if !self.busy {
                restore.error = Some(self.status.clone());
            }
            self.preview_restore = Some(restore);
        }
        let Some(restore) = &mut self.preview_restore else {
            return;
        };
        let Some(error) = &restore.error else { return };
        let mut open = true;
        let mut retry = false;
        let mut forget = false;
        egui::Window::new("Reopen image preview").open(&mut open).collapsible(false)
            .default_width(420.).show(ctx, |ui| {
                ui.label(crate::path_label::display(&restore.state.path).to_string());
                ui.label("The saved preview could not be reopened. Your other recovered work is available.");
                ui.colored_label(tokens::palette(ui.ctx()).error,error);
                ui.horizontal(|ui| {
                    retry = ui.add_enabled(!self.busy,egui::Button::new("Retry preview")).clicked();
                    forget = ui.button("Forget preview").clicked();
                });
            });
        if retry {
            restore.started = false;
            restore.error = None;
        }
        if !open || forget {
            self.preview_restore = None;
            self.recovery_dirty = true;
        }
    }
}
