//! Background preflight for the shared file-opening queue.
use super::*;

pub struct Request {
    pub path: PathBuf,
    pub scripts: Vec<(u64, PathBuf)>,
    pub tabs: usize,
}
pub enum Prepared {
    Directory(PathBuf),
    Script(Box<Document>),
    ExistingScript(u64, PathBuf),
    File(PathBuf, Kind),
}
#[derive(Clone, Copy)]
pub enum Kind {
    Workspace,
    Csv,
    Figure,
    Image,
    PagedImage,
    Eps,
    Model,
}
pub fn prepare(
    request: Request,
    cancel: &std::sync::atomic::AtomicBool,
) -> matrust::Result<Prepared> {
    let path = request.path.clone();
    prepare_inner(request, cancel)
        .map_err(|error| format!("{}: {error}", crate::path_label::display(&path)))
}
fn prepare_inner(
    request: Request,
    cancel: &std::sync::atomic::AtomicBool,
) -> matrust::Result<Prepared> {
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err("Cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    let path = std::fs::canonicalize(&request.path).map_err(|e| e.to_string())?;
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    check()?;
    if metadata.is_dir() {
        return Ok(Prepared::Directory(path));
    }
    if !metadata.is_file() {
        return Err("Open a regular file or directory".into());
    }
    let kind = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mr" | "m" | "txt" => {
            for (id, existing) in request.scripts {
                check()?;
                if matrust::document::same_path(&existing, &path) { return Ok(Prepared::ExistingScript(id, path)); }
            }
            if request.tabs >= session::MAX_DOCUMENTS { return Err("At most 32 script tabs can be open".into()); }
            let text = matrust::files::read_script(&path)?;
            check()?;
            let document = Document::from_loaded_script(path, text)?;
            document.verify_reconstruction(cancel)?;
            return Ok(Prepared::Script(Box::new(document)));
        }
        "mat" => Kind::Workspace,
        "csv" => Kind::Csv,
        "mrfig" | "mfig" | "fig" => Kind::Figure,
        "png" | "jpg" | "jpeg" | "bmp" | "svg" => Kind::Image,
        "pdf" | "tif" | "tiff" => Kind::PagedImage,
        "eps" => Kind::Eps,
        "mrsim" | "slx" => Kind::Model,
        _ => return Err("Supported: .mr, .m, .txt, supported .mat, .csv, .mrfig, .mfig, supported 2-D .fig, .mrsim, PNG, JPEG, BMP, TIFF, static SVG, PDF and configured EPS previews (Windows). SLX import supports explicit scalar single-rate discrete graphs; .mdl and other Simulink semantics are unsupported.".into()),
    };
    check()?;
    Ok(Prepared::File(path, kind))
}
impl Workspace {
    pub(super) fn try_open_path(&mut self, path: PathBuf) -> bool {
        if self.busy || self.file_action_pending() {
            return false;
        }
        let scripts = self
            .documents
            .iter()
            .filter_map(|doc| doc.path.clone().map(|path| (doc.id, path)))
            .collect();
        self.submit(Job::PrepareOpen(Box::new(Request {
            path,
            scripts,
            tabs: self.documents.len(),
        })));
        self.worker.opening = self.busy;
        if self.worker.opening {
            self.status = "Opening file…".into();
        }
        self.worker.opening
    }
    pub(super) fn complete_open(&mut self, prepared: matrust::Result<Prepared>) {
        self.worker.opening = false;
        let activate_document = self.document_save.open_completed();
        if self.worker.cancel.load(Ordering::Relaxed) {
            self.status = "File opening cancelled".into();
            return;
        }
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.status = format!("Could not open file: {error}");
                self.console.push(self.status.clone());
                return;
            }
        };
        let path = match prepared {
            Prepared::Directory(path) => {
                self.change_folder(path);
                return;
            }
            Prepared::ExistingScript(id, path) => {
                let Some(index) = self.documents.iter().position(|doc| doc.id == id) else {
                    self.status = "The script tab closed while opening; open the file again".into();
                    return;
                };
                if activate_document {
                    self.activate_script_index(index);
                    self.selection = None;
                    self.script_tab_reveal = Some(id);
                    self.reveal_editor();
                    self.status = "Selected open script; edits retained".into();
                } else {
                    self.status = "Script is open; current document selection retained".into();
                }
                path
            }
            Prepared::Script(document) => {
                if self.documents.len() >= session::MAX_DOCUMENTS {
                    self.status = "At most 32 script tabs can be open".into();
                    return;
                }
                let path = document.path.clone().unwrap();
                let id = document.id;
                if activate_document && self.has_startup_placeholder() {
                    self.forget_script_editor(self.documents[0].id);
                    self.documents[0] = *document;
                } else {
                    self.documents.push(*document);
                }
                if let Some(center) = &mut self.layout.center {
                    center.editor_revealed = true;
                }
                if activate_document {
                    self.script_tab_reveal = Some(id);
                    self.activate_script_index(self.documents.len() - 1);
                    self.selection = None;
                    self.reveal_editor();
                    self.status = "Script opened".into();
                } else {
                    self.status =
                        "Script opened in a background tab; current document retained".into();
                }
                path
            }
            Prepared::File(path, kind) => {
                match kind {
                    Kind::Workspace => self.submit(Job::InspectWorkspace(path.clone())),
                    Kind::Csv => self.begin_csv_import(path.clone()),
                    Kind::Figure => {
                        if activate_document && self.layout.center.is_none() {
                            self.show_plot_pane();
                        }
                        self.submit(Job::OpenFigureOrSelect(path.clone()));
                        if self.busy {
                            self.document_save.figure_open_started(activate_document);
                        }
                    }
                    Kind::Image | Kind::PagedImage => {
                        self.submit(if matches!(kind, Kind::PagedImage) {
                            Job::OpenPagedImage(path.clone(), 0)
                        } else {
                            Job::OpenImage(path.clone())
                        });
                        if !activate_document {
                            self.document_save.suppress_preview_activation();
                        }
                    }
                    Kind::Eps => {
                        self.begin_eps_preview(path.clone());
                        if !activate_document {
                            self.document_save.suppress_preview_activation();
                        }
                    }
                    Kind::Model => {
                        if !self.toolkits.request_open(path.clone()) {
                            self.status = self.toolkits.message.clone();
                            self.console.push(format!(
                                "Open {}: {}",
                                crate::path_label::display(&path),
                                self.status
                            ));
                            return;
                        }
                        if !activate_document {
                            self.document_save.suppress_model_activation();
                        }
                    }
                }
                path
            }
        };
        // Preflight already canonicalized the path; don't repeat filesystem I/O.
        self.recent.retain(|existing| existing != &path);
        self.recent.insert(0, path);
        self.recent.truncate(20);
        self.recovery_dirty = true;
        self.advance_model_read();
    }
}

#[cfg(test)]
#[path = "figure_reopen_ui_tests.rs"]
mod reopen_tests;

// Root picker gestures are separate from already-admitted native Editor Open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PickerKind {
    Supported,
    WorkspaceCurrentFolder,
    WorkspaceDefaultDirectory,
    Figure,
    FigurePaneSelect,
    FigurePaneCopy,
    Image,
}
impl PickerKind {
    fn filters(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Self::Supported => &[(
                "Supported files",
                &[
                    "mr", "m", "txt", "mat", "csv", "mrfig", "mfig", "fig", "mrsim", "slx", "png",
                    "jpg", "jpeg", "bmp", "tif", "tiff", "svg", "pdf", "eps",
                ],
            )],
            Self::WorkspaceCurrentFolder | Self::WorkspaceDefaultDirectory => {
                &[("MAT workspace", &["mat"])]
            }
            Self::Figure | Self::FigurePaneSelect | Self::FigurePaneCopy => &[
                ("Editable MatRust figure", &["mrfig", "mfig"]),
                ("MATLAB figure (supported 2-D objects)", &["fig"]),
            ],
            Self::Image => &[(
                "Plot image",
                &[
                    "png", "jpg", "jpeg", "bmp", "tif", "tiff", "svg", "pdf", "eps",
                ],
            )],
        }
    }
    fn starts_in_current_folder(self) -> bool {
        !matches!(
            self,
            Self::WorkspaceDefaultDirectory | Self::FigurePaneSelect | Self::FigurePaneCopy
        )
    }
}
#[cfg(test)]
type PickerCallback = fn(&mut Workspace, PickerKind) -> Option<Vec<PathBuf>>;

#[derive(Default)]
pub(super) struct PickerState {
    action: Option<PickerAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
    #[cfg(test)]
    pub(super) picker: Option<PickerCallback>,
    #[cfg(test)]
    pub(super) calls: Vec<PickerKind>,
}
struct PickerAction {
    kind: PickerKind,
    stamp: (egui::ViewportId, u64),
    generation: u64,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    folder: PathBuf,
}
impl Workspace {
    pub(super) fn root_open_context(&self, ctx: &egui::Context) -> bool {
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
    fn valid_root_open(&self, ctx: &egui::Context, action: &PickerAction) -> bool {
        self.root_open_context(ctx)
            && !egui::Popup::is_any_open(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.root_open.generation
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|m| m.focused())
            && action.folder == self.folder
    }
    pub(super) fn queue_root_open(&mut self, ctx: &egui::Context, kind: PickerKind) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        // A menu may still be unwinding; final dispatch requires it closed.
        if !self.root_open_context(ctx)
            || self.root_open.action.is_some()
            || self.root_open.finished == Some(stamp)
        {
            return;
        }
        self.root_open.action = Some(PickerAction {
            kind,
            stamp,
            generation: self.root_open.generation,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|m| m.focused()),
            folder: self.folder.clone(),
        });
    }
    pub(super) fn cancel_root_open(&mut self) {
        self.root_open.generation = self.root_open.generation.wrapping_add(1);
        if let Some(action) = self.root_open.action.take() {
            self.root_open.finished = Some(action.stamp);
        }
    }
    pub(super) fn observe_root_open(&mut self, ctx: &egui::Context) {
        if self
            .root_open
            .action
            .as_ref()
            .is_some_and(|a| !self.valid_root_open(ctx, a))
        {
            self.cancel_root_open();
        }
    }
    pub(super) fn root_open_shortcut(&mut self, ctx: &egui::Context) {
        if !self.root_open_context(ctx)
            || egui::Popup::is_any_open(ctx)
            || self.embedded_editor_owns_focus(ctx)
        {
            return;
        }
        let pressed = ctx.input_mut(|i| {
            i.events.iter().any(|event| {
                matches!(event, egui::Event::Key {
                key: egui::Key::O, pressed: true, repeat: false, modifiers, ..
            } if modifiers.ctrl && !modifiers.shift && !modifiers.alt && !modifiers.mac_cmd)
            }) && i.consume_key(egui::Modifiers::CTRL, egui::Key::O)
        });
        if pressed {
            self.queue_root_open(ctx, PickerKind::Supported);
        }
    }
    pub(super) fn finish_root_open(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.root_open.action.take() else {
            return;
        };
        self.root_open.finished = Some(action.stamp);
        if !self.valid_root_open(ctx, &action) {
            return;
        }
        if let Some(paths) = self.pick_open_paths(action.kind)
            && self.valid_root_open(ctx, &action)
        {
            if matches!(
                action.kind,
                PickerKind::FigurePaneSelect | PickerKind::FigurePaneCopy
            ) {
                // Preserve the pane's single selection and distinct disk-copy job.
                if paths.len() != 1 {
                    self.status = "Select exactly one figure file".into();
                    return;
                }
                let path = paths.into_iter().next().unwrap();
                self.submit(if action.kind == PickerKind::FigurePaneCopy {
                    Job::OpenFigure(path)
                } else {
                    Job::OpenFigureOrSelect(path)
                });
            } else {
                if action.kind == PickerKind::Figure && self.layout.center.is_none() {
                    self.show_plot_pane();
                }
                self.queue_open_paths(paths);
            }
        }
    }
    // Also used by native Editor after its existing final-pass host admission.
    // Do not introduce ROOT-only admission into this low-level chooser.
    pub(super) fn pick_open_paths(&mut self, kind: PickerKind) -> Option<Vec<PathBuf>> {
        #[cfg(test)]
        if let Some(picker) = self.root_open.picker {
            assert!(self.root_open.calls.len() < 128);
            self.root_open.calls.push(kind);
            return picker(self, kind);
        }
        let mut dialog = rfd::FileDialog::new();
        if kind.starts_in_current_folder() {
            dialog = dialog.set_directory(&self.folder);
        }
        for (name, extensions) in kind.filters() {
            dialog = dialog.add_filter(*name, extensions);
        }
        if matches!(
            kind,
            PickerKind::FigurePaneSelect | PickerKind::FigurePaneCopy
        ) {
            dialog.pick_file().map(|path| vec![path])
        } else {
            dialog.pick_files()
        }
    }
}
