//! Panel-card chrome: card headers, ghost and primary buttons, the top bar
//! brand and tabs, the status bar and the Command Window welcome cards.
use super::egui::{self, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use super::tokens::{self, palette};
use super::*;

/// The example the welcome card runs.
pub(super) const EXAMPLE: &str = "x = linspace(0,1,1000); plot(x, sin(2*pi*5*x))";

pub(super) fn semibold_font(ui: &Ui, size: f32) -> egui::FontId {
    let family = egui::FontFamily::Name(tokens::SEMIBOLD.into());
    // A context that never ran `configure` (tests) has no SemiBold family.
    if ui
        .ctx()
        .fonts(|f| f.definitions().families.contains_key(&family))
    {
        tokens::semibold(size)
    } else {
        egui::FontId::proportional(size)
    }
}

/// Table column headers: CAPTION semibold.
pub(super) fn caption_semibold(ui: &Ui) -> egui::FontId {
    semibold_font(ui, tokens::CAPTION)
}

/// Accent square with "M" and the product name.
pub(super) fn brand(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.), Sense::hover());
    ui.painter()
        .rect_filled(rect, tokens::CONTROL_RADIUS, p.accent_fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "M",
        semibold_font(ui, tokens::BODY),
        p.on_accent,
    );
    ui.add_space(tokens::S);
    ui.label(
        RichText::new("MatRust")
            .font(semibold_font(ui, tokens::HEADING))
            .color(p.text),
    );
}

/// The 2 px accent ring of keyboard focus, from the theme's focus visuals.
pub(super) fn focus_ring(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            tokens::CONTROL_RADIUS,
            ui.visuals().widgets.active.bg_stroke,
            StrokeKind::Inside,
        );
    }
}

/// The one filled primary action: accent_fill with an on_accent label and a
/// small shortcut. Disabled, it uses the neutral style (surface fill, border,
/// text_disabled), never a pale accent.
pub(super) fn primary_button(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let label_color = if enabled {
        p.on_accent
    } else {
        p.text_disabled
    };
    let main = ui.painter().layout_no_wrap(
        label.to_owned(),
        semibold_font(ui, tokens::BODY),
        label_color,
    );
    let hint = ui.painter().layout_no_wrap(
        shortcut.to_owned(),
        egui::FontId::proportional(tokens::CAPTION),
        label_color,
    );
    let icon = 8.;
    let width = 14. + icon + tokens::S + main.size().x + tokens::S + hint.size().x + 14.;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.), Sense::click());
    let response = if enabled {
        response
    } else {
        // Disabled primary actions take no clicks.
        ui.interact(rect, response.id, Sense::hover())
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let hovered = enabled && (response.hovered() || response.is_pointer_button_down_on());
    // Dark accent_hover is too light under a white label: dark hover keeps
    // the fill and outlines it instead.
    let dark = ui.visuals().dark_mode;
    let fill = if !enabled {
        p.surface
    } else if hovered && !dark {
        p.accent_hover
    } else {
        p.accent_fill
    };
    let stroke = if !enabled {
        Stroke::new(1., p.border)
    } else if hovered && dark {
        Stroke::new(1., p.accent_hover)
    } else {
        Stroke::NONE
    };
    ui.painter().rect(
        rect,
        tokens::CONTROL_RADIUS,
        fill,
        stroke,
        StrokeKind::Inside,
    );
    focus_ring(ui, &response);
    let mut x = rect.left() + 14.;
    let y = rect.center().y;
    icons::paint(
        ui,
        Rect::from_center_size(egui::pos2(x + icon * 0.5, y), Vec2::splat(14.)),
        icons::Icon::Play,
        label_color,
    );
    x += icon + tokens::S;
    let main_width = main.size().x;
    ui.painter()
        .galley(egui::pos2(x, y - main.size().y * 0.5), main, label_color);
    x += main_width + tokens::S;
    ui.painter().galley(
        egui::pos2(x, y - hint.size().y * 0.5),
        hint,
        label_color.gamma_multiply(0.85),
    );
    response
}

/// Current folder in mono: muted parents, the last segment in text colour.
/// A click asks for another folder (Browse).
pub(super) fn crumb(ui: &mut Ui, path: &std::path::Path) -> Response {
    let p = palette(ui.ctx());
    let text = crate::path_label::display(path).to_string();
    let split = text
        .trim_end_matches(['/', '\\'])
        .rfind(['/', '\\'])
        .map_or(0, |i| i + 1);
    let font = egui::FontId::monospace(tokens::SMALL_CODE);
    let mut job = egui::text::LayoutJob::default();
    let parent = if ui.ctx().content_rect().width() < tokens::OVERFLOW_WIDTH {
        ""
    } else {
        &text[..split]
    };
    for (part, color) in [(parent, p.text_muted), (&text[split..], p.text)] {
        job.append(
            part,
            0.,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    }
    // Truncated to the free space: a long path never covers the search field.
    ui.add(
        egui::Label::new(job)
            .truncate()
            .selectable(false)
            .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(format!("{text}\nClick to browse for another folder"))
}

/// Height of a card header (denser in compact windows).
pub(super) fn header_height(ctx: &egui::Context) -> f32 {
    if tokens::compact(ctx) {
        tokens::INTERACT_HEIGHT + tokens::S
    } else {
        tokens::PANEL_HEADER
    }
}

/// Card header: 36 px, BODY semibold title, muted count, right-aligned tools
/// added by `tools` (right to left) and a 1 px bottom border.
pub(super) fn header<R>(
    ui: &mut Ui,
    title: &str,
    count: Option<&str>,
    tools: impl FnOnce(&mut Ui) -> R,
) -> R {
    header_with(ui, title, count, false, tools)
}

/// `header` whose title is accent_text while its pane is the active one.
pub(super) fn header_with<R>(
    ui: &mut Ui,
    title: &str,
    count: Option<&str>,
    active: bool,
    tools: impl FnOnce(&mut Ui) -> R,
) -> R {
    let p = palette(ui.ctx());
    let height = header_height(ui.ctx());
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height));
    #[cfg(test)]
    log_header(ui.ctx(), title, rect);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1., p.hairline),
    );
    // The tools claim their width first; the title and count truncate into
    // what is left, so a long name never covers a tool.
    let inner = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(tokens::M, 0.)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                let result = tools(ui);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // One caption style on every pane header (plan section 6):
                    // 11 Semibold uppercase, text 2.
                    ui.add(
                        egui::Label::new(
                            RichText::new(title.to_uppercase())
                                .font(caption_semibold(ui))
                                .color(if active { p.accent_text } else { p.text_muted }),
                        )
                        .truncate(),
                    );
                    // The count is dropped, never cut: no half-words in chrome.
                    if let Some(count) = count {
                        let text = RichText::new(count)
                            .size(tokens::CAPTION)
                            .color(p.text_muted);
                        let width = ui
                            .painter()
                            .layout_no_wrap(
                                count.to_owned(),
                                egui::FontId::proportional(tokens::CAPTION),
                                p.text_muted,
                            )
                            .size()
                            .x;
                        if width <= ui.available_width() {
                            ui.label(text);
                        }
                    }
                });
                result
            },
        )
        .inner;
    ui.advance_cursor_after_rect(rect);
    inner
}

/// Pane headers of the latest pass: (pass, [(title, rect)]).
#[cfg(test)]
type HeaderLog = (u64, Vec<(String, Rect)>);

#[cfg(test)]
fn log_header(ctx: &egui::Context, title: &str, rect: Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let log = d.get_temp_mut_or_default::<HeaderLog>(egui::Id::new("pane_header_log"));
        if log.0 != pass {
            *log = (pass, Vec::new());
        }
        log.1.push((title.to_owned(), rect));
    });
}

/// Title and rect of every pane header painted in the latest pass.
#[cfg(test)]
pub(super) fn painted_headers(ctx: &egui::Context) -> Vec<(String, Rect)> {
    ctx.data(|d| d.get_temp::<HeaderLog>(egui::Id::new("pane_header_log")))
        .map(|log| log.1)
        .unwrap_or_default()
}

/// Width of a `header` title, for tools that must leave it whole.
pub(super) fn header_title_width(ui: &Ui, title: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            title.to_uppercase(),
            caption_semibold(ui),
            palette(ui.ctx()).text_muted,
        )
        .size()
        .x
}

/// Ghost tool link in a card header: muted text, text colour on hover.
pub(super) fn tool_link(ui: &mut Ui, label: &str) -> Response {
    link(ui, label, label)
}

/// Width `tool_link` and `header_tool` take for `text`.
pub(super) fn tool_link_width(ui: &Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(tokens::CAPTION),
            egui::Color32::WHITE,
        )
        .size()
        .x
        + tokens::S
}

/// A text button of a pane header: a short visible word (`text`, as
/// "Find" or "More") with the accessible `name` and a tooltip. Pane headers
/// never carry icon-only buttons.
pub(super) fn header_tool(ui: &mut Ui, text: &str, name: &str, tooltip: &str) -> Response {
    link(ui, text, name)
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
}

fn link(ui: &mut Ui, text: &str, name: &str) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(tokens::CAPTION),
        p.text_muted,
    );
    // A 24 px hit target; the caption stays centred in it.
    let size = Vec2::new(
        galley.size().x + tokens::S,
        (galley.size().y + tokens::XS).max(tokens::INTERACT_HEIGHT),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    // Tests find a header button by its accessible name: an invisible text
    // shape where the name differs from the visible word.
    #[cfg(test)]
    if name != text {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(1.),
            egui::Color32::TRANSPARENT,
        );
    }
    let color = if !enabled {
        p.text_disabled
    } else if response.hovered() {
        p.text
    } else {
        p.text_muted
    };
    focus_ring(ui, &response);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, color);
    response
}

/// How many header tools of `widths` (most important first) fit in `room`.
/// When some do not, a "More" menu of `more` width takes the rest and the
/// second value is true.
pub(super) fn header_fit(widths: &[f32], room: f32, more: f32) -> (usize, bool) {
    if widths.iter().sum::<f32>() <= room {
        return (widths.len(), false);
    }
    let mut used = more;
    let shown = widths
        .iter()
        .take_while(|width| {
            used += **width;
            used <= room
        })
        .count();
    (shown, true)
}

/// A panel whose edge is a pane boundary.
pub(super) trait PaneBorder: Sized {
    /// `show` with the panel edge drawn as the 1 px `pane_border`, which
    /// keeps >= 3:1 against both surfaces it separates; dividers inside the
    /// panel keep the hairline.
    fn show_pane<R>(
        self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> egui::InnerResponse<R> {
        self.show_edge(ui, true, add_contents)
    }

    /// `show_pane` when `pane`, else `show` with the hairline edge.
    fn show_edge<R>(
        self,
        ui: &mut Ui,
        pane: bool,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> egui::InnerResponse<R>;
}

impl PaneBorder for egui::Panel {
    fn show_edge<R>(
        self,
        ui: &mut Ui,
        pane: bool,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> egui::InnerResponse<R> {
        // egui draws the separator with the parent's noninteractive stroke.
        let inner = ui.visuals().widgets.noninteractive.bg_stroke;
        if pane {
            ui.visuals_mut().widgets.noninteractive.bg_stroke =
                Stroke::new(1., palette(ui.ctx()).pane_border);
        }
        let response = self.show_separator_line(true).show(ui, |ui| {
            ui.visuals_mut().widgets.noninteractive.bg_stroke = inner;
            add_contents(ui)
        });
        ui.visuals_mut().widgets.noninteractive.bg_stroke = inner;
        response
    }
}

/// Mono size label in the chip's place (right-aligned, text 2, no frame).
pub(super) fn chip(ui: &Ui, right_center: egui::Pos2, text: &str, clip: Rect) -> Rect {
    let p = palette(ui.ctx());
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::monospace(tokens::CAPTION),
        p.text_muted,
    );
    let size = (galley.size() + Vec2::new(tokens::M, tokens::XS / 2.))
        .max(Vec2::new(tokens::SIZE_CHIP_MIN_WIDTH, 0.));
    let rect = Rect::from_min_size(
        egui::pos2(right_center.x - size.x, right_center.y - size.y * 0.5),
        size,
    );
    // Plain mono text in text 2, right-aligned: no pills or chips.
    let painter = ui.painter().with_clip_rect(clip);
    painter.galley(
        egui::pos2(
            rect.right() - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        p.text_muted,
    );
    rect
}

/// Card for the welcome row; `soft` is the accent_soft example card.
fn start_card(ui: &mut Ui, title: &str, detail: &str, soft: bool) -> Response {
    let p = palette(ui.ctx());
    // A plain card; the suggested example only colours its code line, so it
    // never reads as a selected row.
    let (fill, title_color) = (p.surface_sunken, p.text);
    // Code is mono only (plan section 6): the example line is 13 mono text 2.
    let detail_color = p.text_muted;
    let detail_font = if soft {
        egui::FontId::monospace(tokens::CODE)
    } else {
        egui::FontId::proportional(tokens::CAPTION)
    };
    let title_galley = ui.painter().layout_no_wrap(
        title.to_owned(),
        semibold_font(ui, tokens::BODY),
        title_color,
    );
    let detail_galley = ui
        .painter()
        .layout_no_wrap(detail.to_owned(), detail_font, detail_color);
    let width = (title_galley.size().x.max(detail_galley.size().x) + 2. * tokens::M).max(150.);
    // One 48 px row on the 8 px grid; the two lines centre in it.
    let lines = title_galley.size().y + detail_galley.size().y + 2.;
    let height = 48_f32.max((lines + 2. * tokens::XS).ceil());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    let stroke = if response.hovered() {
        Stroke::new(1., p.accent)
    } else {
        Stroke::new(1., p.border)
    };
    ui.painter().rect(
        rect,
        tokens::CONTROL_RADIUS,
        fill,
        stroke,
        StrokeKind::Inside,
    );
    focus_ring(ui, &response);
    let top = rect.left_top() + Vec2::new(tokens::M, ((height - lines) / 2.).round());
    let title_height = title_galley.size().y;
    ui.painter().galley(top, title_galley, title_color);
    ui.painter().galley(
        top + Vec2::new(0., title_height + 2.),
        detail_galley,
        detail_color,
    );
    response
}

impl Workspace {
    /// Status bar on the canvas: state dot, status and run time on the left;
    /// variable count, current folder and encoding on the right.
    pub(super) fn status_bar(&mut self, ui: &mut Ui, compact: bool) {
        let p = palette(ui.ctx());
        ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
        if compact {
            tokens::dense(ui);
        }
        ui.visuals_mut().override_text_color = Some(p.text_muted);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::L;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.), Sense::hover());
                ui.painter().circle_filled(
                    dot.center(),
                    4.,
                    if self.busy { p.accent } else { p.success },
                );
                // While busy the status text says what is running.
                if !self.busy {
                    ui.label("Ready");
                }
            });
            if self.recovery_blocked {
                ui.colored_label(p.error, "Recovery disabled")
                    .on_hover_text("Session recovery was incomplete. The earlier checkpoint is preserved. Save current work explicitly; see the command window for details.");
            }
            if self.busy {
                ui.spinner();
            }
            // The open app's status (filter design summary) sits here, not
            // in its work area.
            if self.mode() == crate::modes::Mode::Apps
                && let Some(tool) = self.toolkits.selected_tool()
                && let Some((text, status)) = self.toolkits.app_status(tool, self.busy)
            {
                if status == toolkits::AppStatus::Plain {
                    ui.label(text);
                } else {
                    ui.colored_label(status.color(ui.ctx()), text);
                }
            }
            if !self.status.is_empty() {
                ui.add(egui::Label::new(&self.status).truncate());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("UTF-8");
                if !compact {
                    ui.add(
                        egui::Label::new(crate::path_label::display(&self.folder).to_string())
                            .truncate(),
                    );
                }
                ui.label(chrome::variable_count(self.variables.len()));
            });
        });
    }

    /// The welcome block shows in the main Command Window and disappears
    /// once anything has run.
    pub(super) fn welcome_visible(&self) -> bool {
        self.docked_tool().is_none()
            && !self.apps_home
            && self.central_command()
            && !self.editor_above_command()
            && self.history.entries.is_empty()
            && self.variables.is_empty()
            && self.all_figures.is_empty()
    }

    /// Welcome block at the top of an empty Command Window: version line and
    /// three start cards in a left-aligned row.
    pub(super) fn welcome(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let rect = egui::Frame::new()
            .inner_margin(egui::Margin::same(tokens::L as i8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("MatRust {}", matrust::VERSION))
                            .font(semibold_font(ui, tokens::HEADING))
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new("alpha · subset of the MATLAB language").color(p.text_muted),
                    );
                });
                ui.add_space(tokens::XS);
                ui.horizontal_wrapped(|ui| {
                    if start_card(ui, "New script", "Ctrl N", false).clicked() {
                        self.new_document("");
                    }
                    if start_card(ui, "Open folder", "Your .m files and data", false).clicked() {
                        let ctx = ui.ctx().clone();
                        if self.folder_browse_context(&ctx) {
                            self.queue_folder_browse(&ctx);
                        }
                    }
                    if start_card(ui, "Try an example", "plot(x, sin(2*pi*5*x))", true).clicked()
                        && !self.busy
                    {
                        let command = EXAMPLE.to_owned();
                        self.console.push(format!(">> {command}"));
                        self.history.push(command.clone());
                        self.recovery_dirty = true;
                        self.run(command);
                    }
                });
            })
            .response
            .rect;
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1., p.hairline),
        );
    }
}

/// Action-row button height: one 28 px control.
pub(super) const STRIP_TILE: f32 = tokens::CONTROL;

/// Action-row icons are 16 px line glyphs left of the label.
const ROW_ICON: f32 = 16.;

fn strip_font() -> egui::FontId {
    egui::FontId::proportional(tokens::BODY)
}

/// Width of one icon + label action; labels never wrap.
pub(super) fn strip_action_width(ui: &Ui, label: &str) -> f32 {
    let width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), strip_font(), egui::Color32::WHITE)
        .size()
        .x;
    tokens::S + ROW_ICON + tokens::S + width + tokens::M
}

/// Ghost action-row button: a 16 px stroke icon (accent for Run) and the
/// label in text 1; hover is a surface step, no outline.
pub(super) fn strip_action(ui: &mut Ui, label: &str, icon: shell::Icon, run: bool) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let width = strip_action_width(ui, label);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, STRIP_TILE), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if enabled && response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, tokens::CONTROL_RADIUS, p.hairline);
    } else if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, tokens::CONTROL_RADIUS, p.canvas);
    }
    focus_ring(ui, &response);
    let icon_color = if !enabled {
        p.text_disabled
    } else if run {
        p.accent
    } else {
        p.text_muted
    };
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + tokens::S + ROW_ICON * 0.5, rect.center().y),
        Vec2::splat(ROW_ICON),
    );
    stroke_icon(ui, icon_rect, icon, icon_color);
    ui.painter().text(
        egui::pos2(icon_rect.right() + tokens::S, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        strip_font(),
        if enabled { p.text } else { p.text_disabled },
    );
    response
}

/// 1 px vertical hairline between action groups.
pub(super) fn strip_divider(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(2. * tokens::S + 1., STRIP_TILE), Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(x, rect.center().y - 8.),
            egui::pos2(x, rect.center().y + 8.),
        ],
        Stroke::new(1., palette(ui.ctx()).hairline),
    );
}

/// Phosphor Regular icons in the existing 16 px action boxes.
pub(super) fn stroke_icon(ui: &Ui, rect: Rect, icon: shell::Icon, color: egui::Color32) {
    use super::icons::{self, Icon};
    let icon = match icon {
        shell::Icon::Document => Icon::FilePlus,
        shell::Icon::Folder => Icon::FolderOpen,
        shell::Icon::Save => Icon::FloppyDisk,
        shell::Icon::Run => Icon::Play,
        shell::Icon::Table => Icon::Table,
        shell::Icon::Plot => Icon::ChartLine,
        shell::Icon::Layout => Icon::Sidebar,
        shell::Icon::Undo => Icon::ArrowUUpLeft,
        shell::Icon::Redo => Icon::ArrowUUpRight,
        shell::Icon::Refresh => Icon::ArrowClockwise,
        shell::Icon::Export => Icon::Export,
        shell::Icon::Properties => Icon::SlidersHorizontal,
        shell::Icon::Simulation => Icon::FlowArrow,
        shell::Icon::Filter => Icon::Funnel,
        shell::Icon::Copy => Icon::Copy,
        shell::Icon::More => Icon::DotsThree,
        shell::Icon::Search => Icon::MagnifyingGlass,
    };
    icons::paint(ui, rect, icon, color);
}

#[derive(Clone, Copy)]
pub(super) enum FileKind {
    Script,
    Folder,
    Data,
    Csv,
    Image,
}

/// Files row: neutral Phosphor file-type icon and truncated name; selection
/// uses a tint and a 2 px accent marker.
pub(super) fn file_row(
    ui: &mut Ui,
    label: &str,
    kind: FileKind,
    selected: bool,
    height: f32,
) -> Response {
    let p = palette(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::CLICK);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let fill = if selected {
        p.accent_soft
    } else if response.hovered() {
        p.canvas
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0., fill);
    if selected {
        // Selected: selection tint and a 2 px accent bar at the left.
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(2., rect.height())),
            0.,
            p.accent,
        );
    }
    focus_ring(ui, &response);
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + tokens::M + 7., rect.center().y),
        Vec2::splat(14.),
    );
    file_glyph(ui, icon, kind, p.text_muted);
    let color = p.text;
    let left = icon.right() + tokens::S;
    let mut job = egui::text::LayoutJob::single_section(
        label.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(tokens::BODY), color),
    );
    job.wrap =
        egui::text::TextWrapping::truncate_at_width((rect.right() - tokens::S - left).max(0.));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(left, rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    response
}

/// Phosphor file marks in text 2, with the existing row geometry.
fn file_glyph(ui: &Ui, r: Rect, kind: FileKind, color: egui::Color32) {
    use super::icons::{self, Icon};
    let icon = match kind {
        FileKind::Script => Icon::FileCode,
        FileKind::Folder => Icon::FolderOpen,
        FileKind::Data => Icon::Table,
        FileKind::Csv => Icon::FileCsv,
        FileKind::Image => Icon::Image,
    };
    icons::paint(ui, r, icon, color);
}

/// Search launcher drawn as a 320 px field with its Ctrl K hint.
pub(super) fn search_field(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    // The folder path next to it truncates first.
    let width = tokens::SEARCH_WIDTH
        .min(ui.available_width() - 120.)
        .max(120.);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.), Sense::click());
    let response = if enabled {
        response
    } else {
        ui.interact(rect, response.id, Sense::hover())
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let stroke = if response.hovered() && enabled {
        Stroke::new(1., p.text_disabled)
    } else {
        Stroke::new(1., p.border)
    };
    ui.painter().rect(
        rect,
        tokens::CONTROL_RADIUS,
        p.surface,
        stroke,
        StrokeKind::Inside,
    );
    focus_ring(ui, &response);
    let kbd = ui.painter().layout_no_wrap(
        "Ctrl K".into(),
        egui::FontId::proportional(tokens::CAPTION),
        p.text_muted,
    );
    let kbd_rect = Rect::from_min_size(
        egui::pos2(
            rect.right() - 10. - kbd.size().x - 8.,
            rect.center().y - kbd.size().y * 0.5 - 1.,
        ),
        kbd.size() + Vec2::new(8., 2.),
    );
    ui.painter().rect_stroke(
        kbd_rect,
        tokens::CHIP_RADIUS,
        Stroke::new(1., p.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(kbd_rect.min + Vec2::new(4., 1.), kbd, p.text_muted);
    let mut job = egui::text::LayoutJob::single_section(
        label.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(tokens::BODY), p.text_disabled),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(
        (kbd_rect.left() - rect.left() - 10. - tokens::S).max(0.),
    );
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(rect.left() + 10., rect.center().y - galley.size().y * 0.5),
        galley,
        p.text_disabled,
    );
    response
}
