//! Paired default-design baseline plus custom windows. Warm unpinned release runs.
use matrust::Result;
use matrust::dsp::{self, Band, FirOptions, MAX_ORDER};
use std::{f64::consts::PI, hint::black_box, sync::atomic::AtomicBool, time::Instant};
// Historical baseline copied from MatRust's own src/dsp.rs, not MathWorks source.
fn legacy_fir1(mut order: usize, cutoffs: &[f64], band: Band) -> Result<Vec<f64>> {
    let count = if matches!(band, Band::Bandpass | Band::Bandstop) {
        2
    } else {
        1
    };
    if order == 0 || order > MAX_ORDER {
        return Err(format!("FIR order must be 1..{MAX_ORDER}"));
    }
    if cutoffs.len() != count
        || cutoffs
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v >= 1.)
        || (count == 2 && cutoffs[0] >= cutoffs[1])
    {
        return Err("Cutoffs must be increasing, strictly between 0 and 1 (Nyquist)".into());
    }
    // A symmetric odd-order FIR has a zero at Nyquist. Match fir1's promotion.
    if matches!(band, Band::Highpass | Band::Bandstop) && !order.is_multiple_of(2) {
        order += 1;
    }
    let middle = order as f64 / 2.;
    let low = |f: f64, t: f64| {
        if t.abs() < f64::EPSILON {
            f
        } else {
            (PI * f * t).sin() / (PI * t)
        }
    };
    let mut b: Vec<f64> = (0..=order)
        .map(|i| {
            let t = i as f64 - middle;
            let impulse = if t == 0. { 1. } else { 0. };
            let ideal = match band {
                Band::Lowpass => low(cutoffs[0], t),
                Band::Highpass => impulse - low(cutoffs[0], t),
                Band::Bandpass => low(cutoffs[1], t) - low(cutoffs[0], t),
                Band::Bandstop => impulse - low(cutoffs[1], t) + low(cutoffs[0], t),
            };
            ideal * (0.54 - 0.46 * (2. * PI * i as f64 / order as f64).cos())
        })
        .collect();
    let reference = match band {
        Band::Highpass => 1.,
        Band::Bandpass => (cutoffs[0] + cutoffs[1]) / 2.,
        _ => 0.,
    };
    let gain = b
        .iter()
        .enumerate()
        .map(|(i, v)| v * (PI * reference * (i as f64 - middle)).cos())
        .sum::<f64>();
    if !gain.is_finite() || gain.abs() < 1e-14 {
        return Err("Degenerate FIR design; increase order or widen the passband".into());
    }
    for v in &mut b {
        *v /= gain;
    }
    Ok(b)
}

fn main() {
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut worker_counts = [1, 8];
    if reverse {
        worker_counts.reverse();
    }
    println!("order,band,workers,operation,repetitions,median_us,min_us,max_us,samples_us");
    for workers in worker_counts {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let cancel = AtomicBool::new(false);
                for order in [16, 64, 256, 4096] {
                    for band in [
                        Band::Lowpass,
                        Band::Highpass,
                        Band::Bandpass,
                        Band::Bandstop,
                    ] {
                        let cutoffs = if matches!(band, Band::Bandpass | Band::Bandstop) {
                            vec![0.2, 0.65]
                        } else {
                            vec![0.3]
                        };
                        let window =
                            dsp::window(dsp::WindowKind::Hann, order + 1, false, &cancel).unwrap();
                        let expected = legacy_fir1(order, &cutoffs, band).unwrap();
                        let actual = dsp::fir1(order, &cutoffs, band).unwrap();
                        assert!(
                            actual
                                .iter()
                                .zip(&expected)
                                .all(|(a, b)| (a - b).abs() < 2e-13)
                        );
                        let mut operations = [
                            "legacy_default",
                            "new_default",
                            "custom_scaled",
                            "custom_noscale",
                        ];
                        if reverse {
                            operations.reverse();
                        }
                        let mut samples = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
                        let evaluate = |name: &str| match name {
                            "legacy_default" => legacy_fir1(order, &cutoffs, band).unwrap(),
                            "new_default" => dsp::fir1(order, &cutoffs, band).unwrap(),
                            _ => dsp::fir1_with_options(
                                order,
                                &cutoffs,
                                band,
                                FirOptions {
                                    window: Some(window.data()),
                                    scale: name == "custom_scaled",
                                },
                                &cancel,
                            )
                            .unwrap(),
                        };
                        let references: Vec<_> =
                            operations.iter().map(|name| evaluate(name)).collect();
                        for round in 0..9 {
                            for offset in 0..4 {
                                let index = (round + offset) % 4;
                                let name = operations[index];
                                let start = Instant::now();
                                let output = black_box(evaluate(black_box(name)));
                                let micros = start.elapsed().as_secs_f64() * 1e6;
                                assert_eq!(output, references[index]);
                                if round >= 2 {
                                    samples[index].push(micros);
                                }
                            }
                        }
                        for (name, values) in operations.into_iter().zip(samples) {
                            let mut sorted = values.clone();
                            sorted.sort_by(f64::total_cmp);
                            println!(
                                "{order},{},{workers},{name},7,{:.6},{:.6},{:.6},\"{:?}\"",
                                band.name(),
                                sorted[3],
                                sorted[0],
                                sorted[6],
                                values
                            );
                        }
                    }
                }
            });
    }
}
