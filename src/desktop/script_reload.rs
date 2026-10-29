//! Explicit, confirmed script reload. No disk I/O occurs on the UI thread.
use super::*;
use crate::theme::ThemedScrollArea;
use std::io::Write;

#[derive(Clone)]
pub struct Snapshot {
    pub id: u64,
    pub path: PathBuf,
    digest: blake3::Hash,
}

struct DigestWriter(blake3::Hasher);
impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn digest(document: &Document) -> matrust::Result<blake3::Hash> {
    // Stream the bounded document, including its private saved baseline, into a
    // fixed-size digest. Never allocate another serialized text buffer. This is
    // done only on request/confirmation/completion, never every layout frame.
    let mut writer = DigestWriter(blake3::Hasher::new());
    serde_json::to_writer(&mut writer, document).map_err(|e| e.to_string())?;
    Ok(writer.0.finalize())
}

impl Snapshot {
    pub(super) fn capture(document: &Document) -> matrust::Result<Self> {
        Ok(Self {
            id: document.id,
            path: document
                .path
                .clone()
                .ok_or("Untitled script has no file to reload")?,
            digest: digest(document)?,
        })
    }
    pub(super) fn matches(&self, document: &Document) -> bool {
        document.id == self.id
            && document.path.as_ref() == Some(&self.path)
            && digest(document).is_ok_and(|value| value == self.digest)
    }
}

pub(super) struct Confirmation {
    snapshot: Snapshot,
    dirty: bool,
}

impl Workspace {
    pub(super) fn can_reload_script(&self) -> bool {
        self.can_reload_script_with_pending(self.file_action_pending())
    }
    pub(super) fn can_reload_script_with_pending(&self, pending: bool) -> bool {
        !self.busy
            && !pending
            && !self.allow_close
            && self.generic_save_target() == document_save::Pane::Script
            && self
                .documents
                .get(self.active)
                .is_some_and(|d| d.path.is_some())
    }

    pub(super) fn request_script_reload(&mut self) {
        if !self.can_reload_script() {
            return;
        }
        let document = &self.documents[self.active];
        match Snapshot::capture(document) {
            Ok(snapshot) => {
                self.script_reload = Some(Confirmation {
                    snapshot,
                    dirty: document.dirty,
                });
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn confirm_script_reload(&mut self) {
        let Some(prompt) = self.script_reload.take() else {
            return;
        };
        if self.busy || self.file_action_pending() || self.allow_close {
            self.status =
                "Reload not started during another operation; editor text retained".into();
            return;
        }
        let Some(index) = self
            .documents
            .iter()
            .position(|d| d.id == prompt.snapshot.id)
        else {
            self.status = "The script closed before reload; no document was replaced".into();
            return;
        };
        if !prompt.snapshot.matches(&self.documents[index]) {
            self.status =
                "The script changed while confirmation was open; request reload again".into();
            return;
        }
        let activate_if = (index == self.active
            && self.generic_save_target() == document_save::Pane::Script)
            .then(|| self.document_save.interaction_generation());
        self.begin_script_io(
            index,
            script_io::Kind::ReloadExplicit {
                expected: prompt.snapshot,
                activate_if,
            },
        );
    }

    pub(super) fn script_reload_ui(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.script_reload else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        let shown = egui::Modal::new(egui::Id::new("script_reload_confirmation")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(200., 480.));
            ui.label(crate::tokens::title(ui.ctx(), "Reload script from disk?"));
            egui::ScrollArea::vertical()
                .id_salt("script_reload_details")
                .max_height((ctx.content_rect().height() - 210.).clamp(48., 260.))
                .show_themed(ui, |ui| {
                    ui.label(crate::path_label::display(&prompt.snapshot.path).to_string());
                    if prompt.dirty {
                        ui.label("Unsaved edits in this script will be discarded.");
                    }
                    ui.label("Reload replaces this editor text and clears its undo history. The file on disk, other documents and workspace variables stay unchanged. The script is not run.");
                });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                confirm = ui.button("Reload script").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            cancel = true;
        }
        if cancel {
            self.script_reload = None;
            self.status = "Reload cancelled; editor text and disk file retained".into();
        } else if confirm {
            self.confirm_script_reload();
        }
    }
}
