use super::*;
#[test]
fn document_invariants_include_identity_and_private_saved_baseline() {
    use matrust::document::Document;
    let path = std::env::temp_dir().join("matrust-eps-document-invariant.m");
    let mut first = Document::from_loaded_script(path.clone(), "original A".into()).unwrap();
    first.text = "current edited text".into();
    first.changed();
    let mut other = Document::from_loaded_script(path, "original B".into()).unwrap();
    other.text.clone_from(&first.text);
    other.changed();
    assert!(same_script_documents(&[first.clone()], &[first.clone()]).unwrap());
    assert!(!same_script_documents(&[first.clone()], &[other.clone()]).unwrap());
    other.id = first.id;
    assert_eq!(first.text, other.text);
    assert_eq!(first.path, other.path);
    assert_eq!(first.dirty, other.dirty);
    assert!(!same_script_documents(&[first.clone()], &[other]).unwrap());
    let mut new_identity = first.clone();
    new_identity.id += 1;
    assert!(!same_script_documents(&[first], &[new_identity]).unwrap());
}
fn frame(app: &mut Workspace, ctx: &egui::Context, discard: bool) {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            ..Default::default()
        },
        |ui| {
            if discard && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("EPS capture final-pass test");
            }
            app.ui(ui);
        },
    );
}
fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
#[test]
fn eps_capture_rejects_unknown_case_and_conflicting_actions_without_environment_access() {
    let options =
        screenshot_options::Options::parse(["--screenshot".into(), "image.png".into()]).unwrap();
    assert!(
        Config::parse(
            vec!["--eps-native-case".into(), "not-a-case".into()],
            &options
        )
        .is_err()
    );
    for other in [
        "--isolated",
        "--open-files",
        "--preview-image",
        "--preview-page",
        "--preview-pdf-page",
        "--plots-demo",
        "--detached-capture",
    ] {
        assert!(
            Config::parse(
                vec!["--eps-native-case".into(), "open".into(), other.into()],
                &options
            )
            .is_err()
        );
    }
    assert!(Config::parse(vec![], &options).unwrap().is_none());
}
#[test]
fn eps_capture_real_saved_reference_stays_dormant_across_discard_and_accepted_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.eps");
    fs::write(&path, RED).unwrap();
    // It is only inspected as a configured external path. This dormant test
    // must never attempt to execute this deliberately invalid file.
    let renderer = dir.path().join("must-not-run.exe");
    fs::write(&renderer, b"not an executable").unwrap();
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[1 2];Alias=A;plot(A);".into());
    finish(&mut app);
    let mut saved = app.snapshot();
    saved.eps_renderer = Some(renderer);
    saved.preview = Some(matrust::session::PreviewState {
        path: path.clone(),
        page: 0,
        fit: true,
        zoom: 1.,
        format: Default::default(),
        navigation: Default::default(),
    });
    let session = dir.path().join("session.json");
    saved.save(&session).unwrap();
    app.restore_session(Session::load(&session).unwrap());
    let original = app.variables["A"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    let (send, received) = std::sync::mpsc::sync_channel(1);
    let worker_send = std::mem::replace(&mut app.worker.send, send);
    app.eps_capture = Some(
        Config {
            directory: fs::canonicalize(dir.path()).unwrap(),
            case: Case::Dormant,
        }
        .start()
        .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.eps_capture.as_ref().unwrap().ready() && Instant::now() < deadline {
        frame(&mut app, &ctx, true);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.eps_capture.as_ref().unwrap().ready());
    assert_eq!(app.eps_capture.as_ref().unwrap().dormant_frames, 3);
    assert!(matches!(
        received.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(
        !app.busy
            && app.image_view.is_none()
            && app.preview_restore.as_ref().is_some_and(|r| !r.started)
    );
    assert!(
        original.same_snapshot(&app.variables["A"])
            && original.same_snapshot(&app.variables["Alias"])
    );
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    let receipt = fs::read(app.eps_capture.as_ref().unwrap().owned.join("receipt.json")).unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
    assert_eq!(receipt["dormant_frames"], 3);
    assert_eq!(receipt["os_picker_or_input_claim"], false);
    app.worker.send = worker_send;
}
#[test]
fn eps_capture_settings_actions_are_final_pass_and_cancel_preserves_saved_renderer() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let path = PathBuf::from("pending.eps");
    let renderer = PathBuf::from("configured.exe");
    app.select_eps_renderer(renderer.clone());
    app.eps_preview.pending = Some(path.clone());
    crate::headless::run(&ctx, Default::default(), |ui| {
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("EPS final-pass action");
        }
        let discarded = ui.ctx().will_discard();
        app.capture_eps_action(ui.ctx(), &path, eps_preview::Action::Cancel);
        if discarded {
            assert!(app.eps_preview.pending.is_some());
        }
    });
    assert!(app.eps_preview.pending.is_none());
    assert!(!app.busy);
    assert_eq!(app.eps_preview.renderer, Some(renderer));
}
#[cfg(windows)]
#[test]
#[ignore = "requires explicit external Ghostscript and matching built preview worker; actual application worker runs"]
fn eps_capture_actual_interpreter_workflow_preserves_existing_workspace_figures_and_documents() {
    let renderer = PathBuf::from(
        std::env::var_os("MATRUST_EPS_TEST_RENDERER")
            .expect("Explicit external Ghostscript required"),
    );
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[1 2 3];Alias=A;plot(A);title('Keep this figure');".into());
    finish(&mut app);
    app.select_eps_renderer(renderer);
    app.eps_capture = Some(
        Config {
            directory: fs::canonicalize(dir.path()).unwrap(),
            case: Case::Workflow,
        }
        .start()
        .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(55);
    while !app.eps_capture.as_ref().unwrap().ready() && Instant::now() < deadline {
        frame(&mut app, &ctx, false);
        let failure = app.eps_capture.as_ref().unwrap().owned.join("failure.txt");
        assert!(
            !failure.exists(),
            "{}",
            fs::read_to_string(failure).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let capture = app.eps_capture.as_ref().unwrap();
    assert!(capture.ready());
    assert_eq!(capture.stage, 8);
    assert!(capture.events.len() >= 11);
    assert!(app.variables["A"].same_snapshot(&app.variables["Alias"]));
    assert!(app.preview_checkpoint().is_some());
    assert!(!app.busy);
}
