//! Release-only filesystem/checkpoint observations; no codec parallelism claim.
use matrust::{
    Array, CharacterRow,
    document::Document,
    files,
    session::{Session, SessionStore, workspace},
    simulation::Model,
};
use std::{collections::BTreeMap, hint::black_box, sync::atomic::AtomicBool, time::Instant};
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

fn equal(source: &BTreeMap<String, Array>, actual: &BTreeMap<String, Array>) {
    assert_eq!(source.len(), actual.len());
    for (name, a) in source {
        let Array::Character(a) = a else { panic!() };
        let Array::Character(b) = &actual[name] else {
            panic!()
        };
        assert_eq!(
            (a.rows(), a.cols(), a.units()),
            (b.rows(), b.cols(), b.units())
        );
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use release mode");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || (args.len() == 1 && args[0] == "reverse"));
    let reverse = !args.is_empty();
    let stop = AtomicBool::new(false);
    let large = CharacterRow::from_units((0..4096).map(|i| i as u16).collect(), &stop).unwrap();
    let mut cases = vec![
        (
            "small_matrix",
            BTreeMap::from([(
                "A".into(),
                CharacterRow::from_column_major(
                    2,
                    3,
                    vec![0, 65, 955, 0xd800, 0xdc00, 0xffff],
                    &stop,
                )
                .unwrap()
                .into(),
            )]),
        ),
        (
            "large_row",
            BTreeMap::from([("A".into(), Array::from(large.clone()))]),
        ),
        (
            "large_matrix",
            BTreeMap::from([(
                "A".into(),
                Array::from(large.reshape(64, 64, &stop).unwrap()),
            )]),
        ),
    ];
    let mut empties = BTreeMap::new();
    for (i, (r, c)) in [(0, 0), (1, 0), (0, 3), (4, 0)].into_iter().enumerate() {
        empties.insert(
            format!("E{i}"),
            CharacterRow::from_column_major(r, c, vec![], &stop)
                .unwrap()
                .into(),
        );
    }
    cases.push(("empty_shapes", empties));
    if reverse {
        cases.reverse();
    }
    let workers = if reverse { [8, 1] } else { [1, 8] };
    let mut ops = vec![
        "v5_save",
        "v5_read",
        "v73_save",
        "v73_read",
        "checkpoint_unchanged",
        "reopen",
    ];
    if reverse {
        ops.reverse();
    }
    println!("case,workers,operation,repetition,elapsed_ns,logical_bytes,variables");
    for worker in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker)
            .build()
            .unwrap();
        for (name, source) in &cases {
            let dir = tempfile::tempdir().unwrap();
            let v5 = dir.path().join("v5.mat");
            let v73 = dir.path().join("v73.mat");
            let recovery = dir.path().join("session.json");
            let mut store = SessionStore::new(&recovery);
            files::save_array_workspace(&v5, source, &stop).unwrap();
            files::save_array_workspace_v73(&v73, source, &stop).unwrap();
            store.save(session(source.clone())).unwrap();
            let names = source.keys().cloned().collect();
            let bytes: usize = source.values().map(Array::bytes).sum();
            for &op in &ops {
                for sample in 0..9 {
                    let state = if op == "checkpoint_unchanged" {
                        Some(session(source.clone()))
                    } else {
                        None
                    };
                    let (ns, result) = pool.install(|| {
                        let start = Instant::now();
                        let result = match op {
                            "v5_save" => {
                                files::save_array_workspace(&v5, source, &stop).unwrap();
                                None
                            }
                            "v73_save" => {
                                files::save_array_workspace_v73(&v73, source, &stop).unwrap();
                                None
                            }
                            "v5_read" | "v73_read" => {
                                let path = if op == "v5_read" { &v5 } else { &v73 };
                                let cat = files::inspect_workspace(path, &stop).unwrap();
                                Some(files::load_selected_arrays(&cat, &names, &stop).unwrap())
                            }
                            "checkpoint_unchanged" => {
                                store.save(state.unwrap()).unwrap();
                                None
                            }
                            "reopen" => {
                                let s = Session::load(&recovery).unwrap();
                                Some(
                                    workspace::restore(
                                        &recovery,
                                        s.workspace_manifest.as_ref().unwrap(),
                                        &stop,
                                    )
                                    .unwrap(),
                                )
                            }
                            _ => unreachable!(),
                        };
                        (start.elapsed().as_nanos(), result)
                    });
                    if let Some(actual) = &result {
                        equal(source, actual);
                    } else if op == "checkpoint_unchanged" {
                        let saved = Session::load(&recovery).unwrap();
                        equal(
                            source,
                            &workspace::restore(
                                &recovery,
                                saved.workspace_manifest.as_ref().unwrap(),
                                &stop,
                            )
                            .unwrap(),
                        );
                    } else {
                        let path = if op == "v5_save" { &v5 } else { &v73 };
                        let cat = files::inspect_workspace(path, &stop).unwrap();
                        equal(
                            source,
                            &files::load_selected_arrays(&cat, &names, &stop).unwrap(),
                        );
                    }
                    if sample >= 2 {
                        println!(
                            "{name},{worker},{op},{},{ns},{bytes},{}",
                            sample - 2,
                            source.len()
                        );
                    }
                    black_box(result);
                }
            }
        }
    }
}
