use matrust::{
    Engine,
    document::Document,
    files::{
        self,
        figure_version::{self as tracked, SaveOutcome},
    },
    session::{
        Session,
        figures::{FigureDocument, FigureSession, SessionStore},
    },
};
use std::{path::PathBuf, sync::atomic::AtomicBool};
fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("fixture directory"));
    std::fs::create_dir_all(&directory).unwrap();
    let path = std::path::absolute(directory.join("shared_figure.mfig")).unwrap();
    let mut engine = Engine::default();
    engine.execute("plot(1:1000,sin((1:1000)./30));title('Saved baseline');xlabel('Sample');ylabel('Amplitude');").unwrap();
    let mut figure = engine.figure.unwrap();
    let SaveOutcome::Saved(v) =
        tracked::save(&path, &figure, None, &AtomicBool::new(false)).unwrap()
    else {
        panic!()
    };
    let mut session: Session =
        serde_json::from_slice(&std::fs::read("tests/fixtures/ui/session.json").unwrap()).unwrap();
    session.documents = vec![Document::new()];
    session.active = 0;
    session.history.clear();
    session.recent.clear();
    session.figure_manifest = None;
    figure.title = "Local unsaved figure".into();
    session.figures = Some(FigureSession {
        current: 1,
        figures: vec![FigureDocument {
            window: None,
            id: 1,
            figure: figure.clone(),
            path: Some(path.clone()),
            source_version: Some(v),
            dirty: true,
            view: None,
        }],
    });
    SessionStore::new(&directory.join("session.json"))
        .save(session)
        .unwrap();
    figure.title = "External disk figure".into();
    files::save_figure(&path, &figure).unwrap();
    println!("FIGURE_CONFLICT_FIXTURE_OK");
}
