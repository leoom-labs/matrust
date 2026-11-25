//! Exact profiled Gaussian marginal likelihood, optimized in log parameters
//! by deterministic BFGS with Armijo backtracking. Cholesky solves are used
//! throughout, including likelihood derivatives and prediction uncertainty.
use super::*;
use nalgebra::Dyn;
#[derive(Clone, Debug)]
pub(super) struct Gp {
    x: Matrix,
    kernel: String,
    basis: String,
    parameters: Vec<f64>,
    sigma: f64,
    beta: DVector<f64>,
    alpha: DVector<f64>,
    chol: Cholesky<f64, Dyn>,
    kinvh: DMatrix<f64>,
    basis_chol: Option<Cholesky<f64, Dyn>>,
    loglik: f64,
}
struct Eval {
    objective: f64,
    gradient: DVector<f64>,
    gp: Gp,
}
fn basis(q: &[f64], name: &str) -> DVector<f64> {
    match name {
        "none" => DVector::zeros(0),
        "linear" => {
            DVector::from_iterator(q.len() + 1, std::iter::once(1.).chain(q.iter().copied()))
        }
        _ => DVector::from_element(1, 1.),
    }
}
// Return covariance and derivatives with respect to log(length), log(shape),
// log(signal). The shape component is zero outside rational quadratic.
fn covariance(d2: f64, kernel: &str, l: f64, a: f64, sf: f64) -> (f64, f64, f64) {
    let u = d2 / (l * l);
    let r = u.sqrt();
    let variance = sf * sf;
    match kernel {
        "squaredexponential" => {
            let k = variance * (-u / 2.).exp();
            (k, k * u, 0.)
        }
        "exponential" => {
            let k = variance * (-r).exp();
            (k, k * r, 0.)
        }
        "matern32" => {
            let z = 3f64.sqrt() * r;
            let b = variance * (-z).exp();
            (b * (1. + z), b * z * z, 0.)
        }
        "matern52" => {
            let z = 5f64.sqrt() * r;
            let b = variance * (-z).exp();
            (b * (1. + z + z * z / 3.), b * z * z * (1. + z) / 3., 0.)
        }
        _ => {
            let z = u / (2. * a);
            let log_t = z.ln_1p();
            let k = variance * (-a * log_t).exp();
            // d log(k) / d log(a) = a * (z/(1+z) - log1p(z)).
            // The difference starts at -z^2/2: evaluating it directly loses
            // relative accuracy as RQ approaches the squared exponential.
            let shape = if z < 1e-3 {
                z * z
                    * (-0.5
                        + z * (2. / 3.
                            + z * (-3. / 4. + z * (4. / 5. + z * (-5. / 6. + z * (6. / 7.))))))
            } else {
                -log_t - (-log_t).exp_m1()
            };
            (k, k * u / (1. + z), k * a * shape)
        }
    }
}
fn evaluate(
    e: &Engine,
    x: &Matrix,
    y: &[f64],
    opts: &GpOptions,
    theta: &DVector<f64>,
    lower: f64,
) -> Result<Eval> {
    e.check_cancel()?;
    let rq = opts.kernel == "rationalquadratic";
    let l = theta[0].exp();
    let a = if rq { theta[1].exp() } else { 1. };
    let sf_index = if rq { 2 } else { 1 };
    let sf = theta[sf_index].exp();
    let sigma = lower + theta[sf_index + 1].exp();
    let n = y.len();
    let d = theta.len();
    if theta.iter().any(|x| !x.is_finite() || x.abs() > 30.) {
        return Err("fitrgp: hyperparameter outside numerical range".into());
    }
    let mut k = DMatrix::zeros(n, n);
    let mut derivatives: Vec<_> = (0..d).map(|_| DMatrix::zeros(n, n)).collect();
    for i in 0..n {
        if i.is_multiple_of(32) {
            e.check_cancel()?;
        }
        for j in 0..=i {
            let d2 = (0..x.cols())
                .map(|c| (x.data()[i + c * n] - x.data()[j + c * n]).powi(2))
                .sum();
            let (v, dl, da) = covariance(d2, &opts.kernel, l, a, sf);
            k[(i, j)] = v;
            k[(j, i)] = v;
            for (index, dv) in [(0, dl), (sf_index, 2. * v)] {
                derivatives[index][(i, j)] = dv;
                derivatives[index][(j, i)] = dv;
            }
            if rq {
                derivatives[1][(i, j)] = da;
                derivatives[1][(j, i)] = da;
            }
        }
        k[(i, i)] += sigma * sigma;
        derivatives[d - 1][(i, i)] = 2. * sigma * (sigma - lower);
    }
    let chol = k
        .cholesky()
        .ok_or("fitrgp: covariance is not positive definite")?;
    let target = DVector::from_column_slice(y);
    let bcols = basis(&row(x, 0), &opts.basis).len();
    let h = DMatrix::from_fn(n, bcols, |i, j| {
        if j == 0 {
            1.
        } else {
            x.data()[i + (j - 1) * n]
        }
    });
    let kinvh = chol.solve(&h);
    let basis_chol = if bcols > 0 {
        Some(
            (h.transpose() * &kinvh)
                .cholesky()
                .ok_or("fitrgp: basis matrix is rank deficient")?,
        )
    } else {
        None
    };
    let beta = if let Some(c) = &basis_chol {
        c.solve(&(h.transpose() * chol.solve(&target)))
    } else {
        DVector::zeros(0)
    };
    let residual = &target - &h * &beta;
    let alpha = chol.solve(&residual);
    let objective = 0.5 * residual.dot(&alpha)
        + chol.l().diagonal().iter().map(|x| x.ln()).sum::<f64>()
        + 0.5 * n as f64 * (2. * std::f64::consts::PI).ln();
    let mut gradient = DVector::zeros(d);
    for j in 0..d {
        e.check_cancel()?;
        let trace = chol.solve(&derivatives[j]).trace();
        gradient[j] = 0.5 * (trace - alpha.dot(&(&derivatives[j] * &alpha)));
    }
    if !objective.is_finite() || gradient.iter().any(|x| !x.is_finite()) {
        return Err("fitrgp: nonfinite marginal likelihood".into());
    }
    let parameters = if rq { vec![l, a, sf] } else { vec![l, sf] };
    let gp = Gp {
        x: x.clone(),
        kernel: opts.kernel.clone(),
        basis: opts.basis.clone(),
        parameters,
        sigma,
        beta,
        alpha,
        chol,
        kinvh,
        basis_chol,
        loglik: -objective,
    };
    Ok(Eval {
        objective,
        gradient,
        gp,
    })
}
pub(super) fn train(e: &Engine, x: &Matrix, y: &[f64], opts: &GpOptions) -> Result<Gp> {
    let n = x.rows();
    if n > 512 {
        return Err("fitrgp: exact training is limited to 512 observations".into());
    }
    size(e, "fitrgp", n, n.saturating_mul(12) + x.cols() + 10)?;
    let (_, scales) = means_scales(x);
    let ym = y.iter().sum::<f64>() / n as f64;
    let sd = (y.iter().map(|v| (v - ym).powi(2)).sum::<f64>() / n.saturating_sub(1).max(1) as f64)
        .sqrt();
    let lower = (0.01 * sd).max(1e-12);
    let rq = opts.kernel == "rationalquadratic";
    let default_l = scales.iter().sum::<f64>() / scales.len() as f64;
    let default_sf = (sd / 2f64.sqrt()).max(1e-6);
    let parameters = opts.parameters.clone().unwrap_or_else(|| {
        if rq {
            vec![default_l, 1., default_sf]
        } else {
            vec![default_l, default_sf]
        }
    });
    if parameters.len() != if rq { 3 } else { 2 } {
        return Err("fitrgp: wrong number of KernelParameters".into());
    }
    let initial_sigma = opts.sigma.unwrap_or(default_sf).max(lower + 1e-3);
    let mut theta = DVector::from_iterator(
        parameters.len() + 1,
        parameters
            .iter()
            .map(|x| x.ln())
            .chain(std::iter::once((initial_sigma - lower).ln())),
    );
    let mut current = evaluate(e, x, y, opts, &theta, lower)?;
    let d = theta.len();
    let mut hessian = DMatrix::identity(d, d);
    let mut evaluations = 1usize;
    // Scale the first search by the gradient norm; subsequent initial steps
    // use the previous objective decrease. This prevents a unit first step
    // from jumping into the flat zero-length-scale likelihood basin.
    let mut previous_objective = current.objective + current.gradient.norm() / 2.;
    // `hessian` holds a BFGS inverse-Hessian estimate once a curvature update
    // was accepted; it is the identity after a start or a reset.
    let mut quasi_newton = false;
    for _ in 0..500 {
        e.check_cancel()?;
        if current.gradient.amax() < 2e-8 {
            return Ok(current.gp);
        }
        let mut direction = -&hessian * &current.gradient;
        if direction.dot(&current.gradient) >= 0. {
            hessian = DMatrix::identity(d, d);
            quasi_newton = false;
            direction = -&current.gradient;
        }
        let slope = direction.dot(&current.gradient);
        let mut step = (1.01 * 2. * (current.objective - previous_objective) / slope).min(1.);
        if step <= 0. || !step.is_finite() {
            step = 1.;
        }
        // A decrease smaller than `floor` (about 450 ulps of the objective)
        // cannot be told apart from rounding in the Cholesky log-determinant
        // and quadratic form, so the line search stops there instead of
        // halving until the trial point equals the current one.
        let floor = 1e-13 * current.objective.abs().max(1.);
        let mut accepted = None;
        for _ in 0..48 {
            let next_theta = &theta + step * &direction;
            if step * -slope < floor || next_theta == theta {
                break;
            }
            evaluations += 1;
            if evaluations > 3000
                || evaluations
                    .saturating_mul(n)
                    .saturating_mul(n)
                    .saturating_mul(n)
                    .saturating_mul(d)
                    > 4_000_000_000
            {
                return Err("fitrgp: exact likelihood optimization work limit exceeded".into());
            }
            if let Ok(next) = evaluate(e, x, y, opts, &next_theta, lower)
                && next.objective <= current.objective + 1e-4 * step * slope
            {
                accepted = Some((next_theta, next));
                break;
            }
            step *= 0.5;
        }
        let Some((next_theta, next)) = accepted else {
            // No step gives a measurable decrease. Converged to working
            // precision when the gradient is small, or when the full step
            // predicts a decrease -slope = g'Hg (H is the BFGS estimate, or
            // the identity after a reset) below 2.2e-9 relative to the
            // objective: the function-decrease test of L-BFGS-B at its default
            // factr = 1e7, which scikit-learn's GP fit also stops on. With
            // Sigma at its lower bound and a long length scale the covariance
            // is ill-conditioned, the objective is only known to about 1e-10
            // relative, and the gradient cannot fall below roughly 1e-4.
            if current.gradient.amax() < 2e-6 || -slope <= 2.2e-9 * current.objective.abs().max(1.)
            {
                return Ok(current.gp);
            }
            if quasi_newton {
                hessian = DMatrix::identity(d, d);
                quasi_newton = false;
                previous_objective = current.objective;
                continue;
            }
            return Err("fitrgp: likelihood line search did not converge".into());
        };
        let s = &next_theta - &theta;
        let change = &next.gradient - &current.gradient;
        let sy = s.dot(&change);
        if sy > 1e-12 * s.norm() * change.norm() {
            let rho = 1. / sy;
            let identity = DMatrix::identity(d, d);
            let factor = &identity - rho * &s * change.transpose();
            hessian = &factor * hessian * factor.transpose() + rho * &s * s.transpose();
            quasi_newton = true;
        } else {
            hessian = DMatrix::identity(d, d);
            quasi_newton = false;
        }
        let objective_change = (next.objective - current.objective).abs();
        previous_objective = current.objective;
        theta = next_theta;
        current = next;
        if objective_change < 1e-13 && current.gradient.amax() < 2e-6 {
            return Ok(current.gp);
        }
    }
    Err("fitrgp: marginal likelihood optimization did not converge".into())
}
impl Gp {
    pub(super) fn bytes(&self) -> usize {
        let n = self.x.rows();
        (self.x.len()
            + n * n
            + self.kinvh.len()
            + self.beta.len()
            + self.alpha.len()
            + self.beta.len().pow(2))
            * 8
            + 256
    }
    pub(super) fn mean(&self, q: &[f64]) -> f64 {
        self.prediction(q, false).0
    }
    pub(super) fn predict(&self, q: &[f64]) -> (f64, f64) {
        self.prediction(q, true)
    }
    fn prediction(&self, q: &[f64], uncertainty: bool) -> (f64, f64) {
        let rq = self.kernel == "rationalquadratic";
        let l = self.parameters[0];
        let a = if rq { self.parameters[1] } else { 1. };
        let sf = *self.parameters.last().unwrap();
        let k = DVector::from_iterator(
            self.x.rows(),
            (0..self.x.rows()).map(|i| {
                let d2 = q
                    .iter()
                    .enumerate()
                    .map(|(j, q)| (q - self.x.data()[i + j * self.x.rows()]).powi(2))
                    .sum();
                covariance(d2, &self.kernel, l, a, sf).0
            }),
        );
        let h = basis(q, &self.basis);
        let mean = h.dot(&self.beta) + k.dot(&self.alpha);
        if !uncertainty {
            return (mean, 0.);
        }
        let mut variance = sf * sf + self.sigma * self.sigma - k.dot(&self.chol.solve(&k));
        if let Some(c) = &self.basis_chol {
            let residual_basis = h - self.kinvh.transpose() * &k;
            variance += residual_basis.dot(&c.solve(&residual_basis));
        }
        (mean, variance.max(0.).sqrt())
    }
    pub(super) fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        match name {
            "Beta" => value(self.beta.len(), 1, self.beta.as_slice().to_vec()),
            "Sigma" => value(1, 1, vec![self.sigma]),
            "LogLikelihood" => value(1, 1, vec![self.loglik]),
            "KernelInformation" => {
                let names = if self.kernel == "rationalquadratic" {
                    vec!["LengthScale".into(), "Alpha".into(), "SigmaF".into()]
                } else {
                    vec!["LengthScale".into(), "SigmaF".into()]
                };
                structure(
                    e,
                    vec![
                        (
                            "Name",
                            Value::Text(
                                match self.kernel.as_str() {
                                    "squaredexponential" => "SquaredExponential",
                                    "exponential" => "Exponential",
                                    "matern32" => "Matern32",
                                    "matern52" => "Matern52",
                                    _ => "RationalQuadratic",
                                }
                                .into(),
                            ),
                        ),
                        (
                            "KernelParameters",
                            value(self.parameters.len(), 1, self.parameters.clone())?,
                        ),
                        ("KernelParameterNames", strings(e, &names)?),
                    ],
                )
            }
            _ => Err(format!("RegressionGP: unknown property '{name}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitrgp_rational_quadratic_gradient_matches_differences() {
        // The 60 observations of docs/tasks/2026-11-12-fitrgp-rq.m.
        let mut state = 1121u32;
        let mut uniform = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (f64::from(state) / 4294967296. - 0.5) * 4.
        };
        let mut data = vec![0.; 120];
        let mut y = Vec::new();
        for i in 0..60 {
            let a = uniform();
            let b = uniform();
            let noise = uniform() * 0.0125;
            data[i] = a;
            data[i + 60] = b;
            y.push(3. * a - 2. * b + 0.5 * a * b + noise);
        }
        let x = Matrix::from_column_vec(60, 2, data).unwrap();
        let mean = y.iter().sum::<f64>() / 60.;
        let lower = 0.01 * (y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 59.).sqrt();
        let e = Engine::default();
        let opts = GpOptions {
            kernel: "rationalquadratic".into(),
            basis: "constant".into(),
            sigma: None,
            parameters: None,
        };
        // Independent mpmath values resolve the small alpha derivative too:
        // central differences cannot resolve its relative error as alpha -> infinity.
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ml_regression.json"
        )))
        .unwrap();
        let reference = fixtures
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "rq_kernel_high_precision")
            .unwrap();
        let values = &reference["values"];
        let tolerance = reference["tolerance"].as_f64().unwrap();
        for i in 0..values["d2"].as_array().unwrap().len() {
            let input = |key: &str| values[key][i].as_f64().unwrap();
            let (k, dl, da) = covariance(
                input("d2"),
                "rationalquadratic",
                input("length"),
                input("shape"),
                input("signal"),
            );
            for (name, actual) in [("k", k), ("dl", dl), ("da", da)] {
                let expected = input(name);
                assert!(
                    (actual - expected).abs() <= tolerance * expected.abs(),
                    "mpmath case {i} {name}: {actual:e} vs {expected:e}"
                );
            }
        }
        // Unit length and signal keep the covariance well conditioned enough
        // to resolve central differences at h=1e-5, including large alpha.
        for alpha in [1_f64, 1e3, 1e6, 1e8] {
            let theta = DVector::from_vec(vec![0., alpha.ln(), 0., (0.1 - lower).ln()]);
            let analytic = evaluate(&e, &x, &y, &opts, &theta, lower).unwrap().gradient;
            let mut differences = DVector::zeros(4);
            for j in 0..4 {
                let mut plus = theta.clone();
                let mut minus = theta.clone();
                plus[j] += 1e-5;
                minus[j] -= 1e-5;
                differences[j] = (evaluate(&e, &x, &y, &opts, &plus, lower).unwrap().objective
                    - evaluate(&e, &x, &y, &opts, &minus, lower)
                        .unwrap()
                        .objective)
                    / 2e-5;
            }
            let relative =
                (&analytic - &differences).amax() / analytic.amax().max(differences.amax()).max(1.);
            eprintln!(
                "alpha={alpha:e} gradient relative error={relative:e}; analytic={analytic:?}; differences={differences:?}"
            );
            assert!(
                relative < 1e-6,
                "alpha={alpha:e}: relative gradient error {relative:e}"
            );
            for j in 0..4 {
                let scale = analytic[j].abs().max(differences[j].abs()).max(1.);
                assert!(
                    (analytic[j] - differences[j]).abs() < 1e-6 * scale,
                    "alpha={alpha:e}, component {j}: analytic {} vs differences {}",
                    analytic[j],
                    differences[j]
                );
            }
        }
    }
}
