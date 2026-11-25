//! Deterministic convex optimisation of mean loss plus lambda penalty.
use super::*;
use nalgebra::DVector;
fn soft(x: f64, t: f64) -> f64 {
    x.signum() * (x.abs() - t).max(0.)
}
pub(super) fn fit(
    e: &Engine,
    x: &Matrix,
    codes: &[usize],
    logistic: bool,
    lasso: bool,
    lambda: f64,
) -> Result<(Vec<f64>, f64)> {
    if !logistic && !lasso {
        let c = 1. / (lambda * x.rows() as f64);
        if !c.is_finite() || c <= 0. {
            return Err("fitclinear: Lambda is outside the solver numerical range".into());
        }
        let (a, b) = super::classifiers::smo(e, x, codes, c, "linear", 1.)?;
        return Ok((
            (0..x.cols())
                .map(|j| {
                    a.iter()
                        .enumerate()
                        .map(|(i, a)| {
                            a * if codes[i] == 1 { 1. } else { -1. } * x.data()[i + j * x.rows()]
                        })
                        .sum()
                })
                .collect(),
            b,
        ));
    }
    if !logistic {
        return hinge_lasso(e, x, codes, lambda);
    }
    let n = x.rows();
    let p = x.cols();
    let lipschitz = 0.25 * (1. + x.data().iter().map(|z| z * z).sum::<f64>() / n as f64)
        + if lasso { 0. } else { lambda };
    if !lipschitz.is_finite() {
        return Err("fitclinear: predictor scale overflows; rescale predictors".into());
    }
    let step = 1. / lipschitz;
    let mut theta = vec![0.; p + 1];
    let mut extrap = theta.clone();
    let mut t = 1.0f64;
    for iteration in 0usize..100000 {
        if iteration.is_multiple_of(32) {
            e.check_cancel()?;
        }
        let mut grad = vec![0.; p + 1];
        for (i, &code) in codes.iter().enumerate() {
            let y = if code == 1 { 1. } else { -1. };
            let f = extrap[p] + (0..p).map(|j| extrap[j] * x.data()[i + j * n]).sum::<f64>();
            let d = -y * super::ml::sigmoid(-y * f) / n as f64;
            for (j, g) in grad.iter_mut().take(p).enumerate() {
                *g += d * x.data()[i + j * n];
            }
            grad[p] += d;
        }
        let mut next = vec![0.; p + 1];
        for j in 0..=p {
            let z = extrap[j]
                - step
                    * (grad[j]
                        + if !lasso && j < p {
                            lambda * extrap[j]
                        } else {
                            0.
                        });
            next[j] = if lasso && j < p {
                soft(z, step * lambda)
            } else {
                z
            };
        }
        let residual = next
            .iter()
            .zip(&extrap)
            .map(|(a, b)| (a - b).abs() / step)
            .fold(0., f64::max);
        if residual < 1e-10 {
            let bias = next.pop().unwrap();
            return Ok((next, bias));
        }
        if (iteration as u128 + 1) * (x.len() as u128) > 300_000_000 {
            return Err("fitclinear: optimisation work limit exceeded before convergence".into());
        }
        let nt = (1. + (1. + 4. * t * t).sqrt()) / 2.;
        // Adaptive restart prevents acceleration oscillations near L1 corners.
        let restart = next
            .iter()
            .zip(&theta)
            .zip(&extrap)
            .map(|((a, b), c)| (c - a) * (a - b))
            .sum::<f64>()
            > 0.;
        for j in 0..=p {
            extrap[j] = next[j]
                + if restart {
                    0.
                } else {
                    (t - 1.) / nt * (next[j] - theta[j])
                };
        }
        theta = next;
        t = if restart { 1. } else { nt };
    }
    Err("fitclinear: optimisation iteration limit exceeded before convergence".into())
}
fn hinge_lasso(e: &Engine, x: &Matrix, codes: &[usize], lambda: f64) -> Result<(Vec<f64>, f64)> {
    let n = x.rows();
    let p = x.cols();
    if n as u128 * p as u128 * p as u128 > 100_000_000 {
        return Err("fitclinear: solver workspace work limit exceeded".into());
    }
    size(e, "fitclinear", p + 1, p + 1)?;
    let a = DMatrix::from_fn(n, p + 1, |i, j| {
        if j == p {
            if codes[i] == 1 { 1. } else { -1. }
        } else {
            x.data()[i + j * n] * if codes[i] == 1 { 1. } else { -1. }
        }
    });
    let mut gram = a.transpose() * &a;
    for j in 0..p {
        gram[(j, j)] += 1.;
    }
    let factor = gram
        .cholesky()
        .ok_or("fitclinear: singular solver system")?;
    let mut z = DVector::zeros(n);
    let mut u = DVector::zeros(n);
    let mut v = DVector::zeros(p + 1);
    let mut w = DVector::zeros(p + 1);
    let mut theta = DVector::zeros(p + 1);
    let mut rho = 1. / n as f64;
    for iter in 0usize..50000 {
        if iter.is_multiple_of(32) {
            e.check_cancel()?;
        }
        let mut rhs = a.transpose() * (&z - &u);
        for j in 0..p {
            rhs[j] += v[j] - w[j];
        }
        theta = factor.solve(&rhs);
        let at = &a * &theta;
        let oldz = z.clone();
        let oldv = v.clone();
        for i in 0..n {
            let q = at[i] + u[i];
            z[i] = if q > 1. {
                q
            } else if q < 1. - 1. / (n as f64 * rho) {
                q + 1. / (n as f64 * rho)
            } else {
                1.
            };
            u[i] += at[i] - z[i];
        }
        for j in 0..p {
            v[j] = soft(theta[j] + w[j], lambda / rho);
            w[j] += theta[j] - v[j];
        }
        let primal = (&at - &z)
            .amax()
            .max((0..p).map(|j| (theta[j] - v[j]).abs()).fold(0., f64::max));
        let mut dual = a.transpose() * (&z - &oldz);
        for j in 0..p {
            dual[j] += v[j] - oldv[j];
        }
        let dual = dual.amax() * rho;
        if primal < 1e-9 && dual < 1e-10 {
            return Ok((theta.as_slice()[..p].to_vec(), theta[p]));
        }
        if iter.is_multiple_of(50) {
            let old = rho;
            if primal > 10. * dual {
                rho *= 2.;
            } else if dual > 10. * primal {
                rho /= 2.;
            }
            u *= old / rho;
            w *= old / rho;
        }
        if (iter as u128 + 1) * x.len() as u128 > 300_000_000 {
            return Err("fitclinear: optimisation work limit exceeded before convergence".into());
        }
    }
    let _ = theta;
    Err("fitclinear: ADMM iteration limit exceeded before convergence".into())
}
