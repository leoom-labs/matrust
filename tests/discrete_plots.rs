use matrust::{
    Engine,
    plot::{AxisScale, AxisTransform, PlotKind, Series, stair_geometry},
};

#[test]
fn discrete_calls_preserve_samples_styles_and_column_series() {
    let mut e = Engine::default();
    e.execute("stairs([3 1 1],[2 -1 4],'r--o');hold on;stem([3 1 1],[2 -1 4],'b-','filled');")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].kind, PlotKind::Stairs);
    assert_eq!(f.series[1].kind, PlotKind::Stem);
    assert_eq!(
        f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
        &[[3., 2.], [1., -1.], [1., 4.]]
    );
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        f.series[0].points.iter().collect::<Vec<_>>().as_slice()
    );
    assert!(f.series[0].markers && f.series[1].markers && f.series[1].marker_filled);
    e.execute("hold off;stem([1 2;3 4;5 6]);").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 2);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        &[[1., 2.], [2., 4.], [3., 6.]]
    );
    let previous = f.clone();
    assert!(e.execute("stairs([1 2],[1 2;3 4;5 6]);").is_err());
    assert!(previous.same_snapshot(e.figure.as_ref().unwrap()));
    // Explicit finite Stem baselines are now supported, with original samples.
    e.execute("stem([1 2],'BaseValue',2);").unwrap();
    let supported = e.figure.as_ref().unwrap().clone();
    assert_eq!(supported.series.len(), 1);
    assert_eq!(supported.series[0].kind, PlotKind::Stem);
    assert_eq!(supported.series[0].base_value, Some(2.));
    assert_eq!(
        supported.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        &[[1., 1.], [2., 2.]]
    );
    let geometry = supported.series[0].display_points(100);
    assert_eq!(geometry[0], [1., 2.]); // Explicit baseline.
    assert_eq!(geometry[1], [1., 1.]); // Original sample.
    assert_eq!(geometry[3], [2., 2.]);
    // Invalid baselines and unsupported use on Stairs remain atomic failures.
    for source in [
        "stem([1 2],'BaseValue',Inf);",
        "stem([1 2],'BaseValue',[1 2]);",
        "stairs([1 2],'BaseValue',2);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(
            supported.same_snapshot(e.figure.as_ref().unwrap()),
            "{source}"
        );
    }
}

#[test]
fn staircase_geometry_is_exact_and_invalid_samples_break_connections() {
    assert_eq!(
        stair_geometry(&[[0., 2.], [3., -1.], [5., 4.]]),
        vec![[0., 2.], [3., 2.], [3., -1.], [5., -1.], [5., 4.]]
    );
    assert!(stair_geometry(&[]).is_empty());
    let geometry = stair_geometry(&[[0., 2.], [1., f64::NAN], [2., 3.], [3., 4.]]);
    assert!(geometry[1].iter().all(|v| v.is_nan()));
    assert!(geometry[3].iter().all(|v| v.is_nan()));
    assert_eq!(&geometry[4..], &[[2., 3.], [3., 3.], [3., 4.]]);
    let mut e = Engine::default();
    e.execute("stairs([0 3 5],[2 -1 4]);axis([0 5 -1 4]);")
        .unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(
        svg.contains("M70.00,232.00 L580.00,232.00 L580.00,490.00 L920.00,490.00 L920.00,60.00"),
        "{svg}"
    );
}

#[test]
fn stems_have_independent_segments_valid_baselines_and_bounded_display() {
    let mut e = Engine::default();
    e.execute("stem([0 1 2],[2 NaN 4]);").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.bounds().1, [0., 4.]);
    let geometry = f.series[0].display_points(100);
    assert_eq!(&geometry[..2], &[[0., 0.], [0., 2.]]);
    assert!(geometry[3..6].iter().flatten().all(|v| v.is_nan()));
    assert_eq!(&geometry[6..8], &[[2., 0.], [2., 4.]]);
    e.execute("stem([1 2],[-10 -100]);yscale('log');").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.bounds().1, [-100., -1.]);
    assert_eq!(
        f.series[0].display_points_axes(100, f.axis_transforms())[1],
        [1., -1.]
    );
    for kind in [PlotKind::Stairs, PlotKind::Stem] {
        let mut series = Series::new(
            (0..100000).map(|i| [i as f64, (i % 19) as f64]).collect(),
            kind,
        );
        let pointer = series.points.downgrade();
        assert!(series.display_points(8192).len() <= 8192);
        assert!(pointer.matches(&series.points));
        series.visible = false;
        assert!(series.display_points(8192).is_empty());
    }
    let series = Series::new(vec![[0., 2.], [1., 3.], [2., 4.]], PlotKind::Stairs);
    let axes = [
        AxisTransform {
            scale: AxisScale::Log,
            negative: false,
        },
        AxisTransform::default(),
    ];
    let result = series.display_points_axes(100, axes);
    assert!(result[1].iter().all(|v| v.is_nan()));
}

#[test]
fn discrete_native_and_csv_roundtrips_keep_original_samples_and_visibility() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/discrete_plots.m"))
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
    f.series[1].visible = false;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("discrete.mfig");
    matrust::files::save_figure(&path, &f).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(f.legend_state.is_some());
    assert_eq!(&bytes[..8], b"MATRFIGJ");
    let loaded = matrust::files::load_figure(&path).unwrap();
    assert_eq!(loaded.svg(), f.svg());
    assert!(loaded.axes.same_metadata(&f.axes));
    assert_eq!(loaded.point_count(), 82);
    assert_eq!(loaded.series[1].kind, PlotKind::Stem);
    assert!(!loaded.series[1].visible);
    let csv = dir.path().join("samples.csv");
    matrust::files::write_figure_csv(&csv, &loaded).unwrap();
    assert_eq!(std::fs::read_to_string(csv).unwrap().lines().count(), 83);
    let fig = dir.path().join("unsupported.fig");
    std::fs::write(&fig, b"keep").unwrap();
    f.legend = false;
    assert!(matrust::files::save_figure(&fig, &f).is_err());
    assert_eq!(std::fs::read(fig).unwrap(), b"keep");
    let mut old = bytes;
    old[..8].copy_from_slice(b"MATRFIG4");
    std::fs::write(&path, old).unwrap();
    assert_eq!(
        matrust::files::load_figure(&path).unwrap_err(),
        "Advanced figure metadata requires MFIGJ"
    );
}
