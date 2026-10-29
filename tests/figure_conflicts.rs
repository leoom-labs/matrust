use matrust::{
    Engine,
    files::{
        self,
        figure_version::{self as tracked, DiskVersion, SaveOutcome},
    },
};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

#[test]
fn figure_fingerprints_match_decoded_and_published_bytes_and_recheck_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine
        .execute("plot([1 2 3],[2 4 8]);title('First');")
        .unwrap();
    for extension in ["mfig", "fig"] {
        let path = dir.path().join(format!("plot.{extension}"));
        let original = engine.figure.clone().unwrap();
        let SaveOutcome::Saved(baseline) = tracked::save(&path, &original, None, &cancel).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            tracked::inspect(&path, &cancel).unwrap(),
            DiskVersion::Present(baseline)
        );
        let (loaded, opened) = tracked::load(&path, &cancel).unwrap();
        assert_eq!(loaded.series[0].points, original.series[0].points);
        assert_eq!(opened, baseline);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut external = original.clone();
        external.title = "Other".into();
        files::save_figure(&path, &external).unwrap();
        // Same byte count and timestamp cannot hide changed data.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), baseline.bytes);
        let SaveOutcome::Conflict(observed) = tracked::save(
            &path,
            &original,
            Some(DiskVersion::Present(baseline)),
            &cancel,
        )
        .unwrap() else {
            panic!("Expected conflict")
        };
        assert_eq!(files::load_figure(&path).unwrap().title, "Other");
        external.title = "Third".into();
        files::save_figure(&path, &external).unwrap();
        let SaveOutcome::Conflict(latest) =
            tracked::save(&path, &original, Some(observed), &cancel).unwrap()
        else {
            panic!("Expected stale confirmation")
        };
        let SaveOutcome::Saved(saved) =
            tracked::save(&path, &original, Some(latest), &cancel).unwrap()
        else {
            panic!()
        };
        assert_eq!(saved, baseline);
        assert_eq!(files::load_figure(&path).unwrap().title, "First");
    }
}

#[test]
fn figure_missing_corrupt_oversized_cancelled_and_failed_saves_preserve_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine.execute("plot([1 2 3]);").unwrap();
    let f = engine.figure.unwrap();
    assert!(matches!(
        tracked::save(&path, &f, Some(DiskVersion::Unknown), &cancel).unwrap(),
        SaveOutcome::Conflict(DiskVersion::Missing)
    ));
    assert!(!path.exists());
    let SaveOutcome::Saved(v) =
        tracked::save(&path, &f, Some(DiskVersion::Missing), &cancel).unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        tracked::save(&path, &f, Some(DiskVersion::Missing), &cancel).unwrap(),
        SaveOutcome::Conflict(_)
    ));
    assert!(matches!(
        tracked::save(&path, &f, Some(DiskVersion::Unknown), &cancel).unwrap(),
        SaveOutcome::Conflict(_)
    ));
    let before = fs::read(&path).unwrap();
    cancel.store(true, Ordering::Relaxed);
    assert!(tracked::load(&path, &cancel).is_err());
    assert!(tracked::inspect(&path, &cancel).is_err());
    assert!(tracked::save(&path, &f, Some(DiskVersion::Present(v)), &cancel).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    cancel.store(false, Ordering::Relaxed);
    fs::write(&path, b"corrupt").unwrap();
    assert!(tracked::load(&path, &cancel).is_err());
    assert!(matches!(
        tracked::save(&path, &f, Some(DiskVersion::Present(v)), &cancel).unwrap(),
        SaveOutcome::Conflict(_)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"corrupt");
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(17_000_001)
        .unwrap();
    assert!(tracked::inspect(&path, &cancel).is_err());
    assert!(tracked::save(&path, &f, Some(DiskVersion::Present(v)), &cancel).is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), 17_000_001);
    assert!(tracked::save(&dir.path().join("missing/copy.mfig"), &f, None, &cancel).is_err());
    assert!(tracked::save(&dir.path().join("wrong.png"), &f, None, &cancel).is_err());
    assert!(!dir.path().join("wrong.png").exists());
}

#[test]
fn figure_recovery_preserves_original_disk_fingerprint_and_accepts_old_manifests() {
    use matrust::session::{
        Session,
        figures::{FigureDocument, FigureSession, SessionStore, restore_figures},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let session_path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine.execute("plot([1 2 3]);").unwrap();
    let figure = engine.figure.unwrap();
    let SaveOutcome::Saved(v) = tracked::save(&path, &figure, None, &cancel).unwrap() else {
        panic!()
    };
    let mut session: Session =
        serde_json::from_slice(&fs::read("tests/fixtures/ui/session.json").unwrap()).unwrap();
    session.figures = Some(FigureSession {
        current: 1,
        figures: vec![FigureDocument {
            window: None,
            id: 1,
            figure: figure.clone(),
            path: Some(path.clone()),
            source_version: Some(v),
            dirty: true,
            view: None,
        }],
    });
    let mut store = SessionStore::new(&session_path);
    store.save(session.clone()).unwrap();
    let mut external = figure;
    external.title = "External edit".into();
    files::save_figure(&path, &external).unwrap();
    let saved: Session = serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
    let mut manifest = saved.figure_manifest.unwrap();
    let restored = restore_figures(&session_path, &manifest, &cancel);
    assert!(restored.warnings.is_empty());
    assert_eq!(restored.figures.figures[0].source_version, Some(v));
    // Checkpoint cache reuse must update fingerprint metadata too.
    session.figures.as_mut().unwrap().figures[0].source_version = None;
    store.save(session).unwrap();
    let saved: Session = serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
    assert!(
        saved.figure_manifest.unwrap().entries[0]
            .source_version
            .is_none()
    );
    manifest.entries[0].source_version = None;
    let mut old = serde_json::to_value(&manifest).unwrap();
    old["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_version");
    let old = serde_json::from_value(old).unwrap();
    let restored = restore_figures(&session_path, &old, &cancel);
    assert!(restored.warnings.is_empty());
    assert!(restored.figures.figures[0].source_version.is_none());
}
