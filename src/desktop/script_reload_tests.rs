fn script_reload_app(ctx: &egui::Context, root: &std::path::Path) -> (Workspace, Vec<PathBuf>) {
    let mut app = figure_save_all_app(ctx, root, 1);
    let paths = named_dirty_scripts(&mut app, root, 2);
    app.reveal_editor();
    frame(&mut app, ctx, vec![]);
    (app, paths)
}

fn finish_explicit_script_reload(app: &mut Workspace, ctx: &egui::Context) {
    finish(app);
    // Real publication occurs only after the next editor/input layout pass.
    frame(app, ctx, vec![]);
    assert!(!app.script_io.pending());
}

#[test]
fn explicit_script_reload_file_menu_reads_external_text_without_saving_or_running() {
    for size in [egui::vec2(1440., 900.), egui::vec2(640., 480.)] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        app.documents[0].discard();
        let id = app.documents[0].id;
        let other = serde_json::to_string(&app.documents[1]).unwrap();
        let values = app.variables["keep"].clone();
        let figure = app.figure.as_ref().unwrap().clone();
        let model = serde_json::to_string(&app.toolkits.model).unwrap();
        std::fs::write(&paths[0], "external_only=42;\n").unwrap();
        click_label_at(&mut app, &ctx, size, "Menu");
        click_label_at(&mut app, &ctx, size, "File");
        click_label_at(&mut app, &ctx, size, "Reload script from disk...");
        assert!(app.script_reload.is_some() && !app.busy);
        assert_eq!(app.documents[0].text, "old=1;\n");
        click_label_at(&mut app, &ctx, size, "Reload script");
        finish_explicit_script_reload(&mut app, &ctx);
        assert_eq!(app.documents[0].id, id);
        assert_eq!(app.documents[0].text, "external_only=42;\n");
        assert!(!app.documents[0].dirty && app.script_reload.is_none());
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "external_only=42;\n");
        assert_eq!(serde_json::to_string(&app.documents[1]).unwrap(), other);
        assert_eq!(serde_json::to_string(&app.toolkits.model).unwrap(), model);
        assert!(values.same_snapshot(&app.variables["keep"]));
        assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
        assert!(!app.variables.contains_key("external_only"));
        assert!(!app.allow_close && !app.pending_quit && app.pending_close.is_none());
    }
}

#[test]
fn explicit_script_reload_file_menu_dirty_confirmation_cancel_and_undo_reset() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        let id = app.documents[0].id;
        let before = serde_json::to_string(&app.documents[0]).unwrap();
        assert!(egui::text_edit::TextEditState::load(&ctx, editor::id(id)).is_some());
        click_label_at(&mut app, &ctx, size, "Menu");
        click_label_at(&mut app, &ctx, size, "File");
        click_label_at(&mut app, &ctx, size, "Reload script from disk...");
        assert!(app.script_reload.is_some());
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert!(output.shapes.iter().any(|s| text_position(&s.shape, "Unsaved edits in this script will be discarded.").is_some()));
        click_label_at(&mut app, &ctx, size, "Cancel");
        assert!(app.script_reload.is_none());
        assert_eq!(serde_json::to_string(&app.documents[0]).unwrap(), before);
        assert!(egui::text_edit::TextEditState::load(&ctx, editor::id(id)).is_some());
        app.request_script_reload();
        app.confirm_script_reload();
        finish_explicit_script_reload(&mut app, &ctx);
        assert_eq!(app.documents[0].text, "old=1;\n");
        assert!(!app.documents[0].dirty);
        assert!(egui::text_edit::TextEditState::load(&ctx, editor::id(id)).is_none());
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "old=1;\n");
    }
}

#[test]
fn explicit_script_reload_is_disabled_for_other_documents_untitled_and_pending_actions() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = script_reload_app(&ctx, dir.path());
    app.show_plot_pane();
    assert!(!app.can_reload_script());
    app.request_script_reload();
    assert!(app.script_reload.is_none());
    app.show_model_document();
    assert!(!app.can_reload_script());
    app.reveal_editor();
    app.busy = true;
    app.request_script_reload();
    assert!(app.script_reload.is_none());
    app.busy = false;
    app.pending_close = Some(0);
    app.request_script_reload();
    assert!(app.script_reload.is_none());
    app.pending_close = None;
    app.documents[0] = Document::new();
    assert!(!app.can_reload_script());
    app.request_script_reload();
    assert!(app.script_reload.is_none());
}

#[test]
fn explicit_script_reload_rechecks_prompt_text_path_baseline_and_missing_document() {
    for change in 0..4 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        app.request_script_reload();
        match change {
            0 => { app.documents[0].text.push_str("newer=7;"); app.documents[0].changed(); }
            1 => app.documents[0].path = Some(dir.path().join("renamed.m")),
            2 => { app.documents[0].save(&paths[0]).unwrap(); }
            _ => { app.documents.remove(0); }
        }
        let before = serde_json::to_string(&app.documents).unwrap();
        let disk = std::fs::read(&paths[0]).unwrap();
        app.confirm_script_reload();
        assert!(!app.busy && !app.script_io.pending() && app.script_reload.is_none());
        assert_eq!(serde_json::to_string(&app.documents).unwrap(), before);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), disk);
    }
}

#[test]
fn explicit_script_reload_rechecks_state_at_reply_and_targets_stable_id() {
    for change in 0..4 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        std::fs::write(&paths[0], "external=8;\n").unwrap();
        let id = app.documents[0].id;
        app.request_script_reload();
        app.confirm_script_reload();
        let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
        match change {
            0 => { app.documents[0].text.push_str("late=3;"); app.documents[0].changed(); }
            1 => app.documents[0].path = Some(dir.path().join("renamed.m")),
            2 => app.documents.swap(0, 1),
            _ => { app.documents.remove(0); }
        }
        let before = serde_json::to_string(&app.documents).unwrap();
        app.busy = false;
        app.receive_script_io(reply.script_io.unwrap());
        app.finish_script_reload(&ctx);
        if change == 2 {
            assert_eq!(app.documents[1].id, id);
            assert_eq!(app.documents[1].text, "external=8;\n");
            assert_eq!(app.documents[0].text, "value=1;\n");
        } else {
            assert_eq!(serde_json::to_string(&app.documents).unwrap(), before);
        }
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "external=8;\n");
        assert!(!app.allow_close && !app.script_io.resume);
    }
}

#[test]
fn explicit_script_reload_errors_stop_and_retry_preserve_local_state() {
    for failure in 0..4 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        let before = serde_json::to_string(&app.documents[0]).unwrap();
        match failure {
            0 => std::fs::remove_file(&paths[0]).unwrap(),
            1 => std::fs::write(&paths[0], [0xff, 0xfe, 0xfd]).unwrap(),
            2 => std::fs::write(&paths[0], vec![b'x'; 1_000_001]).unwrap(),
            _ => std::fs::write(&paths[0], "late=9;").unwrap(),
        }
        app.request_script_reload();
        app.confirm_script_reload();
        if failure == 3 {
            let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
            app.stop_work();
            app.busy = false;
            app.receive_script_io(reply.script_io.unwrap());
            app.finish_script_reload(&ctx);
        } else { finish_explicit_script_reload(&mut app, &ctx); }
        assert_eq!(serde_json::to_string(&app.documents[0]).unwrap(), before);
        assert!(!app.script_io.pending() && !app.script_io.resume);
        std::fs::write(&paths[0], "retry=6;\n").unwrap();
        app.request_script_reload();
        app.confirm_script_reload();
        finish_explicit_script_reload(&mut app, &ctx);
        assert_eq!(app.documents[0].text, "retry=6;\n");
        assert!(!app.documents[0].dirty && !app.variables.contains_key("retry"));
    }
}

#[test]
fn explicit_script_reload_delayed_reply_does_not_steal_another_document() {
    for target in 0..4 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        if target == 3 {
            let image = dir.path().join("preview.png");
            std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/preview/tiff-preview/reference-1.png"), &image).unwrap();
            app.submit(Job::OpenImage(image)); finish(&mut app); frame(&mut app, &ctx, vec![]);
            app.reveal_editor();
        }
        std::fs::write(&paths[0], "fresh=7;\n").unwrap();
        app.request_script_reload(); app.confirm_script_reload();
        let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
        match target {
            0 => { app.active = 1; app.reveal_editor(); }
            1 => app.show_plot_pane(),
            2 => app.show_model_document(),
            _ => app.document_save.activate(document_save::Pane::Preview),
        }
        let active = app.active;
        let pane = app.generic_save_target();
        app.busy = false; app.receive_script_io(reply.script_io.unwrap());
        app.finish_script_reload(&ctx);
        assert_eq!(app.documents[0].text, "fresh=7;\n");
        assert_eq!(app.active, active);
        assert_eq!(app.generic_save_target(), pane);
        assert!(app.status.contains("background"));
    }
}

#[test]
fn explicit_script_reload_completion_frame_typing_retains_newer_editor_text() {
    for event in [
        egui::Event::Text("typed".into()),
        egui::Event::Paste("typed".into()),
        egui::Event::Ime(egui::ImeEvent::Commit("typed".into())),
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "typed".into(), active_range_chars: None,
        }),
    ] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        ctx.memory_mut(|m| m.request_focus(editor::id(app.documents[0].id)));
        frame(&mut app, &ctx, vec![]);
        std::fs::write(&paths[0], "disk=5;\n").unwrap();
        app.request_script_reload(); app.confirm_script_reload();
        let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        assert!(send.send(reply).is_ok());
        let original = std::mem::replace(&mut app.worker.receive, receive);
        frame(&mut app, &ctx, vec![event]);
        app.worker.receive = original;
        assert!(app.documents[0].text.contains("typed"));
        assert!(app.documents[0].text.contains("value=0;"));
        assert!(!app.documents[0].text.contains("disk=5;"));
        assert!(app.status.contains("changed during reload"));
        assert!(!app.script_io.pending());
        assert!(app.documents[0].dirty);
        assert_eq!(app.generic_save_target(), document_save::Pane::Script);
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "disk=5;\n");
    }
}

#[test]
fn explicit_script_reload_confirmation_blocks_quit_and_escape_cancels_without_io() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_reload_app(&ctx, dir.path());
    let before = serde_json::to_string(&app.documents).unwrap();
    app.request_script_reload();
    assert!(app.file_action_pending());
    app.request_quit(&ctx);
    assert!(!app.pending_quit && !app.allow_close && app.figure_close.is_none());
    frame(&mut app, &ctx, vec![]);
    script_action_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape);
    assert!(app.script_reload.is_none() && !app.busy);
    assert_eq!(serde_json::to_string(&app.documents).unwrap(), before);
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "old=1;\n");
    app.request_script_reload(); app.stop_work();
    assert!(app.script_reload.is_none() && !app.script_io.pending());
}

#[test]
fn explicit_script_reload_fingerprint_includes_private_saved_baseline() {
    for after_read in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_reload_app(&ctx, dir.path());
        app.request_script_reload();
        let reply = if after_read {
            app.confirm_script_reload();
            Some(app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap())
        } else { None };
        let id = app.documents[0].id;
        let text = app.documents[0].text.clone();
        let path = app.documents[0].path.clone();
        let dirty = app.documents[0].dirty;
        let mut saved = serde_json::to_value(&app.documents[0]).unwrap();
        assert_eq!(saved["saved"], "old=1;\n");
        saved["saved"] = serde_json::Value::String("different baseline;\n".into());
        let mut changed: Document = serde_json::from_value(saved).unwrap();
        changed.id = id;
        app.documents[0] = changed;
        assert_eq!(app.documents[0].text, text);
        assert_eq!(app.documents[0].path, path);
        assert_eq!(app.documents[0].dirty, dirty);
        let before = serde_json::to_string(&app.documents[0]).unwrap();
        if let Some(reply) = reply {
            app.busy = false;
            app.receive_script_io(reply.script_io.unwrap());
            app.finish_script_reload(&ctx);
        } else { app.confirm_script_reload(); }
        assert_eq!(serde_json::to_string(&app.documents[0]).unwrap(), before);
        assert!(app.status.contains("changed"));
        assert!(!app.script_io.pending() && !app.busy);
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "old=1;\n");
    }
}

#[test]
fn explicit_script_reload_progress_describes_read_and_preserves_write_warning() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = script_reload_app(&ctx, dir.path());
    app.request_script_reload(); app.confirm_script_reload();
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    // Hold the exact real worker result while the progress Window completes
    // its sizing pass. Keep the operation active rather than publishing before
    // first paint and testing a now-disappeared progress window.
    assert!(app.busy && app.script_io.active && app.script_io.pending());
    let read_text = "Reading from disk. Stop keeps the current editor text; no file is written.";
    assert_eq!(app.script_io.progress_text(), read_text);
    for _ in 0..3 { frame(&mut app, &ctx, vec![]); }
    let output = frame(&mut app, &ctx, vec![]);
    assert!(app.busy && app.script_io.active);
    assert!(output.shapes.iter().any(|s| text_position(&s.shape, read_text).is_some()));
    app.busy = false;
    app.receive_script_io(reply.script_io.unwrap());
    finish_explicit_script_reload(&mut app, &ctx);
    app.documents[0].text.push_str("saved=2;"); app.documents[0].changed();
    app.save_document(0, false);
    assert_eq!(app.script_io.progress_text(), "A write already in progress may finish. Stop cancels subsequent actions.");
    finish(&mut app);
}

#[test]
fn explicit_script_reload_stopping_deferred_read_preserves_editor_and_undo() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = script_reload_app(&ctx, dir.path());
    let id = app.documents[0].id;
    let before = serde_json::to_string(&app.documents[0]).unwrap();
    app.request_script_reload(); app.confirm_script_reload();
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    app.busy = false; app.receive_script_io(reply.script_io.unwrap());
    assert!(app.script_io.pending() && app.file_action_pending());
    assert_eq!(serde_json::to_string(&app.documents[0]).unwrap(), before);
    app.stop_work(); app.finish_script_reload(&ctx);
    assert!(!app.script_io.pending());
    assert_eq!(serde_json::to_string(&app.documents[0]).unwrap(), before);
    assert!(egui::text_edit::TextEditState::load(&ctx, editor::id(id)).is_some());
}

#[test]
fn explicit_script_reload_unavailable_worker_clears_pending_and_allows_retry() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = script_reload_app(&ctx, dir.path());
    let before = serde_json::to_string(&app.documents[0]).unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    drop(receive);
    let original = std::mem::replace(&mut app.worker.send, send);
    app.request_script_reload(); app.confirm_script_reload();
    app.worker.send = original;
    assert!(!app.busy && !app.script_io.pending() && app.script_reload.is_none());
    assert!(app.status.contains("worker unavailable"));
    assert_eq!(serde_json::to_string(&app.documents[0]).unwrap(), before);
    app.request_script_reload(); app.confirm_script_reload();
    finish_explicit_script_reload(&mut app, &ctx);
    assert_eq!(app.documents[0].text, "old=1;\n");
}

#[test]
fn explicit_script_reload_layout_retry_retains_reply_until_final_pass_once() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_reload_app(&ctx, dir.path());
    let before = app.documents[0].text.clone();
    std::fs::write(&paths[0], "fresh=1;\n").unwrap();
    app.request_script_reload(); app.confirm_script_reload();
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    app.busy = false; app.receive_script_io(reply.script_io.unwrap());
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960., 650.))),
        ..Default::default()
    }, |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("Reload must wait for final editor pass"); }
        app.ui(root);
        if passes == 1 {
            assert_eq!(app.documents[0].text, before);
            assert!(app.script_io.pending());
        }
    });
    assert!(passes >= 2);
    assert_eq!(app.documents[0].text, "fresh=1;\n");
    assert!(!app.script_io.pending());
    let messages = app.console.iter().filter(|line| line.contains("Reloaded script from disk")).count();
    assert_eq!(messages, 1);
}
