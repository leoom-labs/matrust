//! Draft release-only measurement. Input/pool setup and output checking untimed.
use matrust::{Matrix, dsp};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("release benchmark required");
    }
    let reverse = std::env::args().any(|v| v == "--reverse");
    let mut workers = [1, 8];
    let mut orders = [4, 8, 20];
    if reverse {
        workers.reverse();
        orders.reverse();
    }
    println!("order,samples,channels,workers,repetitions,median_us,min_us,max_us,samples_us");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            for order in orders {
                let coefficients = dsp::butter_sos(order, 0.2, dsp::Band::Lowpass).unwrap();
                for (samples, channels) in [
                    (1024, 1),
                    (16384, 1),
                    (65536, 1),
                    (1024, 8),
                    (16384, 8),
                    (65536, 8),
                ] {
                    let input = Matrix::from_column_vec(
                        samples,
                        channels,
                        (0..samples * channels)
                            .map(|i| (i as f64 / 11.).sin())
                            .collect(),
                    )
                    .unwrap();
                    let reference =
                        dsp::filtfilt_sos(&coefficients, &[1.], &input, &cancel).unwrap();
                    let mut times = Vec::new();
                    for round in 0..9 {
                        let start = Instant::now();
                        let output = black_box(
                            dsp::filtfilt_sos(
                                black_box(&coefficients),
                                &[1.],
                                black_box(&input),
                                &cancel,
                            )
                            .unwrap(),
                        );
                        let us = start.elapsed().as_secs_f64() * 1e6;
                        assert!(
                            output
                                .data()
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
                        "{order},{samples},{channels},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",
                        sorted[3], sorted[0], sorted[6], times
                    );
                }
            }
        });
    }
}
