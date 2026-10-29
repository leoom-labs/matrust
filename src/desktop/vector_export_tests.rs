#[test]
fn vector_export_presets_reopen_at_requested_size_and_keep_editable_figure() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("x=1:10;plot(x,x);title('Vector canvas');".into());
    finish(&mut app);
    let original = app.figure.clone().unwrap();
    let data = app.variables["x"].clone();
    app.export_settings.format = figure_export::Format::Pdf;
    app.begin_figure_export();
    frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    click_label(&mut app, &ctx, "A4 portrait");
    let settings = app.figure_export.as_ref().unwrap().settings;
    assert_eq!((settings.width, settings.height), (1440, 840));
    assert!((settings.vector.width - 595.2756).abs() < 0.001);
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("page.pdf");
    app.export_to_path(pdf.clone());
    finish(&mut app);
    assert!(app.figure_export.is_none());
    assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
    assert!(app.figure_dirty(app.current_figure));
    assert!(data.same_snapshot(&app.variables["x"]));
    app.open_path(pdf);
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    let page = app.image_view.as_ref().unwrap();
    assert_eq!(page.page.as_ref().unwrap().document.page_count, 1);
    let ratio = page.original_size[0] as f32 / page.original_size[1] as f32;
    assert!((ratio - settings.vector.width / settings.vector.height).abs() < 0.01);
    app.close_image_preview();
    app.export_settings.format = figure_export::Format::Svg;
    app.begin_figure_export();
    frame(&mut app, &ctx, vec![]);
    click_label(&mut app, &ctx, "Square");
    let svg = dir.path().join("square.svg");
    app.export_to_path(svg.clone());
    finish(&mut app);
    app.open_path(svg);
    finish(&mut app);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.image_view.as_ref().unwrap().original_size, [1600, 1600]);
    assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
}
#[test]
fn export_preferences_persist_on_cancel_and_restart_and_invalid_sizes_do_not_hide_work() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    app.recovery = Some(recovery::Recovery::open(path.clone()).unwrap());
    app.run("plot(1:4);".into());
    finish(&mut app);
    app.export_settings.format = figure_export::Format::Pdf;
    app.begin_figure_export();
    frame(&mut app, &ctx, vec![]);
    app.recovery_dirty = false;
    click_label(&mut app, &ctx, "Letter");
    assert!(app.recovery_dirty);
    click_label(&mut app, &ctx, "Cancel");
    let expected = app.export_settings;
    assert_eq!(
        expected.vector,
        matrust::plot::Canvas {
            width: 612.,
            height: 792.
        }
    );
    eframe::App::on_exit(&mut app, None);
    drop(app);
    let mut restored = legacy_sample_desktop(ctx.clone(), true);
    restored.restore_session(Session::load(&path).unwrap());
    restored.start_session_recovery(path);
    finish(&mut restored);
    assert_eq!(restored.export_settings, expected);
    restored.begin_figure_export();
    frame(&mut restored, &ctx, vec![]);
    restored
        .figure_export
        .as_mut()
        .unwrap()
        .settings
        .vector
        .width = f32::INFINITY;
    let target = dir.path().join("preserved.pdf");
    std::fs::write(&target, b"keep").unwrap();
    restored.export_to_path(target.clone());
    assert!(!restored.busy && restored.figure_export.as_ref().unwrap().error.is_some());
    assert_eq!(std::fs::read(target).unwrap(), b"keep");
    click_label(&mut restored, &ctx, "Plot canvas");
    assert_eq!(
        restored.figure_export.as_ref().unwrap().settings.vector,
        Default::default()
    );
}

#[test]
fn color_eps_actual_worker_and_batch_preserve_documents_views_and_shared_payloads() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 2);
    // Complete the actual initial plot paint before supplying a manual view.
    // An unconsumed reset_plot would correctly clear a synthetic view on reply.
    frame(&mut app, &ctx, vec![]);
    assert!(!app.reset_plot);
    let originals = app.all_figures.clone();
    let statuses = app.figure_documents.clone();
    let payload = app.variables["keep"].clone();
    let bounds = [1.1, 1.9, 2.1, 2.9];
    app.record_figure_view(bounds, [false; 2]);
    app.view_bounds = Some(bounds);
    app.view_auto = [false; 2];
    app.export_settings.format = figure_export::Format::Eps;
    app.export_settings.vector = matrust::plot::Canvas { width: 612., height: 792. };
    app.begin_figure_export();
    let path = dir.path().join("one.eps");
    app.export_to_path(path.clone());
    assert!(app.busy, "export must submit the real worker");
    finish(&mut app);
    assert!(app.figure_export.is_none(), "{}", app.status);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0\n"));
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("%%BoundingBox: 0 0 612 792\n"));
    let mut expected = originals[&2].clone();
    expected.xlim = Some([bounds[0], bounds[1]]);
    expected.ylim = Some([bounds[2], bounds[3]]);
    let reference = dir.path().join("reference.eps");
    matrust::export::export_eps_with_cancel(&expected, &reference, app.export_settings.vector, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    assert_eq!(bytes, std::fs::read(reference).unwrap());
    assert_eq!(app.figure_documents, statuses);
    assert!(payload.same_snapshot(&app.variables["keep"]));
    assert_eq!(app.view_bounds, Some(bounds));
    app.begin_figure_export();
    app.figure_export.as_mut().unwrap().select_all();
    app.prepare_export_folder(dir.path().into());
    assert!(app.busy);
    finish(&mut app);
    assert!(!dir.path().join("figure-1.eps").exists());
    app.run_export_batch();
    assert!(app.busy);
    finish(&mut app);
    assert!(app.figure_export.is_none(), "{}", app.status);
    assert_eq!(bytes, std::fs::read(dir.path().join("figure-2.eps")).unwrap());
    assert!(std::fs::read(dir.path().join("figure-1.eps")).unwrap().starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    assert_eq!(app.current_figure, 2);
    assert_eq!(app.figure_documents, statuses);
    assert!(payload.same_snapshot(&app.variables["keep"]));
    for (id, before) in originals { assert!(before.same_snapshot(&app.all_figures[&id])); }
}
