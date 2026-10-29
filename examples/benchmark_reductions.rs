//! Allocation-inclusive warmed release timings for sum/mean, min/max with
//! indices, any and find along dimensions 1 and 2; no MATLAB comparison.
//! Prints CSV: workers,rows,cols,operation,median_ms,min_ms (7 timed rounds).
//! Usage: cargo run --release --example benchmark_reductions [-- --large-only]
use matrust::{
    Matrix,
    logical::ArrayRef,
    matrix::{Extreme, find_nonzero, truth_along},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn run(name: &str, a: &Matrix, c: &AtomicBool) -> usize {
    let dim = |n: &str| Some(if n.ends_with('2') { 2 } else { 1 });
    match name {
        "sum1" | "sum2" => a.sum_along(dim(name), false, c).unwrap().len(),
        "mean2" => a.sum_along(dim(name), true, c).unwrap().len(),
        "max1" | "max2" => a
            .extreme_along(Extreme::Max, dim(name), true, c)
            .unwrap()
            .0
            .len(),
        "any1" | "any2" => truth_along(ArrayRef::Numeric(a), false, dim(name), c)
            .unwrap()
            .len(),
        "find" => find_nonzero(ArrayRef::Numeric(a), None, false, c)
            .unwrap()
            .len(),
        "old_sum1" => a.reduce(false).unwrap().len(),
        _ => unreachable!(),
    }
}

fn main() {
    let large_only = std::env::args().any(|x| x == "--large-only");
    let ops = [
        "sum1", "sum2", "mean2", "max1", "max2", "any1", "any2", "find", "old_sum1",
    ];
    let sizes: &[(usize, usize)] = if large_only {
        &[(4000, 4000)]
    } else {
        &[
            (128, 128),
            (256, 256),
            (362, 362),
            (512, 512),
            (724, 724),
            (1024, 1024),
            (4000, 4000),
        ]
    };
    eprintln!(
        "os={} arch={} available_parallelism={:?}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism()
    );
    println!("workers,rows,cols,operation,median_ms,min_ms");
    let c = AtomicBool::new(false);
    for &(rows, cols) in sizes {
        // Every third element is zero so find and any do real work.
        let data = (0..rows * cols)
            .map(|i| {
                if i % 3 == 0 {
                    0.
                } else {
                    ((i * 7919) % 1013) as f64 - 500.
                }
            })
            .collect();
        let a = Matrix::from_column_vec(rows, cols, data).unwrap();
        for workers in [1, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| {
                for name in ops {
                    let rounds = if rows * cols >= 1 << 20 { 7 } else { 31 };
                    black_box(run(name, &a, &c));
                    let mut times: Vec<f64> = (0..rounds)
                        .map(|_| {
                            let started = Instant::now();
                            black_box(run(name, black_box(&a), &c));
                            started.elapsed().as_secs_f64() * 1000.
                        })
                        .collect();
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{workers},{rows},{cols},{name},{:.4},{:.4}",
                        times[rounds / 2],
                        times[0]
                    );
                }
            });
        }
    }
}
