fn request_and_finish_new_model(app: &mut Workspace, ctx: &egui::Context) {
    app.request_new_model();
    assert!(app.new_model_requested);
    frame(app, ctx, vec![]);
    assert!(!app.new_model_requested);
}

fn empty_model_ui_has(app: &mut Workspace, ctx: &egui::Context, label: &str) -> bool {
    let mut found = false;
    for _ in 0..3 {
        found = frame(app, ctx, vec![]).shapes.iter()
            .any(|shape| text_position(&shape.shape, label).is_some());
    }
    found
}

// ComboBox contents have their own scroll clip. Painted text outside that
// clip is not a valid click target, even when its galley exists in FullOutput.
fn model_palette_select_visible(app: &mut Workspace, ctx: &egui::Context, label: &str) {
    fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label =>
                Some(egui::Rect::from_min_size(text.pos, text.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
            _ => None,
        }
    }
    let size = egui::vec2(1440., 900.);
    let nodes = app.toolkits.model.nodes.len();
    for _ in 0..12 {
        let mut output = frame_at(app, ctx, size, vec![]);
        for _ in 0..2 { output = frame_at(app, ctx, size, vec![]); }
        assert!(egui::Popup::is_any_open(ctx));
        let target = output.shapes.iter().rev().find_map(|shape| {
            let rect = text_rect(&shape.shape, label)?;
            let clip = shape.clip_rect.intersect(ctx.content_rect());
            (clip.contains_rect(rect)
                && ctx.layer_id_at(rect.center()).is_some_and(|layer| layer.order == egui::Order::Foreground))
                .then_some(rect.center())
        });
        if let Some(point) = target {
            for pressed in [true, false] { pointer_frame(app, ctx, size, point, pressed); }
            assert!(!egui::Popup::is_any_open(ctx), "Palette selection closes its popup");
            assert_eq!(app.toolkits.model.nodes.len(), nodes, "Selection does not add a block");
            return;
        }
        // Find a currently visible item in the actual foreground palette, then
        // scroll there. Do not mutate add_kind or click a clipped item.
        let point = output.shapes.iter().rev().find_map(|shape| {
            for name in ["Constant", "Sine", "Step", "Gain", "Sum", "Unit Delay",
                "Discrete Filter", "SOS Filter", "Scope", "Abs", "Bias", "Saturation",
                "Product", "Unary Minus", "Switch"] {
                if let Some(rect) = text_rect(&shape.shape, name) {
                    let visible = rect.intersect(shape.clip_rect).intersect(ctx.content_rect());
                    if visible.is_positive() && ctx.layer_id_at(visible.center())
                        .is_some_and(|layer| layer.order == egui::Order::Foreground) {
                        return Some(visible.center());
                    }
                }
            }
            None
        }).expect("Visible foreground palette item for wheel input");
        frame_at(app, ctx, size, vec![
            egui::Event::PointerMoved(point),
            egui::Event::MouseWheel { phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0., -40.),
                modifiers: egui::Modifiers::NONE },
        ]);
        assert_eq!(app.toolkits.model.nodes.len(), nodes);
    }
    panic!("Palette item {label} was not reachable with bounded real scrolling");
}

#[test]
fn empty_model_guidance_disables_run_then_constant_runs_and_undo_restores_guidance() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _, _) = model_document_app(&ctx, dir.path());
    app.toolkits.discard_model();
    request_and_finish_new_model(&mut app, &ctx);
    let guidance = "Add a block to start building this model.";
    assert!(empty_model_ui_has(&mut app, &ctx, guidance));
    assert!(empty_model_ui_has(&mut app, &ctx, "No blocks yet"));
    assert!(!empty_model_ui_has(&mut app, &ctx, "Model must have 1..512 blocks"));
    assert!(app.toolkits.model.validate().is_err()); // Engine policy is unchanged.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let original = std::mem::replace(&mut app.worker.send, send);
    click_label(&mut app, &ctx, "\u{25b6} Run model");
    assert!(matches!(receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
    assert!(!app.busy && !app.variables.contains_key("sim_t"));
    app.worker.send = original;
    click_label(&mut app, &ctx, "Add block");
    assert!(matches!(app.toolkits.model.nodes[0].block, matrust::simulation::Block::Constant { .. }));
    // A source alone still needs a Scope under the unchanged execution rules.
    assert_eq!(app.toolkits.model.validate().unwrap_err(), "Add at least one Scope to capture simulation output");
    assert!(!empty_model_ui_has(&mut app, &ctx, guidance));
    let output = frame(&mut app, &ctx, vec![]);
    let dropdown = output.shapes.iter()
        .filter_map(|shape| text_position(&shape.shape, "Constant"))
        .min_by(|a,b| a.y.total_cmp(&b.y)).expect("Constant palette selection");
    for pressed in [true,false] {
        pointer_frame(&mut app, &ctx, egui::vec2(1440.,900.), dropdown, pressed);
    }
    assert!(egui::Popup::is_any_open(&ctx));
    model_palette_select_visible(&mut app, &ctx, "Scope");
    click_label(&mut app, &ctx, "Add block");
    assert!(matches!(app.toolkits.model.nodes[1].block, matrust::simulation::Block::Scope));
    click_label(&mut app, &ctx, "Input 1: not connected");
    assert!(egui::Popup::is_any_open(&ctx));
    let source_label = format!("1. {}", app.toolkits.model.nodes[0].name);
    click_label(&mut app, &ctx, &source_label);
    assert_eq!(app.toolkits.model.nodes[1].inputs, vec![0]);
    assert!(app.toolkits.model.validate().is_ok());
    click_label(&mut app, &ctx, "\u{25b6} Run model");
    finish(&mut app);
    assert_eq!(app.variables["sim_t"].len(), 1000);
    let signal = app.variables["sim_1"].as_numeric().expect("numeric Scope output");
    assert_eq!((signal.rows(), signal.cols()), (1000, 1));
    assert!(signal.data().iter().all(|&value| value == 1.));
    click_label(&mut app, &ctx, "Undo diagram");
    assert_eq!(app.toolkits.model.nodes.len(), 2);
    assert_eq!(app.toolkits.model.nodes[1].inputs, vec![usize::MAX]);
    click_label(&mut app, &ctx, "Undo diagram");
    assert_eq!(app.toolkits.model.nodes.len(), 1);
    click_label(&mut app, &ctx, "Undo diagram");
    assert!(app.toolkits.model.nodes.is_empty() && !app.toolkits.model_dirty());
    assert!(empty_model_ui_has(&mut app, &ctx, guidance));
}

#[test]
fn empty_model_guidance_does_not_hide_invalid_storage_settings_or_nonempty_graphs() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _, _) = model_document_app(&ctx, dir.path());
    app.toolkits.discard_model();
    request_and_finish_new_model(&mut app, &ctx);
    let empty = app.toolkits.model.clone();
    for fault in 0..3 {
        app.toolkits.model = empty.clone();
        match fault {
            0 => app.toolkits.model.version = 99,
            1 => app.toolkits.model.sample_time = -1.,
            _ => app.toolkits.model.nodes.push(matrust::simulation::Node {
                name: "Unconnected gain".into(),
                block: matrust::simulation::Block::Gain { gain: 1. },
                inputs: vec![usize::MAX],
            }),
        }
        let expected = app.toolkits.model.validate().unwrap_err();
        assert!(empty_model_ui_has(&mut app, &ctx, &expected), "{expected}");
        assert!(!empty_model_ui_has(&mut app, &ctx, "Add a block to start building this model."));
    }
}

#[test]
fn new_model_file_action_creates_clean_untitled_without_touching_other_documents() {
    for size in [egui::vec2(1440., 900.), egui::vec2(640., 480.)] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, script) = model_document_app(&ctx, dir.path());
        app.toolkits.discard_model();
        app.show_model_document();
        let disk = std::fs::read(&path).unwrap();
        let documents = serde_json::to_string(&app.documents).unwrap();
        let values = app.variables["keep"].clone();
        let figure = app.figure.as_ref().unwrap().clone();
        model_document_click_label_at(&mut app, &ctx, size, "Menu");
        model_document_click_label_at(&mut app, &ctx, size, "File");
        click_label_at(&mut app, &ctx, size, "New model");
        assert!(app.toolkits.model.nodes.is_empty());
        assert!(app.toolkits.model.positions.is_empty());
        assert_eq!(app.toolkits.model.version, 1);
        assert_eq!(app.toolkits.model.sample_time, 0.001);
        assert_eq!(app.toolkits.model.steps, 1000);
        assert!(app.toolkits.model_metadata().0.is_none());
        assert!(!app.toolkits.model_dirty() && !app.toolkits.model_file_modal_open());
        assert!(app.toolkits.model_open && !app.busy && !app.allow_close);
        assert_eq!(app.generic_save_target(), document_save::Pane::Model);
        assert_eq!(std::fs::read(&path).unwrap(), disk);
        assert_eq!(std::fs::read_to_string(script).unwrap(), "old=1;\n");
        assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
        assert!(values.same_snapshot(&app.variables["keep"]));
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert!(!app.variables.contains_key("sim_t"));
    }
}

#[test]
fn new_model_window_action_dirty_cancel_escape_and_discard_preserve_disk() {
    for cancel in [true, false] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, _) = model_document_app(&ctx, dir.path());
        app.show_model_document();
        let before = app.toolkits.model.clone();
        let disk = std::fs::read(&path).unwrap();
        click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "New model");
        assert!(app.toolkits.model_file_modal_open());
        assert!(app.toolkits.model.same_storage(&before));
        if cancel {
            click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Cancel");
        } else {
            frame(&mut app, &ctx, vec![]);
            script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape);
        }
        assert!(!app.toolkits.model_file_modal_open());
        assert!(app.toolkits.model.same_storage(&before));
        assert!(app.toolkits.model_dirty());
        request_and_finish_new_model(&mut app, &ctx);
        click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Discard changes");
        assert!(app.toolkits.model.nodes.is_empty() && app.toolkits.model_metadata().0.is_none());
        assert!(!app.toolkits.model_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), disk);
    }
}

#[test]
fn new_model_save_then_replace_clears_old_path_and_saves_empty_to_new_path() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, old_path, _) = model_document_app(&ctx, dir.path());
    request_and_finish_new_model(&mut app, &ctx);
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Save model");
    finish(&mut app);
    assert_eq!(matrust::simulation::Model::load(&old_path).unwrap().steps, 321);
    assert!(app.toolkits.model.nodes.is_empty() && app.toolkits.model_metadata().0.is_none());
    let old_bytes = std::fs::read(&old_path).unwrap();
    let empty_path = dir.path().join("empty.mrsim");
    app.toolkits.model.validate_storage().unwrap();
    assert!(app.toolkits.model.validate().is_err());
    app.toolkits.save_model_path(&empty_path, None);
    app.advance_model_read(); finish(&mut app);
    let empty = matrust::simulation::Model::load(&empty_path).unwrap();
    assert!(empty.nodes.is_empty());
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    assert!(app.toolkits.model_metadata().0.as_ref().is_some_and(|path|
        matrust::document::same_path(path, &empty_path)));
    assert!(!app.toolkits.model_dirty());
}

#[test]
fn new_model_failed_save_and_worker_failure_retain_current_model_for_retry() {
    for disconnected in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, _) = model_document_app(&ctx, dir.path());
        let model = app.toolkits.model.clone();
        let disk = std::fs::read(&path).unwrap();
        let bad = dir.path().join("folder.mrsim");
        std::fs::create_dir(&bad).unwrap();
        request_and_finish_new_model(&mut app, &ctx);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        drop(receive);
        let original = disconnected.then(|| std::mem::replace(&mut app.worker.send, send));
        app.toolkits.save_model_path(if disconnected { &path } else { &bad }, None);
        app.advance_model_read(); finish(&mut app);
        if let Some(original) = original { app.worker.send = original; }
        assert!(app.toolkits.model.same_storage(&model));
        assert!(app.toolkits.model_dirty() && app.toolkits.model_file_modal_open());
        assert!(!app.toolkits.model_io_pending());
        assert_eq!(std::fs::read(&path).unwrap(), disk);
        app.toolkits.save_model(); app.advance_model_read(); finish(&mut app);
        assert!(app.toolkits.model.nodes.is_empty() && app.toolkits.model_metadata().0.is_none());
        assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    }
}

#[test]
fn new_model_stop_before_dispatch_and_after_save_keeps_original_model() {
    for committed in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, _) = model_document_app(&ctx, dir.path());
        let before = app.toolkits.model.clone();
        request_and_finish_new_model(&mut app, &ctx);
        app.toolkits.save_model();
        if committed {
            app.advance_model_read();
            let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
            app.stop_work(); app.busy = false;
            app.receive_model_write(reply.model_write.unwrap());
        } else { app.stop_work(); app.advance_model_read(); }
        frame(&mut app, &ctx, vec![]);
        assert!(app.toolkits.model.same_storage(&before));
        assert!(app.toolkits.model_metadata().0.is_some());
        assert!(!app.toolkits.model_file_modal_open() && !app.busy);
        assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps,
            if committed { 321 } else { 1000 });
        assert_eq!(app.toolkits.model_dirty(), !committed);
        assert!(!app.pending_quit && !app.allow_close);
    }
}

#[test]
fn new_model_external_conflict_cancel_save_or_reload_preserves_each_disk_version() {
    for action in 0..3 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, _) = model_document_app(&ctx, dir.path());
        let mut external = app.toolkits.model.clone(); external.steps = 777;
        external.save(&path).unwrap();
        request_and_finish_new_model(&mut app, &ctx);
        app.toolkits.save_model(); app.advance_model_read(); finish(&mut app);
        assert!(app.toolkits.model_file_modal_open());
        assert_eq!(app.toolkits.model.steps, 321);
        click_label_at(&mut app, &ctx, egui::vec2(960.,650.), match action {
            1 => "Overwrite model", 2 => "Reload model", _ => "Cancel",
        });
        finish(&mut app);
        if action != 0 {
            assert!(app.toolkits.model.nodes.is_empty() && app.toolkits.model_metadata().0.is_none());
        } else {
            assert_eq!(app.toolkits.model.steps, 321);
            assert!(app.toolkits.model_metadata().0.is_some() && app.toolkits.model_dirty());
        }
        assert!(!app.toolkits.model_file_modal_open());
        assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, if action == 1 {321} else {777});
    }
}

#[test]
fn new_model_late_save_does_not_steal_script_or_figure_selection() {
    for script in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _, _) = model_document_app(&ctx, dir.path());
        request_and_finish_new_model(&mut app, &ctx);
        app.toolkits.save_model(); app.advance_model_read();
        let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
        if script { app.reveal_editor(); } else { app.show_plot_pane(); }
        let pane = app.generic_save_target();
        app.busy = false; app.receive_model_write(reply.model_write.unwrap());
        frame(&mut app, &ctx, vec![]);
        assert!(app.toolkits.model.nodes.is_empty());
        assert_eq!(app.generic_save_target(), pane);
    }
}

#[test]
fn new_model_save_retains_newer_model_edits_instead_of_replacing_them() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, _) = model_document_app(&ctx, dir.path());
    request_and_finish_new_model(&mut app, &ctx);
    app.toolkits.save_model(); app.advance_model_read();
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    app.toolkits.model.steps = 456;
    app.busy = false; app.receive_model_write(reply.model_write.unwrap());
    assert_eq!(app.toolkits.model.steps, 456);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 321);
    assert!(app.toolkits.model_dirty() && app.toolkits.model_file_modal_open());
    assert!(app.toolkits.message.contains("newer edits retained"));
    app.toolkits.cancel_new_model();
    assert!(!app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 456);
}

#[test]
fn new_model_pending_operations_and_quit_block_request_and_prompt() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, _) = model_document_app(&ctx, dir.path());
    for reason in 0..3 {
        app.busy = reason == 0;
        app.pending_close = (reason == 1).then_some(0);
        app.allow_close = reason == 2;
        app.request_new_model();
        assert!(!app.new_model_requested);
    }
    app.busy = false; app.pending_close = None; app.allow_close = false;
    request_and_finish_new_model(&mut app, &ctx);
    app.request_quit(&ctx);
    assert!(!app.allow_close && !app.pending_quit);
    app.request_new_model(); assert!(!app.new_model_requested);
    app.stop_work();
    assert!(!app.toolkits.model_file_modal_open());
    assert_eq!(app.toolkits.model.steps, 321);
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().steps, 1000);
}

#[test]
fn new_model_request_waits_for_current_field_edits_and_final_layout_pass() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _, _) = model_document_app(&ctx, dir.path());
    app.toolkits.discard_model();
    app.request_new_model();
    // Simulate the field edit performed after File menu layout in this frame.
    app.toolkits.model.steps = 222;
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.,650.))),
        ..Default::default()
    }, |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("New model must wait for settled edits"); }
        app.ui(root);
        if passes == 1 { assert!(app.new_model_requested); }
    });
    assert!(passes >= 2 && !app.new_model_requested);
    assert_eq!(app.toolkits.model.steps, 222);
    assert!(app.toolkits.model_file_modal_open());
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    app.toolkits.cancel_new_model();
}

#[test]
fn new_model_untitled_save_picker_cancel_retains_prompt_then_chosen_path_saves_old_model() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, _) = model_document_app(&ctx, dir.path());
    let (old_path, baseline) = app.toolkits.model_metadata();
    assert!(old_path.is_some());
    let model = app.toolkits.model.clone();
    app.toolkits.restore_model(model.clone(), None, baseline);
    let disk = std::fs::read(&path).unwrap();
    request_and_finish_new_model(&mut app, &ctx);
    // Exercise the real save command, substituting only the OS picker result.
    app.toolkits.model_path_choice = Some(None);
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Save model");
    assert!(!app.busy && !app.toolkits.model_io_pending());
    assert!(app.toolkits.model_file_modal_open() && app.toolkits.model_dirty());
    assert!(app.toolkits.model_metadata().0.is_none());
    assert!(app.toolkits.model.same_storage(&model));
    let chosen = dir.path().join("chosen.mrsim");
    app.toolkits.model_path_choice = Some(Some(chosen.clone()));
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Save model");
    finish(&mut app);
    assert_eq!(matrust::simulation::Model::load(&chosen).unwrap().steps,321);
    assert!(app.toolkits.model.nodes.is_empty() && app.toolkits.model_metadata().0.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), disk);
}

#[test]
fn new_model_empty_baseline_add_and_undo_has_no_old_diagram_history() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, _) = model_document_app(&ctx, dir.path());
    app.show_model_document();
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Add block");
    assert_eq!(app.toolkits.model.nodes.len(), 6);
    request_and_finish_new_model(&mut app, &ctx);
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Discard changes");
    assert!(app.toolkits.model.nodes.is_empty() && !app.toolkits.model_dirty());
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Add block");
    assert_eq!(app.toolkits.model.nodes.len(), 1);
    assert!(app.toolkits.model_dirty());
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Undo diagram");
    assert!(app.toolkits.model.nodes.is_empty() && !app.toolkits.model_dirty());
    // A second undo cannot restore the previous document's six-block diagram.
    click_label_at(&mut app, &ctx, egui::vec2(960.,650.), "Undo diagram");
    assert!(app.toolkits.model.nodes.is_empty() && app.toolkits.model_metadata().0.is_none());
    assert_eq!(matrust::simulation::Model::load(&path).unwrap().nodes.len(), 5);
}

#[test]
fn new_model_save_click_with_escape_cancels_before_submitting_write() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, _) = model_document_app(&ctx, dir.path());
    let model = app.toolkits.model.clone();
    let disk = std::fs::read(&path).unwrap();
    request_and_finish_new_model(&mut app, &ctx);
    let size = egui::vec2(960.,650.);
    let point = document_save_heading(&mut app, &ctx, size, "Save model");
    pointer_frame(&mut app, &ctx, size, point, true);
    frame_at(&mut app, &ctx, size, vec![
        egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary,
            pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert!(!app.busy && !app.toolkits.model_io_pending());
    assert!(!app.toolkits.model_file_modal_open());
    assert!(app.toolkits.model.same_storage(&model) && app.toolkits.model_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), disk);
}
