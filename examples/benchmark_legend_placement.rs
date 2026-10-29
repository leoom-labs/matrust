//! Serial bounded layout work inside one/eight configured Rayon workers.
//! Source-only proposal: no measured threshold or MATLAB speedup claim.
use matrust::plot::{
    Axes, AxisTransform, LegendBestCache, LegendBestView, LegendRect, PlotKind, Series,
    best_legend_location, outer_allocations,
};
use std::{hint::black_box, time::Instant};

fn main() {
    if std::env::args().any(|a| a == "--metadata") {
        println!(
            "{{\"harness\":\"legend-placement\",\"repetitions\":7,\"warmups\":2,\"samples_per_order\":266,\"placement_is_serial\":true}}"
        );
        return;
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut cases: Vec<_> = [1024, 65_536, 1_000_000]
        .into_iter()
        .flat_map(|n| [1, 64].map(|series| (n, series)))
        .collect();
    let mut workers = [1, 8];
    if reverse {
        cases.reverse();
        workers.reverse();
    }
    println!("case,points,series,workers,repetition,iterations,microseconds_per_call,probes");
    for (n, count) in cases {
        let mut axes = Axes::default();
        for j in 0..count {
            let length = n / count;
            let mut s = Series::new(
                (0..length)
                    .map(|i| {
                        let x = i as f64 / (length - 1) as f64;
                        [x, (x * 37. + j as f64 * 0.1).sin() * 0.4 + 0.5]
                    })
                    .collect(),
                PlotKind::Line,
            );
            s.label = format!("Signal {j}");
            axes.add(vec![s]).unwrap();
        }
        let view = LegendBestView {
            bounds: [0., 1., 0., 1.],
            transforms: [AxisTransform::default(); 2],
            frame: LegendRect::new([0., 0., 850., 430.]),
            extent: [220., 100.],
        };
        let expected = best_legend_location(&axes, view, None).unwrap();
        for worker_count in workers {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(worker_count)
                .build()
                .unwrap();
            for mode in ["cold_best", "warm_cache", "metadata_refresh"] {
                let iterations = if mode == "warm_cache" { 2048 } else { 16 };
                let mut cache = LegendBestCache::default();
                cache.resolve(&axes, view, false).unwrap();
                for repetition in 0..9 {
                    let elapsed = pool.install(|| {
                        let start = Instant::now();
                        for i in 0..iterations {
                            let result = if mode == "cold_best" {
                                best_legend_location(black_box(&axes), view, None).unwrap()
                            } else {
                                if mode == "metadata_refresh" {
                                    axes.series[0].label = format!("Signal {repetition}/{i}");
                                }
                                cache.resolve(black_box(&axes), view, false).unwrap()
                            };
                            black_box(result);
                        }
                        start.elapsed().as_secs_f64() * 1e6 / iterations as f64
                    });
                    assert_eq!(
                        cache.resolve(&axes, view, false).unwrap().location,
                        expected.location
                    );
                    if repetition >= 2 {
                        println!(
                            "{mode},{n},{count},{worker_count},{},{iterations},{elapsed:.6},{}",
                            repetition - 2,
                            expected.probes
                        );
                    }
                }
            }
        }
    }
    let canvas = LegendRect::new([0., 0., 960., 560.]);
    let requests: Vec<_> = (0..16)
        .map(|i| {
            let x = (i % 4) as f64 * 240. + 30.;
            let y = (i / 4) as f64 * 140. + 20.;
            LegendRect::new([x, y, x + 170., y + 90.])
        })
        .collect();
    for worker_count in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .unwrap();
        for repetition in 0..9 {
            let elapsed = pool.install(|| {
                let start = Instant::now();
                for _ in 0..2048 {
                    black_box(
                        outer_allocations(black_box(&requests), canvas, [70., 60., 40., 70.])
                            .unwrap(),
                    );
                }
                start.elapsed().as_secs_f64() * 1e6 / 2048.
            });
            if repetition >= 2 {
                println!(
                    "allocation16,0,0,{worker_count},{},2048,{elapsed:.6},0",
                    repetition - 2
                );
            }
        }
    }
}
