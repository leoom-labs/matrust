fn script_action_key(
    app: &mut Workspace,
    ctx: &egui::Context,
    modifiers: egui::Modifiers,
    key: egui::Key,
) {
    frame(
        app,
        ctx,
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
    );
}
fn named_dirty_scripts(app: &mut Workspace, root: &std::path::Path, count: usize) -> Vec<PathBuf> {
    let paths: Vec<_> = (0..count)
        .map(|i| root.join(format!("script{i}.m")))
        .collect();
    app.documents = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            std::fs::write(path, "old=1;\n").unwrap();
            let mut document = Document::open(path).unwrap();
            document.text = format!("value={i};\n");
            document.changed();
            document
        })
        .collect();
    app.active = 0;
    paths
}
fn finish_script_saves(app: &mut Workspace, ctx: &egui::Context) {
    for _ in 0..40 {
        if app.script_save_queue.is_none() {
            break;
        }
        frame(app, ctx, vec![]);
        finish(app);
    }
    assert!(
        app.script_save_queue.is_none() && app.script_conflict.is_none(),
        "{}",
        app.status
    );
    assert!(!app.pending_quit && !app.allow_close);
}
#[test]
fn script_actions_save_all_shortcut_saves_dirty_only_without_closing_or_running() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let paths = named_dirty_scripts(&mut app, dir.path(), 4);
    app.documents[3].discard();
    std::fs::write(&paths[3], "external_clean=8;\n").unwrap();
    app.active = 2;
    script_action_key(
        &mut app,
        &ctx,
        egui::Modifiers::CTRL | egui::Modifiers::ALT,
        egui::Key::S,
    );
    assert_eq!(app.script_save_queue.as_ref().unwrap().len(), 3);
    finish_script_saves(&mut app, &ctx);
    assert_eq!(app.active, 2);
    assert_eq!(app.documents.len(), 4);
    assert!(app.documents.iter().all(|d| !d.dirty));
    for (i, path) in paths.iter().enumerate().take(3) {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            format!("value={i};\n")
        );
    }
    assert_eq!(
        std::fs::read_to_string(&paths[3]).unwrap(),
        "external_clean=8;\n"
    );
    assert!(app.variables.is_empty());
}
#[test]
fn script_actions_save_all_rechecks_conflicts_and_resumes_after_overwrite() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let paths = named_dirty_scripts(&mut app, dir.path(), 2);
    std::fs::write(&paths[0], "external=1;").unwrap();
    app.begin_save_all_scripts();
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    assert!(app.script_conflict.is_some());
    std::fs::write(&paths[0], "external=2;").unwrap();
    click_label(&mut app, &ctx, "Overwrite file");
    finish(&mut app);
    assert!(app.script_conflict.is_some());
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "old=1;\n");
    click_label(&mut app, &ctx, "Overwrite file");
    finish(&mut app);
    finish_script_saves(&mut app, &ctx);
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "value=0;\n");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "value=1;\n");
}
#[test]
fn script_actions_save_all_cancel_preserves_later_buffers_and_reload_resumes() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let paths = named_dirty_scripts(&mut app, dir.path(), 2);
    std::fs::write(&paths[0], "external=9;").unwrap();
    app.begin_save_all_scripts();
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.script_save_queue.is_none());
    assert!(app.documents.iter().all(|d| d.dirty));
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "old=1;\n");
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Save all scripts");
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    click_label(&mut app, &ctx, "Reload from disk");
    finish_script_saves(&mut app, &ctx);
    assert_eq!(app.documents[0].text, "external=9;");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "value=1;\n");
}
#[test]
fn script_actions_save_all_failure_keeps_earlier_saves_and_stops_later_writes() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let paths = named_dirty_scripts(&mut app, dir.path(), 3);
    std::fs::remove_file(&paths[1]).unwrap();
    std::fs::create_dir(&paths[1]).unwrap();
    app.begin_save_all_scripts();
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    assert!(app.script_save_queue.is_none());
    assert!(!app.documents[0].dirty && app.documents[1].dirty && app.documents[2].dirty);
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "value=0;\n");
    assert_eq!(std::fs::read_to_string(&paths[2]).unwrap(), "old=1;\n");
    assert!(paths[1].is_dir());
}
#[test]
fn script_actions_save_all_progress_can_cancel_without_losing_unsaved_tabs() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let paths = named_dirty_scripts(&mut app, dir.path(), 32);
    app.begin_save_all_scripts();
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::W);
    finish(&mut app);
    assert_eq!(app.documents.len(), 32);
    assert!(app.pending_close.is_none());
    click_label(&mut app, &ctx, "Cancel remaining saves");
    finish(&mut app);
    assert!(app.script_save_queue.is_none());
    let dirty = app.documents.iter().filter(|d| d.dirty).count();
    assert!(dirty > 0 && dirty < 32);
    for (i, doc) in app.documents.iter().enumerate() {
        assert_eq!(
            std::fs::read_to_string(&paths[i]).unwrap(),
            if doc.dirty { "old=1;\n" } else { &doc.text }
        );
    }
    app.begin_save_all_scripts();
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape);
    finish(&mut app);
    assert!(app.script_save_queue.is_none());
    assert!(app.documents.iter().any(|d| d.dirty));
}
#[test]
fn script_actions_tab_shortcuts_wrap_reveal_tabs_and_close_with_unsaved_protection() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    named_dirty_scripts(&mut app, dir.path(), 2);
    app.layout.mode = layout::PaneMode::Figure;
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::Tab);
    assert_eq!(app.active, 1);
    assert_eq!(app.layout.mode, layout::PaneMode::Split);
    assert!(ctx.memory(|memory| memory.has_focus(editor::id(app.documents[1].id))));
    script_action_key(
        &mut app,
        &ctx,
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        egui::Key::Tab,
    );
    assert_eq!(app.active, 0);
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::W);
    assert_eq!(app.pending_close, Some(0));
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::Tab);
    assert_eq!(app.active, 0);
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(app.documents.len(), 2);
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::W);
    click_label(&mut app, &ctx, "Discard changes");
    assert_eq!(app.documents.len(), 1);
    app.documents[0].discard();
    script_action_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::W);
    assert_eq!(app.documents.len(), 1);
    assert!(app.documents[0].text.is_empty() && !app.allow_close);
    named_dirty_scripts(&mut app, dir.path(), 30);
    script_action_key(
        &mut app,
        &ctx,
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        egui::Key::Tab,
    );
    assert_eq!(app.active, 29);
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    let title = app.documents[29].title();
    assert!(output.shapes.iter().any(|shape| {
        text_position(&shape.shape, &title).is_some_and(|p| shape.clip_rect.contains(p))
    }));
}

#[test]
fn script_actions_layout_retries_do_not_replay_drops_or_dispatch_another_file() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..2)
        .map(|i| {
            let path = dir.path().join(format!("loaded{i}.m"));
            std::fs::write(&path, "% Open only").unwrap();
            path
        })
        .collect();
    let mut passes = 0;
    let _ = crate::headless::run(&ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 900.),
            )),
            dropped_files: paths
                .into_iter()
                .map(|path| crate::ui_tests::dropped_file(path))
                .collect(),
            ..Default::default()
        },
        |root| {
                let ctx = &root.ctx().clone();
            passes += 1;
            app.ui(root);
            if passes == 1 {
                ctx.request_discard("Exercise repeated UI layout pass");
            }
        },
    );
    assert!(passes >= 2);
    assert_eq!(app.documents.len(), 1);
    assert!(app.worker.opening);
    assert_eq!(app.open_queue.len(), 1);
    finish(&mut app);
    assert_eq!(app.documents.len(), 2);
    assert_eq!(app.open_queue.len(), 1);
    finish_open_queue(&mut app, &ctx);
    assert_eq!(app.documents.len(), 3);
    assert!(app.open_queue.is_empty());
}
