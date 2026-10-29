#[test]
fn background_model_open_defers_io_and_keeps_local_state_on_invalid_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.mrsim");
    std::fs::write(&path, b"invalid").unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.toolkits.model.steps = 432;
    assert!(app.toolkits.request_open(path.clone()));
    assert!(app.toolkits.model_read_pending());
    assert!(!app.toolkits.request_open(path));
    assert_eq!(app.toolkits.model.steps, 432);
    app.advance_model_read();
    assert!(app.busy && app.worker.opening);
    finish(&mut app);
    assert!(!app.toolkits.model_read_pending() && !app.worker.opening);
    assert_eq!(app.toolkits.model.steps, 432);
    assert!(app.toolkits.model_dirty());
    assert!(app.toolkits.message.contains("bad.mrsim"));
}

#[test]
fn background_model_stop_discards_a_completed_read_and_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("next.mrsim");
    let model = matrust::simulation::Model {
        steps: 456,
        ..Default::default()
    };
    model.save(&path).unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.toolkits.request_open(path.clone());
    app.advance_model_read();
    let reply = app
        .worker
        .receive
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert!(reply.model_read.as_ref().unwrap().result.is_ok());
    app.pending_quit = true;
    app.quit_discarded_figures.insert(9);
    app.stop_work();
    assert!(!app.pending_quit && app.quit_discarded_figures.is_empty());
    app.busy = false;
    app.receive_model_read(reply.model_read.unwrap());
    assert_eq!(app.toolkits.model.steps, 1000);
    assert!(!app.toolkits.model_dirty());
    assert!(app.status.contains("cancelled"));
    app.toolkits.request_open(path);
    app.advance_model_read();
    finish(&mut app);
    assert_eq!(app.toolkits.model.steps, 456);
    assert!(!app.toolkits.model_dirty());
}

#[test]
fn background_model_waits_for_existing_worker_and_stop_clears_waiting_request() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx, true);
    app.run("unchanged = [1 2 3]; plot(unchanged);".into());
    assert!(app.toolkits.request_open(PathBuf::from("missing.mrsim")));
    app.advance_model_read();
    assert!(app.toolkits.model_read_pending());
    app.stop_work();
    finish(&mut app);
    app.receive();
    assert!(!app.busy && !app.toolkits.model_read_pending());
    assert_eq!(app.toolkits.model.steps, 1000);
}

#[test]
fn background_model_open_preserves_shared_workspace_and_figure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("next.mrsim");
    matrust::simulation::Model::default().save(&path).unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("data = [1 2 3]; plot(data);".into());
    finish(&mut app);
    let values = app.variables["data"].clone();
    let points = app.figure.as_ref().unwrap().series[0].points.clone();
    app.open_path(path);
    finish(&mut app);
    assert!(values.same_snapshot(&app.variables["data"]));
    assert!((points).same_storage(&app.figure.as_ref().unwrap().series[0].points));
    assert!(app.toolkits.model_open && !app.toolkits.model_dirty());
    assert!(!app.toolkits.model_read_pending());
}
