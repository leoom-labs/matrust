//! Release-only kernel measurements; pool/input setup, validation/destruction untimed.
use matrust::dsp;
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!("beta,length,workers,repetitions,median_us,min_us,max_us,samples_us");
    for workers in workers {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let cancel = AtomicBool::new(false);
                for n in [1024, 16_384, 32_768, 65_536, 262_144, 1_048_576] {
                    let mut betas = [0., 0.5, 8.6, 50., 700., 10_000., f64::MAX];
                    if reverse {
                        betas.reverse();
                    }
                    for beta in betas {
                        let reference = dsp::kaiser(n, beta, &cancel).unwrap();
                        let mut samples = Vec::new();
                        for round in 0..9 {
                            let start = Instant::now();
                            let output = black_box(
                                dsp::kaiser(black_box(n), black_box(beta), &cancel).unwrap(),
                            );
                            let elapsed = start.elapsed().as_secs_f64() * 1e6;
                            assert_eq!(reference.data(), output.data());
                            if round >= 2 {
                                samples.push(elapsed);
                            }
                        }
                        let mut sorted = samples.clone();
                        sorted.sort_by(f64::total_cmp);
                        println!(
                            "{beta},{n},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",
                            sorted[3], sorted[0], sorted[6], samples
                        );
                    }
                }
            });
    }
}
