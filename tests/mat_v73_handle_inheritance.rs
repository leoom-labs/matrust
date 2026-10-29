#![cfg(windows)]

use matrust::{Array, Matrix, files};
use std::{
    collections::BTreeMap,
    fs,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};
const CHILD_DIRECTORY: &str = "MATRUST_HDF5_INHERIT_TEST_DIRECTORY";
const WAIT: Duration = Duration::from_secs(5);
fn pause() {
    thread::sleep(Duration::from_millis(10));
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        if let Err(error) = self.0.kill() {
            eprintln!("HDF5 helper kill error: {error}");
        }
        let until = Instant::now() + WAIT;
        while Instant::now() < until {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            pause();
        }
        eprintln!("UNCERTAIN_HDF5_HELPER pid={}", self.0.id());
    }
}

#[test]
#[ignore = "Helper-only entrypoint: parent regression invokes this exact child with private directory"]
fn hdf5_inheritance_hold_child() {
    // No HDF5 call: inherited native file handles are the only mechanism under test.
    let directory = std::env::var_os(CHILD_DIRECTORY).expect("Parent-owned directory required");
    let directory = Path::new(&directory);
    assert!(directory.is_absolute() && directory.is_dir());
    fs::write(directory.join("ready"), std::process::id().to_string()).unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if directory.join("release").exists() {
            return;
        }
        pause();
    }
    panic!("HDF5 helper self-deadline expired");
}

#[test]
fn mat_v73_native_handle_does_not_survive_in_unrelated_child() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let source = root.join("source.mat");
    let destination = root.join("renamed.mat");
    let stop = AtomicBool::new(false);
    let values = Matrix::from_column_vec(
        2,
        2,
        vec![-0., 1.25, f64::from_bits(0x7ff8_0000_0000_0042), -7.],
    )
    .unwrap();
    let retained = values.clone();
    let workspace = BTreeMap::from([("samples".to_owned(), Array::from(values))]);
    files::save_array_workspace_v73(&source, &workspace, &stop).unwrap();
    let original_bytes = fs::read(&source).unwrap();
    let file = hdf5::File::with_options()
        .with_fcpl(|p| p.userblock(512))
        .with_fapl(|p| p.chunk_cache(521, 8 * 1024 * 1024, 0.75))
        .open_rw(&source)
        .unwrap();
    file.flush().unwrap();
    let mut child = Owned(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hdf5_inheritance_hold_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_DIRECTORY, &root)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + WAIT;
    loop {
        match fs::read_to_string(root.join("ready")) {
            Ok(value) if value == child.0.id().to_string() => break,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("Child readiness: {error}"),
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Child exited before readiness"
        );
        assert!(Instant::now() < until, "Child readiness deadline");
        pause();
    }
    file.close().unwrap();
    assert!(child.0.try_wait().unwrap().is_none());
    let renamed = fs::rename(&source, &destination);
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "Child exited during rename; invalid control"
    );
    renamed.expect("Closed parent HDF5 handle must not remain locked by unrelated live child");
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), original_bytes);
    fs::write(root.join("release"), b"release").unwrap();
    let until = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "Helper status {status}");
            break;
        }
        assert!(Instant::now() < until, "Child release/reap deadline");
        pause();
    }
    let catalog = files::inspect_workspace(&destination, &stop).unwrap();
    let loaded =
        files::load_selected_arrays(&catalog, &workspace.keys().cloned().collect(), &stop).unwrap();
    let Array::Numeric(actual) = &loaded["samples"] else {
        panic!("Numeric samples expected")
    };
    assert_eq!((actual.rows(), actual.cols()), (2, 2));
    assert_eq!(
        actual
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        retained
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
    assert!(workspace["samples"].same_snapshot(&Array::from(retained)));
    assert_eq!(fs::read(&destination).unwrap(), original_bytes);
}
