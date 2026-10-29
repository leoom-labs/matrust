//! Coalesced root-focus checks, using the existing serial script I/O worker.
use super::*;

#[derive(Clone)]
pub struct Guard {
    pub expected: script_reload::Snapshot,
    interaction: u64,
    epoch: u64,
}

#[derive(Default)]
pub struct State {
    last_focus: Option<bool>,
    observed_frame: Option<u64>,
    epoch: u64,
    pending: Option<Guard>,
    edge_pending: bool,
    deferred: Option<script_io::Finished>,
    pub active: bool,
    pub action: Option<(u64, u8)>,
}

impl Guard {
    pub(super) fn current_target(&self, app: &Workspace) -> bool {
        self.epoch == app.script_io.focus.epoch
            && self.interaction == app.document_save.interaction_generation()
            && app.documents.get(app.active).is_some_and(|document| {
                document.id == self.expected.id
                    && document.path.as_ref() == Some(&self.expected.path)
            })
    }
    pub(super) fn matches(&self, app: &Workspace) -> bool {
        self.current_target(app) && self.expected.matches(&app.documents[app.active])
    }
}

impl Workspace {
    pub(super) fn observe_script_focus(&mut self, ctx: &egui::Context) {
        if ctx.viewport_id() != self.editor_focus_viewport() {
            return;
        }
        let frame = ctx.cumulative_frame_nr();
        if self.script_io.focus.observed_frame == Some(frame) {
            return;
        }
        self.script_io.focus.observed_frame = Some(frame);
        self.record_script_focus(ctx.input(|input| input.viewport().focused));
    }

    pub(super) fn observe_native_script_focus(&mut self, input: &egui::RawInput) {
        // eframe calls raw_input_hook even for hidden/minimized root logic ticks,
        // when no Workspace::ui runs. Observe only a scalar here, never a file or
        // document snapshot; the next visible final UI pass chooses the target.
        if input.viewport_id == self.editor_focus_viewport() {
            self.record_script_focus(input.viewport().focused);
        }
    }

    fn record_script_focus(&mut self, focused: Option<bool>) {
        // Missing metadata is not a focus event. Preserve the last known state;
        // startup still needs an explicit false before a later true.
        let Some(focused) = focused else {
            return;
        };
        let regained = self.script_io.focus.last_focus == Some(false) && focused;
        self.script_io.focus.last_focus = Some(focused);
        if !regained
            || self
                .script_conflict
                .as_ref()
                .is_some_and(|c| c.focus.is_some())
        {
            return;
        }
        self.script_io.focus.epoch = self.script_io.focus.epoch.wrapping_add(1);
        self.script_io.focus.deferred = None;
        self.script_io.focus.pending = None;
        // Capture after visible input selects/edits the target. Coalesce hidden
        // logic ticks and discarded UI passes into one scalar, never a job log.
        self.script_io.focus.edge_pending = true;
    }

    pub(super) fn reset_script_focus_host(&mut self) {
        // Placement alone does not revoke a document-bound disk check or an
        // already displayed root confirmation. Only viewport-local observation
        // caches change; existing epoch/document/interaction guards still apply.
        self.script_io.focus.observed_frame = None;
        self.script_io.focus.last_focus = None;
    }

    pub(super) fn cancel_script_focus(&mut self) {
        self.script_io.focus.epoch = self.script_io.focus.epoch.wrapping_add(1);
        self.script_io.focus.pending = None;
        self.script_io.focus.edge_pending = false;
        self.script_io.focus.deferred = None;
        self.script_io.focus.action = None;
        if self
            .script_conflict
            .as_ref()
            .is_some_and(|c| c.focus.is_some())
        {
            self.script_conflict = None;
        }
    }

    pub(super) fn receive_script_focus(&mut self, reply: script_io::Finished) {
        self.script_io.focus.active = false;
        if !self.worker.cancel.load(Ordering::Relaxed)
            && reply
                .focus
                .as_ref()
                .is_some_and(|guard| guard.current_target(self))
        {
            // The final UI pass observes edits and document switches before any
            // prompt. At most one bounded disk result is retained.
            self.script_io.focus.deferred = Some(reply);
        }
    }

    pub(super) fn finish_script_focus(&mut self, ctx: &egui::Context) {
        if ctx.viewport_id() != self.editor_focus_viewport() {
            return;
        }
        if ctx.will_discard() {
            return;
        }
        if std::mem::take(&mut self.script_io.focus.edge_pending) {
            self.script_io.focus.pending = self
                .documents
                .get(self.active)
                .filter(|document| document.path.is_some())
                .and_then(|document| script_reload::Snapshot::capture(document).ok())
                .map(|expected| Guard {
                    expected,
                    interaction: self.document_save.interaction_generation(),
                    epoch: self.script_io.focus.epoch,
                });
        }
        if self.script_io.focus.last_focus != Some(true)
            || self.busy
            || self.file_action_pending()
            || self.recovering_session
            || !self.open_queue.is_empty()
            || self.folder_browser.navigating()
            || self.allow_close
            || ctx.input(|input| input.viewport().close_requested())
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some())
        {
            return;
        }
        if let Some(mut reply) = self.script_io.focus.deferred.take() {
            let guard = reply
                .focus
                .take()
                .expect("Only focus replies are deferred here");
            if !guard.matches(self) || self.worker.cancel.load(Ordering::Relaxed) {
                return;
            }
            match reply.result {
                Ok(script_io::Outcome::Conflict(path, observed)) => {
                    self.cancel_ordinary_script_conflict();
                    self.script_conflict = Some(script_files::Conflict {
                        action: None,
                        finished: None,
                        document: reply.id,
                        path,
                        observed,
                        focus: Some(guard),
                        conversion: None,
                    });
                    self.status =
                        "The script changed or was deleted on disk; editor text retained".into();
                    ctx.request_repaint();
                }
                Ok(script_io::Outcome::Unchanged) => {}
                Err(error) => {
                    self.status = error;
                    self.console.push(self.status.clone());
                }
                _ => unreachable!("Focus checks never save or reload"),
            }
            return;
        }
        if let Some(guard) = self.script_io.focus.pending.take()
            && guard.matches(self)
        {
            self.begin_script_io(self.active, script_io::Kind::CheckFocus(guard));
        }
    }
}
