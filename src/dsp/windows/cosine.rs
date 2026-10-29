//! Published MathWorks generalized-cosine coefficients; no vendor source copied.
//! See the private source packet for primary references and accuracy limitations.
use super::WindowKind;
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 4096;

fn coefficients(kind: WindowKind) -> &'static [f64] {
    match kind {
        WindowKind::BlackmanHarris => &[0.35875, 0.48829, 0.14128, 0.01168],
        WindowKind::Nuttall => &[0.3635819, 0.4891775, 0.1365995, 0.0106411],
        WindowKind::FlatTop => &[
            0.21557895,
            0.41663158,
            0.277263158,
            0.083578947,
            0.006947368,
        ],
        _ => unreachable!(),
    }
}

pub(super) fn generate(
    kind: WindowKind,
    n: usize,
    periodic: bool,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    // Candidate threshold: public before/after evidence in both modes is still required.
    build(
        kind,
        n,
        periodic,
        n >= 65_536 && rayon::current_num_threads() > 1,
        &|| super::super::check_cancel(cancel),
    )
}

fn build(
    kind: WindowKind,
    n: usize,
    periodic: bool,
    _parallel: bool,
    checkpoint: &(impl Fn() -> Result<()> + Sync),
) -> Result<Matrix> {
    checkpoint()?;
    if n > crate::matrix::MAX_ELEMENTS {
        return Err("Window exceeds matrix element limit".into());
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(n)
        .map_err(|_| "Unable to allocate cosine window".to_string())?;
    while output.len() < n {
        checkpoint()?;
        output.resize((output.len() + CHUNK).min(n), 0.);
    }
    if n <= 1 {
        output.fill(1.);
        checkpoint()?;
        return Matrix::from_column_vec(n, 1, output);
    }
    let denominator = if periodic { n } else { n - 1 };
    let coefficients = coefficients(kind);
    let fill = |(chunk_index, chunk): (usize, &mut [f64])| -> Result<()> {
        checkpoint()?;
        for (offset, value) in chunk.iter_mut().enumerate() {
            let index = chunk_index * CHUNK + offset;
            let folded = index.min(denominator - index);
            let angle = std::f64::consts::TAU * folded as f64 / denominator as f64;
            let mut sum = coefficients[0];
            for (harmonic, coefficient) in coefficients.iter().enumerate().skip(1) {
                let term = coefficient * libm::cos(harmonic as f64 * angle);
                if harmonic % 2 == 0 {
                    sum += term;
                } else {
                    sum -= term;
                }
            }
            // Do not clip negative flat-top samples or normalize rounded coefficient sums.
            *value = sum;
        }
        Ok(())
    };
    if _parallel {
        output
            .par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(fill)?;
        checkpoint()?;
        return Matrix::from_column_vec(n, 1, output);
    }
    for chunk in output.chunks_mut(CHUNK).enumerate() {
        fill(chunk)?;
    }
    checkpoint()?;
    Matrix::from_column_vec(n, 1, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn candidate_parallel_matches_serial_and_cancellation_is_observed_between_chunks() {
        for workers in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    for kind in [
                        WindowKind::BlackmanHarris,
                        WindowKind::Nuttall,
                        WindowKind::FlatTop,
                    ] {
                        for n in [0, 1, 4097, 65_536, 65_537] {
                            for periodic in [false, true] {
                                let serial = build(kind, n, periodic, false, &|| Ok(())).unwrap();
                                let parallel = build(kind, n, periodic, true, &|| Ok(())).unwrap();
                                assert_eq!(serial.data(), parallel.data());
                            }
                        }
                    }
                    for parallel in [false, true] {
                        let calls = AtomicUsize::new(0);
                        // Two initialization chunks; reject the first actual sample chunk.
                        let error = build(WindowKind::FlatTop, 8192, false, parallel, &|| {
                            if calls.fetch_add(1, Ordering::Relaxed) >= 3 {
                                Err("Calculation cancelled".into())
                            } else {
                                Ok(())
                            }
                        })
                        .unwrap_err();
                        assert_eq!(error, "Calculation cancelled");
                    }
                });
        }
    }

    #[test]
    #[ignore = "Fixed release strategy measurement; root-owned execution only"]
    #[allow(
        clippy::assertions_on_constants,
        reason = "This measurement must compile in debug but reject execution there."
    )]
    fn cosine_window_strategy_measurement() {
        use std::{hint::black_box, time::Instant};
        assert!(!cfg!(debug_assertions));
        let reverse = std::env::var_os("MATRUST_WINDOW_REVERSE").is_some();
        let mut workers = [1, 8];
        let mut lengths = [64, 4096, 65_536, 1_048_576];
        let mut kinds = [
            WindowKind::BlackmanHarris,
            WindowKind::Nuttall,
            WindowKind::FlatTop,
        ];
        let mut modes = [false, true];
        if reverse {
            workers.reverse();
            lengths.reverse();
            kinds.reverse();
            modes.reverse();
        }
        for workers in workers {
            rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
                for n in lengths {
                    for kind in kinds {
                        let cancel = AtomicBool::new(false);
                        let checkpoint = || super::super::super::check_cancel(&cancel);
                        let reference = build(kind, n, false, false, &checkpoint).unwrap();
                        for parallel in modes {
                            for round in 0..9 {
                                let start = Instant::now();
                                let result = black_box(build(kind, black_box(n), false, parallel, &checkpoint).unwrap());
                                let elapsed = start.elapsed().as_secs_f64() * 1e6;
                                assert_eq!(result.data(), reference.data());
                                println!("COSINE_WINDOW,{kind:?},{n},{workers},{parallel},{round},{elapsed:.9}");
                            }
                        }
                    }
                }
            });
        }
    }
}
