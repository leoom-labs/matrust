#[test]
fn background_model_save_defers_io_and_keeps_shared_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mrsim");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.run("data=[1 2 3];plot(data);".into());
    finish(&mut app);
    let data = app.variables["data"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    app.toolkits.model.steps = 3456;
    assert!(!app.toolkits.save_model_path(&path, None));
    assert!(!path.exists());
    assert!(app.toolkits.model_io_pending() && app.toolkits.model_dirty());
    app.advance_model_read();
    assert!(app.busy && !app.worker.opening);
    finish(&mut app);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 3456);
    assert!(!app.toolkits.model_dirty() && !app.toolkits.model_io_pending());
    assert!(data.same_snapshot(&app.variables["data"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
}

#[test]
fn background_model_stop_after_commit_records_save_without_opening_or_quitting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mrsim");
    let target = dir.path().join("next.mrsim");
    matrust::simulation::Model {
        steps: 777,
        ..Default::default()
    }
    .save(&target)
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::new()];
    app.toolkits.model.steps = 321;
    app.toolkits.request_open(target);
    app.advance_model_read();
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    app.toolkits.save_model_path(&path, None);
    app.advance_model_read();
    let reply = app
        .worker
        .receive
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    app.pending_quit = true;
    app.quit_discarded_figures.insert(3);
    app.stop_work();
    app.busy = false;
    app.receive_model_write(reply.model_write.unwrap());
    assert_eq!(app.toolkits.model.steps, 321);
    assert!(!app.toolkits.model_dirty());
    assert!(!app.pending_quit && !app.allow_close && app.quit_discarded_figures.is_empty());
    assert!(app.toolkits.model_file_modal_open() && !app.toolkits.model_io_pending());
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.toolkits.model.steps, 321);
    assert!(!app.allow_close);
    assert!(
        app.toolkits
            .message
            .contains("subsequent actions cancelled")
    );
}

#[test]
fn background_model_stop_before_dispatch_leaves_disk_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("never-written.mrsim");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.toolkits.model.steps = 567;
    app.toolkits.save_model_path(&path, None);
    app.stop_work();
    app.advance_model_read();
    assert!(!app.busy && !app.toolkits.model_io_pending());
    assert!(app.toolkits.model_dirty() && !path.exists());
}

#[test]
fn background_model_save_failure_and_snapshot_baseline_preserve_newer_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.mrsim");
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.toolkits.model.steps = 222;
    app.toolkits
        .save_model_path(&dir.path().join("missing/model.mrsim"), None);
    app.advance_model_read();
    finish(&mut app);
    assert!(app.toolkits.model_dirty() && app.toolkits.model_metadata().0.is_none());
    assert!(app.toolkits.message.contains("model.mrsim"));
    app.toolkits.save_model_path(&path, None);
    // The UI is disabled while saving. Simulate a future programmatic edit to
    // prove that the saved baseline belongs to the submitted snapshot only.
    app.toolkits.model.steps = 999;
    app.advance_model_read();
    finish(&mut app);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 222);
    assert_eq!(app.toolkits.model.steps, 999);
    assert!(app.toolkits.model_dirty());
    assert_eq!(app.toolkits.model_metadata().1.unwrap().steps, 222);
}

#[test]
fn background_model_disk_check_cancellation_and_retry_preserve_local_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.mrsim");
    let ctx = egui::Context::default();
    let mut app = model_app(&ctx, &path);
    app.toolkits.preview_model_conflict();
    app.advance_model_read();
    assert!(app.worker.opening);
    let reply = app
        .worker
        .receive
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    app.stop_work();
    app.busy = false;
    app.receive_model_write(reply.model_write.unwrap());
    assert!(!app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 321);
    app.toolkits.preview_model_conflict();
    app.advance_model_read();
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 321);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 654);
}

#[test]
fn background_model_save_resumes_quit_only_after_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.mrsim");
    let ctx = egui::Context::default();
    let mut app = model_app(&ctx, &path);
    // Restore the original disk baseline, so Save needs no conflict decision.
    matrust::simulation::Model::default().save(&path).unwrap();
    app.request_quit(&ctx);
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert!(app.allow_close && !app.toolkits.model_dirty());
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
}

#[test]
fn background_model_stop_button_is_accessible_in_compact_window() {
    fn draw(
        tools: &mut toolkits::Toolkits,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = crate::headless::run(ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                events,
                ..Default::default()
            },
            |ctx| action = tools.ui(ctx, false),
        );
        (output, action)
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-dispatched.mrsim");
    let ctx = egui::Context::default();
    let mut tools = toolkits::Toolkits::default();
    tools.model_open = true;
    tools.save_model_path(&path, None);
    let mut position = None;
    for _ in 0..4 {
        let (output, _) = draw(&mut tools, &ctx, vec![]);
        position = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Stop model operation"));
    }
    let position = position.expect("Visible in-window Stop button");
    assert!((0.0..960.).contains(&position.x) && (0.0..650.).contains(&position.y));
    let mut stopped = false;
    for pressed in [true, false] {
        let (_, action) = draw(
            &mut tools,
            &ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
        stopped |= matches!(action, Some(toolkits::Action::StopModel));
    }
    assert!(stopped && !path.exists());
}
