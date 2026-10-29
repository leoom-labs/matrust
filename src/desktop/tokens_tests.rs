use super::{egui, theme, tokens};
use egui::{Color32, FontFamily, FontId, TextStyle};

fn luminance(color: Color32) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(a: Color32, b: Color32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn wcag_contrast_matches_reference_values() {
    assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.).abs() < 1e-9);
    assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.).abs() < 1e-9);
}

/// Every enabled text/background pair the chrome paints, at least 4.5:1 in
/// both themes (main text at least 7:1 on cards).
#[test]
fn both_palettes_meet_text_contrast_floors() {
    for (name, p) in [("light", &tokens::LIGHT), ("dark", &tokens::DARK)] {
        assert!(contrast(p.text, p.surface) >= 7., "{name} text/surface");
        let pairs = [
            ("text/surface_sunken", p.text, p.surface_sunken),
            ("text/canvas", p.text, p.canvas),
            ("text/accent_soft", p.text, p.accent_soft),
            ("text_muted/surface", p.text_muted, p.surface),
            ("text_muted/surface_sunken", p.text_muted, p.surface_sunken),
            ("text_muted/canvas", p.text_muted, p.canvas),
            ("accent_text/accent_soft", p.accent_text, p.accent_soft),
            ("on_accent/accent_fill", p.on_accent, p.accent_fill),
            ("prompt accent/surface_sunken", p.accent, p.surface_sunken),
            ("prompt accent/surface", p.accent, p.surface),
            ("error/surface", p.error, p.surface),
            ("error/surface_sunken", p.error, p.surface_sunken),
            ("warning/surface", p.warning, p.surface),
            ("warning/surface_sunken", p.warning, p.surface_sunken),
        ];
        for (pair, fg, bg) in pairs {
            assert!(
                contrast(fg, bg) >= 4.5,
                "{name} {pair}: {:.2}",
                contrast(fg, bg)
            );
        }
    }
}

#[test]
fn palettes_are_the_approved_warm_light_and_dark() {
    use egui::Color32 as C;
    assert_eq!(tokens::LIGHT.canvas, C::from_rgb(0xE9, 0xE6, 0xE1));
    assert_eq!(tokens::LIGHT.accent, C::from_rgb(0xC2, 0x41, 0x0C));
    assert_eq!(tokens::DARK.canvas, C::from_rgb(0x12, 0x14, 0x18));
    assert_eq!(tokens::DARK.accent, C::from_rgb(0xEA, 0x6A, 0x2C));
    for p in [&tokens::LIGHT, &tokens::DARK] {
        assert_eq!(p.accent_fill, C::from_rgb(0xC2, 0x41, 0x0C));
        assert_eq!(p.on_accent, C::WHITE);
        // No blue or purple accent: red dominates.
        assert!(p.accent.r() > p.accent.b() && p.accent.r() > p.accent.g());
    }
}

#[test]
fn palette_follows_the_global_appearance() {
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    for (mode, expected) in [
        (egui::Theme::Light, &tokens::LIGHT),
        (egui::Theme::Dark, &tokens::DARK),
    ] {
        ctx.set_theme(mode);
        let _ = crate::headless::run(&ctx, Default::default(), |_| {});
        assert!(std::ptr::eq(tokens::palette(&ctx), expected));
        let visuals = &ctx.style_of(mode).visuals;
        assert_eq!(visuals.panel_fill, expected.surface);
        assert_eq!(visuals.faint_bg_color, expected.surface_sunken);
        assert_eq!(visuals.selection.bg_fill, expected.accent_soft);
        assert_eq!(visuals.selection.stroke.color, expected.accent_text);
        assert_eq!(visuals.hyperlink_color, expected.accent);
        assert_eq!(visuals.error_fg_color, expected.error);
        // Keyboard focus: one 2 px accent ring.
        assert_eq!(
            visuals.widgets.active.bg_stroke,
            egui::Stroke::new(2., expected.accent)
        );
        assert_eq!(visuals.window_shadow, egui::Shadow::NONE);
        assert_eq!(visuals.popup_shadow, egui::Shadow::NONE);
        assert_eq!(
            visuals.widgets.inactive.corner_radius,
            egui::CornerRadius::same(tokens::CONTROL_RADIUS)
        );
    }
}

#[test]
fn configure_installs_type_scale_spacing_and_fonts() {
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    let _ = crate::headless::run(&ctx, Default::default(), |_| {});
    for mode in [egui::Theme::Light, egui::Theme::Dark] {
        let style = ctx.style_of(mode);
        let font = |text: TextStyle| style.text_styles[&text].clone();
        assert_eq!(font(TextStyle::Body), FontId::proportional(13.));
        assert_eq!(font(TextStyle::Button), FontId::proportional(13.));
        assert_eq!(font(TextStyle::Small), FontId::proportional(11.));
        assert_eq!(font(TextStyle::Monospace), FontId::monospace(13.));
        assert_eq!(
            font(TextStyle::Heading),
            FontId::new(15., FontFamily::Name(tokens::SEMIBOLD.into()))
        );
        assert_eq!(style.spacing.item_spacing, egui::vec2(8., 4.));
        assert_eq!(style.spacing.button_padding, egui::vec2(8., 4.));
        assert_eq!(style.spacing.interact_size.y, 24.);
    }
    let defaults = egui::FontDefinitions::default();
    ctx.fonts(|fonts| {
        let families = &fonts.definitions().families;
        let first = |family: FontFamily| families[&family][0].clone();
        assert_eq!(first(FontFamily::Proportional), "Inter-Regular");
        assert_eq!(
            first(FontFamily::Name(tokens::SEMIBOLD.into())),
            "Inter-SemiBold"
        );
        assert_eq!(first(FontFamily::Monospace), "JetBrainsMono-Regular");
        // egui's fonts remain as symbol and emoji fallbacks.
        assert_eq!(
            families[&FontFamily::Proportional][1..],
            defaults.families[&FontFamily::Proportional][..]
        );
        assert_eq!(
            families[&FontFamily::Monospace][1..],
            defaults.families[&FontFamily::Monospace][..]
        );
        assert_eq!(
            families[&FontFamily::Name(theme::FIGURE.into())],
            defaults.families[&FontFamily::Proportional]
        );
        assert_eq!(
            families[&FontFamily::Name(theme::FIGURE_MONO.into())],
            defaults.families[&FontFamily::Monospace]
        );
    });
}

#[test]
fn figure_text_keeps_egui_default_glyphs() {
    let configured = egui::Context::default();
    theme::configure(&configured);
    let plain = egui::Context::default();
    for ctx in [&configured, &plain] {
        let _ = crate::headless::run(ctx, Default::default(), |_| {});
    }
    let text = "Amplitude 0.25 \u{3bc}s";
    let layout = |ctx: &egui::Context, font: FontId| {
        ctx.fonts_mut(|f| f.layout_no_wrap(text.into(), font, Color32::BLACK))
    };
    let figure = layout(&configured, theme::figure_body(&configured));
    let original = layout(&plain, FontId::proportional(12.));
    assert_eq!(figure.size(), original.size());
    assert_eq!(figure.rows[0].visuals.mesh, original.rows[0].visuals.mesh);
    let chrome = layout(&configured, FontId::proportional(12.));
    assert_ne!(chrome.size(), original.size(), "chrome must use Inter");
}

#[test]
fn figure_scope_restores_former_style_inside_configured_chrome() {
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    for mode in [egui::Theme::Light, egui::Theme::Dark] {
        let _ = crate::headless::run(&ctx, Default::default(), |root| {
            root.scope(|ui| {
                theme::apply_figure_style(ui, mode);
                let style = ui.style();
                assert_eq!(style.spacing.item_spacing, egui::vec2(6., 4.));
                assert_eq!(style.spacing.button_padding, egui::vec2(6., 3.));
                assert_eq!(
                    style.text_styles[&TextStyle::Body],
                    FontId::new(12., FontFamily::Name(theme::FIGURE.into()))
                );
                assert_eq!(
                    style.text_styles[&TextStyle::Monospace],
                    FontId::new(13., FontFamily::Name(theme::FIGURE_MONO.into()))
                );
                let dark = mode == egui::Theme::Dark;
                assert_eq!(
                    style.visuals.extreme_bg_color,
                    if dark {
                        Color32::from_gray(23)
                    } else {
                        Color32::WHITE
                    }
                );
                assert_eq!(
                    style.visuals.widgets.noninteractive.bg_stroke.color,
                    Color32::from_gray(if dark { 72 } else { 195 })
                );
            });
            assert_eq!(root.style().spacing.item_spacing, egui::vec2(8., 4.));
        });
    }
}

#[test]
fn title_is_semibold_eighteen_and_survives_an_unconfigured_context() {
    let semibold = tokens::semibold(tokens::TITLE);
    assert_eq!(semibold.size, 18.);
    assert_eq!(semibold.family, FontFamily::Name(tokens::SEMIBOLD.into()));
    let configured = egui::Context::default();
    theme::configure(&configured);
    let _ = crate::headless::run(&configured, Default::default(), |_| {});
    assert_eq!(
        tokens::title(&configured, "Dialog").text(),
        "Dialog",
        "configured context"
    );
    // Tests and previews may run on a context that never ran `configure`.
    let bare = egui::Context::default();
    let _ = crate::headless::run(&bare, Default::default(), |_| {});
    assert_eq!(tokens::title(&bare, "Dialog").text(), "Dialog");
}
