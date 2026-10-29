use matrust::Matrix;
use std::{hint::black_box, time::Instant};

fn main() {
    println!(
        "operation,elements,domain,threads,repetitions,scalar_median_ms,vector_median_ms,speedup"
    );
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for size in [16, 32, 1024, 65536, 1000000] {
                    for (domain, scale) in [("small", 1.), ("wide", 8192.), ("fallback", 1e100)] {
                        let input = Matrix::from_column_vec(
                            1,
                            size,
                            (0..size)
                                .map(|i| (i as f64 * 0.001).sin() * scale)
                                .collect(),
                        )
                        .unwrap();
                        for cosine in [false, true] {
                            let scalar = if cosine { f64::cos } else { f64::sin };
                            let scalar_run = || black_box(input.map(scalar));
                            let vector_run =
                                || black_box(if cosine { input.cos() } else { input.sin() });
                            drop(scalar_run());
                            drop(vector_run());
                            let mut times = [Vec::new(), Vec::new()];
                            for iteration in 0..30 {
                                for path in if iteration % 2 == 0 { [0, 1] } else { [1, 0] } {
                                    let start = Instant::now();
                                    let value = if path == 0 {
                                        scalar_run()
                                    } else {
                                        vector_run()
                                    };
                                    times[path].push(start.elapsed().as_secs_f64() * 1000.);
                                    drop(value);
                                }
                            }
                            let mut median = [0.; 2];
                            for (i, t) in times.iter_mut().enumerate() {
                                t.sort_by(f64::total_cmp);
                                median[i] = (t[14] + t[15]) * 0.5;
                            }
                            println!(
                                "{},{size},{domain},{threads},30,{:.6},{:.6},{:.3}",
                                if cosine { "cos" } else { "sin" },
                                median[0],
                                median[1],
                                median[0] / median[1]
                            );
                        }
                    }
                }
            });
    }
}
