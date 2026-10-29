fn drop_paths(app: &mut Workspace, ctx: &egui::Context, paths: Vec<PathBuf>) {
    let _ = crate::headless::run(ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            dropped_files: paths
                .into_iter()
                .map(|path| crate::ui_tests::dropped_file(path))
                .collect(),
            ..Default::default()
        },
        |root| app.ui(root),
    );
}
fn finish_open_queue(app: &mut Workspace, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while (app.busy || !app.open_queue.is_empty()) && Instant::now() < deadline {
        frame(app, ctx, vec![]);
        std::thread::sleep(Duration::from_millis(1));
    }
    frame(app, ctx, vec![]);
    assert!(
        !app.busy && app.open_queue.is_empty(),
        "Queue stalled: {}",
        app.status
    );
}
#[test]
fn opening_queue_dropped_mixed_files_keep_order_errors_and_unsaved_work() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.layout.mode = layout::PaneMode::Editor;
    app.documents[0].text = "unsaved=99;".into();
    app.documents[0].changed();
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("measurements.csv");
    std::fs::write(&csv, "1,2\n3,4\n").unwrap();
    let script = dir.path().join("analysis.m");
    std::fs::write(&script, "should_not_run=42;").unwrap();
    let broken = dir.path().join("broken.svg");
    std::fs::write(&broken, "not SVG").unwrap();
    let mut engine = matrust::Engine::default();
    engine.execute("plot(1:5);title('First');").unwrap();
    let first = dir.path().join("first.mfig");
    matrust::files::save_figure(&first, engine.figure.as_ref().unwrap()).unwrap();
    engine.execute("title('Last');").unwrap();
    let last = dir.path().join("last.mfig");
    matrust::files::save_figure(&last, engine.figure.as_ref().unwrap()).unwrap();
    let preview = dir.path().join("preview.svg");
    std::fs::write(&preview, engine.figure.as_ref().unwrap().svg()).unwrap();
    drop_paths(
        &mut app,
        &ctx,
        vec![
            csv,
            dir.path().join("missing.m"),
            broken,
            script.clone(),
            first,
            last,
            preview,
            script,
        ],
    );
    assert_eq!(app.open_queue.len(), 7);
    finish(&mut app);assert!(app.csv_import.is_some());
    assert!(!app.variables.contains_key("measurements"));
    click_label(&mut app,&ctx,"Import");
    finish_open_queue(&mut app, &ctx);
    assert_eq!(app.documents.len(), 2);
    assert_eq!(app.documents[0].text, "unsaved=99;");
    assert!(app.documents[0].dirty);
    assert_eq!(app.documents[1].text, "should_not_run=42;");
    assert!(!app.variables.contains_key("should_not_run"));
    assert_eq!(app.variables["measurements"].scalar(1, 1).unwrap().into_real().unwrap(), 4.);
    assert_eq!(app.figure_ids.len(), 2);
    assert_ne!(app.layout.mode, layout::PaneMode::Editor);
    assert_eq!(app.figure.as_ref().unwrap().title, "Last");
    assert_eq!(app.image_view.as_ref().unwrap().original_size, [960, 560]);
    assert!(app.console.iter().any(|s| s.contains("missing.m")));
    assert!(app.console.iter().any(|s| s.starts_with("Error:")));
}
#[test]
fn opening_queue_waits_for_mat_choice_and_accepts_more_drops_while_waiting() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let mat = dir.path().join("selected.mat");
    matrust::files::save_workspace(
        &mat,
        &BTreeMap::from([("answer".into(), matrust::Matrix::scalar(42.))]),
    )
    .unwrap();
    let script = dir.path().join("after.m");
    std::fs::write(&script, "% after MAT choice").unwrap();
    app.queue_open_paths([mat.clone(), script.clone()]);
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    assert!(app.workspace_import.is_some());
    drop_paths(&mut app, &ctx, vec![script]);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.open_queue.len(), 2);
    assert_eq!(app.documents.len(), 1);
    click_label(&mut app, &ctx, "Cancel");
    finish_open_queue(&mut app, &ctx);
    assert!(!app.variables.contains_key("answer"));
    assert_eq!(app.documents.len(), 2);
    app.queue_open_paths([mat, dir.path().join("after.m")]);
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    click_label(&mut app, &ctx, "Select supported");
    click_label(&mut app, &ctx, "Load selected");
    finish_open_queue(&mut app, &ctx);
    assert_eq!(app.variables["answer"].scalar(0, 0).unwrap().into_real().unwrap(), 42.);
}
#[test]
fn opening_queue_waits_for_dirty_model_choice_without_overwriting_it() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.mrsim");
    let mut app = model_app(&ctx, &original);
    let next = dir.path().join("next.mrsim");
    matrust::simulation::Model {
        steps: 888,
        ..Default::default()
    }
    .save(&next)
    .unwrap();
    let script = dir.path().join("last.m");
    std::fs::write(&script, "% after model").unwrap();
    app.queue_open_paths([next.clone(), script.clone()]);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 321);
    assert_eq!(app.open_queue.len(), 1);
    click_label(&mut app, &ctx, "Cancel");
    finish_open_queue(&mut app, &ctx);
    assert_eq!(app.toolkits.model.steps, 321);
    app.queue_open_paths([next, script]);
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    click_label(&mut app, &ctx, "Discard changes");
    finish_open_queue(&mut app, &ctx);
    assert_eq!(app.toolkits.model.steps, 888);
}
#[test]
fn opening_queue_bounds_selection_and_stop_does_not_restart_cancelled_work() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.queue_open_paths([PathBuf::from("examples/signals.m")]);
    let retained = app.open_queue.clone();
    assert!(retained[0].is_absolute());
    app.queue_open_paths((0..32).map(|i| PathBuf::from(format!("{i}.m"))));
    assert_eq!(app.open_queue, retained);
    assert!(app.status.contains("not added"));
    app.run("x=ones(1000,1000);".into());
    app.open_path(PathBuf::from("examples/filter_design.m"));
    assert_eq!(app.open_queue.len(), 2);
    app.stop_work();
    assert!(app.open_queue.is_empty());
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.documents.len(), 1);
    assert!(!app.busy);
}

#[test]
fn opening_queue_rejects_an_oversized_drop_as_one_selection() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.m");
    std::fs::write(&first, "% Must not open part of a rejected selection").unwrap();
    let paths = std::iter::once(first)
        .chain((0..1000).map(|i| dir.path().join(format!("missing-{i}.m"))))
        .collect();
    let messages = app.console.len();
    drop_paths(&mut app, &ctx, paths);
    assert!(app.open_queue.is_empty() && !app.busy);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.console.len(), messages + 1);
    assert!(app.status.contains("not added"));
}

#[test]
fn opening_queue_waits_for_filter_coefficient_editor_before_replacing_model() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.toolkits.open_tool(toolkits::Tool::Model);
    click_label(&mut app, &ctx, "Blocks");
    click_label(&mut app, &ctx, "4. Lowpass");
    click_label(&mut app, &ctx, "Parameters");
    click_label(&mut app, &ctx, "Edit coefficients…");
    assert!(app.file_action_pending());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("next.mrsim");
    matrust::simulation::Model {
        steps: 888,
        ..Default::default()
    }
    .save(&path)
    .unwrap();
    drop_paths(&mut app, &ctx, vec![path]);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.open_queue.len(), 1);
    assert_eq!(app.toolkits.model.steps, 1000);
    click_label(&mut app, &ctx, "Cancel");
    finish_open_queue(&mut app, &ctx);
    assert_eq!(app.toolkits.model.steps, 888);
}

#[test]
fn opening_queue_can_be_cleared_while_mat_decision_remains_open() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.open_path(PathBuf::from("tests/fixtures/mat/mat-import-v7.mat"));
    finish(&mut app);
    drop_paths(&mut app, &ctx, vec![PathBuf::from("examples/signals.m")]);
    assert_eq!(app.open_queue.len(), 1);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::X,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        }],
    );
    assert!(app.open_queue.is_empty() && app.workspace_import.is_some());
    app.queue_open_paths([PathBuf::from("examples/filter_design.m")]);
    click_label(&mut app, &ctx, "Cancel queued files");
    assert!(app.open_queue.is_empty() && app.workspace_import.is_some());
    click_label(&mut app, &ctx, "Cancel");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.documents.len(), 1);
    assert!(app.variables.is_empty() && !app.busy);
}
