//! App Designer window (Engine 9b3).
//! Edits `matrust::engine::app_designer_model::AppModel` directly as the single
//! source of truth — no second model copy.
#[path = "app_designer/canvas.rs"]
mod canvas;
#[path = "app_designer/inspector.rs"]
mod inspector;

use super::AppKind;
use crate::{theme::ThemedScrollArea, tokens, toolkits::Action};
use eframe::egui::{self, RichText, Stroke};
use matrust::engine::app_designer_model::{
    self, AppCallback, AppComponent, AppComponentKind, AppDesignerLaunch, AppGridTrack,
    AppLoadResult, AppModel, AppProblem, AppValue, GridSpanSpec,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) type OpenSlot = Arc<Mutex<Option<AppDesignerLaunch>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignerView {
    Design,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Figure,
    Component(String),
}

#[derive(Clone, Debug)]
pub struct ReadOnlyState {
    pub code: String,
    pub reason: String,
}

/// Draft buffers for the currently selected item in the inspector.
#[derive(Default)]
pub(super) struct InspectorDrafts {
    pub for_target: Option<String>,
    pub class_name: String,
    pub name: String,
    pub row: String,
    pub col: String,
    pub text: String,
    pub title: String,
    pub value_text: String,
    pub value_num: String,
    pub value_lines: String,
    pub items: String,
    pub limits_min: String,
    pub limits_max: String,
    pub column_name: String,
    pub cb_rename_target: Option<String>,
    pub cb_rename_draft: String,
}

pub struct AppDesigner {
    pub model: AppModel,
    pub view: DesignerView,
    pub selection: Selection,
    pub placing: Option<AppComponentKind>,
    pub dragging: Option<String>,
    pub file_path: Option<PathBuf>,
    pub pending_save_path: Option<PathBuf>,
    pub save_path_prompt: Option<String>,
    pub(super) focus_save_path: bool,
    pub dirty: bool,
    /// Bumped on every model edit; a finished save clears `dirty` only if no
    /// edit happened after its code was generated.
    pub(super) edit_generation: u64,
    pub(super) save_generation: Option<u64>,
    pub close_confirm: bool,
    pub read_only: Option<ReadOnlyState>,
    pub saving: bool,
    pub save_as_requested: bool,
    pub run_after_save: bool,
    pub close_after_save: bool,
    pub message: String,
    pub last_error: Option<String>,
    pub focus_callback: Option<String>,
    pub(super) drafts: InspectorDrafts,
}

impl Default for AppDesigner {
    fn default() -> Self {
        Self::new_app()
    }
}

impl AppDesigner {
    pub fn new_app() -> Self {
        let model = AppModel::new("app1");
        let mut s = Self {
            model,
            view: DesignerView::Design,
            selection: Selection::Figure,
            placing: None,
            dragging: None,
            file_path: None,
            pending_save_path: None,
            save_path_prompt: None,
            focus_save_path: false,
            dirty: false,
            edit_generation: 0,
            save_generation: None,
            close_confirm: false,
            read_only: None,
            saving: false,
            save_as_requested: false,
            run_after_save: false,
            close_after_save: false,
            message: String::new(),
            last_error: None,
            focus_callback: None,
            drafts: InspectorDrafts::default(),
        };
        s.sync_drafts(true);
        s
    }

    pub fn set_notice(&mut self, msg: impl Into<String>) {
        self.last_error = None;
        self.message = msg.into();
    }

    pub fn set_error(&mut self, err: impl Into<String>) {
        self.last_error = Some(err.into());
        self.message.clear();
    }

    pub fn clear_status(&mut self) {
        self.last_error = None;
        self.message.clear();
    }

    pub fn open_save_path_fallback(&mut self, default_path: PathBuf, reason: String) {
        self.save_path_prompt = Some(default_path.to_string_lossy().into_owned());
        self.focus_save_path = true;
        let action_word = if self.run_after_save {
            "save and run"
        } else if self.close_after_save {
            "save and close"
        } else {
            "save"
        };
        self.set_error(format!(
            "Could not open a save dialog: {reason}. Enter a .m path above and press Enter to {action_word}."
        ));
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty && self.read_only.is_none()
    }

    #[cfg(test)]
    pub fn is_read_only(&self) -> bool {
        self.read_only.is_some()
    }

    pub fn problems(&self) -> Vec<AppProblem> {
        if self.read_only.is_some() {
            return Vec::new();
        }
        self.model.validate()
    }

    pub fn reset_new(&mut self) {
        *self = Self::new_app();
    }

    pub fn load_launch(&mut self, launch: AppDesignerLaunch) {
        match launch {
            AppDesignerLaunch::New => {
                self.reset_new();
                self.message = "New app: app1".into();
            }
            AppDesignerLaunch::OpenFile { path, result } => {
                self.load_file_result(path, result);
            }
        }
    }

    pub fn load_file_result(&mut self, path: PathBuf, result: AppLoadResult) {
        self.placing = None;
        self.dragging = None;
        self.close_confirm = false;
        self.pending_save_path = None;
        self.save_path_prompt = None;
        self.focus_save_path = false;
        self.run_after_save = false;
        self.close_after_save = false;
        self.last_error = None;
        self.focus_callback = None;
        // A save still in flight belongs to the previous model.
        self.save_generation = None;
        match result {
            AppLoadResult::Editable(model) => {
                self.model = model;
                self.file_path = Some(path.clone());
                self.read_only = None;
                self.view = DesignerView::Design;
                self.selection = Selection::Figure;
                self.dirty = false;
                self.set_notice(format!("Opened {}", crate::path_label::display(&path)));
                self.sync_drafts(true);
            }
            AppLoadResult::ReadOnlyCode { code, reason } => {
                self.file_path = Some(path.clone());
                self.read_only = Some(ReadOnlyState {
                    code,
                    reason: reason.clone(),
                });
                self.view = DesignerView::Code;
                self.selection = Selection::Figure;
                self.dirty = false;
                self.set_notice(reason);
            }
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
        self.edit_generation = self.edit_generation.wrapping_add(1);
    }

    pub fn on_saved(&mut self, path: PathBuf) {
        self.file_path = Some(path.clone());
        self.pending_save_path = None;
        self.save_path_prompt = None;
        self.focus_save_path = false;
        self.saving = false;
        let up_to_date = self.save_generation.take() == Some(self.edit_generation);
        if up_to_date {
            self.dirty = false;
            self.set_notice(format!("Saved {}", crate::path_label::display(&path)));
        } else {
            self.dirty = true;
            self.set_notice(format!(
                "Saved {}; newer edits not saved yet",
                crate::path_label::display(&path)
            ));
        }
        self.sync_drafts(true);
    }

    pub fn on_save_error(&mut self, err: String) {
        self.saving = false;
        self.run_after_save = false;
        self.close_after_save = false;
        self.set_error(err);
    }

    pub fn sync_drafts(&mut self, force: bool) {
        let target_key = match &self.selection {
            Selection::Figure => "__figure__".to_string(),
            Selection::Component(name) => name.clone(),
        };
        if !force && self.drafts.for_target.as_deref() == Some(target_key.as_str()) {
            return;
        }
        self.drafts.for_target = Some(target_key);
        self.drafts.class_name = self.model.class_name.clone();

        if let Selection::Component(ref name) = self.selection
            && let Some(comp) = self.model.component(name)
        {
            self.drafts.name = comp.name.clone();
            self.drafts.row = comp.grid_row.map(format_span_input).unwrap_or_default();
            self.drafts.col = comp.grid_column.map(format_span_input).unwrap_or_default();
            self.drafts.text = comp.props.text.clone().unwrap_or_default();
            self.drafts.title = comp.props.title.clone().unwrap_or_default();
            self.drafts.items = comp
                .props
                .items
                .as_ref()
                .map(|v| v.join(", "))
                .unwrap_or_default();
            self.drafts.column_name = comp
                .props
                .column_name
                .as_ref()
                .map(|v| v.join(", "))
                .unwrap_or_default();
            if let Some(lim) = comp.props.limits {
                self.drafts.limits_min = format!("{}", lim[0]);
                self.drafts.limits_max = format!("{}", lim[1]);
            } else {
                self.drafts.limits_min = "0".into();
                self.drafts.limits_max = "100".into();
            }
            match comp.props.value.as_ref() {
                Some(AppValue::Number(n)) => {
                    self.drafts.value_num = format!("{n}");
                    self.drafts.value_text.clear();
                    self.drafts.value_lines.clear();
                }
                Some(AppValue::Text(s)) => {
                    self.drafts.value_text = s.clone();
                    self.drafts.value_num.clear();
                    self.drafts.value_lines.clear();
                }
                Some(AppValue::Lines(lines)) => {
                    self.drafts.value_lines = lines.join("\n");
                    self.drafts.value_text.clear();
                    self.drafts.value_num.clear();
                }
                _ => {
                    self.drafts.value_num = "0".into();
                    self.drafts.value_text.clear();
                    self.drafts.value_lines.clear();
                }
            }
        }
    }

    pub(super) fn place_component_in_cell(
        &mut self,
        kind: AppComponentKind,
        row: usize,
        col: usize,
    ) {
        if self.read_only.is_some() {
            return;
        }
        let name = self.model.next_component_name(kind);
        let mut comp = AppComponent::new(name.clone(), kind).in_grid(row, col);
        apply_default_props(&mut comp);
        match self.model.add_component(comp) {
            Ok(()) => {
                self.selection = Selection::Component(name.clone());
                self.placing = None;
                self.mark_dirty();
                self.set_notice(format!(
                    "Placed {} '{}' at ({row}, {col})",
                    kind.display_name(),
                    name
                ));
                self.sync_drafts(true);
            }
            Err(err) => {
                self.set_error(err);
            }
        }
    }

    pub(super) fn delete_selected_component(&mut self) {
        if self.read_only.is_some() {
            return;
        }
        let Selection::Component(name) = self.selection.clone() else {
            return;
        };
        match self.model.remove_component(&name) {
            Ok(_) => {
                self.selection = Selection::Figure;
                self.mark_dirty();
                self.set_notice(format!("Removed component '{name}'"));
                self.sync_drafts(true);
            }
            Err(err) => {
                self.set_error(err);
            }
        }
    }

    pub(super) fn add_callback_for(&mut self, component: &str, event: &str) {
        if self.read_only.is_some() {
            return;
        }
        let method_name = self.model.default_callback_method_name(component, event);
        let cb = AppCallback {
            component: component.to_string(),
            event: event.to_string(),
            method_name: method_name.clone(),
            body: String::new(),
        };
        match self.model.add_callback(cb) {
            Ok(()) => {
                self.mark_dirty();
                self.set_notice(format!("Added callback {method_name}"));
            }
            Err(err) => {
                self.set_error(err);
            }
        }
    }

    fn trigger_save(
        &mut self,
        save_as: bool,
        run_after: bool,
        close_after: bool,
        action: &mut Option<Action>,
    ) {
        // A typed fallback path only applies to this save; a refused save must not keep it.
        let pending = self.pending_save_path.take();
        if self.read_only.is_some() {
            self.set_error("Cannot save a read-only file.");
            return;
        }
        let problems = self.model.validate();
        if !problems.is_empty() {
            let joined = problems
                .iter()
                .map(|p| p.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let verb = if run_after {
                "Cannot run"
            } else {
                "Cannot save"
            };
            self.set_error(format!("{verb}: {joined}"));
            return;
        }
        match self.model.generate_m() {
            Ok(code) => {
                self.clear_status();
                self.pending_save_path = pending;
                self.save_as_requested = save_as;
                self.run_after_save = run_after;
                self.close_after_save = close_after;
                *action = Some(Action::SaveAppScript(AppKind::AppDesigner, code));
            }
            Err(err) => {
                self.set_error(err);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool, action: &mut Option<Action>) {
        self.sync_drafts(false);
        let ctx = ui.ctx().clone();
        let p = tokens::palette(&ctx);
        let read_only = self.read_only.is_some();

        // Keyboard shortcuts: Esc cancels placing; Ctrl+S saves; Delete removes selected component in Design View.
        if self.placing.is_some()
            && self.save_path_prompt.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.placing = None;
        }
        if !read_only && !busy && !self.saving {
            let save_shortcut = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S));
            if save_shortcut {
                self.trigger_save(false, false, false, action);
            }
            let delete_pressed = self.view == DesignerView::Design
                && !ctx.egui_wants_keyboard_input()
                && ctx.input(|i| i.key_pressed(egui::Key::Delete));
            if delete_pressed {
                self.delete_selected_component();
            }
        }

        // Top bar: class name, view switch, Save / Run.
        egui::Panel::top("app_designer_toolbar")
            .exact_size(tokens::ACTION_ROW + tokens::XS)
            .frame(
                egui::Frame::new()
                    .fill(p.surface_sunken)
                    .inner_margin(egui::Margin::symmetric(tokens::S as i8, tokens::XS as i8))
                    .stroke(Stroke::new(1.0, p.hairline)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = tokens::S;
                    let title = if read_only {
                        self.file_path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Read-only".into())
                    } else if self.dirty {
                        format!("{} *", self.model.class_name)
                    } else {
                        self.model.class_name.clone()
                    };
                    ui.label(RichText::new(title).strong().color(p.text));
                    ui.separator();

                    if tokens::tab(ui, self.view == DesignerView::Design, "Design View").clicked()
                        && !read_only
                    {
                        self.view = DesignerView::Design;
                    }
                    if tokens::tab(ui, self.view == DesignerView::Code, "Code View").clicked() {
                        self.view = DesignerView::Code;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_act = !read_only && !busy && !self.saving;
                        let run_btn = ui.add_enabled(
                            can_act,
                            egui::Button::new(RichText::new("Run").color(if can_act {
                                p.on_accent
                            } else {
                                p.text_muted
                            }))
                            .fill(if can_act { p.accent_fill } else { p.surface })
                            .corner_radius(tokens::CONTROL_RADIUS)
                            .min_size(egui::vec2(56.0, tokens::CONTROL)),
                        );
                        if run_btn.clicked() {
                            self.trigger_save(false, true, false, action);
                        }
                        if ui
                            .add_enabled(
                                can_act,
                                egui::Button::new("Save As")
                                    .corner_radius(tokens::CONTROL_RADIUS)
                                    .min_size(egui::vec2(64.0, tokens::CONTROL)),
                            )
                            .clicked()
                        {
                            self.trigger_save(true, false, false, action);
                        }
                        if ui
                            .add_enabled(
                                can_act,
                                egui::Button::new("Save")
                                    .corner_radius(tokens::CONTROL_RADIUS)
                                    .min_size(egui::vec2(56.0, tokens::CONTROL)),
                            )
                            .clicked()
                        {
                            self.trigger_save(false, false, false, action);
                        }
                        if ui
                            .add_enabled(
                                !busy && !self.saving,
                                egui::Button::new("New")
                                    .corner_radius(tokens::CONTROL_RADIUS)
                                    .min_size(egui::vec2(48.0, tokens::CONTROL)),
                            )
                            .clicked()
                        {
                            self.reset_new();
                        }
                    });
                });
            });

        // Unsaved changes close confirmation banner.
        if self.close_confirm {
            egui::Panel::top("app_designer_close_confirm")
                .frame(
                    egui::Frame::new()
                        .fill(p.surface_sunken)
                        .stroke(Stroke::new(1.0, p.warning))
                        .inner_margin(egui::Margin::symmetric(tokens::S as i8, tokens::S as i8)),
                )
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Unsaved changes in {}. Save before closing?",
                                self.model.class_name
                            ))
                            .strong()
                            .color(p.text),
                        );
                        if ui.button("Save & Close").clicked() {
                            self.close_confirm = false;
                            self.trigger_save(false, false, true, action);
                        }
                        if ui.button("Discard & Close").clicked() {
                            self.close_confirm = false;
                            self.dirty = false;
                            *action = Some(Action::CloseAppDesigner);
                        }
                        if ui.button("Cancel").clicked() {
                            self.close_confirm = false;
                        }
                    });
                });
        }

        // Inline Save Path fallback bar (when native file dialog cannot open).
        if self.save_path_prompt.is_some() {
            let mut submit_path: Option<String> = None;
            let mut cancel_prompt = false;
            let run_after = self.run_after_save;
            let close_after = self.close_after_save;
            let focus_now = std::mem::take(&mut self.focus_save_path);
            let save_label = if run_after {
                "Save & Run"
            } else if close_after {
                "Save & Close"
            } else {
                "Save Path"
            };
            egui::Panel::top("app_designer_save_path_bar")
                .frame(
                    egui::Frame::new()
                        .fill(p.surface_sunken)
                        .stroke(Stroke::new(1.0, p.accent))
                        .inner_margin(egui::Margin::symmetric(tokens::S as i8, tokens::XS as i8)),
                )
                .show(ui, |ui| {
                    if let Some(ref mut draft) = self.save_path_prompt {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = tokens::S;
                            ui.label(RichText::new("Save path (.m):").color(p.text));
                            let btn_reserve = if run_after || close_after {
                                176.0
                            } else {
                                156.0
                            };
                            let field_w = (ui.available_width() - btn_reserve).max(120.0);
                            let resp = ui.add_sized(
                                [field_w, tokens::CONTROL],
                                tokens::input(egui::TextEdit::singleline(draft))
                                    .id(egui::Id::new("app_designer_save_path")),
                            );
                            if focus_now {
                                resp.request_focus();
                            }
                            let enter = (resp.lost_focus() || resp.has_focus())
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if enter || ui.button(save_label).clicked() {
                                submit_path = Some(draft.clone());
                            }
                            if ui.button("Cancel").clicked()
                                || (resp.has_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Escape)))
                            {
                                cancel_prompt = true;
                            }
                        });
                    }
                });
            if cancel_prompt {
                self.save_path_prompt = None;
                self.pending_save_path = None;
                self.run_after_save = false;
                self.close_after_save = false;
                self.clear_status();
            } else if let Some(raw) = submit_path {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    self.set_error("Enter a file path ending in .m.");
                } else {
                    let mut path = PathBuf::from(trimmed);
                    if path.extension().is_none() {
                        path.set_extension("m");
                    }
                    self.pending_save_path = Some(path);
                    self.save_path_prompt = None;
                    self.trigger_save(false, run_after, close_after, action);
                }
            }
        }

        // Read-only banner.
        if let Some(ref ro) = self.read_only {
            egui::Panel::top("app_designer_readonly_banner")
                .frame(
                    egui::Frame::new()
                        .fill(p.surface_sunken)
                        .stroke(Stroke::new(1.0, p.warning))
                        .inner_margin(egui::Margin::symmetric(tokens::S as i8, tokens::S as i8)),
                )
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("Read-only: {}", ro.reason))
                            .strong()
                            .color(p.warning),
                    );
                });
        }

        // Bottom OUTPUT / Validation Problems strip (the ONE message place).
        // Allocated BEFORE side/central panels with a reserved height (header + at least 2 message lines)
        // and bounded ScrollArea so nothing ever overflows or draws over it.
        let problems = self.problems();
        egui::Panel::bottom("app_designer_output")
            .exact_size(104.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.surface_sunken)
                    .stroke(Stroke::new(1.0, p.pane_border))
                    .inner_margin(egui::Margin::same(tokens::S as i8)),
            )
            .show(ui, |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                let hdr = format!("OUTPUT · Problems ({})", problems.len());
                ui.label(
                    RichText::new(hdr)
                        .font(crate::panels::caption_semibold(ui))
                        .color(if problems.is_empty() {
                            p.text_muted
                        } else {
                            p.error
                        }),
                );
                ui.add_space(tokens::XS);
                let scroll_h = ui.available_height().max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("app_designer_problems_scroll")
                    .min_scrolled_height(0.0)
                    .max_height(scroll_h)
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| {
                        let inner_w = ui.available_width();
                        ui.set_max_width(inner_w);
                        if let Some(ref err) = self.last_error {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(err).size(tokens::BODY).color(p.error),
                                )
                                .wrap(),
                            );
                        } else if !self.message.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.message)
                                        .size(tokens::BODY)
                                        .color(p.text_muted),
                                )
                                .wrap(),
                            );
                        }
                        if problems.is_empty() {
                            if self.last_error.is_none() {
                                tokens::helper(ui, "No validation problems.");
                            }
                        } else {
                            for prob in &problems {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "• [{}] {}",
                                            prob.code, prob.message
                                        ))
                                        .size(tokens::BODY)
                                        .color(p.error),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    });
            });

        // Left pane (INPUTS): Component Palette + Grid Size + Component List.
        let narrow = ui.available_width() < 960.0;
        let left_w = if narrow { 196.0 } else { 276.0 };
        let right_w = 284.0;

        egui::Panel::left("app_designer_inputs")
            .exact_size(left_w)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.surface_sunken)
                    .stroke(Stroke::new(1.0, p.pane_border))
                    .inner_margin(egui::Margin::same(tokens::S as i8)),
            )
            .show(ui, |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                egui::ScrollArea::vertical()
                    .id_salt("app_designer_inputs_scroll")
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| {
                        let inner_w = ui.available_width();
                        ui.set_max_width(inner_w);
                        self.inputs_pane(ui, narrow);
                    });
            });

        // Right pane: Inspector + Callbacks (in wide layout).
        if !narrow {
            egui::Panel::right("app_designer_inspector")
                .exact_size(right_w)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(p.surface_sunken)
                        .stroke(Stroke::new(1.0, p.pane_border))
                        .inner_margin(egui::Margin::same(tokens::S as i8)),
                )
                .show(ui, |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    egui::ScrollArea::vertical()
                        .id_salt("app_designer_inspector_scroll")
                        .auto_shrink([false, false])
                        .show_themed(ui, |ui| {
                            let inner_w = ui.available_width();
                            ui.set_max_width(inner_w);
                            inspector::show(self, ui);
                        });
                });
        }

        // Centre pane (MAIN VIEW): Design Canvas or Code View.
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::same(tokens::S as i8)),
            )
            .show(ui, |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                if narrow {
                    let rows = self.model.grid.as_ref().map(|g| g.rows()).unwrap_or(2);
                    let canvas_h = (rows as f32 * 84.0 + 56.0).clamp(260.0, 420.0);
                    egui::ScrollArea::vertical()
                        .id_salt("app_designer_narrow_scroll")
                        .auto_shrink([false, false])
                        .show_themed(ui, |ui| {
                            let inner_w = ui.available_width();
                            ui.set_max_width(inner_w);
                            let canvas_size = egui::vec2(inner_w, canvas_h);
                            ui.allocate_ui(canvas_size, |ui| {
                                ui.set_min_size(canvas_size);
                                ui.set_max_size(canvas_size);
                                canvas::show(self, ui);
                            });
                            ui.add_space(tokens::S);
                            ui.separator();
                            ui.add_space(tokens::S);
                            inspector::show(self, ui);
                        });
                } else {
                    canvas::show(self, ui);
                }
            });
    }

    fn inputs_pane(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let p = tokens::palette(ui.ctx());
        let read_only = self.read_only.is_some();
        let inner_w = ui.available_width();

        ui.label(
            RichText::new("PALETTE")
                .font(crate::panels::caption_semibold(ui))
                .color(p.text_muted),
        );
        ui.add_space(tokens::XS);

        ui.add_enabled_ui(!read_only, |ui| {
            let two_cols = !narrow && inner_w >= 232.0;
            if two_cols {
                let col_w = ((inner_w - tokens::XS) * 0.5).floor().max(80.0);
                egui::Grid::new("app_designer_palette_grid")
                    .num_columns(2)
                    .min_col_width(col_w)
                    .max_col_width(col_w)
                    .spacing(egui::vec2(tokens::XS, tokens::XS))
                    .show(ui, |ui| {
                        for (idx, &kind) in AppComponentKind::ALL.iter().enumerate() {
                            let active = self.placing == Some(kind);
                            let resp = ui
                                .scope(|ui| {
                                    ui.set_max_width(col_w);
                                    ui.add(
                                        egui::Button::selectable(active, kind.display_name())
                                            .wrap()
                                            .min_size(egui::vec2(col_w, tokens::COMPACT_ROW)),
                                    )
                                })
                                .inner;
                            if resp.clicked() {
                                self.placing = if active { None } else { Some(kind) };
                                self.view = DesignerView::Design;
                            }
                            if idx % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
            } else {
                ui.spacing_mut().item_spacing.y = tokens::XS;
                for &kind in AppComponentKind::ALL {
                    let active = self.placing == Some(kind);
                    let resp = ui.add(
                        egui::Button::selectable(active, kind.display_name())
                            .wrap()
                            .min_size(egui::vec2(inner_w, tokens::COMPACT_ROW)),
                    );
                    if resp.clicked() {
                        self.placing = if active { None } else { Some(kind) };
                        self.view = DesignerView::Design;
                    }
                }
            }
        });

        ui.add_space(tokens::S);
        ui.separator();
        ui.add_space(tokens::XS);

        // Grid size controls
        if let Some(ref mut grid) = self.model.grid {
            ui.label(
                RichText::new("GRID LAYOUT")
                    .font(crate::panels::caption_semibold(ui))
                    .color(p.text_muted),
            );
            ui.add_space(tokens::XS);
            let mut changed = false;
            ui.add_enabled_ui(!read_only, |ui| {
                ui.spacing_mut().item_spacing.x = tokens::XS;
                ui.horizontal(|ui| {
                    ui.label(format!("Rows: {}", grid.rows()));
                    if ui
                        .add_enabled(grid.rows() > 1, egui::Button::new("- Row"))
                        .clicked()
                    {
                        grid.row_height.pop();
                        changed = true;
                    }
                    if ui
                        .add_enabled(grid.rows() < 12, egui::Button::new("+ Row"))
                        .clicked()
                    {
                        grid.row_height.push(AppGridTrack::Weight(1.0));
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Cols: {}", grid.cols()));
                    if ui
                        .add_enabled(grid.cols() > 1, egui::Button::new("- Col"))
                        .clicked()
                    {
                        grid.column_width.pop();
                        changed = true;
                    }
                    if ui
                        .add_enabled(grid.cols() < 12, egui::Button::new("+ Col"))
                        .clicked()
                    {
                        grid.column_width.push(AppGridTrack::Weight(1.0));
                        changed = true;
                    }
                });
            });
            if changed {
                self.mark_dirty();
                self.clear_status();
            }
            ui.add_space(tokens::S);
            ui.separator();
            ui.add_space(tokens::XS);
        }

        // Component list
        ui.label(
            RichText::new("COMPONENTS")
                .font(crate::panels::caption_semibold(ui))
                .color(p.text_muted),
        );
        ui.add_space(tokens::XS);
        let fig_sel = self.selection == Selection::Figure;
        if ui
            .add(
                egui::Button::selectable(fig_sel, format!("{} (Figure)", self.model.figure.name))
                    .wrap()
                    .min_size(egui::vec2(inner_w, tokens::ROW)),
            )
            .clicked()
        {
            self.selection = Selection::Figure;
            self.sync_drafts(true);
        }

        #[allow(clippy::type_complexity)]
        let comp_items: Vec<(String, &'static str, Option<(GridSpanSpec, GridSpanSpec)>)> = self
            .model
            .components
            .iter()
            .map(|c| {
                let cell = c.grid_row.zip(c.grid_column);
                (c.name.clone(), c.kind.display_name(), cell)
            })
            .collect();

        for (name, kind_label, cell) in comp_items {
            let selected = self.selection == Selection::Component(name.clone());
            let loc = match cell {
                Some((r, c)) => format!(" ({},{})", format_span_input(r), format_span_input(c)),
                None => String::new(),
            };
            let label = format!("{name} · {kind_label}{loc}");
            if ui
                .add(
                    egui::Button::selectable(selected, label)
                        .wrap()
                        .min_size(egui::vec2(inner_w, tokens::ROW)),
                )
                .clicked()
            {
                self.selection = Selection::Component(name);
                self.sync_drafts(true);
            }
        }

        if matches!(self.selection, Selection::Component(_)) && !read_only {
            ui.add_space(tokens::XS);
            if ui
                .add_sized(
                    [inner_w, tokens::CONTROL],
                    egui::Button::new("Delete Component"),
                )
                .clicked()
            {
                self.delete_selected_component();
            }
        }
    }
}

fn apply_default_props(comp: &mut AppComponent) {
    match comp.kind {
        AppComponentKind::Button => {
            comp.props.text = Some("Button".into());
        }
        AppComponentKind::StateButton => {
            comp.props.text = Some("Button".into());
            comp.props.value = Some(AppValue::Bool(false));
        }
        AppComponentKind::Label => {
            comp.props.text = Some("Label".into());
        }
        AppComponentKind::EditField => {
            comp.props.value = Some(AppValue::Text(String::new()));
        }
        AppComponentKind::NumericEditField => {
            comp.props.value = Some(AppValue::Number(0.0));
        }
        AppComponentKind::TextArea => {
            comp.props.value = Some(AppValue::Lines(Vec::new()));
        }
        AppComponentKind::DropDown => {
            comp.props.items = Some(vec!["Option 1".into(), "Option 2".into()]);
            comp.props.value = Some(AppValue::Text("Option 1".into()));
        }
        AppComponentKind::ListBox => {
            comp.props.items = Some(vec!["Item 1".into(), "Item 2".into()]);
            comp.props.value = Some(AppValue::Text("Item 1".into()));
        }
        AppComponentKind::CheckBox => {
            comp.props.text = Some("Check Box".into());
            comp.props.value = Some(AppValue::Bool(false));
        }
        AppComponentKind::RadioButton => {
            comp.props.text = Some("Radio Button".into());
            comp.props.value = Some(AppValue::Bool(false));
        }
        AppComponentKind::Slider | AppComponentKind::Spinner => {
            comp.props.limits = Some([0.0, 100.0]);
            comp.props.value = Some(AppValue::Number(0.0));
        }
        AppComponentKind::Switch => {
            comp.props.items = Some(vec!["Off".into(), "On".into()]);
            comp.props.value = Some(AppValue::Text("Off".into()));
        }
        AppComponentKind::Table => {
            comp.props.column_name = Some(vec!["Column 1".into(), "Column 2".into()]);
        }
        AppComponentKind::Panel => {
            comp.props.title = Some("Panel".into());
        }
        AppComponentKind::Tab => {
            comp.props.title = Some("Tab".into());
        }
        AppComponentKind::ButtonGroup => {
            comp.props.title = Some("Button Group".into());
        }
        AppComponentKind::Axes | AppComponentKind::TabGroup => {}
    }
}

pub(super) fn format_span_input(span: GridSpanSpec) -> String {
    match span {
        GridSpanSpec::Single(n) => n.to_string(),
        GridSpanSpec::Range(a, b) => format!("{a}:{b}"),
    }
}

pub(super) fn parse_span_input(text: &str) -> Result<GridSpanSpec, String> {
    let s = text.trim();
    let s = s
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(s)
        .trim();
    if let Some((a, b)) = s.split_once(':') {
        let start: usize = a
            .trim()
            .parse()
            .map_err(|_| "Use a positive integer or range like 1:2.".to_string())?;
        let end: usize = b
            .trim()
            .parse()
            .map_err(|_| "Use a positive integer or range like 1:2.".to_string())?;
        if start < 1 || end < start {
            return Err("Range must have 1 <= start <= end.".into());
        }
        return Ok(if start == end {
            GridSpanSpec::Single(start)
        } else {
            GridSpanSpec::Range(start, end)
        });
    }
    let parts: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .collect();
    match parts.as_slice() {
        [one] => {
            let n: usize = one
                .parse()
                .map_err(|_| "Use a positive integer or range like 1:2.".to_string())?;
            if n < 1 {
                return Err("Grid index must be >= 1.".into());
            }
            Ok(GridSpanSpec::Single(n))
        }
        [a, b] => {
            let start: usize = a
                .parse()
                .map_err(|_| "Use a positive integer or range like 1:2.".to_string())?;
            let end: usize = b
                .parse()
                .map_err(|_| "Use a positive integer or range like 1:2.".to_string())?;
            if start < 1 || end < start {
                return Err("Range must have 1 <= start <= end.".into());
            }
            Ok(if start == end {
                GridSpanSpec::Single(start)
            } else {
                GridSpanSpec::Range(start, end)
            })
        }
        _ => Err("Use a positive integer or range like 1:2.".into()),
    }
}

/// Sync class name to the chosen `.m` filename stem if valid, and regenerate code.
pub(crate) fn prepare_save_code(
    designer: &mut AppDesigner,
    path: &std::path::Path,
) -> Result<String, String> {
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && app_designer_model::is_valid_identifier(stem)
        && designer.model.class_name != stem
    {
        designer.model.class_name = stem.to_string();
        designer.drafts.class_name = stem.to_string();
    }
    let code = designer.model.generate_m()?;
    designer.save_generation = Some(designer.edit_generation);
    Ok(code)
}

pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .app_designer
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if workspace
        .worker
        .cancel
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    if let Some(launch) = launch {
        workspace.toolkits.apps.app_designer.load_launch(launch);
        workspace.open_app(crate::toolkits::Tool::App(AppKind::AppDesigner));
    }
}
