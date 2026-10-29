#[path = "support/filter_before.rs"]
mod before;
use matrust::{Matrix, dsp};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() {
    println!("operation,samples,channels,threads,repetitions,before_ms,after_ms,speedup");
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            for (samples, channels) in [
                (32, 1),
                (63, 1),
                (64, 1),
                (128, 1),
                (256, 1),
                (4096, 1),
                (100000, 1),
                (4096, 8),
                (100000, 8),
            ] {
                let x = Matrix::from_column_vec(
                    samples,
                    channels,
                    (0..samples * channels)
                        .map(|i| ((i % 997) as f64 - 498.) / 997.)
                        .collect(),
                )
                .unwrap();
                for order in [2, 8, 20] {
                    let sos = dsp::butter_sos(order, 0.2, dsp::Band::Lowpass).unwrap();
                    let old = || before::sosfilt(black_box(&sos), black_box(&x), &cancel).unwrap();
                    let new = || dsp::sosfilt(black_box(&sos), black_box(&x), &cancel).unwrap();
                    assert_eq!(old().data(), new().data());
                    for _ in 0..3 {
                        black_box(old());
                        black_box(new());
                    }
                    let mut times = [0.; 2];
                    let reps = if samples < 10000 { 100 } else { 20 };
                    for i in 0..reps {
                        for which in if i % 2 == 0 { [0, 1] } else { [1, 0] } {
                            let start = Instant::now();
                            if which == 0 {
                                black_box(old());
                            } else {
                                black_box(new());
                            }
                            times[which] += start.elapsed().as_secs_f64() * 1000. / reps as f64;
                        }
                    }
                    println!(
                        "sos{order},{samples},{channels},{threads},{reps},{:.6},{:.6},{:.3}",
                        times[0],
                        times[1],
                        times[0] / times[1]
                    );
                }
            }
            for (name, b, a) in [
                ("fir1", vec![0.2, 0.3], vec![1.]),
                ("fir2", vec![0.2, 0.3, 0.2], vec![1.]),
                ("iir1", vec![0.2, 0.3], vec![1., -0.5]),
                ("iir2", vec![0.2, 0.3, 0.2], vec![1., -0.5, 0.2]),
                ("fir64", vec![0.01; 65], vec![1.]),
            ] {
                let mut old = before::Filter::new(&b, &a).unwrap();
                let mut new = dsp::Filter::new(&b, &a).unwrap();
                let source: Vec<_> = (0..100000).map(|i| (i % 31) as f64 / 31.).collect();
                let mut data = source.clone();
                let mut times = [0.; 2];
                for i in 0..20 {
                    for which in if i % 2 == 0 { [0, 1] } else { [1, 0] } {
                        data.copy_from_slice(&source);
                        old.reset();
                        new.reset();
                        let start = Instant::now();
                        if which == 0 {
                            old.process(black_box(&mut data), &cancel).unwrap();
                        } else {
                            new.process(black_box(&mut data), &cancel).unwrap();
                        }
                        times[which] += start.elapsed().as_secs_f64() * 50.;
                        black_box(&data);
                    }
                }
                println!(
                    "{name},100000,1,{threads},20,{:.6},{:.6},{:.3}",
                    times[0],
                    times[1],
                    times[0] / times[1]
                );
            }
        });
    }
}
