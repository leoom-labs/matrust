use super::*;
use crate::theme::ThemedScrollArea;
use std::borrow::Cow;

// Explicit root Figure Save is independent of the generic foreground document.
#[cfg(test)]
type RootPicker = fn(&mut Workspace, &egui::Context) -> Option<PathBuf>;

#[derive(Default)]
pub(super) struct RootSave {
    action: Option<RootSaveAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
    #[cfg(test)]
    pub(super) picker: Option<RootPicker>,
    #[cfg(test)]
    pub(super) picker_calls: Vec<(u32, bool, bool)>,
}
struct RootSaveAction {
    target: figure_target::Key,
    revision: u64,
    path: Option<PathBuf>,
    view: Option<[f64; 4]>,
    auto: [bool; 2],
    theme: matrust::plot::BaseTheme,
    save_as: bool,
    stamp: (egui::ViewportId, u64),
    generation: u64,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    folder: PathBuf,
}
impl Workspace {
    pub(super) fn root_figure_save_context(&self, ctx: &egui::Context) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|i| i.focused && !i.viewport().close_requested())
            && ctx.memory(|m| m.top_modal_layer().is_none())
            && !self.busy
            && !self.file_action_pending()
            && self.open_queue.is_empty()
            && !self.folder_browser.navigating()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
    }
    fn valid_root_figure_save(&self, ctx: &egui::Context, action: &RootSaveAction) -> bool {
        self.root_figure_save_context(ctx)
            && !egui::Popup::is_any_open(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.root_figure_save.generation
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|m| m.focused())
            && action.folder == self.folder
            && action.target.number == self.current_figure
            && self
                .all_figures
                .get(&action.target.number)
                .is_some_and(|f| action.target.identity.same_object(&f.identity))
            && self
                .figure
                .as_ref()
                .is_some_and(|f| action.target.identity.same_object(&f.identity))
            && self
                .figure_documents
                .get(&action.target.number)
                .is_some_and(|d| d.revision == action.revision && d.path == action.path)
            && action.view.map(|v| v.map(f64::to_bits))
                == self.view_bounds.map(|v| v.map(f64::to_bits))
            && action.auto == self.view_auto
            && action.theme == self.worker.theme()
    }
    pub(super) fn queue_root_figure_save(&mut self, ctx: &egui::Context, save_as: bool) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        // A clicked menu is allowed to unwind; final admission requires it closed.
        if !self.root_figure_save_context(ctx)
            || self.root_figure_save.action.is_some()
            || self.root_figure_save.finished == Some(stamp)
        {
            return;
        }
        let Some(figure) = self.all_figures.get(&self.current_figure) else {
            return;
        };
        let Some(document) = self.figure_documents.get(&self.current_figure) else {
            return;
        };
        self.root_figure_save.action = Some(RootSaveAction {
            target: figure_target::Key {
                number: self.current_figure,
                identity: figure.identity.clone(),
            },
            revision: document.revision,
            path: document.path.clone(),
            view: self.view_bounds,
            auto: self.view_auto,
            theme: self.worker.theme(),
            save_as,
            stamp,
            generation: self.root_figure_save.generation,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|m| m.focused()),
            folder: self.folder.clone(),
        });
    }
    pub(super) fn cancel_root_figure_save(&mut self) {
        self.root_figure_save.generation = self.root_figure_save.generation.wrapping_add(1);
        if let Some(action) = self.root_figure_save.action.take() {
            self.root_figure_save.finished = Some(action.stamp);
        }
    }
    pub(super) fn observe_root_figure_save(&mut self, ctx: &egui::Context) {
        // Retire foreign ownership before repainting can surrender that focus.
        if self
            .root_figure_save
            .action
            .as_ref()
            .is_some_and(|action| !self.valid_root_figure_save(ctx, action))
        {
            self.cancel_root_figure_save();
        }
    }
    pub(super) fn finish_root_figure_save(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.root_figure_save.action.take() else {
            return;
        };
        self.root_figure_save.finished = Some(action.stamp);
        if !self.valid_root_figure_save(ctx, &action) {
            return;
        }
        if let Some(path) = self.figure_save_path(action.target.number, action.save_as, Some(ctx))
            && !ctx.will_discard()
            && self.valid_root_figure_save(ctx, &action)
        {
            // Current figure, identity, revision and captured view remain exact.
            self.submit(Job::SaveFigure(path, action.view));
        }
    }
}

pub struct View {
    pub bounds: [f64; 4],
    pub auto: [bool; 2],
    pub dirty: bool,
    pub revision: u64,
}
#[derive(Clone, Copy)]
pub enum Closing {
    Figure(u32),
    Application,
    SaveOnly,
    All,
}
pub struct CloseFlow {
    pub action: Closing,
    pub saving: bool,
}

/// Suggested Save dialog name: new figures default to native .mrfig. A
/// restricted MATLAB .fig switches to .mrfig; an existing .mrfig/.mfig keeps
/// its name.
pub(super) fn figure_save_name(
    id: u32,
    existing: Option<&std::path::Path>,
    restricted: bool,
) -> String {
    let Some(path) = existing else {
        return format!("figure-{id}.mrfig");
    };
    let native = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mrfig") || e.eq_ignore_ascii_case("mfig"));
    let suggested = if restricted && !native {
        path.with_extension("mrfig")
    } else {
        path.to_path_buf()
    };
    suggested
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
impl Workspace {
    pub(super) fn can_close_all_figures(&self) -> bool {
        !self.busy && !self.file_action_pending() && !self.figure_ids.is_empty()
    }
    pub(super) fn request_close_all_figures(&mut self) {
        if !self.can_close_all_figures() {
            return;
        }
        if self.figure_ids.iter().any(|&id| self.figure_dirty(id)) {
            self.figure_close = Some(CloseFlow {
                action: Closing::All,
                saving: false,
            });
        } else {
            self.run("close all;".into());
        }
    }

    pub(super) fn can_reload_figure(&self) -> bool {
        self.can_reload_figure_with_pending(self.file_action_pending())
    }
    pub(super) fn can_reload_figure_with_pending(&self, pending: bool) -> bool {
        !self.busy
            && !pending
            && self
                .figure_documents
                .get(&self.current_figure)
                .is_some_and(|d| d.path.is_some())
    }
    pub(super) fn request_figure_reload(&mut self) {
        if !self.can_reload_figure() {
            return;
        }
        let id = self.current_figure;
        let path = self.figure_documents[&id].path.clone().unwrap();
        self.figure_conflict = Some(worker::FigureConflict {
            reload_requested: true,
            id,
            path,
            view: None,
            observed: matrust::files::figure_version::DiskVersion::Unknown,
        });
        self.status =
            "Reload requested; the current figure is retained until loading succeeds.".into();
    }

    pub(super) fn begin_save_all_figures(&mut self) {
        if self.busy || self.file_action_pending() {
            self.status = "Finish the current operation before saving all figures".into();
            return;
        }
        if !self
            .figure_documents
            .keys()
            .any(|&id| self.figure_dirty(id))
        {
            self.status = "No modified figures to save".into();
            return;
        }
        self.figure_close = Some(CloseFlow {
            action: Closing::SaveOnly,
            saving: true,
        });
    }
    pub(super) fn figure_dirty(&self, id: u32) -> bool {
        self.figure_documents.get(&id).is_some_and(|d| d.dirty)
            || self.figure_views.get(&id).is_some_and(|v| v.dirty)
    }
    pub(super) fn restored_figure_view<'a>(&self, figure: &'a Figure) -> Cow<'a, Figure> {
        if self.restore_plot_view
            && let Some(view) = self.figure_views.get(&self.current_figure)
        {
            let mut f = figure.clone();
            if !view.auto[0] {
                f.xlim = Some([view.bounds[0], view.bounds[1]]);
            }
            if !view.auto[1] {
                f.ylim = Some([view.bounds[2], view.bounds[3]]);
            }
            return Cow::Owned(f);
        }
        Cow::Borrowed(figure)
    }
    pub(super) fn record_figure_view(&mut self, bounds: [f64; 4], auto: [bool; 2]) {
        let Some(doc) = self.figure_documents.get(&self.current_figure) else {
            return;
        };
        let old = self.figure_views.get(&self.current_figure);
        let changed = old.is_some_and(|old| {
            !self.busy
                && !self.restore_plot_view
                && (0..2).any(|axis| {
                    old.auto[axis] != auto[axis]
                        || ((!old.auto[axis] || !auto[axis])
                            && old.bounds[axis * 2..axis * 2 + 2] != bounds[axis * 2..axis * 2 + 2])
                })
        });
        self.recovery_dirty |= changed;
        let dirty = old.is_some_and(|old| {
            old.dirty
                || (!self.busy
                    && !self.restore_plot_view
                    && (0..2).any(|axis| {
                        old.auto[axis] != auto[axis]
                            || ((!old.auto[axis] || !auto[axis])
                                && old.bounds[axis * 2..axis * 2 + 2]
                                    != bounds[axis * 2..axis * 2 + 2])
                    }))
        });
        self.figure_views.insert(
            self.current_figure,
            View {
                bounds,
                auto,
                dirty,
                revision: doc.revision,
            },
        );
    }
    // Shared format/path selection. Existing admitted hosts and continuations
    // pass None; only the explicit root ticket uses the post-picker guard below.
    fn figure_save_path(
        &mut self,
        id: u32,
        save_as: bool,
        _ctx: Option<&egui::Context>,
    ) -> Option<PathBuf> {
        #[cfg(test)]
        let ctx = _ctx;
        let existing = self.figure_documents.get(&id).and_then(|d| d.path.clone());
        let restriction = self.all_figures.get(&id).and_then(|figure| {
            matrust::files::matlab_figure_export_restriction_with_theme(figure, self.worker.theme())
        });
        let needs_mfig = restriction.is_some()
            && existing.as_deref().is_some_and(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("fig"))
            });
        let path = if !save_as && existing.is_some() && !needs_mfig {
            existing
        } else {
            let mut dialog = rfd::FileDialog::new()
                .set_directory(&self.folder)
                .add_filter("Editable MatRust figure", &["mrfig", "mfig"]);
            if restriction.is_none() {
                dialog = dialog.add_filter("MATLAB figure (supported static 2-D subset)", &["fig"]);
            } else {
                dialog = dialog.set_title("Save figure as .mrfig to preserve all figure contents");
            }
            if let Some(parent) = existing.as_deref().and_then(std::path::Path::parent) {
                dialog = dialog.set_directory(parent);
            }
            dialog = dialog.set_file_name(figure_save_name(
                id,
                existing.as_deref(),
                restriction.is_some(),
            ));
            #[cfg(test)]
            {
                if let Some(ctx) = ctx
                    && let Some(picker) = self.root_figure_save.picker
                {
                    assert!(self.root_figure_save.picker_calls.len() < 128);
                    self.root_figure_save
                        .picker_calls
                        .push((id, save_as, restriction.is_none()));
                    picker(self, ctx)
                } else {
                    pick_figure_save(dialog)
                }
            }
            #[cfg(not(test))]
            {
                pick_figure_save(dialog)
            }
        };
        let Some(path) = path else {
            self.status = if restriction.is_some() {
                "Figure save cancelled; unsaved changes retained. Save as .mrfig to preserve this figure's contents."
            } else {
                "Figure save cancelled; unsaved changes retained"
            }.into();
            return None;
        };
        Some(path)
    }
    pub(super) fn save_figure_document(&mut self, id: u32, save_as: bool) -> bool {
        if self.busy {
            return false;
        }
        let Some(path) = self.figure_save_path(id, save_as, None) else {
            return false;
        };
        let view = if id == self.current_figure {
            self.view_bounds
        } else {
            self.figure_views.get(&id).map(|v| v.bounds)
        };
        self.submit(if id == self.current_figure {
            Job::SaveFigure(path, view)
        } else {
            Job::SaveFigureById(id, path, view)
        });
        self.busy
    }
    fn close_existing_figure_document(&mut self, id: u32) {
        if !self.detached.authorizes_close(id, &self.all_figures) {
            self.status = "The original figure document is no longer open; close cancelled.".into();
            return;
        }
        if let Some(figure) = self.all_figures.get(&id) {
            self.submit(Job::CloseExistingFigure(figure_target::Key {
                number: id,
                identity: figure.identity.clone(),
            }));
        }
    }
    pub(super) fn request_figure_close(&mut self, id: u32) {
        if self.busy || self.figure_close.is_some() {
            return;
        }
        if self.figure_dirty(id) {
            self.figure_close = Some(CloseFlow {
                action: Closing::Figure(id),
                saving: false,
            });
        } else {
            self.close_existing_figure_document(id);
        }
    }
    pub(super) fn request_quit(&mut self, ctx: &egui::Context) {
        if self.csv_import.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Finish or cancel CSV import before closing".into();
            return;
        }
        if self
            .figure_close
            .as_ref()
            .is_some_and(|flow| matches!(flow.action, Closing::All))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status =
                "Finish or cancel Close all figures before closing the application".into();
            return;
        }
        if self
            .figure_close
            .as_ref()
            .is_some_and(|flow| matches!(flow.action, Closing::SaveOnly))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Finish or cancel Save all figures before closing".into();
            return;
        }
        if let Some(dialog) = &mut self.figure_export {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            dialog.error = Some("Finish or cancel the export before closing.".into());
            return;
        }
        if self.named_layout.pending()
            || self.original_copy.pending()
            || self.single_export.pending()
            || self.workspace_tools.dialog.is_some()
            || self.script_io.pending()
            || self.script_conflict.is_some()
            || self.script_reload.is_some()
            || self.toolkits.model_file_modal_open()
            || self.figure_conflict.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if !self.pending_quit && self.figure_close.is_none() {
            self.quit_discarded_figures.clear();
        }
        if !self.busy {
            self.workspace_import = None;
        }
        if self.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Wait for the current operation or press Stop before closing.".into();
        } else if self
            .figure_documents
            .keys()
            .any(|&id| self.figure_dirty(id))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.figure_close.is_none() {
                self.figure_close = Some(CloseFlow {
                    action: Closing::Application,
                    saving: false,
                });
            }
        } else if self.documents.iter().any(|d| d.dirty) || self.toolkits.model_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_quit = true;
        }
    }
    fn finish_figure_close(&mut self, ctx: &egui::Context) {
        let Some(flow) = self.figure_close.take() else {
            return;
        };
        match flow.action {
            Closing::SaveOnly => {
                self.status = "Save all figures finished; figures remain open".into()
            }
            Closing::Figure(id) => self.close_existing_figure_document(id),
            Closing::All => self.run("close all;".into()),
            Closing::Application => {
                if self.documents.iter().any(|d| d.dirty) || self.toolkits.model_dirty() {
                    self.pending_quit = true;
                } else {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
    pub(super) fn figure_close_ui(&mut self, ctx: &egui::Context) {
        if self.figure_conflict.is_some() {
            return;
        }
        let Some(flow) = &self.figure_close else {
            return;
        };
        let dirty: Vec<_> = self
            .figure_documents
            .keys()
            .copied()
            .filter(|&id| {
                self.figure_dirty(id)
                    && match flow.action {
                        Closing::Figure(n) => n == id,
                        Closing::Application | Closing::SaveOnly | Closing::All => true,
                    }
            })
            .collect();
        let saving = flow.saving;
        let save_only = matches!(flow.action, Closing::SaveOnly);
        let close_all = matches!(flow.action, Closing::All);
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let viewport = ctx.content_rect();
        let shown = egui::Modal::new(egui::Id::new("unsaved_figures")).show(ctx, |ui| {
            ui.set_width((viewport.width() - 48.).clamp(240., 420.));
            ui.label(crate::tokens::title(ui.ctx(), if save_only {
                "Saving figures"
            } else if close_all {
                "Close all figures"
            } else {
                "Unsaved figures"
            }));
            ui.label(if save_only {
                "Save modified figures and keep them open. Completed saves stay saved."
            } else if close_all {
                "Save modified figures before closing all figures? Scripts and workspace variables stay open."
            } else {
                "Save figure changes before closing?"
            });
            egui::ScrollArea::vertical()
                .id_salt("unsaved_figure_list")
                .min_scrolled_height(0.)
                .max_height((viewport.height() - if close_all {275.} else {235.}).clamp(20., 240.))
                .show_themed(ui, |ui| {
                    for id in &dirty {
                        let name = self.figure_documents[id]
                            .path
                            .as_ref()
                            .map_or_else(|| "Untitled".into(), |p| crate::path_label::display(p).to_string());
                        ui.add(egui::Label::new(format!("Figure {id}: {name}")).truncate())
                            .on_hover_text(name);
                    }
                });
            if save_only {
                cancel = ui.button("Cancel remaining figure saves").clicked();
            } else {
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        save = ui
                            .button(if dirty.len() > 1 { "Save all" } else { "Save" })
                            .clicked();
                        discard = ui.button("Discard changes").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            }
            if close_all && self.busy && ui.button("Cancel closing figures").clicked() { cancel = true; }
            egui::ScrollArea::vertical()
                .id_salt("unsaved_figure_status")
                .min_scrolled_height(0.)
                .max_height(48.)
                .show_themed(ui, |ui| {
                    ui.label(&self.status);
                });
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && (save_only || close_all || !self.busy)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            cancel = true;
        }
        if cancel {
            if save_only || close_all {
                if self.busy {
                    self.worker.cancel.store(true, Ordering::Relaxed);
                }
                self.status = if close_all {
                    "Closing figures cancelled; figures remain open and completed saves stay saved"
                } else {
                    "Figure saves cancelled; unsaved figures remain open"
                }
                .into();
            }
            self.quit_discarded_figures.clear();
            self.figure_close = None;
            return;
        }
        if discard {
            if self
                .figure_close
                .as_ref()
                .is_some_and(|f| matches!(f.action, Closing::Application))
            {
                self.quit_discarded_figures.extend(dirty);
            }
            self.finish_figure_close(ctx);
            return;
        }
        if (save || saving) && !self.busy {
            if let Some(&id) = dirty.first() {
                let started = self.save_figure_document(id, false);
                if let Some(flow) = &mut self.figure_close {
                    flow.saving = started;
                }
                if save_only && !started {
                    self.figure_close = None;
                }
            } else {
                self.finish_figure_close(ctx);
            }
        }
    }

    pub(super) fn figure_conflict_ui(&mut self, ctx: &egui::Context) {
        use matrust::files::figure_version::DiskVersion;
        let Some(conflict) = self.figure_conflict.as_ref() else {
            return;
        };
        let mut action = 0;
        let viewport = ctx.content_rect();
        let shown = egui::Modal::new(egui::Id::new("figure_disk_conflict")).show(ctx, |ui| {
            ui.set_width((viewport.width() - 48.).clamp(240., 540.));
            ui.label(crate::tokens::title(ui.ctx(), if conflict.reload_requested {
                format!("Reload figure {} from disk?", conflict.id)
            } else { format!("Figure {} changed on disk", conflict.id) }));
            egui::ScrollArea::vertical().id_salt("figure_conflict_details")
                .min_scrolled_height(0.).max_height((viewport.height() - 180.).clamp(60., 400.))
                .show_themed(ui, |ui| {
            ui.label(crate::path_label::display(&conflict.path).to_string());
            if conflict.reload_requested {
                ui.label("Reload replaces this figure with the current file contents, including any unsaved edits and zoom. Its undo history will be cleared. Other figures and workspace variables stay available.");
            } else { match conflict.observed {
                DiskVersion::Missing => { ui.label("The figure file was deleted or moved. Overwrite will recreate it with your current figure."); }
                DiskVersion::Present(version) => {
                    ui.label(format!("Disk version: {} bytes. Reload replaces your current figure; overwrite replaces the disk file.", version.bytes));
                }
                DiskVersion::Unknown => { ui.label("The saved disk version is unknown. Choose how to keep your work."); }
            }
            }
            ui.label(&self.status);
            });
            ui.separator();
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(conflict.reload_requested || matches!(conflict.observed, DiskVersion::Present(_)),egui::Button::new("Reload figure")).clicked() { action = 1; }
                    if !conflict.reload_requested && ui.button("Overwrite figure").clicked() { action = 2; }
                    if !conflict.reload_requested && ui.button("Save figure as...").clicked() { action = 3; }
                    if ui.button("Cancel").clicked() { action = 4; }
                });
            });
            if self.busy && ui.button("Stop file operation").clicked() {
                self.worker.cancel.store(true, Ordering::Relaxed);
            }
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.busy {
                self.worker.cancel.store(true, Ordering::Relaxed);
            } else {
                action = 4;
            }
        }
        let conflict = self.figure_conflict.as_ref().unwrap().clone();
        match action {
            1 => self.submit(Job::ReloadFigure(conflict.id, conflict.path)),
            2 => self.submit(Job::SaveFigureConfirmed(
                conflict.id,
                conflict.path,
                conflict.view,
                conflict.observed,
            )),
            3 => {
                if !self.save_figure_document(conflict.id, true)
                    && self
                        .figure_close
                        .as_ref()
                        .is_some_and(|flow| matches!(flow.action, Closing::SaveOnly))
                {
                    self.figure_close = None;
                    self.figure_conflict = None;
                }
            }
            4 => {
                self.figure_conflict = None;
                self.figure_close = None;
                self.pending_quit = false;
                self.quit_discarded_figures.clear();
                self.status = if conflict.reload_requested {
                    "Figure reload cancelled; current figure and disk file retained"
                } else {
                    "Figure save cancelled; both versions retained"
                }
                .into();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "figure_file_layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "figure_reload_tests.rs"]
mod reload_tests;

#[cfg(test)]
#[path = "figure_close_all_tests.rs"]
mod close_all_tests;

fn pick_figure_save(dialog: rfd::FileDialog) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(choice) = FIGURE_SAVE_PICKER.with(|picker| picker.borrow_mut().take()) {
        return choice;
    }
    dialog.save_file()
}
#[cfg(test)]
thread_local! {static FIGURE_SAVE_PICKER:std::cell::RefCell<Option<Option<PathBuf>>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(super) struct SavePickerGuard;
#[cfg(test)]
impl Drop for SavePickerGuard {
    fn drop(&mut self) {
        FIGURE_SAVE_PICKER.with(|picker| {
            picker.borrow_mut().take();
        });
    }
}
#[cfg(test)]
pub(super) fn save_picker(choice: Option<PathBuf>) -> SavePickerGuard {
    FIGURE_SAVE_PICKER.with(|picker| {
        assert!(
            picker.borrow_mut().replace(choice).is_none(),
            "Picker already armed"
        )
    });
    SavePickerGuard
}
