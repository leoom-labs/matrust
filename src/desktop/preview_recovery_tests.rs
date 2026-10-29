#[test]
fn preview_recovery_restores_page_view_and_survives_missing_source_without_retry_loop() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("pages.tiff");
    std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/tiff-preview/pages.tiff"),&source).unwrap();
    app.submit(Job::OpenPagedImage(source.clone(),1)); finish(&mut app); frame(&mut app,&ctx,vec![]);
    let viewer = app.image_view.as_mut().unwrap();
    viewer.fit = false; viewer.zoom = 1.25; viewer.export_format = image_export::Format::Tiff;
    let state = app.snapshot();
    let path = dir.path().join("session.json");
    state.clone().save(&path).unwrap();
    let mut restored = legacy_sample_desktop(ctx.clone(),true);
    restored.restore_session(Session::load(&path).unwrap());
    frame(&mut restored,&ctx,vec![]); finish(&mut restored); frame(&mut restored,&ctx,vec![]);
    let view = restored.image_view.as_ref().unwrap();
    assert_eq!(view.page.as_ref().unwrap().index,1);
    assert_eq!(view.zoom,1.25); assert!(!view.fit);
    assert_eq!(view.export_format,image_export::Format::Tiff);
    assert!(restored.preview_restore.is_none());
    assert_eq!(restored.snapshot().preview,state.preview);
    // A tiny fit scale can become manual when Zoom in is pressed. Keep the
    // resulting manual zoom within the same range as Zoom out and recovery.
    restored.image_view.as_mut().unwrap().zoom = 0.001;
    click_label(&mut restored,&ctx,"Zoom in");
    assert_eq!(restored.image_view.as_ref().unwrap().zoom,0.01);
    assert!(restored.snapshot().preview.unwrap().valid());
    restored.close_image_preview();
    assert!(restored.snapshot().preview.is_none() && restored.recovery_dirty);

    std::fs::remove_file(&source).unwrap();
    let mut missing = legacy_sample_desktop(ctx.clone(),true);
    missing.restore_session(state.clone());
    frame(&mut missing,&ctx,vec![]); finish(&mut missing); frame(&mut missing,&ctx,vec![]);
    assert!(missing.preview_restore.as_ref().unwrap().error.is_some());
    for _ in 0..3 { frame(&mut missing,&ctx,vec![]); assert!(!missing.busy); }
    assert_eq!(missing.snapshot().preview,state.preview);
    std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/tiff-preview/pages.tiff"),&source).unwrap();
    click_label(&mut missing,&ctx,"Retry preview");
    frame(&mut missing,&ctx,vec![]); finish(&mut missing); frame(&mut missing,&ctx,vec![]);
    assert_eq!(missing.image_view.as_ref().unwrap().page.as_ref().unwrap().index,1);
    assert!(missing.preview_restore.is_none());
    missing.close_image_preview(); missing.restore_session(state);
    missing.submit(Job::OpenPagedImage(source,0)); finish(&mut missing); frame(&mut missing,&ctx,vec![]);
    assert_eq!(missing.image_view.as_ref().unwrap().page.as_ref().unwrap().index,0);
    assert!(missing.image_view.as_ref().unwrap().fit);
}

#[test]
fn preview_recovery_waits_for_worker_and_close_or_forget_removes_reference() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(),true);
    let mut state = app.snapshot();
    state.preview = Some(matrust::session::PreviewState {
            navigation: Default::default(),
        path: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/pdf-preview/pages.pdf"),
        page: 1, fit: true, zoom: 1., format: matrust::session::PreviewFormat::Png,
    });
    app.restore_session(state.clone());
    app.run("keep=42;".into());
    app.preview_recovery_ui(&ctx,false);
    assert!(!app.preview_restore.as_ref().unwrap().started);
    finish(&mut app);
    frame(&mut app,&ctx,vec![]);
    assert!(app.busy);
    app.close_image_preview();
    finish(&mut app); frame(&mut app,&ctx,vec![]);
    assert!(app.image_view.is_none() && app.snapshot().preview.is_none());
    assert_eq!(app.variables["keep"].scalar(0, 0).unwrap().into_real().unwrap(),42.);
    state.preview.as_mut().unwrap().path = std::env::temp_dir().join(format!("matrust-missing-preview-file-{}.pdf", std::process::id()));
    app.restore_session(state);
    frame(&mut app,&ctx,vec![]); finish(&mut app); frame(&mut app,&ctx,vec![]);
    click_label(&mut app,&ctx,"Forget preview");
    assert!(app.snapshot().preview.is_none());
    assert_eq!(app.variables["keep"].scalar(0, 0).unwrap().into_real().unwrap(),42.);
}

#[test]
fn preview_recovery_shutdown_preserves_open_and_failed_references_before_cleanup() {
    let missing = std::env::temp_dir().join(format!("matrust-missing-exit-preview-{}.png", std::process::id()));
    for filename in [std::path::PathBuf::from("tests/fixtures/preview/tiff-preview/reference-1.png"), missing] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(),true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut state = app.snapshot();
        state.preview = Some(matrust::session::PreviewState {
            navigation: Default::default(),
            path:std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(filename),
            page:0,fit:false,zoom:1.25,format:matrust::session::PreviewFormat::Tiff,
        });
        let mut expected = state.preview.clone();
        app.restore_session(state);
        frame(&mut app,&ctx,vec![]); finish(&mut app); frame(&mut app,&ctx,vec![]);
        expected.as_mut().unwrap().navigation = app.preview_checkpoint().unwrap().navigation;
        app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
        eframe::App::on_exit(&mut app,None);
        assert_eq!(Session::load(&path).unwrap().preview,expected);
        drop(app);
    }
}
