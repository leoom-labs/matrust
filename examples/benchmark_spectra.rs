//! Run: cargo run --release --no-default-features --example benchmark_spectra
//! Pools exist only in this measurement harness, never in the numerical module.
use matrust::{
    Array, Matrix,
    dsp::{
        self,
        spectral::{self, Estimate, Options, Range},
    },
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn measure(mut run: impl FnMut(), repetitions: usize) -> f64 {
    run();
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..repetitions {
            run();
        }
        samples.push(start.elapsed().as_secs_f64() * 1000. / repetitions as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[3]
}
fn main() {
    println!("kernel,samples,columns,window,nfft,workers,repetitions,median_ms_7_batches");
    let cancel = AtomicBool::new(false);
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows, cols, l, nfft) in [
                (100, 1, 8, 83),
                (8192, 8, 256, 997),
                (8192, 8, 256, 2062),
                (256, 1, 64, 256),
                (4096, 8, 256, 256),
                (8192, 8, 256, 256),
                (131072, 8, 1024, 1024),
                (131072, 1, 131072, 131072),
            ] {
                let x: Array = Matrix::from_column_vec(
                    rows,
                    cols,
                    (0..rows * cols)
                        .map(|j| (0.071 * j as f64).sin() + 0.2 * (0.213 * j as f64).cos())
                        .collect(),
                )
                .unwrap()
                .into();
                let o = Options {
                    window: dsp::window(dsp::WindowKind::Hamming, l, false, &cancel)
                        .unwrap()
                        .data()
                        .to_vec(),
                    overlap: l / 2,
                    nfft,
                    fs: Some(1000.),
                    range: Range::OneSided,
                    power: false,
                };
                for kind in [Estimate::Welch, Estimate::Coherence] {
                    let repetitions = if rows > 10000 { 5 } else { 20 };
                    let ms = measure(
                        || {
                            black_box(
                                spectral::estimate(
                                    black_box(&x),
                                    if kind == Estimate::Coherence {
                                        Some(&x)
                                    } else {
                                        None
                                    },
                                    kind,
                                    &o,
                                    256 * 1024 * 1024,
                                    &cancel,
                                )
                                .unwrap(),
                            );
                        },
                        repetitions,
                    );
                    println!("{kind:?},{rows},{cols},{l},{nfft},{workers},{repetitions},{ms:.6}");
                }
            }
            let x: Array = Matrix::from_column_vec(
                65536,
                1,
                (0..65536).map(|j| (j as f64 * 0.03).sin()).collect(),
            )
            .unwrap()
            .into();
            let o = Options {
                window: dsp::window(dsp::WindowKind::Hann, 256, false, &cancel)
                    .unwrap()
                    .data()
                    .to_vec(),
                overlap: 192,
                nfft: 256,
                fs: None,
                range: Range::Centered,
                power: false,
            };
            let s = spectral::short_time(&x, &o, true, false, 256 * 1024 * 1024, &cancel).unwrap();
            for kernel in ["stft", "istft"] {
                let ms = measure(
                    || {
                        if kernel == "stft" {
                            black_box(
                                spectral::short_time(
                                    black_box(&x),
                                    &o,
                                    true,
                                    true,
                                    256 * 1024 * 1024,
                                    &cancel,
                                )
                                .unwrap(),
                            );
                        } else {
                            black_box(
                                spectral::inverse(
                                    black_box(&s.spectrum),
                                    &o,
                                    None,
                                    256 * 1024 * 1024,
                                    &cancel,
                                )
                                .unwrap(),
                            );
                        }
                    },
                    5,
                );
                println!("{kernel},65536,1,256,256,{workers},5,{ms:.6}");
            }
        });
    }
}
