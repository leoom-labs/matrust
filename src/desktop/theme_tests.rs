fn choose_desktop_theme(app: &mut Workspace, ctx: &egui::Context, label: &str) {
    let size = egui::vec2(960., 650.);
    click_label_at(app, ctx, size, "Menu");
    click_label_at(app, ctx, size, "Layout");
    click_label_at(app, ctx, size, "Appearance");
    click_label_at(app, ctx, size, label);
    if egui::Popup::is_any_open(ctx) {
        frame_at(app, ctx, size, vec![egui::Event::Key {
            key: egui::Key::Escape, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::NONE,
        }]);
    }
    for _ in 0..3 { frame_at(app, ctx, size, vec![]); }
}

fn theme_large_fill(shape: &egui::Shape, fill: Color32) -> bool {
    match shape {
        egui::Shape::Rect(rect) => rect.fill == fill && rect.rect.width() > 120.
            && rect.rect.height() > 120.,
        egui::Shape::Vec(shapes) => shapes.iter().any(|shape| theme_large_fill(shape, fill)),
        _ => false,
    }
}

#[test]
fn desktop_appearance_menu_changes_actual_visuals_without_changing_work() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("theme_kept=[1 2;3 4];".into());
    finish(&mut app);
    let original = app.variables["theme_kept"].clone();
    let text = app.documents[app.active].text.clone();
    let dark = ctx.style_of(egui::Theme::Dark).visuals.panel_fill;
    let light = ctx.style_of(egui::Theme::Light).visuals.panel_fill;
    assert_ne!(dark, light);
    for (label, selected, fill) in [
        ("Dark", matrust::session::DesktopTheme::Dark, dark),
        ("Light", matrust::session::DesktopTheme::Light, light),
    ] {
        choose_desktop_theme(&mut app, &ctx, label);
        assert_eq!(app.layout.theme, Some(selected));
        assert_eq!(ctx.global_style().visuals.dark_mode,
            selected == matrust::session::DesktopTheme::Dark);
        let output = frame_at(&mut app, &ctx, egui::vec2(960.,650.), vec![]);
        assert!(output.shapes.iter().any(|s| theme_large_fill(&s.shape, fill)),
            "Actual desktop panels must paint the selected appearance");
        assert!(app.variables["theme_kept"].same_snapshot(&original));
        assert_eq!(app.documents[app.active].text, text);
        assert!(!app.busy);
    }
    choose_desktop_theme(&mut app, &ctx, "Use system setting");
    assert_eq!(app.layout.theme, None);
    assert_eq!(ctx.options(|options| options.theme_preference), egui::ThemePreference::System);
}

#[test]
fn desktop_appearance_survives_session_restart_and_preserves_layout_preferences() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.layout.sidebar_width = Some(230.);
    app.layout.workspace_width = Some(210.);
    app.layout.console_height = Some(180.);
    app.layout.editor_fraction = Some(0.55);
    choose_desktop_theme(&mut app, &ctx, "Dark");
    let layout = app.layout;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("theme-session.json");
    app.snapshot().save(&path).unwrap();
    let ctx = egui::Context::default();
    let mut restored = legacy_sample_desktop(ctx.clone(), true);
    restored.restore_session(Session::load(&path).unwrap());
    frame_at(&mut restored, &ctx, egui::vec2(960.,650.), vec![]);
    assert_eq!(restored.layout, layout);
    assert_eq!(ctx.theme(), egui::Theme::Dark);
    assert!(ctx.global_style().visuals.dark_mode);
}

#[test]
fn desktop_system_appearance_follows_host_updates_with_identical_metrics() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    for system in [egui::Theme::Dark, egui::Theme::Light, egui::Theme::Dark] {
        let _ = crate::headless::run(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(960.,650.))),
            system_theme: Some(system), ..Default::default()
        }, |root| app.ui(root));
        assert_eq!(ctx.theme(), system);
        assert_eq!(ctx.global_style().visuals.dark_mode, system == egui::Theme::Dark);
        assert_eq!(ctx.global_style().visuals.window_fill.a(), 255);
        assert_eq!(app.layout.theme, None);
    }
    let light = ctx.style_of(egui::Theme::Light);
    let dark = ctx.style_of(egui::Theme::Dark);
    assert_eq!(light.spacing.item_spacing, dark.spacing.item_spacing);
    assert_eq!(light.spacing.button_padding, dark.spacing.button_padding);
    assert_eq!(light.text_styles, dark.text_styles);
    assert_eq!(light.animation_time, 0.);
    assert_eq!(dark.animation_time, 0.);
}

#[test]
fn explicit_capture_theme_overrides_restore_without_changing_saved_preference() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.layout.theme = Some(matrust::session::DesktopTheme::Light);
    app.screenshot_theme_override = Some(egui::ThemePreference::Dark);
    for _ in 0..3 { frame_at(&mut app, &ctx, egui::vec2(960.,650.), vec![]); }
    assert_eq!(ctx.theme(), egui::Theme::Dark);
    assert!(ctx.global_style().visuals.dark_mode);
    assert_eq!(app.snapshot().layout.theme, Some(matrust::session::DesktopTheme::Light));
    choose_desktop_theme(&mut app, &ctx, "Light");
    assert!(app.screenshot_theme_override.is_none());
    assert_eq!(ctx.theme(), egui::Theme::Light);
}

/// Filled rects under `at`, smallest first.
fn filled_rects_at(output: &egui::FullOutput, at: egui::Pos2) -> Vec<egui::Rect> {
    fn visit(shape: &egui::Shape, at: egui::Pos2, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) if r.rect.contains(at) && r.fill != Color32::TRANSPARENT => {
                out.push(r.rect)
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| visit(s, at, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, at, &mut out);
    }
    out.sort_by(|a, b| a.area().total_cmp(&b.area()));
    out
}

#[test]
fn appearance_submenu_opens_beside_its_row_and_marks_the_current_theme() {
    for size in [egui::vec2(1600., 1000.), egui::vec2(1024., 640.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.layout.theme = Some(matrust::session::DesktopTheme::Dark);
        app.apply_desktop_theme(&ctx);
        click_label_at(&mut app, &ctx, size, "Menu");
        click_label_at(&mut app, &ctx, size, "Layout");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let row = output.shapes.iter().rev().find_map(|s| text_position(&s.shape, "Appearance")).unwrap();
        for _ in 0..4 {
            frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(row)]);
        }
        let output = frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(row)]);
        let at = |label| output.shapes.iter().rev().find_map(|s| text_position(&s.shape, label));
        let (dark, light) = (at("Dark").expect("Dark"), at("Light").expect("Light"));
        // The Layout menu card is the largest small rect under the row.
        let menu = *filled_rects_at(&output, row).iter().rfind(|r| r.width() < 400.).unwrap();
        let sub = *filled_rects_at(&output, light).iter().rfind(|r| r.width() < 400.).unwrap();
        assert!(!sub.intersects(menu.shrink(1.)), "{size:?}: {sub:?} over {menu:?}");
        let gap = if sub.left() >= menu.right() { sub.left() - menu.right() } else { menu.left() - sub.right() };
        assert!((0. ..=8.).contains(&gap), "{size:?}: submenu {sub:?} is {gap} px from {menu:?}");
        assert!((sub.top() - row.y).abs() < 24., "{size:?}: submenu {sub:?} beside row {row:?}");
        // The current choice uses the selected row fill, like Desktop scale.
        let marked = |p: egui::Pos2| filled_rects_at(&output, p).iter().any(|r| sub.contains_rect(*r) && *r != sub);
        assert!(marked(dark), "{size:?}: current theme not marked");
        assert!(!marked(light), "{size:?}: other theme marked");
    }
}
