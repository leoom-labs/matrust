use super::*;
fn frame(app: &mut Workspace, ctx: &egui::Context, paths: Vec<PathBuf>) {
    let _ = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            dropped_files: paths
                .into_iter()
                .map(|path| crate::ui_tests::dropped_file(path))
                .collect(),
            ..Default::default()
        },
        |root| app.ui(root),
    );
}
fn finish(app: &mut Workspace, ctx: &egui::Context) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while (app.busy || !app.open_queue.is_empty()) && std::time::Instant::now() < deadline {
        frame(app, ctx, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.busy && app.open_queue.is_empty(), "{}", app.status);
}
#[test]
fn figure_reopen_ui_repeated_drops_select_open_native_and_matlab_figures_with_edits_retained() {
    for ext in ["mfig", "fig"] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("saved.{ext}"));
        let mut e = matrust::Engine::default();
        e.execute("plot([1 2],[3 4]);title('Disk');").unwrap();
        matrust::files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
        let mut app = Workspace::with_context(ctx.clone(), true);
        frame(
            &mut app,
            &ctx,
            vec![
                path.clone(),
                dir.path().join(".").join(format!("saved.{ext}")),
            ],
        );
        finish(&mut app, &ctx);
        assert_eq!(app.figure_ids, vec![1]);
        app.run("keep=[1 2];title('Local');figure(2);plot([5 6]);".into());
        finish(&mut app, &ctx);
        let local = app.all_figures[&1].clone();
        let other = app.all_figures[&2].clone();
        let variable = app.variables["keep"].clone();
        std::fs::write(&path, b"broken external replacement").unwrap();
        app.layout.mode = layout::PaneMode::Editor;
        frame(&mut app, &ctx, vec![path]);
        finish(&mut app, &ctx);
        assert_eq!(app.figure_ids, vec![1, 2]);
        assert_eq!(app.current_figure, 1);
        // The figure is revealed in its own column beside the Editor.
        assert!(app.layout.mode != layout::PaneMode::Editor || app.figure_column());
        assert!(app.figure.as_ref().unwrap().same_snapshot(&local));
        assert!(app.figure_dirty(1));
        assert!(app.all_figures[&2].same_snapshot(&other));
        assert!(app.variables["keep"].same_snapshot(&variable));
        assert!(
            app.console
                .iter()
                .any(|line| line.contains("Selected open figure 1"))
        );
    }
}
