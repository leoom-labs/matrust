use super::super::model_read::{Finished, Purpose, Request};
use super::super::model_write;
use super::{Action, Model, Tool, Toolkits, egui};
use matrust::simulation::document::{DiskVersion, ModelFile, SaveOutcome};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolbarKind {
    Open,
    Save,
    SaveAs,
}

#[cfg(test)]
type ToolbarPicker =
    fn(&mut super::super::Workspace, &egui::Context, ToolbarKind) -> Option<PathBuf>;

#[derive(Default)]
pub(crate) struct ToolbarState {
    pending: Option<ToolbarAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
    #[cfg(test)]
    pub(crate) picker: Option<ToolbarPicker>,
    #[cfg(test)]
    pub(crate) calls: Vec<ToolbarKind>,
}

#[derive(PartialEq, Eq)]
struct ModelStamp {
    model: blake3::Hash,
    baseline: Option<blake3::Hash>,
}

struct ToolbarAction {
    kind: ToolbarKind,
    frame: (egui::ViewportId, u64),
    generation: u64,
    identity: u64,
    model: Option<ModelStamp>,
    path: Option<PathBuf>,
    folder: PathBuf,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    busy: bool,
}

impl Toolkits {
    fn toolbar_model_stamp(&self) -> Option<ModelStamp> {
        use super::model_canvas::validation::gesture_stamp;
        Some(ModelStamp {
            model: gesture_stamp(&self.model)?,
            baseline: match self.file.baseline() {
                Some(model) => Some(gesture_stamp(model)?),
                None => None,
            },
        })
    }
}

impl super::super::Workspace {
    fn model_toolbar_context(&self, ctx: &egui::Context) -> bool {
        // Unlike generic Save, these toolbar actions may queue behind an
        // unrelated calculation. Keep that behavior; existing model I/O cannot
        // overlap and the common predicate excludes foreign file/modal work.
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|i| i.focused && !i.viewport().close_requested())
            && ctx.memory(|m| m.top_modal_layer().is_none())
            && !egui::Popup::is_any_open(ctx)
            && !self.file_action_pending()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
            && self.toolkits.model_open
    }

    fn valid_model_toolbar_owner(&self, ctx: &egui::Context, action: &ToolbarAction) -> bool {
        self.model_toolbar_context(ctx)
            && action.frame == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.model_toolbar.generation
            && action.identity == self.toolkits.model_identity
            && action.path == self.toolkits.file.path
            && action.folder == self.folder
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|m| m.focused())
            && action.busy == self.busy
    }

    pub(crate) fn queue_model_toolbar(&mut self, ctx: &egui::Context, kind: ToolbarKind) {
        let frame = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if !self.model_toolbar_context(ctx)
            || self.model_toolbar.pending.is_some()
            || self.model_toolbar.finished == Some(frame)
        {
            return;
        }
        // Called only after Toolkits::ui: a toolbar click commits any field
        // edit and includes the settings/diagram finalized later in that paint.
        // No model or workspace clone, and no hash on an idle paint.
        self.model_toolbar.pending = Some(ToolbarAction {
            kind,
            frame,
            generation: self.model_toolbar.generation,
            identity: self.toolkits.model_identity,
            model: self.toolkits.toolbar_model_stamp(),
            path: self.toolkits.file.path.clone(),
            folder: self.folder.clone(),
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|m| m.focused()),
            busy: self.busy,
        });
    }

    pub(crate) fn cancel_model_toolbar(&mut self) {
        self.model_toolbar.generation = self.model_toolbar.generation.wrapping_add(1);
        if let Some(action) = self.model_toolbar.pending.take() {
            self.model_toolbar.finished = Some(action.frame);
        }
    }

    pub(crate) fn observe_model_toolbar(&mut self, ctx: &egui::Context) {
        // Preserve evidence of a late foreign focus owner before widget painting
        // can surrender it. Model content is checked only at final admission.
        if self
            .model_toolbar
            .pending
            .as_ref()
            .is_some_and(|action| !self.valid_model_toolbar_owner(ctx, action))
        {
            self.cancel_model_toolbar();
        }
    }

    pub(crate) fn finish_model_toolbar(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.model_toolbar.pending.take() else {
            return;
        };
        self.model_toolbar.finished = Some(action.frame);
        if !self.valid_model_toolbar_owner(ctx, &action) {
            return;
        }
        let Some(expected) = &action.model else {
            self.toolkits.message = "Model exceeds file-action inspection bounds; reduce the model before opening or saving".into();
            return;
        };
        if self.toolkits.toolbar_model_stamp().as_ref() != Some(expected) {
            self.toolkits.message =
                "Model changed before the file action; choose the action again".into();
            return;
        }
        let path = if action.kind == ToolbarKind::Save && action.path.is_some() {
            action.path.clone()
        } else {
            self.pick_model_toolbar_path(ctx, action.kind)
        };
        let Some(path) = path else { return };
        if ctx.will_discard()
            || !self.valid_model_toolbar_owner(ctx, &action)
            || self.toolkits.toolbar_model_stamp().as_ref() != Some(expected)
        {
            return;
        }
        match action.kind {
            ToolbarKind::Open => {
                self.toolkits.request_open(path);
            }
            ToolbarKind::Save | ToolbarKind::SaveAs => {
                self.toolkits.save_model_path(&path, None);
            }
        }
        // The existing coordinator waits if an unrelated calculation is busy.
        self.advance_model_read();
    }

    fn pick_model_toolbar_path(
        &mut self,
        _ctx: &egui::Context,
        kind: ToolbarKind,
    ) -> Option<PathBuf> {
        #[cfg(test)]
        {
            assert!(self.model_toolbar.calls.len() < 128);
            self.model_toolbar.calls.push(kind);
            let picker = self
                .model_toolbar
                .picker
                .expect("headless model toolbar picker");
            picker(self, _ctx, kind)
        }
        #[cfg(not(test))]
        {
            match kind {
                ToolbarKind::Open => rfd::FileDialog::new()
                    .add_filter("Block model (native or supported SLX)", &["mrsim", "slx"])
                    .pick_file(),
                ToolbarKind::Save | ToolbarKind::SaveAs => self.toolkits.choose_model_path(),
            }
        }
    }
}

#[cfg(test)]
impl ToolbarState {
    pub(crate) fn pending_for_test(&self) -> bool {
        self.pending.is_some()
    }
}

pub(super) struct Conflict {
    path: PathBuf,
    observed: DiskVersion,
}
impl Toolkits {
    pub fn model_dirty(&self) -> bool {
        self.file.dirty(&self.model)
    }
    pub fn model_metadata(&self) -> (Option<PathBuf>, Option<Model>) {
        (self.file.path.clone(), self.file.baseline().cloned())
    }
    pub fn restore_model(&mut self, model: Model, path: Option<PathBuf>, baseline: Option<Model>) {
        self.model_identity = self.model_identity.wrapping_add(1);
        self.block_list = Default::default();
        self.model = model;
        self.selected = self.selected.min(self.model.nodes.len().saturating_sub(1));
        self.canvas.reset_for_model(&self.model, self.selected);
        self.file = ModelFile::restored(path, baseline);
        self.coefficient_editor = None;
    }
    pub fn discard_model(&mut self) {
        self.model_identity = self.model_identity.wrapping_add(1);
        self.block_list = Default::default();
        if let Some(model) = self.file.baseline() {
            self.model = model.clone();
        } else {
            self.model = Model::default();
            self.file = ModelFile::default();
        }
        self.selected = self.selected.min(self.model.nodes.len().saturating_sub(1));
        self.canvas.reset_for_model(&self.model, self.selected);
        self.pending_model = None;
        self.new_model_pending = false;
        self.model_conflict = None;
        self.coefficient_editor = None;
    }
    pub fn model_file_modal_open(&self) -> bool {
        self.model_io_pending()
            || self.model_conflict.is_some()
            || self.pending_model.is_some()
            || self.new_model_pending
    }
    pub fn modal_open(&self) -> bool {
        self.model_file_modal_open() || self.coefficient_editor.is_some()
    }
    fn choose_model_path(&self) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(choice) = &self.model_path_choice {
            return choice.clone();
        }
        let mut dialog = rfd::FileDialog::new()
            .add_filter("MatRust model", &["mrsim"])
            .set_file_name("model.mrsim");
        if let Some(path) = &self.file.path {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
            dialog = dialog.set_file_name(path.file_name().unwrap_or_default().to_string_lossy());
        }
        dialog.save_file()
    }
    pub fn save_model(&mut self) -> bool {
        if self.model_conflict.is_some() || self.model_io_pending() {
            return false;
        }
        let Some(path) = self.file.path.clone().or_else(|| self.choose_model_path()) else {
            return false;
        };
        self.save_model_path(&path, None)
    }
    pub(crate) fn save_model_as(&mut self) -> bool {
        if self.model_io_pending() {
            return false;
        }
        let Some(path) = self.choose_model_path() else {
            return false;
        };
        self.save_model_path(&path, None)
    }
    pub fn save_model_path(&mut self, path: &Path, confirmed: Option<&DiskVersion>) -> bool {
        if self.model_io_pending() {
            return false;
        }
        self.model_write = Some(model_write::Request::Save(Box::new(model_write::Save {
            model: self.model.clone(),
            file: ModelFile::restored(self.file.path.clone(), self.file.baseline().cloned()),
            path: path.to_owned(),
            confirmed: confirmed.cloned(),
        })));
        self.message = "Saving model...".into();
        // Existing callers interpret true as completed, not merely queued.
        false
    }
    pub fn model_io_pending(&self) -> bool {
        self.model_read_pending() || self.model_write.is_some() || self.writing_model
    }
    pub fn take_model_write(&mut self) -> Option<model_write::Request> {
        if self.writing_model || self.reading_model.is_some() {
            return None;
        }
        let request = self.model_write.take()?;
        self.writing_model = true;
        Some(request)
    }
    pub fn cancel_model_io(&mut self) {
        self.model_read = None;
        self.model_write = None;
        self.model_resolved = false;
    }
    pub fn finish_model_write(&mut self, reply: model_write::Finished, cancelled: bool) {
        self.writing_model = false;
        match reply {
            model_write::Finished::Save {
                path,
                confirmed,
                result,
            } => match result {
                Ok(saved) => match saved.outcome {
                    SaveOutcome::Saved => {
                        self.file = saved.file;
                        self.model_conflict = None;
                        self.message = if cancelled {
                            "Model saved; subsequent actions cancelled"
                        } else {
                            "Model saved"
                        }
                        .into();
                        if !cancelled {
                            if self.new_model_pending && self.model_dirty() {
                                self.message = "Model changed during save; newer edits retained. Save again, discard or cancel New model.".into();
                            } else if self.pending_model.is_some() || self.new_model_pending {
                                self.complete_model_open();
                            } else {
                                self.model_resolved = true;
                            }
                        }
                    }
                    SaveOutcome::Conflict(observed) if !cancelled => {
                        self.message = if confirmed { "The model changed again. Review the current version before overwriting." }
                                else { "The model changed or was deleted on disk." }.into();
                        self.model_conflict = Some(Conflict { path, observed });
                        self.open_tool(Tool::Model);
                    }
                    SaveOutcome::Conflict(_) => {
                        self.message = "Model save cancelled; local work retained".into()
                    }
                },
                Err(error) => self.message = error,
            },
            model_write::Finished::Check(result) => {
                if cancelled {
                    self.message = "Model file check cancelled".into();
                    return;
                }
                match result {
                    Ok(Some((path, observed))) => {
                        self.model_conflict = Some(Conflict { path, observed });
                        self.message = "The model changed or was deleted on disk.".into();
                        self.open_tool(Tool::Model);
                    }
                    Ok(None) => self.message = "Model file is unchanged".into(),
                    Err(error) => self.message = error,
                }
            }
        }
    }
    pub fn request_open(&mut self, path: PathBuf) -> bool {
        if self.model_file_modal_open() {
            self.message = "Resolve the current model file dialog first.".into();
            return false;
        }
        self.model_read = Some(Request {
            path,
            purpose: Purpose::Open,
        });
        self.message = "Opening block model...".into();
        true
    }
    pub fn model_read_pending(&self) -> bool {
        self.model_read.is_some() || self.reading_model.is_some()
    }
    pub fn take_model_read(&mut self) -> Option<Request> {
        if self.reading_model.is_some() {
            return None;
        }
        let request = self.model_read.take()?;
        self.reading_model = Some(request.purpose);
        Some(request)
    }
    pub fn read_purpose(&self) -> Purpose {
        self.reading_model.unwrap()
    }
    pub fn finish_model_read(&mut self, reply: Finished) {
        self.reading_model = None;
        let (model, file) = match reply.result {
            Ok(loaded) => loaded,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        self.open_tool(Tool::Model);
        let published = !matches!(reply.purpose, Purpose::Open) || !self.model_dirty();
        match reply.purpose {
            Purpose::Open if self.model_dirty() => {
                self.pending_model = reply.source;
                self.message =
                    "Save or discard local changes before opening the selected model".into();
            }
            Purpose::Open => self.replace_model(model, file),
            Purpose::Confirmed => {
                self.replace_model(model, file);
                self.pending_model = None;
                self.model_resolved = true;
            }
            Purpose::Reload => {
                self.replace_model(model, file);
                self.model_conflict = None;
                if self.pending_model.is_some() || self.new_model_pending {
                    self.complete_model_open();
                } else {
                    self.model_resolved = true;
                }
            }
        }
        if published && reply.imported {
            self.message = "SLX imported as an unsaved native model; Save As .mrsim preserves the original SLX. Native block sizes and wires are re-rendered.".into();
        }
    }
    fn replace_model(&mut self, model: Model, file: ModelFile) {
        self.model_identity = self.model_identity.wrapping_add(1);
        self.block_list = Default::default();
        self.model = model;
        self.file = file;
        self.selected = 0;
        self.canvas.reset_for_model(&self.model, 0);
        self.coefficient_editor = None;
        self.message = "Model opened".into();
    }
    pub(super) fn complete_model_open(&mut self) {
        if self.new_model_pending {
            if !self.model_io_pending() {
                self.install_new_model();
            }
            return;
        }
        let Some(path) = self.pending_model.clone() else {
            return;
        };
        if !self.model_io_pending() {
            self.model_read = Some(Request {
                path,
                purpose: Purpose::Confirmed,
            });
            self.message = "Opening selected model...".into();
        }
    }
    pub fn preview_model_conflict(&mut self) {
        if self.model_io_pending() || self.file.path.is_none() {
            return;
        }
        self.model_write = Some(model_write::Request::Check(ModelFile::restored(
            self.file.path.clone(),
            self.file.baseline().cloned(),
        )));
        self.message = "Checking model file...".into();
    }
    pub(super) fn model_conflict_ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        let conflict = self.model_conflict.as_ref()?;
        let mut action = 0;
        egui::Modal::new(egui::Id::new("model_disk_conflict")).show(ctx,|ui|{
            ui.set_max_width(540.);
            ui.label(crate::tokens::title(ui.ctx(), "Model changed on disk"));
            ui.label(crate::path_label::display(&conflict.path).to_string());
            match &conflict.observed {
                DiskVersion::Missing=>{ui.label("The model file was deleted or moved. Overwrite will recreate it with the current model.");}
                DiskVersion::Model(m)=>{
                    ui.label("Reload replaces the current model; overwrite replaces the disk version.");
                    ui.label(format!("Disk version: {} blocks, {} samples, sample time {} s",m.nodes.len(),m.steps,m.sample_time));
                }
            }
            ui.label(&self.message);
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(matches!(conflict.observed,DiskVersion::Model(_)),egui::Button::new("Reload model")).clicked(){action=1;}
                if ui.button("Overwrite model").clicked(){action=2;}
                if ui.button("Save model as...").clicked(){action=3;}
                if ui.button("Cancel").clicked(){action=4;}
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = 4;
        }
        if action == 0 {
            return None;
        }
        let conflict = self.model_conflict.take().unwrap();
        let resolved = match action {
            1 => {
                self.model_read = Some(Request {
                    path: conflict.path.clone(),
                    purpose: Purpose::Reload,
                });
                self.message = "Reloading model...".into();
                false
            }
            2 => self.save_model_path(&conflict.path, Some(&conflict.observed)),
            3 => self.save_model_as(),
            _ => {
                self.pending_model = None;
                self.new_model_pending = false;
                self.message = "Model save cancelled; both versions retained".into();
                return Some(Action::ModelCancelled);
            }
        };
        if resolved {
            self.complete_model_open();
            if self.model_read_pending() {
                None
            } else {
                Some(Action::ModelResolved)
            }
        } else {
            if self.model_conflict.is_none() {
                self.model_conflict = Some(conflict);
            }
            None
        }
    }
}

impl Toolkits {
    pub fn request_new_model(&mut self) -> bool {
        if self.modal_open() {
            return false;
        }
        self.open_tool(Tool::Model);
        self.new_model_pending = true;
        if self.model_dirty() {
            self.message = "Save, discard or cancel before creating a new model".into();
        } else {
            self.install_new_model();
        }
        true
    }

    pub fn cancel_new_model(&mut self) {
        if self.new_model_pending {
            self.new_model_pending = false;
            self.message = "New model cancelled; current model retained".into();
        }
    }

    fn install_new_model(&mut self) {
        // Empty, valid storage: no default FIR calculation and no old buffers.
        // A clean fresh baseline lets subsequent edits determine dirty state.
        let model = Model {
            version: 1,
            sample_time: 0.001,
            steps: 1000,
            nodes: Vec::new(),
            positions: Vec::new(),
        };
        let file = ModelFile::restored(None, Some(model.clone()));
        self.replace_model(model, file);
        self.new_model_pending = false;
        self.pending_model = None;
        self.model_conflict = None;
        self.model_resolved = true;
        self.message = "New empty model; choose a filename when saving".into();
    }
}
