//! Named preferences only. Never restores a Session or interprets document indices.
use super::*;
use crate::theme::ThemedScrollArea;

#[derive(Clone)]
enum Action {
    Save,
    Rename(String),
    Restore(String),
    Delete(String),
    Commit {
        name: String,
        original: Option<String>,
        replace: bool,
    },
    Cancel,
}
struct Intent {
    frame: u64,
    ticket: u64,
    focus: Option<egui::Id>,
    action: Action,
}
struct Dialog {
    original: Option<String>,
    name: String,
    error: String,
    focus: bool,
}
#[derive(Default)]
pub(super) struct State {
    dialog: Option<Dialog>,
    intent: Option<Intent>,
    ticket: u64,
    finished: Option<u64>,
    modal_layer: Option<egui::LayerId>,
}

impl State {
    pub(super) fn pending(&self) -> bool {
        self.dialog.is_some() || self.intent.is_some()
    }
    pub(super) fn cancel(&mut self) {
        self.dialog = None;
        self.intent = None;
        self.modal_layer = None;
        self.ticket = self.ticket.wrapping_add(1);
    }
    fn queue(&mut self, ctx: &egui::Context, action: Action) {
        let frame = ctx.cumulative_frame_nr();
        if self.finished != Some(frame) {
            self.intent = Some(Intent {
                frame,
                ticket: self.ticket,
                focus: ctx.memory(|m| m.focused()),
                action,
            });
        }
    }
}
impl Workspace {
    pub(super) fn observe_named_layout(&mut self, ctx: &egui::Context) {
        if self.named_layout.intent.as_ref().is_some_and(|intent| {
            intent.frame != ctx.cumulative_frame_nr()
                || ctx
                    .memory(|m| m.focused())
                    .is_some_and(|focus| Some(focus) != intent.focus)
        }) {
            self.named_layout.intent = None;
        }
    }
    fn named_layout_available(&self, ctx: &egui::Context) -> bool {
        !self.busy
            && !self.file_action_pending_without_named_layout()
            && !self.recovering_session
            && !self.pending_quit
            && !self.allow_close
            && self.open_queue.is_empty()
            && !self.folder_browser.navigating()
            && ctx.viewport_id() == egui::ViewportId::ROOT
            && !ctx.input(|input| input.viewport().close_requested())
    }
    pub(super) fn named_layout_menu(&mut self, ui: &mut egui::Ui) {
        let enabled = self.named_layout_available(ui.ctx()) && self.named_layout.dialog.is_none();
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("Save layout...").clicked() {
                self.named_layout.queue(ui.ctx(), Action::Save);
                ui.close();
            }
            for (title, operation) in [
                ("Restore layout", 0),
                ("Rename layout", 1),
                ("Delete layout", 2),
            ] {
                ui.add_enabled_ui(!self.layout_presets.is_empty(), |ui| {
                    ui.menu_button(title, |ui| {
                        ui.set_max_width(280.);
                        egui::ScrollArea::vertical()
                            .max_height(320.)
                            .show_themed(ui, |ui| {
                                for entry in self.layout_presets.entries() {
                                    if ui.add(egui::Button::new(entry.name()).wrap()).clicked() {
                                        let name = entry.name().to_owned();
                                        let action = match operation {
                                            0 => Action::Restore(name),
                                            1 => Action::Rename(name),
                                            _ => Action::Delete(name),
                                        };
                                        self.named_layout.queue(ui.ctx(), action);
                                        ui.close();
                                    }
                                }
                            });
                    });
                });
            }
        });
    }
    pub(super) fn named_layout_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.named_layout.dialog else {
            return;
        };
        let mut action = None;
        let shown = egui::Modal::new(egui::Id::new("named_desktop_layout")).show(ctx, |ui| {
            ui.set_max_width(360.);
            ui.label(crate::tokens::title(
                ui.ctx(),
                if dialog.original.is_some() {
                    "Rename layout"
                } else {
                    "Save layout"
                },
            ));
            ui.label("Layout name");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut dialog.name)
                    .id(egui::Id::new("named_layout_name"))
                    .char_limit(64)
                    .desired_width(300.),
            );
            if dialog.focus {
                edit.request_focus();
                dialog.focus = false;
            }
            let name = dialog.name.trim();
            let valid =
                !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control);
            let duplicate = self.layout_presets.get(name).is_some();
            let replace = dialog.original.is_none() && duplicate;
            let rename_conflict =
                dialog.original.as_deref().is_some_and(|old| old != name) && duplicate;
            if rename_conflict {
                ui.label("Another layout already uses that name.");
            }
            if !dialog.error.is_empty() {
                ui.colored_label(tokens::palette(ui.ctx()).error, &dialog.error);
            }
            ui.weak("Saves pane preferences, not documents, data or figure assignments.");
            if self.recovery.is_none() || self.recovery_blocked {
                ui.weak("Recovery is unavailable; this layout is kept in this window only.");
            }
            ui.horizontal(|ui| {
                let label = if dialog.original.is_some() {
                    "Rename"
                } else if replace {
                    "Replace layout"
                } else {
                    "Save layout"
                };
                if ui
                    .add_enabled(valid && !rename_conflict, egui::Button::new(label))
                    .clicked()
                {
                    action = Some(Action::Commit {
                        name: dialog.name.clone(),
                        original: dialog.original.clone(),
                        replace,
                    });
                }
                if ui.button("Cancel").clicked() {
                    action = Some(Action::Cancel);
                }
            });
            if valid
                && !rename_conflict
                && (edit.has_focus() || edit.lost_focus())
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none())
            {
                action = Some(Action::Commit {
                    name: dialog.name.clone(),
                    original: dialog.original.clone(),
                    replace,
                });
            }
        });
        self.named_layout.modal_layer = Some(shown.response.layer_id);
        if shown.should_close() {
            action = Some(Action::Cancel);
        }
        if shown.is_top_modal
            && let Some(action) = action
        {
            self.named_layout.queue(ctx, action);
        }
    }
    pub(super) fn finish_named_layout(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(intent) = self.named_layout.intent.take() else {
            return;
        };
        let frame = ctx.cumulative_frame_nr();
        if intent.frame != frame
            || intent.ticket != self.named_layout.ticket
            || self.named_layout.finished == Some(frame)
        {
            return;
        }
        self.named_layout.finished = Some(frame);
        if matches!(intent.action, Action::Cancel) {
            self.named_layout.cancel();
            return;
        }
        let modal = ctx.memory(|m| m.top_modal_layer());
        if !self.named_layout_available(ctx)
            || egui::Popup::is_any_open(ctx)
            || modal.is_some_and(|layer| Some(layer) != self.named_layout.modal_layer)
            || ctx
                .memory(|m| m.focused())
                .is_some_and(|focus| Some(focus) != intent.focus)
        {
            return;
        }
        match intent.action {
            action @ (Action::Save | Action::Rename(_)) => {
                let original = match action {
                    Action::Rename(name) => Some(name),
                    _ => None,
                };
                if original
                    .as_ref()
                    .is_some_and(|name| self.layout_presets.get(name).is_none())
                {
                    return;
                }
                self.named_layout.ticket = self.named_layout.ticket.wrapping_add(1);
                self.named_layout.dialog = Some(Dialog {
                    name: original.clone().unwrap_or_default(),
                    original,
                    error: String::new(),
                    focus: true,
                });
            }
            Action::Restore(name) => {
                if let Some(preset) = self.layout_presets.get(&name).cloned() {
                    self.restore_named_layout(ctx, &preset);
                }
            }
            Action::Delete(name) => {
                if self.layout_presets.remove(&name) {
                    self.recovery_dirty = true;
                    self.status = format!("Deleted layout {name}");
                }
            }
            Action::Commit {
                name,
                original,
                replace,
            } => {
                let Some(dialog) = &self.named_layout.dialog else {
                    return;
                };
                if dialog.name != name || dialog.original != original {
                    return;
                }
                let result = if let Some(old) = &original {
                    self.layout_presets.rename(old, &name)
                } else {
                    let mut layout = self.layout;
                    layout.panels = self.navigation.panels();
                    self.layout_presets.save(
                        &name,
                        layout,
                        self.editor_columns_snapshot().is_some(),
                        self.compact_figure,
                        replace,
                    )
                };
                match result {
                    Ok(()) => {
                        self.named_layout.cancel();
                        self.recovery_dirty = true;
                        self.status = if self.recovery.is_some() && !self.recovery_blocked {
                            format!(
                                "Saved layout {}; session recovery will persist it",
                                name.trim()
                            )
                        } else {
                            format!(
                                "Saved layout {} in this window; recovery is unavailable",
                                name.trim()
                            )
                        };
                    }
                    Err(error) => self.named_layout.dialog.as_mut().unwrap().error = error,
                }
            }
            Action::Cancel => unreachable!(),
        }
        ctx.request_repaint();
    }
    pub(super) fn restore_named_layout(
        &mut self,
        ctx: &egui::Context,
        preset: &session::NamedLayout,
    ) {
        let revealed = self
            .layout
            .center
            .is_none_or(|center| center.editor_revealed)
            || self
                .documents
                .iter()
                .any(|doc| doc.dirty || doc.path.is_some());
        let mut layout = preset.layout();
        if let Some(center) = &mut layout.center {
            center.editor_revealed |= revealed;
            if !center.command && layout.mode != layout::PaneMode::Figure {
                center.editor_revealed = true;
            }
        }
        // Open tools are session state, not part of a named layout.
        layout.tools = self.toolkits.tab_layout();
        self.layout = layout;
        self.toolkits.deselect_tool();
        self.compact_figure = preset.compact_figure();
        self.navigation = navigation::State::from_panels(layout.panels);
        self.set_named_editor_split(preset.split_editor());
        self.layout.editor_columns = self.editor_columns_snapshot();
        self.document_save.set_tiled(layout.figure_tiling.is_some());
        self.document_save
            .set_script_document(self.documents[self.active].id);
        if self.central_command() {
            self.document_save.command_selected();
        } else {
            self.document_save
                .activate(if self.layout.mode == layout::PaneMode::Figure {
                    document_save::Pane::Figure
                } else {
                    document_save::Pane::Script
                });
        }
        self.clear_layout_panel_sizes(ctx);
        self.screenshot_theme_override = None;
        self.apply_desktop_theme(ctx);
        ctx.set_zoom_factor(f32::from(self.layout.ui_scale_percent()) / 100.);
        self.recovery_dirty = true;
        self.status = if preset.split_editor() && self.editor_columns_snapshot().is_none() {
            format!(
                "Restored layout {}; open a second script for side-by-side editors",
                preset.name()
            )
        } else {
            format!("Restored layout {}", preset.name())
        };
    }
}

#[cfg(test)]
mod intent_tests {
    use super::*;
    #[test]
    fn retained_save_or_rename_cannot_retarget_a_different_dialog_name() {
        for rename in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.layout_presets
                .save(
                    "Alpha",
                    session::DesktopLayout::fresh_desktop(),
                    false,
                    false,
                    false,
                )
                .unwrap();
            app.layout_presets
                .save(
                    "Beta",
                    session::DesktopLayout::default(),
                    false,
                    false,
                    false,
                )
                .unwrap();
            let before = app.layout_presets.clone();
            let original = rename.then(|| "Alpha".to_owned());
            app.named_layout.dialog = Some(Dialog {
                original: original.clone(),
                name: "Alpha".into(),
                error: String::new(),
                focus: false,
            });
            crate::headless::run(&ctx, egui::RawInput::default(), |_| {
                app.named_layout.queue(
                    &ctx,
                    Action::Commit {
                        name: "Alpha".into(),
                        original: original.clone(),
                        replace: !rename,
                    },
                );
                app.named_layout.dialog.as_mut().unwrap().name = "Beta".into();
                app.finish_named_layout(&ctx);
            });
            assert_eq!(app.layout_presets, before);
            assert!(app.named_layout.pending());
            assert!(!app.busy);
        }
    }
}
