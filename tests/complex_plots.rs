use matrust::{
    Complex64, ComplexMatrix, Engine, Matrix, files,
    plot::{self, PlotData},
};
use std::sync::atomic::AtomicBool;

fn plotted(source: &str) -> matrust::plot::Figure {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e.figure.unwrap()
}
fn bits(points: &plot::SeriesPoints) -> Vec<[u64; 2]> {
    points
        .iter()
        .map(|p| [p[0].to_bits(), p[1].to_bits()])
        .collect()
}

#[test]
fn single_complex_vectors_matrices_and_zero_imaginary_are_coordinates() {
    for z in ["complex([1 -2 3],[4 5 -6])", "complex([1;-2;3],[4;5;-6])"] {
        let f = plotted(&format!("Z={z};plot(Z,'r--o','LineWidth',2);"));
        assert_eq!(
            f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 4.], [-2., 5.], [3., -6.]]
        );
        assert_eq!(f.series[0].color, [255, 0, 0]);
        assert_eq!(f.series[0].pattern, plot::LinePattern::Dashed);
        assert_eq!(f.series[0].line_width, Some(2.));
        assert!(f.series[0].markers);
    }
    let f = plotted("plot(complex([1 2 3;4 5 6],[7 8 9;10 11 12]));");
    assert_eq!(f.series.len(), 3);
    assert_eq!(
        f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
        &[[1., 7.], [4., 10.]]
    );
    assert_eq!(
        f.series[2].points.iter().collect::<Vec<_>>().as_slice(),
        &[[3., 9.], [6., 12.]]
    );
    assert_eq!(
        plotted("plot(complex([3 7]));").series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        &[[3., 0.], [7., 0.]]
    );
    assert_eq!(
        plotted("plot(complex(2,-3));").series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        &[[2., -3.]]
    );
    for (shape, count) in [("0,0", 0), ("0,3", 3), ("1,0", 1), ("0,1", 1)] {
        let f = plotted(&format!("plot(complex(zeros({shape})));"));
        assert_eq!(f.series.len(), count);
        assert_eq!(f.point_count(), 0);
    }
    for source in [
        "plot(zeros(0,1),complex(zeros(0,3)));",
        "plot(complex(zeros(0,3)),zeros(0,1));",
        "plot(complex(zeros(3,0)),zeros(1,0));",
    ] {
        let f = plotted(source);
        assert_eq!(f.series.len(), 3);
        assert_eq!(f.point_count(), 0);
    }
}

#[test]
fn explicit_coordinates_project_real_parts_and_keep_matrix_orientation() {
    let cases = [
        (
            "plot(complex([1 2],[30 40]),complex([5 6],[70 80]));",
            "plot([1 2],[5 6]);",
        ),
        (
            "plot([10 20],complex([1 2;3 4;5 6],[9 9;9 9;9 9]));",
            "plot([10 20],[1 2;3 4;5 6]);",
        ),
        (
            "plot(complex([1 2;3 4;5 6],9),[10 20 30]);",
            "plot([1 2;3 4;5 6],[10 20 30]);",
        ),
        (
            "plot(complex([1 2;3 4;5 6],9),[10 20]);",
            "plot([1 2;3 4;5 6],[10 20]);",
        ),
        (
            "plot(complex([1 2;3 4],9),complex([5 6;7 8],-9));",
            "plot([1 2;3 4],[5 6;7 8]);",
        ),
        (
            "plot(complex([1 2],3),logical([0 1]),'r-',logical([1 0]),complex([7 8],4),'b:');",
            "plot([1 2],[0 1],'r-',[1 0],[7 8],'b:');",
        ),
    ];
    for (source, real) in cases {
        let a = plotted(source);
        let b = plotted(real);
        assert_eq!(a.series.len(), b.series.len(), "{source}");
        for (a, b) in a.series.iter().zip(&b.series) {
            assert_eq!(bits(&a.points), bits(&b.points), "{source}");
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.pattern, b.pattern);
            assert_eq!(a.color, b.color);
        }
    }
}

#[test]
fn log_variants_use_same_coordinates_and_existing_scale_rules() {
    for (name, x, y) in [
        ("plot", plot::AxisScale::Linear, plot::AxisScale::Linear),
        ("semilogx", plot::AxisScale::Log, plot::AxisScale::Linear),
        ("semilogy", plot::AxisScale::Linear, plot::AxisScale::Log),
        ("loglog", plot::AxisScale::Log, plot::AxisScale::Log),
    ] {
        let f = plotted(&format!("{name}(complex([1 10],[2 20]));"));
        assert_eq!(
            f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 2.], [10., 20.]]
        );
        assert_eq!(f.xscale, x);
        assert_eq!(f.yscale, y);
        let explicit = plotted(&format!("{name}(complex([1 10],99),complex([2 20],88));"));
        assert_eq!(explicit.series[0].points, f.series[0].points);
    }
}

#[test]
fn borrowed_components_preserve_bits_shared_inputs_and_cancel_before_allocation() {
    let cancel = AtomicBool::new(false);
    let values = vec![
        Complex64::new(-0., 0.),
        Complex64::new(f64::from_bits(0x7ff8_0000_0000_1234), 2.),
        Complex64::new(3., f64::INFINITY),
        Complex64::new(4., f64::NEG_INFINITY),
    ];
    let z = ComplexMatrix::from_column_vec(4, 1, values.clone(), &cancel).unwrap();
    let pointer = z.data().as_ptr();
    let alias = z.clone();
    let f = plot::series_from_plot_data(None, PlotData::Complex(&z), &cancel).unwrap();
    let expected: Vec<_> = values
        .iter()
        .map(|z| [z.re.to_bits(), z.im.to_bits()])
        .collect();
    assert_eq!(bits(&f[0].points), expected);
    assert_eq!(z.data().as_ptr(), pointer);
    assert_eq!(alias.data().as_ptr(), pointer);
    assert!(
        plot::series_from_plot_data(None, PlotData::Complex(&z), &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
    let x = Matrix::from_column_vec(4, 1, vec![1., 2., 3., 4.]).unwrap();
    let p = plot::series_from_plot_data(Some(PlotData::Complex(&z)), PlotData::Real(&x), &cancel)
        .unwrap();
    assert_eq!(p[0].points.get(0).unwrap()[0].to_bits(), (-0_f64).to_bits());
    assert_eq!(
        p[0].points.get(1).unwrap()[0].to_bits(),
        values[1].re.to_bits()
    );
}

#[test]
fn invalid_complex_calls_leave_figure_and_workspace_unchanged() {
    let mut e = Engine::default();
    e.execute("Z=complex([1 2],[3 4]);plot([0 1],[2 3]);hold on;")
        .unwrap();
    let figure = e.figure.clone().unwrap();
    let value = e.array("Z").unwrap();
    for source in [
        "plot(Z,[1 2 3]);",
        "plot(Z,'r-',Z,'b-');",
        "plot(Z,Z,Z);",
        "plot(Z,'LineWidth',complex(2));",
        "scatter(Z,Z);",
        "bar(Z);",
        "stairs(Z);",
        "stem(Z);",
        "plot(complex(zeros(2,65)));",
        "plot(complex(zeros(1000001,1)));",
        "line(Z);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(
            e.figure.as_ref().unwrap().same_snapshot(&figure),
            "{source}"
        );
        assert!(e.array("Z").unwrap().same_snapshot(&value));
    }
    e.execute("plot(Z,'b-');").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 2);
    assert!((e.figure.as_ref().unwrap().series[0].points).same_storage(&figure.series[0].points));
}

#[test]
fn complex_derived_figures_roundtrip_and_matlab_script_preserves_coordinates() {
    let f =
        plotted("plot(complex([1 2 3],[4 5 6]),'r--o','LineWidth',2);title('Complex trajectory');");
    let dir = tempfile::tempdir().unwrap();
    for extension in ["mfig", "fig"] {
        let path = dir.path().join(format!("trajectory.{extension}"));
        files::save_figure(&path, &f).unwrap();
        let restored = files::load_figure(&path).unwrap();
        assert_eq!(restored.series[0].points, f.series[0].points);
        assert_eq!(restored.svg(), f.svg());
    }
    let source = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
    // This exporter targets MATLAB handle graphics, which the MatRust
    // interpreter does not support yet. Verify its actual data contract here;
    // the editable figure formats above are exercised through their readers.
    let data = source
        .split("mr_data = [\n")
        .nth(1)
        .unwrap()
        .split("];\n")
        .next()
        .unwrap();
    let coordinates: Vec<f64> = data
        .split_whitespace()
        .map(|value| value.trim_end_matches(';').parse().unwrap())
        .collect();
    assert_eq!(
        coordinates
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        f.series[0]
            .points
            .iter()
            .flatten()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
    assert!(source.contains("plot(mr_ax"));
    assert!(source.contains("'LineStyle','--'"));
    assert!(source.contains("'LineWidth',2.00000000000000000e0"));
    assert!(source.contains("'Complex trajectory'"));
}

#[test]
fn large_complex_columns_are_worker_invariant_and_do_not_copy_source() {
    let cancel = AtomicBool::new(false);
    let z = ComplexMatrix::from_column_vec(
        12501,
        8,
        (0..100008)
            .map(|i| Complex64::new(i as f64, -(i as f64)))
            .collect(),
        &cancel,
    )
    .unwrap();
    let pointer = z.data().as_ptr();
    let run = |workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| plot::series_from_plot_data(None, PlotData::Complex(&z), &cancel).unwrap())
    };
    let a = run(1);
    let b = run(8);
    assert_eq!(a.len(), 8);
    for (column, (a, b)) in a.iter().zip(&b).enumerate() {
        assert_eq!(a.points, b.points);
        assert_eq!(
            a.points.get(12500).unwrap(),
            [
                ((column + 1) * 12501 - 1) as f64,
                -(((column + 1) * 12501 - 1) as f64)
            ]
        );
    }
    assert_eq!(z.data().as_ptr(), pointer);
}
