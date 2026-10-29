use matrust::{
    Engine, Matrix, files,
    plot::{Figure, LinePattern, PlotKind},
};
fn run(source: &str) -> Figure {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e.figure.unwrap()
}
#[test]
fn matrix_series_pairs_styles_and_hold_preserve_data() {
    let f = run(
        "x=[1 2 3];Y=[2 4;3 5;4 6];plot(x,Y);hold on;plot(x,x.^2,'r--o');legend('a','b','square');grid off;",
    );
    assert_eq!(f.series.len(), 3);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        &[[1., 4.], [2., 5.], [3., 6.]]
    );
    assert_eq!(f.series[2].color, [255, 0, 0]);
    assert_eq!(f.series[2].pattern, LinePattern::Dashed);
    assert!(f.series[2].markers && f.legend && f.hold && !f.xgrid && !f.ygrid);
    let f = run("plot([1 2],[2 3],'b:',[2 4],[5 6],'ko');");
    assert_eq!(f.series.len(), 2);
    assert_eq!(f.series[1].kind, PlotKind::Scatter);
    assert!(run("plot([1 2],'k.');").series[0].marker_filled);
    let f = run("plot([1 2],[2 3]);hold on;plot([1 2],[3 4]);hold off;plot([5 6]);");
    assert_eq!(f.series.len(), 1);
    assert_eq!(f.series[0].points.get(0).unwrap(), [1., 5.]);
}
#[test]
fn row_and_column_orientations_and_shared_y() {
    let f = run("plot([10 20],[1 2;3 4;5 6]);");
    assert_eq!(f.series.len(), 3);
    assert_eq!(
        f.series[2].points.iter().collect::<Vec<_>>().as_slice(),
        &[[10., 5.], [20., 6.]]
    );
    let f = run("plot([1 2;3 4;5 6],[10 20 30]);");
    assert_eq!(f.series.len(), 2);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        &[[2., 10.], [4., 20.], [6., 30.]]
    );
    let f = run("plot([1 2;3 4;5 6],[10 20]);");
    assert_eq!(f.series.len(), 3);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        &[[3., 10.], [4., 20.]]
    );
}
#[test]
fn figure_selection_retains_shared_storage_and_independent_axes() {
    let mut e = Engine::default();
    e.execute("figure(3);plot([1 2],[3 4]);hold on;xlim([0 3]);")
        .unwrap();
    let data = e.figure.as_ref().unwrap().series[0].points.clone();
    e.execute("figure(7);bar([4 -2 5]);figure(3);").unwrap();
    assert!((data).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    assert!(e.figure.as_ref().unwrap().hold);
    assert_eq!(e.figure_ids(), vec![3, 7]);
    e.execute("close(3);").unwrap();
    assert_eq!(e.current_figure, 7);
    assert_eq!(e.figure.as_ref().unwrap().series[0].kind, PlotKind::Bar);
    e.execute("close all;").unwrap();
    assert!(e.figure_ids().is_empty());
    for i in 1..=16 {
        e.select_figure(i).unwrap();
    }
    assert!(e.select_figure(17).is_err());
    assert_eq!(e.figure_ids().len(), 16);
}
#[test]
fn invalid_plot_and_axis_edits_do_not_replace_figure() {
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4]);axis([0 5 -2 6]);").unwrap();
    let data = e.figure.as_ref().unwrap().series[0].points.clone();
    for bad in [
        "plot([1 2],[1 2 3])",
        "plot([1 2],'bad')",
        "axis([0 1 5 2])",
        "legend('a','b')",
        "bar([2 1],[1 2])",
        "plot(zeros(2,65))",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
        assert!((data).same_storage(&e.figure.as_ref().unwrap().series[0].points));
        assert_eq!(e.figure.as_ref().unwrap().ylim, Some([-2., 6.]));
    }
    e.execute("limits=axis();xlim('auto');clf;").unwrap();
    assert_eq!(e.variables["limits"].data(), &[0., 5., -2., 6.]);
    assert!(e.figure.unwrap().series.is_empty());
}
#[test]
fn grouped_bars_have_separate_offsets_and_zero_baseline() {
    let f = run("bar([10 20 30],[1 -2;3 4;5 6]);");
    assert_eq!(f.series.len(), 2);
    assert!(f.series[0].bar_offset < 0. && f.series[1].bar_offset > 0.);
    assert_eq!(f.series[0].bar_width, 4.);
    assert_eq!(f.bounds().1, [-2., 6.]);
    let f = run("bar([2 3]);");
    assert_eq!(f.bounds().1, [0., 3.]);
    assert!(f.svg().contains("<rect x="));
}
#[test]
fn large_series_parallel_construction_matches_serial() {
    let x = Matrix::from_column_vec(20000, 1, (0..20000).map(|i| i as f64).collect()).unwrap();
    let y =
        Matrix::from_column_vec(20000, 8, (0..160000).map(|i| (i as f64).sin()).collect()).unwrap();
    let run = |n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
            .install(|| Figure::new(&x, &y).unwrap())
    };
    let a = run(1);
    let b = run(8);
    for (a, b) in a.series.iter().zip(&b.series) {
        assert_eq!(a.points, b.points);
    }
}
#[test]
fn editable_multiseries_roundtrip_preserves_styles_bits_and_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multi.mfig");
    let f = run(
        "plot([0 1 2],[NaN 2 Inf],'r-.');hold on;scatter([3 4],[5 6]);bar([8 9],[2 -1]);legend('A <&>','scatter','bars');axis([-1 10 -2 8]);title('Multiple series');grid off;",
    );
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.svg(), f.svg());
    assert!(loaded.hold);
    assert!(!loaded.xgrid && !loaded.ygrid);
    for (a, b) in f.series.iter().zip(&loaded.series) {
        assert_eq!(a.label, b.label);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.pattern, b.pattern);
        for (x, y) in a.points.iter().flatten().zip(b.points.iter().flatten()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }
    let csv = dir.path().join("multi.csv");
    files::write_figure_csv(&csv, &loaded).unwrap();
    let text = std::fs::read_to_string(csv).unwrap();
    assert!(text.starts_with("series,x,y\n"));
    assert_eq!(text.lines().count(), 8);
}
#[test]
fn legacy_figure_loads_and_inconsistent_new_metadata_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.mfig");
    let json = br#"{"title":"old","xlabel":"x","ylabel":"y","count":1}"#;
    let mut bytes = b"MATRFIG1".to_vec();
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(json);
    bytes.extend_from_slice(&1f64.to_le_bytes());
    bytes.extend_from_slice(&2f64.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let f = files::load_figure(&path).unwrap();
    assert_eq!(f.series[0].points.get(0).unwrap(), [1., 2.]);
    bytes[..8].copy_from_slice(b"MATRFIG2");
    std::fs::write(&path, &bytes).unwrap();
    assert!(files::load_figure(&path).is_err());
    let mut f = f;
    f.series[0].bar_width = f64::NAN;
    assert!(files::save_figure(&path, &f).is_err());
}
#[test]
fn all_bounds_and_svg_geometry_remain_finite_with_extreme_data() {
    let x = Matrix::from_column_slice(1, 3, &[-f64::MAX, 0., f64::MAX]).unwrap();
    let f = Figure::new(&x, &x).unwrap();
    let svg = f.svg();
    assert!(!svg.contains("NaN") && !svg.contains("inf"));
    let tiny = Matrix::from_column_slice(1, 2, &[1e-310, 2e-310]).unwrap();
    let svg = Figure::new(&tiny, &tiny).unwrap().svg();
    assert!(svg.contains("e-310"));
    assert!(svg.contains("M70.00,490.00 L920.00,60.00"));
    let mut f = f;
    f.xlim = Some([-1., 1.]);
    f.ylim = Some([-1., 1.]);
    let svg = f.svg();
    assert!(!svg.contains("NaN") && !svg.contains("inf"));
}
#[cfg(feature = "plot-export")]
#[test]
fn multiseries_exports_decode_with_both_colors_and_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let f = run(
        "plot([0 1 2],[0 2 0],'r--o');hold on;plot([0 1 2],[2 0 2],'b:');bar([0.5 1.5],[0.3 0.4]);legend('red','blue','bars');",
    );
    for ext in ["png", "jpg", "bmp", "tiff", "svg", "pdf"] {
        let path = dir.path().join(format!("figure.{ext}"));
        matrust::export::export_figure(&f, &path, 960, 560).unwrap();
        if !matches!(ext, "svg" | "pdf") {
            let img = image::open(path).unwrap().to_rgb8();
            for channel in [0, 2] {
                assert!(
                    img.pixels()
                        .filter(|p| p.0[channel] > 150 && p.0[1] < 100)
                        .count()
                        > 100
                );
            }
        }
    }
}
