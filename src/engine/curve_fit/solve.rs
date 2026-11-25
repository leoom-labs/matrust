//! Least-squares kernels of the curve fitting functions: column-pivoted QR
//! for linear models, the unscaled covariance inv(J'J) and leverages from the
//! same factorization, and a bounded Levenberg-Marquardt / trust-region
//! iteration over a Rust model (no engine function handles involved).
use crate::Result;
use nalgebra::{DMatrix, DVector};

/// Column norms with zero columns counted as 1 (scaling must stay invertible).
fn column_scales(a: &DMatrix<f64>) -> Vec<f64> {
    (0..a.ncols())
        .map(|j| {
            let n = a.column(j).norm();
            if n > 0. && n.is_finite() { n } else { 1. }
        })
        .collect()
}

/// Positions of the original columns in a pivoted factorization: entry k is the
/// original index of R's column k.
fn pivot_order(
    qr: &nalgebra::linalg::ColPivQR<f64, nalgebra::Dyn, nalgebra::Dyn>,
    p: usize,
) -> Vec<usize> {
    let mut idx = DVector::from_iterator(p, (0..p).map(|i| i as f64));
    qr.p().permute_rows(&mut idx);
    idx.iter().map(|&v| v as usize).collect()
}

/// Least-squares solution of a x = b by column-pivoted Householder QR of the
/// column-equilibrated a; dependent columns get 0 (basic solution). Returns the
/// solution and the numerical rank.
pub(super) fn lstsq(a: &DMatrix<f64>, b: &[f64]) -> (Vec<f64>, usize) {
    let (n, p) = a.shape();
    if p == 0 || n == 0 {
        return (vec![0.; p], 0);
    }
    let scales = column_scales(a);
    let mut scaled = a.clone();
    for (j, s) in scales.iter().enumerate() {
        scaled.column_mut(j).scale_mut(1. / s);
    }
    let qr = scaled.col_piv_qr();
    let r = qr.r();
    let qtb = qr.q().transpose() * DVector::from_column_slice(b);
    let k = n.min(p);
    let tol = n.max(p) as f64 * f64::EPSILON * r[(0, 0)].abs();
    let rank = (0..k).take_while(|&i| r[(i, i)].abs() > tol).count();
    let mut x = DVector::zeros(p);
    if rank > 0
        && let Some(head) = r
            .view((0, 0), (rank, rank))
            .solve_upper_triangular(&qtb.rows(0, rank).into_owned())
    {
        x.rows_mut(0, rank).copy_from(&head);
    }
    qr.p().inv_permute_rows(&mut x);
    let x = x.iter().zip(&scales).map(|(v, s)| v / s).collect();
    (x, rank)
}

/// inv(J'J) (p-by-p, column-major) from a pivoted QR of the equilibrated J;
/// rows and columns of dependent coefficients are NaN.
pub(super) fn covariance(j: &DMatrix<f64>) -> Vec<f64> {
    let (n, p) = j.shape();
    let mut cov = vec![f64::NAN; p * p];
    if p == 0 || n == 0 || j.iter().any(|v| !v.is_finite()) {
        return cov;
    }
    let scales = column_scales(j);
    let mut scaled = j.clone();
    for (k, s) in scales.iter().enumerate() {
        scaled.column_mut(k).scale_mut(1. / s);
    }
    let qr = scaled.col_piv_qr();
    let r = qr.r();
    let order = pivot_order(&qr, p);
    let tol = n.max(p) as f64 * f64::EPSILON * r[(0, 0)].abs();
    let rank = (0..n.min(p)).take_while(|&i| r[(i, i)].abs() > tol).count();
    if rank == 0 {
        return cov;
    }
    let Some(rinv) = r
        .view((0, 0), (rank, rank))
        .solve_upper_triangular(&DMatrix::identity(rank, rank))
    else {
        return cov;
    };
    let c = &rinv * rinv.transpose();
    for a in 0..rank {
        for b in 0..rank {
            let (i, k) = (order[a], order[b]);
            cov[i + k * p] = c[(a, b)] / (scales[i] * scales[k]);
        }
    }
    cov
}

/// Diagonal of the hat matrix J inv(J'J) J' (leverages), from the same QR.
pub(super) fn leverages(j: &DMatrix<f64>) -> Vec<f64> {
    let (n, p) = j.shape();
    if p == 0 || n == 0 {
        return vec![0.; n];
    }
    let scales = column_scales(j);
    let mut scaled = j.clone();
    for (k, s) in scales.iter().enumerate() {
        scaled.column_mut(k).scale_mut(1. / s);
    }
    let qr = scaled.col_piv_qr();
    let r = qr.r();
    let tol = n.max(p) as f64 * f64::EPSILON * r[(0, 0)].abs();
    let rank = (0..n.min(p)).take_while(|&i| r[(i, i)].abs() > tol).count();
    let q = qr.q();
    (0..n)
        .map(|i| (0..rank).map(|k| q[(i, k)] * q[(i, k)]).sum())
        .collect()
}

/// The weighted least-squares problem a nonlinear fit iterates on.
pub(super) trait Problem {
    /// sqrt(w) .* (f(b) - y); may contain NaN or Inf where the model fails.
    fn residual(&mut self, b: &[f64]) -> Result<Vec<f64>>;
    /// The n-by-p Jacobian of the residual at b and the number of extra
    /// model evaluations it took.
    fn jacobian(&mut self, b: &[f64]) -> Result<(DMatrix<f64>, usize)>;
    fn check(&mut self) -> Result<()>;
    /// One line of 'Display','iter' output.
    fn log(&mut self, text: String) -> Result<()>;
}

pub(super) struct Settings {
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub max_iter: usize,
    pub max_fev: usize,
    pub tol_fun: f64,
    pub tol_x: f64,
    pub iter_display: bool,
}

pub(super) struct Outcome {
    pub x: Vec<f64>,
    pub iterations: usize,
    pub func_count: usize,
    pub exitflag: i32,
    pub firstorder: f64,
    pub message: String,
}

pub(super) const NAN_START: &str = "The model gives NaN or Inf values at the start point, so the fit cannot continue. Give a StartPoint or tighter Lower and Upper bounds.";
/// Converged fits take up to this many further steps while the sum of squares
/// still decreases, so results do not depend on how loose TolFun/TolX are.
const POLISH_STEPS: usize = 10;

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Projected Levenberg-Marquardt with Marquardt scaling (Moré's trust-region
/// form): each step solves [J_F; sqrt(lambda) D_F] dx = [-r; 0] by QR over the
/// free coefficients F (those not held at a bound by the gradient) and
/// projects the result into [lower, upper].
pub(super) fn levenberg_marquardt(
    problem: &mut dyn Problem,
    start: &[f64],
    s: &Settings,
) -> Result<Outcome> {
    let np = start.len();
    let clamp = |x: &mut [f64]| {
        for (j, xj) in x.iter_mut().enumerate().take(np) {
            *xj = xj.max(s.lower[j]).min(s.upper[j]);
        }
    };
    let mut x = start.to_vec();
    clamp(&mut x);
    let mut r = problem.residual(&x)?;
    let mut fev = 1;
    if r.iter().any(|v| !v.is_finite()) {
        return Err(NAN_START.into());
    }
    let mut cost = dot(&r, &r);
    let (mut jac, extra) = problem.jacobian(&x)?;
    fev += extra;
    let mut scale = column_scales(&jac);
    let mut lambda = 1e-3_f64;
    let mut nu = 2.;
    let mut iter = 0;
    let mut exitflag = 0;
    let mut message = String::new();
    let mut polish: Option<usize> = None;
    if s.iter_display {
        problem.log(format!(
            "{:>10} {:>12} {:>16} {:>14}",
            "Iteration", "Func-count", "Sum of squares", "Step norm"
        ))?;
        problem.log(format!("{:>10} {:>12} {:>16.8e} {:>14}", 0, fev, cost, ""))?;
    }
    'outer: loop {
        problem.check()?;
        let g: Vec<f64> = (0..np).map(|j| dot(jac.column(j).as_slice(), &r)).collect();
        let free: Vec<usize> = (0..np)
            .filter(|&j| !((x[j] <= s.lower[j] && g[j] > 0.) || (x[j] >= s.upper[j] && g[j] < 0.)))
            .collect();
        let gmax = free.iter().map(|&j| g[j].abs()).fold(0., f64::max);
        if free.is_empty() || gmax == 0. {
            if polish.is_none() {
                exitflag = 1;
                message = "Success: the first-order optimality measure is zero.".into();
            }
            break;
        }
        if iter >= s.max_iter {
            if polish.is_none() {
                exitflag = 0;
                message = format!(
                    "Stopped: the number of iterations reached MaxIter ({}).",
                    s.max_iter
                );
            }
            break;
        }
        let k = free.len();
        let jf = jac.select_columns(&free);
        let qr = jf.qr();
        let rr = qr.r();
        let qtr = qr.q().transpose() * DVector::from_column_slice(&r);
        let m = rr.nrows();
        let mut rejects = 0;
        loop {
            if fev >= s.max_fev {
                if polish.is_none() {
                    exitflag = 0;
                    message = format!(
                        "Stopped: the number of model evaluations reached MaxFunEvals ({}).",
                        s.max_fev
                    );
                }
                break 'outer;
            }
            let mut a = DMatrix::zeros(m + k, k);
            a.view_mut((0, 0), (m, k)).copy_from(&rr);
            for (i, &j) in free.iter().enumerate() {
                a[(m + i, i)] = lambda.sqrt() * scale[j];
            }
            let mut rhs = DVector::zeros(m + k);
            for i in 0..m {
                rhs[i] = -qtr[i];
            }
            let qr2 = a.qr();
            let Some(dx) = qr2.r().solve_upper_triangular(&(qr2.q().transpose() * rhs)) else {
                lambda *= nu;
                nu *= 2.;
                if lambda > 1e16 {
                    break 'outer;
                }
                continue;
            };
            let mut xn = x.clone();
            for (i, &j) in free.iter().enumerate() {
                xn[j] += dx[i];
            }
            clamp(&mut xn);
            let step: Vec<f64> = xn.iter().zip(&x).map(|(a, b)| a - b).collect();
            let snorm = dot(&step, &step).sqrt();
            if snorm == 0. || step.iter().any(|v| !v.is_finite()) {
                if polish.is_none() {
                    exitflag = 2;
                    message = "Success: the change in coefficients is below TolX.".into();
                }
                break 'outer;
            }
            let rn = problem.residual(&xn)?;
            fev += 1;
            let cn = if rn.iter().all(|v| v.is_finite()) {
                dot(&rn, &rn)
            } else {
                f64::INFINITY
            };
            if cn < cost {
                let js = &jac * DVector::from_column_slice(&step);
                let lin: f64 = r
                    .iter()
                    .zip(js.iter())
                    .map(|(a, b)| (a + b) * (a + b))
                    .sum();
                let pred = cost - lin;
                let rho = if pred > 0. { (cost - cn) / pred } else { 1. };
                lambda = (lambda * (1. - (2. * rho - 1.).powi(3)).max(1. / 3.)).max(1e-20);
                nu = 2.;
                iter += 1;
                let dcost = cost - cn;
                let old = cost;
                let xnorm = dot(&xn, &xn).sqrt();
                x = xn;
                r = rn;
                cost = cn;
                let (j2, extra) = problem.jacobian(&x)?;
                fev += extra;
                jac = j2;
                for (sc, n) in scale.iter_mut().zip(column_scales(&jac)) {
                    *sc = sc.max(n);
                }
                if s.iter_display {
                    problem.log(format!(
                        "{:>10} {:>12} {:>16.8e} {:>14.6e}",
                        iter, fev, cost, snorm
                    ))?;
                }
                match polish.as_mut() {
                    Some(steps) => {
                        *steps += 1;
                        if *steps >= POLISH_STEPS
                            || dcost <= 1e-15 * old
                            || snorm <= 1e-15 * (f64::EPSILON.sqrt() + xnorm)
                        {
                            break 'outer;
                        }
                    }
                    None => {
                        let small_x = snorm <= s.tol_x * (f64::EPSILON.sqrt() + xnorm);
                        let small_f = dcost <= s.tol_fun * old;
                        if small_x || small_f {
                            exitflag = if small_f { 3 } else { 2 };
                            message = if small_f {
                                "Success: the change in residuals is below TolFun.".into()
                            } else {
                                "Success: the change in coefficients is below TolX.".into()
                            };
                            polish = Some(0);
                        }
                    }
                }
                continue 'outer;
            }
            rejects += 1;
            lambda *= nu;
            nu *= 2.;
            if (polish.is_some() && rejects >= 3) || lambda > 1e16 {
                if polish.is_none() {
                    exitflag = 3;
                    message = "Success: the residuals cannot be reduced further.".into();
                }
                break 'outer;
            }
        }
    }
    let g: Vec<f64> = (0..np).map(|j| dot(jac.column(j).as_slice(), &r)).collect();
    let firstorder = (0..np)
        .filter(|&j| !((x[j] <= s.lower[j] && g[j] > 0.) || (x[j] >= s.upper[j] && g[j] < 0.)))
        .map(|j| 2. * g[j].abs())
        .fold(0., f64::max);
    Ok(Outcome {
        x,
        iterations: iter,
        func_count: fev,
        exitflag,
        firstorder,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Exp {
        x: Vec<f64>,
        y: Vec<f64>,
    }
    impl Problem for Exp {
        fn residual(&mut self, b: &[f64]) -> Result<Vec<f64>> {
            Ok(self
                .x
                .iter()
                .zip(&self.y)
                .map(|(x, y)| b[0] * (b[1] * x).exp() - y)
                .collect())
        }
        fn jacobian(&mut self, b: &[f64]) -> Result<(DMatrix<f64>, usize)> {
            let n = self.x.len();
            let mut j = DMatrix::zeros(n, 2);
            for (i, x) in self.x.iter().enumerate() {
                j[(i, 0)] = (b[1] * x).exp();
                j[(i, 1)] = b[0] * x * (b[1] * x).exp();
            }
            Ok((j, 0))
        }
        fn check(&mut self) -> Result<()> {
            Ok(())
        }
        fn log(&mut self, _: String) -> Result<()> {
            Ok(())
        }
    }

    fn settings(lower: Vec<f64>, upper: Vec<f64>) -> Settings {
        Settings {
            lower,
            upper,
            max_iter: 400,
            max_fev: 600,
            tol_fun: 1e-6,
            tol_x: 1e-6,
            iter_display: false,
        }
    }

    #[test]
    fn exact_exponential_data_are_recovered() {
        let x: Vec<f64> = (0..20).map(|i| i as f64 * 0.1).collect();
        let y = x.iter().map(|x| 2. * (0.7 * x).exp()).collect();
        let mut p = Exp { x, y };
        let inf = f64::INFINITY;
        let o =
            levenberg_marquardt(&mut p, &[1., 0.], &settings(vec![-inf; 2], vec![inf; 2])).unwrap();
        assert!(
            (o.x[0] - 2.).abs() < 1e-10 && (o.x[1] - 0.7).abs() < 1e-10,
            "{:?}",
            o.x
        );
        assert!(o.exitflag > 0);
    }

    #[test]
    fn an_active_upper_bound_holds() {
        let x: Vec<f64> = (0..20).map(|i| i as f64 * 0.1).collect();
        let y = x.iter().map(|x| 2. * (0.7 * x).exp()).collect();
        let mut p = Exp { x, y };
        let inf = f64::INFINITY;
        let o = levenberg_marquardt(&mut p, &[1., 0.], &settings(vec![-inf; 2], vec![inf, 0.5]))
            .unwrap();
        assert_eq!(o.x[1], 0.5);
    }

    #[test]
    fn rank_deficient_least_squares_gives_a_basic_solution() {
        let a = DMatrix::from_column_slice(3, 2, &[1., 2., 3., 2., 4., 6.]);
        let (x, rank) = lstsq(&a, &[1., 2., 3.]);
        assert_eq!(rank, 1);
        assert!(x.iter().filter(|v| **v == 0.).count() == 1);
        let cov = covariance(&a);
        assert!(cov.iter().all(|v| v.is_nan()) || cov.iter().any(|v| v.is_nan()));
    }
}
