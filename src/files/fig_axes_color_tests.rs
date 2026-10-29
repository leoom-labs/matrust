use super::*;

// Authored minimal supported projection of the observed axes/text RGB properties.
// It is not a MATLAB-produced fixture and does not import the shipped arrow scene.
fn projection(
    background: mat::Encoded<'static>,
    extra: Option<(&'static str, mat::Encoded<'static>)>,
) -> Vec<u8> {
    projection_fields(background, extra.into_iter().collect())
}
fn projection_fields(
    background: mat::Encoded<'static>,
    extras: Vec<(&'static str, mat::Encoded<'static>)>,
) -> Vec<u8> {
    let mut props = vec![
        ("Color", background),
        ("XColor", num(&[-0., 0.15, 1.])),
        ("YColor", num(&[0.25, 0.5, 0.75])),
        ("XGrid", mat::text("off")),
        ("YGrid", mat::text("off")),
        ("XLim", num(&[0., 1.])),
        ("YLim", num(&[1., 3.])),
    ];
    props.extend(extras);
    let line = node(
        "graph2d.lineseries",
        3.,
        vec![
            ("XData", num(&[-0., 1.])),
            ("YData", num(&[2., 3.])),
            ("Color", num(&[0., 0., 1.])),
        ],
        vec![],
        &[],
    );
    let title = node(
        "text",
        4.,
        vec![
            ("String", mat::text("")),
            ("Color", num(&[0.125, 0.25, 0.5])),
            ("ColorMode", mat::text("auto")),
        ],
        vec![],
        &[],
    );
    let xlabel = node(
        "text",
        5.,
        vec![("String", mat::text("Time")), ("Color", num(&[1., 0., 0.]))],
        vec![],
        &[],
    );
    let ylabel = node(
        "text",
        6.,
        vec![
            ("String", mat::text("Value")),
            ("Color", num(&[0., 1., 0.])),
        ],
        vec![],
        &[],
    );
    let axes = node(
        "axes",
        2.,
        props,
        vec![line, title, xlabel, ylabel],
        &[2., 3., 4., 0.],
    );
    mat::file(mat::structure(
        "hgS_070000",
        node(
            "figure",
            1.,
            vec![("CurrentAxes", num(&[2.]))],
            vec![axes],
            &[],
        ),
    ))
    .unwrap()
}

#[test]
fn genuine_saved_reference_label_rgb_is_imported_without_theme_or_color_mode_inference() {
    for name in ["matlab-lines.fig", "matlab-lines-v6.fig"] {
        let path = Path::new("tests/fixtures/fig").join(name);
        let figure = load_matlab_figure(&path).unwrap();
        assert_eq!(figure.appearance, None);
        let colors = figure.colors.unwrap();
        assert_eq!(colors.background, None);
        assert_eq!(colors.x, None);
        assert_eq!(colors.y, None);
        assert_eq!(colors.title, Some([0.; 3]));
        assert_eq!(colors.xlabel, Some([0.15; 3]));
        assert_eq!(colors.ylabel, Some([0.15; 3]));
        assert_eq!(
            (&*figure.title, &*figure.xlabel, &*figure.ylabel),
            ("Shared figure", "Time", "Amplitude")
        );
        let bytes = crate::files::encode_native_figure(&figure).unwrap();
        let native = crate::files::decode_native_figure(&bytes).unwrap();
        assert!(figure.same_metadata(&native));
        assert_eq!(crate::files::encode_native_figure(&native).unwrap(), bytes);
    }
}

#[test]
fn explicit_light_rgb_fig_projection_keeps_colors_and_empty_label_through_two_saves() {
    let bytes = projection(num(&[1., 1., 1.]), None);
    let source = decode_matlab_figure(&bytes).unwrap();
    let stamp = crate::plot::SnapshotStamp::new(&source);
    let mut current = source.clone();
    for _ in 0..2 {
        let written = encode_matlab_figure(&current).unwrap();
        current = decode_matlab_figure(&written).unwrap();
        assert_eq!(current.colors, source.colors);
        assert!(current.title.is_empty());
        assert_eq!(current.xlabel, "Time");
        assert_eq!(current.ylabel, "Value");
        for (a, b) in current.series[0]
            .points
            .iter()
            .zip(source.series[0].points.iter())
        {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    assert!(stamp.matches(&source));
    assert!(
        source.series[0]
            .points
            .same_storage(&source.clone().series[0].points)
    );
    let mut dark = source.clone();
    dark.set_figure_theme(crate::plot::FigureTheme::Dark);
    dark.series[0].kind = PlotKind::Stairs; // Still outside the Dark static subset.
    let error = encode_matlab_figure(&dark).unwrap_err();
    assert!(error.contains("stairs/stem"), "{error}");
    assert_eq!(dark.colors, source.colors);
}

#[test]
fn unsupported_color_storage_and_unrepresented_explicit_grid_styles_are_not_dropped() {
    for rgb in [
        mat::text("none"),
        num(&[0., 1.]),
        num(&[0., f64::NAN, 1.]),
        num(&[0., -0.1, 1.]),
        num(&[0., f64::INFINITY, 1.]),
    ] {
        assert!(decode_matlab_figure(&projection(rgb, None)).is_err());
    }
    for (key, value) in [
        ("GridColor", num(&[1., 0., 0.])),
        ("GridAlpha", num(&[0.5])),
        ("MinorGridColor", num(&[1., 0., 0.])),
        ("MinorGridAlpha", num(&[0.5])),
    ] {
        let error =
            decode_matlab_figure(&projection(num(&[1.; 3]), Some((key, value)))).unwrap_err();
        assert!(error.contains(key), "{error}");
    }
    for key in ["MinorGridColorMode", "MinorGridAlphaMode"] {
        for mode in ["auto", "manual"] {
            let error =
                decode_matlab_figure(&projection(num(&[1.; 3]), Some((key, mat::text(mode)))))
                    .unwrap_err();
            assert!(error.contains(key), "{error}");
        }
    }
    for key in ["XMinorGrid", "YMinorGrid"] {
        let error = decode_matlab_figure(&projection(num(&[1.; 3]), Some((key, mat::text("on")))))
            .unwrap_err();
        assert!(error.contains(key), "{error}");
        assert!(
            decode_matlab_figure(&projection(num(&[1.; 3]), Some((key, mat::text("off"))))).is_ok()
        );
        assert!(
            decode_matlab_figure(&projection(
                num(&[1.; 3]),
                Some((key, mat::text("invalid")))
            ))
            .is_err()
        );
    }
    // The exact observed shipped storage kind (miUINT8 inside mxDOUBLE) is
    // accepted through the same borrowed number reader; no f64 array copy.
    let data = [1_u8, 0, 1];
    let fields = BTreeMap::from([(
        "Color".into(),
        Value::Number {
            kind: 2,
            endian: mat::Endian::Big,
            bytes: &data,
            count: 3,
        },
    )]);
    assert_eq!(
        optional_axes_rgb(&fields, "Color").unwrap(),
        Some([1., 0., 1.])
    );
}

#[test]
fn complete_authored_grid_records_preserve_independent_modes_through_fig_and_script() {
    use crate::plot::{BaseTheme, ColorMode};
    for x in ["auto", "manual"] {
        for y in ["auto", "manual"] {
            for grid in ["auto", "manual"] {
                let bytes = projection_fields(
                    num(&[1.; 3]),
                    vec![
                        ("GridColor", num(&[0.6, 0.2, 0.4])),
                        ("GridAlpha", num(&[0.4])),
                        ("XColorMode", mat::text(x)),
                        ("YColorMode", mat::text(y)),
                        ("GridColorMode", mat::text(grid)),
                        ("GridAlphaMode", mat::text("manual")),
                    ],
                );
                let source = decode_matlab_figure(&bytes).unwrap();
                let saved = source.colors.unwrap().grid.unwrap();
                assert_eq!(
                    saved.x_mode,
                    if x == "auto" {
                        ColorMode::Auto
                    } else {
                        ColorMode::Manual
                    }
                );
                assert_eq!(
                    saved.y_mode,
                    if y == "auto" {
                        ColorMode::Auto
                    } else {
                        ColorMode::Manual
                    }
                );
                let resolved = source
                    .axes
                    .resolve_colors(source.resolve_appearance(BaseTheme::Light));
                assert_eq!(
                    resolved.grid_x,
                    if grid == "auto" && x == "manual" {
                        [0, 38, 255]
                    } else {
                        [153, 51, 102]
                    }
                );
                assert_eq!(
                    resolved.grid_y,
                    if grid == "auto" && y == "manual" {
                        [64, 128, 191]
                    } else {
                        [153, 51, 102]
                    }
                );
                assert_eq!(resolved.grid_alpha, 0.4);
                let reopened =
                    decode_matlab_figure(&encode_matlab_figure(&source).unwrap()).unwrap();
                assert_eq!(reopened.colors, source.colors);
                let reopened_colors = reopened
                    .axes
                    .resolve_colors(reopened.resolve_appearance(BaseTheme::Light));
                assert_eq!(
                    (
                        reopened_colors.grid_x,
                        reopened_colors.grid_y,
                        reopened_colors.grid_alpha
                    ),
                    (resolved.grid_x, resolved.grid_y, resolved.grid_alpha)
                );
                let native = crate::files::decode_native_figure(
                    &crate::files::encode_native_figure(&source).unwrap(),
                )
                .unwrap();
                assert!(source.same_metadata(&native));
                let mut current = source.clone();
                for _ in 0..2 {
                    let script = crate::files::matlab_plot_script(
                        &current,
                        &std::sync::atomic::AtomicBool::new(false),
                    )
                    .unwrap();
                    let mut engine = crate::Engine::default();
                    engine.execute(&script).unwrap();
                    current = engine.figure.unwrap();
                    assert_eq!(current.colors, source.colors);
                    let actual = current
                        .axes
                        .resolve_colors(current.resolve_appearance(BaseTheme::Light));
                    assert_eq!(
                        (actual.grid_x, actual.grid_y, actual.grid_alpha),
                        (resolved.grid_x, resolved.grid_y, resolved.grid_alpha)
                    );
                }
            }
        }
    }
    for alpha in [0.15, 0.4] {
        let result = decode_matlab_figure(&projection_fields(
            num(&[1.; 3]),
            vec![
                ("GridColor", num(&[0.6, 0.2, 0.4])),
                ("GridAlpha", num(&[alpha])),
                ("XColorMode", mat::text("auto")),
                ("YColorMode", mat::text("auto")),
                ("GridColorMode", mat::text("auto")),
                ("GridAlphaMode", mat::text("auto")),
            ],
        ));
        assert_eq!(result.is_ok(), alpha == 0.15);
    }
}

#[test]
fn visible_ruler_and_modern_implicit_grids_publish_complete_static_light_fig() {
    let source = decode_matlab_figure(&projection(num(&[1.; 3]), None)).unwrap();
    let root = std::env::temp_dir().join(format!(
        "matrust-fig-ruler-grid-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let destination = root.join("existing.fig");
    let old = encode_matlab_figure(&source).unwrap();
    std::fs::write(&destination, &old).unwrap();
    for x in [false, true] {
        let mut edited = source.clone();
        edited.xgrid = x;
        edited.ygrid = !x;
        let stamp = crate::plot::SnapshotStamp::new(&edited);
        save_matlab_figure(&destination, &edited).unwrap();
        let reopened = load_matlab_figure(&destination).unwrap();
        let expected = edited
            .axes
            .resolve_colors(edited.resolve_appearance(crate::plot::BaseTheme::Light));
        let actual = reopened
            .axes
            .resolve_colors(reopened.resolve_appearance(crate::plot::BaseTheme::Light));
        assert_eq!(
            (actual.grid_x, actual.grid_y, actual.grid_alpha),
            (expected.grid_x, expected.grid_y, expected.grid_alpha)
        );
        assert_eq!((reopened.xgrid, reopened.ygrid), (x, !x));
        assert_eq!(reopened.colors.unwrap().grid.unwrap().alpha, 1.);
        let native = crate::files::decode_native_figure(
            &crate::files::encode_native_figure(&edited).unwrap(),
        )
        .unwrap();
        assert!(native.same_metadata(&edited));
        assert!(stamp.matches(&edited));
        assert!(
            edited.series[0]
                .points
                .same_storage(&source.series[0].points)
        );
    }
    let mut modern = source.clone();
    modern.colors = None;
    modern.set_figure_theme(crate::plot::FigureTheme::Auto);
    for x in [false, true] {
        modern.xgrid = x;
        modern.ygrid = !x;
        let stamp = crate::plot::SnapshotStamp::new(&modern);
        save_matlab_figure(&destination, &modern).unwrap();
        let reopened = load_matlab_figure(&destination).unwrap();
        let grid = reopened.colors.unwrap().grid.unwrap();
        assert_eq!(
            (grid.alpha, grid.color_mode, grid.alpha_mode),
            (
                0.15,
                crate::plot::ColorMode::Auto,
                crate::plot::ColorMode::Auto
            )
        );
        assert_eq!((reopened.xgrid, reopened.ygrid), (x, !x));
        assert!(stamp.matches(&modern));
        let native = crate::files::decode_native_figure(
            &crate::files::encode_native_figure(&modern).unwrap(),
        )
        .unwrap();
        assert!(native.same_metadata(&modern));
    }
    modern.xgrid = false;
    modern.ygrid = false;
    save_matlab_figure(&destination, &modern).unwrap();
    let reopened = load_matlab_figure(&destination).unwrap();
    assert!(!reopened.xgrid && !reopened.ygrid);
    assert_eq!(
        reopened.series[0].points.len(),
        modern.series[0].points.len()
    );
    assert!(
        modern.series[0]
            .points
            .same_storage(&source.series[0].points)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn modern_ruler_without_grid_rejects_public_script_atomically_but_legacy_replays() {
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing_plot.m");
    let cancel = AtomicBool::new(false);
    let reason = "Axes ruler RGB on a modern figure needs explicit grid state for script reconstruction; use MFIG";
    let mut engine = crate::Engine::default();
    engine
        .execute("plot([-0 1],[2 3]);xlabel('Time');ylabel('Value');")
        .unwrap();
    for x in [false, true] {
        let mut source = engine.figure.clone().unwrap();
        let rgb = [-0., 0.25, 0.75];
        source.colors = Some(crate::plot::AxesColors {
            x: x.then_some(rgb),
            y: (!x).then_some(rgb),
            ..Default::default()
        });
        assert!(source.appearance.is_some());
        let snapshot = source.clone();
        let native = crate::files::encode_native_figure(&source).unwrap();
        std::fs::write(&destination, b"existing destination remains intact").unwrap();
        assert_eq!(
            crate::files::matlab_plot_script(&source, &cancel).unwrap_err(),
            reason
        );
        assert_eq!(
            crate::files::write_matlab_plot_script_with_cancel(&destination, &source, &cancel)
                .unwrap_err(),
            reason
        );
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"existing destination remains intact"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        assert!(source.same_snapshot(&snapshot));
        assert_eq!(crate::files::encode_native_figure(&source).unwrap(), native);
        assert!(
            source.series[0]
                .points
                .same_storage(&engine.figure.as_ref().unwrap().series[0].points)
        );
        // The supported legacy RGB projection remains writable and executable.
        source.appearance = None;
        let legacy = source.clone();
        crate::files::write_matlab_plot_script_with_cancel(&destination, &source, &cancel).unwrap();
        let script = std::fs::read_to_string(&destination).unwrap();
        let mut replay = crate::Engine::default();
        replay.execute(&script).unwrap();
        let reopened = replay.figure.unwrap();
        let expected = source
            .axes
            .resolve_colors(source.resolve_appearance(crate::plot::BaseTheme::Light));
        let actual = reopened
            .axes
            .resolve_colors(reopened.resolve_appearance(crate::plot::BaseTheme::Light));
        assert_eq!(
            (
                actual.x,
                actual.y,
                actual.grid_x,
                actual.grid_y,
                actual.grid_alpha
            ),
            (
                expected.x,
                expected.y,
                expected.grid_x,
                expected.grid_y,
                expected.grid_alpha
            )
        );
        for (a, b) in reopened.series[0]
            .points
            .iter()
            .zip(source.series[0].points.iter())
        {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
        assert!(source.same_snapshot(&legacy));
    }
}
