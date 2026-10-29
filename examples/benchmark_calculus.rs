//! Allocation-inclusive warmed release kernels; no MATLAB runtime comparison.
//! Run once normally and once with --reverse on a quiet machine, preserving CSVs.
use matrust::Matrix;
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn run(name: &str, a: &Matrix, x1: &Matrix, x2: &Matrix, c: &AtomicBool) -> Matrix {
    match name {
        "diff1" => a.difference(1, 1, c),
        "diff2" => a.difference(1, 2, c),
        "diff_order2" => a.difference(2, a.first_nonsingleton_dimension(), c),
        "trapz1" => a.trapezoid(None, Some(1), false, c),
        "trapz2" => a.trapezoid(None, Some(2), false, c),
        "cumtrapz1" => a.trapezoid(None, Some(1), true, c),
        "cumtrapz2" => a.trapezoid(None, Some(2), true, c),
        "trapz_x1" => a.trapezoid(Some(x1), Some(1), false, c),
        "trapz_x2" => a.trapezoid(Some(x2), Some(2), false, c),
        "cumtrapz_x1" => a.trapezoid(Some(x1), Some(1), true, c),
        "cumtrapz_x2" => a.trapezoid(Some(x2), Some(2), true, c),
        _ => unreachable!(),
    }
    .unwrap()
}
fn main() {
    let reverse = std::env::args().any(|x| x == "--reverse");
    let mut counts = [1, 8];
    if reverse {
        counts.reverse();
    }
    let mut ops = vec![
        "diff1",
        "diff2",
        "diff_order2",
        "trapz1",
        "trapz2",
        "cumtrapz1",
        "cumtrapz2",
        "trapz_x1",
        "trapz_x2",
        "cumtrapz_x1",
        "cumtrapz_x2",
    ];
    if reverse {
        ops.reverse();
    }
    eprintln!(
        "os={} arch={} available_parallelism={:?}; record CPU/compiler/build hashes alongside CSV",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism()
    );
    println!("workers,rows,cols,elements,operation,round,milliseconds");
    for workers in counts {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for (rows, cols) in [
            (16, 16),
            (256, 256),
            (256, 1024),
            (256, 2048),
            (1024, 1024),
            (524288, 2),
            (1, 1048576),
            (1048576, 1),
        ] {
            let a = Matrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols)
                    .map(|i| ((i % 101) as f64 - 50.) / 7.)
                    .collect(),
            )
            .unwrap();
            let x1 = Matrix::from_column_vec(
                rows,
                1,
                (0..rows).map(|i| i as f64 + (i % 7) as f64 / 16.).collect(),
            )
            .unwrap();
            let x2 = Matrix::from_column_vec(
                1,
                cols,
                (0..cols).map(|i| i as f64 + (i % 7) as f64 / 16.).collect(),
            )
            .unwrap();
            let c = AtomicBool::new(false);
            pool.install(|| {
                let expected: Vec<_> = ops.iter().map(|name| run(name, &a, &x1, &x2, &c)).collect();
                for round in 0..9 {
                    for offset in 0..ops.len() {
                        let index = (round + offset) % ops.len();
                        let name = ops[index];
                        let started = Instant::now();
                        let result = black_box(run(name, black_box(&a), &x1, &x2, &c));
                        let elapsed = started.elapsed().as_secs_f64() * 1000.;
                        if round >= 2 {
                            println!(
                                "{workers},{rows},{cols},{},{name},{},{elapsed:.9}",
                                a.len(),
                                round - 2
                            );
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
                                .all(|(a, b)| a.to_bits() == b.to_bits())
                        );
                        black_box(result);
                    }
                }
            });
        }
    }
}
