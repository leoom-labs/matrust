#[test]
fn batch_export_ui_reviews_files_and_preserves_figures_views_and_workspace() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 2);
    app.export_settings.format = figure_export::Format::Svg;
    let bounds = [1.1, 1.9, 1.2, 1.8];
    app.figure_views.insert(
        1,
        figure_files::View {
            bounds,
            auto: [false; 2],
            dirty: true,
            revision: app.figure_documents[&1].revision,
        },
    );
    let original = app.all_figures.clone();
    let workspace = app.variables["keep"].clone();
    app.begin_figure_export();
    click_label(&mut app, &ctx, "All open figures (2)");
    app.prepare_export_folder(dir.path().into());
    finish(&mut app);
    assert!(!dir.path().join("figure-1.svg").exists());
    click_label(&mut app, &ctx, "Export all");
    finish(&mut app);
    assert!(app.figure_export.is_none(), "{}", app.status);
    for (id, before) in &original {
        assert!(before.same_snapshot(&app.all_figures[id]));
        assert!(app.figure_dirty(*id));
    }
    assert_eq!(app.current_figure, 2);
    assert!(workspace.same_snapshot(&app.variables["keep"]));
    assert_eq!(app.figure_views[&1].bounds, bounds);
    let mut expected = original[&1].clone();
    expected.xlim = Some([bounds[0], bounds[1]]);
    expected.ylim = Some([bounds[2], bounds[3]]);
    let reference = dir.path().join("reference.svg");
    matrust::export::export_vector_with_cancel(
        &expected,
        &reference,
        app.export_settings.vector,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(reference).unwrap(),
        std::fs::read(dir.path().join("figure-1.svg")).unwrap()
    );
}
#[test]
fn batch_export_ui_requires_replacement_and_recovers_from_stale_review() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 2);
    app.export_settings.format = figure_export::Format::Csv;
    let path = dir.path().join("figure-1.csv");
    std::fs::write(&path, b"old").unwrap();
    app.begin_figure_export();
    click_label(&mut app, &ctx, "All open figures (2)");
    app.prepare_export_folder(dir.path().into());
    finish(&mut app);
    click_label(&mut app, &ctx, "Export all");
    assert!(!app.busy);
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    click_label(&mut app, &ctx, "Replace existing files");
    std::fs::write(&path, b"new").unwrap();
    click_label(&mut app, &ctx, "Export all");
    finish(&mut app);
    let dialog = app.figure_export.as_ref().unwrap();
    assert!(
        dialog
            .error
            .as_ref()
            .unwrap()
            .contains("changed after review")
    );
    assert!(!dialog.saving);
    assert!(!dir.path().join("figure-2.csv").exists());
    app.prepare_export_folder(dir.path().into());
    finish(&mut app);
    click_label(&mut app, &ctx, "Replace existing files");
    click_label(&mut app, &ctx, "Export all");
    finish(&mut app);
    assert!(app.figure_export.is_none());
    assert_ne!(std::fs::read(&path).unwrap(), b"new");
}
#[test]
fn batch_export_compact_review_can_cancel_sixteen_figures_without_writes() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    for id in 1..=16 {
        app.run(format!("figure({id});plot([0 1],[0 1]);"));
        finish(&mut app);
    }
    app.export_settings.format = figure_export::Format::Csv;
    app.begin_figure_export();
    click_label_at(
        &mut app,
        &ctx,
        egui::vec2(960., 650.),
        "All open figures (16)",
    );
    app.prepare_export_folder(dir.path().into());
    finish(&mut app);
    click_label_at(&mut app, &ctx, egui::vec2(960., 650.), "Cancel");
    assert!(app.figure_export.is_none());
    assert_eq!(app.figure_ids.len(), 16);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn matlab_script_export_uses_worker_preserves_figures_and_reopens_as_unexecuted_text() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 2);
    let original = app.all_figures.clone();
    let workspace = app.variables.clone();
    app.export_settings.format = figure_export::Format::MatlabScript;
    app.begin_figure_export();
    // The Modal's first frame only measures its content (egui sizing pass).
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    assert!(output.shapes.iter().any(|s| {
        text_position(
            &s.shape,
            "Export original data and plotting commands for MATLAB.",
        )
        .is_some()
    }));
    let path = dir.path().join("plot_export.m");
    app.export_to_path(path.clone());
    finish(&mut app);
    assert!(app.figure_export.is_none(), "{}", app.status);
    let script = std::fs::read_to_string(&path).unwrap();
    assert!(script.starts_with("% MATLAB plot reconstruction"));
    assert!(script.contains("mr_figure = figure("));
    assert!(app.figure_dirty(2));
    for (id, before) in &original {
        assert!(before.same_snapshot(&app.all_figures[id]));
    }
    for (name, before) in &workspace {
        assert!(before.same_snapshot(&app.variables[name]));
    }
    app.open_path(path.clone());
    finish(&mut app);
    assert_eq!(app.documents[app.active].text, script);
    assert!(!app.variables.contains_key("mr_figure"));
    assert_eq!(app.all_figures.len(), original.len());
    assert_eq!(
        app.export_settings.format,
        figure_export::Format::MatlabScript
    );
}
