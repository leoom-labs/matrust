use matrust::{Matrix, plot::Figure};
use std::{hint::black_box, time::Instant};
fn main() {
    println!("kernel,points,series,threads,repetitions,ms_per_iteration");
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            for rows in [1000, 100000] {
                let x = Matrix::from_column_vec(rows, 1, (0..rows).map(|i| i as f64).collect())
                    .unwrap();
                let y = Matrix::from_column_vec(
                    rows,
                    8,
                    (0..rows * 8).map(|i| (i as f64 * 0.01).sin()).collect(),
                )
                .unwrap();
                let figure = Figure::new(&x, &y).unwrap();
                let mut log_figure = figure.clone();
                log_figure.xscale = matrust::plot::AxisScale::Log;
                let mut hidden = figure.clone();
                for s in &mut hidden.series {
                    s.visible = false;
                }
                for kind in [
                    "build",
                    "display",
                    "log_display",
                    "hidden_view",
                    "share",
                    "svg",
                ] {
                    if kind == "svg" && rows > 1000 {
                        continue;
                    }
                    let run = || match kind {
                        "build" => {
                            black_box(Figure::new(black_box(&x), black_box(&y)).unwrap());
                        }
                        "display" => {
                            black_box(
                                figure
                                    .series
                                    .iter()
                                    .map(|s| s.display_points(1024))
                                    .collect::<Vec<_>>(),
                            );
                        }
                        "log_display" => {
                            let axes = log_figure.axis_transforms();
                            black_box(
                                log_figure
                                    .series
                                    .iter()
                                    .map(|s| s.display_points_axes(1024, axes))
                                    .collect::<Vec<_>>(),
                            );
                        }
                        "svg" => {
                            black_box(figure.svg());
                        }
                        "hidden_view" => {
                            black_box(hidden.bounds());
                            black_box(hidden.axis_transforms());
                            black_box(
                                hidden
                                    .series
                                    .iter()
                                    .map(|s| s.display_points(1024))
                                    .collect::<Vec<_>>(),
                            );
                        }
                        _ => {
                            black_box(figure.clone());
                        }
                    };
                    run();
                    let start = Instant::now();
                    for _ in 0..20 {
                        run();
                    }
                    println!(
                        "{kind},{rows},8,{threads},20,{:.4}",
                        start.elapsed().as_secs_f64() * 50.
                    );
                }
            }
        });
    }
}
