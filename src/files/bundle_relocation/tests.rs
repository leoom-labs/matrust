use super::*;
use std::sync::atomic::Ordering;

fn fixture(root: &Path) -> (std::path::PathBuf, String) {
    let path = root.join("source.m");
    let directory = root.join("mr_plot_fixture");
    fs::create_dir(&directory).unwrap();
    // This is a transfer/checksum fixture, deliberately not a MAT parsing reference.
    fs::write(directory.join("data.mat"), vec![17_u8; CHUNK * 3 + 7]).unwrap();
    let text = format!("{}x=1;\n", header("mr_plot_fixture").unwrap());
    fs::write(&path, &text).unwrap();
    write_manifest(
        &directory,
        "mr_plot_fixture",
        &text,
        &AtomicBool::new(false),
    )
    .unwrap();
    (path, text)
}

#[test]
fn real_chunk_cancellation_and_poststage_conflict_leave_no_partial_generation() {
    let root = tempfile::tempdir().unwrap();
    let (source, text) = fixture(root.path());
    let target = tempfile::tempdir().unwrap();
    let path = target.path().join("copy.m");
    let cancel = AtomicBool::new(false);
    let mut copied = 0;
    let error = prepare_checked(&source, &text, &text, &path, &cancel, |bytes| {
        copied = bytes;
        cancel.store(true, Ordering::Relaxed);
    })
    .err()
    .unwrap();
    assert_eq!(error, "Calculation cancelled");
    assert_eq!(copied, CHUNK as u64);
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    assert_eq!(fs::read_to_string(&source).unwrap(), text);
    cancel.store(false, Ordering::Relaxed);
    let prepared = prepare(&source, &text, &text, &path, &cancel).unwrap();
    fs::write(&path, "concurrent=1;").unwrap();
    assert!(prepared.publish(false, &cancel, || Ok(())).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "concurrent=1;");
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 1);
    fs::remove_file(&path).unwrap();
    prepare(&source, &text, &text, &path, &cancel)
        .unwrap()
        .publish(false, &cancel, || Ok(()))
        .unwrap();
    verify(&path, &text, &cancel).unwrap();
    assert_eq!(
        fs::read(target.path().join("mr_plot_fixture/data.mat")).unwrap(),
        fs::read(root.path().join("mr_plot_fixture/data.mat")).unwrap()
    );
}

#[test]
fn manifest_and_source_tampering_are_rejected_without_overwriting_any_destination() {
    let root = tempfile::tempdir().unwrap();
    let (source, text) = fixture(root.path());
    let target = tempfile::tempdir().unwrap();
    let path = target.path().join("copy.m");
    let directory = root.path().join("mr_plot_fixture");
    let manifest_path = directory.join(MANIFEST);
    let original = fs::read(&manifest_path).unwrap();
    let cancel = AtomicBool::new(false);
    for property in [
        "version",
        "companion",
        "script_blake3",
        "data_bytes",
        "data_blake3",
        "unexpected",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value[property] = match property {
            "version" => 2.into(),
            "data_bytes" => ((super::super::MAX_MAT_BYTES as u64) + 1).into(),
            "companion" => "../outside".into(),
            "script_blake3" | "data_blake3" => "0".repeat(64).into(),
            _ => true.into(),
        };
        fs::write(&manifest_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            prepare(&source, &text, &text, &path, &cancel).is_err(),
            "{property}"
        );
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    }
    fs::write(&manifest_path, &original).unwrap();
    let data_path = directory.join("data.mat");
    let data = fs::read(&data_path).unwrap();
    fs::write(&data_path, b"changed").unwrap();
    assert!(
        prepare(&source, &text, &text, &path, &cancel)
            .err()
            .unwrap()
            .contains("MAT data changed")
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    fs::write(&data_path, &data).unwrap();
    fs::remove_file(&manifest_path).unwrap();
    assert!(verify(&source, &text, &cancel).is_err());
    fs::write(&manifest_path, &original).unwrap();
    fs::write(directory.join("unlisted.txt"), b"extra").unwrap();
    assert!(verify(&source, &text, &cancel).is_err());
    fs::remove_file(directory.join("unlisted.txt")).unwrap();
    let occupied = target.path().join("mr_plot_fixture");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("unrelated"), b"keep").unwrap();
    assert!(prepare(&source, &text, &text, &path, &cancel).is_err());
    assert_eq!(fs::read(occupied.join("unrelated")).unwrap(), b"keep");
    assert!(!path.exists());
}

#[test]
fn explicit_header_bounds_and_bom_crlf_digest_have_narrow_semantics() {
    for text in [
        "% MatRust reconstruction bundle v2: mr_plot_a\nx=1;",
        "% MatRust reconstruction bundle v1: ../mr_plot_a\nx=1;",
        "% MatRust reconstruction bundle v1: mr_plot_a/data.mat\nx=1;",
        "% MatRust reconstruction bundle v1: mr_plot_\nx=1;",
    ] {
        assert!(declaration(text).is_err());
    }
    assert!(declaration(&format!("{V1}mr_plot_{}\n", "x".repeat(100))).is_err());
    assert_eq!(
        declaration("x=1; % MatRust reconstruction bundle v1: mr_plot_a").unwrap(),
        None
    );
    assert_eq!(
        script_digest("\u{feff}a=1;\r\nb=2;\r\n"),
        script_digest("a=1;\nb=2;\n")
    );
    assert_ne!(script_digest("a=1;\r"), script_digest("a=1;\n"));
}

#[test]
fn linked_bundle_files_are_not_followed_when_platform_allows_link_creation() {
    let root = tempfile::tempdir().unwrap();
    let (source, text) = fixture(root.path());
    let data = root.path().join("mr_plot_fixture/data.mat");
    let outside = root.path().join("outside.mat");
    fs::rename(&data, &outside).unwrap();
    #[cfg(windows)]
    {
        if let Err(error) = std::os::windows::fs::symlink_file(&outside, &data) {
            // Windows policy can disallow unprivileged symlink creation. This
            // test then makes no claim about executing the reparse branch.
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("symlink fixture failed: {error}");
        }
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &data).unwrap();
    assert!(
        verify(&source, &text, &AtomicBool::new(false))
            .unwrap_err()
            .contains("symlinks or reparse")
    );
    assert_eq!(
        fs::metadata(&outside).unwrap().len(),
        (CHUNK * 3 + 7) as u64
    );
}
