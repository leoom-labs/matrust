use matrust::{
    Engine,
    document::Document,
    files,
    session::{Session, SessionStore, figures::*},
    simulation::Model,
};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

fn snapshot() -> Session {
    let mut engine = Engine::default();
    engine.execute("figure(3);plot([1 2 3],[1 4 9],'r--');title('Recovered line');xlabel('Time');ylabel('Value');legend('Measurements');figure(9);bar([3 -1 2]);title('Recovered bars');").unwrap();
    let mut notes = Document::new();
    notes.text = "unsaved_notes = 42;".into();
    notes.changed();
    let figures = FigureSession {
        current: 9,
        figures: engine
            .figures()
            .map(|(id, f)| FigureDocument {
                window: None,
                id,
                figure: f.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: Some(FigureView {
                    bounds: [1., 3., -2., 10.],
                    auto: [false, true],
                    dirty: true,
                }),
            })
            .collect(),
    };
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: None,
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        version: 1,
        documents: vec![notes],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
        layout: Default::default(),
        figure_manifest: None,
        figures: Some(figures),
    }
}
fn manifest(path: &Path) -> FigureManifest {
    Session::load(path).unwrap().figure_manifest.unwrap()
}
fn names(path: &Path) -> Vec<String> {
    manifest(path).entries.into_iter().map(|e| e.file).collect()
}
fn files_in(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(figure_directory(path).unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
#[test]
fn checkpoints_preserve_figures_views_file_identity_and_nan_bits_without_touching_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let source = dir.path().join("original.mfig");
    let mut state = snapshot();
    let figs = state.figures.as_mut().unwrap();
    let first = &mut figs.figures[0];
    files::save_figure(&source, &first.figure).unwrap();
    let original = std::fs::read(&source).unwrap();
    first.path = Some(source.clone());
    first.figure.title = "Unsaved changes".into();
    first.figure.series[0].visible = false;
    Arc::make_mut(
        first.figure.series[0]
            .points
            .dense_mut()
            .expect("dense test fixture"),
    )[1][1] = f64::from_bits(0x7ff8000000000017);
    state.clone().save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.documents[0].text, "unsaved_notes = 42;");
    assert!(loaded.documents[0].dirty);
    let result = restore_figures(
        &path,
        loaded.figure_manifest.as_ref().unwrap(),
        &AtomicBool::new(false),
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.figures.current, 9);
    for (expected, actual) in state
        .figures
        .unwrap()
        .figures
        .iter()
        .zip(&result.figures.figures)
    {
        assert_eq!(expected.id, actual.id);
        assert_eq!(expected.path, actual.path);
        assert_eq!(expected.dirty, actual.dirty);
        assert_eq!(expected.view, actual.view);
        let expected_path = dir.path().join("expected.mfig");
        let actual_path = dir.path().join("actual.mfig");
        files::save_figure(&expected_path, &expected.figure).unwrap();
        files::save_figure(&actual_path, &actual.figure).unwrap();
        assert_eq!(
            std::fs::read(expected_path).unwrap(),
            std::fs::read(actual_path).unwrap()
        );
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
}
#[test]
fn checkpoints_reuse_unchanged_data_release_old_buffers_and_clean_only_owned_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let mut state = snapshot();
    store.save(state.clone()).unwrap();
    let initial = names(&path);
    assert_eq!(
        (state.figures.as_ref().unwrap().figures[0].figure.series[0].points).strong_count(),
        1
    );
    state.documents[0].text.push_str("\nmore_notes=7;");
    state.figures.as_mut().unwrap().figures[0]
        .view
        .as_mut()
        .unwrap()
        .bounds[1] = 2.;
    store.save(state.clone()).unwrap();
    assert_eq!(names(&path), initial);
    let old = (state.figures.as_ref().unwrap().figures[0].figure.series[0].points).downgrade();
    Arc::make_mut(
        state.figures.as_mut().unwrap().figures[0].figure.series[0]
            .points
            .dense_mut()
            .expect("dense test fixture"),
    )[0][1] = 99.;
    assert!(old.upgrade().is_none(), "cache retains old plot data");
    store.save(state.clone()).unwrap();
    let changed = names(&path);
    assert_ne!(changed[0], initial[0]);
    assert_eq!(changed[1], initial[1]);
    let data_dir = figure_directory(&path).unwrap();
    std::fs::write(data_dir.join("figure-0123456789ABCDEF.mfig"), b"orphan").unwrap();
    std::fs::write(data_dir.join("user-note.txt"), b"keep").unwrap();
    std::fs::write(data_dir.join("figure-user.mfig"), b"keep").unwrap();
    store.save(state).unwrap();
    assert!(!data_dir.join("figure-0123456789ABCDEF.mfig").exists());
    assert!(!data_dir.join(&initial[0]).exists());
    assert!(data_dir.join("user-note.txt").exists());
    assert!(data_dir.join("figure-user.mfig").exists());
    assert_eq!(names(&path), changed);
}
#[test]
fn failed_manifest_serialization_preserves_previous_checkpoint_and_removes_staged_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let state = snapshot();
    store.save(state.clone()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let before_files = files_in(&path);
    let mut failing = state.clone();
    failing.figures.as_mut().unwrap().figures[0].figure.title =
        "New data must not replace checkpoint".into();
    failing.documents = (0..17)
        .map(|_| {
            let mut d = Document::new();
            d.text = "\"".repeat(1_000_000);
            d.changed();
            d
        })
        .collect();
    assert!(store.save(failing).unwrap_err().contains("32 MiB"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(files_in(&path), before_files);
    store.save(state).unwrap();
    assert_eq!(files_in(&path), before_files);
}
#[test]
fn corrupt_missing_unsafe_and_cancelled_recovery_keep_other_figures_and_notes_available() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    snapshot().save(&path).unwrap();
    let original = manifest(&path);
    let data_dir = figure_directory(&path).unwrap();
    let broken = data_dir.join(&original.entries[0].file);
    let mut bytes = std::fs::read(&broken).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&broken, &bytes).unwrap();
    let report = restore_figures(&path, &original, &AtomicBool::new(false));
    assert_eq!(report.figures.figures.len(), 1);
    assert!(report.warnings[0].contains("checksum"));
    std::fs::remove_file(broken).unwrap();
    let report = restore_figures(&path, &original, &AtomicBool::new(false));
    assert_eq!(report.figures.figures.len(), 1);
    assert!(!report.warnings.is_empty());
    let mut unsafe_manifest = original.clone();
    unsafe_manifest.entries[0].file = "../outside.mfig".into();
    let report = restore_figures(&path, &unsafe_manifest, &AtomicBool::new(false));
    assert!(report.warnings[0].contains("Invalid figure recovery reference"));
    assert!(
        !restore_figures(&path, &original, &AtomicBool::new(true))
            .warnings
            .is_empty()
    );
    let mut json = serde_json::to_value(Session::load(&path).unwrap()).unwrap();
    json["figure_manifest"] = serde_json::json!({"entries":"invalid"});
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let restored = Session::load(&path).unwrap();
    assert_eq!(restored.documents[0].text, "unsaved_notes = 42;");
    assert!(
        !restore_figures(
            &path,
            restored.figure_manifest.as_ref().unwrap(),
            &AtomicBool::new(false)
        )
        .warnings
        .is_empty()
    );
}
#[test]
fn recovery_bounds_and_engine_installation_are_transactional() {
    let mut state = snapshot();
    let figures = state.figures.take().unwrap();
    let mut e = Engine::default();
    e.execute("unchanged=7;plot([8 9]);").unwrap();
    let original = e.figure.clone().unwrap();
    assert!(
        e.restore_figures(100, vec![(3, figures.figures[0].figure.clone())])
            .is_err()
    );
    assert!(e.figure.as_ref().unwrap().same_snapshot(&original));
    let pair = (3, figures.figures[0].figure.clone());
    assert!(
        e.restore_figures(3, vec![pair.clone(), pair.clone()])
            .is_err()
    );
    assert!(e.restore_figures(3, vec![pair; 17]).is_err());
    assert!(e.figure.as_ref().unwrap().same_snapshot(&original));
    e.restore_figures(
        figures.current,
        figures
            .figures
            .iter()
            .map(|f| (f.id, f.figure.clone()))
            .collect(),
    )
    .unwrap();
    assert_eq!(e.figure_ids(), vec![3, 9]);
    assert_eq!(e.variables["unchanged"].data(), &[7.]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut invalid = figures.clone();
    invalid.figures[0].view.as_mut().unwrap().bounds[0] = f64::NAN;
    state.figures = Some(invalid);
    assert!(state.save(&path).is_err());
    assert!(!path.exists());
    let manifest = FigureManifest {
        version: 1,
        current: 3,
        entries: (0..17)
            .map(|id| FigureReference {
                window: Default::default(),
                id,
                file: "figure-0123456789ABCDEF.mfig".into(),
                bytes: 1,
                crc32: 0,
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            })
            .collect(),
        error: None,
    };
    assert!(
        !restore_figures(&path, &manifest, &AtomicBool::new(false))
            .warnings
            .is_empty()
    );
}

#[test]
fn invalid_optional_window_metadata_preserves_real_figures_workspace_and_warning_policy() {
    use matrust::session::figure_window::{FigureWindow, NormalWindow};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("placement.json");
    let mut state = snapshot();
    let mut engine = Engine::default();
    engine.execute("A = [1 2 3]; alias = A;").unwrap();
    state.workspace = Some(engine.workspace());
    SessionStore::new(&path).save(state.clone()).unwrap();
    let pristine: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(
        pristine["figure_manifest"]["entries"][0]
            .get("window")
            .is_none()
    );
    let payloads: Vec<_> = std::fs::read_dir(figure_directory(&path).unwrap())
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let bytes = std::fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect();
    let accepted = serde_json::to_value(
        FigureWindow::new(
            true,
            Some(NormalWindow {
                inner_size_dip: [800., 600.],
                outer_rect_px: [100., 100., 900., 700.],
                work_area_px: [0., 0., 1920., 1040.],
                monitor_device: None,
            }),
            false,
        )
        .unwrap(),
    )
    .unwrap();
    let mut deep = serde_json::json!(null);
    for _ in 0..20 {
        deep = serde_json::json!([deep]);
    }
    for (optional, resets) in [
        (serde_json::Value::Null, false),
        (accepted.clone(), false),
        (serde_json::json!({"version":2}), true),
        (serde_json::json!({"unknown":"x".repeat(4096)}), true),
        (deep, true),
        (serde_json::json!([1, 2, 3]), true),
    ] {
        let mut changed = pristine.clone();
        changed["figure_manifest"]["entries"][0]["window"] = optional;
        std::fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        let loaded = Session::load(&path).unwrap();
        let manifest = loaded.figure_manifest.as_ref().unwrap();
        assert!(manifest.error.is_none());
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.entries[0].window.was_reset(), resets);
        let restored = restore_figures(&path, manifest, &AtomicBool::new(false));
        assert!(restored.warnings.is_empty(), "{:?}", restored.warnings);
        assert_eq!(restored.placement_notes.len(), usize::from(resets));
        assert_eq!(restored.figures.current, 9);
        assert_eq!(restored.figures.figures.len(), 2);
        for (expected, actual) in state
            .figures
            .as_ref()
            .unwrap()
            .figures
            .iter()
            .zip(&restored.figures.figures)
        {
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.figure.title, expected.figure.title);
            assert_eq!(actual.figure.series.len(), expected.figure.series.len());
            for (a, b) in actual.figure.series.iter().zip(&expected.figure.series) {
                assert_eq!(a.points.len(), b.points.len());
                for (a, b) in a.points.iter().flatten().zip(b.points.iter().flatten()) {
                    assert_eq!(a.to_bits(), b.to_bits());
                }
            }
        }
        let workspace = matrust::session::workspace::restore_with_figures(
            &path,
            loaded.workspace_manifest.as_ref().unwrap(),
            restored.workspace_bindings(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(workspace.warnings.is_empty());
        assert!(workspace.values["A"].same_snapshot(&workspace.values["alias"]));
        let matrust::Array::Numeric(a) = &workspace.values["A"] else {
            panic!("numeric recovery expected");
        };
        assert_eq!(a.data(), &[1., 2., 3.]);
        for (file, expected) in &payloads {
            assert_eq!(&std::fs::read(file).unwrap(), expected);
        }
    }
    // Required manifest failures retain their existing warning behavior.
    let mut changed = pristine;
    changed["figure_manifest"]["entries"][0]["id"] = serde_json::json!("wrong type");
    std::fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert!(loaded.figure_manifest.unwrap().error.is_some());
}

#[test]
fn window_metadata_writeback_reuses_payload_and_restores_with_fresh_figure_identity() {
    use matrust::session::figure_window::{FigureWindow, NormalWindow};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("windows.json");
    let mut state = snapshot();
    let figure = state.figures.as_ref().unwrap().figures[0].figure.clone();
    let normal = NormalWindow {
        inner_size_dip: [800., 600.],
        outer_rect_px: [-1500., 100., -700., 700.],
        work_area_px: [-1920., 0., 0., 1040.],
        monitor_device: Some("left".into()),
    };
    let first = FigureWindow::new(true, Some(normal.clone()), false).unwrap();
    state.figures.as_mut().unwrap().figures[0].window = Some(first.clone());
    let mut store = SessionStore::new(&path);
    store.save(state.clone()).unwrap();
    let before = manifest(&path);
    let files_before = files_in(&path);
    let payload = std::fs::read(
        figure_directory(&path)
            .unwrap()
            .join(&before.entries[0].file),
    )
    .unwrap();
    let mut normal = normal;
    normal.outer_rect_px = [-1400., 120., -600., 720.];
    let second = FigureWindow::new(false, Some(normal), true).unwrap();
    state.figures.as_mut().unwrap().figures[0].window = Some(second.clone());
    store.save(state.clone()).unwrap();
    let after = manifest(&path);
    assert_eq!(files_in(&path), files_before);
    for (a, b) in before.entries.iter().zip(&after.entries) {
        assert_eq!((&a.file, a.bytes, a.crc32), (&b.file, b.bytes, b.crc32));
        assert_eq!(a.dirty, b.dirty);
        assert_eq!(a.view, b.view);
    }
    assert_eq!(before.entries[0].window.as_ref(), Some(&first));
    assert_eq!(after.entries[0].window.as_ref(), Some(&second));
    assert_eq!(
        std::fs::read(
            figure_directory(&path)
                .unwrap()
                .join(&after.entries[0].file)
        )
        .unwrap(),
        payload
    );
    let restored = restore_figures(&path, &after, &AtomicBool::new(false));
    assert!(restored.warnings.is_empty() && restored.placement_notes.is_empty());
    assert_eq!(restored.figures.figures[0].window.as_ref(), Some(&second));
    assert!(
        !restored.figures.figures[0]
            .figure
            .identity
            .same_object(&figure.identity)
    );
    assert!(
        (figure.series[0].points)
            .same_storage(&state.figures.as_ref().unwrap().figures[0].figure.series[0].points)
    );
    let committed = std::fs::read(&path).unwrap();
    state.figures.as_mut().unwrap().figures[0].window = None;
    assert!(
        store
            .save_with_cancel(state.clone(), &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), committed);
    assert_eq!(files_in(&path), files_before);
    store.save(state).unwrap();
    let cleared = manifest(&path);
    assert!(cleared.entries[0].window.is_absent());
    assert_eq!(cleared.entries[0].file, after.entries[0].file);
    assert_eq!(cleared.entries[0].crc32, after.entries[0].crc32);
}
