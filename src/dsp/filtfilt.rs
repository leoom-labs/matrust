//! Bounded real transfer-function forward/backward filtering.
use super::{Filter, MAX_WORK, check_cancel};
use crate::{Matrix, Result};
use nalgebra::{DMatrix, DVector};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

// Bound uncancellable library factorization to 256 states (512 KiB per matrix).
const MAX_IIR_INITIAL_ORDER: usize = 256;
// Candidate only: release measurements must establish whether this is appropriate.
const PARALLEL_WORK: usize = 1_000_000;

/// Odd reflection with max(1, 3*order) samples, matching the reference
/// default. Row vectors retain orientation; other 2-D inputs use columns.
/// Empty input retains its shape, a deliberate MatRust extension. Coefficients
/// must remain nonempty even for empty input. No SOS/options/complex support.
pub fn filtfilt(b: &[f64], a: &[f64], x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    check_cancel(cancel)?;
    let prototype = Filter::new(b, a)?;
    let order = prototype.state.len();
    if prototype.feedback && order > MAX_IIR_INITIAL_ORDER {
        return Err("filtfilt direct-form IIR initial-state solve supports order at most 256; SOS support is not yet implemented".into());
    }
    if x.is_empty() {
        return Ok(x.clone());
    }
    let (samples, channels) = if x.rows() == 1 {
        (x.cols(), 1)
    } else {
        (x.rows(), x.cols())
    };
    let padding = (3 * order).max(1);
    if samples <= padding {
        return Err(format!(
            "filtfilt requires more than {padding} samples per channel"
        ));
    }
    let work = (2 * samples + 3 * padding)
        .saturating_mul(channels)
        .saturating_mul(order + 1);
    let solve_work = if prototype.feedback {
        order.saturating_pow(3)
    } else {
        order
    };
    if work.saturating_add(solve_work) > MAX_WORK {
        return Err(
            "filtfilt operation exceeds the bounded work limit; reduce signal size or filter order"
                .into(),
        );
    }
    let initial = steady_state(&prototype, cancel)?;
    let mut output = Vec::with_capacity(x.len());
    for chunk in x.data().chunks(4096) {
        check_cancel(cancel)?;
        require_finite(chunk, "filtfilt input must be finite")?;
        output.extend_from_slice(chunk);
    }
    let process = |state: &mut (Filter, Vec<f64>), values: &mut [f64]| -> Result<()> {
        let (filter, edge) = state;
        check_cancel(cancel)?;
        if order == 0 {
            let gain = filter.b[0];
            for chunk in values.chunks_mut(4096) {
                check_cancel(cancel)?;
                for value in chunk.iter_mut() {
                    *value = (gain * *value + 0.) * gain + 0.;
                }
                require_finite(chunk, "filtfilt gain overflowed")?;
                #[cfg(test)]
                active_cancellation_tests::after_completed_chunk(cancel, chunk.len());
            }
            return check_cancel(cancel);
        }
        edge.clear();
        for i in (1..=padding).rev() {
            // libm avoids incorrect extreme-range FMA results in the Windows GNU CRT.
            // One rounded operation also avoids overflow in 2*endpoint when
            // the mathematically reflected finite value remains representable.
            edge.push(libm::fma(values[0], 2., -values[i]));
        }
        require_finite(edge, "filtfilt reflection overflowed")?;
        initialize(filter, &initial, edge[0])?;
        process_checked(filter, edge, cancel)?;
        // Preserve the right extension before the input values are overwritten.
        edge.clear();
        let last = values.len() - 1;
        for i in 1..=padding {
            edge.push(libm::fma(values[last], 2., -values[last - i]));
        }
        require_finite(edge, "filtfilt reflection overflowed")?;
        process_checked(filter, values, cancel)?;
        process_checked(filter, edge, cancel)?;
        reverse_checked(edge, cancel)?;
        initialize(filter, &initial, edge[0])?;
        process_checked(filter, edge, cancel)?;
        reverse_checked(values, cancel)?;
        process_checked(filter, values, cancel)?;
        reverse_checked(values, cancel)?;
        // The reverse pass's left extension occurs after all retained outputs,
        // so causality allows it to be omitted without changing those outputs.
        check_cancel(cancel)
    };
    if channels > 1 && work >= PARALLEL_WORK && rayon::current_num_threads() > 1 {
        output
            .par_chunks_mut(samples)
            .try_for_each_init(|| (prototype.clone(), Vec::with_capacity(padding)), process)?;
    } else {
        let mut state = (prototype, Vec::with_capacity(padding));
        for values in output.chunks_mut(samples) {
            process(&mut state, values)?;
        }
    }
    check_cancel(cancel)?;
    Matrix::from_column_vec(x.rows(), x.cols(), output)
}

pub(super) fn require_finite(values: &[f64], error: &str) -> Result<()> {
    if values.iter().any(|v| !v.is_finite()) {
        Err(error.into())
    } else {
        Ok(())
    }
}
pub(super) fn initialize(filter: &mut Filter, initial: &[f64], endpoint: f64) -> Result<()> {
    for (state, value) in filter.state.iter_mut().zip(initial) {
        *state = value * endpoint;
    }
    require_finite(&filter.state, "filtfilt scaled initial state overflowed")
}
fn process_checked(filter: &mut Filter, data: &mut [f64], cancel: &AtomicBool) -> Result<()> {
    for chunk in data.chunks_mut(1024) {
        check_cancel(cancel)?;
        filter.process(chunk, cancel)?;
        require_finite(
            chunk,
            "filtfilt recurrence overflowed; use a better-conditioned filter",
        )?;
        require_finite(&filter.state, "filtfilt recurrence state overflowed")?;
        #[cfg(test)]
        active_cancellation_tests::after_completed_chunk(cancel, chunk.len());
    }
    Ok(())
}
pub(super) fn reverse_checked(data: &mut [f64], cancel: &AtomicBool) -> Result<()> {
    for i in 0..data.len() / 2 {
        if i.is_multiple_of(4096) {
            check_cancel(cancel)?;
        }
        data.swap(i, data.len() - 1 - i);
    }
    check_cancel(cancel)
}

pub(super) fn steady_state(filter: &Filter, cancel: &AtomicBool) -> Result<Vec<f64>> {
    let n = filter.state.len();
    check_cancel(cancel)?;
    if !filter.feedback {
        // A nilpotent FIR state transition needs only suffix sums, not a dense solve.
        let mut state = vec![0.; n];
        let (mut sum, mut correction) = (0., 0.);
        for i in (0..n).rev() {
            let next = filter.b[i + 1] - correction;
            let total = sum + next;
            correction = (total - sum) - next;
            sum = total;
            state[i] = sum;
        }
        require_finite(&state, "filtfilt FIR initial state overflowed")?;
        return Ok(state);
    }
    // DF-II transposed: (I-A)zi = b[1:] - b[0]*a[1:]. Row scaling
    // keeps the matrix and right-hand side finite without forming an inverse.
    // Boost a small numerator by an exact power of two before forming
    // cancellation residuals. Rescale only after solving: a tiny RHS may have
    // a representable zi with much more precision than the unscaled RHS.
    let numerator_max = filter.b.iter().map(|v| v.abs()).fold(0_f64, f64::max);
    let numerator_scale = if numerator_max > 0. && numerator_max < 1. {
        let bits = numerator_max.to_bits();
        let exponent = bits & 0x7ff0_0000_0000_0000;
        f64::from_bits(if exponent != 0 {
            exponent
        } else {
            1_u64 << (63 - bits.leading_zeros())
        })
    } else {
        1.
    };
    let b0 = filter.b[0] / numerator_scale;
    let mut system = DMatrix::<f64>::zeros(n, n);
    let mut rhs = DVector::<f64>::zeros(n);
    for i in 0..n {
        // Form a finite cancellation residual before scaling. Dividing a tiny
        // denominator coefficient first can erase a finite b0*a_i product.
        let bi = filter.b[i + 1] / numerator_scale;
        let unscaled = libm::fma(-b0, filter.a[i + 1], bi);
        if unscaled == 0. && !exact_product_equals(b0, filter.a[i + 1], bi) {
            return Err("filtfilt initial-state RHS underflow is unsupported for this direct-form coefficient range".into());
        }
        // Scale by the matrix row, not numerator amplitude: a large b0 can
        // otherwise underflow a small but representable cancellation residual.
        let scale = 1_f64.max(filter.a[i + 1].abs());
        let factor = if unscaled.is_finite() { 1. } else { 0.5 };
        system[(i, i)] = (1. / scale) * factor;
        system[(i, 0)] += (filter.a[i + 1] / scale) * factor;
        if i + 1 < n {
            system[(i, i + 1)] = -(1. / scale) * factor;
        }
        rhs[i] = if unscaled.is_finite() {
            unscaled / scale
        } else {
            // The extra half-row factor bounds each term by MAX/2, including
            // opposite-sign b_i causing overflow without an overflowing product.
            libm::fma(
                -b0,
                (filter.a[i + 1] / scale) * factor,
                (bi / scale) * factor,
            )
        };
        if unscaled != 0. && rhs[i] == 0. {
            return Err("filtfilt initial-state RHS row scaling underflow is unsupported for this direct-form coefficient range".into());
        }
    }
    require_finite(
        rhs.as_slice(),
        "filtfilt initial-state right-hand side overflowed",
    )?;
    check_cancel(cancel)?;
    let state = system.clone().lu().solve(&rhs).ok_or(
        "filtfilt has singular steady-state initial conditions (for example a pole at DC)",
    )?;
    check_cancel(cancel)?;
    require_finite(
        state.as_slice(),
        "filtfilt initial-state solve is nonfinite",
    )?;
    let value_scale = state
        .iter()
        .chain(rhs.iter())
        .map(|v| v.abs())
        .fold(0_f64, f64::max);
    if value_scale > 0. {
        for i in 0..n {
            let mut residual = -rhs[i] / value_scale;
            let mut magnitude = residual.abs();
            for j in 0..n {
                let value = state[j] / value_scale;
                residual = libm::fma(system[(i, j)], value, residual);
                magnitude += (system[(i, j)] * value).abs();
            }
            if !residual.is_finite() || residual.abs() > 128. * f64::EPSILON * n as f64 * magnitude
            {
                return Err("filtfilt initial-state solve failed its scaled residual check".into());
            }
        }
    }
    let state: Vec<_> = state.iter().map(|value| value * numerator_scale).collect();
    require_finite(&state, "filtfilt rescaled initial state overflowed")?;
    Ok(state)
}

// Compare the exact dyadic product of finite f64 operands, without multiplying
// in f64. A zero FMA can mean either exact cancellation or underflow to zero.
fn exact_product_equals(a: f64, b: f64, c: f64) -> bool {
    if a == 0. || b == 0. {
        return c == 0.;
    }
    if c == 0. || (a.is_sign_negative() ^ b.is_sign_negative()) != c.is_sign_negative() {
        return false;
    }
    fn parts(value: f64) -> (u128, i32) {
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let fraction = (bits & 0x000f_ffff_ffff_ffff) as u128;
        if exponent == 0 {
            (fraction, -1074)
        } else {
            (fraction | (1_u128 << 52), exponent - 1075)
        }
    }
    let (ma, ea) = parts(a);
    let (mb, eb) = parts(b);
    let (mc, ec) = parts(c);
    let product = ma * mb;
    let pzeros = product.trailing_zeros();
    let czeros = mc.trailing_zeros();
    product >> pzeros == mc >> czeros && ea + eb + pzeros as i32 == ec + czeros as i32
}

#[cfg(test)]
mod initial_state_tests {
    use super::*;
    #[test]
    fn scaled_initial_state_preserves_extreme_products_and_cancellation_residuals() {
        let cancel = AtomicBool::new(false);
        for (b0, b1, a1, expected) in [
            (1e308, 0., 1e-308, -1.),
            // (1+2^-52)^2-(1+2^-51)=2^-104; divide by approximately2.
            (
                1_f64.next_up(),
                1_f64.next_up().next_up(),
                1_f64.next_up(),
                -2_f64.powi(-105),
            ),
            // Exact b0*a1=1: the residual2^-52 must survive row scaling.
            (
                f64::from_bits(0x7fe0_0000_0000_0000),
                1_f64.next_up(),
                f64::from_bits(0x0008_0000_0000_0000),
                f64::EPSILON,
            ),
            (0., 1., -1. + f64::EPSILON, 1. / f64::EPSILON),
            (1e-308, -1e-308, (-1_f64).next_up(), -1e-308),
            (1e-300, -1e-300, (-1_f64).next_up(), -1e-300),
            (
                f64::from_bits(1),
                -f64::from_bits(1),
                (-1_f64).next_up(),
                -f64::from_bits(1),
            ),
        ] {
            let filter = Filter::new(&[b0, b1], &[1., a1]).unwrap();
            assert_ne!(expected, 0.);
            let state = steady_state(&filter, &cancel).unwrap();
            assert!(
                (state[0] / expected - 1.).abs() <= 8. * f64::EPSILON,
                "b0={b0:?}, b1={b1:?}, a1={a1:?}, actual={:?}, expected={expected:?}",
                state[0]
            );
        }
        let filter = Filter::new(&[f64::MAX, 0.], &[1., 2.]).unwrap();
        let expected = -(f64::MAX / 3.) * 2.;
        let state = steady_state(&filter, &cancel).unwrap();
        assert!((state[0] / expected - 1.).abs() <= 8. * f64::EPSILON);
        let filter = Filter::new(&[f64::MAX, -f64::MAX], &[1., 1.]).unwrap();
        let state = steady_state(&filter, &cancel).unwrap();
        assert_eq!(state[0], -f64::MAX);
        assert!(exact_product_equals(-0., f64::MAX, 0.));
        assert!(exact_product_equals(
            f64::from_bits(1),
            2.,
            f64::from_bits(2)
        ));
        assert!(!exact_product_equals(f64::from_bits(1), 0.5, 0.));
        assert!(!exact_product_equals(1., 1., -1.));
        let filter = Filter::new(&[f64::from_bits(1), 0., 1.], &[1., 0.5]).unwrap();
        assert!(
            steady_state(&filter, &cancel)
                .unwrap_err()
                .contains("RHS underflow")
        );
    }
}

// This entire probe, its calls and its test are absent from non-test builds.
// Thread-local state prevents unrelated concurrently running tests interfering.
#[cfg(test)]
mod active_cancellation_tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};

    #[derive(Clone, Copy)]
    struct Probe {
        armed: bool,
        stop_after: usize,
        completed_chunks: usize,
        completed_samples: usize,
    }
    const DISARMED: Probe = Probe {
        armed: false,
        stop_after: 0,
        completed_chunks: 0,
        completed_samples: 0,
    };
    thread_local! {
        static PROBE: Cell<Probe> = const { Cell::new(DISARMED) };
    }
    struct ArmedProbe;
    impl Drop for ArmedProbe {
        fn drop(&mut self) {
            PROBE.with(|probe| probe.set(DISARMED));
        }
    }
    fn arm(stop_after: usize) -> ArmedProbe {
        assert!(stop_after > 0);
        PROBE.with(|probe| {
            assert!(!probe.get().armed);
            probe.set(Probe {
                armed: true,
                stop_after,
                ..DISARMED
            });
        });
        ArmedProbe
    }
    pub(super) fn after_completed_chunk(cancel: &AtomicBool, samples: usize) {
        PROBE.with(|probe| {
            let mut state = probe.get();
            if !state.armed {
                return;
            }
            state.completed_chunks += 1;
            state.completed_samples += samples;
            if state.completed_chunks == state.stop_after {
                cancel.store(true, Ordering::Relaxed);
            }
            probe.set(state);
        });
    }

    #[test]
    fn active_cancellation_stops_forward_backward_and_gain_chunks_without_publication() {
        // A single channel deliberately avoids a pool or scheduler dependency.
        // Filtering completes real chunks before the probe raises cancellation.
        let x =
            Matrix::from_column_vec(8192, 1, (0..8192).map(|i| (i as f64 / 97.).sin()).collect())
                .unwrap();
        let alias = x.clone();
        let before = x.data().to_vec();
        let pointer = x.data().as_ptr();
        let cancel = AtomicBool::new(false);
        for (b, a, triggers) in [
            (
                vec![0.25, 0.5, 0.25],
                vec![1.],
                vec![(3, 2054), (13, 10258)],
            ),
            (
                vec![0.2, 0.1, 0.2],
                vec![1., -0.4, 0.1],
                vec![(3, 2054), (13, 10258)],
            ),
            (vec![0.5], vec![1.], vec![(1, 4096)]),
        ] {
            let reference = filtfilt(&b, &a, &x, &cancel).unwrap();
            for (stop_after, samples) in triggers {
                assert!(!cancel.load(Ordering::Relaxed));
                let armed = arm(stop_after);
                let error = filtfilt(&b, &a, &x, &cancel).unwrap_err();
                let progress = PROBE.with(Cell::get);
                assert!(error.contains("cancelled"), "{error}");
                assert!(cancel.load(Ordering::Relaxed));
                // No later chunk may complete after cancellation is raised.
                assert_eq!(progress.completed_chunks, stop_after);
                assert_eq!(progress.completed_samples, samples);
                assert_eq!(x.data().as_ptr(), pointer);
                assert_eq!(alias.data().as_ptr(), pointer);
                assert_eq!(x.data(), before);
                assert_eq!(alias.data(), before);
                drop(armed); // Also resets during unwinding through the guard.
                cancel.store(false, Ordering::Relaxed);
                let recovered = filtfilt(&b, &a, &x, &cancel).unwrap();
                assert_eq!(
                    recovered
                        .data()
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    reference
                        .data()
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
