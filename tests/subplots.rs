use matrust::{
    Engine, files,
    plot::{Figure, Position},
};
use std::sync::atomic::AtomicBool;

#[test]
fn subplot_selection_preserves_independent_axes_and_shared_samples() {
    let mut e = Engine::default();
    e.execute("subplot(2,2,1);plot([1 2],[3 4]);title('First');hold on;xlim([0 3]);")
        .unwrap();
    let points = e.figure.as_ref().unwrap().series[0].points.clone();
    e.execute("subplot(2,2,2);stem([2 5]);title('Second');yscale('log');subplot(2,2,[3 4]);bar([1 -1]);title('Spanned');").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 3);
    assert_eq!(e.figure.as_ref().unwrap().point_count(), 6);
    e.execute("subplot(2,2,1);").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.title, "First");
    assert!(f.hold);
    assert_eq!(f.xlim, Some([0., 3.]));
    assert!((points).same_storage(&f.series[0].points));
    e.execute("plot([1 2],[8 9]);subplot(2,1,2);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().title, "Spanned");
    assert_eq!(e.figure.as_ref().unwrap().series_count(), 4);
    e.execute("subplot(2,2,1,'replace');").unwrap();
    assert!(e.figure.as_ref().unwrap().series.is_empty());
    assert_eq!(e.figure.as_ref().unwrap().point_count(), 4);
    e.execute("subplot(1,1,1);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 1);
    assert!(e.figure.as_ref().unwrap().series.is_empty());
    e.execute("plot([3 2]);subplot(1,1,1);clf;").unwrap();
    assert!(e.figure.as_ref().unwrap().layout.is_none());
}

#[test]
fn subplot_inputs_and_global_limits_are_transactional() {
    let mut e = Engine::default();
    e.execute("plot([1 2]);subplot(1,1,1);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().point_count(), 2);
    let previous = e.figure.clone().unwrap();
    for bad in [
        "subplot(0,2,1)",
        "subplot(2,2,5)",
        "subplot(2,2,1.5)",
        "subplot(17,1,1)",
        "subplot(2,2,[])",
        "subplot(2,2,[1 2;3 4])",
        "subplot(2,2,1,'unknown')",
        "subplot('Position',[0 0 1 1],'unknown')",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
        assert!(e.figure.as_ref().unwrap().same_snapshot(&previous), "{bad}");
    }
    e.execute("clf;for k=1:16;subplot(16,16,k);end;").unwrap();
    let previous = e.figure.clone().unwrap();
    assert!(e.execute("subplot(16,16,17)").is_err());
    assert!(e.figure.as_ref().unwrap().same_snapshot(&previous));
    e.execute("clf;subplot(2,1,1);x=1:600000;plot(x);subplot(2,1,2);")
        .unwrap();
    let previous = e.figure.clone().unwrap();
    assert!(e.execute("plot(x)").is_err());
    assert!(e.figure.as_ref().unwrap().same_snapshot(&previous));
}

#[test]
fn dsp_plots_keep_other_axes_and_enforce_the_combined_sample_budget() {
    let mut e = Engine::default();
    e.execute("subplot(2,1,1);plot([1 2]);title('Neighbor');subplot(2,1,2);freqz(1,1,4);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 2);
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .all_axes()
            .any(|(_, a)| a.title == "Neighbor")
    );
    e.execute("clf;subplot(2,1,1);plot(1:999999);subplot(2,1,2);")
        .unwrap();
    let previous = e.figure.clone().unwrap();
    for source in ["freqz(1,1,4)", "impz(1,1,4)"] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(
            e.figure.as_ref().unwrap().same_snapshot(&previous),
            "{source}"
        );
    }
}

#[test]
fn subplot_native_csv_and_svg_preserve_all_axes_and_reject_nested_files() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/subplots.m")).unwrap();
    let mut f = e.figure.unwrap();
    f.appearance = None; // Historical codec fixture, independent of modern theme intent.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.mfig");
    files::save_figure(&path, &f).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(f.all_axes().any(|(_, axes)| axes.legend_state.is_some()));
    assert_eq!(&bytes[..8], b"MATRFIGK");
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.svg(), f.svg());
    assert_eq!(loaded.point_count(), 450);
    assert_eq!(loaded.active_position(), f.active_position());
    for (p, a) in f.all_axes() {
        let b = loaded.all_axes().find(|(q, _)| *q == p).unwrap().1;
        assert_eq!(a.title, b.title);
        assert!(a.same_metadata(b));
        for (s, t) in a.series.iter().zip(&b.series) {
            assert_eq!(s.points, t.points);
        }
    }
    let csv = dir.path().join("layout.csv");
    files::write_figure_csv(&csv, &f).unwrap();
    let text = std::fs::read_to_string(csv).unwrap();
    assert!(text.starts_with("axes,series,x,y\n"));
    assert_eq!(text.lines().count(), 451);
    let svg = f.svg();
    for i in 0..3 {
        assert_eq!(svg.matches(&format!("id=\"axes{i}\"")).count(), 1);
    }
    // Ordinary 2-by-2 panels retain figure-unit text size. The lower axes
    // spans both columns, with the same local-to-figure scale as its neighbors.
    let panels: Vec<_> = svg
        .split("<svg ")
        .skip(2)
        .map(|part| part.split_once('>').unwrap().0)
        .collect();
    assert_eq!(panels.len(), 3);
    for (panel, expected) in panels.iter().zip([
        [0., 0., 480., 280.],
        [480., 0., 480., 280.],
        [0., 280., 960., 280.],
    ]) {
        let attribute = |name: &str| {
            panel
                .split_once(&format!("{name}=\""))
                .unwrap()
                .1
                .split_once('"')
                .unwrap()
                .0
        };
        for (name, value) in ["x", "y", "width", "height"].into_iter().zip(expected) {
            assert_eq!(attribute(name).parse::<f64>().unwrap(), value);
        }
        let local: Vec<f64> = attribute("viewBox")
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        assert_eq!(local, [0., 0., expected[2], expected[3]]);
    }
    let bad = dir.path().join("no.fig");
    std::fs::write(&bad, b"keep").unwrap();
    assert!(files::save_figure(&bad, &f).is_err());
    assert_eq!(std::fs::read(bad).unwrap(), b"keep");
    let mut downgraded = bytes.clone();
    downgraded[..8].copy_from_slice(b"MATRFIG6");
    std::fs::write(&path, downgraded).unwrap();
    assert_eq!(
        files::load_figure(&path).unwrap_err(),
        "Advanced subplot metadata requires MFIGK"
    );
    let mut nested = bytes.clone();
    let size = u32::from_le_bytes(nested[8..12].try_into().unwrap()) as usize;
    nested[16 + size..24 + size].copy_from_slice(b"MATRFIG6");
    std::fs::write(&path, nested).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
    std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    assert!(files::load_figure(&path).is_err());
}

#[test]
fn recovery_tracks_inactive_axes_without_retaining_old_samples() {
    use matrust::session::{Session, SessionStore, figures::*};
    let mut e = Engine::default();
    e.execute(include_str!("../examples/subplots.m")).unwrap();
    let f = e.figure.take().unwrap();
    let mut state = Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: None,
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        version: 1,
        documents: vec![matrust::document::Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Default::default(),
        model_path: None,
        model_baseline: None,
        layout: Default::default(),
        figure_manifest: None,
        figures: Some(FigureSession {
            current: 1,
            figures: vec![FigureDocument {
                window: None,
                id: 1,
                figure: f,
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            }],
        }),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    store.save(state.clone()).unwrap();
    let first = Session::load(&path)
        .unwrap()
        .figure_manifest
        .unwrap()
        .entries[0]
        .file
        .clone();
    for (_, a) in state.figures.as_ref().unwrap().figures[0].figure.all_axes() {
        for s in &a.series {
            assert_eq!((s.points).strong_count(), 1);
        }
    }
    store.save(state.clone()).unwrap();
    assert_eq!(
        Session::load(&path)
            .unwrap()
            .figure_manifest
            .unwrap()
            .entries[0]
            .file,
        first
    );
    state.figures.as_mut().unwrap().figures[0]
        .figure
        .layout
        .as_mut()
        .unwrap()
        .others[0]
        .axes
        .title = "Changed inactive axes".into();
    store.save(state.clone()).unwrap();
    let manifest = Session::load(&path).unwrap().figure_manifest.unwrap();
    assert_ne!(manifest.entries[0].file, first);
    let restored = restore_figures(&path, &manifest, &AtomicBool::new(false));
    assert!(restored.warnings.is_empty());
    assert_eq!(
        restored.figures.figures[0].figure.svg(),
        state.figures.unwrap().figures[0].figure.svg()
    );
}

#[test]
fn equivalent_positions_and_overlap_rules_are_stable() {
    assert!(Position::from_grid(2, 2, &[1]).unwrap() < Position::from_grid(1, 2, &[2]).unwrap());

    assert_eq!(
        Position::from_grid(2, 2, &[3, 4]).unwrap(),
        Position::from_grid(2, 1, &[2]).unwrap()
    );
    let mut f = Figure::default();
    f.select_subplot(Position::from_grid(2, 2, &[1]).unwrap(), false)
        .unwrap();
    f.title = "Keep".into();
    f.select_subplot(Position::from_grid(2, 2, &[2]).unwrap(), false)
        .unwrap();
    f.select_subplot(Position::from_grid(2, 2, &[3, 4]).unwrap(), false)
        .unwrap();
    f.select_subplot(Position::from_grid(2, 1, &[1]).unwrap(), false)
        .unwrap();
    assert_eq!(f.all_axes().count(), 2);
    assert!(f.all_axes().all(|(_, a)| a.title != "Keep"));
}
