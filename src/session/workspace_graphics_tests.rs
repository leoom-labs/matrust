use super::*;
use std::sync::atomic::Ordering;
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        STOP_AFTER.with(|s| s.set(None));
        crate::session::STOP_BEFORE_COMMIT.with(|s| s.set(false));
    }
}
fn fixture() -> (crate::Engine, FigureSession, FigureManifest) {
    let mut e = crate::Engine::default();
    e.execute("f=figure(3);a=gca;h=plot(a,[1 2],[3 4]);")
        .unwrap();
    let figures = FigureSession {
        current: 3,
        figures: e
            .figures()
            .map(|(id, f)| crate::session::figures::FigureDocument {
                window: None,
                id,
                figure: f.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            })
            .collect(),
    };
    let manifest = FigureManifest {
        version: 1,
        current: 3,
        error: None,
        entries: vec![FigureReference {
            window: Default::default(),
            id: 3,
            file: "figure-0123456789abcdef.mfig".into(),
            bytes: 123,
            crc32: 17,
            path: None,
            source_version: None,
            dirty: true,
            view: None,
        }],
    };
    (e, figures, manifest)
}
fn table(
    e: &crate::Engine,
    figures: &FigureSession,
    manifest: &FigureManifest,
    cancel: &AtomicBool,
) -> Table {
    let mut p = Planner::new(figures, manifest, cancel).unwrap();
    for name in ["f", "a", "h"] {
        let Array::Graphics(a) = e.array(name).unwrap() else {
            panic!()
        };
        p.handle(a.scalar_handle().unwrap(), cancel).unwrap();
    }
    p.table
}
#[test]
fn graph_identity_schema_rejects_corrupt_indices_roles_unused_records_and_overlimits() {
    let cancel = AtomicBool::new(false);
    let (e, figures, manifest) = fixture();
    let original = table(&e, &figures, &manifest, &cancel);
    original.validate(&cancel).unwrap();
    let mut bad = original.clone();
    bad.handles[2].object = bad.identities.len();
    assert!(bad.validate(&cancel).is_err());
    let mut bad = original.clone();
    bad.handles[2].kind = Kind::Figure;
    assert!(bad.validate(&cancel).is_err());
    let mut bad = original.clone();
    bad.handles[2].axes = None;
    assert!(bad.validate(&cancel).is_err());
    let mut bad = original.clone();
    bad.identities.push(None);
    assert!(bad.validate(&cancel).is_err());
    let mut bad = original.clone();
    bad.identities[1] = bad.identities[0].clone();
    assert!(bad.validate(&cancel).is_err());
    let mut bad = original.clone();
    bad.handles = vec![bad.handles[0].clone(); MAX_HANDLES + 1];
    assert!(bad.validate(&cancel).is_err());
    let mut bad = original.clone();
    bad.identities = vec![None; MAX_IDENTITIES + 1];
    assert!(bad.validate(&cancel).is_err());
    assert!(original.validate_budget(LIMIT / HANDLE_BYTES + 1).is_err());
    let mut bad = original.clone();
    bad.figures.get_mut(&3).unwrap().file = "../figure-0123456789abcdef.mfig".into();
    assert!(bad.validate(&cancel).is_err());
}
#[test]
fn matching_generation_is_required_and_missing_object_differs_from_missing_figure() {
    let cancel = AtomicBool::new(false);
    let (e, figures, manifest) = fixture();
    let original = table(&e, &figures, &manifest, &cancel);
    let mut binding = FigureBindings::recovered(&figures, &manifest, &cancel).unwrap();
    binding.expected.get_mut(&3).unwrap().crc32 ^= 1;
    assert!(original.restore(&binding, &cancel).is_err());
    let mut binding = FigureBindings::recovered(&figures, &manifest, &cancel).unwrap();
    binding.available.figures[0].figure.axes.series.clear();
    assert!(original.restore(&binding, &cancel).is_err());
    binding.detach_all();
    let (restored, missing) = original.restore(&binding, &cancel).unwrap();
    assert_eq!(missing, 1);
    assert!(restored[0].object.same_object(&restored[1].figure));
    assert!(
        restored[1]
            .object
            .same_object(restored[2].axes.as_ref().unwrap())
    );
    assert!(
        !restored[0]
            .object
            .same_object(&figures.figures[0].figure.identity)
    );
}
#[test]
fn active_graph_planning_and_identity_restore_cancel_before_exposing_results_and_retry() {
    let _reset = Reset;
    let cancel = AtomicBool::new(false);
    let (e, figures, manifest) = fixture();
    let original = table(&e, &figures, &manifest, &cancel);
    let binding = FigureBindings::recovered(&figures, &manifest, &cancel).unwrap();
    STOP_AFTER.with(|s| s.set(Some(3)));
    assert!(graph::plan(e.workspace(), Some((&figures, &manifest)), &cancel).is_err());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(STOP_AFTER.with(|s| s.get()).is_none());
    cancel.store(false, Ordering::Relaxed);
    let retry = graph::plan(e.workspace(), Some((&figures, &manifest)), &cancel).unwrap();
    assert!(retry.1.unwrap().graphics.is_some());
    STOP_AFTER.with(|s| s.set(Some(3)));
    assert!(original.restore(&binding, &cancel).is_err());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(STOP_AFTER.with(|s| s.get()).is_none());
    cancel.store(false, Ordering::Relaxed);
    let (restored, missing) = original.restore(&binding, &cancel).unwrap();
    assert_eq!(missing, 0);
    let Array::Graphics(h) = e.array("h").unwrap() else {
        panic!()
    };
    assert!(restored[2].same_object(h.scalar_handle().unwrap()));
}

#[test]
fn v4_final_commit_cancellation_preserves_both_old_payload_generations_and_retry() {
    use crate::{
        document::Document,
        session::{Session, SessionStore},
        simulation::Model,
    };
    fn state(value: f64) -> Session {
        let mut e = crate::Engine::default();
        e.execute(&format!(
            "f=figure(3);h=plot([1 2],[{value} 8]);A=ones(100,100);C={{h,f,A}};"
        ))
        .unwrap();
        Session {
            layout_presets: Default::default(),
            export: Default::default(),
            workspace: Some(e.workspace()),
            workspace_manifest: None,
            preview: None,
            eps_renderer: None,
            figure_manifest: None,
            figures: Some(FigureSession {
                current: 3,
                figures: e
                    .figures()
                    .map(|(id, f)| crate::session::figures::FigureDocument {
                        window: None,
                        id,
                        figure: f.clone(),
                        path: None,
                        source_version: None,
                        dirty: true,
                        view: None,
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
    fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                files.insert(entry.path(), vec![]);
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
    store.save_with_cancel(state(3.), &cancel).unwrap();
    let before = tree(dir.path());
    crate::session::STOP_BEFORE_COMMIT.with(|s| s.set(true));
    assert!(
        store
            .save_with_cancel(state(6.), &cancel)
            .unwrap_err()
            .contains("cancel")
    );
    assert!(cancel.load(Ordering::Relaxed));
    assert!(!crate::session::STOP_BEFORE_COMMIT.with(|s| s.get()));
    assert_eq!(tree(dir.path()), before);
    cancel.store(false, Ordering::Relaxed);
    store.save_with_cancel(state(6.), &cancel).unwrap();
    let saved = Session::load(&path).unwrap();
    let manifest = saved.workspace_manifest.unwrap();
    assert_eq!(manifest.version, 4);
    let report =
        crate::session::figures::restore_figures(&path, &saved.figure_manifest.unwrap(), &cancel);
    let got =
        super::super::restore_with_figures(&path, &manifest, report.workspace_bindings(), &cancel)
            .unwrap();
    assert!(got.warnings.is_empty());
    let Array::Graphics(h) = &got.values["h"] else {
        panic!()
    };
    assert!(
        h.scalar_handle()
            .unwrap()
            .object
            .same_object(&report.figures.figures[0].figure.axes.series[0].identity)
    );
    assert_eq!(
        report.figures.figures[0].figure.axes.series[0]
            .points
            .get(0)
            .unwrap()[1],
        6.
    );
}
#[test]
fn graphics_shapes_shared_payloads_and_empty_extents_survive_graph_reconstruction() {
    let cancel = AtomicBool::new(false);
    let (e, figures, manifest) = fixture();
    let Array::Graphics(h) = e.array("h").unwrap() else {
        panic!()
    };
    let row =
        crate::GraphicsArray::from_shared(1, 2, vec![h.get_linear(0).unwrap().clone(); 2], &cancel)
            .unwrap();
    let column = row.reshape_recovery(2, 1, &cancel).unwrap();
    let empty = crate::GraphicsArray::from_shared(0, 32768, vec![], &cancel).unwrap();
    let values = BTreeMap::from([
        ("row".into(), row.into()),
        ("column".into(), column.into()),
        ("empty".into(), empty.into()),
    ]);
    let (leaves, g) = graph::plan(values, Some((&figures, &manifest)), &cancel).unwrap();
    assert!(leaves.is_empty());
    let graph = g.unwrap();
    graph.validate(&BTreeMap::new(), &cancel).unwrap();
    let binding = FigureBindings::recovered(&figures, &manifest, &cancel).unwrap();
    let (handles, _) = graph
        .graphics
        .as_ref()
        .unwrap()
        .restore(&binding, &cancel)
        .unwrap();
    let got = graph.restore(leaves, &handles, &cancel).unwrap();
    let (Array::Graphics(row), Array::Graphics(column), Array::Graphics(empty)) =
        (&got["row"], &got["column"], &got["empty"])
    else {
        panic!()
    };
    assert!(row.same_storage(column));
    assert_eq!((row.rows(), row.cols()), (1, 2));
    assert_eq!((column.rows(), column.cols()), (2, 1));
    assert_eq!((empty.rows(), empty.cols(), empty.len()), (0, 32768, 0));
    assert!(
        row.get_linear(0)
            .unwrap()
            .same_object(row.get_linear(1).unwrap())
    );
}

#[test]
fn identity_index_rejects_ambiguous_addresses_and_cancels_without_sample_work() {
    let _reset = Reset;
    let cancel = AtomicBool::new(false);
    let (_, figures, _) = fixture();
    let samples = figures.figures[0].figure.series[0].points.clone();
    let got = index(&figures, &cancel).unwrap();
    assert_eq!(got.len(), 3);
    assert!((samples).same_storage(&figures.figures[0].figure.series[0].points));
    let mut duplicate = figures.clone();
    duplicate.figures.push(duplicate.figures[0].clone());
    assert!(index(&duplicate, &cancel).is_err());
    let mut oversized = figures.clone();
    oversized.figures[0].figure.series = vec![oversized.figures[0].figure.series[0].clone(); 65];
    assert!(index(&oversized, &cancel).is_err());
    STOP_AFTER.with(|s| s.set(Some(2)));
    assert!(index(&figures, &cancel).is_err());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(STOP_AFTER.with(|s| s.get()).is_none());
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(index(&figures, &cancel).unwrap().len(), got.len());
}

#[test]
fn public_checkpoint_still_rejects_invalid_patch_payload_before_publication() {
    use crate::{
        document::Document,
        session::{Session, SessionStore},
        simulation::Model,
    };
    let cancel = AtomicBool::new(false);
    let mut e = crate::Engine::default();
    e.execute("f=figure(3);a=gca;h=patch('Parent',a,'XData',[0;1;1;0],'YData',[0;0;1;1],'FaceColor',[1 0 0],'EdgeColor','none');").unwrap();
    let mut figures = FigureSession {
        current: 3,
        figures: e
            .figures()
            .map(|(id, f)| crate::session::figures::FigureDocument {
                window: None,
                id,
                figure: f.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            })
            .collect(),
    };
    // Keep the admitted topology, but corrupt the actual quadrilateral payload.
    figures.figures[0].figure.series[0].points =
        Arc::new(vec![[0., 0.], [1., 0.], [0.5, 1.], [0., 1.]]).into();
    assert!(figures.validate().is_err());
    let state = Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(e.workspace()),
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
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    assert!(
        SessionStore::new(&path)
            .save_with_cancel(state, &cancel)
            .is_err()
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
