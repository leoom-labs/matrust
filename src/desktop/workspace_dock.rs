//! One Workspace view, placed on the right or below the left navigation pane.
use super::*;
use crate::theme::ThemedScrollArea;
use matrust::session::WorkspaceDock;

/// Room for the list's own padding, so whole rows fit.
const LIST_SLACK: f32 = 6.;

/// Height of the selected-value preview body.
const PREVIEW_BODY: f32 = 64.;
/// Most lines of the selected-value text; a longer value ends in "... (R x C)".
const PREVIEW_LINES: usize = 20;
/// Larger real arrays preview their leading rows and columns only.
const PREVIEW_ELEMENTS: usize = 100_000;
const PREVIEW_BLOCK_COLUMNS: usize = 50;

/// Cached `disp` text of the selected variable.
pub(super) struct SelectedDisplay {
    value: matrust::Array,
    format: matrust::display_format::DisplayFormat,
    text: Arc<str>,
}

/// What the Command Window prints for `disp(value)` in `format`, capped at
/// `PREVIEW_LINES` lines; a cut ends in "... (R x C)". A very large real
/// array formats only its leading block, other large values keep the
/// bounded summary preview.
pub(super) fn selected_value_display(
    value: &matrust::Array,
    format: &matrust::display_format::DisplayFormat,
) -> String {
    let (rows, cols) = (value.rows(), value.cols());
    let mut cut = false;
    let text = if value.len() <= PREVIEW_ELEMENTS {
        matrust::display_format::disp(value, format).map(Option::unwrap_or_default)
    } else if let matrust::Array::Numeric(m) = value {
        cut = true;
        let (r, c) = (rows.min(PREVIEW_LINES), cols.min(PREVIEW_BLOCK_COLUMNS));
        let block = (0..c)
            .flat_map(|j| m.data()[j * rows..j * rows + r].iter().copied())
            .collect();
        matrust::Matrix::from_column_vec(r, c, block)
            .and_then(|block| matrust::display_format::disp(&block.into(), format))
            .map(Option::unwrap_or_default)
    } else {
        Ok(value.preview(8))
    }
    .unwrap_or_else(|_| value.preview(8));
    if text.is_empty() {
        return "[]".into();
    }
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() + usize::from(cut) > PREVIEW_LINES {
        lines.truncate(PREVIEW_LINES - 1);
        cut = true;
    }
    let mut shown = lines.join("\n");
    if cut {
        if !shown.is_empty() {
            shown.push('\n');
        }
        shown.push_str(&format!("... ({rows} x {cols})"));
    }
    shown
}

impl Workspace {
    pub(super) fn workspace_on_left(&self) -> bool {
        self.layout.workspace_dock == Some(WorkspaceDock::BelowFiles)
    }
    pub(super) fn set_workspace_dock(&mut self, ctx: &egui::Context, dock: WorkspaceDock) {
        if self.file_action_pending() || self.allow_close {
            return;
        }
        let next = (dock != WorkspaceDock::Right).then_some(dock);
        if self.layout.workspace_dock == next {
            return;
        }
        self.layout.workspace_dock = next;
        self.recovery_dirty = true;
        ctx.data_mut(|data| {
            for panel in [
                "workspace",
                "workspace_variables",
                "workspace_variables_left",
                "editor_split",
            ] {
                data.remove::<egui::containers::panel::PanelState>(egui::Id::new(panel));
            }
        });
        ctx.request_repaint();
    }
    pub(super) fn workspace_position_menu(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(!self.file_action_pending() && !self.allow_close, |ui| {
            ui.menu_button("Workspace position", |ui| {
                for (dock, label) in [
                    (WorkspaceDock::Right, "Right"),
                    (WorkspaceDock::BelowFiles, "Below Files"),
                ] {
                    if ui
                        .selectable_label(
                            self.workspace_on_left() == (dock == WorkspaceDock::BelowFiles),
                            label,
                        )
                        .clicked()
                    {
                        self.set_workspace_dock(ui.ctx(), dock);
                        ui.close();
                    }
                }
            });
        });
    }
    /// Natural height of the docked Workspace card: its header, the column
    /// row and one row per variable (at least three), plus the selected-value
    /// line. Includes the card frame.
    fn workspace_card_height(&self, ui: &egui::Ui) -> f32 {
        let ctx = ui.ctx();
        let row = workspace_tools::workspace_row_height(ui);
        let rows = self.variables.len().max(3) as f32;
        let frame = tokens::card(ctx).inner_margin(1).total_margin().sum().y;
        panels::header_height(ctx)
            + row * (1. + rows)
            + self.workspace_preview_reserve(ui)
            + frame
            + LIST_SLACK
    }
    /// Space below the list for the selected-value line and its preview.
    fn workspace_preview_reserve(&self, ui: &egui::Ui) -> f32 {
        let selected = self
            .selected
            .as_ref()
            .is_some_and(|name| self.variables.contains_key(name));
        if !selected {
            return 0.;
        }
        let line = ui.text_style_height(&egui::TextStyle::Body);
        let spacing = ui.spacing().item_spacing.y;
        let open = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            egui::Id::new("workspace_selected_preview"),
            false,
        )
        .is_open();
        line + spacing
            + if open {
                2. * (line + spacing) + PREVIEW_BODY + spacing
            } else {
                0.
            }
    }
    pub(super) fn desktop_side_panels(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let workspace_minimum = workspace_tools::minimum_dock_width(&ctx);
        let sidebar_minimum = if self.workspace_on_left() {
            workspace_minimum
        } else {
            170.
        };
        // The left column: Files above Workspace, two separate cards.
        egui::Panel::left("workspace")
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .default_size(
                self.layout
                    .sidebar_width
                    .unwrap_or(tokens::LEFT_COLUMN.max(sidebar_minimum)),
            )
            .min_size(sidebar_minimum)
            .max_size((ctx.content_rect().width() * 0.25).max(sidebar_minimum))
            .resizable(true)
            .show(root, |ui| {
                if self.workspace_on_left() {
                    let available = ui.available_height().max(1.);
                    let natural = self.workspace_card_height(ui);
                    let row = workspace_tools::workspace_row_height(ui);
                    // Three rows at least; up to 60 % of the column, but never
                    // fewer than twelve rows when the variables need them.
                    let frame = tokens::card(&ctx).inner_margin(1).total_margin().sum().y;
                    let minimum =
                        (panels::header_height(&ctx) + 4. * row + frame).min(available * 0.5);
                    let twelve = panels::header_height(&ctx) + 13. * row + frame + LIST_SLACK;
                    let maximum = (available * 0.6)
                        .max(twelve)
                        .min(available * 0.75)
                        .max(minimum);
                    // Until the divider is dragged the card follows its rows.
                    let preferred = match self.layout.left_workspace_fraction {
                        Some(fraction) => (available * fraction).clamp(minimum, maximum),
                        None => natural.clamp(minimum, maximum),
                    };
                    let key = egui::Id::new("workspace_card_auto_height");
                    if self.layout.left_workspace_fraction.is_none()
                        && ctx.data(|d| d.get_temp::<f32>(key)) != Some(preferred)
                    {
                        ctx.data_mut(|d| {
                            d.insert_temp(key, preferred);
                            d.remove::<egui::containers::panel::PanelState>(egui::Id::new(
                                "workspace_variables_left",
                            ));
                        });
                    }
                    egui::Panel::bottom("workspace_variables_left")
                        .show_separator_line(false)
                        .frame(tokens::card(&ctx).inner_margin(1))
                        .default_size(preferred)
                        .min_size(minimum)
                        .max_size(maximum)
                        .resizable(true)
                        .show(ui, |ui| {
                            ui.set_min_height(ui.available_height());
                            self.workspace_dock_contents(ui);
                        });
                    if let Some(size) = layout::resized_panel(&ctx, "workspace_variables_left") {
                        self.layout.left_workspace_fraction =
                            Some((size.y / available).clamp(0.2, 0.8));
                    }
                }
                egui::CentralPanel::default()
                    .frame(tokens::card(&ctx).inner_margin(1))
                    .show(ui, |ui| self.left_navigation_contents(ui));
            });
        if !self.workspace_on_left() {
            egui::Panel::right("workspace_variables")
                .show_separator_line(false)
                .frame(tokens::card(&ctx).inner_margin(1))
                .default_size(self.layout.workspace_width.unwrap_or(workspace_minimum))
                .min_size(workspace_minimum)
                .max_size((ctx.content_rect().width() * 0.25).max(workspace_minimum))
                .resizable(true)
                .show(root, |ui| self.workspace_dock_contents(ui));
        }
    }
    fn left_navigation_contents(&mut self, ui: &mut egui::Ui) {
        // Capture the finite panel edge before ScrollArea gives its child unbounded height.
        let files_bottom = ui.max_rect().bottom().min(ui.clip_rect().bottom());
        egui::ScrollArea::vertical()
            .id_salt("sidebar_all")
            .auto_shrink([false, false])
            .show_themed(ui, |ui| self.folder_browser_ui(ui, files_bottom));
    }
    fn workspace_dock_contents(&mut self, ui: &mut egui::Ui) {
        // Resolve a finite pane before entering any ScrollArea. Its content UI
        // is deliberately unbounded and cannot be used as a height budget.
        let body = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        let spacing = ui.spacing().item_spacing.y;
        ui.allocate_rect(body, egui::Sense::hover());
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body));
            egui::ScrollArea::vertical()
                .id_salt("workspace_values_sidebar")
                // The finite body can be shorter than egui's default 64px minimum.
                .min_scrolled_height(0.)
                .max_height(body.height())
                .auto_shrink([false, false])
                .show_themed(ui, |ui| {
                    let start = ui.cursor().top();
                    let count = chrome::variable_count(self.variables.len());
                    panels::header(ui, "Workspace", Some(&count), |ui| {
                        self.workspace_header_tools(ui);
                    });
                    let preview_id = egui::Id::new("workspace_selected_preview");
                    let preview = egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        preview_id,
                        false,
                    );
                    let preview_reserve = self.workspace_preview_reserve(ui);
                    let remaining = body.height() - (ui.cursor().top() - start) - preview_reserve;
                    self.workspace_variables_ui_bounded(ui, remaining.max(0.), false);
                    if let Some((name, value)) = self.selected.clone().and_then(|name| {
                        let value = self.variables.get(&name)?.clone();
                        Some((name, value))
                    }) {
                        let text = self.selected_value_text(&value);
                        // The same left inset as the variable rows' text.
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: tokens::M as i8,
                                right: tokens::S as i8,
                                ..Default::default()
                            })
                            .show(ui, |ui| {
                                let mut label_clicked = false;
                                let mut header = preview.show_header(ui, |ui| {
                                    label_clicked = ui
                                        .add(
                                            egui::Label::new("Selected value")
                                                .truncate()
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text("Show or hide the selected variable preview")
                                        .clicked();
                                });
                                // show_header already toggles its separate arrow response.
                                // Only the custom label response needs this second route.
                                if label_clicked {
                                    header.toggle();
                                }
                                header.body_unindented(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&name)
                                                .strong()
                                                .color(tokens::palette(ui.ctx()).accent),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&name);
                                    ui.add(
                                        egui::Label::new(format!(
                                            "{} | {} bytes",
                                            workspace_type_label(&value),
                                            value.bytes()
                                        ))
                                        .truncate(),
                                    );
                                    // Long lines clip at the pane edge; only rows scroll.
                                    egui::ScrollArea::vertical()
                                        .id_salt("matrix_preview")
                                        .auto_shrink([false, true])
                                        .max_height(PREVIEW_BODY.max(spacing))
                                        .show_themed(ui, |ui| {
                                            ui.add(
                                                egui::Label::new(RichText::new(&*text).monospace())
                                                    .wrap_mode(egui::TextWrapMode::Extend),
                                            );
                                        });
                                });
                            });
                    } else {
                        // Nothing to show: release the cached array's storage.
                        self.selected_display = None;
                    }
                });
        });
    }
    /// `disp` text of the selected value in the engine's current `format`,
    /// recomputed only when the value or the format changes.
    pub(super) fn selected_value_text(&mut self, value: &matrust::Array) -> Arc<str> {
        let format = self.display_format;
        if let Some(cached) = &self.selected_display
            && cached.format == format
            && cached.value.same_snapshot(value)
        {
            return cached.text.clone();
        }
        let text: Arc<str> = selected_value_display(value, &format).into();
        self.selected_display = Some(SelectedDisplay {
            value: value.clone(),
            format,
            text: text.clone(),
        });
        text
    }
    /// Header tools of the Workspace card: the overflow menu (variable
    /// actions) and the filter, which is an icon until it is used.
    fn workspace_header_tools(&mut self, ui: &mut egui::Ui) {
        let more = panels::icon_button(ui, panels::Glyph::More, "Workspace actions");
        let mut export = false;
        egui::Popup::menu(&more).show(|ui| {
            let selected = self.selected.is_some();
            if ui
                .add_enabled(selected, egui::Button::new("Inspect / edit…"))
                .on_hover_text("Inspect or edit the selected variable")
                .clicked()
                && let Some(name) = self.selected.clone()
            {
                self.inspect_variable(name);
                ui.close();
            }
            if ui
                .add_enabled(
                    self.can_export_workspace_csv_from_menu(ui.ctx()),
                    egui::Button::new("Export variable CSV…"),
                )
                .on_hover_text("Export the selected real or logical variable as CSV")
                .on_disabled_hover_text(
                    "Select a real or logical variable and finish pending file operations",
                )
                .clicked()
            {
                // Admission needs the menu closed: queue it after `show`.
                export = true;
                ui.close();
            }
        });
        if export {
            self.queue_workspace_csv(ui.ctx());
        }
        self.workspace_filter_control(ui);
    }
}
