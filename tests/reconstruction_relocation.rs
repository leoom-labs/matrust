use matrust::{
    Engine,
    document::{DiskVersion, Document, SaveOutcome},
    files,
    plot::{Figure, PlotKind, Series},
};
use std::{fs, path::Path, sync::atomic::AtomicBool};

fn fixture_bundle(directory: &Path) -> (Figure, files::MatlabPlotBundle) {
    let cancel = AtomicBool::new(false);
    let mut figure = Figure {
        axes: matrust::plot::Axes {
            title: "Relocated ' \u{03bb}\nsecond line".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    figure
        .add(vec![Series::new(
            vec![[1., -0.], [2., 0.25], [3., -9.]],
            PlotKind::Line,
        )])
        .unwrap();
    let bundle = files::write_matlab_plot_bundle_with_cancel(
        &directory.join("original.m"),
        &figure,
        &cancel,
    )
    .unwrap();
    (figure, bundle)
}

#[test]
fn save_as_copies_genuine_mat_and_replays_after_original_directory_is_gone() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("relocated \u{03bb} ' folder");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let (figure, bundle) = fixture_bundle(&source);
    let snapshot = figure.clone();
    let script = fs::read(&bundle.script).unwrap();
    let data = fs::read(&bundle.data).unwrap();
    let manifest = fs::read(bundle.companion_directory.join("reconstruction.json")).unwrap();
    let leaf = bundle.companion_directory.file_name().unwrap();
    let mut document = Document::open(&bundle.script).unwrap();
    let id = document.id;
    let path = target.join("renamed.m");
    document.save(&path).unwrap();
    assert_eq!(document.id, id);
    assert!(!document.dirty);
    assert_eq!(fs::read(&path).unwrap(), script);
    assert_eq!(fs::read(target.join(leaf).join("data.mat")).unwrap(), data);
    assert_eq!(
        fs::read(target.join(leaf).join("reconstruction.json")).unwrap(),
        manifest
    );
    assert_eq!(fs::read(&bundle.script).unwrap(), script);
    fs::remove_dir_all(&source).unwrap();
    let opened = Document::open(&path).unwrap();
    opened
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(root.path()).unwrap();
    engine.execute_document(&opened.text, &path).unwrap();
    let actual = engine.figure.as_ref().unwrap();
    assert_eq!(actual.title, figure.title);
    assert_eq!(actual.series.len(), 1);
    assert_eq!(actual.series[0].kind, PlotKind::Line);
    for (a, b) in actual.series[0]
        .points
        .iter()
        .zip(figure.series[0].points.iter())
    {
        assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
    }
    assert!(figure.same_snapshot(&snapshot));
    assert!(
        figure.series[0]
            .points
            .same_storage(&snapshot.series[0].points)
    );
    // Same-folder Save As reuses the validated generation, with no extra copy.
    let before = fs::read_dir(&target).unwrap().count();
    document.save(&target.join("again.m")).unwrap();
    assert_eq!(fs::read_dir(&target).unwrap().count(), before + 1);
}

#[test]
fn bom_crlf_recovery_and_modified_saved_origin_are_not_silently_rewritten() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let (_, bundle) = fixture_bundle(source.path());
    let text = fs::read_to_string(&bundle.script).unwrap();
    let crlf = text.replace('\n', "\r\n");
    fs::write(&bundle.script, format!("\u{feff}{crlf}")).unwrap();
    let original = Document::open(&bundle.script).unwrap();
    assert_eq!(original.text, crlf); // read_script strips BOM, retains CRLF.
    let encoded = serde_json::to_vec(&original).unwrap();
    let mut recovered: Document = serde_json::from_slice(&encoded).unwrap();
    recovered.validate_recovery().unwrap();
    recovered
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    let path = target.path().join("copy.m");
    recovered.save(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), crlf);
    // A BOM inserted in the editor is manifest-equivalent transport text. Save
    // exactly those bytes, then compare future disk reads without false conflict.
    recovered.text = format!("\u{feff}{crlf}");
    recovered.changed();
    let inserted = recovered.text.clone();
    let bom_path = target.path().join("bom_editor.m");
    recovered.save(&bom_path).unwrap();
    assert_eq!(fs::read_to_string(&bom_path).unwrap(), inserted);
    assert_eq!(recovered.text, inserted);
    assert!(!recovered.dirty && recovered.external_change().unwrap().is_none());
    assert_eq!(
        recovered.try_save(&bom_path, None).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(fs::read_to_string(&bom_path).unwrap(), inserted);
    assert!(recovered.external_change().unwrap().is_none());
    let old_path = recovered.path.clone();
    recovered.text = "arbitrary=1;\n".into();
    recovered.changed();
    let other = target.path().join("edited.m");
    let error = recovered.save(&other).unwrap_err();
    assert!(error.contains("modified") && error.contains("Re-export"));
    assert_eq!(recovered.path, old_path);
    assert!(recovered.dirty && !other.exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), crlf);
    // Old recovery JSON requires no new fields; untrusted declared path is bounded.
    let mut value = serde_json::to_value(&original).unwrap();
    value["saved"] = "% MatRust reconstruction bundle v1: ../escape\n".into();
    let mut invalid: Document = serde_json::from_value(value).unwrap();
    assert!(invalid.validate_recovery().is_err());
    let mut value = serde_json::to_value(&original).unwrap();
    value["path"] = "relative.m".into();
    let mut relative: Document = serde_json::from_value(value).unwrap();
    assert!(relative.validate_recovery().is_err());
    assert!(
        relative
            .verify_reconstruction(&AtomicBool::new(false))
            .is_err()
    );
    assert!(relative.save(&target.path().join("relative.m")).is_err());
    assert!(!target.path().join("relative.m").exists());
}

#[test]
fn bundle_conflicts_and_precancel_preserve_document_and_existing_targets() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let (_, bundle) = fixture_bundle(source.path());
    let mut document = Document::open(&bundle.script).unwrap();
    let original = document.path.clone();
    let path = target.path().join("copy.m");
    for invalid in ["bad-name.m", "load.m"] {
        assert!(document.save(&target.path().join(invalid)).is_err());
        assert_eq!(document.path, original);
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    }
    fs::write(&path, "keep=1;").unwrap();
    let SaveOutcome::Conflict(version) = document.try_save(&path, None).unwrap() else {
        panic!("missing conflict")
    };
    assert_eq!(version, DiskVersion::Text("keep=1;".into()));
    fs::write(&path, "changed=2;").unwrap();
    assert!(matches!(
        document.try_save(&path, Some(&version)).unwrap(),
        SaveOutcome::Conflict(_)
    ));
    let expected = DiskVersion::Text("changed=2;".into());
    assert!(
        document
            .try_save_with_cancel(&path, Some(&expected), &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(document.path, original);
    assert_eq!(fs::read_to_string(&path).unwrap(), "changed=2;");
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 1);
    assert_eq!(
        document.try_save(&path, Some(&expected)).unwrap(),
        SaveOutcome::Saved
    );
    document
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
}

#[test]
fn legacy_generation_explains_relocation_limit_and_ordinary_scripts_keep_normal_save_as() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let path = source.path().join("legacy.m");
    let text = "% Keep this script and mr_plot_old/data.mat together.\nload(fullfile(fileparts(mfilename('fullpath')),'mr_plot_old','data.mat'));\nx=1;\n";
    fs::write(&path, text).unwrap();
    let mut legacy = Document::open(&path).unwrap();
    assert!(
        legacy
            .save(&target.path().join("legacy.m"))
            .unwrap_err()
            .contains("older generated reconstruction")
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    legacy.save(&source.path().join("same_folder.m")).unwrap();
    let plain = source.path().join("ordinary.m");
    fs::write(&plain, "x=1; % mr_plot_old is ordinary text\n").unwrap();
    let mut document = Document::open(&plain).unwrap();
    document.text.push_str("y=2;\n");
    document.changed();
    document.save(&target.path().join("ordinary.m")).unwrap();
    assert_eq!(
        fs::read_to_string(target.path().join("ordinary.m")).unwrap(),
        document.text
    );
}

#[test]
fn explicit_selected_link_parents_support_export_open_and_relocation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("real_source");
    let target = root.path().join("real_target");
    let source_link = root.path().join("selected_source");
    let target_link = root.path().join("selected_target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    for (real, link) in [(&source, &source_link), (&target, &target_link)] {
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(real, link) {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("selected-parent link fixture failed: {error}");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(real, link).unwrap();
    }
    let (_, bundle) = fixture_bundle(&source_link);
    let bytes = fs::read(&bundle.script).unwrap();
    let mut document = Document::open(&bundle.script).unwrap();
    document
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    document.save(&target_link.join("relocated.m")).unwrap();
    assert_eq!(
        document.path.as_deref(),
        Some(
            std::path::absolute(target_link.join("relocated.m"))
                .unwrap()
                .as_path()
        )
    );
    assert!(matrust::document::same_path(
        document.path.as_deref().unwrap(),
        &target.join("relocated.m")
    ));
    assert_eq!(fs::read(target.join("relocated.m")).unwrap(), bytes);
    document
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    let leaf = bundle.companion_directory.file_name().unwrap();
    assert_eq!(
        fs::read(target.join(leaf).join("data.mat")).unwrap(),
        fs::read(bundle.data).unwrap()
    );
}

#[test]
fn explicitly_converted_same_folder_script_reloads_runs_and_retains_companions() {
    let root = tempfile::tempdir().unwrap();
    let (_, bundle) = fixture_bundle(root.path());
    let data = fs::read(&bundle.data).unwrap();
    let metadata = fs::read(bundle.companion_directory.join("reconstruction.json")).unwrap();
    let mut document = Document::open(&bundle.script).unwrap();
    document.text.push_str("\nuser_edit=42;\n");
    document.changed();
    let edited = document.text.clone();
    assert!(document.needs_reconstruction_conversion().unwrap());
    assert!(
        document
            .try_save_edited_reconstruction_with_cancel(
                &bundle.script,
                None,
                &AtomicBool::new(true)
            )
            .is_err()
    );
    assert_eq!(document.text, edited);
    assert!(document.dirty);
    assert_eq!(
        document
            .try_save_edited_reconstruction_with_cancel(
                &bundle.script,
                None,
                &AtomicBool::new(false)
            )
            .unwrap(),
        SaveOutcome::Saved
    );
    assert!(!document.dirty);
    assert_eq!(
        document.text.split_once('\n').unwrap().1,
        edited.split_once('\n').unwrap().1
    );
    assert!(!document.needs_reconstruction_conversion().unwrap());
    document.reload().unwrap();
    document
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    let mut engine = Engine::default();
    engine
        .execute_document(&document.text, &bundle.script)
        .unwrap();
    assert_eq!(engine.variables["user_edit"].get(0, 0), 42.);
    assert!(engine.figure.is_some());
    assert_eq!(fs::read(bundle.data).unwrap(), data);
    assert_eq!(
        fs::read(bundle.companion_directory.join("reconstruction.json")).unwrap(),
        metadata
    );
    // Deleting the declaration in the editor still needs explicit conversion;
    // no executable bytes are then removed, only an explanatory comment added.
    let (_, second) = fixture_bundle(root.path());
    let mut removed = Document::open(&second.script).unwrap();
    let body = removed.text.split_once('\n').unwrap().1.to_owned();
    removed.text = format!("\u{feff}{body}");
    removed.changed();
    removed
        .try_save_edited_reconstruction_with_cancel(&second.script, None, &AtomicBool::new(false))
        .unwrap();
    assert!(removed.text.starts_with("% MatRust ordinary script:"));
    assert_eq!(removed.text.split_once('\n').unwrap().1, body);
    // No intervening Reload: the saved snapshot already matches read_script's
    // BOM-normalized disk version, so a normal next Save needs no conflict.
    removed.text.push_str("\nnext_edit=9;\n");
    removed.changed();
    assert_eq!(
        removed.try_save(&second.script, None).unwrap(),
        SaveOutcome::Saved
    );
    assert!(!removed.dirty);
    let reopened = Document::open(&second.script).unwrap();
    reopened
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    assert!(!reopened.text.contains('\u{feff}'));
    let mut fresh = Engine::default();
    fresh
        .execute_document(&reopened.text, &second.script)
        .unwrap();
    assert!(fresh.figure.is_some());
    assert_eq!(fresh.variables["next_edit"].get(0, 0), 9.);
}

#[test]
fn untitled_copied_declaration_can_explicitly_save_only_script_without_an_origin() {
    let target = tempfile::tempdir().unwrap();
    let path = target.path().join("copied.m");
    let mut document = Document::new();
    document.text = "% MatRust reconstruction bundle v1: mr_plot_missing\nanswer=42;\n".into();
    document.changed();
    assert!(document.needs_reconstruction_conversion().unwrap());
    let body = document.text.split_once('\n').unwrap().1.to_owned();
    assert_eq!(
        document
            .try_save_edited_reconstruction_with_cancel(&path, None, &AtomicBool::new(false))
            .unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(document.text.split_once('\n').unwrap().1, body);
    assert!(!document.dirty);
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 1);
    let opened = Document::open(&path).unwrap();
    opened
        .verify_reconstruction(&AtomicBool::new(false))
        .unwrap();
    let mut engine = Engine::default();
    engine.execute_document(&opened.text, &path).unwrap();
    assert_eq!(engine.variables["answer"].get(0, 0), 42.);
}
