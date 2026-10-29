use super::*;
fn unchanged(engine: &Engine, before: &BTreeMap<String, Array>) {
    assert_eq!(engine.workspace_count(), before.len());
    for (name, value) in before {
        assert!(value.same_snapshot(&engine.array(name).unwrap()), "{name}");
    }
}
#[test]
fn destination_validation_precedes_io_and_requires_snapshot_not_equal_values() {
    let mut engine = Engine::default();
    engine.execute("data=[1 2];Alias=data;").unwrap();
    let before = engine.workspace();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.csv");
    for name in ["", "while", "bad-name", &"x".repeat(64)] {
        let error = perform(
            &mut engine,
            Request {
                path: path.clone(),
                name: name.into(),
                expected: None,
            },
        )
        .unwrap_err();
        assert!(!error.contains("No such file") && !error.contains("cannot find"));
        unchanged(&engine, &before);
    }
    for expected in [
        None,
        Some(
            matrust::Matrix::from_column_vec(1, 2, vec![1., 2.])
                .unwrap()
                .into(),
        ),
    ] {
        let error = perform(
            &mut engine,
            Request {
                path: path.clone(),
                name: "data".into(),
                expected,
            },
        )
        .unwrap_err();
        assert!(error.contains("destination variable changed"));
        unchanged(&engine, &before);
    }
    let stale = before["data"].clone();
    engine.execute("clear data;").unwrap();
    assert!(
        perform(
            &mut engine,
            Request {
                path,
                name: "data".into(),
                expected: Some(stale)
            }
        )
        .unwrap_err()
        .contains("destination variable changed")
    );
}
#[test]
fn csv_worker_replacement_errors_cancel_and_retry_are_atomic() {
    let mut engine = Engine::default();
    engine
        .execute("data=complex([1 2],[3 4]);Alias=data;keep=logical([1 0]);")
        .unwrap();
    let before = engine.workspace();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.csv");
    std::fs::write(&path, "not,a,number\n").unwrap();
    let request = || Request {
        path: path.clone(),
        name: "data".into(),
        expected: Some(before["data"].clone()),
    };
    assert!(perform(&mut engine, request()).is_err());
    unchanged(&engine, &before);
    std::fs::write(&path, "5,6\n7,8\n").unwrap();
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(
        perform(&mut engine, request())
            .unwrap_err()
            .contains("cancelled")
    );
    unchanged(&engine, &before);
    engine.cancellation().store(false, Ordering::Relaxed);
    perform(&mut engine, request()).unwrap();
    assert!(before["data"].same_snapshot(&engine.array("Alias").unwrap()));
    assert!(before["keep"].same_snapshot(&engine.array("keep").unwrap()));
    let actual = engine.array("data").unwrap();
    assert!(!actual.is_complex());
    assert_eq!(actual.as_numeric().unwrap().data(), &[5., 7., 6., 8.]);
    // Name guard applies to scalar, empty, logical and complex destinations alike.
    engine.execute("empty=complex(zeros(0,3));").unwrap();
    assert!(
        perform(
            &mut engine,
            Request {
                path,
                name: "empty".into(),
                expected: None
            }
        )
        .unwrap_err()
        .contains("destination variable changed")
    );
}
#[test]
fn suggested_names_are_valid_bounded_and_do_not_overwrite_suffixes() {
    for stem in ["while.csv", "9data.csv", "bad-name.csv"] {
        assert_eq!(suggested_name(std::path::Path::new(stem)), "data");
    }
    assert_eq!(
        suggested_name(std::path::Path::new("Valid_2.csv")),
        "Valid_2"
    );
    let scalar: Array = matrust::Matrix::scalar(0.).into();
    let variables = (1..=10000)
        .map(|i| (format!("data_{i}"), scalar.clone()))
        .collect();
    assert_eq!(
        unused_name("data", &variables).as_deref(),
        Some("data_10001")
    );
    let name = unused_name(&"a".repeat(63), &BTreeMap::new()).unwrap();
    assert!(name.len() <= 63);
    workspace_edit::validate_name(&name).unwrap();
}
