//! Run with `cargo run --release --example benchmark_numeric_followups`.
//! Engine-level timings include parsing, dispatch and output publication.
use matrust::{Engine, Matrix};
use std::{hint::black_box, time::Instant};

fn measure(engine: &mut Engine, label: &str, workers: usize, n: usize, source: &str) {
    for _ in 0..2 {
        engine.execute(source).unwrap();
    }
    let mut times = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        black_box(engine.execute(black_box(source)).unwrap());
        times.push(start.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{label},{workers},{n},9,{:.3},{:.3},{:.3}",
        times[4], times[0], times[8]
    );
}

fn main() {
    println!("case,workers,size,repetitions,median_us,min_us,max_us");
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                let mut e = Engine::default();
                for n in [2, 1024, 100_000] {
                    e.variables.insert(
                        "x".into(),
                        Matrix::from_column_vec(1, n, vec![1e200; n]).unwrap(),
                    );
                    for (label, source) in [
                        ("norm_p3", "r=norm(x,3);"),
                        ("norm_two", "r=norm(x);"),
                        ("norm_fro", "r=norm(x,'fro');"),
                    ] {
                        measure(&mut e, label, workers, n, source);
                    }
                }
                for n in [32, 256, 512, 1024, 4096] {
                    e.variables.insert(
                        "x".into(),
                        Matrix::from_column_vec(
                            1,
                            n,
                            (0..n).map(|i| (i as f64 * 0.1).sin()).collect(),
                        )
                        .unwrap(),
                    );
                    measure(&mut e, "xcorr_full", workers, n, "r=xcorr(x);");
                    measure(&mut e, "xcorr_lag8", workers, n, "r=xcorr(x,8);");
                }
                e.variables.insert(
                    "x".into(),
                    Matrix::from_column_vec(
                        1,
                        100_000,
                        (0..100_000).map(|i| (i as f64 * 0.1).sin()).collect(),
                    )
                    .unwrap(),
                );
                measure(&mut e, "xcorr_lag8", workers, 100_000, "r=xcorr(x,8);");
                measure(
                    &mut e,
                    "ode45_handle",
                    workers,
                    1,
                    "[t,y]=ode45(@(t,y) -3*y,[0 1],1);",
                );
                measure(
                    &mut e,
                    "ode23_handle",
                    workers,
                    1,
                    "[t,y]=ode23(@(t,y) -3*y,[0 1],1);",
                );
            });
    }
}
