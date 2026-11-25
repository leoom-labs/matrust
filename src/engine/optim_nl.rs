//! Nonlinear optimisation: optimoptions, fminunc, lsqnonlin, lsqcurvefit and fsolve.
//! Free functions with the `onl_` prefix; the engine's dispatch arms only call in.
//!
//! - fminunc is the BFGS quasi-Newton method (J. Nocedal & S. J. Wright, Numerical
//!   Optimization, 2nd ed., Springer 2006, Algorithm 6.1, update eq. 6.19 on the
//!   Hessian approximation B, first scaling eq. 6.20). The direction solves B p = -g by
//!   Cholesky; the step length satisfies the strong Wolfe conditions (Algorithms 3.5
//!   and 3.6, cubic interpolation eq. 3.59, c1 = 1e-4, c2 = 0.9).
//! - lsqnonlin and lsqcurvefit are Levenberg-Marquardt in the trust-region form of
//!   J. J. Moré, "The Levenberg-Marquardt algorithm: implementation and theory",
//!   Numerical Analysis (Dundee 1977), Lecture Notes in Mathematics 630, 1978: scaling
//!   D from the largest column norms of J seen so far (eq. 6.3), the parameter from
//!   the safeguarded Newton iteration on ||D p|| = Delta (sec. 5) and the radius
//!   update of sec. 7. Each trial step solves [J; sqrt(lambda) D] p = [-r; 0] by QR.
//!   Bounds use projection, not a reflective trust region: variables held at a bound
//!   by the gradient are frozen, and the step of the others is projected back into the
//!   box (the projected Levenberg-Marquardt method of C. Kanzow, N. Yamashita &
//!   M. Fukushima, J. Comput. Appl. Math. 172, 2004).
//! - fsolve defaults to trust-region dogleg on the Gauss-Newton model of 0.5 ||F||^2
//!   (Nocedal & Wright 2006, sec. 11.2 with Algorithm 4.1 and the dogleg of sec. 4.1);
//!   the Gauss-Newton step is the minimum-norm least-squares solution (SVD), so a
//!   singular Jacobian still gives a step. Rectangular systems and an explicit
//!   'levenberg-marquardt' selection use the augmented-QR LM step described above.
//!
//! Gradients and Jacobians are forward differences with step sqrt(eps) max(|x|, 1)
//! (central with eps^(1/3) when FiniteDifferenceType is 'central') unless
//! SpecifyObjectiveGradient is set. Every callback checks cancellation and the
//! evaluation budget, including finite differences and rejected trials.
use super::{Engine, Execution, Value};
use crate::{Array, CharacterRow, FunctionHandle, Matrix, Result, StructArray};
use nalgebra::{DMatrix, DVector};
use std::collections::BTreeSet;
use std::sync::Arc;

const NAMES: &[&str] = &[
    "optimoptions",
    "fminunc",
    "lsqnonlin",
    "lsqcurvefit",
    "fsolve",
];
/// Solvers whose options optimoptions builds.
const SOLVERS: &[&str] = &[
    "linprog",
    "quadprog",
    "lsqlin",
    "fmincon",
    "intlinprog",
    "fminunc",
    "lsqnonlin",
    "lsqcurvefit",
    "fsolve",
    "fzero",
    "fminbnd",
    "fminsearch",
    "fminimax",
    "fgoalattain",
    "ga",
    "particleswarm",
    "simulannealbnd",
    "patternsearch",
];
/// Most unknowns: B and the Jacobian hold n^2 and m n doubles.
const MAX_UNKNOWNS: usize = 2000;
const MAX_JACOBIAN: usize = 4 * 1024 * 1024;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn max_outputs(name: &str) -> usize {
    match name {
        "optimoptions" => 1,
        "fminunc" => 6,
        _ => 5,
    }
}

/// Single-output calls from `Engine::call`.
pub(super) fn onl_call(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if !handles(name) {
        return Ok(None);
    }
    Ok(Some(onl_outputs(engine, name, args, 1, out)?.remove(0)))
}

/// `[a, b, ...] = name(args)`; the caller charges the outputs.
pub(super) fn onl_outputs(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    engine.check_cancel()?;
    if count > max_outputs(name) {
        return Err(format!("{name} does not support {count} outputs"));
    }
    if args.iter().any(|v| matches!(v, Value::QuotedString(_))) {
        return Err("String array storage is not implemented; use char(...) explicitly".into());
    }
    let args = engine.text_arguments(args.to_vec())?;
    let count = count.max(1);
    match name {
        "optimoptions" => Ok(vec![onl_optimoptions_with_out(engine, &args, out)?]),
        "fminunc" => onl_fminunc(engine, &args, count, out),
        "fsolve" => onl_fsolve(engine, &args, count, out),
        _ => onl_least_squares(engine, name, &args, count, out),
    }
}

// ---------------------------------------------------------------- options

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Display {
    Off,
    Final,
    Notify,
    Iter,
}

#[derive(Clone)]
pub(super) struct Options {
    pub(super) display: Display,
    pub(super) max_iterations: f64,
    pub(super) max_evaluations: Option<f64>,
    pub(super) optimality_tol: f64,
    pub(super) step_tol: f64,
    pub(super) function_tol: f64,
    pub(super) gradient: bool,
    pub(super) central: bool,
    pub(super) algorithm: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            display: Display::Final,
            max_iterations: 400.,
            max_evaluations: None,
            optimality_tol: 1e-6,
            step_tol: 1e-6,
            function_tol: 1e-6,
            gradient: false,
            central: false,
            algorithm: None,
        }
    }
}

/// Canonical option name for a given (case-insensitive) name; optimset's older
/// names map onto the newer ones. `TolFun` sets both tolerances it used to cover.
pub(super) fn canonical(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "constrainttolerance" | "tolcon" => "ConstraintTolerance",
        "specifyconstraintgradient" => "SpecifyConstraintGradient",
        "maxnodes" => "MaxNodes",
        "integertolerance" => "IntegerTolerance",
        "relativegaptolerance" => "RelativeGapTolerance",
        "absolutegaptolerance" => "AbsoluteGapTolerance",
        "display" => "Display",
        "maxiterations" | "maxiter" => "MaxIterations",
        "maxfunctionevaluations" | "maxfunevals" => "MaxFunctionEvaluations",
        "optimalitytolerance" => "OptimalityTolerance",
        "steptolerance" | "tolx" => "StepTolerance",
        "functiontolerance" => "FunctionTolerance",
        "tolfun" => "TolFun",
        "specifyobjectivegradient" | "gradobj" | "jacobian" => "SpecifyObjectiveGradient",
        "algorithm" => "Algorithm",
        "finitedifferencetype" | "findifftype" => "FiniteDifferenceType",
        _ => return None,
    })
}

/// Additional options accepted with a warning, scoped to the named solver. An
/// untagged update can use these spellings only for a field it already has.
/// R2024b solver tables:
/// https://www.mathworks.com/help/releases/R2024b/optim/ug/lsqnonlin.html
/// https://www.mathworks.com/help/releases/R2024b/optim/ug/fsolve.html
/// https://www.mathworks.com/help/releases/R2024b/optim/ug/fminunc.html
fn onl_ignored_option(solver: Option<&str>, name: &str) -> Option<&'static str> {
    let residual = matches!(solver, None | Some("lsqnonlin" | "fsolve"));
    let fminunc = matches!(solver, None | Some("fminunc"));
    if !residual && !fminunc {
        return None;
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "checkgradients" => "CheckGradients",
        "diagnostics" => "Diagnostics",
        "finitedifferencestepsize" => "FiniteDifferenceStepSize",
        "subproblemalgorithm" => "SubproblemAlgorithm",
        "typicalx" => "TypicalX",
        "useparallel" => "UseParallel",
        "hessianapproximation" if fminunc => "HessianApproximation",
        "outputfcn" if fminunc => "OutputFcn",
        "hessianfcn" if fminunc => "HessianFcn",
        "hessianmultiplyfcn" if fminunc => "HessianMultiplyFcn",
        "objectivelimit" if fminunc => "ObjectiveLimit",
        "derivativecheck" if residual => "CheckGradients",
        "findiffrelstep" if residual => "FiniteDifferenceStepSize",
        "diffminchange" if residual => "DiffMinChange",
        "diffmaxchange" if residual => "DiffMaxChange",
        "funvalcheck" if residual => "FunValCheck",
        "jacobianmultiplyfcn" | "jacobmult" if residual => "JacobianMultiplyFcn",
        "jacobpattern" if residual => "JacobPattern",
        "maxpcgiter" if residual => "MaxPCGIter",
        "precondbandwidth" if residual => "PrecondBandWidth",
        "tolpcg" if residual => "TolPCG",
        "initdamping" if residual => "InitDamping",
        "scaleproblem" if residual => "ScaleProblem",
        "barrierparamupdate" if matches!(solver, None | Some("lsqnonlin")) => "BarrierParamUpdate",
        "initbarrierparam" if matches!(solver, None | Some("lsqnonlin")) => "InitBarrierParam",
        _ => return None,
    })
}

fn onl_field_matches(name: &str, key: &str) -> bool {
    name.eq_ignore_ascii_case(key)
        || canonical(name) == Some(key)
        || onl_ignored_option(None, name) == Some(key)
}

/// Positive finite scalar, row, or column. A 2-D nonvector such as 2x2 is rejected.
fn onl_positive_finite_vector(value: &Value) -> bool {
    matches!(value, Value::Matrix(m) if !m.is_nd()
                && !m.is_empty()
                && (m.rows() == 1 || m.cols() == 1)
                && m.data().iter().all(|v| v.is_finite() && *v > 0.))
}

fn onl_strict_bool(engine: &Engine, who: &str, key: &str, value: &Value) -> Result<bool> {
    match (onl_scalar(value), onl_text(engine, value)) {
        (Some(v), _) if v == 0. || v == 1. => Ok(v == 1.),
        (_, Some(t)) if t.eq_ignore_ascii_case("on") => Ok(true),
        (_, Some(t)) if t.eq_ignore_ascii_case("off") => Ok(false),
        _ => Err(format!(
            "{who}: {key} must be true, false, 0, 1, 'on' or 'off', and finite"
        )),
    }
}

fn onl_bool_or_on_off(engine: &Engine, who: &str, key: &str, value: &Value) -> Result<bool> {
    match (onl_scalar(value), onl_text(engine, value)) {
        (Some(v), _) => Ok(v != 0.),
        (_, Some(t)) if t.eq_ignore_ascii_case("on") => Ok(true),
        (_, Some(t)) if t.eq_ignore_ascii_case("off") => Ok(false),
        _ => Err(format!("{who}: {key} must be true, false, 'on' or 'off'")),
    }
}

fn onl_validate_ignored(engine: &Engine, who: &str, key: &str, value: &Value) -> Result<()> {
    if is_empty_value(value) {
        return Ok(());
    }
    match key {
        "CheckGradients" | "UseParallel" => {
            onl_strict_bool(engine, who, key, value)?;
        }
        "Diagnostics" | "FunValCheck" => {
            let text = onl_text(engine, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "on" | "off") {
                return Err(format!("{who}: {key} must be 'on' or 'off'"));
            }
        }
        "FiniteDifferenceStepSize" => {
            if !onl_positive_finite_vector(value) {
                return Err(format!(
                    "{who}: FiniteDifferenceStepSize must be a finite positive scalar or vector"
                ));
            }
        }
        "JacobianMultiplyFcn" => {
            if !matches!(value, Value::Container(Array::Handle(_))) {
                return Err(format!("{who}: {key} must be a function handle or []"));
            }
        }
        "JacobPattern" => {
            use crate::sparse::SparseValues;
            let valid = match value {
                Value::Matrix(m) => !m.is_nd() && m.data().iter().all(|&x| x == 0. || x == 1.),
                Value::Logical(m) => !m.is_nd(),
                Value::Container(Array::Sparse(m)) => match m.values() {
                    SparseValues::Double(v) => v.iter().all(|&x| x == 1.),
                    SparseValues::Logical(_) => true,
                },
                _ => false,
            };
            if !valid {
                return Err(format!("{who}: {key} must be a matrix of zeros and ones"));
            }
        }
        "ScaleProblem" | "BarrierParamUpdate" => {
            let choices = if key == "ScaleProblem" {
                ["none", "jacobian"]
            } else {
                ["monotone", "predictor-corrector"]
            };
            if !onl_text(engine, value)
                .is_some_and(|v| choices.iter().any(|c| v.eq_ignore_ascii_case(c)))
            {
                return Err(format!(
                    "{who}: {key} must be '{}' or '{}'",
                    choices[0], choices[1]
                ));
            }
        }
        "DiffMinChange" | "DiffMaxChange" | "InitDamping" | "InitBarrierParam" | "MaxPCGIter"
        | "PrecondBandWidth" | "TolPCG" => {
            let valid = onl_scalar(value).is_some_and(|x| match key {
                "DiffMinChange" => x.is_finite() && x >= 0.,
                "DiffMaxChange" => x > 0.,
                "PrecondBandWidth" => x == f64::INFINITY || (x >= 0. && x.fract() == 0.),
                _ => x.is_finite() && x > 0.,
            });
            if !valid {
                return Err(format!("{who}: invalid numeric value for {key}"));
            }
        }
        "HessianApproximation" => {
            let text = onl_text(engine, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "bfgs" | "lbfgs") {
                return Err(format!(
                    "{who}: HessianApproximation must be 'bfgs' or 'lbfgs'"
                ));
            }
        }
        "OutputFcn" => engine.polyopt_parse_fcns(who, key, value, &mut Vec::new())?,
        "HessianFcn" => {
            let text = onl_text(engine, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if text != "objective" {
                return Err(format!("{who}: HessianFcn must be 'objective' or []"));
            }
        }
        "HessianMultiplyFcn" => {
            let valid_handle = matches!(value, Value::Container(Array::Handle(_)));
            let valid_name = onl_text(engine, value).is_some_and(|s| !s.trim().is_empty());
            if !valid_handle && !valid_name {
                return Err(format!(
                    "{who}: HessianMultiplyFcn must be a function handle, function name or []"
                ));
            }
        }
        "ObjectiveLimit" => match onl_scalar(value) {
            Some(v) if !v.is_nan() => {}
            _ => return Err(format!("{who}: ObjectiveLimit must be a real scalar")),
        },
        "SubproblemAlgorithm" => {
            let text = onl_text(engine, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "cg" | "factorization") {
                return Err(format!(
                    "{who}: SubproblemAlgorithm must be 'cg' or 'factorization'"
                ));
            }
        }
        "TypicalX" => match value {
            Value::Matrix(m) if !m.is_empty() && m.data().iter().all(|v| v.is_finite()) => {}
            _ => return Err(format!("{who}: TypicalX must be a finite real array")),
        },
        _ => return Err(format!("{who}: option '{key}' is not implemented")),
    }
    Ok(())
}

/// Map an explicit algorithm preference to the implemented method. The
/// constructor preserves the requested value in the public options struct.
fn onl_algorithm<'a>(solver: &str, requested: &'a str) -> Result<&'a str> {
    let allowed = match solver {
        "fminunc" => match requested {
            "quasi-newton" => return Ok(requested),
            "trust-region" => return Ok("quasi-newton"),
            _ => "'quasi-newton' or 'trust-region'",
        },
        "lsqnonlin" => match requested {
            "levenberg-marquardt" => return Ok(requested),
            "trust-region-reflective" | "interior-point" => return Ok("levenberg-marquardt"),
            _ => "'trust-region-reflective', 'levenberg-marquardt' or 'interior-point'",
        },
        "fsolve" => match requested {
            "trust-region-dogleg" | "levenberg-marquardt" => return Ok(requested),
            "trust-region" => return Ok("levenberg-marquardt"),
            _ => "'trust-region-dogleg', 'trust-region' or 'levenberg-marquardt'",
        },
        _ => return Ok(requested),
    };
    Err(format!(
        "{solver}: Algorithm '{requested}' is not supported; use {allowed}"
    ))
}

fn onl_warning(
    engine: &mut Engine,
    out: &mut Execution,
    id: String,
    message: String,
) -> Result<()> {
    let args = [
        Value::Text(id),
        Value::Text("%s".into()),
        Value::Text(message),
    ];
    engine.lang_utils_call("warning", &args, out)?;
    Ok(())
}

pub(super) fn polyopt_option_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "outputfcn" => Some("OutputFcn"),
        "plotfcns" => Some("PlotFcns"),
        "funvalcheck" => Some("FunValCheck"),
        _ => None,
    }
}

pub(super) fn onl_text(engine: &Engine, value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) => c.to_utf8(&engine.cancel).ok(),
        _ => None,
    }
}

pub(super) fn onl_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        Value::Logical(m) if m.len() == 1 => Some(f64::from(m.data()[0])),
        _ => None,
    }
}

pub(super) fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Matrix(m) => m.is_empty(),
        Value::Logical(m) => m.is_empty(),
        Value::Text(s) => s.is_empty(),
        Value::Container(Array::Character(c)) => c.rows() == 0 || c.cols() == 0,
        _ => false,
    }
}

/// Apply one option to `options`; `key` is canonical. Empty values keep the default.
pub(super) fn onl_set_option(
    engine: &Engine,
    options: &mut Options,
    who: &str,
    key: &str,
    value: &Value,
) -> Result<()> {
    if is_empty_value(value) {
        return Ok(());
    }
    let count = |what: &str| -> Result<f64> {
        match onl_scalar(value) {
            Some(v) if v >= 0. && (v.fract() == 0. || v == f64::INFINITY) => Ok(v),
            _ => Err(format!(
                "{who}: {what} must be a nonnegative integer or Inf"
            )),
        }
    };
    let tolerance = |what: &str| -> Result<f64> {
        match onl_scalar(value) {
            Some(v) if v > 0. && v.is_finite() => Ok(v),
            _ => Err(format!("{who}: {what} must be a finite positive scalar")),
        }
    };
    match key {
        "Display" => {
            let text = onl_text(engine, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            options.display = match text.as_str() {
                "off" | "none" => Display::Off,
                "final" | "final-detailed" => Display::Final,
                "notify" | "notify-detailed" => Display::Notify,
                "iter" | "iter-detailed" => Display::Iter,
                _ => {
                    return Err(format!(
                        "{who}: Display must be 'off', 'final', 'notify' or 'iter'"
                    ));
                }
            };
        }
        "MaxIterations" => options.max_iterations = count("MaxIterations")?,
        "MaxFunctionEvaluations" => {
            options.max_evaluations = Some(count("MaxFunctionEvaluations")?)
        }
        "OptimalityTolerance" => options.optimality_tol = tolerance("OptimalityTolerance")?,
        "StepTolerance" => options.step_tol = tolerance("StepTolerance")?,
        "FunctionTolerance" => options.function_tol = tolerance("FunctionTolerance")?,
        "TolFun" => {
            let v = tolerance("TolFun")?;
            options.function_tol = v;
            options.optimality_tol = v;
        }
        "SpecifyObjectiveGradient" => {
            options.gradient = if matches!(who, "lsqnonlin" | "fsolve") {
                onl_strict_bool(engine, who, key, value)?
            } else {
                onl_bool_or_on_off(engine, who, key, value)?
            };
        }
        "FiniteDifferenceType" => {
            let text = onl_text(engine, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            options.central = match text.as_str() {
                "forward" => false,
                "central" => true,
                _ => {
                    return Err(format!(
                        "{who}: FiniteDifferenceType must be 'forward' or 'central'"
                    ));
                }
            };
        }
        "Algorithm" => {
            let Some(text) = onl_text(engine, value) else {
                return Err(format!("{who}: Algorithm must be text"));
            };
            options.algorithm = Some(onl_algorithm(who, &text.to_ascii_lowercase())?.into());
        }
        _ => return Err(format!("{who}: option '{key}' is not implemented")),
    }
    Ok(())
}

fn onl_solver_canonical(solver: Option<&str>, name: &str) -> Option<&'static str> {
    if matches!(solver, Some("lsqnonlin" | "fsolve")) {
        if name.eq_ignore_ascii_case("GradObj") {
            return None;
        }
        if name.eq_ignore_ascii_case("GradConstr") {
            return Some("SpecifyConstraintGradient");
        }
    }
    canonical(name)
}

/// Options from an optimset or optimoptions struct, or `[]` for the defaults. Fields
/// outside the solver's implemented and accepted-with-warning sets must be empty.
fn onl_options(engine: &Engine, who: &str, value: Option<&Value>) -> Result<Options> {
    let mut options = Options::default();
    match value {
        None => {}
        Some(v) if is_empty_value(v) => {}
        Some(Value::Container(Array::Struct(s))) if s.len() == 1 => {
            for (i, name) in s.field_names().iter().enumerate() {
                let field = Value::from(s.get(i, 0)?.as_ref().clone());
                if is_empty_value(&field) {
                    continue;
                }
                if let Some(key) = onl_ignored_option(Some(who), name) {
                    onl_validate_ignored(engine, who, key, &field)?;
                } else if let Some(key) = onl_solver_canonical(Some(who), name) {
                    onl_set_option(engine, &mut options, who, key, &field)?;
                } else {
                    return Err(format!("{who}: option '{name}' is not implemented"));
                }
            }
        }
        Some(_) => {
            return Err(format!(
                "{who}: options must be a scalar structure from optimoptions or optimset"
            ));
        }
    }
    Ok(options)
}

/// `optimoptions(solver, Name, Value, ...)` or `optimoptions(oldopts, Name, Value, ...)`:
/// a struct of the named options under their current names, checked as they are set.
pub(super) fn onl_optimoptions(engine: &Engine, args: &[Value]) -> Result<Value> {
    Ok(onl_optimoptions_inner(engine, args)?.0)
}

fn onl_optimoptions_with_out(
    engine: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    let (result, pending_warnings) = onl_optimoptions_inner(engine, args)?;
    for (id, message) in pending_warnings {
        onl_warning(engine, out, id, message)?;
    }
    Ok(result)
}

fn onl_optimoptions_inner(
    engine: &Engine,
    args: &[Value],
) -> Result<(Value, Vec<(String, String)>)> {
    let Some(first) = args.first() else {
        return Err("Not enough input arguments.".into());
    };
    let (mut names, mut slots, solver): (Vec<String>, Vec<Arc<Array>>, Option<String>) = match first
    {
        Value::Container(Array::Struct(s)) if s.len() == 1 => {
            let slots = (0..s.field_count())
                .map(|f| s.get(f, 0).cloned())
                .collect::<Result<Vec<_>>>()?;
            (s.field_names().to_vec(), slots, None)
        }
        _ => {
            let solver = match first {
                Value::Container(Array::Handle(h)) => h.func2str().to_string(),
                other => onl_text(engine, other).ok_or(
                    "optimoptions: the first input must be a solver name, a solver handle or an options structure",
                )?,
            };
            if !SOLVERS.contains(&solver.as_str()) {
                return Err(format!(
                    "optimoptions: solver '{solver}' is not supported; supported solvers: {}",
                    SOLVERS.join(", ")
                ));
            }
            (Vec::new(), Vec::new(), Some(solver))
        }
    };
    let pairs = &args[1..];
    if !pairs.len().is_multiple_of(2) {
        return Err("Arguments must occur in name-value pairs.".into());
    }
    let basic_solver = solver
        .as_deref()
        .is_some_and(|s| matches!(s, "fzero" | "fminbnd" | "fminsearch"));
    let solver = solver.as_deref();
    let original = match first {
        Value::Container(Array::Struct(s)) => Some(s),
        _ => None,
    };
    let mut check = Options::default();
    let mut warning_fields = BTreeSet::new();
    for pair in pairs.chunks(2) {
        engine.check_cancel()?;
        let given = onl_text(engine, &pair[0])
            .ok_or("optimoptions: option names must be character vectors")?;
        let callback_option = basic_solver.then(|| polyopt_option_name(&given)).flatten();
        let ignored = onl_ignored_option(solver, &given);
        let Some(key) = onl_solver_canonical(solver, &given)
            .or(callback_option)
            .or(ignored)
        else {
            return Err(format!("optimoptions: unknown option '{given}'"));
        };
        let value = &pair[1];
        if ignored.is_some() {
            if original.is_some_and(|s| !s.field_names().iter().any(|n| onl_field_matches(n, key)))
            {
                return Err(format!(
                    "optimoptions: option '{key}' is solver-specific and this options structure does not name a solver. Pass the solver name to set this option."
                ));
            }
            onl_validate_ignored(engine, "optimoptions", key, value)?;
        } else {
            match callback_option {
                Some("OutputFcn" | "PlotFcns") => {
                    engine.polyopt_parse_fcns("optimoptions", key, value, &mut Vec::new())?;
                }
                Some("FunValCheck") => {
                    polyopt_fun_val_check(engine, "optimoptions", value)?;
                }
                _ => onl_set_option(
                    engine,
                    &mut check,
                    solver.unwrap_or("optimoptions"),
                    key,
                    value,
                )?,
            }
        }
        let keys: &[&str] = if key == "TolFun" {
            &["FunctionTolerance", "OptimalityTolerance"]
        } else {
            &[key]
        };
        for &key in keys {
            let value = Arc::new(pair[1].clone().array(&engine.cancel)?);
            let index = if let Some(i) = names.iter().position(|n| onl_field_matches(n, key)) {
                names[i] = key.to_string();
                slots[i] = value;
                i
            } else {
                names.push(key.to_string());
                slots.push(value);
                slots.len() - 1
            };
            if ignored.is_some() || key == "Algorithm" {
                warning_fields.insert(index);
            }
        }
    }
    // Examine the final setting of each field after all pairs validate. Aliases,
    // overwritten settings and old-struct copies must not repeat a warning.
    let mut pending_warnings = Vec::new();
    for index in warning_fields {
        let prior = match original {
            Some(s) if index < s.field_count() => {
                Some(Value::from(s.get(index, 0)?.as_ref().clone()))
            }
            _ => None,
        };
        let value = Value::from(slots[index].as_ref().clone());
        if let Some(warning) =
            onl_option_warning(engine, solver, &names[index], &value, prior.as_ref())?
        {
            pending_warnings.push(warning);
        }
    }
    let result = Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        names,
        slots,
        &engine.cancel,
    )?));
    Ok((result, pending_warnings))
}

/// Called only for an explicitly set Algorithm or a validated, ignored option.
fn onl_option_warning(
    engine: &Engine,
    solver: Option<&str>,
    key: &str,
    value: &Value,
    prior: Option<&Value>,
) -> Result<Option<(String, String)>> {
    if is_empty_value(value) {
        return Ok(None);
    }
    if let Some(solver) = solver {
        let detail = match key {
            "Algorithm" => {
                let preference = onl_text(engine, value)
                    .ok_or("optimoptions: Algorithm must be text")?
                    .to_ascii_lowercase();
                let effective = onl_algorithm(solver, &preference)?;
                if preference == effective {
                    return Ok(None);
                }
                let method = if solver == "fminunc" { " (BFGS)" } else { "" };
                format!(
                    "value '{preference}' is not implemented; falling back to '{effective}'{method}."
                )
            }
            "JacobianMultiplyFcn" => {
                "is ignored; SpecifyObjectiveGradient requires a full residual Jacobian.".into()
            }
            _ => "is not implemented in this version and will be ignored.".into(),
        };
        return Ok(Some((
            format!("optimoptions:{solver}:{key}"),
            format!("optimoptions: {solver} option '{key}' {detail}"),
        )));
    }
    // A struct records the requested values, not the solver that created it.
    let message = if key == "Algorithm" {
        let preference = onl_text(engine, value)
            .ok_or("optimoptions: Algorithm must be text")?
            .to_ascii_lowercase();
        if prior
            .and_then(|v| onl_text(engine, v))
            .is_some_and(|prev| preference.eq_ignore_ascii_case(&prev))
        {
            return Ok(None);
        }
        let detail = match preference.as_str() {
            "trust-region" => {
                "fminunc uses 'quasi-newton' and fsolve uses 'levenberg-marquardt' for this request."
            }
            "trust-region-reflective" | "interior-point" => {
                "Name the solver to check the available algorithms and fallbacks."
            }
            _ => return Ok(None),
        };
        format!("optimoptions: Algorithm '{preference}' was set without a solver name. {detail}")
    } else if prior.is_some_and(is_empty_value) {
        format!(
            "optimoptions: '{key}' was set without a solver name. Name the solver to check support for this setting."
        )
    } else {
        return Ok(None);
    };
    Ok(Some((format!("optimoptions:{key}"), message)))
}

pub(super) fn polyopt_fun_val_check(engine: &Engine, who: &str, value: &Value) -> Result<bool> {
    if is_empty_value(value) {
        return Ok(false);
    }
    match onl_text(engine, value)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{who}: FunValCheck must be 'on' or 'off'")),
    }
}

// ---------------------------------------------------------------- shared pieces

pub(super) fn onl_real_vector(value: &Value, who: &str, what: &str) -> Result<Matrix> {
    match value {
        Value::Matrix(m) => Ok(m.clone()),
        Value::Logical(l) => Matrix::from_column_vec(
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        Value::Complex(_) => Err(format!("{who}: {what} must be real")),
        _ => Err(format!("{who}: {what} must be a real numeric array")),
    }
}

pub(super) fn onl_handle(engine: &Engine, who: &str, value: &Value) -> Result<FunctionHandle> {
    match value {
        Value::Container(Array::Handle(h)) => Ok(h.clone()),
        other => match onl_text(engine, other) {
            Some(name) if !name.trim().is_empty() => match engine.named_handle(name.trim())? {
                Value::Container(Array::Handle(h)) => Ok(h),
                _ => unreachable!("named_handle returns a handle"),
            },
            _ => Err(format!(
                "{who}: the function must be a function handle or a function name"
            )),
        },
    }
}

fn onl_check_size(who: &str, n: usize, m: usize) -> Result<()> {
    if n == 0 {
        return Err(format!("{who}: the starting point x0 must not be empty"));
    }
    if n > MAX_UNKNOWNS || m.saturating_mul(n) > MAX_JACOBIAN {
        return Err(format!(
            "{who}: the problem is too large ({m} values of {n} unknowns); at most {MAX_UNKNOWNS} unknowns and {MAX_JACOBIAN} Jacobian elements"
        ));
    }
    Ok(())
}

fn inf_norm(v: &[f64]) -> f64 {
    if v.iter().any(|x| x.is_nan()) {
        f64::NAN
    } else {
        v.iter().fold(0f64, |a, x| a.max(x.abs()))
    }
}
fn two_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

type EvaluatedValue = (Vec<f64>, Option<Matrix>);

/// The function being solved, its argument shape and the counters of one run.
struct Problem {
    who: &'static str,
    handle: FunctionHandle,
    rows: usize,
    cols: usize,
    /// Arguments after x (lsqcurvefit's xdata).
    extra: Vec<Value>,
    /// Subtracted from the values (lsqcurvefit's ydata).
    target: Option<Vec<f64>>,
    evaluations: f64,
    max_evaluations: f64,
    /// Shape of the last value array.
    value_shape: (usize, usize),
}

impl Problem {
    /// Values (flattened) and, when `derivative`, the second output as given.
    fn eval(
        &mut self,
        engine: &mut Engine,
        x: &[f64],
        derivative: bool,
        out: &mut Execution,
    ) -> Result<Option<EvaluatedValue>> {
        engine.check_cancel()?;
        if self.evaluations >= self.max_evaluations {
            return Ok(None);
        }
        let mut args = vec![Value::Matrix(Matrix::from_column_slice(
            self.rows, self.cols, x,
        )?)];
        args.extend(self.extra.iter().cloned());
        let requested = if derivative { 2 } else { 1 };
        let mut values = engine.call_handle(&self.handle, args, requested, None, out)?;
        self.evaluations += 1.;
        if values.len() < requested {
            return Err(format!(
                "{}: the function must return {requested} output(s) here",
                self.who
            ));
        }
        let first = onl_real_vector(&values[0], self.who, "the function value")?;
        // Later solver steps index by the first call's length; a changed length must not reach them.
        let (r, c) = self.value_shape;
        if self.evaluations > 1. && first.rows() * first.cols() != r * c {
            return Err(format!(
                "{}: the function returned {} values, but {} at the first call; its result length must not change",
                self.who,
                first.rows() * first.cols(),
                r * c
            ));
        }
        self.value_shape = (first.rows(), first.cols());
        let mut data = first.data().to_vec();
        if let Some(target) = &self.target {
            if target.len() != data.len() {
                return Err(format!(
                    "{}: the function returned {} values but ydata has {}",
                    self.who,
                    data.len(),
                    target.len()
                ));
            }
            for (v, t) in data.iter_mut().zip(target) {
                *v -= t;
            }
        }
        let second = if derivative {
            Some(onl_real_vector(
                &values.swap_remove(1),
                self.who,
                "the derivative",
            )?)
        } else {
            None
        };
        Ok(Some((data, second)))
    }

    /// Jacobian m x n of the values at x (given `fx`): the user's when `given`, else
    /// finite differences that step inwards at a bound.
    #[allow(clippy::too_many_arguments)]
    fn jacobian(
        &mut self,
        engine: &mut Engine,
        x: &[f64],
        fx: &[f64],
        given: Option<Matrix>,
        central: bool,
        bounds: Option<(&[f64], &[f64])>,
        out: &mut Execution,
    ) -> Result<Option<DMatrix<f64>>> {
        let (m, n) = (fx.len(), x.len());
        if let Some(j) = given {
            let ok = (j.rows() == m && j.cols() == n) || (m == 1 && j.len() == n);
            if !ok {
                return Err(format!(
                    "{}: the derivative output must be {m}-by-{n}, not {}-by-{}",
                    self.who,
                    j.rows(),
                    j.cols()
                ));
            }
            return Ok(Some(DMatrix::from_column_slice(m, n, j.data())));
        }
        let mut jac = DMatrix::zeros(m, n);
        let mut probe = x.to_vec();
        for k in 0..n {
            engine.check_cancel()?;
            let scale = x[k].abs().max(1.);
            let (lo, hi) = bounds.map_or((f64::NEG_INFINITY, f64::INFINITY), |(l, u)| (l[k], u[k]));
            let sign = if x[k] < 0. { -1. } else { 1. };
            if central {
                let h = f64::EPSILON.cbrt() * scale;
                if x[k] - h >= lo && x[k] + h <= hi {
                    probe[k] = x[k] + h;
                    let Some((fp, _)) = self.eval(engine, &probe, false, out)? else {
                        return Ok(None);
                    };
                    probe[k] = x[k] - h;
                    let Some((fm, _)) = self.eval(engine, &probe, false, out)? else {
                        return Ok(None);
                    };
                    let width = (x[k] + h) - (x[k] - h);
                    for i in 0..m {
                        jac[(i, k)] = (fp[i] - fm[i]) / width;
                    }
                    probe[k] = x[k];
                    continue;
                }
            }
            let mut h = f64::EPSILON.sqrt() * scale * sign;
            if x[k] + h > hi || x[k] + h < lo {
                h = -h;
            }
            probe[k] = x[k] + h;
            let step = probe[k] - x[k];
            let Some((fp, _)) = self.eval(engine, &probe, false, out)? else {
                return Ok(None);
            };
            for i in 0..m {
                jac[(i, k)] = (fp[i] - fx[i]) / step;
            }
            probe[k] = x[k];
        }
        Ok(Some(jac))
    }

    fn value_matrix(&self, data: &[f64]) -> Result<Value> {
        let (r, c) = self.value_shape;
        let (r, c) = if r * c == data.len() {
            (r, c)
        } else {
            (data.len(), 1)
        };
        Ok(Value::Matrix(Matrix::from_column_slice(r, c, data)?))
    }
    fn x_value(&self, x: &[f64]) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_slice(
            self.rows, self.cols, x,
        )?))
    }
}

fn evaluation_limit(options: &Options, n: usize) -> f64 {
    options.max_evaluations.unwrap_or(100. * n as f64)
}

fn onl_limit_reason(problem: &Problem) -> (f64, String) {
    (
        0.,
        format!(
            "it reached MaxFunctionEvaluations = {}",
            problem.max_evaluations
        ),
    )
}

pub(super) fn onl_struct(engine: &Engine, fields: Vec<(&str, Array)>) -> Result<Value> {
    let (names, slots) = fields
        .into_iter()
        .map(|(name, value)| (name.to_string(), Arc::new(value)))
        .unzip();
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        names,
        slots,
        &engine.cancel,
    )?)))
}

/// The `output` struct shared by the solvers.
struct Report<'a> {
    iterations: usize,
    evaluations: f64,
    step: f64,
    optimality: f64,
    algorithm: &'a str,
    message: String,
}

fn onl_report(engine: &Engine, report: Report<'_>) -> Result<Value> {
    let text = |s: &str| -> Result<Array> {
        Ok(Array::Character(CharacterRow::from_units(
            s.encode_utf16().collect(),
            &engine.cancel,
        )?))
    };
    onl_struct(
        engine,
        vec![
            (
                "iterations",
                Array::Numeric(Matrix::scalar(report.iterations as f64)),
            ),
            (
                "funcCount",
                Array::Numeric(Matrix::scalar(report.evaluations)),
            ),
            ("stepsize", Array::Numeric(Matrix::scalar(report.step))),
            (
                "firstorderopt",
                Array::Numeric(Matrix::scalar(report.optimality)),
            ),
            ("algorithm", text(report.algorithm)?),
            ("message", text(&report.message)?),
        ],
    )
}

pub(super) fn onl_final(
    engine: &mut Engine,
    options: &Options,
    flag: f64,
    message: &str,
    out: &mut Execution,
) -> Result<()> {
    let show = match options.display {
        Display::Off => false,
        Display::Notify => flag <= 0.,
        Display::Final | Display::Iter => true,
    };
    if show {
        engine.emit(out, message.to_string())?;
    }
    Ok(())
}

fn short(v: f64) -> String {
    format!("{v:.4e}")
}

pub(super) fn onl_check_finite(who: &str, what: &str, v: &[f64]) -> Result<()> {
    if v.iter().all(|x| x.is_finite()) {
        Ok(())
    } else {
        Err(format!("{who}: {what} at the starting point is not finite"))
    }
}

// ---------------------------------------------------------------- fminunc

/// Objective value and gradient at x.
fn onl_value_gradient(
    engine: &mut Engine,
    problem: &mut Problem,
    options: &Options,
    x: &[f64],
    out: &mut Execution,
) -> Result<Option<(f64, Vec<f64>)>> {
    let Some((values, given)) = problem.eval(engine, x, options.gradient, out)? else {
        return Ok(None);
    };
    let [f] = values[..] else {
        return Err("fminunc: the objective must return a real scalar".into());
    };
    if !f.is_finite() || f < -1e20 {
        return Ok(Some((f, vec![f64::NAN; x.len()])));
    }
    let Some(jac) = problem.jacobian(engine, x, &values, given, options.central, None, out)? else {
        return Ok(None);
    };
    Ok(Some((f, jac.iter().copied().collect())))
}

struct Trial {
    alpha: f64,
    x: Vec<f64>,
    f: f64,
    g: Vec<f64>,
}

/// Line search for the strong Wolfe conditions along p from x (Nocedal & Wright 2006,
/// Algorithm 3.5 with zoom, Algorithm 3.6). At most 30 trial points across
/// bracketing and zoom; all finite differences also consume the global budget.
/// None when the search or evaluation budget is exhausted.
#[allow(clippy::too_many_arguments)]
fn onl_wolfe(
    engine: &mut Engine,
    problem: &mut Problem,
    options: &Options,
    x: &[f64],
    f0: f64,
    g0: &[f64],
    p: &[f64],
    alpha0: f64,
    out: &mut Execution,
) -> Result<Option<Trial>> {
    const C1: f64 = 1e-4;
    const C2: f64 = 0.9;
    const SEARCH_STEPS: usize = 30;
    let d0 = dot(g0, p);
    let mut at =
        |engine: &mut Engine, problem: &mut Problem, alpha: f64| -> Result<Option<Trial>> {
            let xt: Vec<f64> = x.iter().zip(p).map(|(a, b)| a + alpha * b).collect();
            Ok(
                onl_value_gradient(engine, problem, options, &xt, out)?.map(|(f, g)| Trial {
                    alpha,
                    x: xt,
                    f,
                    g,
                }),
            )
        };
    let slope = |t: &Trial| dot(&t.g, p);
    let armijo = |t: &Trial| t.f <= f0 + C1 * t.alpha * d0;
    let curvature = |t: &Trial| slope(t).abs() <= -C2 * d0;
    let origin = Trial {
        alpha: 0.,
        x: x.to_vec(),
        f: f0,
        g: g0.to_vec(),
    };
    let mut previous = origin;
    let mut alpha = alpha0;
    let mut trials = 0;
    let (mut lo, mut hi) = 'bracket: {
        while trials < SEARCH_STEPS {
            engine.check_cancel()?;
            let Some(t) = at(engine, problem, alpha)? else {
                return Ok(None);
            };
            trials += 1;
            if t.f < -1e20 || alpha * two_norm(p) > 1e20 {
                return Ok(Some(t));
            }
            if !t.f.is_finite() || !armijo(&t) || (trials > 1 && t.f >= previous.f) {
                break 'bracket (previous, t);
            }
            if curvature(&t) {
                return Ok(Some(t));
            }
            if slope(&t) >= 0. {
                break 'bracket (t, previous);
            }
            // Geometric expansion exposes an unbounded descent within the default
            // budget as well as allowing long steps on poorly scaled objectives.
            alpha *= 10.;
            previous = t;
        }
        return Ok(None);
    };
    while trials < SEARCH_STEPS {
        engine.check_cancel()?;
        let width = hi.alpha - lo.alpha;
        // Cubic through lo and hi (eq. 3.59), kept inside the middle 80 %.
        let (dl, dh) = (slope(&lo), slope(&hi));
        let d1 = dl + dh - 3. * (lo.f - hi.f) / (lo.alpha - hi.alpha);
        let d2 = width.signum() * (d1 * d1 - dl * dh).sqrt();
        let cubic = hi.alpha - (hi.alpha - lo.alpha) * (dh + d2 - d1) / (dh - dl + 2. * d2);
        let (a, b) = (lo.alpha + 0.1 * width, hi.alpha - 0.1 * width);
        let inside = cubic.is_finite() && (cubic - a) * (cubic - b) <= 0.;
        let trial_alpha = if inside {
            cubic
        } else {
            lo.alpha + 0.5 * width
        };
        if trial_alpha == lo.alpha || trial_alpha == hi.alpha {
            break;
        }
        let Some(t) = at(engine, problem, trial_alpha)? else {
            return Ok(None);
        };
        trials += 1;
        if t.f < -1e20 || trial_alpha * two_norm(p) > 1e20 {
            return Ok(Some(t));
        }
        if !t.f.is_finite() || !armijo(&t) || t.f >= lo.f {
            hi = t;
        } else {
            if curvature(&t) {
                return Ok(Some(t));
            }
            if slope(&t) * width >= 0. {
                hi = lo;
            }
            lo = t;
        }
    }
    Ok(None)
}

/// Nocedal & Wright (2006), Algorithm 3.1: Armijo backtracking along -g.
#[allow(clippy::too_many_arguments)]
fn onl_armijo(
    engine: &mut Engine,
    problem: &mut Problem,
    options: &Options,
    x: &[f64],
    f: f64,
    g: &[f64],
    scale: f64,
    out: &mut Execution,
) -> Result<Option<Trial>> {
    let mut alpha = 1. / scale;
    for _ in 0..30 {
        let xt: Vec<f64> = x.iter().zip(g).map(|(x, g)| x - alpha * g).collect();
        let Some((ft, gt)) = onl_value_gradient(engine, problem, options, &xt, out)? else {
            return Ok(None);
        };
        if ft < -1e20
            || alpha * two_norm(g) > 1e20
            || (ft.is_finite() && ft < f && ft <= f - 1e-4 * alpha * dot(g, g))
        {
            return Ok(Some(Trial {
                alpha,
                x: xt,
                f: ft,
                g: gt,
            }));
        }
        alpha *= 0.5;
    }
    Ok(None)
}

/// Curvature safeguard from amendment 4, before any scaling or BFGS update.
pub(super) fn onl_bfgs_update(
    b: &mut DMatrix<f64>,
    fresh: &mut bool,
    s: &[f64],
    y: &[f64],
) -> Option<f64> {
    let sy = dot(s, y);
    if sy <= 1e-10 * two_norm(s) * two_norm(y) || !sy.is_finite() {
        return None;
    }
    let scale = dot(y, y) / sy;
    let sv = DVector::from_column_slice(s);
    let yv = DVector::from_column_slice(y);
    if *fresh {
        // Nocedal & Wright eq. 6.20, applied to B = H^-1.
        *b = DMatrix::identity(s.len(), s.len()) * scale;
    }
    let bs = &*b * &sv;
    let sbs = sv.dot(&bs);
    if sbs > 0. {
        *b += &yv * yv.transpose() / sy - &bs * bs.transpose() / sbs;
    }
    *fresh = false;
    Some(scale)
}

fn onl_fminunc(
    engine: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    const WHO: &str = "fminunc";
    if args.len() < 2 {
        return Err("Not enough input arguments.".into());
    }
    if args.len() > 3 {
        return Err("Too many input arguments.".into());
    }
    let handle = onl_handle(engine, WHO, &args[0])?;
    let x0 = onl_real_vector(&args[1], WHO, "x0")?;
    let options = onl_options(engine, WHO, args.get(2))?;
    // OutputFcn is accepted but never called: say so at solve time too.
    if let Some(Value::Container(Array::Struct(st))) = args.get(2)
        && let Some(i) = st
            .field_names()
            .iter()
            .position(|n| n.eq_ignore_ascii_case("OutputFcn"))
        && !is_empty_value(&Value::from(st.get(i, 0)?.as_ref().clone()))
    {
        onl_warning(
            engine,
            out,
            "optimoptions:fminunc:OutputFcn".into(),
            "optimoptions: fminunc option 'OutputFcn' is not implemented in this version and will be ignored.".into(),
        )?;
    }
    let n = x0.len();
    onl_check_size(WHO, n, n)?;
    onl_check_finite(WHO, "x0", x0.data())?;
    let mut problem = Problem {
        who: WHO,
        handle,
        rows: x0.rows(),
        cols: x0.cols(),
        extra: Vec::new(),
        target: None,
        evaluations: 0.,
        max_evaluations: evaluation_limit(&options, n),
        value_shape: (1, 1),
    };
    let mut x = x0.data().to_vec();
    let initial = problem.eval(engine, &x, options.gradient, out)?;
    let (values, given) = initial.unwrap_or((vec![f64::NAN], None));
    let [mut f] = values[..] else {
        return Err("fminunc: the objective must return a real scalar".into());
    };
    if problem.evaluations > 0. && !f.is_finite() {
        return Err("fminunc: the objective at the starting point is not finite".into());
    }
    let jac = problem.jacobian(engine, &x, &values, given, options.central, None, out)?;
    let ready = jac.is_some();
    let mut g: Vec<f64> = jac.map_or_else(|| vec![f64::NAN; n], |j| j.iter().copied().collect());
    if ready {
        onl_check_finite(WHO, "the gradient", &g)?;
    }
    let opt_scale = inf_norm(&g).max(1.);
    let mut b = DMatrix::<f64>::identity(n, n);
    // Initial inverse-Hessian scale limits steepest descent to a unit-sized step;
    // subsequent resets use the most recent valid secant scaling (eq. 6.20).
    let mut b_scale = inf_norm(&g).max(1.);
    let mut fresh = true;
    let mut iterations = 0usize;
    let mut last_step = 0.;
    if options.display == Display::Iter {
        engine.emit(
            out,
            " iter  f-count        f(x)   step length  optimality".into(),
        )?;
        engine.emit(
            out,
            format!(
                "{:>5} {:>8} {:>11} {:>13} {:>11}",
                0,
                problem.evaluations,
                short(f),
                "",
                short(inf_norm(&g))
            ),
        )?;
    }
    let (flag, reason) = loop {
        engine.check_cancel()?;
        if f < -1e20 {
            break (-3., "the objective decreased without bound".to_string());
        }
        if !ready {
            break onl_limit_reason(&problem);
        }
        let optimality = inf_norm(&g);
        if optimality <= options.optimality_tol * opt_scale {
            break (
                1.,
                format!(
                    "the gradient is small (first-order optimality {} <= {} x {})",
                    short(optimality),
                    short(options.optimality_tol),
                    short(opt_scale)
                ),
            );
        }
        if iterations as f64 >= options.max_iterations {
            break (
                0.,
                format!("it reached MaxIterations = {}", options.max_iterations),
            );
        }
        if problem.evaluations >= problem.max_evaluations {
            break onl_limit_reason(&problem);
        }
        let rhs = DVector::from_iterator(n, g.iter().map(|v| -v));
        let mut p: Vec<f64> = match nalgebra::linalg::Cholesky::new(b.clone()) {
            Some(ch) => ch.solve(&rhs).iter().copied().collect(),
            None => Vec::new(),
        };
        let descent = if p.len() == n { dot(&p, &g) } else { f64::NAN };
        if descent.is_nan() || descent >= 0. {
            b = DMatrix::identity(n, n) * b_scale;
            fresh = true;
            p = g.iter().map(|v| -v).collect();
        }
        let alpha0 = if iterations == 0 || fresh {
            (1. / inf_norm(&p)).min(1.)
        } else {
            1.
        };
        let mut trial = onl_wolfe(engine, &mut problem, &options, &x, f, &g, &p, alpha0, out)?;
        if trial.is_none() && problem.evaluations < problem.max_evaluations {
            b = DMatrix::identity(n, n) * b_scale;
            fresh = true;
            trial = onl_armijo(engine, &mut problem, &options, &x, f, &g, b_scale, out)?;
        }
        let Some(trial) = trial else {
            if problem.evaluations >= problem.max_evaluations {
                break onl_limit_reason(&problem);
            }
            break (
                -3.,
                "the Wolfe search and Armijo backtracking found no further decrease".to_string(),
            );
        };
        let s: Vec<f64> = trial.x.iter().zip(&x).map(|(a, b)| a - b).collect();
        let y: Vec<f64> = trial.g.iter().zip(&g).map(|(a, b)| a - b).collect();
        if let Some(scale) = onl_bfgs_update(&mut b, &mut fresh, &s, &y) {
            b_scale = scale;
        }
        let old_f = f;
        last_step = two_norm(&s);
        x = trial.x;
        f = trial.f;
        g = trial.g;
        iterations += 1;
        if options.display == Display::Iter {
            engine.emit(
                out,
                format!(
                    "{:>5} {:>8} {:>11} {:>13} {:>11}",
                    iterations,
                    problem.evaluations,
                    short(f),
                    short(trial.alpha),
                    short(inf_norm(&g))
                ),
            )?;
        }
        if f < -1e20 || last_step > 1e20 {
            break (-3., "the objective decreased without bound".to_string());
        }
        if !g.iter().all(|v| v.is_finite()) {
            break (-3., "the gradient is no longer finite".to_string());
        }
        if inf_norm(&g) <= options.optimality_tol * opt_scale {
            continue;
        }
        if last_step < options.step_tol * (1. + two_norm(&x)) {
            break (
                2.,
                format!(
                    "the step {} is below StepTolerance relative to x",
                    short(last_step)
                ),
            );
        }
        if (old_f - f).abs() < options.function_tol * old_f.abs().max(1.) {
            break (
                3.,
                format!(
                    "the relative change in f is within FunctionTolerance ({})",
                    short(options.function_tol)
                ),
            );
        }
    };
    let message = format!(
        "fminunc stopped after {iterations} iterations and {} function evaluations because {reason}; exitflag {flag}.",
        problem.evaluations
    );
    onl_final(engine, &options, flag, &message, out)?;
    let mut outputs = vec![problem.x_value(&x)?];
    if count > 1 {
        outputs.push(Value::Matrix(Matrix::scalar(f)));
    }
    if count > 2 {
        outputs.push(Value::Matrix(Matrix::scalar(flag)));
    }
    if count > 3 {
        let report = Report {
            iterations,
            evaluations: problem.evaluations,
            step: last_step,
            optimality: inf_norm(&g),
            algorithm: "quasi-newton (BFGS, strong Wolfe line search)",
            message,
        };
        outputs.push(onl_report(engine, report)?);
    }
    if count > 4 {
        outputs.push(Value::Matrix(Matrix::from_column_slice(n, 1, &g)?));
    }
    if count > 5 {
        outputs.push(Value::Matrix(Matrix::from_column_slice(
            n,
            n,
            b.as_slice(),
        )?));
    }
    Ok(outputs)
}

// ---------------------------------------------------------------- lsqnonlin / lsqcurvefit

fn onl_bound(
    value: Option<&Value>,
    n: usize,
    who: &str,
    what: &str,
    default: f64,
) -> Result<Vec<f64>> {
    match value {
        None => Ok(vec![default; n]),
        Some(v) if is_empty_value(v) => Ok(vec![default; n]),
        Some(v) => {
            let m = onl_real_vector(v, who, what)?;
            if m.len() != n {
                return Err(format!(
                    "{who}: {what} must be empty or have one element per element of x0"
                ));
            }
            Ok(m.data()
                .iter()
                .map(|&b| if b.is_nan() { default } else { b })
                .collect())
        }
    }
}

/// Moré's step: p minimising ||J p + r|| subject to ||D p|| <= delta (approximately,
/// to 10 %), and the parameter used, starting from `par`. `d` is the scaling.
fn onl_lm_step(j: &DMatrix<f64>, r: &[f64], d: &[f64], delta: f64, par: f64) -> (Vec<f64>, f64) {
    let (m, n) = j.shape();
    let solve = |alpha: f64| -> Option<(Vec<f64>, DMatrix<f64>)> {
        let rows = if alpha > 0. { m + n } else { m };
        if rows < n {
            return None;
        }
        let mut a = DMatrix::zeros(rows, n);
        a.view_mut((0, 0), (m, n)).copy_from(j);
        let mut rhs = DVector::zeros(rows);
        for i in 0..m {
            rhs[i] = -r[i];
        }
        if alpha > 0. {
            let s = alpha.sqrt();
            for k in 0..n {
                a[(m + k, k)] = s * d[k];
            }
        }
        let qr = a.qr();
        let rmat = qr.r();
        let diag_max = (0..n).fold(0f64, |acc, k| acc.max(rmat[(k, k)].abs()));
        let tiny = diag_max * f64::EPSILON * (rows.max(n) as f64);
        if (0..n).any(|k| rmat[(k, k)].abs() <= tiny) {
            return None;
        }
        qr.q_tr_mul(&mut rhs);
        let top = DVector::from_iterator(n, rhs.iter().take(n).copied());
        let p = rmat.solve_upper_triangular(&top)?;
        Some((p.iter().copied().collect(), rmat))
    };
    let scaled_norm = |p: &[f64]| -> f64 {
        p.iter()
            .zip(d)
            .map(|(a, b)| (a * b).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    // phi'(alpha) = -||R^-T D^2 p||^2 / ||D p|| (Moré 1978, sec. 5).
    let derivative = |p: &[f64], rmat: &DMatrix<f64>| -> f64 {
        let dp = scaled_norm(p);
        let q = DVector::from_iterator(n, p.iter().zip(d).map(|(a, b)| a * b * b / dp));
        match rmat.tr_solve_upper_triangular(&q) {
            Some(w) => -w.norm_squared() * dp,
            None => f64::NAN,
        }
    };
    let mut lower = 0.;
    if let Some((p, rmat)) = solve(0.) {
        let phi = scaled_norm(&p) - delta;
        if phi <= 0.1 * delta {
            return (p, 0.);
        }
        let dphi = derivative(&p, &rmat);
        if dphi.is_finite() && dphi < 0. {
            lower = (-phi / dphi).max(0.);
        }
    }
    // Upper bound ||D^-1 J^T r|| / delta.
    let jtr: Vec<f64> = (0..n)
        .map(|k| (0..m).map(|i| j[(i, k)] * r[i]).sum::<f64>() / d[k])
        .collect();
    let mut upper = two_norm(&jtr) / delta;
    if upper == 0. || !upper.is_finite() {
        upper = f64::MIN_POSITIVE / delta.min(0.1);
    }
    let mut alpha = par;
    let mut best: Option<(Vec<f64>, f64)> = None;
    for _ in 0..10 {
        if !(alpha > lower && alpha < upper) {
            alpha = (0.001 * upper).max((lower * upper).sqrt());
        }
        let Some((p, rmat)) = solve(alpha) else {
            alpha *= 10.;
            continue;
        };
        let dp = scaled_norm(&p);
        let phi = dp - delta;
        best = Some((p.clone(), alpha));
        if phi.abs() <= 0.1 * delta || (lower == 0. && phi <= 0. && alpha <= f64::MIN_POSITIVE) {
            break;
        }
        let dphi = derivative(&p, &rmat);
        if !(dphi.is_finite() && dphi < 0.) {
            break;
        }
        if phi < 0. {
            upper = alpha;
        }
        lower = lower.max(alpha - phi / dphi);
        alpha -= (phi + delta) / delta * (phi / dphi);
    }
    best.unwrap_or_else(|| {
        // Every system was singular: a short scaled steepest-descent step.
        let g: Vec<f64> = (0..n)
            .map(|k| -(0..m).map(|i| j[(i, k)] * r[i]).sum::<f64>())
            .collect();
        let gn = scaled_norm(&g).max(f64::MIN_POSITIVE);
        (g.iter().map(|v| v * delta / gn).collect(), upper)
    })
}

fn onl_least_squares(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let curve = name == "lsqcurvefit";
    let who: &'static str = if curve { "lsqcurvefit" } else { "lsqnonlin" };
    let (min, max) = if curve { (4, 7) } else { (2, 5) };
    if args.len() < min {
        return Err("Not enough input arguments.".into());
    }
    if args.len() > max {
        return Err("Too many input arguments.".into());
    }
    let handle = onl_handle(engine, who, &args[0])?;
    let x0 = onl_real_vector(&args[1], who, "x0")?;
    let at = if curve { 4 } else { 2 };
    let n = x0.len();
    let lb = onl_bound(args.get(at), n, who, "lb", f64::NEG_INFINITY)?;
    let ub = onl_bound(args.get(at + 1), n, who, "ub", f64::INFINITY)?;
    let options = onl_options(engine, who, args.get(at + 2))?;
    if let Some(a) = &options.algorithm
        && a != "levenberg-marquardt"
    {
        return Err(format!(
            "{who}: Algorithm '{a}' is not supported; this version runs Levenberg-Marquardt"
        ));
    }
    onl_check_size(who, n, 0)?;
    onl_check_finite(who, "x0", x0.data())?;
    let (extra, target) = if curve {
        let ydata = onl_real_vector(&args[3], who, "ydata")?;
        (vec![args[2].clone()], Some(ydata.data().to_vec()))
    } else {
        (Vec::new(), None)
    };
    let mut problem = Problem {
        who,
        handle,
        rows: x0.rows(),
        cols: x0.cols(),
        extra,
        target,
        evaluations: 0.,
        max_evaluations: evaluation_limit(&options, n),
        value_shape: (0, 0),
    };
    if let Some(k) = lb.iter().zip(&ub).position(|(l, u)| l > u) {
        return Err(format!("{who}: lb({}) exceeds ub({})", k + 1, k + 1));
    }
    let bounded = lb.iter().chain(&ub).any(|b| b.is_finite());
    let project = |x: &mut [f64]| {
        for ((v, l), u) in x.iter_mut().zip(&lb).zip(&ub) {
            *v = v.clamp(*l, *u);
        }
    };
    let mut x = x0.data().to_vec();
    project(&mut x);
    let projected = x != x0.data();
    let (mut r, given) = problem
        .eval(engine, &x, options.gradient, out)?
        .unwrap_or_default();
    let m = r.len();
    onl_check_size(who, n, m)?;
    if m == 0 && problem.evaluations > 0. {
        return Err(format!("{who}: the function returned no values"));
    }
    onl_check_finite(who, "the function value", &r)?;
    let fd_bounds = Some((lb.as_slice(), ub.as_slice()));
    let initial_jac = problem.jacobian(engine, &x, &r, given, options.central, fd_bounds, out)?;
    let ready = initial_jac.is_some();
    let mut jac = initial_jac.unwrap_or_else(|| DMatrix::from_element(m, n, f64::NAN));
    if ready {
        onl_check_finite(who, "the Jacobian", jac.as_slice())?;
    }
    let column_norms =
        |j: &DMatrix<f64>| -> Vec<f64> { (0..n).map(|k| j.column(k).norm()).collect() };
    let mut d: Vec<f64> = column_norms(&jac)
        .into_iter()
        .map(|v| if v > 0. { v } else { 1. })
        .collect();
    let dx_norm = |x: &[f64], d: &[f64]| {
        x.iter()
            .zip(d)
            .map(|(a, b)| (a * b).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let mut delta = 100. * dx_norm(&x, &d);
    if delta == 0. {
        delta = 100.;
    }
    let mut par = 0.;
    let mut resnorm = if r.is_empty() { f64::NAN } else { dot(&r, &r) };
    let gradient = |j: &DMatrix<f64>, r: &[f64]| -> Vec<f64> {
        (0..n)
            .map(|k| (0..m).map(|i| j[(i, k)] * r[i]).sum())
            .collect()
    };
    // First-order optimality: ||g||_inf, or for bounds the projected-gradient step.
    let optimality = |x: &[f64], g: &[f64]| -> f64 {
        if g.iter().any(|v| v.is_nan()) {
            return f64::NAN;
        }
        x.iter()
            .zip(g)
            .zip(lb.iter().zip(&ub))
            .fold(0f64, |acc, ((xv, gv), (l, u))| {
                acc.max(((xv - gv).clamp(*l, *u) - xv).abs())
            })
    };
    let mut g = gradient(&jac, &r);
    let opt_scale = optimality(&x, &g).max(1.);
    let mut iterations = 0usize;
    let mut last_step = 0.;
    if options.display == Display::Iter {
        engine.emit(
            out,
            " iter  f-count      resnorm   optimality       lambda".into(),
        )?;
        engine.emit(
            out,
            format!(
                "{:>5} {:>8} {:>12} {:>12} {:>12}",
                0,
                problem.evaluations,
                short(resnorm),
                short(optimality(&x, &g)),
                ""
            ),
        )?;
    }
    let (flag, reason) = loop {
        engine.check_cancel()?;
        if !ready {
            break onl_limit_reason(&problem);
        }
        let first_order = optimality(&x, &g);
        if first_order <= options.optimality_tol * opt_scale {
            break (
                1.,
                format!(
                    "the first-order optimality {} is within OptimalityTolerance x {}",
                    short(first_order),
                    short(opt_scale)
                ),
            );
        }
        if resnorm == 0. {
            break (1., "the residual is zero".to_string());
        }
        if iterations as f64 >= options.max_iterations {
            break (
                0.,
                format!("it reached MaxIterations = {}", options.max_iterations),
            );
        }
        if problem.evaluations >= problem.max_evaluations {
            break onl_limit_reason(&problem);
        }
        // Variables held at a bound by the gradient do not move this iteration.
        let free: Vec<usize> = (0..n)
            .filter(|&k| !((x[k] <= lb[k] && g[k] > 0.) || (x[k] >= ub[k] && g[k] < 0.)))
            .collect();
        if free.is_empty() {
            break (
                1.,
                "every variable is held at a bound by the gradient".to_string(),
            );
        }
        let jf = DMatrix::from_fn(m, free.len(), |i, k| jac[(i, free[k])]);
        let df: Vec<f64> = free.iter().map(|&k| d[k]).collect();
        let (pf, used) = onl_lm_step(&jf, &r, &df, delta, par);
        par = used;
        let mut trial = x.clone();
        for (k, &idx) in free.iter().enumerate() {
            trial[idx] += pf[k];
        }
        if bounded {
            project(&mut trial);
        }
        let s: Vec<f64> = trial.iter().zip(&x).map(|(a, b)| a - b).collect();
        let step_norm = two_norm(&s);
        let scaled_step = dx_norm(&s, &d);
        if step_norm < options.step_tol * (1. + two_norm(&x)) {
            last_step = step_norm;
            break (
                2.,
                format!(
                    "the search direction {} is below StepTolerance relative to x",
                    short(step_norm)
                ),
            );
        }
        let Some((rt, given)) = problem.eval(engine, &trial, options.gradient, out)? else {
            break onl_limit_reason(&problem);
        };
        let rt_norm = dot(&rt, &rt);
        let js: Vec<f64> = (0..m)
            .map(|i| (0..n).map(|k| jac[(i, k)] * s[k]).sum())
            .collect();
        let model: f64 = r.iter().zip(&js).map(|(a, b)| (a + b).powi(2)).sum();
        let predicted = (resnorm - model) / resnorm;
        let actual = if rt_norm.is_finite() {
            1. - rt_norm / resnorm
        } else {
            -1.
        };
        let ratio = if predicted > 0. {
            actual / predicted
        } else {
            -1.
        };
        // Radius update (Moré 1978, sec. 7).
        if ratio <= 0.25 {
            let dirder = -predicted;
            let mut mu = if actual >= 0. {
                0.5
            } else {
                0.5 * dirder / (dirder + 0.5 * actual)
            };
            if mu.is_nan() || mu < 0.1 || 0.1 * rt_norm.sqrt() >= resnorm.sqrt() {
                mu = 0.1;
            }
            mu = mu.min(0.5);
            delta = mu * delta.min(scaled_step / 0.1);
            par /= mu;
        } else if par == 0. || ratio >= 0.75 {
            delta = scaled_step / 0.5;
            par *= 0.5;
        }
        if ratio >= 1e-4 {
            let old = resnorm;
            x = trial;
            r = rt;
            resnorm = rt_norm;
            iterations += 1;
            last_step = step_norm;
            let Some(next_jac) =
                problem.jacobian(engine, &x, &r, given, options.central, fd_bounds, out)?
            else {
                g.fill(f64::NAN);
                break onl_limit_reason(&problem);
            };
            jac = next_jac;
            for (dk, c) in d.iter_mut().zip(column_norms(&jac)) {
                *dk = dk.max(c);
            }
            g = gradient(&jac, &r);
            if options.display == Display::Iter {
                engine.emit(
                    out,
                    format!(
                        "{:>5} {:>8} {:>12} {:>12} {:>12}",
                        iterations,
                        problem.evaluations,
                        short(resnorm),
                        short(optimality(&x, &g)),
                        short(par)
                    ),
                )?;
            }
            if optimality(&x, &g) <= options.optimality_tol * opt_scale {
                continue;
            }
            if step_norm < options.step_tol * (1. + two_norm(&x)) {
                break (
                    2.,
                    format!(
                        "the step {} is below StepTolerance relative to x",
                        short(step_norm)
                    ),
                );
            }
            if (old - resnorm).abs() < options.function_tol * old {
                break (
                    3.,
                    format!(
                        "the relative change in resnorm is within FunctionTolerance ({})",
                        short(options.function_tol)
                    ),
                );
            }
        } else if delta < options.step_tol * (1. + dx_norm(&x, &d)) {
            last_step = step_norm;
            break (
                -3.,
                "the trust region shrank below StepTolerance without a better point".to_string(),
            );
        }
    };
    let projection_note = if projected {
        " x0 was projected into [lb, ub]."
    } else {
        ""
    };
    let message = format!(
        "{who} stopped after {iterations} iterations and {} function evaluations because {reason}; exitflag {flag}.{projection_note}",
        problem.evaluations
    );
    onl_final(engine, &options, flag, &message, out)?;
    let mut outputs = vec![problem.x_value(&x)?];
    if count > 1 {
        outputs.push(Value::Matrix(Matrix::scalar(resnorm)));
    }
    if count > 2 {
        outputs.push(problem.value_matrix(&r)?);
    }
    if count > 3 {
        outputs.push(Value::Matrix(Matrix::scalar(flag)));
    }
    if count > 4 {
        let report = Report {
            iterations,
            evaluations: problem.evaluations,
            step: last_step,
            optimality: optimality(&x, &g),
            algorithm: if bounded {
                "levenberg-marquardt (projected onto the bounds)"
            } else {
                "levenberg-marquardt"
            },
            message,
        };
        outputs.push(onl_report(engine, report)?);
    }
    Ok(outputs)
}

// ---------------------------------------------------------------- fsolve

fn onl_fsolve(
    engine: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    const WHO: &str = "fsolve";
    if args.len() < 2 {
        return Err("Not enough input arguments.".into());
    }
    if args.len() > 3 {
        return Err("Too many input arguments.".into());
    }
    let handle = onl_handle(engine, WHO, &args[0])?;
    let x0 = onl_real_vector(&args[1], WHO, "x0")?;
    let options = onl_options(engine, WHO, args.get(2))?;
    let n = x0.len();
    onl_check_size(WHO, n, 0)?;
    onl_check_finite(WHO, "x0", x0.data())?;
    let mut problem = Problem {
        who: WHO,
        handle,
        rows: x0.rows(),
        cols: x0.cols(),
        extra: Vec::new(),
        target: None,
        evaluations: 0.,
        max_evaluations: evaluation_limit(&options, n),
        value_shape: (0, 0),
    };
    let mut x = x0.data().to_vec();
    let (mut fv, given) = problem
        .eval(engine, &x, options.gradient, out)?
        .unwrap_or_default();
    let m = fv.len();
    onl_check_size(WHO, n, m)?;
    onl_check_finite(WHO, "the function value", &fv)?;
    let initial_jac = problem.jacobian(engine, &x, &fv, given, options.central, None, out)?;
    let ready = initial_jac.is_some();
    let mut jac = initial_jac.unwrap_or_else(|| DMatrix::from_element(m, n, f64::NAN));
    if ready {
        onl_check_finite(WHO, "the Jacobian", jac.as_slice())?;
    }
    let requested_lm = options.algorithm.as_deref() == Some("levenberg-marquardt");
    // A zero evaluation budget leaves the residual dimensions unknown.
    let rectangular = problem.evaluations > 0. && m != n;
    if rectangular && !requested_lm {
        onl_warning(
            engine,
            out,
            "fsolve:NonSquareSystem".into(),
            format!(
                "fsolve: 'trust-region-dogleg' requires a square system. Using 'levenberg-marquardt' for {m} equations in {n} variables."
            ),
        )?;
    }
    let use_lm = rectangular || requested_lm;
    let algorithm = if use_lm {
        "levenberg-marquardt"
    } else {
        "trust-region-dogleg"
    };
    let mut d: Vec<f64> = (0..n)
        .map(|k| {
            let norm = jac.column(k).norm();
            if use_lm && norm > 0. { norm } else { 1. }
        })
        .collect();
    let scaled_norm =
        |v: &[f64], d: &[f64]| two_norm(&v.iter().zip(d).map(|(v, d)| v * d).collect::<Vec<_>>());
    let mut par = 0.;
    let half_sq = |v: &[f64]| 0.5 * dot(v, v);
    let mut merit = half_sq(&fv);
    let mut delta = scaled_norm(&x, &d).max(1.);
    let delta_max = 1e10 * delta;
    let gradient = |j: &DMatrix<f64>, f: &[f64]| -> Vec<f64> {
        (0..n)
            .map(|k| (0..m).map(|i| j[(i, k)] * f[i]).sum())
            .collect()
    };
    let mut g = gradient(&jac, &fv);
    let opt_scale = inf_norm(&g).max(1.);
    // A root: sum(F.^2) within sqrt(FunctionTolerance).
    let solved = |merit: f64| 2. * merit <= options.function_tol.sqrt();
    let mut iterations = 0usize;
    let mut last_step = 0.;
    if options.display == Display::Iter {
        engine.emit(
            out,
            " iter  f-count    sum(F.^2)   optimality       radius".into(),
        )?;
        engine.emit(
            out,
            format!(
                "{:>5} {:>8} {:>12} {:>12} {:>12}",
                0,
                problem.evaluations,
                short(2. * merit),
                short(inf_norm(&g)),
                short(delta)
            ),
        )?;
    }
    let (flag, reason) = loop {
        engine.check_cancel()?;
        if !ready {
            break onl_limit_reason(&problem);
        }
        let optimality = inf_norm(&g);
        if merit == 0. {
            break (1., "F(x) is exactly zero".to_string());
        }
        if optimality <= options.optimality_tol * opt_scale {
            break if solved(merit) {
                (
                    1.,
                    format!(
                        "sum(F.^2) = {} is small and the first-order optimality {} is within OptimalityTolerance x {}",
                        short(2. * merit),
                        short(optimality),
                        short(opt_scale)
                    ),
                )
            } else {
                (
                    -2.,
                    format!(
                        "x is a local minimum of sum(F.^2) = {} that is not a root",
                        short(2. * merit)
                    ),
                )
            };
        }
        if iterations as f64 >= options.max_iterations {
            break (
                0.,
                format!("it reached MaxIterations = {}", options.max_iterations),
            );
        }
        if problem.evaluations >= problem.max_evaluations {
            break onl_limit_reason(&problem);
        }
        let p = if use_lm {
            // Moré's augmented QR solve also handles rectangular/rank-deficient J.
            let (step, used) = onl_lm_step(&jac, &fv, &d, delta, par);
            par = used;
            step
        } else {
            // Gauss-Newton step: minimum-norm least-squares solution of J p = -F.
            let rhs = DVector::from_iterator(m, fv.iter().map(|v| -v));
            let svd = jac.clone().svd(true, true);
            let tol = f64::EPSILON * (m.max(n) as f64) * svd.singular_values.max();
            let newton: Vec<f64> = match svd.solve(&rhs, tol) {
                Ok(p) => p.iter().copied().collect(),
                Err(_) => vec![0.; n],
            };
            // Cauchy point along -g: -(g'g / ||J g||^2) g.
            let jg: Vec<f64> = (0..m)
                .map(|i| (0..n).map(|k| jac[(i, k)] * g[k]).sum())
                .collect();
            let gg = dot(&g, &g);
            let jgjg = dot(&jg, &jg);
            let cauchy: Vec<f64> = if jgjg > 0. {
                g.iter().map(|v| -gg / jgjg * v).collect()
            } else {
                vec![0.; n]
            };
            let nn = two_norm(&newton);
            if nn <= delta && newton.iter().all(|v| v.is_finite()) {
                newton.clone()
            } else if two_norm(&cauchy) >= delta || !newton.iter().all(|v| v.is_finite()) {
                let gn = gg.sqrt();
                g.iter().map(|v| -delta / gn * v).collect()
            } else {
                // ||c + t (b - c)|| = delta with t in [0, 1].
                let diff: Vec<f64> = newton.iter().zip(&cauchy).map(|(b, c)| b - c).collect();
                let a = dot(&diff, &diff);
                let b = 2. * dot(&cauchy, &diff);
                let c = dot(&cauchy, &cauchy) - delta * delta;
                let t = (-b + (b * b - 4. * a * c).max(0.).sqrt()) / (2. * a);
                cauchy.iter().zip(&diff).map(|(c, d)| c + t * d).collect()
            }
        };
        let step_norm = two_norm(&p);
        let radius_step = scaled_norm(&p, &d);
        let jp: Vec<f64> = (0..m)
            .map(|i| (0..n).map(|k| jac[(i, k)] * p[k]).sum())
            .collect();
        let model = 0.5
            * fv.iter()
                .zip(&jp)
                .map(|(a, b)| (a + b).powi(2))
                .sum::<f64>();
        let predicted = merit - model;
        let trial: Vec<f64> = x.iter().zip(&p).map(|(a, b)| a + b).collect();
        let Some((ft, given)) = problem.eval(engine, &trial, options.gradient, out)? else {
            break onl_limit_reason(&problem);
        };
        let trial_merit = half_sq(&ft);
        let ratio = if predicted > 0. && trial_merit.is_finite() {
            (merit - trial_merit) / predicted
        } else {
            -1.
        };
        // Nocedal & Wright Algorithm 4.1.
        if ratio < 0.25 {
            delta = 0.25 * radius_step;
            par *= 4.;
        } else if ratio > 0.75 && radius_step >= 0.99 * delta {
            delta = (2. * delta).min(delta_max);
            par *= 0.5;
        }
        if ratio > 1e-4 {
            x = trial;
            fv = ft;
            merit = trial_merit;
            iterations += 1;
            last_step = step_norm;
            let Some(next_jac) =
                problem.jacobian(engine, &x, &fv, given, options.central, None, out)?
            else {
                jac.fill(f64::NAN);
                g.fill(f64::NAN);
                break onl_limit_reason(&problem);
            };
            jac = next_jac;
            g = gradient(&jac, &fv);
            if use_lm {
                for (k, dk) in d.iter_mut().enumerate() {
                    *dk = dk.max(jac.column(k).norm());
                }
            }
            if options.display == Display::Iter {
                engine.emit(
                    out,
                    format!(
                        "{:>5} {:>8} {:>12} {:>12} {:>12}",
                        iterations,
                        problem.evaluations,
                        short(2. * merit),
                        short(inf_norm(&g)),
                        short(delta)
                    ),
                )?;
            }
            if merit == 0. || inf_norm(&g) <= options.optimality_tol * opt_scale {
                continue;
            }
            if step_norm < options.step_tol * (1. + two_norm(&x)) {
                break if solved(merit) {
                    (
                        2.,
                        format!(
                            "sum(F.^2) = {} is small and the step is below StepTolerance",
                            short(2. * merit)
                        ),
                    )
                } else {
                    (
                        -2.,
                        format!(
                            "the step is below StepTolerance but sum(F.^2) = {} is not small",
                            short(2. * merit)
                        ),
                    )
                };
            }
        } else if delta < options.step_tol * (1. + scaled_norm(&x, &d)) {
            last_step = step_norm;
            break (
                -3.,
                "the trust region shrank below StepTolerance without a better point".to_string(),
            );
        }
    };
    let message = format!(
        "fsolve stopped after {iterations} iterations and {} function evaluations because {reason}; exitflag {flag}.",
        problem.evaluations
    );
    onl_final(engine, &options, flag, &message, out)?;
    let mut outputs = vec![problem.x_value(&x)?];
    if count > 1 {
        outputs.push(problem.value_matrix(&fv)?);
    }
    if count > 2 {
        outputs.push(Value::Matrix(Matrix::scalar(flag)));
    }
    if count > 3 {
        let report = Report {
            iterations,
            evaluations: problem.evaluations,
            step: last_step,
            optimality: inf_norm(&g),
            algorithm,
            message,
        };
        outputs.push(onl_report(engine, report)?);
    }
    if count > 4 {
        outputs.push(Value::Matrix(Matrix::from_column_slice(
            m,
            n,
            jac.as_slice(),
        )?));
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bfgs_skips_weak_curvature_before_scaling_the_hessian() {
        // s'y is positive but almost orthogonal: below the amendment's 1e-10
        // relative curvature threshold, well above machine epsilon.
        for initial in [true, false] {
            let mut b = DMatrix::identity(2, 2) * 3.;
            let original = b.clone();
            let mut fresh = initial;
            for y in [[5e-11, 1.], [1e-10, 1.], [-1., 0.], [0., 0.]] {
                assert!(onl_bfgs_update(&mut b, &mut fresh, &[1., 0.], &y).is_none());
                assert_eq!(b, original);
                assert_eq!(fresh, initial);
            }
            // A well-conditioned secant pair must still update B and satisfy B s=y.
            assert!(onl_bfgs_update(&mut b, &mut fresh, &[1., 0.], &[2., 1.]).is_some());
            assert!((b[(0, 0)] - 2.).abs() < 1e-14);
            assert!((b[(1, 0)] - 1.).abs() < 1e-14);
            assert!(!fresh);
        }
    }
}
