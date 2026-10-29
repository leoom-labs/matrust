use matrust::{
    document::Document,
    session::{PreviewFormat, PreviewState, Session},
    simulation::Model,
};
use std::{hint::black_box, time::Instant};
fn main() {
    let root = std::env::current_dir().unwrap();
    let folder = root.join("target/evidence/ui/preview-recovery");
    std::fs::create_dir_all(&folder).unwrap();
    let mut document = Document::new();
    document.text = "restored_marker = 42;".into();
    document.changed();
    let base = Session {
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
        documents: vec![document],
        active: 0,
        folder: root.clone(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    };
    let svg = folder.join("preview.svg");
    std::fs::write(&svg,r#"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="white"/><circle cx="160" cy="100" r="70" fill="steelblue"/></svg>"#).unwrap();
    for (name, path, page) in [
        (
            "tiff",
            root.join("tests/fixtures/preview/tiff-preview/pages.tiff"),
            1,
        ),
        (
            "pdf",
            root.join("tests/fixtures/preview/pdf-preview/pages.pdf"),
            1,
        ),
        (
            "png",
            root.join("tests/fixtures/preview/tiff-preview/reference-1.png"),
            0,
        ),
        ("svg", svg, 0),
        ("missing", folder.join("absent.pdf"), 0),
    ] {
        let mut session = base.clone();
        session.preview = Some(PreviewState {
            navigation: Default::default(),
            path,
            page,
            fit: false,
            zoom: 1.25,
            format: PreviewFormat::Tiff,
        });
        session
            .save(&folder.join(format!("{name}-session.json")))
            .unwrap();
    }
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,workers,repetitions,median_us,min_us,max_us,json_bytes");
    let mut preview = base.clone();
    preview.preview = Some(PreviewState {
        navigation: Default::default(),
        path: root.join("tests/fixtures/preview/tiff-preview/pages.tiff"),
        page: 1,
        fit: false,
        zoom: 1.25,
        format: PreviewFormat::Tiff,
    });
    let mut navigation = preview.clone();
    navigation.preview.as_mut().unwrap().navigation = matrust::session::PreviewNavigation {
        scroll: [80., 120.],
        window: Some(matrust::session::PreviewWindow {
            position: [700., 500.],
            size: [640., 480.],
        }),
    };
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for (name, state) in [
                ("without_preview", base),
                ("with_preview", preview),
                ("with_navigation", navigation),
            ] {
                let bytes = serde_json::to_vec(&state).unwrap();
                let mut times = Vec::new();
                for i in 0..34 {
                    let start = Instant::now();
                    for _ in 0..1000 {
                        let mut restored: Session =
                            serde_json::from_slice(black_box(&bytes)).unwrap();
                        restored.validate().unwrap();
                        drop(black_box(restored));
                    }
                    if i >= 3 {
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "{name},{workers},31,{:.6},{:.6},{:.6},{}",
                    times[15],
                    times[0],
                    times[30],
                    bytes.len()
                );
            }
        });
}
