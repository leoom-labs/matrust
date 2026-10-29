//! Real 2-D SOS forward/backward filtering. Whole-cascade boundary semantics.
use super::filtfilt::{initialize, require_finite, reverse_checked, steady_state};
use super::{Filter, MAX_WORK, check_cancel, sos_sections};
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

// Provisional shared-pool threshold; parent release measurements must verify it.
const PARALLEL_WORK: usize = 1_000_000;

/// Current documented MATLAB matrix/gain syntax, with a whole-cascade
/// implementation awaiting SciPy validation. MATLAB numerical parity is unverified.
/// SOS requires 1..64 rows; scalar or exactly L+1 scale values are supported.
pub fn filtfilt_sos(
    sos: &Matrix,
    gains: &[f64],
    x: &Matrix,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    check_cancel(cancel)?;
    let mut prototype = sos_sections(sos)?;
    let count = prototype.len();
    if !matches!(gains.len(), 1) && gains.len() != count + 1 {
        return Err(
            "filtfilt SOS gain must be a scalar or a vector with sections+1 elements".into(),
        );
    }
    require_finite(gains, "filtfilt SOS gains must be finite")?;
    let overall = gains[if gains.len() == 1 { 0 } else { count }];
    let uniform = libm::pow(overall.abs(), 1. / count as f64);
    for (index, section) in prototype.iter_mut().enumerate() {
        check_cancel(cancel)?;
        let local = if gains.len() == 1 { 1. } else { gains[index] };
        for coefficient in std::sync::Arc::make_mut(&mut section.b) {
            // Multiply finite factors through exponent decomposition. This
            // avoids an overflowing intermediate when the final product fits.
            *coefficient = multiply_three(*coefficient, local, uniform);
            if index + 1 == count && overall.is_sign_negative() {
                *coefficient = -*coefficient;
            }
        }
        require_finite(
            &section.b,
            "filtfilt SOS gain-scaled coefficients overflowed",
        )?;
    }
    // Degrees of polynomial products add. No expansion into an ill-conditioned
    // high-order transfer function is needed, and exact trailing zeros count.
    let numerator_zero = prototype.iter().any(|s| s.b.iter().all(|v| *v == 0.));
    let numerator_degree = if numerator_zero {
        0
    } else {
        prototype.iter().map(|s| last_degree(&s.b)).sum::<usize>()
    };
    let denominator_degree = prototype.iter().map(|s| last_degree(&s.a)).sum::<usize>();
    let padding = 3 * numerator_degree.max(denominator_degree);
    if x.is_empty() {
        return Ok(x.clone());
    }
    let (samples, channels) = if x.rows() == 1 {
        (x.cols(), 1)
    } else {
        (x.rows(), x.cols())
    };
    if samples <= padding {
        return Err(format!(
            "filtfilt SOS requires more than {padding} samples per channel"
        ));
    }
    let work = (2 * samples + 3 * padding)
        .saturating_mul(channels)
        .saturating_mul(count * 3);
    if work.saturating_add(count * 8) > MAX_WORK {
        return Err(
            "filtfilt SOS exceeds the bounded work limit; reduce channels, samples or sections"
                .into(),
        );
    }
    // Only two states per section: reuse the proven, range-checked LU/FIR
    // initial-state calculation. Never form a cascade polynomial or inverse.
    let mut initial = Vec::with_capacity(count);
    let mut dc = Vec::with_capacity(count);
    for section in &prototype {
        check_cancel(cancel)?;
        initial.push(sos_initial_state(section, cancel)?);
        let numerator = finite_sum(&section.b)?;
        let denominator = finite_sum(&section.a)?;
        if denominator == 0. {
            return Err("filtfilt SOS has a singular DC steady state".into());
        }
        dc.push((numerator, denominator));
    }
    let mut output = Vec::with_capacity(x.len());
    #[cfg(test)]
    cancellation_tests::output_allocation();
    for chunk in x.data().chunks(4096) {
        check_cancel(cancel)?;
        require_finite(chunk, "filtfilt SOS input must be finite")?;
        output.extend_from_slice(chunk);
    }
    let process = |state: &mut (Vec<Filter>, Vec<f64>), values: &mut [f64]| -> Result<()> {
        let (sections, edge) = state;
        check_cancel(cancel)?;
        if padding == 0 {
            initialize_cascade(sections, &initial, &dc, values[0])?;
            process_cascade(sections, values, cancel)?;
            reverse_checked(values, cancel)?;
            initialize_cascade(sections, &initial, &dc, values[0])?;
            process_cascade(sections, values, cancel)?;
            reverse_checked(values, cancel)?;
            return check_cancel(cancel);
        }
        edge.clear();
        for i in (1..=padding).rev() {
            edge.push(libm::fma(values[0], 2., -values[i]));
        }
        require_finite(edge, "filtfilt SOS reflection overflowed")?;
        initialize_cascade(sections, &initial, &dc, edge[0])?;
        process_cascade(sections, edge, cancel)?;
        edge.clear();
        let last = values.len() - 1;
        for i in 1..=padding {
            edge.push(libm::fma(values[last], 2., -values[last - i]));
        }
        require_finite(edge, "filtfilt SOS reflection overflowed")?;
        process_cascade(sections, values, cancel)?;
        process_cascade(sections, edge, cancel)?;
        reverse_checked(edge, cancel)?;
        initialize_cascade(sections, &initial, &dc, edge[0])?;
        process_cascade(sections, edge, cancel)?;
        reverse_checked(values, cancel)?;
        process_cascade(sections, values, cancel)?;
        reverse_checked(values, cancel)?;
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

fn last_degree(values: &[f64]) -> usize {
    values.iter().rposition(|v| *v != 0.).unwrap_or(0)
}

fn multiply_three(a: f64, b: f64, c: f64) -> f64 {
    let (a, ae) = libm::frexp(a);
    let (b, be) = libm::frexp(b);
    let (c, ce) = libm::frexp(c);
    libm::scalbn((a * b) * c, ae + be + ce)
}

fn finite_sum(values: &[f64]) -> Result<f64> {
    let (mut sum, mut correction) = (0_f64, 0_f64);
    for &value in values {
        let next = sum + value;
        if !next.is_finite() {
            return Err(
                "filtfilt SOS DC coefficient sum exceeds the supported finite range".into(),
            );
        }
        correction += if sum.abs() >= value.abs() {
            (sum - next) + value
        } else {
            (value - next) + sum
        };
        sum = next;
    }
    let result = sum + correction;
    if !result.is_finite() {
        return Err("filtfilt SOS DC coefficient sum exceeds the supported finite range".into());
    }
    Ok(result)
}

fn initialize_cascade(
    sections: &mut [Filter],
    initial: &[Vec<f64>],
    dc: &[(f64, f64)],
    mut endpoint: f64,
) -> Result<()> {
    for (index, section) in sections.iter_mut().enumerate() {
        initialize(section, &initial[index], endpoint)?;
        if index + 1 < initial.len() {
            let (e, ee) = libm::frexp(endpoint);
            let (n, ne) = libm::frexp(dc[index].0);
            let (d, de) = libm::frexp(dc[index].1);
            endpoint = libm::scalbn((e * n) / d, ee + ne - de);
            if !endpoint.is_finite() {
                return Err("filtfilt SOS intermediate DC endpoint overflowed".into());
            }
        }
    }
    Ok(())
}

fn process_cascade(sections: &mut [Filter], values: &mut [f64], cancel: &AtomicBool) -> Result<()> {
    for chunk in values.chunks_mut(1024) {
        for section in sections.iter_mut() {
            check_cancel(cancel)?;
            // Existing fixed-size DF-II kernel, retaining nonzero state across
            // chunks. The old zero-state SIMD pipeline cannot be reused here.
            section.process(chunk, cancel)?;
            require_finite(chunk, "filtfilt SOS recurrence overflowed")?;
            require_finite(&section.state, "filtfilt SOS recurrence state overflowed")?;
        }
        #[cfg(test)]
        cancellation_tests::after_chunk(cancel);
    }
    check_cancel(cancel)
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};
    thread_local! {
        static REMAINING: Cell<Option<usize>> = const { Cell::new(None) };
        static OUTPUT_ALLOCATED: Cell<bool> = const { Cell::new(false) };
    }
    pub(super) fn output_allocation() {
        OUTPUT_ALLOCATED.with(|value| value.set(true));
    }
    pub(super) fn after_chunk(cancel: &AtomicBool) {
        REMAINING.with(|remaining| {
            if let Some(n) = remaining.get() {
                if n == 1 {
                    remaining.set(None);
                    cancel.store(true, Ordering::Relaxed);
                } else {
                    remaining.set(Some(n - 1));
                }
            }
        });
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REMAINING.with(|n| n.set(None));
        }
    }
    #[test]
    fn invalid_gain_short_signal_and_work_bound_reject_before_large_output_allocation() {
        let c = AtomicBool::new(false);
        let sos = Matrix::from_column_slice(
            2,
            6,
            &[0.5, 0.5, 0., 0., 0., 0., 1., 1., -0.5, -0.5, 0., 0.],
        )
        .unwrap();
        let short = Matrix::from_column_vec(6, 1, vec![1.; 6]).unwrap();
        for gains in [&[1.][..], &[1., 1.][..]] {
            OUTPUT_ALLOCATED.with(|value| value.set(false));
            assert!(filtfilt_sos(&sos, gains, &short, &c).is_err());
            OUTPUT_ALLOCATED.with(|value| assert!(!value.get()));
        }
        let identity = [1., 0., 0., 1., 0., 0.];
        let many = Matrix::from_column_vec(
            64,
            6,
            (0..6)
                .flat_map(|c| std::iter::repeat_n(identity[c], 64))
                .collect(),
        )
        .unwrap();
        let input = Matrix::from_column_vec(1_000_000, 1, vec![1.; 1_000_000]).unwrap();
        OUTPUT_ALLOCATED.with(|value| value.set(false));
        assert!(
            filtfilt_sos(&many, &[1.], &input, &c)
                .unwrap_err()
                .contains("work limit")
        );
        OUTPUT_ALLOCATED.with(|value| assert!(!value.get()));
    }
    #[test]
    fn cancelled_after_completed_cascade_chunks_never_publishes_partial_output() {
        // One channel pins this test hook to the caller, independent of Rayon.
        let sos = Matrix::from_column_slice(
            2,
            6,
            &[0.5, 0.5, 0., 0., 0., 0., 1., 1., -0.5, -0.5, 0., 0.],
        )
        .unwrap();
        let data: Vec<_> = (0..8192).map(|i| (i as f64 * 0.01).sin()).collect();
        let x = Matrix::from_column_slice(8192, 1, &data).unwrap();
        let alias = x.clone();
        let cancel = AtomicBool::new(false);
        let reference = filtfilt_sos(&sos, &[1.], &x, &cancel).unwrap();
        for completed_chunks in [2, 12] {
            REMAINING.with(|n| n.set(Some(completed_chunks)));
            let _reset = Reset;
            cancel.store(false, Ordering::Relaxed);
            assert!(
                filtfilt_sos(&sos, &[1.], &x, &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert!(cancel.load(Ordering::Relaxed));
            REMAINING.with(|n| assert_eq!(n.get(), None));
            assert_eq!(x.data(), data);
            assert_eq!(alias.data(), data);
        }
        cancel.store(false, Ordering::Relaxed);
        let retried = filtfilt_sos(&sos, &[1.], &x, &cancel).unwrap();
        assert!(
            reference
                .data()
                .iter()
                .zip(retried.data())
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }
}

// Exploit the two-state SOS structure while preserving the existing proven
// solver as a fallback for cancellation/range cases this fast path cannot
// handle conservatively. This changes preparation only, never the sample recurrence.
fn sos_initial_state(section: &Filter, cancel: &AtomicBool) -> Result<Vec<f64>> {
    check_cancel(cancel)?;
    let candidate = analytical_initial_state(section);
    check_cancel(cancel)?;
    match candidate {
        Some(state) => Ok(state),
        None => steady_state(section, cancel),
    }
}

fn analytical_initial_state(section: &Filter) -> Option<Vec<f64>> {
    if section.state.len() != 2 {
        return None;
    }
    let largest = section.b.iter().map(|v| v.abs()).fold(0_f64, f64::max);
    if largest == 0. {
        return None;
    }
    // Exact power-of-two boost only: division cannot underflow any coefficient.
    // This is the existing steady_state numerator normalization policy.
    let scale = if largest < 1. {
        let bits = largest.to_bits();
        let exponent = bits & 0x7ff0_0000_0000_0000;
        f64::from_bits(if exponent != 0 {
            exponent
        } else {
            1_u64 << (63 - bits.leading_zeros())
        })
    } else {
        1.
    };
    let b0 = section.b[0] / scale;
    let b1 = section.b[1] / scale;
    let b2 = section.b[2] / scale;
    let a1 = section.a[1];
    let a2 = section.a[2];
    let denominator = finite_sum(&section.a).ok()?;
    if !denominator.is_normal() {
        return None;
    }
    let rhs0 = libm::fma(-b0, a1, b1);
    let rhs1 = libm::fma(-b0, a2, b2);
    // A rounded zero or subnormal cancellation residual can hide a meaningful
    // finite solution near DC. Delegate these cases to the established solver.
    for (rhs, ai, bi) in [(rhs0, a1, b1), (rhs1, a2, b2)] {
        if !normal_or_zero(rhs) || (rhs == 0. && (bi != 0. || (b0 != 0. && ai != 0.))) {
            return None;
        }
    }
    let numerator = finite_sum(&[rhs0, rhs1]).ok()?;
    if !normal_or_zero(numerator) || (numerator == 0. && (rhs0 != 0. || rhs1 != 0.)) {
        return None;
    }
    // Each FMA RHS is rounded once. If opposite RHS terms lose more than one
    // magnitude bit on addition, those roundoffs can dominate a small state.
    // Keep that cancellation regime on the existing solver rather than claim
    // componentwise accuracy from a small backward residual.
    if rhs0.is_sign_negative() != rhs1.is_sign_negative()
        && numerator.abs() < 0.5 * rhs0.abs().max(rhs1.abs())
    {
        return None;
    }
    let z0 = numerator / denominator;
    if !normal_or_zero(z0) || (z0 == 0. && numerator != 0.) {
        return None;
    }
    let z1 = libm::fma(-a2, z0, rhs1);
    if !normal_or_zero(z1) || (z1 == 0. && (rhs1 != 0. || (a2 != 0. && z0 != 0.))) {
        return None;
    }
    let product = a2 * z0;
    if !normal_or_zero(product) || (product == 0. && a2 != 0. && z0 != 0.) {
        return None;
    }
    if product.is_sign_negative() == rhs1.is_sign_negative()
        && z1.abs() < 0.5 * product.abs().max(rhs1.abs())
    {
        return None;
    }
    // Check coefficient equations with the already-rounded RHS, without first
    // rounding 1+a1 into a matrix entry. This is a backward check, not an exact
    // RHS or componentwise forward-error certificate. Scale states/RHS by one.
    let unit = z0.abs().max(z1.abs()).max(rhs0.abs()).max(rhs1.abs());
    if unit == 0. || !unit.is_finite() {
        return None;
    }
    let terms = [z0 / unit, z1 / unit, rhs0 / unit, rhs1 / unit];
    for (raw, scaled) in [z0, z1, rhs0, rhs1].into_iter().zip(terms) {
        if !normal_or_zero(scaled) || (raw != 0. && scaled == 0.) {
            return None;
        }
    }
    let [s0, s1, r0, r1] = terms;
    let residual0 = finite_sum(&[s0, libm::fma(a1, s0, -r0), -s1]).ok()?;
    let residual1 = libm::fma(a2, s0, s1 - r1);
    let magnitude0 = s0.abs() + (a1 * s0).abs() + s1.abs() + r0.abs();
    let magnitude1 = (a2 * s0).abs() + s1.abs() + r1.abs();
    for (residual, magnitude) in [(residual0, magnitude0), (residual1, magnitude1)] {
        if !residual.is_finite()
            || !magnitude.is_finite()
            || residual.abs() > 256. * f64::EPSILON * magnitude
        {
            return None;
        }
    }
    let state = [z0 * scale, z1 * scale];
    for (raw, rescaled) in [z0, z1].into_iter().zip(state) {
        if !normal_or_zero(rescaled) || (raw != 0. && rescaled == 0.) {
            return None;
        }
    }
    Some(state.to_vec())
}

fn normal_or_zero(value: f64) -> bool {
    value == 0. || value.is_normal()
}

#[cfg(test)]
mod analytical_initial_state_tests {
    use super::*;
    #[test]
    fn two_state_equations_and_cancellation_are_preserved() {
        let section = Filter::new(&[1., 2., 3.], &[1., -0.5, -0.25]).unwrap();
        assert_eq!(analytical_initial_state(&section), Some(vec![23., 9.]));
        assert_eq!(
            sos_initial_state(&section, &AtomicBool::new(false)).unwrap(),
            vec![23., 9.]
        );
        assert!(
            sos_initial_state(&section, &AtomicBool::new(true))
                .unwrap_err()
                .contains("cancelled")
        );
    }
    #[test]
    fn near_dc_double_pole_retains_exact_representable_initial_states() {
        let q = 2_f64.powi(-42);
        let section = Filter::new(
            &[q, 2. * q, q],
            &[1., -2. + 2_f64.powi(-19), 1. - 2_f64.powi(-19) + 4. * q],
        )
        .unwrap();
        let expected = [1. - q, -1. + 2_f64.powi(-19) - 3. * q];
        let actual =
            analytical_initial_state(&section).expect("Near-DC case exercises analytical path");
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }
    #[test]
    fn range_and_ambiguous_cancellation_delegate_to_existing_solver() {
        let cancel = AtomicBool::new(false);
        for (b, a) in [
            ([f64::MAX, -f64::MAX, 0.], [1., 1., 0.]),
            ([1e-308, -1e-308, 0.], [1., (-1_f64).next_up(), 0.]),
            ([f64::from_bits(1), 0., 1.], [1., 0.5, 0.]),
            ([0., 0., 0.], [1., -0.5, 0.25]),
            ([1., -1., 0.], [1., -1., 0.]),
            // Stable poles; rounded opposite RHS terms would halve tiny z0.
            (
                [1., 2_f64.powi(-54), 0.25 + 2_f64.powi(-54)],
                [1., 0.5, -0.25],
            ),
        ] {
            let section = Filter::new(&b, &a).unwrap();
            assert!(analytical_initial_state(&section).is_none(), "{b:?} {a:?}");
            match (
                sos_initial_state(&section, &cancel),
                steady_state(&section, &cancel),
            ) {
                (Ok(actual), Ok(expected)) => assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                ),
                (Err(actual), Err(expected)) => assert_eq!(actual, expected),
                result => panic!("Fallback changed existing behavior: {result:?}"),
            }
        }
    }
}
