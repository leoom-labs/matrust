fn document_close_key(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
    frame_at(app, ctx, size, vec![egui::Event::Key {
        key: egui::Key::W, physical_key: None, pressed: true, repeat: false,
        modifiers: egui::Modifiers::CTRL,
    }]);
}

#[test]
fn document_close_figure_key_keeps_hidden_script_and_workspace_cancel_then_discard() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 2);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 2);
    let ids: Vec<_> = app.documents.iter().map(|doc| doc.id).collect();
    let storage = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
    let first_points = app.all_figures[&1].series[0].points.clone();
    let old_figure = std::fs::read(dir.path().join("2.mfig")).unwrap();
    app.layout.mode = layout::PaneMode::Figure;
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(matches!(app.figure_close.as_ref().map(|flow| flow.action), Some(figure_files::Closing::Figure(2))));
    assert!(app.pending_close.is_none());
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.figure_close.is_none() && app.figure_dirty(2));
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    click_label(&mut app, &ctx, "Discard changes");
    finish(&mut app);
    assert_eq!(app.figure_ids, vec![1]);
    assert_eq!(app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(), ids);
    assert!(app.documents.iter().all(|doc| doc.dirty));
    assert_eq!(storage, app.variables["keep"].as_numeric().unwrap().data().as_ptr());
    assert!((first_points).same_storage(&app.all_figures[&1].series[0].points));
    assert_eq!(std::fs::read(dir.path().join("2.mfig")).unwrap(), old_figure);
    for script in scripts { assert_eq!(std::fs::read_to_string(script).unwrap(), "old=1;\n"); }
    assert!(!app.allow_close && !app.pending_quit);
}

#[test]
fn document_close_file_menu_saves_current_figure_before_closing_and_retains_explicit_script() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 2);
    named_dirty_scripts(&mut app, dir.path(), 2);
    app.layout.mode = layout::PaneMode::Figure;
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Close figure");
    click_label(&mut app, &ctx, "Save");
    finish_save_all_figures(&mut app, &ctx);
    assert_eq!(app.figure_ids, vec![1]);
    assert_eq!(matrust::files::load_figure(&dir.path().join("2.mfig")).unwrap().title, "Local 2");
    assert!(app.documents.iter().all(|doc| doc.dirty));
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Close script");
    assert_eq!(app.pending_close, Some(0));
    assert!(app.figure_close.is_none());
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(app.documents.len(), 2);
    // The separately named all-figure action retains its existing scope.
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Figures");
    click_label(&mut app, &ctx, "Close all figures");
    assert!(matches!(app.figure_close.as_ref().map(|flow| flow.action), Some(figure_files::Closing::All)));
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(app.figure_ids, vec![1]);
}

#[test]
fn document_close_preview_key_and_menu_close_only_preview_restore_prior_target() {
    for menu in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = figure_save_all_app(&ctx, dir.path(), 1);
        named_dirty_scripts(&mut app, dir.path(), 1);
        app.show_plot_pane();
        let source = dir.path().join("source.png");
        std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/preview/tiff-preview/reference-1.png"), &source).unwrap();
        let original = std::fs::read(&source).unwrap();
        app.submit(Job::OpenImage(source.clone()));
        finish(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.generic_save_target(), document_save::Pane::Preview);
        if menu {
            click_label(&mut app, &ctx, "Menu");
            click_label(&mut app, &ctx, "File");
            click_label(&mut app, &ctx, "Close preview");
        } else {
            document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
        }
        assert!(app.image_view.is_none() && app.pending_image.is_none());
        assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
        assert!(app.pending_close.is_none() && app.figure_close.is_none());
        assert!(app.documents[0].dirty && app.figure_dirty(1));
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
}

#[test]
fn document_close_compact_selected_tab_controls_target_and_script_save_before_close() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 2);
    let remaining_id = app.documents[1].id;
    let size = egui::vec2(640., 480.);
    app.layout.mode = layout::PaneMode::Split;
    click_label_at(&mut app, &ctx, size, "Plot figure");
    document_close_key(&mut app, &ctx, size);
    assert!(app.figure_close.is_some() && app.pending_close.is_none());
    click_label_at(&mut app, &ctx, size, "Cancel");
    click_label_at(&mut app, &ctx, size, "Script editor");
    document_close_key(&mut app, &ctx, size);
    assert_eq!(app.pending_close, Some(0));
    assert!(app.figure_close.is_none());
    click_label_at(&mut app, &ctx, size, "Save");
    finish(&mut app);
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "value=0;\n");
    assert!(!app.busy && !app.script_io.pending());
    // finish() receives the save reply; the normal UI frame resumes close.
    // A layout retry may already have consumed that reply on a fast worker.
    if let Some(index) = app.documents.iter().position(|doc| doc.path.as_ref() == Some(&scripts[0])) {
        assert!(!app.documents[index].dirty);
        assert_eq!(app.pending_close, Some(index));
        assert!(app.script_io.resume);
    }
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(app.pending_close.is_none() && !app.script_io.resume);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.documents[0].id, remaining_id);
    assert!(app.figure_dirty(1) && !app.allow_close);
}

#[test]
fn document_close_same_frame_plot_press_wins_over_stale_editor_focus_and_hover() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.layout.mode = layout::PaneMode::Split;
    let size = egui::vec2(1440., 900.);
    let plot = document_save_heading(&mut app, &ctx, size, "Figure 1");
    let editor_pos = document_save_heading(&mut app, &ctx, size, "Editor");
    ctx.memory_mut(|memory| memory.request_focus(editor::id(app.documents[0].id)));
    frame(&mut app, &ctx, vec![]);
    frame_at(&mut app, &ctx, size, vec![
        egui::Event::PointerMoved(plot),
        egui::Event::PointerButton { pos: plot, button: egui::PointerButton::Primary,
            pressed: true, modifiers: egui::Modifiers::CTRL },
        egui::Event::PointerMoved(editor_pos),
        egui::Event::Key { key: egui::Key::W, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::CTRL },
    ]);
    assert!(app.figure_close.is_some() && app.pending_close.is_none());
    pointer_frame(&mut app, &ctx, size, editor_pos, false);
    click_label(&mut app, &ctx, "Cancel");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(editor_pos)]);
    document_close_key(&mut app, &ctx, size);
    assert!(app.figure_close.is_some() && app.pending_close.is_none());
}

#[test]
fn document_close_empty_plot_busy_and_pending_dialog_do_not_close_hidden_scripts() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    named_dirty_scripts(&mut app, dir.path(), 2);
    let ids: Vec<_> = app.documents.iter().map(|doc| doc.id).collect();
    app.layout.mode = layout::PaneMode::Figure;
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(app.status.contains("No editable figure to close"));
    assert!(app.pending_close.is_none());
    app.layout.mode = layout::PaneMode::Editor;
    app.busy = true;
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    app.busy = false;
    assert!(app.pending_close.is_none());
    app.pending_close = Some(1);
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert_eq!(app.pending_close, Some(1));
    click_label(&mut app, &ctx, "Cancel");
    frame(&mut app, &ctx, vec![]);
    assert!(app.pending_close.is_none());
    assert_eq!(app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(), ids);
    assert!(app.documents.iter().all(|doc| doc.dirty));
}

#[test]
fn document_close_layout_retry_removes_only_one_clean_script() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    named_dirty_scripts(&mut app, dir.path(), 3);
    for doc in &mut app.documents { doc.discard(); }
    let remaining: Vec<_> = app.documents[1..].iter().map(|doc| doc.id).collect();
    app.layout.mode = layout::PaneMode::Editor;
    frame(&mut app, &ctx, vec![]);
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.))),
        events: vec![egui::Event::ModifiersChanged(egui::Modifiers::CTRL), egui::Event::Key {
            key: egui::Key::W, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::CTRL,
        }],
        ..Default::default()
    }, |root| {
        passes += 1;
        app.ui(root);
        if passes == 1 { root.ctx().request_discard("Close dispatch must not repeat"); }
    });
    assert!(passes >= 2);
    assert_eq!(app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(), remaining);
    assert!(!app.allow_close);
}

#[test]
fn document_close_figure_save_conflict_cancel_retains_local_and_external_documents() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflict.mfig");
    let mut app = figure_conflict_app(&ctx, &path);
    named_dirty_scripts(&mut app, dir.path(), 1);
    let external = std::fs::read(&path).unwrap();
    app.layout.mode = layout::PaneMode::Figure;
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    assert!(app.figure_conflict.is_some());
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert!(app.figure_dirty(1) && app.documents[0].dirty);
    assert_eq!(app.figure_ids, vec![1]);
    assert!(app.figure_close.is_none() && app.pending_close.is_none());
    assert!(!app.allow_close && !app.pending_quit);
}

#[test]
fn document_close_clean_figure_closes_without_prompt_and_empty_pane_stays_empty() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 1);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.submit(Job::SaveFigure(dir.path().join("1.mfig"), None));
    finish(&mut app);
    assert!(!app.figure_dirty(1));
    app.layout.mode = layout::PaneMode::Figure;
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    finish(&mut app);
    assert!(app.figure_ids.is_empty() && app.figure.is_none());
    assert!(app.figure_close.is_none() && app.pending_close.is_none());
    document_close_key(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(app.pending_close.is_none() && app.documents[0].dirty);
    assert!(!app.allow_close);
}

#[test]
fn document_close_and_save_in_one_frame_publish_only_the_first_accepted_intent() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let scripts = named_dirty_scripts(&mut app, dir.path(), 2);
    app.layout.mode = layout::PaneMode::Editor;
    frame(&mut app, &ctx, vec![]);
    // Keyboard processing accepts Save before Close. Both actual key events in
    // one frame must not save and then unexpectedly remove a document.
    frame(&mut app, &ctx, vec![egui::Key::S, egui::Key::W].into_iter().map(|key| {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false,
            modifiers: egui::Modifiers::CTRL }
    }).collect());
    finish(&mut app);
    assert_eq!(std::fs::read_to_string(&scripts[0]).unwrap(), "value=0;\n");
    assert_eq!(app.documents.len(), 2);
    assert!(app.pending_close.is_none() && !app.documents[0].dirty);
    // Exercise the reverse acceptance order through the same bounded queue.
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.))),
        ..Default::default()
    }, |root| {
        let frame = root.ctx().cumulative_frame_nr();
        app.document_save.queue_close(frame, app.generic_save_target());
        app.document_save.queue(frame, false, app.generic_save_target());
        app.ui(root);
    });
    assert_eq!(app.documents.len(), 1);
    assert_eq!(std::fs::read_to_string(&scripts[1]).unwrap(), "old=1;\n");
    assert!(app.documents[0].dirty && !app.allow_close);
}
