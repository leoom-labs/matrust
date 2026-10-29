//! Fully implicit equations F(t, y, y') = 0. Variable-coefficient BDF orders 1–5
//! and consistent initialisation by damped Newton iteration.
//!
//! Mathematics: Brenan, Campbell & Petzold, Numerical Solution of Initial-Value
//! Problems in Differential-Algebraic Equations (1996), chapters 3 and 5;
//! Shampine, Solving 0 = F(t,y(t),y'(t)) in MATLAB (2002), BDF interpolation
//! and consistent initial conditions; Golub & Van Loan, Matrix Computations
//! (2013), section 5.5, minimum-norm least squares. No vendor implementation
//! is used. Step selection and work counts belong to this implementation.
use super::ivp::{OutAt, format_e};
use super::num::{Lu, R, eps_of, norm_inf, norm2, numjac};
use crate::engine::Fma;

pub trait Implicit {
    fn f(&mut self, t: f64, y: &[f64], yp: &[f64]) -> R<Vec<f64>>;
    fn check_cancel(&self) -> R<()>;
}

pub struct IOut {
    pub t: Vec<f64>,
    pub y: Vec<Vec<f64>>,
    /// Order used on each step (sol.idata.kvec, first entry for t0).
    pub kvec: Vec<usize>,
    pub stats: [usize; 6],
    pub warnings: Vec<String>,
    pub ypfinal: Vec<f64>,
}

pub struct IOpts {
    pub rtol: f64,
    pub threshold: Vec<f64>,
    pub atol: Vec<f64>,
    pub hmax: f64,
    pub htry: Option<f64>,
    pub htspan: f64,
    pub maxorder: usize,
}

pub fn lagrange(ts: &[f64], ys: &[&[f64]], x: f64) -> Vec<f64> {
    // Shifted Newton interpolation avoids products of absolute times. Returning
    // a node verbatim also preserves exact initial conditions in requested output.
    if let Some(i) = ts.iter().position(|&t| t == x) {
        return ys[i].to_vec();
    }
    if ts.is_empty() {
        return Vec::new();
    }
    let n = ts.len();
    let scale = (ts[n - 1] - ts[0]).abs().max(f64::MIN_POSITIVE);
    let nodes: Vec<f64> = ts.iter().map(|t| (t - ts[0]) / scale).collect();
    let z = (x - ts[0]) / scale;
    let mut result = vec![0.; ys[0].len()];
    let mut differences = vec![0.; n];
    for (i, value) in result.iter_mut().enumerate() {
        for (d, y) in differences.iter_mut().zip(ys) {
            *d = y[i];
        }
        for degree in 1..n {
            for j in (degree..n).rev() {
                differences[j] =
                    (differences[j] - differences[j - 1]) / (nodes[j] - nodes[j - degree]);
            }
        }
        *value = differences[n - 1];
        for j in (0..n - 1).rev() {
            *value = (z - nodes[j]).fma(*value, differences[j]);
        }
    }
    result
}

fn derivative_weights(ts: &[f64]) -> Vec<f64> {
    // Differentiate the Lagrange basis at its first node. BDF coefficients
    // follow directly, including on nonuniform and decreasing meshes.
    let n = ts.len();
    if n < 2 {
        return vec![0.; n];
    }
    let scale = ts[0] - ts[1];
    let nodes: Vec<f64> = ts.iter().map(|t| (t - ts[0]) / scale).collect();
    let mut w = vec![0.; n];
    w[0] = -nodes[1..].iter().map(|v| 1. / v).sum::<f64>() / scale;
    for i in 1..n {
        let mut weight = 1. / nodes[i];
        for j in 1..n {
            if j != i {
                weight *= -nodes[j] / (nodes[i] - nodes[j]);
            }
        }
        w[i] = weight / scale;
    }
    w
}

struct Jacs {
    dfdy: Vec<f64>,
    dfdyp: Vec<f64>,
    fac_y: Vec<f64>,
    fac_yp: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn jacobians(
    m: &mut dyn Implicit,
    t: f64,
    y: &[f64],
    yp: &[f64],
    fv: &[f64],
    thresh: &[f64],
    j: &mut Jacs,
    stats: &mut [usize; 6],
) -> R<()> {
    let (dfdy, calls) = numjac(
        &mut |v| {
            m.check_cancel()?;
            m.f(t, v, yp)
        },
        y,
        fv,
        thresh,
        &mut j.fac_y,
    )?;
    stats[2] += calls;
    let (dfdyp, calls) = numjac(
        &mut |v| {
            m.check_cancel()?;
            m.f(t, y, v)
        },
        yp,
        fv,
        thresh,
        &mut j.fac_yp,
    )?;
    stats[2] += calls;
    stats[3] += 1;
    j.dfdy = dfdy;
    j.dfdyp = dfdyp;
    Ok(())
}

/// Adjust only free initial components until F(t0,y0,yp0) = 0. Damped Newton
/// with minimum-norm SVD corrections (Golub & Van Loan, 2013, section 5.5).
/// Returns the initial values, derivatives, and Euclidean residual norm.
#[allow(clippy::too_many_arguments)]
pub fn decic(
    m: &mut dyn Implicit,
    t0: f64,
    y0: &[f64],
    fixed_y: &[bool],
    yp0: &[f64],
    fixed_yp: &[bool],
    rtol: f64,
    atol: &[f64],
) -> R<(Vec<f64>, Vec<f64>, f64)> {
    let n = y0.len();
    if n == 0 || yp0.len() != n || fixed_y.len() != n || fixed_yp.len() != n {
        return Err(
            "The vectors y0, fixed_y0, yp0 and fixed_yp0 must have the same length.".into(),
        );
    }
    let free: Vec<usize> = fixed_y
        .iter()
        .chain(fixed_yp)
        .enumerate()
        .filter_map(|(i, &fixed)| (!fixed).then_some(i))
        .collect();
    if free.len() < n {
        return Err(format!(
            "Cannot specify more than {n} components of y0 and yp0."
        ));
    }
    check_workspace(n, free.len())?;
    let threshold: Vec<f64> = (0..n)
        .map(|i| atol[if atol.len() == 1 { 0 } else { i }] / rtol)
        .collect();
    let mut y = y0.to_vec();
    let mut yp = yp0.to_vec();
    let mut j = empty_jacobians();
    let mut stats = [0; 6];
    let mut residual = evaluate(m, t0, &y, &yp, &mut stats)?;
    let initial_norm = norm2(&residual);
    if initial_norm == 0. {
        return Ok((y, yp, 0.));
    }
    let target = initial_norm * (0.01 * rtol).clamp(64. * f64::EPSILON, 1e-6);
    for iteration in 0..30 {
        m.check_cancel()?;
        let old_norm = norm2(&residual);
        if !old_norm.is_finite() {
            break;
        }
        jacobians(m, t0, &y, &yp, &residual, &threshold, &mut j, &mut stats)?;
        if iteration == 0 {
            // Map the component tolerances into residual units using the free
            // columns. This absolute floor accepts initially consistent data
            // without requiring a relative reduction of rounding noise. A
            // constant, nonzero residual still has zero attainable tolerance.
            let residual_tolerance: Vec<f64> = (0..n)
                .map(|row| {
                    free.iter()
                        .map(|&column| {
                            let i = column % n;
                            let (partial, value) = if column < n {
                                (&j.dfdy, y[i])
                            } else {
                                (&j.dfdyp, yp[i])
                            };
                            let absolute = atol[if atol.len() == 1 { 0 } else { i }];
                            partial[row + i * n].abs() * (absolute + rtol * value.abs())
                        })
                        .sum()
                })
                .collect();
            let floor = norm2(&residual_tolerance);
            if floor.is_finite() && old_norm <= floor {
                return Ok((y, yp, old_norm));
            }
        }
        let mut a = Vec::with_capacity(n * free.len());
        for &column in &free {
            let source = if column < n { &j.dfdy } else { &j.dfdyp };
            let col = column % n;
            a.extend_from_slice(&source[col * n..(col + 1) * n]);
        }
        let rhs: Vec<f64> = residual.iter().map(|v| -v).collect();
        let correction = min_norm_solve(&a, n, free.len(), &rhs);
        let correction_norm = norm_inf(correction.iter().zip(&free).map(|(&d, &i)| {
            let value = if i < n { y[i] } else { yp[i - n] };
            d / (rtol * value.abs().max(threshold[i % n]))
        }));
        if !correction_norm.is_finite() {
            break;
        }
        let mut damping = 1.;
        let mut accepted = None;
        for _ in 0..14 {
            let mut trial_y = y.clone();
            let mut trial_yp = yp.clone();
            for (&i, &d) in free.iter().zip(&correction) {
                let value = if i < n {
                    &mut trial_y[i]
                } else {
                    &mut trial_yp[i - n]
                };
                *value = damping.fma(d, *value);
            }
            let trial = evaluate(m, t0, &trial_y, &trial_yp, &mut stats)?;
            let new_norm = norm2(&trial);
            // Return the corrected iterate and its freshly evaluated residual,
            // including when the final correction only changes rounding bits.
            if new_norm.is_finite()
                && new_norm <= old_norm
                && old_norm <= target
                && correction_norm <= 0.1
            {
                return Ok((trial_y, trial_yp, new_norm));
            }
            if new_norm.is_finite() && new_norm < old_norm * (1. - 1e-4 * damping) {
                accepted = Some((trial_y, trial_yp, trial, new_norm));
                break;
            }
            damping *= 0.5;
        }
        let Some((trial_y, trial_yp, trial, new_norm)) = accepted else {
            break;
        };
        y = trial_y;
        yp = trial_yp;
        residual = trial;
        if new_norm == 0. || (new_norm <= target && damping * correction_norm <= 0.1) {
            return Ok((y, yp, new_norm));
        }
    }
    Err("Convergence failure in DECIC.".into())
}

fn min_norm_solve(a: &[f64], rows: usize, cols: usize, b: &[f64]) -> Vec<f64> {
    // Rank-revealing SVD avoids squaring the condition number in A*A^T.
    // Uniform scaling leaves the Euclidean minimum-norm solution unchanged.
    let scale = norm_inf(a.iter().copied());
    if scale == 0. {
        return vec![0.; cols];
    }
    if !scale.is_finite() || b.iter().any(|v| !v.is_finite()) {
        return vec![f64::NAN; cols];
    }
    // Factor A^T: the tall orientation also avoids losing a small left-vector
    // rotation in nalgebra's wide/upper-bidiagonal SVD. Such a lost rotation
    // corrupts solutions along small singular directions even when the
    // singular values themselves are accurate.
    let matrix = nalgebra::DMatrix::from_fn(cols, rows, |i, j| a[j + i * rows] / scale);
    let rhs = nalgebra::DVector::from_iterator(rows, b.iter().map(|v| v / scale));
    let Some(svd) =
        nalgebra::linalg::SVD::try_new(matrix, true, true, f64::EPSILON, 100 * rows.max(cols))
    else {
        return vec![f64::NAN; cols];
    };
    let cutoff = f64::EPSILON * rows.max(cols) as f64 * svd.singular_values[0];
    // A^T = U*S*V^T, hence x = U*S^+*V^T*b; only a diagonal solve is
    // performed, without forming a pseudoinverse or normal equations.
    let mut projected = svd.v_t.as_ref().unwrap() * rhs;
    for (value, &singular) in projected.iter_mut().zip(svd.singular_values.iter()) {
        *value = if singular > cutoff {
            *value / singular
        } else {
            0.
        };
    }
    (svd.u.as_ref().unwrap() * projected).as_slice().to_vec()
}

pub fn ode15i(
    m: &mut dyn Implicit,
    tspan: &[f64],
    y0: &[f64],
    yp0: &[f64],
    o: &IOpts,
    at: OutAt,
) -> R<IOut> {
    let n = y0.len();
    check_workspace(n, n)?;
    m.check_cancel()?;
    let mut out = IOut {
        t: vec![tspan[0]],
        y: vec![y0.to_vec()],
        kvec: vec![1],
        stats: [0; 6],
        warnings: Vec::new(),
        ypfinal: yp0.to_vec(),
    };
    let mut times = vec![tspan[0]];
    let mut values = vec![y0.to_vec()];
    let final_time = tspan[tspan.len() - 1];
    let direction = (final_time - tspan[0]).signum();
    let span = (final_time - tspan[0]).abs();
    let slope = norm_inf(
        yp0.iter()
            .enumerate()
            .map(|(i, v)| v / y0[i].abs().max(o.threshold[i])),
    );
    let initial = (0.01 * span).min(if slope > 0. {
        0.1 * o.rtol.sqrt() / slope
    } else {
        span
    });
    let mut step = o
        .htry
        .unwrap_or(initial)
        .min(o.htspan)
        .max(16. * eps_of(tspan[0]))
        .min(o.hmax)
        .min(span);
    let mut order = 1;
    let maxorder = o.maxorder.clamp(1, 5);
    let mut order_age = 0;
    let mut next_output = 1;
    let mut j = empty_jacobians();
    let mut refresh_jacobian = true;
    let mut rejected = false;
    loop {
        m.check_cancel()?;
        let t = times[0];
        if t == final_time {
            return Ok(out);
        }
        let remaining = (final_time - t).abs();
        let minimum = 16. * eps_of(t);
        step = step.min(o.hmax).min(remaining);
        if (step < minimum && step < remaining) || !step.is_finite() || t + direction * step == t {
            integration_warning(&mut out, t, minimum);
            return Ok(out);
        }
        let tnew = if step >= remaining {
            final_time
        } else {
            t + direction * step
        };
        let h = tnew - t;
        order = order.min(times.len());
        let mut nodes = vec![tnew];
        nodes.extend_from_slice(&times[..order]);
        let coefficients = derivative_weights(&nodes);
        let predictor = if times.len() > order {
            let refs: Vec<&[f64]> = values[..=order].iter().map(Vec::as_slice).collect();
            lagrange(&times[..=order], &refs, tnew)
        } else {
            values[0]
                .iter()
                .zip(&out.ypfinal)
                .map(|(&y, &yp)| h.fma(yp, y))
                .collect()
        };
        let jacobians_before = out.stats[3];
        let mut attempt = bdf_correct(
            m,
            tnew,
            &predictor,
            &values[..order],
            &coefficients,
            o,
            &mut j,
            refresh_jacobian,
            &mut out.stats,
        )?;
        // A stale partial Jacobian gets one fresh attempt before reducing h.
        if attempt.is_none() && !refresh_jacobian && out.stats[3] == jacobians_before {
            attempt = bdf_correct(
                m,
                tnew,
                &predictor,
                &values[..order],
                &coefficients,
                o,
                &mut j,
                true,
                &mut out.stats,
            )?;
        }
        // Saved partials are refreshed only after corrector failure. A failed
        // fresh attempt reduces the step instead of repeating the same solve.
        refresh_jacobian = attempt.is_none();
        let Some((ynew, ypnew)) = attempt else {
            out.stats[1] += 1;
            step *= 0.25;
            order = 1;
            order_age = 0;
            rejected = true;
            continue;
        };
        let multiplier = if times.len() > order {
            1. / (coefficients[0] * (tnew - times[order]))
        } else {
            0.5
        };
        let error = weighted_norm(
            ynew.iter()
                .zip(&predictor)
                .map(|(y, p)| multiplier * (y - p)),
            &values[0],
            &ynew,
            o,
        );
        if !error.is_finite() || error > 1. {
            out.stats[1] += 1;
            step *= step_factor(error, order).clamp(0.1, 0.5);
            if rejected {
                order = order.saturating_sub(1).max(1);
            }
            order_age = 0;
            rejected = true;
            continue;
        }
        let mut output_nodes = vec![tnew];
        output_nodes.extend_from_slice(&times[..order]);
        let mut output_values: Vec<&[f64]> = vec![&ynew];
        output_values.extend(values[..order].iter().map(Vec::as_slice));
        match at {
            OutAt::Sol | OutAt::Steps => push_output(&mut out, tnew, ynew.clone(), order)?,
            OutAt::Requested => {
                while next_output < tspan.len() && direction * (tnew - tspan[next_output]) >= 0. {
                    m.check_cancel()?;
                    let tout = tspan[next_output];
                    push_output(
                        &mut out,
                        tout,
                        lagrange(&output_nodes, &output_values, tout),
                        order,
                    )?;
                    next_output += 1;
                }
            }
            OutAt::Refined(refine) => {
                check_output_size(out.t.len().saturating_add(refine), n)?;
                for index in 1..=refine {
                    m.check_cancel()?;
                    let tout = if index == refine {
                        tnew
                    } else {
                        (index as f64 / refine as f64).fma(h, t)
                    };
                    push_output(
                        &mut out,
                        tout,
                        lagrange(&output_nodes, &output_values, tout),
                        order,
                    )?;
                }
            }
        }
        out.stats[0] += 1;
        out.ypfinal = ypnew;
        order_age += 1;
        let mut next_order = order;
        let mut factor = step_factor(error, order);
        if order_age > order && !rejected {
            // Compare predicted allowable step lengths at neighbouring orders.
            // Each estimate uses one extra past value, hence one extra divided
            // difference; no second nonlinear solve is needed.
            for candidate in [order.saturating_sub(1), order + 1] {
                if candidate == 0 || candidate > maxorder || times.len() <= candidate {
                    continue;
                }
                let trial_error = history_error(tnew, &ynew, candidate, &times, &values, o);
                let trial_factor = step_factor(trial_error, candidate);
                if trial_factor > 1.2 * factor {
                    next_order = candidate;
                    factor = trial_factor;
                }
            }
        }
        if next_order != order {
            order_age = 0;
        }
        times.insert(0, tnew);
        values.insert(0, ynew);
        times.truncate(maxorder + 2);
        values.truncate(maxorder + 2);
        order = next_order;
        step = h.abs() * factor.clamp(0.5, if rejected { 1. } else { 2. });
        rejected = false;
    }
}

fn empty_jacobians() -> Jacs {
    Jacs {
        dfdy: Vec::new(),
        dfdyp: Vec::new(),
        fac_y: Vec::new(),
        fac_yp: Vec::new(),
    }
}

fn check_workspace(rows: usize, cols: usize) -> R<()> {
    if rows
        .checked_mul(cols)
        .is_none_or(|n| n > crate::matrix::MAX_ELEMENTS)
    {
        return Err(format!(
            "Matrix exceeds the {}-element limit",
            crate::matrix::MAX_ELEMENTS
        ));
    }
    Ok(())
}

fn check_output_size(points: usize, n: usize) -> R<()> {
    let limit = super::ivp::MAX_VALUES;
    if points.saturating_mul(n.saturating_add(1)) > limit {
        return Err(format!(
            "ODE solution exceeds the {limit}-element limit; shorten tspan or request fewer output times"
        ));
    }
    Ok(())
}

fn push_output(out: &mut IOut, t: f64, y: Vec<f64>, order: usize) -> R<()> {
    check_output_size(out.t.len() + 1, y.len())?;
    out.t.push(t);
    out.y.push(y);
    out.kvec.push(order);
    Ok(())
}

fn integration_warning(out: &mut IOut, t: f64, minimum: f64) {
    out.warnings.push(format!(
        "Warning: Integration stopped at t={}; meeting the tolerances would require a step below the minimum ({}).",
        format_e(t), format_e(minimum),
    ));
}

fn evaluate(
    m: &mut dyn Implicit,
    t: f64,
    y: &[f64],
    yp: &[f64],
    stats: &mut [usize; 6],
) -> R<Vec<f64>> {
    m.check_cancel()?;
    let residual = m.f(t, y, yp)?;
    stats[2] += 1;
    if residual.len() != y.len() {
        return Err("Implicit function returned a vector of the wrong length.".into());
    }
    Ok(residual)
}

fn weighted_norm(v: impl Iterator<Item = f64>, old: &[f64], new: &[f64], o: &IOpts) -> f64 {
    norm_inf(v.enumerate().map(|(i, e)| {
        let absolute = o.atol[if o.atol.len() == 1 { 0 } else { i }];
        e / (o.rtol * old[i].abs().max(new[i].abs())).max(absolute)
    }))
}

fn step_factor(error: f64, order: usize) -> f64 {
    if !error.is_finite() {
        0.1
    } else if error == 0. {
        2.
    } else {
        0.85 * libm::pow(error, -1. / (order + 1) as f64)
    }
}

fn history_error(
    tnew: f64,
    ynew: &[f64],
    order: usize,
    times: &[f64],
    values: &[Vec<f64>],
    o: &IOpts,
) -> f64 {
    let refs: Vec<&[f64]> = values[..=order].iter().map(Vec::as_slice).collect();
    let predicted = lagrange(&times[..=order], &refs, tnew);
    let mut nodes = vec![tnew];
    nodes.extend_from_slice(&times[..order]);
    let multiplier = 1. / (derivative_weights(&nodes)[0] * (tnew - times[order]));
    weighted_norm(
        ynew.iter()
            .zip(&predicted)
            .map(|(y, p)| multiplier * (y - p)),
        &values[0],
        ynew,
        o,
    )
}

/// Solve F(t,Y,a0*Y+c)=0. Subtract the newest history value before applying
/// coefficients so that a constant solution has an exactly zero derivative.
#[allow(clippy::too_many_arguments)]
fn bdf_correct(
    m: &mut dyn Implicit,
    t: f64,
    predictor: &[f64],
    history: &[Vec<f64>],
    coefficients: &[f64],
    o: &IOpts,
    j: &mut Jacs,
    refresh: bool,
    stats: &mut [usize; 6],
) -> R<Option<(Vec<f64>, Vec<f64>)>> {
    let n = predictor.len();
    let mut y = predictor.to_vec();
    let mut yp = vec![0.; n];
    let derivative = |y: &[f64], yp: &mut [f64]| {
        for (i, value) in yp.iter_mut().enumerate() {
            *value = coefficients[0] * (y[i] - history[0][i]);
            for (past, &a) in history.iter().zip(&coefficients[1..]).skip(1) {
                *value = a.fma(past[i] - history[0][i], *value);
            }
        }
    };
    derivative(&y, &mut yp);
    if y.iter().chain(&yp).any(|v| !v.is_finite()) {
        return Ok(None);
    }
    let mut residual = evaluate(m, t, &y, &yp, stats)?;
    if !norm_inf(residual.iter().copied()).is_finite() {
        return Ok(None);
    }
    if refresh || j.dfdy.is_empty() {
        jacobians(m, t, &y, &yp, &residual, &o.threshold, j, stats)?;
    }
    let matrix: Vec<f64> = j
        .dfdy
        .iter()
        .zip(&j.dfdyp)
        .map(|(&a, &b)| coefficients[0].fma(b, a))
        .collect();
    if matrix.iter().any(|v| !v.is_finite()) {
        return Ok(None);
    }
    let lu = Lu::new(&matrix, n);
    stats[4] += 1;
    let mut previous = f64::INFINITY;
    for iteration in 0..8 {
        m.check_cancel()?;
        let rhs: Vec<f64> = residual.iter().map(|v| -v).collect();
        let correction = lu.solve(&rhs);
        stats[5] += 1;
        let size = weighted_norm(correction.iter().copied(), &history[0], &y, o);
        if !size.is_finite() || (iteration > 1 && size > 0.9 * previous) {
            return Ok(None);
        }
        for (value, &change) in y.iter_mut().zip(&correction) {
            *value += change;
        }
        derivative(&y, &mut yp);
        if y.iter().chain(&yp).any(|v| !v.is_finite()) {
            return Ok(None);
        }
        if size <= 0.03 {
            return Ok(Some((y, yp)));
        }
        previous = size;
        residual = evaluate(m, t, &y, &yp, stats)?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Model<F> {
        residual: F,
        calls: usize,
        cancel_after: usize,
        checks: Cell<usize>,
    }

    impl<F: Fn(f64, &[f64], &[f64]) -> Vec<f64>> Implicit for Model<F> {
        fn f(&mut self, t: f64, y: &[f64], yp: &[f64]) -> R<Vec<f64>> {
            self.calls += 1;
            Ok((self.residual)(t, y, yp))
        }
        fn check_cancel(&self) -> R<()> {
            self.checks.set(self.checks.get() + 1);
            if self.checks.get() > self.cancel_after {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        }
    }

    fn model<F: Fn(f64, &[f64], &[f64]) -> Vec<f64>>(residual: F) -> Model<F> {
        Model {
            residual,
            calls: 0,
            cancel_after: usize::MAX,
            checks: Cell::new(0),
        }
    }

    fn options(n: usize, rtol: f64) -> IOpts {
        IOpts {
            rtol,
            threshold: vec![1e-3; n],
            atol: vec![rtol * 1e-3; n],
            hmax: 0.1,
            htry: None,
            htspan: 1.,
            maxorder: 5,
        }
    }

    #[test]
    fn fix1_decic_accepts_initial_roundoff_and_small_residuals() {
        for offset in [0., 1e-12] {
            let mut m = model(|_, y, yp| vec![yp[0] * yp[0] - y[0]]);
            let yp0 = 2f64.sqrt() + offset;
            let result = decic(&mut m, 0., &[2.], &[true], &[yp0], &[false], 1e-3, &[1e-6]);
            println!("initial residual={:e}, result={result:?}", yp0 * yp0 - 2.);
            let (y, yp, res) = result.unwrap();
            assert_eq!(y, [2.]);
            assert_eq!(yp, [yp0]);
            assert_eq!(res, (yp0 * yp0 - 2.).abs());
        }
    }

    #[test]
    fn fix1_decic_applies_final_newton_correction() {
        let mut m = model(|_, y, yp| vec![yp[0] + y[0]]);
        let result = decic(&mut m, 0., &[1.], &[true], &[0.], &[false], 1e-4, &[1e-6]);
        println!("RelTol=1e-4 linear decic: {result:?}");
        let (y, yp, res) = result.unwrap();
        assert_eq!(y, [1.]);
        assert!((yp[0] + 1.).abs() < 1e-12);
        assert_eq!(res, (yp[0] + 1.).abs());
    }

    #[test]
    fn fix1_linear_decay_reuses_partial_jacobians() {
        let mut m = model(|_, y, yp| vec![yp[0] + y[0]]);
        let out = ode15i(
            &mut m,
            &[0., 1.],
            &[1.],
            &[-1.],
            &options(1, 1e-3),
            OutAt::Sol,
        )
        .unwrap();
        println!("linear decay stats={:?}", out.stats);
        assert_eq!(out.stats[3], 1);
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn fix1_long_requested_run_has_no_iteration_limit() {
        let mut m = model(|_, y, yp| vec![yp[0] + y[0]]);
        let mut o = options(1, 1e-3);
        o.hmax = 5e-7;
        let result = ode15i(&mut m, &[0., 0.5, 1.], &[1.], &[-1.], &o, OutAt::Requested);
        println!(
            "long requested run: {:?}",
            result.as_ref().map(|out| (&out.t, out.stats))
        );
        let out = result.unwrap();
        assert_eq!(out.t, [0., 0.5, 1.]);
        assert!(out.stats[0] > 1_000_000);
        assert!(out.warnings.is_empty());
        assert!((out.y[2][0] - libm::exp(-1.)).abs() < 1e-5);
    }

    #[test]
    fn polynomials_on_irregular_forward_and_reverse_meshes() {
        for direction in [1., -1.] {
            let ts: Vec<f64> = [0., -0.13, -0.4, -0.75, -1.1, -1.8]
                .iter()
                .map(|v| 2. + direction * v)
                .collect();
            for order in 1..=5 {
                let ys: Vec<Vec<f64>> = ts[..=order]
                    .iter()
                    .map(|&t| {
                        let z = t - 2.;
                        vec![z.powi(order as i32), 3. + 2. * z]
                    })
                    .collect();
                let refs: Vec<&[f64]> = ys.iter().map(Vec::as_slice).collect();
                let x = 2. - direction * 0.07;
                let interpolated = lagrange(&ts[..=order], &refs, x);
                assert!((interpolated[0] - (x - 2.).powi(order as i32)).abs() < 2e-13);
                assert!((interpolated[1] - (3. + 2. * (x - 2.))).abs() < 2e-13);
                let weights = derivative_weights(&ts[..=order]);
                for power in 0..=order {
                    let got: f64 = weights
                        .iter()
                        .zip(&ts)
                        .map(|(w, t)| w * (t - 2.).powi(power as i32))
                        .sum();
                    let expected = if power == 1 { 1. } else { 0. };
                    assert!(
                        (got - expected).abs() < 3e-12,
                        "order={order}, power={power}: {got}"
                    );
                }
                assert_eq!(lagrange(&ts[..=order], &refs, ts[order]), ys[order]);
            }
        }
    }

    #[test]
    fn interpolation_handles_large_time_origins_and_short_intervals() {
        for (origin, step) in [(1e12, 0.125), (0., 1e-120)] {
            let ts: Vec<f64> = (0..=5).map(|i| origin + i as f64 * step).collect();
            let ys: Vec<Vec<f64>> = (0..=5).map(|i| vec![(i as f64).powi(3)]).collect();
            let refs: Vec<&[f64]> = ys.iter().map(Vec::as_slice).collect();
            let x = origin + 2.5 * step;
            assert!((lagrange(&ts, &refs, x)[0] - 2.5f64.powi(3)).abs() < 1e-12);
        }
    }

    #[test]
    fn svd_correction_is_minimum_norm_and_rank_revealing() {
        // [1 1] x = 2 has minimum-norm solution [1,1].
        let x = min_norm_solve(&[1., 1.], 1, 2, &[2.]);
        assert!((x[0] - 1.).abs() < 1e-14 && (x[1] - 1.).abs() < 1e-14);
        // Duplicate equations and an unused free component.
        let x = min_norm_solve(&[1., 2., 1., 2., 0., 0.], 2, 3, &[2., 4.]);
        assert!((x[0] - 1.).abs() < 1e-14 && (x[1] - 1.).abs() < 1e-14 && x[2] == 0.);
        // A*A^T would lose the 1e-10 singular direction by squaring it.
        for scale in [1e-150, 1., 1e150] {
            let a: Vec<f64> = [1., 0., 1., 1e-10].iter().map(|v| v * scale).collect();
            let x = min_norm_solve(&a, 2, 2, &[2. * scale, 1e-10 * scale]);
            assert!(
                (x[0] - 1.).abs() < 1e-5 && (x[1] - 1.).abs() < 1e-5,
                "scale={scale}: {x:?}"
            );
        }
    }

    #[test]
    fn decic_preserves_fixed_components_and_exact_linear_answers() {
        let mut m = model(|_, y, yp| vec![y[0] + yp[0]]);
        let result = decic(&mut m, 0., &[1.], &[true], &[0.], &[false], 1e-3, &[1e-6]).unwrap();
        assert_eq!(result, (vec![1.], vec![-1.], 0.));
        let result = decic(&mut m, 0., &[3.], &[false], &[0.], &[true], 1e-3, &[1e-6]).unwrap();
        assert_eq!(result, (vec![0.], vec![0.], 0.));
        let mut m = model(|_, y, yp| vec![yp[0] + y[0], y[1] - 2. * y[0]]);
        let result = decic(
            &mut m,
            0.,
            &[1., 0.],
            &[true, false],
            &[0., 0.],
            &[false, false],
            1e-3,
            &[1e-6],
        )
        .unwrap();
        assert_eq!(result, (vec![1., 2.], vec![-1., 0.], 0.));
    }

    #[test]
    fn decic_all_free_components_share_the_minimum_norm_correction() {
        let mut m = model(|_, y, yp| vec![y[0] + yp[0] - 2.]);
        let (y, yp, res) =
            decic(&mut m, 0., &[0.], &[false], &[0.], &[false], 1e-8, &[1e-10]).unwrap();
        assert!((y[0] - 1.).abs() < 1e-8 && (yp[0] - 1.).abs() < 1e-8);
        assert!(res < 1e-12);
    }

    #[test]
    fn decic_damping_converges_to_a_nonlinear_root() {
        let mut m = model(|_, y, yp| vec![yp[0] * yp[0] - y[0]]);
        let (y, yp, res) = decic(
            &mut m,
            0.,
            &[2.],
            &[true],
            &[0.01],
            &[false],
            1e-8,
            &[1e-10],
        )
        .unwrap();
        assert_eq!(y, [2.]);
        assert!(
            (yp[0] - 2f64.sqrt()).abs() < 1e-10 && res < 1e-10,
            "{yp:?}, {res}"
        );
    }

    #[test]
    fn decic_does_not_accept_stagnation_as_consistency() {
        for constant in [1., 1e-120, 1e120] {
            let mut m = model(|_, _, _| vec![constant]);
            assert_eq!(
                decic(&mut m, 0., &[1.], &[true], &[0.], &[false], 1e-3, &[1e-6]).unwrap_err(),
                "Convergence failure in DECIC."
            );
        }
        let mut m = model(|_, y, _| vec![y[0] * y[0] + 1.]);
        assert_eq!(
            decic(&mut m, 0., &[1.], &[false], &[0.], &[true], 1e-3, &[1e-6]).unwrap_err(),
            "Convergence failure in DECIC."
        );
    }

    #[test]
    fn implicit_decay_converges_and_reaches_variable_order() {
        let mut errors = Vec::new();
        for rtol in [1e-3, 1e-6, 1e-9] {
            let mut m = model(|_, y, yp| vec![yp[0] + 2. * y[0]]);
            let out = ode15i(
                &mut m,
                &[0., 1.],
                &[1.],
                &[-2.],
                &options(1, rtol),
                OutAt::Sol,
            )
            .unwrap();
            let error = (out.y.last().unwrap()[0] - libm::exp(-2.)).abs();
            println!(
                "rtol={rtol:e} error={error:e} stats={:?} maxorder={:?}",
                out.stats,
                out.kvec.iter().max()
            );
            assert!(out.warnings.is_empty(), "{:?}", out.warnings);
            assert_eq!(*out.t.last().unwrap(), 1.);
            assert!(error < 10. * rtol, "{error}");
            assert!(out.stats[0] < 1000);
            assert!(out.kvec.iter().any(|k| *k >= 3));
            assert_eq!(out.stats[2], m.calls);
            assert_eq!(out.stats[0] + 1, out.t.len());
            assert_eq!(out.kvec.len(), out.t.len());
            assert!((out.ypfinal[0] + 2. * out.y.last().unwrap()[0]).abs() < 1e-8);
            errors.push(error);
        }
        assert!(
            errors[1] < errors[0] * 0.1 && errors[2] < errors[1] * 0.1,
            "{errors:?}"
        );
    }

    #[test]
    fn index_one_dae_keeps_its_algebraic_constraint() {
        let mut m = model(|_, y, yp| vec![yp[0] + y[0], y[1] - 2. * y[0]]);
        let out = ode15i(
            &mut m,
            &[0., 0.5, 1.],
            &[1., 2.],
            &[-1., 0.],
            &options(2, 1e-7),
            OutAt::Requested,
        )
        .unwrap();
        assert_eq!(out.t, [0., 0.5, 1.]);
        assert!(out.warnings.is_empty());
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - libm::exp(-t)).abs() < 2e-6, "{t}: {y:?}");
            assert!((y[1] - 2. * y[0]).abs() < 1e-10);
        }
    }

    #[test]
    fn nonlinear_derivative_and_stiff_transient_match_closed_forms() {
        // y' = -sqrt(y), y(0)=1, so y(t)=(1-t/2)^2 on this interval.
        let mut m = model(|_, y, yp| vec![yp[0] * yp[0] - y[0]]);
        let out = ode15i(
            &mut m,
            &[0., 1.],
            &[1.],
            &[-1.],
            &options(1, 1e-7),
            OutAt::Steps,
        )
        .unwrap();
        assert!(out.warnings.is_empty());
        assert!((out.y.last().unwrap()[0] - 0.25).abs() < 1e-6);
        assert!((out.ypfinal[0] + 0.5).abs() < 2e-6);
        let mut m = model(|t, y, yp| vec![yp[0] + 1000. * (y[0] - libm::cos(t)) + libm::sin(t)]);
        let out = ode15i(
            &mut m,
            &[0., 1.],
            &[0.],
            &[1000.],
            &options(1, 1e-6),
            OutAt::Steps,
        )
        .unwrap();
        assert!(out.warnings.is_empty());
        assert!((out.y.last().unwrap()[0] - libm::cos(1.)).abs() < 2e-6);
        assert!(out.stats[0] < 1000, "{:?}", out.stats);
    }

    #[test]
    fn reverse_integration_and_requested_output_use_the_same_polynomial() {
        let mut m = model(|_, y, yp| vec![yp[0] + y[0]]);
        let o = options(1, 1e-8);
        let initial = libm::exp(-1.);
        let out = ode15i(
            &mut m,
            &[1., 0.5, 0.],
            &[initial],
            &[-initial],
            &o,
            OutAt::Requested,
        )
        .unwrap();
        assert_eq!(out.t, [1., 0.5, 0.]);
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - libm::exp(-t)).abs() < 5e-7);
        }
        let steps = ode15i(&mut m, &[0., 1.], &[1.], &[-1.], &o, OutAt::Sol).unwrap();
        let refined = ode15i(&mut m, &[0., 1.], &[1.], &[-1.], &o, OutAt::Refined(4)).unwrap();
        assert_eq!(refined.t.len(), 4 * (steps.t.len() - 1) + 1);
        for b in 1..steps.t.len() {
            let k = steps.kvec[b];
            let refs: Vec<&[f64]> = steps.y[b - k..=b].iter().map(Vec::as_slice).collect();
            for r in 1..=4 {
                let index = 4 * (b - 1) + r;
                let expected = lagrange(&steps.t[b - k..=b], &refs, refined.t[index]);
                assert!((refined.y[index][0] - expected[0]).abs() < 2e-13);
            }
        }
    }

    #[test]
    fn order_limits_and_inconsistent_initial_derivatives_are_supported() {
        for maxorder in 1..=5 {
            let mut m = model(|_, y, yp| vec![yp[0] + y[0]]);
            let mut o = options(1, 1e-4);
            o.maxorder = maxorder;
            let out = ode15i(&mut m, &[0., 1.], &[1.], &[0.], &o, OutAt::Sol).unwrap();
            assert!(
                out.warnings.is_empty(),
                "order={maxorder}: {:?}",
                out.warnings
            );
            assert!(out.kvec.iter().all(|&k| k >= 1 && k <= maxorder));
            assert!((out.y.last().unwrap()[0] - libm::exp(-1.)).abs() < 0.008);
        }
    }

    #[test]
    fn cancellation_is_checked_during_jacobians_and_initialisation() {
        let mut m = model(|_, y, yp| y.iter().zip(yp).map(|(a, b)| a + b).collect());
        m.cancel_after = 7;
        let error = ode15i(
            &mut m,
            &[0., 1.],
            &[1.; 8],
            &[-1.; 8],
            &options(8, 1e-6),
            OutAt::Steps,
        )
        .err()
        .unwrap();
        assert_eq!(error, "cancelled");
        assert!(m.calls < 16);
        m.checks.set(0);
        let error = decic(
            &mut m,
            0.,
            &[1.; 8],
            &[true; 8],
            &[0.; 8],
            &[false; 8],
            1e-6,
            &[1e-9],
        )
        .unwrap_err();
        assert_eq!(error, "cancelled");
    }

    #[test]
    fn bounds_and_singular_step_failure_are_reported() {
        assert!(check_workspace(100_000, 100_000).is_err());
        assert!(check_output_size(usize::MAX, 2).is_err());
        let mut m = model(|_, _, _| vec![1.]);
        let out = ode15i(
            &mut m,
            &[1., 2.],
            &[1.],
            &[0.],
            &options(1, 1e-6),
            OutAt::Steps,
        )
        .unwrap();
        assert_eq!(out.t, [1.]);
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].starts_with("Warning: Integration stopped at t=1.000000e+00;"));
        assert!(out.stats[1] > 0 && out.stats[1] < 100);
        // Each failed attempt already has fresh partials: do not recompute
        // them and repeat Newton at the same predictor before reducing h.
        assert_eq!(out.stats[3], out.stats[1]);
        assert_eq!(out.stats[4], out.stats[1]);
    }

    #[test]
    fn linear_solutions_can_start_at_large_times_and_finish_a_short_last_step() {
        let start = 1e12;
        let end = start + 1.;
        let mut o = options(1, 1e-10);
        o.hmax = 0.125;
        o.htry = Some(1e-10);
        let mut m = model(|_, _, yp| vec![yp[0] - 1.]);
        let out = ode15i(&mut m, &[start, end], &[0.], &[1.], &o, OutAt::Sol).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(*out.t.last().unwrap(), end);
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - (t - start)).abs() < 1e-12);
        }
        let short_end = start + eps_of(start);
        let out = ode15i(&mut m, &[start, short_end], &[0.], &[1.], &o, OutAt::Sol).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.t, [start, short_end]);
        assert_eq!(out.y[1][0], short_end - start);
    }

    #[test]
    fn nonlinear_index_one_constraints_converge_at_accepted_points() {
        let mut m = model(|_, y, yp| vec![yp[0] + y[1], y[1] - y[0] * y[0]]);
        let mut o = options(2, 1e-8);
        o.hmax = 0.3;
        let out = ode15i(&mut m, &[0., 5.], &[1., 1.], &[-1., -2.], &o, OutAt::Sol).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(*out.t.last().unwrap(), 5.);
        assert!(out.stats[0] < 1000, "{:?}", out.stats);
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - 1. / (1. + t)).abs() < 2e-7, "{t}: {y:?}");
            assert!((y[1] - y[0] * y[0]).abs() < 1e-10, "{t}: {y:?}");
        }
    }

    #[test]
    fn residual_scaling_preserves_the_implicit_solution() {
        for factor in [1e-80, 1., 1e80] {
            let mut m = model(|_, y, yp| vec![factor * (yp[0] + y[0])]);
            let out = ode15i(
                &mut m,
                &[0., 1.],
                &[1.],
                &[-1.],
                &options(1, 1e-8),
                OutAt::Steps,
            )
            .unwrap();
            assert!(out.warnings.is_empty());
            assert!((out.y.last().unwrap()[0] - libm::exp(-1.)).abs() < 2e-7);
        }
    }
}
