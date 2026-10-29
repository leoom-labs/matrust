fn scale_frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    pixels: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    // Like the native integration, inputs are logical points at the previous
    // frame's scale. egui rescales that rect when applying a pending zoom.
    frame_at(app, ctx, pixels / ctx.zoom_factor(), events)
}

fn scale_click(app: &mut Workspace, ctx: &egui::Context, pixels: egui::Vec2, label: &str) {
    let mut position = None;
    for _ in 0..3 {
        let output = scale_frame(app, ctx, pixels, vec![]);
        position = output
            .shapes
            .iter()
            .rev()
            .find_map(|s| text_position(&s.shape, label).filter(|p| s.clip_rect.contains(*p)));
    }
    let pos = position.unwrap_or_else(|| panic!("Missing visible control: {label}"));
    assert!(
        ctx.content_rect().contains(pos),
        "Off-screen control: {label}"
    );
    for pressed in [true, false] {
        scale_frame(
            app,
            ctx,
            pixels,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn desktop_scale_menu_recovery_reset_preserve_documents_and_shared_samples() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    // This regression intentionally exercises the supported right-dock boundary.
    app.set_workspace_dock(&ctx, session::WorkspaceDock::Right);
    let pixels = egui::vec2(960., 650.);
    app.run("x=1:16;plot(x,x);title('Saved samples');xlabel('Elapsed time');".into());
    finish(&mut app);
    let points = app.figure.as_ref().unwrap().series[0].points.clone();
    app.documents[0].text = "% Cafe\u{301}\nx = 42;".into();
    app.documents[0].changed();
    let text = app.documents[0].text.clone();
    app.layout.editor_fraction = Some(0.6);
    scale_click(&mut app, &ctx, pixels, "Menu");
    scale_click(&mut app, &ctx, pixels, "Layout");
    scale_click(&mut app, &ctx, pixels, "Desktop scale: 100%");
    scale_click(&mut app, &ctx, pixels, "150%");
    for _ in 0..3 {
        scale_frame(&mut app, &ctx, pixels, vec![]);
    }
    assert_eq!(ctx.zoom_factor(), 1.5);
    // Viewport rectangles are rounded to the GUI's pixel/point grid.
    assert!((ctx.content_rect().size() * ctx.pixels_per_point() - pixels).length() < 1.);
    assert_eq!(app.layout.editor_fraction, Some(0.6));
    assert!(app.recovery_dirty);
    assert!((points).same_storage(&app.figure.as_ref().unwrap().series[0].points));
    assert_eq!(app.documents[0].text, text);
    scale_click(&mut app, &ctx, pixels, "Plot figure");
    assert!(app.compact_figure);
    let output = scale_scroll_figure(&mut app, &ctx, pixels);
    assert!(
        output.shapes.iter().any(|s| {
            text_position(&s.shape, "Elapsed time")
                .is_some_and(|p| s.clip_rect.contains(p) && ctx.content_rect().contains(p))
        }),
        "Compact figure axis label must remain reachable: viewport={:?}, pixels={pixels:?}, zoom={}, plot_frame={:?}, console={:?}",
        ctx.content_rect(), ctx.zoom_factor(),
        egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).map(|memory| *memory.transform().frame()),
        egui::containers::panel::PanelState::load(&ctx, egui::Id::new("console")).map(|panel| panel.outer_rect)
    );
    scale_click(&mut app, &ctx, pixels, "Script editor");
    assert!(!app.compact_figure);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    app.snapshot().save(&path).unwrap();
    let restored_ctx = egui::Context::default();
    let mut restored = legacy_sample_desktop(restored_ctx.clone(), true);
    restored.restore_session(Session::load(&path).unwrap());
    for _ in 0..4 {
        scale_frame(&mut restored, &restored_ctx, pixels, vec![]);
    }
    assert_eq!(restored_ctx.zoom_factor(), 1.5);
    assert_eq!(restored.documents[0].text, text);
    assert!(restored.documents[0].dirty);
    scale_click(&mut restored, &restored_ctx, pixels, "Menu");
    scale_click(&mut restored, &restored_ctx, pixels, "Layout");
    scale_click(&mut restored, &restored_ctx, pixels, "Reset layout");
    for _ in 0..3 {
        scale_frame(&mut restored, &restored_ctx, pixels, vec![]);
    }
    assert_eq!(restored_ctx.zoom_factor(), 1.);
    assert_eq!(restored.layout, session::DesktopLayout {center:Some(session::CenterLayout {command:true,editor_revealed:true}),..session::DesktopLayout::fresh_desktop()});
    assert_eq!(restored.documents[0].text, text);
}

#[test]
fn desktop_scale_shortcuts_are_bounded_and_compact_tabs_do_not_change_split_preference() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    // Split/tab geometry requires an actual figure; an empty desktop intentionally shows only Editor.
    app.run("plot([1 2],[2 3]);".into());
    finish(&mut app);
    let pixels = egui::vec2(960., 650.);
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: true,
        modifiers,
    };
    for _ in 0..8 {
        scale_frame(
            &mut app,
            &ctx,
            pixels,
            vec![key(
                egui::Key::Plus,
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            )],
        );
    }
    scale_frame(&mut app, &ctx, pixels, vec![]);
    assert_eq!(ctx.zoom_factor(), 1.5);
    app.recovery_dirty = false;
    scale_frame(
        &mut app,
        &ctx,
        pixels,
        vec![key(egui::Key::Plus, egui::Modifiers::CTRL)],
    );
    assert!(
        !app.recovery_dirty,
        "A shortcut at the limit must not queue another checkpoint"
    );
    let output = scale_frame(&mut app, &ctx, pixels, vec![]);
    for label in [
        "Menu",
        "Help",
        "Workspace",
        "Script editor",
        "Plot figure",
        "Command Window",
    ] {
        let p = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, label).filter(|p| s.clip_rect.contains(*p)))
            .unwrap_or_else(|| panic!("Missing {label}"));
        assert!(ctx.content_rect().contains(p), "Off-screen {label}: {p:?}");
    }
    assert_eq!(app.layout.mode, layout::PaneMode::Split);
    for _ in 0..3 {
        scale_frame(&mut app, &ctx, egui::vec2(1920., 1080.), vec![]);
    }
    assert!(
        egui::containers::panel::PanelState::load(&ctx, egui::Id::new("editor_split")).is_some()
    );
    assert_eq!(app.layout.mode, layout::PaneMode::Split);
    for _ in 0..8 {
        scale_frame(
            &mut app,
            &ctx,
            pixels,
            vec![key(egui::Key::Minus, egui::Modifiers::CTRL)],
        );
    }
    scale_frame(&mut app, &ctx, pixels, vec![]);
    assert_eq!(ctx.zoom_factor(), 0.75);
    scale_frame(
        &mut app,
        &ctx,
        pixels,
        vec![key(egui::Key::Num0, egui::Modifiers::CTRL | egui::Modifiers::ALT)],
    );
    scale_frame(&mut app, &ctx, pixels, vec![]);
    assert_eq!(ctx.zoom_factor(), 1.);
    assert_eq!(app.layout.ui_scale_percent, None);
}

#[test]
fn desktop_scale_reapplies_saved_panel_proportions_after_viewport_changes() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    // This regression intentionally exercises the supported right-dock boundary.
    app.set_workspace_dock(&ctx, session::WorkspaceDock::Right);
    // Split/tab geometry requires an actual figure; an empty desktop intentionally shows only Editor.
    app.run("plot([1 2],[2 3]);".into());
    finish(&mut app);
    app.layout.editor_fraction = Some(0.5);
    let wide = egui::vec2(1920., 1080.);
    for percent in [75, 125, 150, 100, 75] {
        app.set_desktop_scale(&ctx, percent);
        for _ in 0..4 {
            scale_frame(&mut app, &ctx, wide, vec![]);
        }
        let sidebar = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("workspace"))
            .unwrap()
            .outer_rect;
        let editor = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("editor_split"))
            .unwrap()
            .outer_rect;
        let workspace = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("workspace_variables"))
            .unwrap().outer_rect;
        // The central card lies between both docked sidebars: 4 px card
        // margins, the 1 px outline and 1 px inner margin on each side.
        let available = workspace.left() - sidebar.right() - 12.;
        assert!(available >= 580., "This fixture must exercise the actual split at scale {percent}");
        assert!(
            (editor.width() - available * 0.5).abs() < 2.,
            "scale {percent}: editor {editor:?}, available {available}"
        );
        assert_eq!(app.layout.editor_fraction, Some(0.5));
        for _ in 0..3 {
            scale_frame(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
        }
    }
}

#[test]
fn desktop_scale_short_figure_only_scroll_reveals_complete_axis_label() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    // This regression intentionally exercises the supported right-dock boundary.
    app.set_workspace_dock(&ctx, session::WorkspaceDock::Right);
    app.run("plot([1 2 3],[2 1 4]);xlabel('Complete axis label');".into());
    finish(&mut app);
    let samples = app.figure.as_ref().unwrap().series[0].points.clone();
    app.layout.mode = layout::PaneMode::Figure;
    app.set_desktop_scale(&ctx, 150);
    let pixels = egui::vec2(960., 650.);
    for _ in 0..4 {
        scale_frame(&mut app, &ctx, pixels, vec![]);
    }
    let auto = app.view_auto;
    let output = scale_scroll_figure(&mut app, &ctx, pixels);
    assert!(output.shapes.iter().any(|s| {
        if let egui::Shape::Text(text) = &s.shape {
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            text.galley.text() == "Complete axis label"
                && s.clip_rect.contains_rect(rect)
                && ctx.content_rect().contains_rect(rect)
        } else {
            false
        }
    }));
    assert!((samples).same_storage(&app.figure.as_ref().unwrap().series[0].points));
    assert_eq!(app.view_auto, auto);
}

fn scale_scroll_figure(
    app: &mut Workspace,
    ctx: &egui::Context,
    pixels: egui::Vec2,
) -> egui::FullOutput {
    for _ in 0..3 {
        scale_frame(app, ctx, pixels, vec![]);
    }
    let console = egui::containers::panel::PanelState::load(ctx, egui::Id::new("console"))
        .unwrap()
        .outer_rect;
    let workspace = egui::containers::panel::PanelState::load(ctx, egui::Id::new("workspace_variables"))
        .unwrap().outer_rect;
    // Select the actual interactive scrollbar at the central pane edge, not the right Workspace edge.
    let bar = ctx
        .interactive_rects_last_pass()
        .into_iter()
        .filter(|r| {
            r.width() >= 4.
                && r.width() <= 24.
                && r.height() > 80.
                && r.right() > workspace.left() - 24.
                && r.right() <= workspace.left()
                && r.left() >= console.left()
                && r.bottom() <= console.top() + 1.
        })
        .max_by(|a, b| a.height().total_cmp(&b.height()))
        .expect("Visible figure scrollbar");
    let from = egui::pos2(bar.center().x, bar.top() + 8.);
    let to = egui::pos2(bar.center().x, bar.bottom() - 2.);
    scale_frame(app, ctx, pixels, vec![egui::Event::PointerMoved(from)]);
    pointer_frame(app, ctx, pixels / ctx.zoom_factor(), from, true);
    scale_frame(app, ctx, pixels, vec![egui::Event::PointerMoved(to)]);
    pointer_frame(app, ctx, pixels / ctx.zoom_factor(), to, false);
    scale_frame(app, ctx, pixels, vec![])
}
