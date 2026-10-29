//! Desktop CSV naming decisions; the worker owns parsing and atomic publication.
use super::*;
use crate::theme::ThemedScrollArea;
use matrust::{Array, Engine, Result, files};

// One picker gesture, without copying workspace values or document text.
#[derive(Default)]
pub(super) struct PickerState {
    action: Option<PickerAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
}
struct PickerAction {
    stamp: (egui::ViewportId, u64),
    generation: u64,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    folder: PathBuf,
}

// One retained naming decision. Shared Array handles never copy payloads.
#[derive(Default)]
pub(super) struct NamingState {
    action: Option<NamingAction>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum NamingChoice {
    Submit,
    NewName,
    CancelQueue,
    Cancel,
    Stop,
}
struct NamingAction {
    stamp: (egui::ViewportId, u64),
    generation: u64,
    choice: NamingChoice,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    layer: egui::LayerId,
    folder: PathBuf,
    loading: bool,
    request: Request,
}

pub struct Request {
    pub path: PathBuf,
    pub name: String,
    /// None explicitly requires absence; Some approves replacing this snapshot only.
    pub expected: Option<Array>,
}
pub struct Import {
    path: PathBuf,
    name: String,
    loading: bool,
    error: Option<String>,
}
fn suggested_name(path: &std::path::Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("data");
    if workspace_edit::validate_name(stem).is_ok() {
        stem.into()
    } else {
        "data".into()
    }
}
fn unused_name(name: &str, variables: &BTreeMap<String, Array>) -> Option<String> {
    let base = if workspace_edit::validate_name(name).is_ok() {
        name
    } else {
        "data"
    };
    let prefix = &base[..base.len().min(57)];
    (1..=10001)
        .map(|i| format!("{prefix}_{i}"))
        .find(|n| !variables.contains_key(n))
}
pub fn perform(engine: &mut Engine, request: Request) -> Result<String> {
    let cancel = engine.cancellation();
    if cancel.load(Ordering::Relaxed) {
        return Err("CSV import cancelled".into());
    }
    workspace_edit::validate_name(&request.name)?;
    check_destination(engine, &request)?;
    if request.expected.is_none() && engine.workspace_count() >= 10000 {
        return Err(
            "Workspace contains 10000 variables; replace a variable or remove one first".into(),
        );
    }
    let data = files::read_csv_with_cancel(&request.path, &cancel)?;
    // Defensive recheck at publication; this worker exclusively owns Engine.
    check_destination(engine, &request)?;
    engine.merge_workspace(BTreeMap::from([(request.name.clone(), data.into())]))?;
    Ok(format!(
        "Imported {} as {}",
        crate::path_label::display(&request.path),
        request.name
    ))
}
fn check_destination(engine: &Engine, request: &Request) -> Result<()> {
    let current = engine.array(&request.name);
    let matches = match (&request.expected, &current) {
        (None, None) => true,
        (Some(expected), Some(actual)) => expected.same_snapshot(actual),
        _ => false,
    };
    if !matches {
        return Err("The destination variable changed. Review the name and try again.".into());
    }
    Ok(())
}
impl Workspace {
    fn csv_import_picker_context(&self, ctx: &egui::Context) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|input| input.focused && !input.viewport().close_requested())
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && !self.busy
            && !self.file_action_pending()
            && self.open_queue.is_empty()
            && !self.folder_browser.navigating()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
    }
    fn valid_csv_import_picker(&self, ctx: &egui::Context, action: &PickerAction) -> bool {
        self.csv_import_picker_context(ctx)
            && !egui::Popup::is_any_open(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.csv_import_picker.generation
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|memory| memory.focused())
            && action.folder == self.folder
    }
    pub(super) fn request_csv_import_files(&mut self, ctx: &egui::Context) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        // An originating menu can still be unwinding. Final admission requires
        // it closed; Global Search closes its popup before invoking this route.
        if !self.csv_import_picker_context(ctx)
            || self.csv_import_picker.action.is_some()
            || self.csv_import_picker.finished == Some(stamp)
        {
            return;
        }
        self.csv_import_picker.action = Some(PickerAction {
            stamp,
            generation: self.csv_import_picker.generation,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|memory| memory.focused()),
            folder: self.folder.clone(),
        });
    }
    pub(super) fn cancel_csv_import_picker(&mut self) {
        self.csv_import_picker.generation = self.csv_import_picker.generation.wrapping_add(1);
        if let Some(action) = self.csv_import_picker.action.take() {
            self.csv_import_picker.finished = Some(action.stamp);
        }
    }
    pub(super) fn observe_csv_import_picker(&mut self, ctx: &egui::Context) {
        // Retire a newer focus owner before widget painting can surrender it.
        if self
            .csv_import_picker
            .action
            .as_ref()
            .is_some_and(|action| !self.valid_csv_import_picker(ctx, action))
        {
            self.cancel_csv_import_picker();
        }
    }
    pub(super) fn finish_csv_import_picker(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.csv_import_picker.action.take() else {
            return;
        };
        // Consume before the native picker, including Cancel and later discards.
        self.csv_import_picker.finished = Some(action.stamp);
        if !self.valid_csv_import_picker(ctx, &action) {
            return;
        }
        if let Some(paths) = pick_paths(self)
            && self.valid_csv_import_picker(ctx, &action)
        {
            self.queue_open_paths(paths);
        }
    }
    pub(super) fn begin_csv_import(&mut self, path: PathBuf) {
        if self.busy || self.file_action_pending() || self.allow_close {
            return;
        }
        self.cancel_csv_naming_action();
        self.csv_import = Some(Import {
            name: suggested_name(&path),
            path,
            loading: false,
            error: None,
        });
        self.status = "Choose a name for the CSV data".into();
    }
    pub(super) fn finish_csv_import(&mut self, result: &matrust::Result<matrust::Execution>) {
        if self
            .csv_import
            .as_ref()
            .is_some_and(|import| import.loading)
        {
            self.cancel_csv_naming_action();
        }
        if let Some(import) = &mut self.csv_import
            && import.loading
        {
            if let Err(error) = result {
                import.loading = false;
                import.error = Some(error.clone());
            } else {
                self.csv_import = None;
            }
        }
    }
    pub(super) fn csv_import_ui(&mut self, ctx: &egui::Context) {
        let Some(import) = &mut self.csv_import else {
            return;
        };
        let mut close = false;
        let mut submit = false;
        let mut stop = false;
        let mut new_name = false;
        let mut cancel_queue = false;
        let shown = egui::Modal::new(egui::Id::new("csv_import")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(260., 480.));
            ui.label(crate::tokens::title(ui.ctx(), "Import CSV data"));
            ui.add(
                egui::Label::new(
                    import
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                )
                .truncate(),
            )
            .on_hover_text(crate::path_label::display(&import.path).to_string());
            ui.label("Variable name");
            ui.add_enabled(
                !self.busy,
                egui::TextEdit::singleline(&mut import.name)
                    .id(egui::Id::new("csv_import_name"))
                    .char_limit(63)
                    .desired_width(f32::INFINITY),
            );
            let validation = workspace_edit::validate_name(&import.name);
            let occupied = self.variables.contains_key(&import.name);
            if occupied {
                ui.label(
                    "A variable with this name already exists. Replace it or choose a new name.",
                );
            }
            if let Err(error) = &validation {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(error) = &import.error {
                egui::ScrollArea::vertical()
                    .id_salt("csv_import_error")
                    .max_height(64.)
                    .auto_shrink([false, true])
                    .show_themed(ui, |ui| {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    });
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if !self.open_queue.is_empty() && ui.button("Cancel queued files").clicked() {
                    cancel_queue = true;
                }
                if self.busy {
                    ui.spinner();
                    ui.label("Reading CSV data...");
                    if ui.button("Stop").clicked() {
                        stop = true;
                    }
                } else {
                    if ui
                        .add_enabled(
                            validation.is_ok(),
                            egui::Button::new(if occupied {
                                "Replace variable"
                            } else {
                                "Import"
                            }),
                        )
                        .clicked()
                    {
                        submit = true;
                    }
                    if occupied && ui.button("Use new name").clicked() {
                        new_name = true;
                        submit = false;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
            });
        });
        // Resolve Escape before any side effect, including a same-frame click.
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            submit = false;
            if self.busy {
                stop = true;
            } else {
                close = true;
            }
        }
        let choice = if stop {
            Some(NamingChoice::Stop)
        } else if close {
            Some(NamingChoice::Cancel)
        } else if new_name {
            Some(NamingChoice::NewName)
        } else if cancel_queue {
            Some(NamingChoice::CancelQueue)
        } else if submit {
            Some(NamingChoice::Submit)
        } else {
            None
        };
        if let Some(choice) = choice {
            self.retain_csv_naming_action(
                ctx,
                choice,
                shown.response.layer_id,
                shown.is_top_modal,
                shown.any_popup_open,
            );
        }
    }
    fn csv_naming_owner(&self, ctx: &egui::Context, action: &NamingAction) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.csv_naming.generation
            && ctx.input(|i| i.focused && !i.viewport().close_requested())
            && ctx.memory(|m| m.top_modal_layer() == Some(action.layer))
            && !egui::Popup::is_any_open(ctx)
            && !self.file_action_pending_without_csv_naming()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
            && !self.folder_browser.navigating()
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|m| m.focused())
            && action.folder == self.folder
            && self.busy == action.loading
            && self.csv_import.as_ref().is_some_and(|import| {
                import.path == action.request.path
                    && import.name == action.request.name
                    && import.loading == action.loading
            })
    }
    fn csv_naming_destination(&self, action: &NamingAction) -> bool {
        match (
            &action.request.expected,
            self.variables.get(&action.request.name),
        ) {
            (None, None) => true,
            (Some(before), Some(now)) => before.same_snapshot(now),
            _ => false,
        }
    }
    pub(super) fn cancel_csv_naming_action(&mut self) {
        self.csv_naming.generation = self.csv_naming.generation.wrapping_add(1);
        if let Some(action) = self.csv_naming.action.take() {
            self.csv_naming.finished = Some(action.stamp);
        }
    }
    pub(super) fn observe_csv_naming_action(&mut self, ctx: &egui::Context) {
        // Observe foreign ownership before painting can surrender its focus.
        if self.csv_naming.action.as_ref().is_some_and(|action| {
            !self.csv_naming_owner(ctx, action) || !self.csv_naming_destination(action)
        }) {
            self.cancel_csv_naming_action();
        }
    }
    fn retain_csv_naming_action(
        &mut self,
        ctx: &egui::Context,
        choice: NamingChoice,
        layer: egui::LayerId,
        top: bool,
        popup: bool,
    ) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if self.csv_naming.finished == Some(stamp) || !top || popup {
            return;
        }
        if let Some(action) = &mut self.csv_naming.action {
            // Escape/Cancel or Stop supersedes a previously retained submit.
            if matches!(choice, NamingChoice::Cancel | NamingChoice::Stop) {
                action.choice = choice;
            }
            return;
        }
        let Some(import) = &self.csv_import else {
            return;
        };
        let action = NamingAction {
            stamp,
            generation: self.csv_naming.generation,
            choice,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|m| m.focused()),
            layer,
            folder: self.folder.clone(),
            loading: import.loading,
            request: Request {
                path: import.path.clone(),
                name: import.name.clone(),
                expected: self.variables.get(&import.name).cloned(),
            },
        };
        if self.csv_naming_owner(ctx, &action) {
            self.csv_naming.action = Some(action);
        }
    }
    pub(super) fn finish_csv_naming_action(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.csv_naming.action.take() else {
            return;
        };
        // Consume before changing the dialog or dispatching, including failures.
        self.csv_naming.finished = Some(action.stamp);
        if !self.csv_naming_owner(ctx, &action) || !self.csv_naming_destination(&action) {
            return;
        }
        match action.choice {
            NamingChoice::Stop if action.loading => {
                self.cancel_csv_naming_action();
                self.open_queue.clear();
                self.worker.cancel.store(true, Ordering::Relaxed);
            }
            NamingChoice::CancelQueue => self.open_queue.clear(),
            NamingChoice::Cancel if !action.loading => {
                self.cancel_csv_naming_action();
                self.csv_import = None;
                self.status = "CSV import cancelled".into();
            }
            NamingChoice::NewName if !action.loading => {
                let import = self.csv_import.as_mut().unwrap();
                if let Some(name) = unused_name(&import.name, &self.variables) {
                    import.name = name;
                    import.error = None;
                } else {
                    import.error =
                        Some("No available variable name; enter a different name".into());
                }
            }
            NamingChoice::Submit
                if !action.loading
                    && workspace_edit::validate_name(&action.request.name).is_ok() =>
            {
                let import = self.csv_import.as_mut().unwrap();
                import.loading = true;
                import.error = None;
                self.submit(Job::ImportCsv(Box::new(action.request)));
                if !self.busy
                    && let Some(import) = &mut self.csv_import
                {
                    import.loading = false;
                    import.error = Some(self.status.clone());
                }
            }
            _ => {}
        }
    }
}

fn pick_paths(app: &mut Workspace) -> Option<Vec<PathBuf>> {
    #[cfg(test)]
    if let Some((choice, callback)) = PICKER.with(|slot| {
        slot.borrow_mut().as_mut().map(|(choice, calls, callback)| {
            *calls += 1;
            (choice.clone(), *callback)
        })
    }) {
        return if let Some(callback) = callback {
            callback(app)
        } else {
            choice
        };
    }
    rfd::FileDialog::new()
        .set_directory(&app.folder)
        .add_filter("Numeric CSV", &["csv"])
        .pick_files()
}
#[cfg(test)]
type PickerCallback = fn(&mut Workspace) -> Option<Vec<PathBuf>>;
#[cfg(test)]
type PickerStub = (Option<Vec<PathBuf>>, usize, Option<PickerCallback>);
#[cfg(test)]
thread_local! {static PICKER:std::cell::RefCell<Option<PickerStub>>=const {std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(super) struct PickerChoice;
#[cfg(test)]
impl PickerChoice {
    pub(super) fn calls(&self) -> usize {
        PICKER.with(|p| p.borrow().as_ref().unwrap().1)
    }
}
#[cfg(test)]
impl Drop for PickerChoice {
    fn drop(&mut self) {
        PICKER.with(|p| *p.borrow_mut() = None);
    }
}
#[cfg(test)]
pub(super) fn picker_choice(choice: Option<Vec<PathBuf>>) -> PickerChoice {
    PICKER.with(|p| {
        assert!(p.borrow().is_none());
        *p.borrow_mut() = Some((choice, 0, None));
    });
    PickerChoice
}

#[cfg(test)]
pub(super) fn picker_callback(
    callback: fn(&mut Workspace) -> Option<Vec<PathBuf>>,
) -> PickerChoice {
    PICKER.with(|p| {
        assert!(p.borrow().is_none());
        *p.borrow_mut() = Some((None, 0, Some(callback)));
    });
    PickerChoice
}

#[cfg(test)]
#[path = "csv_import_worker_tests.rs"]
mod tests;
