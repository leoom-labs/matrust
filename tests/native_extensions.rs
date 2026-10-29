//! Native .mr scripts/functions and .mrfig figures beside .m and legacy .mfig.
use matrust::{
    Engine, files,
    plot::Figure,
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, figure_directory, restore_figures},
    },
    simulation::Model,
};
use std::{path::Path, sync::atomic::AtomicBool};

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

/// Same-format re-encoding: equal bytes mean the decoded figures are identical.
fn encoded(figure: &Figure, dir: &Path, name: &str) -> Vec<u8> {
    let path = dir.join(name);
    files::save_figure(&path, figure).unwrap();
    std::fs::read(path).unwrap()
}

#[test]
fn folder_functions_prefer_mr_then_m_and_new_mr_wins_on_the_next_run() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("pick.m"), "function y=pick(x);y=x+1;end").unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    // Only pick.m: it still runs.
    e.execute("R=pick(1);").unwrap();
    assert_eq!(scalar(&e, "R"), 2.);
    // The cached pick.m resolution must not outlive the run once pick.mr exists.
    std::fs::write(
        folder.path().join("pick.mr"),
        "function y=pick(x);y=x+10;end",
    )
    .unwrap();
    e.execute("R=pick(1);").unwrap();
    assert_eq!(scalar(&e, "R"), 11.);
    // Both files: every call in one run uses pick.mr.
    e.execute("R=pick(1)+pick(2);").unwrap();
    assert_eq!(scalar(&e, "R"), 23.);
    std::fs::write(folder.path().join("native.mr"), "function y=native;y=5;end").unwrap();
    e.execute("R=native;").unwrap();
    assert_eq!(scalar(&e, "R"), 5.);
    // Removing pick.mr falls back to pick.m on the next run.
    std::fs::remove_file(folder.path().join("pick.mr")).unwrap();
    e.execute("R=pick(1);").unwrap();
    assert_eq!(scalar(&e, "R"), 2.);
    // The running document is searched before the folder's .mr and .m files.
    std::fs::write(
        folder.path().join("pick.mr"),
        "function y=pick(x);y=x+10;end",
    )
    .unwrap();
    e.execute("R=pick(1);function y=pick(x);y=x+100;end")
        .unwrap();
    assert_eq!(scalar(&e, "R"), 101.);
    // A broken .mr is reported, not silently skipped for a valid .m.
    std::fs::write(folder.path().join("wrong.mr"), "function y=other;y=1;end").unwrap();
    std::fs::write(folder.path().join("wrong.m"), "function y=wrong;y=1;end").unwrap();
    let error = e.execute("wrong();").unwrap_err();
    assert!(
        error.contains("wrong.mr") && error.contains("must be named 'wrong'"),
        "{error}"
    );
}

#[test]
fn cli_runs_an_mr_script_with_sibling_mr_and_m_functions() {
    let folder = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("scale.mr"),
        "function y=scale(x);y=x*7;end",
    )
    .unwrap();
    std::fs::write(
        folder.path().join("offset.m"),
        "function y=offset(x);y=x+1;end",
    )
    .unwrap();
    let script = folder.path().join("driver.mr");
    std::fs::write(&script, "R=offset(scale(6))\n").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg(&script)
        .arg("--threads")
        .arg("1")
        .current_dir(other.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("R =\n\n    43\n"));
}

#[test]
fn mrfig_saves_the_native_format_and_mfig_fixture_loads_identically() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "plot([1 2 3],[4 5 6]);savefig('native.mrfig');savefig('legacy.mfig');saveas(gcf,'via_saveas.mrfig');",
    )
    .unwrap();
    // The extension changes, the file content format does not.
    let native = std::fs::read(dir.path().join("native.mrfig")).unwrap();
    assert_eq!(
        native,
        std::fs::read(dir.path().join("legacy.mfig")).unwrap()
    );
    assert_eq!(
        native,
        std::fs::read(dir.path().join("via_saveas.mrfig")).unwrap()
    );
    let error = e.execute("savefig('plot.png');").unwrap_err();
    assert!(error.contains(".mrfig"), "{error}");
    // A committed legacy .mfig and its .mrfig copy decode identically.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui/signal.mfig");
    let copy = dir.path().join("signal.mrfig");
    std::fs::copy(&fixture, &copy).unwrap();
    let legacy = files::load_figure(&fixture).unwrap();
    let renamed = files::load_figure(&copy).unwrap();
    assert!(legacy.series_count() > 0);
    assert_eq!(
        encoded(&legacy, dir.path(), "a.mrfig"),
        encoded(&renamed, dir.path(), "b.mrfig")
    );
    // openfig accepts both extensions.
    for (name, series) in [("signal.mrfig", legacy.series_count()), ("legacy.mfig", 1)] {
        e.execute(&format!("close all;openfig('{name}');")).unwrap();
        assert_eq!(e.figure.as_ref().unwrap().series_count(), series, "{name}");
    }
}

fn session(e: &Engine) -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(e.workspace()),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: Some(FigureSession {
            current: e.current_figure,
            figures: e
                .figures()
                .map(|(id, f)| FigureDocument {
                    window: None,
                    id,
                    figure: f.clone(),
                    path: None,
                    source_version: None,
                    dirty: true,
                    view: None,
                })
                .collect(),
        }),
        layout: Default::default(),
        version: 1,
        documents: vec![matrust::document::Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    }
}

#[test]
fn recovery_writes_mrfig_and_still_restores_legacy_mfig_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = Engine::default();
    e.execute("plot([1 2 3],[3 1 2]);").unwrap();
    let original = e.figures().next().unwrap().1.clone();
    SessionStore::new(&path).save(session(&e)).unwrap();
    let figures = figure_directory(&path).unwrap();
    let names: Vec<String> = std::fs::read_dir(&figures)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    let native = &names[0];
    assert!(
        native.starts_with("figure-") && native.ends_with(".mrfig") && native.len() == 29,
        "{native}"
    );
    let saved = Session::load(&path).unwrap();
    let mut manifest = saved.figure_manifest.unwrap();
    assert_eq!(&manifest.entries[0].file, native);
    let cancel = AtomicBool::new(false);
    let restored = restore_figures(&path, &manifest, &cancel);
    assert!(restored.warnings.is_empty(), "{:?}", restored.warnings);
    let current = restored.figures.figures[0].figure.clone();
    // A recovery directory written before .mrfig keeps restoring.
    let legacy = native.replace(".mrfig", ".mfig");
    std::fs::rename(figures.join(native), figures.join(&legacy)).unwrap();
    manifest.entries[0].file = legacy;
    let restored = restore_figures(&path, &manifest, &cancel);
    assert!(restored.warnings.is_empty(), "{:?}", restored.warnings);
    assert_eq!(restored.figures.figures.len(), 1);
    let old = &restored.figures.figures[0].figure;
    assert_eq!(
        encoded(old, dir.path(), "old.mrfig"),
        encoded(&current, dir.path(), "current.mrfig")
    );
    assert_eq!(
        encoded(old, dir.path(), "old.mrfig"),
        encoded(&original, dir.path(), "original.mrfig")
    );
    // Names outside the owned pattern stay rejected.
    for bad in [
        "figure-0123456789abcdef.fig",
        "figure-short.mrfig",
        "../figure-0123456789abcdef.mrfig",
    ] {
        manifest.entries[0].file = bad.into();
        let restored = restore_figures(&path, &manifest, &cancel);
        assert!(restored.figures.figures.is_empty(), "{bad}");
        assert!(!restored.warnings.is_empty(), "{bad}");
    }
}
