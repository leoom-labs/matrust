//! Synthetic event tests of the real coordinator. These are not native evidence.
use super::*;

fn token() -> Token {
    Token {
        nonce: 1,
        number: 1,
        revision: 0,
    }
}
fn screenshot(viewport: egui::ViewportId, token: Token) -> egui::Event {
    egui::Event::Screenshot {
        viewport_id: viewport,
        user_data: egui::UserData::new(token),
        image: Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::WHITE)),
    }
}
fn reply(
    events: Vec<egui::Event>,
    viewport: egui::ViewportId,
    requested: bool,
) -> Option<matrust::Result<Arc<egui::ColorImage>>> {
    let ctx = egui::Context::default();
    let mut result = None;
    crate::headless::run(
        &ctx,
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            result = image_reply(ui.ctx(), viewport, token(), requested);
        },
    );
    result
}
#[test]
fn native_replies_require_exact_viewport_nonce_single_response_and_bounded_image() {
    let id = egui::ViewportId::from_hash_of("child");
    assert!(
        reply(vec![screenshot(id, token())], id, true)
            .unwrap()
            .is_ok()
    );
    assert!(
        reply(vec![screenshot(egui::ViewportId::ROOT, token())], id, true)
            .unwrap()
            .is_err()
    );
    assert!(
        reply(
            vec![screenshot(
                id,
                Token {
                    nonce: 2,
                    ..token()
                }
            )],
            id,
            true
        )
        .unwrap()
        .is_err()
    );
    assert!(
        reply(vec![screenshot(id, token())], id, false)
            .unwrap()
            .is_err()
    );
    assert!(
        reply(
            vec![screenshot(id, token()), screenshot(id, token())],
            id,
            true
        )
        .unwrap()
        .is_err()
    );
    let bad = egui::Event::Screenshot {
        viewport_id: id,
        user_data: egui::UserData::new(token()),
        image: Arc::new(egui::ColorImage::filled([4097, 1], egui::Color32::WHITE)),
    };
    assert!(reply(vec![bad], id, true).unwrap().is_err());
    assert!(reply(vec![], id, true).is_none());
}

#[test]
fn capture_arguments_require_fresh_absolute_private_recovery_without_demo_or_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("private.json");
    std::fs::write(&session, b"not parsed by the argument validator").unwrap();
    let output = dir.path().join("frames");
    let args = vec![
        "--detached-capture".into(),
        output.to_string_lossy().into_owned(),
        "--screenshot".into(),
        output.join("root.png").to_string_lossy().into_owned(),
        "--theme".into(),
        "dark".into(),
    ];
    let options = screenshot_options::Options::parse(args.clone()).unwrap();
    assert!(
        Config::parse(args.clone(), &options, Some(session.clone()))
            .unwrap()
            .is_some()
    );
    assert!(Config::parse(args.clone(), &options, None).is_err());
    for forbidden in ["--isolated", "--plots-demo", "--open-files", "--unknown"] {
        let mut bad = args.clone();
        bad.push(forbidden.into());
        assert!(Config::parse(bad, &options, Some(session.clone())).is_err());
    }
    assert!(
        Config::parse(
            vec!["--open-files".into(), "--detached-capture".into()],
            &options,
            None
        )
        .unwrap()
        .is_none()
    );
    let config = Config::parse(args.clone(), &options, Some(session.clone()))
        .unwrap()
        .unwrap();
    config.start().unwrap();
    assert!(
        Config::parse(args, &options, Some(session))
            .unwrap()
            .unwrap()
            .start()
            .is_err()
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
fn root_frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 900.),
            )),
            events,
            ..Default::default()
        },
        |ui| app.ui(ui),
    )
}
fn child(app: &mut Workspace, ctx: &egui::Context, id: u32, events: Vec<egui::Event>) {
    let out = root_frame(app, ctx, vec![]);
    let viewport = app.detached.windows[&id].lock().unwrap().viewport;
    let callback = out.viewport_output[&viewport]
        .viewport_ui_cb
        .as_ref()
        .unwrap()
        .clone();
    let mut input = egui::RawInput {
        viewport_id: viewport,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900., 650.),
        )),
        events,
        ..Default::default()
    };
    input.viewports.entry(viewport).or_default().parent = Some(egui::ViewportId::ROOT);
    crate::headless::run(ctx, input, move |ui| callback(ui));
}

#[test]
fn coordinator_waits_for_both_real_deferred_callbacks_before_root_and_rejects_reused_identity() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.run("figure(1);plot([1 2 3]);figure(2);plot([4 5 6]);".into());
    finish(&mut app);
    for _ in 0..4 {
        root_frame(&mut app, &ctx, vec![]);
    }
    let output = dir.path().join("frames");
    let mut capture = Config {
        root: output.join("root.png"),
        directory: output.clone(),
        compact: false,
    }
    .start()
    .unwrap();
    assert!(!capture.advance(&mut app, &ctx).unwrap());
    for id in [1, 2] {
        for _ in 0..8 {
            root_frame(&mut app, &ctx, vec![]);
            finish(&mut app);
        }
        assert!(!capture.advance(&mut app, &ctx).unwrap());
        assert!(!capture.root_requested);
        assert!(!app.allow_close);
        for _ in 0..4 {
            child(&mut app, &ctx, id, vec![]);
            finish(&mut app);
        }
        let (viewport, request_token) = {
            let state = app.detached.windows[&id].lock().unwrap();
            let request = state.capture.as_ref().unwrap();
            assert!(request.requested);
            (state.viewport, request.token)
        };
        // Synthetic reply tests association/order only, never native pixels.
        child(
            &mut app,
            &ctx,
            id,
            vec![screenshot(viewport, request_token)],
        );
        assert!(!capture.advance(&mut app, &ctx).unwrap());
        assert_eq!(capture.records.len(), id as usize);
        assert!(!output.join("capture.json").exists());
    }
    for _ in 0..3 {
        root_frame(&mut app, &ctx, vec![]);
        finish(&mut app);
        capture.advance(&mut app, &ctx).unwrap();
    }
    assert!(capture.root_requested);
    assert!(!app.allow_close);
    app.run("close(1);figure(1);plot([7 8]);".into());
    finish(&mut app);
    assert!(
        capture
            .advance(&mut app, &ctx)
            .unwrap_err()
            .contains("identity/model")
    );
    assert!(!output.join("capture.json").exists());
}

#[test]
fn capture_wait_reply_and_deferred_requests_wake_only_after_child_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.run("figure(1);plot([1 2 3]);".into());
    finish(&mut app);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        root_frame(&mut app, &ctx, vec![]);
        finish(&mut app);
        if Coordinator::idle(&app) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Private fixture must finish startup before capture"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let output = dir.path().join("unlocked-capture");
    let mut capture = Config {
        root: output.join("root.png"),
        directory: output.clone(),
        compact: false,
    }
    .start()
    .unwrap();
    assert!(!capture.advance(&mut app, &ctx).unwrap());
    let child = app.detached.windows[&1].clone();
    let check = |action: &mut dyn FnMut(&egui::Context)| {
        let wake = egui::Context::default();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        let shared = child.clone();
        wake.set_request_repaint_callback(move |_| {
            assert!(
                shared.try_lock().is_ok(),
                "Capture wake must not hold child mutex"
            );
            observed.fetch_add(1, Ordering::Relaxed);
        });
        action(&wake);
        assert!(count.load(Ordering::Relaxed) > 0);
    };
    check(&mut |wake| {
        assert!(!capture.advance(&mut app, wake).unwrap());
    });
    assert_eq!(capture.index, 0);
    assert!(child.lock().unwrap().capture.is_some());
    // The same deferred effects used by the real child callback are dispatched
    // only after that callback releases its render-state guard.
    let commands = {
        let state = child.lock().unwrap();
        FrameRequests {
            screenshot: Some(state.capture.as_ref().unwrap().token),
            repaint: true,
            wake_root: true,
        }
    };
    let mut commands = Some(commands);
    check(&mut |wake| commands.take().unwrap().dispatch(wake));
    child.lock().unwrap().capture.as_mut().unwrap().reply = Some(Ok(Arc::new(
        egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
    )));
    check(&mut |wake| {
        assert!(!capture.advance(&mut app, wake).unwrap());
    });
    assert_eq!(capture.index, 1);
    assert_eq!(capture.records.len(), 1);
    assert!(child.lock().unwrap().capture.is_none());
    assert!(output.join("figure-1.png").is_file());
    assert!(!output.join("capture.json").exists());
}

#[test]
fn stored_coordinator_runs_through_actual_root_ui_and_deferred_callbacks() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.run("figure(1);plot([1 2 3]);figure(2);plot([4 5 6]);".into());
    finish(&mut app);
    let original = app.all_figures.clone();
    let output = dir.path().join("stored-coordinator");
    app.screenshot_path = Some(output.join("root.png"));
    app.detached_capture = Some(
        Config {
            root: output.join("root.png"),
            directory: output.clone(),
            compact: false,
        }
        .start()
        .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.detached_capture.as_ref().unwrap().targets.is_none() {
        root_frame(&mut app, &ctx, vec![]);
        finish(&mut app);
        assert!(
            Instant::now() < deadline,
            "Stored coordinator never reached genuine Undock"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !app.screenshot_requested,
        "Ordinary root screenshot must remain suppressed"
    );
    assert_eq!(app.detached.windows.len(), 2);
    assert!(app.detached.capture_mode);
    for id in [1, 2] {
        let mut requested = None;
        for _ in 0..32 {
            root_frame(&mut app, &ctx, vec![]);
            finish(&mut app);
            child(&mut app, &ctx, id, vec![]);
            finish(&mut app);
            let state = app.detached.windows[&id].lock().unwrap();
            if let Some(request) = &state.capture
                && request.requested
            {
                requested = Some((state.viewport, request.token));
                break;
            }
        }
        let (viewport, token) =
            requested.expect("Actual deferred callback did not request capture");
        assert!(!app.detached_capture.as_ref().unwrap().root_requested);
        child(&mut app, &ctx, id, vec![screenshot(viewport, token)]);
        root_frame(&mut app, &ctx, vec![]);
        assert!(output.join(format!("figure-{id}.png")).is_file());
        assert_eq!(app.detached_capture.as_ref().unwrap().index, id as usize);
        assert!(!app.allow_close);
    }
    for _ in 0..8 {
        root_frame(&mut app, &ctx, vec![]);
        finish(&mut app);
        if app.detached_capture.as_ref().unwrap().root_requested {
            break;
        }
    }
    assert!(app.detached_capture.as_ref().unwrap().root_requested);
    root_frame(
        &mut app,
        &ctx,
        vec![screenshot(
            egui::ViewportId::ROOT,
            Token {
                nonce: MAX_WINDOWS as u64 + 1,
                number: 0,
                revision: 0,
            },
        )],
    );
    assert!(app.allow_close);
    assert!(app.detached_capture.as_ref().unwrap().finished);
    assert_eq!(app.detached_capture.as_ref().unwrap().records.len(), 3);
    assert!(app.detached.windows.is_empty());
    assert!(output.join("root.png").is_file());
    assert!(output.join("capture.json").is_file());
    assert!(!output.join("failure.txt").exists());
    assert_eq!(app.all_figures.len(), original.len());
    for (id, figure) in original {
        assert!(app.all_figures[&id].same_snapshot(&figure));
    }
    assert!(!app.screenshot_requested);
}

#[test]
fn capture_frame_requests_wait_without_self_repaint_then_finish_finite_ready_sequence() {
    fn frame(ctx: &egui::Context, state: &mut Child, events: Vec<egui::Event>) -> FrameRequests {
        let viewport = state.viewport;
        let mut input = egui::RawInput {
            viewport_id: viewport,
            events,
            ..Default::default()
        };
        input.viewports.entry(viewport).or_default().parent = Some(egui::ViewportId::ROOT);
        let mut commands = None;
        crate::headless::run(ctx, input, |ui| {
            commands = Some(child_frame(state, ui.ctx(), egui::ViewportClass::Deferred));
            let repeated = child_frame(state, ui.ctx(), egui::ViewportClass::Deferred);
            assert!(!repeated.repaint && !repeated.wake_root && repeated.screenshot.is_none());
        });
        commands.unwrap()
    }
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("figure(1);plot([1 2 3]);".into());
    finish(&mut app);
    app.undock_capture_figure(1, &ctx);
    let shared = app.detached.windows[&1].clone();
    let mut state = shared.lock().unwrap();
    let original = state.presentation.figure.clone();
    let request_token = Token {
        nonce: 1,
        number: 1,
        revision: state.presentation.revision,
    };
    let viewport = state.viewport;
    state.capture = Some(Request {
        token: request_token,
        key: state.key.clone(),
        viewport,
        ready: 0,
        last_frame: None,
        requested: false,
        reply: None,
        scale: 1.,
    });
    // Every readiness blocker must wait, including a child without a render
    // cache. Repeated frames cannot schedule an unbounded child repaint loop.
    for blocker in 0..4 {
        state.geometry = if blocker == 0 {
            None
        } else {
            Some(Geometry::prepare(1, &original, &AtomicBool::new(false)).unwrap())
        };
        let presentation = Arc::get_mut(&mut state.presentation).unwrap();
        presentation.busy = blocker == 1;
        presentation.modal = blocker == 2;
        state.minimized = blocker == 3;
        for _ in 0..8 {
            let commands = frame(&ctx, &mut state, vec![]);
            assert!(!commands.repaint && !commands.wake_root && commands.screenshot.is_none());
            assert_eq!(state.capture.as_ref().unwrap().ready, 0);
            assert!(!state.capture.as_ref().unwrap().requested);
        }
    }
    state.geometry = Some(Geometry::prepare(1, &original, &AtomicBool::new(false)).unwrap());
    Arc::get_mut(&mut state.presentation).unwrap().busy = false;
    Arc::get_mut(&mut state.presentation).unwrap().modal = false;
    state.minimized = false;
    let first = frame(&ctx, &mut state, vec![]);
    assert!(first.repaint && first.screenshot.is_none() && !first.wake_root);
    assert_eq!(state.capture.as_ref().unwrap().ready, 1);
    // Losing readiness resets the finite sequence, rather than accepting two
    // later frames as if the interrupted first frame were still current.
    Arc::get_mut(&mut state.presentation).unwrap().busy = true;
    let waiting = frame(&ctx, &mut state, vec![]);
    assert!(!waiting.repaint && !waiting.wake_root && waiting.screenshot.is_none());
    assert_eq!(state.capture.as_ref().unwrap().ready, 0);
    Arc::get_mut(&mut state.presentation).unwrap().busy = false;
    for expected in 1..=3 {
        let commands = frame(&ctx, &mut state, vec![]);
        assert!(commands.repaint && !commands.wake_root);
        assert_eq!(state.capture.as_ref().unwrap().ready, expected);
        assert_eq!(
            commands.screenshot,
            (expected == 3).then_some(request_token)
        );
    }
    // Once sent, screenshot delivery is backend-driven, not a polling loop.
    for _ in 0..8 {
        let commands = frame(&ctx, &mut state, vec![]);
        assert!(!commands.repaint && !commands.wake_root && commands.screenshot.is_none());
    }
    let delivered = frame(&ctx, &mut state, vec![screenshot(viewport, request_token)]);
    assert!(delivered.wake_root && !delivered.repaint && delivered.screenshot.is_none());
    assert!(
        state
            .capture
            .as_ref()
            .unwrap()
            .reply
            .as_ref()
            .unwrap()
            .is_ok()
    );
    assert!(original.same_snapshot(&state.presentation.figure));
}
