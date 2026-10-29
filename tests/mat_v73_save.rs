use matrust::{Array, LogicalMatrix, Matrix, files};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

#[test]
fn v73_save_roundtrip_preserves_types_bits_empty_shapes_and_chunk_edges() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("\u{5e9}\u{6570}.mat");
    let numeric = Matrix::from_column_vec(
        2,
        3,
        vec![
            1.,
            -0.,
            f64::from_bits(0x7ff8000000000042),
            f64::INFINITY,
            f64::NEG_INFINITY,
            -7.25,
        ],
    )
    .unwrap();
    let data = BTreeMap::from([
        ("numeric".into(), Array::from(numeric)),
        (
            "mask".into(),
            LogicalMatrix::from_column_vec(2, 3, vec![0, 1, 1, 0, 1, 0], &cancel)
                .unwrap()
                .into(),
        ),
        (
            "empty_rows".into(),
            Matrix::from_column_vec(0, 7, vec![]).unwrap().into(),
        ),
        (
            "empty_columns".into(),
            Matrix::from_column_vec(5, 0, vec![]).unwrap().into(),
        ),
        (
            "empty_logical".into(),
            LogicalMatrix::filled(0, 3, false, &cancel).unwrap().into(),
        ),
        (
            "chunked".into(),
            Matrix::from_column_vec(
                5,
                210013,
                (0..5 * 210013).map(|i| i as f64 / 1024.).collect(),
            )
            .unwrap()
            .into(),
        ),
        (
            "long_column".into(),
            Matrix::from_column_vec(1048581, 2, (0..1048581 * 2).map(|i| i as f64).collect())
                .unwrap()
                .into(),
        ),
    ]);
    files::save_array_workspace_v73(&path, &data, &cancel).unwrap();
    {
        let hdf = hdf5::File::open(&path).unwrap();
        let chunk = hdf.dataset("chunked").unwrap().chunk().unwrap();
        assert!(chunk[0] > 1, "Short matrices must group adjacent columns");
        assert!(chunk.iter().product::<usize>() * 8 <= 256 * 1024);
        assert!(
            hdf.dataset("chunked")
                .unwrap()
                .filters()
                .contains(&hdf5::filters::Filter::Shuffle)
        );
    }
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    let loaded =
        files::load_selected_arrays(&catalog, &data.keys().cloned().collect(), &cancel).unwrap();
    for (name, expected) in &data {
        let actual = &loaded[name];
        assert_eq!(
            (actual.rows(), actual.cols(), actual.class()),
            (expected.rows(), expected.cols(), expected.class())
        );
        match (expected, actual) {
            (Array::Numeric(a), Array::Numeric(b)) => assert!(
                a.data()
                    .iter()
                    .zip(b.data())
                    .all(|(x, y)| x.to_bits() == y.to_bits()),
                "{name}"
            ),
            (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
            _ => panic!("wrong class"),
        }
    }
    // Replacement publishes a complete new workspace with no stale datasets.
    files::save_array_workspace_v73(&path, &BTreeMap::new(), &cancel).unwrap();
    assert_eq!(
        files::inspect_workspace(&path, &cancel)
            .unwrap()
            .variables()
            .len(),
        0
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn v73_save_logical_writes_cross_byte_sized_batch_and_chunk_boundaries() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("logical.mat");
    let count = 8 * 1024 * 1024 + 5;
    let mask = LogicalMatrix::from_column_vec(
        count,
        1,
        (0..count).map(|i| u8::from(i % 7 == 0)).collect(),
        &cancel,
    )
    .unwrap();
    let expected = mask.clone();
    let values = BTreeMap::from([("mask".into(), mask.into())]);
    files::save_array_workspace_v73(&path, &values, &cancel).unwrap();
    {
        let file = hdf5::File::open(&path).unwrap();
        assert_eq!(
            file.dataset("mask").unwrap().chunk().unwrap(),
            vec![1, 256 * 1024]
        );
    }
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    let loaded =
        files::load_selected_arrays(&catalog, &values.keys().cloned().collect(), &cancel).unwrap();
    let Array::Logical(actual) = &loaded["mask"] else {
        panic!("Logical class lost")
    };
    assert_eq!(actual.data(), expected.data());
}

#[test]
fn v73_save_rejects_invalid_names_and_cancellation_without_changing_target() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.mat");
    std::fs::write(&path, b"original").unwrap();
    let bad = BTreeMap::from([("bad/name".into(), Matrix::scalar(1.).into())]);
    assert!(
        files::save_array_workspace_v73(&path, &bad, &AtomicBool::new(false))
            .unwrap_err()
            .contains("Invalid variable name")
    );
    assert!(
        files::save_array_workspace_v73(&path, &BTreeMap::new(), &AtomicBool::new(true)).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn v73_save_enforces_workspace_and_variable_limits_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.mat");
    std::fs::write(&path, b"original").unwrap();
    let array = Matrix::from_column_vec(1024 * 1024, 12, vec![1.; 12 * 1024 * 1024]).unwrap();
    let oversized = BTreeMap::from([
        ("A".into(), array.clone().into()),
        ("B".into(), array.clone().into()),
        ("C".into(), array.into()),
    ]);
    assert!(
        files::save_array_workspace_v73(&path, &oversized, &AtomicBool::new(false))
            .unwrap_err()
            .contains("256 MiB")
    );
    drop(oversized);
    let empty = Matrix::from_column_vec(0, 0, vec![]).unwrap();
    let too_many = (0..10001)
        .map(|i| (format!("x{i}"), empty.clone().into()))
        .collect();
    assert!(
        files::save_array_workspace_v73(&path, &too_many, &AtomicBool::new(false))
            .unwrap_err()
            .contains("10000")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
