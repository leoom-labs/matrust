fn run_edit_key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

fn run_edit_focus(app: &mut Workspace, ctx: &egui::Context, range: std::ops::Range<usize>) {
    for _ in 0..3 { frame(app, ctx, vec![]); }
    let id = editor::id(app.documents[app.active].id);
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(range.start), egui::text::CCursor::new(range.end))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

#[test]
fn run_shortcut_uses_text_paste_and_ime_commit_processed_in_the_same_frame() {
    for event in [egui::Event::Text("fresh=42;".into()), egui::Event::Paste("fresh=42;".into()),
        egui::Event::Ime(egui::ImeEvent::Commit("fresh=42;".into()))]
    {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.documents[app.active].text = "stale=1;".into();
        run_edit_focus(&mut app, &ctx, 0..8);
        frame(&mut app, &ctx, vec![event, run_edit_key(egui::Key::Enter, egui::Modifiers::CTRL)]);
        assert_eq!(app.documents[app.active].text, "fresh=42;");
        assert_eq!(app.running_origin.as_ref().unwrap().source.as_ref(), "fresh=42;");
        finish(&mut app);
        assert_eq!(app.variables["fresh"].as_numeric().unwrap().data(), &[42.]);
        assert!(!app.variables.contains_key("stale"));
    }
}

#[test]
fn run_selection_uses_same_frame_keyboard_selection_instead_of_prior_range() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents[app.active].text = "skipped=1;\nchosen=42;".into();
    let end = app.documents[app.active].text.chars().count();
    run_edit_focus(&mut app, &ctx, end..end);
    // The previous frame's selection is deliberately a different statement.
    app.selection = Some((app.active, 0..10));
    frame(&mut app, &ctx, vec![
        run_edit_key(egui::Key::Home, egui::Modifiers::SHIFT),
        run_edit_key(egui::Key::F9, egui::Modifiers::NONE),
    ]);
    finish(&mut app);
    assert_eq!(app.variables["chosen"].as_numeric().unwrap().data(), &[42.]);
    assert!(!app.variables.contains_key("skipped"));
}

#[test]
fn queued_run_cannot_follow_a_same_frame_new_tab_or_dirty_close_confirmation() {
    for key in [egui::Key::N, egui::Key::W] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.documents[app.active].text = "must_not_run=1;".into();
        app.documents[app.active].changed();
        run_edit_focus(&mut app, &ctx, 0..0);
        let document = app.documents[app.active].id;
        frame(&mut app, &ctx, vec![
            run_edit_key(egui::Key::Enter, egui::Modifiers::CTRL),
            run_edit_key(key, egui::Modifiers::CTRL),
        ]);
        assert!(!app.busy);
        assert!(!app.variables.contains_key("must_not_run"));
        if key == egui::Key::N {
            assert_ne!(app.documents[app.active].id, document);
        } else {
            assert!(app.pending_close.is_some());
            click_label(&mut app, &ctx, "Cancel");
            assert!(app.pending_close.is_none());
        }
        for _ in 0..3 { frame(&mut app, &ctx, vec![]); }
        assert!(!app.busy && !app.variables.contains_key("must_not_run"));
    }
}

#[test]
fn run_intent_waits_for_final_layout_and_dispatches_once_across_retries() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("count=0;".into()); finish(&mut app);
    app.documents[app.active].text = "count=count+1;".into();
    run_edit_focus(&mut app, &ctx, 0..0);
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.,900.))),
        events: vec![run_edit_key(egui::Key::Enter, egui::Modifiers::CTRL)],
        ..Default::default()
    }, |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("Run waits for final editor pass"); }
        app.ui(root);
        if passes == 1 { assert!(!app.busy); }
    });
    assert!(passes >= 2);
    finish(&mut app);
    assert_eq!(app.variables["count"].as_numeric().unwrap().data(), &[1.]);
}

#[test]
fn named_document_runs_dirty_buffer_with_real_path_and_preserves_selection_origin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dirty_script.m");
    std::fs::write(&path, "from_disk=1;").unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let elsewhere = tempfile::tempdir().unwrap();
    app.folder = elsewhere.path().to_owned();
    app.documents[app.active] = matrust::document::Document::open(&path).unwrap();
    // Document::open retains the filesystem's canonical identity. On Windows
    // this includes the verbatim prefix; execution must preserve that identity.
    let canonical_path = std::fs::canonicalize(&path).unwrap();
    assert_eq!(app.documents[app.active].path.as_ref(), Some(&canonical_path));
    app.documents[app.active].text = "from_buffer=42;identity=mfilename('fullpath');plot([1 2],[3 4]);".into();
    app.documents[app.active].changed();
    let source = app.documents[app.active].text.clone();
    let document = app.documents[app.active].id;
    app.run_document(None);
    assert_eq!(app.running_origin.as_ref().unwrap().document, document);
    assert_eq!(app.running_origin.as_ref().unwrap().source.as_ref(), source);
    finish(&mut app);
    assert_eq!(app.variables["from_buffer"].as_numeric().unwrap().data(), &[42.]);
    assert!(!app.variables.contains_key("from_disk"));
    let matrust::Array::Character(identity) = &app.variables["identity"] else { panic!("character identity") };
    assert_eq!(String::from_utf16(identity.units()).unwrap(), canonical_path.with_extension("").to_str().unwrap());
    assert_eq!(app.documents[app.active].path.as_ref(), Some(&canonical_path));
    assert_eq!(app.documents[app.active].id, document);
    assert_eq!(app.documents[app.active].text, source);
    assert!(app.documents[app.active].dirty);
    assert_eq!(std::fs::read(&path).unwrap(), b"from_disk=1;");
    app.documents[app.active].text = "not_selected=1;\nselection_name=mfilename();".into();
    let start = "not_selected=1;\n".chars().count();
    let end = app.documents[app.active].text.chars().count();
    app.run_document(Some(start..end));
    finish(&mut app);
    assert!(!app.variables.contains_key("not_selected"));
    let matrust::Array::Character(name) = &app.variables["selection_name"] else { panic!("character name") };
    assert!(name.units().is_empty()); // existing selection has no named-document identity
    assert_eq!(app.folder, elsewhere.path());
    app.documents[app.active].text = "kept_before_error=3;\nundefined_bundle_function();".into();
    let error_source = app.documents[app.active].text.clone();
    app.run_document(None);
    finish(&mut app);
    let issue = app.editor_issue.as_ref().unwrap();
    assert_eq!(issue.diagnostic.line, 2);
    assert_eq!(issue.origin.document, document);
    assert_eq!(issue.origin.source.as_ref(), error_source);
    assert_eq!(std::fs::read(&path).unwrap(), b"from_disk=1;");
}

#[test]
fn worker_single_export_then_document_open_run_replays_large_original_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker_export.m");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("x=1:30000;plot(x,x/7);title('Large worker plot');".into());
    finish(&mut app);
    let original = app.figure.as_ref().unwrap().clone();
    app.export_settings.format = figure_export::Format::MatlabScript;
    app.begin_figure_export();
    for _ in 0..3 { frame(&mut app, &ctx, vec![]); }
    let output = frame(&mut app, &ctx, vec![]);
    assert!(output.shapes.iter().any(|shape| text_position(&shape.shape, "Large plots include a companion data folder. Keep it beside the script when moving or sharing both.").is_some()));
    app.export_to_path(path.clone());
    finish(&mut app);
    assert!(path.is_file(), "{}", app.status);
    assert!(app.figure.as_ref().unwrap().same_snapshot(&original));
    assert!((app.figure.as_ref().unwrap().series[0].points).same_storage(&original.series[0].points));
    let elsewhere = tempfile::tempdir().unwrap();
    app.folder = elsewhere.path().to_owned();
    app.documents[app.active] = matrust::document::Document::open(&path).unwrap();
    app.run_document(None);
    finish(&mut app);
    let actual = app.figure.as_ref().unwrap();
    assert_eq!(actual.title, original.title);
    assert_eq!(actual.series[0].points.iter().collect::<Vec<_>>().as_slice(), original.series[0].points.iter().collect::<Vec<_>>().as_slice());
    assert_eq!(actual.series[0].points.len(), 30_000);
}

#[test]
fn startup_placeholder_run_controls_and_shortcuts_do_not_reveal_or_submit() {
    for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for size in [egui::vec2(960.,650.), egui::vec2(1440.,900.)] {
            let ctx=egui::Context::default();
            let mut app=Workspace::with_context(ctx.clone(),true);
            app.layout.theme=Some(theme);app.apply_desktop_theme(&ctx);
            let document=app.documents[0].id;
            let original=serde_json::to_value(&app.documents).unwrap();
            for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
            click_label_at(&mut app,&ctx,size,"Run");
            // The toolstrip hides Run script while there is nothing to run.
            assert!(frame_at(&mut app,&ctx,size,vec![]).shapes.iter().all(|s|text_position(&s.shape,"Run section").is_none()));
            for (key,modifiers) in [(egui::Key::Enter,egui::Modifiers::CTRL),(egui::Key::F9,egui::Modifiers::NONE)] {
                command_first_key(&mut app,&ctx,size,key,modifiers);
            }
            for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
            assert!(app.has_startup_placeholder() && app.central_command());
            assert!(!app.busy && app.running_origin.is_none());
            assert!(app.variables.is_empty() && app.all_figures.is_empty());
            assert_eq!(app.documents[0].id,document);
            assert_eq!(serde_json::to_value(&app.documents).unwrap(),original);
            assert!(ctx.read_response(editor::id(document)).is_none());
            assert_eq!(ctx.global_style().visuals.dark_mode,theme==session::DesktopTheme::Dark);
        }
    }
}

#[test]
fn explicit_new_empty_and_hidden_real_document_keep_run_behavior() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let size=egui::vec2(1440.,900.);
    command_first_key(&mut app,&ctx,size,egui::Key::N,egui::Modifiers::CTRL);
    assert!(!app.has_startup_placeholder());
    let document=app.documents[app.active].id;
    click_label_at(&mut app,&ctx,size,"Run");
    assert!(!app.busy && app.running_origin.is_none()); // Existing empty-script no-op.
    assert!(!app.central_command());
    assert_eq!(app.documents[app.active].id,document);
    run_edit_focus(&mut app,&ctx,0..0);
    frame(&mut app,&ctx,vec![egui::Event::Text("real_hidden=43;".into())]);
    assert_eq!(app.documents[app.active].text,"real_hidden=43;");
    app.show_command_center();
    assert!(app.central_command() && !app.has_startup_placeholder());
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::CTRL);
    finish(&mut app);
    assert_eq!(app.variables["real_hidden"].as_numeric().unwrap().data(),&[43.]);
    assert_eq!(app.documents[app.active].id,document);
    assert!(!app.central_command());
}

#[test]
fn final_run_admission_rechecks_placeholder_without_retaining_source_text() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    app.new_document("");let document=app.documents[app.active].id;
    let _=crate::headless::run(&ctx,egui::RawInput::default(),|root| {
        app.request_script_run(root.ctx(),false);
        // Exercise the final admission independently with the same document ID;
        // a same-frame return to the explicit placeholder policy must win.
        app.layout.center=Some(session::CenterLayout::default());
        app.finish_script_run(root.ctx());
    });
    assert!(app.has_startup_placeholder() && app.central_command());
    assert!(!app.busy && app.running_origin.is_none());
    assert_eq!(app.documents[app.active].id,document);
}
