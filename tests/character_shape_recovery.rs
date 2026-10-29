use matrust::{
    Array, CellArray, CharacterRow,
    document::Document,
    session::{Session, SessionStore, workspace},
    simulation::Model,
};
use std::{
    collections::BTreeMap,
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

#[test]
fn character_recovery_v5_keeps_view_shapes_payload_aliases_and_legacy_row_wire() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let row = CharacterRow::from_units(vec![65, 66, 67, 68, 69, 70], &stop).unwrap();
    let plain = BTreeMap::from([("Row".to_owned(), Array::from(row.clone()))]);
    store.save(session(plain.clone())).unwrap();
    let first = std::fs::read(&path).unwrap();
    let old = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(old.version, 3);
    store.save(session(plain)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
    let mut source = BTreeMap::from([
        ("Arow".to_owned(), row.clone().into()),
        ("Zmatrix".into(), row.reshape(2, 3, &stop).unwrap().into()),
    ]);
    let empty = CharacterRow::from_units(vec![], &stop).unwrap();
    for (name, r, c) in [("E00", 0, 0), ("E10", 1, 0), ("E03", 0, 3), ("E40", 4, 0)] {
        source.insert(name.into(), empty.reshape(r, c, &stop).unwrap().into());
    }
    let large = CharacterRow::from_units((0..4096).map(|i| i as u16).collect(), &stop).unwrap();
    source.insert("Large".into(), large.reshape(64, 64, &stop).unwrap().into());
    source.insert("LargeRow".into(), large.clone().into());
    let nested = CellArray::from_shared(
        1,
        2,
        vec![
            Arc::new(row.reshape(3, 2, &stop).unwrap().into()),
            Arc::new(empty.reshape(4, 0, &stop).unwrap().into()),
        ],
        &stop,
    )
    .unwrap();
    source.insert("Cell".into(), nested.into());
    store.save(session(source.clone())).unwrap();
    let before = std::fs::read(&path).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.version, 5);
    let restored = workspace::restore(&path, &manifest, &stop).unwrap();
    for (name, value) in &source {
        assert_eq!(
            (value.class(), value.rows(), value.cols()),
            (
                restored[name].class(),
                restored[name].rows(),
                restored[name].cols()
            )
        );
        if let Array::Character(a) = value {
            let Array::Character(b) = &restored[name] else {
                panic!()
            };
            assert_eq!(a.units(), b.units());
        }
    }
    let text = |name: &str| {
        let Array::Character(c) = &restored[name] else {
            panic!()
        };
        c
    };
    assert!(text("Arow").same_storage(text("Zmatrix")));
    assert!(text("Large").same_storage(text("LargeRow")));
    assert!(text("E00").same_storage(text("E40")));
    assert!(!restored["Arow"].same_snapshot(&restored["Zmatrix"]));
    let Array::Cell(c) = &restored["Cell"] else {
        panic!()
    };
    let Array::Character(n) = c.get_linear(0).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!((n.rows(), n.cols()), (3, 2));
    assert!(n.same_storage(text("Arow")));
    let mut downgraded = manifest.clone();
    downgraded.version = 3;
    assert!(
        workspace::restore(&path, &downgraded, &stop)
            .unwrap_err()
            .contains("version 5")
    );
    let mut malformed = manifest.clone();
    let leaf = malformed
        .entries
        .values_mut()
        .find(|r| r.character)
        .unwrap();
    leaf.rows = 65537;
    assert!(workspace::restore(&path, &malformed, &stop).is_err());
    stop.store(true, Ordering::Relaxed);
    assert!(
        store
            .save_with_cancel(session(source.clone()), &stop)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    stop.store(false, Ordering::Relaxed);
    store.save(session(source)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn v5_capability_is_required_by_character_alias_edges_without_shaped_stored_leaves() {
    let stop = AtomicBool::new(false);
    for nested in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut store = SessionStore::new(&path);
        let row =
            CharacterRow::from_units(vec![65, 0, 955, 0xd800, 0xdc00, 0xffff], &stop).unwrap();
        let matrix = row.reshape(2, 3, &stop).unwrap();
        // BTree order deliberately stores Arow's canonical payload first. Every
        // stored character leaf must remain canonical; only graph edges need v5.
        let mut source = BTreeMap::from([("Arow".to_owned(), Array::from(row.clone()))]);
        if nested {
            source.insert(
                "Zcell".into(),
                CellArray::from_shared(
                    1,
                    2,
                    vec![
                        Arc::new(Array::from(matrix.clone())),
                        Arc::new(Array::from(row.reshape(3, 2, &stop).unwrap())),
                    ],
                    &stop,
                )
                .unwrap()
                .into(),
            );
        } else {
            source.insert("Zmatrix".into(), matrix.clone().into());
        }
        store.save(session(source.clone())).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
        assert_eq!(manifest.version, 5);
        assert_eq!(
            manifest.entries.len(),
            1,
            "One canonical character allocation, not a second shaped leaf"
        );
        for leaf in manifest.entries.values() {
            assert!(leaf.character);
            assert_eq!((leaf.rows, leaf.cols), (1, 6));
        }
        let wire = serde_json::to_value(&manifest).unwrap();
        let graph = &wire["graph"];
        if nested {
            // Neither root is a shaped character view: capability comes only
            // from the nested Cell node's character edges.
            assert_eq!(graph["roots"]["Arow"]["rows"], 1);
            assert_eq!(graph["roots"]["Zcell"]["rows"], 1);
            let cell = graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["kind"] == "Cell")
                .unwrap();
            assert_eq!(cell["children"][0]["rows"], 2);
            assert_eq!(cell["children"][1]["rows"], 3);
        } else {
            assert_eq!(graph["roots"]["Zmatrix"]["rows"], 2);
        }
        let restored = workspace::restore(&path, &manifest, &stop).unwrap();
        let Array::Character(saved_row) = &restored["Arow"] else {
            panic!()
        };
        let saved_matrix = if nested {
            let Array::Cell(c) = &restored["Zcell"] else {
                panic!()
            };
            let Array::Character(a) = c.get_linear(0).unwrap().as_ref() else {
                panic!()
            };
            a
        } else {
            let Array::Character(a) = &restored["Zmatrix"] else {
                panic!()
            };
            a
        };
        assert_eq!(
            (
                saved_matrix.rows(),
                saved_matrix.cols(),
                saved_matrix.units()
            ),
            (2, 3, row.units())
        );
        assert!(saved_row.same_storage(saved_matrix));
        let mut downgraded = manifest.clone();
        downgraded.version = 3;
        assert!(
            workspace::restore(&path, &downgraded, &stop)
                .unwrap_err()
                .contains("version 5")
        );
        for rows in [3usize, 65537] {
            let mut tampered = wire.clone();
            let edge = if nested {
                &mut tampered["graph"]["nodes"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|n| n["kind"] == "Cell")
                    .unwrap()["children"][0]
            } else {
                &mut tampered["graph"]["roots"]["Zmatrix"]
            };
            edge["rows"] = serde_json::json!(rows); // count mismatch or dimension overflow
            let malformed = serde_json::from_value(tampered).unwrap();
            assert!(workspace::restore(&path, &malformed, &stop).is_err());
            // No partial map is returned, no persisted generation is changed,
            // and retry retains the same complete valid alias graph.
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert!(row.same_storage(&matrix));
            let retry = workspace::restore(&path, &manifest, &stop).unwrap();
            assert_eq!(retry.len(), source.len());
        }
    }
}

#[test]
fn canonical_only_character_checkpoint_cannot_claim_v5_capability() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let row = CharacterRow::from_units(vec![65, 0, 0xd800], &stop).unwrap();
    let cell = CellArray::from_shared(1, 1, vec![Arc::new(row.clone().into())], &stop).unwrap();
    store
        .save(session(BTreeMap::from([
            ("Arow".into(), row.into()),
            ("Zcell".into(), cell.into()),
        ])))
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.version, 3);
    let mut false_capability = manifest.clone();
    false_capability.version = 5;
    assert!(workspace::restore(&path, &false_capability, &stop).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        workspace::restore(&path, &manifest, &stop).unwrap().len(),
        2
    );
}

#[test]
fn character_capability_checks_precede_payload_reads_and_retain_cancel_retry() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let row = CharacterRow::from_units((0..128).map(|i| i as u16).collect(), &stop).unwrap();
    let shaped = row.reshape(2, 64, &stop).unwrap();
    store
        .save(session(BTreeMap::from([
            ("Arow".into(), row.clone().into()),
            ("Zshape".into(), shaped.into()),
        ])))
        .unwrap();
    let saved = std::fs::read(&path).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.version, 5);
    let check_values = |values: &BTreeMap<String, Array>| {
        let Array::Character(a) = &values["Arow"] else {
            panic!()
        };
        let Array::Character(z) = &values["Zshape"] else {
            panic!()
        };
        assert_eq!((a.rows(), a.cols()), (1, 128));
        assert_eq!((z.rows(), z.cols()), (2, 64));
        assert_eq!(a.units(), row.units());
        assert_eq!(z.units(), row.units());
        assert!(a.same_storage(z));
    };
    check_values(&workspace::restore(&path, &manifest, &stop).unwrap());
    assert_eq!(manifest.entries.len(), 1);
    let leaf = manifest.entries.values().next().unwrap();
    assert!(leaf.inline.is_none());
    let payload = workspace::directory(&path).unwrap().join(&leaf.file);
    let bytes = std::fs::read(&payload).unwrap();
    std::fs::remove_file(&payload).unwrap();
    // Valid v5 now genuinely reaches the missing external payload. Invalid
    // capability claims must still fail earlier, without a partial result.
    let missing = workspace::restore(&path, &manifest, &stop).unwrap_err();
    assert!(!missing.contains("recovery version"), "{missing}");
    for (version, diagnostic) in [(3, "version 5"), (6, "version 6")] {
        let mut invalid = manifest.clone();
        invalid.version = version;
        let error = workspace::restore(&path, &invalid, &stop).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
        assert_ne!(error, missing);
        stop.store(true, Ordering::Relaxed);
        assert_eq!(
            workspace::restore(&path, &invalid, &stop).unwrap_err(),
            "Calculation cancelled"
        );
        stop.store(false, Ordering::Relaxed);
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    std::fs::write(&payload, bytes).unwrap();
    check_values(&workspace::restore(&path, &manifest, &stop).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), saved);
}
