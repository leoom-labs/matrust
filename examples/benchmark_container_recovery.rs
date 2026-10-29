//! Serial checkpoint I/O; configured pool sizes do not imply a parallel codec.
use matrust::{
    Array, CellArray, CharacterRow, Matrix,
    document::Document,
    session::{Session, SessionStore, workspace},
    simulation::Model,
};
use std::{
    collections::BTreeMap,
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
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
fn fixture(name: &str, changed: bool, cancel: &AtomicBool) -> BTreeMap<String, Array> {
    let number = if changed { 2. } else { 1. };
    let text: Array = CharacterRow::from_units(
        vec![if changed { 0xd800 } else { 955 }; if name == "max_char" { 65536 } else { 16 }],
        cancel,
    )
    .unwrap()
    .into();
    let leaf: Array = Matrix::filled(if name == "shared_large" { 262144 } else { 16 }, 1, number)
        .unwrap()
        .into();
    if name == "numeric_control" {
        return BTreeMap::from([("A".into(), leaf)]);
    }
    if name == "max_char" {
        return BTreeMap::from([("T".into(), text.clone()), ("U".into(), text)]);
    }
    let count = match name {
        "wide" => 4096,
        "shared_large" => 8,
        _ => 2,
    };
    let shared = Arc::new(leaf.clone());
    let mut slots = vec![shared; count];
    slots[0] = Arc::new(text);
    let mut cell = CellArray::from_shared(1, count, slots, cancel).unwrap();
    if name == "deep" {
        for _ in 1..16 {
            cell = CellArray::from_shared(1, 1, vec![Arc::new(cell.into())], cancel).unwrap();
        }
    }
    BTreeMap::from([
        ("A".into(), leaf),
        ("C".into(), cell.clone().into()),
        (
            "R".into(),
            cell.reshape(cell.len(), 1, cancel).unwrap().into(),
        ),
    ])
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
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                equal(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Unexpected recovery fixture type"),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut shapes = [
        "numeric_control",
        "small",
        "wide",
        "deep",
        "shared_large",
        "max_char",
    ];
    let mut workers = [1, 8];
    let mut ops = ["first", "unchanged", "changed_leaf", "restore"];
    if !args.is_empty() {
        shapes.reverse();
        workers.reverse();
        ops.reverse();
    }
    let cancel = AtomicBool::new(false);
    println!("case,workers,operation,repetition,nanoseconds,logical_bytes,leaf_files,json_bytes");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for name in shapes {
            for operation in ops {
                for repetition in 0..9 {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("session.json");
                    let original = fixture(name, false, &cancel);
                    let mut store = SessionStore::new(&path);
                    if operation != "first" {
                        store.save(state(original.clone())).unwrap();
                    }
                    let mut selected = original.clone();
                    if operation == "changed_leaf" {
                        // Replace one root leaf; unchanged cells keep sharing the original nested allocation.
                        let (key, value) = if name == "max_char" {
                            (
                                "T",
                                CharacterRow::from_units(vec![0xd800; 65536], &cancel)
                                    .unwrap()
                                    .into(),
                            )
                        } else {
                            (
                                "A",
                                Matrix::filled(
                                    if name == "shared_large" { 262144 } else { 16 },
                                    1,
                                    2.,
                                )
                                .unwrap()
                                .into(),
                            )
                        };
                        selected.insert(key.into(), value);
                    }
                    let checkpoint = state(selected.clone());
                    let start = Instant::now();
                    let restored = pool.install(|| {
                        if operation == "restore" {
                            let manifest =
                                Session::load(&path).unwrap().workspace_manifest.unwrap();
                            Some(workspace::restore(&path, &manifest, &cancel).unwrap())
                        } else {
                            store.save_with_cancel(checkpoint, &cancel).unwrap();
                            None
                        }
                    });
                    let nanos = start.elapsed().as_nanos();
                    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
                    let got = restored
                        .unwrap_or_else(|| workspace::restore(&path, &manifest, &cancel).unwrap());
                    assert_eq!(got.len(), selected.len());
                    for (key, value) in &selected {
                        equal(value, &got[key]);
                    }
                    if let (Some(Array::Cell(c)), Some(Array::Cell(r))) =
                        (got.get("C"), got.get("R"))
                    {
                        assert!(c.same_storage(r));
                    }
                    for (key, value) in &original {
                        if operation != "changed_leaf" {
                            assert!(selected[key].same_snapshot(value));
                        }
                    }
                    black_box(&got);
                    if repetition >= 2 {
                        println!(
                            "{name},{workers},{operation},{},{nanos},{},{},{}",
                            repetition - 2,
                            selected.values().map(Array::bytes).sum::<usize>(),
                            manifest
                                .entries
                                .values()
                                .filter(|r| r.inline.is_none())
                                .count(),
                            std::fs::metadata(&path).unwrap().len()
                        );
                    }
                }
            }
        }
    }
}
