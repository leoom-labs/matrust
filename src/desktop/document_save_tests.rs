fn document_save_key(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, save_as: bool) {
    let modifiers = if save_as {
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::CTRL
    };
    frame_at(app, ctx, size, vec![egui::Event::Key {
        key: egui::Key::S, physical_key: None, pressed: true, repeat: false, modifiers,
    }]);
}

fn document_save_heading(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, label: &str) -> egui::Pos2 {
    let mut position = None;
    for _ in 0..3 {
        position = frame_at(app, ctx, size, vec![]).shapes.iter().rev()
            .find_map(|shape| text_position(&shape.shape, label));
    }
    position.unwrap_or_else(|| panic!("Missing document heading {label}"))
}

#[test]
fn document_save_figure_only_writes_mfig_and_fig_without_touching_hidden_script_or_samples() {
    for extension in ["mfig", "fig"] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = figure_save_all_app(&ctx, dir.path(), 1);
        let figure_path = dir.path().join(format!("saved.{extension}"));
        app.submit(Job::SaveFigure(figure_path.clone(), None));
        finish(&mut app);
        app.run("title('Changed visible figure');".into());
        finish(&mut app);
        let scripts = named_dirty_scripts(&mut app, dir.path(), 1);
        let samples = app.figure.as_ref().unwrap().series[0].points.clone();
        let data = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
        app.layout.mode = layout::PaneMode::Figure;
        frame(&mut app, &ctx, vec![]);
        app.view_bounds = Some([1.1, 1.9, 1.2, 1.8]);
        app.view_auto = [false; 2];
        app.record_figure_view([1.1, 1.9, 1.2, 1.8], [false; 2]);
        app.reset_plot = true;
        app.restore_plot_view = true;
        document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
        finish(&mut app);
        let saved = matrust::files::load_figure(&figure_path).unwrap();
        assert_eq!(saved.title, "Changed visible figure");
        assert_eq!(saved.xlim, Some([1.1, 1.9]));
        assert_eq!(saved.ylim, Some([1.2, 1.8]));
        assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "old=1;\n");
        assert!(app.documents[0].dirty && !app.figure_dirty(1));
        assert!((samples).same_storage(&app.figure.as_ref().unwrap().series[0].points));
        assert_eq!(data, app.variables["keep"].as_numeric().unwrap().data().as_ptr());
        assert_eq!(app.current_figure, 1);
    }
}

#[test]
fn document_save_compact_tab_clicks_and_resizing_select_the_visible_document() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 1);
    let small = egui::vec2(640., 480.);
    app.layout.mode = layout::PaneMode::Split;
    click_label_at(&mut app, &ctx, small, "Plot figure");
    document_save_key(&mut app, &ctx, small, false);
    finish(&mut app);
    assert!(!app.figure_dirty(1) && app.documents[0].dirty);
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "old=1;\n");
    click_label_at(&mut app, &ctx, small, "Script editor");
    document_save_key(&mut app, &ctx, small, false);
    finish(&mut app);
    assert!(!app.documents[0].dirty);
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "value=0;\n");
    for percent in [75, 100, 150] {
        app.set_desktop_scale(&ctx, percent);
        app.document_save.intercepted = Some(Vec::new());
        app.show_plot_pane();
        document_save_key(&mut app, &ctx, small, true);
        assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
            document_save::Request::Figure { figure: 1, save_as: true },
        ][..]));
        app.reveal_editor();
        document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), true);
        assert_eq!(app.document_save.intercepted.as_ref().unwrap().last(), Some(&
            document_save::Request::Script { document: app.documents[0].id, save_as: true }
        ));
    }
}

#[test]
fn document_save_split_pointer_and_key_win_over_editor_focus_and_hover_does_not_switch() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 1);
    let size = egui::vec2(1440., 900.);
    app.layout.mode = layout::PaneMode::Split;
    let plot = document_save_heading(&mut app, &ctx, size, "Figure 1");
    let editor_pos = document_save_heading(&mut app, &ctx, size, "Editor");
    let editor_id = editor::id(app.documents[0].id);
    ctx.memory_mut(|memory| memory.request_focus(editor_id));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    frame_at(&mut app, &ctx, size, vec![
        egui::Event::PointerMoved(plot),
        egui::Event::PointerButton { pos: plot, button: egui::PointerButton::Primary,
            pressed: true, modifiers: egui::Modifiers::CTRL },
        egui::Event::Key { key: egui::Key::S, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::CTRL },
    ]);
    finish(&mut app);
    assert!(!app.figure_dirty(1) && app.documents[0].dirty);
    pointer_frame(&mut app, &ctx, size, plot, false);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(editor_pos)]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    // A genuine later keyboard-focus transition activates the script again.
    ctx.memory_mut(|memory| memory.surrender_focus(editor_id));
    frame(&mut app, &ctx, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(editor_id));
    document_save_key(&mut app, &ctx, size, false);
    finish(&mut app);
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "value=0;\n");
    assert!(!app.documents[0].dirty);
}

#[test]
fn document_save_file_menu_names_and_saves_the_active_figure_then_script() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.layout.mode = layout::PaneMode::Figure;
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Save figure");
    finish(&mut app);
    assert!(!app.figure_dirty(1) && app.documents[0].dirty);
    app.layout.mode = layout::PaneMode::Editor;
    // Use Plots toolstrip so there is no duplicate explicit Save script label.
    app.layout.toolstrip = matrust::session::ToolstripTab::Plots;
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Save script");
    finish(&mut app);
    assert!(!app.documents[0].dirty);
}

#[test]
fn document_save_keyboard_focus_in_a_long_scrolled_editor_uses_the_visible_interaction_rect() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.documents[0].text = format!("% {}\n", "long editor content ".repeat(20)).repeat(400);
    app.documents[0].changed();
    app.layout.mode = layout::PaneMode::Split;
    let size = egui::vec2(1440., 900.);
    let editor_id = editor::id(app.documents[0].id);
    let plot = document_save_heading(&mut app, &ctx, size, "Figure 1");
    let response = ctx.read_response(editor_id).unwrap();
    assert!(response.interact_rect.is_positive());
    assert!(!response.interact_rect.contains(response.rect.center()),
        "Regression requires a full TextEdit rectangle larger than its clipped pane");
    for pressed in [true, false] {
        pointer_frame(&mut app, &ctx, size, plot, pressed);
    }
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    ctx.memory_mut(|memory| memory.surrender_focus(editor_id));
    frame(&mut app, &ctx, vec![]);
    app.document_save.intercepted = Some(Vec::new());
    ctx.memory_mut(|memory| memory.request_focus(editor_id));
    document_save_key(&mut app, &ctx, size, false);
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Script { document: app.documents[0].id, save_as: false },
    ][..]));
    assert!(app.documents[0].dirty && app.figure_dirty(1));
}

#[test]
fn document_save_uses_press_origin_when_pointer_moves_or_releases_before_save_in_one_frame() {
    for release in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = figure_save_all_app(&ctx, dir.path(), 1);
        named_dirty_scripts(&mut app, dir.path(), 1);
        app.layout.mode = layout::PaneMode::Split;
        let size = egui::vec2(1440., 900.);
        let plot = document_save_heading(&mut app, &ctx, size, "Figure 1");
        let editor_pos = document_save_heading(&mut app, &ctx, size, "Editor");
        app.document_save.intercepted = Some(Vec::new());
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        let mut events = vec![
            egui::Event::PointerMoved(plot),
            egui::Event::PointerButton { pos: plot, button: egui::PointerButton::Primary,
                pressed: true, modifiers },
            egui::Event::PointerMoved(editor_pos),
        ];
        if release {
            events.push(egui::Event::PointerButton { pos: editor_pos,
                button: egui::PointerButton::Primary, pressed: false, modifiers });
        }
        events.push(egui::Event::Key { key: egui::Key::S, physical_key: None,
            pressed: true, repeat: false, modifiers });
        frame_at(&mut app, &ctx, size, events);
        assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
            document_save::Request::Figure { figure: 1, save_as: true },
        ][..]));
    }
}

#[test]
fn document_save_preview_routes_both_shortcuts_to_copy_and_restores_previous_figure() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 1);
    app.show_plot_pane();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/preview/tiff-preview/reference-1.png");
    let source = dir.path().join("source.png");
    std::fs::copy(fixture, &source).unwrap();
    let original = std::fs::read(&source).unwrap();
    app.submit(Job::OpenImage(source.clone()));
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    let pixels = app.image_view.as_ref().unwrap().pixels.clone();
    app.document_save.intercepted = Some(Vec::new());
    for save_as in [false, true] {
        document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), save_as);
    }
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::PreviewCopy, document_save::Request::PreviewCopy,
    ][..]));
    // Click the actual foreground preview, whose rectangle covers document panes.
    click_label(&mut app, &ctx, "1:1 preview");
    assert_eq!(app.generic_save_target(), document_save::Pane::Preview);
    assert!(Arc::ptr_eq(&pixels, &app.image_view.as_ref().unwrap().pixels));
    app.export_preview_to(source.clone());
    finish(&mut app);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(app.image_view.as_ref().unwrap().error.is_some());
    app.export_preview_to(dir.path().join("preview-copy.png"));
    finish(&mut app);
    assert!(dir.path().join("preview-copy.png").is_file());
    assert!(app.documents[0].dirty && app.figure_dirty(1));
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "old=1;\n");
    app.close_image_preview();
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), true);
    assert_eq!(app.document_save.intercepted.as_ref().unwrap().last(), Some(&
        document_save::Request::Figure { figure: 1, save_as: true }
    ));
}

#[test]
fn document_save_empty_plot_busy_and_modal_never_save_the_hidden_script() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 1);
    app.layout.mode = layout::PaneMode::Figure;
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    assert!(app.status.contains("No editable figure"));
    app.layout.mode = layout::PaneMode::Split;
    let compact = egui::vec2(640., 480.);
    // Empty Split deliberately shows only the editor. Select the empty Figure
    // through the real layout menu instead of searching for a nonexistent tab.
    for _ in 0..3 {
        frame_at(&mut app, &ctx, compact, vec![]);
    }
    let output = frame_at(&mut app, &ctx, compact, vec![]);
    assert!(output.shapes.iter().all(|shape| {
        text_position(&shape.shape, "Plot figure").is_none()
    }));
    click_label_at(&mut app, &ctx, compact, "Menu");
    click_label_at(&mut app, &ctx, compact, "Layout");
    click_label_at(&mut app, &ctx, compact, "Figure only");
    assert_eq!(app.layout.mode, layout::PaneMode::Figure);
    document_save_key(&mut app, &ctx, compact, false);
    assert!(app.status.contains("No editable figure"));
    app.document_save.intercepted = Some(Vec::new());
    app.layout.mode = layout::PaneMode::Editor;
    app.busy = true;
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    app.busy = false;
    app.pending_close = Some(0);
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    app.pending_close = None;
    frame(&mut app, &ctx, vec![]);
    assert!(app.document_save.intercepted.as_ref().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "old=1;\n");
    assert!(app.documents[0].dirty);
}

#[test]
fn document_save_retry_dispatches_save_as_once_without_opening_a_native_picker() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    app.layout.mode = layout::PaneMode::Figure;
    app.document_save.intercepted = Some(Vec::new());
    frame(&mut app, &ctx, vec![]);
    let modifiers = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.))),
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key {
            key: egui::Key::S, physical_key: None, pressed: true, repeat: false, modifiers,
        }],
        ..Default::default()
    }, |root| {
        passes += 1;
        app.ui(root);
        if passes == 1 { root.ctx().request_discard("Generic save publishes once"); }
    });
    assert!(passes >= 2);
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Figure { figure: 1, save_as: true },
    ][..]));
}

#[test]
fn document_save_preserves_figure_conflict_cancel_and_explicit_script_commands() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflict.mfig");
    let mut app = figure_conflict_app(&ctx, &path);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 2);
    app.layout.mode = layout::PaneMode::Figure;
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    finish(&mut app);
    assert!(app.figure_conflict.is_some());
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(matrust::files::load_figure(&path).unwrap().title, "External");
    assert!(app.figure_dirty(1) && app.documents.iter().all(|doc| doc.dirty));
    // Explicit Home toolstrip command still means script, even with Figure-only.
    app.layout.toolstrip = matrust::session::ToolstripTab::Home;
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    assert!(!app.documents[0].dirty && app.documents[1].dirty && app.figure_dirty(1));
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL | egui::Modifiers::ALT, egui::Key::S);
    finish_script_saves(&mut app, &ctx);
    assert!(app.documents.iter().all(|doc| !doc.dirty) && app.figure_dirty(1));
    for (i, path) in scripts.iter().enumerate() {
        assert_eq!(std::fs::read_to_string(path).unwrap(), format!("value={i};\n"));
    }
}

#[test]
fn document_save_late_preview_and_figure_completion_preserve_intervening_script_activation() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/preview/tiff-preview/reference-1.png");
    // Direct new preview and same-path reload both capture activation at accepted
    // submission. Explicit script activation occurs before receiving completion.
    for _ in 0..2 {
        app.submit(Job::OpenImage(source.clone()));
        assert!(app.busy);
        app.reveal_editor();
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert!(app.image_view.is_some());
        assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    }
    app.close_image_preview();
    // The preflight phase must carry an old activation decision through the
    // decoder phase, rather than recapturing an unrelated newer activation.
    assert!(app.try_open_path(source));
    app.reveal_editor();
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert!(app.image_view.is_some());
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    app.close_image_preview();
    app.layout.mode = layout::PaneMode::Editor;
    assert!(app.try_open_path(dir.path().join("1.mfig")));
    app.reveal_editor();
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.layout.mode, layout::PaneMode::Editor);
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    app.run("plot([9 8 7]);".into());
    app.reveal_editor();
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    assert!(app.documents[0].dirty);
}

#[test]
fn document_save_late_script_open_keeps_intervening_figure_and_existing_script_identity() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    let active_id = app.documents[app.active].id;
    let new_path = dir.path().join("background.m");
    std::fs::write(&new_path, "% opened without executing\n").unwrap();
    assert!(app.try_open_path(new_path.clone()));
    app.show_plot_pane();
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.documents.len(), 2);
    assert_eq!(app.documents[app.active].id, active_id);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    assert!(app.try_open_path(new_path));
    app.show_plot_pane();
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.documents.len(), 2);
    assert_eq!(app.documents[app.active].id, active_id);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
}

#[test]
fn document_save_input_in_preflight_completion_frame_retains_the_edited_script() {
    for (kind, event) in [
        ("script", egui::Event::Text("typed".into())),
        ("figure", egui::Event::Paste("typed".into())),
        ("image", egui::Event::Ime(egui::ImeEvent::Commit("typed".into()))),
        ("figure", egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "typed".into(), active_range_chars: None,
        })),
    ] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = figure_save_all_app(&ctx, dir.path(), 1);
        named_dirty_scripts(&mut app, dir.path(), 1);
        app.layout.mode = layout::PaneMode::Split;
        let active_id = app.documents[0].id;
        frame(&mut app, &ctx, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(editor::id(active_id)));
        frame(&mut app, &ctx, vec![]);
        app.show_plot_pane();
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(editor::id(active_id)));
        let path = match kind {
            "script" => {
                let path = dir.path().join("late.m");
                std::fs::write(&path, "% untouched background tab\n").unwrap();
                path
            }
            "figure" => dir.path().join("1.mfig"),
            _ => std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/preview/tiff-preview/reference-1.png"),
        };
        assert!(app.try_open_path(path));
        // Retain a real worker reply, then deliver it deterministically in the
        // same frame as typing. The original bounded channel is restored before
        // waiting for any decoder job that complete_open subsequently submits.
        let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        assert!(send.send(reply).is_ok());
        let original_receive = std::mem::replace(&mut app.worker.receive, receive);
        frame(&mut app, &ctx, vec![event]);
        app.worker.receive = original_receive;
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.documents[app.active].id, active_id);
        assert_eq!(app.generic_save_target(), document_save::Pane::Script);
        assert!(app.documents[0].text.contains("typed"));
        if kind == "script" {
            assert_eq!(app.documents[1].text, "% untouched background tab\n");
        }
    }
}

#[test]
fn document_save_retained_editor_focus_does_not_reroute_figure_save_or_empty_ime_notifications() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.layout.mode = layout::PaneMode::Split;
    frame(&mut app, &ctx, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(editor::id(app.documents[0].id)));
    frame(&mut app, &ctx, vec![]);
    app.show_plot_pane();
    frame(&mut app, &ctx, vec![]);
    app.document_save.intercepted = Some(Vec::new());
    frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))]);
    for save_as in [false, true] {
        document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), save_as);
    }
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Figure { figure: 1, save_as: false },
        document_save::Request::Figure { figure: 1, save_as: true },
    ][..]));
    assert!(app.documents[0].dirty && app.figure_dirty(1));
}

#[test]
fn document_save_keyboard_navigation_in_file_menu_preserves_the_figure_target() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.layout.mode = layout::PaneMode::Split;
    frame(&mut app, &ctx, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(editor::id(app.documents[0].id)));
    frame(&mut app, &ctx, vec![]);
    app.show_plot_pane();
    frame(&mut app, &ctx, vec![]);
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    assert!(egui::Popup::is_any_open(&ctx));
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::ArrowDown);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape);
    assert!(!egui::Popup::is_any_open(&ctx));
    app.document_save.intercepted = Some(Vec::new());
    document_save_key(&mut app, &ctx, egui::vec2(1440., 900.), false);
    assert_eq!(app.document_save.intercepted.as_deref(), Some(&[
        document_save::Request::Figure { figure: 1, save_as: false },
    ][..]));
    assert!(app.documents[0].dirty && app.figure_dirty(1));
}


mod explicit_root_figure_save_cases {
    use super::*;
    const WIDE: egui::Vec2 = egui::vec2(1440., 900.);

    fn setup(size: egui::Vec2, legacy: bool) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let mut app = if legacy { legacy_sample_desktop(ctx.clone(), true) }
            else { Workspace::with_context(ctx.clone(), true) };
        let dir = tempfile::tempdir().unwrap();
        assert!(app.change_folder(dir.path().to_path_buf()));
        finish_folder(&mut app);
        app.run("keep=[11 22 33];alias=keep;figure(2);plot([1 2],[4 5]);title('Other');figure(1);plot([1 2],[2 3]);title('Explicit root save');".into());
        finish(&mut app);
        named_dirty_scripts(&mut app, dir.path(), 1);
        app.layout.mode = layout::PaneMode::Figure;
        finish_folder(&mut app);
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        (ctx, app, dir)
    }
    fn point(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output.shapes.iter().rev().find_map(|shape| text_position(&shape.shape, label))
            .unwrap_or_else(|| panic!("Missing Figure Save control {label}"))
    }
    fn menu(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, save_as: bool) -> egui::Pos2 {
        if save_as {
            click_label_at(app, ctx, size, "Menu");
            click_label_at(app, ctx, size, "File");
            click_label_at(app, ctx, size, "Figures");
        } else {
            click_label_at(app, ctx, size, "Actions");
        }
        let mut output = frame_at(app, ctx, size, vec![]);
        for _ in 0..2 { output = frame_at(app, ctx, size, vec![]); }
        point(&output, if save_as { "Save as\u{2026}" } else { "Save figure" })
    }
    fn ribbon(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
        click_label_at(app, ctx, size, "Plots");
        let mut output = frame_at(app, ctx, size, vec![]);
        for _ in 0..2 { output = frame_at(app, ctx, size, vec![]); }
        point(&output, "Save figure")
    }
    fn release_discarded(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
        pos: egui::Pos2, mut between: impl FnMut(&mut Workspace, &egui::Context)) {
        let calls = app.root_figure_save.picker_calls.len();
        let mut passes = 0;
        crate::headless::run(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE,
            }], ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("Explicit Figure Save must wait for final admission"); }
            app.ui(root);
            if passes == 1 {
                assert_eq!(app.root_figure_save.picker_calls.len(), calls);
                assert!(!app.busy, "No save dispatch on discarded release");
                between(app, ctx);
            }
        });
        assert!(passes >= 2);
    }
    fn search(app: &mut Workspace, ctx: &egui::Context) {
        click_label_at(app, ctx, WIDE, "Search functions and actions");
        for _ in 0..2 { frame_at(app, ctx, WIDE, vec![]); }
        assert!(app.global_search.is_open());
        assert!(ctx.memory(|m| m.has_focus(global_search::query_id())));
        frame_at(app, ctx, WIDE, vec![egui::Event::Text("Save figure".into())]);
        frame_at(app, ctx, WIDE, vec![]);
        command_first_key(app, ctx, WIDE, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.global_search.is_open());
    }

    #[test]
    fn root_figure_ribbon_saves_real_formats_despite_script_foreground_and_preserves_view_and_aliases() {
        for size in [egui::vec2(960., 650.), WIDE] {
            for legacy in [false, true] {
                let (ctx, mut app, dir) = setup(size, legacy);
                app.layout.mode = layout::PaneMode::Editor;
                for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
                assert_eq!(app.generic_save_target(), document_save::Pane::Script);
                let before = app.all_figures.clone();
                let value = app.variables["keep"].clone();
                let script = (app.documents[0].id, app.documents[0].text.clone(), app.documents[0].path.clone());
                let view = [1.1, 1.9, 2.1, 2.9];
                app.view_bounds = Some(view); app.view_auto = [false; 2];
                app.record_figure_view(view, [false; 2]);
                app.root_figure_save.picker = Some(if legacy {
                    |app, _| Some(app.folder.join("explicit.fig"))
                } else { |app, _| Some(app.folder.join("explicit.mfig")) });
                // Static FIG freezes the displayed theme RGB. Native MFIG
                // instead retains the automatic palette intent checked below.
                let expected_rgb = before[&1]
                    .resolve_appearance(app.worker.theme())
                    .series_color(&before[&1].series[0]);
                let pos = ribbon(&mut app, &ctx, size);
                pointer_frame(&mut app, &ctx, size, pos, true);
                release_discarded(&mut app, &ctx, size, pos, |_, _| {});
                assert_eq!(app.root_figure_save.picker_calls.len(), 1);
                assert!(app.busy, "Explicit Figure Save dispatched: {}", app.status);
                finish(&mut app);
                let saved = matrust::files::load_figure(&dir.path().join(if legacy {"explicit.fig"} else {"explicit.mfig"})).unwrap();
                assert_eq!(saved.title, before[&1].title);
                assert_eq!(saved.xlim, Some([view[0], view[1]]));
                assert_eq!(saved.ylim, Some([view[2], view[3]]));
                assert_eq!(saved.series[0].points, before[&1].series[0].points);
                if !legacy {
                    assert_eq!(saved.appearance, before[&1].appearance);
                    assert_eq!(saved.palette_cycle, before[&1].palette_cycle);
                    assert_eq!(saved.series[0].auto_palette, before[&1].series[0].auto_palette);
                    assert!(saved.series[0].auto_palette.is_some());
                } else {
                    assert_eq!(saved.series[0].color, expected_rgb);
                    assert!(saved.series[0].explicit_color);
                    assert!(saved.series[0].auto_palette.is_none());
                }
                assert_eq!(app.current_figure, 1);
                assert!(!app.figure_dirty(1));
                assert!(before[&2].same_snapshot(&app.all_figures[&2]));
                assert!(before[&1].identity.same_object(&app.all_figures[&1].identity));
                assert!(before[&1].series[0].points.same_storage(&app.all_figures[&1].series[0].points));
                assert!(value.same_snapshot(&app.variables["keep"]) && value.same_snapshot(&app.variables["alias"]));
                assert_eq!((app.documents[0].id, app.documents[0].text.clone(), app.documents[0].path.clone()), script);
                assert!(app.documents[0].dirty);
                assert_eq!(std::fs::read_to_string(dir.path().join("script0.m")).unwrap(), "old=1;\n");
                frame_at(&mut app, &ctx, size, vec![]);
                assert_eq!(app.root_figure_save.picker_calls.len(), 1);
            }
        }
    }

    #[test]
    fn root_figure_actions_save_as_and_existing_path_save_both_wait_for_final_release() {
        for size in [egui::vec2(960., 650.), WIDE] {
            let (ctx, mut app, dir) = setup(size, false);
            app.root_figure_save.picker = Some(|app, _| Some(app.folder.join("menu.mfig")));
            let pos = menu(&mut app, &ctx, size, true);
            pointer_frame(&mut app, &ctx, size, pos, true);
            release_discarded(&mut app, &ctx, size, pos, |_, _| {});
            assert_eq!(app.root_figure_save.picker_calls.len(), 1);
            assert_eq!(app.root_figure_save.picker_calls[0].0, 1);
            assert!(app.root_figure_save.picker_calls[0].1);
            finish(&mut app);
            assert_eq!(matrust::files::load_figure(&dir.path().join("menu.mfig")).unwrap().title, "Explicit root save");
            app.run("title('Saved through existing path');".into()); finish(&mut app);
            for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
            app.root_figure_save.picker = Some(|_, _| panic!("Existing path Save must not open picker"));
            let pos = menu(&mut app, &ctx, size, false);
            pointer_frame(&mut app, &ctx, size, pos, true);
            release_discarded(&mut app, &ctx, size, pos, |_, _| {});
            assert!(app.busy);
            finish(&mut app);
            assert_eq!(app.root_figure_save.picker_calls.len(), 1);
            assert_eq!(matrust::files::load_figure(&dir.path().join("menu.mfig")).unwrap().title, "Saved through existing path");
            assert!(app.documents[0].dirty);
        }
    }

    #[test]
    fn root_figure_cancel_is_consumed_through_late_discard_and_new_search_gesture_is_admitted() {
        let (ctx, mut app, _dir) = setup(WIDE, false);
        app.root_figure_save.picker = Some(|_, _| None);
        let before = app.all_figures[&1].clone();
        let pos = menu(&mut app, &ctx, WIDE, true);
        pointer_frame(&mut app, &ctx, WIDE, pos, true);
        let mut passes = 0;
        crate::headless::run(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, WIDE)),
            events: vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE,
            }], ..Default::default()
        }, |root| {
            passes += 1; app.ui(root);
            if passes == 1 { root.ctx().request_discard("Completed Cancel must not reopen chooser"); }
        });
        assert!(passes >= 2);
        assert_eq!(app.root_figure_save.picker_calls.len(), 1);
        assert!(!app.busy && app.figure_dirty(1));
        assert!(before.same_snapshot(&app.all_figures[&1]));
        assert!(app.status.contains("cancelled"));
        search(&mut app, &ctx);
        assert_eq!(app.root_figure_save.picker_calls.len(), 2);
        assert!(!app.busy && app.documents[0].dirty && app.figure_dirty(1));
        assert!(before.same_snapshot(&app.all_figures[&1]));
    }

    fn invalidate(app: &mut Workspace, ctx: &egui::Context, case: usize) {
        match case {
            0 => app.stop_work(),
            1 => app.busy = true,
            2 => app.pending_quit = true,
            3 => app.allow_close = true,
            4 => app.recovering_session = true,
            5 => app.recovery_blocked = true,
            6 => app.folder.push("different"),
            7 => app.new_document("different_owner=1;"),
            8 => { app.begin_workspace_action(false, true); assert!(app.workspace_tools.dialog.is_some()); },
            9 => ctx.memory_mut(|m| m.request_focus(egui::Id::new("late_figure_save_owner"))),
            10 => app.current_figure = 2,
            11 => {
                let identity = app.all_figures[&2].identity.clone();
                app.all_figures.get_mut(&1).unwrap().identity = identity;
            },
            12 => app.figure_documents.get_mut(&1).unwrap().revision += 1,
            13 => app.view_bounds = Some([0., 9., 0., 9.]),
            14 => app.figure_documents.get_mut(&1).unwrap().path = Some(app.folder.join("changed.mfig")),
            _ => unreachable!(),
        }
    }
    #[test]
    fn root_figure_save_rejects_late_authority_changes_before_picker_and_after_picker() {
        for after_picker in [false, true] {
            for case in 0..15 {
                let (ctx, mut app, dir) = setup(WIDE, false);
                let samples = app.all_figures[&1].series[0].points.clone();
                app.root_figure_save.picker = Some(if after_picker {
                    |app, ctx| {
                        let path = app.folder.join("must-not-exist.mfig");
                        let case = app.status.strip_prefix("Save test ").unwrap().parse().unwrap();
                        invalidate(app, ctx, case); Some(path)
                    }
                } else { |_, _| panic!("Stale root Figure Save opened picker") });
                let pos = menu(&mut app, &ctx, WIDE, true);
                pointer_frame(&mut app, &ctx, WIDE, pos, true);
                app.status = format!("Save test {case}");
                if after_picker { pointer_frame(&mut app, &ctx, WIDE, pos, false); }
                else { release_discarded(&mut app, &ctx, WIDE, pos, |app, ctx| invalidate(app, ctx, case)); }
                assert_eq!(app.root_figure_save.picker_calls.len(), usize::from(after_picker), "after={after_picker}, case={case}");
                assert_eq!(app.busy, case == 1, "after={after_picker}, case={case}");
                assert!(!dir.path().join("must-not-exist.mfig").exists());
                assert!(samples.same_storage(&app.all_figures[&1].series[0].points));
                assert!(app.documents[0].dirty);
            }
        }
    }

    #[test]
    fn root_figure_save_keeps_restricted_fig_mfig_fallback_and_original_disk_file() {
        let (ctx, mut app, dir) = setup(WIDE, true);
        let original = dir.path().join("original.fig");
        app.submit(Job::SaveFigure(original.clone(), None)); finish(&mut app);
        let bytes = std::fs::read(&original).unwrap();
        app.run("legend('line','Location','southwest');".into()); finish(&mut app);
        assert!(matrust::files::matlab_figure_export_restriction_with_theme(&app.all_figures[&1], app.worker.theme()).is_some());
        for _ in 0..3 { frame_at(&mut app, &ctx, WIDE, vec![]); }
        app.root_figure_save.picker = Some(|app, _| Some(app.folder.join("preserved.mfig")));
        search(&mut app, &ctx); finish(&mut app);
        assert_eq!(app.root_figure_save.picker_calls, vec![(1, false, false)]);
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        let saved = matrust::files::load_figure(&dir.path().join("preserved.mfig")).unwrap();
        assert_eq!(saved.legend, app.all_figures[&1].legend);
        assert_eq!(saved.legend_location, app.all_figures[&1].legend_location);
        assert_eq!(saved.series[0].points, app.all_figures[&1].series[0].points);
        assert!(app.documents[0].dirty && !app.figure_dirty(1));
    }

    fn visible_toolbar_save(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
        fn label_rect(shape: &egui::epaint::Shape) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == "Save figure" => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(label_rect),
                _ => None,
            }
        }
        // The figure card's Save lives in its overflow menu.
        click_label_at(app, ctx, size, "Actions");
        let mut output = frame_at(app, ctx, size, vec![]);
        for _ in 0..2 { output = frame_at(app, ctx, size, vec![]); }
        let labels: Vec<_> = output.shapes.iter()
            .filter_map(|shape| label_rect(&shape.shape).map(|rect| (shape.clip_rect, rect)))
            .collect();
        assert_eq!(labels.len(), 1, "Exactly one visible toolbar Save label");
        let (clip, rect) = labels[0];
        assert!(clip.contains_rect(rect) && egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
            "Toolbar Save text must fit its finite viewport: size={size:?}, clip={clip:?}, rect={rect:?}");
        rect.center()
    }

    #[test]
    fn visible_figure_toolbar_saves_selected_existing_document_without_touching_dirty_script() {
        for size in [egui::vec2(960., 650.), WIDE] {
            let (ctx, mut app, dir) = setup(size, false);
            app.run("figure(2);".into());
            finish(&mut app);
            let path = dir.path().join("toolbar-selected.mfig");
            app.submit(Job::SaveFigure(path.clone(), None));
            finish(&mut app);
            app.run("title('Toolbar selected figure');".into());
            finish(&mut app);
            finish_folder(&mut app);
            assert_eq!(app.current_figure, 2);
            assert!(app.figure_dirty(2));
            let other = app.all_figures[&1].clone();
            let selected = app.all_figures[&2].clone();
            let value = app.variables["keep"].clone();
            let script = (app.documents[0].id, app.documents[0].text.clone(), app.documents[0].path.clone());
            app.root_figure_save.picker = Some(|_, _| panic!("Toolbar existing-path Save must not open a chooser"));
            let pos = visible_toolbar_save(&mut app, &ctx, size);
            pointer_frame(&mut app, &ctx, size, pos, true);
            release_discarded(&mut app, &ctx, size, pos, |_, _| {});
            assert!(app.busy, "Actual toolbar release submits the selected figure");
            finish(&mut app);
            let saved = matrust::files::load_figure(&path).unwrap();
            assert_eq!(saved.title, "Toolbar selected figure");
            assert_eq!(saved.series[0].points, selected.series[0].points);
            assert_eq!(app.current_figure, 2);
            assert!(!app.figure_dirty(2));
            assert!(app.root_figure_save.picker_calls.is_empty());
            assert!(other.same_snapshot(&app.all_figures[&1]));
            assert!(selected.identity.same_object(&app.all_figures[&2].identity));
            assert!(selected.series[0].points.same_storage(&app.all_figures[&2].series[0].points));
            assert!(value.same_snapshot(&app.variables["keep"]) && value.same_snapshot(&app.variables["alias"]));
            assert_eq!((app.documents[0].id, app.documents[0].text.clone(), app.documents[0].path.clone()), script);
            assert!(app.documents[0].dirty);
            assert_eq!(std::fs::read_to_string(dir.path().join("script0.m")).unwrap(), "old=1;\n");
            frame_at(&mut app, &ctx, size, vec![]);
            assert!(app.root_figure_save.picker_calls.is_empty());
        }
    }

    #[test]
    fn visible_figure_toolbar_cancel_is_once_and_stop_retires_discarded_release() {
        for size in [egui::vec2(960., 650.), WIDE] {
            let (ctx, mut app, dir) = setup(size, false);
            let selected = app.all_figures[&1].clone();
            let other = app.all_figures[&2].clone();
            app.root_figure_save.picker = Some(|_, _| None);
            let pos = visible_toolbar_save(&mut app, &ctx, size);
            pointer_frame(&mut app, &ctx, size, pos, true);
            release_discarded(&mut app, &ctx, size, pos, |_, _| {});
            assert_eq!(app.root_figure_save.picker_calls.len(), 1);
            assert_eq!(app.root_figure_save.picker_calls[0].0, 1);
            assert!(!app.root_figure_save.picker_calls[0].1);
            assert!(!app.busy && app.figure_dirty(1) && app.documents[0].dirty);
            assert!(app.status.contains("cancelled"));
            let pos = visible_toolbar_save(&mut app, &ctx, size);
            assert_eq!(app.root_figure_save.picker_calls.len(), 1, "Cancel does not replay on repaint");
            app.root_figure_save.picker = Some(|_, _| panic!("Stop must retire toolbar Save before picker"));
            pointer_frame(&mut app, &ctx, size, pos, true);
            release_discarded(&mut app, &ctx, size, pos, |app, _| app.stop_work());
            assert_eq!(app.root_figure_save.picker_calls.len(), 1);
            assert!(!app.busy && app.figure_dirty(1) && app.documents[0].dirty);
            frame_at(&mut app, &ctx, size, vec![]);
            assert_eq!(app.root_figure_save.picker_calls.len(), 1);
            assert!(selected.same_snapshot(&app.all_figures[&1]));
            assert!(other.same_snapshot(&app.all_figures[&2]));
            assert!(app.figure_documents[&1].path.is_none());
            assert_eq!(std::fs::read_to_string(dir.path().join("script0.m")).unwrap(), "old=1;\n");
        }
    }
}
