//! Pool/input setup, validation and destruction untimed. Allocation and solve timed.
use matrust::{Matrix, dsp};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("release benchmark required");
    }
    let reverse = std::env::args().any(|v| v == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!("case,samples,channels,workers,repetitions,median_us,min_us,max_us,samples_us");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let c = AtomicBool::new(false);
            let mut cases = vec![
                ("gain", vec![0.5], vec![1.]),
                ("fir8", vec![1. / 9.; 9], vec![1.]),
                ("fir64", vec![1. / 65.; 65], vec![1.]),
                ("iir2", vec![0.2, 0.1, 0.2], vec![1., -0.4, 0.1]),
            ];
            if reverse {
                cases.reverse();
            }
            for (label, b, a) in cases {
                for (samples, channels) in [
                    (1024, 1),
                    (16384, 1),
                    (65536, 1),
                    (1024, 8),
                    (16384, 8),
                    (65536, 8),
                ] {
                    let x = Matrix::from_column_vec(
                        samples,
                        channels,
                        (0..samples * channels)
                            .map(|i| (i as f64 / 11.).sin())
                            .collect(),
                    )
                    .unwrap();
                    let reference = dsp::filtfilt(&b, &a, &x, &c).unwrap();
                    let mut times = Vec::new();
                    for round in 0..9 {
                        let start = Instant::now();
                        let y = black_box(
                            dsp::filtfilt(black_box(&b), black_box(&a), black_box(&x), &c).unwrap(),
                        );
                        let us = start.elapsed().as_secs_f64() * 1e6;
                        assert!(
                            y.data()
                                .iter()
                                .zip(reference.data())
                                .all(|(a, b)| a.to_bits() == b.to_bits())
                        );
                        if round >= 2 {
                            times.push(us);
                        }
                    }
                    let mut sorted = times.clone();
                    sorted.sort_by(f64::total_cmp);
                    println!(
                        "{label},{samples},{channels},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",
                        sorted[3], sorted[0], sorted[6], times
                    );
                }
            }
        });
    }
}
