use matrust::{
    Matrix,
    dsp::{self, Band},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    println!("operation,samples,channels,threads,repetitions,ms_per_iteration");
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            let b = dsp::fir1(64, &[0.2], Band::Lowpass).unwrap();
            for (samples, channels) in [(256, 1), (4096, 8), (100000, 8)] {
                let x = Matrix::filled(samples, channels, 0.5).unwrap();
                let rows = x.transpose();
                let zi = Matrix::filled(64, channels, 0.1).unwrap();
                for operation in ["zero_state", "column_state", "row_state", "gain"] {
                    let run = || {
                        if operation == "gain" {
                            black_box(
                                dsp::filter_with_state(
                                    &[3.],
                                    &[2.],
                                    black_box(&x),
                                    None,
                                    None,
                                    &cancel,
                                )
                                .unwrap(),
                            );
                        } else if operation == "zero_state" {
                            black_box(dsp::filter(&b, &[1.], black_box(&x), &cancel).unwrap());
                        } else {
                            black_box(
                                dsp::filter_with_state(
                                    &b,
                                    &[1.],
                                    black_box(if operation == "row_state" { &rows } else { &x }),
                                    Some(&zi),
                                    Some(if operation == "row_state" { 2 } else { 1 }),
                                    &cancel,
                                )
                                .unwrap(),
                            );
                        }
                    };
                    run();
                    let start = Instant::now();
                    for _ in 0..20 {
                        run();
                    }
                    println!(
                        "{operation},{samples},{channels},{threads},20,{:.4}",
                        start.elapsed().as_secs_f64() * 50.
                    );
                }
            }
        });
    }
}
