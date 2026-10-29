use matrust::{
    Array, Engine, GraphicsArray,
    document::Document,
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, restore_figures},
        workspace,
    },
    simulation::Model,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn snapshot(e: &Engine) -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(e.workspace()),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: Some(FigureSession {
            current: e.current_figure,
            figures: e
                .figures()
                .map(|(id, f)| FigureDocument {
                    id,
                    figure: f.clone(),
                    path: None,
                    source_version: None,
                    dirty: true,
                    view: None,
                    window: None,
                })
                .collect(),
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
fn handles(e: &Engine, name: &str) -> GraphicsArray {
    let Array::Graphics(a) = e.array(name).unwrap() else {
        panic!("Expected Graphics {name}")
    };
    a
}
fn recover(path: &Path) -> (Engine, Vec<String>) {
    let s = Session::load(path).unwrap();
    let cancel = AtomicBool::new(false);
    let report = restore_figures(path, s.figure_manifest.as_ref().unwrap(), &cancel);
    let binding = report.workspace_bindings().clone();
    let mut warnings = report.warnings;
    let mut e = Engine::default();
    e.restore_figures(
        report.figures.current,
        report
            .figures
            .figures
            .into_iter()
            .map(|d| (d.id, d.figure))
            .collect(),
    )
    .unwrap();
    let recovered = workspace::restore_with_figures(
        path,
        s.workspace_manifest.as_ref().unwrap(),
        &binding,
        &cancel,
    )
    .unwrap();
    warnings.extend(recovered.warnings);
    e.merge_workspace(recovered.values).unwrap();
    (e, warnings)
}

#[test]
fn v5_character_views_and_v4_live_graphics_rebind_in_one_checkpoint() {
    let mut e = Engine::default();
    e.execute("f=figure(7);h=plot([1 2],[3 4]);s=char([65 67;66 68]);r=reshape(s,1,4);C={h,s};")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    SessionStore::new(&path).save(snapshot(&e)).unwrap();
    assert_eq!(
        Session::load(&path)
            .unwrap()
            .workspace_manifest
            .unwrap()
            .version,
        5
    );
    let (mut restored, warnings) = recover(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    restored.execute("live=isgraphics(h);").unwrap();
    assert_eq!(restored.logical_variables["live"].data(), &[1]);
    assert!(
        !handles(&e, "h")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&restored, "h").scalar_handle().unwrap())
    );
    let Array::Character(s) = restored.array("s").unwrap() else {
        panic!()
    };
    let Array::Character(r) = restored.array("r").unwrap() else {
        panic!()
    };
    assert_eq!((s.rows(), s.cols()), (2, 2));
    assert_eq!(s.units(), &[65, 66, 67, 68]);
    assert!(s.same_storage(&r));
    let Array::Cell(c) = restored.array("C").unwrap() else {
        panic!()
    };
    assert!(
        c.get_linear(0)
            .unwrap()
            .same_snapshot(&restored.array("h").unwrap())
    );
    assert!(c.get_linear(1).unwrap().same_snapshot(&s.into()));
}
