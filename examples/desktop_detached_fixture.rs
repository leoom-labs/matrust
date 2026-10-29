//! Creates private, genuine typed recovery sessions for native detached native viewport captures.
//! This is a fixture generator, not an application startup mode.
use matrust::{
    Array, Engine,
    document::Document,
    session::{
        self, DesktopLayout, DesktopTheme, Session, SessionStore,
        figures::{FigureDocument, FigureSession},
    },
    simulation::Model,
};
use std::{collections::BTreeMap, path::PathBuf, sync::atomic::AtomicBool};

const SCRIPT: &str = "real_data=[1 2 3;4 5 6];real_alias=real_data;\nlogical_data=logical([0 1 0;1 0 1]);\ncomplex_data=complex([1 2;3 4],[4 3;2 1]);\nzero_imag=complex([1 2]);empty_complex=complex(zeros(0,3));\nchar_data='Engineering workspace';cell_data={real_data,'signal';logical_data,complex_data};\nfigure(1);plot([0 1 2 3],[0 1 -1 0]);title('Signal overview');xlabel('Time');ylabel('Amplitude');\nfigure(2);subplot(1,2,1);plot([0 1 2],[2 1 3]);title('Left axes');xlabel('Sample');ylabel('Value');subplot(1,2,2);stem([0 1 2],[1 3 2]);title('Right axes');legend('Samples','Location','best');\nfigure(1);\n";

fn exact(a: &Array, b: &Array) {
    assert_eq!(
        (a.rows(), a.cols(), a.class(), a.is_complex()),
        (b.rows(), b.cols(), b.class(), b.is_complex())
    );
    match (a, b) {
        (Array::Numeric(a), Array::Numeric(b)) => assert!(
            a.data()
                .iter()
                .zip(b.data())
                .all(|(a, b)| a.to_bits() == b.to_bits())
        ),
        (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
        (Array::Complex(a), Array::Complex(b)) => assert!(
            a.data()
                .iter()
                .zip(b.data())
                .all(|(a, b)| a.re.to_bits() == b.re.to_bits() && a.im.to_bits() == b.im.to_bits())
        ),
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                exact(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Unexpected fixture type or graphics handle"),
    }
}
fn verify(output: &std::path::Path) {
    let mut engine = Engine::default();
    engine.execute(SCRIPT).unwrap();
    let values = engine.workspace();
    let cancel = AtomicBool::new(false);
    let expected: BTreeMap<_, _> = engine.figures().map(|(id, f)| (id, f.clone())).collect();
    for theme in ["light", "dark"] {
        for size in ["wide", "compact"] {
            let path = output.join(format!("{theme}-{size}")).join("session.json");
            let loaded = Session::load(&path).unwrap();
            assert_eq!(loaded.documents.len(), 1);
            assert_eq!(loaded.documents[0].text, SCRIPT);
            assert!(!loaded.documents[0].dirty);
            let restored = session::workspace::restore(
                &path,
                loaded.workspace_manifest.as_ref().unwrap(),
                &cancel,
            )
            .unwrap();
            assert_eq!(restored.len(), values.len());
            for (name, value) in &values {
                exact(value, &restored[name]);
            }
            assert!(restored["real_data"].same_snapshot(&restored["real_alias"]));
            let figures = session::figures::restore_figures(
                &path,
                loaded.figure_manifest.as_ref().unwrap(),
                &cancel,
            );
            assert!(figures.warnings.is_empty());
            assert_eq!(figures.figures.figures.len(), 2);
            assert!([1, 2].contains(&figures.figures.current));
            for document in &figures.figures.figures {
                assert_eq!(document.figure.svg(), expected[&document.id].svg());
                assert!(document.dirty);
                assert!(document.path.is_none());
            }
        }
    }
    println!("Verified 4 detached capture checkpoints with public readers");
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--verify") {
        let mut args = std::env::args_os().skip(2);
        let output = PathBuf::from(args.next().expect("Supply fixture directory"));
        assert!(args.next().is_none());
        verify(&output);
        return;
    }
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(
        args.next()
            .expect("Supply a fresh private output directory"),
    );
    assert!(args.next().is_none());
    // Refuse to overwrite even an existing empty directory.
    std::fs::create_dir(&output).unwrap();
    let output = output.canonicalize().unwrap();
    let mut engine = Engine::default();
    engine.execute(SCRIPT).unwrap();
    let values = engine.workspace();
    assert_eq!(values.len(), 8);
    assert!(values["real_alias"].same_snapshot(&values["real_data"]));
    assert!(values["empty_complex"].is_complex());
    assert_eq!(
        (
            values["empty_complex"].rows(),
            values["empty_complex"].cols()
        ),
        (0, 3)
    );
    let source_figures: BTreeMap<_, _> = engine.figures().map(|(id, f)| (id, f.clone())).collect();
    assert_eq!(source_figures.len(), 2);
    assert_eq!(engine.current_figure, 1);
    let figures = FigureSession {
        current: 1,
        figures: source_figures
            .iter()
            .map(|(&id, figure)| FigureDocument {
                window: None,
                id,
                figure: figure.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            })
            .collect(),
    };
    figures.validate().unwrap();
    let cancel = AtomicBool::new(false);
    let mut cases = Vec::new();
    for (theme_name, theme) in [("Light", DesktopTheme::Light), ("Dark", DesktopTheme::Dark)] {
        for (size_name, width, height) in [("wide", 1440, 900), ("compact", 960, 650)] {
            let case = format!("{}-{size_name}", theme_name.to_ascii_lowercase());
            let dir = output.join(&case);
            std::fs::create_dir(&dir).unwrap();
            let script = dir.join("workspace_demo.m");
            std::fs::write(&script, SCRIPT).unwrap();
            let document = Document::open(&script).unwrap();
            let path = dir.join("session.json");
            let layout = DesktopLayout {
                theme: Some(theme),
                workspace_dock: Some(session::WorkspaceDock::BelowFiles),
                left_workspace_fraction: Some(0.5),
                sidebar_width: Some(230.),
                workspace_width: Some(if size_name == "compact" { 180. } else { 220. }),
                console_height: Some(if size_name == "compact" { 130. } else { 190. }),
                ..Default::default()
            };
            let state = Session {
                layout_presets: Default::default(),
                export: Default::default(),
                workspace: Some(values.clone()),
                workspace_manifest: None,
                preview: None,
                eps_renderer: None,
                figure_manifest: None,
                figures: Some(figures.clone()),
                layout,
                version: 1,
                documents: vec![document],
                active: 0,
                folder: dir.clone(),
                recent: vec![script.clone()],
                history: vec!["size(complex_data)".into()],
                model: Model::default(),
                model_path: None,
                model_baseline: None,
            };
            SessionStore::new(&path).save(state).unwrap();
            let loaded = Session::load(&path).unwrap();
            assert_eq!(loaded.layout, layout);
            assert_eq!(loaded.documents.len(), 1);
            assert_eq!(loaded.documents[0].text, SCRIPT);
            assert!(!loaded.documents[0].dirty);
            let restored = session::workspace::restore(
                &path,
                loaded.workspace_manifest.as_ref().unwrap(),
                &cancel,
            )
            .unwrap();
            assert_eq!(restored.len(), values.len());
            for (name, expected) in &values {
                exact(expected, &restored[name]);
            }
            assert!(restored["real_data"].same_snapshot(&restored["real_alias"]));
            let restored_figures = session::figures::restore_figures(
                &path,
                loaded.figure_manifest.as_ref().unwrap(),
                &cancel,
            );
            assert!(restored_figures.warnings.is_empty());
            assert_eq!(restored_figures.figures.current, 1);
            assert_eq!(restored_figures.figures.figures.len(), 2);
            for doc in &restored_figures.figures.figures {
                assert_eq!(doc.figure.svg(), source_figures[&doc.id].svg());
                assert!(doc.dirty);
                assert!(doc.path.is_none());
            }
            assert_eq!(std::fs::read_to_string(&script).unwrap(), SCRIPT);
            cases.push(serde_json::json!({"case":case,"session":path,"script":script,"theme":theme_name,"width":width,"height":height,
                    "variables":values.iter().map(|(name,a)|serde_json::json!({"name":name,"rows":a.rows(),"cols":a.cols(),"class":a.class(),"complex":a.is_complex(),"bytes":a.bytes()})).collect::<Vec<_>>(),"figure_ids":[1,2],"current_figure":1}));
        }
    }
    for (name, value) in &values {
        assert!(value.same_snapshot(&engine.workspace()[name]));
    }
    assert_eq!(cases.len(), 4);
    let manifest=serde_json::to_vec_pretty(&serde_json::json!({"format":1,"origin":"actual Engine and SessionStore; public recovery readers verified","cases":cases})).unwrap();
    assert!(manifest.len() <= 65536);
    std::fs::write(output.join("fixture-manifest.json"), manifest).unwrap();
    println!("Created and verified 4 private detached-figure sessions");
}
