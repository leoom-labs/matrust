//! Script document actions share existing save/conflict and close safeguards.
use super::*;

impl Workspace {
    pub(super) fn begin_save_all_scripts(&mut self) {
        if self.busy || self.file_action_pending() {
            self.status = "Finish the current operation before saving all scripts".into();
            return;
        }
        let ids: std::collections::VecDeque<_> = self
            .documents
            .iter()
            .filter(|doc| doc.dirty)
            .map(|doc| doc.id)
            .collect();
        if ids.is_empty() {
            self.status = "No modified scripts to save".into();
        } else {
            self.script_save_queue = Some(ids);
        }
    }

    pub(super) fn advance_script_saves(&mut self, ctx: &egui::Context) {
        if self.busy
            || self.script_io.pending()
            || self.script_conflict.is_some()
            || self.allow_close
        {
            return;
        }
        let Some(id) = self
            .script_save_queue
            .as_ref()
            .and_then(|q| q.front())
            .copied()
        else {
            return;
        };
        let saved = match self.documents.iter().position(|doc| doc.id == id) {
            Some(index) if self.documents[index].dirty => self.save_document(index, false),
            _ => true,
        };
        if saved {
            let queue = self.script_save_queue.as_mut().unwrap();
            queue.pop_front();
            if queue.is_empty() {
                self.script_save_queue = None;
                self.status = "Save all scripts finished".into();
            }
        } else if !self.script_io.pending() && self.script_conflict.is_none() {
            // A failed write or cancelled filename choice stops the sequence;
            // earlier atomic saves remain saved and later buffers remain dirty.
            self.script_save_queue = None;
        }
        ctx.request_repaint();
    }

    pub(super) fn script_saves_ui(&mut self, ctx: &egui::Context) {
        if self.script_conflict.is_some() && !self.script_io.pending() {
            return;
        }
        if self.script_save_queue.is_some()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.stop_work();
            self.status = "Remaining script saves cancelled; buffers retained".into();
            return;
        }
        if let Some(queue) = &self.script_save_queue {
            let remaining = queue.len();
            egui::Modal::new(egui::Id::new("save_all_scripts")).show(ctx, |ui| {
                ui.label(crate::tokens::title(ui.ctx(), "Saving scripts"));
                ui.label(format!("{remaining} scripts remaining"));
                if ui.button("Cancel remaining saves").clicked() {
                    self.stop_work();
                    self.status = "Remaining script saves cancelled; buffers retained".into();
                }
            });
            ctx.request_repaint();
        }
    }

    pub(super) fn request_script_close(&mut self, index: usize) {
        if self.file_action_pending() || index >= self.documents.len() {
            return;
        }
        if self.documents[index].dirty {
            self.pending_close = Some(index);
        } else {
            self.remove_document(index);
        }
    }

    pub(super) fn cycle_script(&mut self, ctx: &egui::Context, backwards: bool) {
        let count = self.documents.len();
        let index = if backwards {
            (self.active + count - 1) % count
        } else {
            (self.active + 1) % count
        };
        self.activate_script_index(index);
        self.reveal_editor();
        self.script_tab_reveal = Some(self.documents[self.active].id);
        ctx.memory_mut(|memory| {
            // Tab traversal is derived before widgets consume keyboard events.
            memory.move_focus(egui::FocusDirection::None);
            memory.request_focus(editor::id(self.documents[self.active].id));
        });
    }
}
