use super::*;

#[test]
fn host_resolution_is_read_only_and_manual_transitions_capture_effective_colors() {
    let mut f = Figure::new(
        &Matrix::from_column_slice(1, 2, &[-0.0, 1.]).unwrap(),
        &Matrix::from_column_slice(1, 2, &[2., 3.]).unwrap(),
    )
    .unwrap();
    // Historical fixed series policy; adaptive palette cases are separate.
    f.appearance.as_mut().unwrap().series_palette = None;
    f.axes.palette_cycle = None;
    for series in &mut f.series {
        series.auto_palette = None;
    }
    let original = f.clone();
    let stamp = SnapshotStamp::new(&f);
    let light = f.resolve_appearance(BaseTheme::Light);
    let dark = f.resolve_appearance(BaseTheme::Dark);
    assert_ne!(light.canvas, dark.canvas);
    assert_ne!(light.axes, dark.axes);
    assert!(original.same_snapshot(&f));
    assert!(stamp.matches(&f));
    f.set_canvas_mode(ColorMode::Manual, BaseTheme::Dark);
    f.set_theme_mode(ColorMode::Manual, BaseTheme::Dark);
    assert_eq!(f.resolve_appearance(BaseTheme::Light), dark);
    assert!(!stamp.matches(&f));
    assert!(original.series[0].points.same_storage(&f.series[0].points));
    f.set_canvas_color([255; 3]);
    assert_eq!(f.resolve_appearance(BaseTheme::Dark).canvas, [255; 3]);
    f.set_theme_mode(ColorMode::Auto, BaseTheme::Light);
    assert_eq!(f.resolve_appearance(BaseTheme::Dark).canvas, [255; 3]);
    // Stored series RGB is stable; this increment does not implement glow.
    assert_eq!(
        light.series_color(&f.series[0]),
        dark.series_color(&f.series[0])
    );
}

#[test]
fn native_theme_capabilities_preserve_intent_and_reject_downgrades_before_samples() {
    let mut f = Figure::default();
    f.appearance.as_mut().unwrap().series_palette = None;
    f.axes.palette_cycle = None;
    f.add(vec![Series::new(vec![[0., 1.], [1., 2.]], PlotKind::Line)])
        .unwrap();
    f.set_figure_theme(FigureTheme::Dark);
    for subplot in [false, true] {
        let source = if subplot {
            let mut source = f.clone();
            let axes = source.axes.clone();
            source
                .select_subplot(Position::from_grid(1, 2, &[1]).unwrap(), false)
                .unwrap();
            source.axes = axes;
            source
                .select_subplot(Position::from_grid(1, 2, &[2]).unwrap(), false)
                .unwrap();
            source
                .add(vec![Series::new(vec![[2., 3.]], PlotKind::Line)])
                .unwrap();
            source
        } else {
            f.clone()
        };
        let bytes = crate::files::encode_native_figure(&source).unwrap();
        assert!(bytes.starts_with(if subplot { b"MATRFIGS" } else { b"MATRFIGR" }));
        let reopened = crate::files::decode_native_figure(&bytes).unwrap();
        assert_eq!(reopened.appearance, source.appearance);
        assert!(source.same_metadata(&reopened));
        assert_eq!(
            reopened.resolve_appearance(BaseTheme::Light),
            source.resolve_appearance(BaseTheme::Light)
        );
        let mut downgraded = bytes.clone();
        downgraded[7] = if subplot { b'Q' } else { b'P' };
        // Remove all sample/child bytes: appearance admission must fail first.
        let size = u32::from_le_bytes(downgraded[8..12].try_into().unwrap()) as usize;
        downgraded.truncate(12 + size);
        let error = crate::files::decode_native_figure(&downgraded).unwrap_err();
        assert!(error.contains("appearance intent"), "{error}");
    }
}

#[test]
fn legacy_native_absence_and_explicit_light_manual_are_semantically_fixed() {
    let mut old = Figure {
        appearance: None,
        axes: Axes::default(),
        color: [18, 52, 86],
        ..Figure::default()
    };
    old.add(vec![Series::new(vec![[0., 1.]], PlotKind::Line)])
        .unwrap();
    let before = crate::files::encode_native_figure(&old).unwrap();
    assert!(!before.starts_with(b"MATRFIGR"));
    let decoded = crate::files::decode_native_figure(&before).unwrap();
    assert_eq!(decoded.appearance, None);
    assert_eq!(
        crate::files::encode_native_figure(&decoded).unwrap(),
        before
    );
    assert_eq!(
        decoded.resolve_appearance(BaseTheme::Dark),
        ResolvedAppearance::legacy(old.color)
    );
    let mut explicit = old.clone();
    explicit.set_figure_theme(FigureTheme::Light);
    assert!(!old.same_snapshot(&explicit));
    assert_eq!(
        old.resolve_appearance(BaseTheme::Dark),
        explicit.resolve_appearance(BaseTheme::Dark)
    );
    assert_eq!(
        explicit.resolve_appearance(BaseTheme::Dark).canvas,
        old.color
    );
    assert_eq!(explicit.resolve_appearance(BaseTheme::Dark).axes, [255; 3]);
    // Explicit Light/manual keeps the opaque legacy grid; modern Light uses [38; 3] at 0.15.
    assert_eq!(explicit.resolve_appearance(BaseTheme::Dark).grid, [222; 3]);
    explicit.set_canvas_mode(ColorMode::Auto, BaseTheme::Dark);
    let modern = explicit.resolve_appearance(BaseTheme::Dark);
    assert_eq!((modern.legacy, modern.grid), (false, [38; 3]));
}

#[test]
fn fig_static_light_and_dark_preserve_source_and_reject_unsupported_objects_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("static.fig");
    let mut f = Figure::default();
    f.add(vec![Series::new(vec![[0., 1.], [1., 2.]], PlotKind::Line)])
        .unwrap();
    let before = f.clone();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    crate::files::save_matlab_figure_with_cancel(&path, &f, &cancel).unwrap();
    let decoded = crate::files::load_matlab_figure(&path).unwrap();
    assert_eq!(decoded.color, f.resolve_appearance(BaseTheme::Light).canvas);
    assert_eq!(decoded.appearance, None);
    assert!(before.same_snapshot(&f));
    // Dark is now an explicit static RGB projection, never inferred on import.
    let assert_static_dark = |source: &Figure, host: BaseTheme| {
        let snapshot = source.clone();
        let native = crate::files::encode_native_figure(source).unwrap();
        crate::files::save_matlab_figure_with_theme(&path, source, host, &cancel).unwrap();
        let loaded = crate::files::load_matlab_figure(&path).unwrap();
        let appearance = source.resolve_appearance(host);
        assert_eq!(appearance.theme, BaseTheme::Dark);
        assert_eq!(loaded.appearance, None);
        assert_eq!(loaded.color, appearance.canvas);
        assert_eq!(loaded.invert_hardcopy, Some(false));
        let a = source.resolve_colors(appearance);
        let b = loaded.resolve_colors(loaded.resolve_appearance(BaseTheme::Light));
        assert_eq!(
            [
                a.background,
                a.x,
                a.y,
                a.title,
                a.xlabel,
                a.ylabel,
                a.grid_x,
                a.grid_y
            ],
            [
                b.background,
                b.x,
                b.y,
                b.title,
                b.xlabel,
                b.ylabel,
                b.grid_x,
                b.grid_y
            ]
        );
        assert_eq!(a.grid_alpha.to_bits(), b.grid_alpha.to_bits());
        assert_eq!(
            loaded.series[0].color,
            appearance.series_color(&source.series[0])
        );
        assert_eq!(loaded.series[0].points, source.series[0].points);
        assert!(snapshot.same_snapshot(source));
        assert!(
            snapshot.series[0]
                .points
                .same_storage(&source.series[0].points)
        );
        assert_eq!(crate::files::encode_native_figure(source).unwrap(), native);
    };
    assert_static_dark(&f, BaseTheme::Dark);
    assert!(before.same_snapshot(&f));
    f.set_canvas_color([255; 3]);
    crate::files::save_matlab_figure_with_cancel(&path, &f, &cancel).unwrap();
    assert_eq!(
        crate::files::load_matlab_figure(&path).unwrap().color,
        [255; 3]
    );
    f.set_figure_theme(FigureTheme::Dark);
    assert!(crate::files::matlab_figure_export_restriction(&f).is_none());
    assert_static_dark(&f, BaseTheme::Light);
    // Keep rejection/atomicity coverage for a genuinely unsupported Dark scene.
    let bytes = std::fs::read(&path).unwrap();
    let mut unsupported = f.clone();
    unsupported.series[0].kind = PlotKind::Bar;
    let snapshot = unsupported.clone();
    assert!(
        crate::files::matlab_figure_export_restriction(&unsupported)
            .unwrap()
            .contains("bar export")
    );
    assert!(
        crate::files::save_matlab_figure_with_theme(&path, &unsupported, BaseTheme::Light, &cancel)
            .unwrap_err()
            .contains("bar export")
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(snapshot.same_snapshot(&unsupported));
    assert!(
        snapshot.series[0]
            .points
            .same_storage(&unsupported.series[0].points)
    );
}

#[test]
fn plot_and_clf_keep_figure_intent_but_reset_is_not_silently_substituted() {
    let mut engine = crate::Engine::default();
    engine.set_graphics_theme_context(BaseTheme::Dark);
    engine
        .execute("f=figure('Name','persist','Color',[1 1 1],'Theme','dark');plot([1 2],[3 4]);")
        .unwrap();
    let figure = engine.figure.as_ref().unwrap().clone();
    engine.execute("clf;").unwrap();
    let cleared = engine.figure.as_ref().unwrap();
    assert_eq!(cleared.appearance, figure.appearance);
    assert_eq!(cleared.color, figure.color);
    assert_eq!(cleared.name, figure.name);
    assert!(cleared.identity.same_object(&figure.identity));
    assert!(cleared.series.is_empty());
    let cleared = cleared.clone();
    assert!(engine.execute("clf('reset');").is_err());
    assert!(cleared.same_snapshot(engine.figure.as_ref().unwrap()));
    engine.execute("plot([2 4],[6 8]);").unwrap();
    assert_eq!(
        engine.figure.as_ref().unwrap().appearance,
        figure.appearance
    );
}

#[test]
fn tiled_conversions_keep_figure_properties_and_existing_tiles_keep_shared_samples() {
    let fresh = Figure::tiled_layout(TileGrid::new(Arrangement::Fixed, 1, 2).unwrap()).unwrap();
    assert_eq!(fresh.appearance, Some(FigureAppearance::default()));
    for conversion in ["tiledlayout(1,2);nexttile;", "nexttile;"] {
        for legacy in [false, true] {
            let mut engine = crate::Engine::default();
            engine.execute("f=figure('Name','tiled intent','Color',[0.1 0.2 0.3],'Theme','dark');plot([0 1],[1 2]);").unwrap();
            if legacy {
                engine.figure.as_mut().unwrap().appearance = None;
            }
            let before = engine.figure.as_ref().unwrap().clone();
            engine.execute(conversion).unwrap();
            let converted = engine.figure.as_ref().unwrap();
            assert!(converted.identity.same_object(&before.identity));
            assert_eq!(converted.name, before.name);
            assert_eq!(converted.color, before.color);
            assert_eq!(converted.appearance, before.appearance);
            engine.execute("plot([0 1],[3 4]);").unwrap();
            let points = engine.figure.as_ref().unwrap().series[0].points.clone();
            engine.execute("nexttile(1);").unwrap();
            let selected = engine.figure.as_ref().unwrap();
            assert!(selected.series[0].points.same_storage(&points));
            assert_eq!(selected.name, before.name);
            assert_eq!(selected.color, before.color);
            assert_eq!(selected.appearance, before.appearance);
        }
    }
}
