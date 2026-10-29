use matrust::{
    Engine, files,
    plot::{Figure, PlotKind, Series},
};
use std::sync::atomic::AtomicBool;
fn figure() -> Figure {
    let mut e = Engine::default();
    e.execute("plot([0 1 2],[1 1 1],'ro-');").unwrap();
    e.figure.unwrap()
}
#[test]
fn line_width_native_roundtrip_legacy_defaults_and_version_protection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("width.mfig");
    let mut f = figure();
    f.appearance = None; // Exercise the pre-theme native capability.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    files::save_figure(&path, &f).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"MATRFIG4"));
    assert_eq!(
        files::load_figure(&path).unwrap().series[0].line_width,
        None
    );
    let old = f.clone();
    f.series[0].line_width = Some(3.25);
    assert!(!old.same_snapshot(&f));
    assert!((old.series[0].points).same_storage(&f.series[0].points));
    files::save_figure(&path, &f).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIG9"));
    let reopened = files::load_figure(&path).unwrap();
    assert_eq!(reopened.series[0].line_width, Some(3.25));
    assert_eq!(reopened.series[0].points, f.series[0].points);
    bytes[..8].copy_from_slice(b"MATRFIG4");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIG9"));
}
#[test]
fn line_width_subplots_preserve_inactive_styles_and_reject_older_container_headers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("panels.mfig");
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([1 2]);subplot(1,2,2);stem([3 4]);")
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
    f.series[0].line_width = Some(2.);
    f.layout.as_mut().unwrap().others[0].axes.series[0].line_width = Some(4.);
    files::save_figure(&path, &f).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGA"));
    let reopened = files::load_figure(&path).unwrap();
    assert_eq!(reopened.series[0].line_width, Some(2.));
    assert_eq!(
        reopened.layout.as_ref().unwrap().others[0].axes.series[0].line_width,
        Some(4.)
    );
    bytes[..8].copy_from_slice(b"MATRFIG6");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGA"));
}
#[test]
fn line_width_fig_fixture_import_and_export_preserve_point_units() {
    let imported =
        files::load_figure(std::path::Path::new("tests/fixtures/fig/matlab-lines.fig")).unwrap();
    assert!(imported.series.iter().all(|s| s.line_width == Some(0.5)));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("width.fig");
    let mut f = figure();
    f.series[0].line_width = Some(3.25);
    files::save_figure(&path, &f).unwrap();
    assert_eq!(
        files::load_figure(&path).unwrap().series[0].line_width,
        Some(3.25)
    );
    f.series[0].line_width = None;
    let implicit_svg = f.svg();
    files::save_figure(&path, &f).unwrap();
    let reopened = files::load_figure(&path).unwrap();
    // Public FIG LineWidth remains 1.5pt (checked by the MAT hierarchy tests).
    // MatRust's validated metadata retains the implicit native/SVG defaults.
    assert_eq!(reopened.series[0].line_width, None);
    assert_eq!(reopened.svg(), implicit_svg);
    assert_eq!(reopened.series[0].stroke_width(1.7), 1.7);
    f.series[0].line_width = Some(1.5);
    files::save_figure(&path, &f).unwrap();
    let reopened = files::load_figure(&path).unwrap();
    assert_eq!(reopened.series[0].line_width, Some(1.5));
    assert_eq!(reopened.svg(), f.svg());
    assert_eq!(reopened.series[0].stroke_width(1.7), 2.);
}
#[test]
fn line_width_bounds_reject_invalid_styles_without_replacing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("width.mfig");
    let mut f = figure();
    for width in [0., -1., 0.09, 50.1, f64::NAN, f64::INFINITY] {
        f.series[0].line_width = Some(width);
        std::fs::write(&path, b"keep").unwrap();
        assert!(f.validate().is_err());
        assert!(files::save_figure(&path, &f).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
    for width in [0.1, 0.5, 50.] {
        f.series[0].line_width = Some(width);
        f.validate().unwrap();
    }
}
#[test]
fn line_width_svg_plot_and_matlab_use_requested_widths_with_bounded_legend() {
    let mut f = figure();
    f.legend = true;
    f.series[0].label = "Width".into();
    f.series[0].line_width = Some(3.);
    let svg = f.svg();
    // The plotted line and its three markers keep the requested four-pixel
    // width. Legend previews cap their stroke so they cannot cover labels.
    assert_eq!(svg.matches("stroke-width=\"4\"").count(), 4);
    assert_eq!(svg.matches("stroke-width=\"3\"").count(), 2);
    let script = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
    let encoded = script
        .lines()
        .find_map(|line| line.strip_prefix("set(mr_h,'LineWidth',"))
        .unwrap();
    assert_eq!(encoded.trim_end_matches(");").parse::<f64>().unwrap(), 3.);
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::Stairs,
        PlotKind::Stem,
    ] {
        let mut s = Series::new(vec![[0., 0.], [1., 1.]], kind);
        s.line_width = Some(2.);
        assert!((s.stroke_width(99.) - 8. / 3.).abs() < 1e-6);
    }
}
#[cfg(feature = "plot-export")]
#[test]
fn line_width_raster_ink_thickness_changes_with_width() {
    let mut f = figure();
    f.series[0].markers = false;
    f.xgrid = false;
    f.ygrid = false;
    f.xlim = Some([0., 2.]);
    f.ylim = Some([0., 2.]);
    let cancel = AtomicBool::new(false);
    let mut ink = Vec::new();
    for width in [0.75, 6.] {
        f.series[0].line_width = Some(width);
        let image = matrust::export::render_figure_with_cancel(&f, 960, 560, &cancel).unwrap();
        ink.push(
            (80..490)
                .filter(|&y| {
                    let p = image.get_pixel(480, y).0;
                    p[0] > 230 && p[1] < 220 && p[2] < 220
                })
                .count(),
        );
    }
    assert!(ink[0] >= 1 && ink[0] <= 2, "{ink:?}");
    assert!(ink[1] >= 7 && ink[1] <= 9, "{ink:?}");
}

#[test]
fn line_width_custom_position_container_roundtrip_and_nested_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("position.mfig");
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([1 2]);subplot(1,2,2);plot([3 4]);")
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
    f.reposition_axes([0.55, 0.15, 0.35, 0.7]).unwrap();
    f.series[0].line_width = Some(3.);
    files::save_figure(&path, &f).unwrap();
    let reopened = files::load_figure(&path).unwrap();
    assert_eq!(f.svg(), reopened.svg());
    assert_eq!(f.active_position(), reopened.active_position());
    let mut bytes = std::fs::read(&path).unwrap();
    let offset = 16 + u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    bytes[offset..offset + 8].copy_from_slice(b"MATRFIGA");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
}
