//! Explicit New model intent is applied after current-frame model field edits.
use super::*;

impl Workspace {
    pub(super) fn can_new_model(&self) -> bool {
        !self.busy && !self.file_action_pending() && !self.allow_close
    }

    pub(super) fn request_new_model(&mut self) {
        if self.can_new_model() {
            self.new_model_requested = true;
        }
    }

    pub(super) fn finish_new_model(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() || !self.new_model_requested {
            return;
        }
        self.new_model_requested = false;
        if !self.can_new_model() {
            self.status =
                "New model not started during another operation; current model retained".into();
            return;
        }
        if self.toolkits.request_new_model() {
            self.document_save.activate(document_save::Pane::Model);
            self.recovery_dirty = true;
            self.status = self.toolkits.message.clone();
            ctx.request_repaint();
        }
    }
}
