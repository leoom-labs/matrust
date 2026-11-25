//! Read-only, bounded diagnostics of the nominal unconstrained MPC realization.
//! The exact move objective supplies H and f; no parallel solver or cost model.
use super::super::{Engine, Execution, Value, objects};
use super::{
    estimator::{self, EstimatorMode},
    r#move::review_objective,
    object::{self, ControlHorizon, MpcObject},
    prediction::{self, dm, matrix},
};
use crate::{Array, CharacterRow, Matrix, Result};
use nalgebra::{DMatrix, DVector, linalg::Schur};
use std::{fmt::Write, ops::AddAssign};

const STABILITY_TOL: f64 = 1e-9;
const OFFSET_TOL: f64 = 1e-6;
const CONDITION_LIMIT: f64 = 1e12;
const MAX_TEXT: usize = 32_768;

struct Check {
    assessed: bool,
    pass: bool,
    reason: String,
    metric: f64,
    matrix: DMatrix<f64>,
}
impl Check {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            assessed: false,
            pass: false,
            reason: reason.into(),
            metric: f64::NAN,
            matrix: DMatrix::zeros(0, 0),
        }
    }
    fn value(
        e: &Engine,
        assessed: bool,
        pass: bool,
        reason: &str,
        fields: Vec<(&str, Array)>,
    ) -> Result<Value> {
        let mut base = vec![
            ("Assessed", number(f64::from(assessed))),
            ("Pass", number(f64::from(pass))),
            ("Reason", text(e, reason)?),
        ];
        base.extend(fields);
        super::structure(e, base)
    }
    fn export(self, e: &Engine, metric: &str) -> Result<Array> {
        Self::value(
            e,
            self.assessed,
            self.pass,
            &self.reason,
            vec![
                (metric, number(self.metric)),
                ("Matrix", Array::Numeric(matrix(self.matrix)?)),
            ],
        )?
        .array(&e.cancel)
    }
    fn line(&self, name: &str, metric: &str) -> String {
        format!(
            "{name}: {}; {metric}={:.12e}; {}\n",
            if !self.assessed {
                "not assessed"
            } else if self.pass {
                "pass"
            } else {
                "FAIL"
            },
            self.metric,
            self.reason
        )
    }
}
fn number(x: f64) -> Array {
    Array::Numeric(Matrix::scalar(x))
}
fn text(e: &Engine, s: &str) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(s, &e.cancel)?))
}
fn spectral(e: &Engine, a: DMatrix<f64>, reason: &str) -> Result<Check> {
    e.check_cancel()?;
    if a.iter().any(|v| !v.is_finite()) {
        return Ok(Check::unavailable("realization contains nonfinite entries"));
    }
    let radius = if a.nrows() == 0 {
        0.
    } else {
        let Some(s) = Schur::try_new(a.clone(), f64::EPSILON, 20_000) else {
            return Ok(Check::unavailable(
                "bounded Schur iteration did not converge",
            ));
        };
        s.complex_eigenvalues()
            .iter()
            .map(|z| z.norm())
            .fold(0., f64::max)
    };
    e.check_cancel()?;
    Ok(Check {
        assessed: true,
        pass: radius < 1. - STABILITY_TOL,
        reason: format!("{reason}; strict Schur stability requires max |eig| < 1-1e-9"),
        metric: radius,
        matrix: a,
    })
}

fn at(m: &Matrix, k: usize) -> f64 {
    m.data()[k.min(m.len() - 1)]
}
fn preview(m: &Matrix) -> String {
    let mut s = String::new();
    write!(s, "{}x{} [", m.rows(), m.cols()).unwrap();
    for (i, v) in m.data().iter().take(8).enumerate() {
        if i != 0 {
            s.push(' ');
        }
        write!(s, "{v:.6e}").unwrap();
    }
    if m.len() > 8 {
        s.push_str(" ... column-major");
    }
    s.push(']');
    s
}

// Hard intervals are propagated from nominal LastMove through every horizon
// step, including held increments. Only algebraic contradictions are claimed.
fn constraints(e: &Engine, m: &MpcObject) -> Result<(Value, String)> {
    let p = m.prediction_horizon;
    if p.saturating_mul(m.n_mv() + m.ny()) > 1_000_000 {
        let reason =
            "hard-constraint scan not assessed: horizon times MV/output channels exceeds 1000000";
        let value = Check::value(
            e,
            false,
            false,
            reason,
            vec![
                ("IssueCount", number(f64::NAN)),
                ("FiniteBoundEntries", number(f64::NAN)),
                ("Details", text(e, reason)?),
            ],
        )?;
        return Ok((value, format!("Hard constraints: not assessed; {reason}\n")));
    }
    let starts: Vec<usize> = match &m.control_horizon {
        ControlHorizon::Scalar(n) => (0..*n).collect(),
        ControlHorizon::Blocking(b) => {
            let mut sum = 0;
            b.iter()
                .map(|&n| {
                    let k = sum;
                    sum += n;
                    k
                })
                .collect()
        }
    };
    let mut details = String::new();
    let mut issue_details = String::new();
    let mut issues = 0;
    let mut finite = 0;
    for (j, mv) in m.mv.iter().enumerate() {
        for (name, v) in [
            ("Min", &mv.min),
            ("Max", &mv.max),
            ("RateMin", &mv.rate_min),
            ("RateMax", &mv.rate_max),
        ] {
            finite += v.data().iter().filter(|v| v.is_finite()).count();
            if v.data().iter().any(|v| v.is_finite()) && details.len() < 10_000 {
                writeln!(details, "MV({}) {name}: {}", j + 1, preview(v)).unwrap();
            }
        }
        let nominal = m.nominal.u.get(m.mv_idx[j], 0);
        let (mut low, mut high) = (nominal, nominal);
        for k in 0..p {
            e.check_cancel()?;
            let free = starts.binary_search(&k).is_ok();
            let mut dl = if at(&mv.rate_min_ecr, k) == 0. {
                at(&mv.rate_min, k)
            } else {
                f64::NEG_INFINITY
            };
            let mut dh = if at(&mv.rate_max_ecr, k) == 0. {
                at(&mv.rate_max, k)
            } else {
                f64::INFINITY
            };
            if !free {
                dl = dl.max(0.);
                dh = dh.min(0.);
            }
            low += dl;
            high += dh;
            if at(&mv.min_ecr, k) == 0. {
                low = low.max(at(&mv.min, k));
            }
            if at(&mv.max_ecr, k) == 0. {
                high = high.min(at(&mv.max, k));
            }
            if dl > dh || low > high {
                issues += 1;
                if issue_details.len() < 4_000 {
                    writeln!(issue_details, "MV({}) step {}: contradictory hard bound/rate or held move; reachable interval [{low:.6e}, {high:.6e}] from nominal LastMove={nominal:.6e}", j+1, k+1).unwrap();
                }
                break;
            }
        }
    }
    for (j, ov) in m.ov.iter().enumerate() {
        for (name, v) in [("Min", &ov.min), ("Max", &ov.max)] {
            finite += v.data().iter().filter(|v| v.is_finite()).count();
            if v.data().iter().any(|v| v.is_finite()) && details.len() < 10_000 {
                writeln!(details, "OV({}) {name}: {}", j + 1, preview(v)).unwrap();
            }
        }
        for k in 0..p {
            e.check_cancel()?;
            if at(&ov.min_ecr, k) == 0.
                && at(&ov.max_ecr, k) == 0.
                && at(&ov.min, k) > at(&ov.max, k)
            {
                issues += 1;
                if issue_details.len() < 4_000 {
                    writeln!(
                        issue_details,
                        "OV({}) step {}: contradictory hard Min/Max",
                        j + 1,
                        k + 1
                    )
                    .unwrap();
                }
                break;
            }
        }
    }
    if details.len() >= 10_000 {
        details.push_str("Further finite-bound previews omitted.\n");
    }
    if issue_details.len() >= 4_000 {
        issue_details
            .push_str("Further issue details omitted; IssueCount includes all scanned channels.\n");
    }
    details = issue_details + &details;
    let reason = "Hard interval contradictions only; MV reachability starts at nominal LastMove. No guarantee of closed-loop feasibility; output/plant reachability and mixed constraints are not assessed.";
    let value = Check::value(
        e,
        true,
        issues == 0,
        reason,
        vec![
            ("IssueCount", number(issues as f64)),
            ("FiniteBoundEntries", number(finite as f64)),
            ("Details", text(e, &details)?),
        ],
    )?;
    Ok((
        value,
        format!(
            "Hard constraints: {}; issues={issues}; finite bound entries={finite}\n{details}{reason}\n",
            if issues == 0 { "pass" } else { "FAIL" }
        ),
    ))
}

struct Analysis {
    internal: Check,
    nominal: Check,
    offset: Check,
    hessian: Check,
    regularization: f64,
    baseline_error: DVector<f64>,
}
impl Analysis {
    fn unavailable(reason: &str) -> Self {
        Self {
            internal: Check::unavailable(reason),
            nominal: Check::unavailable(reason),
            offset: Check::unavailable(reason),
            hessian: Check::unavailable(reason),
            regularization: 0.,
            baseline_error: DVector::zeros(0),
        }
    }
}

fn analyze(e: &mut Engine, m: &MpcObject, out: &mut Execution) -> Result<Analysis> {
    let blocks = match &m.control_horizon {
        ControlHorizon::Scalar(n) => *n,
        ControlHorizon::Blocking(b) => b.len(),
    };
    // Bound work, scratch, and all returned dense matrices before allocation.
    if m.nx() > 32
        || m.nu() > 16
        || m.ny() > 16
        || m.prediction_horizon > 128
        || blocks * m.n_mv() + 1 > 64
    {
        return Ok(Analysis::unavailable(
            "analysis limit: 32 plant/augmented states, 16 inputs/outputs, horizon 128, 64 QP variables",
        ));
    }
    if e.limits.memory_bytes < 8 * (4_000 * 64 * 3 + 64 * 64 * 40 + 128 * 128 * 60) {
        return Ok(Analysis::unavailable(
            "review workspace exceeds memory limit",
        ));
    }
    let model = prediction::build(e, m, out)?;
    let n = model.a.rows();
    if n > 32 {
        return Ok(Analysis::unavailable(
            "analysis limit: augmented states exceed 32",
        ));
    }
    let nu = m.n_mv();
    let ny = m.ny();
    let r0 = Matrix::from_column_vec(1, ny, model.y_offset.data().to_vec())?;
    let zero = DVector::zeros(n);
    let last = DVector::zeros(nu);
    let (h, f0, regularization) = review_objective(e, m, &model, &zero, &last, &r0)?;
    e.check_cancel()?;
    if h.iter().any(|v| !v.is_finite()) || f0.iter().any(|v| !v.is_finite()) {
        return Ok(Analysis::unavailable("nonfinite condensed objective"));
    }
    let Some(eig) = nalgebra::linalg::SymmetricEigen::try_new(h.clone(), f64::EPSILON, 20_000)
    else {
        return Ok(Analysis::unavailable(
            "bounded Hessian eigensolve did not converge",
        ));
    };
    let min = eig.eigenvalues.min();
    let max = eig.eigenvalues.max();
    let condition = if min > 0. { max / min } else { f64::INFINITY };
    let mut result =
        Analysis::unavailable("unconstrained realization requires a positive definite Hessian");
    result.regularization = regularization;
    result.hessian = Check {
        assessed: true,
        pass: min > 0. && condition <= CONDITION_LIMIT && regularization == 0.,
        reason: format!(
            "exact solver H, including ECR slack; min eigenvalue={min:.12e}; max eigenvalue={max:.12e}; added move diagonal={regularization:.12e}; healthy requires SPD, cond2 <= 1e12 and no regularization"
        ),
        metric: condition,
        matrix: h.clone(),
    };
    let Some(chol) = h.cholesky() else {
        return Ok(result);
    };
    let unsupported = if matches!(m.estimator, EstimatorMode::Custom) {
        Some("custom estimator: no autonomous measurement feedback realization")
    } else if matches!(m.control_horizon, ControlHorizon::Blocking(_)) {
        Some("blocking-vector realization is not assessed; Hessian uses actual blocking")
    } else if m.weights.ov.rows() > 1 || m.weights.mv.rows() > 1 || m.weights.mv_rate.rows() > 1 {
        Some("time-varying weight rows: stability and steady-state analysis not assessed")
    } else {
        None
    };
    if let Some(reason) = unsupported {
        result.internal = Check::unavailable(reason);
        result.nominal = Check::unavailable(reason);
        result.offset = Check::unavailable(reason);
        return Ok(result);
    }
    // Measure the affine first-move law on independent private deviation vectors.
    // H is fixed; every f comes from the exact move assembly, not a second cost.
    let u0 = -chol.solve(&f0).rows(0, nu).into_owned();
    let mut ux = DMatrix::zeros(nu, n);
    for j in 0..n {
        e.check_cancel()?;
        let mut x = zero.clone();
        x[j] = 1.;
        let (_, f, _) = review_objective(e, m, &model, &x, &last, &r0)?;
        ux.set_column(j, &(-chol.solve(&(f - &f0)).rows(0, nu)));
    }
    let mut ul = DMatrix::identity(nu, nu);
    for j in 0..nu {
        e.check_cancel()?;
        let mut u = last.clone();
        u[j] = 1.;
        let (_, f, _) = review_objective(e, m, &model, &zero, &u, &r0)?;
        let column = ul.column(j).into_owned() - chol.solve(&(f - &f0)).rows(0, nu);
        ul.set_column(j, &column);
    }
    let mut ur = DMatrix::zeros(nu, ny);
    for j in 0..ny {
        e.check_cancel()?;
        let mut r = model.y_offset.data().to_vec();
        r[j] += 1.;
        let r = Matrix::from_column_vec(1, ny, r)?;
        let (_, f, _) = review_objective(e, m, &model, &zero, &last, &r)?;
        ur.set_column(j, &(-chol.solve(&(f - &f0)).rows(0, nu)));
    }
    let gains = estimator::design(e, &model, &m.estimator, out)?;
    let a = dm(&model.a);
    let b = dm(&model.bu);
    let cm = dm(&model.cm);
    let mg = dm(&gains.m);
    let lg = dm(&gains.l);
    let nc = n + nu;
    let mut uc = DMatrix::zeros(nu, nc);
    uc.columns_mut(0, n)
        .copy_from(&(&ux * (DMatrix::identity(n, n) - &mg * &cm)));
    uc.columns_mut(n, nu).copy_from(&ul);
    let dy = &ux * &mg;
    let mut ac = DMatrix::zeros(nc, nc);
    ac.view_mut((0, 0), (n, n)).copy_from(&(&a - &lg * &cm));
    ac.rows_mut(0, n).add_assign(&(&b * &uc));
    ac.rows_mut(n, nu).copy_from(&uc);
    let mut by = DMatrix::zeros(nc, cm.nrows());
    by.rows_mut(0, n).copy_from(&(&lg + &b * &dy));
    by.rows_mut(n, nu).copy_from(&dy);
    let mut br = DMatrix::zeros(nc, ny);
    br.rows_mut(0, n).copy_from(&(&b * &ur));
    br.rows_mut(n, nu).copy_from(&ur);
    let constant_y = dm(&model.dvm).column(model.md_idx.len()).into_owned();
    let affine_u = u0 - &dy * &constant_y;
    let affine_x =
        dm(&model.bv).column(model.md_idx.len()).into_owned() - &lg * constant_y + &b * &affine_u;
    let mut bc = DVector::zeros(nc);
    bc.rows_mut(0, n).copy_from(&affine_x);
    bc.rows_mut(n, nu).copy_from(&affine_u);
    result.internal = spectral(
        e,
        ac.clone(),
        "Ac on [augmented observer prior; previous scaled MV], ym and reference held at nominal",
    )?;
    if result.internal.assessed {
        result.internal.pass = result.internal.metric <= 1. + STABILITY_TOL;
        result.internal.reason = "Ac controller pole test: max |eig| <= 1+1e-9; unit poles allowed, not an asymptotic stability claim".into();
    }
    // True physical plant, with MD/UD fixed at nominal and no stochastic inputs.
    let nx = m.nx();
    let bp = b.rows(0, nx).into_owned();
    let cp = DMatrix::from_fn(m.mo_idx.len(), nx, |i, j| {
        m.cd.get(m.mo_idx[i], j) / m.ov[m.mo_idx[i]].scale_factor
    });
    let total = nx + nc;
    let mut acl = DMatrix::zeros(total, total);
    acl.view_mut((0, 0), (nx, nx))
        .copy_from(&(dm(&m.ad) + &bp * &dy * &cp));
    acl.view_mut((0, nx), (nx, nc)).copy_from(&(&bp * &uc));
    acl.view_mut((nx, 0), (nc, nx)).copy_from(&(&by * &cp));
    acl.view_mut((nx, nx), (nc, nc)).copy_from(&ac);
    let mut bref = DMatrix::zeros(total, ny);
    bref.rows_mut(0, nx).copy_from(&(&bp * &ur));
    bref.rows_mut(nx, nc).copy_from(&br);
    let mut affine = DVector::zeros(total);
    affine.rows_mut(0, nx).copy_from(
        &(dm(&model.bv)
            .view((0, model.md_idx.len()), (nx, 1))
            .into_owned()
            + &bp * affine_u),
    );
    affine.rows_mut(nx, nc).copy_from(&bc);
    result.nominal = spectral(
        e,
        acl.clone(),
        "Acl on [physical plant deviation; controller state], MD/UD fixed at nominal, constraints ignored",
    )?;
    if !result.nominal.assessed || !result.nominal.pass {
        result.offset = Check::unavailable(
            "nominal closed loop is not strictly stable; no convergent steady-state step response",
        );
        return Ok(result);
    }
    let system = DMatrix::identity(total, total) - acl;
    e.check_cancel()?;
    let Some(svd) =
        nalgebra::linalg::SVD::try_new(system.clone(), false, false, f64::EPSILON, 20_000)
    else {
        result.offset =
            Check::unavailable("bounded steady-state singular-value iteration did not converge");
        return Ok(result);
    };
    if svd.singular_values.min() <= 1e-12 * svd.singular_values.max().max(1.) {
        result.offset = Check::unavailable(
            "I-Acl is singular or numerically rank deficient (relative tolerance 1e-12)",
        );
        return Ok(result);
    }
    let mut rhs = DMatrix::zeros(total, ny + 1);
    rhs.columns_mut(0, ny).copy_from(&bref);
    rhs.set_column(ny, &affine);
    let Some(solution) = system.clone().lu().solve(&rhs) else {
        result.offset = Check::unavailable("steady-state linear solve failed");
        return Ok(result);
    };
    let residual =
        (&system * &solution - &rhs).norm() / (1. + system.norm() * solution.norm() + rhs.norm());
    if !residual.is_finite() || residual > 1e-10 {
        result.offset = Check::unavailable("steady-state solve residual exceeds 1e-10");
        return Ok(result);
    }
    let output = dm(&m.cd) * solution.rows(0, nx);
    let error = DMatrix::identity(ny, ny) - output.columns(0, ny);
    result.baseline_error = -output.column(ny).into_owned();
    let controlled: Vec<_> = m
        .mo_idx
        .iter()
        .chain(&m.uo_idx)
        .filter_map(|&physical| (m.weights.ov.get(0, physical) > 0.).then_some(physical))
        .collect();
    if controlled.is_empty() {
        result.offset = Check::unavailable("no controlled outputs with positive output weight");
        return Ok(result);
    }
    let mut worst: f64 = 0.;
    for &i in &controlled {
        worst = worst.max(result.baseline_error[i].abs());
        for &j in &controlled {
            worst = worst.max(error[(i, j)].abs());
        }
    }
    result.offset = Check {
        assessed: true,
        pass: worst <= OFFSET_TOL,
        metric: worst,
        reason: format!(
            "physical unit steps on positive-weight outputs; Error=I-Ccl*(I-Acl)\\Bref; includes nominal baseline error; tolerance 1e-6; solve residual={residual:.3e}"
        ),
        matrix: error,
    };
    e.check_cancel()?;
    Ok(result)
}

pub(super) fn outputs(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.len() != 1 || count > 1 {
        return Err("review: expected one mpc controller and at most one output".into());
    }
    let m = objects::mpc_value(e, &args[0]).ok_or("review: input must be an mpc controller")?;
    e.check_cancel()?;
    // Small bounded summary scratch plus the largest allowed blocking vector.
    if e.limits.memory_bytes < 2 * MAX_TEXT + 8 * 10_000 {
        return Err("review: summary workspace exceeds memory limit".into());
    }
    // Summary loops are bounded by existing object limits, even for an analysis
    // outside the smaller numerical budget. Retained matrices stay shared.
    let (constraints, constraint_text) = constraints(e, &m)?;
    let mut diagnostics = Execution::default();
    let a = match analyze(e, &m, &mut diagnostics) {
        Ok(a) => a,
        Err(reason) => {
            e.check_cancel()?;
            Analysis::unavailable(&format!(
                "model/objective/estimator construction failed: {reason}"
            ))
        }
    };
    let mut body = format!(
        "MPC review: nominal unconstrained model (no safety guarantee)\nPlant: nx={} nu={} ny={} MV={} MD={} UD={} MO={} UO={} Ts={:.12e}\nPredictionHorizon={} ControlHorizon={}\nWeights OV={} MV={} MVRate={} ECR={:.12e}\n",
        m.nx(),
        m.nu(),
        m.ny(),
        m.n_mv(),
        m.md_idx.len(),
        m.ud_idx.len(),
        m.mo_idx.len(),
        m.uo_idx.len(),
        m.ts,
        m.prediction_horizon,
        preview(&match &m.control_horizon {
            ControlHorizon::Scalar(n) => Matrix::scalar(*n as f64),
            ControlHorizon::Blocking(b) =>
                Matrix::from_column_vec(1, b.len(), b.iter().map(|&n| n as f64).collect())?,
        }),
        preview(&m.weights.ov),
        preview(&m.weights.mv),
        preview(&m.weights.mv_rate),
        m.weights.ecr
    );
    body.push_str(
        &a.internal
            .line("Internal controller stability", "max |eig|"),
    );
    body.push_str(&a.nominal.line("Nominal closed-loop stability", "max |eig|"));
    body.push_str(&a.offset.line("Steady-state output offset", "max |offset|"));
    body.push_str(&a.hessian.line("Condensed-QP Hessian", "cond2(H)"));
    body.push_str(&constraint_text);
    if body.len() > MAX_TEXT
        || body.len() > e.limits.display_bytes
        || (count == 0 && e.limits.output_bytes.is_some_and(|n| body.len() > n))
    {
        return Err("review: bounded report exceeds output limit".into());
    }
    e.check_cancel()?;
    if count == 0 {
        out.output.push(body);
        return Ok(Vec::new());
    }
    let plant = super::structure(
        e,
        vec![
            ("States", number(m.nx() as f64)),
            ("Inputs", number(m.nu() as f64)),
            ("Outputs", number(m.ny() as f64)),
            ("MV", number(m.n_mv() as f64)),
            ("MD", number(m.md_idx.len() as f64)),
            ("UD", number(m.ud_idx.len() as f64)),
            ("MO", number(m.mo_idx.len() as f64)),
            ("UO", number(m.uo_idx.len() as f64)),
            ("Ts", number(m.ts)),
        ],
    )?
    .array(&e.cancel)?;
    Ok(vec![super::structure(
        e,
        vec![
            ("Plant", plant),
            ("PredictionHorizon", number(m.prediction_horizon as f64)),
            (
                "ControlHorizon",
                object::property(e, &m, "ControlHorizon")?.array(&e.cancel)?,
            ),
            (
                "Weights",
                object::property(e, &m, "Weights")?.array(&e.cancel)?,
            ),
            (
                "InternalStability",
                a.internal.export(e, "MaxAbsEigenvalue")?,
            ),
            ("NominalStability", a.nominal.export(e, "MaxAbsEigenvalue")?),
            ("SteadyState", a.offset.export(e, "MaxAbsOffset")?),
            (
                "BaselineError",
                Array::Numeric(matrix(DMatrix::from_column_slice(
                    a.baseline_error.len(),
                    1,
                    a.baseline_error.as_slice(),
                ))?),
            ),
            ("HessianMatrix", a.hessian.export(e, "ConditionNumber")?),
            ("Regularization", number(a.regularization)),
            ("HardConstraints", constraints.array(&e.cancel)?),
            ("Text", text(e, &body)?),
        ],
    )?])
}
