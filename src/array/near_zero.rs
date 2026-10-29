//! Bounded serial real-double near-zero functions using the locked libm kernels.
use super::sort::check;
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 4096;

#[derive(Clone, Copy)]
enum Operation {
    Log1p,
    Expm1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Validate,
    Allocate,
    Map,
    Publish,
}

fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

/// Compute log(1+x) without explicitly rounding 1+x first.
/// Real double 2-D input retains its shape, including shaped empties. Any x<-1
/// (including -Inf) rejects the call; complex promotion is not supported here.
/// -1 maps to -Inf, +Inf to +Inf, NaN to NaN, and signed zeros are preserved.
/// Borrows the input, allocates one result and cooperatively checks cancellation.
pub fn log1p(input: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    evaluate(input, cancel, Operation::Log1p)
}

/// Compute exp(x)-1 accurately near zero for real double 2-D input.
/// Shape and signed zeros are preserved; -Inf maps to -1, +Inf to +Inf and NaN
/// to NaN. Finite overflow may produce +Inf. No NaN payload or exception-flag
/// guarantee is made. Execution is serial and does not initialize a thread pool.
pub fn expm1(input: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    evaluate(input, cancel, Operation::Expm1)
}

fn evaluate(input: &Matrix, cancel: &AtomicBool, operation: Operation) -> Result<Matrix> {
    check(cancel)?;
    Matrix::checked_len(input.rows(), input.cols())?;
    if matches!(operation, Operation::Log1p) {
        for chunk in input.data().chunks(CHUNK) {
            checkpoint(cancel, Phase::Validate)?;
            if chunk.iter().any(|&value| value < -1.) {
                return Err("log1p requires real values >= -1 or NaN; complex results for values below -1 are unsupported".into());
            }
        }
    }
    checkpoint(cancel, Phase::Allocate)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(input.len())
        .map_err(|_| "Cannot allocate near-zero function output")?;
    checkpoint(cancel, Phase::Allocate)?;
    for chunk in input.data().chunks(CHUNK) {
        checkpoint(cancel, Phase::Map)?;
        match operation {
            Operation::Log1p => output.extend(chunk.iter().copied().map(libm::log1p)),
            Operation::Expm1 => output.extend(chunk.iter().copied().map(libm::expm1)),
        }
    }
    checkpoint(cancel, Phase::Publish)?;
    Matrix::from_column_vec(input.rows(), input.cols(), output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::atomic::Ordering,
    };

    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static TRIGGER: Cell<Option<(Phase, usize)>> = const { Cell::new(None) };
        static TRACE: RefCell<Vec<Phase>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn probe(cancel: &AtomicBool, phase: Phase) {
        if !ACTIVE.get() {
            return;
        }
        TRACE.with_borrow_mut(|trace| trace.push(phase));
        if let Some((target, remaining)) = TRIGGER.get()
            && target == phase
        {
            if remaining == 1 {
                cancel.store(true, Ordering::Relaxed);
            } else {
                TRIGGER.set(Some((target, remaining - 1)));
            }
        }
    }

    struct Observation;
    impl Observation {
        fn begin(trigger: Option<(Phase, usize)>) -> Self {
            assert!(!ACTIVE.replace(true));
            TRACE.with_borrow_mut(Vec::clear);
            TRIGGER.set(trigger);
            Self
        }
    }
    impl Drop for Observation {
        fn drop(&mut self) {
            ACTIVE.set(false);
            TRIGGER.set(None);
            TRACE.with_borrow_mut(Vec::clear);
        }
    }

    #[test]
    fn cancellation_covers_validation_reservation_mapping_and_publication_with_retry() {
        let input = Matrix::from_column_vec(1, CHUNK * 2 + 1, vec![0.; CHUNK * 2 + 1]).unwrap();
        let alias = input.clone();
        for operation in [Operation::Log1p, Operation::Expm1] {
            for (phase, count) in [
                (Phase::Validate, 2),
                (Phase::Allocate, 1),
                (Phase::Allocate, 2),
                (Phase::Map, 2),
                (Phase::Publish, 1),
            ] {
                if matches!(operation, Operation::Expm1) && phase == Phase::Validate {
                    continue;
                }
                let cancel = AtomicBool::new(false);
                {
                    let _observation = Observation::begin(Some((phase, count)));
                    assert_eq!(
                        evaluate(&input, &cancel, operation).unwrap_err(),
                        "Calculation cancelled"
                    );
                    TRACE.with_borrow(|trace| {
                        assert_eq!(trace.last(), Some(&phase));
                        if phase == Phase::Validate {
                            assert!(!trace.contains(&Phase::Allocate));
                        }
                    });
                }
                cancel.store(false, Ordering::Relaxed);
                let result = evaluate(&input, &cancel, operation).unwrap();
                assert!(result.data().iter().all(|v| v.to_bits() == 0));
                assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
            }
        }
    }

    #[test]
    fn domain_rejects_before_any_output_reservation_even_at_last_element() {
        for index in [0, CHUNK, CHUNK * 2] {
            let mut values = vec![0.; CHUNK * 2 + 1];
            values[index] = -2.;
            let input = Matrix::from_column_vec(1, values.len(), values).unwrap();
            let _observation = Observation::begin(None);
            assert!(
                log1p(&input, &AtomicBool::new(false))
                    .unwrap_err()
                    .contains("below -1")
            );
            TRACE.with_borrow(|trace| {
                assert!(!trace.contains(&Phase::Allocate));
                assert!(!trace.contains(&Phase::Map));
                assert_eq!(trace.len(), index / CHUNK + 1);
            });
        }
    }
}
