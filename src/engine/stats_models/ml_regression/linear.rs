//! Half-MSE plus lambda/2 ||beta||^2 or lambda ||beta||_1; intercept
//! is unpenalized. Centered augmented QR and coordinate descent respectively.
use super::*;
#[derive(Clone, Debug)]
pub(super) struct Linear {
    pub beta: Vec<f64>,
    pub bias: f64,
}
pub(super) fn train(
    e: &mut Engine,
    x: &Matrix,
    y: &[f64],
    svm: bool,
    lasso: bool,
    lambda: f64,
    epsilon: Option<f64>,
) -> Result<Linear> {
    let (n, p) = (x.rows(), x.cols());
    if svm {
        if lasso || lambda == 0. {
            return l1_svm(
                e,
                x,
                y,
                lambda,
                epsilon.unwrap_or_else(|| default_epsilon(y)),
            );
        }
        let s = super::svm::train(
            e,
            x,
            y,
            &SvmOptions {
                kernel: "linear".into(),
                scale: Some(1.),
                order: 3,
                c: Some(1. / (n as f64 * lambda)),
                epsilon: Some(epsilon.unwrap_or_else(|| default_epsilon(y))),
                standardize: false,
            },
        )?;
        let beta = (0..p)
            .map(|j| {
                s.alpha
                    .iter()
                    .enumerate()
                    .map(|(i, a)| a * x.data()[i + j * n])
                    .sum()
            })
            .collect();
        return Ok(Linear { beta, bias: s.bias });
    }
    let (means, _) = means_scales(x);
    let ym = y.iter().sum::<f64>() / n as f64;
    let z = DMatrix::from_fn(n, p, |i, j| x.data()[i + j * n] - means[j]);
    let target = DVector::from_iterator(n, y.iter().map(|y| y - ym));
    let beta = if lasso {
        let mut beta = vec![0.; p];
        let mut residual = target.as_slice().to_vec();
        let mut converged = false;
        let norms: Vec<_> = (0..p)
            .map(|j| z.column(j).norm_squared() / n as f64)
            .collect();
        if n.saturating_mul(p) > 5_000_000 {
            return Err("fitrlinear: coordinate descent workspace limit exceeded".into());
        }
        for iteration in 0usize..100_000 {
            e.check_cancel()?;
            if iteration.saturating_mul(n).saturating_mul(p) > 200_000_000 {
                return Err("fitrlinear: coordinate descent work limit exceeded".into());
            }
            let mut change: f64 = 0.;
            for j in 0..p {
                if norms[j] == 0. {
                    continue;
                }
                let rho = (0..n).map(|i| z[(i, j)] * residual[i]).sum::<f64>() / n as f64
                    + norms[j] * beta[j];
                let b = rho.signum() * (rho.abs() - lambda).max(0.) / norms[j];
                let delta = b - beta[j];
                for i in 0..n {
                    residual[i] -= z[(i, j)] * delta;
                }
                change = change.max(delta.abs());
                beta[j] = b;
            }
            if change < 1e-12 {
                let violation = (0..p)
                    .map(|j| {
                        let grad = -(0..n).map(|i| z[(i, j)] * residual[i]).sum::<f64>() / n as f64;
                        if beta[j] == 0. {
                            (grad.abs() - lambda).max(0.)
                        } else {
                            (grad + lambda * beta[j].signum()).abs()
                        }
                    })
                    .fold(0., f64::max);
                if violation < 1e-10 {
                    converged = true;
                    break;
                }
            }
        }
        if !converged {
            return Err("fitrlinear: lasso did not converge".into());
        }
        beta
    } else {
        size(e, "fitrlinear", n + p, p)?;
        let mut augmented = DMatrix::zeros(n + p, p);
        augmented.view_mut((0, 0), (n, p)).copy_from(&z);
        for j in 0..p {
            augmented[(n + j, j)] = (n as f64 * lambda).sqrt();
        }
        let mut rhs = DVector::zeros(n + p);
        rhs.rows_mut(0, n).copy_from(&target);
        super::super::regression::least_squares(e, &augmented, &rhs)?
            .b
            .as_slice()
            .to_vec()
    };
    let bias = ym - beta.iter().zip(&means).map(|(a, b)| a * b).sum::<f64>();
    if !bias.is_finite() || beta.iter().any(|v| !v.is_finite()) {
        return Err("fitrlinear: numerical overflow; rescale the data".into());
    }
    Ok(Linear { beta, bias })
}
// L1 epsilon-insensitive regression is a linear program. A deterministic
// primal-dual hybrid gradient method solves its saddle form. The dual prox is
// soft-thresholding by epsilon followed by projection to [-1/n,1/n]; the primal
// prox is soft-thresholding by lambda, leaving the intercept unpenalized.
fn l1_svm(e: &Engine, x: &Matrix, y: &[f64], lambda: f64, epsilon: f64) -> Result<Linear> {
    let (n, p) = (x.rows(), x.cols());
    let norm = (x.data().iter().map(|x| x * x).sum::<f64>() + n as f64).sqrt();
    if !norm.is_finite() {
        return Err("fitrlinear: predictor overflow".into());
    }
    let step = 0.99 / norm.max(1.);
    let mut beta = vec![0.; p + 1];
    let mut extrap = beta.clone();
    let mut dual = vec![0.; n];
    for iteration in 0usize..1_000_000 {
        if iteration.is_multiple_of(64) {
            e.check_cancel()?;
        }
        if iteration.saturating_mul(n).saturating_mul(p + 1) > 100_000_000 {
            return Err("fitrlinear: L1 SVM work limit exceeded before convergence".into());
        }
        for i in 0..n {
            let pred = extrap[p] + (0..p).map(|j| x.data()[i + j * n] * extrap[j]).sum::<f64>();
            let a = dual[i] + step * (pred - y[i]);
            dual[i] = (a.signum() * (a.abs() - step * epsilon).max(0.))
                .clamp(-1. / n as f64, 1. / n as f64);
        }
        let old = beta.clone();
        for j in 0..=p {
            let grad = (0..n)
                .map(|i| dual[i] * if j == p { 1. } else { x.data()[i + j * n] })
                .sum::<f64>();
            let a = beta[j] - step * grad;
            beta[j] = if j == p {
                a
            } else {
                a.signum() * (a.abs() - step * lambda).max(0.)
            };
            extrap[j] = 2. * beta[j] - old[j];
        }
        if iteration.is_multiple_of(32) {
            let objective = (0..n)
                .map(|i| {
                    let r =
                        beta[p] + (0..p).map(|j| x.data()[i + j * n] * beta[j]).sum::<f64>() - y[i];
                    (r.abs() - epsilon).max(0.) / n as f64
                })
                .sum::<f64>()
                + lambda * beta[..p].iter().map(|v| v.abs()).sum::<f64>();
            let feasible = dual.iter().sum::<f64>().abs() < 1e-9
                && (0..p).all(|j| {
                    (0..n)
                        .map(|i| dual[i] * x.data()[i + j * n])
                        .sum::<f64>()
                        .abs()
                        <= lambda + 1e-9
                });
            let lower = -dual
                .iter()
                .zip(y)
                .map(|(a, y)| a * y + epsilon * a.abs())
                .sum::<f64>();
            if feasible && (objective - lower).abs() < 1e-9 {
                return Ok(Linear {
                    beta: beta[..p].to_vec(),
                    bias: beta[p],
                });
            }
        }
    }
    Err("fitrlinear: L1 SVM did not converge".into())
}
