//! Dense MPC in scaled deviations. Public specification:
//! https://www.mathworks.com/help/mpc/ug/optimization-problem.html
//! J = sum((wy*(y-r)/sy)^2 + (wu*(u-target)/su)^2
//!         + (wdu*du/su)^2) + rho*epsilon^2, epsilon >= 0.
//! Only increments at block starts are free; states are eliminated recursively.
use super::super::{Engine, Execution, Value, objects};
use super::{
    estimator,
    object::{ControlHorizon, MpcObject, MvTarget},
    prediction::{self, dm, matrix},
    qp::{self, QpOptions, QpStatus},
};
use crate::{Array, CharacterRow, Matrix, Result};
use nalgebra::{DMatrix, DVector};

const OPTION_FIELDS: &[&str] = &[
    "OutputWeights",
    "MVWeights",
    "MVRateWeights",
    "ECRWeight",
    "OutputMin",
    "OutputMax",
    "MVMin",
    "MVMax",
    "MVUsed",
];

pub(super) fn options(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if !args.is_empty() || count > 1 {
        return Err("mpcmoveopt: expected no inputs and one output".into());
    }
    Ok(vec![super::structure(
        e,
        OPTION_FIELDS
            .iter()
            .map(|&k| Ok((k, Array::Numeric(Matrix::filled(0, 0, 0.)?))))
            .collect::<Result<_>>()?,
    )?])
}

fn real(v: &Value, name: &str) -> Result<Matrix> {
    match v {
        Value::Matrix(m) if !m.is_nd() => Ok(m.clone()),
        _ => Err(format!("mpcmove: {name} must be a real double matrix")),
    }
}
fn rows(m: &Matrix, p: usize, n: usize, finite: bool, name: &str) -> Result<()> {
    if m.rows() == 0
        || m.rows() > p
        || m.cols() != n
        || m.data()
            .iter()
            .any(|v| v.is_nan() || (finite && !v.is_finite()))
    {
        return Err(format!(
            "mpcmove: {name} must have 1..{p} rows and {n} columns with {}values",
            if finite { "finite " } else { "non-NaN " }
        ));
    }
    Ok(())
}
fn at(m: &Matrix, k: usize, j: usize) -> f64 {
    m.get(k.min(m.rows() - 1), j)
}
fn bound(m: &Matrix, k: usize) -> f64 {
    m.data()[k.min(m.len() - 1)]
}
fn sequence(v: Option<&Value>, default: &[f64], p: usize, name: &str) -> Result<Matrix> {
    let m = v.map(|v| real(v, name)).transpose()?;
    let m = match m {
        Some(m) if !m.is_empty() => m,
        _ => Matrix::from_column_vec(1, default.len(), default.to_vec())?,
    };
    // A controller without MD ports accepts only [].
    if default.is_empty() && m.is_empty() {
        return Ok(m);
    }
    rows(&m, p, default.len(), true, name)?;
    Ok(m)
}
fn validate(m: &MpcObject) -> Result<()> {
    let p = m.prediction_horizon;
    for (w, n, name) in [
        (&m.weights.ov, m.ny(), "OutputWeights"),
        (&m.weights.mv, m.n_mv(), "MVWeights"),
        (&m.weights.mv_rate, m.n_mv(), "MVRateWeights"),
    ] {
        rows(w, p, n, true, name)?;
    }
    for v in &m.mv {
        for b in [
            &v.min,
            &v.max,
            &v.min_ecr,
            &v.max_ecr,
            &v.rate_min,
            &v.rate_max,
            &v.rate_min_ecr,
            &v.rate_max_ecr,
        ] {
            if b.is_empty() || b.len() > p {
                return Err("mpcmove: MV bound/ECR rows exceed current PredictionHorizon".into());
            }
        }
        if matches!(&v.target, MvTarget::Vector(t) if t.len()>p) {
            return Err("mpcmove: MV.Target rows exceed current PredictionHorizon".into());
        }
    }
    for v in &m.ov {
        for b in [&v.min, &v.max, &v.min_ecr, &v.max_ecr] {
            if b.is_empty() || b.len() > p {
                return Err(
                    "mpcmove: Output bound/ECR rows exceed current PredictionHorizon".into(),
                );
            }
        }
    }
    Ok(())
}
fn replace_bound(old: &Matrix, v: &Matrix, j: usize, name: &str) -> Result<Matrix> {
    let first = old
        .data()
        .iter()
        .find(|v| v.is_finite())
        .copied()
        .ok_or_else(|| format!("mpcmove: {name} requires an existing finite controller bound"))?;
    if v.rows() == 1 && old.len() > 1 {
        // A constant override translates the original finite profile.
        Matrix::from_column_vec(
            old.len(),
            1,
            old.data()
                .iter()
                .map(|&b| {
                    if b.is_finite() {
                        b + v.get(0, j) - first
                    } else {
                        b
                    }
                })
                .collect(),
        )
    } else {
        Matrix::from_column_vec(v.rows(), 1, (0..v.rows()).map(|i| v.get(i, j)).collect())
    }
}
fn overrides(e: &Engine, m: &mut MpcObject, v: &Value) -> Result<Option<Matrix>> {
    let Value::Container(Array::Struct(s)) = v else {
        return Err("mpcmove: options must be a scalar mpcmoveopt structure".into());
    };
    if s.len() != 1 {
        return Err("mpcmove: options must be a scalar structure".into());
    }
    let mut used = None;
    for (i, key) in s.field_names().iter().enumerate() {
        let name = key.as_str();
        if !OPTION_FIELDS.contains(&name) && name != "MVused" {
            return Err(format!("mpcmove: option '{name}' is not supported"));
        }
        let v = real(&Value::from(s.get(i, 0)?.as_ref().clone()), name)?;
        if v.is_empty() {
            continue;
        }
        match name {
            "OutputWeights" | "MVWeights" | "MVRateWeights" => {
                let w = match name {
                    "OutputWeights" => &mut m.weights.ov,
                    "MVWeights" => &mut m.weights.mv,
                    _ => &mut m.weights.mv_rate,
                };
                rows(&v, m.prediction_horizon, w.cols(), true, name)?;
                if v.data().iter().any(|&v| v < 0.) || (v.rows() > 1) != (w.rows() > 1) {
                    return Err(format!(
                        "mpcmove: {name} must be nonnegative and match controller weight format"
                    ));
                }
                *w = v;
            }
            "ECRWeight" => {
                if v.len() != 1 || !v.data()[0].is_finite() || v.data()[0] <= 0. {
                    return Err("mpcmove: ECRWeight must be a positive finite scalar".into());
                }
                m.weights.ecr = v.data()[0];
            }
            "MVUsed" | "MVused" => {
                rows(&v, 1, m.n_mv(), true, name)?;
                used = Some(v.reshape(m.n_mv(), 1)?);
            }
            _ => {
                let ov = name.starts_with("Output");
                rows(
                    &v,
                    m.prediction_horizon,
                    if ov { m.ny() } else { m.n_mv() },
                    false,
                    name,
                )?;
                if v.data().iter().any(|&b| {
                    if name.ends_with("Min") {
                        b == f64::INFINITY
                    } else {
                        b == f64::NEG_INFINITY
                    }
                }) {
                    return Err(format!("mpcmove: {name} has an invalid infinite bound"));
                }
                if ov {
                    // Override column j replaces OV(j), just like OutputWeights.
                    for (j, ov) in m.ov.iter_mut().enumerate() {
                        let b = if name == "OutputMin" {
                            &mut ov.min
                        } else {
                            &mut ov.max
                        };
                        *b = replace_bound(b, &v, j, name)?;
                    }
                } else {
                    for (j, mv) in m.mv.iter_mut().enumerate() {
                        let b = if name == "MVMin" {
                            &mut mv.min
                        } else {
                            &mut mv.max
                        };
                        *b = replace_bound(b, &v, j, name)?;
                    }
                }
            }
        }
    }
    e.check_cancel()?;
    Ok(used)
}

struct Quadratic {
    h: DMatrix<f64>,
    f: DVector<f64>,
    constant: f64,
    a: Vec<Vec<f64>>,
    b: Vec<f64>,
}
impl Quadratic {
    fn term(&mut self, row: &[f64], residual: f64, weight: f64) {
        if weight == 0. {
            return;
        }
        let w = weight * weight;
        self.constant += w * residual * residual;
        for (j, &rj) in row.iter().enumerate() {
            if rj == 0. {
                continue;
            }
            self.f[j] += 2. * w * residual * rj;
            for (i, &ri) in row.iter().enumerate() {
                self.h[(i, j)] += 2. * w * ri * rj;
            }
        }
    }
    fn limit(&mut self, row: &[f64], base: f64, limit: f64, ecr: f64, sign: f64) -> Result<()> {
        if !limit.is_finite() {
            return Ok(());
        }
        if self.a.len() == qp::MAX_M {
            return Err("mpcmove: constraint count exceeds 4000".into());
        }
        let mut r: Vec<f64> = row.iter().map(|v| sign * v).collect();
        *r.last_mut().unwrap() = -ecr;
        self.a.push(r);
        self.b.push(sign * (limit - base));
        Ok(())
    }
}
fn starts(m: &MpcObject) -> Vec<usize> {
    match &m.control_horizon {
        ControlHorizon::Scalar(n) => (0..*n).collect(),
        ControlHorizon::Blocking(b) => {
            let mut k = 0;
            b.iter()
                .map(|&size| {
                    let start = k;
                    k += size;
                    start
                })
                .collect()
        }
    }
}

// Shared read-only objective input for review. This is the very same assembly and
// regularization used below by mpcmove, including its ECR slack coordinate.
fn objective(
    e: &Engine,
    m: &MpcObject,
    model: &prediction::MpcPredictionModel,
    corrected: &DVector<f64>,
    last: &DVector<f64>,
    r: &Matrix,
    v: &Matrix,
) -> Result<(Quadratic, f64)> {
    let p = m.prediction_horizon;
    let nu = m.n_mv();
    let block = starts(m);
    let d = nu * block.len() + 1;
    let n = model.a.rows();
    let a = dm(&model.a);
    let bu = dm(&model.bu);
    let bv = dm(&model.bv);
    let c = dm(&model.c_prediction);
    let dv = dm(&model.dv);
    let disturbances = |k| {
        DVector::from_iterator(
            model.md_idx.len() + 1,
            model
                .md_idx
                .iter()
                .enumerate()
                .map(|(j, &physical)| {
                    (at(v, k, j) - model.u_offset.get(physical, 0))
                        / model.input_scale.get(physical, 0)
                })
                .chain([1.]),
        )
    };
    let mut xb = corrected.clone();
    let mut xf = DMatrix::zeros(n, d);
    let mut uf = DMatrix::zeros(nu, d);
    let mut q = Quadratic {
        h: DMatrix::zeros(d, d),
        f: DVector::zeros(d),
        constant: 0.,
        a: Vec::new(),
        b: Vec::new(),
    };
    q.h[(d - 1, d - 1)] = 2. * m.weights.ecr;
    let mut slack_row = vec![0.; d];
    slack_row[d - 1] = -1.;
    q.a.push(slack_row);
    q.b.push(0.);
    for k in 0..p {
        e.check_cancel()?;
        let mut rate = DMatrix::zeros(nu, d);
        if let Some(b) = block.iter().position(|&t| t == k) {
            for j in 0..nu {
                uf[(j, b * nu + j)] = 1.;
                rate[(j, b * nu + j)] = 1.;
            }
        }
        for j in 0..nu {
            let mv = &m.mv[j];
            let ur: Vec<_> = uf.row(j).iter().copied().collect();
            let dr: Vec<_> = rate.row(j).iter().copied().collect();
            let offset = model.u_offset.get(model.mv_idx[j], 0);
            let target = match &mv.target {
                MvTarget::Nominal => offset,
                MvTarget::Scalar(t) => *t,
                MvTarget::Vector(t) => bound(t, k),
            };
            q.term(
                &ur,
                last[j] - (target - offset) / mv.scale_factor,
                at(&m.weights.mv, k, j),
            );
            q.term(&dr, 0., at(&m.weights.mv_rate, k, j));
            q.limit(
                &ur,
                last[j],
                (bound(&mv.min, k) - offset) / mv.scale_factor,
                bound(&mv.min_ecr, k),
                -1.,
            )?;
            q.limit(
                &ur,
                last[j],
                (bound(&mv.max, k) - offset) / mv.scale_factor,
                bound(&mv.max_ecr, k),
                1.,
            )?;
            q.limit(
                &dr,
                0.,
                bound(&mv.rate_min, k) / mv.scale_factor,
                bound(&mv.rate_min_ecr, k),
                -1.,
            )?;
            q.limit(
                &dr,
                0.,
                bound(&mv.rate_max, k) / mv.scale_factor,
                bound(&mv.rate_max_ecr, k),
                1.,
            )?;
        }
        xb = &a * xb + &bu * last + &bv * disturbances(k);
        xf = &a * xf + &bu * &uf;
        let yb = &c * &xb + &dv * disturbances(k + 1);
        let yf = &c * &xf;
        for (j, &physical) in model.mo_idx.iter().chain(&model.uo_idx).enumerate() {
            let ov = &m.ov[physical];
            let yr: Vec<_> = yf.row(j).iter().copied().collect();
            let offset = model.y_offset.get(physical, 0);
            let scale = ov.scale_factor;
            q.term(
                &yr,
                yb[j] - (at(r, k, physical) - offset) / scale,
                at(&m.weights.ov, k, physical),
            );
            q.limit(
                &yr,
                yb[j],
                (bound(&ov.min, k) - offset) / scale,
                bound(&ov.min_ecr, k),
                -1.,
            )?;
            q.limit(
                &yr,
                yb[j],
                (bound(&ov.max, k) - offset) / scale,
                bound(&ov.max_ecr, k),
                1.,
            )?;
        }
    }
    // A zero rate weight can leave a singular or ill-conditioned move objective.
    let mut regularization = 0.;
    if m.weights.mv_rate.data().contains(&0.) && d > 1 {
        let eigen =
            q.h.view((0, 0), (d - 1, d - 1))
                .into_owned()
                .symmetric_eigen()
                .eigenvalues;
        let max = eigen.max();
        let min = eigen.min();
        if min <= 0. || max / min > 1e12 {
            regularization = 2. * 10. * f64::EPSILON.sqrt();
            for i in 0..d - 1 {
                q.h[(i, i)] += regularization;
            }
        }
    }
    Ok((q, regularization))
}

/// Exact solver Hessian and linear objective for independent, private analysis
/// vectors. Callers bound their dimensions/workspace before invoking this.
pub(super) fn review_objective(
    e: &Engine,
    m: &MpcObject,
    model: &prediction::MpcPredictionModel,
    corrected: &DVector<f64>,
    last: &DVector<f64>,
    r: &Matrix,
) -> Result<(DMatrix<f64>, DVector<f64>, f64)> {
    validate(m)?;
    let v = Matrix::from_column_vec(
        1,
        model.md_idx.len(),
        model
            .md_idx
            .iter()
            .map(|&j| model.u_offset.get(j, 0))
            .collect(),
    )?;
    let (q, regularization) = objective(e, m, model, corrected, last, r, &v)?;
    Ok((q.h, q.f, regularization))
}

pub(super) fn outputs(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !(2..=6).contains(&args.len()) || count > 2 {
        return Err(
            "mpcmove: expected controller,state[,ym,r,v,options] and at most two outputs".into(),
        );
    }
    let original =
        objects::mpc_value(e, &args[0]).ok_or("mpcmove: first input must be an mpc controller")?;
    let mut m = original.as_ref().clone();
    validate(&m)?;
    let (state_id, state) = objects::mpc_state_value(e, &args[1])
        .ok_or("mpcmove: second input must be an mpcstate handle")?;
    let mut next = state.as_ref().clone();
    let option_index = args
        .iter()
        .enumerate()
        .skip(2)
        .find_map(|(i, v)| matches!(v, Value::Container(Array::Struct(_))).then_some(i));
    let end = option_index.unwrap_or(args.len());
    if option_index.is_some_and(|i| i + 1 != args.len()) || end > 5 {
        return Err("mpcmove: options must be the last input".into());
    }
    let used = if let Some(i) = option_index {
        overrides(e, &mut m, &args[i])?
    } else {
        None
    };
    validate(&m)?;
    let p = m.prediction_horizon;
    let nu = m.n_mv();
    let ny = m.ny();
    let block = starts(&m);
    let d = nu
        .checked_mul(block.len())
        .and_then(|n| n.checked_add(1))
        .filter(|&n| n <= qp::MAX_N)
        .ok_or("mpcmove: blocked moves plus slack exceed 200 variables")?;
    // Check a conservative allocation bound before building the augmented model.
    let cells = (p + 1)
        .checked_mul(ny + nu + prediction::MAX_DIM)
        .and_then(|v| v.checked_add(qp::MAX_M * d * 3 + d * d * 12 + prediction::MAX_DIM * d * 8))
        .ok_or("mpcmove: workspace size overflow")?;
    if cells
        .saturating_mul(8)
        .saturating_add(qp::workspace_bytes(d, qp::MAX_M, 0)?)
        > e.limits.memory_bytes
    {
        return Err("mpcmove: workspace/output exceeds memory limit".into());
    }
    let model = prediction::build(e, &original, out)?;
    let n = model.a.rows();
    if next.plant.len() != model.plant_states.len()
        || next.disturbance.len()
            != model.input_disturbance_states.len() + model.output_disturbance_states.len()
        || next.noise.len() != model.noise_states.len()
        || next.last_move.len() != nu
    {
        return Err("mpcmove: mpcstate dimensions do not match the current controller".into());
    }
    let ym_arg = args
        .get(2)
        .filter(|_| end > 2)
        .map(|v| real(v, "ym"))
        .transpose()?;
    let ym = match ym_arg {
        Some(v) if !v.is_empty() => {
            if v.len() != m.mo_idx.len()
                || (v.rows() != 1 && v.cols() != 1)
                || v.data().iter().any(|x| !x.is_finite())
            {
                return Err(
                    "mpcmove: ym must be a finite vector with one value per measured output".into(),
                );
            }
            v
        }
        _ => Matrix::from_column_vec(
            m.mo_idx.len(),
            1,
            m.mo_idx.iter().map(|&j| model.y_offset.get(j, 0)).collect(),
        )?,
    };
    let r = sequence(
        args.get(3).filter(|_| end > 3),
        model.y_offset.data(),
        p,
        "r",
    )?;
    let v = sequence(
        args.get(4).filter(|_| end > 4),
        &model
            .md_idx
            .iter()
            .map(|&j| model.u_offset.get(j, 0))
            .collect::<Vec<_>>(),
        p + 1,
        "v",
    )?;
    let a = dm(&model.a);
    let bu = dm(&model.bu);
    let bv = dm(&model.bv);
    let c = dm(&model.c_prediction);
    let dv = dm(&model.dv);
    let disturbances = |k| {
        DVector::from_iterator(
            model.md_idx.len() + 1,
            model
                .md_idx
                .iter()
                .enumerate()
                .map(|(j, &physical)| {
                    (at(&v, k, j) - model.u_offset.get(physical, 0))
                        / model.input_scale.get(physical, 0)
                })
                .chain([1.]),
        )
    };
    let previous = used.as_ref().unwrap_or(&next.last_move);
    let last = DVector::from_iterator(
        nu,
        (0..nu).map(|j| {
            (previous.data()[j] - model.u_offset.get(model.mv_idx[j], 0)) / m.mv[j].scale_factor
        }),
    );
    let mut prior = DVector::from_iterator(
        n,
        next.plant
            .data()
            .iter()
            .zip(model.x_offset.data())
            .map(|(x, o)| x - o)
            .chain(next.disturbance.data().iter().copied())
            .chain(next.noise.data().iter().copied()),
    );
    let custom = matches!(m.estimator, estimator::EstimatorMode::Custom);
    if used.is_some() && !custom {
        let delta = DVector::from_iterator(
            nu,
            (0..nu).map(|j| (previous.data()[j] - next.last_move.data()[j]) / m.mv[j].scale_factor),
        );
        prior += &bu * delta;
    }
    let gains = estimator::design(e, &model, &m.estimator, out)?;
    let innovation = DVector::from_iterator(
        m.mo_idx.len(),
        m.mo_idx.iter().enumerate().map(|(j, &physical)| {
            (ym.data()[j] - model.y_offset.get(physical, 0)) / model.output_scale.get(j, 0)
        }),
    ) - dm(&model.cm) * &prior
        - dm(&model.dvm) * disturbances(0);
    let corrected = if custom {
        prior.clone()
    } else {
        &prior + dm(&gains.m) * &innovation
    };
    let (q, regularization) = objective(e, &m, &model, &corrected, &last, &r, &v)?;
    let constraints = Matrix::from_column_vec(
        q.a.len(),
        d,
        (0..d)
            .flat_map(|j| q.a.iter().map(move |row| row[j]))
            .collect(),
    )?;
    let warm = if next.active.len() == q.a.len() && next.active_variables == d {
        next.active.clone()
    } else {
        vec![false; q.a.len()]
    };
    let opts = QpOptions {
        max_iterations: m.optimizer.max_iterations,
        constraint_tolerance: m.optimizer.constraint_tolerance,
        cancel: Some(&e.cancel),
        ..QpOptions::default()
    };
    let result = match qp::solve(
        &matrix(q.h.clone())?,
        q.f.as_slice(),
        &constraints,
        &q.b,
        &Matrix::filled(0, d, 0.)?,
        &[],
        &warm,
        &opts,
    ) {
        Ok(result) => result,
        Err(err) if err == "mpcActiveSetSolver: H must be symmetric positive definite" => {
            qp::QpResult {
                x: vec![0.; d],
                status: QpStatus::Unreliable,
                iterations: 0,
                active: Vec::new(),
                lambda: qp::QpLambda {
                    ineqlin: Vec::new(),
                    eqlin: Vec::new(),
                },
            }
        }
        Err(err) => return Err(err),
    };
    let flag = result.exitflag();
    let feasible = result.status == QpStatus::Optimal;
    let code = match result.status {
        QpStatus::Optimal => "feasible",
        QpStatus::MaxIterations | QpStatus::Unreliable => "unreliable",
        _ => "infeasible",
    };
    let mut z = if feasible {
        DVector::from_column_slice(&result.x)
    } else {
        DVector::zeros(d)
    };
    if !feasible {
        for j in 0..nu {
            z[j] = (next.last_move.data()[j] - previous.data()[j]) / m.mv[j].scale_factor;
        }
    }
    let slack = z[d - 1].max(0.);
    let first = &last + z.rows(0, nu);
    let mv = Matrix::from_column_vec(
        nu,
        1,
        (0..nu)
            .map(|j| model.u_offset.get(model.mv_idx[j], 0) + m.mv[j].scale_factor * first[j])
            .collect(),
    )?;
    // L advances the a-priori state, M corrects the current state: no double correction.
    if !custom {
        let predicted =
            &a * prior + &bu * &first + &bv * disturbances(0) + dm(&gains.l) * innovation;
        next.plant = Matrix::from_column_vec(
            model.plant_states.len(),
            1,
            model
                .plant_states
                .clone()
                .map(|i| predicted[i] + model.x_offset.get(i, 0))
                .collect(),
        )?;
        let range = model.input_disturbance_states.start..model.output_disturbance_states.end;
        next.disturbance =
            Matrix::from_column_vec(range.len(), 1, range.map(|i| predicted[i]).collect())?;
        next.noise = Matrix::from_column_vec(
            model.noise_states.len(),
            1,
            model.noise_states.clone().map(|i| predicted[i]).collect(),
        )?;
        next.covariance = gains.covariance;
    }
    next.last_move = mv.clone();
    next.active = if feasible { result.active } else { Vec::new() };
    next.active_variables = d;
    let mut values = vec![Value::Matrix(mv)];
    if count > 1 {
        let mut uopt = DMatrix::zeros(p + 1, nu);
        let mut yopt = DMatrix::zeros(p + 1, ny);
        let nx = model.noise_states.start;
        let mut xopt = DMatrix::zeros(p + 1, nx);
        let mut x = corrected;
        let mut u = last;
        for k in 0..=p {
            e.check_cancel()?;
            if let Some(b) = block.iter().position(|&t| t == k) {
                u += z.rows(b * nu, nu);
            }
            for j in 0..nu {
                uopt[(k, j)] = model.u_offset.get(model.mv_idx[j], 0) + m.mv[j].scale_factor * u[j];
            }
            for j in 0..nx {
                xopt[(k, j)] = x[j]
                    + if j < model.plant_states.end {
                        model.x_offset.get(j, 0)
                    } else {
                        0.
                    };
            }
            let y = &c * &x + &dv * disturbances(k);
            for (j, &physical) in model.mo_idx.iter().chain(&model.uo_idx).enumerate() {
                yopt[(k, physical)] =
                    model.y_offset.get(physical, 0) + m.ov[physical].scale_factor * y[j];
            }
            if k < p {
                x = &a * x + &bu * &u + &bv * disturbances(k);
            }
        }
        let cost = (0.5 * z.dot(&(&q.h * &z)) + q.f.dot(&z) + q.constant
            - 0.5 * regularization * z.rows(0, d - 1).norm_squared())
        .max(0.);
        values.push(super::structure(
            e,
            vec![
                ("Uopt", Array::Numeric(matrix(uopt)?)),
                ("Yopt", Array::Numeric(matrix(yopt)?)),
                ("Xopt", Array::Numeric(matrix(xopt)?)),
                (
                    "Topt",
                    Array::Numeric(Matrix::from_column_vec(
                        p + 1,
                        1,
                        (0..=p).map(|k| k as f64 * m.ts).collect(),
                    )?),
                ),
                ("Slack", Array::Numeric(Matrix::scalar(slack))),
                ("Iterations", Array::Numeric(Matrix::scalar(flag))),
                (
                    "QPCode",
                    Array::Character(CharacterRow::from_text(code, &e.cancel)?),
                ),
                ("Cost", Array::Numeric(Matrix::scalar(cost))),
            ],
        )?);
    }
    objects::mpc_state_update(e, state_id, next)?;
    Ok(values)
}
