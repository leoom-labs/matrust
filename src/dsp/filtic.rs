//! Bounded initial state from newest-first histories; existing DF-II conventions.
use super::{Filter, MAX_WORK, check_cancel};
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 1024;

#[cfg(test)]
type WorkProbe<'a> = Option<&'a dyn Fn(usize, f64)>;

/// Construct the transposed direct-form II delay column from past outputs/inputs.
/// Coefficients are finite, nonempty and order <=4096; a[0] must equal one.
/// Histories are newest first; omitted input and missing entries are zero.
/// Only the needed history prefixes are read or validated. Nonfinite needed
/// entries or computed states reject. Histories stay borrowed; the existing
/// Filter constructor makes bounded coefficient copies. Inputs remain immutable.
pub fn filtic(
    b: &[f64],
    a: &[f64],
    y: &[f64],
    x: Option<&[f64]>,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    initial_state(
        b,
        a,
        y,
        x,
        cancel,
        #[cfg(test)]
        None,
    )
}

fn initial_state(
    b: &[f64],
    a: &[f64],
    y: &[f64],
    x: Option<&[f64]>,
    cancel: &AtomicBool,
    #[cfg(test)] probe: WorkProbe<'_>,
) -> Result<Matrix> {
    check_cancel(cancel)?;
    if a.first().is_some_and(|value| *value != 1.) {
        return Err(
            "filtic currently requires a(1)=1; nonunit denominator normalization is unsupported"
                .into(),
        );
    }
    // Reuse the proven finite coefficient, order and normalized-storage checks.
    // Its bounded <=4097-element allocations/scans remain unchanged.
    let prototype = Filter::new(b, a)?;
    check_cancel(cancel)?;
    let order = prototype.state.len();
    Matrix::checked_len(order, 1)?;
    let x = x.unwrap_or(&[]);
    let x = &x[..x.len().min(b.len() - 1)];
    let y = &y[..y.len().min(a.len() - 1)];
    if order.saturating_mul(x.len().saturating_add(y.len())) > MAX_WORK {
        return Err("filtic exceeds the bounded filter work limit".into());
    }
    for history in [x, y] {
        for chunk in history.chunks(CHUNK) {
            check_cancel(cancel)?;
            if chunk.iter().any(|value| !value.is_finite()) {
                return Err("filtic requires finite values in the needed history prefixes".into());
            }
        }
    }
    let numerator_order = b.len() - 1;
    let denominator_order = a.len() - 1;
    let Filter {
        b, a, mut state, ..
    } = prototype;
    for (k, output) in state.iter_mut().enumerate() {
        check_cancel(cancel)?;
        let nx = numerator_order.saturating_sub(k).min(x.len());
        let ny = denominator_order.saturating_sub(k).min(y.len());
        let mut sums = [0., 0.];
        for (sum, (coefficients, history)) in sums.iter_mut().zip([
            (&b[k + 1..k + 1 + nx], &x[..nx]),
            (&a[k + 1..k + 1 + ny], &y[..ny]),
        ]) {
            for (coefficients, history) in coefficients.chunks(CHUNK).zip(history.chunks(CHUNK)) {
                check_cancel(cancel)?;
                for (&coefficient, &past) in coefficients.iter().zip(history) {
                    *sum += coefficient * past;
                }
                #[cfg(test)]
                if let Some(probe) = probe {
                    probe(coefficients.len(), *sum);
                }
                check_cancel(cancel)?;
            }
        }
        *output = sums[0] - sums[1];
        if !output.is_finite() {
            return Err("filtic initial-state arithmetic overflowed".into());
        }
    }
    check_cancel(cancel)?;
    Matrix::from_column_vec(order, 1, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};

    #[test]
    fn cancellation_after_real_history_products_never_publishes_partial_state() {
        let b = vec![1.; 4097];
        let x = vec![1.; 4096];
        let cancel = AtomicBool::new(false);
        let completed = Cell::new(0);
        let probe = |count: usize, sum: f64| {
            assert_eq!(count, CHUNK);
            assert_eq!(sum, 1024.);
            completed.set(completed.get() + count);
            cancel.store(true, Ordering::Relaxed);
        };
        assert!(
            initial_state(&b, &[1.], &[], Some(&x), &cancel, Some(&probe))
                .unwrap_err()
                .contains("cancelled")
        );
        assert_eq!(completed.get(), 1024);
        assert_eq!(b, vec![1.; 4097]);
        assert_eq!(x, vec![1.; 4096]);
        cancel.store(false, Ordering::Relaxed);
        let z = filtic(&b, &[1.], &[], Some(&x), &cancel).unwrap();
        assert_eq!((z.rows(), z.cols()), (4096, 1));
        for (k, value) in z.data().iter().enumerate() {
            assert_eq!(*value, (4096 - k) as f64);
        }
    }
}
