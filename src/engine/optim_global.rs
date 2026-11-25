//! Derivative-free and stochastic global optimization:
//! - patternsearch: generalized pattern search with the 2N positive basis
//!   [I -I] (Torczon 1997, SIAM J. Optim. 7, 1-25; Audet & Dennis 2003,
//!   SIAM J. Optim. 13, 889-903), plus tangent-cone generators for nearly
//!   active linear constraints (Lewis & Torczon 2000, SIAM J. Optim. 10,
//!   917-941). Infeasible poll points are rejected.
//! - particleswarm: Kennedy & Eberhart (1995) with an inertia weight (Shi &
//!   Eberhart 1998) adapted between 0.1 and 1.1 and random neighbourhoods.
//! - ga: real-coded genetic algorithm (Goldberg 1989) with binary tournaments,
//!   elitism and Deb's (2000) feasibility rules for constraint violations.
//! - simulannealbnd: Kirkpatrick, Gelatt & Vecchi (1983) with exponential
//!   cooling and Ingber's (1989) sensitivity reannealing.
//!
//! Every random number comes from the engine's rand/randn stream, so
//! rng(seed) repeats a run. Helpers are free functions with an og_ prefix.
use super::optim_nl::{self as onl, Display, Options};
use super::{Engine, Execution, Value};
use crate::{Array, CharacterRow, FunctionHandle, Matrix, Result, StructArray};
use nalgebra::{DMatrix, DVector};
use std::sync::Arc;

const NAMES: &[&str] = &["patternsearch", "ga", "particleswarm", "simulannealbnd"];
const MAX_VARIABLES: usize = 1000;
const MAX_POPULATION: usize = 10_000;
const MAX_ROWS: usize = 4000;
/// Population/swarm storage: individuals times variables.
const MAX_ELEMENTS: usize = 4_000_000;

pub(super) fn og_handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Single-output calls from `Engine::call`.
pub(super) fn og_call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if !og_handles(name) {
        return Ok(None);
    }
    Ok(Some(og_outputs(e, name, args, 1, out)?.remove(0)))
}

/// `[x, fval, exitflag, output, ...] = solver(args)`; the caller charges the outputs.
pub(super) fn og_outputs(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = if name == "ga" { 6 } else { 4 };
    if count > max {
        return Err(format!("{name} does not support {count} outputs"));
    }
    if args.iter().any(|v| matches!(v, Value::QuotedString(_))) {
        return Err("String array storage is not implemented; use char(...) explicitly".into());
    }
    let args = e.text_arguments(args.to_vec())?;
    let count = count.max(1);
    let mut values = match name {
        "patternsearch" => og_patternsearch(e, &args, out)?,
        "ga" => og_ga(e, &args, out)?,
        "particleswarm" => og_particleswarm(e, &args, out)?,
        _ => og_simulanneal(e, &args, out)?,
    };
    values.truncate(count);
    Ok(values)
}

// ---------------------------------------------------------------- options

#[derive(Clone)]
struct GOptions {
    display: Display,
    max_iterations: Option<f64>,
    max_evaluations: Option<f64>,
    function_tol: f64,
    step_tol: f64,
    mesh_tol: f64,
    initial_mesh: f64,
    constraint_tol: f64,
    stall: Option<f64>,
    swarm: Option<usize>,
    population: Option<usize>,
    temperature: Option<Vec<f64>>,
    reanneal: f64,
    elite: Option<usize>,
    crossover: f64,
}

impl Default for GOptions {
    fn default() -> Self {
        Self {
            display: Display::Final,
            max_iterations: None,
            max_evaluations: None,
            function_tol: 1e-6,
            step_tol: 1e-6,
            mesh_tol: 1e-6,
            initial_mesh: 1.,
            constraint_tol: 1e-6,
            stall: None,
            swarm: None,
            population: None,
            temperature: None,
            reanneal: 100.,
            elite: None,
            crossover: 0.8,
        }
    }
}

/// Options only these solvers use (canonical names; older optimset aliases).
fn og_key(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "maxgenerations" | "generations" => "MaxGenerations",
        "maxstallgenerations" | "stallgenlimit" => "MaxStallGenerations",
        "maxstalliterations" | "stalliterlimit" => "MaxStallIterations",
        "swarmsize" => "SwarmSize",
        "populationsize" => "PopulationSize",
        "initialtemperature" => "InitialTemperature",
        "reannealinterval" => "ReannealInterval",
        "initialmeshsize" => "InitialMeshSize",
        "meshtolerance" | "tolmesh" => "MeshTolerance",
        "elitecount" => "EliteCount",
        "crossoverfraction" => "CrossoverFraction",
        "constrainttolerance" | "tolcon" => "ConstraintTolerance",
        _ => return None,
    })
}

/// Shared option names these solvers accept.
fn og_base_key(name: &str) -> Option<&'static str> {
    onl::canonical(name).filter(|k| {
        matches!(
            *k,
            "Display"
                | "MaxIterations"
                | "MaxFunctionEvaluations"
                | "FunctionTolerance"
                | "StepTolerance"
                | "TolFun"
        )
    })
}

fn og_applies(who: &str, key: &str) -> bool {
    if !og_handles(who) {
        return true;
    }
    match key {
        "Display" | "MaxIterations" | "MaxFunctionEvaluations" | "FunctionTolerance" | "TolFun" => {
            true
        }
        "MaxGenerations"
        | "MaxStallGenerations"
        | "PopulationSize"
        | "EliteCount"
        | "CrossoverFraction" => who == "ga",
        "MaxStallIterations" => who == "particleswarm" || who == "simulannealbnd",
        "SwarmSize" => who == "particleswarm",
        "InitialTemperature" | "ReannealInterval" => who == "simulannealbnd",
        "InitialMeshSize" | "MeshTolerance" | "StepTolerance" => who == "patternsearch",
        "ConstraintTolerance" => who == "ga" || who == "patternsearch",
        _ => false,
    }
}

fn og_positive_integer(who: &str, key: &str, v: &Value, allow_inf: bool) -> Result<f64> {
    match onl::onl_scalar(v) {
        Some(x) if x >= 1. && (x.fract() == 0. || (allow_inf && x == f64::INFINITY)) => Ok(x),
        _ => Err(format!(
            "{who}: {key} must be a positive integer{}",
            if allow_inf { " or Inf" } else { "" }
        )),
    }
}

fn og_positive(who: &str, key: &str, v: &Value) -> Result<f64> {
    match onl::onl_scalar(v) {
        Some(x) if x > 0. && x.is_finite() => Ok(x),
        _ => Err(format!("{who}: {key} must be a finite positive scalar")),
    }
}

/// Apply one canonical option; empty values keep the default.
fn og_set(e: &Engine, o: &mut GOptions, who: &str, key: &str, v: &Value) -> Result<()> {
    if onl::is_empty_value(v) {
        return Ok(());
    }
    match key {
        "Display"
        | "MaxIterations"
        | "MaxFunctionEvaluations"
        | "FunctionTolerance"
        | "StepTolerance"
        | "TolFun" => {
            let mut base = Options::default();
            onl::onl_set_option(e, &mut base, who, key, v)?;
            match key {
                "Display" => o.display = base.display,
                "MaxIterations" => {
                    o.max_iterations = Some(og_positive_integer(who, key, v, true)?);
                }
                "MaxFunctionEvaluations" => {
                    o.max_evaluations = Some(og_positive_integer(who, key, v, true)?);
                }
                "StepTolerance" => o.step_tol = base.step_tol,
                _ => o.function_tol = base.function_tol,
            }
        }
        "MaxGenerations" => o.max_iterations = Some(og_positive_integer(who, key, v, true)?),
        "MaxStallGenerations" | "MaxStallIterations" => {
            o.stall = Some(og_positive_integer(who, key, v, false)?);
        }
        "SwarmSize" | "PopulationSize" => {
            let x = og_positive_integer(who, key, v, false)?;
            if !(2. ..=MAX_POPULATION as f64).contains(&x) {
                return Err(format!(
                    "{who}: {key} must be an integer between 2 and {MAX_POPULATION}"
                ));
            }
            if key == "SwarmSize" {
                o.swarm = Some(x as usize);
            } else {
                o.population = Some(x as usize);
            }
        }
        "EliteCount" => match onl::onl_scalar(v) {
            Some(x) if x >= 0. && x.fract() == 0. && x <= MAX_POPULATION as f64 => {
                o.elite = Some(x as usize);
            }
            _ => return Err(format!("{who}: EliteCount must be a nonnegative integer")),
        },
        "CrossoverFraction" => match onl::onl_scalar(v) {
            Some(x) if (0. ..=1.).contains(&x) => o.crossover = x,
            _ => return Err(format!("{who}: CrossoverFraction must be between 0 and 1")),
        },
        "InitialTemperature" => {
            let m = onl::onl_real_vector(v, who, "InitialTemperature")?;
            if m.is_empty()
                || (m.rows() != 1 && m.cols() != 1)
                || !m.data().iter().all(|t| t.is_finite() && *t > 0.)
            {
                return Err(format!(
                    "{who}: InitialTemperature must be a finite positive scalar or vector"
                ));
            }
            o.temperature = Some(m.data().to_vec());
        }
        "ReannealInterval" => o.reanneal = og_positive_integer(who, key, v, false)?,
        "InitialMeshSize" => o.initial_mesh = og_positive(who, key, v)?,
        "MeshTolerance" => o.mesh_tol = og_positive(who, key, v)?,
        "ConstraintTolerance" => o.constraint_tol = og_positive(who, key, v)?,
        _ => return Err(format!("{who}: option '{key}' is not implemented")),
    }
    Ok(())
}

/// Options from an optimoptions/optimset structure, or [] for the defaults.
fn og_options(e: &Engine, who: &str, v: Option<&Value>) -> Result<GOptions> {
    let mut o = GOptions::default();
    let Some(v) = v else {
        return Ok(o);
    };
    if onl::is_empty_value(v) {
        return Ok(o);
    }
    let Value::Container(Array::Struct(s)) = v else {
        return Err(format!(
            "{who}: options must be a scalar structure from optimoptions or optimset"
        ));
    };
    if s.len() != 1 {
        return Err(format!("{who}: options must be a scalar structure"));
    }
    for (i, name) in s.field_names().iter().enumerate() {
        e.check_cancel()?;
        let value = Value::from(s.get(i, 0)?.as_ref().clone());
        if onl::is_empty_value(&value) {
            continue;
        }
        let Some(key) = og_key(name).or_else(|| og_base_key(name)) else {
            return Err(format!("{who}: option '{name}' is not implemented"));
        };
        if !og_applies(who, key) {
            return Err(format!("{who}: option '{key}' does not apply to {who}"));
        }
        og_set(e, &mut o, who, key, &value)?;
    }
    Ok(o)
}

/// optimoptions for ga, particleswarm, simulannealbnd and patternsearch, or an
/// options structure that already holds their options. None leaves the call
/// to the other optimoptions builders.
pub(super) fn og_options_builder(e: &Engine, args: &[Value]) -> Result<Option<Value>> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    let solver = match first {
        Value::Container(Array::Handle(h)) => Some(h.func2str().to_string()),
        Value::Container(Array::Struct(_)) => None,
        other => onl::onl_text(e, other),
    };
    let own = |n: &str| og_key(n).is_some_and(|k| k != "ConstraintTolerance");
    let is_struct = matches!(first, Value::Container(Array::Struct(_)));
    let struct_own = matches!(first, Value::Container(Array::Struct(s)) if s.field_names().iter().any(|n| own(n)));
    let pairs_own = args[1..]
        .chunks(2)
        .any(|p| onl::onl_text(e, &p[0]).is_some_and(|n| own(&n)));
    let global = solver.as_deref().is_some_and(og_handles);
    if !global && !struct_own && !(is_struct && pairs_own) {
        return Ok(None);
    }
    if !(args.len() - 1).is_multiple_of(2) {
        return Err("Arguments must occur in name-value pairs.".into());
    }
    let who = solver.unwrap_or_else(|| "optimoptions".into());
    let mut fields: Vec<(String, Arc<Array>)> = Vec::new();
    if let Value::Container(Array::Struct(s)) = first {
        if s.len() != 1 {
            return Err("optimoptions: expected a scalar structure".into());
        }
        for (i, n) in s.field_names().iter().enumerate() {
            fields.push((n.clone(), s.get(i, 0)?.clone()));
        }
    }
    let mut check = GOptions::default();
    for p in args[1..].chunks(2) {
        e.check_cancel()?;
        let n = onl::onl_text(e, &p[0]).ok_or("optimoptions: option names must be text")?;
        let key = og_key(&n)
            .or_else(|| og_base_key(&n))
            .ok_or_else(|| format!("optimoptions: unknown option '{n}' for {who}"))?;
        if !og_applies(&who, key) {
            return Err(format!(
                "optimoptions: option '{key}' does not apply to {who}"
            ));
        }
        og_set(e, &mut check, "optimoptions", key, &p[1])?;
        let key = if key == "TolFun" {
            "FunctionTolerance"
        } else {
            key
        };
        let value = Arc::new(p[1].clone().array(&e.cancel)?);
        if let Some(f) = fields.iter_mut().find(|f| {
            f.0.eq_ignore_ascii_case(key)
                || og_key(&f.0) == Some(key)
                || og_base_key(&f.0) == Some(key)
        }) {
            *f = (key.into(), value);
        } else {
            fields.push((key.into(), value));
        }
    }
    let (names, slots) = fields.into_iter().unzip();
    Ok(Some(Value::Container(Array::Struct(
        StructArray::from_slots(1, 1, names, slots, &e.cancel)?,
    ))))
}

// ---------------------------------------------------------------- problem

fn og_matrix(v: &Value, who: &str, what: &str) -> Result<DMatrix<f64>> {
    let m = onl::onl_real_vector(v, who, what)?;
    onl::onl_check_finite(who, what, m.data())?;
    if m.len() > MAX_ELEMENTS {
        return Err(format!("{who}: {what} is too large"));
    }
    Ok(DMatrix::from_column_slice(m.rows(), m.cols(), m.data()))
}

fn og_vector(v: &Value, who: &str, what: &str) -> Result<Vec<f64>> {
    let m = og_matrix(v, who, what)?;
    if !m.is_empty() && m.nrows() != 1 && m.ncols() != 1 {
        return Err(format!("{who}: {what} must be a vector"));
    }
    Ok(m.as_slice().to_vec())
}

/// Bounds, linear inequalities A*x <= b and equalities Aeq*x = beq.
struct Lin {
    a: DMatrix<f64>,
    b: Vec<f64>,
    aeq: DMatrix<f64>,
    beq: Vec<f64>,
    lb: Vec<f64>,
    ub: Vec<f64>,
}

impl Lin {
    fn general(&self) -> bool {
        self.a.nrows() + self.aeq.nrows() > 0
    }
    fn in_bounds(&self, x: &[f64]) -> bool {
        x.iter()
            .zip(self.lb.iter().zip(&self.ub))
            .all(|(v, (l, u))| *v >= *l && *v <= *u)
    }
    /// Largest violation of the bounds and linear rows.
    fn violation(&self, x: &[f64]) -> f64 {
        let mut worst = 0f64;
        for (i, v) in x.iter().enumerate() {
            worst = worst.max(self.lb[i] - v).max(v - self.ub[i]);
        }
        for r in 0..self.a.nrows() {
            worst = worst.max(og_row_dot(&self.a, r, x) - self.b[r]);
        }
        for r in 0..self.aeq.nrows() {
            worst = worst.max((og_row_dot(&self.aeq, r, x) - self.beq[r]).abs());
        }
        worst
    }
    fn clamp(&self, x: &mut [f64]) {
        for (i, v) in x.iter_mut().enumerate() {
            *v = v.clamp(self.lb[i], self.ub[i]);
        }
    }
    /// Repair by cyclic projections onto the violated rows and the box
    /// (Bregman 1965 / POCS). Feasible points are returned unchanged.
    fn repair(&self, x: &mut [f64]) {
        self.clamp(x);
        if !self.general() {
            return;
        }
        for _ in 0..200 {
            let mut worst = 0f64;
            for r in 0..self.a.nrows() {
                let excess = og_row_dot(&self.a, r, x) - self.b[r];
                if excess > 0. {
                    og_row_project(&self.a, r, excess, x);
                    worst = worst.max(excess);
                }
            }
            for r in 0..self.aeq.nrows() {
                let excess = og_row_dot(&self.aeq, r, x) - self.beq[r];
                if excess != 0. {
                    og_row_project(&self.aeq, r, excess, x);
                    worst = worst.max(excess.abs());
                }
            }
            self.clamp(x);
            if worst <= 1e-13 {
                break;
            }
        }
    }
}

fn og_row_dot(a: &DMatrix<f64>, r: usize, x: &[f64]) -> f64 {
    (0..a.ncols()).map(|j| a[(r, j)] * x[j]).sum()
}

fn og_row_project(a: &DMatrix<f64>, r: usize, excess: f64, x: &mut [f64]) {
    let norm: f64 = (0..a.ncols()).map(|j| a[(r, j)] * a[(r, j)]).sum();
    if norm > 0. {
        for (j, v) in x.iter_mut().enumerate() {
            *v -= excess / norm * a[(r, j)];
        }
    }
}

fn og_bound(v: Option<&Value>, n: usize, lower: bool, who: &str) -> Result<Vec<f64>> {
    let default = if lower {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    let Some(v) = v else {
        return Ok(vec![default; n]);
    };
    let m = onl::onl_real_vector(v, who, if lower { "lb" } else { "ub" })?;
    if m.is_empty() {
        return Ok(vec![default; n]);
    }
    if m.len() != n || (m.rows() != 1 && m.cols() != 1) {
        return Err(format!("{who}: bounds must have {n} entries"));
    }
    if m.data().iter().any(|x| x.is_nan() || *x == -default) {
        return Err(format!("{who}: invalid infinite or NaN bound"));
    }
    Ok(m.data().to_vec())
}

/// `args` = [A, b, Aeq, beq, lb, ub], each optional.
fn og_linear(who: &str, n: usize, args: &[Value]) -> Result<Lin> {
    let pair =
        |a: Option<&Value>, b: Option<&Value>, what: &str| -> Result<(DMatrix<f64>, Vec<f64>)> {
            let a = a
                .map(|v| og_matrix(v, who, what))
                .transpose()?
                .unwrap_or_else(|| DMatrix::zeros(0, n));
            let b = b
                .map(|v| og_vector(v, who, what))
                .transpose()?
                .unwrap_or_default();
            if a.is_empty() && b.is_empty() {
                return Ok((DMatrix::zeros(0, n), b));
            }
            if a.ncols() != n || a.nrows() != b.len() {
                return Err(format!("{who}: {what} dimensions do not agree"));
            }
            if a.nrows() > MAX_ROWS {
                return Err(format!("{who}: {what} has more than {MAX_ROWS} rows"));
            }
            Ok((a, b))
        };
    let (a, b) = pair(args.first(), args.get(1), "A,b")?;
    let (aeq, beq) = pair(args.get(2), args.get(3), "Aeq,beq")?;
    let lb = og_bound(args.get(4), n, true, who)?;
    let ub = og_bound(args.get(5), n, false, who)?;
    if lb.iter().zip(&ub).any(|(l, u)| l > u) {
        return Err(format!("{who}: lb must not exceed ub"));
    }
    Ok(Lin {
        a,
        b,
        aeq,
        beq,
        lb,
        ub,
    })
}

fn og_nvars(e: &Engine, v: &Value, who: &str) -> Result<usize> {
    let _ = e;
    match onl::onl_scalar(v) {
        Some(x) if x >= 1. && x.fract() == 0. && x <= MAX_VARIABLES as f64 => Ok(x as usize),
        _ => Err(format!(
            "{who}: nvars must be a positive integer no larger than {MAX_VARIABLES}"
        )),
    }
}

fn og_x0(v: &Value, who: &str) -> Result<Matrix> {
    let x0 = onl::onl_real_vector(v, who, "x0")?;
    if x0.is_empty() {
        return Err(format!("{who}: the starting point x0 must not be empty"));
    }
    onl::onl_check_finite(who, "x0", x0.data())?;
    if x0.len() > MAX_VARIABLES {
        return Err(format!(
            "{who}: at most {MAX_VARIABLES} variables are supported"
        ));
    }
    Ok(x0)
}

/// Objective and nonlinear constraint callbacks with an evaluation count.
struct GProblem {
    who: &'static str,
    fun: FunctionHandle,
    nonlcon: Option<FunctionHandle>,
    shape: (usize, usize),
    evaluations: f64,
    sizes: Option<(usize, usize)>,
}

impl GProblem {
    fn new(
        e: &Engine,
        who: &'static str,
        fun: &Value,
        nonlcon: Option<&Value>,
        shape: (usize, usize),
    ) -> Result<Self> {
        Ok(Self {
            who,
            fun: onl::onl_handle(e, who, fun)?,
            nonlcon: match nonlcon {
                Some(v) if !onl::is_empty_value(v) => Some(onl::onl_handle(e, who, v)?),
                _ => None,
            },
            shape,
            evaluations: 0.,
            sizes: None,
        })
    }
    fn x_value(&self, x: &[f64]) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_slice(
            self.shape.0,
            self.shape.1,
            x,
        )?))
    }
    /// Objective value; NaN counts as +Inf (the worst value) so a search moves on.
    fn objective(&mut self, e: &mut Engine, x: &[f64], out: &mut Execution) -> Result<f64> {
        e.check_cancel()?;
        let who = self.who;
        let v = e.call_handle(&self.fun, vec![self.x_value(x)?], 1, None, out)?;
        self.evaluations += 1.;
        let value = v
            .first()
            .ok_or_else(|| format!("{who}: missing objective output"))?;
        if matches!(value, Value::Complex(_)) {
            return Err(format!("{who}: the objective must return a real scalar"));
        }
        let f = onl::onl_real_vector(value, who, "objective")?;
        if f.len() != 1 {
            return Err(format!("{who}: the objective must return a real scalar"));
        }
        let f = f.data()[0];
        Ok(if f.is_nan() { f64::INFINITY } else { f })
    }
    /// (largest violation, sum of squared violations) of nonlcon at x.
    fn nonlinear(&mut self, e: &mut Engine, x: &[f64], out: &mut Execution) -> Result<(f64, f64)> {
        let Some((c, ce)) = self.constraints(e, x, out)? else {
            return Ok((0., 0.));
        };
        let mut worst = 0f64;
        let mut squares = 0.;
        for v in c
            .iter()
            .map(|v| v.max(0.))
            .chain(ce.iter().map(|v| v.abs()))
        {
            worst = worst.max(v);
            squares += v * v;
        }
        Ok((worst, squares))
    }
    /// Raw nonlcon values [c, ceq] at x; None without nonlcon.
    fn constraints(
        &mut self,
        e: &mut Engine,
        x: &[f64],
        out: &mut Execution,
    ) -> Result<Option<(Vec<f64>, Vec<f64>)>> {
        let Some(h) = self.nonlcon.clone() else {
            return Ok(None);
        };
        e.check_cancel()?;
        let who = self.who;
        let v = e.call_handle(&h, vec![self.x_value(x)?], 2, None, out)?;
        if v.len() < 2 {
            return Err(format!("{who}: nonlcon must return [c, ceq]"));
        }
        let c = og_vector(&v[0], who, "nonlinear inequalities")?;
        let ce = og_vector(&v[1], who, "nonlinear equalities")?;
        if self.sizes.is_some_and(|s| s != (c.len(), ce.len())) {
            return Err(format!(
                "{who}: nonlinear constraint result length must not change"
            ));
        }
        self.sizes = Some((c.len(), ce.len()));
        Ok(Some((c, ce)))
    }
    /// Forward-difference normals of the nonlinear constraints within `mesh`
    /// of activity (all equalities), as (row, is_equality). Constraint calls
    /// do not count as objective evaluations.
    fn active_normals(
        &mut self,
        e: &mut Engine,
        x: &[f64],
        mesh: f64,
        out: &mut Execution,
    ) -> Result<Vec<(Vec<f64>, bool)>> {
        let Some((c, ce)) = self.constraints(e, x, out)? else {
            return Ok(Vec::new());
        };
        let active: Vec<(usize, bool)> = c
            .iter()
            .enumerate()
            .filter(|(_, v)| **v >= -mesh)
            .map(|(i, _)| (i, false))
            .chain((0..ce.len()).map(|i| (c.len() + i, true)))
            .collect();
        if active.is_empty() {
            return Ok(Vec::new());
        }
        let n = x.len();
        let mut rows = vec![vec![0.; n]; active.len()];
        let mut probe = x.to_vec();
        for j in 0..n {
            let h = 1e-7 * x[j].abs().max(1.);
            probe[j] = x[j] + h;
            let Some((cp, ep)) = self.constraints(e, &probe, out)? else {
                unreachable!("nonlcon present")
            };
            probe[j] = x[j];
            for (r, &(i, _)) in active.iter().enumerate() {
                let (now, then) = if i < c.len() {
                    (c[i], cp[i])
                } else {
                    (ce[i - c.len()], ep[i - c.len()])
                };
                rows[r][j] = (then - now) / h;
            }
        }
        Ok(rows
            .into_iter()
            .zip(active)
            .filter(|(row, _)| row.iter().all(|v| v.is_finite()) && row.iter().any(|v| *v != 0.))
            .map(|(row, (_, eq))| (row, eq))
            .collect())
    }
}

fn og_text(e: &Engine, s: &str) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_units(
        s.encode_utf16().collect(),
        &e.cancel,
    )?))
}

fn og_scalar(v: f64) -> Array {
    Array::Numeric(Matrix::scalar(v))
}

fn og_final(
    e: &mut Engine,
    o: &GOptions,
    flag: f64,
    message: &str,
    out: &mut Execution,
) -> Result<()> {
    let show = match o.display {
        Display::Off => false,
        Display::Notify => flag <= 0.,
        Display::Final | Display::Iter => true,
    };
    if show {
        e.emit(out, message.to_string())?;
    }
    Ok(())
}

fn og_limit(v: Option<f64>, default: f64) -> f64 {
    v.unwrap_or(default)
}

fn og_uniform(e: &mut Engine) -> f64 {
    e.random.next_f64()
}

fn og_normal(e: &mut Engine) -> f64 {
    e.random.next_normal()
}

/// Uniform index in 0..n.
fn og_index(e: &mut Engine, n: usize) -> usize {
    ((og_uniform(e) * n as f64) as usize).min(n - 1)
}

/// Total order with NaN as the worst value.
fn og_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    let key = |v: f64| if v.is_nan() { f64::INFINITY } else { v };
    key(a).total_cmp(&key(b))
}

/// Initial sampling range per variable: the bounds when both are finite,
/// otherwise `span` wide next to the finite bound or centred on 0.
fn og_span(lb: f64, ub: f64, span: f64) -> (f64, f64) {
    match (lb.is_finite(), ub.is_finite()) {
        (true, true) => (lb, ub),
        (true, false) => (lb, lb + span),
        (false, true) => (ub - span, ub),
        (false, false) => (-0.5 * span, 0.5 * span),
    }
}

// ---------------------------------------------------------------- patternsearch

/// Poll directions: tangent-cone generators of the nearly active linear
/// constraints (Lewis & Torczon 2000; Kolda, Lewis & Torczon 2003), then
/// the 2N basis [I -I]. `extra` holds linearized nonlinear constraints
/// (normal, is_equality). When the equalities and the rows within `mesh`
/// are linearly dependent, the activity radius shrinks to the next smaller
/// row distance until they are independent (Lewis & Torczon 2000, sec. 8);
/// a set that stays dependent (a degenerate vertex such as an equality plus
/// two active bounds in 2-D) gets the extreme rays of its cone.
fn og_directions(l: &Lin, x: &[f64], mesh: f64, extra: &[(Vec<f64>, bool)]) -> Vec<Vec<f64>> {
    let n = x.len();
    let mut dirs = Vec::with_capacity(2 * n + 2);
    if l.general() || !extra.is_empty() {
        // Rows of N: equalities (kept) then nearly active inequalities, each
        // with its distance (slack over the row norm) for the radius shrink.
        let mut rows: Vec<(Vec<f64>, bool, f64)> = Vec::new();
        for r in 0..l.aeq.nrows() {
            rows.push(((0..n).map(|j| l.aeq[(r, j)]).collect(), true, 0.));
        }
        rows.extend(extra.iter().map(|(row, eq)| (row.clone(), *eq, 0.)));
        let mut general_active = l.aeq.nrows() > 0 || !extra.is_empty();
        for r in 0..l.a.nrows() {
            let row: Vec<f64> = (0..n).map(|j| l.a[(r, j)]).collect();
            let norm = row.iter().map(|v| v * v).sum::<f64>().sqrt();
            let slack = l.b[r] - og_row_dot(&l.a, r, x);
            if norm > 0. && slack <= mesh * norm {
                rows.push((row, false, slack / norm));
                general_active = true;
            }
        }
        for i in 0..n {
            if x[i] - l.lb[i] <= mesh {
                let mut row = vec![0.; n];
                row[i] = -1.;
                rows.push((row, false, x[i] - l.lb[i]));
            }
            if l.ub[i] - x[i] <= mesh {
                let mut row = vec![0.; n];
                row[i] = 1.;
                rows.push((row, false, l.ub[i] - x[i]));
            }
        }
        if general_active {
            let mut radius = f64::INFINITY;
            loop {
                let set: Vec<(&[f64], bool)> = rows
                    .iter()
                    .filter(|(_, eq, distance)| *eq || *distance <= radius)
                    .map(|(row, eq, _)| (row.as_slice(), *eq))
                    .collect();
                if let Some(found) = og_cone_regular(&set, n) {
                    dirs = found;
                    break;
                }
                let distances = || {
                    rows.iter()
                        .filter(|(_, eq, distance)| !*eq && *distance <= radius)
                        .map(|(_, _, distance)| *distance)
                };
                let top = distances().fold(f64::NEG_INFINITY, f64::max);
                let next = distances()
                    .filter(|distance| *distance < top)
                    .fold(f64::NEG_INFINITY, f64::max);
                if next == f64::NEG_INFINITY {
                    dirs = og_cone_degenerate(&set, n);
                    break;
                }
                radius = next;
            }
        }
    }
    for d in &mut dirs {
        let scale = d.iter().fold(0f64, |a, v| a.max(v.abs()));
        for v in d.iter_mut() {
            *v /= scale;
            if v.abs() < 1e-12 {
                *v = 0.;
            }
        }
    }
    dirs.retain(|d| d.iter().any(|v| *v != 0.));
    for sign in [1., -1.] {
        for i in 0..n {
            let mut d = vec![0.; n];
            d[i] = sign;
            dirs.push(d);
        }
    }
    dirs
}

/// Generators of {d : E d = 0, G d <= 0} when the k rows of N = [E; G]
/// are linearly independent (k <= n): a null-space basis of N with both
/// signs, then the columns -N'(NN')^-1 of the inequality rows, which move
/// off one inequality while keeping every other row active. None when N is
/// rank deficient.
fn og_cone_regular(set: &[(&[f64], bool)], n: usize) -> Option<Vec<Vec<f64>>> {
    let k = set.len();
    if k > n {
        return None;
    }
    let mut dirs = Vec::new();
    let nm = DMatrix::from_fn(k, n, |r, j| set[r].0[j]);
    let gram = &nm * nm.transpose();
    let inv = gram.clone().try_inverse()?;
    if (&gram * &inv - DMatrix::identity(k, k)).amax() >= 1e-8 {
        return None;
    }
    // Null-space basis from the projector I - N'(NN')^-1 N.
    let projector = DMatrix::identity(n, n) - nm.transpose() * &inv * &nm;
    let eig = nalgebra::SymmetricEigen::new(projector);
    for (c, value) in eig.eigenvalues.iter().enumerate() {
        if *value > 0.5 {
            let z: Vec<f64> = eig.eigenvectors.column(c).iter().copied().collect();
            dirs.push(z.clone());
            dirs.push(z.iter().map(|v| -v).collect());
        }
    }
    // Moving off one inequality while keeping the others active.
    let pseudo = nm.transpose() * inv;
    for (r, (_, equality)) in set.iter().enumerate() {
        if !equality {
            dirs.push(pseudo.column(r).iter().map(|v| -v).collect());
        }
    }
    Some(dirs)
}

/// Orthonormal basis (columns) of the eigenvectors of the symmetric `m`
/// whose eigenvalues are at most `tiny` (`small`) or above it.
fn og_eigen_basis(m: DMatrix<f64>, tiny: f64, small: bool) -> DMatrix<f64> {
    let eig = nalgebra::SymmetricEigen::new(m);
    let columns: Vec<_> = eig
        .eigenvalues
        .iter()
        .enumerate()
        .filter(|(_, value)| (**value <= tiny) == small)
        .map(|(c, _)| eig.eigenvectors.column(c).into_owned())
        .collect();
    if columns.is_empty() {
        return DMatrix::zeros(eig.eigenvectors.nrows(), 0);
    }
    DMatrix::from_columns(&columns)
}

/// Generators of {d : E d = 0, G d <= 0} for any rows, including the
/// rank-deficient sets of a degenerate vertex. With Z an orthonormal basis
/// of null(E) and H = G Z (unit rows), the cone {y : H y <= 0} is its
/// lineality space null(H) (both signs) plus the extreme rays of the pointed
/// rest in range(H') of rank r; each extreme ray spans the null space of r-1
/// independent rows of H (Kolda, Lewis & Torczon 2003, sec. 8.2), so all
/// (r-1)-subsets are tried. Above `LIMIT` subsets only the rays of the
/// first r independent rows that lie in the cone are kept.
fn og_cone_degenerate(set: &[(&[f64], bool)], n: usize) -> Vec<Vec<f64>> {
    const TINY: f64 = 1e-10;
    const LIMIT: usize = 4096;
    let unit = |row: &[f64]| {
        let norm = row.iter().map(|v| v * v).sum::<f64>().sqrt();
        (norm > TINY).then(|| row.iter().map(|v| v / norm).collect::<Vec<f64>>())
    };
    let equalities: Vec<Vec<f64>> = set
        .iter()
        .filter(|(_, eq)| *eq)
        .filter_map(|(row, _)| unit(row))
        .collect();
    let z = if equalities.is_empty() {
        DMatrix::identity(n, n)
    } else {
        let e = DMatrix::from_fn(equalities.len(), n, |r, j| equalities[r][j]);
        og_eigen_basis(e.transpose() * e, TINY, true)
    };
    let m = z.ncols();
    if m == 0 {
        return Vec::new();
    }
    let h_rows: Vec<Vec<f64>> = set
        .iter()
        .filter(|(_, eq)| !*eq)
        .filter_map(|(row, _)| {
            let projected: Vec<f64> = (0..m)
                .map(|c| (0..n).map(|j| row[j] * z[(j, c)]).sum())
                .collect();
            unit(&projected)
        })
        .collect();
    let p = h_rows.len();
    let mut ys: Vec<DVector<f64>> = Vec::new();
    let mut rays: Vec<DVector<f64>> = Vec::new();
    if p == 0 {
        for c in 0..m {
            let y = DVector::from_fn(m, |i, _| if i == c { 1. } else { 0. });
            ys.push(-&y);
            ys.push(y);
        }
    } else {
        let h = DMatrix::from_fn(p, m, |r, c| h_rows[r][c]);
        let hth = h.transpose() * &h;
        let lineality = og_eigen_basis(hth.clone(), TINY, true);
        for c in 0..lineality.ncols() {
            let y = lineality.column(c).into_owned();
            ys.push(y.clone());
            ys.push(-y);
        }
        let w = og_eigen_basis(hth, TINY, false);
        let r = w.ncols();
        let a = &h * &w;
        let in_cone = |u: &DVector<f64>| (&a * u).iter().all(|v| *v <= 1e-9);
        let add = |u: DVector<f64>, rays: &mut Vec<DVector<f64>>| {
            if in_cone(&u) && rays.iter().all(|v| (v - &u).amax() > 1e-8) {
                rays.push(u);
            }
        };
        // Subset count C(p, r-1), stopping as soon as it passes LIMIT.
        let mut count = 1usize;
        for i in 0..r.saturating_sub(1) {
            count = count * (p - i) / (i + 1);
            if count > LIMIT {
                break;
            }
        }
        if r == 0 {
            // Only zero rows: nothing beyond the lineality space.
        } else if r == 1 {
            let u = DVector::from_element(1, 1.);
            add(-&u, &mut rays);
            add(u, &mut rays);
        } else if count <= LIMIT {
            let mut pick: Vec<usize> = (0..r - 1).collect();
            loop {
                let sub = DMatrix::from_fn(r - 1, r, |i, j| a[(pick[i], j)]);
                let null = og_eigen_basis(sub.transpose() * sub, TINY, true);
                if null.ncols() == 1 {
                    let u = null.column(0).into_owned();
                    add(-&u, &mut rays);
                    add(u, &mut rays);
                }
                // Next (r-1)-subset of 0..p in lexicographic order.
                let Some(i) = (0..r - 1).rev().find(|&i| pick[i] < p - (r - 1) + i) else {
                    break;
                };
                pick[i] += 1;
                for j in i + 1..r - 1 {
                    pick[j] = pick[j - 1] + 1;
                }
            }
        } else {
            // First r independent rows (greedy), then the rays -B^-1 e_i.
            let mut basis: Vec<usize> = Vec::new();
            for row in 0..p {
                let mut trial = basis.clone();
                trial.push(row);
                let sub = DMatrix::from_fn(trial.len(), r, |i, j| a[(trial[i], j)]);
                let gram = &sub * sub.transpose();
                if nalgebra::SymmetricEigen::new(gram).eigenvalues.min() > TINY {
                    basis = trial;
                }
                if basis.len() == r {
                    break;
                }
            }
            let b = DMatrix::from_fn(r, r, |i, j| a[(basis[i], j)]);
            if let Some(inverse) = b.try_inverse() {
                for c in 0..r {
                    let u = -inverse.column(c).normalize();
                    add(u, &mut rays);
                }
            }
        }
        ys.extend(rays.iter().map(|u| &w * u));
    }
    ys.iter()
        .map(|y| (&z * y).iter().copied().collect())
        .collect()
}

struct PsPoint {
    x: Vec<f64>,
    f: f64,
    merit: f64,
    violation: f64,
}

fn og_ps_eval(
    e: &mut Engine,
    p: &mut GProblem,
    x: Vec<f64>,
    rho: f64,
    out: &mut Execution,
) -> Result<PsPoint> {
    let f = p.objective(e, &x, out)?;
    let (violation, squares) = p.nonlinear(e, &x, out)?;
    Ok(PsPoint {
        merit: f + rho * squares,
        f,
        violation,
        x,
    })
}

fn og_patternsearch(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "patternsearch";
    if args.len() < 2 || args.len() > 10 {
        return Err("patternsearch: expected fun,x0[,A,b,Aeq,beq,lb,ub,nonlcon,options]".into());
    }
    let x0 = og_x0(&args[1], who)?;
    let n = x0.len();
    let l = og_linear(who, n, &args[2..args.len().min(8)])?;
    let mut p = GProblem::new(e, who, &args[0], args.get(8), (x0.rows(), x0.cols()))?;
    let o = og_options(e, who, args.get(9))?;
    let max_iterations = og_limit(o.max_iterations, 100. * n as f64);
    let max_evaluations = og_limit(o.max_evaluations, 2000. * n as f64);
    let mut x = x0.data().to_vec();
    l.repair(&mut x);
    let feasible_start = l.violation(&x) <= o.constraint_tol;
    // Quadratic penalty stages for nonlcon: rho = 10, 1e3, ..., 1e15.
    let stages = if p.nonlcon.is_some() { 8 } else { 1 };
    let mut rho = if p.nonlcon.is_some() { 10. } else { 0. };
    let mut current = og_ps_eval(e, &mut p, x, rho, out)?;
    let mut mesh = o.initial_mesh;
    let mut iterations = 0usize;
    let (mut flag, mut message) = (
        0.,
        String::from("Maximum number of iterations or function evaluations reached"),
    );
    if !feasible_start {
        flag = -2.;
        message = "No point satisfies the bounds and linear constraints".into();
    } else {
        'stages: for stage in 0..stages {
            if stage > 0 {
                rho *= 100.;
                current.merit = current.f
                    + rho * {
                        let (_, s) = p.nonlinear(e, &current.x, out)?;
                        s
                    };
                mesh = (o.initial_mesh * 0.1f64.powi(stage)).max(100. * o.mesh_tol);
            }
            loop {
                e.check_cancel()?;
                if mesh <= o.mesh_tol {
                    flag = 1.;
                    message = "Mesh size is below MeshTolerance".into();
                    break;
                }
                if iterations as f64 >= max_iterations || p.evaluations >= max_evaluations {
                    flag = 0.;
                    message = "Maximum number of iterations or function evaluations reached".into();
                    break 'stages;
                }
                let mut improved = None;
                let normals = p.active_normals(e, &current.x, mesh, out)?;
                for d in og_directions(&l, &current.x, mesh, &normals) {
                    if p.evaluations >= max_evaluations {
                        break;
                    }
                    let trial: Vec<f64> = current
                        .x
                        .iter()
                        .zip(&d)
                        .map(|(x, d)| x + mesh * d)
                        .collect();
                    if !l.in_bounds(&trial) || l.violation(&trial) > o.constraint_tol {
                        continue;
                    }
                    let point = og_ps_eval(e, &mut p, trial, rho, out)?;
                    if point.merit < current.merit {
                        improved = Some(point);
                        break;
                    }
                }
                iterations += 1;
                if let Some(point) = improved {
                    let change = (current.merit - point.merit).abs();
                    current = point;
                    mesh *= 2.;
                    if mesh * 0.5 <= o.step_tol
                        && change <= o.function_tol * current.merit.abs().max(1.)
                    {
                        flag = 2.;
                        message = "Step and objective change are below StepTolerance and FunctionTolerance".into();
                        break;
                    }
                } else {
                    mesh *= 0.5;
                }
                if o.display == Display::Iter {
                    e.emit(
                        out,
                        format!(
                            "Iteration {iterations}: f(x) = {:.6e}, mesh size = {:.4e}",
                            current.f, mesh
                        ),
                    )?;
                }
            }
            if current.violation <= o.constraint_tol {
                break;
            }
        }
        if flag > 0. && current.violation > o.constraint_tol {
            flag = -2.;
            message = "No point satisfying the nonlinear constraints was found".into();
        }
    }
    let maxconstraint = l.violation(&current.x).max(current.violation);
    let output = onl::onl_struct(
        e,
        vec![
            ("function", Array::Handle(p.fun.clone())),
            (
                "problemtype",
                og_text(e, og_problem_type(&l, p.nonlcon.is_some()))?,
            ),
            ("pollmethod", og_text(e, "GPSPositiveBasis2N")?),
            ("iterations", og_scalar(iterations as f64)),
            ("funccount", og_scalar(p.evaluations)),
            ("meshsize", og_scalar(mesh)),
            ("maxconstraint", og_scalar(maxconstraint)),
            ("message", og_text(e, &message)?),
        ],
    )?;
    og_final(e, &o, flag, &message, out)?;
    Ok(vec![
        p.x_value(&current.x)?,
        Value::Matrix(Matrix::scalar(current.f)),
        Value::Matrix(Matrix::scalar(flag)),
        output,
    ])
}

fn og_problem_type(l: &Lin, nonlinear: bool) -> &'static str {
    if nonlinear {
        "nonlinearconstr"
    } else if l.general() {
        "linearconstraints"
    } else if l.lb.iter().chain(&l.ub).any(|v| v.is_finite()) {
        "boundconstraints"
    } else {
        "unconstrained"
    }
}

// ---------------------------------------------------------------- particleswarm

fn og_particleswarm(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "particleswarm";
    if args.len() < 2 || args.len() > 5 {
        return Err("particleswarm: expected fun,nvars[,lb,ub,options]".into());
    }
    let n = og_nvars(e, &args[1], who)?;
    let lb = og_bound(args.get(2), n, true, who)?;
    let ub = og_bound(args.get(3), n, false, who)?;
    if lb.iter().zip(&ub).any(|(l, u)| l > u) {
        return Err("particleswarm: lb must not exceed ub".into());
    }
    let mut p = GProblem::new(e, who, &args[0], None, (1, n))?;
    let o = og_options(e, who, args.get(4))?;
    let size = o.swarm.unwrap_or((10 * n).min(100)).max(2);
    if size.saturating_mul(n) > MAX_ELEMENTS {
        return Err("particleswarm: SwarmSize*nvars exceeds the limit (4000000)".into());
    }
    let max_iterations = og_limit(o.max_iterations, 200. * n as f64);
    let max_evaluations = og_limit(o.max_evaluations, f64::INFINITY);
    let stall = og_limit(o.stall, 20.) as usize;
    let (y1, y2) = (1.49, 1.49);
    let (w_min, w_max) = (0.1, 1.1);
    let min_neighbours = ((0.25 * size as f64) as usize).max(2).min(size - 1).max(1);
    let spans: Vec<(f64, f64)> = (0..n).map(|i| og_span(lb[i], ub[i], 2000.)).collect();
    let mut x = vec![vec![0.; n]; size];
    let mut v = vec![vec![0.; n]; size];
    for k in 0..size {
        for i in 0..n {
            let (lo, hi) = spans[i];
            x[k][i] = lo + (hi - lo) * og_uniform(e);
            v[k][i] = (hi - lo) * (2. * og_uniform(e) - 1.);
        }
    }
    let mut f = Vec::with_capacity(size);
    for xk in &x {
        f.push(p.objective(e, xk, out)?);
    }
    let mut best_x = x.clone();
    let mut best_f = f.clone();
    let mut g = og_argmin(&best_f);
    let mut history = vec![best_f[g]];
    let mut inertia = w_max;
    let mut counter = 0usize;
    let mut neighbours = min_neighbours;
    let mut iterations = 0usize;
    let mut others: Vec<usize> = (0..size).collect();
    let (mut flag, mut message) = (
        0.,
        String::from("Maximum number of iterations or function evaluations reached"),
    );
    loop {
        e.check_cancel()?;
        if iterations as f64 >= max_iterations || p.evaluations >= max_evaluations {
            break;
        }
        iterations += 1;
        let previous = best_f[g];
        for k in 0..size {
            e.check_cancel()?;
            // Best of a random neighbourhood of `neighbours` other particles.
            let mut local = k;
            let mut count = 0;
            let pool = size - 1;
            others.clear();
            others.extend((0..size).filter(|&j| j != k));
            while count < neighbours.min(pool) {
                let pick = count + og_index(e, pool - count);
                others.swap(count, pick);
                let j = others[count];
                if og_cmp(best_f[j], best_f[local]).is_lt() || local == k {
                    local = j;
                }
                count += 1;
            }
            for i in 0..n {
                let u1 = og_uniform(e);
                let u2 = og_uniform(e);
                v[k][i] = inertia * v[k][i]
                    + y1 * u1 * (best_x[k][i] - x[k][i])
                    + y2 * u2 * (best_x[local][i] - x[k][i]);
                x[k][i] += v[k][i];
                if x[k][i] < lb[i] {
                    x[k][i] = lb[i];
                    v[k][i] = 0.;
                } else if x[k][i] > ub[i] {
                    x[k][i] = ub[i];
                    v[k][i] = 0.;
                }
                if !x[k][i].is_finite() {
                    x[k][i] = best_x[k][i];
                    v[k][i] = 0.;
                }
            }
        }
        for k in 0..size {
            if p.evaluations >= max_evaluations {
                break;
            }
            f[k] = p.objective(e, &x[k], out)?;
            if og_cmp(f[k], best_f[k]).is_lt() {
                best_f[k] = f[k];
                best_x[k].clone_from(&x[k]);
            }
        }
        g = og_argmin(&best_f);
        if og_cmp(best_f[g], previous).is_lt() {
            counter = counter.saturating_sub(1);
            neighbours = min_neighbours;
            if counter < 2 {
                inertia *= 2.;
            }
            if counter > 5 {
                inertia /= 2.;
            }
            inertia = inertia.clamp(w_min, w_max);
        } else {
            counter += 1;
            // A stalled swarm also contracts (Shi & Eberhart 1998: lower
            // inertia favours local search).
            if counter > 5 {
                inertia = (inertia / 2.).clamp(w_min, w_max);
            }
            neighbours = (neighbours + min_neighbours).min(size - 1);
        }
        history.push(best_f[g]);
        if o.display == Display::Iter {
            e.emit(
                out,
                format!(
                    "Iteration {iterations}: best f(x) = {:.6e}, inertia = {inertia:.3}",
                    best_f[g]
                ),
            )?;
        }
        if history.len() > stall {
            let old = history[history.len() - 1 - stall];
            let now = best_f[g];
            if (old - now).abs() <= o.function_tol * now.abs().max(1.) {
                flag = 1.;
                message = "Relative change in the best objective value over MaxStallIterations is below FunctionTolerance".into();
                break;
            }
        }
    }
    let output = onl::onl_struct(
        e,
        vec![
            ("iterations", og_scalar(iterations as f64)),
            ("funccount", og_scalar(p.evaluations)),
            ("message", og_text(e, &message)?),
        ],
    )?;
    og_final(e, &o, flag, &message, out)?;
    Ok(vec![
        p.x_value(&best_x[g])?,
        Value::Matrix(Matrix::scalar(best_f[g])),
        Value::Matrix(Matrix::scalar(flag)),
        output,
    ])
}

fn og_argmin(v: &[f64]) -> usize {
    let mut best = 0;
    for (i, x) in v.iter().enumerate() {
        if og_cmp(*x, v[best]).is_lt() {
            best = i;
        }
    }
    best
}

// ---------------------------------------------------------------- ga

struct Individual {
    x: Vec<f64>,
    f: f64,
    violation: f64,
}

/// Deb (2000) ordering keys: feasible points by objective, infeasible ones
/// after every feasible point by violation.
fn og_keys(pop: &[Individual], tol: f64) -> Vec<f64> {
    let worst = pop
        .iter()
        .filter(|i| i.violation <= tol && i.f.is_finite())
        .map(|i| i.f)
        .fold(f64::NEG_INFINITY, f64::max);
    let worst = if worst.is_finite() { worst } else { 0. };
    pop.iter()
        .map(|i| {
            if i.violation <= tol {
                i.f
            } else {
                worst + i.violation.max(tol)
            }
        })
        .collect()
}

fn og_ga_eval(
    e: &mut Engine,
    p: &mut GProblem,
    l: &Lin,
    x: Vec<f64>,
    out: &mut Execution,
) -> Result<Individual> {
    let f = p.objective(e, &x, out)?;
    let (worst, _) = p.nonlinear(e, &x, out)?;
    Ok(Individual {
        violation: worst.max(l.violation(&x)),
        f,
        x,
    })
}

fn og_ga(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "ga";
    if args.len() < 2 || args.len() > 10 {
        return Err("ga: expected fun,nvars[,A,b,Aeq,beq,lb,ub,nonlcon,options]".into());
    }
    if args.get(9).is_some_and(|v| {
        !onl::is_empty_value(v) && !matches!(v, Value::Container(Array::Struct(_)))
    }) {
        return Err("ga: integer constraints (intcon) are not implemented".into());
    }
    let n = og_nvars(e, &args[1], who)?;
    let l = og_linear(who, n, &args[2..args.len().min(8)])?;
    let mut p = GProblem::new(e, who, &args[0], args.get(8), (1, n))?;
    let o = og_options(e, who, args.get(9))?;
    let size = o.population.unwrap_or(if n <= 5 { 50 } else { 200 });
    if size.saturating_mul(n) > MAX_ELEMENTS {
        return Err("ga: PopulationSize*nvars exceeds the limit (4000000)".into());
    }
    let elite = o
        .elite
        .unwrap_or((0.05 * size as f64).ceil() as usize)
        .min(size);
    let generations = og_limit(o.max_iterations, 100. * n as f64);
    let max_evaluations = og_limit(o.max_evaluations, f64::INFINITY);
    let stall = og_limit(o.stall, 50.) as usize;
    let tol = o.constraint_tol;
    let spans: Vec<(f64, f64)> = (0..n).map(|i| og_span(l.lb[i], l.ub[i], 20.)).collect();
    let ranges: Vec<f64> = spans.iter().map(|(lo, hi)| hi - lo).collect();
    let mut pop = Vec::with_capacity(size);
    for _ in 0..size {
        let mut x: Vec<f64> = spans
            .iter()
            .map(|(lo, hi)| lo + (hi - lo) * og_uniform(e))
            .collect();
        l.repair(&mut x);
        pop.push(og_ga_eval(e, &mut p, &l, x, out)?);
    }
    let mut keys = og_keys(&pop, tol);
    let mut history = vec![keys[og_argmin(&keys)]];
    let mut generation = 0usize;
    let (mut flag, mut message) = (
        0.,
        String::from("Maximum number of generations or function evaluations reached"),
    );
    loop {
        e.check_cancel()?;
        if generation as f64 >= generations || p.evaluations >= max_evaluations {
            break;
        }
        generation += 1;
        let mut order: Vec<usize> = (0..size).collect();
        order.sort_by(|&a, &b| og_cmp(keys[a], keys[b]));
        let mut next: Vec<Individual> = order[..elite]
            .iter()
            .map(|&i| Individual {
                x: pop[i].x.clone(),
                f: pop[i].f,
                violation: pop[i].violation,
            })
            .collect();
        let children = size - elite;
        let crossovers = (o.crossover * children as f64).round() as usize;
        // Gaussian mutation scale shrinks linearly to 0 at the last generation.
        let shrink = if generations.is_finite() {
            (1. - generation as f64 / generations).max(0.)
        } else {
            1.
        };
        for c in 0..children {
            e.check_cancel()?;
            if p.evaluations >= max_evaluations {
                break;
            }
            let mut x = if c < crossovers {
                let a = og_tournament(e, &keys);
                let b = og_tournament(e, &keys);
                // Arithmetic crossover (always with linear constraints, since
                // convex combinations of feasible parents stay feasible) or,
                // with equal probability, scattered crossover.
                if l.general() || og_uniform(e) < 0.5 {
                    let t = og_uniform(e);
                    (0..n)
                        .map(|i| t * pop[a].x[i] + (1. - t) * pop[b].x[i])
                        .collect::<Vec<_>>()
                } else {
                    (0..n)
                        .map(|i| {
                            if og_uniform(e) < 0.5 {
                                pop[a].x[i]
                            } else {
                                pop[b].x[i]
                            }
                        })
                        .collect()
                }
            } else {
                let a = og_tournament(e, &keys);
                (0..n)
                    .map(|i| pop[a].x[i] + shrink * ranges[i] * og_normal(e))
                    .collect()
            };
            l.repair(&mut x);
            next.push(og_ga_eval(e, &mut p, &l, x, out)?);
        }
        // An evaluation budget can leave the generation short: keep the best old ones.
        let mut k = elite;
        while next.len() < size {
            let i = order[k.min(size - 1)];
            next.push(Individual {
                x: pop[i].x.clone(),
                f: pop[i].f,
                violation: pop[i].violation,
            });
            k += 1;
        }
        pop = next;
        keys = og_keys(&pop, tol);
        let best = keys[og_argmin(&keys)];
        history.push(best);
        if o.display == Display::Iter {
            e.emit(
                out,
                format!("Generation {generation}: best fitness = {best:.6e}"),
            )?;
        }
        if history.len() > stall {
            let old = history[history.len() - 1 - stall];
            if (old - best).abs() <= stall as f64 * o.function_tol * best.abs().max(1.) {
                flag = 1.;
                message = "Average change in the best fitness over MaxStallGenerations is below FunctionTolerance".into();
                break;
            }
        }
    }
    let b = og_argmin(&keys);
    if pop[b].violation > tol {
        flag = -2.;
        message = "No feasible point found".into();
    }
    let output = onl::onl_struct(
        e,
        vec![
            (
                "problemtype",
                og_text(e, og_problem_type(&l, p.nonlcon.is_some()))?,
            ),
            ("generations", og_scalar(generation as f64)),
            ("funccount", og_scalar(p.evaluations)),
            ("message", og_text(e, &message)?),
            ("maxconstraint", og_scalar(pop[b].violation)),
        ],
    )?;
    og_final(e, &o, flag, &message, out)?;
    let mut population = vec![0.; size * n];
    for (r, ind) in pop.iter().enumerate() {
        for (c, v) in ind.x.iter().enumerate() {
            population[r + c * size] = *v;
        }
    }
    let scores: Vec<f64> = pop.iter().map(|i| i.f).collect();
    Ok(vec![
        p.x_value(&pop[b].x)?,
        Value::Matrix(Matrix::scalar(pop[b].f)),
        Value::Matrix(Matrix::scalar(flag)),
        output,
        Value::Matrix(Matrix::from_column_vec(size, n, population)?),
        Value::Matrix(Matrix::from_column_vec(size, 1, scores)?),
    ])
}

/// Binary tournament: the better of two uniformly drawn individuals.
fn og_tournament(e: &mut Engine, keys: &[f64]) -> usize {
    let a = og_index(e, keys.len());
    let b = og_index(e, keys.len());
    if og_cmp(keys[b], keys[a]).is_lt() {
        b
    } else {
        a
    }
}

// ---------------------------------------------------------------- simulannealbnd

fn og_simulanneal(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "simulannealbnd";
    if args.len() < 2 || args.len() > 5 {
        return Err("simulannealbnd: expected fun,x0[,lb,ub,options]".into());
    }
    let x0 = og_x0(&args[1], who)?;
    let n = x0.len();
    let lb = og_bound(args.get(2), n, true, who)?;
    let ub = og_bound(args.get(3), n, false, who)?;
    if lb.iter().zip(&ub).any(|(l, u)| l > u) {
        return Err("simulannealbnd: lb must not exceed ub".into());
    }
    let mut p = GProblem::new(e, who, &args[0], None, (x0.rows(), x0.cols()))?;
    let o = og_options(e, who, args.get(4))?;
    let t0 = match &o.temperature {
        None => vec![100.; n],
        Some(t) if t.len() == 1 => vec![t[0]; n],
        Some(t) if t.len() == n => t.clone(),
        Some(_) => {
            return Err(
                "simulannealbnd: InitialTemperature must be a scalar or have one entry per variable"
                    .into(),
            );
        }
    };
    let max_iterations = og_limit(o.max_iterations, f64::INFINITY);
    let max_evaluations = og_limit(o.max_evaluations, 3000. * n as f64);
    let stall = og_limit(o.stall, 500. * n as f64) as usize;
    let cooling: f64 = 0.95;
    let mut k = vec![0f64; n];
    let mut temperature = t0.clone();
    let mut x: Vec<f64> = (0..n).map(|i| x0.data()[i].clamp(lb[i], ub[i])).collect();
    let mut fx = p.objective(e, &x, out)?;
    let (mut best_x, mut best_f) = (x.clone(), fx);
    let mut accepted = 0usize;
    let mut iterations = 0usize;
    let mut changes: std::collections::VecDeque<f64> = std::collections::VecDeque::new();
    let mut change_sum = 0.;
    let (mut flag, mut message) = (
        0.,
        String::from("Maximum number of iterations or function evaluations reached"),
    );
    loop {
        e.check_cancel()?;
        if iterations as f64 >= max_iterations || p.evaluations >= max_evaluations {
            break;
        }
        iterations += 1;
        // Step of length T in a uniformly random direction; a coordinate that
        // leaves the bounds is redrawn between x and the violated bound.
        let mut u: Vec<f64> = (0..n).map(|_| og_normal(e)).collect();
        let norm = u
            .iter()
            .map(|v| v * v)
            .sum::<f64>()
            .sqrt()
            .max(f64::MIN_POSITIVE);
        for v in &mut u {
            *v /= norm;
        }
        let mut y = x.clone();
        for i in 0..n {
            y[i] = x[i] + temperature[i] * u[i];
            if y[i] < lb[i] {
                y[i] = x[i] + og_uniform(e) * (lb[i] - x[i]);
            } else if y[i] > ub[i] {
                y[i] = x[i] + og_uniform(e) * (ub[i] - x[i]);
            }
            y[i] = y[i].clamp(lb[i], ub[i]);
        }
        let fy = p.objective(e, &y, out)?;
        let delta = fy - fx;
        let t = temperature.iter().fold(0f64, |a, v| a.max(*v));
        // Boltzmann (Metropolis) acceptance.
        let accept = delta <= 0. || (t > 0. && og_uniform(e) < (-delta / t).exp());
        let previous = fx;
        if accept {
            x = y;
            fx = fy;
            accepted += 1;
            if og_cmp(fx, best_f).is_lt() {
                best_f = fx;
                best_x.clone_from(&x);
            }
        }
        for i in 0..n {
            k[i] += 1.;
            temperature[i] = t0[i] * cooling.powf(k[i]);
        }
        if accept && accepted.is_multiple_of(o.reanneal as usize) {
            og_reanneal(
                e,
                &mut p,
                &x,
                fx,
                &lb,
                &ub,
                &t0,
                &mut temperature,
                &mut k,
                cooling,
                out,
            )?;
        }
        let change = if (fx - previous).is_finite() {
            (fx - previous).abs()
        } else {
            f64::INFINITY
        };
        changes.push_back(change);
        change_sum += change;
        if changes.len() > stall {
            change_sum -= changes.pop_front().unwrap_or(0.);
        }
        if o.display == Display::Iter && iterations.is_multiple_of(50) {
            e.emit(
                out,
                format!("Iteration {iterations}: best f(x) = {best_f:.6e}, f(x) = {fx:.6e}, temperature = {t:.4e}"),
            )?;
        }
        if changes.len() >= stall {
            // Recompute the window sum to avoid drift from Inf entries.
            if !change_sum.is_finite() {
                change_sum = changes.iter().sum();
            }
            if change_sum / stall as f64 <= o.function_tol {
                flag = 1.;
                message = "Average change in the objective over MaxStallIterations is below FunctionTolerance".into();
                break;
            }
        }
    }
    let output = onl::onl_struct(
        e,
        vec![
            ("iterations", og_scalar(iterations as f64)),
            ("funccount", og_scalar(p.evaluations)),
            ("message", og_text(e, &message)?),
            (
                "temperature",
                Array::Numeric(Matrix::from_column_vec(n, 1, temperature)?),
            ),
        ],
    )?;
    og_final(e, &o, flag, &message, out)?;
    Ok(vec![
        p.x_value(&best_x)?,
        Value::Matrix(Matrix::scalar(best_f)),
        Value::Matrix(Matrix::scalar(flag)),
        output,
    ])
}

/// Reannealing after Ingber (1989): s are finite-difference sensitivities at
/// x; the annealing parameters are reset from them, raising the temperatures.
#[allow(clippy::too_many_arguments)]
fn og_reanneal(
    e: &mut Engine,
    p: &mut GProblem,
    x: &[f64],
    fx: f64,
    lb: &[f64],
    ub: &[f64],
    t0: &[f64],
    temperature: &mut [f64],
    k: &mut [f64],
    cooling: f64,
    out: &mut Execution,
) -> Result<()> {
    let n = x.len();
    let mut s = vec![0.; n];
    let mut probe = x.to_vec();
    for i in 0..n {
        let h = 1e-6 * x[i].abs().max(1.);
        let step = if x[i] + h <= ub[i] {
            h
        } else {
            -h.min(x[i] - lb[i])
        };
        if step == 0. {
            continue;
        }
        probe[i] = x[i] + step;
        let f = p.objective(e, &probe, out)?;
        probe[i] = x[i];
        s[i] = ((f - fx) / step).abs();
    }
    let smax = s
        .iter()
        .fold(0f64, |a, v| if v.is_finite() { a.max(*v) } else { a });
    if smax <= 0. {
        return Ok(());
    }
    // Annealing parameters k_i = ln((T0_i/T_i) * max(s)/s_i): far below the
    // iteration count, so reannealing raises every temperature, most in the
    // least sensitive directions.
    for i in 0..n {
        let ratio = if s[i] > 0. && s[i].is_finite() {
            smax / s[i]
        } else {
            1.
        };
        k[i] = ((t0[i] / temperature[i]) * ratio).ln().max(0.);
        temperature[i] = t0[i] * cooling.powf(k[i]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lin(a: DMatrix<f64>, b: Vec<f64>, n: usize) -> Lin {
        Lin {
            a,
            b,
            aeq: DMatrix::zeros(0, n),
            beq: vec![],
            lb: vec![f64::NEG_INFINITY; n],
            ub: vec![f64::INFINITY; n],
        }
    }

    #[test]
    fn og_repair_reaches_the_halfspace() {
        let l = lin(DMatrix::from_row_slice(1, 2, &[1., 1.]), vec![1.], 2);
        let mut x = vec![2., 2.];
        l.repair(&mut x);
        assert!(l.violation(&x) <= 1e-12, "{x:?}");
        assert!((x[0] - 0.5).abs() < 1e-12 && (x[1] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn og_tangent_directions_follow_an_active_constraint() {
        // x1 + x2 <= 1 active at (0.5, 0.5): ±(1,-1)/1 and the inward -(1,1)/2.
        let l = lin(DMatrix::from_row_slice(1, 2, &[1., 1.]), vec![1.], 2);
        let dirs = og_directions(&l, &[0.5, 0.5], 0.1, &[]);
        assert_eq!(dirs.len(), 3 + 4);
        assert!(dirs[..2].iter().all(|d| (d[0] + d[1]).abs() < 1e-12));
        assert!((dirs[2][0] + 1.).abs() < 1e-12 && (dirs[2][1] + 1.).abs() < 1e-12);
    }

    fn simplex(n: usize, ub: f64) -> Lin {
        Lin {
            a: DMatrix::zeros(0, n),
            b: vec![],
            aeq: DMatrix::from_element(1, n, 1.),
            beq: vec![1.],
            lb: vec![0.; n],
            ub: vec![ub; n],
        }
    }

    #[test]
    fn og_tangent_directions_at_a_degenerate_vertex() {
        // x1 + x2 = 1 with x1 >= 0 and x2 <= 1 exactly active at (0, 1):
        // three dependent rows in 2-D. The cone is the ray (1, -1).
        let dirs = og_directions(&simplex(2, 1.), &[0., 1.], 1., &[]);
        assert_eq!(dirs.len(), 1 + 4, "{dirs:?}");
        assert!((dirs[0][0] - 1.).abs() < 1e-12 && (dirs[0][1] + 1.).abs() < 1e-12);
    }

    #[test]
    fn og_tangent_directions_shrink_the_activity_radius() {
        // sum(x) = 1, x >= 0 at (1, 0, 0) with mesh 1: x1 >= 0 is within the
        // mesh too (4 rows in 3-D); without it N is square and the cone is
        // generated by (-1, 1, 0) and (-1, 0, 1).
        let dirs = og_directions(&simplex(3, f64::INFINITY), &[1., 0., 0.], 1., &[]);
        assert_eq!(dirs.len(), 2 + 6, "{dirs:?}");
        for (d, j) in dirs[..2].iter().zip([1, 2]) {
            let mut expected = vec![-1., 0., 0.];
            expected[j] = 1.;
            assert!(
                d.iter().zip(&expected).all(|(a, b)| (a - b).abs() < 1e-12),
                "{d:?}"
            );
        }
    }
}
