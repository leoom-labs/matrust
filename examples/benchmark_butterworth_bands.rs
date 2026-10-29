//! Release only. Design remains serial; filtering reuses existing shared pools.
use matrust::{
    Matrix,
    dsp::{self, Band},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("release benchmark required")
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    let mut orders = [1, 4, 10, 20];
    let mut bands = [Band::Bandpass, Band::Bandstop];
    if reverse {
        workers.reverse();
        orders.reverse();
        bands.reverse();
    }
    println!(
        "operation,band,order,samples,channels,workers,repetitions,median_us,min_us,max_us,samples_us"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            for order in orders {
                for band in bands {
                    let expected =
                        dsp::butter_sos_with_cutoffs(order, &[0.2, 0.4], band, &cancel).unwrap();
                    let mut values = Vec::new();
                    for round in 0..9 {
                        let start = Instant::now();
                        let result = black_box(
                            dsp::butter_sos_with_cutoffs(
                                black_box(order),
                                black_box(&[0.2, 0.4]),
                                band,
                                &cancel,
                            )
                            .unwrap(),
                        );
                        let elapsed = start.elapsed().as_secs_f64() * 1e6;
                        assert_eq!(result.data(), expected.data());
                        if round >= 2 {
                            values.push(elapsed);
                        }
                    }
                    report("design", band, order, 0, 0, workers, &values);
                    for (samples, channels) in [(1024, 1), (65536, 1), (1024, 8), (65536, 8)] {
                        let input = Matrix::from_column_vec(
                            samples,
                            channels,
                            (0..samples * channels)
                                .map(|i| ((i % 97) as f64 - 48.) / 48.)
                                .collect(),
                        )
                        .unwrap();
                        let reference = dsp::sosfilt(&expected, &input, &cancel).unwrap();
                        let mut values = Vec::new();
                        for round in 0..9 {
                            let start = Instant::now();
                            let result = black_box(
                                dsp::sosfilt(black_box(&expected), black_box(&input), &cancel)
                                    .unwrap(),
                            );
                            let elapsed = start.elapsed().as_secs_f64() * 1e6;
                            assert_eq!(result.data(), reference.data());
                            if round >= 2 {
                                values.push(elapsed);
                            }
                        }
                        report("sosfilt", band, order, samples, channels, workers, &values);
                    }
                }
            }
        });
    }
}
fn report(
    op: &str,
    band: Band,
    order: usize,
    samples: usize,
    channels: usize,
    workers: usize,
    times: &[f64],
) {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    println!(
        "{op},{},{order},{samples},{channels},{workers},7,{:.9},{:.9},{:.9},\"{:?}\"",
        band.name(),
        sorted[3],
        sorted[0],
        sorted[6],
        times
    );
}
