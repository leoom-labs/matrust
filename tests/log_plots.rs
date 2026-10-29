use matrust::{
    Engine, files,
    plot::{AxisScale, AxisTransform, Figure, PlotKind, Series},
};
fn run(script: &str) -> Figure {
    let mut e = Engine::default();
    e.execute(script).unwrap();
    e.figure.unwrap()
}

#[test]
fn log_plot_commands_hold_scales_and_transactional_limits() {
    for (name, x, y) in [
        ("semilogx", AxisScale::Log, AxisScale::Linear),
        ("semilogy", AxisScale::Linear, AxisScale::Log),
        ("loglog", AxisScale::Log, AxisScale::Log),
    ] {
        let f = run(&format!("{name}([1 10 100],[2 20 200],'r--o');"));
        assert_eq!((f.xscale, f.yscale), (x, y));
        assert_eq!(
            f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 2.], [10., 20.], [100., 200.]]
        );
    }
    let mut e = Engine::default();
    e.execute("semilogx([1 10],[1 2]);hold on;semilogy([1 10],[2 3]);")
        .unwrap();
    assert_eq!(
        (
            e.figure.as_ref().unwrap().xscale,
            e.figure.as_ref().unwrap().yscale
        ),
        (AxisScale::Log, AxisScale::Linear)
    );
    let source = e.figure.as_ref().unwrap().series[0].points.clone();
    e.execute("yscale('log');xlim([1 100]);").unwrap();
    assert!((source).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    for invalid in ["axis([-1 1 1 10]);", "ylim([0 5]);", "xscale('invalid');"] {
        assert!(e.execute(invalid).is_err());
        assert_eq!(e.figure.as_ref().unwrap().xlim, Some([1., 100.]));
        assert_eq!(e.figure.as_ref().unwrap().yscale, AxisScale::Log);
    }
    e.execute("hold off;plot([1 2]);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().xscale, AxisScale::Linear);
    let f = run("hold on;semilogx([1 10],[2 3]);");
    assert_eq!(f.xscale, AxisScale::Linear);
}

#[test]
fn transforms_preserve_sign_order_extremes_and_break_invalid_runs() {
    for negative in [false, true] {
        let axis = AxisTransform {
            scale: AxisScale::Log,
            negative,
        };
        for x in [f64::from_bits(1), 1e-200, 0.1, 1., 10., 1e200, f64::MAX] {
            let x = if negative { -x } else { x };
            let v = axis.forward(x);
            assert!(v.is_finite());
            let inverse = axis.inverse(v);
            assert!(axis.accepts(inverse));
            assert!((inverse - x).abs() <= x.abs() * 2e-13 || inverse == x);
        }
        assert!(!axis.accepts(0.));
        assert!(!axis.accepts(f64::INFINITY));
        assert!(axis.ticks([-323., 309.]).len() <= 10);
    }
    let f = run("loglog([-100 -10 -1],[1 10 100]);");
    assert!(f.axis_transforms()[0].negative);
    assert_eq!(
        f.series[0].display_points_axes(100, f.axis_transforms()),
        vec![[-2., 0.], [-1., 1.], [0., 2.]]
    );
    let f = run("semilogx([-10 0 1 10 NaN 100],[1 2 3 4 5 6]);");
    let display = f.series[0].display_points_axes(100, f.axis_transforms());
    assert!(display[0][0].is_nan() && display[1][0].is_nan() && display[4][0].is_nan());
    assert_eq!(f.bounds(), ([1., 100.], [3., 6.]));
    let mut points: Vec<_> = (1..=100000).map(|i| [i as f64, 1.]).collect();
    points[12345][0] = 0.;
    points[54321][1] = 1000.;
    let s = Series::new(points, PlotKind::Line);
    let display = s.display_points_axes(
        600,
        [AxisTransform {
            scale: AxisScale::Log,
            negative: false,
        }; 2],
    );
    assert!(display.len() <= 600 && display.iter().any(|p| p[0].is_nan()));
    assert!(display.iter().any(|p| p[1] == 3.));
    for script in [
        "loglog([0 0],[0 0]);",
        "loglog([1e300 1e300],[1e-300 1e-300]);",
        "loglog([-1e300 -1e300],[-1e-300 -1e-300]);",
    ] {
        let f = run(script);
        let (x, y) = f.bounds();
        f.xscale.validate_limits(x).unwrap();
        f.yscale.validate_limits(y).unwrap();
        let svg = f.svg();
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
}

#[test]
fn logarithmic_figures_roundtrip_exact_samples_and_export_in_data_units() {
    let f = run("loglog([1 10 100],[1 10 100],'b-o');axis([1 100 1 100]);");
    let svg = f.svg();
    assert!(svg.contains("M70.00,490.00 L495.00,275.00 L920.00,60.00"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.mfig");
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(
        (loaded.xscale, loaded.yscale),
        (AxisScale::Log, AxisScale::Log)
    );
    assert_eq!(loaded.xlim, f.xlim);
    assert_eq!(loaded.svg(), svg);
    assert_eq!(
        loaded.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        f.series[0].points.iter().collect::<Vec<_>>().as_slice()
    );
    let csv = dir.path().join("data.csv");
    files::write_figure_csv(&csv, &loaded).unwrap();
    let data = files::read_csv(&csv).unwrap();
    assert_eq!(data.get(2, 0), 100.);
    // Version 2 figures without scale metadata still load as linear.
    let bytes = std::fs::read(&path).unwrap();
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut meta: serde_json::Value = serde_json::from_slice(&bytes[12..12 + n]).unwrap();
    meta.as_object_mut().unwrap().remove("xscale");
    meta.as_object_mut().unwrap().remove("yscale");
    // Version 2 predates appearance intent as well as logarithmic scale metadata.
    meta.as_object_mut().unwrap().remove("appearance");
    meta.as_object_mut().unwrap().remove("palette_cycle");
    for series in meta["series"].as_array_mut().unwrap() {
        series.as_object_mut().unwrap().remove("auto_palette");
    }
    let json = serde_json::to_vec(&meta).unwrap();
    let mut old = b"MATRFIG2".to_vec();
    old.extend_from_slice(&(json.len() as u32).to_le_bytes());
    old.extend(json);
    old.extend_from_slice(&bytes[12 + n..]);
    std::fs::write(&path, old).unwrap();
    assert_eq!(files::load_figure(&path).unwrap().xscale, AxisScale::Linear);
}

#[test]
fn log_bars_and_mixed_domain_exports_remain_finite() {
    for script in [
        "bar([1 10],[0.1 100]);yscale('log');xscale('log');",
        "bar([-10 -1],[-10 -100]);yscale('log');xscale('log');",
        "semilogx([1 10 0 100],[1 2 3 4]);hold on;scatter([1 0 100],[4 5 6]);",
    ] {
        let f = run(script);
        f.validate().unwrap();
        let svg = f.svg();
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
}
