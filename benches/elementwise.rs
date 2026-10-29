use matrust::Matrix;
use std::{hint::black_box, time::Instant};

type Operation = (&'static str, fn(f64) -> f64, fn(&Matrix) -> Matrix);
fn main() {
    let ops: [Operation; 8] = [
        ("tan", f64::tan, Matrix::tan),
        ("exp", f64::exp, Matrix::exp),
        ("log", f64::ln, Matrix::ln),
        ("sqrt", f64::sqrt, Matrix::sqrt),
        ("abs", f64::abs, Matrix::abs),
        ("floor", f64::floor, Matrix::floor),
        ("ceil", f64::ceil, Matrix::ceil),
        ("round", f64::round, Matrix::round),
    ];
    println!(
        "operation,elements,threads,repetitions,previous_median_ms,current_median_ms,speedup,previous_min_ms,previous_max_ms,current_min_ms,current_max_ms"
    );
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for size in [1, 32, 1024, 65536, 1000000] {
                    for (name, old, new) in ops {
                        let input = Matrix::from_column_vec(
                            1,
                            size,
                            (0..size)
                                .map(|i| {
                                    let x = (i % 10001) as f64 / 10000.;
                                    if name == "log" || name == "sqrt" {
                                        0.01 + x * 100.
                                    } else {
                                        (x - 0.5) * 20.
                                    }
                                })
                                .collect(),
                        )
                        .unwrap();
                        // Match the previous engine's runtime function pointer, excluding dispatch/parsing.
                        let old = black_box(old);
                        let run = |path| {
                            if path == 0 {
                                black_box(input.map(old))
                            } else {
                                black_box(new(&input))
                            }
                        };
                        let a = run(0);
                        let b = run(1);
                        for (&x, &y) in a.data().iter().zip(b.data()) {
                            assert!(
                                (x - y).abs() <= 4e-14 * x.abs().max(1.),
                                "{name}: {x} vs {y}"
                            );
                        }
                        drop(a);
                        drop(b);
                        let mut times = [Vec::new(), Vec::new()];
                        for iteration in 0..30 {
                            for path in if iteration % 2 == 0 { [0, 1] } else { [1, 0] } {
                                let start = Instant::now();
                                let output = run(path);
                                times[path].push(start.elapsed().as_secs_f64() * 1000.);
                                drop(output);
                            }
                        }
                        for t in &mut times {
                            t.sort_by(f64::total_cmp);
                        }
                        let median = times.each_ref().map(|t| (t[14] + t[15]) * 0.5);
                        println!(
                            "{name},{size},{threads},30,{:.6},{:.6},{:.3},{:.6},{:.6},{:.6},{:.6}",
                            median[0],
                            median[1],
                            median[0] / median[1],
                            times[0][0],
                            times[0][29],
                            times[1][0],
                            times[1][29]
                        );
                    }
                }
            });
    }
}
