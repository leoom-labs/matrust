//! Workspace selection, filtering and guarded metadata actions.
use super::workspace_edit::{Action, Change, Request};
use super::*;
use crate::theme::ThemedScrollArea;
use matrust::Array;
use std::collections::BTreeSet;
#[path = "workspace_row.rs"]
mod row;

#[derive(Default)]
pub struct Tools {
    pub filter: String,
    /// The card header shows the filter field instead of its icon.
    pub filter_open: bool,
    filter_focus: bool,
    demo_pending: bool,
    pub names: BTreeSet<String>,
    anchor: Option<String>,
    focus: Option<egui::Id>,
    cursor: Option<String>,
    scroll_offset: f32,
    viewport_height: f32,
    row_stride: f32,
    pub dialog: Option<Dialog>,
    csv_action: Option<CsvAction>,
    csv_finished: Option<(egui::ViewportId, u64)>,
    csv_generation: u64,
    #[cfg(test)]
    pub(super) csv_picker: Option<fn(&mut Workspace) -> Option<PathBuf>>,
    #[cfg(test)]
    pub(super) csv_picker_calls: usize,
}
/// Height of one Workspace table row (and of its column header row).
pub(super) fn workspace_row_height(ui: &egui::Ui) -> f32 {
    row::Layout::new(ui).height
}
/// Shared row and card geometry determines the right dock's minimum/default.
pub(super) fn minimum_dock_width(ctx: &egui::Context) -> f32 {
    row::minimum_table_width() + tokens::card(ctx).inner_margin(1).total_margin().sum().x
}
pub struct Dialog {
    targets: BTreeMap<String, Array>,
    rename: bool,
    all: bool,
    name: String,
    focus: bool,
    pub submitting: bool,
    pub error: Option<String>,
}
impl Tools {
    #[cfg(test)]
    pub(super) fn list_metrics(&self) -> (f32, f32) {
        (self.viewport_height, self.scroll_offset)
    }
    pub fn new() -> Self {
        Self {
            demo_pending: std::env::args().any(|a| a == "--workspace-delete-demo"),
            ..Default::default()
        }
    }

    fn navigate(
        &mut self,
        visible: &[&str],
        key: egui::Key,
        modifiers: egui::Modifiers,
        page: usize,
    ) -> Option<usize> {
        if visible.is_empty() {
            return None;
        }
        let current = self
            .cursor
            .as_deref()
            .and_then(|n| visible.iter().position(|v| *v == n));
        let last = visible.len() - 1;
        let index = match key {
            egui::Key::Home => 0,
            egui::Key::End => last,
            egui::Key::ArrowUp => current.unwrap_or(0).saturating_sub(1),
            egui::Key::ArrowDown => current.map_or(0, |i| (i + 1).min(last)),
            egui::Key::PageUp => current.unwrap_or(0).saturating_sub(page),
            egui::Key::PageDown => current.map_or(0, |i| i.saturating_add(page).min(last)),
            _ => return None,
        };
        let name = visible[index];
        if !modifiers.ctrl || modifiers.shift {
            self.select(name, visible, modifiers);
        }
        self.cursor = Some(name.into());
        Some(index)
    }

    pub fn select(&mut self, name: &str, visible: &[&str], modifiers: egui::Modifiers) {
        self.cursor = Some(name.into());
        if modifiers.shift
            && let Some(anchor) = self.anchor.as_deref()
            && let (Some(a), Some(b)) = (
                visible.iter().position(|n| *n == anchor),
                visible.iter().position(|n| *n == name),
            )
        {
            if !modifiers.ctrl {
                self.names.clear();
            }
            self.names
                .extend(visible[a.min(b)..=a.max(b)].iter().map(|n| n.to_string()));
        } else if modifiers.ctrl {
            if !self.names.remove(name) {
                self.names.insert(name.into());
            }
            self.anchor = Some(name.into());
        } else {
            self.names = BTreeSet::from([name.into()]);
            self.anchor = Some(name.into());
        }
    }
}
impl Workspace {
    pub(super) fn workspace_tools_demo(&mut self) {
        if self.workspace_tools.demo_pending && !self.busy && !self.variables.is_empty() {
            self.workspace_tools.demo_pending = false;
            self.workspace_tools.names = self.variables.keys().take(2).cloned().collect();
            self.selected = self.workspace_tools.names.first().cloned();
            self.begin_workspace_action(false, false);
        }
    }

    pub(super) fn workspace_selection_count(&self) -> usize {
        if self.workspace_tools.names.is_empty() {
            usize::from(
                self.selected
                    .as_ref()
                    .is_some_and(|n| self.variables.contains_key(n)),
            )
        } else {
            self.workspace_tools
                .names
                .iter()
                .filter(|n| self.variables.contains_key(*n))
                .count()
        }
    }
    pub(super) fn workspace_targets(&self, all: bool) -> BTreeMap<String, Array> {
        if all {
            return self.variables.clone();
        }
        if self.workspace_tools.names.is_empty() {
            return self
                .selected
                .as_ref()
                .and_then(|n| {
                    self.variables
                        .get(n)
                        .map(|v| BTreeMap::from([(n.clone(), v.clone())]))
                })
                .unwrap_or_default();
        }
        self.workspace_tools
            .names
            .iter()
            .filter_map(|n| self.variables.get(n).map(|v| (n.clone(), v.clone())))
            .collect()
    }
    pub(super) fn begin_workspace_action(&mut self, rename: bool, all: bool) {
        if self.busy || self.file_action_pending() {
            return;
        }
        let targets = self.workspace_targets(all);
        if targets.is_empty() || (rename && targets.len() != 1) {
            self.status = if rename {
                "Select one variable to rename"
            } else {
                "Select variables first"
            }
            .into();
            return;
        }
        let name = targets.keys().next().unwrap().clone();
        self.workspace_tools.dialog = Some(Dialog {
            targets,
            rename,
            all,
            name,
            focus: true,
            submitting: false,
            error: None,
        });
    }
    pub(super) fn reconcile_workspace_change(
        &mut self,
        change: Option<Change>,
        error: Option<&String>,
    ) {
        if let Some(change) = change {
            if let Change::Renamed(from, to) = change {
                if self.workspace_tools.cursor.as_ref() == Some(&from) {
                    self.workspace_tools.cursor = Some(to.clone());
                }
                if self.workspace_tools.anchor.as_ref() == Some(&from) {
                    self.workspace_tools.anchor = Some(to.clone());
                }
                if self.selected.as_ref() == Some(&from) {
                    self.selected = Some(to.clone());
                }
                if self.editing_variable.as_ref() == Some(&from) {
                    self.editing_variable = Some(to.clone());
                }
                if self.workspace_tools.names.remove(&from) {
                    self.workspace_tools.names.insert(to);
                }
            }
            self.workspace_tools.dialog = None;
        } else if let Some(dialog) = &mut self.workspace_tools.dialog
            && dialog.submitting
        {
            dialog.submitting = false;
            dialog.error = error.cloned();
        }
        self.workspace_tools
            .names
            .retain(|n| self.variables.contains_key(n));
        if self
            .selected
            .as_ref()
            .is_some_and(|n| !self.variables.contains_key(n))
        {
            self.selected = None;
        }
        if self
            .editing_variable
            .as_ref()
            .is_some_and(|n| !self.variables.contains_key(n))
        {
            self.editing_variable = None;
        }
    }
    #[cfg(test)]
    pub(super) fn workspace_variables_ui(&mut self, ui: &mut egui::Ui) {
        let height = ui
            .available_rect_before_wrap()
            .intersect(ui.clip_rect())
            .height();
        self.workspace_variables_ui_bounded(ui, height, true);
    }
    /// Header filter of the Workspace card: a filter icon until it is used,
    /// then the field. An emptied field folds back into the icon.
    pub(super) fn workspace_filter_control(&mut self, ui: &mut egui::Ui) {
        if !self.workspace_tools.filter_open && self.workspace_tools.filter.is_empty() {
            let icon = panels::icon_button(ui, panels::Glyph::Filter, "Filter variables");
            focus_areas::register(ui, &icon, focus_areas::Area::Workspace);
            if icon.clicked() {
                self.workspace_tools.filter_open = true;
                self.workspace_tools.filter_focus = true;
            }
            return;
        }
        // The icon clears; the field sits left of it. Leave the title ("Workspace") its room.
        if panels::icon_button(ui, panels::Glyph::Filter, "Clear variable filter").clicked() {
            self.workspace_tools.filter.clear();
            self.workspace_tools.filter_open = false;
            return;
        }
        let field = self.workspace_filter_field(ui, (ui.available_width() - 124.).clamp(40., 96.));
        if std::mem::take(&mut self.workspace_tools.filter_focus) {
            field.request_focus();
        }
        if field.lost_focus() && self.workspace_tools.filter.is_empty() {
            self.workspace_tools.filter_open = false;
        }
    }
    /// Variable filter field.
    pub(super) fn workspace_filter_field(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
    ) -> egui::Response {
        let focus_area = ui.add(
            egui::TextEdit::singleline(&mut self.workspace_tools.filter)
                .desired_width(width)
                .char_limit(63)
                .hint_text(RichText::new("Filter…").color(tokens::palette(ui.ctx()).text_disabled)),
        );
        focus_areas::register(ui, &focus_area, focus_areas::Area::Workspace);
        focus_area
    }
    pub(super) fn workspace_variables_ui_bounded(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        with_filter: bool,
    ) {
        let start = ui.cursor().top();
        if with_filter {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 48.).max(20.);
                self.workspace_filter_field(ui, width);
                if ui
                    .small_button("x")
                    .on_hover_text("Clear variable filter")
                    .clicked()
                {
                    self.workspace_tools.filter.clear();
                }
            });
        }
        let needle = self.workspace_tools.filter.as_bytes();
        let visible: Vec<&str> = self
            .variables
            .keys()
            .filter(|name| {
                needle.is_empty()
                    || name
                        .as_bytes()
                        .windows(needle.len())
                        .any(|w| w.eq_ignore_ascii_case(needle))
            })
            .map(String::as_str)
            .collect();
        let mut command = None;
        if self.variables.is_empty() {
            // No header, list or counter: one centred hint in the pane space.
            let rest = height - (ui.cursor().top() - start);
            chrome::empty_state(ui, "workspace", rest, |ui| {
                ui.weak(chrome::NO_VARIABLES);
            });
            return;
        }
        if visible.is_empty() {
            ui.weak("No matching variables");
        }
        // Rows sit edge to edge, separated by their 1 px lines.
        let item_spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.;
        let row_layout = row::Layout::new(ui);
        row_layout.header(ui);
        let counter = !self.workspace_tools.filter.is_empty();
        let row_height = row_layout.height;
        let stride = row_height + ui.spacing().item_spacing.y;
        let old_stride = self.workspace_tools.row_stride;
        let resized_offset = (old_stride > 0. && (old_stride - stride).abs() > 0.01)
            .then(|| self.workspace_tools.scroll_offset / old_stride * stride);
        self.workspace_tools.row_stride = stride;
        let focused = self
            .workspace_tools
            .focus
            .is_some_and(|id| ui.memory(|m| m.has_focus(id)));
        let navigation = if focused && !self.busy && !self.file_action_pending() {
            ui.input_mut(|i| {
                let index = i.events.iter().position(|event| matches!(event,
                    egui::Event::Key { key: egui::Key::ArrowUp | egui::Key::ArrowDown |
                        egui::Key::Home | egui::Key::End | egui::Key::PageUp | egui::Key::PageDown |
                        egui::Key::Enter, pressed: true, modifiers, .. } if !modifiers.alt && !modifiers.mac_cmd
                ))?;
                match i.events.remove(index) {
                    egui::Event::Key { key, modifiers, .. } => Some((key, modifiers)),
                    _ => unreachable!(),
                }
            })
        } else {
            None
        };
        let mut target = None;
        if let Some((key, modifiers)) = navigation {
            // A newly focused virtual row has no focus lock until the next frame.
            // Cancel egui's queued spatial movement when this list owns the key.
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            if key == egui::Key::Enter {
                if let Some(name) = &self.workspace_tools.cursor
                    && visible.contains(&name.as_str())
                {
                    command = Some((0, name.clone()));
                }
            } else {
                let page = (self.workspace_tools.viewport_height / stride)
                    .floor()
                    .max(1.) as usize;
                target = self
                    .workspace_tools
                    .navigate(&visible, key, modifiers, page);
                if target.is_some() && (!modifiers.ctrl || modifiers.shift) {
                    self.selected = self.workspace_tools.cursor.clone();
                }
            }
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("workspace_variables")
            // Keep one real row usable without forcing a 64px inner viewport
            // that can exceed the enclosing metadata body's actual clip.
            .min_scrolled_height(row_height)
            // Relative consumed height is stable when the enclosing pane scrolls.
            // Reserve the counter, then use all remaining finite pane space.
            .max_height(
                (height
                    - (ui.cursor().top() - start)
                    - if counter {
                        ui.text_style_height(&egui::TextStyle::Body) + item_spacing
                    } else {
                        0.
                    })
                .max(row_height),
            );
        if let Some(offset) = resized_offset {
            scroll = scroll.vertical_scroll_offset(offset);
            self.workspace_tools.scroll_offset = offset;
        }
        if let Some(index) = target {
            let top = index as f32 * stride;
            let bottom = top + row_height;
            let offset = self.workspace_tools.scroll_offset;
            let height = self.workspace_tools.viewport_height.max(row_height);
            scroll = scroll.vertical_scroll_offset(if top < offset {
                top
            } else if bottom > offset + height {
                bottom - height
            } else {
                offset
            });
        }
        let mut reveal = None;
        let output = scroll.show_rows_themed(ui, row_height, visible.len(), |ui, range| {
            for index in range {
                let name = visible[index];
                let value = &self.variables[name];
                let selected = self.workspace_tools.names.contains(name)
                    || (self.workspace_tools.names.is_empty()
                        && self.selected.as_deref() == Some(name));
                let response = ui
                    .push_id(name, |ui| row_layout.row(ui, name, value, selected))
                    .inner;
                if target == Some(index) {
                    reveal = Some(response.rect);
                    response.request_focus();
                    self.workspace_tools.focus = Some(response.id);
                }
                if response.has_focus() {
                    self.workspace_tools.focus = Some(response.id);
                    self.workspace_tools.cursor = Some(name.into());
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            response.id,
                            egui::EventFilter {
                                vertical_arrows: true,
                                ..Default::default()
                            },
                        )
                    });
                }
                if response.clicked() {
                    response.request_focus();
                    self.workspace_tools.focus = Some(response.id);
                    self.workspace_tools
                        .select(name, &visible, ui.input(|i| i.modifiers));
                    self.selected = self
                        .workspace_tools
                        .names
                        .contains(name)
                        .then(|| name.to_owned())
                        .or_else(|| self.workspace_tools.names.first().cloned());
                }
                if response.double_clicked() && !ui.input(|i| i.modifiers.ctrl || i.modifiers.shift)
                {
                    command = Some((0, name.to_owned()));
                }
                response.context_menu(|ui| {
                    if ui.button("Inspect variable").clicked() {
                        command = Some((0, name.to_owned()));
                        ui.close();
                    }
                    if ui.button("Rename variable...").clicked() {
                        command = Some((1, name.to_owned()));
                        ui.close();
                    }
                    if ui.button("Delete selected variables...").clicked() {
                        command = Some((2, name.to_owned()));
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            matches!(value, Array::Numeric(_) | Array::Logical(_)),
                            egui::Button::new("Export variable CSV…"),
                        )
                        .clicked()
                    {
                        command = Some((3, name.to_owned()));
                        ui.close();
                    }
                });
            }
        });
        if let Some(rect) = reveal {
            // The inner ScrollArea consumes row scroll requests. After it ends,
            // reveal the actual keyboard row in the enclosing metadata scroller.
            // A whole viewport may span both ancestor clip edges and not scroll.
            ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
        }
        self.workspace_tools.scroll_offset = output.state.offset.y;
        self.workspace_tools.viewport_height = output.inner_rect.height();
        ui.spacing_mut().item_spacing.y = item_spacing;
        // The card header shows the total; a filtered list shows its share.
        if counter {
            ui.weak(format!(
                "{} of {} variables",
                visible.len(),
                self.variables.len()
            ));
        }
        let focused = self
            .workspace_tools
            .focus
            .is_some_and(|id| ui.memory(|m| m.has_focus(id)));
        let mut keyboard_action = None;
        if focused && !self.busy && !self.file_action_pending() {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::A)) {
                self.workspace_tools.names = visible.iter().map(|n| n.to_string()).collect();
                self.selected = self.workspace_tools.names.first().cloned();
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
                keyboard_action = Some(true);
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
                keyboard_action = Some(false);
            }
        }
        drop(visible);
        if let Some(rename) = keyboard_action {
            self.begin_workspace_action(rename, false);
        }
        if let Some((action, name)) = command {
            if action != 2 || !self.workspace_tools.names.contains(&name) {
                self.workspace_tools.names = BTreeSet::from([name.clone()]);
                self.selected = Some(name.clone());
            }
            match action {
                0 => self.inspect_variable(name),
                1 => self.begin_workspace_action(true, false),
                3 => self.queue_workspace_csv(ui.ctx()),
                _ => self.begin_workspace_action(false, false),
            }
        }
    }
    pub(super) fn workspace_action_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.workspace_tools.dialog else {
            return;
        };
        let mut cancel = false;
        let mut submit = false;
        egui::Modal::new(egui::Id::new("workspace_action")).show(ctx, |ui| {
            ui.set_max_width(470.);
            ui.label(crate::tokens::title(
                ui.ctx(),
                if dialog.rename {
                    "Rename workspace variable"
                } else if dialog.all {
                    "Clear workspace"
                } else {
                    "Delete workspace variables"
                },
            ));
            if dialog.rename {
                ui.label(format!(
                    "Current name: {}",
                    dialog.targets.keys().next().unwrap()
                ));
                let input = ui.add_enabled(
                    !dialog.submitting,
                    egui::TextEdit::singleline(&mut dialog.name)
                        .id(egui::Id::new("workspace_rename_input"))
                        .char_limit(63),
                );
                if dialog.focus {
                    input.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, input.id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(dialog.name.chars().count()),
                            )));
                        state.store(ctx, input.id);
                    }
                    dialog.focus = false;
                }
                ui.small("Scripts and figure labels keep their existing text.");
            } else {
                ui.label(format!(
                    "Remove {} variable(s), {} bytes of logical workspace storage?",
                    dialog.targets.len(),
                    dialog.targets.values().map(Array::bytes).sum::<usize>()
                ));
                ui.label("This cannot be undone. Open figures and saved files remain available.");
                egui::ScrollArea::vertical()
                    .max_height(130.)
                    .show_rows_themed(
                        ui,
                        ui.text_style_height(&egui::TextStyle::Body),
                        dialog.targets.len(),
                        |ui, range| {
                            for name in dialog.targets.keys().skip(range.start).take(range.len()) {
                                ui.label(name);
                            }
                        },
                    );
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(tokens::palette(ui.ctx()).error, error);
            }
            ui.horizontal(|ui| {
                submit = ui
                    .add_enabled(
                        !dialog.submitting,
                        egui::Button::new(if dialog.rename {
                            "Rename"
                        } else {
                            "Delete variables"
                        }),
                    )
                    .clicked();
                cancel = ui
                    .button(if dialog.submitting { "Stop" } else { "Cancel" })
                    .clicked();
            });
        });
        cancel |= ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if cancel {
            if dialog.submitting {
                self.stop_work();
            }
            self.workspace_tools.dialog = None;
        } else if submit {
            if dialog.rename {
                if let Err(error) = workspace_edit::validate_name(&dialog.name) {
                    dialog.error = Some(error);
                    return;
                }
                if self.variables.contains_key(&dialog.name) {
                    dialog.error =
                        Some("That variable name already exists; choose another name".into());
                    return;
                }
            }
            let request = Request {
                targets: dialog.targets.clone(),
                action: if dialog.rename {
                    Action::Rename(dialog.name.clone())
                } else {
                    Action::Remove { all: dialog.all }
                },
            };
            self.submit(Job::EditWorkspace(Box::new(request)));
            if let Some(dialog) = &mut self.workspace_tools.dialog {
                dialog.submitting = self.busy;
            }
        }
    }
}

#[cfg(test)]
#[path = "workspace_row_tests.rs"]
mod row_tests;

// A single footer gesture retains only bounded metadata and shared immutable storage.
struct CsvAction {
    stamp: (egui::ViewportId, u64),
    generation: u64,
    name: String,
    value: Array,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    folder: PathBuf,
    dock: Option<matrust::session::WorkspaceDock>,
}
impl Workspace {
    /// `from_menu`: the Workspace actions menu itself is the open popup.
    fn workspace_csv_context(&self, ctx: &egui::Context, from_menu: bool) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && ctx.input(|input| input.focused && !input.viewport().close_requested())
            && !ctx.memory(|memory| memory.top_modal_layer().is_some())
            && (from_menu || !egui::Popup::is_any_open(ctx))
            && self.navigation.workspace
            && !self.busy
            && !self.file_action_pending()
            && self.open_queue.is_empty()
            && !self.folder_browser.navigating()
            && !self.recovering_session
            && !self.recovery_blocked
            && !self.allow_close
    }
    pub(super) fn can_export_workspace_csv(&self, ctx: &egui::Context) -> bool {
        self.workspace_csv_context(ctx, false) && self.selected_csv_value()
    }
    /// Enabled state of the menu item, evaluated while its menu is open.
    pub(super) fn can_export_workspace_csv_from_menu(&self, ctx: &egui::Context) -> bool {
        self.workspace_csv_context(ctx, true) && self.selected_csv_value()
    }
    fn selected_csv_value(&self) -> bool {
        self.selected
            .as_ref()
            .and_then(|name| self.variables.get(name))
            .is_some_and(|value| matches!(value, Array::Numeric(_) | Array::Logical(_)))
    }
    fn valid_workspace_csv(&self, ctx: &egui::Context, action: &CsvAction) -> bool {
        self.can_export_workspace_csv(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.workspace_tools.csv_generation
            && self.selected.as_deref() == Some(action.name.as_str())
            && self
                .variables
                .get(&action.name)
                .is_some_and(|now| action.value.same_snapshot(now))
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|memory| memory.focused())
            && action.folder == self.folder
            && action.dock == self.layout.workspace_dock
    }
    pub(super) fn queue_workspace_csv(&mut self, ctx: &egui::Context) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if !self.can_export_workspace_csv(ctx)
            || self.workspace_tools.csv_action.is_some()
            || self.workspace_tools.csv_finished == Some(stamp)
        {
            return;
        }
        let name = self
            .selected
            .clone()
            .expect("CSV admission requires a selected value");
        let value = self.variables[&name].clone();
        self.workspace_tools.csv_action = Some(CsvAction {
            stamp,
            generation: self.workspace_tools.csv_generation,
            name,
            value,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|memory| memory.focused()),
            folder: self.folder.clone(),
            dock: self.layout.workspace_dock,
        });
    }
    pub(super) fn cancel_workspace_csv(&mut self) {
        self.workspace_tools.csv_generation = self.workspace_tools.csv_generation.wrapping_add(1);
        if let Some(action) = self.workspace_tools.csv_action.take() {
            self.workspace_tools.csv_finished = Some(action.stamp);
        }
    }
    pub(super) fn observe_workspace_csv(&mut self, ctx: &egui::Context) {
        // Retire changed ownership before painting can surrender another widget's focus.
        if self
            .workspace_tools
            .csv_action
            .as_ref()
            .is_some_and(|action| !self.valid_workspace_csv(ctx, action))
        {
            self.cancel_workspace_csv();
        }
    }
    fn pick_workspace_csv(&mut self) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(picker) = self.workspace_tools.csv_picker {
            self.workspace_tools.csv_picker_calls += 1;
            return picker(self);
        }
        rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("data.csv")
            .save_file()
    }
    pub(super) fn finish_workspace_csv(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.workspace_tools.csv_action.take() else {
            return;
        };
        // Consume before the native picker: Cancel cannot replay this release.
        self.workspace_tools.csv_finished = Some(action.stamp);
        if !self.valid_workspace_csv(ctx, &action) {
            return;
        }
        if let Some(path) = self.pick_workspace_csv()
            && self.valid_workspace_csv(ctx, &action)
        {
            // submit synchronously marks the sole serial worker busy on admission.
            self.submit(Job::ExportVariable(path, action.name));
        }
    }
}
