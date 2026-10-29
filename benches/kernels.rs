use matrust::Matrix;
use std::{hint::black_box, time::Instant};
fn main() {
    println!("kernel,size,threads,iterations,ms_per_iteration");
    for threads in [
        1,
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(8),
    ] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            for n in [64, 256, 512] {
                let a = Matrix::from_column_slice(
                    n,
                    n,
                    &(0..n * n)
                        .map(|i| (i % 97) as f64 / 97.0)
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                black_box(a.mul(&a).unwrap());
                let reps = if n == 512 { 5 } else { 20 };
                let start = Instant::now();
                for _ in 0..reps {
                    black_box(a.mul(black_box(&a)).unwrap());
                }
                println!(
                    "gemm,{n},{threads},{reps},{:.4}",
                    start.elapsed().as_secs_f64() * 1000.0 / reps as f64
                );
            }
            let a = Matrix::filled(1, 1_000_000, 0.5).unwrap();
            black_box(a.with_value(0, 500_000, 1.).unwrap());
            let copy_start = Instant::now();
            for _ in 0..20 {
                black_box(a.with_value(0, 500_000, 1.).unwrap());
            }
            println!(
                "cell_edit,1000000,{threads},20,{:.4}",
                copy_start.elapsed().as_secs_f64() * 50.
            );
            black_box(a.sin());
            let start = Instant::now();
            for _ in 0..20 {
                black_box(a.sin());
            }
            println!(
                "sin,1000000,{threads},20,{:.4}",
                start.elapsed().as_secs_f64() * 50.0
            );
        });
    }
}
