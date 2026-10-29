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
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.), Sense::hover());
    ui.painter()
        .rect_filled(rect, tokens::CONTROL_RADIUS, p.accent_fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "M",
        semibold_font(ui, tokens::CAPTION + 1.),
        p.on_accent,
    );
    ui.label(
        RichText::new("MatRust")
            .font(semibold_font(ui, tokens::BODY + 1.))
            .color(p.text),
    );
}

/// Top-bar tab: muted text; the selected one sits on a surface chip with the
/// 1 px border.
pub(super) fn ghost_tab(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let p = palette(ui.ctx());
    let font = semibold_font(ui, tokens::BODY);
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        font,
        if selected { p.text } else { p.text_muted },
    );
    let size = galley.size() + Vec2::new(2. * tokens::M, tokens::M);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    if selected {
        ui.painter().rect(
            rect,
            tokens::CONTROL_RADIUS,
            p.surface,
            Stroke::new(1., p.border),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, tokens::CONTROL_RADIUS, p.surface_sunken);
    }
    focus_ring(ui, &response);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, p.text_muted);
    response
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
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x, y - icon * 0.5),
            egui::pos2(x + icon, y),
            egui::pos2(x, y + icon * 0.5),
        ],
        label_color,
        Stroke::NONE,
    ));
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
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1., p.border),
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
                    ui.add(
                        egui::Label::new(
                            RichText::new(title)
                                .font(semibold_font(ui, tokens::BODY))
                                .color(if active { p.accent_text } else { p.text }),
                        )
                        .truncate(),
                    );
                    // The count is dropped, never cut: no half-words in chrome.
                    if let Some(count) = count {
                        let text = RichText::new(count)
                            .size(tokens::CAPTION + 1.)
                            .color(p.text_muted);
                        let width = ui
                            .painter()
                            .layout_no_wrap(
                                count.to_owned(),
                                egui::FontId::proportional(tokens::CAPTION + 1.),
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

/// Width of a `header` title, for tools that must leave it whole.
pub(super) fn header_title_width(ui: &Ui, title: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            title.to_owned(),
            semibold_font(ui, tokens::BODY),
            palette(ui.ctx()).text,
        )
        .size()
        .x
}

/// Ghost tool link in a card header: muted text, text colour on hover.
pub(super) fn tool_link(ui: &mut Ui, label: &str) -> Response {
    let p = palette(ui.ctx());
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(tokens::CAPTION + 1.),
        p.text_muted,
    );
    let (rect, response) = ui.allocate_exact_size(
        galley.size() + Vec2::new(tokens::S, tokens::XS),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let color = if response.hovered() {
        p.text
    } else {
        p.text_muted
    };
    focus_ring(ui, &response);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, color);
    response
}

/// Mono size chip: sunken fill, 1 px border, radius 4.
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
    let painter = ui.painter().with_clip_rect(clip);
    painter.rect(
        rect,
        tokens::CHIP_RADIUS,
        p.surface_sunken,
        Stroke::new(1., p.border),
        StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() * 0.5, galley, p.text_muted);
    rect
}

/// Card for the welcome row; `soft` is the accent_soft example card.
fn start_card(ui: &mut Ui, title: &str, detail: &str, soft: bool) -> Response {
    let p = palette(ui.ctx());
    let (fill, title_color) = if soft {
        (p.accent_soft, p.accent_text)
    } else {
        (p.surface_sunken, p.text)
    };
    let detail_color = if soft { p.accent_text } else { p.text_muted };
    let title_galley = ui.painter().layout_no_wrap(
        title.to_owned(),
        semibold_font(ui, tokens::BODY),
        title_color,
    );
    let detail_galley = ui.painter().layout_no_wrap(
        detail.to_owned(),
        egui::FontId::proportional(tokens::CAPTION + 1.),
        detail_color,
    );
    let width = (title_galley.size().x.max(detail_galley.size().x) + 2. * tokens::M).max(150.);
    let height = title_galley.size().y + detail_galley.size().y + 2. + 2. * tokens::S;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    let stroke = if response.hovered() {
        Stroke::new(1., p.accent)
    } else {
        Stroke::new(1., if soft { p.accent_soft } else { p.border })
    };
    ui.painter().rect(
        rect,
        tokens::CONTROL_RADIUS,
        fill,
        stroke,
        StrokeKind::Inside,
    );
    focus_ring(ui, &response);
    let top = rect.left_top() + Vec2::new(tokens::M, tokens::S);
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
        self.toolkits.selected_tool().is_none()
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
                    if start_card(ui, "Try an example", "plot(x, sin(x))", true).clicked()
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
            Stroke::new(1., p.border),
        );
    }
}

/// Toolstrip tile height inside the 64 px card.
pub(super) const STRIP_TILE: f32 = 56.;

fn strip_font() -> egui::FontId {
    egui::FontId::proportional(tokens::CAPTION + 1.)
}

/// Width of one icon-over-label tile; labels never wrap.
pub(super) fn strip_action_width(ui: &Ui, label: &str) -> f32 {
    let width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), strip_font(), egui::Color32::WHITE)
        .size()
        .x;
    (width + 2. * tokens::S).max(64.)
}

/// Ghost toolstrip action: 20 px stroke icon (accent for Run) over a label;
/// hover is surface_sunken, no pill background.
pub(super) fn strip_action(ui: &mut Ui, label: &str, icon: shell::Icon, run: bool) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let width = strip_action_width(ui, label);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, STRIP_TILE), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if enabled && (response.hovered() || response.is_pointer_button_down_on()) {
        ui.painter()
            .rect_filled(rect, tokens::CONTROL_RADIUS, p.surface_sunken);
    }
    focus_ring(ui, &response);
    let icon_color = if !enabled {
        p.text_disabled
    } else if run {
        p.accent
    } else {
        p.text_muted
    };
    stroke_icon(
        ui,
        Rect::from_center_size(
            rect.center_top() + Vec2::new(0., tokens::S + tokens::ICON * 0.5),
            Vec2::splat(tokens::ICON),
        ),
        icon,
        icon_color,
    );
    ui.painter().text(
        rect.center_top() + Vec2::new(0., tokens::S + tokens::ICON + tokens::XS),
        egui::Align2::CENTER_TOP,
        label,
        strip_font(),
        if enabled { p.text } else { p.text_disabled },
    );
    response
}

/// 1 px vertical divider between toolstrip groups.
pub(super) fn strip_divider(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(2. * tokens::S + 1., STRIP_TILE), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment(
        [
            egui::pos2(x, rect.center().y - 18.),
            egui::pos2(x, rect.center().y + 18.),
        ],
        Stroke::new(1., palette(ui.ctx()).border),
    );
}

/// Line icons on a 16 unit grid, 1.6 px strokes.
pub(super) fn stroke_icon(ui: &Ui, rect: Rect, icon: shell::Icon, color: egui::Color32) {
    use shell::Icon;
    let painter = ui.painter();
    let stroke = Stroke::new(1.6, color);
    let point = |x: f32, y: f32| {
        egui::pos2(
            rect.left() + x * rect.width() / 16.,
            rect.top() + y * rect.height() / 16.,
        )
    };
    let path = |points: &[(f32, f32)], closed: bool| {
        let points = points.iter().map(|&(x, y)| point(x, y)).collect();
        painter.add(if closed {
            egui::Shape::closed_line(points, stroke)
        } else {
            egui::Shape::line(points, stroke)
        });
    };
    match icon {
        Icon::Document => {
            path(
                &[(3., 1.), (10., 1.), (13., 4.), (13., 15.), (3., 15.)],
                true,
            );
            path(&[(10., 1.), (10., 4.), (13., 4.)], false);
        }
        Icon::Folder => path(
            &[
                (1., 3.),
                (6., 3.),
                (7.5, 5.),
                (15., 5.),
                (15., 14.),
                (1., 14.),
            ],
            true,
        ),
        Icon::Save => {
            path(
                &[(2., 2.), (12., 2.), (14., 4.), (14., 14.), (2., 14.)],
                true,
            );
            path(&[(5., 2.), (5., 6.), (11., 6.), (11., 2.)], false);
            path(&[(5., 14.), (5., 10.), (11., 10.), (11., 14.)], false);
        }
        Icon::Run => path(&[(4., 2.), (14., 8.), (4., 14.)], true),
        Icon::Table => {
            path(&[(1., 2.), (15., 2.), (15., 14.), (1., 14.)], true);
            path(&[(1., 6.), (15., 6.)], false);
            path(&[(1., 10.), (15., 10.)], false);
            path(&[(6., 2.), (6., 14.)], false);
        }
        Icon::Plot => {
            path(&[(2., 1.), (2., 14.), (15., 14.)], false);
            path(&[(4., 11.), (7., 6.), (10., 9.), (14., 3.)], false);
        }
        Icon::Tools => {
            painter.circle_stroke(point(8., 8.), rect.width() * 5. / 16., stroke);
            painter.circle_stroke(point(8., 8.), rect.width() * 2. / 16., stroke);
        }
        Icon::Layout => {
            path(&[(1., 2.), (15., 2.), (15., 14.), (1., 14.)], true);
            path(&[(8., 2.), (8., 14.)], false);
        }
        Icon::Undo => {
            path(
                &[(3., 6.), (11., 6.), (14., 9.), (11., 13.), (6., 13.)],
                false,
            );
            path(&[(6., 3.), (3., 6.), (6., 9.)], false);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum FileKind {
    Script,
    Folder,
    Data,
}

/// Files row: 14 px type icon (script = accent square, folder = muted square,
/// data = outlined) and the truncated name; selected = accent_soft fill with
/// accent_text.
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
        p.surface_sunken
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0., fill);
    focus_ring(ui, &response);
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + tokens::M + 7., rect.center().y),
        Vec2::splat(14.),
    );
    match kind {
        FileKind::Script => {
            ui.painter().rect_filled(icon, 3., p.accent);
        }
        FileKind::Folder => {
            ui.painter().rect_filled(icon, 3., p.text_disabled);
        }
        FileKind::Data => {
            ui.painter().rect_stroke(
                icon,
                3.,
                Stroke::new(1.5, p.text_disabled),
                StrokeKind::Inside,
            );
        }
    }
    let color = if selected { p.accent_text } else { p.text };
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

/// Search launcher drawn as a 260 px field with its Ctrl K hint.
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

/// Vector glyphs of the 24 px ghost icon buttons in card headers.
#[derive(Clone, Copy)]
pub(super) enum Glyph {
    Up,
    Back,
    Forward,
    Browse,
    Refresh,
    Filter,
    /// Three dots: the overflow menu of a card.
    More,
    Search,
}

/// 24 px ghost icon button with a tooltip. The label is the accessible name
/// and the tooltip; nothing is painted as text.
pub(super) fn icon_button(ui: &mut Ui, glyph: Glyph, label: &str) -> Response {
    icon_button_with(ui, glyph, label, label)
}

/// `icon_button` whose tooltip differs from its accessible name (shortcuts).
pub(super) fn icon_button_with(ui: &mut Ui, glyph: Glyph, label: &str, tooltip: &str) -> Response {
    icon_button_sized(ui, tokens::INTERACT_HEIGHT, glyph, label, tooltip)
}

/// `icon_button_with` in the HEADER_ICON box of the crowded Files header.
pub(super) fn header_icon_button(
    ui: &mut Ui,
    glyph: Glyph,
    label: &str,
    tooltip: &str,
) -> Response {
    icon_button_sized(ui, tokens::HEADER_ICON, glyph, label, tooltip)
}

fn icon_button_sized(ui: &mut Ui, size: f32, glyph: Glyph, label: &str, tooltip: &str) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    // Tests find icon-only buttons by their label: an invisible text shape.
    #[cfg(test)]
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(1.),
        egui::Color32::TRANSPARENT,
    );
    if enabled && (response.hovered() || response.is_pointer_button_down_on()) {
        ui.painter()
            .rect_filled(rect, tokens::CONTROL_RADIUS, p.surface_sunken);
    }
    focus_ring(ui, &response);
    let color = if !enabled {
        p.text_disabled
    } else if response.hovered() {
        p.text
    } else {
        p.text_muted
    };
    stroke_glyph(
        ui,
        Rect::from_center_size(rect.center(), Vec2::splat(16.)),
        glyph,
        color,
    );
    response
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
}

/// Line glyphs on a 16 unit grid, 1.5 px strokes.
fn stroke_glyph(ui: &Ui, rect: Rect, glyph: Glyph, color: egui::Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.5, color);
    let scale = rect.width() / 16.;
    let point = |x: f32, y: f32| rect.min + Vec2::new(x, y) * scale;
    let path = |points: &[(f32, f32)], closed: bool| {
        let points = points.iter().map(|&(x, y)| point(x, y)).collect();
        painter.add(if closed {
            egui::Shape::closed_line(points, stroke)
        } else {
            egui::Shape::line(points, stroke)
        });
    };
    match glyph {
        Glyph::Up => {
            path(&[(8., 13.), (8., 3.)], false);
            path(&[(4., 7.), (8., 3.), (12., 7.)], false);
        }
        Glyph::Back => {
            path(&[(13., 8.), (3., 8.)], false);
            path(&[(7., 4.), (3., 8.), (7., 12.)], false);
        }
        Glyph::Forward => {
            path(&[(3., 8.), (13., 8.)], false);
            path(&[(9., 4.), (13., 8.), (9., 12.)], false);
        }
        Glyph::Browse => path(
            &[
                (1.5, 3.),
                (6., 3.),
                (7.5, 5.),
                (14.5, 5.),
                (14.5, 13.5),
                (1.5, 13.5),
            ],
            true,
        ),
        Glyph::Refresh => {
            // Three quarters of a circle and an arrow head at its open end.
            let arc: Vec<_> = (0..=14)
                .map(|i| {
                    let angle = (-40. + 270. * i as f32 / 14.).to_radians();
                    (8. + 5. * angle.cos(), 8. + 5. * angle.sin())
                })
                .collect();
            path(&arc, false);
            let end = arc[0];
            path(
                &[(end.0 + 3., end.1 - 1.), end, (end.0 + 1., end.1 + 3.5)],
                false,
            );
        }
        Glyph::Filter => path(
            &[
                (2., 3.),
                (14., 3.),
                (9.5, 8.5),
                (9.5, 13.),
                (6.5, 11.5),
                (6.5, 8.5),
            ],
            true,
        ),
        Glyph::More => {
            for x in [3., 8., 13.] {
                painter.circle_filled(point(x, 8.), 1.5 * scale, color);
            }
        }
        Glyph::Search => {
            painter.circle_stroke(point(7., 7.), 4.5 * scale, stroke);
            path(&[(10.5, 10.5), (14., 14.)], false);
        }
    }
}
