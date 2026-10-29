use matrust::plot::{Figure, PlotKind, Series};
use std::{hint::black_box, time::Instant};
fn main() {
    println!("total_points,series,threads,repetitions,ms_per_edit");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for points in [1000, 1_000_000] {
                    for series_count in [1, 64] {
                        let mut f = Figure::default();
                        f.add(
                            (0..series_count)
                                .map(|c| {
                                    Series::new(
                                        (0..points / series_count)
                                            .map(|i| [i as f64, (i + c) as f64])
                                            .collect(),
                                        PlotKind::Line,
                                    )
                                })
                                .collect(),
                        )
                        .unwrap();
                        let run = || {
                            let original = black_box(&f).clone();
                            let mut updated = original.clone();
                            updated.title = "Edited figure".into();
                            updated.series[0].label = "Signal".into();
                            updated.series[0].color = [33, 100, 200];
                            assert!(f.same_snapshot(&original));
                            updated.validate().unwrap();
                            assert!(updated.series[0].points.same_storage(&f.series[0].points));
                            black_box(updated);
                        };
                        run();
                        let start = Instant::now();
                        for _ in 0..10000 {
                            run();
                        }
                        println!(
                            "{},{series_count},{threads},10000,{:.8}",
                            f.point_count(),
                            start.elapsed().as_secs_f64() / 10.
                        );
                    }
                }
            });
    }
}
