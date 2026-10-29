use matrust::{
    Engine, files,
    plot::{MarkerShape, PlotKind},
};
use std::sync::atomic::AtomicBool;

#[test]
fn marker_styles_script_shapes_sizes_and_precedence_preserve_samples() {
    for shape in MarkerShape::ALL {
        let mut e = Engine::default();
        e.execute(&format!(
            "plot([1 2],[3 4],'r-','Marker','{}','MarkerSize',12,'LineWidth',1.5);",
            shape.symbol()
        ))
        .unwrap();
        let s = &e.figure.as_ref().unwrap().series[0];
        assert_eq!(s.marker_shape, Some(shape));
        assert_eq!(s.marker_size, Some(12.));
        assert!(s.markers);
        assert_eq!(
            s.points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 3.], [2., 4.]]
        );
        let expected = if shape == MarkerShape::Point {
            8. / 3.
        } else {
            8.
        };
        assert!((s.marker_radius() - expected).abs() < 1e-6);
        let script =
            files::matlab_plot_script(e.figure.as_ref().unwrap(), &AtomicBool::new(false)).unwrap();
        assert!(script.contains(&format!("'Marker','{}'", shape.symbol())));
        assert!(script.contains("set(mr_h,'MarkerSize',"));
    }
    let mut e = Engine::default();
    e.execute("plot([1 2],'rx','LineStyle','--','Marker','square','MarkerSize',9);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series[0].kind, PlotKind::Line);
    e.execute("plot([1 2],'ro-','Marker','none');").unwrap();
    assert!(!e.figure.as_ref().unwrap().series[0].markers);
    for spec in ["'filled'", "'rs','filled'"] {
        e.execute(&format!(
            "stem([1 2],[3 4],{spec},'Marker','square','MarkerSize',12);"
        ))
        .unwrap();
        let f = e.figure.as_ref().unwrap();
        assert!(
            f.series[0].marker_is_filled(),
            "named marker must retain stem filled option"
        );
        assert_eq!(f.series[0].marker_shape, Some(MarkerShape::Square));
        assert!(f.svg().contains("<path fill=\"#"));
        let script = files::matlab_plot_script(f, &AtomicBool::new(false)).unwrap();
        assert!(script.contains("'MarkerFaceColor',["));
    }
    e.execute("stem([1 2],[3 4],'filled','Marker','none');")
        .unwrap();
    assert!(!e.figure.as_ref().unwrap().series[0].markers);
    assert!(!e.figure.as_ref().unwrap().series[0].marker_filled);
    e.execute("scatter([1 2],[3 4],'Marker','diamond','SizeData',144,'LineWidth',1.5);")
        .unwrap();
    let s = &e.figure.as_ref().unwrap().series[0];
    assert_eq!(s.marker_shape, Some(MarkerShape::Diamond));
    assert_eq!(s.marker_size, Some(12.));
}

#[test]
fn marker_styles_line_specs_and_legacy_defaults_remain_distinct() {
    for shape in MarkerShape::ALL {
        let mut e = Engine::default();
        e.execute(&format!("plot([1 2],'r{}');", shape.symbol()))
            .unwrap();
        let s = &e.figure.as_ref().unwrap().series[0];
        assert_eq!(s.effective_marker(), shape);
        assert_eq!(s.kind, PlotKind::Scatter);
    }
    let mut e = Engine::default();
    e.execute("plot([1 2],'ro-', [1 2],[3 4],'k.');").unwrap();
    let f = e.figure.unwrap();
    assert_eq!(f.series[0].marker_radius(), 3.);
    assert_eq!(f.series[1].marker_radius(), 1.5);
    assert!(f.series.iter().all(|s| !s.has_marker_style()));
}

#[test]
fn marker_styles_native_fig_roundtrip_all_shapes_and_filled_polygons() {
    let dir = tempfile::tempdir().unwrap();
    for shape in MarkerShape::ALL {
        let mut e = Engine::default();
        e.execute(&format!(
            "plot([1 2],[3 4],'r-','Marker','{}','MarkerSize',12,'LineWidth',1.5);",
            shape.symbol()
        ))
        .unwrap();
        let mut f = e.figure.unwrap();
        f.series[0].marker_filled = true;
        for ext in ["mfig", "fig"] {
            let path = dir.path().join(format!("marker.{ext}"));
            files::save_figure(&path, &f).unwrap();
            let restored = files::load_figure(&path).unwrap();
            assert_eq!(restored.series[0].marker_shape, Some(shape));
            assert_eq!(restored.series[0].marker_size, Some(12.));
            assert_eq!(restored.svg(), f.svg(), "{} {ext}", shape.symbol());
        }
    }
}

#[test]
fn marker_styles_native_version_guards_layouts_and_shared_snapshots() {
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([1 2],'r+','MarkerSize',12);subplot(1,2,2);plot([3 4],'b^');")
        .unwrap();
    let mut f = e.figure.unwrap();
    f.appearance = None; // Exercise the pre-theme native capability.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    f.reposition_axes([0.55, 0.1, 0.35, 0.7]).unwrap();
    let before = f.clone();
    f.series[0].marker_size = Some(20.);
    assert!(!before.same_snapshot(&f));
    assert!((before.series[0].points).same_storage(&f.series[0].points));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("markers.mfig");
    files::save_figure(&path, &f).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGC"));
    let restored = files::load_figure(&path).unwrap();
    assert_eq!(restored.svg(), f.svg());
    assert_eq!(
        restored.layout.as_ref().unwrap().others[0].axes.series[0].marker_size,
        Some(12.)
    );
    let mut nested = bytes.clone();
    let offset = 16 + u32::from_le_bytes(nested[8..12].try_into().unwrap()) as usize;
    nested[offset..offset + 8].copy_from_slice(b"MATRFIGC");
    std::fs::write(&path, nested).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
    bytes[..8].copy_from_slice(b"MATRFIGA");
    std::fs::write(&path, &bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGC"));
    let mut single = matrust::plot::Figure {
        appearance: None,
        axes: matrust::plot::Axes::default(),
        ..matrust::plot::Figure::default()
    };
    single.add(f.series.clone()).unwrap();
    files::save_figure(&path, &single).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGB"));
    bytes[..8].copy_from_slice(b"MATRFIG9");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGB"));
}

#[test]
fn marker_styles_invalid_settings_preserve_figure_and_existing_file() {
    let mut e = Engine::default();
    e.execute("plot([1 2],'ro-');").unwrap();
    let before = e.figure.clone().unwrap();
    for call in [
        "plot([1 2],'MarkerSize',0)",
        "plot([1 2],'MarkerSize',101)",
        "plot([1 2],'MarkerSize',NaN)",
        "plot([1 2],'Marker','pentagram')",
        "plot([1 2],'rx','Marker','none')",
        "bar([1 2],'Marker','s')",
        "scatter([1 2],[3 4],'SizeData',[1 2])",
        "scatter([1 2],[3 4],'SizeData',-1)",
        "scatter([1 2],[3 4],'SizeData',0)",
        "scatter([1 2],[3 4],'Marker','none')",
    ] {
        assert!(e.execute(call).is_err(), "{call}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.mfig");
    std::fs::write(&path, b"keep").unwrap();
    let mut bad = before;
    bad.series[0].marker_size = Some(f64::INFINITY);
    assert!(files::save_figure(&path, &bad).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
}

#[cfg(feature = "plot-export")]
#[test]
fn marker_styles_raster_size_changes_marker_extent() {
    let mut spans = Vec::new();
    for area in [9, 144] {
        let mut e = Engine::default();
        e.execute(&format!("scatter([1],[1],'Marker','o','SizeData',{area},'MarkerEdgeColor','r','LineWidth',0.75);axis([0 2 0 2]);grid off;")).unwrap();
        let image = matrust::export::render_figure_with_cancel(
            e.figure.as_ref().unwrap(),
            960,
            560,
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut xs = Vec::new();
        for (x, _, pixel) in image.enumerate_pixels() {
            if pixel[0] > 230 && pixel[1] < 200 && pixel[2] < 200 {
                xs.push(x);
            }
        }
        assert!(!xs.is_empty());
        spans.push(xs.iter().max().unwrap() - xs.iter().min().unwrap() + 1);
    }
    assert!(
        (4..=6).contains(&spans[0]) && (16..=18).contains(&spans[1]),
        "{spans:?}"
    );
}
