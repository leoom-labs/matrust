#[test]
fn background_script_save_keeps_newer_edits_and_stable_document_identity() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx, true);
    let paths = named_dirty_scripts(&mut app, dir.path(), 2);
    let id = app.documents[0].id;
    app.save_document(0, false);
    app.documents[0].text = "newer=123;".into();
    app.documents[0].changed();
    // Completion targets the stable ID, even if the tab collection is reordered.
    app.documents.swap(0, 1);
    finish(&mut app);
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "value=0;\n");
    assert_eq!(app.documents[1].id, id);
    assert_eq!(app.documents[1].text, "newer=123;");
    assert!(app.documents[1].dirty);
    let mut baseline = app.documents[1].clone();
    baseline.discard();
    assert_eq!(baseline.text, "value=0;\n");
    assert_eq!(app.documents[0].text, "value=1;\n");
}

#[test]
fn background_script_stop_after_commit_acknowledges_save_without_closing() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let paths = named_dirty_scripts(&mut app, dir.path(), 1);
    app.pending_quit = true;
    app.save_document(0, false);
    let reply = app
        .worker
        .receive
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "value=0;\n");
    app.stop_work();
    app.busy = false;
    app.receive_script_io(reply.script_io.unwrap());
    frame(&mut app, &ctx, vec![]);
    assert!(!app.documents[0].dirty);
    assert!(!app.pending_quit && !app.allow_close && !app.script_io.resume);
    assert_eq!(app.documents.len(), 1);
}

#[test]
fn background_script_reload_retains_edits_made_during_read_and_can_retry() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let paths = named_dirty_scripts(&mut app, dir.path(), 1);
    std::fs::write(&paths[0], "external=5;").unwrap();
    app.preview_script_conflict();
    finish(&mut app);
    app.begin_script_io(0, script_io::Kind::Reload);
    app.documents[0].text = "late_edit=9;".into();
    app.documents[0].changed();
    finish(&mut app);
    assert_eq!(app.documents[0].text, "late_edit=9;");
    assert!(app.documents[0].dirty && app.script_conflict.is_some());
    assert!(app.status.contains("Editor changed"));
    click_label(&mut app, &ctx, "Reload from disk");
    finish(&mut app);
    assert_eq!(app.documents[0].text, "external=5;");
    assert!(!app.documents[0].dirty && app.script_conflict.is_none());
}

#[test]
fn background_script_alias_check_failure_retains_both_tabs_and_files() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx, true);
    let paths = named_dirty_scripts(&mut app, dir.path(), 2);
    let alias = dir.path().join(".").join("script1.m");
    app.save_script_path(0, &alias, None);
    assert!(app.script_io.pending());
    app.request_script_close(0);
    assert!(app.pending_close.is_none());
    finish(&mut app);
    assert!(app.status.contains("another script tab"));
    assert!(app.documents.iter().all(|d| d.dirty));
    for path in paths {
        assert_eq!(std::fs::read_to_string(path).unwrap(), "old=1;\n");
    }
}

#[test]
fn background_script_stop_before_dispatch_and_ready_reload_leave_work_intact() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx, true);
    let paths = named_dirty_scripts(&mut app, dir.path(), 1);
    app.run("a=1;".into());
    app.save_document(0, false);
    assert!(app.script_io.queued.is_some());
    app.stop_work();
    finish(&mut app);
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "old=1;\n");
    assert!(app.documents[0].dirty && !app.script_io.pending());
    app.begin_script_io(0, script_io::Kind::Reload);
    let reply = app
        .worker
        .receive
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    app.stop_work();
    app.busy = false;
    app.receive_script_io(reply.script_io.unwrap());
    assert_eq!(app.documents[0].text, "value=0;\n");
    assert!(app.documents[0].dirty);
}

#[test]
fn background_script_saves_and_reload_preserve_shared_workspace_and_figure() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx, true);
    named_dirty_scripts(&mut app, dir.path(), 1);
    app.run("keep=[2 4 6];plot(keep);".into());
    finish(&mut app);
    let data = app.variables["keep"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    app.save_document(0, false);
    finish(&mut app);
    app.begin_script_io(0, script_io::Kind::Reload);
    finish(&mut app);
    assert!(data.same_snapshot(&app.variables["keep"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    assert!(!app.variables.contains_key("value"));
}


#[test]
fn background_generated_bundle_save_as_relocates_and_preserves_late_edits_by_id() {
    let ctx = egui::Context::default();
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("keep=[2 4 6];plot(keep);title('Relocation worker');".into());
    finish(&mut app);
    let data = app.variables["keep"].clone();
    let figure = app.figure.as_ref().unwrap().clone();
    let bundle = matrust::files::write_matlab_plot_bundle_with_cancel(
        &source.path().join("generated.m"), &figure, &std::sync::atomic::AtomicBool::new(false),
    ).unwrap();
    let expected = std::fs::read(&bundle.script).unwrap();
    assert!(app.try_open_path(bundle.script.clone()));
    finish(&mut app);
    let index = app.documents.iter().position(|d| d.path.as_ref().is_some_and(|p| matrust::document::same_path(p, &bundle.script))).unwrap();
    let id = app.documents[index].id;
    let saved = app.documents[index].text.clone();
    let path = target.path().join("relocated.m");
    app.save_script_path(index, &path, None);
    assert!(app.script_io.pending());
    app.documents[index].text.push_str("\nlate_editor_change=1;\n");
    app.documents[index].changed();
    app.documents.swap(0, index);
    finish(&mut app);
    let document = app.documents.iter().find(|d| d.id == id).unwrap();
    assert!(document.path.as_ref().is_some_and(|p| matrust::document::same_path(p, &path)));
    assert!(document.dirty && document.text.contains("late_editor_change"));
    let mut baseline = document.clone();
    baseline.discard();
    assert_eq!(baseline.text, saved);
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let leaf = bundle.companion_directory.file_name().unwrap();
    assert_eq!(std::fs::read(target.path().join(leaf).join("data.mat")).unwrap(), std::fs::read(&bundle.data).unwrap());
    assert!(data.same_snapshot(&app.variables["keep"]));
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    let document_index = app.documents.iter().position(|d| d.id == id).unwrap();
    app.save_script_path(document_index, &target.path().join("edited.m"), None);
    finish(&mut app);
    assert!(app.script_conflict.as_ref().is_some_and(|conflict| conflict.conversion.is_some()));
    assert!(!target.path().join("edited.m").exists());
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.script_conflict.is_none());
    assert!(app.documents[document_index].dirty);
}

#[test]
fn background_bundle_open_rejects_changed_data_and_stop_after_commit_adopts_real_path() {
    let ctx = egui::Context::default();
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("keep=[1 2 3];plot(keep);".into());
    finish(&mut app);
    let figure = app.figure.as_ref().unwrap().clone();
    let bundle = matrust::files::write_matlab_plot_bundle_with_cancel(
        &source.path().join("generated.m"), &figure, &std::sync::atomic::AtomicBool::new(false),
    ).unwrap();
    let bytes = std::fs::read(&bundle.data).unwrap();
    let count = app.documents.len();
    std::fs::write(&bundle.data, b"tampered").unwrap();
    assert!(app.try_open_path(bundle.script.clone()));
    finish(&mut app);
    assert_eq!(app.documents.len(), count);
    assert!(app.status.contains("MAT data changed"));
    std::fs::write(&bundle.data, &bytes).unwrap();
    assert!(app.try_open_path(bundle.script.clone()));
    finish(&mut app);
    let index = app.documents.iter().position(|d| d.path.as_ref().is_some_and(|p| matrust::document::same_path(p, &bundle.script))).unwrap();
    let id = app.documents[index].id;
    let path = target.path().join("committed.m");
    app.save_script_path(index, &path, None);
    let reply = app.worker.receive.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(path.is_file());
    app.stop_work();
    app.busy = false;
    app.receive_script_io(reply.script_io.unwrap());
    frame(&mut app, &ctx, vec![]);
    let document = app.documents.iter().find(|d| d.id == id).unwrap();
    assert!(!document.dirty);
    assert!(document.path.as_ref().is_some_and(|p| matrust::document::same_path(p, &path)));
    document.verify_reconstruction(&std::sync::atomic::AtomicBool::new(false)).unwrap();
    assert!(!app.pending_quit && !app.allow_close);
    assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
}


#[test]
fn generated_script_conversion_prompt_cancel_stale_version_disk_conflict_and_late_edit() {
    let ctx = egui::Context::default();
    let directory = tempfile::tempdir().unwrap();
    let mut figure = matrust::plot::Figure::default();
    figure.add(vec![matrust::plot::Series::new(vec![[1., 2.], [2., 3.]], matrust::plot::PlotKind::Line)]).unwrap();
    let bundle = matrust::files::write_matlab_plot_bundle_with_cancel(
        &directory.path().join("generated.m"), &figure, &std::sync::atomic::AtomicBool::new(false),
    ).unwrap();
    let original = std::fs::read_to_string(&bundle.script).unwrap();
    let data = std::fs::read(&bundle.data).unwrap();
    let manifest = std::fs::read(bundle.companion_directory.join("reconstruction.json")).unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::open(&bundle.script).unwrap()];
    app.active = 0;
    let id = app.documents[0].id;
    app.documents[0].text.push_str("\nedited=1;\n");
    app.documents[0].changed();
    app.save_document(0, false); finish(&mut app);
    assert!(app.script_conflict.as_ref().unwrap().conversion.is_some());
    assert_eq!(std::fs::read_to_string(&bundle.script).unwrap(), original);
    click_label(&mut app, &ctx, "Cancel");
    assert!(app.documents[0].dirty && app.script_conflict.is_none());
    app.save_document(0, false); finish(&mut app);
    app.documents[0].text.push_str("changed_after_prompt=2;\n");
    app.documents[0].changed();
    click_label(&mut app, &ctx, "Save edited script");
    assert!(!app.script_io.pending() && app.script_conflict.is_none());
    assert!(app.status.contains("changed since"));
    assert_eq!(std::fs::read_to_string(&bundle.script).unwrap(), original);
    app.save_document(0, false); finish(&mut app);
    std::fs::write(&bundle.script, "external=3;\n").unwrap();
    click_label(&mut app, &ctx, "Save edited script"); finish(&mut app);
    assert!(app.script_conflict.as_ref().unwrap().conversion.is_none());
    assert_eq!(std::fs::read_to_string(&bundle.script).unwrap(), "external=3;\n");
    click_label(&mut app, &ctx, "Overwrite file"); finish(&mut app);
    assert!(app.script_conflict.as_ref().unwrap().conversion.is_some());
    let captured = app.documents[0].text.clone();
    click_label(&mut app, &ctx, "Save edited script");
    app.documents[0].text.push_str("later=4;\n");
    app.documents[0].changed();
    finish(&mut app);
    let saved = std::fs::read_to_string(&bundle.script).unwrap();
    assert!(saved.starts_with("% MatRust ordinary script:"));
    assert_eq!(saved.split_once('\n').unwrap().1, captured.split_once('\n').unwrap().1);
    assert_eq!(app.documents[0].id, id);
    assert!(app.documents[0].dirty && app.documents[0].text.ends_with("later=4;\n"));
    let mut baseline = app.documents[0].clone(); baseline.discard();
    assert_eq!(baseline.text, saved);
    // The newer editor still carries the old header, while its saved baseline
    // now has the ordinary notice. A second explicit Save remains usable.
    let later = app.documents[0].text.clone();
    app.save_document(0, false); finish(&mut app);
    assert!(app.script_conflict.as_ref().unwrap().conversion.is_some());
    click_label(&mut app, &ctx, "Save edited script"); finish(&mut app);
    assert!(!app.documents[0].dirty && app.script_conflict.is_none());
    assert_eq!(app.documents[0].text.split_once('\n').unwrap().1, later.split_once('\n').unwrap().1);
    assert!(app.documents[0].text.ends_with("later=4;\n"));
    assert_eq!(std::fs::read_to_string(&bundle.script).unwrap(), app.documents[0].text);
    assert_eq!(std::fs::read(&bundle.data).unwrap(), data);
    assert_eq!(std::fs::read(bundle.companion_directory.join("reconstruction.json")).unwrap(), manifest);
}

#[test]
fn generated_script_only_cross_folder_choice_dispatches_once_after_layout_retry() {
    let ctx = egui::Context::default();
    let directory = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let bundle = matrust::files::write_matlab_plot_bundle_with_cancel(
        &directory.path().join("generated.m"), &matrust::plot::Figure::default(), &std::sync::atomic::AtomicBool::new(false),
    ).unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::open(&bundle.script).unwrap()];
    app.active = 0;
    app.documents[0].text.push_str("\nedited=5;\n");
    app.documents[0].changed();
    let original = std::fs::read(&bundle.script).unwrap();
    let path = target.path().join("script_only.m");
    app.save_script_path(0, &path, None); finish(&mut app);
    assert!(!app.script_conflict.as_ref().unwrap().conversion.as_ref().unwrap().same_folder);
    let size = egui::vec2(960., 650.);
    let mut position = None;
    for _ in 0..3 {
        position = frame_at(&mut app, &ctx, size, vec![]).shapes.iter().rev()
            .find_map(|shape| text_position(&shape.shape, "Save script only"));
    }
    let pos = position.expect("compact script-only choice is reachable");
    pointer_frame(&mut app, &ctx, size, pos, true);
    let mut passes = 0;
    let _ = crate::headless::run(&ctx, egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events: vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE,
        }], ..Default::default()
    }, |root| {
        passes += 1;
        if passes == 1 { root.ctx().request_discard("Explicit script-only consent waits for final pass"); }
        app.ui(root);
        if passes == 1 { assert!(!app.script_io.pending()); assert!(!path.exists()); }
    });
    assert!(passes >= 2);
    finish(&mut app);
    assert!(app.script_conflict.is_none() && !app.documents[0].dirty);
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 1);
    assert_eq!(std::fs::read(&bundle.script).unwrap(), original);
    assert!(app.documents[0].text.starts_with("% MatRust ordinary script:"));
    assert!(app.documents[0].text.ends_with("edited=5;\n"));
    app.begin_script_io(0, script_io::Kind::Reload); finish(&mut app);
    assert!(!app.documents[0].dirty);
    let saved_messages = app.console.iter().filter(|line| line.starts_with("Saved ")).count();
    assert_eq!(saved_messages, 1);
}
