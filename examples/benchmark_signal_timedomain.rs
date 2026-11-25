//! Task-local release benchmark. Run with --no-default-features; creates only
//! explicit local Rayon pools. No hardware-independent speed claims intended.
use matrust::{
    Matrix,
    dsp::{Band, timedomain as td},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn measure<T>(pool: &rayon::ThreadPool, label: &str, reps: usize, f: impl Fn() -> T + Sync) {
    pool.install(|| {
        black_box(f());
        let start = Instant::now();
        for _ in 0..reps {
            black_box(f());
        }
        println!(
            "workers={}, {label}, reps={reps}, mean_ms={:.6}",
            pool.current_num_threads(),
            start.elapsed().as_secs_f64() * 1000. / reps as f64
        );
    });
}
fn main() {
    let c = AtomicBool::new(false);
    println!(
        "time-domain benchmark; one warmup per case; package {}; release {}",
        env!("CARGO_PKG_VERSION"),
        !cfg!(debug_assertions)
    );
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for (rows, cols) in [(4096, 1), (16384, 1), (65536, 1), (8192, 8), (65536, 8)] {
            let x = Matrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols).map(|j| (j as f64 * 0.031).sin()).collect(),
            )
            .unwrap();
            measure(&pool, &format!("resample3/2 {rows}x{cols}"), 10, || {
                td::resample(black_box(&x), 3, 2, 10, 5., &c).unwrap()
            });
        }
        let x = Matrix::from_column_vec(
            32768,
            8,
            (0..32768 * 8)
                .map(|j| (j as f64 * 0.031).sin() + 0.3 * (j as f64 * 1.7).cos())
                .collect(),
        )
        .unwrap();
        measure(&pool, "hilbert 32768x8", 5, || {
            td::hilbert(black_box(&x), &c).unwrap()
        });
        measure(&pool, "decimate4 32768x8", 5, || {
            td::decimate(black_box(&x), 4, None, false, &c).unwrap()
        });
        measure(&pool, "interp4 32768x8", 5, || {
            td::interp(black_box(&x), 4, 4, 0.5, &c).unwrap()
        });
        measure(&pool, "median7 32768x8", 5, || {
            td::medfilt1(black_box(&x), 7, false, &c).unwrap()
        });
        measure(&pool, "sgolayfilt3/7 32768x8", 5, || {
            td::sgolayfilt(black_box(&x), 3, 7, &c).unwrap()
        });
        measure(&pool, "envelope rms7 32768x8", 5, || {
            td::envelope(black_box(&x), 7, td::EnvelopeKind::Rms, &c)
                .unwrap()
                .0
        });
        measure(&pool, "lowpass .3 32768x8", 5, || {
            td::convenience_filter(black_box(&x), &[0.3], None, Band::Lowpass, 0.75, 60., &c)
                .unwrap()
                .0
        });
        let peaks = Matrix::from_column_vec(
            1,
            32768,
            (0..32768)
                .map(|j| if j % 2 == 0 { 0. } else { 2. })
                .collect(),
        )
        .unwrap();
        measure(&pool, "findpeaks equal 32768", 5, || {
            td::findpeaks(black_box(&peaks), None, &td::PeakOptions::default(), &c).unwrap()
        });
        measure(&pool, "sgolay3/7", 10, || td::sgolay(3, 7, &c).unwrap().0);
        measure(&pool, "fir2 order64 mesh512", 5, || {
            td::fir2(
                64,
                &[0., 0.3, 0.5, 1.],
                &[1., 1., 0., 0.],
                None,
                0,
                None,
                &c,
            )
            .unwrap()
        });
        measure(&pool, "firpm order32 lowpass", 5, || {
            td::firpm(
                32,
                &[0., 0.3, 0.45, 1.],
                &[1., 1., 0., 0.],
                None,
                td::RemezKind::Symmetric,
                &c,
            )
            .unwrap()
        });
        for order in [8, 50] {
            measure(
                &pool,
                &format!("firpm order{order} round1 bandpass"),
                5,
                || {
                    td::firpm(
                        order,
                        &[0., 0.3, 0.4, 0.6, 0.7, 1.],
                        &[0., 0., 1., 1., 0., 0.],
                        None,
                        td::RemezKind::Symmetric,
                        &c,
                    )
                    .unwrap()
                },
            );
        }
        for order in [160, 252] {
            measure(&pool, &format!("firpm order{order} lowpass"), 5, || {
                td::firpm(
                    order,
                    &[0., 0.2, 0.3, 1.],
                    &[1., 1., 0., 0.],
                    None,
                    td::RemezKind::Symmetric,
                    &c,
                )
                .unwrap()
            });
        }
        measure(&pool, "firpm order21 differentiator", 5, || {
            td::firpm(
                21,
                &[0., 1.],
                &[0., std::f64::consts::PI],
                None,
                td::RemezKind::Differentiator,
                &c,
            )
            .unwrap()
        });
        let large = Matrix::from_column_vec(
            1_048_576,
            1,
            (0..1_048_576)
                .map(|j| (j as f64 * 0.031).sin() + 0.3 * (j as f64 * 1.7).cos())
                .collect(),
        )
        .unwrap();
        measure(&pool, "median101 1048576x1", 3, || {
            td::medfilt1(black_box(&large), 101, false, &c).unwrap()
        });
        measure(&pool, "bandpass .1-.2 defaults 1048576x1", 3, || {
            td::convenience_filter(
                black_box(&large),
                &[0.1, 0.2],
                None,
                Band::Bandpass,
                0.85,
                60.,
                &c,
            )
            .unwrap()
            .0
        });
    }
}
