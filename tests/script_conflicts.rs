use matrust::document::{DiskVersion, Document, SaveOutcome, same_path};
use std::{fs, path::Path};

fn edited(path: &Path) -> Document {
    fs::write(path, "x = 1;\n").unwrap();
    let mut d = Document::open(path).unwrap();
    d.text = "x = 2;\n".into();
    d.changed();
    d
}

#[test]
fn changed_scripts_require_current_content_confirmation_even_with_same_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.m");
    let mut d = edited(&path);
    let timestamp = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "x = 3;\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(timestamp))
        .unwrap();
    let SaveOutcome::Conflict(first) = d.try_save(&path, None).unwrap() else {
        panic!("missing conflict")
    };
    assert_eq!(first, DiskVersion::Text("x = 3;\n".into()));
    assert!(d.dirty);
    assert!(d.save(&path).is_err());
    fs::write(&path, "x = 4;\n").unwrap();
    let SaveOutcome::Conflict(second) = d.try_save(&path, Some(&first)).unwrap() else {
        panic!("stale confirmation accepted")
    };
    assert_eq!(fs::read_to_string(&path).unwrap(), "x = 4;\n");
    assert_eq!(
        d.try_save(&path, Some(&second)).unwrap(),
        SaveOutcome::Saved
    );
    assert!(!d.dirty);
    assert_eq!(fs::read_to_string(&path).unwrap(), "x = 2;\n");
}

#[test]
fn deleted_scripts_are_not_silently_recreated_and_reload_is_transactional() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.m");
    let mut d = edited(&path);
    let id = d.id;
    fs::remove_file(&path).unwrap();
    assert!(same_path(d.path.as_ref().unwrap(), &path));
    assert_eq!(
        d.try_save(&path, None).unwrap(),
        SaveOutcome::Conflict(DiskVersion::Missing)
    );
    assert!(d.reload().is_err());
    assert_eq!(d.text, "x = 2;\n");
    assert!(d.dirty && !path.exists());
    fs::write(&path, "replacement = 7;\n").unwrap();
    assert!(matches!(
        d.try_save(&path, Some(&DiskVersion::Missing)).unwrap(),
        SaveOutcome::Conflict(DiskVersion::Text(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "replacement = 7;\n");
    fs::remove_file(&path).unwrap();
    assert_eq!(
        d.try_save(&path, Some(&DiskVersion::Missing)).unwrap(),
        SaveOutcome::Saved
    );
    fs::write(&path, "x = 8;\n").unwrap();
    d.reload().unwrap();
    assert_eq!(d.id, id);
    assert_eq!(d.text, "x = 8;\n");
    assert!(!d.dirty);
}

#[test]
fn unreadable_or_oversized_disk_versions_and_failed_save_as_preserve_both_copies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.m");
    let mut d = edited(&path);
    let identity = d.path.clone();
    for bytes in [vec![0xff, 0xfe], vec![b'x'; 1_000_001]] {
        fs::write(&path, &bytes).unwrap();
        assert!(d.try_save(&path, None).is_err());
        assert!(d.reload().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(d.text, "x = 2;\n");
        assert!(d.dirty);
    }
    assert!(d.try_save(&path, None).unwrap_err().contains("1 MB"));
    assert!(d.save(&dir.path().join("missing/child.m")).is_err());
    assert_eq!(d.path, identity);
    let copy = dir.path().join("copy.m");
    d.save(&copy).unwrap();
    assert!(same_path(d.path.as_ref().unwrap(), &copy));
    assert_eq!(fs::read_to_string(copy).unwrap(), "x = 2;\n");
    assert_eq!(fs::metadata(path).unwrap().len(), 1_000_001);
}

#[test]
fn recovered_saved_baseline_protects_external_edits_and_bom_text_is_normalized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.m");
    let d = edited(&path);
    let checkpoint = serde_json::to_string(&d).unwrap();
    fs::write(&path, "outside = 9;\n").unwrap();
    let mut restored: Document = serde_json::from_str(&checkpoint).unwrap();
    restored.validate_recovery().unwrap();
    assert!(matches!(
        restored.try_save(&path, None).unwrap(),
        SaveOutcome::Conflict(_)
    ));
    fs::write(&path, "\u{feff}\u{feff}x = 1;\n").unwrap();
    assert_eq!(restored.try_save(&path, None).unwrap(), SaveOutcome::Saved);
    restored.text.push_str("next = 3;\n");
    restored.changed();
    restored.save(&path).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), restored.text);
}
