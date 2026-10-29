//! Engine-independent floating-point, linear-solve, Jacobian and event helpers.
//! IEEE 754 spacing; Higham, Accuracy and Stability of Numerical Algorithms
//! (2002), chapters 2 and 6; Salane, SAND86-1319 (1986); Shampine and Reichelt,
//! The MATLAB ODE Suite (1997), section 5; Shampine, Gladwell and Brankin,
//! Reliable solution of special event location problems for ODEs (1991).
#![allow(clippy::needless_range_loop)]

pub type R<T> = std::result::Result<T, String>;

/// Event values, terminal flags and directions at (t, y).
pub type Events<'a> = dyn FnMut(f64, &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> + 'a;

pub fn eps_of(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let exponent = ((x.to_bits() >> 52) & 0x7ff) as i32;
    if exponent <= 52 {
        f64::from_bits(1u64 << (exponent - 1).max(0))
    } else {
        f64::from_bits(((exponent - 52) as u64) << 52)
    }
}

pub fn norm_inf(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut norm = 0f64;
    for value in values {
        if value.is_nan() {
            return f64::NAN;
        }
        norm = norm.max(value.abs());
    }
    norm
}

/// Scaled sum of squares avoids overflow/underflow in intermediate squares
/// (Higham, 2002, chapter 6).
pub fn norm2(values: &[f64]) -> f64 {
    let scale = norm_inf(values.iter().copied());
    if scale == 0. || !scale.is_finite() {
        return scale;
    }
    scale
        * values
            .iter()
            .map(|v| (v / scale).powi(2))
            .sum::<f64>()
            .sqrt()
}

pub fn mmax(a: f64, b: f64) -> f64 {
    a.max(b)
}

pub fn mmin(a: f64, b: f64) -> f64 {
    a.min(b)
}

pub fn sign(x: f64) -> f64 {
    if x == 0. { 0. } else { x.signum() }
}

pub fn matvec(a: &[f64], n: usize, v: &[f64]) -> Vec<f64> {
    let matrix = nalgebra::DMatrixView::from_slice(a, n, n);
    let vector = nalgebra::DVectorView::from_slice(v, n);
    (matrix * vector).as_slice().to_vec()
}

/// Reusable partial-pivoting LU factors, formed by nalgebra's dense kernel.
pub struct Lu {
    n: usize,
    /// Column-major combined factors: unit lower L below the diagonal, U on and above.
    lu: Vec<f64>,
    perm: Vec<usize>,
}

impl Lu {
    pub fn new(a: &[f64], n: usize) -> Lu {
        let factor = nalgebra::DMatrix::from_column_slice(n, n, a).lu();
        let mut permutation = nalgebra::DVector::from_iterator(n, 0..n);
        factor.p().permute_rows(&mut permutation);
        Lu {
            n,
            lu: factor.lu_internal().as_slice().to_vec(),
            perm: permutation.as_slice().to_vec(),
        }
    }
    pub fn solve(&self, b: &[f64]) -> Vec<f64> {
        let mut result = nalgebra::DVector::from_iterator(self.n, self.perm.iter().map(|&i| b[i]));
        let factors = nalgebra::DMatrixView::from_slice(&self.lu, self.n, self.n);
        factors.solve_lower_triangular_with_diag_mut(&mut result, 1.);
        // A singular iteration matrix must produce a nonfinite correction,
        // which the caller's existing convergence check rejects.
        if !factors.solve_upper_triangular_mut(&mut result) {
            result.fill(f64::NAN);
        }
        result.as_slice().to_vec()
    }
}

pub fn solve(a: &[f64], n: usize, b: &[f64]) -> Vec<f64> {
    Lu::new(a, n).solve(b)
}

/// Forward differences with remembered, adaptively scaled increments.
/// Salane (1986), differencing-increment strategy, and Shampine & Reichelt
/// (1997), section 5. Each column normally costs one callback; columns whose
/// difference is lost in rounding are retried at a larger increment.
pub fn numjac(
    f: &mut dyn FnMut(&[f64]) -> R<Vec<f64>>,
    y: &[f64],
    fvalue: &[f64],
    thresh: &[f64],
    fac: &mut Vec<f64>,
) -> R<(Vec<f64>, usize)> {
    let n = y.len();
    let m = fvalue.len();
    if thresh.len() != 1 && thresh.len() != n {
        return Err("Numerical Jacobian threshold has the wrong length.".into());
    }
    let length = n.checked_mul(m).ok_or("Numerical Jacobian is too large.")?;
    let epsilon = f64::EPSILON;
    let lower = epsilon.powf(0.75);
    let upper = 0.1;
    let small = epsilon.powf(0.75);
    let enlarge = epsilon.powf(0.5);
    let shrink = epsilon.powf(0.25);
    if fac.len() != n {
        *fac = vec![epsilon.sqrt(); n];
    }
    let mut jac = vec![0.; length];
    let mut calls = 0;
    let mut perturbed = y.to_vec();
    for j in 0..n {
        let scale = y[j]
            .abs()
            .max(thresh[if thresh.len() == 1 { 0 } else { j }].abs());
        let scale = scale.max(f64::MIN_POSITIVE);
        let direction = if m == n && fvalue[j] < 0. { -1. } else { 1. };
        let evaluate = |factor: f64,
                        point: &mut [f64],
                        f: &mut dyn FnMut(&[f64]) -> R<Vec<f64>>|
         -> R<(f64, Vec<f64>)> {
            let proposed = y[j] + direction * factor * scale;
            point[j] = if proposed == y[j] {
                y[j] + direction * eps_of(y[j])
            } else {
                proposed
            };
            let delta = point[j] - y[j];
            let result = f(point)?;
            if result.len() != m {
                return Err("Numerical Jacobian function changed output length.".into());
            }
            Ok((delta, result))
        };
        fac[j] = fac[j].clamp(lower, upper);
        let (mut delta, mut value) = evaluate(fac[j], &mut perturbed, f)?;
        calls += 1;
        let largest = |value: &[f64]| -> (usize, f64) {
            value
                .iter()
                .zip(fvalue)
                .enumerate()
                .map(|(i, (a, b))| (i, (a - b).abs()))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((0, 0.))
        };
        let (mut row, mut difference) = largest(&value);
        let mut magnitude = if m == 0 {
            0.
        } else {
            value[row].abs().max(fvalue[row].abs())
        };
        if difference <= small * magnitude {
            let candidate = fac[j].sqrt().min(upper);
            if candidate > fac[j] {
                let (new_delta, new_value) = evaluate(candidate, &mut perturbed, f)?;
                calls += 1;
                let (new_row, new_difference) = largest(&new_value);
                // Prefer the estimate with the larger resolved signal per increment.
                if new_difference * delta.abs() >= difference * new_delta.abs() {
                    delta = new_delta;
                    value = new_value;
                    row = new_row;
                    difference = new_difference;
                    magnitude = if m == 0 {
                        0.
                    } else {
                        value[row].abs().max(fvalue[row].abs())
                    };
                    fac[j] = candidate;
                }
            }
        }
        for i in 0..m {
            jac[i + j * m] = (value[i] - fvalue[i]) / delta;
        }
        if difference <= enlarge * magnitude {
            fac[j] = (10. * fac[j]).min(upper);
        } else if difference > shrink * magnitude {
            fac[j] = (0.1 * fac[j]).max(lower);
        }
        perturbed[j] = y[j];
    }
    Ok((jac, calls))
}

/// One located event: time, state and 1-based event index.
pub struct Located {
    pub t: Vec<f64>,
    pub y: Vec<Vec<f64>>,
    pub i: Vec<usize>,
    pub stop: bool,
}

pub fn validate_events(v: &[f64], term: &[f64], dir: &[f64], n: usize) -> R<()> {
    if v.len() != n || term.len() != n || (!dir.is_empty() && dir.len() != n) {
        return Err("Event function outputs must have consistent lengths.".into());
    }
    if v.iter().chain(term).chain(dir).any(|x| !x.is_finite()) {
        return Err("Event function outputs must be finite.".into());
    }
    Ok(())
}

/// Bracketed inverse quadratic interpolation, guarded by bisection.
/// Brent, Algorithms for Minimization without Derivatives (1973), chapter 4.
fn event_root(
    f: &mut dyn FnMut(f64) -> R<f64>,
    mut a: f64,
    mut fa: f64,
    mut b: f64,
    mut fb: f64,
) -> R<f64> {
    if fa == 0. {
        return Ok(a);
    }
    if fb == 0. {
        return Ok(b);
    }
    // Brent swaps endpoints to keep the smaller residual at b. A terminal
    // event must instead return zero or the new sign along the original step,
    // so restarting there cannot encounter this same crossing again.
    let crossed_sign = fb.is_sign_positive();
    let crossed_endpoint = |a, b, fb: f64| {
        if fb == 0. || fb.is_sign_positive() == crossed_sign {
            b
        } else {
            a
        }
    };
    if fa.abs() < fb.abs() {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut fa, &mut fb);
    }
    let mut c = a;
    let mut fc = fa;
    let mut bisected = true;
    let mut d = c;
    // Binary64 spans over a thousand binades. Allow safeguarded contractions
    // to cross that range when the root is tiny relative to the initial bracket.
    for _ in 0..4096 {
        let tolerance = 4. * eps_of(a.abs().max(b.abs()));
        if fb == 0. || (b - a).abs() <= tolerance {
            return Ok(crossed_endpoint(a, b, fb));
        }
        let mut trial = if fa != fc && fb != fc {
            a * (fb / (fa - fb)) * (fc / (fa - fc))
                + b * (fa / (fb - fa)) * (fc / (fb - fc))
                + c * (fa / (fc - fa)) * (fb / (fc - fb))
        } else {
            b - fb * ((b - a) / (fb - fa))
        };
        let guard = a + 0.25 * (b - a);
        let inside = trial > guard.min(b) && trial < guard.max(b);
        let movement = if bisected {
            (b - c).abs()
        } else {
            (c - d).abs()
        };
        let use_bisection = !trial.is_finite()
            || !inside
            || (trial - b).abs() >= 0.5 * movement
            || movement <= tolerance;
        if use_bisection {
            trial = a + 0.5 * (b - a);
        }
        if trial == a || trial == b {
            return Ok(crossed_endpoint(a, b, fb));
        }
        let ft = f(trial)?;
        d = c;
        c = b;
        fc = fb;
        bisected = use_bisection;
        if (fa > 0.) != (ft > 0.) {
            b = trial;
            fb = ft;
        } else {
            a = trial;
            fa = ft;
        }
        if fa.abs() < fb.abs() {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut fa, &mut fb);
        }
    }
    Err("Event location did not converge.".into())
}

/// Locate sign-changing events on the accepted continuous solution, then order
/// them in integration direction and retain all events at the first terminal
/// time. Shampine, Gladwell & Brankin (1991), event-location formulation.
pub fn odezero(
    ntrp: &mut dyn FnMut(f64) -> Vec<f64>,
    events: &mut Events,
    t0: f64,
    v: &[f64],
    t: f64,
    tnew: f64,
    ynew: &[f64],
) -> R<(Located, Vec<f64>)> {
    let (right, terminal, direction) = events(tnew, ynew)?;
    validate_events(&right, &terminal, &direction, v.len())?;
    let mut found = Located {
        t: Vec::new(),
        y: Vec::new(),
        i: Vec::new(),
        stop: false,
    };
    let forward = sign(tnew - t);
    let mut roots = Vec::new();
    for j in 0..v.len() {
        let slope = sign(right[j] - v[j]);
        let dir = sign(direction.get(j).copied().unwrap_or(0.));
        if (dir != 0. && dir != slope)
            || (v[j] != 0. && right[j] != 0. && sign(v[j]) == sign(right[j]))
        {
            continue;
        }
        if v[j] == 0. {
            // Record an initial zero once without stopping. A later left-endpoint
            // zero was already reported by the preceding accepted step.
            if t == t0 {
                roots.push((t, j, false));
            }
            continue;
        }
        let root = event_root(
            &mut |x| {
                let state = ntrp(x);
                let (values, terms, dirs) = events(x, &state)?;
                validate_events(&values, &terms, &dirs, v.len())?;
                Ok(values[j])
            },
            t,
            v[j],
            tnew,
            right[j],
        )?;
        roots.push((root, j, terminal[j] != 0.));
    }
    roots.sort_by(|a, b| {
        (forward * a.0)
            .total_cmp(&(forward * b.0))
            .then(a.1.cmp(&b.1))
    });
    let terminal_time = roots.iter().find(|r| r.2).map(|r| r.0);
    // Count only the events retained at/before the first terminal root. Bound
    // state columns plus time/index vectors before constructing any states,
    // using the same 32M-element output budget as the solver modules.
    let retained = roots
        .iter()
        .take_while(|root| {
            terminal_time.is_none_or(|limit| forward * (root.0 - limit) <= 8. * eps_of(limit))
        })
        .count();
    if retained.saturating_mul(ynew.len().saturating_add(2)) > 32 * 1024 * 1024 {
        return Err("Event output exceeds the 33554432-element limit.".into());
    }
    for (time, j, stop) in roots {
        if terminal_time.is_some_and(|limit| forward * (time - limit) > 8. * eps_of(limit)) {
            break;
        }
        found.t.push(time);
        found.y.push(if time == tnew {
            ynew.to_vec()
        } else {
            ntrp(time)
        });
        found.i.push(j + 1);
        found.stop |= stop;
    }
    Ok((found, right))
}

/// General-format output with six significant decimal digits.
pub fn fmt_g(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == f64::INFINITY {
        return "Inf".into();
    }
    if x == f64::NEG_INFINITY {
        return "-Inf".into();
    }
    if x == 0. {
        return if x.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let scientific = format!("{x:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    if !(-4..6).contains(&exponent) {
        format!(
            "{}e{}{:02}",
            trim_zeros(mantissa),
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        )
    } else {
        let places = (5 - exponent).max(0) as usize;
        let fixed = format!("{x:.places$}");
        trim_zeros(&fixed).to_string()
    }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_and_scaled_norm_cover_binary64_extremes() {
        assert_eq!(eps_of(0.), f64::from_bits(1));
        assert_eq!(eps_of(f64::from_bits(1)), f64::from_bits(1));
        assert_eq!(eps_of(f64::MIN_POSITIVE), f64::from_bits(1));
        assert_eq!(eps_of(1.), f64::EPSILON);
        assert_eq!(eps_of(-2.), 2. * f64::EPSILON);
        assert_eq!(eps_of(f64::MAX), 2f64.powi(971));
        assert!(eps_of(f64::INFINITY).is_nan());
        assert!((norm2(&[3e200, 4e200]) / 5e200 - 1.).abs() < 1e-15);
        assert!((norm2(&[3e-200, 4e-200]) / 5e-200 - 1.).abs() < 1e-15);
        assert!(norm_inf([f64::NAN, -2., 1.]).is_nan());
        assert_eq!(mmin(f64::NAN, 2.), 2.);
    }

    #[test]
    fn norms_propagate_nan_in_any_position_even_with_infinity() {
        for values in [
            [f64::NAN, -2., 1.],
            [1., f64::NAN, -2.],
            [-2., 1., f64::NAN],
            [f64::NAN, 0., 0.],
            [f64::INFINITY, f64::NAN, f64::NEG_INFINITY],
        ] {
            assert!(norm_inf(values).is_nan(), "{values:?}");
            assert!(norm2(&values).is_nan(), "{values:?}");
        }
        assert_eq!(norm_inf([]), 0.);
        assert_eq!(norm2(&[]), 0.);
        assert_eq!(norm_inf([-3., 4.]), 4.);
        assert_eq!(norm2(&[3., 4.]), 5.);
        assert_eq!(norm2(&[f64::INFINITY, 1.]), f64::INFINITY);
    }

    #[test]
    fn lu_pivots_and_reuses_factors_for_multiple_right_sides() {
        let a = [0., 4., 7., 2., 5., 8., 3., 6., 10.];
        let lu = Lu::new(&a, 3);
        for expected in [[1., -2., 3.], [2., 1., -4.]] {
            let b = matvec(&a, 3, &expected);
            let x = lu.solve(&b);
            for (got, want) in x.iter().zip(expected) {
                assert!((got - want).abs() < 2e-14, "{x:?}");
            }
        }
        assert!(solve(&[0.; 4], 2, &[1., 1.]).iter().all(|x| x.is_nan()));
    }

    #[test]
    fn adaptive_jacobian_matches_a_rectangular_polynomial() {
        let mut f = |y: &[f64]| Ok(vec![y[0] * y[0] + 3. * y[1], y[0] * y[1], 7.]);
        let y = [2., -3.];
        let initial = f(&y).unwrap();
        let mut fac = vec![];
        let (jac, calls) = numjac(&mut f, &y, &initial, &[1e-6; 2], &mut fac).unwrap();
        for (got, want) in jac.iter().zip([4., -3., 0., 3., 2., 0.]) {
            assert!((got - want).abs() < 2e-7, "{jac:?}");
        }
        assert_eq!(calls, 2);
        assert_eq!(fac.len(), 2);
        let (_, calls) = numjac(&mut |_| Ok(vec![1.]), &[0.], &[1.], &[1.], &mut vec![]).unwrap();
        assert_eq!(
            calls, 2,
            "a lost difference requires a larger-increment retry"
        );
    }

    #[test]
    fn general_format_rounds_before_selecting_notation() {
        for (value, expected) in [
            (100., "100"),
            (1.25, "1.25"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (999999.9, "1e+06"),
            (100000., "100000"),
            (100. * f64::EPSILON, "2.22045e-14"),
        ] {
            assert_eq!(fmt_g(value), expected);
        }
    }

    #[test]
    fn brent_handles_nonlinear_and_flat_crossings() {
        for power in [1, 3, 7] {
            let mut f = |x: f64| Ok((x - 0.3).powi(power));
            let root =
                event_root(&mut f, -1., (-1.3f64).powi(power), 2., 1.7f64.powi(power)).unwrap();
            assert!((root - 0.3).abs() < 2e-14, "power {power}: {root}");
        }
        let root = event_root(&mut |x| Ok(x * x - 2.), 0., -2., 2., 2.).unwrap();
        assert!((root - 2f64.sqrt()).abs() < 2e-15);
    }

    #[test]
    fn brent_returns_the_crossed_side_in_both_time_and_value_directions() {
        for k in 1..200 {
            let level = 1. + k as f64 * 1.7 / 200.;
            for (a, b) in [(0., 1.), (1., 0.)] {
                for polarity in [-1., 1.] {
                    let mut f = |x: f64| Ok(polarity * (libm::exp(x) - level));
                    let fa = f(a).unwrap();
                    let fb = f(b).unwrap();
                    let root = event_root(&mut f, a, fa, b, fb).unwrap();
                    let residual = f(root).unwrap();
                    assert!(
                        residual == 0. || sign(residual) == sign(fb),
                        "k={k} [{a},{b}] polarity={polarity} residual={residual:e}"
                    );
                    assert!((root - libm::log(level)).abs() < 2e-14);
                }
            }
        }
    }

    #[test]
    fn event_flags_use_nonzero_terminals_and_direction_signs() {
        for terminal in [-2., 0., 2.] {
            for direction in [-2., -0.5, 0., 0.5, 2.] {
                for slope in [-1., 1.] {
                    let mut interpolation = |t| vec![t];
                    let mut events = |t: f64, _: &[f64]| {
                        Ok((vec![slope * (t - 0.5)], vec![terminal], vec![direction]))
                    };
                    let (found, _) = odezero(
                        &mut interpolation,
                        &mut events,
                        0.,
                        &[-0.5 * slope],
                        0.,
                        1.,
                        &[1.],
                    )
                    .unwrap();
                    let crosses = direction == 0. || sign(direction) == slope;
                    assert_eq!(found.t.len(), usize::from(crosses));
                    assert_eq!(found.stop, crosses && terminal != 0.);
                    if crosses {
                        assert_eq!(found.t, [0.5]);
                    }
                }
            }
        }
        assert!(validate_events(&[0.], &[f64::NAN], &[0.], 1).is_err());
        assert!(validate_events(&[0.], &[0.], &[f64::INFINITY], 1).is_err());
        assert!(validate_events(&[0.], &[0.], &[1., 2.], 1).is_err());
    }

    #[test]
    fn events_sort_filter_stop_and_keep_simultaneous_roots() {
        let mut interpolation = |t| vec![t];
        let mut events = |t: f64, _: &[f64]| {
            Ok((
                vec![t - 0.8, t - 0.3, t - 0.3, t - 0.1],
                vec![0., 1., 0., 0.],
                vec![0., 1., 0., -1.],
            ))
        };
        let (found, right) = odezero(
            &mut interpolation,
            &mut events,
            0.,
            &[-0.8, -0.3, -0.3, -0.1],
            0.,
            1.,
            &[1.],
        )
        .unwrap();
        assert!(found.stop);
        assert_eq!(found.i, [2, 3]);
        assert!(found.t.iter().all(|t| (t - 0.3).abs() < 1e-14));
        assert_eq!(right.len(), 4);
        let mut events = |t: f64, _: &[f64]| Ok((vec![t - 0.2, t - 0.7], vec![0., 0.], vec![]));
        let (found, _) = odezero(
            &mut interpolation,
            &mut events,
            1.,
            &[0.8, 0.3],
            1.,
            0.,
            &[0.],
        )
        .unwrap();
        assert_eq!(found.i, [2, 1]);
        assert!(!found.stop);
    }

    #[test]
    fn initial_zero_is_recorded_once_without_stopping() {
        let mut interpolation = |t| vec![t];
        let mut events = |t: f64, _: &[f64]| Ok((vec![t], vec![1.], vec![0.]));
        let (found, _) =
            odezero(&mut interpolation, &mut events, 0., &[0.], 0., 1., &[1.]).unwrap();
        assert_eq!(found.t, [0.]);
        assert_eq!(found.y, [vec![0.]]);
        assert_eq!(found.i, [1]);
        assert!(!found.stop);
        let (found, _) =
            odezero(&mut interpolation, &mut events, -1., &[0.], 0., 1., &[1.]).unwrap();
        assert!(found.t.is_empty());
    }

    #[test]
    fn events_bound_total_state_output_before_interpolating() {
        let mut calls = 0;
        let mut interpolation = |_| {
            calls += 1;
            vec![0.; 4096]
        };
        let mut events = |_: f64, _: &[f64]| Ok((vec![0.; 8192], vec![0.; 8192], vec![]));
        let error = odezero(
            &mut interpolation,
            &mut events,
            0.,
            &vec![-1.; 8192],
            0.,
            1.,
            &vec![0.; 4096],
        )
        .err()
        .unwrap();
        assert!(error.contains("Event output exceeds"));
        assert_eq!(
            calls, 0,
            "reject before interpolating/allocating event states"
        );
    }

    #[test]
    fn brent_resolves_a_root_far_below_the_initial_time_scale() {
        let root = event_root(&mut |x| Ok(x * x - 1e-200), 0., -1e-200, 1., 1.).unwrap();
        assert!((root / 1e-100 - 1.).abs() < 5e-13, "{root:e}");
    }
}
