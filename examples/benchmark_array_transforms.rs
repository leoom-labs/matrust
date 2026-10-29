//! Warm in-process release kernel timings; correctness checks are outside timing.
//! Run twice, once with --reverse. Record machine/compiler and both raw CSV files.
use matrust::Matrix;
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn execute(name: &str, a: &Matrix, flag: &AtomicBool) -> Matrix {
    match name {
        "flip1" => a.flip_dimension(1, flag),
        "flip2" => a.flip_dimension(2, flag),
        "tril" => a.triangular(0, false, flag),
        "triu" => a.triangular(0, true, flag),
        "diag" => a.diagonal(0, flag),
        "cumsum1" => a.cumulative(1, false, flag),
        "cumsum2" => a.cumulative(2, false, flag),
        "cumprod1" => a.cumulative(1, true, flag),
        "cumprod2" => a.cumulative(2, true, flag),
        _ => unreachable!(),
    }
    .unwrap()
}

fn main() {
    let reverse = std::env::args().any(|arg| arg == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!(
        "case,rows,cols,elements,workers,operation,repetitions,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let c = AtomicBool::new(false);
            for (case, rows, cols) in [
                ("matrix", 16, 16),
                ("matrix", 256, 64),
                ("matrix", 256, 256),
                ("matrix", 256, 512),
                ("matrix", 256, 1024),
                ("matrix", 256, 2048),
                ("matrix", 1024, 1024),
                ("matrix", 4096, 1024),
                ("row", 1, 1024 * 1024),
                ("column", 1024 * 1024, 1),
                ("diag_build", 1, 8),
                ("diag_build", 1, 256),
                ("diag_build", 1, 1024),
                ("diag_build", 1, 4096),
            ] {
                let input: Vec<_> = (0..rows * cols)
                    .map(|i| 1. + (i % 23) as f64 / 1000000.)
                    .collect();
                let a = Matrix::from_column_vec(rows, cols, input).unwrap();
                let mut operations = if case == "diag_build" {
                    vec!["diag"]
                } else {
                    let mut ops = vec![
                        "flip1", "flip2", "tril", "triu", "cumsum1", "cumsum2", "cumprod1",
                        "cumprod2",
                    ];
                    if rows > 1 && cols > 1 {
                        ops.push("diag");
                    }
                    ops
                };
                if reverse {
                    operations.reverse();
                }
                let expected: Vec<_> = operations
                    .iter()
                    .map(|name| execute(name, &a, &c))
                    .collect();
                let mut samples: Vec<Vec<f64>> = vec![Vec::new(); operations.len()];
                for round in 0..9 {
                    for offset in 0..operations.len() {
                        let index = (round + offset) % operations.len();
                        let start = Instant::now();
                        let result = black_box(execute(operations[index], black_box(&a), &c));
                        let elapsed = start.elapsed().as_secs_f64() * 1000.;
                        if round >= 2 {
                            samples[index].push(elapsed);
                        }
                        assert_eq!(
                            (result.rows(), result.cols()),
                            (expected[index].rows(), expected[index].cols())
                        );
                        assert!(
                            result
                                .data()
                                .iter()
                                .zip(expected[index].data())
                                .all(|(&a, &b)| a.to_bits() == b.to_bits())
                        );
                        black_box(result);
                    }
                }
                for (name, samples) in operations.iter().zip(samples) {
                    let mut sorted = samples.clone();
                    sorted.sort_by(f64::total_cmp);
                    println!(
                        "{case},{rows},{cols},{},{workers},{name},7,{:.6},{:.6},{:.6},\"{:?}\"",
                        rows * cols,
                        sorted[3],
                        sorted[0],
                        sorted[6],
                        samples
                    );
                }
            }
        });
    }
}
