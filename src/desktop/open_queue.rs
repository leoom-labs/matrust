//! Bounded, ordered file opening across worker jobs and user decisions.
use super::*;

pub(super) const MAX_QUEUED_FILES: usize = 32;

impl Workspace {
    pub(super) fn file_action_pending(&self) -> bool {
        self.eps_preview.pending.is_some() || self.file_action_pending_without_eps()
    }
    pub(super) fn file_action_pending_without_eps(&self) -> bool {
        self.named_layout.pending()
            || self.global_search.is_open()
            || self.file_action_pending_common()
    }
    pub(super) fn file_action_pending_without_named_layout(&self) -> bool {
        self.eps_preview.pending.is_some()
            || self.global_search.is_open()
            || self.file_action_pending_common()
    }
    pub(super) fn file_action_pending_without_global_search(&self) -> bool {
        self.eps_preview.pending.is_some()
            || self.named_layout.pending()
            || self.file_action_pending_common()
    }
    pub(super) fn file_action_pending_without_ordinary_script_conflict(&self) -> bool {
        self.eps_preview.pending.is_some()
            || self.named_layout.pending()
            || self.global_search.is_open()
            || self.file_action_pending_common_excluding_conflict(true, false)
    }
    pub(super) fn file_action_pending_without_csv_naming(&self) -> bool {
        self.eps_preview.pending.is_some()
            || self.named_layout.pending()
            || self.global_search.is_open()
            || self.file_action_pending_common_excluding_conflict(false, true)
    }
    fn file_action_pending_common(&self) -> bool {
        self.file_action_pending_common_excluding_conflict(false, false)
    }
    fn file_action_pending_common_excluding_conflict(
        &self,
        own_conflict: bool,
        own_csv: bool,
    ) -> bool {
        self.focus_areas.is_open()
            || self.chrome.about
            || self.original_copy.pending()
            || self.single_export.pending()
            || self.detached.has_transition()
            || self.workspace_tools.dialog.is_some()
            || self.script_io.pending()
            || self.worker.opening
            || (!own_conflict && self.pending_quit)
            || (!own_conflict && self.script_save_queue.is_some())
            || self.toolkits.modal_open()
            || self.figure_conflict.is_some()
            || (!own_conflict && self.script_conflict.is_some())
            || self.script_reload.is_some()
            || self.new_model_requested
            || (!own_conflict && self.pending_close.is_some())
            || self.figure_editor.is_some()
            || self.figure_export.is_some()
            || self.plot_data.is_some()
            || self.workspace_import.is_some()
            || (!own_csv && self.csv_import.is_some())
            || self.figure_close.is_some()
    }

    pub(super) fn queue_open_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let paths: Vec<_> = paths.into_iter().take(MAX_QUEUED_FILES + 1).collect();
        if paths.is_empty() {
            return;
        }
        if paths.len() + self.open_queue.len() > MAX_QUEUED_FILES {
            self.status =
                "Open at most 32 queued files at a time; this selection was not added".into();
            self.console.push(self.status.clone());
            return;
        }
        // Resolve relative paths once. An earlier queued directory must not
        // redirect a later file, and no file data are loaded into the queue.
        let paths: std::io::Result<Vec<_>> = paths.iter().map(std::path::absolute).collect();
        match paths {
            Ok(paths) => self.open_queue.extend(paths),
            Err(error) => {
                self.status = format!("Could not queue files: {error}");
                self.console.push(self.status.clone());
            }
        }
    }

    pub(super) fn advance_open_queue(&mut self, ctx: &egui::Context) {
        if self.busy
            || self.folder_browser.navigating()
            || self.file_action_pending()
            || self.allow_close
            || ctx.input(|i| i.viewport().close_requested())
        {
            return;
        }
        // One dispatch per frame keeps scripts/directories responsive too.
        if let Some(path) = self.open_queue.pop_front() {
            if !self.try_open_path(path.clone()) {
                self.console.push(format!(
                    "Open {}: {}",
                    crate::path_label::display(&path),
                    self.status
                ));
            }
            ctx.request_repaint();
        }
    }

    pub(super) fn cancel_queued_files(&mut self) {
        let queued = self.open_queue.len();
        self.open_queue.clear();
        self.status = format!("Cancelled {queued} queued files");
    }

    pub(super) fn stop_work(&mut self) {
        self.cancel_workspace_csv();
        self.cancel_workspace_mat_save();
        self.cancel_root_open();
        self.cancel_root_figure_save();
        self.cancel_designer_save();
        self.cancel_model_toolbar();
        self.cancel_folder_browse();
        self.cancel_csv_import_picker();
        self.cancel_csv_naming_action();
        self.cancel_ordinary_script_conflict();
        if let Some(dialog) = &mut self.figure_export {
            dialog.cancel_action();
        }
        self.focus_areas.cancel();
        self.editor_window.cancel();
        self.cancel_script_tab_order();
        self.global_search.cancel();
        self.named_layout.cancel();
        self.cancel_folder_search();
        self.folder_browser.cancel_action();
        self.cancel_original_copy();
        self.cancel_single_export();
        self.cancel_preview_save();
        self.detached.cancel_pending();
        self.cancel_script_focus();
        self.script_reload = None;
        self.new_model_requested = false;
        self.toolkits.cancel_new_model();
        if self.script_io.pending() || self.script_save_queue.is_some() || self.script_io.resume {
            self.script_io.queued = None;
            self.script_io.deferred_reload = None;
            self.script_io.resume = false;
            self.script_save_queue = None;
            self.pending_quit = false;
            self.pending_close = None;
            self.quit_discarded_figures.clear();
        }
        if self.toolkits.model_io_pending() {
            self.toolkits.cancel_model_io();
            self.document_save.cancel_model_activation();
            self.pending_quit = false;
            self.pending_close = None;
            self.quit_discarded_figures.clear();
        }
        let navigating = self.folder_browser.navigating();
        if navigating {
            self.folder_browser.stop();
        }
        let queued = self.open_queue.len();
        self.open_queue.clear();
        self.worker.cancel.store(true, Ordering::Relaxed);
        self.status = if self.busy {
            format!("Stopping after current operation; cancelled {queued} queued files")
        } else if navigating {
            format!("Cancelled folder change and {queued} queued files")
        } else {
            format!("Cancelled {queued} queued files")
        };
    }
}
