use matrust::{
    Array, Engine, LogicalMatrix, Matrix,
    document::Document,
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession},
        workspace::{self, Manifest},
    },
    simulation::Model,
};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
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
fn manifest(path: &Path) -> Manifest {
    Session::load(path).unwrap().workspace_manifest.unwrap()
}
fn file_names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(workspace::directory(path).unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
// Count this thread's allocated and freed bytes only inside a measured window:
// the typed recovery test checks that the writer cache keeps no old payload.
struct FreeCounter;
thread_local! {
    static COUNTS: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}
fn record(allocated: usize, freed: usize) {
    let _ = COUNTS.try_with(|counter| {
        if let Some((a, f)) = counter.get() {
            counter.set(Some((a.saturating_add(allocated), f.saturating_add(freed))));
        }
    });
}
// SAFETY: every operation delegates to System with the original layout and
// pointer. The thread-local counter never allocates or dereferences the pointer.
unsafe impl std::alloc::GlobalAlloc for FreeCounter {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record(layout.size(), 0);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        record(layout.size(), 0);
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record(size, layout.size());
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        record(0, layout.size());
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: FreeCounter = FreeCounter;
#[test]
fn workspace_recovery_roundtrips_bits_types_empty_shapes_and_shared_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let values = vec![
        f64::from_bits(0x7ff8000000000123),
        -0.,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(1),
        42.,
    ];
    let a = Matrix::from_column_vec(2, 3, values.clone()).unwrap();
    let l = LogicalMatrix::from_column_vec(2, 3, vec![0, 1, 1, 0, 1, 0], &AtomicBool::new(false))
        .unwrap();
    let vars = BTreeMap::from([
        ("A".into(), a.clone().into()),
        ("B".into(), a.clone().into()),
        ("R".into(), a.reshape(3, 2).unwrap().into()),
        ("L".into(), l.clone().into()),
        ("Q".into(), l.reshape(3, 2).unwrap().into()),
        ("E".into(), Matrix::filled(0, 7, 0.).unwrap().into()),
    ]);
    state(vars).save(&path).unwrap();
    let m = manifest(&path);
    assert_eq!(file_names(&path).len(), 0);
    let got = workspace::restore(&path, &m, &AtomicBool::new(false)).unwrap();
    let numeric = got["A"].as_numeric().unwrap();
    assert_eq!(
        numeric
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        values.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    assert!(got["A"].same_snapshot(&got["B"]));
    assert_eq!(
        numeric.data().as_ptr(),
        got["R"].as_numeric().unwrap().data().as_ptr()
    );
    assert_eq!((got["R"].rows(), got["R"].cols()), (3, 2));
    assert_eq!((got["E"].rows(), got["E"].cols()), (0, 7));
    let (Array::Logical(l), Array::Logical(q)) = (&got["L"], &got["Q"]) else {
        panic!("Logical type lost")
    };
    assert_eq!(l.data(), &[0, 1, 1, 0, 1, 0]);
    assert_eq!(l.data().as_ptr(), q.data().as_ptr());
}
#[test]
fn workspace_recovery_reuses_payloads_rolls_back_failed_publication_and_bounds_generations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let a: Array = Matrix::filled(1024, 1024, 3.).unwrap().into();
    let s = state(BTreeMap::from([("A".into(), a.clone())]));
    store.save(s.clone()).unwrap();
    let first = manifest(&path);
    let bytes = std::fs::read(&path).unwrap();
    store.save(s).unwrap();
    assert_eq!(first.entries, manifest(&path).entries);
    assert_eq!(file_names(&path).len(), 1);
    let mut failure = state(BTreeMap::from([(
        "A".into(),
        Matrix::filled(32, 32, 4.).unwrap().into(),
    )]));
    failure.figures = Some(FigureSession {
        current: 0,
        figures: vec![FigureDocument {
            window: None,
            id: 0,
            figure: Default::default(),
            path: None,
            source_version: None,
            dirty: true,
            view: None,
        }],
    });
    assert!(store.save(failure).is_err());
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    assert_eq!(file_names(&path).len(), 1);
    for value in [4., 5., 6.] {
        store
            .save(state(BTreeMap::from([(
                "A".into(),
                Matrix::scalar(value).into(),
            )])))
            .unwrap();
        assert!(file_names(&path).len() <= 2);
    }
    assert!(
        !workspace::directory(&path)
            .unwrap()
            .join(&first.entries["A"].file)
            .exists()
    );
    store.save(state(BTreeMap::new())).unwrap();
    assert!(manifest(&path).entries.is_empty());
    store.save(state(BTreeMap::new())).unwrap();
    assert!(file_names(&path).is_empty());
}
#[test]
fn workspace_recovery_rejects_corruption_traversal_alias_conflicts_and_oversized_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = Engine::default();
    e.execute("A=reshape(1:100,10,10);B=A;L=logical([1 0]);")
        .unwrap();
    state(e.workspace()).save(&path).unwrap();
    let good = manifest(&path);
    let cancel = AtomicBool::new(false);
    for kind in 0..7 {
        let mut bad = good.clone();
        match kind {
            0 => bad.version = 3, // Versions 1 and 2 are supported.
            1 => bad.entries.get_mut("A").unwrap().file = "../outside.bin".into(),
            2 => bad.entries.get_mut("A").unwrap().rows = usize::MAX,
            3 => bad.entries.get_mut("A").unwrap().bytes = usize::MAX,
            4 => bad.entries.get_mut("B").unwrap().crc32 ^= 1,
            5 => {
                let r = bad.entries.remove("A").unwrap();
                bad.entries.insert("while".into(), r);
            }
            _ => {
                let r = bad.entries["A"].clone();
                bad.entries = (0..10001).map(|i| (format!("a{i}"), r.clone())).collect();
            }
        }
        assert!(
            workspace::restore(&path, &bad, &cancel).is_err(),
            "case {kind}"
        );
    }
    let numeric = workspace::directory(&path)
        .unwrap()
        .join(&good.entries["A"].file);
    let bytes = std::fs::read(&numeric).unwrap();
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 1;
    std::fs::write(&numeric, &corrupt).unwrap();
    assert!(
        workspace::restore(&path, &good, &cancel)
            .unwrap_err()
            .contains("checksum")
    );
    std::fs::write(&numeric, bytes).unwrap();
    let mut invalid = good.clone();
    let logical = invalid.entries.get_mut("L").unwrap();
    logical.inline = Some(vec![0, 2]);
    let mut crc = flate2::Crc::new();
    crc.update(&[0, 2]);
    logical.crc32 = crc.sum();
    assert!(
        workspace::restore(&path, &invalid, &cancel)
            .unwrap_err()
            .contains("0 or 1")
    );
    assert!(
        Session::load(&path).is_ok(),
        "Corrupt workspace must not hide scripts"
    );
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    json["workspace_manifest"] = serde_json::json!({"broken":true});
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert!(loaded.workspace_manifest.unwrap().error.is_some());
    assert_eq!(loaded.documents.len(), 1);
    json.as_object_mut().unwrap().remove("workspace_manifest");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(Session::load(&path).unwrap().workspace_manifest.is_none());
}

#[test]
fn workspace_recovery_inline_threshold_and_legacy_files_preserve_exact_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let vars = BTreeMap::from([
        ("A".into(), Matrix::filled(8, 1, -0.).unwrap().into()),
        ("B".into(), Matrix::filled(9, 1, 0.75).unwrap().into()),
        (
            "L".into(),
            LogicalMatrix::filled(64, 1, true, &AtomicBool::new(false))
                .unwrap()
                .into(),
        ),
        (
            "Q".into(),
            LogicalMatrix::filled(65, 1, false, &AtomicBool::new(false))
                .unwrap()
                .into(),
        ),
    ]);
    state(vars).save(&path).unwrap();
    let mut m = manifest(&path);
    assert!(m.entries["A"].inline.is_some() && m.entries["L"].inline.is_some());
    assert!(m.entries["B"].inline.is_none() && m.entries["Q"].inline.is_none());
    assert_eq!(file_names(&path).len(), 2);
    for kind in 0..3 {
        let mut bad = m.clone();
        let a = bad.entries.get_mut("A").unwrap();
        match kind {
            0 => a.inline.as_mut().unwrap().push(0),
            1 => a.inline.as_mut().unwrap()[0] ^= 1,
            _ => a.file = "array-0123456789ABCDEF.bin".into(),
        }
        assert!(workspace::restore(&path, &bad, &AtomicBool::new(false)).is_err());
    }
    let a = m.entries.get_mut("A").unwrap();
    let bytes = a.inline.take().unwrap();
    a.file = "array-0123456789ABCDEF.bin".into();
    std::fs::write(workspace::directory(&path).unwrap().join(&a.file), bytes).unwrap();
    let recovered = workspace::restore(&path, &m, &AtomicBool::new(false)).unwrap();
    assert!(
        recovered["A"]
            .as_numeric()
            .unwrap()
            .data()
            .iter()
            .all(|v| v.to_bits() == (-0.0f64).to_bits())
    );
    assert_eq!(
        recovered["B"].scalar(8, 0).unwrap().into_real().unwrap(),
        0.75
    );
    assert_eq!(
        recovered["L"].scalar(63, 0).unwrap().into_real().unwrap(),
        1.
    );
    assert_eq!(
        recovered["Q"].scalar(64, 0).unwrap().into_real().unwrap(),
        0.
    );
}
#[test]
fn workspace_recovery_cancellation_and_truncated_payloads_never_return_partial_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = Engine::default();
    e.execute("A=1:10000;Z=2:10001;").unwrap();
    state(e.workspace()).save(&path).unwrap();
    let m = manifest(&path);
    assert!(
        workspace::restore(&path, &m, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancel")
    );
    let payload = workspace::directory(&path)
        .unwrap()
        .join(&m.entries["Z"].file);
    std::fs::write(payload, [0; 8]).unwrap();
    assert!(workspace::restore(&path, &m, &AtomicBool::new(false)).is_err());
    assert_eq!(
        e.workspace()["A"]
            .scalar(0, 0)
            .unwrap()
            .into_real()
            .unwrap(),
        1.
    );
}
#[test]
fn workspace_recovery_names_the_struct_variables_it_leaves_out() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let cancel = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("A = [1 2 3]; s.a = 1; c = {1, s}; t = struct('b', 'x');")
        .unwrap();
    let mut store = SessionStore::new(&path);
    store.save(state(e.workspace())).unwrap();
    let saved = manifest(&path);
    assert_eq!(saved.skipped, ["c", "s", "t"]);
    assert_eq!(saved.entries.keys().collect::<Vec<_>>(), ["A"]);
    assert_eq!(
        saved.skipped_note().as_deref(),
        Some("Not recovered (structs are not supported in recovery yet): c, s, t")
    );
    let restored = workspace::restore(&path, &saved, &cancel).unwrap();
    assert_eq!(restored.keys().collect::<Vec<_>>(), ["A"]);
    let mut bad = saved.clone();
    bad.skipped.push("while".into());
    assert!(workspace::restore(&path, &bad, &cancel).is_err());
    let mut many = saved.clone();
    many.skipped = (0..25).map(|i| format!("v{i}")).collect();
    assert!(many.skipped_note().unwrap().ends_with("v19 and 5 more"));
    // A later checkpoint without structs no longer reports them.
    e.execute("clear s c t").unwrap();
    store.save(state(e.workspace())).unwrap();
    let later = manifest(&path);
    assert!(later.skipped.is_empty() && later.skipped_note().is_none());
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"skipped\"")
    );
}

#[test]
fn typed_recovery_retains_all_classes_bits_shapes_and_nested_aliases() {
    use matrust::{CellArray, NumericClass, TypedArray};
    use std::sync::Arc;
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-session.json");
    let mut values = BTreeMap::new();
    for class in NumericClass::INTEGERS {
        values.insert(
            class.name().to_owned(),
            TypedArray::from_ints(
                class,
                2,
                2,
                [class.min(), class.max(), class.max() - 1, 0].into_iter(),
            )
            .unwrap()
            .into(),
        );
        values.insert(
            format!("empty_{}", class.name()),
            TypedArray::from_ints(class, 3, 0, std::iter::empty())
                .unwrap()
                .into(),
        );
    }
    let bits = [
        0x7fc00123u32,
        0x80000000,
        1,
        0x7f800000,
        0xff800000,
        0x3dcccccd,
    ];
    let single = TypedArray::from_le_bytes(
        NumericClass::Single,
        2,
        3,
        &bits
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    values.insert("single".into(), single.clone().into());
    values.insert(
        "single_reshaped".into(),
        single.reshaped(3, 2).unwrap().into(),
    );
    values.insert(
        "empty_single".into(),
        TypedArray::from_reals(NumericClass::Single, 0, 7, std::iter::empty())
            .unwrap()
            .into(),
    );
    let large = TypedArray::from_ints(
        NumericClass::Uint64,
        128,
        256,
        (0..128 * 256).map(|i| u64::MAX as i128 - i),
    )
    .unwrap();
    values.insert("large".into(), large.clone().into());
    values.insert(
        "large_reshaped".into(),
        large.reshaped(256, 128).unwrap().into(),
    );
    let slot: Arc<Array> = Arc::new(large.clone().into());
    let nested: Array = CellArray::from_shared(1, 2, vec![slot.clone(), slot], &cancel)
        .unwrap()
        .into();
    values.insert("nested".into(), nested);
    values.insert("double".into(), Matrix::scalar(2.).into());
    state(values.clone()).save(&path).unwrap();
    let m = manifest(&path);
    assert_eq!(m.version, 7);
    assert_eq!(file_names(&path).len(), 1);
    let restored = workspace::restore(&path, &m, &cancel).unwrap();
    assert_eq!(restored.len(), values.len());
    for (name, value) in &values {
        if let Array::Typed(a) = value {
            let Array::Typed(b) = &restored[name] else {
                panic!("{name} lost typed storage")
            };
            assert_eq!(
                (a.class(), a.rows(), a.cols()),
                (b.class(), b.rows(), b.cols()),
                "{name}"
            );
            assert_eq!(a.to_le_bytes(), b.to_le_bytes(), "{name}");
        }
    }
    for (a, b) in [("single", "single_reshaped"), ("large", "large_reshaped")] {
        let (Array::Typed(a), Array::Typed(b)) = (&restored[a], &restored[b]) else {
            panic!("typed")
        };
        assert!(a.same_storage(b));
    }
    let Array::Cell(cell) = &restored["nested"] else {
        panic!("cell")
    };
    assert!(
        cell.get_linear(0)
            .unwrap()
            .same_snapshot(&restored["large"])
    );
    assert!(
        cell.get_linear(1)
            .unwrap()
            .same_snapshot(&restored["large"])
    );
    assert_eq!(
        restored["double"]
            .scalar(0, 0)
            .unwrap()
            .into_real()
            .unwrap(),
        2.
    );
}

#[test]
fn typed_recovery_rejects_class_size_flags_versions_and_corrupt_payloads() {
    use matrust::{NumericClass, TypedArray};
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-corrupt.json");
    state(BTreeMap::from([
        (
            "small".into(),
            TypedArray::scalar_int(NumericClass::Uint64, u64::MAX as i128).into(),
        ),
        (
            "big".into(),
            TypedArray::from_ints(NumericClass::Int16, 1, 100, (0..100).map(i128::from))
                .unwrap()
                .into(),
        ),
    ]))
    .save(&path)
    .unwrap();
    let m = manifest(&path);
    for scenario in 0..6 {
        let mut bad = m.clone();
        let r = bad
            .entries
            .values_mut()
            .find(|r| r.numeric_class == Some(NumericClass::Uint64))
            .unwrap();
        match scenario {
            0 => r.numeric_class = Some(NumericClass::Uint8),
            1 => r.logical = true,
            2 => r.complex = true,
            3 => r.character = true,
            4 => r.inline.as_mut().unwrap()[0] ^= 1,
            _ => bad.version = 3,
        }
        assert!(
            workspace::restore(&path, &bad, &cancel).is_err(),
            "scenario {scenario}"
        );
    }
    let mut json = serde_json::to_value(&m).unwrap();
    let reference = json["entries"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    reference["numeric_class"] = "int128".into();
    assert!(serde_json::from_value::<Manifest>(json).is_err());
    let r = m.entries.values().find(|r| r.inline.is_none()).unwrap();
    let payload = workspace::directory(&path).unwrap().join(&r.file);
    let mut bytes = std::fs::read(&payload).unwrap();
    bytes[10] ^= 1;
    std::fs::write(&payload, bytes).unwrap();
    assert!(
        workspace::restore(&path, &m, &cancel)
            .unwrap_err()
            .contains("checksum")
    );
}

#[test]
fn typed_recovery_cache_reuses_unchanged_storage_and_never_reuses_after_a_write() {
    use matrust::{NumericClass, TypedArray};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-cache.json");
    let mut store = SessionStore::new(&path);
    let mut e = Engine::default();
    let typed = TypedArray::from_ints(
        NumericClass::Uint64,
        1,
        100,
        std::iter::repeat(u64::MAX as i128),
    )
    .unwrap();
    e.merge_workspace(BTreeMap::from([("x".into(), typed.into())]))
        .unwrap();
    store.save(state(e.workspace())).unwrap();
    let first = manifest(&path);
    store.save(state(e.workspace())).unwrap();
    assert_eq!(first.entries, manifest(&path).entries);
    e.execute("x(2)=uint64(9007199254740993);").unwrap();
    store.save(state(e.workspace())).unwrap();
    let changed = manifest(&path);
    assert_ne!(first.entries, changed.entries);
    let cancel = AtomicBool::new(false);
    let original = workspace::restore(&path, &first, &cancel).unwrap();
    let updated = workspace::restore(&path, &changed, &cancel).unwrap();
    assert_eq!(
        original["x"].scalar(0, 1).unwrap(),
        matrust::Scalar::Integer(u64::MAX as i128)
    );
    assert_eq!(
        updated["x"].scalar(0, 1).unwrap(),
        matrust::Scalar::Integer(9007199254740993)
    );
    let stopped = AtomicBool::new(true);
    assert!(
        workspace::restore(&path, &changed, &stopped)
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn typed_recovery_cache_frees_written_and_cleared_payloads_before_the_next_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-release.json");
    let mut store = SessionStore::new(&path);
    let mut e = Engine::default();
    e.execute("x=zeros(1,1e6,'int64'); y=ones(1,1e6,'uint32');")
        .unwrap();
    // The writer cache now holds weak identities for both payloads.
    store.save(state(e.workspace())).unwrap();
    let measure = |e: &mut Engine, statement: &str| {
        COUNTS.with(|counter| counter.set(Some((0, 0))));
        let result = e.execute(statement);
        let counts = COUNTS.with(|counter| counter.replace(None).unwrap());
        result.unwrap();
        counts
    };
    // The write detaches from the cached identity with one 8 MB copy, and the
    // old payload returns to the allocator at once, not at the next commit:
    // live memory does not grow.
    let (allocated, freed) = measure(&mut e, "x(1)=int64(-1);");
    assert!(
        freed >= 8_000_000 && allocated < freed + 64 * 1024,
        "written payload kept: allocated {allocated}, freed {freed}"
    );
    let (_, freed) = measure(&mut e, "clear y");
    assert!(freed >= 4_000_000, "cleared payload kept: freed {freed}");
    store.save(state(e.workspace())).unwrap();
    let restored = workspace::restore(&path, &manifest(&path), &AtomicBool::new(false)).unwrap();
    assert_eq!(restored.keys().collect::<Vec<_>>(), ["x"]);
    assert_eq!(
        restored["x"].scalar(0, 0).unwrap(),
        matrust::Scalar::Integer(-1)
    );
}
