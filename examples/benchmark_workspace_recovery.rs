use matrust::{
    Array, Engine, Matrix,
    document::Document,
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession},
        workspace,
    },
    simulation::Model,
};
use std::{
    collections::BTreeMap, hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant,
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
fn measure(mut f: impl FnMut()) -> (f64, f64, f64) {
    let mut samples = vec![];
    for i in 0..14 {
        let start = Instant::now();
        f();
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if i >= 3 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    (samples[5], samples[0], samples[10])
}
fn fixtures(root: &Path) {
    let folder = root.join("target/evidence/ui/workspace-recovery-inline");
    std::fs::create_dir_all(&folder).unwrap();
    let mut e = Engine::default();
    e.execute("gain=0.75;t=linspace(0,10,1000);signal=sin(t);alias=signal;selected=signal>0.5;plot(t,signal);title('Recovered workspace signal');xlabel('Time');ylabel('Amplitude');").unwrap();
    let mut s = state(e.workspace());
    s.documents[0].text =
        "% Variables and figure restored without rerunning this script\npeak = max(signal);\n"
            .into();
    s.documents[0].changed();
    s.figures = Some(FigureSession {
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
    });
    for name in ["wide", "compact"] {
        s.clone()
            .save(&folder.join(format!("{name}-session.json")))
            .unwrap();
    }
    let bad = folder.join("damaged-session.json");
    s.save(&bad).unwrap();
    let m = Session::load(&bad).unwrap().workspace_manifest.unwrap();
    std::fs::write(
        workspace::directory(&bad)
            .unwrap()
            .join(&m.entries["signal"].file),
        [0; 8],
    )
    .unwrap();
}
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    let root = std::env::current_dir().unwrap();
    println!("case,payload_bytes,variables,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for size in [0, 8 * 1024 * 1024, 64 * 1024 * 1024] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("session.json");
                let values = if size == 0 {
                    (0..100)
                        .map(|i| (format!("v{i}"), Matrix::scalar(i as f64).into()))
                        .collect()
                } else {
                    let a = Matrix::from_column_vec(
                        size / 8,
                        1,
                        (0..size / 8).map(|i| (i as f64 * 0.01).sin()).collect(),
                    )
                    .unwrap();
                    BTreeMap::from([
                        ("A".into(), a.clone().into()),
                        ("alias".into(), a.into()),
                        ("counter".into(), Matrix::scalar(0.).into()),
                    ])
                };
                let mut s = state(values);
                let bytes: usize = s
                    .workspace
                    .as_ref()
                    .unwrap()
                    .values()
                    .map(Array::bytes)
                    .sum();
                let count = s.workspace.as_ref().unwrap().len();
                let report = |name: &str, f: &mut dyn FnMut()| {
                    let (median, min, max) = measure(f);
                    println!("{name},{bytes},{count},{workers},11,{median:.6},{min:.6},{max:.6}");
                };
                report("fresh_writer_checkpoint", &mut || {
                    SessionStore::new(&path).save(black_box(s.clone())).unwrap();
                });
                let mut store = SessionStore::new(&path);
                store.save(s.clone()).unwrap();
                report("unchanged_checkpoint", &mut || {
                    store.save(black_box(s.clone())).unwrap();
                });
                let mut counter = 0.;
                report("scalar_edit_checkpoint", &mut || {
                    counter += 1.;
                    s.workspace.as_mut().unwrap().insert(
                        if size == 0 { "v0" } else { "counter" }.into(),
                        Matrix::scalar(counter).into(),
                    );
                    store.save(black_box(s.clone())).unwrap();
                });
                let m = Session::load(&path).unwrap().workspace_manifest.unwrap();
                let cancel = AtomicBool::new(false);
                report("restore", &mut || {
                    drop(black_box(workspace::restore(&path, &m, &cancel).unwrap()));
                });
                let restored = workspace::restore(&path, &m, &cancel).unwrap();
                for (name, a) in s.workspace.as_ref().unwrap() {
                    let b = &restored[name];
                    assert_eq!(
                        (a.rows(), a.cols(), a.is_logical()),
                        (b.rows(), b.cols(), b.is_logical())
                    );
                    for col in 0..a.cols() {
                        for row in 0..a.rows() {
                            assert_eq!(
                                a.scalar(row, col).unwrap().into_real().unwrap().to_bits(),
                                b.scalar(row, col).unwrap().into_real().unwrap().to_bits()
                            );
                        }
                    }
                }
                if size != 0 {
                    assert!(restored["A"].same_snapshot(&restored["alias"]));
                }
            }
        });
    fixtures(&root);
}
