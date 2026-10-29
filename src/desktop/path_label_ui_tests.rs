#[cfg(windows)]
#[test]
fn readable_figure_paths_keep_real_worker_identity_and_painted_caption() {
    fn texts(shape: &egui::Shape, found: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => found.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => for shape in shapes { texts(shape, found); },
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    assert!(root.to_str().unwrap().starts_with(r"\\?\"));
    let path = root.join("readable.mfig");
    let expected = path.to_str().unwrap().strip_prefix(r"\\?\").unwrap().to_owned();
    let mut app = figure_save_all_app(&ctx, &root, 1);
    let variable = app.variables["keep"].clone();
    let original = app.figure.as_ref().unwrap().clone();
    app.submit(Job::SaveFigureById(1, path.clone(), None));
    finish(&mut app);
    assert!(app.console.iter().any(|line| line == &format!("Saved editable figure {expected}")));
    assert_eq!(app.figure_documents[&1].path.as_ref(), Some(&path));
    let saved_bytes = std::fs::read(&path).unwrap();
    app.submit(Job::OpenFigure(path.clone()));
    finish(&mut app);
    let id = app.current_figure;
    assert_ne!(id, 1);
    assert!(app.console.iter().any(|line| line == &format!("Opened figure {expected}")));
    assert_eq!(app.figure_documents[&id].path.as_ref(), Some(&path));
    app.submit(Job::ReloadFigure(id, path.clone()));
    finish(&mut app);
    assert!(app.console.iter().any(|line| line == &format!("Reloaded figure {expected}")));
    assert_eq!(app.figure_documents[&id].path.as_ref(), Some(&path));
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    assert!(original.same_snapshot(&app.all_figures[&1]));
    assert!(variable.same_snapshot(&app.variables["keep"]));
    assert_eq!(app.all_figures[&id].svg(), original.svg());
    let details = app.figure_choice_label(id).1;
    assert!(details.contains(&expected) && !details.contains(r"\\?\"));
    app.show_plot_pane();
    app.layout.mode = session::PaneMode::Figure;
    let documents = app.figure_documents.clone();
    let history = app.figure_history.clone();
    for _ in 0..3 { frame(&mut app, &ctx, vec![]); }
    let output = frame(&mut app, &ctx, vec![]);
    let mut labels = Vec::new();
    for shape in &output.shapes { texts(&shape.shape, &mut labels); }
    assert!(labels.iter().any(|label| label == &expected), "{labels:?}");
    assert!(labels.iter().all(|label| !label.contains(r"\\?\")));
    assert_eq!(app.figure_documents, documents);
    assert_eq!(app.figure_history, history);
    assert!(variable.same_snapshot(&app.variables["keep"]));
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
}

#[cfg(windows)]
#[test]
fn readable_preview_error_keeps_exact_missing_path_without_creating_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("missing.png");
    let original = path.clone();
    let expected = path.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
    let error = image_load::read_file(&path, 1024, &std::sync::atomic::AtomicBool::new(false)).unwrap_err();
    assert!(error.starts_with(&format!("{expected}: ")));
    assert!(!error.contains(r"\\?\"));
    assert_eq!(path, original);
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
