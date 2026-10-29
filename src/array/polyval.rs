//! Bounded real polynomial evaluation. Inputs are borrowed; the output is owned.
use super::sort::check;
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

const MAX_COEFFICIENTS: usize = 4096;
const MAX_STEPS: usize = 64 * 1024 * 1024;
const CHUNK: usize = 1024;
const PARALLEL_QUERIES: usize = 262_144;

// Measured: 4/5/8/12/16/17 coefficients, threshold neighbors and 1048576 queries,
// in local and registered global eight-worker contexts. Other admitted lengths
// and larger sizes remain an inference. See the milestone-56c8 performance report.
fn parallel_for(coefficients: usize, queries: usize) -> bool {
    (4..=17).contains(&coefficients)
        && queries >= PARALLEL_QUERIES
        // Root-owned helper reads existing caller/registered global state only.
        && crate::parallel::current_worker_count() == 8
}

#[cfg(test)]
type WorkProbe<'a> = Option<&'a (dyn Fn(bool, &[f64]) + Sync)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Validate,
    Allocate,
    Evaluate,
}
fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

fn preflight(coefficients: usize, queries: usize) -> Result<()> {
    if coefficients == 0 || coefficients > MAX_COEFFICIENTS {
        return Err("polyval requires 1..4096 real coefficients".into());
    }
    if (coefficients - 1)
        .checked_mul(queries)
        .is_none_or(|n| n > MAX_STEPS)
    {
        return Err("polyval exceeds its 67108864-step Horner work limit".into());
    }
    Ok(())
}

/// Evaluate finite real coefficients in descending powers at finite real points.
/// The nonempty coefficient vector may be a row or column (at most 4096 entries).
/// Queries retain their 2-D shape, including shaped empties. At most 64 Mi Horner
/// steps and the existing Matrix result limit are admitted. Ordinary separate
/// multiply/add operations can overflow; no scaling or accuracy estimate is used.
/// A constant polynomial copies its coefficient bits, including signed zero.
/// Large 4..17-coefficient evaluations use an existing eight-worker pool; other
/// calls remain serial. No pool or input copy is created.
pub fn evaluate_polynomial(p: &Matrix, x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    evaluate_polynomial_inner(
        p,
        x,
        cancel,
        #[cfg(test)]
        None,
    )
}

fn evaluate_polynomial_inner(
    p: &Matrix,
    x: &Matrix,
    cancel: &AtomicBool,
    #[cfg(test)] probe: WorkProbe<'_>,
) -> Result<Matrix> {
    check(cancel)?;
    if p.is_empty() || (p.rows() != 1 && p.cols() != 1) {
        return Err("polyval requires a nonempty coefficient row or column vector".into());
    }
    Matrix::checked_len(x.rows(), x.cols())?;
    preflight(p.len(), x.len())?;
    for values in [p.data(), x.data()] {
        for chunk in values.chunks(CHUNK) {
            checkpoint(cancel, Phase::Validate)?;
            if chunk.iter().any(|v| !v.is_finite()) {
                return Err("polyval requires finite real coefficients and query points".into());
            }
        }
    }
    checkpoint(cancel, Phase::Allocate)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(x.len())
        .map_err(|_| "Cannot allocate polyval output")?;
    // Initializing the only result buffer is bounded and cancellable.
    while output.len() < x.len() {
        checkpoint(cancel, Phase::Allocate)?;
        output.resize((output.len() + CHUNK).min(x.len()), 0.);
    }
    if parallel_for(p.len(), x.len()) {
        // Identical coarse partition to the measured candidate; no nested pool.
        let task_queries = (65536 / p.len()).clamp(64, 16384);
        output
            .par_chunks_mut(task_queries)
            .enumerate()
            .try_for_each(|(task, values)| {
                evaluate_queries(
                    p.data(),
                    &x.data()[task * task_queries..task * task_queries + values.len()],
                    values,
                    cancel,
                    #[cfg(test)]
                    probe,
                    #[cfg(test)]
                    true,
                )?;
                checkpoint(cancel, Phase::Evaluate)
            })?;
    } else {
        evaluate_queries(
            p.data(),
            x.data(),
            &mut output,
            cancel,
            #[cfg(test)]
            probe,
            #[cfg(test)]
            false,
        )?;
    }
    checkpoint(cancel, Phase::Evaluate)?;
    Matrix::from_column_vec(x.rows(), x.cols(), output)
}

fn evaluate_queries(
    coefficients: &[f64],
    queries: &[f64],
    output: &mut [f64],
    cancel: &AtomicBool,
    #[cfg(test)] probe: WorkProbe<'_>,
    #[cfg(test)] parallel: bool,
) -> Result<()> {
    // At most 1024 coefficient steps between checks for short polynomials.
    // Long polynomials additionally check each 1024-coefficient block.
    let queries_per_chunk = (CHUNK / coefficients.len()).max(1);
    for (chunk_index, values) in output.chunks_mut(queries_per_chunk).enumerate() {
        checkpoint(cancel, Phase::Evaluate)?;
        for (offset, result) in values.iter_mut().enumerate() {
            let query = queries[chunk_index * queries_per_chunk + offset];
            let mut value = coefficients[0];
            for block in coefficients[1..].chunks(CHUNK) {
                if coefficients.len() > CHUNK {
                    checkpoint(cancel, Phase::Evaluate)?;
                }
                for &coefficient in block {
                    value = value * query + coefficient;
                }
            }
            *result = value;
        }
        #[cfg(test)]
        if let Some(probe) = probe {
            probe(parallel, values);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};
    thread_local! {
        static TRIGGER: Cell<Option<(Phase, usize)>> = const { Cell::new(None) };
    }
    pub(super) fn probe(cancel: &AtomicBool, phase: Phase) {
        TRIGGER.with(|trigger| {
            if let Some((target, remaining)) = trigger.get()
                && phase == target
            {
                if remaining == 0 {
                    trigger.set(None);
                    cancel.store(true, Ordering::Relaxed);
                } else {
                    trigger.set(Some((target, remaining - 1)));
                }
            }
        });
    }

    #[test]
    fn exact_work_boundary_and_cancellation_during_each_phase() {
        assert!(preflight(65, 1024 * 1024).is_ok());
        assert!(
            preflight(65, 1024 * 1024 + 1)
                .unwrap_err()
                .contains("work limit")
        );
        assert!(preflight(4096, 0).is_ok());
        assert!(preflight(4097, 0).is_err());
        assert!(preflight(0, 0).is_err());
        assert!(preflight(4096, usize::MAX).is_err());
        let p = Matrix::from_column_vec(1, 4096, vec![1.; 4096]).unwrap();
        let x = Matrix::from_column_vec(1, 4096, vec![0.; 4096]).unwrap();
        let aliases = (p.clone(), x.clone());
        for (phase, remaining) in [
            (Phase::Validate, 1),
            (Phase::Allocate, 2),
            (Phase::Evaluate, 2),
        ] {
            let cancel = AtomicBool::new(false);
            TRIGGER.with(|trigger| trigger.set(Some((phase, remaining))));
            assert!(
                evaluate_polynomial(&p, &x, &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert!(cancel.load(Ordering::Relaxed));
            TRIGGER.with(|trigger| assert!(trigger.get().is_none()));
            assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
            assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
            cancel.store(false, Ordering::Relaxed);
            let small = Matrix::scalar(0.);
            assert_eq!(
                evaluate_polynomial(&p, &small, &cancel).unwrap().data(),
                &[1.]
            );
        }
    }

    #[test]
    fn dispatch_respects_existing_caller_pool_and_bounded_region() {
        for workers in [1, 7, 8, 9] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    assert!(rayon::current_thread_index().is_some());
                    assert_eq!(rayon::current_num_threads(), workers);
                    for coefficients in [1, 3, 4, 9, 17, 18, 4096] {
                        for queries in [
                            0,
                            PARALLEL_QUERIES - 1,
                            PARALLEL_QUERIES,
                            PARALLEL_QUERIES + 1,
                        ] {
                            assert_eq!(
                                parallel_for(coefficients, queries),
                                workers == 8
                                    && (4..=17).contains(&coefficients)
                                    && queries >= PARALLEL_QUERIES
                            );
                        }
                    }
                });
        }
    }

    #[test]
    fn production_routes_cancel_only_after_verified_completed_output_and_reuse_pool() {
        use std::sync::{Mutex, atomic::AtomicUsize};
        for workers in [1, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            for count in [4, 17] {
                let p = Matrix::from_column_vec(1, count, vec![1.; count]).unwrap();
                let x = Matrix::from_column_vec(
                    PARALLEL_QUERIES + 1,
                    1,
                    vec![1.; PARALLEL_QUERIES + 1],
                )
                .unwrap();
                let aliases = (p.clone(), x.clone());
                for should_cancel in [false, true] {
                    let cancel = AtomicBool::new(false);
                    let completed = AtomicUsize::new(0);
                    let first = Mutex::new(None);
                    let probe = |parallel: bool, values: &[f64]| {
                        assert!(!values.is_empty());
                        assert!(
                            values
                                .iter()
                                .all(|v| v.to_bits() == (count as f64).to_bits())
                        );
                        assert_eq!(parallel, workers == 8);
                        assert!(rayon::current_thread_index().is_some());
                        assert_eq!(rayon::current_num_threads(), workers);
                        if completed.fetch_add(values.len(), Ordering::Relaxed) == 0 {
                            *first.lock().unwrap() = Some((parallel, values.len()));
                            if should_cancel {
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                    };
                    let result =
                        pool.install(|| evaluate_polynomial_inner(&p, &x, &cancel, Some(&probe)));
                    let (parallel, first_count) = first.lock().unwrap().unwrap();
                    assert_eq!(parallel, workers == 8);
                    // Any coarse task may run first, including the shorter final task.
                    assert!((1..=CHUNK / count).contains(&first_count));
                    assert!(completed.load(Ordering::Relaxed) >= first_count);
                    if should_cancel {
                        assert_eq!(result.unwrap_err(), "Calculation cancelled");
                        assert!(cancel.load(Ordering::Relaxed));
                    } else {
                        let result = result.unwrap();
                        assert_eq!((result.rows(), result.cols()), (x.rows(), x.cols()));
                        assert_eq!(completed.load(Ordering::Relaxed), x.len());
                        assert!(
                            result
                                .data()
                                .iter()
                                .all(|v| v.to_bits() == (count as f64).to_bits())
                        );
                    }
                    assert_eq!(p.data().as_ptr(), aliases.0.data().as_ptr());
                    assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
                    cancel.store(false, Ordering::Relaxed);
                    let recovered = pool
                        .install(|| evaluate_polynomial(&p, &x, &cancel))
                        .unwrap();
                    assert!(
                        recovered
                            .data()
                            .iter()
                            .all(|v| v.to_bits() == (count as f64).to_bits())
                    );
                }
            }
        }
    }
}
