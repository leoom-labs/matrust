//! Release draft, no previous kernel/MATLAB baseline. Pool setup/checks/destruction untimed.
use matrust::{
    Matrix,
    dsp::{self, ChirpMethod, ChirpOptions, ChirpShape},
};
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
                    let mut names = [
                        "linear",
                        "quadratic_zero",
                        "quadratic_reference",
                        "logarithmic",
                        "linear_extreme",
                        "quadratic_scaled",
                        "quadratic_cancellation",
                        "logarithmic_scaled",
                        "constant_tone",
                    ];
                    if reverse {
                        names.reverse();
                    }
                    for name in names {
                        let method = match name {
                            "linear" | "linear_extreme" | "constant_tone" => ChirpMethod::Linear,
                            "logarithmic" | "logarithmic_scaled" => ChirpMethod::Logarithmic,
                            _ => ChirpMethod::Quadratic,
                        };
                        let mut options = ChirpOptions {
                            f0: 10.,
                            f1: 400.,
                            t1: 2.,
                            method,
                            phase_degrees: 37.,
                            shape: if name == "quadratic_reference" {
                                ChirpShape::Convex
                            } else {
                                ChirpShape::Default
                            },
                        };
                        match name {
                            "linear_extreme" => {
                                options.f0 = -1e308;
                                options.f1 = 1e308;
                                options.t1 = 1e-308;
                            }
                            "quadratic_scaled" => {
                                options.f0 = 0.;
                                options.f1 = 1e-300;
                                options.t1 = 1e-300;
                            }
                            "quadratic_cancellation" => {
                                options.f0 = 0.;
                                options.f1 = 1e15;
                                options.t1 = 1.;
                                options.shape = ChirpShape::Convex;
                            }
                            "logarithmic_scaled" => {
                                options.f0 = f64::from_bits(1);
                                options.f1 = 1e300;
                                options.t1 = 1.;
                            }
                            "constant_tone" => {
                                options.f0 = 1f64.next_up();
                                options.f1 = options.f0;
                            }
                            _ => {}
                        }
                        let input = Matrix::from_column_vec(
                            1,
                            n,
                            (0..n)
                                .map(|i| {
                                    let fraction = i as f64 / (n - 1) as f64;
                                    match name {
                                        "linear_extreme" => fraction * 1e-308,
                                        "quadratic_scaled" => fraction * 1e-100,
                                        "quadratic_cancellation" => {
                                            f64::from_bits(3f64.to_bits() - 8 + (i % 17) as u64)
                                        }
                                        "logarithmic_scaled" => 0.5 + 0.02 * fraction,
                                        "constant_tone" => 2f64.powi(50) + i as f64 * 0.5,
                                        _ => 2. * fraction,
                                    }
                                })
                                .collect(),
                        )
                        .unwrap();
                        let call =
                            || dsp::chirp(black_box(&input), black_box(options), &cancel).unwrap();
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
                            "{name},{n},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",
                            sorted[3], sorted[0], sorted[6], samples
                        );
                    }
                }
            });
    }
}
