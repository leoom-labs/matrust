//! The four workbench modes (Command, Editor, Figures, Apps): one top-bar
//! switcher, Ctrl+1..4, and the Apps launcher. A mode is a view of the
//! existing layout state: the Layout presets keep working and set a mode.
use super::egui::{self, Rect, Response, RichText, Sense, Ui, Vec2};
use super::tokens::{self, palette};
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Command,
    Editor,
    Figures,
    Apps,
}

impl Mode {
    pub(super) const ALL: [Self; 4] = [Self::Command, Self::Editor, Self::Figures, Self::Apps];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Command => "Command",
            Self::Editor => "Editor",
            Self::Figures => "Figures",
            Self::Apps => "Apps",
        }
    }
    fn shortcut(self) -> &'static str {
        match self {
            Self::Command => "Ctrl+1",
            Self::Editor => "Ctrl+2",
            Self::Figures => "Ctrl+3",
            Self::Apps => "Ctrl+4",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Command => "Command Window, Files and Workspace",
            Self::Editor => "Script editor with the Command drawer",
            Self::Figures => "The current figure on its own canvas",
            Self::Apps => "Filter Designer, Block Simulation and examples",
        }
    }
}

/// Example scripts of the Apps launcher and the Examples menu.
pub(super) const EXAMPLES: [(&str, &str); 13] = [
    ("Damped oscillation", DEMO),
    (
        "Linear system",
        include_str!("../../examples/linear_system.m"),
    ),
    (
        "Digital filtering",
        include_str!("../../examples/filter_design.m"),
    ),
    (
        "Streaming filter",
        include_str!("../../examples/streaming_filter.m"),
    ),
    (
        "Logical selection",
        include_str!("../../examples/logical_selection.m"),
    ),
    (
        "Loops and filtering",
        include_str!("../../examples/control_flow_filtering.m"),
    ),
    (
        "User functions",
        include_str!("../../examples/user_functions.m"),
    ),
    (
        "Multiple figures",
        include_str!("../../examples/multiple_plots.m"),
    ),
    ("Subplots", include_str!("../../examples/subplots.m")),
    (
        "Positioned axes / FIG",
        include_str!("../../examples/fig_layouts.m"),
    ),
    (
        "Tiled layouts",
        include_str!("../../examples/tiled_layout.m"),
    ),
    (
        "Discrete signals",
        include_str!("../../examples/discrete_plots.m"),
    ),
    (
        "Logarithmic plots",
        include_str!("../../examples/log_plots.m"),
    ),
];

const APPS: [(toolkits::Tool, &str); 2] = [
    (
        toolkits::Tool::Designer,
        "FIR and IIR filters in Hz: response, coefficients, design scripts.",
    ),
    (
        toolkits::Tool::Model,
        "Block diagrams: sources, filters and scopes, simulated in steps.",
    ),
];

/// The built-in tools, then every built framework app.
fn apps() -> impl Iterator<Item = (toolkits::Tool, &'static str)> {
    APPS.into_iter()
        .chain(crate::apps::AppKind::built().map(|kind| (toolkits::Tool::App(kind), kind.detail())))
}

impl Workspace {
    /// The mode the window shows now.
    pub(super) fn mode(&self) -> Mode {
        if self.docked_tool().is_some() || self.apps_home {
            return Mode::Apps;
        }
        if self.central_command() {
            return if self.editor_above_command() {
                Mode::Editor
            } else {
                Mode::Command
            };
        }
        match self.layout.mode {
            layout::PaneMode::Figure => Mode::Figures,
            _ => Mode::Editor,
        }
    }

    /// Switches mode the way its Layout preset does.
    pub(super) fn set_mode(&mut self, mode: Mode) {
        self.navigation.inspector_open = false;
        match mode {
            Mode::Command => {
                self.show_command_center();
                self.command_only = true;
            }
            Mode::Editor => {
                self.apps_home = false;
                if self.docked_tool().is_some() {
                    self.toolkits.deselect_tool();
                }
                self.command_only = false;
                if self.editor_window.detached {
                    // Focus the separate Editor and leave the main window on Command.
                    if let Some(center) = self.layout.center.as_mut() {
                        center.command = true;
                    }
                    self.navigation.command = true;
                    self.reveal_editor_window();
                } else if self.mode() != Mode::Editor {
                    self.reveal_editor();
                } else {
                    self.document_save.activate(document_save::Pane::Script);
                }
            }
            Mode::Figures => {
                self.navigation.drawer_collapsed = true;
                self.select_document_layout(layout::PaneMode::Figure);
                self.document_save.activate(document_save::Pane::Figure);
            }
            Mode::Apps => {
                self.navigation.drawer_collapsed = true;
                // The most recently opened app comes back; else the launcher.
                if let Some(tool) = self.toolkits.tabs().last() {
                    self.select_tool_tab(tool);
                } else {
                    self.toolkits.deselect_tool();
                    self.document_save.command_selected();
                    self.apps_home = true;
                }
            }
        }
        // The saved toolstrip tab follows the mode's action row.
        self.layout.toolstrip = match mode {
            Mode::Command | Mode::Editor => matrust::session::ToolstripTab::Home,
            Mode::Figures => matrust::session::ToolstripTab::Plots,
            Mode::Apps => matrust::session::ToolstripTab::Apps,
        };
        self.recovery_dirty = true;
    }

    /// Ctrl+1..4 switch modes, like the switcher.
    pub(super) fn mode_shortcuts(&mut self, ctx: &egui::Context, pending: bool) {
        if pending {
            return;
        }
        let chosen = ctx.input_mut(|i| {
            [
                (egui::Key::Num1, Mode::Command),
                (egui::Key::Num2, Mode::Editor),
                (egui::Key::Num3, Mode::Figures),
                (egui::Key::Num4, Mode::Apps),
            ]
            .into_iter()
            .find(|(key, _)| i.consume_key(egui::Modifiers::CTRL, *key))
            .map(|(_, mode)| mode)
        });
        if let Some(mode) = chosen {
            if self.global_search.is_open() {
                self.global_search.close(ctx);
            }
            self.set_mode(mode);
        }
    }

    /// Segmented switcher in the top bar: icon and label; the active mode is
    /// text 1 on a 2 px accent underline.
    pub(super) fn mode_switcher(&mut self, ui: &mut Ui, height: f32) {
        let current = self.mode();
        let compact = ui.ctx().content_rect().width() < 820.;
        for mode in Mode::ALL {
            let response = switcher_tab(ui, mode, mode == current, height, compact).on_hover_text(
                format!("{} ({})\n{}", mode.label(), mode.shortcut(), mode.hint()),
            );
            if response.clicked() {
                if self.global_search.is_open() {
                    self.global_search.close(ui.ctx());
                }
                self.set_mode(mode);
            }
        }
    }

    /// Apps Navigator: the apps (open ones carry a close mark) and the
    /// example scripts, which open in the Editor.
    pub(super) fn apps_navigator(&mut self, ui: &mut Ui) {
        let pending = self.file_action_pending() || self.allow_close;
        egui::ScrollArea::vertical()
            .id_salt("apps_navigator")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                section_caption(ui, "APPS");
                let selected = self.toolkits.selected_tool();
                for (tool, _) in apps() {
                    let open = self.toolkits.is_open(tool);
                    let label = if open {
                        self.toolkits.tab_label(tool)
                    } else {
                        tool.label().to_owned()
                    };
                    let (row, close) = nav_row(ui, &label, selected == Some(tool), open, !pending);
                    if close {
                        self.close_tool_tab(tool);
                    } else if row.clicked() {
                        self.open_app(tool);
                    }
                }
                ui.add_space(tokens::S);
                section_caption(ui, "EXAMPLES");
                for (label, source) in [
                    (
                        "Filtering example",
                        include_str!("../../examples/streaming_filter.m"),
                    ),
                    (
                        "Plotting example",
                        include_str!("../../examples/tiled_layout.m"),
                    ),
                ]
                .into_iter()
                .chain(EXAMPLES)
                {
                    let (row, _) = nav_row(ui, label, false, false, !pending && !self.busy);
                    if row
                        .on_hover_text("Opens the example script in the Editor")
                        .clicked()
                    {
                        self.apps_home = false;
                        self.new_document(source);
                    }
                }
            });
    }

    /// Opens (or shows) one app in the work area.
    pub(super) fn open_app(&mut self, tool: toolkits::Tool) {
        self.apps_home = false;
        self.navigation.drawer_collapsed = true;
        match tool {
            toolkits::Tool::Designer | toolkits::Tool::App(_) => {
                if self.toolkits.is_open(tool) {
                    self.select_tool_tab(tool);
                } else {
                    self.toolkits.open_tool(tool);
                }
            }
            toolkits::Tool::Model => self.show_model_document(),
        }
    }

    /// The Apps work area before an app is chosen: one titled section per
    /// app and the example scripts.
    pub(super) fn apps_home_ui(&mut self, ui: &mut Ui) {
        let separate = self.separate_windows();
        let p = palette(ui.ctx());
        ui.painter()
            .rect_filled(ui.max_rect(), 0., p.surface_sunken);
        egui::ScrollArea::vertical()
            .id_salt("apps_home")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(tokens::XL as i8))
                    .show(ui, |ui| {
                        ui.set_max_width(720.);
                        ui.label(
                            RichText::new("Apps")
                                .font(panels::semibold_font(ui, tokens::HEADING))
                                .color(p.text),
                        );
                        let where_ = if separate {
                            "Each app opens in its own window; its parameters sit beside it."
                        } else {
                            "Each app opens in this work area; its parameters sit beside it."
                        };
                        ui.label(RichText::new(where_).color(p.text_muted));
                        ui.add_space(tokens::L);
                        for (tool, detail) in apps() {
                            section(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(tool.label())
                                                .font(panels::semibold_font(ui, tokens::BODY))
                                                .color(p.text),
                                        );
                                        ui.label(RichText::new(detail).color(p.text_muted));
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(format!(
                                                        "Open {}",
                                                        tool.label()
                                                    ))
                                                    .min_size(egui::vec2(0., tokens::CONTROL)),
                                                )
                                                .clicked()
                                            {
                                                self.open_app(tool);
                                            }
                                        },
                                    );
                                });
                            });
                            ui.add_space(tokens::L);
                        }
                        section(ui, |ui| {
                            ui.label(
                                RichText::new("EXAMPLES")
                                    .font(panels::caption_semibold(ui))
                                    .color(p.text_muted),
                            );
                            ui.add_space(tokens::S);
                            // The Navigator is the one Examples list (plan section 4).
                            ui.label(
                                RichText::new(format!(
                                    "{} example scripts are listed in the Navigator under \
                                     EXAMPLES. Each opens as a new script in the Editor.",
                                    EXAMPLES.len() + 2
                                ))
                                .color(p.text_muted),
                            );
                        });
                    });
            });
    }
}

/// A titled work-area section: surface 1 on the surface 2 work background,
/// 16 px padding, no frame.
pub(super) fn section<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    // A surface-1 block on the surface-2 work area: the surface step separates
    // blocks (no card frame); inputs, selects and buttons are 28 px.
    egui::Frame::new()
        .fill(p.surface)
        .inner_margin(egui::Margin::same(tokens::L as i8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().interact_size.y = tokens::CONTROL;
            add(ui)
        })
        .inner
}

/// Pane caption: CAPTION semibold, text 2, 12 px inset, 32 px tall.
pub(super) fn section_caption(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), tokens::PANEL_HEADER),
        Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(rect.left() + tokens::M, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        panels::caption_semibold(ui),
        p.text_muted,
    );
}

/// A 24 px Navigator row: selection tint with a 2 px accent bar; an open
/// app carries a close mark at the right. Returns (row, close clicked).
pub(super) fn nav_row(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    closable: bool,
    enabled: bool,
) -> (Response, bool) {
    let p = palette(ui.ctx());
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), tokens::ROW), Sense::hover());
    let response = ui.interact(
        rect,
        egui::Id::new(("app_nav", label)),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0., p.accent_soft);
        painter.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(2., rect.height())),
            0.,
            p.accent,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, 0., p.canvas);
    }
    panels::focus_ring(ui, &response);
    let mut close = false;
    let mut right = rect.right() - tokens::S;
    if closable {
        let close_rect = Rect::from_center_size(
            egui::pos2(rect.right() - tokens::M - 4., rect.center().y),
            Vec2::splat(20.),
        );
        let close_response = ui
            .interact(close_rect, response.id.with("close"), Sense::click())
            .on_hover_text(format!("Close {label}"));
        if close_response.hovered() {
            ui.painter()
                .rect_filled(close_rect, tokens::CONTROL_RADIUS, p.hairline);
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(16.),
            p.text_muted,
        );
        close = enabled && close_response.clicked();
        right = close_rect.left() - tokens::XS;
    }
    let font = if selected {
        panels::semibold_font(ui, tokens::BODY)
    } else {
        egui::FontId::proportional(tokens::BODY)
    };
    let color = if enabled { p.text } else { p.text_disabled };
    let left = rect.left() + tokens::M + tokens::XS;
    let mut job = egui::text::LayoutJob::single_section(
        label.to_owned(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(0.));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(left, rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    (response, close)
}

fn switcher_tab(ui: &mut Ui, mode: Mode, selected: bool, height: f32, compact: bool) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let font = panels::semibold_font(ui, tokens::BODY);
    let color = if !enabled {
        p.text_disabled
    } else if selected {
        p.text
    } else {
        p.text_muted
    };
    let galley = ui
        .painter()
        .layout_no_wrap(mode.label().to_owned(), font, color);
    let icon = 16.;
    let pad = if compact { tokens::S } else { tokens::M };
    let width = pad + icon + tokens::S + galley.size().x + pad;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, mode.label())
    });
    if enabled && response.hovered() && !selected {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(0., tokens::XS)),
            tokens::CONTROL_RADIUS,
            p.surface_sunken,
        );
    }
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_max(
                egui::pos2(rect.left() + tokens::XS, rect.bottom() - 2.),
                rect.right_bottom() - Vec2::new(tokens::XS, 0.),
            ),
            0.,
            p.accent,
        );
    }
    panels::focus_ring(ui, &response);
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + pad + icon * 0.5, rect.center().y),
        Vec2::splat(icon),
    );
    mode_icon(
        ui,
        icon_rect,
        mode,
        if selected && enabled { p.accent } else { color },
    );
    ui.painter().galley(
        egui::pos2(
            icon_rect.right() + tokens::S,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        color,
    );
    response
}

/// Phosphor glyphs of the four modes.
fn mode_icon(ui: &Ui, rect: Rect, mode: Mode, color: egui::Color32) {
    use super::icons::{self, Icon};
    let icon = match mode {
        Mode::Command => Icon::Terminal,
        Mode::Editor => Icon::Code,
        Mode::Figures => Icon::ChartLine,
        Mode::Apps => Icon::SquaresFour,
    };
    icons::paint(
        ui,
        Rect::from_center_size(rect.center(), Vec2::splat(20.)),
        icon,
        color,
    );
}
