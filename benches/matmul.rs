//! Alternating benchmark of the previous nalgebra column split and current product.
use matrust::Matrix;
use nalgebra::{DMatrix, DMatrixViewMut};
use rayon::prelude::*;
use std::{hint::black_box, time::Instant};
fn baseline(a: &DMatrix<f64>, b: &DMatrix<f64>, threads: usize) -> Matrix {
    let mut out = DMatrix::zeros(a.nrows(), b.ncols());
    if a.nrows() * a.ncols() * b.ncols() >= 8 * 1024 * 1024 && b.ncols() >= 32 && threads > 1 {
        let columns = b.ncols().div_ceil(threads).max(32);
        out.as_mut_slice()
            .par_chunks_mut(a.nrows() * columns)
            .enumerate()
            .for_each(|(i, chunk)| {
                let count = chunk.len() / a.nrows();
                let block = b.columns(i * columns, count);
                DMatrixViewMut::from_slice(chunk, a.nrows(), count).gemm(1., a, &block, 0.);
            });
    } else {
        out.gemm(1., a, b, 0.);
    }
    Matrix::from_column_vec(a.nrows(), b.ncols(), Vec::from(out.data)).unwrap()
}
fn main() {
    println!(
        "rows,inner,cols,threads,repetitions,old_median_ms,new_median_ms,speedup,new_min_ms,new_max_ms,max_abs_difference"
    );
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| run(threads));
    }
}
fn run(threads: usize) {
    for (rows, inner, cols) in [
        (16, 16, 16),
        (32, 32, 32),
        (64, 64, 64),
        (128, 128, 128),
        (256, 256, 256),
        (512, 512, 512),
        (1024, 1024, 1024),
        (257, 129, 511),
        (8192, 32, 64),
        (64, 512, 4096),
        (65536, 16, 1),
        (4096, 1, 512),
        (4096, 8, 256),
        (1024, 32, 512),
    ] {
        let a = Matrix::from_column_vec(
            rows,
            inner,
            (0..rows * inner).map(|i| (i % 97) as f64 / 97.).collect(),
        )
        .unwrap();
        let b = Matrix::from_column_vec(
            inner,
            cols,
            (0..inner * cols).map(|i| (i % 89) as f64 / 89.).collect(),
        )
        .unwrap();
        let old_a = DMatrix::from_column_slice(rows, inner, a.data());
        let old_b = DMatrix::from_column_slice(inner, cols, b.data());
        let reference = baseline(&old_a, &old_b, threads);
        let current = a.mul(&b).unwrap();
        let difference = reference
            .data()
            .iter()
            .zip(current.data())
            .map(|(a, b)| (a - b).abs())
            .fold(0., f64::max);
        assert!(difference <= 32. * f64::EPSILON * inner as f64);
        let mut times = [Vec::new(), Vec::new()];
        for iteration in 0..20 {
            for method in if iteration % 2 == 0 { [0, 1] } else { [1, 0] } {
                let start = Instant::now();
                let result = black_box(if method == 0 {
                    baseline(&old_a, &old_b, threads)
                } else {
                    a.mul(&b).unwrap()
                });
                times[method].push(start.elapsed().as_secs_f64() * 1000.);
                drop(result);
            }
        }
        for time in &mut times {
            time.sort_by(f64::total_cmp);
        }
        let old = (times[0][9] + times[0][10]) * 0.5;
        let new = (times[1][9] + times[1][10]) * 0.5;
        println!(
            "{rows},{inner},{cols},{threads},20,{old:.6},{new:.6},{:.3},{:.6},{:.6},{difference:e}",
            old / new,
            times[1][0],
            times[1][19]
        );
    }
}
