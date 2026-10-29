//! Simulated viewport metadata and real worker ownership; no OS focus/DPI proof.
use super::*;

fn areas() -> Vec<MonitorArea> {
    vec![MonitorArea {
        work: PhysicalRect::new([-1600., 0., 0., 1000.]).unwrap(),
        device: Some("left".into()),
        primary: true,
    }]
}
fn observed() -> Observation {
    Observation {
        inner: [-1400., 100., -600., 700.],
        outer: [-1408., 60., -592., 708.],
        native: 1.,
        zoom: 1.,
        minimized: false,
        maximized: false,
        fullscreen: false,
        focused: false,
    }
}
fn state(now: Instant) -> State {
    let key = Key {
        number: 1,
        identity: matrust::ObjectIdentity::default(),
    };
    let mut manager = Manager::default();
    manager.created(key, egui::ViewportId::from_hash_of(1), None, false);
    let mut state = manager.states.remove(&1).unwrap();
    state.started = now;
    state.observation = Some(observed());
    state.sequence = 1;
    state
}
fn controls() -> Option<monitors::Controls> {
    Some(monitors::Controls {
        minimum: [200., 100.],
        title: 32.,
    })
}
fn applied(
    old: Observation,
    target: matrust::session::figure_window::ResolvedWindow,
) -> Observation {
    let r = target.outer.edges();
    let ppp = old.native * old.zoom;
    Observation {
        outer: r,
        inner: [
            r[0] + 8.,
            r[1] + 40.,
            r[0] + 8. + target.inner_size_points[0] * ppp,
            r[1] + 40. + target.inner_size_points[1] * ppp,
        ],
        ..old
    }
}
#[test]
fn correction_requires_new_actual_observation_and_recomputes_changed_child_dpi() {
    let now = Instant::now();
    let areas = areas();
    let mut s = state(now);
    let Step::Correct(first) = s.step(&areas, None, controls(), now, 0) else {
        panic!("must correct")
    };
    assert_eq!(s.attempts, 1);
    assert!(areas[0].work.contains(first.outer));
    s.observation = Some(applied(observed(), first));
    assert!(
        matches!(
            s.step(
                &areas,
                None,
                controls(),
                now + Duration::from_millis(160),
                0
            ),
            Step::Correct(_)
        ),
        "same sequence is not an acknowledgement"
    );
    let mut changed = s.observation.unwrap();
    changed.native = 2.;
    changed.zoom = 1.25;
    s.observation = Some(changed);
    s.sequence += 1;
    let Step::Correct(second) = s.step(
        &areas,
        None,
        controls(),
        now + Duration::from_millis(320),
        0,
    ) else {
        panic!("changed DPI needs correction")
    };
    assert!(areas[0].work.contains(second.outer));
    assert_ne!(first.inner_size_points, second.inner_size_points);
    s.observation = Some(applied(changed, second));
    s.sequence += 1;
    assert!(matches!(
        s.step(
            &areas,
            None,
            controls(),
            now + Duration::from_millis(480),
            0
        ),
        Step::Confirm
    ));
    assert!(!s.restore);
    assert!(s.saved.normal().is_some());
}
#[test]
fn three_unapplied_commands_timeout_and_tiny_native_controls_fall_back_to_dock() {
    let now = Instant::now();
    let areas = areas();
    let mut s = state(now);
    for i in 0..3 {
        assert!(matches!(
            s.step(
                &areas,
                None,
                controls(),
                now + Duration::from_millis(i * 200),
                0
            ),
            Step::Correct(_)
        ));
    }
    assert_eq!(s.attempts, 3);
    assert!(matches!(
        s.step(&areas, None, controls(), now + Duration::from_secs(1), 0),
        Step::Wait
    ));
    assert!(matches!(
        s.step(&areas, None, controls(), now + Duration::from_secs(2), 0),
        Step::Dock
    ));
    let mut tiny = areas;
    tiny[0].work = PhysicalRect::new([0., 0., 150., 90.]).unwrap();
    assert!(matches!(
        state(now).step(&tiny, None, controls(), now, 0),
        Step::Dock
    ));
    assert!(matches!(
        state(now).step(&[], None, controls(), now, 0),
        Step::Dock
    ));
}
#[test]
fn stale_identity_generation_and_sequence_cannot_publish_native_metadata() {
    let mut manager = Manager::default();
    let key = Key {
        number: 1,
        identity: matrust::ObjectIdentity::default(),
    };
    let viewport = egui::ViewportId::from_hash_of(1);
    manager.created(key.clone(), viewport, None, false);
    let wrong = Key {
        number: 1,
        identity: matrust::ObjectIdentity::default(),
    };
    manager.accept(
        &wrong,
        Message {
            viewport,
            sequence: 10,
            observation: observed(),
        },
    );
    manager.accept(
        &key,
        Message {
            viewport: egui::ViewportId::from_hash_of(2),
            sequence: 10,
            observation: observed(),
        },
    );
    assert_eq!(manager.states[&1].sequence, 0);
    manager.accept(
        &key,
        Message {
            viewport,
            sequence: 2,
            observation: observed(),
        },
    );
    let mut different = observed();
    different.zoom = 2.;
    manager.accept(
        &key,
        Message {
            viewport,
            sequence: 1,
            observation: different,
        },
    );
    assert_eq!(manager.states[&1].observation, Some(observed()));
    different.native = f64::NAN;
    manager.accept(
        &key,
        Message {
            viewport,
            sequence: 3,
            observation: different,
        },
    );
    assert_eq!(manager.states[&1].sequence, 2);
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
fn real_worker_creation_no_focus_dock_and_quit_cleanup_preserve_document_aliases() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=1:20; B=A;figure(1);plot([1 2],[3 4]);figure(2);plot([5 6]);".into());
    finish(&mut app);
    let original = app.all_figures[&1].clone();
    let variables = app.variables.clone();
    let documents = app.figure_documents.clone();
    let mut registry = Registry::default();
    registry.create_window(&mut app, 1, &ctx, false);
    let child = registry.windows[&1].clone();
    let viewport = child.lock().unwrap().viewport;
    assert!(!child.lock().unwrap().activate);
    assert_eq!(app.current_figure, 2);
    let output = crate::headless::run(&ctx, egui::RawInput::default(), |_| {
        registry.register(&ctx, &app.worker.cancel)
    });
    assert_eq!(
        output.viewport_output[&viewport].builder.active,
        Some(false)
    );
    assert!(output.viewport_output[&viewport].builder.position.is_none());
    assert!(!output.viewport_output.values().any(|v| {
        v.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Focus))
    }));
    let key = child.lock().unwrap().key.clone();
    let state = registry.placement.states.get_mut(&1).unwrap();
    state.restore = false;
    state.observation = Some(observed());
    registry.placement.checkpoint(&mut app, &areas());
    let normal = app.figure_windows[&1].window.normal().unwrap().clone();
    registry.placement.docked(&mut app, &key);
    assert!(!app.figure_windows[&1].window.detached());
    assert_eq!(app.figure_windows[&1].window.normal(), Some(&normal));
    registry.retire(1, &ctx);
    registry.create_window(&mut app, 1, &ctx, false);
    assert!(app.figure_windows[&1].window.detached());
    // Accepted quit drains the latest mailbox even when topology is unavailable;
    // it must not run repair/fallback and accidentally persist Dock.
    registry.placement.states.get_mut(&1).unwrap().restore = false;
    registry.placement.topology = None;
    let mut moved = observed();
    moved.outer[0] += 20.;
    moved.outer[2] += 20.;
    moved.inner[0] += 20.;
    moved.inner[2] += 20.;
    {
        let mut child = registry.windows[&1].lock().unwrap();
        child.mailbox.native = Some(Message {
            viewport: child.viewport,
            sequence: 10,
            observation: moved,
        });
    }
    app.allow_close = true;
    registry.advance(&mut app, &ctx);
    assert!(!registry.contains(1));
    assert!(app.figure_windows[&1].window.detached());
    let latest = app
        .figure_checkpoint()
        .figures
        .into_iter()
        .find(|d| d.id == 1)
        .unwrap()
        .window
        .unwrap();
    assert_eq!(latest.normal().unwrap().outer_rect_px, moved.outer);
    assert_eq!(latest.normal().unwrap().work_area_px, normal.work_area_px);
    assert!(original.same_snapshot(&app.all_figures[&1]));
    assert_eq!(documents, app.figure_documents);
    for (name, value) in variables {
        assert!(value.same_snapshot(&app.variables[&name]));
    }
}
#[test]
fn capture_creation_never_changes_persisted_mode_and_minimize_retains_last_normal() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("plot([1 2],[3 4]);".into());
    finish(&mut app);
    let id = app.current_figure;
    app.undock_capture_figure(id, &ctx);
    assert!(app.detached.capture_mode);
    assert!(app.figure_windows.is_empty());
    assert!(app.detached.placement.states.is_empty());
    let key = Key {
        number: id,
        identity: app.all_figures[&id].identity.clone(),
    };
    let mut manager = Manager::default();
    manager.created(key.clone(), egui::ViewportId::from_hash_of(1), None, false);
    let state = manager.states.get_mut(&id).unwrap();
    state.restore = false;
    state.observation = Some(observed());
    manager.checkpoint(&mut app, &areas());
    let normal = app.figure_windows[&id].window.normal().unwrap().clone();
    let state = manager.states.get_mut(&id).unwrap();
    let mut o = observed();
    o.maximized = true;
    state.observation = Some(o);
    manager.checkpoint(&mut app, &areas());
    assert!(app.figure_windows[&id].window.maximized());
    o.minimized = true;
    o.maximized = false;
    manager.states.get_mut(&id).unwrap().observation = Some(o);
    manager.checkpoint(&mut app, &areas());
    assert!(app.figure_windows[&id].window.maximized());
    assert_eq!(app.figure_windows[&id].window.normal(), Some(&normal));
    let other = Key {
        number: id,
        identity: matrust::ObjectIdentity::default(),
    };
    manager.docked(&mut app, &other);
    assert!(manager.states.contains_key(&id));
    assert!(app.figure_windows[&id].window.detached());
}

#[test]
fn maximized_invisible_frame_overhang_keeps_visible_caption_and_confirms_restore() {
    let now = Instant::now();
    let mut state = state(now);
    let areas = vec![MonitorArea {
        work: PhysicalRect::new([0., 0., 1920., 1040.]).unwrap(),
        device: None,
        primary: true,
    }];
    let controls = monitors::Controls {
        minimum: [200., 100.],
        title: 23.,
    };
    let observation = Observation {
        outer: [-8., -8., 1928., 1048.],
        inner: [0., 23., 1920., 1040.],
        maximized: true,
        ..observed()
    };
    assert!(observation.metrics().is_some());
    assert!(
        !areas[0]
            .work
            .contains(PhysicalRect::new(observation.outer).unwrap())
    );
    assert_eq!(visible_controls(observation, &areas, controls), Some(0));
    state.maximizing = true;
    state.observation = Some(observation);
    assert!(matches!(
        state.step(&areas, None, Some(controls), now, 0),
        Step::Confirm
    ));
    assert!(!state.restore);
    let mut lost = observation;
    lost.inner[2] += 2500.;
    lost.outer[2] += 2500.;
    assert_eq!(visible_controls(lost, &areas, controls), None);
}

#[test]
fn saved_maximize_intent_waits_for_focus_after_an_unfocused_observation() {
    let now = Instant::now();
    let areas = areas();
    let mut state = state(now);
    state.saved = FigureWindow::new(true, None, true).unwrap();
    let Step::Correct(target) = state.step(&areas, None, controls(), now, 0) else {
        panic!("normal correction")
    };
    state.observation = Some(applied(observed(), target));
    state.sequence += 1;
    assert!(matches!(
        state.step(
            &areas,
            None,
            controls(),
            now + Duration::from_millis(200),
            0
        ),
        Step::Confirm
    ));
    assert!(state.deferred_maximize && state.saved.maximized());
    assert!(!state.restore);
    assert!(matches!(
        state.step(&areas, None, controls(), now + Duration::from_secs(1), 0),
        Step::Confirm
    ));
    state.observation.as_mut().unwrap().focused = true;
    assert!(matches!(
        state.step(&areas, None, controls(), now + Duration::from_secs(2), 0),
        Step::Maximize
    ));
    assert!(state.maximizing && !state.deferred_maximize);
}

#[test]
fn explicit_undock_restores_saved_maximize_while_already_focused() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("plot([1 2],[3 4]);".into());
    finish(&mut app);
    let id = app.current_figure;
    let original = app.all_figures[&id].clone();
    let key = Key {
        number: id,
        identity: original.identity.clone(),
    };
    let normal = observed().normal(&areas()[0]).unwrap();
    publish(
        &mut app,
        &key,
        FigureWindow::new(false, Some(normal), true).unwrap(),
    );
    app.undock_figure(id, &ctx);
    assert!(app.detached.windows[&id].lock().unwrap().activate);
    let state = app.detached.placement.states.get_mut(&id).unwrap();
    assert!(state.explicit_activation);
    assert!(!state.seen_unfocused);
    let mut o = observed();
    o.focused = true;
    state.observation = Some(o);
    state.sequence = 1;
    let now = state.started;
    let Step::Correct(target) = state.step(&areas(), None, controls(), now, 0) else {
        panic!("normal correction")
    };
    state.observation = Some(applied(o, target));
    state.sequence += 1;
    assert!(matches!(
        state.step(
            &areas(),
            None,
            controls(),
            now + Duration::from_millis(200),
            0
        ),
        Step::Maximize
    ));
    assert!(
        !state.seen_unfocused,
        "Explicit activation is not a fabricated unfocused observation"
    );
    assert!(state.saved.maximized());
    assert!(original.same_snapshot(&app.all_figures[&id]));
}
