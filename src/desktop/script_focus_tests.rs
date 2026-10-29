fn script_focus_input(focused: Option<bool>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960., 650.))),
        viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo {
            focused,
            ..Default::default()
        })].into_iter().collect(),
        ..Default::default()
    }
}

fn script_focus_frame(app: &mut Workspace, ctx: &egui::Context, focused: Option<bool>) {
    let _ = crate::headless::run(ctx, script_focus_input(focused), |root| app.ui(root));
}

fn script_focus_app(ctx: &egui::Context, root: &std::path::Path) -> (Workspace, Vec<PathBuf>) {
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("keep=[2 4 6];alias=keep;plot(keep);".into());
    finish(&mut app);
    let paths = named_dirty_scripts(&mut app, root, 2);
    app.reveal_editor();
    script_focus_frame(&mut app, ctx, None);
    (app, paths)
}

fn script_focus_regain(app: &mut Workspace, ctx: &egui::Context) {
    script_focus_frame(app, ctx, Some(false));
    script_focus_frame(app, ctx, Some(true));
}

fn script_focus_reply(app: &mut Workspace) -> script_io::Finished {
    assert!(app.busy && app.script_io.active);
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(3)).unwrap();
    let script = reply.script_io.expect("Real script I/O worker reply");
    assert!(script.focus.is_some());
    script
}

fn script_focus_publish(app: &mut Workspace, ctx: &egui::Context, reply: script_io::Finished) {
    app.busy = false;
    app.receive_script_io(reply);
    script_focus_frame(app, ctx, Some(true));
}

#[test]
fn script_focus_startup_unknown_and_unchanged_are_quiet_and_do_not_run() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = script_focus_app(&ctx, dir.path());
    let documents = serde_json::to_string(&app.documents).unwrap();
    let data = app.variables["keep"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    let status = app.status.clone();
    let console = app.console.clone();
    for focused in [None, Some(true), Some(true)] {
        script_focus_frame(&mut app, &ctx, focused);
        assert!(!app.busy && !app.script_io.pending() && app.script_conflict.is_none());
    }
    script_focus_frame(&mut app, &ctx, Some(false));
    script_focus_frame(&mut app, &ctx, None);
    assert!(!app.busy && !app.script_io.pending());
    script_focus_frame(&mut app, &ctx, Some(true));
    let reply = script_focus_reply(&mut app);
    assert!(matches!(&reply.result, Ok(script_io::Outcome::Unchanged)));
    script_focus_publish(&mut app, &ctx, reply);
    for _ in 0..4 { script_focus_frame(&mut app, &ctx, Some(true)); }
    assert_eq!(app.status, status);
    assert_eq!(app.console, console);
    assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
    assert!(data.same_snapshot(&app.variables["keep"]));
    assert!(data.same_snapshot(&app.variables["alias"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    assert!(!app.variables.contains_key("old") && !app.variables.contains_key("value"));
    assert!(matches!(app.worker.receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
}

#[test]
fn script_focus_dirty_change_requires_explicit_keep_or_reload_and_preserves_other_work() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    let documents = serde_json::to_string(&app.documents).unwrap();
    let other = serde_json::to_string(&app.documents[1]).unwrap();
    let data = app.variables["keep"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    std::fs::write(&paths[0], "external_only=42;\n").unwrap();
    script_focus_regain(&mut app, &ctx);
    let reply = script_focus_reply(&mut app);
    script_focus_publish(&mut app, &ctx, reply);
    assert!(app.script_conflict.as_ref().unwrap().focus.is_some());
    assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
    click_label(&mut app, &ctx, "Keep editor text");
    assert!(app.script_conflict.is_none() && !app.busy);
    assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "external_only=42;\n");
    script_focus_regain(&mut app, &ctx);
    let reply = script_focus_reply(&mut app);
    script_focus_publish(&mut app, &ctx, reply);
    click_label(&mut app, &ctx, "Reload from disk");
    finish_explicit_script_reload(&mut app, &ctx);
    assert_eq!(app.documents[0].text, "external_only=42;\n");
    assert!(!app.documents[0].dirty && app.script_conflict.is_none());
    assert_eq!(app.active, 0);
    assert_eq!(serde_json::to_string(&app.documents[1]).unwrap(), other);
    assert!(data.same_snapshot(&app.variables["keep"]));
    assert!(data.same_snapshot(&app.variables["alias"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    assert!(!app.variables.contains_key("external_only"));
}

#[test]
fn script_focus_deleted_file_offers_keep_and_save_as_without_recreating_it() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    let documents = serde_json::to_string(&app.documents).unwrap();
    std::fs::remove_file(&paths[0]).unwrap();
    script_focus_regain(&mut app, &ctx);
    let reply = script_focus_reply(&mut app);
    assert!(matches!(&reply.result, Ok(script_io::Outcome::Conflict(_, matrust::document::DiskVersion::Missing))));
    script_focus_publish(&mut app, &ctx, reply);
    // The Modal's first frame only measures its content (egui sizing pass).
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    assert!(output.shapes.iter().any(|s| text_position(&s.shape, "Save as...").is_some()));
    click_label(&mut app, &ctx, "Reload from disk");
    assert!(!app.busy && app.script_conflict.is_some());
    click_label(&mut app, &ctx, "Keep editor text");
    assert!(!paths[0].exists());
    assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
}

#[test]
fn script_focus_real_worker_reply_rejects_edits_paths_closed_tabs_switches_and_stop() {
    for change in 0..6 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_focus_app(&ctx, dir.path());
        std::fs::write(&paths[0], "external=9;\n").unwrap();
        script_focus_regain(&mut app, &ctx);
        let reply = script_focus_reply(&mut app);
        match change {
            0 => { app.documents[0].text.push_str("new_edit=3;"); app.documents[0].changed(); }
            1 => app.documents[0].path = Some(dir.path().join("renamed.m")),
            2 => { app.documents.remove(0); }
            3 => app.active = 1,
            4 => {
                app.document_save.activate(document_save::Pane::Figure);
                app.document_save.activate(document_save::Pane::Script);
            }
            _ => app.stop_work(),
        }
        let active = app.active;
        let documents = serde_json::to_string(&app.documents).unwrap();
        script_focus_publish(&mut app, &ctx, reply);
        assert!(app.script_conflict.is_none() && !app.busy);
        assert_eq!(app.active, active);
        assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "external=9;\n");
    }
}

#[test]
fn script_focus_coalesces_regains_while_worker_busy_and_rejects_superseded_reply() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    std::fs::write(&paths[0], "external=9;\n").unwrap();
    script_focus_regain(&mut app, &ctx);
    let old = script_focus_reply(&mut app); // Hold the actual completed reply; UI still busy.
    for _ in 0..3 { script_focus_regain(&mut app, &ctx); }
    assert!(matches!(app.worker.receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
    script_focus_publish(&mut app, &ctx, old);
    assert!(app.script_conflict.is_none());
    let latest = script_focus_reply(&mut app);
    script_focus_publish(&mut app, &ctx, latest);
    assert!(app.script_conflict.is_some());
    assert!(matches!(app.worker.receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
}

#[test]
fn script_focus_discarded_passes_admit_and_publish_once_after_final_input() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    std::fs::write(&paths[0], "external=9;\n").unwrap();
    script_focus_frame(&mut app, &ctx, Some(false));
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, script_focus_input(Some(true)), |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("Focus request waits for final input"); }
        app.ui(root);
        if passes == 1 { assert!(!app.busy && !app.script_io.pending()); }
    });
    assert!(passes >= 2);
    let reply = script_focus_reply(&mut app);
    app.busy = false;
    app.receive_script_io(reply);
    passes = 0;
    let _ = crate::headless::run(&ctx, script_focus_input(Some(true)), |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("Focus publication waits for editor input"); }
        app.ui(root);
        if passes == 1 {
            assert!(app.script_conflict.is_none());
            app.documents[0].text.push_str("typed_in_completion_frame=1;");
            app.documents[0].changed();
        }
    });
    assert!(passes >= 2 && app.script_conflict.is_none() && !app.busy);
    assert!(app.documents[0].text.contains("typed_in_completion_frame"));
    assert!(matches!(app.worker.receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
}

#[test]
fn script_focus_pending_checks_defer_to_operations_and_stop_clears_them() {
    for block in 0..6 {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_focus_app(&ctx, dir.path());
        std::fs::write(&paths[0], "external=9;\n").unwrap();
        match block {
            0 => app.busy = true,
            1 => app.recovering_session = true,
            2 => app.pending_quit = true,
            3 => app.open_queue.push_back(paths[1].clone()),
            4 => app.request_script_reload(),
            _ => app.allow_close = true,
        }
        // Exercise the real focus input/admission phase without advancing the
        // blocking operation itself (e.g. draining the queued file).
        for focused in [false, true, false, true] {
            let _ = crate::headless::run(&ctx, script_focus_input(Some(focused)), |root| {
                app.observe_script_focus(root.ctx());
                app.finish_script_focus(root.ctx());
            });
        }
        assert!(!app.script_io.pending());
        app.stop_work();
        app.busy = false;
        app.recovering_session = false;
        app.pending_quit = false;
        app.allow_close = false;
        script_focus_frame(&mut app, &ctx, Some(true));
        assert!(!app.busy && app.script_conflict.is_none());
        script_focus_regain(&mut app, &ctx);
        let reply = script_focus_reply(&mut app);
        script_focus_publish(&mut app, &ctx, reply);
        assert!(app.script_conflict.is_some());
    }
}

#[test]
fn script_focus_clean_change_and_invalid_disk_reads_keep_editor_and_saved_baseline() {
    for bytes in [b"changed_clean=7;\n".to_vec(), vec![0xff, 0xfe], vec![b'x'; 1_000_001]] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, paths) = script_focus_app(&ctx, dir.path());
        app.documents[0].discard();
        let documents = serde_json::to_string(&app.documents).unwrap();
        std::fs::write(&paths[0], &bytes).unwrap();
        script_focus_regain(&mut app, &ctx);
        let reply = script_focus_reply(&mut app);
        let valid = bytes.len() < 1_000_000 && bytes[0] != 0xff;
        assert_eq!(reply.result.is_ok(), valid);
        script_focus_publish(&mut app, &ctx, reply);
        assert_eq!(app.script_conflict.is_some(), valid);
        assert_eq!(serde_json::to_string(&app.documents).unwrap(), documents);
        assert!(!app.documents[0].dirty);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), bytes);
        assert!(!app.variables.contains_key("changed_clean"));
    }
}

#[test]
fn script_focus_actual_modal_defers_check_until_user_finishes_confirmation() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    std::fs::write(&paths[0], "external=9;\n").unwrap();
    app.request_script_reload();
    assert!(app.script_reload.is_some());
    script_focus_regain(&mut app, &ctx);
    assert!(!app.busy && app.script_conflict.is_none());
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.script_reload.is_none());
    // The named script has not changed; the original coalesced intent resumes.
    for _ in 0..3 {
        if app.busy { break; }
        script_focus_frame(&mut app, &ctx, Some(true));
    }
    let reply = script_focus_reply(&mut app);
    script_focus_publish(&mut app, &ctx, reply);
    assert!(app.script_conflict.is_some());
    assert_eq!(app.documents[0].text, "value=0;\n");
}

#[test]
fn script_focus_explicit_file_action_supersedes_a_ready_automatic_reply() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    std::fs::write(&paths[0], "external=9;\n").unwrap();
    script_focus_regain(&mut app, &ctx);
    let reply = script_focus_reply(&mut app);
    app.busy = false;
    app.receive_script_io(reply);
    // An explicit save of the other document wins before automatic publication.
    std::fs::write(&paths[1], "other_external=2;\n").unwrap();
    app.save_script_path(1, &paths[1], None);
    finish(&mut app);
    // This is the existing manual save conflict, never the older focus target.
    let conflict = app.script_conflict.as_ref().unwrap();
    assert_eq!(conflict.document, app.documents[1].id);
    assert!(conflict.focus.is_none());
    click_label(&mut app, &ctx, "Cancel");
    for _ in 0..3 { script_focus_frame(&mut app, &ctx, Some(true)); }
    assert!(app.script_conflict.is_none() && !app.busy);
    assert_eq!(app.documents[0].text, "value=0;\n");
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "external=9;\n");
}

#[test]
fn script_focus_native_input_hook_retains_hidden_transition_until_visible_final_pass() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, paths) = script_focus_app(&ctx, dir.path());
    std::fs::write(&paths[0], "external=9;\n").unwrap();
    // This is the actual eframe hook, also called by update_logic_only for an
    // occluded/minimized window. No UI frame or file request runs in these ticks.
    for focused in [None, Some(true), Some(false), None, Some(true)] {
        let mut input = script_focus_input(focused);
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert!(!app.busy && !app.script_io.pending() && app.script_conflict.is_none());
    }
    assert!(matches!(app.worker.receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, script_focus_input(Some(true)), |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("Hidden focus edge awaits visible final input"); }
        app.ui(root);
        if passes == 1 { assert!(!app.busy && !app.script_io.pending()); }
    });
    assert!(passes >= 2);
    let reply = script_focus_reply(&mut app);
    script_focus_publish(&mut app, &ctx, reply);
    assert!(app.script_conflict.is_some());
    assert_eq!(app.documents[0].text, "value=0;\n");
    assert!(matches!(app.worker.receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
}
