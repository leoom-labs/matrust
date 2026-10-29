//! Complete checkpoint/readback workloads; configured pools do not parallelize the codec.
use matrust::{
    Array, CellArray, CharacterRow, ComplexMatrix, Engine, GraphicsArray, Matrix,
    complex_matrix::Complex64,
    document::Document,
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, restore_figures},
        workspace,
    },
    simulation::Model,
};
use std::{
    collections::BTreeMap,
    hint::black_box,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

const CASES: [&str; 7] = [
    "ordinary_v1",
    "ordinary_v2",
    "ordinary_v3",
    "handles_small",
    "handles_large",
    "handles_many",
    "partial_large",
];
const HANDLE_NAMES: [&str; 8] = ["f", "a", "h", "lg", "t", "other", "other_line", "old"];

fn scene() -> Engine {
    let mut e = Engine::default();
    // Replacing a real line makes old genuinely deleted. Both saved figures remain live.
    e.execute("f=figure(7);a=gca;old=plot(a,[1 2],[0 3]);h=plot(a,[1 2],[-0 4]);lg=legend(a,h,{'line'},'Interpreter','none','AutoUpdate','off');t=annotation(f,'textbox',[.1 .8 .8 .1],'String','literal <&>','Interpreter','none','EdgeColor','none');other=figure(9);other_line=plot([1 2],[7 8]);").unwrap();
    e
}
fn figures(e: &Engine) -> FigureSession {
    FigureSession {
        current: e.current_figure,
        figures: e
            .figures()
            .map(|(id, f)| FigureDocument {
                window: None,
                id,
                figure: f.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            })
            .collect(),
    }
}
fn state(values: BTreeMap<String, Array>, figures: FigureSession) -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(values),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: Some(figures),
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
fn fixture(name: &str, scene: &Engine, cancel: &AtomicBool) -> BTreeMap<String, Array> {
    let len = if matches!(name, "handles_small" | "handles_many") {
        16
    } else {
        262144
    };
    let a: Array = if name == "ordinary_v2" {
        ComplexMatrix::filled(len, 1, Complex64::new(-0., 3.25), cancel)
            .unwrap()
            .into()
    } else {
        Matrix::filled(len, 1, -0.).unwrap().into()
    };
    let mut values = BTreeMap::from([("A".into(), a.clone()), ("AA".into(), a.clone())]);
    if matches!(name, "ordinary_v1" | "ordinary_v2") {
        return values;
    }
    let text: Array = CharacterRow::from_units(vec![0xd800; 16], cancel)
        .unwrap()
        .into();
    let leaf = Arc::new(a.clone());
    let c = CellArray::from_shared(
        1,
        4,
        vec![Arc::new(text), leaf.clone(), leaf.clone(), leaf],
        cancel,
    )
    .unwrap();
    values.insert("R".into(), c.reshape(2, 2, cancel).unwrap().into());
    values.insert("C".into(), c.into());
    if name == "ordinary_v3" {
        return values;
    }
    let n = if name == "handles_many" { 4096 } else { 8 };
    let handles: Vec<_> = HANDLE_NAMES
        .iter()
        .map(|name| {
            let Array::Graphics(g) = scene.array(name).unwrap() else {
                panic!()
            };
            g.get_linear(0).unwrap().clone()
        })
        .collect();
    let g = GraphicsArray::from_shared(
        1,
        n,
        (0..n).map(|i| handles[i % 8].clone()).collect(),
        cancel,
    )
    .unwrap();
    let array: Array = g.into();
    values.insert("G".into(), array.clone());
    values.insert("GA".into(), array.clone());
    let h = CellArray::from_shared(1, 2, vec![Arc::new(array), Arc::new(a)], cancel).unwrap();
    values.insert("HR".into(), h.reshape(2, 1, cancel).unwrap().into());
    values.insert("H".into(), h.into());
    assert!(values.values().map(Array::bytes).sum::<usize>() < 256 * 1024 * 1024);
    values
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
        (Array::Complex(a), Array::Complex(b)) => assert!(
            a.data()
                .iter()
                .zip(b.data())
                .all(|(a, b)| a.re.to_bits() == b.re.to_bits() && a.im.to_bits() == b.im.to_bits())
        ),
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                equal(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        (Array::Graphics(a), Array::Graphics(b)) => {
            for i in 0..a.len() {
                // Kinds/shape are preserved, but restored identity tokens must be fresh.
                assert_eq!(
                    a.get_linear(i).unwrap().kind(),
                    b.get_linear(i).unwrap().kind()
                );
                assert!(
                    !a.get_linear(i)
                        .unwrap()
                        .same_object(b.get_linear(i).unwrap())
                );
            }
        }
        _ => panic!("Unexpected fixture type"),
    }
}
fn recover(path: &Path, engine: &mut Engine, cancel: &AtomicBool) -> Vec<String> {
    let saved = Session::load(path).unwrap();
    let report = restore_figures(path, saved.figure_manifest.as_ref().unwrap(), cancel);
    let binding = report.workspace_bindings().clone();
    let mut warnings = report.warnings;
    engine
        .restore_figures(
            report.figures.current,
            report
                .figures
                .figures
                .into_iter()
                .map(|d| (d.id, d.figure))
                .collect(),
        )
        .unwrap();
    let got = workspace::restore_with_figures(
        path,
        saved.workspace_manifest.as_ref().unwrap(),
        &binding,
        cancel,
    )
    .unwrap();
    warnings.extend(got.warnings);
    engine.merge_workspace(got.values).unwrap();
    warnings
}
fn validate(
    name: &str,
    source: &BTreeMap<String, Array>,
    expected: &FigureSession,
    got: &mut Engine,
    warnings: &[String],
) {
    assert_eq!(got.workspace_count(), source.len());
    for (key, value) in source {
        equal(value, &got.array(key).unwrap());
    }
    assert!(
        got.array("A")
            .unwrap()
            .same_snapshot(&got.array("AA").unwrap())
    );
    if let (Some(Array::Cell(c)), Some(Array::Cell(r))) = (got.array("C"), got.array("R")) {
        assert!(c.same_storage(&r));
        assert!(
            c.get_linear(1)
                .unwrap()
                .same_snapshot(&got.array("A").unwrap())
        );
    }
    let partial = name == "partial_large";
    assert_eq!(warnings.is_empty(), !partial);
    let actual: Vec<_> = got.figures().collect();
    assert_eq!(actual.len(), if partial { 1 } else { 2 });
    for (id, figure) in actual {
        let before = &expected.figures.iter().find(|d| d.id == id).unwrap().figure;
        assert!(!before.identity.same_object(&figure.identity));
        assert_eq!(before.series.len(), figure.series.len());
        for (a, b) in before.series.iter().zip(&figure.series) {
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.points.len(), b.points.len());
            for (p, q) in a.points.iter().zip(b.points.iter()) {
                assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
            }
        }
    }
    if let Some(Array::Graphics(g)) = got.array("G") {
        let Array::Graphics(alias) = got.array("GA").unwrap() else {
            panic!()
        };
        assert!(g.same_storage(&alias));
        let (Some(Array::Cell(h)), Some(Array::Cell(hr))) = (got.array("H"), got.array("HR"))
        else {
            panic!()
        };
        assert!(h.same_storage(&hr));
        assert!(
            h.get_linear(0)
                .unwrap()
                .same_snapshot(&got.array("G").unwrap())
        );
        assert!(
            h.get_linear(1)
                .unwrap()
                .same_snapshot(&got.array("A").unwrap())
        );
        for i in 0..g.len() {
            assert!(
                g.get_linear(i)
                    .unwrap()
                    .same_object(g.get_linear(i % 8).unwrap())
            );
        }
        for i in 0..8 {
            for j in i + 1..8 {
                assert!(
                    !g.get_linear(i)
                        .unwrap()
                        .same_object(g.get_linear(j).unwrap())
                );
            }
        }
        // These are actual engine queries over rebound handles, including deleted old.
        got.execute("live=isgraphics(G);").unwrap();
        let expected = [
            !partial, !partial, !partial, !partial, !partial, true, true, false,
        ];
        assert_eq!(got.logical_variables["live"].data().len(), g.len());
        for (i, &v) in got.logical_variables["live"].data().iter().enumerate() {
            assert_eq!(v, u8::from(expected[i % 8]));
        }
        if partial {
            got.execute("figure(7);still_dead=isgraphics(G(3));")
                .unwrap();
            assert_eq!(got.logical_variables["still_dead"].data(), &[0]);
        } else {
            got.execute("set(G(3),'DisplayName','rebound');").unwrap();
            assert_eq!(
                got.figures().find(|(id, _)| *id == 7).unwrap().1.series[0].label,
                "rebound"
            );
        }
    }
}
fn remove_owned_figure(path: &Path, artifacts: &Path, workers: usize, rep: usize) {
    let saved = Session::load(path).unwrap();
    let entry = saved
        .figure_manifest
        .unwrap()
        .entries
        .into_iter()
        .find(|e| e.id == 7)
        .unwrap();
    let owned = path.parent().unwrap().canonicalize().unwrap();
    let matches: Vec<_> = std::fs::read_dir(&owned)
        .unwrap()
        .map(Result::unwrap)
        .filter_map(|d| {
            if d.file_type().unwrap().is_dir() {
                let p = d.path().join(&entry.file);
                p.is_file().then_some(p)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(matches.len(), 1);
    let payload = matches[0].canonicalize().unwrap();
    assert!(payload.starts_with(&owned) && payload != owned);
    let bytes = std::fs::read(&payload).unwrap();
    assert_eq!(bytes.len() as u64, entry.bytes);
    let mut crc = flate2::Crc::new();
    crc.update(&bytes);
    assert_eq!(crc.sum(), entry.crc32);
    let stem = format!("partial-{workers}-{rep}");
    let preserved = artifacts.join(format!("{stem}.mfig"));
    std::fs::write(&preserved, &bytes).unwrap();
    assert_eq!(std::fs::read(&preserved).unwrap(), bytes);
    std::fs::remove_file(&payload).unwrap();
    assert!(!payload.exists());
    std::fs::write(artifacts.join(format!("{stem}.json")),serde_json::to_vec_pretty(&serde_json::json!({
        "workers":workers,"iteration":rep,"removed_figure":7,"private_root":owned,"removed_path":payload,
        "preserved_file":preserved.file_name().unwrap().to_str().unwrap(),"bytes":entry.bytes,"crc32":entry.crc32,
        "removed_after_preservation":true,"removal_outside_timer":true
    })).unwrap()).unwrap();
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.len() == 2 || (args.len() == 3 && args[2] == "--reverse"));
    assert_eq!(args[0], "--artifacts");
    let artifacts = PathBuf::from(&args[1]);
    std::fs::create_dir(&artifacts).unwrap();
    let reverse = args.len() == 3;
    let mut cases = CASES;
    let mut workers = [1, 8];
    let mut ops = ["first", "unchanged", "restore"];
    if reverse {
        cases.reverse();
        workers.reverse();
        ops.reverse();
    }
    let cancel = AtomicBool::new(false);
    let scene = scene();
    let expected_figures = figures(&scene);
    let mut checked = 0;
    println!(
        "case,workers,operation,repetition,nanoseconds,version,logical_bytes,leaf_files,figure_files,leaf_bytes,figure_bytes,json_bytes,handle_slots,cell_slot_bytes,handle_bytes"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for case in cases {
            for op in ops {
                if case == "partial_large" && op != "restore" {
                    continue;
                }
                for iteration in 0..9 {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("session.json");
                    let source = fixture(case, &scene, &cancel);
                    let checkpoint = if op == "restore" {
                        None
                    } else {
                        Some(state(source.clone(), expected_figures.clone()))
                    };
                    let mut store = SessionStore::new(&path);
                    if op != "first" {
                        store
                            .save(state(source.clone(), expected_figures.clone()))
                            .unwrap();
                    }
                    if case == "partial_large" {
                        remove_owned_figure(&path, &artifacts, workers, iteration);
                    }
                    let mut restored = Engine::default();
                    let start = Instant::now();
                    let warnings = pool.install(|| {
                        if op == "restore" {
                            recover(&path, &mut restored, &cancel)
                        } else {
                            store
                                .save_with_cancel(checkpoint.unwrap(), &cancel)
                                .unwrap();
                            Vec::new()
                        }
                    });
                    let nanos = start.elapsed().as_nanos();
                    let warnings = if op == "restore" {
                        warnings
                    } else {
                        recover(&path, &mut restored, &cancel)
                    };
                    let saved = Session::load(&path).unwrap();
                    let manifest = saved.workspace_manifest.unwrap();
                    let version = match case {
                        "ordinary_v1" => 1,
                        "ordinary_v2" => 2,
                        "ordinary_v3" => 3,
                        _ => 4,
                    };
                    assert_eq!(manifest.version, version);
                    if version < 4 {
                        assert!(
                            !serde_json::to_string(&manifest)
                                .unwrap()
                                .contains("\"graphics\"")
                        );
                    } else {
                        let encoded = serde_json::to_value(&manifest).unwrap();
                        assert_eq!(
                            encoded["graph"]["graphics"]["handles"]
                                .as_array()
                                .unwrap()
                                .len(),
                            8
                        );
                    }
                    validate(case, &source, &expected_figures, &mut restored, &warnings);
                    checked += 1;
                    let slots = if case == "handles_many" {
                        4096
                    } else if version == 4 {
                        8
                    } else {
                        0
                    };
                    assert_eq!(
                        manifest
                            .entries
                            .values()
                            .filter(|r| r.inline.is_none())
                            .map(|r| &r.file)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len(),
                        1
                    );
                    let figure_manifest = saved.figure_manifest.unwrap();
                    assert_eq!(figure_manifest.entries.len(), 2);
                    let figure_bytes: u64 = figure_manifest.entries.iter().map(|r| r.bytes).sum();
                    let leaf_bytes = manifest
                        .entries
                        .values()
                        .find(|r| r.inline.is_none())
                        .unwrap()
                        .bytes;
                    black_box(&restored);
                    if iteration >= 2 {
                        println!(
                            "{case},{workers},{op},{},{nanos},{version},{},1,2,{leaf_bytes},{figure_bytes},{},{slots},{},{}",
                            iteration - 2,
                            source.values().map(Array::bytes).sum::<usize>(),
                            std::fs::metadata(&path).unwrap().len(),
                            std::mem::size_of::<Arc<Array>>() + std::mem::size_of::<Array>(),
                            std::mem::size_of::<matrust::GraphicsHandle>()
                                + std::mem::size_of::<Arc<matrust::GraphicsHandle>>()
                                + 8 * std::mem::size_of::<usize>()
                        );
                    }
                }
            }
        }
    }
    assert_eq!(checked, 342);
    std::fs::write(artifacts.join("validation.json"),serde_json::to_vec_pretty(&serde_json::json!({"checked":checked,"retained":266,"order":if reverse {"reverse"}else{"normal"},"exact_values":true,"aliases":true,"live_deleted_partial_handles":true,"private_removals":18})).unwrap()).unwrap();
}
