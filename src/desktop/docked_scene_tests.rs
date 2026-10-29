use super::*;
use std::time::{Duration, Instant};

#[test]
fn keyed_style_actions_keep_log_manual_view_through_prepare_save_and_undo() {
    for action in [Action::Grid, Action::Legend, Action::Hold] {
        for mixed_auto in [false, true] {
            let (mut app, ctx, dir, keys) = setup();
            // Establish every initial scene before scripts or saves cancel pending work.
            settle(&mut app, &ctx, &keys);
            assert!(keys.iter().all(|key| app.detached.docked.has_geometry(key)));
            if mixed_auto {
                // Genuine model limits give a manual log X and automatic Y.
                app.run("figure(2);xlim([2 20]);figure(1);".into());
                finish(&mut app);
            }
            // A save deliberately cancels pending display preparation. Establish
            // the actual canvas first, as the subsequent zoom requires PlotMemory.
            settle(&mut app, &ctx, &keys);
            assert!(keys.iter().all(|key| app.detached.docked.has_geometry(key)));
            let path = dir.path().join("style-log-view.mfig");
            app.submit(Job::SaveFigureById(2, path.clone(), None));
            finish(&mut app);
            let originals = app.all_figures.clone();
            settle(&mut app, &ctx, &keys);
            let ids: Vec<_> = keys
                .iter()
                .map(|key| app.detached.docked.surface_id(key).unwrap_or_else(|| panic!(
                    "style setup lost scene {} (mixed_auto={mixed_auto}, live={}, owners={:?}, figures={:?}, status={})",
                    key.number, live(&app, key),
                    app.detached.docked.scenes.iter().map(|s| s.key.number).collect::<Vec<_>>(),
                    app.all_figures.keys().collect::<Vec<_>>(), app.status,
                )))
                .collect();
            let before = *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds();
            if !mixed_auto {
                let point = egui_plot::PlotMemory::load(&ctx, ids[1])
                    .unwrap()
                    .transform()
                    .frame()
                    .center();
                frame(
                    &mut app,
                    &ctx,
                    &keys,
                    vec![egui::Event::PointerMoved(point), egui::Event::Zoom(2.)],
                    None,
                    false,
                );
            }
            let displayed = *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds();
            let view = app.figure_views[&2].bounds;
            let auto = app.figure_views[&2].auto;
            assert_eq!(auto, [false, mixed_auto]);
            assert!(view[0] > 0. && view[1] > view[0]);
            // Verify log display/data association independently of the action setter.
            for (data, coordinate) in [(view[0], displayed.min()[0]), (view[1], displayed.max()[0])]
            {
                assert!((data.log10() - coordinate).abs() < 1e-12);
            }
            if !mixed_auto {
                assert!(displayed.width() < before.width());
            }
            let mut expected_undo = originals[&2].clone();
            expected_undo.xlim = Some([view[0], view[1]]);
            if !mixed_auto {
                expected_undo.ylim = Some([view[2], view[3]]);
            }
            let mut expected_edit = expected_undo.clone();
            match action {
                Action::Grid => {
                    let enabled = !(expected_edit.xgrid && expected_edit.ygrid);
                    expected_edit.xgrid = enabled;
                    expected_edit.ygrid = enabled;
                }
                Action::Legend => expected_edit.legend = !expected_edit.legend,
                Action::Hold => expected_edit.hold = !expected_edit.hold,
                _ => unreachable!(),
            }
            frame(
                &mut app,
                &ctx,
                &keys,
                vec![],
                Some((keys[1].clone(), action)),
                true,
            );
            settle(&mut app, &ctx, &keys);
            assert_eq!(app.current_figure, 2);
            assert!(app.all_figures[&2].same_snapshot(&expected_edit));
            assert_eq!(app.figure_views[&2].auto, auto);
            assert_eq!(app.figure_views[&2].bounds[0..2], view[0..2]);
            if mixed_auto {
                assert!(app.all_figures[&2].ylim.is_none());
            }
            for (index, key) in keys.iter().enumerate() {
                assert_eq!(app.detached.docked.surface_id(key), Some(ids[index]));
                if key.number != 2 {
                    assert!(app.all_figures[&key.number].same_snapshot(&originals[&key.number]));
                }
            }
            frame(
                &mut app,
                &ctx,
                &keys,
                vec![],
                Some((keys[1].clone(), Action::Save)),
                false,
            );
            settle(&mut app, &ctx, &keys);
            let saved = matrust::files::load_figure(&path).unwrap();
            assert_eq!(saved.xlim, Some([view[0], view[1]]));
            if !mixed_auto {
                assert_eq!(saved.ylim, Some([view[2], view[3]]));
            }
            assert_eq!(saved.xgrid, expected_edit.xgrid);
            assert_eq!(saved.legend, expected_edit.legend);
            assert_eq!(saved.hold, expected_edit.hold);
            frame(
                &mut app,
                &ctx,
                &keys,
                vec![],
                Some((keys[1].clone(), Action::Undo)),
                false,
            );
            settle(&mut app, &ctx, &keys);
            assert!(app.all_figures[&2].same_snapshot(&expected_undo));
            assert_eq!(app.figure_views[&2].auto, auto);
            assert_eq!(app.figure_views[&2].bounds[0..2], view[0..2]);
            for (index, key) in keys.iter().enumerate() {
                assert_eq!(app.detached.docked.surface_id(key), Some(ids[index]));
                let current = &app.all_figures[&key.number];
                let original = &originals[&key.number];
                if key.number != 2 {
                    assert!(current.same_snapshot(original));
                }
                for ((_, a), (_, b)) in current.all_axes().zip(original.all_axes()) {
                    for (a, b) in a.series.iter().zip(&b.series) {
                        assert!(a.points.same_storage(&b.points));
                    }
                }
            }
            assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
            assert!(
                root_cache_bytes(&app) + app.detached.retained_bytes()
                    <= figure_geometry::CACHE_BYTES
            );
        }
    }
}

fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn setup() -> (Workspace, egui::Context, tempfile::TempDir, Vec<Key>) {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.run("keep=1:12;alias=keep;figure(1);plot([1 2 4]);figure(2);semilogx([1 10 100],[3 7 2]);figure(3);subplot(1,2,1);plot([2 4]);subplot(1,2,2);plot([3 1]);figure(4);plot([9 2 6]);figure(1);".into());
    finish(&mut app);
    let keys: Vec<_> = (1..=4)
        .map(|number| Key {
            number,
            identity: app.all_figures[&number].identity.clone(),
        })
        .collect();
    let mut registry = std::mem::take(&mut app.detached);
    registry
        .docked
        .configure(&app, &ctx, &keys, &registry.windows)
        .unwrap();
    app.detached = registry;
    (app, ctx, dir, keys)
}
fn frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    keys: &[Key],
    events: Vec<egui::Event>,
    action: Option<(Key, Action)>,
    discard: bool,
) -> egui::FullOutput {
    let mut action = action;
    let mut pass = 0;
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600., 1000.),
            )),
            events,
            ..Default::default()
        },
        |root| {
            let mut registry = std::mem::take(&mut app.detached);
            registry.sync(app, ctx);
            egui::CentralPanel::default().show(root, |ui| {
                let area = ui.available_rect_before_wrap();
                for (slot, key) in keys.iter().enumerate() {
                    let size = egui::vec2((area.width() - 16.) / 2., (area.height() - 16.) / 2.);
                    let rect = egui::Rect::from_min_size(
                        area.min
                            + egui::vec2(
                                (slot % 2) as f32 * (size.x + 16.),
                                (slot / 2) as f32 * (size.y + 16.),
                            ),
                        size,
                    );
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    child.set_clip_rect(rect.intersect(ui.clip_rect()));
                    registry
                        .docked
                        .paint(&mut child, app, key, &registry.sequence)
                        .unwrap();
                }
            });
            if let Some((key, command)) = action.take() {
                assert!(
                    registry
                        .docked
                        .action(app, ctx, &key, command, &registry.sequence)
                );
            }
            if discard && pass == 0 {
                ctx.request_discard("retain keyed scene action on final pass");
            }
            pass += 1;
            registry.advance(app, ctx);
            app.detached = registry;
        },
    )
}
fn settle(app: &mut Workspace, ctx: &egui::Context, keys: &[Key]) {
    for _ in 0..12 {
        finish(app);
        frame(app, ctx, keys, vec![], None, false);
    }
    finish(app);
}

#[test]
fn four_real_worker_scenes_keep_identity_across_reorder_and_inactive_zoom() {
    for dark in [false, true] {
        let (mut app, ctx, _dir, keys) = setup();
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let before = app.all_figures.clone();
        let history = app.figure_history.clone();
        settle(&mut app, &ctx, &keys);
        assert_eq!(ctx.global_style().visuals.dark_mode, dark);
        assert_eq!(app.current_figure, 1);
        assert!(
            app.detached
                .docked
                .scenes
                .iter()
                .all(|s| s.geometry.is_some())
        );
        assert!(
            root_cache_bytes(&app) + app.detached.retained_bytes() <= figure_geometry::CACHE_BYTES
        );
        let ids: Vec<_> = app
            .detached
            .docked
            .scenes
            .iter()
            .map(|s| s.identity.plot_id())
            .collect();
        let first = *egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap().bounds();
        let second = *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds();
        frame(
            &mut app,
            &ctx,
            &keys,
            vec![
                egui::Event::PointerMoved(egui::pos2(1200., 250.)),
                egui::Event::Zoom(2.),
            ],
            None,
            false,
        );
        let zoomed = *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds();
        assert!(zoomed.width() < second.width());
        assert_eq!(
            *egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap().bounds(),
            first
        );
        assert_eq!(app.figure_views[&2].auto, [false; 2]);
        assert_eq!(app.current_figure, 1);
        let mut reversed = keys.clone();
        reversed.reverse();
        let mut registry = std::mem::take(&mut app.detached);
        registry
            .docked
            .configure(&app, &ctx, &reversed, &registry.windows)
            .unwrap();
        assert_eq!(registry.docked.scenes[2].identity.plot_id(), ids[1]);
        app.detached = registry;
        frame(&mut app, &ctx, &reversed, vec![], None, false);
        assert_eq!(
            *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds(),
            zoomed
        );
        assert_eq!(app.figure_history, history);
        for (id, original) in before {
            assert!(app.all_figures[&id].same_snapshot(&original));
            for ((_, a), (_, b)) in original.all_axes().zip(app.all_figures[&id].all_axes()) {
                for (a, b) in a.series.iter().zip(&b.series) {
                    assert!(a.points.same_storage(&b.points));
                }
            }
        }
        assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
    }
}

#[test]
fn inactive_keyed_edit_save_and_undo_use_the_existing_selection_acknowledgement() {
    let (mut app, ctx, dir, keys) = setup();
    let path = dir.path().join("second.mfig");
    app.submit(Job::SaveFigureById(2, path.clone(), None));
    finish(&mut app);
    let original = app.all_figures[&2].clone();
    let first = app.all_figures[&1].clone();
    settle(&mut app, &ctx, &keys);
    assert_eq!(app.current_figure, 1);
    frame(
        &mut app,
        &ctx,
        &keys,
        vec![],
        Some((keys[1].clone(), Action::Grid)),
        true,
    );
    // Paint and admission select explicitly through the worker; no model mutation here.
    assert!(app.all_figures[&2].same_snapshot(&original));
    settle(&mut app, &ctx, &keys);
    assert_eq!(app.current_figure, 2);
    assert_eq!(app.all_figures[&2].xgrid, !original.xgrid);
    assert_eq!(app.all_figures[&2].ygrid, !original.ygrid);
    let updated = app.all_figures[&2].clone();
    assert_eq!(app.figure_history[&2].0, 1);
    frame(
        &mut app,
        &ctx,
        &keys,
        vec![],
        Some((keys[1].clone(), Action::Save)),
        false,
    );
    settle(&mut app, &ctx, &keys);
    let mut reader = matrust::Engine::default();
    reader
        .execute(&format!(
            "openfig('{}');",
            path.to_string_lossy()
                .replace('\\', "/")
                .replace('\'', "''")
        ))
        .unwrap();
    let loaded = reader.figure.unwrap();
    assert_eq!(loaded.xgrid, updated.xgrid);
    assert_eq!(loaded.ygrid, updated.ygrid);
    assert_eq!(
        loaded.series[0].points.iter().collect::<Vec<_>>(),
        updated.series[0].points.iter().collect::<Vec<_>>()
    );
    frame(
        &mut app,
        &ctx,
        &keys,
        vec![],
        Some((keys[1].clone(), Action::Undo)),
        false,
    );
    settle(&mut app, &ctx, &keys);
    assert!(app.all_figures[&2].same_snapshot(&original));
    assert!(app.all_figures[&1].same_snapshot(&first));
    assert!(
        original.series[0]
            .points
            .same_storage(&app.all_figures[&2].series[0].points)
    );
}

#[test]
fn aggregate_reservation_counts_root_subplots_and_detached_and_does_not_requeue_evictions() {
    let (mut app, ctx, _dir, keys) = setup();
    settle(&mut app, &ctx, &keys);
    let snapshots = app.all_figures.clone();
    // Reserve root plus one real in-flight maximum. Small caches must yield;
    // no giant data fixture is needed to test actual capacity admission.
    let mut registry = std::mem::take(&mut app.detached);
    registry.preparation_in_flight = true;
    assert!(registry.reserve_root_cache(0, figure_geometry::ONE_CACHE_BYTES));
    assert_eq!(registry.retained_bytes(), 0);
    assert!(registry.docked.candidate().is_none());
    assert!(!registry.reserve_root_cache(1, figure_geometry::ONE_CACHE_BYTES));
    registry.preparation_in_flight = false;
    app.detached = registry;
    for _ in 0..3 {
        frame(&mut app, &ctx, &keys, vec![], None, false);
        assert!(!app.busy);
    }
    app.subplot_canvas =
        crate::subplot_view::Canvas::prepare(3, &app.all_figures[&3], &AtomicBool::new(false))
            .unwrap();
    let ordinary = app
        .plot_points
        .iter()
        .map(|v| v.capacity() * std::mem::size_of::<PlotPoint>())
        .sum::<usize>();
    assert!(app.subplot_canvas.retained_point_bytes() > 0);
    assert_eq!(
        root_cache_bytes(&app),
        ordinary + app.subplot_canvas.retained_point_bytes()
    );
    let mut changed = app.all_figures[&3].clone();
    changed.title = "Only the active subplot changed".into();
    assert_eq!(app.subplot_canvas.retained_points(), 4);
    app.subplot_canvas.retain_source(3, &changed);
    assert_eq!(
        app.subplot_canvas.retained_points(),
        2,
        "unchanged subplot cache must survive"
    );
    assert_eq!(
        root_cache_bytes(&app),
        ordinary + app.subplot_canvas.retained_point_bytes()
    );
    let mut registry = std::mem::take(&mut app.detached);
    registry
        .docked
        .configure(&app, &ctx, &keys[..3], &registry.windows)
        .unwrap();
    registry.create_window(&mut app, 4, &ctx, false);
    let geometry = Geometry::prepare(4, &app.all_figures[&4], &AtomicBool::new(false)).unwrap();
    let detached_bytes = geometry.retained_bytes();
    {
        let mut child = registry.windows[&4].lock().unwrap();
        child.geometry = Some(geometry);
        child.geometry_source = Some(app.all_figures[&4].clone());
    }
    assert_eq!(registry.retained_bytes(), detached_bytes);
    assert!(registry.reserve(figure_geometry::CACHE_BYTES, 0));
    assert_eq!(registry.retained_bytes(), 0);
    assert!(registry.windows[&4].lock().unwrap().geometry.is_none());
    assert!(!registry.windows[&4].lock().unwrap().prepare_requested);
    assert!(
        registry
            .docked
            .configure(&app, &ctx, &keys, &registry.windows)
            .is_err()
    );
    assert_eq!(registry.docked.scenes.len(), 3);
    for (id, source) in snapshots {
        assert!(app.all_figures[&id].same_snapshot(&source));
    }
}

#[test]
fn stop_and_reused_number_cannot_publish_a_queued_scene_action() {
    let (mut app, ctx, _dir, keys) = setup();
    settle(&mut app, &ctx, &keys);
    let original = app.all_figures[&2].clone();
    frame(
        &mut app,
        &ctx,
        &keys,
        vec![],
        Some((keys[1].clone(), Action::Grid)),
        false,
    );
    app.stop_work();
    finish(&mut app);
    settle(&mut app, &ctx, &keys);
    assert!(app.all_figures[&2].same_snapshot(&original));
    assert!(app.detached.pending.is_none());
    let old_id = app.detached.docked.scenes[1].identity.plot_id();
    app.run("close(2);figure(2);plot([99 88]);".into());
    finish(&mut app);
    let mut registry = std::mem::take(&mut app.detached);
    registry.sync(&mut app, &ctx);
    assert!(
        !registry
            .docked
            .action(&app, &ctx, &keys[1], Action::Undo, &registry.sequence)
    );
    assert!(!registry.docked.scenes.iter().any(|s| s.key.same(&keys[1])));
    assert!(egui_plot::PlotMemory::load(&ctx, old_id).is_none());
    let replacement = Key {
        number: 2,
        identity: app.all_figures[&2].identity.clone(),
    };
    registry
        .docked
        .configure(
            &app,
            &ctx,
            std::slice::from_ref(&replacement),
            &registry.windows,
        )
        .unwrap();
    assert_ne!(registry.docked.scenes[0].identity.plot_id(), old_id);
    assert!(
        registry
            .docked
            .configure(
                &app,
                &ctx,
                &[replacement.clone(), replacement],
                &registry.windows
            )
            .is_err()
    );
    assert_eq!(registry.docked.scenes.len(), 1);
}

#[test]
fn late_selection_acknowledgement_rejects_a_real_same_identity_revision_change() {
    let (mut app, ctx, _dir, keys) = setup();
    settle(&mut app, &ctx, &keys);
    let original = app.all_figures[&2].clone();
    let old_revision = app.figure_documents[&2].revision;
    // Hold only this selection request. The later model edit still uses the real
    // worker; then deliver the obsolete acknowledgement deterministically.
    let (sender, requests) = std::sync::mpsc::sync_channel(1);
    let real = std::mem::replace(&mut app.worker.send, sender);
    frame(
        &mut app,
        &ctx,
        &keys,
        vec![],
        Some((keys[1].clone(), Action::Grid)),
        false,
    );
    let Job::SelectExistingFigure(select) = requests.try_recv().unwrap().job else {
        panic!("typed selection required");
    };
    let pending = app.detached.pending.take().unwrap();
    app.worker.send = real;
    app.busy = false;
    app.run("figure(2);title('Newer source revision');".into());
    finish(&mut app);
    assert!(app.all_figures[&2].identity.same_object(&original.identity));
    assert_ne!(app.figure_documents[&2].revision, old_revision);
    let newer = app.all_figures[&2].clone();
    app.detached.pending = Some(pending);
    let mut registry = std::mem::take(&mut app.detached);
    registry.complete(
        &app,
        Some(Acknowledgement {
            key: select.key,
            ticket: select.ticket,
            accepted: true,
        }),
        None,
    );
    assert!(registry.pending.is_none());
    app.detached = registry;
    settle(&mut app, &ctx, &keys);
    assert!(app.all_figures[&2].same_snapshot(&newer));
    assert_eq!(app.all_figures[&2].xgrid, original.xgrid);
    assert_eq!(app.all_figures[&2].ygrid, original.ygrid);
    assert!(
        app.all_figures[&2].series[0]
            .points
            .same_storage(&original.series[0].points)
    );
}

#[test]
fn undocking_transfers_prepared_vectors_once_and_retires_only_the_old_surface() {
    let (mut app, ctx, _dir, keys) = setup();
    settle(&mut app, &ctx, &keys);
    let scene = &app.detached.docked.scenes[3];
    let id = scene.identity.plot_id();
    let Geometry::Ordinary(data) = scene.geometry.as_ref().unwrap() else {
        panic!("ordinary fixture");
    };
    let allocation = data.points[0].as_ptr();
    let total = app.detached.retained_bytes();
    let original = app.all_figures[&4].clone();
    let other = app.detached.docked.scenes[0].identity.plot_id();
    assert!(egui_plot::PlotMemory::load(&ctx, id).is_some());
    app.undock_figure(4, &ctx);
    assert_eq!(app.detached.docked.scenes.len(), 3);
    assert_eq!(app.detached.retained_bytes(), total);
    assert!(egui_plot::PlotMemory::load(&ctx, id).is_none());
    assert!(egui_plot::PlotMemory::load(&ctx, other).is_some());
    let child = app.detached.windows[&4].lock().unwrap();
    let Geometry::Ordinary(data) = child.geometry.as_ref().unwrap() else {
        panic!("transferred geometry");
    };
    assert_eq!(data.points[0].as_ptr(), allocation);
    assert!(!child.prepare_requested);
    assert!(
        child
            .geometry_source
            .as_ref()
            .unwrap()
            .same_snapshot(&original)
    );
    assert_eq!(app.current_figure, 1);
    assert!(app.all_figures[&4].same_snapshot(&original));
    assert!(!app.busy);
}
