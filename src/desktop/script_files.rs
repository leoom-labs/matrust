use super::{Workspace, egui, script_io};
use crate::theme::ThemedScrollArea;
use matrust::document::DiskVersion;
use std::path::{Path, PathBuf};

pub(super) struct Conversion {
    pub(super) document: matrust::document::Document,
    pub(super) confirmed: Option<DiskVersion>,
    pub(super) same_folder: bool,
    pub(super) pending: Option<(u64, u8)>,
}

pub(super) struct Conflict {
    pub(super) action: Option<OrdinaryAction>,
    pub(super) finished: Option<u64>,
    pub(super) document: u64,
    pub(super) path: PathBuf,
    pub(super) observed: DiskVersion,
    pub(super) focus: Option<super::script_focus::Guard>,
    // observed is unused for this explicit choice, not a claimed disk version.
    pub(super) conversion: Option<Conversion>,
}
impl Workspace {
    pub(super) fn preview_script_conflict(&mut self) {
        if self.documents[self.active].path.is_some() {
            self.begin_script_io(self.active, script_io::Kind::Check);
        }
    }
    pub(super) fn save_script_path(
        &mut self,
        index: usize,
        path: &Path,
        confirmed: Option<&DiskVersion>,
    ) -> bool {
        if index >= self.documents.len() || self.script_io.pending() {
            return false;
        }
        let other_paths = self
            .documents
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (i != index).then(|| d.path.clone()).flatten())
            .collect();
        self.begin_script_io(
            index,
            script_io::Kind::Save {
                path: path.to_owned(),
                confirmed: confirmed.cloned(),
                other_paths,
            },
        );
        // Success is reported only after the worker publishes the file.
        false
    }

    pub(super) fn save_pending_scripts(&mut self, ctx: &egui::Context) {
        if self.script_io.pending() {
            return;
        }
        let indexes = if self.pending_quit {
            self.documents
                .iter()
                .enumerate()
                .filter_map(|(i, d)| d.dirty.then_some(i))
                .collect::<Vec<_>>()
        } else {
            self.pending_close
                .into_iter()
                .filter(|&i| self.documents[i].dirty)
                .collect()
        };
        for i in indexes {
            if !self.save_document(i, false) {
                return;
            }
        }
        if self.pending_quit && self.toolkits.model_dirty() && !self.toolkits.save_model() {
            self.status = self.toolkits.message.clone();
            self.advance_model_read();
            return;
        }
        if self.pending_quit {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if let Some(i) = self.pending_close {
            self.remove_document(i);
        }
    }

    pub(super) fn script_conflict_ui(&mut self, ctx: &egui::Context) {
        if self.script_io.pending() {
            return;
        }
        if self
            .script_conflict
            .as_ref()
            .and_then(|c| c.focus.as_ref())
            .is_some_and(|guard| !guard.current_target(self))
        {
            self.script_conflict = None;
            self.script_io.focus.action = None;
        }
        let Some(conflict) = &self.script_conflict else {
            return;
        };
        let Some(index) = self
            .documents
            .iter()
            .position(|d| d.id == conflict.document)
        else {
            self.script_conflict = None;
            return;
        };
        let focus_check = conflict.focus.is_some();
        if conflict.conversion.is_some() {
            self.script_conversion_ui(ctx, index);
            return;
        }
        let mut action = 0;
        let shown = egui::Modal::new(egui::Id::new("script_disk_conflict")).show(ctx, |ui| {
            ui.set_max_width(540.);
            ui.label(crate::tokens::title(ui.ctx(), "Script changed on disk"));
            ui.label(crate::path_label::display(&conflict.path).to_string());
            match &conflict.observed {
                DiskVersion::Missing => { ui.label(if focus_check { "The file was deleted or moved. Keep your editor text or save it under another name." } else { "The file was deleted or moved. Overwrite will recreate it with your editor text." }); }
                DiskVersion::Text(text) => {
                    ui.label(if focus_check { "Another program changed this file. Reload replaces your editor text, including unsaved edits. Keep retains your text without changing the disk file." } else { "Another program changed this file. Reload replaces your editor text; overwrite replaces the disk version." });
                    ui.collapsing("Preview disk version", |ui| {
                        let preview: String = text.chars().take(4000).collect();
                        egui::ScrollArea::both().max_height(180.).show_themed(ui, |ui| { ui.monospace(preview); });
                    });
                }
            }
            ui.label(&self.status);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(matches!(conflict.observed, DiskVersion::Text(_)), egui::Button::new("Reload from disk")).clicked() { action = 1; }
                if !focus_check && ui.button("Overwrite file").clicked() { action = 2; }
                if ui.button("Save as...").clicked() { action = 3; }
                if ui.button(if focus_check { "Keep editor text" } else { "Cancel" }).clicked() { action = 4; }
            });
        });
        if focus_check && (!shown.is_top_modal || shown.any_popup_open) {
            self.script_io.focus.action = None;
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = 4;
        }
        if !focus_check {
            self.ordinary_script_conflict_action(
                ctx,
                action,
                shown.response.layer_id,
                shown.is_top_modal,
                shown.any_popup_open,
            );
            return;
        }
        if focus_check {
            let frame = ctx.cumulative_frame_nr();
            if action != 0 {
                self.script_io.focus.action = Some((frame, action));
            }
            if ctx.will_discard() {
                return;
            }
            action = self
                .script_io
                .focus
                .action
                .take()
                .filter(|(queued, _)| *queued == frame)
                .map_or(0, |(_, action)| action);
        }
        if action == 0 {
            return;
        }
        let mut conflict = self.script_conflict.take().unwrap();
        if let Some(guard) = conflict.focus.take() {
            if self.busy
                || self.recovering_session
                || self.allow_close
                || self.file_action_pending()
                || !self.open_queue.is_empty()
                || !guard.matches(self)
            {
                return;
            }
            match action {
                1 => self.begin_script_io(
                    index,
                    script_io::Kind::ReloadExplicit {
                        expected: guard.expected,
                        activate_if: None,
                    },
                ),
                3 => {
                    self.save_document(index, true);
                }
                _ => self.status = "Kept editor text; disk file unchanged".into(),
            }
        }
    }

    fn script_conversion_ui(&mut self, ctx: &egui::Context, index: usize) {
        let conflict = self.script_conflict.as_ref().unwrap();
        let conversion = conflict.conversion.as_ref().unwrap();
        let mut action = 0_u8;
        egui::Modal::new(egui::Id::new("script_disk_conflict")).show(ctx, |ui| {
            ui.set_max_width(540.);
            ui.label(crate::tokens::title(ui.ctx(), "Save edited generated script?"));
            ui.label(crate::path_label::display(&conflict.path).to_string());
            ui.label("Your code has changed. Saving as an ordinary script changes only the MatRust declaration comment. Executable code and existing companion data are retained.");
            if conversion.same_folder {
                ui.label("The companion remains in this folder. Automatic companion copying ends for this script; a fresh figure export creates a new relocatable bundle.");
            } else {
                ui.label("Script only: companion data is NOT copied to this folder. Its load statements are unchanged. Copy dependencies yourself or re-export the figure for automatic relocation.");
            }
            ui.horizontal_wrapped(|ui| {
                let title = if conversion.same_folder { "Save edited script" } else { "Save script only" };
                if ui.add_enabled(!self.busy, egui::Button::new(title)).clicked() { action = 1; }
                if ui.button("Cancel").clicked() { action = 2; }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = 2;
        }
        let frame = ctx.cumulative_frame_nr();
        let conversion = self
            .script_conflict
            .as_mut()
            .unwrap()
            .conversion
            .as_mut()
            .unwrap();
        if conversion.pending.is_some_and(|(old, _)| old != frame) {
            conversion.pending = None;
        }
        if action != 0 {
            conversion.pending = Some((frame, action));
        }
        if ctx.will_discard() {
            return;
        }
        let Some((_, action)) = conversion.pending.take() else {
            return;
        };
        if action == 1 && self.busy {
            return;
        }
        let conflict = self.script_conflict.take().unwrap();
        let conversion = conflict.conversion.unwrap();
        if action == 2 || !conversion.document.same_snapshot(&self.documents[index]) {
            self.script_save_queue = None;
            self.pending_close = None;
            self.pending_quit = false;
            self.quit_discarded_figures.clear();
            self.status = if action == 2 {
                "Save cancelled; editor text, script and companion data retained"
            } else {
                "Script changed since the conversion choice; request Save again to review it"
            }
            .into();
            return;
        }
        let other_paths = self
            .documents
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (i != index).then(|| d.path.clone()).flatten())
            .collect();
        self.begin_script_io(
            index,
            script_io::Kind::SaveEdited {
                path: conflict.path,
                confirmed: conversion.confirmed,
                other_paths,
            },
        );
    }
}

// One ordinary-conflict gesture. No document text or observed disk text copied.
pub(super) struct OrdinaryAction {
    frame: u64,
    epoch: u64,
    choice: u8,
    document: u64,
    active: u64,
    path: Option<PathBuf>,
    digest: blake3::Hash,
    interaction: u64,
    focus: Option<egui::Id>,
    layer: egui::LayerId,
    continuation: blake3::Hash,
    folder: PathBuf,
}
impl Workspace {
    fn script_continuation_identity(&self) -> blake3::Hash {
        // Small existing ID collections only; never stream document contents here.
        let mut hash = blake3::Hasher::new();
        hash.update(&[u8::from(self.pending_quit)]);
        match self.pending_close {
            None => {
                hash.update(&[0]);
            }
            Some(index) => {
                hash.update(&[1]);
                hash.update(&self.documents.get(index).map_or(0, |d| d.id).to_le_bytes());
            }
        }
        if let Some(queue) = &self.script_save_queue {
            hash.update(&[1]);
            hash.update(&(queue.len() as u64).to_le_bytes());
            for id in queue {
                hash.update(&id.to_le_bytes());
            }
        } else {
            hash.update(&[0]);
        }
        hash.update(&(self.quit_discarded_figures.len() as u64).to_le_bytes());
        for id in &self.quit_discarded_figures {
            hash.update(&id.to_le_bytes());
        }
        hash.finalize()
    }
    fn ordinary_conflict_context(&self, ctx: &egui::Context, layer: egui::LayerId) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|i| i.focused && !i.viewport().close_requested())
            && ctx.memory(|m| m.top_modal_layer() == Some(layer))
            && !egui::Popup::is_any_open(ctx)
            && !self.busy
            && !self.script_io.pending()
            && !self.file_action_pending_without_ordinary_script_conflict()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
            && self.open_queue.is_empty()
            && !self.folder_browser.navigating()
    }
    fn ordinary_conflict_owner(&self, ctx: &egui::Context, action: &OrdinaryAction) -> bool {
        self.ordinary_conflict_context(ctx, action.layer)
            && action.frame == ctx.cumulative_frame_nr()
            && action.epoch == self.script_io.conflict_epoch
            && action.interaction == self.document_save.interaction_generation()
            && action.active == self.documents[self.active].id
            && action.focus == ctx.memory(|m| m.focused())
            && action.folder == self.folder
            && action.continuation == self.script_continuation_identity()
            && self.script_conflict.as_ref().is_some_and(|c| {
                c.document == action.document && c.focus.is_none() && c.conversion.is_none()
            })
            && self
                .documents
                .iter()
                .any(|d| d.id == action.document && d.path == action.path)
    }
    fn ordinary_conflict_snapshot(
        &self,
        ctx: &egui::Context,
        action: &OrdinaryAction,
    ) -> Option<usize> {
        if !self.ordinary_conflict_owner(ctx, action) {
            return None;
        }
        let index = self
            .documents
            .iter()
            .position(|d| d.id == action.document)?;
        super::script_reload::digest(&self.documents[index])
            .is_ok_and(|digest| digest == action.digest)
            .then_some(index)
    }
    pub(super) fn cancel_ordinary_script_conflict(&mut self) {
        self.script_io.conflict_epoch = self.script_io.conflict_epoch.wrapping_add(1);
        if let Some(conflict) = &mut self.script_conflict
            && let Some(action) = conflict.action.take()
        {
            conflict.finished = Some(action.frame);
        }
    }
    pub(super) fn observe_ordinary_script_conflict(&mut self, ctx: &egui::Context) {
        // Check cheap authority before widget painting can surrender foreign focus.
        // Full document digest is checked only when consuming a gesture.
        if self
            .script_conflict
            .as_ref()
            .and_then(|c| c.action.as_ref())
            .is_some_and(|action| !self.ordinary_conflict_owner(ctx, action))
        {
            self.cancel_ordinary_script_conflict();
        }
    }
    fn ordinary_script_conflict_action(
        &mut self,
        ctx: &egui::Context,
        choice: u8,
        layer: egui::LayerId,
        top: bool,
        popup: bool,
    ) {
        if !top || popup || !self.ordinary_conflict_context(ctx, layer) {
            self.cancel_ordinary_script_conflict();
            return;
        }
        let frame = ctx.cumulative_frame_nr();
        let Some(conflict) = &self.script_conflict else {
            return;
        };
        if conflict.finished == Some(frame) {
            return;
        }
        if choice != 0 {
            if let Some(action) = self.script_conflict.as_mut().unwrap().action.as_mut() {
                // Escape/Cancel wins even if a pointer action was retained first.
                if choice == 4 {
                    action.choice = 4;
                }
            } else {
                let document = self.script_conflict.as_ref().unwrap().document;
                let Some(index) = self.documents.iter().position(|d| d.id == document) else {
                    return;
                };
                let digest = match super::script_reload::digest(&self.documents[index]) {
                    Ok(digest) => digest,
                    Err(error) => {
                        self.status = error;
                        return;
                    }
                };
                let action = OrdinaryAction {
                    frame,
                    epoch: self.script_io.conflict_epoch,
                    choice,
                    document,
                    active: self.documents[self.active].id,
                    path: self.documents[index].path.clone(),
                    digest,
                    interaction: self.document_save.interaction_generation(),
                    focus: ctx.memory(|m| m.focused()),
                    layer,
                    continuation: self.script_continuation_identity(),
                    folder: self.folder.clone(),
                };
                self.script_conflict.as_mut().unwrap().action = Some(action);
            }
        }
        if ctx.will_discard() {
            return;
        }
        let conflict = self.script_conflict.as_mut().unwrap();
        let Some(action) = conflict.action.take() else {
            return;
        };
        conflict.finished = Some(frame);
        let Some(index) = self.ordinary_conflict_snapshot(ctx, &action) else {
            return;
        };
        match action.choice {
            1 => self.begin_script_io(index, script_io::Kind::Reload),
            2 => {
                // Move the existing decision while submitting, so its observed
                // disk text is copied only by the existing worker request path.
                let conflict = self.script_conflict.take().unwrap();
                self.save_script_path(index, &conflict.path, Some(&conflict.observed));
                if self.script_conflict.is_none() {
                    self.script_conflict = Some(conflict);
                }
            }
            3 => {
                // Keep the decision identity present while the chooser is open.
                // The consumed ticket and epoch still invalidate Stop/replacement.
                let path = self.pick_script_save_path(index);
                let Some(index) = self.ordinary_conflict_snapshot(ctx, &action) else {
                    return;
                };
                if let Some(path) = path {
                    self.save_script_path(index, &path, None);
                } else {
                    self.status = "Save cancelled; script remains open".into();
                }
            }
            _ => {
                self.script_conflict = None;
                self.script_save_queue = None;
                self.pending_close = None;
                self.pending_quit = false;
                self.quit_discarded_figures.clear();
                self.status = "Save cancelled; editor text and disk file retained".into();
            }
        }
    }
}
