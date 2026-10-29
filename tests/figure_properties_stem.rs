#![cfg(feature = "plot-export")]
use matrust::{
    files,
    plot::{AxisScale, Figure, PlotKind, Position, Series},
};
use resvg::usvg::roxmltree;
use std::sync::{Arc, atomic::AtomicBool};

fn stem(base: Option<f64>) -> Series {
    let mut s = Series::new(vec![[1., 2.], [2., 4.]], PlotKind::Stem);
    s.base_value = base;
    s
}
fn stored(figure: &Figure) -> (Vec<u8>, Figure) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.mfig");
    files::save_figure(&path, figure).unwrap();
    (
        std::fs::read(&path).unwrap(),
        files::load_figure(&path).unwrap(),
    )
}
fn baselines(figure: &Figure) -> usize {
    let svg = figure.svg();
    let doc = roxmltree::Document::parse(&svg).unwrap();
    doc.descendants()
        .filter(|n| n.has_tag_name("path") && n.attribute("stroke") == Some("#777777"))
        .count()
}

#[test]
fn defaults_preserve_legacy_metadata_and_new_properties_change_shared_snapshot() {
    let mut f = Figure {
        appearance: None,
        axes: matrust::plot::Axes::default(),
        ..Figure::default()
    };
    f.add(vec![stem(None)]).unwrap();
    let old = f.clone();
    let (bytes, _) = stored(&f);
    assert!(bytes.starts_with(b"MATRFIG5"));
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let meta: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    assert!(meta.get("name").is_none() && meta.get("color").is_none());
    assert!(meta["series"][0].get("base_value").is_none());
    f.name = "A real window name \u{03bb}".into();
    assert!(!f.same_snapshot(&old));
    let named = f.clone();
    f.set_canvas_color([80, 120, 160]);
    assert!(!f.same_snapshot(&named));
    let colored = f.clone();
    f.series[0].base_value = Some(-3.);
    assert!(!f.same_snapshot(&colored));
    assert!((f.series[0].points).same_storage(&old.series[0].points));
    let (bytes, g) = stored(&f);
    assert!(bytes.starts_with(b"MATRFIGJ"));
    assert_eq!(g.name, f.name);
    assert_eq!(g.color, f.color);
    assert_eq!(g.series[0].base_value, Some(-3.));
    assert_eq!(g.svg(), f.svg());
    assert_eq!(stored(&g).0, bytes);
}

#[test]
fn explicit_stem_baselines_control_geometry_limits_and_distinct_horizontal_lines() {
    let mut f = Figure::default();
    f.add(vec![stem(Some(-3.)), stem(Some(1.)), stem(Some(-3.))])
        .unwrap();
    assert_eq!(f.bounds(), ([1., 2.], [-3., 4.]));
    let points = f.series[0].display_points_axes(8192, f.axis_transforms());
    assert_eq!(&points[..2], &[[1., -3.], [1., 2.]]);
    assert_eq!(&points[3..5], &[[2., -3.], [2., 4.]]);
    assert!(points[2].iter().all(|p| p.is_nan()));
    assert_eq!(baselines(&f), 2);
    f.series[1].visible = false;
    assert_eq!(baselines(&f), 1);
    f.series[0].base_value = Some(-f64::MAX);
    assert!(f.validate().is_ok());
    let svg = f.svg();
    assert!(!svg.contains("NaN") && !svg.contains("inf"));
}

#[test]
fn log_stem_geometry_uses_actual_base_and_invalid_add_is_atomic() {
    let mut f = Figure {
        axes: matrust::plot::Axes {
            yscale: AxisScale::Log,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut s = stem(Some(10.));
    s.points = Arc::new(vec![[1., 100.], [2., 1000.]]).into();
    f.add(vec![s]).unwrap();
    assert_eq!(f.bounds().1, [10., 1000.]);
    assert_eq!(
        f.series[0].display_points_axes(8192, f.axis_transforms())[0],
        [1., 1.]
    );
    for value in [0., -1., f64::NAN, f64::INFINITY] {
        let old = f.clone();
        assert!(f.add(vec![stem(Some(value))]).is_err());
        assert!(f.same_snapshot(&old));
    }
    let mut negative = Figure {
        axes: matrust::plot::Axes {
            yscale: AxisScale::Log,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut s = stem(Some(-10.));
    s.points = Arc::new(vec![[1., -100.], [2., -1000.]]).into();
    negative.add(vec![s]).unwrap();
    assert_eq!(negative.bounds().1, [-1000., -10.]);
    assert_eq!(
        negative.series[0].display_points_axes(8192, negative.axis_transforms())[0],
        [1., -1.]
    );
    let old = negative.clone();
    assert!(negative.add(vec![stem(None)]).is_err());
    assert!(negative.same_snapshot(&old));
    let mut nonstem = stem(Some(1.));
    nonstem.kind = PlotKind::Line;
    assert!(Figure::default().add(vec![nonstem]).is_err());
}

#[test]
fn figure_background_is_distinct_from_axes_and_survives_sized_subplot_export() {
    let mut f = Figure {
        appearance: None,
        axes: matrust::plot::Axes::default(),
        ..Figure::default()
    };
    f.add(vec![stem(None)]).unwrap();
    f.name = "Not an axes title".into();
    f.title = "Actual axes title".into();
    f.set_canvas_color([18, 52, 86]);
    for panels in [false, true] {
        if panels {
            f.select_subplot(Position::from_grid(1, 2, &[1]).unwrap(), false)
                .unwrap();
            f.select_subplot(Position::from_grid(1, 2, &[2]).unwrap(), false)
                .unwrap();
            f.add(vec![stem(Some(-1.))]).unwrap();
        }
        let svg = f.svg();
        let doc = roxmltree::Document::parse(&svg).unwrap();
        assert!(
            doc.descendants()
                .any(|n| n.has_tag_name("rect") && n.attribute("fill") == Some("#123456"))
        );
        assert!(doc.descendants().any(|n| n.has_tag_name("rect")
            && n.attribute("fill") == Some("white")
            && n.attribute("x").is_some()));
        assert!(!svg.contains("Not an axes title"));
        let resized = f
            .svg_on_canvas(matrust::plot::Canvas {
                width: 640.,
                height: 480.,
            })
            .unwrap();
        assert!(resized.contains("width=\"640\" height=\"480\" fill=\"#123456\""));
        let (bytes, g) = stored(&f);
        assert_eq!(g.name, f.name);
        assert_eq!(g.color, f.color);
        assert_eq!(g.svg(), svg);
        if panels {
            assert!(bytes.starts_with(b"MATRFIGK"));
        }
    }
}

#[test]
fn advanced_properties_cannot_be_disguised_as_old_native_records() {
    let mut f = Figure {
        appearance: None,
        axes: matrust::plot::Axes::default(),
        ..Figure::default()
    };
    f.add(vec![stem(Some(-2.))]).unwrap();
    f.name = "named".into();
    f.set_canvas_color([1, 2, 3]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.mfig");
    for panels in [false, true] {
        if panels {
            f.select_subplot(Position::from_grid(1, 2, &[1]).unwrap(), false)
                .unwrap();
        }
        let (mut bytes, _) = stored(&f);
        bytes[..8].copy_from_slice(if panels { b"MATRFIGI" } else { b"MATRFIGH" });
        std::fs::write(&path, bytes).unwrap();
        assert!(files::load_figure(&path).is_err());
    }
    f.name = "x".repeat(4097);
    std::fs::write(&path, b"keep").unwrap();
    assert!(files::save_figure(&path, &f).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
}

#[test]
fn fig_and_script_preserve_public_name_color_without_admitting_unsupported_stems() {
    let mut f = Figure {
        name: "Caption's \u{03bb}".into(),
        ..Default::default()
    };
    f.set_canvas_color([18, 52, 86]);
    f.add(vec![Series::new(vec![[1., 2.], [2., 4.]], PlotKind::Line)])
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.fig");
    files::save_matlab_figure(&path, &f).unwrap();
    let g = files::load_matlab_figure(&path).unwrap();
    assert_eq!(g.name, f.name);
    assert_eq!(g.color, f.color);
    assert_eq!(g.series[0].points, f.series[0].points);
    let saved = std::fs::read(&path).unwrap();
    f.series[0].kind = PlotKind::Stem;
    f.series[0].base_value = Some(-7.);
    assert!(files::save_matlab_figure(&path, &f).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    let script = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("'Name',char([67 97 112 116 105 111 110 39 115 32 955])"));
    assert!(script.contains(&format!("'Color',[{:.17e}", 18f64 / 255.)));
    assert!(script.contains("'BaseValue',-7.00000000000000000e0"));
}

#[test]
fn pending_tiled_layout_is_not_activated_by_rejected_baseline() {
    let grid = matrust::plot::TileGrid::new(matrust::plot::Arrangement::Fixed, 1, 2).unwrap();
    let mut f = Figure::tiled_layout(grid).unwrap();
    f.yscale = AxisScale::Log;
    assert!(f.empty_layout());
    assert!(f.add(vec![stem(Some(0.))]).is_err());
    assert!(f.empty_layout());
}
