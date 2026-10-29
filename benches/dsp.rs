use matrust::{
    Matrix,
    dsp::{self, Band},
    simulation::Model,
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    println!("kernel,samples,channels,threads,repetitions,ms_per_iteration");
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            let b = dsp::fir1(64, &[0.2], Band::Lowpass).unwrap();
            let sos = dsp::butter_sos(8, 0.2, Band::Lowpass).unwrap();
            for kind in [
                dsp::Analysis::Phase,
                dsp::Analysis::Impulse,
                dsp::Analysis::Step,
            ] {
                let run = || dsp::analyze(kind, black_box(&b), &[1.], 4096, &cancel).unwrap();
                black_box(run());
                let start = Instant::now();
                for _ in 0..20 {
                    black_box(run());
                }
                println!(
                    "{}_fir64,4096,1,{threads},20,{:.4}",
                    kind.function(false),
                    start.elapsed().as_secs_f64() * 50.
                );
            }
            for (order, points) in [(64, 1024), (256, 8192)] {
                let coefficients = dsp::fir1(order, &[0.2], Band::Lowpass).unwrap();
                let run = || {
                    dsp::analyze(
                        dsp::Analysis::GroupDelay,
                        black_box(&coefficients),
                        &[1.],
                        points,
                        &cancel,
                    )
                    .unwrap()
                };
                black_box(run());
                let start = Instant::now();
                for _ in 0..20 {
                    black_box(run());
                }
                println!(
                    "grpdelay{order},{points},1,{threads},20,{:.4}",
                    start.elapsed().as_secs_f64() * 50.
                );
            }
            for (rows, cols) in [(256, 1), (4096, 8), (100000, 8)] {
                let x = Matrix::filled(rows, cols, 0.5).unwrap();
                for kind in ["fir64", "sos8"] {
                    let run = || {
                        if kind == "fir64" {
                            dsp::filter(&b, &[1.], black_box(&x), &cancel).unwrap()
                        } else {
                            dsp::sosfilt(&sos, black_box(&x), &cancel).unwrap()
                        }
                    };
                    black_box(run());
                    let start = Instant::now();
                    for _ in 0..10 {
                        black_box(run());
                    }
                    println!(
                        "{kind},{rows},{cols},{threads},10,{:.4}",
                        start.elapsed().as_secs_f64() * 100.
                    );
                }
            }
            let model = Model {
                steps: 100000,
                ..Model::default()
            };
            black_box(model.run(&cancel).unwrap());
            let start = Instant::now();
            for _ in 0..5 {
                black_box(model.run(&cancel).unwrap());
            }
            println!(
                "simulation,100000,1,{threads},5,{:.4}",
                start.elapsed().as_secs_f64() * 200.
            );
        });
    }
}
