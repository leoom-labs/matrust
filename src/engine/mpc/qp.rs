//! Dense strictly convex QPs, with the public mpcActiveSetSolver sense Ax <= b.
//! MathWorks: "subject to inequality constraints Ax ≤ b" (public solver page).
//! https://www.mathworks.com/help/mpc/ref/mpcactivesetsolver.html
//! Own dual active-set method: whiten with one Hessian Cholesky, then project
//! violated halfspaces while retaining dual feasibility. Twice-reorthogonalized
//! QR is rebuilt on every basis change; no Hessian or KKT inverse is formed.
//! Iteration counts describe this implementation, not KWIK's trajectory.
use crate::{Matrix, Result};
use nalgebra::{DMatrix, DVector};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const MAX_N: usize = 200;
pub(crate) const MAX_M: usize = 4000;
const RANK_TOL: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QpStatus {
    Optimal,
    MaxIterations,
    Infeasible,
    Unreliable,
}
#[derive(Debug)]
pub(crate) struct QpLambda {
    pub ineqlin: Vec<f64>,
    pub eqlin: Vec<f64>,
}
#[derive(Debug)]
pub(crate) struct QpResult {
    pub x: Vec<f64>,
    pub status: QpStatus,
    pub iterations: usize,
    /// Independent working inequalities, suitable for a subsequent warm start.
    pub active: Vec<bool>,
    pub lambda: QpLambda,
}
impl QpResult {
    pub(crate) fn exitflag(&self) -> f64 {
        match self.status {
            QpStatus::Optimal => self.iterations.max(1) as f64,
            QpStatus::MaxIterations => 0.,
            QpStatus::Infeasible => -1.,
            QpStatus::Unreliable => -2.,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct QpOptions<'a> {
    pub max_iterations: usize,
    pub constraint_tolerance: f64,
    pub use_hessian_as_input: bool,
    pub integrity_checks: bool,
    pub cancel: Option<&'a AtomicBool>,
}
impl Default for QpOptions<'_> {
    fn default() -> Self {
        Self {
            max_iterations: 200,
            constraint_tolerance: 1e-6,
            use_hessian_as_input: true,
            integrity_checks: true,
            cancel: None,
        }
    }
}
fn cancelled(opts: &QpOptions<'_>) -> Result<()> {
    if opts.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err("Execution cancelled".into())
    } else {
        Ok(())
    }
}

struct Row {
    normal: DVector<f64>,
    bound: f64,
    scale: f64,
}
impl Row {
    fn residual(&self, y: &DVector<f64>) -> f64 {
        if self.scale == 0. {
            -self.bound
        } else {
            (self.normal.dot(y) - self.bound) * self.scale
        }
    }
}
struct Basis {
    q: Vec<DVector<f64>>,
    r: DMatrix<f64>,
}
impl Basis {
    fn new(rows: &[Row], working: &[usize]) -> Option<Self> {
        let mut basis = Self {
            q: Vec::with_capacity(working.len()),
            r: DMatrix::zeros(working.len(), working.len()),
        };
        for (j, &index) in working.iter().enumerate() {
            let (z, coefficients) = basis.project(&rows[index].normal);
            let norm = z.norm();
            if norm <= RANK_TOL || !norm.is_finite() {
                return None;
            }
            for (i, v) in coefficients.iter().enumerate() {
                basis.r[(i, j)] = *v;
            }
            basis.r[(j, j)] = norm;
            basis.q.push(z / norm);
        }
        Some(basis)
    }
    /// Projection on the orthogonal complement, with a second pass to retain
    /// accuracy for nearly parallel constraint normals.
    fn project(&self, a: &DVector<f64>) -> (DVector<f64>, DVector<f64>) {
        let mut z = a.clone();
        let mut v = DVector::zeros(self.q.len());
        for _ in 0..2 {
            for (i, q) in self.q.iter().enumerate() {
                let d = q.dot(&z);
                v[i] += d;
                z.axpy(-d, q, 1.);
            }
        }
        (z, v)
    }
    fn representation(&self, v: &DVector<f64>) -> DVector<f64> {
        let mut r = v.clone();
        for i in (0..r.len()).rev() {
            for j in i + 1..r.len() {
                r[i] -= self.r[(i, j)] * r[j];
            }
            r[i] /= self.r[(i, i)];
        }
        r
    }
    fn optimum(
        &self,
        rows: &[Row],
        working: &[usize],
        y0: &DVector<f64>,
    ) -> (DVector<f64>, DVector<f64>) {
        let mut v = DVector::zeros(working.len());
        for (i, &index) in working.iter().enumerate() {
            v[i] = rows[index].normal.dot(y0) - rows[index].bound;
            for j in 0..i {
                v[i] -= self.r[(j, i)] * v[j];
            }
            v[i] /= self.r[(i, i)];
        }
        let mut y = y0.clone();
        for (q, &v) in self.q.iter().zip(v.iter()) {
            y.axpy(-v, q, 1.);
        }
        (y, self.representation(&v))
    }
}

enum Factor {
    Lower(DMatrix<f64>),
    InverseLower(DMatrix<f64>),
}
impl Factor {
    fn whiten(&self, v: &DVector<f64>) -> DVector<f64> {
        match self {
            Self::Lower(l) => l
                .solve_lower_triangular(v)
                .expect("positive Cholesky diagonal"),
            Self::InverseLower(linv) => linv * v,
        }
    }
    fn unwhiten(&self, y: &DVector<f64>) -> DVector<f64> {
        match self {
            Self::Lower(l) => l
                .transpose()
                .solve_upper_triangular(y)
                .expect("positive Cholesky diagonal"),
            Self::InverseLower(linv) => linv.transpose() * y,
        }
    }
}

/// Scratch is bounded independently of engine callers (n <= 200, m <= 4000).
/// A conservative byte estimate also lets the builtin enforce its host budget.
pub(crate) fn workspace_bytes(n: usize, m: usize, p: usize) -> Result<usize> {
    if n == 0 || n > MAX_N || m > MAX_M || p > n {
        return Err(
            "mpcActiveSetSolver: dimensions require 1 <= n <= 200, m <= 4000, q <= n".into(),
        );
    }
    Ok(8 * (12 * n * n + 4 * (m + p) * (n + 4)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn solve(
    h: &Matrix,
    f: &[f64],
    a_ineq: &Matrix,
    b_ineq: &[f64],
    a_eq: &Matrix,
    b_eq: &[f64],
    warm_active: &[bool],
    opts: &QpOptions<'_>,
) -> Result<QpResult> {
    let n = f.len();
    let m = b_ineq.len();
    let p = b_eq.len();
    workspace_bytes(n, m, p)?;
    if (h.rows(), h.cols()) != (n, n)
        || (a_ineq.rows(), a_ineq.cols()) != (m, n)
        || (a_eq.rows(), a_eq.cols()) != (p, n)
        || warm_active.len() != m
    {
        return Err(
            "mpcActiveSetSolver: inconsistent matrix, vector or active-set dimensions".into(),
        );
    }
    if opts.max_iterations == 0
        || opts.max_iterations > 1_000_000
        || !opts.constraint_tolerance.is_finite()
        || opts.constraint_tolerance <= 0.
    {
        return Err("mpcActiveSetSolver: invalid iteration limit or constraint tolerance".into());
    }
    cancelled(opts)?;
    // These guards remain mandatory even with IntegrityChecks=false: turning
    // off diagnostics must not allow NaNs or unchecked allocations into kernels.
    if h.data()
        .iter()
        .chain(f)
        .chain(a_ineq.data())
        .chain(b_ineq)
        .chain(a_eq.data())
        .chain(b_eq)
        .any(|x| !x.is_finite())
    {
        return Err("mpcActiveSetSolver: inputs must be finite real arrays".into());
    }
    let input = DMatrix::from_column_slice(n, n, h.data());
    let factor = if opts.use_hessian_as_input {
        for j in 0..n {
            for i in 0..j {
                if (input[(i, j)] - input[(j, i)]).abs()
                    > 1e-12 * input[(i, j)].abs().max(input[(j, i)].abs()).max(1.)
                {
                    return Err("mpcActiveSetSolver: H must be symmetric positive definite".into());
                }
            }
        }
        Factor::Lower(
            input
                .cholesky()
                .ok_or("mpcActiveSetSolver: H must be symmetric positive definite")?
                .l(),
        )
    } else {
        for j in 0..n {
            if input[(j, j)] <= 0. || (0..j).any(|i| input[(i, j)] != 0.) {
                return Err(
                    "mpcActiveSetSolver: Linv must be lower triangular with positive diagonal"
                        .into(),
                );
            }
        }
        Factor::InverseLower(input)
    };
    let y0 = -factor.whiten(&DVector::from_column_slice(f));
    let mut rows = Vec::with_capacity(p + m);
    for (a, b) in [(a_eq, b_eq), (a_ineq, b_ineq)] {
        for (i, &bound) in b.iter().enumerate() {
            cancelled(opts)?;
            let v = DVector::from_iterator(n, (0..n).map(|j| a.data()[j * a.rows() + i]));
            let v = factor.whiten(&v);
            let scale = v.norm();
            if !scale.is_finite() || !bound.is_finite() {
                return Err("mpcActiveSetSolver: constraint scaling overflow".into());
            }
            rows.push(Row {
                normal: if scale > 0. { v / scale } else { v },
                bound: if scale > 0. { bound / scale } else { bound },
                scale,
            });
        }
    }
    let mut state = State {
        y: y0.clone(),
        working: Vec::new(),
        u: DVector::zeros(0),
        iterations: 0,
    };
    let status = optimize(&rows, p, warm_active, &y0, opts, &mut state)?;
    let mut x = factor.unwhiten(&state.y);
    let status = if x.iter().all(|x| x.is_finite()) {
        status
    } else {
        x.fill(0.);
        QpStatus::Unreliable
    };
    let mut result = QpResult {
        x: x.iter().copied().collect(),
        status,
        iterations: state.iterations,
        active: vec![false; m],
        lambda: QpLambda {
            ineqlin: vec![0.; m],
            eqlin: vec![0.; p],
        },
    };
    for (&index, &u) in state.working.iter().zip(state.u.iter()) {
        if index < p {
            result.lambda.eqlin[index] = u / rows[index].scale;
        } else {
            result.active[index - p] = true;
            result.lambda.ineqlin[index - p] = u / rows[index].scale;
        }
    }
    Ok(result)
}

struct State {
    y: DVector<f64>,
    working: Vec<usize>,
    u: DVector<f64>,
    iterations: usize,
}
fn optimize(
    rows: &[Row],
    p: usize,
    warm: &[bool],
    y0: &DVector<f64>,
    opts: &QpOptions<'_>,
    state: &mut State,
) -> Result<QpStatus> {
    let tol = opts.constraint_tolerance;
    // First satisfy equalities, detecting inconsistent dependent rows. A dual
    // method needs no primal phase 1; infeasibility is certified by a violated
    // row with no primal direction and no permissible dual blocking step.
    for index in 0..p {
        cancelled(opts)?;
        let basis = Basis::new(rows, &state.working).expect("independent basis");
        if basis.project(&rows[index].normal).0.norm() <= RANK_TOL {
            if rows[index].residual(&state.y).abs() > tol {
                return Ok(QpStatus::Infeasible);
            }
            continue;
        }
        state.working.push(index);
        let Some(basis) = Basis::new(rows, &state.working) else {
            return Ok(QpStatus::Unreliable);
        };
        (state.y, state.u) = basis.optimum(rows, &state.working, y0);
    }
    // A guessed working set is not a feasibility requirement. Keep independent
    // guesses, and discard negative multipliers until dual feasibility holds.
    let mut warm_basis = Basis::new(rows, &state.working).expect("independent basis");
    for (i, &active) in warm.iter().enumerate() {
        cancelled(opts)?;
        if warm_basis.q.len() == y0.len() {
            break;
        }
        if active && warm_basis.project(&rows[p + i].normal).0.norm() > RANK_TOL {
            state.working.push(p + i);
            let Some(basis) = Basis::new(rows, &state.working) else {
                return Ok(QpStatus::Unreliable);
            };
            warm_basis = basis;
        }
    }
    loop {
        cancelled(opts)?;
        let Some(basis) = Basis::new(rows, &state.working) else {
            return Ok(QpStatus::Unreliable);
        };
        (state.y, state.u) = basis.optimum(rows, &state.working, y0);
        let drop = state
            .working
            .iter()
            .enumerate()
            .filter(|&(i, &k)| k >= p && state.u[i] < 0.)
            .min_by(|&(i, _), &(j, _)| state.u[i].total_cmp(&state.u[j]))
            .map(|(i, _)| i);
        if let Some(i) = drop {
            state.working.remove(i);
        } else {
            break;
        }
        state.iterations += 1;
        if state.iterations >= opts.max_iterations {
            return Ok(QpStatus::MaxIterations);
        }
    }
    loop {
        cancelled(opts)?;
        if state.iterations >= opts.max_iterations {
            return Ok(QpStatus::MaxIterations);
        }
        state.iterations += 1;
        if !state.y.iter().chain(state.u.iter()).all(|x| x.is_finite()) {
            return Ok(QpStatus::Unreliable);
        }
        let candidate = (p..rows.len())
            .filter(|k| !state.working.contains(k))
            .map(|k| (k, rows[k].residual(&state.y)))
            .filter(|&(_, v)| v > tol)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k);
        let Some(candidate) = candidate else {
            // IntegrityChecks adds a final equality residual diagnostic. Basic
            // input safety and primal inequality checks are always retained.
            if rows[p..].iter().any(|row| row.residual(&state.y) > tol)
                || (opts.integrity_checks && (0..p).any(|i| rows[i].residual(&state.y).abs() > tol))
            {
                return Ok(QpStatus::Unreliable);
            }
            return Ok(QpStatus::Optimal);
        };
        if rows[candidate].scale == 0. {
            return Ok(QpStatus::Infeasible);
        }
        let mut candidate_u = 0.;
        loop {
            cancelled(opts)?;
            let Some(basis) = Basis::new(rows, &state.working) else {
                return Ok(QpStatus::Unreliable);
            };
            let (z, v) = basis.project(&rows[candidate].normal);
            let r = basis.representation(&v);
            let curvature = z.norm_squared();
            let full = if curvature > RANK_TOL * RANK_TOL {
                ((rows[candidate].normal.dot(&state.y) - rows[candidate].bound) / curvature).max(0.)
            } else {
                f64::INFINITY
            };
            let blocker = state
                .working
                .iter()
                .enumerate()
                .filter(|&(i, &k)| k >= p && r[i] > RANK_TOL)
                .map(|(i, _)| (i, (state.u[i] / r[i]).max(0.)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let partial = blocker.map_or(f64::INFINITY, |(_, t)| t);
            let step = full.min(partial);
            if !step.is_finite() {
                return Ok(QpStatus::Infeasible);
            }
            state.y.axpy(-step, &z, 1.);
            state.u.axpy(-step, &r, 1.);
            candidate_u += step;
            if full <= partial {
                state.working.push(candidate);
                state.u = state.u.clone().insert_row(state.u.len(), candidate_u);
                break;
            }
            let i = blocker.expect("finite blocking step").0;
            state.working.remove(i);
            state.u = state.u.clone().remove_row(i);
            if state.iterations >= opts.max_iterations {
                return Ok(QpStatus::MaxIterations);
            }
            state.iterations += 1;
        }
    }
}
