#[test]
fn script_conflict_dialog_rechecks_overwrite_and_cancels_close_without_data_loss() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.m");
    std::fs::write(&path, "old = 1;\n").unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::open(&path).unwrap()];
    app.active = 0;
    app.documents[0].text = "mine = 2;\n".into();
    app.documents[0].changed();
    std::fs::write(&path, "external = 3;\n").unwrap();
    app.pending_close = Some(0);
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    assert!(app.script_conflict.is_some());
    assert!(!app.allow_close);
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.pending_close.is_none() && app.script_conflict.is_none());
    assert!(app.documents[0].dirty);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external = 3;\n");
    assert!(!app.save_document(0, false));
    finish(&mut app);
    std::fs::write(&path, "external = 4;\n").unwrap();
    click_label(&mut app, &ctx, "Overwrite file");
    finish(&mut app);
    assert!(app.script_conflict.is_some());
    assert!(app.status.contains("changed again"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external = 4;\n");
    click_label(&mut app, &ctx, "Overwrite file");
    finish(&mut app);
    assert!(app.script_conflict.is_none());
    assert!(!app.documents[0].dirty);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine = 2;\n");
}

#[test]
fn script_conflict_reload_and_save_all_resume_the_original_close_flow() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [dir.path().join("one.m"), dir.path().join("two.m")];
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = paths
        .iter()
        .map(|path| {
            std::fs::write(path, "old = 1;\n").unwrap();
            let mut d = Document::open(path).unwrap();
            d.text = "mine = 2;\n".into();
            d.changed();
            d
        })
        .collect();
    app.active = 0;
    std::fs::write(&paths[0], "external = 3;\n").unwrap();
    app.pending_quit = true;
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    assert!(app.script_conflict.is_some() && !app.allow_close);
    click_label(&mut app, &ctx, "Reload from disk");
    finish(&mut app);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
        finish(&mut app);
    }
    assert!(app.script_conflict.is_none() && app.allow_close);
    assert!(app.documents.iter().all(|d| !d.dirty));
    assert_eq!(app.documents[0].text, "external = 3;\n");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "mine = 2;\n");
    assert!(app.recovery_dirty);
}

#[test]
fn script_save_as_cannot_replace_another_open_tab_and_conflict_survives_failed_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one.m");
    std::fs::write(&path, "original\n").unwrap();
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::open(&path).unwrap(), Document::new()];
    app.active = 1;
    app.documents[1].text = "other\n".into();
    app.documents[1].changed();
    assert!(!app.save_script_path(1, &path, None));
    finish(&mut app);
    assert!(app.status.contains("another script tab"));
    assert!(app.documents[1].path.is_none() && app.documents[1].dirty);
    app.active = 0;
    app.documents[0].text = "mine\n".into();
    app.documents[0].changed();
    std::fs::write(&path, "external\n").unwrap();
    assert!(!app.save_document(0, false));
    finish(&mut app);
    std::fs::remove_file(&path).unwrap();
    click_label(&mut app, &ctx, "Reload from disk");
    finish(&mut app);
    assert!(app.script_conflict.is_some());
    assert_eq!(app.documents[0].text, "mine\n");
    assert!(app.documents[0].dirty);
    click_label(&mut app, &ctx, "Cancel");
    // Reopening the same ordinary/canonical path focuses its existing tab.
    std::fs::write(&path, "external\n").unwrap();
    app.open_path(path);
    finish(&mut app);
    assert_eq!(app.documents.len(), 2);
}

mod ordinary_conflict_gestures {
    use super::*;
    const SIZE: egui::Vec2 = egui::vec2(1440., 900.);

    fn setup(untitled: bool) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.m");
        std::fs::write(&path, "old=1;\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        app.run("keep=[1 2];Alias=keep;".into());
        finish(&mut app); finish_folder(&mut app);
        let mut document = if untitled { Document::new() } else { Document::open(&path).unwrap() };
        document.text = "mine=2;\n".into(); document.changed();
        let mut other = Document::new();
        other.text = "other_unsaved=9;\n".into(); other.changed();
        app.documents = vec![document, other]; app.active = 0;
        std::fs::write(&path, "external=3;\n").unwrap();
        let absent = matrust::document::DiskVersion::Missing;
        // An untitled Save As can encounter a newly occupied destination.
        assert!(!app.save_script_path(0, &path, untitled.then_some(&absent))); finish(&mut app);
        let conflict = app.script_conflict.as_ref().unwrap();
        assert_eq!(conflict.document, app.documents[0].id);
        assert!(conflict.focus.is_none() && conflict.conversion.is_none());
        assert!(matches!(&conflict.observed, matrust::document::DiskVersion::Text(t) if t == "external=3;\n"));
        assert!(!app.busy && !app.script_io.pending());
        for _ in 0..3 { frame(&mut app, &ctx, vec![]); }
        (ctx, app, dir)
    }
    fn press(app: &mut Workspace, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        let mut point = None;
        for _ in 0..3 {
            point = frame(app, ctx, vec![]).shapes.iter().rev().find_map(|s|
                text_position(&s.shape, label).filter(|p| s.clip_rect.contains(*p)));
        }
        let point = point.unwrap_or_else(|| panic!("Visible ordinary conflict action {label}"));
        pointer_frame(app, ctx, SIZE, point, true); point
    }
    fn release(app: &mut Workspace, ctx: &egui::Context, point: egui::Pos2,
        escape: bool, mut between: impl FnMut(&mut Workspace, &egui::Context)) {
        let calls = app.script_io.save_picker_calls;
        let mut passes = 0;
        let mut events = vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
            pos: point, button: egui::PointerButton::Primary, pressed: false,
            modifiers: egui::Modifiers::NONE }];
        if escape { events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }); }
        crate::headless::run(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
            events, ..Default::default()
        }, |root| {
            passes += 1;
            if passes == 1 { root.ctx().request_discard("Ordinary conflict decision waits for final pass"); }
            app.ui(root);
            if passes == 1 {
                assert!(!app.busy && !app.script_io.pending());
                assert_eq!(app.script_io.save_picker_calls, calls);
                assert!(app.script_conflict.is_some());
                between(app, ctx);
            }
        });
        assert!(passes >= 2);
    }
    fn unchanged(app: &Workspace, other: &Document, values: &std::collections::BTreeMap<String, matrust::Array>) {
        assert!(app.documents.iter().any(|d| d.same_snapshot(other)));
        for (name, value) in values { assert!(value.same_snapshot(&app.variables[name])); }
        assert!(app.variables["keep"].same_snapshot(&app.variables["Alias"]));
    }
    #[test]
    fn ordinary_reload_and_overwrite_release_dispatch_once_with_real_worker_and_disk_guard() {
        for overwrite in [false, true] {
            let (ctx, mut app, dir) = setup(false);
            let other = app.documents[1].clone(); let values = app.variables.clone();
            let id = app.documents[0].id;
            let label = if overwrite { "Overwrite file" } else { "Reload from disk" };
            let point = press(&mut app, &ctx, label);
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            let original = std::mem::replace(&mut app.worker.send, send);
            release(&mut app, &ctx, point, false, |_,_| {});
            assert!(app.busy && app.script_io.pending());
            let request = receive.try_recv().expect("Exactly one accepted script worker request");
            assert!(receive.try_recv().is_err());
            app.worker.send = original; assert!(app.worker.send.send(request).is_ok());
            finish(&mut app);
            assert!(app.script_conflict.is_none()); assert!(!app.documents[0].dirty);
            assert_eq!(app.documents[0].id, id);
            let expected = if overwrite { "mine=2;\n" } else { "external=3;\n" };
            assert_eq!(app.documents[0].text, expected);
            assert_eq!(std::fs::read_to_string(dir.path().join("notes.m")).unwrap(), expected);
            unchanged(&app, &other, &values);
        }
        let (ctx, mut app, dir) = setup(false);
        let point = press(&mut app, &ctx, "Overwrite file");
        release(&mut app, &ctx, point, false, |_,_| {
            std::fs::write(dir.path().join("notes.m"), "external=4;\n").unwrap();
        });
        finish(&mut app);
        assert!(app.script_conflict.is_some() && app.documents[0].dirty);
        assert!(app.status.contains("changed again"));
        assert_eq!(app.documents[0].text, "mine=2;\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("notes.m")).unwrap(), "external=4;\n");
    }
    #[test]
    fn ordinary_save_as_cancel_is_consumed_and_named_or_untitled_retry_uses_real_worker() {
        for untitled in [false, true] {
            let (ctx, mut app, dir) = setup(untitled);
            let before = app.documents[0].clone(); let other = app.documents[1].clone();
            let values = app.variables.clone();
            app.script_io.save_picker = Some(|_,_| None);
            let point = press(&mut app, &ctx, "Save as...");
            release(&mut app, &ctx, point, false, |_,_| {});
            assert_eq!(app.script_io.save_picker_calls, 1);
            assert!(app.documents[0].same_snapshot(&before));
            assert!(app.script_conflict.is_some() && !app.busy);
            for _ in 0..2 { frame(&mut app, &ctx, vec![]); }
            assert_eq!(app.script_io.save_picker_calls, 1);
            app.script_io.save_picker = Some(|app,_| Some(app.folder.join("copy.m")));
            let point = press(&mut app, &ctx, "Save as...");
            release(&mut app, &ctx, point, false, |_,_| {});
            assert_eq!(app.script_io.save_picker_calls, 2); finish(&mut app);
            assert!(app.script_conflict.is_none() && !app.documents[0].dirty);
            assert_eq!(app.documents[0].id, before.id);
            assert!(matrust::document::same_path(app.documents[0].path.as_ref().unwrap(), &dir.path().join("copy.m")));
            assert_eq!(std::fs::read_to_string(dir.path().join("copy.m")).unwrap(), "mine=2;\n");
            assert_eq!(std::fs::read_to_string(dir.path().join("notes.m")).unwrap(), "external=3;\n");
            unchanged(&app, &other, &values);
        }
    }
    fn invalidate(app: &mut Workspace, case: usize) {
        match case {
            0 => app.stop_work(),
            1 => { app.documents[0].text.push_str("newer=5;\n"); app.documents[0].changed(); }
            2 => app.documents[0] = Document::new(),
            3 => app.pending_close = Some(1),
            4 => app.pending_quit = true,
            5 => app.busy = true,
            6 => app.recovering_session = true,
            7 => app.recovery_blocked = true,
            8 => app.allow_close = true,
            9 => app.folder.push("new_owner"),
            10 => {
                // Construct a real competing dialog while its ordinary admission is open,
                // then restore the retained conflict to model a late competing owner.
                // Never call the constructor through its intentional own-conflict guard.
                let conflict = app.script_conflict.take().unwrap();
                assert!(!app.busy && !app.file_action_pending());
                app.begin_workspace_action(false, true);
                assert!(app.workspace_tools.dialog.is_some());
                app.script_conflict = Some(conflict);
            }
            11 => { app.cancel_ordinary_script_conflict(); app.script_conflict.as_mut().unwrap().observed = matrust::document::DiskVersion::Missing; }
            _ => unreachable!(),
        }
    }
    #[test]
    fn ordinary_retained_decision_rejects_late_document_continuation_stop_modal_and_focus() {
        for case in 0..13 {
            let (ctx, mut app, dir) = setup(false);
            let other = app.documents[1].clone(); let values = app.variables.clone();
            app.script_io.save_picker = Some(|_,_| panic!("Stale conflict opened chooser"));
            let point = press(&mut app, &ctx, "Save as...");
            release(&mut app, &ctx, point, false, |app,ctx| {
                if case == 12 { ctx.memory_mut(|m| m.request_focus(egui::Id::new("new_conflict_owner"))); }
                else { invalidate(app, case); }
            });
            assert_eq!(app.script_io.save_picker_calls, 0, "case {case}");
            assert!(!app.script_io.pending(), "case {case}");
            assert_eq!(std::fs::read_to_string(dir.path().join("notes.m")).unwrap(), "external=3;\n");
            assert!(!dir.path().join("copy.m").exists()); unchanged(&app, &other, &values);
            if case == 1 { assert!(app.documents[0].text.ends_with("newer=5;\n")); }
            if case == 5 { app.busy = false; }
        }
    }
    #[test]
    fn ordinary_save_as_revalidates_after_picker_without_overwriting_newer_work() {
        for case in 0..12 {
            let (ctx, mut app, dir) = setup(false);
            let other = app.documents[1].clone(); let values = app.variables.clone();
            app.script_io.save_picker = Some(|app,_| {
                let path = app.folder.join("copy.m");
                let case = app.status.strip_prefix("Conflict picker case ").unwrap().parse().unwrap();
                invalidate(app, case); Some(path)
            });
            let point = press(&mut app, &ctx, "Save as...");
            app.status = format!("Conflict picker case {case}");
            pointer_frame(&mut app, &ctx, SIZE, point, false);
            assert_eq!(app.script_io.save_picker_calls, 1, "case {case}");
            assert!(!app.script_io.pending(), "case {case}");
            assert!(!dir.path().join("copy.m").exists());
            assert_eq!(std::fs::read_to_string(dir.path().join("notes.m")).unwrap(), "external=3;\n");
            unchanged(&app, &other, &values);
            if case == 1 { assert!(app.documents[0].text.ends_with("newer=5;\n")); }
            if case == 5 { app.busy = false; }
        }
    }
    #[test]
    fn ordinary_escape_wins_retained_overwrite_and_quit_reload_continuation_still_finishes() {
        let (ctx, mut app, dir) = setup(false);
        app.pending_close = Some(0); app.pending_quit = true;
        app.script_save_queue = Some([app.documents[0].id].into());
        let before = app.documents[0].clone();
        let point = press(&mut app, &ctx, "Overwrite file");
        release(&mut app, &ctx, point, true, |_,_| {});
        assert!(app.script_conflict.is_none() && app.pending_close.is_none());
        assert!(!app.pending_quit && app.script_save_queue.is_none() && !app.allow_close);
        assert!(app.documents[0].same_snapshot(&before));
        assert_eq!(std::fs::read_to_string(dir.path().join("notes.m")).unwrap(), "external=3;\n");

        let (ctx, mut app, dir) = setup(false);
        // The untouched second dirty buffer is named and joins the actual Save-all continuation.
        let second = dir.path().join("second.m");
        std::fs::write(&second, "old_second=0;\n").unwrap();
        app.documents[1] = Document::open(&second).unwrap();
        app.documents[1].text = "new_second=1;\n".into(); app.documents[1].changed();
        app.pending_quit = true;
        let point = press(&mut app, &ctx, "Reload from disk");
        release(&mut app, &ctx, point, false, |_,_| {}); finish(&mut app);
        for _ in 0..4 { frame(&mut app, &ctx, vec![]); finish(&mut app); finish_folder(&mut app); }
        assert!(app.script_conflict.is_none() && app.allow_close);
        assert!(app.documents.iter().all(|d| !d.dirty));
        assert_eq!(app.documents[0].text, "external=3;\n");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "new_second=1;\n");
    }
}
