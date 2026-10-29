use super::*;
use crate::{CellArray, CharacterRow};
use std::sync::atomic::Ordering;
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        STOP_AFTER.with(|s| s.set(None));
        crate::session::STOP_BEFORE_COMMIT.with(|s| s.set(false));
    }
}
fn names(path: &Path) -> BTreeSet<String> {
    std::fs::read_dir(directory(path).unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_owned())
        .collect()
}
#[test]
fn active_leaf_write_and_read_cancellation_discard_private_work_and_retry() {
    let _reset = Reset;
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = Store::new(&path);
    let old: Array = Matrix::filled(20000, 1, 1.).unwrap().into();
    store.commit(
        store
            .stage(BTreeMap::from([("A".into(), old.clone())]))
            .unwrap(),
    );
    let before = names(&path);
    STOP_AFTER.with(|s| s.set(Some(1)));
    let changed: Array = Matrix::filled(20000, 1, 2.).unwrap().into();
    assert!(
        store
            .stage_with_cancel(BTreeMap::from([("A".into(), changed.clone())]), &cancel)
            .is_err()
    );
    assert!(cancel.load(Ordering::Relaxed));
    assert_eq!(names(&path), before);
    cancel.store(false, Ordering::Relaxed);
    let pending = store
        .stage_with_cancel(BTreeMap::from([("A".into(), changed)]), &cancel)
        .unwrap();
    STOP_AFTER.with(|s| s.set(Some(1)));
    assert!(restore(&path, &pending.manifest, &cancel).is_err());
    assert!(cancel.load(Ordering::Relaxed));
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(
        restore(&path, &pending.manifest, &cancel).unwrap()["A"]
            .as_numeric()
            .unwrap()
            .data()[19999],
        2.
    );
    drop(pending);
    assert_eq!(names(&path), before);
    assert_eq!(old.as_numeric().unwrap().data()[0], 1.);
}
#[test]
fn graph_planning_cancellation_and_weak_caches_do_not_retain_payloads() {
    let _reset = Reset;
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = Store::new(&path);
    let text = CharacterRow::from_units(vec![0xd800; 65536], &cancel).unwrap();
    let weak = text.weak_storage();
    let shared = Arc::new(text.clone().into());
    let cell = CellArray::from_shared(1, 1024, vec![shared; 1024], &cancel).unwrap();
    let weak_cell = cell.weak_storage();
    // Logical payload is ~128 MiB; physical character storage is one 128 KiB allocation.
    let values = BTreeMap::from([("C".into(), cell.into())]);
    drop(text);
    STOP_AFTER.with(|s| s.set(Some(10)));
    assert!(store.stage_with_cancel(values.clone(), &cancel).is_err());
    assert!(!directory(&path).unwrap().exists());
    assert!(cancel.load(Ordering::Relaxed));
    cancel.store(false, Ordering::Relaxed);
    let pending = store.stage_with_cancel(values, &cancel).unwrap();
    assert!(weak.upgrade().is_none());
    assert!(weak_cell.upgrade().is_none());
    store.commit(pending);
    assert!(weak.upgrade().is_none());
    assert!(weak_cell.upgrade().is_none());
}
#[test]
fn graph_validation_recomputes_transitive_slot_and_depth_limits() {
    use graph::{Edge, Graph, Node};
    let cancel = AtomicBool::new(false);
    let mut graph = Graph {
        roots: BTreeMap::new(),
        graphics: None,
        nodes: vec![Node::Cell { children: vec![] }],
    };
    for i in 1..17 {
        graph.nodes.push(Node::Cell {
            children: vec![Edge {
                node: i - 1,
                rows: usize::from(i > 1),
                cols: usize::from(i > 1),
            }],
        });
    }
    graph.roots.insert(
        "C".into(),
        Edge {
            node: 16,
            rows: 1,
            cols: 1,
        },
    );
    assert!(
        graph
            .validate(&BTreeMap::new(), &cancel)
            .unwrap_err()
            .contains("depth")
    );
    let mut graph = Graph {
        roots: BTreeMap::new(),
        graphics: None,
        nodes: vec![Node::Cell { children: vec![] }],
    };
    graph.nodes.push(Node::Cell {
        children: vec![
            Edge {
                node: 0,
                rows: 0,
                cols: 0
            };
            32769
        ],
    });
    graph.nodes.push(Node::Cell {
        children: vec![
            Edge {
                node: 1,
                rows: 1,
                cols: 32769
            };
            2
        ],
    });
    graph.roots.insert(
        "C".into(),
        Edge {
            node: 2,
            rows: 1,
            cols: 2,
        },
    );
    assert!(
        graph
            .validate(&BTreeMap::new(), &cancel)
            .unwrap_err()
            .contains("slot")
    );
}

#[test]
fn stop_at_json_commit_rolls_back_new_workspace_and_figure_files_then_retries() {
    use crate::{
        document::Document,
        session::{
            Session, SessionStore,
            figures::{FigureDocument, FigureSession},
        },
        simulation::Model,
    };
    fn state(value: u16, cancel: &AtomicBool) -> Session {
        let text: Array = CharacterRow::from_units(vec![value; 65536], cancel)
            .unwrap()
            .into();
        let cell: Array = CellArray::from_shared(1, 1, vec![Arc::new(text)], cancel)
            .unwrap()
            .into();
        let figure = crate::plot::Figure {
            axes: crate::plot::Axes {
                title: format!("Generation {value}"),
                ..Default::default()
            },
            ..Default::default()
        };
        Session {
            layout_presets: Default::default(),
            export: Default::default(),
            workspace: Some(BTreeMap::from([("C".into(), cell)])),
            workspace_manifest: None,
            preview: None,
            eps_renderer: None,
            figure_manifest: None,
            figures: Some(FigureSession {
                current: 1,
                figures: vec![FigureDocument {
                    window: None,
                    id: 1,
                    figure,
                    path: None,
                    source_version: None,
                    dirty: true,
                    view: None,
                }],
            }),
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
    fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                files.insert(entry.path(), Vec::new());
                files.extend(tree(&entry.path()));
            } else {
                files.insert(entry.path(), std::fs::read(entry.path()).unwrap());
            }
        }
        files
    }
    let _reset = Reset;
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    store.save_with_cancel(state(65, &cancel), &cancel).unwrap();
    let before = tree(dir.path());
    let replacement = state(66, &cancel);
    crate::session::STOP_BEFORE_COMMIT.with(|armed| armed.set(true));
    assert!(
        store
            .save_with_cancel(replacement.clone(), &cancel)
            .unwrap_err()
            .contains("cancel")
    );
    crate::session::STOP_BEFORE_COMMIT.with(|armed| {
        assert!(
            !armed.get(),
            "Final publication checkpoint actually executed"
        )
    });
    assert!(cancel.load(Ordering::Relaxed));
    assert_eq!(
        tree(dir.path()),
        before,
        "Both newly staged generations were removed"
    );
    cancel.store(false, Ordering::Relaxed);
    store.save_with_cancel(replacement, &cancel).unwrap();
    let session = Session::load(&path).unwrap();
    let restored = restore(&path, &session.workspace_manifest.unwrap(), &cancel).unwrap();
    let Array::Cell(c) = &restored["C"] else {
        panic!()
    };
    let Array::Character(text) = c.get_linear(0).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(text.units().len(), 65536);
    assert!(text.units().iter().all(|u| *u == 66));
    assert_ne!(tree(dir.path()), before);
}
