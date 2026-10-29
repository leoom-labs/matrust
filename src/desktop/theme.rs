//! Native appearance preferences. Figure documents retain their own colors.
use super::{Workspace, egui, tokens};
use matrust::session::DesktopTheme;
use std::sync::Arc;

/// Figure artwork keeps egui's original default fonts so screen output and
/// captures stay identical while the chrome uses Inter and JetBrains Mono.
pub(super) const FIGURE: &str = "figure";
pub(super) const FIGURE_MONO: &str = "figure_mono";

pub(super) fn figure_font(ctx: &egui::Context, size: f32) -> egui::FontId {
    let figure = egui::FontFamily::Name(FIGURE.into());
    // A context without `configure` still has egui's default proportional
    // family, which holds the same font list.
    let family = if ctx.fonts(|f| f.definitions().families.contains_key(&figure)) {
        figure
    } else {
        egui::FontFamily::Proportional
    };
    egui::FontId::new(size, family)
}

/// Body text of figure artwork: axes titles, labels, ticks and legends.
pub(super) fn figure_body(ctx: &egui::Context) -> egui::FontId {
    figure_font(ctx, 12.)
}

fn fonts() -> egui::FontDefinitions {
    use egui::FontFamily::{Monospace, Name, Proportional};
    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts.families[&Proportional].clone();
    let monospace = fonts.families[&Monospace].clone();
    let files: [(&str, &'static [u8]); 3] = [
        (
            "Inter-Regular",
            include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
        ),
        (
            "Inter-SemiBold",
            include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
        ),
        (
            "JetBrainsMono-Regular",
            include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
        ),
    ];
    for (name, data) in files {
        fonts
            .font_data
            .insert(name.into(), Arc::new(egui::FontData::from_static(data)));
    }
    // egui's own fonts stay as fallbacks for symbols and emoji.
    let first = |name: &str, fallback: &[String]| {
        std::iter::once(name.to_owned())
            .chain(fallback.iter().cloned())
            .collect::<Vec<_>>()
    };
    fonts
        .families
        .insert(Proportional, first("Inter-Regular", &proportional));
    fonts.families.insert(
        Name(tokens::SEMIBOLD.into()),
        first("Inter-SemiBold", &proportional),
    );
    fonts
        .families
        .insert(Monospace, first("JetBrainsMono-Regular", &monospace));
    fonts.families.insert(Name(FIGURE.into()), proportional);
    fonts.families.insert(Name(FIGURE_MONO.into()), monospace);
    fonts
}

pub(super) fn configure(ctx: &egui::Context) {
    ctx.options_mut(|options| {
        options.zoom_with_keyboard = false;
        // Keep a readable light default if the host cannot report its theme.
        options.fallback_theme = egui::Theme::Light;
    });
    ctx.set_fonts(fonts());
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let mut style = (*ctx.style_of(theme)).clone();
        let dark = theme == egui::Theme::Dark;
        let p = tokens::palette_for(dark);
        ctx.data_mut(|d| d.insert_temp(figure_style_id(theme), Arc::new(figure_style(dark))));
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        // Default panels sit inside cards; the canvas is painted by the window.
        visuals.panel_fill = p.surface;
        visuals.window_fill = p.surface;
        visuals.extreme_bg_color = p.surface;
        visuals.faint_bg_color = p.surface_sunken;
        visuals.window_stroke = egui::Stroke::new(1., p.border);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1., p.border);
        visuals.widgets.noninteractive.fg_stroke.color = p.text;
        visuals.widgets.inactive.fg_stroke.color = p.text;
        // Neutral buttons: surface fill and the 1 px border; disabled ones fade
        // from the same neutral style, never from a pale accent.
        visuals.widgets.inactive.bg_fill = p.surface;
        visuals.widgets.inactive.weak_bg_fill = p.surface;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1., p.border);
        visuals.weak_text_color = Some(p.text_muted);
        visuals.widgets.hovered.bg_fill = p.surface_sunken;
        visuals.widgets.hovered.weak_bg_fill = p.surface_sunken;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1., p.border);
        visuals.widgets.hovered.fg_stroke.color = p.text;
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(tokens::CONTROL_RADIUS);
            widget.expansion = 0.;
        }
        // Keyboard focus (egui's `active` state) is one 2 px accent ring on
        // every interactive widget; text fields use the selection stroke.
        visuals.widgets.active.bg_stroke = egui::Stroke::new(2., p.accent);
        visuals.widgets.active.bg_fill = p.accent_soft;
        visuals.widgets.active.weak_bg_fill = p.surface_sunken;
        visuals.widgets.active.fg_stroke.color = p.text;
        visuals.widgets.open.weak_bg_fill = p.surface_sunken;
        visuals.selection.bg_fill = p.accent_soft;
        visuals.selection.stroke = egui::Stroke::new(2., p.accent_text);
        visuals.hyperlink_color = p.accent;
        visuals.error_fg_color = p.error;
        visuals.warn_fg_color = p.warning;
        visuals.window_corner_radius = egui::CornerRadius::same(tokens::PANEL_RADIUS);
        visuals.menu_corner_radius = egui::CornerRadius::same(tokens::CONTROL_RADIUS);
        // Elevation is the 1 px outline only; no shadows.
        visuals.window_shadow = egui::Shadow::NONE;
        visuals.popup_shadow = egui::Shadow::NONE;
        style.visuals = visuals;
        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(tokens::S, tokens::XS);
        spacing.button_padding = egui::vec2(tokens::S, tokens::XS);
        spacing.interact_size.y = tokens::INTERACT_HEIGHT;
        spacing.menu_margin = egui::Margin::same(tokens::S as i8);
        spacing.window_margin = egui::Margin::same(tokens::S as i8);
        use egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(tokens::CAPTION)),
            (TextStyle::Body, FontId::proportional(tokens::BODY)),
            (TextStyle::Button, FontId::proportional(tokens::BODY)),
            (TextStyle::Heading, tokens::semibold(tokens::HEADING)),
            (TextStyle::Monospace, FontId::monospace(tokens::CODE)),
        ]
        .into();
        style.spacing.scroll = scroll_style();
        // Both stored styles must be opaque on the early capture frames.
        style.animation_time = 0.;
        ctx.set_style_of(theme, style);
    }
}

/// The one scrollbar every scroll area uses: a 6 px, fully rounded handle
/// in its own gutter beside the content.
pub(super) fn scroll_style() -> egui::style::ScrollStyle {
    // Solid (not floating) bars keep a gutter, so they never overlap text.
    egui::style::ScrollStyle {
        bar_width: 6.,
        ..egui::style::ScrollStyle::solid()
    }
}

/// Resting handle colour: the border pulled slightly towards the text, so it
/// is darker in light themes and lighter in dark ones but quieter than text.
pub(super) fn scrollbar_handle(p: &tokens::Palette) -> egui::Color32 {
    p.border.lerp_to_gamma(p.text, 0.2)
}

/// egui paints the handle with the widget `bg_fill` (inactive, hovered on the
/// handle, active while dragged) and the track with `extreme_bg_color`.
fn apply_scrollbar_paint(style: &mut egui::Style, p: &tokens::Palette) {
    style.spacing.scroll = scroll_style();
    style.visuals.extreme_bg_color = egui::Color32::TRANSPARENT;
    let radius = egui::CornerRadius::same((style.spacing.scroll.bar_width * 0.5) as u8);
    let widgets = &mut style.visuals.widgets;
    for widget in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
    ] {
        widget.corner_radius = radius;
    }
    widgets.noninteractive.bg_fill = scrollbar_handle(p);
    widgets.inactive.bg_fill = scrollbar_handle(p);
    widgets.hovered.bg_fill = p.text_muted;
    widgets.active.bg_fill = p.text_muted;
}

/// Apply handle visuals only to the parent used by egui to paint scrollbars.
/// Content receives its original style, including checkbox/radio/slider fills.
/// No extra Ui scope is introduced: persistent scroll and child IDs stay stable.
fn scroll_paint<R>(
    ui: &mut egui::Ui,
    show: impl FnOnce(&mut egui::Ui, Arc<egui::Style>) -> R,
) -> R {
    let content_style = ui.style().clone();
    let p = tokens::palette(ui.ctx());
    apply_scrollbar_paint(ui.style_mut(), p);
    let result = show(ui, content_style.clone());
    ui.set_style(content_style);
    result
}

pub(crate) trait ThemedScrollArea {
    fn show_themed<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R>;
    fn show_rows_themed<R>(
        self,
        ui: &mut egui::Ui,
        row_height: f32,
        total_rows: usize,
        add_contents: impl FnOnce(&mut egui::Ui, std::ops::Range<usize>) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R>;
    fn show_viewport_themed<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui, egui::Rect) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R>;
}

impl ThemedScrollArea for egui::ScrollArea {
    fn show_themed<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R> {
        scroll_paint(ui, |ui, style| {
            self.show(ui, |content| {
                content.set_style(style);
                add_contents(content)
            })
        })
    }
    fn show_rows_themed<R>(
        self,
        ui: &mut egui::Ui,
        row_height: f32,
        total_rows: usize,
        add_contents: impl FnOnce(&mut egui::Ui, std::ops::Range<usize>) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R> {
        scroll_paint(ui, |ui, style| {
            self.show_rows(ui, row_height, total_rows, |content, range| {
                content.set_style(style);
                add_contents(content, range)
            })
        })
    }
    fn show_viewport_themed<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui, egui::Rect) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R> {
        scroll_paint(ui, |ui, style| {
            self.show_viewport(ui, |content, viewport| {
                content.set_style(style);
                add_contents(content, viewport)
            })
        })
    }
}

fn figure_style_id(theme: egui::Theme) -> egui::Id {
    egui::Id::new(("desktop_figure_style", theme == egui::Theme::Dark))
}

/// Figure artwork keeps the appearance it had before the chrome tokens:
/// egui's default fonts at the former sizes, the former spacing and the former
/// light/dark visuals. Chrome palette changes must never reach plot pixels.
/// An unconfigured context keeps using its own stored visuals, as before.
pub(super) fn apply_figure_style(ui: &mut egui::Ui, theme: egui::Theme) {
    // Dense compact chrome overrides the text style; plot text never follows it.
    ui.style_mut().override_text_style = None;
    let figure = ui
        .ctx()
        .data(|d| d.get_temp::<Arc<egui::Style>>(figure_style_id(theme)));
    let Some(figure) = figure else {
        *ui.visuals_mut() = ui.ctx().style_of(theme).visuals.clone();
        return;
    };
    let style = ui.style_mut();
    style.visuals = figure.visuals.clone();
    style.spacing = figure.spacing.clone();
    style.text_styles = figure.text_styles.clone();
    // Scroll areas around and inside figures are chrome: same quiet bar.
    style.spacing.scroll = scroll_style();
}

fn figure_style(dark: bool) -> egui::Style {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.error_fg_color = egui::Color32::LIGHT_RED;
    }
    visuals.panel_fill = if dark {
        egui::Color32::from_gray(30)
    } else {
        egui::Color32::WHITE
    };
    visuals.window_fill = if dark {
        egui::Color32::from_gray(35)
    } else {
        egui::Color32::WHITE
    };
    visuals.extreme_bg_color = if dark {
        egui::Color32::from_gray(23)
    } else {
        egui::Color32::WHITE
    };
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1., egui::Color32::from_gray(if dark { 72 } else { 195 }));
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(2);
    }
    let selected = if dark {
        egui::Color32::from_rgb(39, 77, 108)
    } else {
        egui::Color32::from_rgb(205, 228, 246)
    };
    visuals.selection.bg_fill = selected;
    visuals.widgets.active.bg_fill = selected;
    egui::Style {
        visuals,
        spacing: egui::Spacing {
            item_spacing: egui::vec2(6., 4.),
            button_padding: egui::vec2(6., 3.),
            ..Default::default()
        },
        text_styles: figure_text_styles(),
        ..Default::default()
    }
}

fn figure_text_styles() -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
    use egui::{FontFamily, TextStyle};
    let mut styles = egui::Style::default().text_styles;
    styles.insert(TextStyle::Body, egui::FontId::proportional(12.));
    styles.insert(TextStyle::Button, egui::FontId::proportional(12.));
    styles.insert(TextStyle::Monospace, egui::FontId::monospace(13.));
    for font in styles.values_mut() {
        font.family = FontFamily::Name(
            if font.family == FontFamily::Monospace {
                FIGURE_MONO
            } else {
                FIGURE
            }
            .into(),
        );
    }
    styles
}

fn preference(theme: DesktopTheme) -> egui::ThemePreference {
    match theme {
        DesktopTheme::Light => egui::ThemePreference::Light,
        DesktopTheme::Dark => egui::ThemePreference::Dark,
        DesktopTheme::System => egui::ThemePreference::System,
    }
}

impl Workspace {
    pub(super) fn apply_desktop_theme(&self, ctx: &egui::Context) {
        let selected = self
            .screenshot_theme_override
            .unwrap_or_else(|| preference(self.layout.theme.unwrap_or_default()));
        if ctx.options(|options| options.theme_preference) != selected {
            ctx.set_theme(selected);
            ctx.request_repaint();
        }
    }

    pub(super) fn set_desktop_theme(&mut self, ctx: &egui::Context, theme: DesktopTheme) {
        let setting = (theme != DesktopTheme::System).then_some(theme);
        self.recovery_dirty |= self.layout.theme != setting;
        self.layout.theme = setting;
        self.screenshot_theme_override = None;
        self.apply_desktop_theme(ctx);
    }

    pub(super) fn desktop_theme_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Appearance", |ui| {
            let selected = self
                .screenshot_theme_override
                .unwrap_or_else(|| preference(self.layout.theme.unwrap_or_default()));
            for (theme, label) in [
                (DesktopTheme::Light, "Light"),
                (DesktopTheme::Dark, "Dark"),
                (DesktopTheme::System, "Use system setting"),
            ] {
                if ui
                    .selectable_label(selected == preference(theme), label)
                    .clicked()
                {
                    self.set_desktop_theme(ui.ctx(), theme);
                    ui.close();
                }
            }
        });
    }
}

// Status text in a figure pane keeps its former colours even when a plot's
// document style has been scoped to Light. Chrome uses `tokens::palette`.
// Never use this helper for series/data colors.
pub(super) fn error_color(ui: &egui::Ui, light: egui::Color32) -> egui::Color32 {
    if ui.ctx().global_style().visuals.dark_mode {
        egui::Color32::LIGHT_RED
    } else {
        light
    }
}

#[cfg(test)]
pub(super) fn assert_painted_status(
    output: &egui::FullOutput,
    label: &str,
    expected: egui::Color32,
) {
    fn count(shape: &egui::Shape, clip: egui::Rect, label: &str, expected: egui::Color32) -> usize {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                assert!(
                    clip.contains_rect(text.visual_bounding_rect()),
                    "Status must be visible: {label}"
                );
                assert_eq!(text.opacity_factor, 1.);
                assert!(text.override_text_color.is_none_or(|c| c == expected));
                assert!(!text.galley.job.sections.is_empty());
                assert!(
                    text.galley
                        .job
                        .sections
                        .iter()
                        .all(|s| s.format.color == expected)
                );
                1
            }
            egui::Shape::Vec(shapes) => {
                shapes.iter().map(|s| count(s, clip, label, expected)).sum()
            }
            _ => 0,
        }
    }
    assert_eq!(
        output
            .shapes
            .iter()
            .map(|s| count(&s.shape, s.clip_rect, label, expected))
            .sum::<usize>(),
        1
    );
}

#[cfg(test)]
mod scrollbar_tests {
    use super::*;
    use egui::{Color32, Rect, Shape};

    fn handles(shape: &Shape, color: Color32, out: &mut Vec<Rect>) {
        match shape {
            Shape::Rect(r) if (r.rect.width() - 6.).abs() < 0.1 && r.rect.height() >= 99. => {
                assert_eq!(
                    r.fill,
                    Color32::TRANSPARENT,
                    "scroll track must be transparent"
                );
            }
            Shape::Rect(r) if r.fill == color && (r.rect.width() - 6.).abs() < 0.1 => {
                assert_eq!(r.corner_radius, egui::CornerRadius::same(3));
                out.push(r.rect);
            }
            Shape::Vec(shapes) => shapes.iter().for_each(|s| handles(s, color, out)),
            _ => {}
        }
    }

    #[test]
    fn scrollbars_share_quiet_paint_without_changing_content_controls() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for figure in [false, true] {
                let ctx = egui::Context::default();
                configure(&ctx);
                ctx.set_theme(theme);
                let p = tokens::palette_for(theme == egui::Theme::Dark);
                let widgets = &ctx.global_style().visuals.widgets;
                assert_eq!(widgets.inactive.bg_fill, p.surface);
                assert_eq!(widgets.hovered.bg_fill, p.surface_sunken);
                assert_eq!(widgets.active.bg_fill, p.accent_soft);
                let mut handle = Rect::NOTHING;
                for state in 0..4 {
                    let mut events = Vec::new();
                    if state >= 2 {
                        events.push(egui::Event::PointerMoved(handle.center()));
                    }
                    if state == 3 {
                        events.push(egui::Event::PointerButton {
                            pos: handle.center(),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                    let output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(300., 200.),
                            )),
                            events,
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                if figure {
                                    apply_figure_style(ui, theme);
                                }
                                let original = ui.style().clone();
                                egui::ScrollArea::vertical().max_height(100.).show_themed(
                                    ui,
                                    |content| {
                                        assert_eq!(content.style(), &original);
                                        let mut checked = true;
                                        content.checkbox(&mut checked, "Checkbox");
                                        let _ = content.radio(true, "Radio");
                                        content.allocate_space(egui::vec2(100., 400.));
                                    },
                                );
                                assert_eq!(ui.style(), &original);
                            });
                        },
                    );
                    if state == 0 {
                        continue;
                    }
                    let color = if state < 2 {
                        scrollbar_handle(p)
                    } else {
                        p.text_muted
                    };
                    let mut painted = Vec::new();
                    for shape in &output.shapes {
                        handles(&shape.shape, color, &mut painted);
                    }
                    assert_eq!(
                        painted.len(),
                        1,
                        "{theme:?}, figure={figure}, state={state}"
                    );
                    handle = painted[0];
                }
            }
        }
    }
}
