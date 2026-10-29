#[test]
fn native_extension_open_kinds_detect_mr_scripts_and_mrfig_figures() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    // Distinct stems: X.MR and x.mr are one file on Windows.
    for (name, script) in [
        ("x.mr", true),
        ("Y.MR", true),
        ("z.m", true),
        ("w.mat", false),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, "value=1;\n").unwrap();
        let request = open_files::Request {
            path,
            scripts: vec![],
            tabs: 0,
        };
        match open_files::prepare(request, &cancel) {
            Ok(open_files::Prepared::Script(document)) => {
                assert!(script, "{name}");
                assert_eq!(document.text, "value=1;\n");
            }
            Ok(open_files::Prepared::File(_, open_files::Kind::Workspace)) => {
                assert!(!script, "{name}");
            }
            _ => panic!("{name}: unexpected open kind"),
        }
    }
    for name in ["n.mrfig", "o.MRFIG", "l.mfig"] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"not read during preflight").unwrap();
        let request = open_files::Request {
            path,
            scripts: vec![],
            tabs: 0,
        };
        assert!(
            matches!(
                open_files::prepare(request, &cancel),
                Ok(open_files::Prepared::File(_, open_files::Kind::Figure))
            ),
            "{name}"
        );
    }
}

#[test]
fn native_mr_script_opens_in_a_tab_without_running() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("native.mr"), "should_not_run=5;\n").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    let count = app.documents.len();
    app.open_path("native.mr".into());
    finish(&mut app);
    assert_eq!(app.documents.len(), count + 1);
    assert_eq!(app.documents[app.active].text, "should_not_run=5;\n");
    assert_eq!(app.documents[app.active].title(), "native.mr");
    assert!(!app.variables.contains_key("should_not_run"));
}

#[test]
fn new_scripts_default_to_mr_and_saving_a_loaded_m_keeps_its_path() {
    assert_eq!(Document::new().title(), "Untitled.mr");
    let (filters, name) = script_save_choices(None);
    assert_eq!(name, "untitled.mr");
    assert_eq!(filters.map(|(_, e)| e), [&["mr"][..], &["m"][..]]);
    assert_eq!(filters[0].0, "MatRust script (*.mr)");
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.m");
    std::fs::write(&path, "kept=1;\n").unwrap();
    let mut app = folder_navigation_app(&ctx, dir.path());
    app.open_path("keep.m".into());
    finish(&mut app);
    let index = app.active;
    // Save As suggests the same .m name with the .m filter (default extension) first.
    let (filters, name) = script_save_choices(app.documents[index].path.as_deref());
    assert_eq!(name, "keep.m");
    assert_eq!(filters.map(|(_, e)| e), [&["m"][..], &["mr"][..]]);
    app.documents[index].text = "kept=2;\n".into();
    app.documents[index].changed();
    app.script_io.save_picker = Some(|_, _| panic!("an existing .m path must not prompt"));
    // Success is published by the background writer, not the return value.
    app.save_document(index, false);
    finish(&mut app);
    assert_eq!(app.script_io.save_picker_calls, 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept=2;\n");
    assert!(!dir.path().join("keep.mr").exists());
    let saved = app.documents[index].path.as_deref().unwrap();
    assert!(matrust::document::same_path(saved, &path));
    assert_eq!(saved.extension().unwrap(), "m");
    assert!(!app.documents[index].dirty);
}

#[test]
fn figure_save_suggests_mrfig_and_never_renames_an_existing_native_figure() {
    use figure_files::figure_save_name;
    use std::path::Path;
    assert_eq!(figure_save_name(3, None, false), "figure-3.mrfig");
    assert_eq!(figure_save_name(3, None, true), "figure-3.mrfig");
    let legacy = Path::new("dir").join("old.mfig");
    assert_eq!(figure_save_name(3, Some(&legacy), true), "old.mfig");
    let native = Path::new("dir").join("new.mrfig");
    assert_eq!(figure_save_name(3, Some(&native), true), "new.mrfig");
    let matlab = Path::new("dir").join("plot.fig");
    assert_eq!(figure_save_name(3, Some(&matlab), false), "plot.fig");
    // A MATLAB .fig that cannot hold the figure falls back to native .mrfig.
    assert_eq!(figure_save_name(3, Some(&matlab), true), "plot.mrfig");
}
