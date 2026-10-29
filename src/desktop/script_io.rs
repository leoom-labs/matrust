//! Background script file operations with publication by stable document ID.
use super::*;
use matrust::document::{DiskVersion, SaveOutcome, absolute_path, same_path};

pub enum Kind {
    Save {
        path: PathBuf,
        confirmed: Option<DiskVersion>,
        other_paths: Vec<PathBuf>,
    },
    SaveEdited {
        path: PathBuf,
        confirmed: Option<DiskVersion>,
        other_paths: Vec<PathBuf>,
    },
    Reload,
    ReloadExplicit {
        expected: script_reload::Snapshot,
        activate_if: Option<u64>,
    },
    Check,
    CheckFocus(script_focus::Guard),
}
pub struct Request {
    pub document: Document,
    pub kind: Kind,
}
pub enum Outcome {
    Saved(Document),
    Converted {
        document: Document,
        before: Document,
    },
    Conversion {
        document: Document,
        path: PathBuf,
        confirmed: Option<DiskVersion>,
        same_folder: bool,
    },
    Reloaded {
        document: Document,
        before: String,
    },
    ReloadedExplicit {
        document: Document,
        expected: script_reload::Snapshot,
        activate_if: Option<u64>,
    },
    Conflict(PathBuf, DiskVersion),
    Unchanged,
}
pub struct Finished {
    pub id: u64,
    pub confirmed: bool,
    pub focus: Option<script_focus::Guard>,
    pub result: matrust::Result<Outcome>,
}
#[derive(Default)]
pub struct State {
    pub(super) conflict_epoch: u64,
    #[cfg(test)]
    pub(super) save_picker: Option<fn(&mut Workspace, usize) -> Option<PathBuf>>,
    #[cfg(test)]
    pub(super) save_picker_calls: usize,
    pub queued: Option<Request>,
    pub active: bool,
    active_writing: bool,
    // One completed explicit read waits for this frame's editor/input pass.
    pub deferred_reload: Option<Finished>,
    pub resume: bool,
    pub focus: script_focus::State,
}
impl State {
    pub fn pending(&self) -> bool {
        self.queued.is_some() || self.active || self.deferred_reload.is_some()
    }
}
impl State {
    pub(super) fn progress_text(&self) -> &'static str {
        let writing = self
            .queued
            .as_ref()
            .map_or(self.active_writing, Request::writing);
        if writing {
            "A write already in progress may finish. Stop cancels subsequent actions."
        } else {
            "Reading from disk. Stop keeps the current editor text; no file is written."
        }
    }
}
impl Request {
    pub fn writing(&self) -> bool {
        matches!(self.kind, Kind::Save { .. } | Kind::SaveEdited { .. })
    }
    pub fn failure(&self, error: String) -> Finished {
        Finished {
            id: self.document.id,
            focus: match &self.kind {
                Kind::CheckFocus(guard) => Some(guard.clone()),
                _ => None,
            },
            confirmed: matches!(
                &self.kind,
                Kind::Save {
                    confirmed: Some(_),
                    ..
                } | Kind::SaveEdited {
                    confirmed: Some(_),
                    ..
                }
            ),
            result: Err(error),
        }
    }
}
pub fn perform(mut request: Request, cancel: &std::sync::atomic::AtomicBool) -> Finished {
    let mut reply = request.failure("Script file operation cancelled before starting".into());
    if cancel.load(Ordering::Relaxed) {
        return reply;
    }
    let converting = matches!(&request.kind, Kind::SaveEdited { .. });
    reply.result = (|| {
        match request.kind {
            Kind::Save {
                path,
                confirmed,
                other_paths,
            }
            | Kind::SaveEdited {
                path,
                confirmed,
                other_paths,
            } => {
                let requested_path = path.clone();
                let path = absolute_path(&path)?;
                for other in &other_paths {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Script save cancelled before writing".into());
                    }
                    if same_path(other, &path) {
                        return Err(
                            "This file is already open in another script tab. Choose another name."
                                .into(),
                        );
                    }
                }
                if cancel.load(Ordering::Relaxed) {
                    return Err("Script save cancelled before writing".into());
                }
                if !converting && request.document.needs_reconstruction_conversion()? {
                    let same_folder = request
                        .document
                        .path
                        .as_ref()
                        .and_then(|p| p.parent())
                        .zip(path.parent())
                        .is_some_and(|(a, b)| same_path(a, b));
                    return Ok(Outcome::Conversion {
                        document: request.document,
                        path,
                        confirmed,
                        same_folder,
                    });
                }
                // A completed write must be acknowledged, even after Stop.
                if converting {
                    let before = request.document.clone();
                    match request
                        .document
                        .try_save_edited_reconstruction_with_cancel(
                            &requested_path,
                            confirmed.as_ref(),
                            cancel,
                        )? {
                        SaveOutcome::Saved => Ok(Outcome::Converted {
                            document: request.document,
                            before,
                        }),
                        SaveOutcome::Conflict(version) => Ok(Outcome::Conflict(path, version)),
                    }
                } else {
                    match request.document.try_save_with_cancel(
                        &requested_path,
                        confirmed.as_ref(),
                        cancel,
                    )? {
                        SaveOutcome::Saved => Ok(Outcome::Saved(request.document)),
                        SaveOutcome::Conflict(version) => Ok(Outcome::Conflict(path, version)),
                    }
                }
            }
            Kind::Reload => {
                let before = request.document.text.clone();
                request.document.reload()?;
                request.document.verify_reconstruction(cancel)?;
                Ok(Outcome::Reloaded {
                    document: request.document,
                    before,
                })
            }
            Kind::ReloadExplicit {
                expected,
                activate_if,
            } => {
                request.document.reload()?;
                request.document.verify_reconstruction(cancel)?;
                Ok(Outcome::ReloadedExplicit {
                    document: request.document,
                    expected,
                    activate_if,
                })
            }
            Kind::Check | Kind::CheckFocus(_) => Ok(match request.document.external_change()? {
                Some(version) => Outcome::Conflict(request.document.path.unwrap(), version),
                None => Outcome::Unchanged,
            }),
        }
    })();
    reply
}
impl Workspace {
    pub(super) fn begin_script_io(&mut self, index: usize, kind: Kind) {
        if self.script_io.pending() || index >= self.documents.len() {
            return;
        }
        if !matches!(&kind, Kind::CheckFocus(_)) {
            // Explicit file actions supersede an older automatic observation.
            self.cancel_script_focus();
        }
        self.script_io.queued = Some(Request {
            document: self.documents[index].clone(),
            kind,
        });
        self.advance_script_io();
    }
    pub(super) fn advance_script_io(&mut self) {
        if self.busy || self.script_io.active || self.script_io.deferred_reload.is_some() {
            return;
        }
        let Some(request) = self.script_io.queued.take() else {
            return;
        };
        let background = matches!(&request.kind, Kind::CheckFocus(_));
        let previous_status = background.then(|| self.status.clone());
        let writing = request.writing();
        self.script_io.active_writing = writing;
        let failure = request.failure("Script worker unavailable".into());
        self.submit(Job::ScriptIo(Box::new(request)));
        self.script_io.active = self.busy;
        self.script_io.focus.active = background && self.busy;
        self.worker.opening = self.busy && !writing;
        if let Some(status) = previous_status {
            self.status = status;
        }
        if self.busy && !background {
            self.status = if writing {
                "Saving script..."
            } else {
                "Reading script file..."
            }
            .into();
        } else if !self.busy {
            self.receive_script_io(failure);
        }
    }
    pub(super) fn receive_script_io(&mut self, reply: Finished) {
        self.script_io.active = false;
        self.worker.opening = false;
        if reply.focus.is_some() {
            self.receive_script_focus(reply);
            return;
        }
        if matches!(&reply.result, Ok(Outcome::ReloadedExplicit { .. }))
            && !self.worker.cancel.load(Ordering::Relaxed)
        {
            // Receive runs before editor layout. Do not replace text until all
            // input from the completion frame has been applied and observed.
            self.script_io.deferred_reload = Some(reply);
            return;
        }
        self.publish_script_io(reply);
    }
    pub(super) fn finish_script_reload(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        if let Some(reply) = self.script_io.deferred_reload.take() {
            self.publish_script_io(reply);
            ctx.request_repaint();
        }
    }
    fn publish_script_io(&mut self, reply: Finished) {
        let cancelled = self.worker.cancel.load(Ordering::Relaxed);
        let Some(index) = self.documents.iter().position(|d| d.id == reply.id) else {
            return;
        };
        let mut success = false;
        let standalone_reload = matches!(&reply.result, Ok(Outcome::ReloadedExplicit { .. }));
        let result = match reply.result {
            Ok(Outcome::Converted { document, before }) => {
                // Adopt the explicitly converted comment only when the editor
                // still equals the consented snapshot; newer edits stay dirty.
                if self.documents[index].same_snapshot(&before) {
                    self.documents[index].text = document.text.clone();
                }
                Ok(Outcome::Saved(document))
            }
            other => other,
        };
        match result {
            Ok(Outcome::Saved(mut saved)) => {
                // Keep edits made while the submitted snapshot was being saved.
                saved.text = std::mem::take(&mut self.documents[index].text);
                saved.changed();
                let path = saved.path.clone().unwrap();
                self.documents[index] = saved;
                self.recent.retain(|p| p != &path);
                self.recent.insert(0, path.clone());
                self.recent.truncate(20);
                self.refresh_folder();
                self.status = format!(
                    "Saved {}{}",
                    crate::path_label::display(&path),
                    if cancelled {
                        "; subsequent actions cancelled"
                    } else {
                        ""
                    }
                );
                success = true;
            }
            _ if cancelled => {
                self.status = "Script file operation cancelled; editor text retained".into()
            }
            Ok(Outcome::Reloaded { document, before }) => {
                if self.documents[index].text != before {
                    self.status =
                        "Editor changed during reload; edits retained. Reload again or cancel."
                            .into();
                } else {
                    self.documents[index] = document;
                    self.forget_script_editor(reply.id);
                    self.selection = None;
                    self.editor_issue = None;
                    self.activate_script_index(index);
                    self.reveal_editor();
                    self.status = "Reloaded script from disk".into();
                    success = true;
                }
            }
            Ok(Outcome::ReloadedExplicit {
                document,
                expected,
                activate_if,
            }) => {
                if !expected.matches(&self.documents[index]) {
                    self.status = "Script changed during reload; text, path and saved state retained. Request reload again.".into();
                } else {
                    let activate = activate_if.is_some_and(|generation| {
                        generation == self.document_save.interaction_generation()
                    });
                    self.documents[index] = document;
                    self.forget_script_editor(reply.id);
                    if self.active == index {
                        self.selection = None;
                    }
                    if self
                        .editor_issue
                        .as_ref()
                        .is_some_and(|issue| issue.origin.document == reply.id)
                    {
                        self.editor_issue = None;
                    }
                    if activate {
                        self.activate_script_index(index);
                        self.selection = None;
                        self.script_tab_reveal = Some(reply.id);
                        self.reveal_editor();
                    }
                    self.status = if activate {
                        "Reloaded script from disk"
                    } else {
                        "Reloaded script in background; current document selection retained"
                    }
                    .into();
                    success = true;
                }
            }
            Ok(Outcome::Conflict(path, observed)) => {
                self.cancel_ordinary_script_conflict();
                self.script_conflict = Some(script_files::Conflict {
                    action: None,
                    finished: None,
                    document: reply.id,
                    path,
                    observed,
                    focus: None,
                    conversion: None,
                });
                self.status = if reply.confirmed {
                    "The file changed again. Review the latest version before overwriting."
                } else {
                    "The script changed or was deleted on disk."
                }
                .into();
            }
            Ok(Outcome::Conversion {
                document,
                path,
                confirmed,
                same_folder,
            }) => {
                if !document.same_snapshot(&self.documents[index]) {
                    self.status =
                        "Script changed while preparing the choice; request Save again.".into();
                    self.script_save_queue = None;
                } else {
                    self.cancel_ordinary_script_conflict();
                    self.script_conflict = Some(script_files::Conflict {
                        action: None,
                        finished: None,
                        document: reply.id,
                        path,
                        observed: DiskVersion::Missing,
                        focus: None,
                        conversion: Some(script_files::Conversion {
                            document,
                            confirmed,
                            same_folder,
                            pending: None,
                        }),
                    });
                    self.status =
                        "Edited generated script needs an explicit script-only save choice.".into();
                }
            }
            Ok(Outcome::Converted { .. }) => unreachable!("converted publication normalized above"),
            Ok(Outcome::Unchanged) => self.status = "Script file is unchanged".into(),
            Err(error) => {
                self.status = error;
                self.script_save_queue = None;
            }
        }
        if success {
            if !standalone_reload {
                self.script_conflict = None;
            }
            self.recovery_dirty = true;
            if !cancelled && !standalone_reload {
                if let Some(queue) = &mut self.script_save_queue
                    && queue.front() == Some(&reply.id)
                {
                    queue.pop_front();
                    if queue.is_empty() {
                        self.script_save_queue = None;
                        self.status = "Save all scripts finished".into();
                    }
                }
                self.script_io.resume = self.pending_quit || self.pending_close.is_some();
            }
        }
        self.console.push(self.status.clone());
    }
    pub(super) fn script_io_ui(&mut self, ctx: &egui::Context) {
        if self.script_io.resume && !self.script_io.pending() {
            self.script_io.resume = false;
            if self.pending_quit || self.pending_close.is_some() {
                self.save_pending_scripts(ctx);
            }
        }
        if self.script_io.pending()
            && !self.script_io.focus.active
            && self.script_save_queue.is_none()
        {
            egui::Window::new("Script file operation")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(&self.status);
                    ui.label(self.script_io.progress_text());
                    if ui.button("Stop script operation").clicked() {
                        self.stop_work();
                    }
                });
        }
    }
}
