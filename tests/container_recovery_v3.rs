use matrust::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, Matrix,
    document::Document,
    session::{
        Session, SessionStore,
        workspace::{self, Manifest},
    },
    simulation::Model,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn session(values: BTreeMap<String, Array>) -> Session {
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
fn tree(path: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            result.insert(entry.path(), Vec::new());
            result.extend(tree(&entry.path()));
        } else {
            result.insert(entry.path(), std::fs::read(entry.path()).unwrap());
        }
    }
    result
}
fn load(path: &Path, cancel: &AtomicBool) -> BTreeMap<String, Array> {
    workspace::restore(
        path,
        &Session::load(path).unwrap().workspace_manifest.unwrap(),
        cancel,
    )
    .unwrap()
}
fn equal(a: &Array, b: &Array) {
    assert_eq!(
        (a.rows(), a.cols(), a.class()),
        (b.rows(), b.cols(), b.class())
    );
    match (a, b) {
        (Array::Numeric(a), Array::Numeric(b)) => assert!(
            a.data()
                .iter()
                .zip(b.data())
                .all(|(a, b)| a.to_bits() == b.to_bits())
        ),
        (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
        (Array::Complex(a), Array::Complex(b)) => assert!(
            a.data()
                .iter()
                .zip(b.data())
                .all(|(a, b)| (a.re.to_bits(), a.im.to_bits()) == (b.re.to_bits(), b.im.to_bits()))
        ),
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                equal(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Recovery storage type changed"),
    }
}
#[test]
fn recovery_roundtrips_exact_utf16_nested_types_and_shared_reshaped_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let text = CharacterRow::from_units(vec![0, 0xd800, 0xdc00, 0xffff, 955], &cancel).unwrap();
    let numeric: Array = Matrix::from_column_slice(
        2,
        2,
        &[
            -0.,
            f64::from_bits(0x7ff8000000000042),
            f64::INFINITY,
            f64::from_bits(1),
        ],
    )
    .unwrap()
    .into();
    let complex: Array = ComplexMatrix::from_column_vec(
        1,
        2,
        vec![
            Complex64::new(1., -0.),
            Complex64::new(f64::from_bits(1), 2.),
        ],
        &cancel,
    )
    .unwrap()
    .into();
    let nested = CellArray::from_shared(
        2,
        2,
        vec![
            Arc::new(numeric.clone()),
            Arc::new(text.clone().into()),
            Arc::new(complex),
            Arc::new(CellArray::empty(0, 7, &cancel).unwrap().into()),
        ],
        &cancel,
    )
    .unwrap();
    let values = BTreeMap::from([
        ("A".into(), numeric),
        ("T".into(), text.into()),
        ("C".into(), nested.clone().into()),
        ("R".into(), nested.reshape(1, 4, &cancel).unwrap().into()),
        (
            "E".into(),
            CharacterRow::from_units(vec![], &cancel).unwrap().into(),
        ),
    ]);
    session(values.clone()).save(&path).unwrap();
    assert_eq!(
        Session::load(&path)
            .unwrap()
            .workspace_manifest
            .unwrap()
            .version,
        3
    );
    let got = load(&path, &cancel);
    for (name, value) in &values {
        equal(value, &got[name]);
    }
    let (Array::Cell(c), Array::Cell(r)) = (&got["C"], &got["R"]) else {
        panic!()
    };
    assert!(c.same_storage(r));
    assert!(got["A"].same_snapshot(c.get_linear(0).unwrap()));
    assert!(got["T"].same_snapshot(c.get_linear(1).unwrap()));
    assert_eq!((got["E"].rows(), got["E"].cols()), (0, 0));
}
#[test]
fn recovery_preserves_checkpoint_on_stop_and_failed_json_then_reuses_unchanged_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("A=ones(100,100);C={A,'kept',logical([0 1])};s='title';")
        .unwrap();
    let values = e.workspace();
    let mut store = SessionStore::new(&path);
    store.save(session(values.clone())).unwrap();
    let before = tree(dir.path());
    store.save(session(values.clone())).unwrap();
    assert_eq!(tree(dir.path()), before);
    cancel.store(true, Ordering::Relaxed);
    assert!(
        store
            .save_with_cancel(session(values.clone()), &cancel)
            .unwrap_err()
            .contains("cancel")
    );
    assert_eq!(tree(dir.path()), before);
    let fresh = dir.path().join("never_created.json");
    assert!(
        SessionStore::new(&fresh)
            .save_with_cancel(session(values.clone()), &cancel)
            .is_err()
    );
    assert_eq!(tree(dir.path()), before);
    cancel.store(false, Ordering::Relaxed);
    let mut failed = session(values.clone());
    failed.history = vec!["x".repeat(2_000_001)];
    assert!(store.save_with_cancel(failed, &cancel).is_err());
    assert_eq!(tree(dir.path()), before);
    let changed = CharacterRow::from_units(vec![0x1234; 65536], &cancel).unwrap();
    let mut failed = session(BTreeMap::from([("changed".into(), changed.clone().into())]));
    failed.figures = Some(matrust::session::figures::FigureSession {
        current: 0,
        figures: vec![matrust::session::figures::FigureDocument {
            window: None,
            id: 0,
            figure: Default::default(),
            path: None,
            source_version: None,
            dirty: true,
            view: None,
        }],
    });
    assert!(store.save_with_cancel(failed, &cancel).is_err());
    assert_eq!(tree(dir.path()), before);
    let mut failed = session(BTreeMap::from([("changed".into(), changed.into())]));
    // JSON escaping exceeds the bounded 32 MiB writer after the leaf was staged.
    failed.folder = std::path::PathBuf::from("\u{0001}".repeat(6_000_000));
    assert!(
        store
            .save_with_cancel(failed, &cancel)
            .unwrap_err()
            .contains("32 MiB")
    );
    assert_eq!(tree(dir.path()), before);
    let restored = load(&path, &cancel);
    for (name, value) in &values {
        assert!(value.same_snapshot(&e.array(name).unwrap()));
        equal(value, &restored[name]);
    }
    SessionStore::new(&path).save(session(values)).unwrap();
    e.execute("clear C s;").unwrap();
    store.save(session(e.workspace())).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.version, 1);
    assert!(manifest.graph.is_none());
}
#[test]
fn malformed_graph_rejects_cycles_missing_nodes_alias_shapes_and_unreachable_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("A=[1 2];C={A,'text'};").unwrap();
    session(e.workspace()).save(&path).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    let original = serde_json::to_value(&manifest).unwrap();
    let cell = original["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|n| n["kind"] == "Cell")
        .unwrap();
    for mode in 0..7 {
        let mut bad = original.clone();
        match mode {
            0 => bad["graph"]["nodes"][cell]["children"][0]["node"] = serde_json::json!(cell),
            1 => bad["graph"]["roots"]["C"]["node"] = serde_json::json!(usize::MAX),
            2 => bad["graph"]["roots"]["C"]["rows"] = serde_json::json!(65537),
            3 => {
                bad["graph"]["roots"].as_object_mut().unwrap().remove("C");
            }
            4 => bad["version"] = serde_json::json!(2),
            5 => {
                let child = bad["graph"]["nodes"][cell]["children"][0].clone();
                bad["graph"]["nodes"][cell]["children"] = serde_json::json!(vec![child; 65537]);
            }
            _ => {
                let key = bad["entries"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .unwrap()
                    .clone();
                bad["entries"][key]["crc32"] = serde_json::json!(0);
            }
        }
        let bad: Manifest = serde_json::from_value(bad).unwrap();
        assert!(
            workspace::restore(&path, &bad, &cancel).is_err(),
            "Mode {mode}"
        );
    }
    assert_eq!(serde_json::to_value(manifest).unwrap(), original);
}
#[test]
fn nested_leaf_checksum_failure_is_atomic_and_empty_cell_orientations_survive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("A=ones(100,100);C={A,cell(0,7),cell(4,0)};clear A;")
        .unwrap();
    session(e.workspace()).save(&path).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    let got = load(&path, &cancel);
    equal(&got["C"], &e.array("C").unwrap());
    let leaf = manifest
        .entries
        .values()
        .find(|r| r.inline.is_none())
        .unwrap();
    let file = workspace::directory(&path).unwrap().join(&leaf.file);
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[0] ^= 1;
    std::fs::write(file, bytes).unwrap();
    assert!(
        workspace::restore(&path, &manifest, &cancel)
            .unwrap_err()
            .contains("checksum")
    );
    assert!(
        got.contains_key("C"),
        "An already restored independent snapshot remains intact"
    );
}

#[test]
fn graphics_handles_without_figure_context_reject_before_staging_and_allow_typed_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine
        .execute("A=ones(100,100);alias=A;C={A,'retained'};")
        .unwrap();
    let clean = engine.workspace();
    let mut store = SessionStore::new(&path);
    store.save(session(clean.clone())).unwrap();
    let before = tree(dir.path());
    engine
        .execute("f=figure(7);a=gca;h=plot(a,[1 2],[3 4]);")
        .unwrap();
    let figure = engine.figure.clone().unwrap();
    for name in ["f", "a", "h"] {
        let handle = engine.array(name).unwrap();
        assert!(matches!(&handle, Array::Graphics(_)));
        let inner: Array = CellArray::from_shared(
            1,
            2,
            vec![Arc::new(clean["A"].clone()), Arc::new(handle.clone())],
            &cancel,
        )
        .unwrap()
        .into();
        let nested: Array = CellArray::from_shared(1, 1, vec![Arc::new(inner)], &cancel)
            .unwrap()
            .into();
        for value in [handle.clone(), nested] {
            let mut rejected = clean.clone();
            rejected.insert("Zhandle".into(), value.clone());
            let error = store
                .save_with_cancel(session(rejected.clone()), &cancel)
                .unwrap_err();
            assert!(error.contains("graphics handles"), "{error}");
            assert_eq!(
                tree(dir.path()),
                before,
                "Old JSON and payloads must remain byte-identical"
            );
            let fresh = dir.path().join("never_created.json");
            assert!(
                SessionStore::new(&fresh)
                    .save_with_cancel(session(rejected), &cancel)
                    .is_err()
            );
            assert_eq!(
                tree(dir.path()),
                before,
                "Rejection must precede directories and files"
            );
            assert!(handle.same_snapshot(&engine.array(name).unwrap()));
            assert!(engine.figure.as_ref().unwrap().same_snapshot(&figure));
            for (key, original) in &clean {
                assert!(original.same_snapshot(&engine.array(key).unwrap()));
            }
            let restored = load(&path, &cancel);
            for (key, original) in &clean {
                equal(original, &restored[key]);
            }
            assert!(restored["A"].same_snapshot(&restored["alias"]));
            let Array::Cell(cell) = &restored["C"] else {
                panic!("Cell type changed");
            };
            assert!(restored["A"].same_snapshot(cell.get_linear(0).unwrap()));
            store
                .save_with_cancel(session(clean.clone()), &cancel)
                .unwrap();
            assert_eq!(
                tree(dir.path()),
                before,
                "Unchanged retry must reuse prior leaves"
            );
        }
    }
}
