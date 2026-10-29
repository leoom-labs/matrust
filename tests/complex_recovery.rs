use matrust::{
    Array, Complex64, ComplexMatrix, Matrix,
    document::Document,
    session::{Session, SessionStore, workspace},
    simulation::Model,
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn state(values: BTreeMap<String, Array>) -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(values),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    }
}
fn bits(a: &Array) -> Vec<(u64, u64)> {
    a.as_complex()
        .unwrap()
        .data()
        .iter()
        .map(|v| (v.re.to_bits(), v.im.to_bits()))
        .collect()
}
#[test]
fn complex_recovery_streams_exact_components_and_reuses_reshaped_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let a = ComplexMatrix::from_column_vec(
        10001,
        1,
        (0..10001)
            .map(|i| {
                Complex64::new(
                    if i == 1 { -0. } else { i as f64 },
                    if i == 2 {
                        f64::from_bits(0x7ff8000000000123)
                    } else {
                        -((i as f64) + 0.5)
                    },
                )
            })
            .collect(),
        &cancel,
    )
    .unwrap();
    let scalar = ComplexMatrix::scalar(Complex64::new(f64::from_bits(1), -0.), &cancel).unwrap();
    let values = BTreeMap::from([
        ("A".into(), a.clone().into()),
        ("B".into(), a.clone().into()),
        ("R".into(), a.reshape(1, 10001, &cancel).unwrap().into()),
        ("S".into(), scalar.into()),
        (
            "E".into(),
            ComplexMatrix::filled(0, 7, Complex64::new(0., 0.), &cancel)
                .unwrap()
                .into(),
        ),
    ]);
    let mut store = SessionStore::new(&path);
    store.save(state(values.clone())).unwrap();
    let first = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(first.version, 2);
    assert!(first.entries["A"].complex);
    assert!(!first.entries["A"].logical);
    assert_eq!(first.entries["A"].file, first.entries["B"].file);
    assert_eq!(first.entries["A"].file, first.entries["R"].file);
    assert_eq!(first.entries["S"].inline.as_ref().unwrap().len(), 16);
    let restored = workspace::restore(&path, &first, &cancel).unwrap();
    for (name, value) in &values {
        assert_eq!(bits(value), bits(&restored[name]));
        assert_eq!(
            (value.rows(), value.cols()),
            (restored[name].rows(), restored[name].cols())
        );
    }
    assert!(restored["A"].same_snapshot(&restored["B"]));
    assert!(
        restored["A"]
            .as_complex()
            .unwrap()
            .shares_storage(restored["R"].as_complex().unwrap())
    );
    store.save(state(values)).unwrap();
    assert_eq!(
        first.entries,
        Session::load(&path)
            .unwrap()
            .workspace_manifest
            .unwrap()
            .entries
    );
    let mut invalid = first.clone();
    invalid.version = 1;
    assert!(workspace::restore(&path, &invalid, &cancel).is_err());
    invalid = first.clone();
    invalid.entries.get_mut("A").unwrap().logical = true;
    assert!(workspace::restore(&path, &invalid, &cancel).is_err());
    invalid = first.clone();
    invalid.entries.get_mut("A").unwrap().bytes -= 8;
    assert!(workspace::restore(&path, &invalid, &cancel).is_err());
    assert!(workspace::restore(&path, &first, &AtomicBool::new(true)).is_err());
    let payload = workspace::directory(&path)
        .unwrap()
        .join(&first.entries["A"].file);
    let mut bytes = std::fs::read(&payload).unwrap();
    bytes[8] ^= 1;
    std::fs::write(&payload, bytes).unwrap();
    assert!(
        workspace::restore(&path, &first, &cancel)
            .unwrap_err()
            .contains("checksum")
    );
}
#[test]
fn real_recovery_keeps_version_one_without_complex_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    state(BTreeMap::from([("A".into(), Matrix::scalar(7.).into())]))
        .save(&path)
        .unwrap();
    let data = std::fs::read_to_string(&path).unwrap();
    assert!(!data.contains("\"complex\""));
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(
        workspace::restore(&path, &manifest, &AtomicBool::new(false)).unwrap()["A"]
            .as_numeric()
            .unwrap()
            .data(),
        &[7.]
    );
}
