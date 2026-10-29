//! One cell slot, one selected immutable workspace binding, one worker job.
use super::cell_replace_job::{Finished, Request};
use super::variable_edit_job::Token;
use super::*;
use matrust::variable_edit::cell_replace::{Binding, CellTarget};

enum Intent {
    Select(Binding),
    Submit(Request),
    Cancel,
}
#[derive(Default)]
pub(super) struct State {
    target: Option<CellTarget>,
    source: Option<Binding>,
    name: String,
    revision: Option<Token>,
    editor: Token,
    pending: bool,
    notice: Option<(String, bool)>,
    intent: Option<(u64, Intent)>,
    action_frame: Option<u64>,
}
impl State {
    pub(super) fn open(&mut self, target: CellTarget, revision: Option<Token>) {
        *self = Self {
            target: Some(target),
            revision,
            ..Default::default()
        };
    }
    fn invalidate(&mut self, notice: Option<(String, bool)>) {
        *self = Self {
            notice,
            ..Default::default()
        };
    }
    fn admit(&mut self, frame: u64, intent: Intent) {
        if self.action_frame != Some(frame)
            && (self.intent.is_none() || matches!(intent, Intent::Cancel))
        {
            self.intent = Some((frame, intent));
        }
    }
}

impl Workspace {
    pub(super) fn receive_cell_replacement(&mut self, finished: Option<Finished>) {
        let state = &mut self.typed_inspector.replacement;
        let Some(finished) = finished else {
            if state.target.is_some() {
                state.invalidate(Some((
                    "Workspace operation completed; select the replacement values again.".into(),
                    false,
                )));
            }
            return;
        };
        let matches = state.pending
            && state.editor.same(&finished.editor)
            && self.editing_variable.as_deref() == Some(finished.target.binding().name())
            && state.target.as_ref().is_some_and(|target| {
                target.path() == finished.target.path()
                    && target.binding().name() == finished.target.binding().name()
                    && target
                        .binding()
                        .expected()
                        .same_snapshot(finished.target.binding().expected())
            });
        if !matches {
            return;
        }
        match finished.result {
            Ok(root) => {
                state.invalidate(Some((
                    "Cell contents replaced; returned to the root value.".into(),
                    false,
                )));
                self.typed_inspector
                    .accept_edit(finished.target.binding().expected(), &root);
                self.typed_inspector.reset_navigation();
                self.cell_error = None;
            }
            Err(error) => state.invalidate(Some((error, true))),
        }
    }
    pub(super) fn cell_replacement_ui(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let frame = ui.ctx().cumulative_frame_nr();
        let state = &mut self.typed_inspector.replacement;
        if let Some(target) = &state.target {
            let valid = self.editing_variable.as_deref() == Some(target.binding().name())
                && self
                    .variables
                    .get(target.binding().name())
                    .is_some_and(|root| root.same_snapshot(target.binding().expected()))
                && self.typed_inspector.path == target.path()[..target.path().len() - 1]
                && state
                    .revision
                    .as_ref()
                    .zip(self.numeric_editor.revision().as_ref())
                    .is_some_and(|(a, b)| a.same(b));
            if !valid && !state.pending {
                state.invalidate(Some((
                    "Target or workspace changed; select replacement values again.".into(),
                    false,
                )));
            }
        }
        if state.intent.as_ref().is_some_and(|(old, _)| *old != frame) {
            state.intent = None;
        }
        if let Some((notice, error)) = &state.notice {
            if *error {
                ui.colored_label(tokens::palette(ui.ctx()).error, notice);
            } else {
                ui.label(notice);
            }
        }
        if let Some(target) = state.target.clone() {
            ui.group(|ui| {
                let mut path=target.binding().name().to_owned();
                for index in target.path() {use std::fmt::Write;let _=write!(&mut path,"{{{}}}",index+1);}
                ui.strong(format!("Replace contents of {path}"));
                ui.small("Select an existing workspace variable. Expressions and cell resizing are not supported.");
                ui.add_enabled_ui(enabled && !state.pending,|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Value name");
                        if ui.add(egui::TextEdit::singleline(&mut state.name).hint_text("Workspace variable name").char_limit(63).desired_width(180.)).changed() {
                            state.source=None;
                        }
                        let value=self.variables.get(&state.name);
                        if ui.add_enabled(value.is_some(),egui::Button::new("Select value")).clicked()
                            && let Some(value)=value
                        {
                            match Binding::new(state.name.clone(),value.clone()) {
                                Ok(binding)=>state.admit(frame,Intent::Select(binding)),
                                Err(error)=>state.notice=Some((error,true)),
                            }
                        }
                    });
                    if let Some(source)=&state.source {
                        let a=source.expected();
                        ui.label(format!("Selected {}: {} x {} | {}",source.name(),a.rows(),a.cols(),workspace_type_label(a)));
                    }
                    let source_current=state.source.as_ref().is_some_and(|source|self.variables.get(source.name())
                        .is_some_and(|value|value.same_snapshot(source.expected())));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(source_current && state.revision.is_some(),egui::Button::new("Replace contents")).clicked() {
                            let request=Request {revision:state.revision.clone().unwrap(),editor:state.editor.clone(),
                                target:target.clone(),source:state.source.clone().unwrap()};
                            state.admit(frame,Intent::Submit(request));
                        }
                        if ui.button("Cancel replacement").clicked() {state.admit(frame,Intent::Cancel);}
                    });
                });
                if state.pending {
                    ui.label("Replacing cell contents…");
                    if ui.button("Stop replacement").clicked() {self.worker.cancel.store(true,Ordering::Relaxed);}
                } else if ui.input(|input|input.key_pressed(egui::Key::Escape)) {
                    state.admit(frame,Intent::Cancel);
                }
            });
        }
        let intent = if !ui.ctx().will_discard() && state.action_frame != Some(frame) {
            state
                .intent
                .take()
                .filter(|(at, _)| *at == frame)
                .map(|(_, intent)| intent)
        } else {
            None
        };
        if let Some(intent) = intent {
            state.action_frame = Some(frame);
            match intent {
                Intent::Cancel => state.invalidate(Some((
                    "Replacement cancelled; cell contents unchanged.".into(),
                    false,
                ))),
                Intent::Select(source) if enabled => {
                    state.source = Some(source);
                    state.notice = None;
                }
                Intent::Submit(request) if enabled => {
                    self.submit(Job::ReplaceCell(Box::new(request)));
                    let state = &mut self.typed_inspector.replacement;
                    state.pending = self.busy;
                    state.notice = if self.busy {
                        None
                    } else {
                        Some((self.status.clone(), true))
                    };
                }
                _ => {}
            }
        }
    }
}
