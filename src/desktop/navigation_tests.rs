// Files, Workspace and the Command Window are always-visible cards: there are
// no rails. History opens from the Command Window header; examples from Apps.

#[test]
fn history_menu_fills_the_input_and_keeps_documents_and_workspace() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run("A=[1 2;3 4];".into());
        finish(&mut app);
        let data = app.variables["A"].clone();
        let doc = app.documents[app.active].id;
        let text = app.documents[app.active].text.clone();
        app.command = "A + 1".into();
        app.history.push("A + 2".into());
        let prefs = app.layout;
        // No rails: the three cards are painted without any toggle.
        let output = frame_at(&mut app, &ctx, size, vec![]);
        for label in ["Files", "Workspace", "Command Window"] {
            assert!(
                output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()),
                "{label} card"
            );
        }
        assert!(ctx.read_response(egui::Id::new(("desktop_rail", "files"))).is_none());
        click_label_at(&mut app, &ctx, size, "History");
        frame_at(&mut app, &ctx, size, vec![]);
        click_label_at(&mut app, &ctx, size, "A + 2");
        assert_eq!(app.command, "A + 2");
        assert_eq!(app.layout, prefs);
        assert_eq!(app.documents[app.active].id, doc);
        assert_eq!(app.documents[app.active].text, text);
        assert!(data.same_snapshot(&app.variables["A"]));
    }
}

#[test]
fn apps_examples_menu_opens_a_real_script() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let size = egui::vec2(960., 650.);
    let count = app.documents.len();
    click_label_at(&mut app, &ctx, size, "Apps");
    click_label_at(&mut app, &ctx, size, "More");
    click_label_at(&mut app, &ctx, size, "Examples");
    click_label_at(&mut app, &ctx, size, "Damped oscillation");
    assert_eq!(app.documents.len(), count + 1);
    assert_eq!(app.documents[app.active].text, DEMO);
}

#[test]
fn reset_layout_retains_theme_and_panels_are_always_shown() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.layout.theme = Some(session::DesktopTheme::Dark);
    app.reset_layout(&ctx);
    assert_eq!(app.layout.theme, Some(session::DesktopTheme::Dark));
    assert!(app.workspace_on_left());
    assert_eq!(app.navigation.left, Some(navigation::Left::Files));
    assert!(app.navigation.workspace && app.navigation.command);
}

fn pane_focus_key(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        frame_at(app, ctx, size, vec![egui::Event::Key {
            key: egui::Key::Num0,
            physical_key: Some(egui::Key::Num0),
            pressed,
            repeat: false,
            modifiers,
        }]);
    }
}

#[test]
fn matlab_pane_shortcuts_reveal_and_edit_real_fields_in_both_themes() {
    for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.), egui::vec2(640., 480.)] {
            let ctx = egui::Context::default();
            let mut app = legacy_sample_desktop(ctx.clone(), true);
            app.layout.theme = Some(theme);
            app.apply_desktop_theme(&ctx);
            app.run("x=[1 2 3];alias=x;plot(x,x);".into());
            finish(&mut app);
            let data = app.variables["x"].clone();
            let points = app.figure.as_ref().unwrap().series[0].points.clone();
            let document = app.documents[app.active].id;
            app.documents[app.active].text.clear();
            app.command.clear();
            app.layout.mode = layout::PaneMode::Figure;
            app.compact_figure = true;
            app.navigation.command = false;
            for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
            assert_eq!(ctx.global_style().visuals.dark_mode, theme == session::DesktopTheme::Dark);

            pane_focus_key(&mut app, &ctx, size, egui::Modifiers::CTRL);
            assert!(app.navigation.command);
            let command = egui::Id::new("command_input");
            assert!(ctx.memory(|m| m.has_focus(command)));
            assert!(ctx.content_rect().contains_rect(ctx.read_response(command).unwrap().rect));
            frame_at(&mut app, &ctx, size, vec![egui::Event::Text("command_only=9;".into())]);
            assert_eq!(app.command, "command_only=9;");
            assert!(app.documents[app.active].text.is_empty());
            assert!(!app.busy && !app.variables.contains_key("command_only"));

            pane_focus_key(&mut app, &ctx, size, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
            assert_eq!(app.layout.mode, layout::PaneMode::Split);
            assert!(!app.compact_figure);
            let editor = editor::id(document);
            assert!(ctx.memory(|m| m.has_focus(editor)));
            assert!(ctx.content_rect().intersects(ctx.read_response(editor).unwrap().rect));
            frame_at(&mut app, &ctx, size, vec![egui::Event::Text("script_only=7;".into())]);
            assert_eq!(app.documents[app.active].text, "script_only=7;");
            assert!(app.documents[app.active].dirty);
            assert_eq!(app.documents[app.active].id, document);
            assert_eq!(app.command, "command_only=9;");
            assert_eq!(app.generic_save_target(), document_save::Pane::Script);
            assert!(!app.busy && !app.variables.contains_key("script_only"));
            assert!(data.same_snapshot(&app.variables["x"]));
            assert!(data.same_snapshot(&app.variables["alias"]));
            assert!(points.same_storage(&app.figure.as_ref().unwrap().series[0].points));
        }
    }
}

#[test]
fn matlab_pane_focus_menu_and_scale_reset_are_discoverable_and_distinct() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let size = egui::vec2(960., 650.);
    app.set_desktop_scale(&ctx, 125);
    // Ctrl+0 changes focus, never the user's scale.
    pane_focus_key(&mut app, &ctx, size, egui::Modifiers::CTRL);
    assert_eq!(app.layout.ui_scale_percent(), 125);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
    pane_focus_key(&mut app, &ctx, size, egui::Modifiers::CTRL | egui::Modifiers::ALT);
    assert_eq!(app.layout.ui_scale_percent(), 100);
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    click_label_at(&mut app, &ctx, size, "Menu");
    click_label_at(&mut app, &ctx, size, "Layout");
    click_label_at(&mut app, &ctx, size, "Focus Editor");
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(ctx.memory(|m| m.has_focus(editor::id(app.documents[app.active].id))));
    frame_at(&mut app, &ctx, size, vec![egui::Event::Text("menu_editor=1;".into())]);
    assert!(app.documents[app.active].text.contains("menu_editor=1;"));
    click_label_at(&mut app, &ctx, size, "Menu");
    click_label_at(&mut app, &ctx, size, "Layout");
    click_label_at(&mut app, &ctx, size, "Focus Command Window");
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
}

#[test]
fn matlab_pane_focus_respects_modal_and_busy_and_does_not_queue_late_focus() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let size = egui::vec2(960., 650.);
    app.documents[app.active].text = "keep=42;".into();
    app.documents[app.active].changed();
    app.layout.mode = layout::PaneMode::Figure;
    app.compact_figure = true;
    app.navigation.command = false;
    app.request_script_close(app.active);
    assert!(app.file_action_pending());
    for modifiers in [egui::Modifiers::CTRL, egui::Modifiers::CTRL | egui::Modifiers::SHIFT] {
        pane_focus_key(&mut app, &ctx, size, modifiers);
        assert!(!app.navigation.command);
        assert_eq!(app.layout.mode, layout::PaneMode::Figure);
        assert!(app.compact_figure);
        assert_eq!(app.documents[app.active].text, "keep=42;");
    }
    click_label_at(&mut app, &ctx, size, "Cancel");
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    assert!(!app.navigation.command && app.compact_figure);
    // Busy-state rejection is a UI-state test, not a timed worker simulation.
    app.busy = true;
    pane_focus_key(&mut app, &ctx, size, egui::Modifiers::CTRL);
    app.busy = false;
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    assert!(!app.navigation.command);
    pane_focus_key(&mut app, &ctx, size, egui::Modifiers::CTRL);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
}

#[test]
fn matlab_pane_focus_waits_for_final_pass_and_rechecks_late_modal() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let size = egui::vec2(960., 650.);
    app.navigation.command = false;
    for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
    let modifiers = egui::Modifiers::CTRL;
    let mut passes = 0;
    crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key {
            key: egui::Key::Num0, physical_key: Some(egui::Key::Num0),
            pressed: true, repeat: false, modifiers,
        }], ..Default::default()
    }, |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("pane focus retry"); }
        app.ui(root);
        if passes == 1 { assert!(!app.navigation.command); }
    });
    assert!(passes >= 2);
    assert!(app.navigation.command);
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
    app.navigation.command = false;
    app.documents[app.active].text = "late=1;".into();
    app.documents[app.active].changed();
    crate::headless::run(&ctx, egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key {
            key: egui::Key::Num0, physical_key: Some(egui::Key::Num0),
            pressed: true, repeat: false, modifiers,
        }], ..Default::default()
    }, |root| {
        app.pane_focus_shortcuts(root.ctx());
        app.request_script_close(app.active);
        app.finish_pane_focus(root.ctx());
        assert!(!app.navigation.command);
    });
    assert!(app.file_action_pending());
}


#[test]
fn sessions_saved_with_hidden_panels_restore_the_always_visible_cards() {
    use session::SidebarPanel;
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("panel-session.json");
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().to_owned();
    app.run("a=[1 2;3 4];".into());
    finish(&mut app);
    let data = app.variables["a"].clone();
    // An older build could hide panels from its rails.
    app.layout.panels = Some(session::PanelLayout {
        sidebar: SidebarPanel::Hidden,
        workspace: false,
        command: false,
    });
    app.snapshot().save(&path).unwrap();
    let mut restored = Workspace::with_context(egui::Context::default(), true);
    restored.restore_session(Session::load(&path).unwrap());
    restored.start_session_recovery(path.clone());
    finish(&mut restored);
    assert_eq!(restored.navigation.left, Some(navigation::Left::Files));
    assert!(restored.navigation.workspace && restored.navigation.command);
    assert_eq!(restored.variables["a"].as_numeric().unwrap().data(), data.as_numeric().unwrap().data());
}
