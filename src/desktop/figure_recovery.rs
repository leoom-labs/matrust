use super::*;
use matrust::session::figures::{FigureDocument, FigureSession, FigureView};

/// Metadata only. This map outlives native child retirement and never owns samples.
#[derive(Clone)]
pub struct Placement {
    pub identity: matrust::ObjectIdentity,
    pub window: matrust::session::figure_window::FigureWindow,
}

impl Workspace {
    pub(super) fn start_session_recovery(&mut self, path: PathBuf) {
        if self.recovery_manifest.is_none() && self.workspace_manifest.is_none() {
            return;
        }
        self.recovering_session = true;
        self.submit(Job::RestoreSession(
            path,
            self.recovery_manifest.clone(),
            self.workspace_manifest.clone(),
        ));
        if !self.busy {
            self.recovering_session = false;
            self.recovery_blocked = true;
        }
    }
    pub(super) fn figure_checkpoint(&self) -> FigureSession {
        FigureSession {
            current: self.current_figure,
            figures: self
                .all_figures
                .iter()
                .map(|(&id, figure)| {
                    let status = self.figure_documents.get(&id);
                    FigureDocument {
                        window: self
                            .figure_windows
                            .get(&id)
                            .filter(|p| p.identity.same_object(&figure.identity))
                            .map(|p| p.window.clone()),
                        id,
                        figure: figure.clone(),
                        path: status.and_then(|s| s.path.clone()),
                        source_version: status.and_then(|s| s.source_version),
                        dirty: status.is_none_or(|s| s.dirty),
                        view: self.figure_views.get(&id).map(|v| FigureView {
                            bounds: v.bounds,
                            auto: v.auto,
                            dirty: v.dirty,
                        }),
                    }
                })
                .collect(),
        }
    }
    pub(super) fn recovered_figures(&mut self, recovered: worker::RecoveredFigures) {
        self.recovering_session = false;
        self.recovery_dirty = true;
        // The worker binds only after successful figure admission. Recheck the
        // published snapshots here; a number/title/path is never sufficient.
        self.figure_windows = recovered.windows;
        self.figure_windows.retain(|id, p| {
            self.all_figures
                .get(id)
                .is_some_and(|f| p.identity.same_object(&f.identity))
        });
        for note in recovered.placement_notes {
            self.console.push(format!("Session layout: {note}"));
        }
        for (id, view) in recovered.views {
            if let Some(status) = self.figure_documents.get(&id) {
                self.figure_views.insert(
                    id,
                    figure_files::View {
                        bounds: view.bounds,
                        auto: view.auto,
                        dirty: view.dirty,
                        revision: status.revision,
                    },
                );
            }
        }
        if !recovered.warnings.is_empty() {
            self.recovery_blocked = true;
            for warning in recovered.warnings {
                self.console.push(format!("Session recovery: {warning}"));
            }
            self.console.push("Automatic recovery is disabled to preserve the incomplete checkpoint. Save recovered variables, figures and other current work explicitly.".into());
        }
    }
    pub(super) fn exit_snapshot(&self) -> Session {
        let mut session = self.snapshot();
        if let Some(figures) = &mut session.figures {
            figures
                .figures
                .retain(|f| !self.quit_discarded_figures.contains(&f.id));
            if !figures.figures.iter().any(|f| f.id == figures.current) {
                figures.current = figures.figures.first().map_or(1, |f| f.id);
            }
        }
        session
    }
}
