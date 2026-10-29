use super::*;
use crate::theme::ThemedScrollArea;

pub(super) use matrust::session::PaneMode;

/// egui's resize handle owns the drag; incidental window resizing does not
/// overwrite the user's preferred dimensions. Actual pointer-drag tests guard
/// this panel handle convention when the pinned egui dependency is upgraded.
pub(super) fn resized_panel(ctx: &egui::Context, panel: &str) -> Option<egui::Vec2> {
    let id = egui::Id::new(panel);
    let response = ctx.read_response(id.with("__resize"))?;
    (response.dragged() || response.drag_stopped())
        .then(|| egui::containers::panel::PanelState::load(ctx, id).map(|s| s.size()))
        .flatten()
}

// A caption inside the current axes, never a popup over the figure header.
fn plot_inspection_hint(ui: &egui::Ui, response: &egui::Response) {
    let Some(plot) = figure_view::hovered_plot(ui.ctx()) else {
        return;
    };
    if !response.enabled()
        || !response.rect.intersects(plot.frame)
        || !ui.rect_contains_pointer(plot.frame)
        || egui::Popup::is_any_open(ui.ctx())
        || ui.input(|i| i.pointer.any_down())
    {
        return;
    }
    let inset = plot.frame.shrink(8.);
    let color = tokens::palette(ui.ctx()).text_muted;
    let font = egui::FontId::proportional(tokens::CAPTION);
    let galley = ui
        .painter()
        .layout_no_wrap(PLOT_INSPECTION_HINT.into(), font.clone(), color);
    let left = egui::Rect::from_min_size(
        inset.left_bottom() - egui::vec2(0., galley.size().y),
        galley.size().min(inset.size()),
    );
    let pointer_over_caption =
        ui.input(|i| i.pointer.hover_pos().is_some_and(|p| left.contains(p)));
    let coordinates = plot
        .coordinates
        .filter(|r| r.y_range().intersects(left.y_range()));
    let move_right = pointer_over_caption || coordinates.is_some_and(|r| r.intersects(left));
    let mut available = inset;
    if let Some(coordinates) = coordinates {
        // Use the actual coordinate popup bounds, including its padding. In
        // compact axes truncate the caption rather than cover the coordinates.
        if move_right {
            available.min.x = available.min.x.max(coordinates.right() + 8.);
        } else {
            available.max.x = available.max.x.min(coordinates.left() - 8.);
        }
    }
    if available.width() <= 0. || available.height() < galley.size().y {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        PLOT_INSPECTION_HINT.into(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(available.width());
    let galley = ui.painter().layout_job(job);
    let x = if move_right {
        available.right() - galley.size().x
    } else {
        available.left()
    };
    let pos = egui::pos2(x, inset.bottom() - galley.size().y);
    ui.painter()
        .with_clip_rect(ui.clip_rect().intersect(plot.frame))
        .galley(pos, galley, color);
}

pub(super) const PLOT_INSPECTION_HINT: &str =
    "Ctrl+click a plot to inspect its nearest visible original sample.";

impl Workspace {
    pub(super) fn prepare_layout(&self, ctx: &egui::Context) {
        let size = ctx.content_rect().size();
        ctx.data_mut(|data| {
            let key = egui::Id::new("desktop_layout_viewport");
            let previous = data.get_temp::<egui::Vec2>(key);
            if previous.is_none_or(|old| (old - size).length() > 0.5) {
                for panel in [
                    "workspace",
                    "workspace_variables",
                    "workspace_variables_left",
                    "console",
                    "editor_split",
                ] {
                    data.remove::<egui::containers::panel::PanelState>(egui::Id::new(panel));
                }
                data.insert_temp(key, size);
            }
        });
    }

    pub(super) fn prepare_split(&self, ctx: &egui::Context, width: f32) {
        ctx.data_mut(|data| {
            let key = egui::Id::new("desktop_split_available_width");
            if data
                .get_temp::<f32>(key)
                .is_none_or(|old| (old - width).abs() > 0.5)
            {
                data.remove::<egui::containers::panel::PanelState>(egui::Id::new("editor_split"));
                data.insert_temp(key, width);
            }
        });
    }

    pub(super) fn desktop_scale_shortcuts(&mut self, ctx: &egui::Context) {
        let change = ctx.input_mut(|i| {
            if i.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::ALT,
                egui::Key::Num0,
            ) {
                Some(100)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Plus)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::Equals)
            {
                Some((self.layout.ui_scale_percent() + 25).min(150))
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Minus) {
                Some(self.layout.ui_scale_percent().saturating_sub(25).max(75))
            } else {
                None
            }
        });
        if let Some(percent) = change {
            self.set_desktop_scale(ctx, percent);
        }
    }

    pub(super) fn set_desktop_scale(&mut self, ctx: &egui::Context, percent: u16) {
        let percent = percent.clamp(75, 150);
        let setting = (percent != 100).then_some(percent);
        self.recovery_dirty |= self.layout.ui_scale_percent != setting;
        self.layout.ui_scale_percent = setting;
        ctx.set_zoom_factor(f32::from(percent) / 100.);
    }

    pub(super) fn desktop_scale_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(
            format!("Desktop scale: {}%", self.layout.ui_scale_percent()),
            |ui| {
                for percent in [75, 100, 125, 150] {
                    if ui
                        .selectable_label(
                            self.layout.ui_scale_percent() == percent,
                            format!("{percent}%"),
                        )
                        .clicked()
                    {
                        self.set_desktop_scale(ui.ctx(), percent);
                        ui.close();
                    }
                }
                ui.separator();
                ui.label("Ctrl++ / Ctrl+- to resize; Ctrl+Alt+0 for 100%.");
                ui.weak("Relative to your display settings. Saved with the session.");
            },
        );
    }

    pub(super) fn has_startup_placeholder(&self) -> bool {
        self.layout.center.is_some_and(|c| !c.editor_revealed)
            && self.documents.len() == 1
            && self.documents[0].path.is_none()
            && !self.documents[0].dirty
    }

    pub(super) fn central_command(&self) -> bool {
        self.layout.center.is_some_and(|center| center.command)
    }

    /// The current figure takes the right column beside the Command Window or
    /// the Editor. Without a figure the middle column takes the width.
    pub(super) fn figure_column(&self) -> bool {
        self.layout.center.is_some()
            && self.toolkits.selected_tool().is_none()
            // An image or PDF preview is its own window: it never opens the
            // column, which would only show the empty "No figures" card.
            && self.figure.is_some()
            && (self.central_command() || self.layout.mode == PaneMode::Editor)
    }

    /// An open script sits above the Command Window in the middle column.
    pub(super) fn editor_above_command(&self) -> bool {
        self.toolkits.selected_tool().is_none()
            && self.central_command()
            && self
                .layout
                .center
                .is_some_and(|center| center.editor_revealed)
            && !self.has_startup_placeholder()
            && !self.editor_window.detached
    }

    pub(super) fn show_command_center(&mut self) {
        self.toolkits.deselect_tool();
        self.document_save.command_selected();
        self.layout
            .center
            .get_or_insert(session::CenterLayout {
                command: false,
                editor_revealed: true,
            })
            .command = true;
        self.navigation.command = true;
        self.recovery_dirty = true;
    }

    pub(super) fn select_document_layout(&mut self, mode: PaneMode) {
        self.toolkits.deselect_tool();
        if let Some(center) = &mut self.layout.center {
            center.command = false;
            if mode != PaneMode::Figure {
                center.editor_revealed = true;
            }
            self.compact_figure = mode == PaneMode::Figure;
        }
        self.layout.mode = mode;
        self.document_save.activate(if mode == PaneMode::Figure {
            document_save::Pane::Figure
        } else {
            document_save::Pane::Script
        });
        self.recovery_dirty = true;
    }

    pub(super) fn center_tabs(&mut self, root: &mut egui::Ui) {
        let center = self.layout.center;
        // Documents have no tab row of their own: the Command Window, the
        // editor and the figure are cards. Open tools add the row.
        if self.toolkits.tabs().next().is_none() {
            return;
        }
        let pending = self.file_action_pending() || self.allow_close;
        let documents = self.toolkits.selected_tool().is_none();
        egui::Panel::top("desktop_center_tabs")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(tokens::palette(root.ctx()).canvas)
                    .inner_margin(egui::Margin::symmetric(tokens::XS as i8, 2)),
            )
            .resizable(false)
            .show(root, |ui| {
                if tokens::compact(ui.ctx()) {
                    tokens::dense(ui);
                }
                // A long tab row scrolls rather than cutting the last tab.
                egui::ScrollArea::horizontal()
                    .id_salt("desktop_center_tabs_scroll")
                    .show_themed(ui, |ui| {
                        ui.horizontal(|ui| {
                            let Some(center) = center else {
                                // Legacy layouts have no document tabs; one tab returns to them.
                                let label = match self.layout.mode {
                                    PaneMode::Editor => "Editor",
                                    PaneMode::Figure => "Figure",
                                    PaneMode::Split => "Editor and figure",
                                };
                                if ui
                                    .add_enabled(
                                        !pending,
                                        egui::Button::selectable(documents, label),
                                    )
                                    .clicked()
                                {
                                    self.toolkits.deselect_tool();
                                    self.legacy_documents_selected();
                                }
                                self.tool_tabs(ui, pending);
                                return;
                            };
                            if ui
                                .add_enabled(
                                    !pending,
                                    egui::Button::selectable(
                                        documents && center.command,
                                        "Command",
                                    ),
                                )
                                .clicked()
                            {
                                self.show_command_center();
                            }
                            if center.editor_revealed
                                && !center.command
                                && ui
                                    .add_enabled(
                                        !pending,
                                        egui::Button::selectable(
                                            documents
                                                && !center.command
                                                && self.layout.mode == PaneMode::Editor,
                                            "Editor",
                                        ),
                                    )
                                    .clicked()
                            {
                                self.select_document_layout(PaneMode::Editor);
                            }
                            if self.figure.is_some()
                                && !center.command
                                && ui
                                    .add_enabled(
                                        !pending,
                                        egui::Button::selectable(
                                            documents
                                                && !center.command
                                                && self.layout.mode == PaneMode::Figure,
                                            "Figure",
                                        ),
                                    )
                                    .clicked()
                            {
                                self.select_document_layout(PaneMode::Figure);
                            }
                            self.tool_tabs(ui, pending);
                        });
                    });
            });
    }

    // Tool tabs follow the document tabs in opening order. Close (x) matches
    // the former window close and stays available like it.
    fn tool_tabs(&mut self, ui: &mut egui::Ui, pending: bool) {
        let selected = self.toolkits.selected_tool();
        let tabs: Vec<_> = self.toolkits.tabs().collect();
        for tool in tabs {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.;
                let label = self.toolkits.tab_label(tool);
                if ui
                    .add_enabled(
                        !pending,
                        egui::Button::selectable(selected == Some(tool), label),
                    )
                    .clicked()
                {
                    self.select_tool_tab(tool);
                }
                if ui
                    .add(egui::Button::new("\u{d7}").frame(false))
                    .on_hover_text(format!("Close {}", tool.label()))
                    .clicked()
                {
                    self.close_tool_tab(tool);
                }
            });
        }
    }

    // The shown document takes Save/Close back from a hidden model.
    fn legacy_documents_selected(&mut self) {
        self.document_save.activate(match self.layout.mode {
            PaneMode::Figure => document_save::Pane::Figure,
            PaneMode::Split if self.compact_figure => document_save::Pane::Figure,
            _ => document_save::Pane::Script,
        });
    }

    /// Closes a tool tab. When it was shown, the other open tool takes over
    /// exactly as if its tab were clicked, else the documents tab.
    pub(super) fn close_tool_tab(&mut self, tool: toolkits::Tool) {
        let shown = self.toolkits.selected_tool() == Some(tool);
        self.toolkits.close_tool(tool);
        if tool == toolkits::Tool::Model {
            self.document_save.model_closed();
        }
        if !shown {
            return;
        }
        match self.toolkits.selected_tool() {
            Some(next) => self.select_tool_tab(next),
            None if self.layout.center.is_none() => self.legacy_documents_selected(),
            None => self.document_save.command_selected(),
        }
    }

    pub(super) fn select_tool_tab(&mut self, tool: toolkits::Tool) {
        self.toolkits.select_tool(tool);
        if tool == toolkits::Tool::Model {
            self.document_save.activate(document_save::Pane::Model);
        } else {
            // Like Command: no document of its own; keep the last real one.
            self.document_save.command_selected();
        }
    }

    pub(super) fn reset_layout(&mut self, ctx: &egui::Context) {
        if self.editor_window.detached {
            self.dock_editor_window(ctx);
        }
        let editor_revealed = self.layout.center.is_none_or(|c| c.editor_revealed)
            || self.documents.iter().any(|d| d.dirty || d.path.is_some());
        self.document_save.activate(document_save::Pane::Script);
        self.layout = session::DesktopLayout {
            theme: self.layout.theme,
            ..session::DesktopLayout::fresh_desktop()
        };
        self.layout.center.as_mut().unwrap().editor_revealed = editor_revealed;
        // Tools stay open; the fresh layout selects Command.
        self.toolkits.deselect_tool();
        self.compact_figure = false;
        self.navigation = Default::default();
        self.collapse_editor_columns();
        ctx.set_zoom_factor(1.);
        self.clear_layout_panel_sizes(ctx);
        self.recovery_dirty = true;
    }

    pub(super) fn clear_layout_panel_sizes(&self, ctx: &egui::Context) {
        ctx.data_mut(|data| {
            for panel in [
                "workspace",
                "workspace_variables",
                "workspace_variables_left",
                "console",
                "editor_split",
                "figure_column",
            ] {
                data.remove::<egui::containers::panel::PanelState>(egui::Id::new(panel));
            }
        });
    }

    pub(super) fn reveal_editor(&mut self) {
        if self.layout.center.is_some() {
            self.select_document_layout(PaneMode::Editor);
            self.reveal_editor_window();
            return;
        }
        self.toolkits.deselect_tool();
        self.document_save.activate(document_save::Pane::Script);
        self.compact_figure = false;
        if self.layout.mode == PaneMode::Figure {
            self.layout.mode = PaneMode::Split;
        }
        self.reveal_editor_window();
    }

    pub(super) fn compact_panes(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            if tokens::compact(ui.ctx()) {
                tokens::dense(ui);
            }
            ui.horizontal(|ui| {
                if ui
                    .selectable_value(&mut self.compact_figure, false, "Script editor")
                    .clicked()
                {
                    self.document_save.activate(document_save::Pane::Script);
                }
                if ui
                    .selectable_value(&mut self.compact_figure, true, "Plot figure")
                    .clicked()
                {
                    self.document_save.activate(document_save::Pane::Figure);
                }
            });
        });
        ui.separator();
        if self.compact_figure {
            self.figure_pane(ui);
        } else {
            self.editor_pane(ui);
        }
    }

    fn pane_toggle(&mut self, ui: &mut egui::Ui, pane: PaneMode) {
        let expanded = self.layout.mode == pane;
        if ui
            .small_button(if expanded { "Restore split" } else { "Expand" })
            .on_hover_text(if expanded {
                "Show editor and figure"
            } else {
                "Expand this pane"
            })
            .clicked()
        {
            self.select_document_layout(if expanded { PaneMode::Split } else { pane });
            self.document_save.activate(if pane == PaneMode::Figure {
                document_save::Pane::Figure
            } else {
                document_save::Pane::Script
            });
            ui.close();
        }
    }
    pub(super) fn editor_pane(&mut self, editor: &mut egui::Ui) {
        if self.editor_window.detached && !self.editor_window.in_callback {
            self.editor_window_placeholder(editor);
            return;
        }
        if !self.begin_editor_host_paint(editor.ctx()) {
            return;
        }
        // This group moves between two hosts but is painted exactly once.
        // Keep implicit ScrollArea/tab/search IDs stable as well as TextEdit IDs.
        editor.scope_builder(
            egui::UiBuilder::new().id(editor_window::group_id()),
            |editor| {
                self.editor_pane_contents(editor);
            },
        );
    }

    fn editor_pane_contents(&mut self, editor: &mut egui::Ui) {
        if !self.editor_window.in_callback {
            self.document_save
                .register_ui(document_save::Pane::Script, editor);
        }
        if self.has_startup_placeholder() {
            self.editor_empty_state(editor);
            return;
        }

        let active = self.generic_save_target() == document_save::Pane::Script
            && !editor.memory(|m| m.has_focus(egui::Id::new("command_input")));
        let name = self.documents[self.active].title();
        panels::header_with(editor, "Editor", Some(&name), active, |ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            let more = panels::icon_button(ui, panels::Glyph::More, "Editor actions");
            egui::Popup::menu(&more)
                .id(egui::Id::new("editor_actions"))
                .show(|ui| {
                    self.editor_columns_control(ui);
                    self.editor_window_control(ui);
                    if !self.editor_window.in_callback {
                        self.pane_toggle(ui, PaneMode::Editor);
                    }
                });
            if panels::icon_button_with(
                ui,
                panels::Glyph::Search,
                "Find / replace",
                "Find / replace · Ctrl F",
            )
            .clicked()
            {
                self.editor.open(false);
            }
        });
        let mut close = None;
        let mut activate = None;
        editor.horizontal(|row| {
            // A floating scrollbar overlaps these short draggable tabs. Reserve
            // its own strip so scrollbar dragging and tab dragging stay distinct.
            row.spacing_mut().scroll.floating = false;
            self.script_tab_order_ui(row);
            let mut tabs = egui::ScrollArea::horizontal()
                .id_salt("script_tabs")
                .animated(false)
                .min_scrolled_width(0.)
                .max_width(row.available_width().max(0.));
            if let Some(offset) = self.script_tab_drag_scroll() {
                tabs = tabs.horizontal_scroll_offset(offset);
            }
            let tabs = tabs.show_themed(row, |editor| {
                editor.horizontal(|ui| {
                    for i in 0..self.documents.len() {
                        let document = self.documents[i].id;
                        let title = self.documents[i].title();
                        let tab = ui
                            .push_id(("script_tab", document), |ui| {
                                ui.add(
                                    egui::Button::selectable(self.active == i, title)
                                        .sense(egui::Sense::click_and_drag())
                                        .corner_radius(3.),
                                )
                            })
                            .inner;
                        self.script_tab_drag_response(ui, &tab, document);

                        if self.active == i {
                            ui.painter().line_segment(
                                [tab.rect.left_bottom(), tab.rect.right_bottom()],
                                egui::Stroke::new(2., tokens::palette(ui.ctx()).accent),
                            );
                        }
                        if self.script_tab_reveal == Some(document) {
                            tab.scroll_to_me(Some(egui::Align::Center));
                            // A layout retry can initially have no measured
                            // overflow. Retain the request until it is visible.
                            if ui.clip_rect().contains(tab.rect.center()) {
                                self.script_tab_reveal = None;
                            } else {
                                ui.ctx().request_repaint();
                            }
                        }
                        if tab.clicked() {
                            activate = Some(i);
                        }
                        if ui.small_button("×").on_hover_text("Close script").clicked() {
                            close = Some(i);
                        }
                    }
                })
            });
            // ScrollArea returns its pre-content inner rect. Use the measured
            // tab row height after the nonfloating scrollbar has reserved space.
            let edge_viewport = egui::Rect::from_x_y_ranges(
                tabs.inner_rect.x_range(),
                tabs.inner.response.rect.y_range(),
            )
            .intersect(row.clip_rect());
            self.update_script_tab_drag_scroll(
                row.ctx(),
                edge_viewport,
                tabs.content_size.x,
                tabs.state.offset.x,
            );
        });
        if let Some(i) = activate {
            self.activate_script_index(i);
            self.document_save.activate(document_save::Pane::Script);
            // TextEdit can surrender pre-paint focus for this outside click.
            // Retain the exact document/frame intent and focus after painting.
            self.queue_pane_focus(editor.ctx(), navigation::FocusPane::Editor);
        }
        if let Some(i) = close {
            if self.editor_window.in_callback {
                self.queue_editor_tab_close(editor.ctx(), i);
            } else {
                self.request_script_close(i);
            }
        }
        editor.separator();
        if let Some(issue) = &self.editor_issue {
            let message = format!("{} · {}", issue.origin.title, issue.diagnostic);
            let label: String = message.chars().take(200).collect();
            let mut jump = false;
            editor.horizontal_wrapped(|ui| {
                jump = ui.small_button("Go to error").clicked();
                ui.colored_label(tokens::palette(ui.ctx()).error, label)
                    .on_hover_text(message);
            });
            if jump {
                self.jump_to_error();
            }
        }
        self.script_editor_views(editor);
    }
    pub(super) fn figure_pane(&mut self, figure_ui: &mut egui::Ui) {
        if self.layout.figure_tiling.is_some() {
            // Tile rows are chrome too: compact windows use the dense rows of
            // the single-figure header, so each tile keeps room for its axes.
            if tokens::compact(figure_ui.ctx()) {
                tokens::dense(figure_ui);
            }
            // Tiled, the mode and size choices are the card's own controls.
            panels::header(figure_ui, "Figures", None, |ui| {
                ui.spacing_mut().item_spacing.x = tokens::XS;
                self.figure_tile_controls(ui, true);
            });
            self.figure_tiles(figure_ui);
            return;
        }
        self.detached.hide_tiles();
        if let Some(figure) = &self.figure {
            if figure.layout.is_some() {
                self.plot_points = Vec::new();
                if !self
                    .subplot_canvas
                    .matches_source(self.current_figure, figure)
                {
                    self.subplot_canvas
                        .retain_source(self.current_figure, figure);
                    let _ = self
                        .detached
                        .reserve_root_cache(0, crate::figure_geometry::ONE_CACHE_BYTES);
                }
            } else {
                self.subplot_canvas.release_geometry();
            }
        }
        self.document_save
            .register_ui(document_save::Pane::Figure, figure_ui);
        // ScrollArea gives its content unbounded height. Preserve this finite
        // viewport extent so a fitting title, controls and axes stay together.
        // Genuinely smaller panes retain a bounded scrollable body.
        let body_height = figure_ui.available_height().max(180.);
        if figure_ui.available_height() < 260. {
            // This covers both compact tabs and the expanded figure-only pane.
            figure_ui.scope(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("short_figure_scroll")
                    // Wheel/drag gestures belong to the plot; the solid bar
                    // scrolls its surrounding controls and axis labels.
                    .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show_themed(ui, |ui| {
                        let rect = egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), body_height),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            self.figure_pane_contents(ui, true);
                        });
                    });
            });
        } else {
            self.figure_pane_contents(figure_ui, false);
        }
    }

    /// First section of the figure card's overflow menu: view mode, expand,
    /// edit history, view options and save.
    fn figure_menu_head(&mut self, ui: &mut egui::Ui, command: &mut Option<String>) {
        ui.horizontal(|ui| self.figure_tile_controls(ui, false));
        self.pane_toggle(ui, PaneMode::Figure);
        if self.figure.is_none() {
            return;
        }
        let (undo, redo) = self
            .figure_history
            .get(&self.current_figure)
            .copied()
            .unwrap_or_default();
        if ui
            .add_enabled(!self.busy && undo > 0, egui::Button::new("Undo edit"))
            .on_hover_text(format!("Undo a desktop figure edit ({undo} available)"))
            .clicked()
        {
            self.submit(Job::UndoFigure(
                self.current_figure,
                self.view_bounds.map(|v| (v, self.view_auto)),
            ));
            ui.close();
        }
        if ui
            .add_enabled(!self.busy && redo > 0, egui::Button::new("Redo edit"))
            .on_hover_text(format!("Redo a desktop figure edit ({redo} available)"))
            .clicked()
        {
            self.submit(Job::RedoFigure(
                self.current_figure,
                self.view_bounds.map(|v| (v, self.view_auto)),
            ));
            ui.close();
        }
        if let Some(f) = &self.figure {
            if ui.button("Reset view").clicked() {
                self.reset_plot = true;
                if f.layout.is_some() {
                    *command = Some("axis auto".into());
                }
                ui.close();
            }
            figure_view_options(ui, self.busy, f, command);
        }
        if ui
            .add_enabled(
                self.root_figure_save_context(ui.ctx()),
                egui::Button::new("Save figure"),
            )
            .on_hover_text(
                "Save the selected figure to its current file, or choose a file for an untitled figure.",
            )
            .clicked()
        {
            self.queue_root_figure_save(ui.ctx(), false);
            ui.close();
        }
    }

    pub(super) fn figure_file_menu(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        if ui
            .add_enabled(
                !self.busy && !self.file_action_pending() && self.figure.is_some(),
                egui::Button::new(if self.detached.contains(self.current_figure) {
                    "Dock figure"
                } else {
                    "Undock figure"
                }),
            )
            .clicked()
        {
            if self.detached.contains(self.current_figure) {
                self.dock_figure(self.current_figure, ui.ctx());
            } else {
                self.undock_figure(self.current_figure, ui.ctx());
            }
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                !self.busy
                    && self
                        .figure_documents
                        .keys()
                        .any(|&id| self.figure_dirty(id)),
                egui::Button::new("Save all figures"),
            )
            .clicked()
        {
            self.begin_save_all_figures();
            ui.close();
        }
        if ui
            .add_enabled(
                self.can_close_all_figures(),
                egui::Button::new("Close all figures"),
            )
            .clicked()
        {
            self.request_close_all_figures();
            ui.close();
        }
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.menu_button("New layout", |ui| {
                use matrust::plot::{Arrangement, TileGrid};
                ui.label("Create in a new figure");
                for (label, arrangement, rows, columns) in [
                    ("2 by 2", Arrangement::Fixed, 2, 2),
                    ("2 by 1", Arrangement::Fixed, 2, 1),
                    ("1 by 2", Arrangement::Fixed, 1, 2),
                    ("Flow", Arrangement::Flow, 1, 1),
                    ("Vertical stack", Arrangement::Vertical, 1, 1),
                    ("Horizontal stack", Arrangement::Horizontal, 1, 1),
                ] {
                    if ui.button(label).clicked() {
                        self.submit(Job::NewLayout(
                            TileGrid::new(arrangement, rows, columns).unwrap(),
                        ));
                        ui.close();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Rows");
                    ui.add(egui::DragValue::new(&mut self.layout_grid[0]).range(1..=16));
                    ui.label("Columns");
                    ui.add(egui::DragValue::new(&mut self.layout_grid[1]).range(1..=16));
                });
                if ui.button("Create custom layout").clicked() {
                    self.submit(Job::NewLayout(
                        TileGrid::new(Arrangement::Fixed, self.layout_grid[0], self.layout_grid[1])
                            .unwrap(),
                    ));
                    ui.close();
                }
            });
        });
        if ui
            .add_enabled(!self.busy, egui::Button::new("New"))
            .clicked()
        {
            self.run("figure".into());
        }
        if ui
            .add_enabled(
                !self.busy && self.figure.is_some(),
                egui::Button::new("Close"),
            )
            .clicked()
        {
            self.request_figure_close(self.current_figure);
        }
        if ui
            .add_enabled(self.root_open_context(ui.ctx()), egui::Button::new("Open…"))
            .clicked()
        {
            self.queue_root_open(ui.ctx(), open_files::PickerKind::FigurePaneSelect);
            ui.close();
        }
        if ui
            .add_enabled(
                self.root_open_context(ui.ctx()),
                egui::Button::new("Open separate disk copy..."),
            )
            .clicked()
        {
            self.queue_root_open(ui.ctx(), open_files::PickerKind::FigurePaneCopy);
            ui.close();
        }
        if ui
            .add_enabled(
                !self.busy && !self.file_action_pending() && self.figure.is_some(),
                egui::Button::new("Save as…"),
            )
            .clicked()
        {
            self.queue_root_figure_save(ui.ctx(), true);
            ui.close();
        }
        if ui
            .add_enabled(
                self.can_reload_figure(),
                egui::Button::new("Reload from disk..."),
            )
            .clicked()
        {
            self.request_figure_reload();
            ui.close();
        }
        if ui
            .add_enabled(
                !self.busy && self.figure.is_some(),
                egui::Button::new("Properties…"),
            )
            .clicked()
            && let Some(f) = &self.figure
        {
            self.figure_editor = Some(figure_editor::Editor::new(
                self.current_figure,
                f,
                self.view_bounds
                    .or_else(|| self.subplot_canvas.active_bounds(f)),
            ));
        }

        if self
            .figure
            .as_ref()
            .is_some_and(|f| f.layout.as_ref().is_some_and(|l| l.tiled.is_some()))
            && ui
                .add_enabled(!self.busy, egui::Button::new("Next tile"))
                .clicked()
        {
            self.run("nexttile".into());
            ui.close();
        }
    }

    /// Card header name of the current figure: its file, else the figure's
    /// own name, else "Untitled"; a trailing `*` marks unsaved changes.
    fn figure_name(&self) -> String {
        let name = self
            .figure_documents
            .get(&self.current_figure)
            .and_then(|d| d.path.as_ref())
            .map(|p| crate::path_label::display(p).to_string())
            .or_else(|| {
                self.figure
                    .as_ref()
                    .map(|f| f.name.clone())
                    .filter(|n| !n.is_empty())
            })
            .unwrap_or_else(|| "Untitled".into());
        if self.figure_dirty(self.current_figure) {
            format!("{name} *")
        } else {
            name
        }
    }

    /// A recorded manual axis range that the painted `view` does not show.
    fn recorded_view_differs(&self, view: &figure_view::View) -> bool {
        self.figure_views
            .get(&self.current_figure)
            .is_some_and(|rec| {
                (0..2).any(|axis| {
                    !rec.auto[axis]
                        && (view.auto[axis]
                            || (0..2).any(|k| {
                                let (a, b) = (view.bounds[axis * 2 + k], rec.bounds[axis * 2 + k]);
                                (a - b).abs() > 1e-9 * a.abs().max(b.abs()).max(1.)
                            }))
                })
            })
    }

    fn figure_pane_contents(&mut self, figure_ui: &mut egui::Ui, scrollable: bool) {
        let mut command = None;
        let mut picked = None;
        let title = if self.figure.is_some() {
            format!("Figure {}", self.current_figure)
        } else {
            "Figure".into()
        };
        let name = self.figure.is_some().then(|| self.figure_name());
        panels::header(figure_ui, &title, name.as_deref(), |ui| {
            ui.spacing_mut().item_spacing.x = tokens::XS;
            let more = panels::icon_button(ui, panels::Glyph::More, "Actions");
            focus_areas::register(ui, &more, focus_areas::Area::Figure(self.current_figure));
            egui::Popup::menu(&more).show(|ui| {
                self.figure_menu_head(ui, &mut command);
            });
            let export = toolstrip::Action::ExportFigure;
            if export.enabled(self) && panels::tool_link(ui, "Export").clicked() {
                export.invoke(self, ui.ctx());
            }
            // More than one figure: the compact picker sits in the header.
            if self.figure_ids.len() > 1
                && let Some(id) = self.named_figure_selector(ui)
            {
                picked = Some(id);
            }
        });
        if let Some(picked) = picked.filter(|id| *id != self.current_figure) {
            self.document_save.activate(document_save::Pane::Figure);
            if self.detached.contains(picked) {
                self.focus_detached(picked, figure_ui.ctx());
            } else {
                self.run(format!("figure({picked})"));
            }
        }
        if tokens::compact(figure_ui.ctx()) {
            tokens::dense(figure_ui);
        }
        if self.detached.contains(self.current_figure) {
            figure_ui.label("This figure is open in its own window.");
            figure_ui.horizontal(|ui| {
                if ui.button("Show figure window").clicked() {
                    self.focus_detached(self.current_figure, ui.ctx());
                }
                if ui
                    .add_enabled(
                        !self.busy && !self.file_action_pending(),
                        egui::Button::new("Dock figure"),
                    )
                    .clicked()
                {
                    self.dock_figure(self.current_figure, ui.ctx());
                }
            });
            return;
        }
        self.sample_inspector_ui(figure_ui);
        if let Some(f) = &self.figure {
            let marker = self.sample_inspector.marker(self.current_figure, f);
            let position = f.active_position();
            if f.layout.is_none() {
                // Hidden series intentionally have no display geometry. Compare
                // only visible lengths: bounded O(series), without reading or
                // cloning the immutable full-resolution sample payloads.
                let visible_samples: usize = f
                    .series
                    .iter()
                    .filter(|series| series.visible)
                    .map(|series| series.points.len())
                    .sum();
                if visible_samples > self.plot_points.iter().map(Vec::len).sum::<usize>() {
                    let notice = format!(
                        "{visible_samples} visible samples (display simplified; saved data retains all samples)"
                    );
                    figure_ui
                        .add(egui::Label::new(&notice).truncate())
                        .on_hover_text(&notice);
                }
            }
            if f.layout.is_some() {
                let original = f.clone();
                let rendered = figure_ui.add_enabled_ui(!self.busy, |ui| {
                    self.subplot_canvas.show(
                        ui,
                        self.current_figure,
                        &original,
                        self.reset_plot,
                        marker,
                    )
                });
                plot_inspection_hint(figure_ui, &rendered.response);
                let updated = rendered.inner;
                self.reset_plot = false;
                self.restore_plot_view = false;
                // Subplot manual views live in each axes and are committed through
                // the worker, so save/export/recovery see the same whole figure.
                self.view_bounds = None;
                if let Some((position, query)) = self.subplot_canvas.pick.take() {
                    self.inspect_plot_sample(position, query);
                } else if !self.busy
                    && let Some(updated) = updated
                {
                    let edit = Box::new(worker::FigureEdit {
                        view_before: None,
                        id: self.current_figure,
                        original,
                        updated,
                    });
                    self.submit(if self.subplot_canvas.undoable_change {
                        Job::EditFigure(edit)
                    } else {
                        Job::ViewFigure(edit)
                    });
                } else if let Some(command) = command {
                    self.run(command);
                }
                return;
            }
            if scrollable {
                // Controls have their actual wrapped heights now. Grow only
                // the scrollable ordinary artwork, never a fixed axes position.
                figure_view::reserve_readable_title_canvas(figure_ui, f);
            }
            let plot_rect = figure_ui.available_rect_before_wrap();
            let settled = !self.reset_plot && !self.restore_plot_view;
            let rendered = figure_ui.add_enabled_ui(!self.busy, |ui| {
                figure_view::show_interactive(
                    ui,
                    &self.restored_figure_view(f),
                    &self.plot_points,
                    self.plot_axes,
                    self.reset_plot,
                    marker.map(|(_, point)| point),
                )
            });
            plot_inspection_hint(figure_ui, &rendered.response);
            let view = rendered.inner;
            if !self.busy
                && let Some(visible) = &view.visibility
            {
                let mut updated = f.clone();
                for (series, visible) in updated.series.iter_mut().zip(visible) {
                    series.visible = *visible;
                }
                let mut edit = worker::FigureEdit {
                    view_before: None,
                    id: self.current_figure,
                    original: f.clone(),
                    updated,
                };
                edit.preserve_manual_view(Some(view.bounds), view.auto);
                self.submit(Job::EditFigure(Box::new(edit)));
            }
            // The figure column also paints beside the Editor and Command
            // layouts. An untouched paint that shows another view than the
            // recorded manual one must not replace it: keep the record and
            // restore it on the next paint.
            let touched = figure_ui.input(|i| {
                i.pointer
                    .latest_pos()
                    .is_some_and(|p| plot_rect.contains(p))
                    && (i.pointer.any_down()
                        || i.pointer.any_released()
                        || i.smooth_scroll_delta != egui::Vec2::ZERO
                        || i.zoom_delta() != 1.)
            });
            if settled && !touched && self.recorded_view_differs(&view) {
                self.reset_plot = true;
                self.restore_plot_view = true;
                figure_ui.ctx().request_repaint();
            } else {
                self.record_figure_view(view.bounds, view.auto);
                self.view_bounds = Some(view.bounds);
                self.view_auto = view.auto;
                self.reset_plot = false;
                self.restore_plot_view = false;
            }
            if let Some(query) = view.pick {
                self.inspect_plot_sample(position, query);
            } else if let Some(command) = command {
                self.run(command);
            }
        } else {
            self.figure_empty_state(figure_ui);
        }
    }
}

fn figure_view_options(ui: &mut egui::Ui, busy: bool, f: &Figure, command: &mut Option<String>) {
    ui.menu_button("View options", |ui| {
        for (name, scale) in [("X", f.xscale), ("Y", f.yscale)] {
            let log = scale == matrust::plot::AxisScale::Log;
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(format!("{name}: {}", if log { "log" } else { "linear" })),
                )
                .clicked()
            {
                *command = Some(format!(
                    "{}scale('{}')",
                    name.to_lowercase(),
                    if log { "linear" } else { "log" }
                ));
            }
        }
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(match (f.xgrid, f.ygrid) {
                    (true, true) => "Grid: on",
                    (false, false) => "Grid: off",
                    _ => "Grid: mixed",
                }),
            )
            .clicked()
        {
            *command = Some("grid".to_owned());
        }
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(if f.legend {
                    "Legend: on"
                } else {
                    "Legend: off"
                }),
            )
            .clicked()
        {
            *command = Some(format!("legend('{}')", if f.legend { "off" } else { "on" }));
        }
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(if f.hold { "Hold: on" } else { "Hold: off" }),
            )
            .clicked()
        {
            *command = Some("hold".into());
        }
    });
}
