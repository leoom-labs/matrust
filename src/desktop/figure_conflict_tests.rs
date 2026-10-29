fn figure_conflict_app(ctx: &egui::Context, path: &std::path::Path) -> Workspace {
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.documents = vec![Document::new()];
    app.run("plot([1 2 3],[2 4 8]);title('Saved');".into());
    finish(&mut app);
    app.submit(Job::SaveFigure(path.to_owned(), None));
    finish(&mut app);
    let mut external = app.figure.clone().unwrap();
    external.title = "External".into();
    matrust::files::save_figure(path, &external).unwrap();
    app.run("title('Local');".into());
    finish(&mut app);
    app
}
#[test]
fn figure_conflict_cancel_and_stale_overwrite_keep_versions_and_stop_close() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let ctx = egui::Context::default();
    let mut app = figure_conflict_app(&ctx, &path);
    app.request_figure_close(1);
    click_label(&mut app, &ctx, "Save");
    finish(&mut app);
    assert!(app.figure_conflict.is_some() && app.figure_close.is_some());
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
        assert!(!app.busy);
    }
    app.quit_discarded_figures.insert(1);
    click_label(&mut app, &ctx, "Cancel");
    assert!(
        app.figure_conflict.is_none()
            && app.figure_close.is_none()
            && app.quit_discarded_figures.is_empty()
    );
    assert!(app.figure_dirty(1));
    assert_eq!(app.figure.as_ref().unwrap().title, "Local");
    assert_eq!(
        matrust::files::load_figure(&path).unwrap().title,
        "External"
    );
    app.save_figure_document(1, false);
    finish(&mut app);
    let mut external = app.figure.clone().unwrap();
    external.title = "Changed again".into();
    matrust::files::save_figure(&path, &external).unwrap();
    click_label(&mut app, &ctx, "Overwrite figure");
    finish(&mut app);
    assert!(app.figure_conflict.is_some() && app.figure_dirty(1));
    assert!(app.status.contains("changed again"));
    assert_eq!(
        matrust::files::load_figure(&path).unwrap().title,
        "Changed again"
    );
    click_label(&mut app, &ctx, "Overwrite figure");
    finish(&mut app);
    assert!(app.figure_conflict.is_none() && !app.figure_dirty(1));
    assert_eq!(matrust::files::load_figure(&path).unwrap().title, "Local");
}
#[test]
fn figure_conflict_failed_reload_retains_data_and_reload_resumes_save_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let ctx = egui::Context::default();
    let mut app = figure_conflict_app(&ctx, &path);
    app.run("figure(2);plot([5 6 7]);".into());
    finish(&mut app);
    let second = dir.path().join("second.mfig");
    app.submit(Job::SaveFigure(second.clone(), None));
    finish(&mut app);
    app.run("title('Second local');".into());
    finish(&mut app);
    app.request_quit(&ctx);
    click_label(&mut app, &ctx, "Save all");
    finish(&mut app);
    assert_eq!(app.figure_conflict.as_ref().unwrap().id, 1);
    assert_eq!(app.current_figure, 2);
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"corrupt").unwrap();
    click_label(&mut app, &ctx, "Reload figure");
    finish(&mut app);
    assert!(app.figure_conflict.is_some() && !app.allow_close);
    assert_eq!(app.all_figures[&1].title, "Local");
    std::fs::write(&path, original).unwrap();
    click_label(&mut app, &ctx, "Reload figure");
    finish(&mut app);
    assert!(app.figure_conflict.is_none());
    assert_eq!(app.current_figure, 2);
    assert_eq!(app.all_figures[&1].title, "External");
    assert!(!app.figure_dirty(1));
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
        finish(&mut app);
    }
    assert!(app.allow_close && app.figure_close.is_none());
    assert_eq!(
        matrust::files::load_figure(&second).unwrap().title,
        "Second local"
    );
}
#[test]
fn figure_conflict_reload_clears_manual_view_and_failed_save_as_keeps_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let ctx = egui::Context::default();
    let mut app = figure_conflict_app(&ctx, &path);
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    app.record_figure_view([1.2, 1.8, 2.1, 3.1], [false, false]);
    app.save_figure_document(1, false);
    finish(&mut app);
    assert!(app.figure_conflict.is_some());
    let identity = app.figure_documents[&1].clone();
    app.submit(Job::SaveFigure(dir.path().join("missing/copy.mfig"), None));
    finish(&mut app);
    assert_eq!(app.figure_documents[&1], identity);
    assert!(app.figure_conflict.is_some());
    click_label(&mut app, &ctx, "Reload figure");
    finish(&mut app);
    assert!(app.figure_conflict.is_none() && !app.figure_views.contains_key(&1));
    assert!(app.reset_plot && !app.figure_dirty(1));
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.figure.as_ref().unwrap().title, "External");
    assert!(!app.figure_dirty(1));
    // Save-as resolves a later conflict and preserves the external original.
    app.run("title('New local');".into());
    finish(&mut app);
    let mut external = app.figure.clone().unwrap();
    external.title = "New disk".into();
    matrust::files::save_figure(&path, &external).unwrap();
    app.save_figure_document(1, false);
    finish(&mut app);
    let copy = dir.path().join("copy.mfig");
    app.submit(Job::SaveFigure(copy.clone(), None));
    finish(&mut app);
    assert!(app.figure_conflict.is_none() && !app.figure_dirty(1));
    assert_eq!(app.figure_documents[&1].path, Some(copy));
    assert_eq!(
        matrust::files::load_figure(&path).unwrap().title,
        "New disk"
    );
}
#[test]
fn recovered_figure_conflict_uses_original_baseline_and_old_recovery_stays_unknown() {
    use matrust::session::figures::SessionStore;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let ctx = egui::Context::default();
    let app = figure_conflict_app(&ctx, &path);
    let session = dir.path().join("session.json");
    SessionStore::new(&session).save(app.snapshot()).unwrap();
    let mut restored = legacy_sample_desktop(ctx.clone(), true);
    restored.restore_session(Session::load(&session).unwrap());
    restored.start_session_recovery(session.clone());
    finish(&mut restored);
    assert_eq!(
        restored.figure_documents[&1].source_version,
        app.figure_documents[&1].source_version
    );
    restored.save_figure_document(1, false);
    finish(&mut restored);
    assert!(restored.figure_conflict.is_some());
    assert_eq!(restored.figure.as_ref().unwrap().title, "Local");
    let mut snapshot = app.snapshot();
    snapshot.figures.as_mut().unwrap().figures[0].source_version = None;
    SessionStore::new(&session).save(snapshot).unwrap();
    let mut legacy = legacy_sample_desktop(egui::Context::default(), true);
    legacy.restore_session(Session::load(&session).unwrap());
    legacy.start_session_recovery(session);
    finish(&mut legacy);
    assert!(legacy.figure_documents[&1].source_version.is_none());
    legacy.save_figure_document(1, false);
    finish(&mut legacy);
    assert!(legacy.figure_conflict.is_some());
}
