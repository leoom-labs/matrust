use super::*;
use crate::plot::{AxesColors, AxesGridStyle, BaseTheme, ColorMode, FigureTheme, SnapshotStamp};
use std::sync::atomic::AtomicBool;

fn source() -> Figure {
    let mut engine = crate::Engine::default();
    engine
        .execute("plot([-0 1],[2 3]);xlabel('Time');ylabel('Value');grid on;")
        .unwrap();
    engine.figure.unwrap()
}
fn assert_samples(expected: &Figure, actual: &Figure) {
    assert_eq!(expected.series.len(), actual.series.len());
    for (a, b) in expected.series.iter().zip(&actual.series) {
        assert_eq!(a.points.len(), b.points.len());
        for (a, b) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
}
fn effective(figure: &Figure) -> ([[u8; 3]; 5], f64) {
    let c = figure
        .axes
        .resolve_colors(figure.resolve_appearance(BaseTheme::Light));
    ([c.x, c.y, c.xlabel, c.ylabel, c.grid_x], c.grid_alpha)
}
fn axis_properties<'a>(hierarchy: &'a Value<'a>) -> &'a Fields<'a> {
    let root = object(hierarchy).unwrap();
    let axes = children(root)
        .unwrap()
        .iter()
        .find(|node| string(node, "type", "").unwrap() == "axes")
        .unwrap();
    object(field(axes, "properties").unwrap()).unwrap()
}

#[test]
fn public_modern_grid_on_and_off_save_open_and_script_replay_preserve_static_light() {
    let directory = tempfile::tempdir().unwrap();
    for enabled in [false, true] {
        let mut engine = crate::Engine::default();
        engine.set_file_directory(directory.path()).unwrap();
        engine
            .execute("plot([-0 1],[2 3]);xlabel('Time');ylabel('Value');")
            .unwrap();
        engine
            .execute(if enabled { "grid on;" } else { "grid off;" })
            .unwrap();
        let source = engine.figure.as_ref().unwrap().clone();
        let stamp = SnapshotStamp::new(&source);
        for round in 0..2 {
            engine.execute("savefig('modern.fig');").unwrap();
            let bytes = std::fs::read(directory.path().join("modern.fig")).unwrap();
            let expanded = mat::expand(&bytes).unwrap();
            let hierarchy = expanded.hierarchy().unwrap();
            let props = axis_properties(&hierarchy);
            assert!(!props.contains_key("XColor") && !props.contains_key("YColor"));
            let before = engine.figure.as_ref().unwrap();
            assert_samples(&source, before);
            if round == 0 {
                assert!(stamp.matches(before));
                assert!(
                    source.series[0]
                        .points
                        .same_storage(&before.series[0].points)
                );
            }
            engine.execute("close all;openfig('modern.fig');").unwrap();
            let reopened = engine.figure.as_ref().unwrap();
            assert_eq!((reopened.xgrid, reopened.ygrid), (enabled, enabled));
            assert_eq!(effective(&source), effective(reopened));
            assert_eq!(
                reopened.color,
                source.resolve_appearance(BaseTheme::Light).canvas
            );
            let grid = reopened.colors.unwrap().grid.unwrap();
            assert_eq!(
                (
                    grid.alpha,
                    grid.x_mode,
                    grid.y_mode,
                    grid.color_mode,
                    grid.alpha_mode
                ),
                (
                    0.15,
                    ColorMode::Auto,
                    ColorMode::Auto,
                    ColorMode::Auto,
                    ColorMode::Auto
                )
            );
            assert_samples(&source, reopened);
            let script =
                crate::files::matlab_plot_script(reopened, &AtomicBool::new(false)).unwrap();
            let mut replay = crate::Engine::default();
            replay.execute(&script).unwrap();
            let replayed = replay.figure.as_ref().unwrap();
            assert_eq!(replayed.colors, reopened.colors);
            assert_eq!(effective(replayed), effective(reopened));
            assert_samples(reopened, replayed);
        }
        assert!(stamp.matches(&source));
        assert!(
            source.series[0]
                .points
                .same_storage(&source.clone().series[0].points)
        );
    }
}

#[test]
fn complete_grid_values_and_direction_modes_are_exact_and_serialized_modes_last() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("explicit.fig");
    for x in [ColorMode::Auto, ColorMode::Manual] {
        for y in [ColorMode::Auto, ColorMode::Manual] {
            for mode in [ColorMode::Auto, ColorMode::Manual] {
                for (alpha_mode, alpha) in [
                    (ColorMode::Auto, 0.15),
                    (ColorMode::Manual, 0.),
                    (ColorMode::Manual, 0.4),
                    (ColorMode::Manual, 1.),
                ] {
                    let mut figure = source();
                    let grid = AxesGridStyle {
                        color: [-0., 0.2, 0.4],
                        alpha,
                        x_mode: x,
                        y_mode: y,
                        color_mode: mode,
                        alpha_mode,
                    };
                    figure.colors = Some(AxesColors {
                        grid: Some(grid),
                        x: Some([-0., 0.15, 1.]),
                        y: Some([0.25, 0.5, 0.75]),
                        xlabel: Some([1., 0., 0.]),
                        ylabel: Some([0., 1., 0.]),
                        ..Default::default()
                    });
                    let original = figure.clone();
                    save_matlab_figure(&path, &figure).unwrap();
                    let bytes = std::fs::read(&path).unwrap();
                    let expanded = mat::expand(&bytes).unwrap();
                    let hierarchy = expanded.hierarchy().unwrap();
                    let props = axis_properties(&hierarchy);
                    assert_eq!(
                        optional_axes_rgb(props, "GridColor")
                            .unwrap()
                            .unwrap()
                            .map(f64::to_bits),
                        grid.color.map(f64::to_bits)
                    );
                    assert_eq!(
                        scalar(props, "GridAlpha", -1.).unwrap().to_bits(),
                        alpha.to_bits()
                    );
                    for (name, value) in [
                        ("XColorMode", x),
                        ("YColorMode", y),
                        ("GridColorMode", mode),
                        ("GridAlphaMode", alpha_mode),
                    ] {
                        assert_eq!(string(props, name, "missing").unwrap(), value.as_str());
                    }
                    // The bounded uncompressed MAT field-name table retains
                    // structure order; decoded BTreeMap alone cannot prove it.
                    let location = |name: &str| {
                        let needle = format!("{name}\0");
                        let offsets: Vec<_> = bytes
                            .windows(needle.len())
                            .enumerate()
                            .filter_map(|(i, b)| (b == needle.as_bytes()).then_some(i))
                            .collect();
                        assert_eq!(offsets.len(), 1, "{name}");
                        offsets[0]
                    };
                    let last_value = ["XColor", "YColor", "GridColor", "GridAlpha"]
                        .into_iter()
                        .map(location)
                        .max()
                        .unwrap();
                    let positions = ["XColorMode", "YColorMode", "GridColorMode", "GridAlphaMode"]
                        .map(location);
                    assert!(positions[0] > last_value);
                    assert!(positions.windows(2).all(|p| p[0] < p[1]));
                    let reopened = load_matlab_figure(&path).unwrap();
                    assert_eq!(reopened.colors, figure.colors);
                    assert_eq!(effective(&reopened), effective(&figure));
                    let a = reopened
                        .axes
                        .resolve_colors(reopened.resolve_appearance(BaseTheme::Light));
                    let b = figure
                        .axes
                        .resolve_colors(figure.resolve_appearance(BaseTheme::Light));
                    assert_eq!(a.grid_y, b.grid_y);
                    assert_samples(&figure, &reopened);
                    assert!(original.same_snapshot(&figure));
                    assert!(
                        original.series[0]
                            .points
                            .same_storage(&figure.series[0].points)
                    );
                }
            }
        }
    }
}

#[test]
fn legacy_unstyled_fig_has_no_new_grid_or_ruler_fields_and_keeps_historical_bytes() {
    let mut figure = source();
    figure.appearance = None;
    figure.colors = None;
    let bytes = encode_matlab_figure(&figure).unwrap();
    let expanded = mat::expand(&bytes).unwrap();
    let hierarchy = expanded.hierarchy().unwrap();
    let props = axis_properties(&hierarchy);
    for key in [
        "XColor",
        "YColor",
        "GridColor",
        "GridAlpha",
        "XColorMode",
        "YColorMode",
        "GridColorMode",
        "GridAlphaMode",
    ] {
        assert!(!props.contains_key(key), "{key}");
    }
    let reopened = decode_matlab_figure(&bytes).unwrap();
    assert_eq!(reopened.colors, None);
    assert_eq!(encode_matlab_figure(&reopened).unwrap(), bytes);
    assert_samples(&figure, &reopened);
    // The pre-existing legacy-writer fixture's exact serialized comparison is
    // retained unchanged in width_serialization_tests; no fixture is replaced.
}

#[test]
fn unsupported_or_cancelled_grid_saves_preserve_existing_destination_and_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keep.fig");
    let source = source();
    save_matlab_figure(&path, &source).unwrap();
    let old = std::fs::read(&path).unwrap();
    for case in 0..4 {
        let mut figure = source.clone();
        let cancel = AtomicBool::new(case == 3);
        match case {
            0 => {
                figure.set_figure_theme(FigureTheme::Dark);
                figure.series[0].kind = PlotKind::Stairs;
            }
            1 => {
                figure.colors = Some(AxesColors {
                    grid: Some(AxesGridStyle {
                        alpha: 0.4,
                        ..AxesGridStyle::from_appearance(
                            figure.resolve_appearance(BaseTheme::Light),
                            true,
                        )
                    }),
                    ..Default::default()
                })
            }
            2 => figure.legend = true,
            _ => {}
        }
        let stamp = SnapshotStamp::new(&figure);
        let error =
            save_matlab_figure_with_theme(&path, &figure, BaseTheme::Light, &cancel).unwrap_err();
        assert!(
            error.contains(["stairs/stem", "Automatic GridAlpha", "legends", "Cancelled"][case]),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert!(stamp.matches(&figure));
        assert!(
            figure.series[0]
                .points
                .same_storage(&source.series[0].points)
        );
    }
}
