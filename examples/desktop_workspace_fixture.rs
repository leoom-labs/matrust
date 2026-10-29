//! Creates private, genuine typed recovery sessions for native desktop captures.
//! This is a fixture generator, not an application startup mode.
use matrust::{
    Array, Engine,
    document::Document,
    session::{
        self, DesktopLayout, DesktopTheme, Session, SessionStore, WorkspaceDock,
        figures::{FigureDocument, FigureSession},
    },
    simulation::Model,
};
use std::{collections::BTreeMap, path::PathBuf, sync::atomic::AtomicBool};

const SCRIPT: &str = "real_data=[1 2 3;4 5 6];real_alias=real_data;\nlogical_data=logical([0 1 0;1 0 1]);\ncomplex_data=complex([1 2;3 4],[4 3;2 1]);\nzero_imag=complex([1 2]);empty_complex=complex(zeros(0,3));\nchar_data='Engineering workspace';cell_data={real_data,'signal';logical_data,complex_data};\nfigure(1);plot([0 1 2 3],[0 1 -1 0]);title('Signal overview');xlabel('Time');ylabel('Amplitude');\nfigure(2);plot([0 1 2],[2 1 3]);title('Second figure');xlabel('Sample');ylabel('Value');\nfigure(1);\n";

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
fn main() {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(
        args.next()
            .expect("Supply a fresh private output directory"),
    );
    let tiling = match args.next() {
        None => false,
        Some(flag) if flag == "--tiling" => true,
        Some(flag) if flag == "--axes-colors" => {
            assert!(args.next().is_none());
            tiling_sessions_with_style(output, true);
            return;
        }
        Some(_) => panic!("Only optional --tiling or --axes-colors is supported"),
    };
    assert!(args.next().is_none());
    if tiling {
        tiling_sessions(output);
        return;
    }
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
    for (dock_name, dock) in [
        ("right", None),
        ("below-files", Some(WorkspaceDock::BelowFiles)),
    ] {
        for (theme_name, theme) in [("Light", DesktopTheme::Light), ("Dark", DesktopTheme::Dark)] {
            for (size_name, width, height) in [("wide", 1440, 900), ("compact", 960, 650)] {
                let case = format!(
                    "{dock_name}-{}-{size_name}",
                    theme_name.to_ascii_lowercase()
                );
                let dir = output.join(&case);
                std::fs::create_dir(&dir).unwrap();
                let script = dir.join("workspace_demo.m");
                std::fs::write(&script, SCRIPT).unwrap();
                let document = Document::open(&script).unwrap();
                let path = dir.join("session.json");
                let layout = DesktopLayout {
                    theme: Some(theme),
                    workspace_dock: dock,
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
                cases.push(serde_json::json!({"case":case,"session":path,"script":script,"dock":dock_name,"theme":theme_name,"width":width,"height":height,
                    "variables":values.iter().map(|(name,a)|serde_json::json!({"name":name,"rows":a.rows(),"cols":a.cols(),"class":a.class(),"complex":a.is_complex(),"bytes":a.bytes()})).collect::<Vec<_>>(),"figure_ids":[1,2],"current_figure":1}));
            }
        }
    }
    for (name, value) in &values {
        assert!(value.same_snapshot(&engine.workspace()[name]));
    }
    assert_eq!(cases.len(), 8);
    let manifest=serde_json::to_vec_pretty(&serde_json::json!({"format":1,"origin":"actual Engine and SessionStore; public recovery readers verified","cases":cases})).unwrap();
    assert!(manifest.len() <= 65536);
    std::fs::write(output.join("fixture-manifest.json"), manifest).unwrap();
    println!("Created and verified 8 private typed Workspace sessions");
}

// This optional fixture uses public recovery types, never edited session JSON.
// The ordinary eight-case path above remains unchanged.
fn tiling_sessions(output: PathBuf) {
    tiling_sessions_with_style(output, false);
}
fn tiling_sessions_with_style(output: PathBuf, axes_colors: bool) {
    use matrust::session::{CenterLayout, FigureTiling, PaneMode};
    const TILES: &str = "figure(1);plot([-2 -1 0 1 2],[4 1 0 1 4]);title('1 - Quadratic line');xlabel('Position');ylabel('Square');grid on;\nfigure(2);scatter([0 1 2 3],[3 1 4 2]);title('2 - Sample scatter');xlabel('Sample');ylabel('Value');grid on;\nfigure(3);stem([0 1 2 3],[1 -2 3 -1]);title('3 - Signed impulses');xlabel('Index');ylabel('Amplitude');grid on;\nfigure(4);stairs([0 1 2 3],[2 2 -1 1]);title('4 - Step sequence');xlabel('Time');ylabel('Level');grid on;\nfigure(1);\n";
    // Refuse even an existing empty directory, matching the ordinary fixture.
    std::fs::create_dir(&output).unwrap();
    let output = output.canonicalize().unwrap();
    let mut script_text = format!("{SCRIPT}{TILES}");
    if axes_colors {
        script_text.push_str(AXES_COLORS);
    }
    let mut engine = Engine::default();
    engine.execute(&script_text).unwrap();
    let values = engine.workspace();
    assert_eq!(values.len(), 8);
    assert!(values["real_alias"].same_snapshot(&values["real_data"]));
    let source_figures: BTreeMap<_, _> = engine.figures().map(|(id, f)| (id, f.clone())).collect();
    assert_eq!(
        source_figures.keys().copied().collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    if axes_colors {
        assert_eq!(source_figures[&2].all_axes().count(), 2);
        assert_eq!(
            source_figures
                .values()
                .map(|f| f.all_axes().count())
                .sum::<usize>(),
            5
        );
    }
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
    for (mode_name, mode, visible) in [
        ("two", FigureTiling::Two, 2),
        ("four", FigureTiling::Four, 4),
    ] {
        for (theme_name, theme) in [("Light", DesktopTheme::Light), ("Dark", DesktopTheme::Dark)] {
            for (size_name, width, height) in [("wide", 1440, 900), ("compact", 960, 650)] {
                let case = format!(
                    "tiles-{mode_name}-{}-{size_name}",
                    theme_name.to_ascii_lowercase()
                );
                let dir = output.join(&case);
                std::fs::create_dir(&dir).unwrap();
                let script = dir.join("tiling_demo.m");
                std::fs::write(&script, &script_text).unwrap();
                let document = Document::open(&script).unwrap();
                let path = dir.join("session.json");
                let layout = DesktopLayout {
                    center: Some(CenterLayout {
                        command: false,
                        editor_revealed: true,
                    }),
                    mode: PaneMode::Figure,
                    figure_tiling: Some(mode),
                    theme: Some(theme),
                    workspace_dock: Some(WorkspaceDock::BelowFiles),
                    left_workspace_fraction: Some(0.5),
                    sidebar_width: Some(170.),
                    console_height: Some(130.),
                    ..Default::default()
                };
                SessionStore::new(&path)
                    .save(Session {
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
                        history: vec!["figure(1)".into()],
                        model: Model::default(),
                        model_path: None,
                        model_baseline: None,
                    })
                    .unwrap();
                let loaded = Session::load(&path).unwrap();
                assert_eq!(loaded.layout, layout);
                assert_eq!(loaded.documents.len(), 1);
                assert_eq!(loaded.documents[0].text, script_text);
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
                assert_eq!(restored_figures.figures.figures.len(), 4);
                for doc in &restored_figures.figures.figures {
                    let source = &source_figures[&doc.id];
                    assert_eq!(doc.figure.svg(), source.svg());
                    if axes_colors {
                        verify_axes_colors(source, &doc.figure);
                    }
                    assert_eq!(doc.figure.series.len(), source.series.len());
                    for (actual, expected) in doc.figure.series.iter().zip(&source.series) {
                        assert_eq!(actual.points.len(), expected.points.len());
                        for (actual, expected) in actual.points.iter().zip(expected.points.iter()) {
                            assert_eq!(
                                (actual[0].to_bits(), actual[1].to_bits()),
                                (expected[0].to_bits(), expected[1].to_bits())
                            );
                        }
                    }
                    assert!(doc.dirty && doc.path.is_none() && doc.window.is_none());
                }
                assert_eq!(std::fs::read_to_string(&script).unwrap(), script_text);
                cases.push(serde_json::json!({"case":case,"session":path,"script":script,
                    "theme":theme_name,"width":width,"height":height,"tiling":mode_name,
                    "figure_ids":[1,2,3,4],"expected_tile_slots":visible,"current_figure":1,
                    "expected_titles":["1 - Quadratic line","2 - Sample scatter","3 - Signed impulses","4 - Step sequence"]}));
                if axes_colors {
                    let case = cases.last_mut().unwrap().as_object_mut().unwrap();
                    case.insert("authored_axes_colors".into(), serde_json::json!(true));
                    case.insert(
                        "expected_titles".into(),
                        serde_json::json!(
                            source_figures
                                .values()
                                .flat_map(|figure| figure
                                    .all_axes()
                                    .map(|(_, axes)| axes.title.clone()))
                                .collect::<Vec<_>>()
                        ),
                    );
                    case.insert("expected_axes".into(), serde_json::json!(source_figures.iter()
                        .flat_map(|(&id, figure)| figure.all_axes().enumerate().map(move |(index, (_, axes))|
                            serde_json::json!({"figure":id,"axes":index,"title":axes.title,
                                "xlabel":axes.xlabel,"ylabel":axes.ylabel,"colors":axes.colors,
                                "xgrid":axes.xgrid,"ygrid":axes.ygrid,"samples":axes.series.iter().map(|s|s.points.len()).collect::<Vec<_>>()})))
                        .collect::<Vec<_>>()));
                }
            }
        }
    }
    for (name, value) in &values {
        assert!(value.same_snapshot(&engine.workspace()[name]));
    }
    for (id, figure) in engine.figures() {
        assert!(source_figures[&id].same_snapshot(figure));
        if axes_colors {
            for ((_, expected), (_, actual)) in
                source_figures[&id].all_axes().zip(figure.all_axes())
            {
                assert_eq!(expected.series.len(), actual.series.len());
                for (expected, actual) in expected.series.iter().zip(&actual.series) {
                    assert!(expected.points.same_storage(&actual.points));
                }
            }
        }
    }
    assert_eq!(cases.len(), 8);
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({"format":1,
        "origin":if axes_colors {"authored public Engine commands and SessionStore; exact recovery checks; not a MATLAB-produced fixture"} else {"actual Engine and SessionStore; public recovery readers verified"},
        "mode":if axes_colors {"axes-colors"} else {"tiling"},
        "cases":cases}))
    .unwrap();
    assert!(manifest.len() <= 65536);
    std::fs::write(output.join("fixture-manifest.json"), manifest).unwrap();
    if axes_colors {
        println!(
            "Created and verified 8 authored axes-color recovery sessions; native appearance unreviewed"
        );
    } else {
        println!("Created and verified 8 private typed tiling sessions");
    }
}

// Authored public commands, not a MATLAB-produced scene. No temporary graphics
// variables are introduced, so the original eight typed workspace values remain.
const AXES_COLORS: &str = "figure(1);title('1 - Directional grid','Color',[0.65 0.25 0.8]);xlabel('X red','Color',[0.1 0.65 0.35]);ylabel('Y blue','Color',[0.9 0.45 0.1]);set(gca,'XColor',[0.85 0.15 0.1],'YColor',[0.1 0.4 0.9],'GridColor',[0.55 0.2 0.65],'GridAlpha',0.35,'XColorMode','manual','YColorMode','auto','GridColorMode','auto','GridAlphaMode','manual');\nfigure(2);clf;subplot(1,2,1);plot([-1 0 1],[1 0 1]);title('2a - X grid','Color',[0.65 0.25 0.8]);xlabel('X red');ylabel('Y blue');grid on;set(gca,'XColor',[0.85 0.15 0.1],'YColor',[0.1 0.4 0.9],'GridColor',[0.4 0.65 0.2],'GridAlpha',0.15,'XColorMode','manual','YColorMode','auto','GridColorMode','auto','GridAlphaMode','auto');\nsubplot(1,2,2);plot([-1 0 1],[-1 0 1]);title('2b - Y grid','Color',[0.65 0.25 0.8]);xlabel('X green','Color',[0.1 0.65 0.35]);ylabel('Y orange','Color',[0.9 0.45 0.1]);grid on;set(gca,'XColor',[0.85 0.15 0.1],'YColor',[0.1 0.4 0.9],'GridColor',[0.55 0.2 0.65],'GridAlpha',0.5,'XColorMode','auto','YColorMode','manual','GridColorMode','auto','GridAlphaMode','manual');\nfigure(3);title('3 - Manual grid','Color',[0.65 0.25 0.8]);xlabel('Index green','Color',[0.1 0.65 0.35]);ylabel('Level orange','Color',[0.9 0.45 0.1]);set(gca,'XColor',[0.85 0.15 0.1],'YColor',[0.1 0.4 0.9],'GridColor',[0.15 0.65 0.45],'GridAlpha',0.6,'XColorMode','manual','YColorMode','manual','GridColorMode','manual','GridAlphaMode','manual');\nfigure(4);title('4 - Auto alpha','Color',[0.65 0.25 0.8]);xlabel('Time red');ylabel('Level blue');set(gca,'XColor',[0.85 0.15 0.1],'YColor',[0.1 0.4 0.9],'GridColor',[0.55 0.2 0.65],'GridAlpha',0.15,'XColorMode','auto','YColorMode','auto','GridColorMode','manual','GridAlphaMode','auto');\nfigure(1);\n";

fn verify_axes_colors(source: &matrust::plot::Figure, restored: &matrust::plot::Figure) {
    use matrust::plot::{BaseTheme, ColorMode};
    assert_eq!(source.appearance, restored.appearance);
    assert_eq!(source.color, restored.color);
    assert_eq!(source.all_axes().count(), restored.all_axes().count());
    for ((position, expected), (actual_position, actual)) in
        source.all_axes().zip(restored.all_axes())
    {
        assert_eq!(position, actual_position);
        let colors = expected.colors.expect("authored complete axes colors");
        assert_eq!(actual.colors, Some(colors));
        assert!(colors.x.is_some() && colors.y.is_some() && colors.title.is_some());
        let grid = colors.grid.expect("authored complete grid record");
        assert_eq!(colors.x, Some([0.85, 0.15, 0.1]));
        assert_eq!(colors.y, Some([0.1, 0.4, 0.9]));
        assert_eq!(colors.title, Some([0.65, 0.25, 0.8]));
        use ColorMode::{Auto, Manual};
        let (rgb, alpha, modes, explicit_labels) = match expected.title.as_str() {
            "1 - Directional grid" => ([0.55, 0.2, 0.65], 0.35, [Manual, Auto, Auto, Manual], true),
            "2a - X grid" => ([0.4, 0.65, 0.2], 0.15, [Manual, Auto, Auto, Auto], false),
            "2b - Y grid" => ([0.55, 0.2, 0.65], 0.5, [Auto, Manual, Auto, Manual], true),
            "3 - Manual grid" => (
                [0.15, 0.65, 0.45],
                0.6,
                [Manual, Manual, Manual, Manual],
                true,
            ),
            "4 - Auto alpha" => ([0.55, 0.2, 0.65], 0.15, [Auto, Auto, Manual, Auto], false),
            _ => panic!("Unexpected authored axes title"),
        };
        assert_eq!(grid.color, rgb);
        assert_eq!(grid.alpha, alpha);
        assert_eq!(
            [grid.x_mode, grid.y_mode, grid.color_mode, grid.alpha_mode],
            modes
        );
        assert_eq!(colors.xlabel, explicit_labels.then_some([0.1, 0.65, 0.35]));
        assert_eq!(colors.ylabel, explicit_labels.then_some([0.9, 0.45, 0.1]));
        assert!(grid.alpha > 0. && grid.alpha <= 1.);
        if grid.alpha_mode == ColorMode::Auto {
            assert_eq!(grid.alpha, 0.15);
        }
        assert!(actual.xgrid && actual.ygrid);
        assert_eq!(actual.series.len(), expected.series.len());
        for (actual, expected) in actual.series.iter().zip(&expected.series) {
            assert_eq!(actual.points.len(), expected.points.len());
            for (actual, expected) in actual.points.iter().zip(expected.points.iter()) {
                assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
        for host in [BaseTheme::Light, BaseTheme::Dark] {
            let a = actual.resolve_colors(restored.resolve_appearance(host));
            let e = expected.resolve_colors(source.resolve_appearance(host));
            assert_eq!(
                (
                    a.x,
                    a.y,
                    a.xlabel,
                    a.ylabel,
                    a.grid_x,
                    a.grid_y,
                    a.grid_alpha
                ),
                (
                    e.x,
                    e.y,
                    e.xlabel,
                    e.ylabel,
                    e.grid_x,
                    e.grid_y,
                    e.grid_alpha
                )
            );
        }
    }
    // Public same_snapshot includes the crate-private whole-Figure metadata check.
    // Recovery owns different storage: first verify every bit below, then compare
    // a small metadata clone using shared source handles (no sample payload copy).
    let mut metadata = restored.clone();
    assert_eq!(source.all_axes().count(), metadata.all_axes().count());
    for ((_, expected), actual) in source.all_axes().zip(metadata.all_axes_mut()) {
        assert_eq!(expected.series.len(), actual.series.len());
        for (expected, actual) in expected.series.iter().zip(&mut actual.series) {
            actual.points = expected.points.clone();
        }
    }
    assert!(source.same_snapshot(&metadata));
    for host in [BaseTheme::Light, BaseTheme::Dark] {
        assert_eq!(source.svg_with_theme(host), restored.svg_with_theme(host));
    }
}
