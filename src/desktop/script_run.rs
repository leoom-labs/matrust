//! One bounded GUI run intent, published after the editor handles this frame.
use super::*;

#[derive(Default)]
pub(super) struct State {
    pending: Option<(u64, u64, bool)>, // frame, stable document ID, selection-only
    finished_frame: Option<u64>,
}

impl Workspace {
    pub(super) fn request_script_run(&mut self, ctx: &egui::Context, selection: bool) {
        let frame = ctx.cumulative_frame_nr();
        if self.has_startup_placeholder()
            || self.busy
            || self.file_action_pending()
            || self.allow_close
            || self.script_run.finished_frame == Some(frame)
        {
            return;
        }
        // Keep the first gesture across layout retries; never retain source text.
        if self
            .script_run
            .pending
            .is_none_or(|(queued, _, _)| queued != frame)
        {
            self.script_run.pending = Some((frame, self.script_action_document(ctx), selection));
        }
    }

    pub(super) fn finish_script_run(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some((frame, document, selection)) = self.script_run.pending.take() else {
            return;
        };
        if frame != ctx.cumulative_frame_nr() || self.script_run.finished_frame == Some(frame) {
            return;
        }
        self.script_run.finished_frame = Some(frame);
        // A same-frame close, new tab, modal or worker action wins over Run.
        // Dropping the intent here also prevents a later Cancel from reviving it.
        if self.has_startup_placeholder()
            || self.busy
            || self.file_action_pending()
            || self.allow_close
            || self.documents[self.active].id != document
        {
            return;
        }
        if self.layout.center.is_some() {
            self.reveal_editor();
        }
        if selection {
            self.run_selection();
        } else {
            self.run_document(None);
        }
    }
}
