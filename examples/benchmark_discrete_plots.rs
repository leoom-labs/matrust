use matrust::{
    Engine, Matrix,
    plot::{Figure, PlotKind, Series, series_from_matrices},
};
use std::{hint::black_box, time::Instant};

fn measure(mut operation: impl FnMut(), case: &str, kind: PlotKind, size: usize, workers: usize) {
    let mut times = Vec::new();
    for i in 0..14 {
        let start = Instant::now();
        operation();
        if i >= 3 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{case},{kind:?},{size},{workers},11,{:.6},{:.6},{:.6}",
        times[5], times[0], times[10]
    );
}
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,kind,points,workers,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let x = Matrix::from_column_vec(100000, 1, (0..100000).map(|i| i as f64).collect())
                .unwrap();
            let y = Matrix::from_column_vec(
                100000,
                8,
                (0..800000).map(|i| (i % 101) as f64 - 50.).collect(),
            )
            .unwrap();
            for kind in [PlotKind::Line, PlotKind::Stairs, PlotKind::Stem] {
                let series = series_from_matrices(Some(&x), &y, kind).unwrap();
                assert_eq!(series.len(), 8);
                assert_eq!(
                    series[7].points.get(99999).unwrap(),
                    [99999., (799999 % 101) as f64 - 50.]
                );
                measure(
                    || {
                        black_box(series_from_matrices(Some(&x), &y, kind).unwrap());
                    },
                    "construct",
                    kind,
                    800000,
                    workers,
                );
                let sample = Series::new(
                    (0..1000000)
                        .map(|i| {
                            [
                                i as f64,
                                if i == 500000 {
                                    999.
                                } else {
                                    (i % 101) as f64 - 50.
                                },
                            ]
                        })
                        .collect(),
                    kind,
                );
                let display = sample.display_points(8192);
                assert!(display.len() <= 8192 && display.iter().any(|p| p[1] == 999.));
                measure(
                    || {
                        black_box(sample.display_points(8192));
                    },
                    "display",
                    kind,
                    1000000,
                    workers,
                );
                let mut figure = Figure::default();
                figure
                    .add(vec![Series::new(
                        sample
                            .points
                            .range(0..2000)
                            .unwrap()
                            .collect::<Vec<_>>()
                            .to_vec(),
                        kind,
                    )])
                    .unwrap();
                let svg = figure.svg();
                assert!(!svg.contains("NaN"));
                measure(
                    || {
                        black_box(figure.svg());
                    },
                    "svg",
                    kind,
                    2000,
                    workers,
                );
            }
            if workers == 1 {
                let mut engine = Engine::default();
                engine.execute(include_str!("discrete_plots.m")).unwrap();
                let figure = engine.figure.as_ref().unwrap();
                let directory = std::path::Path::new("target/evidence/plots/discrete");
                std::fs::create_dir_all(directory).unwrap();
                matrust::files::save_figure(&directory.join("signals.mfig"), figure).unwrap();
                matrust::files::write_figure_csv(&directory.join("signals.csv"), figure).unwrap();
                std::fs::write(directory.join("signals.svg"), figure.svg()).unwrap();
                #[cfg(feature = "plot-export")]
                {
                    matrust::export::export_figure(
                        figure,
                        &directory.join("signals.png"),
                        1440,
                        840,
                    )
                    .unwrap();
                    std::fs::create_dir_all("output/pdf").unwrap();
                    matrust::export::export_figure(
                        figure,
                        std::path::Path::new("output/pdf/discrete-signals.pdf"),
                        1440,
                        840,
                    )
                    .unwrap();
                }
            }
        });
}
