//! Warm, unpinned release benchmark. Pool setup and output verification are untimed.
use matrust::dsp::{WindowKind, window};
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
                for n in [4096, 16_384, 32_768, 65_536, 262_144, 1_048_576] {
                    let mut names = [
                        "hann", "hamming", "blackman", "bartlett", "triang", "rectwin",
                    ];
                    if reverse {
                        names.reverse();
                    }
                    for name in names {
                        let kind = WindowKind::from_name(name).unwrap();
                        let cancel = AtomicBool::new(false);
                        let reference = window(kind, n, false, &cancel).unwrap();
                        let mut times = Vec::new();
                        for round in 0..9 {
                            let start = Instant::now();
                            let output =
                                black_box(window(kind, black_box(n), false, &cancel).unwrap());
                            let micros = start.elapsed().as_secs_f64() * 1e6;
                            assert_eq!(output.data(), reference.data());
                            if round >= 2 {
                                times.push(micros);
                            }
                        }
                        let mut sorted = times.clone();
                        sorted.sort_by(f64::total_cmp);
                        println!(
                            "{name},{n},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",
                            sorted[3], sorted[0], sorted[6], times
                        );
                    }
                }
            });
    }
}
