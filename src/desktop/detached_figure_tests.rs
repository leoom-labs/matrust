//! Actual deferred callback, root coordinator and shared-worker tests. No OS proof.
use super::*;

fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn frame(app: &mut Workspace, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
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
fn settle(app: &mut Workspace, ctx: &egui::Context) {
    for _ in 0..8 {
        finish(app);
        frame(app, ctx, vec![]);
    }
    finish(app);
}
fn setup() -> (Workspace, egui::Context, tempfile::TempDir) {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.run("keep=1:12; alias=keep;figure(1);plot([1 2 3]);figure(2);plot([7 8 9]);".into());
    finish(&mut app);
    (app, ctx, dir)
}
fn child_frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    id: u32,
    close: bool,
    focused: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let output = frame(app, ctx, vec![]);
    let viewport = app.detached.windows[&id].lock().unwrap().viewport;
    let callback = output.viewport_output[&viewport]
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
    let info = input.viewports.entry(viewport).or_default();
    info.parent = Some(egui::ViewportId::ROOT);
    info.focused = Some(focused);
    if close {
        info.events.push(egui::ViewportEvent::Close);
    }
    crate::headless::run(ctx, input, move |ui| callback(ui))
}
fn position(shape: &egui::epaint::Shape, text: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.text() == text => {
            Some(t.pos + t.galley.size() * 0.5)
        }
        egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| position(s, text)),
        _ => None,
    }
}
fn click(app: &mut Workspace, ctx: &egui::Context, id: Option<u32>, text: &str) {
    // New egui Areas have an invisible sizing frame. Wait only for the real
    // painted target, with a fixed bound and no fallback coordinates.
    let pos = (0..8)
        .find_map(|_| {
            let out = if let Some(id) = id {
                child_frame(app, ctx, id, false, true, vec![])
            } else {
                frame(app, ctx, vec![])
            };
            let found = |s: &egui::epaint::ClippedShape| position(&s.shape, text);
            if id.is_some() {
                out.shapes.iter().find_map(found)
            } else {
                // Root dialogs are modal layers above the chrome, whose
                // toolstrip has its own Save: take the topmost match.
                out.shapes.iter().rev().find_map(found)
            }
        })
        .unwrap_or_else(|| panic!("Missing painted {text} after 8 frames"));
    for pressed in [true, false] {
        let events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        if let Some(id) = id {
            child_frame(app, ctx, id, false, true, events);
        } else {
            frame(app, ctx, events);
        }
    }
}

#[test]
fn deferred_clean_inactive_x_closes_exact_document_after_real_selection_ack() {
    let (mut app, ctx, dir) = setup();
    app.submit(Job::SaveFigureById(1, dir.path().join("one.mfig"), None));
    finish(&mut app);
    let original = app.all_figures[&2].clone();
    let variables = app.variables.clone();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.current_figure, 2);
    let viewport = app.detached.windows[&1].lock().unwrap().viewport;
    let output = child_frame(&mut app, &ctx, 1, true, false, vec![]);
    assert!(
        output.viewport_output[&viewport]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
    );
    settle(&mut app, &ctx);
    assert!(!app.detached.contains(1));
    assert_eq!(app.figure_ids, vec![2]);
    assert!(original.same_snapshot(&app.all_figures[&2]));
    assert!(!app.allow_close && !app.pending_quit);
    for (name, value) in variables {
        assert!(value.same_snapshot(&app.variables[&name]));
    }
}

#[test]
fn deferred_dirty_x_cancel_then_discard_retains_window_and_shared_history_until_commit() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let original = app.all_figures[&1].clone();
    let documents = app.figure_documents.clone();
    let history = app.figure_history.clone();
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    settle(&mut app, &ctx);
    assert!(matches!(
        app.figure_close.as_ref().map(|f| f.action),
        Some(figure_files::Closing::Figure(1))
    ));
    click(&mut app, &ctx, None, "Cancel");
    settle(&mut app, &ctx);
    assert!(app.detached.contains(1));
    assert!(app.figure_close.is_none());
    assert!(original.same_snapshot(&app.all_figures[&1]));
    assert!((original.series[0].points).same_storage(&app.all_figures[&1].series[0].points));
    assert_eq!(app.figure_documents, documents);
    assert_eq!(app.figure_history, history);
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    settle(&mut app, &ctx);
    click(&mut app, &ctx, None, "Discard changes");
    settle(&mut app, &ctx);
    assert!(!app.all_figures.contains_key(&1));
    assert!(!app.detached.contains(1));
    assert!(app.all_figures.contains_key(&2));
    assert!(!app.allow_close);
}

#[test]
fn child_real_properties_undo_redo_and_dock_keep_target_samples_and_document() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let original = app.all_figures[&1].clone();
    click(&mut app, &ctx, Some(1), "Properties");
    settle(&mut app, &ctx);
    assert_eq!(app.current_figure, 1);
    assert!(app.figure_editor.is_some());
    // Real modal Cancel, then a real worker edit to establish history for the child buttons.
    click(&mut app, &ctx, None, "Cancel");
    settle(&mut app, &ctx);
    let mut updated = original.clone();
    updated.title = "Detached edit".into();
    app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
        id: 1,
        original: original.clone(),
        updated: updated.clone(),
        view_before: None,
    })));
    finish(&mut app);
    settle(&mut app, &ctx);
    click(&mut app, &ctx, Some(1), "Undo");
    settle(&mut app, &ctx);
    assert!(original.same_snapshot(&app.all_figures[&1]));
    click(&mut app, &ctx, Some(1), "Redo");
    settle(&mut app, &ctx);
    assert!(updated.same_snapshot(&app.all_figures[&1]));
    let history = app.figure_history.clone();
    let documents = app.figure_documents.clone();
    let previous = app.detached.windows[&1].lock().unwrap().viewport;
    click(&mut app, &ctx, Some(1), "Dock figure");
    settle(&mut app, &ctx);
    assert!(!app.detached.contains(1));
    assert_eq!(app.figure_documents, documents);
    assert_eq!(app.figure_history, history);
    assert!((original.series[0].points).same_storage(&app.all_figures[&1].series[0].points));
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    assert_ne!(previous, app.detached.windows[&1].lock().unwrap().viewport);
}

#[test]
fn stale_selection_and_reused_number_cannot_apply_old_child_action() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let key = app.detached.windows[&1].lock().unwrap().key.clone();
    app.run("close(1);figure(1);plot([40 50]);".into());
    finish(&mut app);
    let replacement = app.all_figures[&1].clone();
    assert!(!key.identity.same_object(&replacement.identity));
    app.submit(Job::SelectExistingFigure(Select {
        key: key.clone(),
        ticket: 77,
    }));
    finish(&mut app);
    assert!(app.console.iter().any(|s| s.contains("no longer open")));
    app.submit(Job::CloseExistingFigure(key));
    finish(&mut app);
    settle(&mut app, &ctx);
    assert!(replacement.same_snapshot(&app.all_figures[&1]));
    assert!(!app.detached.contains(1));
}

#[test]
fn worker_preparation_cancel_is_retryable_and_retains_exact_patch_and_bar_geometry() {
    let mut engine = matrust::Engine::default();
    engine
        .execute("bar(1:10000);hold on;patch('XData',[0;1;1;0],'YData',[0;0;1;1],'FaceColor',[1 0 0],'EdgeColor','none');")
        .unwrap();
    let figure = engine.figure.clone().unwrap();
    let cancel = AtomicBool::new(true);
    assert!(Geometry::prepare(1, &figure, &cancel).is_err());
    cancel.store(false, Ordering::Relaxed);
    let Geometry::Ordinary(prepared) = Geometry::prepare(1, &figure, &cancel).unwrap() else {
        panic!("ordinary figure")
    };
    assert_eq!(prepared.points[0].len(), 10000);
    assert_eq!(prepared.points[1].len(), 4);
    for (series, points) in figure.series.iter().zip(&prepared.points) {
        let reference = series.display_points_axes(4096, prepared.transforms);
        assert_eq!(reference.len(), points.len());
        for (expected, actual) in reference.iter().zip(points) {
            assert_eq!(expected[0].to_bits(), actual.x.to_bits());
            assert_eq!(expected[1].to_bits(), actual.y.to_bits());
        }
    }
    assert!(figure.same_snapshot(engine.figure.as_ref().unwrap()));
    assert!(
        prepared
            .points
            .iter()
            .map(|v| v.capacity() * std::mem::size_of::<egui_plot::PlotPoint>())
            .sum::<usize>()
            <= figure_geometry::ONE_CACHE_BYTES
    );
}

#[test]
fn child_close_is_not_queued_behind_busy_or_unrelated_modal_and_stop_clears_pending() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    // Deterministic busy source: intercept this one worker request, preserving the real sender for retry.
    let (sender, requests) = std::sync::mpsc::sync_channel(1);
    let real = std::mem::replace(&mut app.worker.send, sender);
    let key = app.detached.windows[&1].lock().unwrap().key.clone();
    app.submit(Job::SelectExistingFigure(Select { key, ticket: 90 }));
    frame(&mut app, &ctx, vec![]);
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    assert!(
        app.detached.windows[&1]
            .lock()
            .unwrap()
            .mailbox
            .command
            .is_none()
    );
    app.stop_work();
    assert!(app.detached.pending.is_none());
    requests.try_recv().unwrap();
    app.worker.send = real;
    app.busy = false;
    app.worker.cancel.store(false, Ordering::Relaxed);
    app.pending_close = Some(0);
    frame(&mut app, &ctx, vec![]);
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    assert!(
        app.detached.windows[&1]
            .lock()
            .unwrap()
            .mailbox
            .command
            .is_none()
    );
    app.pending_close = None;
    settle(&mut app, &ctx);
    assert!(app.all_figures.contains_key(&1));
    assert!(app.figure_close.is_none());
}

#[test]
fn dirty_child_close_picker_cancel_save_failure_and_real_save_preserve_then_close() {
    let (mut app, ctx, dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let original = app.all_figures[&1].clone();
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    settle(&mut app, &ctx);
    {
        let _picker = figure_files::save_picker(None);
        click(&mut app, &ctx, None, "Save");
        settle(&mut app, &ctx);
    }
    assert!(app.detached.contains(1));
    assert!(original.same_snapshot(&app.all_figures[&1]));
    assert!(app.figure_close.is_some());
    {
        let _picker = figure_files::save_picker(Some(dir.path().join("missing").join("one.mfig")));
        click(&mut app, &ctx, None, "Save");
        settle(&mut app, &ctx);
    }
    assert!(app.detached.contains(1));
    assert!(original.same_snapshot(&app.all_figures[&1]));
    assert!(app.figure_dirty(1));
    assert!(app.figure_close.as_ref().is_some_and(|flow| !flow.saving));
    let path = dir.path().join("one.mfig");
    {
        let _picker = figure_files::save_picker(Some(path.clone()));
        click(&mut app, &ctx, None, "Save");
        settle(&mut app, &ctx);
    }
    assert!(!app.detached.contains(1));
    assert!(!app.all_figures.contains_key(&1));
    assert!(app.all_figures.contains_key(&2));
    let loaded = matrust::files::load_figure(&path).unwrap();
    assert_eq!(
        loaded.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        original.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice()
    );
    assert!(!app.allow_close);
}

#[test]
fn unchanged_child_paints_do_not_post_repeated_view_or_prepare_requests() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    child_frame(&mut app, &ctx, 1, false, false, vec![]);
    settle(&mut app, &ctx);
    for _ in 0..3 {
        child_frame(&mut app, &ctx, 1, false, false, vec![]);
        settle(&mut app, &ctx);
    }
    child_frame(&mut app, &ctx, 1, false, false, vec![]);
    let child = app.detached.windows[&1].lock().unwrap();
    assert!(child.mailbox.view.is_none());
    assert!(!child.prepare_requested);
    assert!(child.geometry.is_some());
}

#[test]
fn root_quit_uses_all_documents_while_child_close_targets_only_one() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    app.undock_figure(2, &ctx);
    settle(&mut app, &ctx);
    app.request_quit(&ctx);
    assert!(matches!(
        app.figure_close.as_ref().map(|flow| flow.action),
        Some(figure_files::Closing::Application)
    ));
    click(&mut app, &ctx, None, "Cancel");
    settle(&mut app, &ctx);
    assert!(app.detached.contains(1) && app.detached.contains(2));
    assert!(!app.allow_close);
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    settle(&mut app, &ctx);
    assert!(matches!(
        app.figure_close.as_ref().map(|flow| flow.action),
        Some(figure_files::Closing::Figure(1))
    ));
}

#[test]
fn prepared_subplot_changed_style_view_and_selection_fail_closed_before_ui_rebuild() {
    let mut engine = matrust::Engine::default();
    engine
        .execute("subplot(1,2,1);plot(1:20000);subplot(1,2,2);plot(1:20000);")
        .unwrap();
    let original = engine.figure.clone().unwrap();
    let cancel = AtomicBool::new(false);
    let Geometry::Subplots(mut canvas) = Geometry::prepare(1, &original, &cancel).unwrap() else {
        panic!("subplot figure")
    };
    assert!(canvas.matches_prepared(1, &original));
    let bytes = canvas.retained_point_bytes();
    let ctx = egui::Context::default();
    for changed in [
        {
            let mut f = original.clone();
            f.title = "New title".into();
            f
        },
        {
            let mut f = original.clone();
            f.xlim = Some([100., 200.]);
            f
        },
        {
            let mut f = original.clone();
            f.series[0].visible = false;
            f
        },
    ] {
        let mut rejected = false;
        crate::headless::run(&ctx, Default::default(), |ui| {
            rejected = canvas.show_prepared(ui, 1, &changed, false, None).is_err();
        });
        assert!(rejected);
        assert_eq!(canvas.retained_point_bytes(), bytes);
        assert!(canvas.matches_prepared(1, &original));
        let Geometry::Subplots(fresh) = Geometry::prepare(1, &changed, &cancel).unwrap() else {
            panic!("subplot figure")
        };
        assert!(fresh.matches_prepared(1, &changed));
    }
    assert!(!canvas.matches_prepared(2, &original));
    let position = original
        .all_axes()
        .find(|(p, _)| *p != original.active_position())
        .unwrap()
        .0;
    let mut selected = original.clone();
    selected.select_subplot(position, false).unwrap();
    assert!(canvas.matches_prepared(1, &selected));
    let mut accepted = false;
    crate::headless::run(&ctx, Default::default(), |ui| {
        accepted = canvas.show_prepared(ui, 1, &selected, false, None).is_ok();
    });
    assert!(accepted);
}

fn discarded_close_frame(app: &mut Workspace, ctx: &egui::Context) -> usize {
    let output = frame(app, ctx, vec![]);
    let viewport = app.detached.windows[&1].lock().unwrap().viewport;
    let callback = output.viewport_output[&viewport]
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
        ..Default::default()
    };
    let info = input.viewports.entry(viewport).or_default();
    info.parent = Some(egui::ViewportId::ROOT);
    info.events.push(egui::ViewportEvent::Close);
    let mut passes = 0;
    crate::headless::run(ctx, input, |ui| {
        if passes == 0 {
            ui.ctx().request_discard("actual child close retry");
        }
        passes += 1;
        callback(ui);
    });
    passes
}

#[test]
fn discarded_child_close_pass_and_repeated_x_submit_one_typed_selection() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let passes = discarded_close_frame(&mut app, &ctx);
    assert!(passes >= 2);
    assert!(
        app.detached.windows[&1]
            .lock()
            .unwrap()
            .mailbox
            .command
            .is_some()
    );
    let (sender, requests) = std::sync::mpsc::sync_channel(1);
    let real = std::mem::replace(&mut app.worker.send, sender);
    frame(&mut app, &ctx, vec![]);
    assert!(matches!(
        requests.try_recv().unwrap().job,
        Job::SelectExistingFigure(_)
    ));
    child_frame(&mut app, &ctx, 1, true, false, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(matches!(
        requests.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    app.worker.send = real;
    app.busy = false;
    app.stop_work();
    assert!(app.detached.pending.is_none());
    assert!(app.all_figures.contains_key(&1));
}

#[test]
fn detached_retirement_releases_owned_plot_caches_without_touching_root_or_other_window() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    app.undock_figure(2, &ctx);
    settle(&mut app, &ctx);
    for id in [1, 2] {
        for _ in 0..3 {
            child_frame(&mut app, &ctx, id, false, false, vec![]);
        }
    }
    let first = egui::Id::new((
        "detached_figure",
        app.detached.windows[&1].lock().unwrap().viewport,
    ));
    let second = egui::Id::new((
        "detached_figure",
        app.detached.windows[&2].lock().unwrap().viewport,
    ));
    assert!(egui_plot::PlotMemory::load(&ctx, first).is_some());
    assert!(egui_plot::PlotMemory::load(&ctx, second).is_some());
    ctx.data_mut(|d| {
        d.insert_temp(
            first.with("legend_best"),
            matrust::plot::LegendBestCache::default(),
        )
    });
    let source = app.all_figures[&1].clone();
    app.detached.retire(1, &ctx);
    assert!(egui_plot::PlotMemory::load(&ctx, first).is_none());
    assert!(
        ctx.data(|d| d.get_temp::<matrust::plot::LegendBestCache>(first.with("legend_best")))
            .is_none()
    );
    assert!(egui_plot::PlotMemory::load(&ctx, second).is_some());
    assert!(app.all_figures[&1].same_snapshot(&source));
}

#[test]
fn detached_window_and_mailbox_admission_are_bounded_and_source_aliases_survive() {
    let (mut app, ctx, _dir) = setup();
    app.run("for k=3:16;figure(k);plot([k k+1]);end;".into());
    finish(&mut app);
    let before = app.all_figures.clone();
    for id in 1..=16 {
        app.undock_figure(id, &ctx);
    }
    assert_eq!(app.detached.windows.len(), MAX_WINDOWS);
    let first = app.detached.windows[&1].lock().unwrap().viewport;
    app.undock_figure(1, &ctx);
    assert_eq!(app.detached.windows.len(), MAX_WINDOWS);
    assert_eq!(first, app.detached.windows[&1].lock().unwrap().viewport);
    for _ in 0..1000 {
        app.dock_figure(1, &ctx);
    }
    assert!(
        app.detached.windows[&1]
            .lock()
            .unwrap()
            .mailbox
            .command
            .is_some()
    );
    assert_eq!(app.detached.windows.len(), MAX_WINDOWS);
    app.detached.cancel_pending();
    assert!(
        app.detached
            .windows
            .values()
            .all(|c| c.lock().unwrap().mailbox.command.is_none())
    );
    for (id, figure) in before {
        assert!(figure.same_snapshot(&app.all_figures[&id]));
    }
}

#[test]
fn detached_prepared_subplot_selection_avoids_ui_validation_and_worker_rejects_invalid_candidate() {
    let (mut app, ctx, _dir) = setup();
    app.run("figure(1);subplot(1,2,1);stem([1 2],[3 4]);title('Left');legend('Samples','Location','best');subplot(1,2,2);plot([1 2],[4 5]);title('Right');".into());
    finish(&mut app);
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let original = app.all_figures[&1].clone();
    let variables = app.variables.clone();
    for _ in 0..3 {
        child_frame(&mut app, &ctx, 1, false, false, vec![]);
    }
    subplot_view::take_ui_edit_validations();
    click(&mut app, &ctx, Some(1), "Axes 1");
    settle(&mut app, &ctx);
    assert_eq!(
        subplot_view::take_ui_edit_validations(),
        0,
        "Prepared edits must not call full UI-side model validation"
    );
    let selected = app.all_figures[&1].clone();
    assert_eq!(selected.title, "Left");
    assert_ne!(selected.active_position(), original.active_position());
    for (position, axes) in original.all_axes() {
        let (_, current) = selected.all_axes().find(|(p, _)| *p == position).unwrap();
        assert!(axes.same_snapshot(current));
    }
    let mut invalid = selected.clone();
    invalid.xlim = Some([1., 1.]);
    app.submit(Job::ViewFigure(Box::new(worker::FigureEdit {
        id: 1,
        original: selected.clone(),
        updated: invalid,
        view_before: None,
    })));
    finish(&mut app);
    assert!(app.all_figures[&1].same_snapshot(&selected));
    assert!(
        app.console
            .iter()
            .any(|line| line == "Error: Axis limits must be finite and increasing")
    );
    for (name, value) in variables {
        assert!(value.same_snapshot(&app.variables[&name]));
    }
}

// A fresh Context has no coalesced repaint request, so its actual backend wake
// callback must run. Never reacquire the Context from inside that callback.
fn assert_control_wakes_without_child_lock(
    child: &Arc<Mutex<Child>>,
    action: impl FnOnce(&egui::Context),
) {
    let ctx = egui::Context::default();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let child = child.clone();
    ctx.set_request_repaint_callback(move |_| {
        assert!(
            child.try_lock().is_ok(),
            "Backend wake ran while child mutex was held"
        );
        observed.fetch_add(1, Ordering::Relaxed);
    });
    action(&ctx);
    assert!(
        calls.load(Ordering::Relaxed) > 0,
        "Actual repaint callback must be exercised"
    );
}
#[test]
fn detached_focus_dock_sync_and_retire_release_mutex_before_backend_wakes() {
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    let child = app.detached.windows[&1].clone();
    let before = app.all_figures[&1].clone();
    assert_control_wakes_without_child_lock(&child, |wake| app.focus_detached(1, wake));
    assert!(child.lock().unwrap().mailbox.focus.is_some());
    assert_control_wakes_without_child_lock(&child, |wake| app.dock_figure(1, wake));
    assert!(matches!(
        child
            .lock()
            .unwrap()
            .mailbox
            .command
            .as_ref()
            .map(|e| &e.command),
        Some(Command::Action(Action::Dock))
    ));
    let mut registry = std::mem::take(&mut app.detached);
    app.busy = true;
    assert_control_wakes_without_child_lock(&child, |wake| registry.sync(&mut app, wake));
    assert!(child.lock().unwrap().presentation.busy);
    app.busy = false;
    registry.owner = Some(child.lock().unwrap().key.clone());
    assert_control_wakes_without_child_lock(&child, |wake| registry.sync(&mut app, wake));
    assert!(registry.owner.is_none());
    assert_control_wakes_without_child_lock(&child, |wake| {
        registry.retire(1, wake);
    });
    assert!(child.lock().unwrap().retired);
    assert!(!registry.contains(1));
    assert!(app.all_figures[&1].same_snapshot(&before));
    app.detached = registry;
}

#[test]
fn discarded_close_rejects_busy_and_modal_without_delayed_close() {
    for busy in [true, false] {
        let (mut app, ctx, _dir) = setup();
        app.undock_figure(1, &ctx);
        settle(&mut app, &ctx);
        let before = app.all_figures[&1].clone();
        let (sender, requests) = std::sync::mpsc::sync_channel(1);
        let real = std::mem::replace(&mut app.worker.send, sender);
        if busy {
            let key = app.detached.windows[&1].lock().unwrap().key.clone();
            app.submit(Job::SelectExistingFigure(Select { key, ticket: 92 }));
        } else {
            app.pending_close = Some(0);
        }
        assert!(discarded_close_frame(&mut app, &ctx) >= 2);
        {
            let child = app.detached.windows[&1].lock().unwrap();
            assert!(child.mailbox.command.is_none());
            assert!(child.close_intent.is_none());
        }
        if busy {
            assert!(matches!(
                requests.try_recv().unwrap().job,
                Job::SelectExistingFigure(_)
            ));
        }
        assert!(matches!(
            requests.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        app.worker.send = real;
        app.busy = false;
        app.pending_close = None;
        app.worker.cancel.store(false, Ordering::Relaxed);
        settle(&mut app, &ctx);
        assert!(app.figure_close.is_none());
        assert!(app.detached.pending.is_none());
        assert!(before.same_snapshot(&app.all_figures[&1]));
        assert!(app.detached.contains(1));
        assert!(!app.allow_close);
    }
}

#[test]
fn retired_cached_callback_wakes_root_without_repeating_close_or_child_repaint() {
    fn input(viewport: egui::ViewportId, child: egui::ViewportId) -> egui::RawInput {
        egui::RawInput {
            viewport_id: viewport,
            viewports: [
                (egui::ViewportId::ROOT, egui::ViewportInfo::default()),
                (
                    child,
                    egui::ViewportInfo {
                        parent: Some(egui::ViewportId::ROOT),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        }
    }
    let (mut app, ctx, _dir) = setup();
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let output = frame(&mut app, &ctx, vec![]);
    let child = app.detached.windows[&1].clone();
    let viewport = child.lock().unwrap().viewport;
    let cached = output.viewport_output[&viewport]
        .viewport_ui_cb
        .as_ref()
        .unwrap()
        .clone();
    let before = app.all_figures.clone();
    let selected = app.figure.clone();
    let sequence = app.detached.sequence.load(Ordering::Relaxed);
    let retired = ctx.run_logic(&input(egui::ViewportId::ROOT, viewport), |_| {
        app.detached.retire(1, &ctx);
    });
    assert!(!app.detached.contains(1));
    assert_eq!(
        retired.viewport_commands[&viewport]
            .iter()
            .filter(|command| { matches!(command, egui::ViewportCommand::Close) })
            .count(),
        1,
        "Registry retirement owns exactly one native Close"
    );

    // Consume the one-time native command/repaint demand before invoking the
    // cached closure that a backend may still hold after registry removal.
    for _ in 0..3 {
        let _ = crate::headless::run(&ctx, input(viewport, viewport), |_| {});
    }
    for _ in 0..8 {
        for _ in 0..3 {
            let _ = ctx.run_logic(&input(egui::ViewportId::ROOT, viewport), |_| {});
            let _ = ctx.run_logic(&input(viewport, viewport), |_| {});
        }
        assert!(!ctx.has_requested_repaint_for(&egui::ViewportId::ROOT));
        assert!(!ctx.has_requested_repaint_for(&viewport));
        let output = crate::headless::run(&ctx, input(viewport, viewport), |ui| cached(ui));
        assert!(ctx.has_requested_repaint_for(&egui::ViewportId::ROOT));
        assert!(!ctx.has_requested_repaint_for(&viewport));
        assert_eq!(
            output.viewport_output[&viewport].repaint_delay,
            Duration::MAX
        );
        assert!(output.viewport_output.values().all(|out| {
            out.commands
                .iter()
                .all(|command| !matches!(command, egui::ViewportCommand::Close))
        }));
        let state = child.lock().unwrap();
        assert!(state.retired);
        assert!(
            state.mailbox.command.is_none()
                && state.mailbox.view.is_none()
                && state.mailbox.focus.is_none()
                && state.mailbox.native.is_none()
        );
        assert!(!state.mailbox.stop && !state.mailbox.unavailable);
    }
    assert_eq!(app.detached.sequence.load(Ordering::Relaxed), sequence);
    assert_eq!(app.all_figures.len(), before.len());
    for (number, figure) in before {
        assert!(app.all_figures[&number].same_snapshot(&figure));
    }
    assert!(
        app.figure
            .as_ref()
            .unwrap()
            .same_snapshot(selected.as_ref().unwrap())
    );
    assert!(!app.busy);
}

fn titled_child_at(
    app: &mut Workspace,
    ctx: &egui::Context,
    id: u32,
    size: egui::Vec2,
) -> egui::FullOutput {
    let registered = frame(app, ctx, vec![]);
    let viewport = app.detached.windows[&id].lock().unwrap().viewport;
    let callback = registered.viewport_output[&viewport]
        .viewport_ui_cb
        .as_ref()
        .unwrap()
        .clone();
    let mut input = egui::RawInput {
        viewport_id: viewport,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    let info = input.viewports.entry(viewport).or_default();
    info.parent = Some(egui::ViewportId::ROOT);
    info.focused = Some(false);
    crate::headless::run(ctx, input, move |ui| callback(ui))
}

fn visible_title_count(output: &egui::FullOutput, title: &str, size: egui::Vec2) -> usize {
    fn inspect(shape: &egui::Shape, clip: egui::Rect, title: &str, viewport: egui::Rect) -> usize {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == title => {
                let ink = text.visual_bounding_rect();
                assert!(
                    clip.contains_rect(ink),
                    "{title}: {ink:?} outside clip {clip:?}"
                );
                assert!(
                    viewport.contains_rect(ink),
                    "{title}: {ink:?} outside {viewport:?}"
                );
                1
            }
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .map(|s| inspect(s, clip, title, viewport))
                .sum(),
            _ => 0,
        }
    }
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    output
        .shapes
        .iter()
        .map(|s| inspect(&s.shape, s.clip_rect, title, viewport))
        .sum()
}

#[test]
fn detached_ordinary_title_is_visible_without_changing_document_or_shared_points() {
    for theme in [
        matrust::session::DesktopTheme::Light,
        matrust::session::DesktopTheme::Dark,
    ] {
        let (mut app, ctx, _dir) = setup();
        app.layout.theme = Some(theme);
        app.apply_desktop_theme(&ctx);
        app.run("figure(1);title('Signal overview');xlabel('Time');ylabel('Amplitude');".into());
        finish(&mut app);
        app.undock_figure(1, &ctx);
        settle(&mut app, &ctx);
        let original = app.all_figures[&1].clone();
        let other = app.all_figures[&2].clone();
        let documents = app.figure_documents.clone();
        let history = app.figure_history.clone();
        let variables = app.variables.clone();
        let child = app.detached.windows[&1].clone();
        let key = child.lock().unwrap().key.clone();
        let viewport = child.lock().unwrap().viewport;
        assert!(original.layout.is_none());
        for size in [egui::vec2(900., 650.), egui::vec2(640., 480.)] {
            for _ in 0..3 {
                titled_child_at(&mut app, &ctx, 1, size);
            }
            let output = titled_child_at(&mut app, &ctx, 1, size);
            assert_eq!(visible_title_count(&output, "Signal overview", size), 1);
            let appearance = original.resolve_appearance(figure_view::host_theme(&ctx));
            let expected =
                Color32::from_rgb(appearance.text[0], appearance.text[1], appearance.text[2]);
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .map(|s| detached_title_color_count(
                        &s.shape,
                        s.clip_rect,
                        "Signal overview",
                        expected
                    ))
                    .sum::<usize>(),
                1
            );
            assert_eq!(visible_title_count(&output, "Time", size), 1);
            assert_eq!(visible_title_count(&output, "Amplitude", size), 1);
            assert!(
                egui_plot::PlotMemory::load(&ctx, egui::Id::new(("detached_figure", viewport)))
                    .is_some()
            );
            let state = child.lock().unwrap();
            assert!(state.key.same(&key));
            assert_eq!(state.viewport, viewport);
            assert!(state.presentation.figure.same_snapshot(&original));
            assert!(state.mailbox.command.is_none());
        }
        assert!(original.same_snapshot(&app.all_figures[&1]));
        assert!((original.series[0].points).same_storage(&app.all_figures[&1].series[0].points));
        assert!(other.same_snapshot(&app.all_figures[&2]));
        assert_eq!(app.figure_documents, documents);
        assert_eq!(app.figure_history, history);
        for (name, value) in variables {
            assert!(value.same_snapshot(&app.variables[&name]));
        }
    }
}

#[test]
fn detached_subplot_titles_remain_per_panel_without_duplicate_active_title() {
    let (mut app, ctx, _dir) = setup();
    app.run("figure(1);subplot(1,2,1);plot([1 2]);title('Left axes');subplot(1,2,2);plot([3 4]);title('Right axes');".into());
    finish(&mut app);
    app.undock_figure(1, &ctx);
    settle(&mut app, &ctx);
    let original = app.all_figures[&1].clone();
    assert!(original.layout.is_some());
    for size in [egui::vec2(900., 650.), egui::vec2(640., 480.)] {
        for _ in 0..3 {
            titled_child_at(&mut app, &ctx, 1, size);
        }
        let output = titled_child_at(&mut app, &ctx, 1, size);
        assert_eq!(visible_title_count(&output, "Left axes", size), 1);
        assert_eq!(visible_title_count(&output, "Right axes", size), 1);
        assert!(original.same_snapshot(&app.all_figures[&1]));
    }
}

fn detached_title_color_count(
    shape: &egui::Shape,
    clip: egui::Rect,
    label: &str,
    expected: Color32,
) -> usize {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            assert!(clip.contains_rect(text.visual_bounding_rect()));
            assert_eq!(text.opacity_factor, 1.);
            assert!(
                text.override_text_color
                    .is_none_or(|color| color == expected)
            );
            assert!(!text.galley.job.sections.is_empty());
            assert!(
                text.galley
                    .job
                    .sections
                    .iter()
                    .all(|s| s.format.color == expected)
            );
            1
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .map(|s| detached_title_color_count(s, clip, label, expected))
            .sum(),
        _ => 0,
    }
}

// Exercise the actual deferred closure with simulated native metadata. This
// proves command routing/state preservation, not Windows activation behavior.
fn restore_child_input(
    viewport: egui::ViewportId,
    minimized: bool,
    maximized: bool,
) -> egui::RawInput {
    egui::RawInput {
        viewport_id: viewport,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900., 650.),
        )),
        viewports: [
            (egui::ViewportId::ROOT, egui::ViewportInfo::default()),
            (
                viewport,
                egui::ViewportInfo {
                    parent: Some(egui::ViewportId::ROOT),
                    focused: Some(false),
                    minimized: Some(minimized),
                    maximized: Some(maximized),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    }
}

#[test]
fn explicit_show_restores_minimized_inactive_child_before_focus_and_preserves_view() {
    for maximized in [false, true] {
        let (mut app, ctx, _dir) = setup();
        assert_eq!(app.current_figure, 2);
        let bounds = [-2., 4., 0., 12.];
        app.figure_views.insert(
            1,
            figure_files::View {
                bounds,
                auto: [false; 2],
                dirty: true,
                revision: app.figure_documents[&1].revision,
            },
        );
        // Use the same deferred callback/worker while bypassing OS monitor
        // enumeration, which this headless command test cannot establish.
        app.undock_capture_figure(1, &ctx);
        settle(&mut app, &ctx);
        let original = app.all_figures.clone();
        let variables = app.variables.clone();
        let documents = app.figure_documents.clone();
        let history = app.figure_history.clone();
        let child = app.detached.windows[&1].clone();
        let viewport = child.lock().unwrap().viewport;
        let output = frame(&mut app, &ctx, vec![]);
        let callback = output.viewport_output[&viewport]
            .viewport_ui_cb
            .as_ref()
            .unwrap()
            .clone();
        let _ = crate::headless::run(&ctx, restore_child_input(viewport, true, maximized), |ui| {
            callback(ui)
        });
        assert!(child.lock().unwrap().minimized);
        assert_eq!(app.current_figure, 2);
        let mut input = restore_child_input(viewport, true, maximized);
        input.viewport_id = egui::ViewportId::ROOT;
        let output = ctx.run_logic(&input, |_| app.focus_detached(1, &ctx));
        let commands = &output.viewport_commands[&viewport];
        assert_eq!(commands.len(), 2);
        assert!(matches!(
            commands[0],
            egui::ViewportCommand::Minimized(false)
        ));
        assert!(matches!(commands[1], egui::ViewportCommand::Focus));
        assert!(
            child.lock().unwrap().minimized,
            "A requested restore is not an observed restore"
        );
        assert!(child.lock().unwrap().mailbox.focus.is_some());
        // The real shared worker must acknowledge the inactive figure identity.
        settle(&mut app, &ctx);
        assert_eq!(app.current_figure, 1);
        let output = frame(&mut app, &ctx, vec![]);
        let callback = output.viewport_output[&viewport]
            .viewport_ui_cb
            .as_ref()
            .unwrap()
            .clone();
        let _ = crate::headless::run(
            &ctx,
            restore_child_input(viewport, false, maximized),
            |ui| callback(ui),
        );
        settle(&mut app, &ctx);
        assert!(!child.lock().unwrap().minimized);
        assert_eq!(child.lock().unwrap().viewport, viewport);
        assert_eq!(app.figure_views[&1].bounds, bounds);
        assert_eq!(app.figure_views[&1].auto, [false; 2]);
        assert_eq!(app.view_bounds, Some(bounds));
        assert_eq!(app.view_auto, [false; 2]);
        assert_eq!(app.figure_documents, documents);
        assert_eq!(app.figure_history, history);
        for (&id, figure) in &original {
            assert!(figure.same_snapshot(&app.all_figures[&id]));
            for (a, b) in figure.series.iter().zip(&app.all_figures[&id].series) {
                assert!(a.points.same_storage(&b.points));
            }
        }
        for (name, value) in variables {
            assert!(value.same_snapshot(&app.variables[&name]));
        }
    }
}

#[test]
fn explicit_restore_rejects_busy_modal_and_reused_figure_identity() {
    let (mut app, ctx, _dir) = setup();
    app.undock_capture_figure(1, &ctx);
    settle(&mut app, &ctx);
    let child = app.detached.windows[&1].clone();
    let viewport = child.lock().unwrap().viewport;
    let mut input = restore_child_input(viewport, true, false);
    input.viewport_id = egui::ViewportId::ROOT;
    for blocked in [0, 1] {
        if blocked == 0 {
            app.busy = true;
        } else {
            app.pending_close = Some(0);
        }
        let output = ctx.run_logic(&input, |_| app.focus_detached(1, &ctx));
        assert!(
            output
                .viewport_commands
                .get(&viewport)
                .is_none_or(|v| v.is_empty())
        );
        assert!(child.lock().unwrap().mailbox.focus.is_none());
        app.busy = false;
        app.pending_close = None;
    }
    let old_identity = child.lock().unwrap().key.identity.clone();
    app.run("close(1);figure(1);plot([40 50]);".into());
    finish(&mut app);
    let replacement = app.all_figures[&1].clone();
    assert!(!replacement.identity.same_object(&old_identity));
    // Before the next root sync retires the old callback, the same figure number
    // must not restore its obsolete window or publish a stale focus request.
    let output = ctx.run_logic(&input, |_| app.focus_detached(1, &ctx));
    assert!(
        output
            .viewport_commands
            .get(&viewport)
            .is_none_or(|v| v.is_empty())
    );
    assert!(child.lock().unwrap().mailbox.focus.is_none());
    assert!(replacement.same_snapshot(&app.all_figures[&1]));
    settle(&mut app, &ctx);
    assert!(!app.detached.contains(1));
}
