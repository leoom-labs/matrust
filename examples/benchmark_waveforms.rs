//! Release benchmark draft: no old kernel or MATLAB baseline is claimed.
use matrust::{Matrix, dsp};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let reverse = std::env::args().any(|arg| arg == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!("name,length,workers,repetitions,median_us,min_us,max_us,samples_us");
    for workers in workers {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let cancel = AtomicBool::new(false);
                for n in [4096, 16_384, 32_768, 65_536, 262_144, 1_048_576] {
                    let input = Matrix::from_column_vec(
                        1,
                        n,
                        (0..n)
                            .map(|i| (i as f64 * 0.125) % 16000. - 8000.)
                            .collect(),
                    )
                    .unwrap();
                    let mut names = ["sinc", "square", "sawtooth"];
                    if reverse {
                        names.reverse();
                    }
                    for name in names {
                        let call = || {
                            match name {
                                "sinc" => dsp::sinc(black_box(&input), &cancel),
                                "square" => dsp::square(black_box(&input), 37., &cancel),
                                _ => dsp::sawtooth(black_box(&input), 0.37, &cancel),
                            }
                            .unwrap()
                        };
                        let reference = call();
                        let mut samples = Vec::new();
                        for round in 0..9 {
                            let start = Instant::now();
                            let output = black_box(call());
                            let elapsed = start.elapsed().as_secs_f64() * 1e6;
                            assert_eq!(output.data(), reference.data());
                            if round >= 2 {
                                samples.push(elapsed);
                            }
                        }
                        let mut sorted = samples.clone();
                        sorted.sort_by(f64::total_cmp);
                        println!(
                            "{name},{n},{workers},7,{:0.6},{:0.6},{:0.6},\"{:?}\"",
                            sorted[3], sorted[0], sorted[6], samples
                        );
                    }
                }
            });
    }
}
