//! Exploratory scheduling comparison; final regression benchmark is benches/elementwise.rs.
use matrust::Matrix;
use rayon::prelude::*;
use std::{hint::black_box, time::Instant};

fn candidate(input: &Matrix, parallel: bool, f: impl Fn(f64) -> f64 + Send + Sync) -> Matrix {
    let data = if parallel {
        let mut output = vec![0.; input.len()];
        output
            .par_chunks_mut(8192)
            .zip(input.data().par_chunks(8192))
            .for_each(|(out, input)| {
                for (out, &x) in out.iter_mut().zip(input) {
                    *out = f(x);
                }
            });
        output
    } else {
        input.data().iter().map(|&x| f(x)).collect()
    };
    Matrix::from_column_vec(input.rows(), input.cols(), data).unwrap()
}
fn main() {
    println!("operation,elements,threads,previous_ms,serial_ms,chunked_ms");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for size in [65536, 1000000, 8000000] {
                    let input = Matrix::from_column_vec(
                        1,
                        size,
                        (0..size).map(|i| i as f64 * 0.001).collect(),
                    )
                    .unwrap();
                    for op in ["sqrt", "abs", "floor", "ceil", "round", "log"] {
                        let run = |path| {
                            macro_rules! kernel {
                                ($f:expr) => {
                                    match path {
                                        0 => input.map($f),
                                        1 => candidate(&input, false, $f),
                                        _ => candidate(&input, true, $f),
                                    }
                                };
                            }
                            black_box(match op {
                                "sqrt" => kernel!(f64::sqrt),
                                "abs" => kernel!(f64::abs),
                                "floor" => kernel!(f64::floor),
                                "ceil" => kernel!(f64::ceil),
                                "round" => kernel!(f64::round),
                                _ => kernel!(libm::log),
                            })
                        };
                        for path in 0..3 {
                            drop(run(path));
                        }
                        let mut times = [Vec::new(), Vec::new(), Vec::new()];
                        for rep in 0..15 {
                            for offset in 0..3 {
                                let path = (rep + offset) % 3;
                                let start = Instant::now();
                                let output = run(path);
                                times[path].push(start.elapsed().as_secs_f64() * 1000.);
                                drop(output);
                            }
                        }
                        for t in &mut times {
                            t.sort_by(f64::total_cmp);
                        }
                        println!(
                            "{op},{size},{threads},{:.6},{:.6},{:.6}",
                            times[0][7], times[1][7], times[2][7]
                        );
                    }
                }
            });
    }
}
