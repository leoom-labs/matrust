//! Iterative linear solvers: pcg, gmres, bicg, bicgstab, bicgstabl, cgs, qmr,
//! tfqmr, minres, symmlq and lsqr. Outputs include flags 0-5, relative
//! residuals, iteration counts and residual histories. Fewer than two
//! outputs also report status in the console. Restart and stagnation
//! checks can trigger additional iterations.
//!
//! A is a dense matrix or a function handle (`afun(x)`; bicg, qmr and lsqr
//! call `afun(x,'notransp')` and `afun(x,'transp')`), as are the
//! preconditioners M1 and M2. Extra arguments after x0 are passed on to the
//! function handles.
//!
//! Shared dense kernels accumulate `A*x` column by column with fused
//! multiply-add, use sequential dot products and the scaled BLAS dnrm2
//! norm. Elementwise updates round each operation. The public output corpus
//! is `tests/fixtures/fn/solvers-expected.txt`.
//!
//! PCG follows Barrett et al., Templates for the Solution of Linear Systems
//! (1994), section 2.3.1. MINRES and SYMMLQ follow Paige and Saunders,
//! Solution of sparse indefinite systems of linear equations (1975), sections
//! 3, 5 and 6. LSQR follows Paige and Saunders, LSQR (1982), sections 3--6.
//! CGS, QMR and TFQMR use the published recurrences cited at their definitions.
use super::{Engine, Execution, Value};
use crate::engine::Fma;
use crate::matrix::MAX_ELEMENTS;
use crate::{Array, FunctionHandle, Matrix, Result};

const EPS: f64 = f64::EPSILON;
/// Stagnation steps before flag 3, as in the reference.
const MAX_STAG_STEPS: usize = 3;

pub(super) fn slv_is_solver(name: &str) -> bool {
    matches!(
        name,
        "pcg"
            | "gmres"
            | "bicg"
            | "bicgstab"
            | "bicgstabl"
            | "cgs"
            | "qmr"
            | "tfqmr"
            | "minres"
            | "symmlq"
            | "lsqr"
    )
}

fn slv_max_outputs(name: &str) -> usize {
    if matches!(name, "minres" | "symmlq" | "lsqr") {
        6
    } else {
        5
    }
}

/// Solvers that call A and the preconditioners with 'notransp'/'transp'.
fn slv_two_way(name: &str) -> bool {
    matches!(name, "bicg" | "qmr" | "lsqr")
}

// ------------------------------------------------------------------ kernels

fn slv_dot(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0.;
    for (x, y) in a.iter().zip(b) {
        s += x * y;
    }
    s
}

/// The reference BLAS dnrm2 (scale and sum of squares).
fn slv_norm(a: &[f64]) -> f64 {
    let mut scale = 0f64;
    let mut ssq = 1f64;
    for &x in a {
        if x != 0. {
            let ax = x.abs();
            if scale < ax {
                let q = scale / ax;
                ssq = 1. + ssq * (q * q);
                scale = ax;
            } else {
                let q = ax / scale;
                ssq += q * q;
            }
        }
    }
    scale * ssq.sqrt()
}

fn slv_norm2(a: f64, b: f64) -> f64 {
    slv_norm(&[a, b])
}

/// `y + a*x`, each operation rounded.
fn slv_axpy(a: f64, x: &[f64], y: &[f64]) -> Vec<f64> {
    y.iter().zip(x).map(|(yi, xi)| yi + a * xi).collect()
}

fn slv_sub(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x - y).collect()
}

fn slv_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

fn slv_scale(a: f64, x: &[f64]) -> Vec<f64> {
    x.iter().map(|v| a * v).collect()
}

fn slv_div(x: &[f64], d: f64) -> Vec<f64> {
    x.iter().map(|v| v / d).collect()
}

fn slv_finite(x: &[f64]) -> bool {
    x.iter().all(|v| v.is_finite())
}

/// Dense column-major `A*x` (`transp`: `A'*x`), fused multiply-add per column.
fn slv_gemv(rows: usize, cols: usize, data: &[f64], x: &[f64], transp: bool) -> Vec<f64> {
    if transp {
        (0..cols)
            .map(|i| {
                let column = &data[i * rows..(i + 1) * rows];
                let mut s = 0f64;
                for (a, v) in column.iter().zip(x) {
                    s = a.fma(*v, s);
                }
                s
            })
            .collect()
    } else {
        let mut y = vec![0.; rows];
        for (j, column) in data.chunks_exact(rows.max(1)).enumerate().take(cols) {
            let xj = x[j];
            for (yi, a) in y.iter_mut().zip(column) {
                *yi = a.fma(xj, *yi);
            }
        }
        y
    }
}

/// `M\x` (`transp`: `M'\x`) for a square dense M: substitution when M is
/// triangular (diagonal included), as MATLAB's mldivide does, else LU.
fn slv_mldivide(n: usize, data: &[f64], x: &[f64], transp: bool) -> Result<Vec<f64>> {
    let at = |i: usize, j: usize| {
        if transp {
            data[i * n + j]
        } else {
            data[j * n + i]
        }
    };
    let upper = (0..n).all(|j| (j + 1..n).all(|i| at(i, j) == 0.));
    let lower = (0..n).all(|j| (0..j).all(|i| at(i, j) == 0.));
    let mut y = x.to_vec();
    if upper {
        for j in (0..n).rev() {
            y[j] /= at(j, j);
            let yj = y[j];
            for (i, yi) in y.iter_mut().enumerate().take(j) {
                *yi -= yj * at(i, j);
            }
        }
        return Ok(y);
    }
    if lower {
        for j in 0..n {
            y[j] /= at(j, j);
            let yj = y[j];
            for (i, yi) in y.iter_mut().enumerate().skip(j + 1) {
                *yi -= yj * at(i, j);
            }
        }
        return Ok(y);
    }
    let mut m = Matrix::from_column_slice(n, n, data)?;
    if transp {
        m = m.transpose();
    }
    if !slv_finite(data) || !slv_finite(x) {
        return Ok(vec![f64::NAN; n]);
    }
    match m.solve(&Matrix::from_column_slice(n, 1, x)?) {
        Ok(s) => Ok(s.data().to_vec()),
        Err(_) => Ok(vec![f64::INFINITY; n]),
    }
}

/// Back substitution `R(1:k,1:k) \ w(1:k)` on gmres's column-major R.
fn slv_triu_solve(r: &[Vec<f64>], w: &[f64], k: usize) -> Vec<f64> {
    let mut y = w[..k].to_vec();
    for j in (0..k).rev() {
        y[j] /= r[j][j];
        let yj = y[j];
        for (i, yi) in y.iter_mut().enumerate().take(j) {
            *yi -= yj * r[j][i];
        }
    }
    y
}

/// Householder reflector `P = I - 2*w*w'` with `w[..j] = 0` that maps `z` to
/// `(z[..j], pivot, 0, ...)`, the pivot's sign opposite to `z[j]` (Saad 2003,
/// §6.5.2). `w` is zero when `z[j..]` already vanishes.
fn slv_house(z: &[f64], j: usize) -> (Vec<f64>, f64) {
    let sigma = slv_norm(&z[j..]);
    let pivot = if z[j] > 0. { -sigma } else { sigma };
    let mut w = vec![0.; z.len()];
    w[j] = z[j] - pivot;
    w[j + 1..].copy_from_slice(&z[j + 1..]);
    let length = slv_norm(&w[j..]);
    if length != 0. {
        for v in &mut w[j..] {
            *v /= length;
        }
    }
    (w, pivot)
}

/// `x = P*x` for the reflector `w` whose first `j` entries are zero.
fn slv_reflect(w: &[f64], j: usize, x: &mut [f64]) {
    let d = 2. * slv_dot(&w[j..], &x[j..]);
    for (xi, wi) in x[j..].iter_mut().zip(&w[j..]) {
        *xi -= d * wi;
    }
}

/// `x + V_k*y` with `y = R(1:k,1:k) \ g(1:k)` and the Arnoldi basis
/// `V_k e_j = P_0 ... P_j e_j` applied without forming it:
/// `V_k*y = P_0 (y_0 e_0 + P_1 (y_1 e_1 + ... + P_(k-1) y_(k-1) e_(k-1)))`
/// (Walker, SIAM J. Sci. Stat. Comput. 9, 1988; Saad 2003, §6.5.2).
fn slv_gmres_update(
    x: &[f64],
    ws: &[Vec<f64>],
    rcols: &[Vec<f64>],
    g: &[f64],
    k: usize,
) -> Vec<f64> {
    if k == 0 {
        return x.to_vec();
    }
    let y = slv_triu_solve(rcols, g, k);
    let mut u = vec![0.; x.len()];
    for j in (0..k).rev() {
        u[j] += y[j];
        slv_reflect(&ws[j], j, &mut u);
    }
    slv_add(x, &u)
}

/// The iterate that a breakdown at step `j + 1` of a cycle returns: the latest
/// of `x_j`, `x_(j-1)`, ... that is finite, `x` (the cycle's start) at worst.
/// `resvec` holds the history of the `j` completed steps of the cycle after
/// the entry of `x`; it is cut to the entry of the returned iterate, so that
/// the iterate and its reported residual belong together.
fn slv_gmres_breakdown(
    x: &[f64],
    ws: &[Vec<f64>],
    rcols: &[Vec<f64>],
    g: &[f64],
    j: usize,
    resvec: &mut Vec<f64>,
) -> Vec<f64> {
    for steps in (1..=j).rev() {
        let candidate = slv_gmres_update(x, ws, rcols, g, steps);
        if slv_finite(&candidate) {
            resvec.truncate(resvec.len() - (j - steps));
            return candidate;
        }
    }
    resvec.truncate(resvec.len() - j);
    x.to_vec()
}

/// Elements one gmres cycle holds at its peak: `inner + 1` reflectors of
/// length n, the triangular R (`inner*(inner+1)/2`), the rotations (two per
/// step) and the rotated right-hand side, and eight vectors of length n (the
/// iterate, the preconditioned residual, the basis vector, `A*v` and its
/// reflected copy, the candidate iterate and its two residuals).
fn slv_gmres_workspace(n: usize, inner: usize) -> usize {
    let steps = inner.saturating_add(1);
    n.saturating_mul(steps)
        .saturating_add(inner.saturating_mul(steps) / 2)
        .saturating_add(inner.saturating_mul(3).saturating_add(1))
        .saturating_add(n.saturating_mul(8))
}

// ------------------------------------------------------------------ messages

/// `%0.2g` with ties rounded away from zero, as the reference prints it on Windows.
fn slv_g2(v: f64) -> String {
    if v == 0. {
        return "0".into();
    }
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0. { "Inf".into() } else { "-Inf".into() };
    }
    // The exact decimal expansion of the double, rounded by hand.
    let text = format!("{:.80e}", v.abs());
    let (mantissa, exponent) = text.split_once('e').expect("exponent");
    let mut exponent: i32 = exponent.parse().expect("exponent digits");
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    let mut kept = ((digits[0] - b'0') as u32) * 10 + (digits[1] - b'0') as u32;
    if digits[2] >= b'5' {
        kept += 1;
    }
    if kept >= 100 {
        kept /= 10;
        exponent += 1;
    }
    let sign = if v < 0. { "-" } else { "" };
    let (first, second) = (kept / 10, kept % 10);
    if (-4..2).contains(&exponent) {
        // Fixed notation with 1 - exponent decimals, trailing zeros dropped.
        let value = kept as f64 * 10f64.powi(exponent - 1);
        let decimals = (1 - exponent).max(0) as usize;
        let mut s = format!("{value:.decimals$}");
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        return format!("{sign}{s}");
    }
    let mantissa = if second == 0 {
        format!("{first}")
    } else {
        format!("{first}.{second}")
    };
    let esign = if exponent < 0 { '-' } else { '+' };
    format!("{sign}{mantissa}e{esign}{:02}", exponent.abs())
}

fn slv_count(v: f64) -> String {
    if v.fract() == 0. && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        slv_g2(v)
    }
}

/// What the message names as "iteration ...": one count, or gmres(restart)'s
/// outer and inner iterations.
#[derive(Clone, Copy)]
enum Count {
    One(f64),
    Pair(f64, f64),
}

impl Count {
    fn phrase(self) -> String {
        match self {
            Count::One(i) => format!("iteration {}", slv_count(i)),
            Count::Pair(o, i) => format!(
                "outer iteration {} (inner iteration {})",
                slv_count(o),
                slv_count(i)
            ),
        }
    }
    fn number(self) -> String {
        match self {
            Count::One(i) => slv_count(i),
            Count::Pair(o, i) => format!("{}({})", slv_count(o), slv_count(i)),
        }
    }
}

/// Solver status message. `stopped` is the last loop index, or None if no loop ran.
fn slv_message(
    label: &str,
    tol: f64,
    stopped: Option<Count>,
    flag: u8,
    iter: Count,
    relres: f64,
) -> Result<String> {
    let none = match iter {
        Count::One(i) => i == 0.,
        Count::Pair(o, i) => o == 0. && i == 0.,
    };
    if flag == 0 {
        if relres.is_nan() {
            return Ok(slv_zero_message(label));
        }
        if none {
            return Ok(format!(
                "The initial guess has relative residual {} which is within\nthe desired tolerance {} so {label} returned it without iterating.",
                slv_g2(relres),
                slv_g2(tol)
            ));
        }
        return Ok(format!(
            "{label} converged at {} to a solution with relative residual {}.",
            iter.phrase(),
            slv_g2(relres)
        ));
    }
    let Some(stopped) = stopped else {
        return Err("In 'MATLAB:itermsg:IterationInteger', parameter {1} must be a scalar.".into());
    };
    let reason = match flag {
        1 => "because the maximum number of iterations was reached.",
        2 => "because the system involving the preconditioner was ill conditioned.",
        3 => "because the method stagnated.",
        4 => "because a scalar quantity became too small or too large to continue computing.",
        _ => "because the preconditioner is not symmetric positive definite.",
    };
    Ok(format!(
        "{label} stopped at {} without converging to the desired tolerance {}\n{reason}\nThe iterate returned (number {}) has relative residual {}.",
        stopped.phrase(),
        slv_g2(tol),
        iter.number(),
        slv_g2(relres)
    ))
}

fn slv_small_tol_warning(name: &'static str) -> String {
    if name == "gmres" {
        "Warning: Input tol may not be achievable by GMRES.\n Try to use a bigger tolerance.".into()
    } else {
        format!(
            "Warning: Input tol may not be achievable by {}\n         Try to use a bigger tolerance",
            slv_name(name)
        )
    }
}

fn slv_big_tol_warning(name: &'static str) -> String {
    if name == "gmres" {
        "Warning: Input tol is bigger than 1.\n Try to use a smaller tolerance.".into()
    } else {
        "Warning: Input tol is bigger than 1\n         Try to use a smaller tolerance".into()
    }
}

/// A loop bound `1:v` as a count of iterations.
fn slv_count_of(v: f64) -> usize {
    if v >= 1. {
        v.floor().min(usize::MAX as f64) as usize
    } else {
        0
    }
}

fn slv_zero_message(name: &str) -> String {
    format!(
        "The right hand side vector is all zero so {name}\nreturned an all zero solution without iterating."
    )
}

// ------------------------------------------------------------------ problem

/// A, M1 or M2: a dense matrix or a function.
#[derive(Clone)]
enum Operator {
    Dense {
        rows: usize,
        cols: usize,
        data: Matrix,
    },
    Function(FunctionHandle),
}

struct Problem {
    name: &'static str,
    a: Operator,
    b: Vec<f64>,
    /// Unknowns (columns of A).
    n: usize,
    tol: f64,
    maxit: usize,
    /// maxit as given, for the message and `n - maxit`.
    maxit_value: f64,
    m1: Option<Operator>,
    m2: Option<Operator>,
    x0: Vec<f64>,
    /// LSQR handle setup measures this before discovering the column count.
    initial_residual: Option<Vec<f64>>,
    extra: Vec<Value>,
    warned: bool,
    n2b: f64,
    /// gmres: restarted (a restart other than [] and n), inner and outer counts.
    restarted: bool,
    restart: usize,
    inner: usize,
    outer: usize,
}

/// What a solver returns before the message: `stopped` is the last loop index.
struct Solved {
    x: Vec<f64>,
    flag: u8,
    relres: f64,
    iter: Vec<f64>,
    resvec: Vec<f64>,
    sixth: Vec<f64>,
    stopped: Option<Count>,
}

/// Invalid operator values stop a solve; evaluation errors still propagate.
enum SlvError {
    Evaluation(String),
    InvalidResult,
}

impl From<String> for SlvError {
    fn from(message: String) -> Self {
        Self::Evaluation(message)
    }
}

type SlvResult<T> = std::result::Result<T, SlvError>;

fn slv_checked(values: Vec<f64>, expected: usize) -> SlvResult<Vec<f64>> {
    if values.len() != expected || !slv_finite(&values) {
        Err(SlvError::InvalidResult)
    } else {
        Ok(values)
    }
}

/// Count associated with a history of completed scalar-solver updates.
fn slv_completed(p: &Problem, history_len: usize) -> f64 {
    let steps = history_len.saturating_sub(1);
    match p.name {
        "bicgstab" => steps as f64 / 2.,
        "bicgstabl" => steps as f64 / 4.,
        "tfqmr" => (steps / 2) as f64,
        _ => steps as f64,
    }
}

/// Correct the partial MR point's history only when that point is returned
/// after a failed application. The algorithm's work residual is different.
fn slv_history_with_last(history: &[f64], normr: f64) -> Vec<f64> {
    let mut history = history.to_vec();
    if let Some(last) = history.last_mut() {
        *last = normr;
    }
    history
}

/// Construct the flag-2 result only on failure, without copying an iterate on
/// every successful application or calling the failing operator again.
fn slv_failed(p: &Problem, x: &[f64], history: &[f64], sixth: &[f64], position: Count) -> Solved {
    // The supplied history contains only completed updates to x. An invalid
    // application can occur later, so its loop position belongs only in the
    // stopping message, never in the returned iterate's count.
    let steps = history.len().saturating_sub(1);
    let iter = match p.name {
        "gmres" if steps == 0 => vec![0., 0.],
        "gmres" => vec![
            ((steps - 1) / p.inner.max(1) + 1) as f64,
            ((steps - 1) % p.inner.max(1) + 1) as f64,
        ],
        _ => vec![slv_completed(p, history.len())],
    };
    let stopped = match position {
        Count::Pair(_, inner) if !p.restarted => Count::One(inner),
        _ => position,
    };
    let sixth_len = match p.name {
        "lsqr" => steps,
        "minres" | "symmlq" if steps > 0 => steps + 2,
        _ => 1,
    };
    Solved {
        x: x.to_vec(),
        flag: 2,
        relres: history.last().copied().unwrap_or(p.n2b) / p.n2b,
        iter,
        resvec: history.to_vec(),
        sixth: sixth[..sixth.len().min(sixth_len)].to_vec(),
        stopped: Some(stopped),
    }
}

macro_rules! slv_try {
    ($call:expr, $p:expr, $x:expr, $history:expr, $sixth:expr, $position:expr) => {
        slv_try!($call, $p, $x, $history, $sixth, $p.n2b, false, $position)
    };
    ($call:expr, $p:expr, $x:expr, $history:expr, $sixth:expr, $n2b:expr, $silent:expr, $position:expr) => {
        match $call {
            Ok(value) => value,
            Err(SlvError::Evaluation(error)) => return Err(error),
            Err(SlvError::InvalidResult) => {
                let mut failed = slv_failed($p, &$x, &$history, &$sixth, $position);
                failed.relres = failed.resvec.last().copied().unwrap_or($n2b) / $n2b;
                if $silent {
                    failed.stopped = None;
                }
                return Ok(failed);
            }
        }
    };
}

fn slv_empty(value: &Value) -> bool {
    match value {
        Value::Matrix(m) => m.is_empty(),
        Value::Logical(m) => m.is_empty(),
        Value::Text(t) => t.is_empty(),
        _ => false,
    }
}

fn slv_name(name: &'static str) -> &'static str {
    match name {
        "pcg" => "PCG",
        "gmres" => "GMRES",
        "bicg" => "BICG",
        "bicgstab" => "BICGSTAB",
        "bicgstabl" => "BICGSTABL",
        "cgs" => "CGS",
        "qmr" => "QMR",
        "tfqmr" => "TFQMR",
        "minres" => "MINRES",
        "symmlq" => "SYMMLQ",
        _ => "LSQR",
    }
}

fn slv_static(name: &str) -> &'static str {
    match name {
        "pcg" => "pcg",
        "gmres" => "gmres",
        "bicg" => "bicg",
        "bicgstab" => "bicgstab",
        "bicgstabl" => "bicgstabl",
        "cgs" => "cgs",
        "qmr" => "qmr",
        "tfqmr" => "tfqmr",
        "minres" => "minres",
        "symmlq" => "symmlq",
        _ => "lsqr",
    }
}

fn slv_column(values: Vec<f64>) -> Result<Value> {
    let n = values.len();
    Ok(Value::Matrix(Matrix::from_column_vec(n, 1, values)?))
}

fn slv_row(values: Vec<f64>) -> Result<Value> {
    let n = values.len();
    Ok(Value::Matrix(Matrix::from_column_vec(1, n, values)?))
}

impl Engine {
    /// Single-output calls; `None` when `name` is not an iterative solver.
    pub(super) fn slv_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !slv_is_solver(name) {
            return Ok(None);
        }
        Ok(Some(self.slv_outputs(name, args, 1, out)?.remove(0)))
    }

    /// `[x, flag, relres, iter, resvec, ...] = name(args...)` with `count` outputs.
    pub(super) fn slv_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let name = slv_static(name);
        if count > slv_max_outputs(name) {
            return Err("Too many output arguments.".into());
        }
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let solved = match self.slv_setup(name, args, out)? {
            Ok(problem) => {
                let solved = match name {
                    "pcg" => self.slv_pcg(&problem, out)?,
                    "bicg" => self.slv_bicg(&problem, out)?,
                    "cgs" => self.slv_cgs(&problem, out)?,
                    "bicgstab" => self.slv_bicgstab(&problem, out)?,
                    "bicgstabl" => self.slv_bicgstabl(&problem, out)?,
                    "qmr" => self.slv_qmr(&problem, out)?,
                    "tfqmr" => self.slv_tfqmr(&problem, out)?,
                    "minres" => self.slv_minres(&problem, out)?,
                    "symmlq" => self.slv_symmlq(&problem, out)?,
                    "lsqr" => self.slv_lsqr(&problem, out)?,
                    _ => self.slv_gmres(&problem, out)?,
                };
                // gmres returns silently when the preconditioned residual is not finite.
                let silent = solved.flag == 2 && solved.stopped.is_none() && name == "gmres";
                if count < 2 && !silent {
                    let text = if name == "lsqr" && solved.flag == 0 && solved.stopped.is_none() {
                        slv_zero_message(name)
                    } else if name == "gmres"
                        && problem.restarted
                        && !(solved.flag == 0 && solved.iter == [0., 0.])
                    {
                        slv_message(
                            &format!("gmres({})", problem.restart),
                            problem.tol,
                            solved.stopped,
                            solved.flag,
                            Count::Pair(solved.iter[0], solved.iter[1]),
                            solved.relres,
                        )?
                    } else if name == "gmres" {
                        slv_message(
                            name,
                            problem.tol,
                            solved.stopped,
                            solved.flag,
                            Count::One(solved.iter[1]),
                            solved.relres,
                        )?
                    } else {
                        slv_message(
                            name,
                            problem.tol,
                            solved.stopped,
                            solved.flag,
                            Count::One(solved.iter[0]),
                            solved.relres,
                        )?
                    };
                    self.emit(out, text)?;
                }
                solved
            }
            Err((zero, tol)) => {
                if count < 2 {
                    let text = if zero.flag == 2 {
                        slv_message(name, tol, zero.stopped, 2, Count::One(0.), zero.relres)?
                    } else {
                        slv_zero_message(name)
                    };
                    self.emit(out, text)?;
                }
                zero
            }
        };
        self.slv_pack(solved, count)
    }

    fn slv_pack(&mut self, solved: Solved, count: usize) -> Result<Vec<Value>> {
        let mut outputs = vec![slv_column(solved.x)?];
        if count >= 2 {
            outputs.push(Value::Matrix(Matrix::scalar(solved.flag as f64)));
        }
        if count >= 3 {
            outputs.push(Value::Matrix(Matrix::scalar(solved.relres)));
        }
        if count >= 4 {
            outputs.push(slv_row(solved.iter)?);
        }
        if count >= 5 {
            outputs.push(slv_column(solved.resvec)?);
        }
        if count >= 6 {
            outputs.push(slv_column(solved.sixth)?);
        }
        self.charge_function_outputs(&outputs)?;
        self.check_cancel()?;
        Ok(outputs)
    }

    fn slv_operator(&self, value: &Value, what: &str) -> Result<Operator> {
        match value {
            Value::Container(Array::Handle(h)) => Ok(Operator::Function(h.clone())),
            Value::Text(t) => match self.named_handle(t.trim())? {
                Value::Container(Array::Handle(h)) => Ok(Operator::Function(h)),
                _ => Err(format!("{what} must be a matrix or a function handle.")),
            },
            other => {
                let m = self.slv_real(other, what)?;
                Ok(Operator::Dense {
                    rows: m.rows(),
                    cols: m.cols(),
                    data: m,
                })
            }
        }
    }

    fn slv_real(&self, value: &Value, what: &str) -> Result<Matrix> {
        match value {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(m) => m.to_numeric(&self.cancel),
            Value::Container(Array::Character(c)) if c.is_text_row() => Err(format!(
                "{what} must be a numeric matrix or a function handle."
            )),
            Value::Complex(_) => Err(format!("{what}: complex systems are not supported yet.")),
            _ => Err(format!("{what} must be a double matrix.")),
        }
    }

    fn slv_scalar(&self, value: &Value, what: &str) -> Result<f64> {
        let m = self.slv_real(value, what)?;
        if m.len() != 1 {
            return Err(format!("{what} must be a scalar."));
        }
        Ok(m.data()[0])
    }

    /// The reference's input checks in its order; `Err(zero)` for a right-hand side
    /// whose norm is zero. Infinite norms reach the initial-residual test.
    fn slv_setup(
        &mut self,
        name: &'static str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<std::result::Result<Problem, (Solved, f64)>> {
        let a = self.slv_operator(&args[0], "A")?;
        let b = self.slv_real(&args[1], "B")?;
        let (m, n) = match &a {
            Operator::Dense { rows, cols, .. } => {
                if name != "lsqr" && rows != cols {
                    return Err("Matrix must be square.".into());
                }
                if b.rows() != *rows || b.cols() != 1 {
                    return Err(if name == "lsqr" {
                        format!("Right hand side must be a column vector of length {rows}.")
                    } else {
                        format!(
                            "Right hand side must be a column vector of length {rows} to match the coefficient matrix."
                        )
                    });
                }
                (*rows, *cols)
            }
            Operator::Function(_) => {
                if b.cols() != 1 {
                    return Err("Right hand side must be a column vector.".into());
                }
                (b.rows(), b.rows())
            }
        };
        if m.max(n).saturating_mul(16) > MAX_ELEMENTS {
            return Err("Iterative solver problem exceeds the element limit.".into());
        }
        let mut problem = Problem {
            name,
            a,
            b: b.data().to_vec(),
            n,
            tol: 1e-6,
            maxit: 0,
            maxit_value: 0.,
            m1: None,
            m2: None,
            x0: Vec::new(),
            initial_residual: None,
            extra: Vec::new(),
            warned: false,
            n2b: 0.,
            restarted: false,
            restart: 0,
            inner: 0,
            outer: 0,
        };
        // gmres(A, b, restart, tol, maxit, M1, M2, x0, ...) shifts the others.
        let off = usize::from(name == "gmres");
        let mut restart = None;
        if off == 1
            && let Some(v) = args.get(2).filter(|v| !slv_empty(v))
        {
            restart = Some(self.slv_scalar(v, "RESTART")?);
        }
        problem.restarted = restart.is_some_and(|r| r != n as f64);
        if let Some(t) = args.get(2 + off).filter(|v| !slv_empty(v)) {
            problem.tol = self.slv_scalar(t, "TOL")?;
        }
        let small = if name == "pcg" {
            problem.tol <= EPS
        } else {
            problem.tol < EPS
        };
        if small {
            self.emit(out, slv_small_tol_warning(name))?;
            problem.warned = true;
            problem.tol = EPS;
        } else if problem.tol >= 1. {
            self.emit(out, slv_big_tol_warning(name))?;
            problem.warned = true;
            problem.tol = 1. - EPS;
        }
        let given_maxit = args.get(3 + off).filter(|v| !slv_empty(v));
        let mut maxit = match given_maxit {
            Some(v) => self.slv_scalar(v, "MAXIT")?,
            None => match name {
                "lsqr" => m.min(n).min(20) as f64,
                "gmres" if problem.restarted => (n as f64 / restart.unwrap_or(1.)).ceil().min(10.),
                "gmres" => n.min(10) as f64,
                _ => n.min(20) as f64,
            },
        };
        if name == "gmres" {
            if problem.restarted {
                let mut r = restart.unwrap_or(0.);
                if r > n as f64 {
                    self.emit(
                        out,
                        format!(
                            "Warning: RESTART is {} but it should be bounded by SIZE(A,1).\n  Setting RESTART to {n}.",
                            slv_count(r)
                        ),
                    )?;
                    r = n as f64;
                }
                problem.outer = slv_count_of(maxit);
                problem.restart = slv_count_of(r);
                problem.inner = problem.restart;
            } else {
                if maxit > n as f64 {
                    self.emit(
                        out,
                        format!(
                            "Warning: MAXIT is {} but it should be bounded by SIZE(A,1).\n Setting MAXIT to {n}.",
                            slv_count(maxit)
                        ),
                    )?;
                    maxit = n as f64;
                }
                problem.outer = 1;
                problem.inner = slv_count_of(maxit);
            }
        }
        problem.maxit_value = maxit;
        problem.maxit = slv_count_of(maxit);
        problem.n2b = slv_norm(&problem.b);
        if problem.n2b == 0. {
            let n = problem.n;
            let sixth = if name == "lsqr" { Vec::new() } else { vec![0.] };
            return Ok(Err((
                Solved {
                    x: vec![0.; n],
                    flag: 0,
                    relres: 0.,
                    iter: if name == "gmres" {
                        vec![0., 0.]
                    } else {
                        vec![0.]
                    },
                    resvec: vec![0.],
                    sixth,
                    stopped: Some(Count::One(0.)),
                },
                problem.tol,
            )));
        }
        // Parameters are needed by the adjoint size probe as well.
        if args.len() > 7 + off {
            problem.extra = args[7 + off..].to_vec();
        }
        let handle_lsqr = name == "lsqr" && matches!(problem.a, Operator::Function(_));
        let mut probe_normr = problem.n2b;
        if handle_lsqr {
            // X0's column length is not known yet. Its residual is still in
            // the row space and can be measured before a failed adjoint probe.
            if let Some(x0) = args.get(6).filter(|v| !slv_empty(v)) {
                let x0 = self.slv_real(x0, "X0")?;
                if x0.cols() != 1 {
                    return Err("Initial guess must be a column vector.".into());
                }
                if x0.len().saturating_mul(16) > MAX_ELEMENTS {
                    return Err("Iterative solver problem exceeds the element limit.".into());
                }
                problem.x0 = x0.data().to_vec();
                probe_normr = match self.slv_residual(&problem, &problem.x0, out) {
                    Ok(r) => {
                        let normr = slv_norm(&r);
                        problem.initial_residual = Some(r);
                        normr
                    }
                    Err(SlvError::Evaluation(error)) => return Err(error),
                    Err(SlvError::InvalidResult) => {
                        let failed =
                            slv_failed(&problem, &problem.x0, &[f64::NAN], &[], Count::One(0.));
                        return Ok(Err((failed, problem.tol)));
                    }
                };
            }
            if let Operator::Function(h) = &problem.a {
                problem.n = match self.slv_function(&problem, h, &problem.b, true, None, out) {
                    Ok(v) => v.len(),
                    Err(SlvError::Evaluation(error)) => return Err(error),
                    Err(SlvError::InvalidResult) => {
                        if problem.x0.is_empty() {
                            problem.x0 = vec![0.; problem.n];
                        }
                        let failed =
                            slv_failed(&problem, &problem.x0, &[probe_normr], &[], Count::One(0.));
                        return Ok(Err((failed, problem.tol)));
                    }
                };
            }
            if problem.n.saturating_mul(16) > MAX_ELEMENTS {
                return Err("Iterative solver problem exceeds the element limit.".into());
            }
            if given_maxit.is_none() {
                problem.maxit = m.min(problem.n).min(20);
                problem.maxit_value = problem.maxit as f64;
            }
        }
        let size = if name == "lsqr" { problem.n } else { m };
        problem.m1 = self.slv_preconditioner(args.get(4 + off), size)?;
        problem.m2 = self.slv_preconditioner(args.get(5 + off), size)?;
        if let Some(x0) = args.get(6 + off).filter(|v| !slv_empty(v)) {
            let x0 = self.slv_real(x0, "X0")?;
            if x0.rows() != problem.n || x0.cols() != 1 {
                return Err(format!(
                    "Initial guess must be a column vector of length {} to match the problem size.",
                    problem.n
                ));
            }
            problem.x0 = x0.data().to_vec();
        } else {
            problem.x0 = vec![0.; problem.n];
        }
        if !problem.extra.is_empty()
            && matches!(problem.a, Operator::Dense { .. })
            && !matches!(problem.m1, Some(Operator::Function(_)))
            && !matches!(problem.m2, Some(Operator::Function(_)))
        {
            return Err("Too many input arguments.".into());
        }
        Ok(Ok(problem))
    }

    fn slv_preconditioner(&self, value: Option<&Value>, n: usize) -> Result<Option<Operator>> {
        let Some(value) = value.filter(|v| !slv_empty(v)) else {
            return Ok(None);
        };
        let op = self.slv_operator(value, "M")?;
        if let Operator::Dense { rows, cols, .. } = &op
            && (*rows != n || *cols != n)
        {
            return Err(format!(
                "Preconditioner must be a square matrix of size {n} to match the problem size."
            ));
        }
        Ok(Some(op))
    }

    /// `A*x` (`transp`: `A'*x`) or `afun(x, extra...[, 'notransp'|'transp'])`.
    fn slv_apply(
        &mut self,
        p: &Problem,
        op: &Operator,
        x: &[f64],
        transp: bool,
        out: &mut Execution,
    ) -> SlvResult<Vec<f64>> {
        self.check_cancel()?;
        match op {
            Operator::Dense { rows, cols, data } => slv_checked(
                slv_gemv(*rows, *cols, data.data(), x, transp),
                if transp { *cols } else { *rows },
            ),
            Operator::Function(h) => {
                // lsqr's A maps n unknowns to m equations and back.
                let expected = match (p.name == "lsqr", transp) {
                    (false, _) => x.len(),
                    (true, true) => p.n,
                    (true, false) => p.b.len(),
                };
                self.slv_function(p, h, x, transp, Some(expected), out)
            }
        }
    }

    /// Calls a user function; `expected` is the length its result must have.
    fn slv_function(
        &mut self,
        p: &Problem,
        h: &FunctionHandle,
        x: &[f64],
        transp: bool,
        expected: Option<usize>,
        out: &mut Execution,
    ) -> SlvResult<Vec<f64>> {
        self.charge(x.len())?;
        let mut args = vec![slv_column(x.to_vec())?];
        args.extend(p.extra.iter().cloned());
        if slv_two_way(p.name) {
            args.push(Value::Text(
                if transp { "transp" } else { "notransp" }.into(),
            ));
        }
        let wrap = |message: String| {
            let message = match message.split_once(": ") {
                Some((at, rest)) if at.starts_with("Line ") && at.contains(", column ") => {
                    rest.to_string()
                }
                _ => message,
            };
            format!(
                "user supplied function ==> {} failed with the following error:\n \n {message}",
                h.func2str()
            )
        };
        let value = self
            .call_handle(h, args, 1, None, out)
            .map_err(wrap)?
            .into_iter()
            .next()
            .ok_or_else(|| wrap("The function must return a value.".into()))?;
        let m = match value {
            Value::Matrix(m) => m,
            Value::Logical(m) => m.to_numeric(&self.cancel)?,
            _ => return Err(SlvError::InvalidResult),
        };
        if m.cols() != 1
            || expected.is_some_and(|length| m.rows() != length)
            || !slv_finite(m.data())
        {
            return Err(SlvError::InvalidResult);
        }
        Ok(m.data().to_vec())
    }

    /// `M\x` (`transp`: `M'\x`) or `mfun(x[, mode], extra...)`.
    fn slv_precondition(
        &mut self,
        p: &Problem,
        m: &Operator,
        x: &[f64],
        transp: bool,
        out: &mut Execution,
    ) -> SlvResult<Vec<f64>> {
        self.check_cancel()?;
        match m {
            Operator::Dense { rows, data, .. } => {
                slv_checked(slv_mldivide(*rows, data.data(), x, transp)?, *rows)
            }
            Operator::Function(h) => self.slv_function(p, h, x, transp, Some(x.len()), out),
        }
    }

    /// `M2\(M1\x)`, either factor optional.
    fn slv_both(&mut self, p: &Problem, x: &[f64], out: &mut Execution) -> SlvResult<Vec<f64>> {
        let mut y = x.to_vec();
        if let Some(m1) = &p.m1 {
            y = self.slv_precondition(p, m1, &y, false, out)?;
        }
        if let Some(m2) = &p.m2 {
            y = self.slv_precondition(p, m2, &y, false, out)?;
        }
        Ok(y)
    }

    fn slv_residual(&mut self, p: &Problem, x: &[f64], out: &mut Execution) -> SlvResult<Vec<f64>> {
        let ax = self.slv_apply(p, &p.a, x, false, out)?;
        Ok(slv_sub(&p.b, &ax))
    }

    fn slv_mul(&mut self, p: &Problem, x: &[f64], out: &mut Execution) -> SlvResult<Vec<f64>> {
        self.slv_apply(p, &p.a, x, false, out)
    }

    fn slv_mul_t(&mut self, p: &Problem, x: &[f64], out: &mut Execution) -> SlvResult<Vec<f64>> {
        self.slv_apply(p, &p.a, x, true, out)
    }

    fn slv_warn_small(&mut self, p: &Problem, out: &mut Execution) -> Result<()> {
        if p.warned {
            return Ok(());
        }
        self.emit(out, slv_small_tol_warning(p.name))
    }

    /// The good-initial-guess return.
    fn slv_initial(p: &Problem, x: Vec<f64>, normr: f64) -> Solved {
        Solved {
            x,
            flag: 0,
            relres: normr / p.n2b,
            iter: vec![0.],
            resvec: vec![normr],
            sixth: vec![normr],
            stopped: Some(Count::One(0.)),
        }
    }

    /// Start of one iteration: cancellation, and the statement's evaluation
    /// budget for the vectors it creates (at least 16 elements, so a tiny
    /// system with NaN data cannot iterate without end).
    fn slv_step(&mut self, elements: usize) -> Result<()> {
        self.check_cancel()?;
        self.charge(elements.max(16))
    }

    fn slv_maxmsteps(p: &Problem) -> f64 {
        ((p.n / 50) as f64).min(5.).min(p.n as f64 - p.maxit_value)
    }

    /// The common end of the pcg family: the minimal-residual iterate unless
    /// the method converged.
    #[allow(clippy::too_many_arguments)]
    fn slv_finish(
        &mut self,
        p: &Problem,
        flag: u8,
        x: &[f64],
        xmin: &[f64],
        imin: f64,
        ii: f64,
        normr_act: f64,
        out: &mut Execution,
    ) -> SlvResult<(Vec<f64>, f64, f64)> {
        if flag == 0 {
            return Ok((x.to_vec(), ii, normr_act / p.n2b));
        }
        let r = self.slv_residual(p, xmin, out)?;
        let nr = slv_norm(&r);
        if nr <= normr_act {
            Ok((xmin.to_vec(), imin, nr / p.n2b))
        } else {
            Ok((x.to_vec(), ii, normr_act / p.n2b))
        }
    }
}

/// Shared state of the "check convergence with the true residual, then take
/// a few more steps" logic of the reference solvers.
struct Monitor {
    tolb: f64,
    stag: usize,
    moresteps: f64,
    maxmsteps: f64,
    normr_act: f64,
    normrmin: f64,
}

enum Check {
    Converged,
    Stagnated,
    Continue,
}

impl Engine {
    /// After an update with recursive residual norm `normr`: the reference's
    /// convergence test, true residual, more steps and stagnation logic.
    /// Returns the residual norm to store in resvec and the outcome.
    fn slv_check(
        &mut self,
        p: &Problem,
        mon: &mut Monitor,
        x: &[f64],
        normr: f64,
        out: &mut Execution,
    ) -> SlvResult<(f64, Check, Option<Vec<f64>>)> {
        mon.normr_act = normr;
        if normr <= mon.tolb || mon.stag >= MAX_STAG_STEPS || mon.moresteps > 0. {
            let r = self.slv_residual(p, x, out)?;
            mon.normr_act = slv_norm(&r);
            if mon.normr_act <= mon.tolb {
                return Ok((mon.normr_act, Check::Converged, Some(r)));
            }
            if mon.stag >= MAX_STAG_STEPS && mon.moresteps == 0. {
                mon.stag = 0;
            }
            mon.moresteps += 1.;
            if mon.moresteps >= mon.maxmsteps {
                self.slv_warn_small(p, out)?;
                return Ok((mon.normr_act, Check::Stagnated, Some(r)));
            }
            return Ok((mon.normr_act, Check::Continue, Some(r)));
        }
        Ok((normr, Check::Continue, None))
    }

    fn slv_monitor(p: &Problem, normr: f64) -> Monitor {
        Monitor {
            tolb: p.tol * p.n2b,
            stag: 0,
            moresteps: 0.,
            maxmsteps: Self::slv_maxmsteps(p),
            normr_act: normr,
            normrmin: normr,
        }
    }

    // -------------------------------------------------------------- pcg

    /// Barrett et al., Templates for the Solution of Linear Systems (1994),
    /// section 2.3.1, Figure 2.5: preconditioned conjugate gradients.
    fn slv_pcg(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut x = p.x0.clone();
        let mut history = vec![p.n2b];
        let mut r = slv_try!(
            self.slv_residual(p, &x, out),
            p,
            x,
            history,
            [],
            Count::One(0.)
        );
        let initial = slv_norm(&r);
        history[0] = initial;
        if initial <= p.tol * p.n2b {
            return Ok(Self::slv_initial(p, x, initial));
        }
        let mut monitor = Self::slv_monitor(p, initial);
        let mut best = x.clone();
        let mut best_step = 0.;
        let mut direction = vec![0.; p.n];
        let mut previous_rho = 1.;
        let mut flag = 1;
        // A NaN RHS has no meaningful Krylov space. Preserve its initial point
        // and report it as stopped before the first iteration.
        let mut stopped = initial.is_nan().then_some(Count::One(0.));
        if !initial.is_nan() {
            for step in 1..=p.maxit {
                self.slv_step(p.n)?;
                let position = Count::One(step as f64);
                stopped = Some(position);
                let z = slv_try!(self.slv_both(p, &r, out), p, x, history, [], position);
                let rho = slv_dot(&r, &z);
                if rho == 0. || !rho.is_finite() {
                    flag = 4;
                    break;
                }
                if step == 1 {
                    direction = z;
                } else {
                    let beta = rho / previous_rho;
                    if !beta.is_finite() {
                        flag = 4;
                        break;
                    }
                    direction = slv_axpy(beta, &direction, &z);
                }
                let image = slv_try!(
                    self.slv_mul(p, &direction, out),
                    p,
                    x,
                    history,
                    [],
                    position
                );
                let curvature = slv_dot(&direction, &image);
                let alpha = rho / curvature;
                if curvature <= 0. || !curvature.is_finite() || !alpha.is_finite() {
                    flag = 4;
                    break;
                }
                monitor.stag = if alpha.abs() * slv_norm(&direction) < EPS * slv_norm(&x) {
                    monitor.stag + 1
                } else {
                    0
                };
                let next_x = slv_axpy(alpha, &direction, &x);
                let next_r = slv_axpy(-alpha, &image, &r);
                if !slv_finite(&next_x) || !slv_finite(&next_r) {
                    flag = 4;
                    break;
                }
                x = next_x;
                r = next_r;
                history.push(slv_norm(&r));
                let (normr, check, refreshed) = slv_try!(
                    self.slv_check(p, &mut monitor, &x, *history.last().unwrap(), out),
                    p,
                    x,
                    history,
                    [],
                    position
                );
                *history.last_mut().unwrap() = normr;
                if let Some(residual) = refreshed {
                    r = residual;
                }
                if normr < monitor.normrmin {
                    monitor.normrmin = normr;
                    best.clone_from(&x);
                    best_step = step as f64;
                }
                match check {
                    Check::Converged => {
                        flag = 0;
                        break;
                    }
                    Check::Stagnated => {
                        flag = 3;
                        break;
                    }
                    Check::Continue => {}
                }
                if monitor.stag >= MAX_STAG_STEPS {
                    flag = 3;
                    break;
                }
                previous_rho = rho;
            }
        }
        let completed = (history.len() - 1) as f64;
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &best,
                best_step,
                completed,
                monitor.normr_act,
                out
            ),
            p,
            x,
            history,
            [],
            stopped.unwrap_or(Count::One(0.))
        );
        // Without an iteration there is no history and no iteration count.
        if p.maxit == 0 {
            history.clear();
        }
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec: history,
            sixth: Vec::new(),
            stopped,
        })
    }

    // -------------------------------------------------------------- gmres

    /// Restarted GMRES (Saad and Schultz, SIAM J. Sci. Stat. Comput. 7, 1986)
    /// with Householder Arnoldi and Givens rotations (Saad, Iterative Methods
    /// for Sparse Linear Systems, 2nd ed., 2003, §6.5.2 and §6.5.3), left
    /// preconditioned (§9.3.1): the cycle minimises ||M\(b - A*x)||, and
    /// resvec and relres use that norm relative to ||M\b||. resvec holds the
    /// least-squares residual |g(k+1)| of each inner step. When it meets the
    /// tolerance, the iterate is formed and its true residual checked; a true
    /// residual still above the tolerance is recorded instead, and a run that
    /// ends that way has stagnated (flag 3).
    fn slv_gmres(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let n = p.n;
        let (inner, outer) = (p.inner, p.outer);
        let workspace = slv_gmres_workspace(n, inner);
        if workspace > MAX_ELEMENTS {
            return Err("Iterative solver problem exceeds the element limit.".into());
        }
        let mut position = Count::Pair(0., 0.);
        let mut x = p.x0.clone();
        let r = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&r);
        if normr <= p.tol * p.n2b {
            return Ok(Solved {
                x,
                flag: 0,
                relres: normr / p.n2b,
                iter: vec![0., 0.],
                resvec: vec![normr],
                sixth: Vec::new(),
                stopped: Some(Count::One(0.)),
            });
        }
        // A non-finite preconditioned residual returns without a message.
        let mut z = slv_try!(
            self.slv_both(p, &r, out),
            p,
            x,
            [normr],
            [],
            p.n2b,
            true,
            position
        );
        let n2minv_b = if p.m1.is_none() && p.m2.is_none() {
            p.n2b
        } else if x.iter().all(|&v| v == 0.) {
            // r = b - A*0 = b.
            slv_norm(&z)
        } else {
            slv_norm(&slv_try!(
                self.slv_both(p, &p.b, out),
                p,
                x,
                [normr],
                [],
                p.n2b,
                true,
                position
            ))
        };
        if outer == 0 {
            return Err("Undefined function or variable 'initer'.".into());
        }
        if inner == 0 {
            return Err("Matrix dimensions must agree.".into());
        }
        let tolb = p.tol * n2minv_b;
        let mut resvec = vec![slv_norm(&z)];
        let mut flag = 1u8;
        // Whether the last check found the true residual above the tolerance.
        let mut mismatch = false;
        'cycles: for o in 1..=outer {
            self.charge(workspace.max(16))?;
            // Reflectors P_0..P_k, columns of the rotated Hessenberg matrix (R),
            // the rotations and the rotated right-hand side g = Q'*beta*e1.
            let mut ws: Vec<Vec<f64>> = Vec::with_capacity(inner + 1);
            let mut rcols: Vec<Vec<f64>> = Vec::with_capacity(inner);
            let mut rotations: Vec<(f64, f64)> = Vec::with_capacity(inner);
            let (w0, beta) = slv_house(&z, 0);
            ws.push(w0);
            let mut g = vec![beta];
            for k in 1..=inner {
                let j = k - 1;
                position = Count::Pair(o as f64, k as f64);
                // An invalid operator result stops with flag 2 at the latest
                // finite iterate of the j completed steps, its history cut to
                // match.
                macro_rules! apply {
                    ($call:expr) => {
                        match $call {
                            Ok(value) => value,
                            Err(SlvError::Evaluation(error)) => return Err(error),
                            Err(SlvError::InvalidResult) => {
                                let mut history = resvec.clone();
                                let latest =
                                    slv_gmres_breakdown(&x, &ws, &rcols, &g, j, &mut history);
                                let mut failed = slv_failed(p, &latest, &history, &[], position);
                                failed.relres =
                                    history.last().copied().unwrap_or(n2minv_b) / n2minv_b;
                                return Ok(failed);
                            }
                        }
                    };
                }
                self.slv_step(n)?;
                // v_j = P_0 P_1 ... P_j e_j.
                let mut v = vec![0.; n];
                v[j] = 1.;
                for (i, w) in ws.iter().enumerate().rev() {
                    slv_reflect(w, i, &mut v);
                }
                let av = apply!(self.slv_mul(p, &v, out));
                let mut u = apply!(self.slv_both(p, &av, out));
                for (i, w) in ws.iter().enumerate() {
                    slv_reflect(w, i, &mut u);
                }
                // Column j of the Hessenberg matrix; its subdiagonal entry is
                // the next reflector's pivot (zero once the space is exhausted).
                let mut h = u[..=j].to_vec();
                if j + 1 < n {
                    let (w, pivot) = slv_house(&u, j + 1);
                    ws.push(w);
                    h.push(pivot);
                } else {
                    h.push(0.);
                }
                for (i, &(c, s)) in rotations.iter().enumerate() {
                    let top = c * h[i] + s * h[i + 1];
                    h[i + 1] = c * h[i + 1] - s * h[i];
                    h[i] = top;
                }
                let rho = slv_norm2(h[j], h[j + 1]);
                if rho == 0. || !rho.is_finite() {
                    flag = 4;
                    x = slv_gmres_breakdown(&x, &ws, &rcols, &g, j, &mut resvec);
                    break 'cycles;
                }
                let (c, s) = (h[j] / rho, h[j + 1] / rho);
                h[j] = rho;
                h.truncate(k);
                rotations.push((c, s));
                rcols.push(h);
                let gj = g[j];
                g[j] = c * gj;
                g.push(-s * gj);
                let estimate = g[k].abs();
                if estimate > tolb && k < inner {
                    resvec.push(estimate);
                    continue;
                }
                let xk = slv_gmres_update(&x, &ws, &rcols, &g, k);
                if !slv_finite(&xk) {
                    // The triangular solve overflowed.
                    flag = 4;
                    x = slv_gmres_breakdown(&x, &ws, &rcols, &g, j, &mut resvec);
                    break 'cycles;
                }
                if estimate > tolb && o == outer {
                    // The last cycle ends without a check.
                    resvec.push(estimate);
                    x = xk;
                    mismatch = false;
                    break 'cycles;
                }
                let rk = apply!(self.slv_residual(p, &xk, out));
                let zk = apply!(self.slv_both(p, &rk, out));
                let normz = slv_norm(&zk);
                if normz <= tolb {
                    // The least-squares residual that met the tolerance is
                    // reported; a restart check reports the true one.
                    resvec.push(if estimate <= tolb { estimate } else { normz });
                    x = xk;
                    flag = 0;
                    break 'cycles;
                }
                if estimate <= tolb {
                    // Rounding keeps the true residual above the estimate.
                    resvec.push(normz);
                    mismatch = true;
                } else {
                    resvec.push(estimate);
                    mismatch = false;
                }
                if k == inner {
                    // Restart from the formed iterate and its true residual.
                    x = xk;
                    z = zk;
                }
            }
        }
        if flag == 1 && mismatch {
            self.slv_warn_small(p, out)?;
            flag = 3;
        }
        let mut solved = slv_failed(p, &x, &resvec, &[], position);
        solved.flag = flag;
        solved.relres = resvec.last().copied().unwrap_or(n2minv_b) / n2minv_b;
        Ok(solved)
    }

    // -------------------------------------------------------------- bicg

    /// Biconjugate gradients (Fletcher, Lecture Notes in Mathematics 506,
    /// 1976) in the preconditioned form of Barrett et al., Templates for the
    /// Solution of Linear Systems, 1994, §2.3.5, with the shadow residual
    /// r~ = r0. M = M1*M2 is applied as M2\(M1\r), its transpose as M1'\(M2'\r~).
    fn slv_bicg(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut position = Count::One(0.);
        let mut x = p.x0.clone();
        let mut r = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&r);
        let mut mon = Self::slv_monitor(p, normr);
        if normr <= mon.tolb {
            return Ok(Self::slv_initial(p, x, normr));
        }
        let mut rt = r.clone();
        let mut resvec = vec![normr];
        let (mut xmin, mut imin) = (x.clone(), 0.);
        let (mut rho, mut flag) = (1f64, 1u8);
        let (mut pv, mut pt): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
        let mut ii = 0usize;
        for k in 1..=p.maxit {
            ii = k;
            position = Count::One(k as f64);
            self.slv_step(p.n)?;
            let z = slv_try!(self.slv_both(p, &r, out), p, x, resvec, [], position);
            let zt = slv_try!(self.slv_both_t(p, &rt, out), p, x, resvec, [], position);
            let rho_prev = rho;
            rho = slv_dot(&z, &rt);
            if rho == 0. || !rho.is_finite() {
                flag = 4;
                break;
            }
            if k == 1 {
                pv = z;
                pt = zt;
            } else {
                let beta = rho / rho_prev;
                if beta == 0. || !beta.is_finite() {
                    flag = 4;
                    break;
                }
                pv = slv_axpy(beta, &pv, &z);
                pt = slv_axpy(beta, &pt, &zt);
            }
            let q = slv_try!(self.slv_mul(p, &pv, out), p, x, resvec, [], position);
            let qt = slv_try!(self.slv_mul_t(p, &pt, out), p, x, resvec, [], position);
            let ptq = slv_dot(&pt, &q);
            if ptq == 0. || !ptq.is_finite() {
                flag = 4;
                break;
            }
            let alpha = rho / ptq;
            if !alpha.is_finite() {
                flag = 4;
                break;
            }
            if alpha.abs() * slv_norm(&pv) < EPS * slv_norm(&x) {
                mon.stag += 1;
            } else {
                mon.stag = 0;
            }
            let next_x = slv_axpy(alpha, &pv, &x);
            let previous_x = std::mem::replace(&mut x, next_x);
            r = slv_axpy(-alpha, &q, &r);
            rt = slv_axpy(-alpha, &qt, &rt);
            let (stored, check, true_r) = slv_try!(
                self.slv_check(p, &mut mon, &x, slv_norm(&r), out),
                p,
                previous_x,
                resvec,
                [],
                position
            );
            if let Some(t) = true_r {
                r = t;
            }
            resvec.push(stored);
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            if mon.normr_act < mon.normrmin {
                mon.normrmin = mon.normr_act;
                xmin = x.clone();
                imin = k as f64;
            }
            if mon.stag >= MAX_STAG_STEPS {
                flag = 3;
                break;
            }
        }
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &xmin,
                imin,
                slv_completed(p, resvec.len()),
                mon.normr_act,
                out
            ),
            p,
            x,
            resvec,
            [],
            position
        );
        if p.maxit == 0 {
            resvec.clear();
        }
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec,
            sixth: Vec::new(),
            stopped: (ii > 0).then_some(Count::One(ii as f64)),
        })
    }

    // -------------------------------------------------------------- cgs

    /// Sonneveld, "CGS, a Fast Lanczos-Type Solver for Nonsymmetric Linear
    /// Systems" (1989); right-preconditioned recurrence as in Barrett et al.,
    /// "Templates for the Solution of Linear Systems" (1994), §2.3.7, Fig. 2.9.
    /// The order of the preconditioner solves and products follows the netlib
    /// templates (Barrett et al. 1994) reference code.
    fn slv_cgs(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut position = Count::One(0.);
        let mut x = p.x0.clone();
        let mut r = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&r);
        let mut mon = Self::slv_monitor(p, normr);
        if normr <= mon.tolb || normr.is_nan() {
            let mut solved = Self::slv_initial(p, x, normr);
            solved.flag = u8::from(normr.is_nan());
            return Ok(solved);
        }
        let shadow = r.clone();
        let mut resvec = vec![normr];
        let (mut xmin, mut imin) = (x.clone(), 0.);
        let mut direction = vec![0.; p.n];
        let mut q = vec![0.; p.n];
        let mut previous_rho = 1.;
        let mut flag = 1;
        let mut ii = 0usize;

        for k in 1..=p.maxit {
            ii = k;
            position = Count::One(k as f64);
            self.slv_step(p.n)?;
            let rho = slv_dot(&shadow, &r);
            if rho == 0. || !rho.is_finite() {
                flag = 4;
                break;
            }
            let u = if k == 1 {
                direction.clone_from(&r);
                r.clone()
            } else {
                let beta = rho / previous_rho;
                if !beta.is_finite() {
                    flag = 4;
                    break;
                }
                let u = slv_axpy(beta, &q, &r);
                direction = slv_axpy(beta, &slv_axpy(beta, &direction, &q), &u);
                u
            };
            let phat = slv_try!(
                self.slv_both(p, &direction, out),
                p,
                x,
                resvec,
                [],
                position
            );
            let v = slv_try!(self.slv_mul(p, &phat, out), p, x, resvec, [], position);
            let sigma = slv_dot(&shadow, &v);
            if sigma == 0. || !sigma.is_finite() {
                flag = 4;
                break;
            }
            let alpha = rho / sigma;
            if !alpha.is_finite() {
                flag = 4;
                break;
            }
            q = slv_axpy(-alpha, &v, &u);
            let uhat = slv_try!(
                self.slv_both(p, &slv_add(&u, &q), out),
                p,
                x,
                resvec,
                [],
                position
            );
            let au = slv_try!(self.slv_mul(p, &uhat, out), p, x, resvec, [], position);
            if alpha.abs() * slv_norm(&uhat) < EPS * slv_norm(&x) {
                mon.stag += 1;
            } else {
                mon.stag = 0;
            }
            // Commit only after every operator needed for this update succeeds.
            let next_x = slv_axpy(alpha, &uhat, &x);
            let next_r = slv_axpy(-alpha, &au, &r);
            if !slv_finite(&next_x) || !slv_finite(&next_r) {
                flag = 4;
                break;
            }
            let (stored, check, true_r) = slv_try!(
                self.slv_check(p, &mut mon, &next_x, slv_norm(&next_r), out),
                p,
                x,
                resvec,
                [],
                position
            );
            x = next_x;
            r = true_r.unwrap_or(next_r);
            resvec.push(stored);
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            if stored < mon.normrmin {
                mon.normrmin = stored;
                xmin.clone_from(&x);
                imin = k as f64;
            }
            if mon.stag >= MAX_STAG_STEPS {
                flag = 3;
                break;
            }
            previous_rho = rho;
        }
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &xmin,
                imin,
                slv_completed(p, resvec.len()),
                mon.normr_act,
                out
            ),
            p,
            x,
            resvec,
            [],
            position
        );
        if p.maxit == 0 {
            resvec.clear();
        }
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec,
            sixth: Vec::new(),
            stopped: (ii > 0).then_some(Count::One(ii as f64)),
        })
    }

    // -------------------------------------------------------------- bicgstab

    /// BiCGSTAB (van der Vorst, SIAM J. Sci. Stat. Comput. 13, 1992) in the
    /// preconditioned form of Barrett et al., Templates, 1994, §2.3.8, with
    /// r~ = r0. Both half steps, the BiCG step to s and the one-dimensional
    /// minimal-residual step to r, are recorded, so counts move in halves.
    fn slv_bicgstab(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut position = Count::One(0.);
        let mut x = p.x0.clone();
        let mut r = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&r);
        let mut mon = Self::slv_monitor(p, normr);
        if normr <= mon.tolb {
            return Ok(Self::slv_initial(p, x, normr));
        }
        let rt = r.clone();
        let mut resvec = vec![normr];
        let (mut xmin, mut imin) = (x.clone(), 0.);
        let (mut rho, mut alpha, mut omega) = (1f64, 1f64, 1f64);
        let mut flag = 1u8;
        let (mut pv, mut v): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
        let mut ii = 0usize;
        for k in 1..=p.maxit {
            ii = k;
            position = Count::One(k as f64);
            self.slv_step(p.n)?;
            let rho_prev = rho;
            rho = slv_dot(&rt, &r);
            if rho == 0. || !rho.is_finite() {
                flag = 4;
                break;
            }
            if k == 1 {
                pv = r.clone();
            } else {
                let beta = (rho / rho_prev) * (alpha / omega);
                if beta == 0. || !beta.is_finite() {
                    flag = 4;
                    break;
                }
                pv = slv_axpy(beta, &slv_axpy(-omega, &v, &pv), &r);
            }
            let ph = slv_try!(self.slv_both(p, &pv, out), p, x, resvec, [], position);
            v = slv_try!(self.slv_mul(p, &ph, out), p, x, resvec, [], position);
            let rtv = slv_dot(&rt, &v);
            if rtv == 0. || !rtv.is_finite() {
                flag = 4;
                break;
            }
            alpha = rho / rtv;
            if !alpha.is_finite() {
                flag = 4;
                break;
            }
            // BiCG half step.
            if alpha.abs() * slv_norm(&ph) < EPS * slv_norm(&x) {
                mon.stag += 1;
            } else {
                mon.stag = 0;
            }
            let half = slv_axpy(alpha, &ph, &x);
            let previous_x = std::mem::replace(&mut x, half);
            let mut s = slv_axpy(-alpha, &v, &r);
            let (stored, check, true_s) = slv_try!(
                self.slv_check(p, &mut mon, &x, slv_norm(&s), out),
                p,
                previous_x,
                resvec,
                [],
                position
            );
            if let Some(t) = true_s {
                s = t;
            }
            resvec.push(stored);
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            if mon.normr_act < mon.normrmin {
                mon.normrmin = mon.normr_act;
                xmin = x.clone();
                imin = k as f64 - 0.5;
            }
            if mon.stag >= MAX_STAG_STEPS {
                flag = 3;
                break;
            }
            // Minimal-residual half step.
            let sh = slv_try!(self.slv_both(p, &s, out), p, x, resvec, [], position);
            let t = slv_try!(self.slv_mul(p, &sh, out), p, x, resvec, [], position);
            let tt = slv_dot(&t, &t);
            if tt == 0. || !tt.is_finite() {
                flag = 4;
                break;
            }
            omega = slv_dot(&t, &s) / tt;
            if omega == 0. || !omega.is_finite() {
                flag = 4;
                break;
            }
            if omega.abs() * slv_norm(&sh) < EPS * slv_norm(&x) {
                mon.stag += 1;
            } else {
                mon.stag = 0;
            }
            let next_x = slv_axpy(omega, &sh, &x);
            let previous_x = std::mem::replace(&mut x, next_x);
            r = slv_axpy(-omega, &t, &s);
            let (stored, check, true_r) = slv_try!(
                self.slv_check(p, &mut mon, &x, slv_norm(&r), out),
                p,
                previous_x,
                resvec,
                [],
                position
            );
            if let Some(t) = true_r {
                r = t;
            }
            resvec.push(stored);
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            if mon.normr_act < mon.normrmin {
                mon.normrmin = mon.normr_act;
                xmin = x.clone();
                imin = k as f64;
            }
            if mon.stag >= MAX_STAG_STEPS {
                flag = 3;
                break;
            }
        }
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &xmin,
                imin,
                slv_completed(p, resvec.len()),
                mon.normr_act,
                out
            ),
            p,
            x,
            resvec,
            [],
            position
        );
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec,
            sixth: Vec::new(),
            stopped: (ii > 0).then_some(Count::One(ii as f64)),
        })
    }

    // -------------------------------------------------------------- bicgstabl

    /// BiCGstab(l) with l = 2 (Sleijpen and Fokkema, ETNA 1, 1993, the
    /// algorithm of §3 with its modified Gram-Schmidt minimal-residual part),
    /// right preconditioned: the iteration solves A*inv(M)*y = r0 for
    /// x = x0 + M\y, and x is formed only when the true residual is checked
    /// or returned. Each iteration updates y l times in its BiCG part and l
    /// times in its minimal-residual part; resvec records the residual norm
    /// after each update, so counts move in steps of 1/(2l).
    fn slv_bicgstabl(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        const L: usize = 2;
        let n = p.n;
        let mut position = Count::One(0.);
        let mut x = p.x0.clone();
        let r = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&r);
        let mut mon = Self::slv_monitor(p, normr);
        if normr <= mon.tolb {
            return Ok(Self::slv_initial(p, x, normr));
        }
        let rt = r.clone();
        let mut resvec = vec![normr];
        // x was last formed for the first `formed` entries of resvec.
        let mut formed = 1usize;
        let mut y = vec![0.; n];
        let (mut ymin, mut at_min) = (y.clone(), 1usize);
        // rs[0] is the residual of y; rs[j] and us[j] hold A*inv(M) powers.
        let mut rs = vec![r; L + 1];
        let mut us = vec![vec![0.; n]; L + 1];
        let (mut rho0, mut alpha, mut omega) = (1f64, 0f64, 1f64);
        let mut flag = 1u8;
        let mut ii = 0usize;
        'iterations: for k in 1..=p.maxit {
            ii = k;
            position = Count::One(k as f64);
            self.slv_step(n)?;
            rho0 *= -omega;
            // BiCG part.
            for j in 0..L {
                let rho1 = slv_dot(&rs[j], &rt);
                if rho1 == 0. || !rho1.is_finite() {
                    flag = 4;
                    break 'iterations;
                }
                let beta = alpha * rho1 / rho0;
                if !beta.is_finite() {
                    flag = 4;
                    break 'iterations;
                }
                rho0 = rho1;
                for i in 0..=j {
                    us[i] = slv_axpy(-beta, &us[i], &rs[i]);
                }
                let mu = slv_try!(
                    self.slv_both(p, &us[j], out),
                    p,
                    x,
                    resvec[..formed],
                    [],
                    position
                );
                us[j + 1] = slv_try!(
                    self.slv_mul(p, &mu, out),
                    p,
                    x,
                    resvec[..formed],
                    [],
                    position
                );
                let sigma = slv_dot(&us[j + 1], &rt);
                if sigma == 0. || !sigma.is_finite() {
                    flag = 4;
                    break 'iterations;
                }
                alpha = rho0 / sigma;
                if !alpha.is_finite() {
                    flag = 4;
                    break 'iterations;
                }
                for i in 0..=j {
                    rs[i] = slv_axpy(-alpha, &us[i + 1], &rs[i]);
                }
                let step = alpha.abs() * slv_norm(&us[0]);
                y = slv_axpy(alpha, &us[0], &y);
                let (check, true_r) = slv_try!(
                    self.slv_bicgstabl_check(
                        p,
                        &mut mon,
                        &mut resvec,
                        (&y, step, slv_norm(&rs[0]), true),
                        (&mut x, &mut formed),
                        out
                    ),
                    p,
                    x,
                    resvec[..formed],
                    [],
                    position
                );
                if let Some(t) = true_r {
                    rs[0] = t;
                }
                match check {
                    Check::Converged => {
                        flag = 0;
                        break 'iterations;
                    }
                    Check::Stagnated => {
                        flag = 3;
                        break 'iterations;
                    }
                    Check::Continue => {}
                }
                if mon.normr_act < mon.normrmin {
                    mon.normrmin = mon.normr_act;
                    ymin = y.clone();
                    at_min = resvec.len();
                }
                if mon.stag >= MAX_STAG_STEPS {
                    flag = 3;
                    break 'iterations;
                }
                let mr = slv_try!(
                    self.slv_both(p, &rs[j], out),
                    p,
                    x,
                    resvec[..formed],
                    [],
                    position
                );
                rs[j + 1] = slv_try!(
                    self.slv_mul(p, &mr, out),
                    p,
                    x,
                    resvec[..formed],
                    [],
                    position
                );
            }
            // Minimal-residual part: modified Gram-Schmidt on rs[1..=L], then
            // the coefficients gamma (of the polynomial), gamma' and gamma''.
            let mut tau = [[0f64; L + 1]; L + 1];
            let mut sigma = [0f64; L + 1];
            let mut gamma1 = [0f64; L + 1];
            for j in 1..=L {
                for i in 1..j {
                    tau[i][j] = slv_dot(&rs[j], &rs[i]) / sigma[i];
                    rs[j] = slv_axpy(-tau[i][j], &rs[i], &rs[j]);
                }
                sigma[j] = slv_dot(&rs[j], &rs[j]);
                if sigma[j] == 0. || !sigma[j].is_finite() {
                    flag = 4;
                    break 'iterations;
                }
                gamma1[j] = slv_dot(&rs[0], &rs[j]) / sigma[j];
            }
            let mut gamma = [0f64; L + 1];
            gamma[L] = gamma1[L];
            for j in (1..L).rev() {
                let sum: f64 = (j + 1..=L).map(|i| tau[j][i] * gamma[i]).sum();
                gamma[j] = gamma1[j] - sum;
            }
            let mut gamma2 = [0f64; L + 1];
            for j in 1..L {
                let sum: f64 = (j + 1..L).map(|i| tau[j][i] * gamma[i + 1]).sum();
                gamma2[j] = gamma[j + 1] + sum;
            }
            if !(slv_finite(&gamma) && slv_finite(&gamma2)) {
                flag = 4;
                break 'iterations;
            }
            omega = gamma[L];
            for j in 0..L {
                // j = 0: y += gamma_1 r_0, r_0 -= gamma'_l r_l, u_0 -= gamma_l u_l;
                // then y += gamma''_j r_j, r_0 -= gamma'_j r_j, u_0 -= gamma_j u_j.
                let (coefficient, residual_coefficient, index) = if j == 0 {
                    (gamma[1], gamma1[L], L)
                } else {
                    (gamma2[j], gamma1[j], j)
                };
                let step = coefficient.abs() * slv_norm(&rs[j]);
                y = slv_axpy(coefficient, &rs[j], &y);
                rs[0] = slv_axpy(-residual_coefficient, &rs[index], &rs[0]);
                us[0] = slv_axpy(-gamma[index], &us[index], &us[0]);
                let (check, true_r) = slv_try!(
                    self.slv_bicgstabl_check(
                        p,
                        &mut mon,
                        &mut resvec,
                        (&y, step, slv_norm(&rs[0]), j + 1 == L),
                        (&mut x, &mut formed),
                        out
                    ),
                    p,
                    x,
                    resvec[..formed],
                    [],
                    position
                );
                // Before the last update, y and rs[0] do not yet match.
                if let Some(t) = true_r.filter(|_| j + 1 == L) {
                    rs[0] = t;
                }
                match check {
                    Check::Converged => {
                        flag = 0;
                        break 'iterations;
                    }
                    Check::Stagnated => {
                        flag = 3;
                        break 'iterations;
                    }
                    Check::Continue => {}
                }
                if mon.normr_act < mon.normrmin {
                    mon.normrmin = mon.normr_act;
                    ymin = y.clone();
                    at_min = resvec.len();
                }
                if mon.stag >= MAX_STAG_STEPS {
                    flag = 3;
                    break 'iterations;
                }
            }
        }
        if flag != 0 && formed < resvec.len() {
            x = slv_try!(
                self.slv_bicgstabl_x(p, &y, out),
                p,
                x,
                resvec[..formed],
                [],
                position
            );
            formed = resvec.len();
        }
        let xmin = if flag == 0 || at_min == formed {
            x.clone()
        } else {
            slv_try!(
                self.slv_bicgstabl_x(p, &ymin, out),
                p,
                x,
                resvec,
                [],
                position
            )
        };
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &xmin,
                slv_completed(p, at_min),
                slv_completed(p, resvec.len()),
                mon.normr_act,
                out
            ),
            p,
            x,
            resvec,
            [],
            position
        );
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec,
            sixth: Vec::new(),
            stopped: (ii > 0).then_some(Count::One(ii as f64)),
        })
    }

    /// `x0 + M\y` (M2\(M1\y)); `x0` itself while y is zero.
    fn slv_bicgstabl_x(
        &mut self,
        p: &Problem,
        y: &[f64],
        out: &mut Execution,
    ) -> SlvResult<Vec<f64>> {
        if y.iter().all(|&v| v == 0.) {
            return Ok(p.x0.clone());
        }
        let correction = self.slv_both(p, y, out)?;
        Ok(slv_add(&p.x0, &correction))
    }

    /// The pcg family's monitor after one BiCGstab(l) update of y with
    /// residual norm `normr` and update size `step`: counts stagnation against
    /// ||y||, forms x (one preconditioner solve) only when the monitor needs
    /// the true residual, and records the history entry. Returns the outcome
    /// and the true residual when one was computed. `settled` says that
    /// `normr` belongs to y (the BiCG updates and the last minimal-residual
    /// update); between the minimal-residual updates r_0 is only partly
    /// updated, so x is formed for such an entry only when its true residual
    /// is checked, and a later failure still returns an x that the history
    /// describes.
    fn slv_bicgstabl_check(
        &mut self,
        p: &Problem,
        mon: &mut Monitor,
        resvec: &mut Vec<f64>,
        (y, step, normr, settled): (&[f64], f64, f64, bool),
        (x, formed): (&mut Vec<f64>, &mut usize),
        out: &mut Execution,
    ) -> SlvResult<(Check, Option<Vec<f64>>)> {
        if step < EPS * slv_norm(y) {
            mon.stag += 1;
        } else {
            mon.stag = 0;
        }
        let preconditioned = p.m1.is_some() || p.m2.is_some();
        let checks = normr <= mon.tolb || mon.stag >= MAX_STAG_STEPS || mon.moresteps > 0.;
        let current = if checks || (!preconditioned && settled) {
            Some(self.slv_bicgstabl_x(p, y, out)?)
        } else {
            None
        };
        let (stored, check, true_r) = self.slv_check(
            p,
            mon,
            current.as_deref().unwrap_or(x.as_slice()),
            normr,
            out,
        )?;
        resvec.push(stored);
        if let Some(current) = current {
            *x = current;
            *formed = resvec.len();
        }
        Ok((check, true_r))
    }

    // -------------------------------------------------------------- qmr

    /// Freund and Nachtigal, "QMR: a Quasi-Minimal Residual Method for
    /// Non-Hermitian Linear Systems" (1991). This is the coupled recurrence
    /// without look-ahead in Barrett et al., "Templates" (1994), §2.3.6,
    /// Fig. 2.8, with separate left M1 and right M2 solves. The order of the
    /// preconditioner solves and transposed products follows the netlib
    /// templates (Barrett et al. 1994) reference code.
    fn slv_qmr(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut position = Count::One(0.);
        let mut x = p.x0.clone();
        let mut r = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&r);
        let mut mon = Self::slv_monitor(p, normr);
        if normr <= mon.tolb || normr.is_nan() {
            let mut solved = Self::slv_initial(p, x, normr);
            solved.flag = u8::from(normr.is_nan());
            return Ok(solved);
        }
        let mut resvec = vec![normr];
        let (mut xmin, mut imin) = (x.clone(), 0.);
        let mut v_tilde = r.clone();
        let mut w_tilde = r.clone();
        let mut y = match &p.m1 {
            Some(m) => slv_try!(
                self.slv_precondition(p, m, &v_tilde, false, out),
                p,
                x,
                resvec,
                [],
                position
            ),
            None => v_tilde.clone(),
        };
        let mut z = match &p.m2 {
            Some(m) => slv_try!(
                self.slv_precondition(p, m, &w_tilde, true, out),
                p,
                x,
                resvec,
                [],
                position
            ),
            None => w_tilde.clone(),
        };
        let (mut rho, mut xi) = (slv_norm(&y), slv_norm(&z));
        let (mut gamma, mut theta, mut eta) = (1f64, 0f64, -1f64);
        let mut epsilon = 1.;
        let mut direction = vec![0.; p.n];
        let mut dual_direction = vec![0.; p.n];
        let mut d = vec![0.; p.n];
        let mut s = vec![0.; p.n];
        let mut flag = 1;
        let mut ii = 0usize;

        for k in 1..=p.maxit {
            ii = k;
            position = Count::One(k as f64);
            self.slv_step(p.n)?;
            if rho == 0. || xi == 0. || !rho.is_finite() || !xi.is_finite() {
                flag = 4;
                break;
            }
            let v = slv_div(&v_tilde, rho);
            let w = slv_div(&w_tilde, xi);
            y = slv_div(&y, rho);
            z = slv_div(&z, xi);
            let delta = slv_dot(&z, &y);
            if delta == 0. || !delta.is_finite() {
                flag = 4;
                break;
            }
            let y_tilde = match &p.m2 {
                Some(m) => slv_try!(
                    self.slv_precondition(p, m, &y, false, out),
                    p,
                    x,
                    resvec,
                    [],
                    position
                ),
                None => y.clone(),
            };
            let z_tilde = match &p.m1 {
                Some(m) => slv_try!(
                    self.slv_precondition(p, m, &z, true, out),
                    p,
                    x,
                    resvec,
                    [],
                    position
                ),
                None => z.clone(),
            };
            if k == 1 {
                direction = y_tilde;
                dual_direction = z_tilde;
            } else {
                direction = slv_axpy(-xi * delta / epsilon, &direction, &y_tilde);
                dual_direction = slv_axpy(-rho * delta / epsilon, &dual_direction, &z_tilde);
            }
            let ad = slv_try!(self.slv_mul(p, &direction, out), p, x, resvec, [], position);
            epsilon = slv_dot(&dual_direction, &ad);
            let beta = epsilon / delta;
            if beta == 0. || !beta.is_finite() {
                flag = 4;
                break;
            }
            v_tilde = slv_axpy(-beta, &v, &ad);
            y = match &p.m1 {
                Some(m) => slv_try!(
                    self.slv_precondition(p, m, &v_tilde, false, out),
                    p,
                    x,
                    resvec,
                    [],
                    position
                ),
                None => v_tilde.clone(),
            };
            let old_rho = rho;
            rho = slv_norm(&y);
            let atd = slv_try!(
                self.slv_mul_t(p, &dual_direction, out),
                p,
                x,
                resvec,
                [],
                position
            );
            w_tilde = slv_axpy(-beta, &w, &atd);
            z = match &p.m2 {
                Some(m) => slv_try!(
                    self.slv_precondition(p, m, &w_tilde, true, out),
                    p,
                    x,
                    resvec,
                    [],
                    position
                ),
                None => w_tilde.clone(),
            };
            xi = slv_norm(&z);

            let old_gamma = gamma;
            let old_theta = theta;
            theta = rho / (old_gamma * beta.abs());
            gamma = 1. / slv_norm2(1., theta);
            if gamma == 0. || !gamma.is_finite() {
                flag = 4;
                break;
            }
            eta = -eta * old_rho * gamma * gamma / (beta * old_gamma * old_gamma);
            let carry = (old_theta * gamma).powi(2);
            d = slv_axpy(carry, &d, &slv_scale(eta, &direction));
            s = slv_axpy(carry, &s, &slv_scale(eta, &ad));
            if slv_norm(&d) < EPS * slv_norm(&x) {
                mon.stag += 1;
            } else {
                mon.stag = 0;
            }
            let next_x = slv_add(&x, &d);
            let next_r = slv_sub(&r, &s);
            if !slv_finite(&next_x) || !slv_finite(&next_r) {
                flag = 4;
                break;
            }
            let (stored, check, true_r) = slv_try!(
                self.slv_check(p, &mut mon, &next_x, slv_norm(&next_r), out),
                p,
                x,
                resvec,
                [],
                position
            );
            x = next_x;
            r = true_r.unwrap_or(next_r);
            resvec.push(stored);
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            if stored < mon.normrmin {
                mon.normrmin = stored;
                xmin.clone_from(&x);
                imin = k as f64;
            }
            if mon.stag >= MAX_STAG_STEPS {
                flag = 3;
                break;
            }
        }
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &xmin,
                imin,
                slv_completed(p, resvec.len()),
                mon.normr_act,
                out
            ),
            p,
            x,
            resvec,
            [],
            position
        );
        if p.maxit == 0 {
            resvec.clear();
        }
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec,
            sixth: Vec::new(),
            stopped: (ii > 0).then_some(Count::One(ii as f64)),
        })
    }

    // -------------------------------------------------------------- tfqmr

    /// Freund, "A Transpose-Free Quasi-Minimal Residual Algorithm for
    /// Non-Hermitian Linear Systems" (1993). The two quasi-minimization steps
    /// are also given in Freund, Golub and Nachtigal, "Iterative Solution of
    /// Linear Systems" (1992), Algorithm 3.9. Apply it to A M^-1, keeping d in
    /// solution coordinates. Since x_m = x_{m-1} + eta_m d_m and
    /// d_m = z_m + psi d_{m-1}, the physical residual follows from
    /// A d_m = u_m + psi A d_{m-1} without another product with x.
    fn slv_tfqmr(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut position = Count::One(0.);
        let mut x = p.x0.clone();
        let mut w = slv_try!(self.slv_residual(p, &x, out), p, x, [p.n2b], [], position);
        let normr = slv_norm(&w);
        let mut mon = Self::slv_monitor(p, normr);
        if normr <= mon.tolb || normr.is_nan() {
            let mut solved = Self::slv_initial(p, x, normr);
            solved.flag = u8::from(normr.is_nan());
            return Ok(solved);
        }
        let mut resvec = vec![normr];
        let (mut xmin, mut imin) = (x.clone(), 0.);
        let shadow = w.clone();
        let mut r = w.clone();
        let mut y = w.clone();
        let mut z = slv_try!(self.slv_both(p, &y, out), p, x, resvec, [], position);
        let mut u = slv_try!(self.slv_mul(p, &z, out), p, x, resvec, [], position);
        let mut v = u.clone();
        let mut d = vec![0.; p.n];
        let mut ad = vec![0.; p.n];
        let (mut tau, mut theta, mut eta) = (normr, 0f64, 0f64);
        let mut rho = slv_dot(&shadow, &w);
        let mut flag = 1;
        let mut ii = 0usize;

        'iterations: for k in 0..p.maxit {
            ii = 2 * k + 1;
            position = Count::One(ii as f64);
            let sigma = slv_dot(&shadow, &v);
            if rho == 0. || sigma == 0. || !rho.is_finite() || !sigma.is_finite() {
                flag = 4;
                break;
            }
            let alpha = rho / sigma;
            if alpha == 0. || !alpha.is_finite() {
                flag = 4;
                break;
            }
            // The even search direction belongs to the same CGS polynomial.
            let even_y = slv_axpy(-alpha, &v, &y);
            for half in 0..2 {
                position = Count::One((ii + half) as f64);
                self.slv_step(p.n)?;
                if half == 1 {
                    y = even_y.clone();
                    z = slv_try!(self.slv_both(p, &y, out), p, x, resvec, [], position);
                    u = slv_try!(self.slv_mul(p, &z, out), p, x, resvec, [], position);
                }
                w = slv_axpy(-alpha, &u, &w);
                let psi = theta * theta * eta / alpha;
                d = slv_axpy(psi, &d, &z);
                ad = slv_axpy(psi, &ad, &u);
                let normw = slv_norm(&w);
                theta = normw / tau;
                let cosine = 1. / slv_norm2(1., theta);
                tau = normw * cosine;
                eta = cosine * cosine * alpha;
                let next_x = slv_axpy(eta, &d, &x);
                let next_r = slv_axpy(-eta, &ad, &r);
                if !slv_finite(&next_x)
                    || !slv_finite(&next_r)
                    || !tau.is_finite()
                    || !eta.is_finite()
                {
                    flag = 4;
                    break 'iterations;
                }
                if eta.abs() * slv_norm(&d) < EPS * slv_norm(&x) {
                    mon.stag += 1;
                } else {
                    mon.stag = 0;
                }
                // The history reports ||b-A*x|| at every half step; the
                // recursive residual stands in until slv_check needs the true one.
                let (stored, check, true_r) = slv_try!(
                    self.slv_check(p, &mut mon, &next_x, slv_norm(&next_r), out),
                    p,
                    x,
                    resvec,
                    [],
                    position
                );
                x = next_x;
                r = true_r.unwrap_or(next_r);
                resvec.push(stored);
                match check {
                    Check::Converged => {
                        flag = 0;
                        break 'iterations;
                    }
                    Check::Stagnated => {
                        flag = 3;
                        break 'iterations;
                    }
                    Check::Continue => {}
                }
                if stored < mon.normrmin {
                    mon.normrmin = stored;
                    xmin.clone_from(&x);
                    imin = slv_completed(p, resvec.len());
                }
                if mon.stag >= MAX_STAG_STEPS {
                    flag = 3;
                    break 'iterations;
                }
            }
            // Do not evaluate unused directions after the iteration limit.
            if k + 1 == p.maxit {
                break;
            }
            let next_rho = slv_dot(&shadow, &w);
            let beta = next_rho / rho;
            if next_rho == 0. || !beta.is_finite() {
                flag = 4;
                break;
            }
            y = slv_axpy(beta, &y, &w);
            z = slv_try!(self.slv_both(p, &y, out), p, x, resvec, [], position);
            let next_u = slv_try!(self.slv_mul(p, &z, out), p, x, resvec, [], position);
            v = slv_axpy(beta, &slv_axpy(beta, &v, &u), &next_u);
            u = next_u;
            rho = next_rho;
        }
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &xmin,
                imin,
                slv_completed(p, resvec.len()),
                mon.normr_act,
                out
            ),
            p,
            x,
            resvec,
            [],
            position
        );
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec,
            sixth: Vec::new(),
            stopped: (ii > 0).then_some(position),
        })
    }

    // -------------------------------------------------------------- minres

    /// Paige and Saunders (1975), sections 3 and 6: Lanczos followed by
    /// incremental QR of the extended tridiagonal matrix, equations (6.6--9).
    fn slv_minres(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut x = p.x0.clone();
        let mut history = vec![p.n2b];
        let mut cg_history = vec![p.n2b];
        let initial_r = slv_try!(
            self.slv_residual(p, &x, out),
            p,
            x,
            history,
            cg_history,
            Count::One(0.)
        );
        let initial_norm = slv_norm(&initial_r);
        history[0] = initial_norm;
        cg_history[0] = initial_norm;
        if initial_norm <= p.tol * p.n2b {
            return Ok(Self::slv_initial(p, x, initial_norm));
        }
        let mut raw = initial_r.clone();
        let mut pre = slv_try!(
            self.slv_both(p, &raw, out),
            p,
            x,
            history,
            cg_history,
            Count::One(0.)
        );
        let energy = slv_dot(&raw, &pre);
        let preconditioned = p.m1.is_some() || p.m2.is_some();
        if energy <= 0. || !energy.is_finite() {
            // r'*inv(M)*r is positive for a symmetric positive definite M, so
            // zero or less with a preconditioner is a bad preconditioner.
            return Ok(Solved {
                x,
                flag: if energy < 0. || (energy == 0. && preconditioned) {
                    5
                } else {
                    4
                },
                relres: initial_norm / p.n2b,
                iter: vec![0.],
                resvec: history,
                sixth: cg_history,
                stopped: Some(Count::One(0.)),
            });
        }
        let mut beta = energy.sqrt();
        let mut phi_bar = beta;
        let mut residual_basis = slv_div(&raw, beta);
        let mut previous_raw = vec![0.; p.n];
        let mut previous_beta = 1.;
        let mut direction_old = vec![0.; p.n];
        let mut direction = vec![0.; p.n];
        let (mut cosine, mut sine) = (-1., 0.);
        let (mut next_diagonal, mut next_super) = (0., 0.);
        let mut monitor = Self::slv_monitor(p, initial_norm);
        let mut best = x.clone();
        let mut best_step = 0.;
        let mut flag = 1;
        let mut stopped = None;
        for step in 1..=p.maxit {
            self.slv_step(p.n)?;
            let position = Count::One(step as f64);
            stopped = Some(position);
            let v = slv_div(&pre, beta);
            let mut next_raw = slv_try!(
                self.slv_mul(p, &v, out),
                p,
                x,
                history,
                cg_history,
                position
            );
            if step > 1 {
                next_raw = slv_axpy(-beta / previous_beta, &previous_raw, &next_raw);
            }
            let alpha = slv_dot(&v, &next_raw);
            next_raw = slv_axpy(-alpha / beta, &raw, &next_raw);
            let next_pre = slv_try!(
                self.slv_both(p, &next_raw, out),
                p,
                x,
                history,
                cg_history,
                position
            );
            let energy = slv_dot(&next_raw, &next_pre);
            if energy < 0. || !energy.is_finite() || !alpha.is_finite() {
                flag = if energy < 0. { 5 } else { 4 };
                break;
            }
            let next_beta = energy.sqrt();
            // The previous two rotations leave just three entries in this column.
            let super2 = next_super;
            let super1 = cosine * next_diagonal + sine * alpha;
            let diagonal = sine * next_diagonal - cosine * alpha;
            next_super = sine * next_beta;
            next_diagonal = -cosine * next_beta;
            let gamma = slv_norm2(diagonal, next_beta);
            if gamma == 0. || !gamma.is_finite() {
                flag = 4;
                break;
            }
            cosine = diagonal / gamma;
            sine = next_beta / gamma;
            let previous_phi_bar = phi_bar;
            let phi = cosine * phi_bar;
            phi_bar *= sine;
            let next_direction = slv_div(
                &slv_axpy(-super1, &direction, &slv_axpy(-super2, &direction_old, &v)),
                gamma,
            );
            monitor.stag = if phi.abs() * slv_norm(&next_direction) < EPS * slv_norm(&x) {
                monitor.stag + 1
            } else {
                0
            };
            let next_x = slv_axpy(phi, &next_direction, &x);
            if !slv_finite(&next_x) {
                flag = 4;
                break;
            }
            x = next_x;
            // Apply the same rotation to the dual Lanczos vectors. This keeps
            // the Euclidean residual available even with a scaled SPD M.
            let next_basis = if next_beta == 0. {
                vec![0.; p.n]
            } else {
                slv_axpy(
                    -cosine / next_beta,
                    &next_raw,
                    &slv_scale(sine, &residual_basis),
                )
            };
            let normr = phi_bar.abs() * slv_norm(&next_basis);
            history.push(normr);
            let (normr, check, _) = slv_try!(
                self.slv_check(p, &mut monitor, &x, normr, out),
                p,
                x,
                history,
                cg_history,
                position
            );
            *history.last_mut().unwrap() = normr;
            // Sixth output: the residual of the CG point, which lies on the
            // line through the last two MINRES points at a distance
            // (sine/cosine)^2 beyond the newer one (Paige and Saunders 1975,
            // section 6 with equation (7.1)). Its history has a reserved zero
            // between its first two updates.
            let correction_scale = (sine / cosine).powi(2);
            let cg_norm = if !correction_scale.is_finite() {
                f64::INFINITY
            } else {
                // A*(phi*direction) = r_previous - r_current, so the
                // residual recurrence saves an operator application per step.
                // The unpreconditioned convention (the saved reference
                // values) applies that correction to the initial residual;
                // with a preconditioner it is applied to the current one.
                let correction_residual: Vec<f64> = initial_r
                    .iter()
                    .zip(&residual_basis)
                    .zip(&next_basis)
                    .map(|((&initial, &old), &new)| {
                        let base = if preconditioned {
                            phi_bar * new
                        } else {
                            initial
                        };
                        base - correction_scale * (previous_phi_bar * old - phi_bar * new)
                    })
                    .collect();
                slv_norm(&correction_residual)
            };
            residual_basis = next_basis;
            cg_history.push(cg_norm);
            if step == 1 {
                cg_history.push(0.);
            }
            if normr < monitor.normrmin {
                monitor.normrmin = normr;
                best.clone_from(&x);
                best_step = step as f64;
            }
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            if monitor.stag >= MAX_STAG_STEPS {
                flag = 3;
                break;
            }
            if next_beta == 0. {
                flag = 4;
                break;
            }
            direction_old = direction;
            direction = next_direction;
            previous_raw = raw;
            raw = next_raw;
            pre = next_pre;
            previous_beta = beta;
            beta = next_beta;
        }
        let completed = (history.len() - 1) as f64;
        let (x, iter, relres) = slv_try!(
            self.slv_finish(
                p,
                flag,
                &x,
                &best,
                best_step,
                completed,
                monitor.normr_act,
                out
            ),
            p,
            x,
            history,
            cg_history,
            stopped.unwrap_or(Count::One(0.))
        );
        // Without an iteration there is no residual history.
        if p.maxit == 0 {
            history.clear();
        }
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec: history,
            sixth: cg_history,
            stopped,
        })
    }

    // -------------------------------------------------------------- symmlq

    /// Paige and Saunders (1975), section 5, equations (5.5--11), (5.17--19):
    /// LQ of the Lanczos tridiagonal, with the optional CG point. The public
    /// iteration count excludes the initial Lanczos column.
    fn slv_symmlq(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut x = p.x0.clone();
        let mut history = vec![p.n2b];
        let mut cg_history = vec![p.n2b];
        let residual = slv_try!(
            self.slv_residual(p, &x, out),
            p,
            x,
            history,
            cg_history,
            Count::One(0.)
        );
        let initial_norm = slv_norm(&residual);
        history[0] = initial_norm;
        cg_history[0] = initial_norm;
        if initial_norm <= p.tol * p.n2b {
            return Ok(Self::slv_initial(p, x, initial_norm));
        }
        if p.maxit == 0 {
            return Ok(Solved {
                x,
                flag: 1,
                relres: initial_norm / p.n2b,
                iter: vec![0.],
                resvec: Vec::new(),
                sixth: cg_history,
                stopped: None,
            });
        }
        let pre = slv_try!(
            self.slv_both(p, &residual, out),
            p,
            x,
            history,
            cg_history,
            Count::One(0.)
        );
        let energy = slv_dot(&residual, &pre);
        if energy <= 0. || !energy.is_finite() {
            // Zero or less with a preconditioner: it is not positive definite.
            let preconditioned = p.m1.is_some() || p.m2.is_some();
            return Ok(Solved {
                x,
                flag: if energy < 0. || (energy == 0. && preconditioned) {
                    5
                } else {
                    4
                },
                relres: initial_norm / p.n2b,
                iter: vec![0.],
                resvec: history,
                sixth: cg_history,
                stopped: Some(Count::One(0.)),
            });
        }
        let first_beta = energy.sqrt();
        let mut dual = slv_div(&residual, first_beta);
        let mut v = slv_div(&pre, first_beta);
        let (alpha, energy, mut raw_next, mut pre_next) = slv_try!(
            self.slv_cr_lanczos(p, &v, &dual, None, out),
            p,
            x,
            history,
            cg_history,
            Count::One(0.)
        );
        if energy < 0. || !energy.is_finite() || !alpha.is_finite() {
            return Ok(Solved {
                x,
                flag: if energy < 0. { 5 } else { 4 },
                relres: initial_norm / p.n2b,
                iter: vec![0.],
                resvec: history,
                sixth: cg_history,
                stopped: Some(Count::One(0.)),
            });
        }
        let mut beta = energy.sqrt();
        let first_cg_norm = if alpha == 0. {
            f64::INFINITY
        } else {
            (first_beta / alpha).abs() * slv_norm(&raw_next)
        };
        cg_history.push(first_cg_norm);
        // An invariant one-dimensional space needs no second Lanczos column.
        if beta <= EPS * alpha.abs() && alpha != 0. {
            let next_x = slv_axpy(first_beta / alpha, &v, &x);
            if !slv_finite(&next_x) {
                return Ok(Solved {
                    x,
                    flag: 4,
                    relres: initial_norm / p.n2b,
                    iter: vec![0.],
                    resvec: history,
                    sixth: cg_history,
                    stopped: Some(Count::One(0.)),
                });
            }
            x = next_x;
            history.push(first_cg_norm);
            let r = slv_try!(
                self.slv_residual(p, &x, out),
                p,
                x,
                history,
                cg_history,
                Count::One(1.)
            );
            let normr = slv_norm(&r);
            history[1] = normr;
            return Ok(Solved {
                x,
                flag: if normr <= p.tol * p.n2b { 0 } else { 4 },
                relres: normr / p.n2b,
                iter: vec![1.],
                resvec: history,
                sixth: cg_history,
                stopped: Some(Count::One(1.)),
            });
        }
        let mut lq_x = p.x0.clone();
        let mut rotating_direction = v.clone();
        let mut diagonal = alpha;
        let (mut previous_cosine, mut previous_sine) = (-1., 0.);
        let mut rhs = first_beta;
        let mut previous_zeta = 0.;
        let mut monitor = Self::slv_monitor(p, initial_norm);
        let mut best = x.clone();
        let mut best_step = 0.;
        let mut norm_selected = initial_norm;
        let mut flag = 1;
        let mut stopped = Some(Count::One(0.));
        for step in 1..=p.maxit {
            self.slv_step(p.n)?;
            let position = Count::One(step as f64);
            stopped = Some(position);
            if beta == 0. {
                flag = 4;
                break;
            }
            let old_dual = dual;
            dual = slv_div(&raw_next, beta);
            v = slv_div(&pre_next, beta);
            let (next_alpha, energy, next_raw, next_pre) = slv_try!(
                self.slv_cr_lanczos(p, &v, &dual, Some((&old_dual, beta)), out),
                p,
                x,
                slv_history_with_last(&history, norm_selected),
                cg_history,
                position
            );
            if energy < 0. || !energy.is_finite() || !next_alpha.is_finite() {
                flag = if energy < 0. { 5 } else { 4 };
                break;
            }
            let next_beta = energy.sqrt();
            let gamma = slv_norm2(diagonal, beta);
            if gamma == 0. || !gamma.is_finite() {
                flag = 4;
                break;
            }
            let cosine = diagonal / gamma;
            let sine = beta / gamma;
            let zeta = rhs / gamma;
            let fixed_direction = slv_axpy(sine, &v, &slv_scale(cosine, &rotating_direction));
            rotating_direction = slv_axpy(-cosine, &v, &slv_scale(sine, &rotating_direction));
            monitor.stag = if zeta.abs() * slv_norm(&fixed_direction) < EPS * slv_norm(&lq_x) {
                monitor.stag + 1
            } else {
                0
            };
            let next_lq = slv_axpy(zeta, &fixed_direction, &lq_x);
            if !slv_finite(&next_lq) {
                flag = 4;
                break;
            }
            lq_x = next_lq;
            let sub2 = previous_sine * beta;
            let sub1 = -cosine * previous_cosine * beta + sine * next_alpha;
            diagonal = -sine * previous_cosine * beta - cosine * next_alpha;
            rhs = -sub2 * previous_zeta - sub1 * zeta;
            // Equation (5.17), in the dual basis, gives an unpreconditioned
            // residual without another application of A.
            let lq_residual = slv_axpy(-sine * zeta, &next_raw, &slv_scale(rhs, &dual));
            let lq_norm = slv_norm(&lq_residual);
            let cg_scale = rhs / diagonal;
            let cg_norm = if diagonal == 0. || !cg_scale.is_finite() {
                f64::INFINITY
            } else {
                (sine * zeta - cosine * cg_scale).abs() * slv_norm(&next_raw)
            };
            if cg_norm < lq_norm {
                let next_x = slv_axpy(cg_scale, &rotating_direction, &lq_x);
                if !slv_finite(&next_x) {
                    flag = 4;
                    break;
                }
                x = next_x;
                norm_selected = cg_norm;
            } else {
                x.clone_from(&lq_x);
                norm_selected = lq_norm;
            }
            history.push(lq_norm);
            cg_history.push(cg_norm);
            let (checked_norm, check, _) = slv_try!(
                self.slv_check(p, &mut monitor, &x, norm_selected, out),
                p,
                x,
                slv_history_with_last(&history, norm_selected),
                cg_history,
                position
            );
            norm_selected = checked_norm;
            if norm_selected < monitor.normrmin {
                monitor.normrmin = norm_selected;
                best.clone_from(&x);
                best_step = step as f64;
            }
            match check {
                Check::Converged => {
                    flag = 0;
                    break;
                }
                Check::Stagnated => {
                    flag = 3;
                    break;
                }
                Check::Continue => {}
            }
            raw_next = next_raw;
            pre_next = next_pre;
            beta = next_beta;
            previous_cosine = cosine;
            previous_sine = sine;
            previous_zeta = zeta;
        }
        let completed = (history.len() - 1) as f64;
        let (x, iter, relres) = slv_try!(
            self.slv_finish(p, flag, &x, &best, best_step, completed, norm_selected, out),
            p,
            x,
            slv_history_with_last(&history, norm_selected),
            cg_history,
            stopped.unwrap_or(Count::One(0.))
        );
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec: history,
            sixth: cg_history,
            stopped,
        })
    }

    /// One preconditioned Lanczos column, Paige and Saunders (1975), section 3.
    /// `dual = M*v`; keeping both vectors avoids forming M or its square root.
    /// Return the signed next squared norm so the caller can report flag 5.
    fn slv_cr_lanczos(
        &mut self,
        p: &Problem,
        v: &[f64],
        dual: &[f64],
        previous: Option<(&[f64], f64)>,
        out: &mut Execution,
    ) -> SlvResult<(f64, f64, Vec<f64>, Vec<f64>)> {
        let mut raw = self.slv_mul(p, v, out)?;
        if let Some((old_dual, beta)) = previous {
            raw = slv_axpy(-beta, old_dual, &raw);
        }
        let alpha = slv_dot(v, &raw);
        raw = slv_axpy(-alpha, dual, &raw);
        let pre = self.slv_both(p, &raw, out)?;
        Ok((alpha, slv_dot(&raw, &pre), raw, pre))
    }

    // -------------------------------------------------------------- lsqr

    /// Paige and Saunders, LSQR (1982), sections 3 and 4.1: Golub--Kahan
    /// bidiagonalization and plane rotations. Norm estimates follow section 5.1;
    /// the least-squares stopping test follows section 6.
    fn slv_lsqr(&mut self, p: &Problem, out: &mut Execution) -> Result<Solved> {
        let mut x = p.x0.clone();
        let mut history = vec![p.n2b];
        let mut ls_history = Vec::new();
        let residual = match &p.initial_residual {
            Some(r) => r.clone(),
            None => slv_try!(
                self.slv_residual(p, &x, out),
                p,
                x,
                history,
                ls_history,
                Count::One(0.)
            ),
        };
        let mut beta = slv_norm(&residual);
        history[0] = beta;
        if beta <= p.tol * p.n2b {
            let mut solved = Self::slv_initial(p, x, beta);
            solved.sixth.clear();
            return Ok(solved);
        }
        let mut u = slv_div(&residual, beta);
        let adjoint = slv_try!(
            self.slv_mul_t(p, &u, out),
            p,
            x,
            history,
            ls_history,
            Count::One(0.)
        );
        let mut v = slv_try!(
            self.slv_both_t(p, &adjoint, out),
            p,
            x,
            history,
            ls_history,
            Count::One(0.)
        );
        let mut alpha = slv_norm(&v);
        if alpha == 0. {
            // The initial point already satisfies the normal equations. A
            // nonzero start keeps its true residual; the zero start reports 0.
            let zero_start = x.iter().all(|&value| value == 0.);
            if p.maxit == 0 {
                history.clear();
            }
            return Ok(Solved {
                x,
                flag: 0,
                relres: if zero_start { 0. } else { beta / p.n2b },
                iter: vec![0.],
                resvec: history,
                sixth: ls_history,
                stopped: None,
            });
        }
        let mut flag = 1;
        let mut stopped = None;
        if alpha.is_finite() && beta.is_finite() {
            v = slv_div(&v, alpha);
            // The search direction lives in the unpreconditioned space:
            // d = M\w, advanced with the same recurrence as w (section 4.1)
            // from the M\v the operator product needs anyway.
            let mut direction = Vec::new();
            let mut direction_ratio = 0.;
            let mut rho_bar = alpha;
            let mut phi_bar = beta;
            let mut norm_a = 0.;
            let mut norm_ar = alpha * beta;
            let mut stagnation = 0;
            for step in 1..=p.maxit {
                self.slv_step(p.n.max(p.b.len()))?;
                let position = Count::One(step as f64);
                stopped = Some(position);
                // Right preconditioning: B = A M^-1 and B' = M^-T A'.
                let physical_v = slv_try!(
                    self.slv_both(p, &v, out),
                    p,
                    x,
                    history,
                    ls_history,
                    position
                );
                direction = if step == 1 {
                    physical_v.clone()
                } else {
                    slv_axpy(-direction_ratio, &direction, &physical_v)
                };
                let image = slv_try!(
                    self.slv_mul(p, &physical_v, out),
                    p,
                    x,
                    history,
                    ls_history,
                    position
                );
                u = slv_axpy(-alpha, &u, &image);
                beta = slv_norm(&u);
                norm_a = slv_norm(&[norm_a, alpha, beta]);
                if !beta.is_finite() || !norm_a.is_finite() || !norm_ar.is_finite() {
                    flag = 4;
                    break;
                }
                if beta != 0. {
                    u = slv_div(&u, beta);
                }
                let adjoint = slv_try!(
                    self.slv_mul_t(p, &u, out),
                    p,
                    x,
                    history,
                    ls_history,
                    position
                );
                let next_v = slv_try!(
                    self.slv_both_t(p, &adjoint, out),
                    p,
                    x,
                    history,
                    ls_history,
                    position
                );
                v = slv_axpy(-beta, &v, &next_v);
                alpha = slv_norm(&v);
                if !alpha.is_finite() {
                    flag = 4;
                    break;
                }
                if alpha != 0. {
                    v = slv_div(&v, alpha);
                }
                let rho = slv_norm2(rho_bar, beta);
                if rho == 0. || !rho.is_finite() {
                    flag = 4;
                    break;
                }
                let cosine = rho_bar / rho;
                let sine = beta / rho;
                let theta = sine * alpha;
                rho_bar = -cosine * alpha;
                let phi = cosine * phi_bar;
                phi_bar *= sine;
                let scale = phi / rho;
                if !scale.is_finite() {
                    flag = 4;
                    break;
                }
                stagnation = if scale.abs() * slv_norm(&direction) < EPS * slv_norm(&x) {
                    stagnation + 1
                } else {
                    0
                };
                let next_x = slv_axpy(scale, &direction, &x);
                if !slv_finite(&next_x) {
                    flag = 4;
                    break;
                }
                x = next_x;
                direction_ratio = theta / rho;
                history.push(phi_bar.abs());
                // Public lsvec samples the preceding normal residual, scaled
                // by the newly available Frobenius norm estimate (section 5.3).
                ls_history.push(norm_ar / norm_a);
                norm_ar = alpha * (cosine * phi_bar).abs();
                if phi_bar.abs() <= p.tol * p.n2b || alpha * cosine.abs() <= p.tol * norm_a {
                    flag = 0;
                    break;
                }
                if stagnation >= MAX_STAG_STEPS {
                    flag = 3;
                    break;
                }
            }
        } else {
            // A non-finite first alpha is reported before any iteration.
            flag = 4;
            stopped = Some(Count::One(0.));
        }
        let relres = history.last().copied().unwrap() / p.n2b;
        let iter = (history.len() - 1) as f64;
        // Without an iteration there is no residual history.
        if p.maxit == 0 {
            history.clear();
        }
        Ok(Solved {
            x,
            flag,
            relres,
            iter: vec![iter],
            resvec: history,
            sixth: ls_history,
            stopped,
        })
    }

    /// `M1'\(M2'\x)`: the adjoint of `slv_both`, `M2\(M1\x)`, applies the
    /// factors in the reverse order.
    fn slv_both_t(&mut self, p: &Problem, x: &[f64], out: &mut Execution) -> SlvResult<Vec<f64>> {
        let mut y = x.to_vec();
        if let Some(m2) = &p.m2 {
            y = self.slv_precondition(p, m2, &y, true, out)?;
        }
        if let Some(m1) = &p.m1 {
            y = self.slv_precondition(p, m1, &y, true, out)?;
        }
        Ok(y)
    }
}

#[cfg(test)]
mod clean_room_tests {
    use super::*;

    pub(super) fn problem(name: &'static str, n: usize, a: &[f64], b: &[f64]) -> Problem {
        Problem {
            name,
            a: dense(n, a),
            b: b.to_vec(),
            n,
            tol: 1e-10,
            maxit: 4 * n,
            maxit_value: (4 * n) as f64,
            m1: None,
            m2: None,
            x0: vec![0.; n],
            initial_residual: None,
            extra: Vec::new(),
            warned: false,
            n2b: slv_norm(b),
            restarted: false,
            restart: 0,
            inner: 0,
            outer: 0,
        }
    }

    fn dense(n: usize, data: &[f64]) -> Operator {
        Operator::Dense {
            rows: n,
            cols: n,
            data: Matrix::from_column_slice(n, n, data).unwrap(),
        }
    }

    pub(super) fn solve(engine: &mut Engine, p: &Problem) -> Solved {
        let out = &mut Execution::default();
        match p.name {
            "cgs" => engine.slv_cgs(p, out),
            "qmr" => engine.slv_qmr(p, out),
            "tfqmr" => engine.slv_tfqmr(p, out),
            _ => unreachable!(),
        }
        .unwrap()
    }

    fn close(actual: &[f64], expected: &[f64], tolerance: f64) {
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            assert!((a - b).abs() <= tolerance, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn nonsymmetric_system_with_noncommuting_split_factors() {
        // Exact solution [1, -2, 3, 1/2], from integer/rational arithmetic.
        let a = [
            4., 2., 0., 1., 1., 5., -1., 0., 0., 1., 3., 2., 0., 0., 1., 6.,
        ];
        let b = [2., -5., 11.5, 10.];
        let truth = [1., -2., 3., 0.5];
        for name in ["cgs", "qmr", "tfqmr"] {
            for split in [false, true] {
                let mut p = problem(name, 4, &a, &b);
                p.x0 = vec![0.5, 1., -1., 2.];
                if split {
                    p.m1 = Some(dense(
                        4,
                        &[
                            2., 0.5, 0., 0., 0., 3., 0.5, 0., 0., 0., 2., 0.5, 0., 0., 0., 3.,
                        ],
                    ));
                    p.m2 = Some(dense(
                        4,
                        &[
                            2., 0., 0., 0., -0.25, 2., 0., 0., 0., -0.25, 2., 0., 0., 0., -0.25, 2.,
                        ],
                    ));
                }
                let mut engine = Engine::default();
                let s = solve(&mut engine, &p);
                assert_eq!(s.flag, 0, "{name}, split={split}");
                close(&s.x, &truth, 1e-8);
                let actual = engine
                    .slv_residual(&p, &s.x, &mut Execution::default())
                    .ok()
                    .unwrap();
                assert!(slv_norm(&actual) / p.n2b <= p.tol);
                assert!((s.relres - slv_norm(&actual) / p.n2b).abs() <= 1e-14);
                assert_eq!(s.iter, vec![slv_completed(&p, s.resvec.len())]);
            }
        }
    }

    #[test]
    fn published_recurrences_match_recorded_residual_histories() {
        // Output facts from tests/fixtures/fn/solvers-expected.txt, cases
        // 135, 152, 170. These are historical oracle data, not a new oracle run.
        for (name, expected, iter) in [
            (
                "cgs",
                &[3.741657386773941, 0.542545319245014, 0.024299272816881, 0.][..],
                3.,
            ),
            (
                "qmr",
                &[3.741657386773941, 1.239616559529473, 0.184500024342452, 0.][..],
                3.,
            ),
            (
                "tfqmr",
                &[
                    3.741657386773941,
                    1.239616559529472,
                    0.627111264445580,
                    0.091297457768649,
                    0.025125526345106,
                    0.,
                ][..],
                2.,
            ),
        ] {
            let p = problem(
                name,
                3,
                &[4., 2., 0., 1., 5., 1., 0., 1., 3.],
                &[1., 2., 3.],
            );
            let s = solve(&mut Engine::default(), &p);
            assert_eq!(s.flag, 0, "{name}");
            assert_eq!(s.iter, vec![iter]);
            close(&s.x, &[0.22, 0.12, 0.96], 1e-14);
            close(&s.resvec, expected, 1e-13);
        }
    }

    #[test]
    fn finite_iteration_limit_retains_history_and_best_point() {
        for name in ["cgs", "qmr", "tfqmr"] {
            let mut p = problem(
                name,
                3,
                &[4., 2., 0., 1., 5., 1., 0., 1., 3.],
                &[1., 2., 3.],
            );
            p.maxit = 1;
            p.maxit_value = 1.;
            let s = solve(&mut Engine::default(), &p);
            assert_eq!(s.flag, 1);
            assert_eq!(s.iter, vec![1.]);
            let steps = if name == "tfqmr" { 2 } else { 1 };
            assert_eq!(s.resvec.len(), steps + 1);
            assert!((s.relres * p.n2b - s.resvec[steps]).abs() < 1e-13);
            assert!(matches!(s.stopped, Some(Count::One(k)) if k == steps as f64));
        }
    }

    #[test]
    fn zero_iteration_limit_has_no_history_and_no_loop_count() {
        // As in pcg and bicg: the loop never runs, so there is no iteration to
        // report and (except tfqmr) the history is empty, while x0 is returned unchanged.
        for name in ["cgs", "qmr", "tfqmr"] {
            let mut p = problem(
                name,
                3,
                &[4., 2., 0., 1., 5., 1., 0., 1., 3.],
                &[1., 2., 3.],
            );
            p.maxit = 0;
            p.maxit_value = 0.;
            let s = solve(&mut Engine::default(), &p);
            assert_eq!(s.flag, 1, "{name}");
            assert_eq!(s.iter, vec![0.], "{name}");
            // tfqmr keeps its preallocated first history entry, like bicgstab
            // (tests/fn_solvers.rs zero_iterations_keep_initial_residual_for_inline_histories).
            assert_eq!(s.resvec.is_empty(), name != "tfqmr", "{name}");
            assert!(s.stopped.is_none(), "{name}");
            assert_eq!(s.x, p.x0, "{name}");
            assert_eq!(s.relres, 1., "{name}");
        }
    }

    #[test]
    fn tfqmr_zero_shadow_product_after_an_iteration_is_breakdown() {
        // Lower triangular A and shadow e1: after the first iteration the
        // residual w = [0; 1/4] is orthogonal to the shadow (exact in binary).
        let p = problem("tfqmr", 2, &[2., 1., 0., 3.], &[1., 0.]);
        let s = solve(&mut Engine::default(), &p);
        assert_eq!(s.flag, 4);
        assert_eq!(s.resvec.len(), 3);
        assert!(matches!(s.stopped, Some(Count::One(k)) if k == 2.));
    }

    #[test]
    fn exact_initial_guess_and_tfqmr_half_step_count() {
        for name in ["cgs", "qmr", "tfqmr"] {
            let mut p = problem(name, 2, &[2., 0., 0., 2.], &[1., 2.]);
            p.x0 = vec![0.5, 1.];
            // A successful initial guess does not touch a singular preconditioner.
            p.m1 = Some(dense(2, &[0.; 4]));
            let s = solve(&mut Engine::default(), &p);
            assert_eq!(s.flag, 0);
            assert_eq!(s.iter, vec![0.]);
            assert_eq!(s.resvec, vec![0.]);
        }
        let p = problem(
            "tfqmr",
            3,
            &[2., 0., 0., 0., 2., 0., 0., 0., 2.],
            &[1., 2., 3.],
        );
        let s = solve(&mut Engine::default(), &p);
        assert_eq!(s.flag, 0);
        assert_eq!(s.relres, 0.);
        assert_eq!(s.iter, vec![0.]);
        assert_eq!(s.resvec.len(), 2);
    }

    #[test]
    fn breakdown_and_singular_preconditioners_keep_initial_state() {
        for name in ["cgs", "qmr", "tfqmr"] {
            // A skew-symmetric operator is orthogonal to the starting shadow.
            let p = problem(name, 2, &[0., 1., -1., 0.], &[1., 2.]);
            let s = solve(&mut Engine::default(), &p);
            assert_eq!(s.flag, 4, "{name}");
            assert_eq!(s.x, p.x0);
            assert_eq!(s.iter, vec![0.]);
            assert_eq!(s.resvec, vec![p.n2b]);
            for second in [false, true] {
                let mut p = problem(name, 2, &[4., 2., 1., 5.], &[1., 2.]);
                if second {
                    p.m2 = Some(dense(2, &[0.; 4]));
                } else {
                    p.m1 = Some(dense(2, &[0.; 4]));
                }
                let s = solve(&mut Engine::default(), &p);
                assert_eq!(s.flag, 2, "{name}");
                assert_eq!(s.x, p.x0);
                assert_eq!(s.iter, vec![0.]);
                assert_eq!(s.resvec, vec![p.n2b]);
            }
        }
    }

    #[test]
    fn larger_nonsymmetric_tridiagonal_system() {
        let n = 200;
        let mut a = vec![0.; n * n];
        let mut b = vec![2.5; n];
        b[0] = 3.;
        b[n - 1] = 3.5;
        for i in 0..n {
            a[i * n + i] = 4.;
            if i + 1 < n {
                a[i * n + i + 1] = -0.5;
                a[(i + 1) * n + i] = -1.;
            }
        }
        for name in ["cgs", "qmr", "tfqmr"] {
            let mut p = problem(name, n, &a, &b);
            p.maxit = 100;
            p.maxit_value = 100.;
            let s = solve(&mut Engine::default(), &p);
            assert_eq!(s.flag, 0, "{name}");
            close(&s.x, &vec![1.; n], 1e-8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printed(source: &str, show: &str) -> String {
        let mut engine = Engine::default();
        engine
            .execute(source)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        let shown = engine.execute(show).unwrap();
        shown
            .output
            .iter()
            .map(|entry| format!("{entry}\n"))
            .collect::<String>()
            .trim()
            .to_string()
    }

    #[test]
    fn gmres_workspace_counts_reflectors_triangle_and_vectors() {
        // 4000 reflectors of length 4000 alone fit, the triangular R does not.
        const { assert!(4000 * 4001 <= MAX_ELEMENTS) };
        assert!(slv_gmres_workspace(4000, 4000) > MAX_ELEMENTS);
        assert!(slv_gmres_workspace(4000, 2000) <= MAX_ELEMENTS);
        let error = Engine::default()
            .execute("[x, f] = gmres(@(v) 2*v, ones(4000, 1), [], 1e-6, 4000);")
            .unwrap_err();
        assert!(error.contains("element limit"), "{error}");
        assert_eq!(
            printed(
                "[x, f] = gmres(@(v) 2*v, ones(4000, 1), [], 1e-6, 2000);",
                "fprintf('%g', f)"
            ),
            "0"
        );
    }

    #[test]
    fn gmres_charges_the_cycle_workspace() {
        // A converges in one step, so the steps and operator calls charge a
        // few vectors; the workspace of the cycle comes on top.
        let setup = "b = ones(1000, 1);";
        let mut engine = Engine::default();
        engine.execute(setup).unwrap();
        let before = engine.evaluated_elements;
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "{setup} [x, f] = gmres(@(v) 2*v, b, [], 1e-6, 1);"
            ))
            .unwrap();
        let charged = engine.evaluated_elements - before;
        assert!(charged >= slv_gmres_workspace(1000, 1), "{charged}");
    }

    #[test]
    fn gmres_overflowing_solution_keeps_the_last_finite_iterate() {
        // The least-squares solution is b/1e-308: it overflows in the last
        // cycle, so the unchanged starting guess is returned with its residual.
        assert_eq!(
            printed(
                "[x, f, r, it, rv] = gmres(diag([1e-308 2e-308]), [10; 10], [], 1e-6, 1);",
                "fprintf('%g %g %g %g %g', f, all(isfinite(x)), r, numel(rv), norm(x))"
            ),
            "4 1 1 1 0"
        );
        // Step 1 is finite, the step-2 solution (1e303/1e-10) overflows: step 1
        // is returned and the history stops at its residual. With restart 1 the
        // same happens in the second cycle, which returns the first cycle's x.
        // With a third eigenvalue, step 2 overflows as well and is skipped.
        let show = "fprintf('%g %g %g %g %g %g', f, all(isfinite(x)), numel(rv), it(1) * 10 + it(2), abs(norm(b - A*x) / norm(b) - r) < 1e-12, abs(rv(end) / norm(b) - r) < 1e-12)";
        for (diagonal, restart) in [
            ("[1 1e-10]", "[]"),
            ("[1 1e-10]", "1"),
            ("[1 1e-10 1e-20]", "[]"),
        ] {
            let source = format!(
                "A = diag({diagonal}); b = 1e303 * ones(numel(A(:,1)), 1); [x, f, r, it, rv] = gmres(A, b, {restart}, 1e-6, 3);"
            );
            assert_eq!(
                printed(&source, show),
                "4 1 2 11 1 1",
                "{diagonal}, restart {restart}"
            );
        }
    }

    #[test]
    fn gmres_failed_operator_call_reports_the_returned_iterate() {
        // The operator returns Inf on one call. Whichever call that is, the
        // iterate that comes back is finite and its true relative residual,
        // relres and the last resvec entry agree. In the first system the
        // step-2 solution overflows, so a failure in step 3 must not return it.
        let show = "fprintf('%g %g %g %g %g', f, all(isfinite(x)), calls, abs(norm(b - A * x) / norm(b) - r) < 1e-10, abs(rv(end) / norm(b) - r) < 1e-10)";
        for (diagonal, rhs, tol) in [
            ("[1 1e-10 1e-20]", "1e303 * ones(3, 1)", "1e-6"),
            ("[2 3 4]", "[1; 2; 3]", "1e-10"),
            ("[1 2 4]", "[1; 1; 1]", "1e-10"),
        ] {
            let program = |fail: usize| {
                format!(
                    "global calls; calls = 0; A = diag({diagonal}); b = {rhs}; [x, f, r, it, rv] = gmres(@checked, b, [], {tol}, 3); function y = checked(v), global calls; calls = calls + 1; if calls == {fail}, y = Inf * ones(size(v)); else, y = diag({diagonal}) * v; end; end"
                )
            };
            let count: usize = printed(&program(0), "fprintf('%g', calls)")
                .parse()
                .unwrap();
            assert!(count >= 4, "{diagonal}: {count} calls");
            for fail in 1..=count {
                assert_eq!(
                    printed(&program(fail), show),
                    format!("2 1 {fail} 1 1"),
                    "{diagonal}, call {fail}/{count}"
                );
            }
        }
    }

    #[test]
    fn bicgstabl_failed_residual_check_reports_the_returned_iterate() {
        let truth = "abs(norm([1;2;3]-diag([2 3 4])*x)-r*norm([1;2;3]))<1e-10";
        let last = "abs(rv(end)-r*norm([1;2;3]))<1e-10";
        let show = format!("fprintf('%g %g %g %g %g',f,all(isfinite(x)),calls,{truth},{last})");
        let program = |tol: &str, maxit: usize, fail: usize| {
            format!(
                "global calls; calls=0; [x,f,r,it,rv]=bicgstabl(@checked,[1;2;3],{tol},{maxit}); function y=checked(v), global calls; calls=calls+1; if calls=={fail}, y=Inf*ones(size(v)); else, y=diag([2 3 4])*v; end; end"
            )
        };
        // Call 6 is the true-residual check after the second minimal-residual
        // update; the first one left x with a residual that rv did not record.
        assert_eq!(printed(&program("0.01", 2, 6), &show), "2 1 6 1 1");
        for (tol, maxit) in [("0.01", 2), ("0.05", 3), ("0.001", 2), ("1e-10", 3)] {
            let count: usize = printed(&program(tol, maxit, 0), "fprintf('%g',calls)")
                .parse()
                .unwrap();
            for fail in 1..=count {
                assert_eq!(
                    printed(&program(tol, maxit, fail), &show),
                    format!("2 1 {fail} 1 1"),
                    "tol {tol}, maxit {maxit}, call {fail}/{count}"
                );
            }
        }
    }
}
