use matrust::{
    Array, Engine, Matrix,
    document::Document,
    session::{Session, SessionStore},
    simulation::Model,
};
use std::collections::BTreeMap;

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
fn tree(path: &std::path::Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
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
#[test]
fn recovery_restores_containers_without_omission_and_preserves_source_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    let keep: Array = Matrix::filled(100, 100, 7.).unwrap().into();
    store
        .save(session(BTreeMap::from([("keep".into(), keep)])))
        .unwrap();
    let mut e = Engine::default();
    e.execute("A=[1 2];C={A};s='kept';").unwrap();
    let values = e.workspace();
    store.save(session(values.clone())).unwrap();
    let before = tree(dir.path());
    store.save(session(values.clone())).unwrap();
    let after = tree(dir.path());
    // A third checkpoint can collect the unreferenced first generation. Every
    // retained artifact, including the current JSON, must remain byte-identical.
    assert!(
        after
            .get(&path)
            .is_some_and(|bytes| before.get(&path) == Some(bytes))
    );
    for (file, bytes) in &after {
        assert!(
            before.get(file) == Some(bytes),
            "Unchanged checkpoint rewrote or added {file:?}"
        );
    }
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    let got = matrust::session::workspace::restore(
        &path,
        &manifest,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(got.len(), 3);
    let Array::Cell(cell) = &got["C"] else {
        panic!()
    };
    assert!(cell.get_linear(0).unwrap().same_snapshot(&got["A"]));
    let Array::Character(text) = &got["s"] else {
        panic!()
    };
    assert_eq!(text.units(), &[107, 101, 112, 116]);
    for (name, value) in &values {
        assert!(value.same_snapshot(&e.array(name).unwrap()));
    }
    e.execute("clear C s;").unwrap();
    store.save(session(e.workspace())).unwrap();
    let recovered = Session::load(&path).unwrap();
    assert_eq!(recovered.workspace_manifest.unwrap().entries.len(), 1);
}
