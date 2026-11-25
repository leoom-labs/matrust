//! Minimisers for likelihood fits: Nelder-Mead (J. A. Nelder and R. Mead,
//! Comput. J. 7 (1965); coefficients and ordering as in Lagarias, Reeds,
//! Wright and Wright, SIAM J. Optim. 9 (1998)) followed by Newton steps with
//! a central-difference gradient and Hessian, plus the Hessian itself for
//! the parameter covariance. Objectives return +Inf outside their domain.
use crate::Result;

#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub max_iter: usize,
    pub max_evals: usize,
    pub tol_x: f64,
    pub tol_fun: f64,
}

impl Limits {
    /// Tight defaults for the built-in families (the Newton polish then
    /// removes the remaining simplex error).
    pub fn family(n: usize) -> Self {
        Self {
            max_iter: 2000 * n.max(1),
            max_evals: 4000 * n.max(1),
            tol_x: 1e-10,
            tol_fun: 1e-12,
        }
    }
    /// MATLAB's documented statset defaults for mle with a custom
    /// distribution.
    pub fn custom(n: usize) -> Self {
        Self {
            max_iter: 200 * n.max(1),
            max_evals: 400 * n.max(1),
            tol_x: 1e-6,
            tol_fun: 1e-6,
        }
    }
}

type Objective<'a> = dyn FnMut(&[f64]) -> Result<f64> + 'a;

fn ascending(a: f64, b: f64) -> std::cmp::Ordering {
    // NaN sorts last, as +Inf does.
    let key = |v: f64| if v.is_nan() { f64::INFINITY } else { v };
    key(a).total_cmp(&key(b))
}

/// Nelder-Mead from x0 with initial edge lengths `step`.
pub(super) fn nelder_mead(
    f: &mut Objective<'_>,
    x0: &[f64],
    step: &[f64],
    limits: &Limits,
) -> Result<(Vec<f64>, f64)> {
    let n = x0.len();
    let mut simplex: Vec<(f64, Vec<f64>)> = Vec::with_capacity(n + 1);
    let f0 = f(x0)?;
    if !f0.is_finite() {
        return Err("the likelihood is not finite at the starting point".into());
    }
    simplex.push((f0, x0.to_vec()));
    for j in 0..n {
        let mut v = x0.to_vec();
        v[j] += step[j];
        let fv = f(&v)?;
        simplex.push((fv, v));
    }
    let mut evaluations = n + 1;
    let mut iterations = 0;
    let centroid = |s: &[(f64, Vec<f64>)]| -> Vec<f64> {
        let mut c = vec![0.; n];
        for (_, v) in &s[..n] {
            for (ci, vi) in c.iter_mut().zip(v) {
                *ci += vi / n as f64;
            }
        }
        c
    };
    let along = |c: &[f64], w: &[f64], t: f64| -> Vec<f64> {
        c.iter().zip(w).map(|(ci, wi)| ci + t * (wi - ci)).collect()
    };
    loop {
        simplex.sort_by(|a, b| ascending(a.0, b.0));
        let (fb, best) = (simplex[0].0, simplex[0].1.clone());
        let fspread = simplex[1..]
            .iter()
            .map(|(fv, _)| (fv - fb).abs())
            .fold(0., f64::max);
        let xspread = simplex[1..]
            .iter()
            .flat_map(|(_, v)| v.iter().zip(&best).map(|(a, b)| (a - b).abs()))
            .fold(0., f64::max);
        let xscale = best.iter().fold(1f64, |m, v| m.max(v.abs()));
        if (fspread <= limits.tol_fun.max(10. * f64::EPSILON * fb.abs())
            && xspread <= limits.tol_x * xscale)
            || iterations >= limits.max_iter
            || evaluations >= limits.max_evals
        {
            return Ok((best, fb));
        }
        iterations += 1;
        let c = centroid(&simplex);
        let worst = simplex[n].1.clone();
        let fw = simplex[n].0;
        let fsecond = simplex[n - 1].0;
        let r = along(&c, &worst, -1.);
        let fr = f(&r)?;
        evaluations += 1;
        if ascending(fr, fb).is_lt() {
            let x = along(&c, &worst, -2.);
            let fx = f(&x)?;
            evaluations += 1;
            simplex[n] = if ascending(fx, fr).is_lt() {
                (fx, x)
            } else {
                (fr, r)
            };
            continue;
        }
        if ascending(fr, fsecond).is_lt() {
            simplex[n] = (fr, r);
            continue;
        }
        let (point, fp) = if ascending(fr, fw).is_lt() {
            let x = along(&c, &worst, -0.5);
            (x, fr)
        } else {
            let x = along(&c, &worst, 0.5);
            (x, fw)
        };
        let fc = f(&point)?;
        evaluations += 1;
        if ascending(fc, fp).is_le() {
            simplex[n] = (fc, point);
            continue;
        }
        // Shrink towards the best vertex.
        for vertex in simplex.iter_mut().skip(1) {
            let v: Vec<f64> = vertex
                .1
                .iter()
                .zip(&best)
                .map(|(vi, bi)| bi + 0.5 * (vi - bi))
                .collect();
            let fv = f(&v)?;
            *vertex = (fv, v);
        }
        evaluations += n;
    }
}

/// Central-difference gradient and Hessian with steps h.
pub(super) fn derivatives(
    f: &mut Objective<'_>,
    x: &[f64],
    f0: f64,
    h: &[f64],
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n = x.len();
    let mut g = vec![0.; n];
    let mut hess = vec![0.; n * n];
    let at = |dx: &[(usize, f64)], f: &mut Objective<'_>| -> Result<f64> {
        let mut y = x.to_vec();
        for &(i, d) in dx {
            y[i] += d;
        }
        f(&y)
    };
    for i in 0..n {
        let fp = at(&[(i, h[i])], f)?;
        let fm = at(&[(i, -h[i])], f)?;
        g[i] = (fp - fm) / (2. * h[i]);
        hess[i * n + i] = (fp - 2. * f0 + fm) / (h[i] * h[i]);
        for j in 0..i {
            let fpp = at(&[(i, h[i]), (j, h[j])], f)?;
            let fpm = at(&[(i, h[i]), (j, -h[j])], f)?;
            let fmp = at(&[(i, -h[i]), (j, h[j])], f)?;
            let fmm = at(&[(i, -h[i]), (j, -h[j])], f)?;
            let v = (fpp - fpm - fmp + fmm) / (4. * h[i] * h[j]);
            hess[i * n + j] = v;
            hess[j * n + i] = v;
        }
    }
    Ok((g, hess))
}

/// Solves a x = b for a small dense symmetric or general matrix by Gaussian
/// elimination with partial pivoting; None when singular.
pub(super) fn solve(n: usize, a: &[f64], b: &[f64]) -> Option<Vec<f64>> {
    let mut m = a.to_vec();
    let mut x = b.to_vec();
    let at = |r: usize, c: usize| c * n + r;
    for col in 0..n {
        let pivot =
            (col..n).max_by(|&p, &q| m[at(p, col)].abs().total_cmp(&m[at(q, col)].abs()))?;
        if m[at(pivot, col)] == 0. || !m[at(pivot, col)].is_finite() {
            return None;
        }
        if pivot != col {
            for c in 0..n {
                m.swap(at(pivot, c), at(col, c));
            }
            x.swap(pivot, col);
        }
        for r in col + 1..n {
            let factor = m[at(r, col)] / m[at(col, col)];
            for c in col..n {
                m[at(r, c)] -= factor * m[at(col, c)];
            }
            x[r] -= factor * x[col];
        }
    }
    for col in (0..n).rev() {
        let mut v = x[col];
        for c in col + 1..n {
            v -= m[at(col, c)] * x[c];
        }
        x[col] = v / m[at(col, col)];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// The inverse of a small matrix, column by column through `solve`.
pub(super) fn inverse(n: usize, a: &[f64]) -> Option<Vec<f64>> {
    let mut out = vec![0.; n * n];
    for j in 0..n {
        let mut e = vec![0.; n];
        e[j] = 1.;
        let col = solve(n, a, &e)?;
        out[j * n..(j + 1) * n].copy_from_slice(&col);
    }
    // Symmetrise the round-off of a covariance.
    for i in 0..n {
        for j in 0..i {
            let v = (out[i * n + j] + out[j * n + i]) / 2.;
            out[i * n + j] = v;
            out[j * n + i] = v;
        }
    }
    Some(out)
}

/// Newton steps from x with a step-halving line search; each accepted step
/// lowers f. Stops at a non-descent direction or a negligible step.
pub(super) fn polish(
    f: &mut Objective<'_>,
    mut x: Vec<f64>,
    mut fx: f64,
    h: &[f64],
) -> Result<(Vec<f64>, f64)> {
    let n = x.len();
    for _ in 0..30 {
        let (g, hess) = derivatives(f, &x, fx, h)?;
        if g.iter().any(|v| !v.is_finite()) || hess.iter().any(|v| !v.is_finite()) {
            break;
        }
        let minus: Vec<f64> = g.iter().map(|v| -v).collect();
        let Some(d) = solve(n, &hess, &minus) else {
            break;
        };
        // Newton direction must descend.
        if d.iter().zip(&g).map(|(a, b)| a * b).sum::<f64>() >= 0. {
            break;
        }
        let mut t = 1.;
        let mut accepted = false;
        // Near the minimum f changes by less than its rounding, so a full
        // Newton step that keeps f within that noise is trusted (the
        // gradient, not f, then decides the last digits).
        let noise = 64. * f64::EPSILON * fx.abs().max(1.);
        for _ in 0..30 {
            let y: Vec<f64> = x.iter().zip(&d).map(|(a, b)| a + t * b).collect();
            let fy = f(&y)?;
            if fy < fx || (t == 1. && fy <= fx + noise) {
                let small = y
                    .iter()
                    .zip(&x)
                    .zip(h)
                    .all(|((a, b), s)| (a - b).abs() <= 1e-6 * s);
                x = y;
                fx = fy;
                accepted = !small;
                break;
            }
            t /= 2.;
        }
        if !accepted {
            break;
        }
    }
    Ok((x, fx))
}

/// Hessian alone (for covariances) at x.
pub(super) fn hessian(f: &mut Objective<'_>, x: &[f64], h: &[f64]) -> Result<Vec<f64>> {
    let f0 = f(x)?;
    Ok(derivatives(f, x, f0, h)?.1)
}
