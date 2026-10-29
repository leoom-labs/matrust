fn model_app(ctx: &egui::Context, path: &std::path::Path) -> Workspace {
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::new()];
    app.active = 0;
    matrust::simulation::Model::default().save(path).unwrap();
    app.toolkits.request_open(path.to_owned());
    app.advance_model_read();
    finish(&mut app);
    app.toolkits.model.steps = 321;
    let external = matrust::simulation::Model {
        steps: 654,
        ..Default::default()
    };
    external.save(path).unwrap();
    app
}

#[test]
fn model_conflicts_cancel_quit_and_recheck_changed_versions_before_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.mrsim");
    let ctx = egui::Context::default();
    let mut app = model_app(&ctx, &path);
    app.request_quit(&ctx);
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open() && !app.allow_close);
    app.quit_discarded_figures.insert(9);
    click_label(&mut app, &ctx, "Cancel");
    assert!(!app.pending_quit && app.quit_discarded_figures.is_empty());
    assert_eq!(app.toolkits.model.steps, 321);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 654);
    assert!(!app.toolkits.save_model());
    app.advance_model_read();
    finish(&mut app);
    let external = matrust::simulation::Model {
        steps: 987,
        ..Default::default()
    };
    external.save(&path).unwrap();
    click_label(&mut app, &ctx, "Overwrite model");
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    assert!(app.toolkits.message.contains("changed again"));
    click_label(&mut app, &ctx, "Overwrite model");
    finish(&mut app);
    assert!(!app.toolkits.model_file_modal_open() && !app.toolkits.model_dirty());
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    assert!(app.recovery_dirty);
}

#[test]
fn model_reload_resumes_quit_and_failed_reload_keeps_the_local_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.mrsim");
    let ctx = egui::Context::default();
    let mut app = model_app(&ctx, &path);
    app.request_quit(&ctx);
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    std::fs::remove_file(&path).unwrap();
    click_label(&mut app, &ctx, "Reload model");
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert!(app.toolkits.model_file_modal_open() && !app.allow_close);
    assert_eq!(app.toolkits.model.steps, 321);
    let external = matrust::simulation::Model {
        steps: 222,
        ..Default::default()
    };
    external.save(&path).unwrap();
    click_label(&mut app, &ctx, "Reload model");
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert!(app.allow_close && !app.toolkits.model_dirty());
    assert_eq!(app.toolkits.model.steps, 222);
    assert_eq!(app.snapshot().model_baseline.unwrap().steps, 222);
}

#[test]
fn save_before_model_open_reads_current_target_and_failed_open_preserves_local_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("first.mrsim");
    let target = dir.path().join("next.mrsim");
    let ctx = egui::Context::default();
    let mut app = model_app(&ctx, &path);
    let mut next = matrust::simulation::Model {
        steps: 555,
        ..Default::default()
    };
    next.save(&target).unwrap();
    app.toolkits.request_open(target.clone());
    app.advance_model_read();
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    next.steps = 777;
    next.save(&target).unwrap();
    click_label(&mut app, &ctx, "Save model");
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    click_label(&mut app, &ctx, "Overwrite model");
    finish(&mut app);
    assert!(!app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 777);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    app.toolkits.model.steps = 888;
    app.toolkits.request_open(path.clone());
    app.advance_model_read();
    finish(&mut app);
    std::fs::write(&path, b"corrupt").unwrap();
    click_label(&mut app, &ctx, "Discard changes");
    finish(&mut app);
    assert!(app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 888);
    click_label(&mut app, &ctx, "Cancel");
    assert!(!app.toolkits.model_file_modal_open() && app.toolkits.model_dirty());
    assert_eq!(app.toolkits.model.steps, 888);
}

mod model_toolbar_tests {
    use super::*;
    use toolkits::model_files::ToolbarKind;

    const SIZES: [egui::Vec2; 2] = [egui::vec2(960., 650.), egui::vec2(1440., 900.)];
    const KINDS: [ToolbarKind; 3] = [ToolbarKind::Open, ToolbarKind::Save, ToolbarKind::SaveAs];

    fn label(kind: ToolbarKind) -> &'static str {
        match kind {
            ToolbarKind::Open => "Open\u{2026}",
            ToolbarKind::Save => "Save model",
            ToolbarKind::SaveAs => "Save as\u{2026}",
        }
    }

    fn setup(size: egui::Vec2) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = Workspace::with_context(ctx.clone(), true);
        assert!(app.change_folder(dir.path().to_owned()));
        finish_folder(&mut app);
        app.run("keep=[1 2 3]; alias=keep; plot(keep);".into());
        finish(&mut app);
        app.new_document("unsaved_script=7;");
        let model = matrust::simulation::Model { steps: 321, ..Default::default() };
        app.toolkits.restore_model(model.clone(), None, Some(model));
        app.show_model_document();
        app.model_toolbar.picker = Some(|_, _, _| None);
        for _ in 0..5 { frame_at(&mut app, &ctx, size, vec![]); }
        assert!(!app.busy && !app.file_action_pending());
        (ctx, app, dir)
    }

    fn ink(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(value) if value.galley.text() == text => {
                Some(value.galley.rect.translate(value.pos.to_vec2()))
            }
            egui::Shape::Vec(values) => values.iter().find_map(|shape| ink(shape, text)),
            _ => None,
        }
    }

    fn button(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, kind: ToolbarKind) -> egui::Pos2 {
        let output = frame_at(app, ctx, size, vec![]);
        let (window, layer) = app.toolkits.model_document_region().unwrap();
        let rect = output.shapes.iter().rev().find_map(|shape| {
            ink(&shape.shape, label(kind)).filter(|rect| {
                shape.clip_rect.contains_rect(*rect) && window.contains_rect(*rect)
                    && ctx.content_rect().contains_rect(*rect)
                    && ctx.layer_id_at(rect.center()) == Some(layer)
            })
        }).unwrap_or_else(|| panic!("Visible model toolbar {} at {size:?}", label(kind)));
        rect.center()
    }

    fn pointer(position: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![egui::Event::PointerMoved(position), egui::Event::PointerButton {
            pos: position, button: egui::PointerButton::Primary,
            pressed, modifiers: egui::Modifiers::NONE,
        }]
    }

    fn release(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, position: egui::Pos2,
               mut between: impl FnMut(&mut Workspace, &egui::Context)) {
        let calls = app.model_toolbar.calls.len();
        let mut passes = 0;
        crate::headless::run(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: pointer(position, false), ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("Model toolbar must wait for final pass"); }
            app.ui(root);
            if passes == 1 {
                assert!(app.model_toolbar.pending_for_test());
                assert_eq!(app.model_toolbar.calls.len(), calls);
                assert!(!app.toolkits.model_io_pending());
                between(app, ctx);
            }
        });
        assert!(passes >= 2);
        assert!(!app.model_toolbar.pending_for_test());
    }

    #[test]
    fn model_toolbar_all_three_cancel_discard_once_and_accept_a_fresh_gesture() {
        for size in SIZES {
            for kind in KINDS {
                let (ctx, mut app, _dir) = setup(size);
                for expected in 1..=2 {
                    let position = button(&mut app, &ctx, size, kind);
                    frame_at(&mut app, &ctx, size, pointer(position, true));
                    release(&mut app, &ctx, size, position, |_, _| {});
                    assert_eq!(app.model_toolbar.calls, vec![kind; expected]);
                    assert!(!app.busy && !app.toolkits.model_io_pending());
                }
                let position = button(&mut app, &ctx, size, kind);
                frame_at(&mut app, &ctx, size, pointer(position, true));
                let mut passes = 0;
                crate::headless::run(&ctx, egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: pointer(position, false), ..Default::default()
                }, |root| {
                    passes += 1;
                    app.ui(root);
                    if passes == 1 {
                        assert_eq!(app.model_toolbar.calls.len(), 3);
                        root.ctx().request_discard("Consumed model Cancel must not replay");
                    }
                });
                assert!(passes >= 2);
                assert_eq!(app.model_toolbar.calls, vec![kind; 3]);
                assert!(!app.busy && !app.toolkits.model_io_pending());
            }
        }
    }

    fn invalidate(app: &mut Workspace, ctx: &egui::Context, case: usize) {
        match case {
            0 => app.stop_work(),
            1 => app.busy = !app.busy,
            2 => app.pending_quit = true,
            3 => app.allow_close = true,
            4 => app.recovering_session = true,
            5 => app.folder.push("late-folder"),
            6 => app.toolkits.model.steps += 1,
            7 => {
                let (path, baseline) = app.toolkits.model_metadata();
                app.toolkits.restore_model(app.toolkits.model.clone(), path, baseline);
            }
            8 => ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("late_model_owner"))),
            9 => app.toolkits.hide_model_view(),
            10 => {
                app.begin_workspace_action(false, true);
                assert!(app.workspace_tools.dialog.is_some());
            }
            11 => app.new_document("late_document=9;"),
            12 => { ctx.request_discard("Model picker lost final-pass authority"); }
            _ => unreachable!(),
        }
    }

    #[test]
    fn model_toolbar_late_and_post_picker_authority_never_submits_stale_jobs() {
        for size in SIZES {
            for kind in KINDS {
                for after in [false, true] {
                    for case in 0..(if after { 13 } else { 12 }) {
                        let (ctx, mut app, dir) = setup(size);
                        let destination = dir.path().join("sentinel.mrsim");
                        std::fs::write(&destination, b"unchanged sentinel").unwrap();
                        if after {
                            app.model_toolbar.picker = Some(|app, ctx, _| {
                                let path = app.folder.join("sentinel.mrsim");
                                let case = app.command.parse().unwrap();
                                invalidate(app, ctx, case);
                                Some(path)
                            });
                        } else {
                            app.model_toolbar.picker = Some(|_, _, _| panic!("Stale request opened a picker"));
                        }
                        let position = button(&mut app, &ctx, size, kind);
                        frame_at(&mut app, &ctx, size, pointer(position, true));
                        let (send, receive) = std::sync::mpsc::sync_channel(8);
                        let original_send = std::mem::replace(&mut app.worker.send, send);
                        if after {
                            app.command = case.to_string();
                            frame_at(&mut app, &ctx, size, pointer(position, false));
                        } else {
                            release(&mut app, &ctx, size, position, |app, ctx| invalidate(app, ctx, case));
                        }
                        assert_eq!(app.model_toolbar.calls.len(), usize::from(after), "{kind:?}/{case}/{after}");
                        assert!(!app.toolkits.model_io_pending());
                        assert!(matches!(receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
                        assert_eq!(std::fs::read(destination).unwrap(), b"unchanged sentinel");
                        app.worker.send = original_send;
                        if case == 1 { app.busy = false; }
                    }
                }
            }
        }
    }

    #[test]
    fn model_toolbar_worker_save_open_and_save_as_preserve_other_documents_and_aliases() {
        for size in SIZES {
            let (ctx, mut app, dir) = setup(size);
            let documents: Vec<_> = app.documents.iter().map(|d| (d.id, d.text.clone(), d.path.clone(), d.dirty)).collect();
            let variables = app.variables.clone();
            let figures = app.all_figures.clone();
            let saved = dir.path().join("toolbar.mrsim");
            app.model_toolbar.picker = Some(|app, _, _| Some(app.folder.join("toolbar.mrsim")));
            let position = button(&mut app, &ctx, size, ToolbarKind::Save);
            frame_at(&mut app, &ctx, size, pointer(position, true));
            app.toolkits.model.steps = 456; // Final settings belong to this release, not the earlier paint.
            release(&mut app, &ctx, size, position, |_, _| {});
            assert_eq!(app.model_toolbar.calls, [ToolbarKind::Save]);
            finish(&mut app);
            assert_eq!(matrust::simulation::Model::load(&saved).unwrap().steps, 456);
            assert!(!app.toolkits.model_dirty());
            app.toolkits.model.steps = 567;
            app.model_toolbar.picker = Some(|_, _, _| panic!("Existing-path Save must not choose a path"));
            let position = button(&mut app, &ctx, size, ToolbarKind::Save);
            frame_at(&mut app, &ctx, size, pointer(position, true));
            release(&mut app, &ctx, size, position, |_, _| {});
            finish(&mut app);
            assert_eq!(matrust::simulation::Model::load(&saved).unwrap().steps, 567);
            assert_eq!(app.model_toolbar.calls.len(), 1);
            app.model_toolbar.picker = Some(|app, _, _| Some(app.folder.join("copy.mrsim")));
            let position = button(&mut app, &ctx, size, ToolbarKind::SaveAs);
            frame_at(&mut app, &ctx, size, pointer(position, true));
            release(&mut app, &ctx, size, position, |_, _| {});
            finish(&mut app);
            assert_eq!(matrust::simulation::Model::load(&dir.path().join("copy.mrsim")).unwrap().steps, 567);
            let target = matrust::simulation::Model { steps: 789, ..Default::default() };
            target.save(&dir.path().join("target.mrsim")).unwrap();
            app.model_toolbar.picker = Some(|app, _, _| Some(app.folder.join("target.mrsim")));
            app.toolkits.model.steps = 678;
            let position = button(&mut app, &ctx, size, ToolbarKind::Open);
            frame_at(&mut app, &ctx, size, pointer(position, true));
            release(&mut app, &ctx, size, position, |_, _| {});
            finish(&mut app);
            assert_eq!(app.toolkits.model.steps, 678);
            assert!(app.toolkits.model_file_modal_open());
            model_document_click_label_at(&mut app, &ctx, size, "Discard changes");
            finish(&mut app);
            assert_eq!(app.toolkits.model.steps, 789);
            assert!(!app.toolkits.model_dirty());
            assert_eq!(matrust::simulation::Model::load(&saved).unwrap().steps, 567);
            assert_eq!(documents, app.documents.iter().map(|d| (d.id, d.text.clone(), d.path.clone(), d.dirty)).collect::<Vec<_>>());
            for (name, value) in variables { assert!(value.same_snapshot(&app.variables[&name])); }
            assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
            for (id, figure) in figures { assert!(figure.same_snapshot(&app.all_figures[&id])); }
        }
    }

    #[test]
    fn model_toolbar_all_routes_park_behind_unrelated_worker_and_stop_can_retire_them() {
        for size in SIZES {
            for kind in KINDS {
                for stop in [false, true] {
                    let (ctx, mut app, dir) = setup(size);
                    let path = dir.path().join("queued.mrsim");
                    if kind == ToolbarKind::Open {
                        matrust::simulation::Model { steps: 901, ..Default::default() }.save(&path).unwrap();
                    }
                    app.model_toolbar.picker = Some(|app, _, _| Some(app.folder.join("queued.mrsim")));
                    let position = button(&mut app, &ctx, size, kind);
                    frame_at(&mut app, &ctx, size, pointer(position, true));
                    let (send, receive) = std::sync::mpsc::sync_channel(8);
                    let original_send = std::mem::replace(&mut app.worker.send, send);
                    app.run("unrelated=42;".into());
                    let held = receive.recv_timeout(Duration::from_secs(1)).unwrap();
                    assert!(app.busy && !app.worker.opening);
                    release(&mut app, &ctx, size, position, |_, _| {});
                    assert_eq!(app.model_toolbar.calls, [kind]);
                    assert!(app.toolkits.model_io_pending());
                    assert_eq!(app.toolkits.model_read_pending(), kind == ToolbarKind::Open);
                    assert!(matches!(receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
                    if stop {
                        app.stop_work();
                        assert!(!app.toolkits.model_io_pending());
                    }
                    app.worker.send = original_send;
                    assert!(app.worker.send.send(held).is_ok());
                    finish(&mut app);
                    app.advance_model_read();
                    finish(&mut app);
                    if stop {
                        assert_eq!(app.toolkits.model.steps, 321);
                        if kind != ToolbarKind::Open { assert!(!path.exists()); }
                    } else if kind == ToolbarKind::Open {
                        assert_eq!(app.toolkits.model.steps, 901);
                    } else {
                        assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
                    }
                }
            }
        }
    }

    #[test]
    fn model_toolbar_bounds_report_and_invalid_math_preserve_worker_validation() {
        for size in SIZES {
            let (ctx, mut app, dir) = setup(size);
            app.toolkits.model.nodes[0].name = "x".repeat(513);
            let position = button(&mut app, &ctx, size, ToolbarKind::Save);
            frame_at(&mut app, &ctx, size, pointer(position, true));
            release(&mut app, &ctx, size, position, |_, _| {});
            assert!(app.toolkits.message.contains("inspection bounds"));
            assert!(app.model_toolbar.calls.is_empty() && !app.toolkits.model_io_pending());
            app.toolkits.model.nodes[0].name = "Signal".into();
            app.toolkits.model.sample_time = f64::NAN;
            app.model_toolbar.picker = Some(|app, _, _| Some(app.folder.join("invalid.mrsim")));
            let position = button(&mut app, &ctx, size, ToolbarKind::Save);
            frame_at(&mut app, &ctx, size, pointer(position, true));
            release(&mut app, &ctx, size, position, |_, _| {});
            assert_eq!(app.model_toolbar.calls, [ToolbarKind::Save]);
            assert!(app.busy, "Existing worker validates invalid-but-editable model");
            finish(&mut app);
            assert!(!dir.path().join("invalid.mrsim").exists());
            // Painting the settings DragValue clamps NaN to +inf (egui orders NaN
            // above +inf); the value stays invalid and the save stays rejected.
            assert!(!app.toolkits.model.sample_time.is_finite());
            assert!(app.toolkits.model_dirty());
        }
    }
}
