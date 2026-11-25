//! Native dense SQP for conditional likelihoods, shared by mean and volatility
//! estimation. The QP kernel is the same one used by fmincon; its parsed-objective
//! driver is deliberately unchanged. Powell-damped BFGS models the Lagrangian.
//! Feasibility is explicit, independent of whether a likelihood trial is finite.
use super::*;
use crate::engine::econ_vol::Kind;
use nalgebra::{DMatrix, DVector};

pub(in crate::engine) struct Settings {
    pub iterations: usize,
    pub evaluations: usize,
    pub gradient_tol: f64,
    pub step_tol: f64,
    pub function_tol: f64,
    pub constraint_tol: f64,
    pub difference_step: f64,
    pub central: bool,
    pub iter_display: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            iterations: 1000,
            evaluations: 20000,
            gradient_tol: 1e-9,
            step_tol: 1e-10,
            function_tol: 0.,
            constraint_tol: 1e-7,
            difference_step: 3e-6,
            central: true,
            iter_display: false,
        }
    }
}
pub(in crate::engine) struct Optimum {
    pub x: Vec<f64>,
    pub flag: f64,
    pub iterations: usize,
    pub evaluations: usize,
}

struct Affine {
    a: DVector<f64>,
    constant: f64,
}
impl Affine {
    fn value(&self, x: &DVector<f64>) -> f64 {
        self.constant + self.a.dot(x)
    }
}
struct Polynomial {
    coefficients: Vec<Affine>,
}
/// All rows have the convention c(x) <= 0, including bounds. Fixed parameters
/// enter the constant part of each row and are never perturbed by the solver.
pub(in crate::engine) struct Constraints {
    lower: Vec<f64>,
    upper: Vec<f64>,
    linear: Vec<Affine>,
    polynomials: Vec<Polynomial>,
    margin: f64,
}

/// Construct constraints in scaled physical coordinates. Names preserve the
/// template's structural zero mask, including seasonal and sparse lag indices.
pub(in crate::engine) fn constraints(
    initial: &[f64],
    names: &[String],
    free: &[usize],
    scale: &[f64],
    kind: Option<Kind>,
    tolerance: f64,
) -> Constraints {
    let n = free.len();
    let mut c = Constraints {
        lower: vec![f64::NEG_INFINITY; n],
        upper: vec![f64::INFINITY; n],
        linear: vec![],
        polynomials: vec![],
        // Model validators also reserve 1e-12 at the open unit-circle boundary.
        margin: tolerance.max(2e-12),
    };
    let affine = |terms: &[(usize, f64)], constant: f64| {
        let mut row = Affine {
            a: DVector::zeros(n),
            constant,
        };
        for &(i, weight) in terms {
            if let Some(j) = free.iter().position(|&k| k == i) {
                row.a[j] += weight * scale[j];
            } else {
                row.constant += weight * initial[i];
            }
        }
        row
    };
    let prefix = if names.iter().any(|s| s.starts_with("Variance.")) {
        "Variance."
    } else {
        ""
    };
    for (i, name) in names.iter().enumerate() {
        let variance = name == "Variance"
            || (kind.is_some_and(|k| k != Kind::Egarch) && name == &format!("{prefix}Constant"));
        let nonnegative = kind.is_some_and(|k| k != Kind::Egarch)
            && (name.starts_with(&format!("{prefix}GARCH{{"))
                || name.starts_with(&format!("{prefix}ARCH{{")));
        let lower = if name == "DoF" {
            Some(2. + c.margin)
        } else if variance {
            // Strict positivity in the parameter's own units; do not floor a
            // small financial variance at the dimensionless constraint tolerance.
            Some(
                free.iter()
                    .position(|&k| k == i)
                    .map_or(initial[i], |j| scale[j])
                    * 1e-12,
            )
        } else if nonnegative {
            Some(0.)
        } else {
            None
        };
        if let Some(lower) = lower {
            if let Some(j) = free.iter().position(|&k| k == i) {
                c.lower[j] = lower / scale[j];
            } else {
                c.linear.push(affine(&[(i, -1.)], lower));
            }
        }
    }
    for (group, sign) in [("AR", -1.), ("SAR", -1.), ("MA", 1.), ("SMA", 1.)]
        .into_iter()
        .chain((kind == Some(Kind::Egarch)).then_some(("GARCH", -1.)))
    {
        let group = if group == "GARCH" {
            format!("{prefix}{group}{{")
        } else {
            format!("{group}{{")
        };
        let lags: Vec<_> = names
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.strip_prefix(&group)
                    .and_then(|s| s.strip_suffix('}'))
                    .and_then(|s| s.parse::<usize>().ok())
                    .map(|lag| (i, lag))
            })
            .collect();
        if let Some(degree) = lags.iter().map(|&(_, lag)| lag).max() {
            let coefficients = (1..=degree)
                .map(|lag| {
                    let terms: Vec<_> = lags
                        .iter()
                        .filter(|&&(_, k)| k == lag)
                        .map(|&(i, _)| (i, sign))
                        .collect();
                    affine(&terms, 0.)
                })
                .collect();
            c.polynomials.push(Polynomial { coefficients });
        }
    }
    if kind.is_some_and(|k| k != Kind::Egarch) {
        let terms: Vec<_> = names
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let name = name.strip_prefix(prefix)?;
                if name.starts_with("GARCH{") || name.starts_with("ARCH{") {
                    Some((i, 1.))
                } else if name.starts_with("Leverage{") {
                    Some((i, 0.5))
                } else {
                    None
                }
            })
            .collect();
        c.linear.push(affine(&terms, -(1. - c.margin)));
        if kind == Some(Kind::Gjr) {
            let degree = names
                .iter()
                .filter_map(|s| {
                    s.strip_prefix(&format!("{prefix}Leverage{{"))?
                        .strip_suffix('}')?
                        .parse::<usize>()
                        .ok()
                })
                .max()
                .unwrap_or(0);
            for lag in 1..=degree {
                let terms: Vec<_> = names
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| {
                        **name == format!("{prefix}ARCH{{{lag}}}")
                            || **name == format!("{prefix}Leverage{{{lag}}}")
                    })
                    .map(|(i, _)| (i, -1.))
                    .collect();
                // Leverage >= -ARCH; the persistence row supplies its upper
                // bound. Public EGARCH constraints impose no leverage bounds.
                c.linear.push(affine(&terms, 0.));
            }
        }
    }
    c
}
impl Constraints {
    fn project(&self, x: &mut DVector<f64>) {
        for (i, v) in x.iter_mut().enumerate() {
            *v = v.clamp(self.lower[i], self.upper[i]);
        }
    }
    fn nonlinear(&self, x: &DVector<f64>) -> Vec<f64> {
        let mut values = vec![];
        for polynomial in &self.polynomials {
            let mut a: Vec<_> = std::iter::once(1.)
                // Stability at radius R is equivalent to applying Schur-Cohn
                // to coefficients a_j/R^j. This controls companion roots,
                // including sparse/seasonal lags, without an eigenproblem.
                .chain(
                    polynomial
                        .coefficients
                        .iter()
                        .enumerate()
                        .map(|(j, a)| a.value(x) / (1. - self.margin).powi((j + 1) as i32)),
                )
                .collect();
            // Schur-Cohn step-down on the scaled polynomial: separate signed
            // rows avoid abs at zero. The tiny guard retains an open boundary
            // for the public model validator; the user margin is in root units.
            for k in (1..a.len()).rev() {
                let r = a[k];
                values.extend([r - (1. - 2e-12), -r - (1. - 2e-12)]);
                let den = (1. - r * r).max(1e-12);
                a = std::iter::once(1.)
                    .chain((1..k).map(|i| (a[i] - r * a[k - i]) / den))
                    .collect();
            }
        }
        values
    }
    fn values(&self, x: &DVector<f64>) -> DVector<f64> {
        let mut v: Vec<_> = self.linear.iter().map(|a| a.value(x)).collect();
        for i in 0..x.len() {
            if self.lower[i].is_finite() {
                v.push(self.lower[i] - x[i]);
            }
            if self.upper[i].is_finite() {
                v.push(x[i] - self.upper[i]);
            }
        }
        v.extend(self.nonlinear(x));
        DVector::from_vec(v)
    }
    fn jacobian(&self, x: &DVector<f64>) -> DMatrix<f64> {
        let nonlinear = self.nonlinear(x);
        let mut rows = self.linear.len();
        let bounds = self.lower.iter().filter(|x| x.is_finite()).count()
            + self.upper.iter().filter(|x| x.is_finite()).count();
        let mut j = DMatrix::zeros(rows + bounds + nonlinear.len(), x.len());
        for (i, a) in self.linear.iter().enumerate() {
            j.row_mut(i).copy_from(&a.a.transpose());
        }
        for i in 0..x.len() {
            if self.lower[i].is_finite() {
                j[(rows, i)] = -1.;
                rows += 1;
            }
            if self.upper[i].is_finite() {
                j[(rows, i)] = 1.;
                rows += 1;
            }
        }
        if !nonlinear.is_empty() {
            let mut probe = x.clone();
            for k in 0..x.len() {
                let h = 1e-6 * x[k].abs().max(1.);
                probe[k] = x[k] + h;
                let hi = self.nonlinear(&probe);
                probe[k] = x[k] - h;
                let lo = self.nonlinear(&probe);
                probe[k] = x[k];
                for i in 0..nonlinear.len() {
                    j[(rows + i, k)] = (hi[i] - lo[i]) / (2. * h);
                }
            }
        }
        j
    }
}
fn violation(c: &DVector<f64>) -> f64 {
    c.iter().copied().fold(0., |v, c| {
        if c.is_finite() {
            v.max(c)
        } else {
            f64::INFINITY
        }
    })
}
fn l1(c: &DVector<f64>) -> f64 {
    c.iter()
        .map(|x| {
            if x.is_finite() {
                x.max(0.)
            } else {
                f64::INFINITY
            }
        })
        .sum()
}
fn gradient<F>(
    e: &Engine,
    f: &mut F,
    x: &DVector<f64>,
    fx: f64,
    calls: &mut usize,
    o: &Settings,
) -> Result<Option<DVector<f64>>>
where
    F: FnMut(&Engine, &[f64]) -> Result<f64>,
{
    let mut probe = x.clone();
    let mut g = DVector::zeros(x.len());
    for k in 0..x.len() {
        let mut h = o.difference_step * x[k].abs().max(1.);
        let mut found = false;
        for _ in 0..12 {
            e.check_cancel()?;
            if *calls + 2 > o.evaluations {
                return Ok(None);
            }
            probe[k] = x[k] + h;
            let hi = f(e, probe.as_slice())?;
            *calls += 1;
            if !o.central && hi.is_finite() {
                g[k] = (hi - fx) / h;
                found = true;
                break;
            }
            probe[k] = x[k] - h;
            let lo = f(e, probe.as_slice())?;
            *calls += 1;
            if hi.is_finite() && lo.is_finite() {
                g[k] = (hi - lo) / (2. * h);
                found = true;
                break;
            }
            // Only undefined LIKELIHOODS require one-sided/shrinking probes;
            // stationarity and coefficient bounds do not gate derivatives.
            if hi.is_finite() || lo.is_finite() {
                g[k] = if hi.is_finite() {
                    (hi - fx) / h
                } else {
                    (fx - lo) / h
                };
                found = true;
                break;
            }
            h *= 0.1;
        }
        probe[k] = x[k];
        if !found || !g[k].is_finite() {
            return Err(err(
                "estimate",
                "InvalidGradient",
                "Cannot compute a finite likelihood gradient.",
            ));
        }
    }
    Ok(Some(g))
}

pub(in crate::engine) fn minimize<F>(
    e: &mut Engine,
    mut f: F,
    x0: Vec<f64>,
    constraints: &Constraints,
    o: &Settings,
    out: &mut Execution,
) -> Result<Optimum>
where
    F: FnMut(&Engine, &[f64]) -> Result<f64>,
{
    let n = x0.len();
    let mut x = DVector::from_vec(x0);
    constraints.project(&mut x);
    let mut calls = 1;
    let mut fx = f(e, x.as_slice())?;
    if !fx.is_finite() {
        return Err(err(
            "estimate",
            "InvalidInitialPoint",
            "Initial parameters must define a finite likelihood.",
        ));
    }
    let feasible_tol = o.constraint_tol.min(1e-10);
    let mut values = constraints.values(&x);
    e.preflight_elements(
        n.saturating_mul(n)
            .saturating_mul(8)
            .saturating_add(values.len().saturating_mul(n).saturating_mul(6)),
    )?;
    let mut best = (violation(&values) <= feasible_tol).then(|| (x.clone(), fx));
    let mut flag = 0.;
    let mut iterations = 0;
    if n > 0 && o.iterations > 0 {
        let mut b = DMatrix::identity(n, n);
        let mut penalty: f64 = 10.;
        if let Some(mut g) = gradient(e, &mut f, &x, fx, &mut calls, o)? {
            for it in 0..o.iterations {
                e.check_cancel()?;
                let j = constraints.jacobian(&x);
                let (direction, dual, solved) =
                    super::super::optim_con::ocn_likelihood_qp(e, &b, &g, &j, &values)?;
                if !solved {
                    break;
                }
                let lag = &g + j.transpose() * &dual;
                let optimality = dual
                    .iter()
                    .zip(values.iter())
                    .fold(lag.amax(), |v, (&z, &c)| v.max(-z).max((z * c).abs()));
                if violation(&values) <= feasible_tol && optimality <= o.gradient_tol {
                    flag = 1.;
                    break;
                }
                if direction.amax() <= o.step_tol * (1. + x.amax())
                    && violation(&values) <= feasible_tol
                {
                    flag = 2.;
                    break;
                }
                penalty = penalty.max(1. + 1.1 * dual.amax());
                let merit = fx + penalty * l1(&values);
                let predicted = g.dot(&direction) - penalty * l1(&values);
                let mut alpha = 1.;
                let mut accepted = None;
                for _ in 0..50 {
                    e.check_cancel()?;
                    if calls >= o.evaluations {
                        break;
                    }
                    let mut trial = &x + alpha * &direction;
                    constraints.project(&mut trial);
                    let ft = f(e, trial.as_slice())?;
                    calls += 1;
                    let cv = constraints.values(&trial);
                    if ft.is_finite()
                        && ft + penalty * l1(&cv) <= merit + 1e-4 * alpha * predicted.min(-1e-20)
                    {
                        accepted = Some((trial, ft, cv));
                        break;
                    }
                    // Correct curvature of nonlinear boundaries at a full SQP
                    // step (Nocedal & Wright, sec. 18.3, Maratos correction).
                    if alpha == 1.
                        && ft.is_finite()
                        && cv.iter().all(|v| v.is_finite())
                        && calls < o.evaluations
                        && !constraints.polynomials.is_empty()
                    {
                        let tj = constraints.jacobian(&trial);
                        let (correction, _, ok) = super::super::optim_con::ocn_likelihood_qp(
                            e,
                            &DMatrix::identity(n, n),
                            &DVector::zeros(n),
                            &tj,
                            &cv,
                        )?;
                        if ok && correction.amax() <= 0.5 * direction.amax().max(1e-8) {
                            trial += correction;
                            constraints.project(&mut trial);
                            let fc = f(e, trial.as_slice())?;
                            calls += 1;
                            let cv = constraints.values(&trial);
                            if fc.is_finite()
                                && fc + penalty * l1(&cv) <= merit + 1e-4 * predicted.min(-1e-20)
                            {
                                accepted = Some((trial, fc, cv));
                                break;
                            }
                        }
                    }
                    alpha *= 0.5;
                }
                let Some((next, fnx, cv)) = accepted else {
                    flag = if calls >= o.evaluations { 0. } else { 2. };
                    break;
                };
                iterations = it + 1;
                if violation(&cv) <= feasible_tol && best.as_ref().is_none_or(|(_, f)| fnx < *f) {
                    best = Some((next.clone(), fnx));
                }
                let Some(gn) = gradient(e, &mut f, &next, fnx, &mut calls, o)? else {
                    break;
                };
                let s = &next - &x;
                let next_j = constraints.jacobian(&next);
                let y = &gn + next_j.transpose() * &dual - lag;
                // Powell damping preserves a positive-definite QP Hessian.
                let bs = &b * &s;
                let sbs = s.dot(&bs);
                let sy = s.dot(&y);
                if sbs > 1e-25 && sbs.is_finite() {
                    let theta = if sy >= 0.2 * sbs {
                        1.
                    } else {
                        0.8 * sbs / (sbs - sy)
                    };
                    let r = theta * y + (1. - theta) * &bs;
                    let sr = s.dot(&r);
                    if sr > 0. {
                        b += (&r * r.transpose()) / sr - (&bs * bs.transpose()) / sbs;
                        b = 0.5 * (&b + b.transpose());
                    }
                }
                let improvement = (fx - fnx).abs();
                x = next;
                fx = fnx;
                values = cv;
                g = gn;
                if o.iter_display {
                    e.emit(out, format!("SQP iteration {iterations}: evaluations {calls}, negative log likelihood {fx:.12}, constraint violation {:.3e}", violation(&values)))?;
                }
                if violation(&values) <= feasible_tol
                    && (s.amax() <= o.step_tol * (1. + x.amax())
                        || (o.function_tol > 0.
                            && improvement <= o.function_tol * fx.abs().max(1.)))
                {
                    flag = 2.;
                    break;
                }
            }
        }
    } else if n == 0 && best.is_some() {
        flag = 1.;
    }
    let Some((x, _)) = best else {
        return Err(err(
            "estimate",
            "InfeasibleConstraints",
            "No likelihood point satisfies ConstraintTolerance and the fixed model parameters.",
        ));
    };
    Ok(Optimum {
        x: x.as_slice().to_vec(),
        flag,
        iterations,
        evaluations: calls,
    })
}
