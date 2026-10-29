use matrust::{Engine, files, plot::Position};
use std::path::Path;

#[test]
fn matlab_generated_subplot_fixture_retains_positions_and_samples() {
    let f = files::load_figure(Path::new("tests/fixtures/fig/matlab-subplots.fig")).unwrap();
    let mut axes: Vec<_> = f.all_axes().collect();
    axes.sort_by_key(|(p, _)| *p);
    assert_eq!(axes.len(), 2);
    assert_eq!(f.point_count(), 4);
    let expected = [
        (
            [0.13, 0.5838372093023255, 0.775, 0.3411627906976745],
            [1., 2.],
        ),
        ([0.13, 0.11, 0.775, 0.3411627906976744], [3., 4.]),
    ];
    for ((p, a), (position, y)) in axes.iter().zip(expected) {
        assert_eq!(p.normalized_plot_box().unwrap(), position);
        assert_eq!(
            a.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., y[0]], [2., y[1]]]
        );
    }
    let dir = tempfile::tempdir().unwrap();
    for ext in ["fig", "mfig"] {
        let path = dir.path().join(format!("layout.{ext}"));
        files::save_figure(&path, &f).unwrap();
        let restored = files::load_figure(&path).unwrap();
        assert_eq!(restored.svg(), f.svg());
        assert_eq!(restored.active_position(), f.active_position());
    }
}

#[test]
fn custom_layout_roundtrips_native_and_matlab_without_copying_selection_data() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/fig_layouts.m"))
        .unwrap();
    e.figure.as_mut().unwrap().appearance = None; // Historical native capability.
    {
        let historical = e.figure.as_mut().unwrap();
        historical.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
        for axes in historical.all_axes_mut() {
            axes.palette_cycle = None;
            for series in &mut axes.series {
                series.auto_palette = None;
            }
        }
    }
    let points = e.figure.as_ref().unwrap().series[0].points.clone();
    let original = e.figure.clone().unwrap();
    e.execute("subplot('Position',[0.1 0.62 0.35 0.3]);subplot('Position',[0.1 0.12 0.82 0.33]);")
        .unwrap();
    assert!((points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    let dir = tempfile::tempdir().unwrap();
    for ext in ["mfig", "fig"] {
        let path = dir.path().join(format!("plot.{ext}"));
        files::save_figure(&path, &original).unwrap();
        let loaded = files::load_figure(&path).unwrap();
        assert_eq!(loaded.svg(), original.svg());
        assert_eq!(loaded.active_position(), original.active_position());
        assert_eq!(loaded.point_count(), 600);
        if ext == "mfig" {
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(&bytes[..8], b"MATRFIG8");
            let mut bad = bytes.clone();
            bad[..8].copy_from_slice(b"MATRFIG7");
            std::fs::write(&path, bad).unwrap();
            assert!(files::load_figure(&path).is_err());
            let mut nested = bytes;
            let size = u32::from_le_bytes(nested[8..12].try_into().unwrap()) as usize;
            nested[16 + size..24 + size].copy_from_slice(b"MATRFIG8");
            std::fs::write(&path, nested).unwrap();
            assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
        }
    }
}

#[test]
fn custom_position_validation_overlap_and_ordering_are_bounded() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/fig_layouts.m"))
        .unwrap();
    let old = e.figure.clone().unwrap();
    for call in [
        "subplot('Position',[0 0 0 1])",
        "subplot('Position',[-0.1 0 0.5 1])",
        "subplot('Position',[0.5 0.5 0.6 0.4])",
        "subplot('Position',[0 0 0.001 1])",
        "subplot('Position',[0 0;1 1])",
        "subplot('Position',[0 0 1 NaN])",
    ] {
        assert!(e.execute(call).is_err(), "{call}");
        assert!(old.same_snapshot(e.figure.as_ref().unwrap()));
    }
    let positions = [
        Position::from_grid(2, 2, &[1]).unwrap(),
        Position::from_normalized([0.1, 0.12, 0.82, 0.33]).unwrap(),
        Position::from_normalized([0.57, 0.62, 0.35, 0.3]).unwrap(),
    ];
    for a in positions {
        for b in positions {
            assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
            assert_eq!(a == b, a.cmp(&b).is_eq());
        }
    }
    e.execute("subplot('Position',[0.05 0.55 0.9 0.4]);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 2); // Replaces both upper axes.
    assert_eq!(e.figure.as_ref().unwrap().point_count(), 200);
}

#[test]
fn grid_fig_export_uses_real_normalized_axes_and_preserves_active_selection() {
    let mut e = Engine::default();
    e.execute("tiledlayout(2,2);nexttile;plot([1 2]);nexttile;scatter([3 4]);nexttile([1 2]);plot([5 6]);nexttile(2);").unwrap();
    let f = e.figure.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grid.fig");
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.all_axes().count(), 3);
    assert_eq!(loaded.point_count(), 6);
    assert_eq!(loaded.series[0].points.get(0).unwrap()[1], 3.);
    let mut positions: Vec<_> = loaded.all_axes().map(|(p, _)| p).collect();
    positions.sort();
    let [left, top, right, bottom] = positions[0].rectangle();
    assert!((left - 35. / 960.).abs() < 1e-14);
    assert!((top - 30. / 560.).abs() < 1e-14);
    assert!((right - (0.5 - 20. / 960.)).abs() < 1e-14);
    assert!((bottom - (0.5 - 35. / 560.)).abs() < 1e-14);
    assert!(loaded.layout.as_ref().unwrap().tiled.is_none()); // Ordinary MATLAB axes, not a layout object.
    let mut titled = f;
    titled.set_shared_title("Keep this".into()).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(files::save_figure(&path, &titled).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
