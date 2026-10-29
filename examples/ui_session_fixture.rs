//! Creates an isolated, inspectable recovery fixture for native UI verification.
use matrust::{document::Document, session::Session, simulation::Model};
fn main() -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let folder = root.join("target/evidence/ui");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let saved = Document::open(&root.join("examples/filter_design.m"))?;
    let mut scratch = Document::new();
    scratch.text = "% Recovered unsaved notes\ncutoff = 50;\n".into();
    scratch.changed();
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: None,
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![saved, scratch],
        active: 0,
        folder: root.join("examples"),
        recent: vec![
            root.join("examples/filter_design.m"),
            root.join("examples/signals.m"),
        ],
        history: vec![
            "b = fir1(64,0.1);".into(),
            "y = filter(b,1,x);".into(),
            "freqz(b,1,1024);".into(),
        ],
        model: Model::default(),
        model_path: None,
        model_baseline: Some(Model::default()),
    }
    .save(&folder.join("session.json"))?;
    println!("{}", folder.join("session.json").display());
    Ok(())
}
