//! Dense constrained optimization, derived from the KKT equations in
//! Mehrotra (1992), SIAM J. Optimization 2, 575–601; Wright, Primal-Dual
//! Interior-Point Methods (1997), chs. 9–10; Xu, Hung & Ye (1996),
//! homogeneous self-dual path-following; Nocedal & Wright, Numerical
//! Optimization (2006), secs. 16.6 and 18.3–18.4. No external solver runtime.
//! Inequalities use Gx-h <= 0; stationarity is Hx+f+G'z+E'y=0.
use super::optim_nl::{self as onl, Options};
use super::{Engine, Execution, Value};
use crate::{Array, FunctionHandle, Matrix, Result, StructArray};
use nalgebra::{DMatrix, DVector};
use std::sync::Arc;

const NAMES: &[&str] = &[
    "linprog",
    "quadprog",
    "lsqlin",
    "fmincon",
    "intlinprog",
    "fminimax",
    "fgoalattain",
];
// Dense factorizations: bound both the KKT matrix and constraint storage.
const MAX_ELEMENTS: usize = 4_000_000;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
/// Solvers that run the SQP path (fminimax and fgoalattain through an epigraph).
fn ocn_sqp_family(who: &str) -> bool {
    matches!(who, "fmincon" | "fminimax" | "fgoalattain")
}
pub(super) fn ocn_call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if name == "optimoptions" {
        return ocn_options_builder(e, args, out);
    }
    if !handles(name) {
        return Ok(None);
    }
    Ok(Some(ocn_outputs(e, name, args, 1, out)?.remove(0)))
}

#[derive(Clone)]
struct ConOptions {
    base: Options,
    feasibility: f64,
    constraint_gradient: bool,
    nodes: usize,
    integer_tol: f64,
    relative_gap: f64,
    absolute_gap: f64,
}
fn ocn_extra(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "constrainttolerance" | "tolcon" => Some("ConstraintTolerance"),
        "specifyconstraintgradient" | "gradconstr" => Some("SpecifyConstraintGradient"),
        "maxnodes" => Some("MaxNodes"),
        "integertolerance" => Some("IntegerTolerance"),
        "relativegaptolerance" => Some("RelativeGapTolerance"),
        "absolutegaptolerance" => Some("AbsoluteGapTolerance"),
        _ => None,
    }
}
fn ocn_check_extra(who: &str, key: &str, value: &Value) -> Result<()> {
    let supported = match key {
        "ConstraintTolerance" => handles(who),
        "SpecifyConstraintGradient" => who == "fmincon",
        _ => who == "intlinprog",
    };
    if !supported && !onl::is_empty_value(value) {
        return Err(format!("{who}: option '{key}' is not implemented"));
    }
    Ok(())
}
fn ocn_extra_set(e: &Engine, o: &mut ConOptions, name: &str, v: &Value) -> Result<()> {
    if onl::is_empty_value(v) {
        return Ok(());
    }
    match name {
        "ConstraintTolerance" => {
            o.feasibility = onl::onl_scalar(v)
                .filter(|x| x.is_finite() && *x > 0.)
                .ok_or("ConstraintTolerance must be finite and positive")?;
        }
        "IntegerTolerance" | "RelativeGapTolerance" | "AbsoluteGapTolerance" => {
            let x = onl::onl_scalar(v)
                .filter(|x| x.is_finite() && *x >= 0.)
                .ok_or_else(|| format!("{name} must be finite and nonnegative"))?;
            match name {
                "IntegerTolerance" if x > 0. && x < 0.5 => o.integer_tol = x,
                "IntegerTolerance" => {
                    return Err("IntegerTolerance must be between 0 and 0.5".into());
                }
                "RelativeGapTolerance" => o.relative_gap = x,
                _ => o.absolute_gap = x,
            }
        }
        "SpecifyConstraintGradient" => {
            let mut base = Options::default();
            onl::onl_set_option(e, &mut base, "fmincon", "SpecifyObjectiveGradient", v).map_err(
                |err| err.replace("SpecifyObjectiveGradient", "SpecifyConstraintGradient"),
            )?;
            o.constraint_gradient = base.gradient;
        }
        "MaxNodes" => {
            let x = onl::onl_scalar(v)
                .filter(|x| x.is_finite() && *x >= 0. && x.fract() == 0. && *x <= 1_000_000.)
                .ok_or("MaxNodes must be an integer between 0 and 1000000")?;
            o.nodes = x as usize;
        }
        _ => unreachable!(),
    }
    Ok(())
}
/// Documented R2024b `fmincon` options (`https://www.mathworks.com/help/optim/ug/fmincon.html`)
/// that the engine validates and accepts for `fmincon`, but does not implement in its SQP solver.
fn fmincon_unimplemented_canonical(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "checkgradients" => Some("CheckGradients"),
        "diagnostics" => Some("Diagnostics"),
        "diffmaxchange" => Some("DiffMaxChange"),
        "diffminchange" => Some("DiffMinChange"),
        "enablefeasibilitymode" => Some("EnableFeasibilityMode"),
        "finitedifferencestepsize" => Some("FiniteDifferenceStepSize"),
        "funvalcheck" => Some("FunValCheck"),
        "hessianapproximation" => Some("HessianApproximation"),
        "hessianfcn" => Some("HessianFcn"),
        "hessianmultiplyfcn" => Some("HessianMultiplyFcn"),
        "honorbounds" => Some("HonorBounds"),
        "initbarrierparam" => Some("InitBarrierParam"),
        "inittrustregionradius" => Some("InitTrustRegionRadius"),
        "maxprojcgiter" => Some("MaxProjCGIter"),
        "maxsqpiter" => Some("MaxSQPIter"),
        "objectivelimit" => Some("ObjectiveLimit"),
        "outputfcn" => Some("OutputFcn"),
        "plotfcn" | "plotfcns" => Some("PlotFcn"),
        "rellinesrchbnd" => Some("RelLineSrchBnd"),
        "rellinesrchbndduration" => Some("RelLineSrchBndDuration"),
        "scaleproblem" => Some("ScaleProblem"),
        "subproblemalgorithm" => Some("SubproblemAlgorithm"),
        "tolconsqp" => Some("TolConSQP"),
        "typicalx" => Some("TypicalX"),
        "useparallel" => Some("UseParallel"),
        _ => None,
    }
}

/// Documented R2024b `linprog` options
/// (`https://www.mathworks.com/help/releases/R2024b/optim/ug/linprog.html`)
/// that this engine validates and accepts, but does not apply in `ocn_qp`.
fn linprog_ineffective_canonical(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "diagnostics" => Some("Diagnostics"),
        "maxtime" => Some("MaxTime"),
        "presolve" => Some("Presolve"),
        "preprocess" => Some("Preprocess"),
        _ => None,
    }
}

/// Documented R2024b `quadprog` options
/// (`https://www.mathworks.com/help/releases/R2024b/optim/ug/quadprog.html`)
/// that this engine validates and accepts, but does not apply in `ocn_qp`.
fn quadprog_ineffective_canonical(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "diagnostics" => Some("Diagnostics"),
        "functiontolerance" => Some("FunctionTolerance"),
        "hessianmultiplyfcn" | "hessmult" => Some("HessianMultiplyFcn"),
        "linearsolver" => Some("LinearSolver"),
        "maxpcgiter" => Some("MaxPCGIter"),
        "objectivelimit" => Some("ObjectiveLimit"),
        "precondbandwidth" => Some("PrecondBandWidth"),
        "steptolerance" | "tolx" => Some("StepTolerance"),
        "subproblemalgorithm" => Some("SubproblemAlgorithm"),
        "tolpcg" => Some("TolPCG"),
        "typicalx" => Some("TypicalX"),
        _ => None,
    }
}

fn lp_qp_ineffective_canonical(who: &str, name: &str) -> Option<&'static str> {
    match who {
        "linprog" => linprog_ineffective_canonical(name),
        "quadprog" => quadprog_ineffective_canonical(name),
        _ => None,
    }
}

fn lp_qp_rejects_shared(who: &str, key: &str) -> bool {
    match who {
        "linprog" => matches!(
            key,
            "MaxFunctionEvaluations"
                | "StepTolerance"
                | "FunctionTolerance"
                | "SpecifyObjectiveGradient"
                | "FiniteDifferenceType"
        ),
        "quadprog" => matches!(
            key,
            "MaxFunctionEvaluations" | "SpecifyObjectiveGradient" | "FiniteDifferenceType"
        ),
        _ => false,
    }
}

/// `Ok(true)` for documented algorithms not implemented by `ocn_qp` (warn once at construction);
/// `Ok(false)` for the effective algorithm or preserved compatibility aliases.
fn lp_qp_algorithm_warns(who: &str, text: &str) -> Result<bool> {
    match who {
        "linprog" => match text {
            "interior-point" | "interior-point-legacy" | "dual-simplex" => Ok(false),
            "dual-simplex-highs" | "dual-simplex-legacy" => Ok(true),
            _ => Err(format!(
                "optimoptions: linprog Algorithm '{text}' is not supported; use 'dual-simplex-highs', 'dual-simplex-legacy', 'interior-point' or 'interior-point-legacy'"
            )),
        },
        "quadprog" => match text {
            "interior-point"
            | "interior-point-legacy"
            | "dual-simplex"
            | "sqp"
            | "sqp-legacy"
            | "active-set" => Ok(false),
            "interior-point-convex" | "trust-region-reflective" => Ok(true),
            _ => Err(format!(
                "optimoptions: quadprog Algorithm '{text}' is not supported; use 'interior-point-convex', 'trust-region-reflective' or 'active-set'"
            )),
        },
        _ => Err(format!("optimoptions: Algorithm '{text}' is not supported")),
    }
}

fn ocn_validate_lp_qp_ineffective(e: &Engine, who: &str, key: &str, value: &Value) -> Result<()> {
    if onl::is_empty_value(value) {
        return Ok(());
    }
    match key {
        "Diagnostics" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "on" | "off") {
                return Err(format!("{who}: Diagnostics must be 'on' or 'off'"));
            }
        }
        "MaxTime" => {
            if onl::onl_scalar(value)
                .filter(|x| *x >= 0. && (x.is_finite() || *x == f64::INFINITY))
                .is_none()
            {
                return Err(format!(
                    "{who}: MaxTime must be a nonnegative scalar or Inf"
                ));
            }
        }
        "Presolve" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "auto" | "off" | "on") {
                return Err(format!("{who}: Presolve must be 'auto', 'off' or 'on'"));
            }
        }
        "Preprocess" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "basic" | "none") {
                return Err(format!("{who}: Preprocess must be 'basic' or 'none'"));
            }
        }
        "StepTolerance" | "FunctionTolerance" | "TolPCG" => {
            if onl::onl_scalar(value)
                .filter(|x| *x > 0. && x.is_finite())
                .is_none()
            {
                return Err(format!("{who}: {key} must be a finite positive scalar"));
            }
        }
        "HessianMultiplyFcn" => {
            let valid_handle = matches!(value, Value::Container(Array::Handle(_)));
            let valid_name = onl::onl_text(e, value).is_some_and(|s| !s.trim().is_empty());
            if !valid_handle && !valid_name {
                return Err(format!(
                    "{who}: HessianMultiplyFcn must be a function handle, function name or []"
                ));
            }
        }
        "LinearSolver" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "auto" | "sparse" | "dense") {
                return Err(format!(
                    "{who}: LinearSolver must be 'auto', 'sparse' or 'dense'"
                ));
            }
        }
        "MaxPCGIter" => {
            if onl::onl_scalar(value)
                .filter(|x| *x > 0. && (x.is_finite() || *x == f64::INFINITY))
                .is_none()
            {
                return Err(format!(
                    "{who}: MaxPCGIter must be a positive scalar or Inf"
                ));
            }
        }
        "ObjectiveLimit" => {
            if onl::onl_scalar(value).filter(|x| !x.is_nan()).is_none() {
                return Err(format!("{who}: ObjectiveLimit must be a real scalar"));
            }
        }
        "PrecondBandWidth" => {
            if onl::onl_scalar(value)
                .filter(|x| *x >= 0. && (x.fract() == 0. || *x == f64::INFINITY))
                .is_none()
            {
                return Err(format!(
                    "{who}: PrecondBandWidth must be a nonnegative integer or Inf"
                ));
            }
        }
        "SubproblemAlgorithm" => {
            let text = onl::onl_text(e, value)
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

fn ocn_strict_bool(e: &Engine, who: &str, key: &str, value: &Value) -> Result<()> {
    match (onl::onl_scalar(value), onl::onl_text(e, value)) {
        (Some(v), _) if v == 0. || v == 1. => Ok(()),
        (_, Some(t)) if t.eq_ignore_ascii_case("on") || t.eq_ignore_ascii_case("off") => Ok(()),
        _ => Err(format!(
            "{who}: {key} must be true, false, 0, 1, 'on' or 'off', and finite"
        )),
    }
}

fn ocn_positive_finite_vector(value: &Value) -> bool {
    matches!(value, Value::Matrix(m) if !m.is_nd()
                && !m.is_empty()
                && (m.rows() == 1 || m.cols() == 1)
                && m.data().iter().all(|v| v.is_finite() && *v > 0.))
}

fn ocn_validate_fmincon_unimplemented(
    e: &Engine,
    who: &str,
    key: &str,
    value: &Value,
) -> Result<()> {
    if onl::is_empty_value(value) {
        return Ok(());
    }
    match key {
        "CheckGradients" | "EnableFeasibilityMode" | "HonorBounds" | "UseParallel" => {
            ocn_strict_bool(e, who, key, value)?;
        }
        "Diagnostics" | "FunValCheck" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "on" | "off") {
                return Err(format!("{who}: {key} must be 'on' or 'off'"));
            }
        }
        "DiffMaxChange" => {
            if onl::onl_scalar(value)
                .filter(|x| *x > 0. && !x.is_nan())
                .is_none()
            {
                return Err(format!("{who}: DiffMaxChange must be a positive scalar"));
            }
        }
        "DiffMinChange" => {
            if onl::onl_scalar(value)
                .filter(|x| *x >= 0. && x.is_finite())
                .is_none()
            {
                return Err(format!(
                    "{who}: DiffMinChange must be a finite nonnegative scalar"
                ));
            }
        }
        "FiniteDifferenceStepSize" => {
            if !ocn_positive_finite_vector(value) {
                return Err(format!(
                    "{who}: FiniteDifferenceStepSize must be a finite positive scalar or vector"
                ));
            }
        }
        "HessianApproximation" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(
                text.as_str(),
                "bfgs" | "lbfgs" | "fin-diff-grads" | "on" | "off"
            ) {
                return Err(format!(
                    "{who}: HessianApproximation must be 'bfgs', 'lbfgs', 'fin-diff-grads', 'on' or 'off'"
                ));
            }
        }
        "HessianFcn" | "HessianMultiplyFcn" => {
            let valid_handle = matches!(value, Value::Container(Array::Handle(_)));
            let valid_name = onl::onl_text(e, value).is_some_and(|s| !s.trim().is_empty());
            if !valid_handle && !valid_name {
                return Err(format!(
                    "{who}: {key} must be a function handle, function name or []"
                ));
            }
        }
        "InitBarrierParam" | "InitTrustRegionRadius" | "TolConSQP" => {
            if onl::onl_scalar(value)
                .filter(|x| *x > 0. && x.is_finite())
                .is_none()
            {
                return Err(format!("{who}: {key} must be a finite positive scalar"));
            }
        }
        "MaxProjCGIter" => {
            if onl::onl_scalar(value)
                .filter(|x| *x >= 0. && (x.fract() == 0. || *x == f64::INFINITY))
                .is_none()
            {
                return Err(format!(
                    "{who}: MaxProjCGIter must be a nonnegative integer or Inf"
                ));
            }
        }
        "MaxSQPIter" | "RelLineSrchBndDuration" => {
            if onl::onl_scalar(value)
                .filter(|x| *x >= 1. && (x.fract() == 0. || *x == f64::INFINITY))
                .is_none()
            {
                return Err(format!("{who}: {key} must be a positive integer or Inf"));
            }
        }
        "ObjectiveLimit" => {
            if onl::onl_scalar(value).filter(|x| !x.is_nan()).is_none() {
                return Err(format!("{who}: ObjectiveLimit must be a real scalar"));
            }
        }
        "OutputFcn" | "PlotFcn" => {
            e.polyopt_parse_fcns(who, key, value, &mut Vec::new())?;
        }
        "RelLineSrchBnd" => {
            if onl::onl_scalar(value)
                .filter(|x| *x >= 0. && !x.is_nan())
                .is_none()
            {
                return Err(format!(
                    "{who}: RelLineSrchBnd must be a nonnegative scalar or []"
                ));
            }
        }
        "ScaleProblem" => {
            let valid_text = onl::onl_text(e, value).is_some_and(|t| {
                matches!(t.to_ascii_lowercase().as_str(), "obj-and-constr" | "none")
            });
            if !valid_text && ocn_strict_bool(e, who, key, value).is_err() {
                return Err(format!(
                    "{who}: ScaleProblem must be true, false, 'obj-and-constr' or 'none'"
                ));
            }
        }
        "SubproblemAlgorithm" => {
            let text = onl::onl_text(e, value)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(text.as_str(), "ldl-factorization" | "cg" | "factorization") {
                return Err(format!(
                    "{who}: SubproblemAlgorithm must be 'ldl-factorization', 'cg' or 'factorization'"
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

/// Record the warning for the latest request of `key`. A later pair replaces the detail.
fn ocn_pending_set(
    pending: &mut Vec<(&'static str, Option<String>)>,
    key: &'static str,
    detail: Option<String>,
) {
    if let Some(slot) = pending.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = detail;
    } else {
        pending.push((key, detail));
    }
}

/// Drop a warning when a later pair clears that request or replaces it with an effective one.
fn ocn_pending_clear(pending: &mut Vec<(&'static str, Option<String>)>, key: &'static str) {
    pending.retain(|(k, _)| *k != key);
}

fn ocn_warn_option(
    e: &mut Engine,
    out: &mut Execution,
    solver: &str,
    key: &'static str,
    detail: Option<&str>,
) -> Result<()> {
    let (id, message) = if solver == "fmincon" && key == "Algorithm" {
        (
            format!("optimoptions:fmincon:{key}"),
            format!(
                "optimoptions: fmincon option 'Algorithm' value '{}' is not implemented; falling back to 'sqp'.",
                detail.unwrap_or("interior-point")
            ),
        )
    } else if solver == "fmincon" && key == "FunctionTolerance" {
        (
            format!("optimoptions:fmincon:{key}"),
            "optimoptions: fmincon option 'FunctionTolerance' does not stop this SQP solver and is ignored. SQP stops on OptimalityTolerance.".to_string(),
        )
    } else if key == "Algorithm" {
        (
            format!("optimoptions:{solver}:{key}"),
            format!(
                "optimoptions: {solver} option 'Algorithm' value '{}' is not implemented; falling back to 'primal-dual interior-point'.",
                detail.unwrap_or("interior-point")
            ),
        )
    } else {
        (
            format!("optimoptions:{solver}:{key}"),
            format!(
                "optimoptions: {solver} option '{key}' is not implemented in this version and will be ignored."
            ),
        )
    };
    let args = [
        Value::Text(id),
        Value::Text("%s".into()),
        Value::Text(message),
    ];
    e.lang_utils_call("warning", &args, out)?;
    Ok(())
}

fn ocn_options(e: &Engine, who: &str, v: Option<&Value>) -> Result<ConOptions> {
    let mut o = ConOptions {
        base: Options::default(),
        feasibility: 1e-6,
        constraint_gradient: false,
        nodes: 10_000,
        integer_tol: 1e-6,
        relative_gap: 0.,
        absolute_gap: 0.,
    };
    if ocn_sqp_family(who) {
        o.base.max_iterations = 400.;
        o.base.max_evaluations = Some(3000.);
        o.base.optimality_tol = 1e-6;
        o.base.step_tol = 1e-10;
    }
    if let Some(v) = v {
        if onl::is_empty_value(v) {
            return Ok(o);
        }
        let Value::Container(Array::Struct(s)) = v else {
            return Err(format!("{who}: options must be a scalar structure"));
        };
        if s.len() != 1 {
            return Err(format!("{who}: options must be a scalar structure"));
        }
        let lp_qp = matches!(who, "linprog" | "quadprog");
        let has_field = |target: &str| {
            s.field_names()
                .iter()
                .any(|n| n.eq_ignore_ascii_case(target))
        };
        let canonical_field_present = |target: &str, raw_name: &str| {
            lp_qp && !raw_name.eq_ignore_ascii_case(target) && has_field(target)
        };
        for (i, name) in s.field_names().iter().enumerate() {
            let v = Value::from(s.get(i, 0)?.as_ref().clone());
            if onl::is_empty_value(&v) {
                continue;
            }
            if let Some(k) = ocn_extra(name) {
                ocn_check_extra(who, k, &v)?;
                if !canonical_field_present(k, name) {
                    ocn_extra_set(e, &mut o, k, &v)?;
                }
            } else if let Some(k) = lp_qp_ineffective_canonical(who, name) {
                ocn_validate_lp_qp_ineffective(e, who, k, &v)?;
            } else if let Some(k) = onl::canonical(name) {
                if who == "linprog" && k == "FunctionTolerance" && has_field("OptimalityTolerance")
                {
                    // Untyped TolFun updates write both FunctionTolerance and OptimalityTolerance.
                    onl::onl_set_option(e, &mut o.base, who, k, &v)?;
                } else if lp_qp_rejects_shared(who, k) {
                    return Err(format!("{who}: option '{name}' is not implemented"));
                } else if lp_qp && k == "TolFun" && has_field("OptimalityTolerance") {
                    // An empty OptimalityTolerance resets it without resurrecting TolFun.
                    onl::onl_set_option(e, &mut o.base, who, "FunctionTolerance", &v)?;
                } else if !canonical_field_present(k, name) {
                    onl::onl_set_option(e, &mut o.base, who, k, &v)?;
                }
            } else if who == "fmincon"
                && let Some(k) = fmincon_unimplemented_canonical(name)
            {
                ocn_validate_fmincon_unimplemented(e, who, k, &v)?;
            } else {
                return Err(format!("{who}: option '{name}' is not implemented"));
            }
        }
    }
    Ok(o)
}
/// Keep optim_nl behavior intact: route only constrained solver names / options here.
pub(super) fn ocn_options_builder(
    e: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    if let Some(value) = super::optim_global::og_options_builder(e, args)? {
        return Ok(Some(value));
    }
    let name = match first {
        Value::Container(Array::Handle(h)) => Some(h.func2str().to_string()),
        _ => onl::onl_text(e, first),
    };
    // Named residual constructors own validation, including empty constrained defaults.
    if matches!(name.as_deref(), Some("lsqnonlin" | "fsolve")) {
        return Ok(None);
    }
    let updating = matches!(first, Value::Container(Array::Struct(s)) if s.len() == 1);
    // A shared empty default says nothing about the solver of an untagged struct.
    let needs_constrained = |n: &str, empty: bool| {
        ocn_extra(n).is_some() && !(updating && empty && onl::canonical(n).is_some())
    };
    let extra_pairs = args[1..].chunks(2).any(|p| {
        onl::onl_text(e, &p[0])
            .is_some_and(|n| needs_constrained(&n, p.get(1).is_some_and(onl::is_empty_value)))
    });
    let extra_struct = matches!(first, Value::Container(Array::Struct(s)) if s.field_names().iter().enumerate()
        .any(|(i, n)| needs_constrained(n, s.get(i, 0).is_ok_and(|v| v.is_empty()))));
    if !name.as_deref().is_some_and(handles) && !extra_pairs && !extra_struct {
        return Ok(None);
    }
    if !(args.len() - 1).is_multiple_of(2) {
        return Err("Arguments must occur in name-value pairs.".into());
    }
    let mut fields: Vec<(String, Arc<Array>)> = Vec::new();
    if let Value::Container(Array::Struct(s)) = first {
        if s.len() != 1 {
            return Err("optimoptions: expected a scalar structure".into());
        }
        for (i, n) in s.field_names().iter().enumerate() {
            fields.push((n.clone(), s.get(i, 0)?.clone()));
        }
    } else {
        let solver = name
            .as_deref()
            .ok_or("optimoptions: expected a solver name")?;
        // Let the shared builder validate names outside this module.
        if !handles(solver) {
            onl::onl_optimoptions(e, std::slice::from_ref(first))?;
        }
    }
    let named_fmincon = name.as_deref() == Some("fmincon");
    let named_lp_qp = match name.as_deref() {
        Some("linprog") => Some("linprog"),
        Some("quadprog") => Some("quadprog"),
        _ => None,
    };
    let mut check = ConOptions {
        base: Options::default(),
        feasibility: 1e-6,
        constraint_gradient: false,
        nodes: 10_000,
        integer_tol: 1e-6,
        relative_gap: 0.,
        absolute_gap: 0.,
    };
    // Warnings describe the values that survive every pair. Validation still
    // rejects a bad pair immediately, and nothing is emitted unless construction succeeds.
    let mut pending_warnings: Vec<(&'static str, Option<String>)> = Vec::new();
    for p in args[1..].chunks(2) {
        let n = onl::onl_text(e, &p[0]).ok_or("optimoptions: option names must be text")?;
        let existing_callback = updating
            && n.eq_ignore_ascii_case("OutputFcn")
            && fields
                .iter()
                .any(|(field, _)| field.eq_ignore_ascii_case("OutputFcn"));
        let fmincon_unimpl = named_fmincon
            .then(|| fmincon_unimplemented_canonical(&n))
            .flatten();
        let lp_qp_unimpl = named_lp_qp.and_then(|who| lp_qp_ineffective_canonical(who, &n));
        let k = ocn_extra(&n)
            .or(lp_qp_unimpl)
            .or_else(|| onl::canonical(&n))
            .or(fmincon_unimpl)
            .or(existing_callback.then_some("OutputFcn"))
            .ok_or_else(|| format!("optimoptions: unknown option '{n}'"))?;
        if ocn_extra(k).is_some() {
            if let Some(who) = name.as_deref() {
                ocn_check_extra(who, k, &p[1])?;
            }
            ocn_extra_set(e, &mut check, k, &p[1])?;
        } else if existing_callback {
            e.polyopt_parse_fcns("optimoptions", k, &p[1], &mut Vec::new())?;
        } else if fmincon_unimpl.is_some() || lp_qp_unimpl.is_some() {
            if onl::is_empty_value(&p[1]) {
                ocn_pending_clear(&mut pending_warnings, k);
            } else if fmincon_unimpl.is_some() {
                ocn_validate_fmincon_unimplemented(e, "optimoptions", k, &p[1])?;
                ocn_pending_set(&mut pending_warnings, k, None);
            } else {
                ocn_validate_lp_qp_ineffective(e, named_lp_qp.unwrap_or("optimoptions"), k, &p[1])?;
                ocn_pending_set(&mut pending_warnings, k, None);
            }
        } else if named_fmincon && k == "Algorithm" {
            if onl::is_empty_value(&p[1]) {
                ocn_pending_clear(&mut pending_warnings, "Algorithm");
            } else {
                let text = onl::onl_text(e, &p[1])
                    .ok_or("optimoptions: Algorithm must be text")?
                    .to_ascii_lowercase();
                match text.as_str() {
                    "sqp" => ocn_pending_clear(&mut pending_warnings, "Algorithm"),
                    "interior-point" | "trust-region-reflective" | "sqp-legacy" | "active-set" => {
                        ocn_pending_set(&mut pending_warnings, "Algorithm", Some(text));
                    }
                    _ => {
                        return Err(format!(
                            "optimoptions: fmincon Algorithm '{text}' is not supported; use 'interior-point', 'trust-region-reflective', 'sqp', 'sqp-legacy' or 'active-set'"
                        ));
                    }
                }
            }
        } else if let Some(who) = named_lp_qp.filter(|_| k == "Algorithm") {
            if onl::is_empty_value(&p[1]) {
                ocn_pending_clear(&mut pending_warnings, "Algorithm");
            } else {
                let text = onl::onl_text(e, &p[1])
                    .ok_or("optimoptions: Algorithm must be text")?
                    .to_ascii_lowercase();
                if lp_qp_algorithm_warns(who, &text)? {
                    ocn_pending_set(&mut pending_warnings, "Algorithm", Some(text));
                } else {
                    ocn_pending_clear(&mut pending_warnings, "Algorithm");
                }
            }
        } else if named_lp_qp.is_some_and(|who| lp_qp_rejects_shared(who, k)) {
            return Err(format!("optimoptions: unknown option '{n}'"));
        } else {
            onl::onl_set_option(e, &mut check.base, "optimoptions", k, &p[1])?;
            // SQP stops on optimality_tol. A nonempty FunctionTolerance is stored
            // and checked here, then ignored by the solver. TolFun also writes
            // OptimalityTolerance, so it replaces that ineffective request.
            if named_fmincon && k == "FunctionTolerance" {
                if onl::is_empty_value(&p[1]) {
                    ocn_pending_clear(&mut pending_warnings, "FunctionTolerance");
                } else {
                    ocn_pending_set(&mut pending_warnings, "FunctionTolerance", None);
                }
            } else if named_fmincon && k == "TolFun" {
                ocn_pending_clear(&mut pending_warnings, "FunctionTolerance");
            }
        }
        let keys = if k == "TolFun" && named_lp_qp == Some("linprog") {
            vec!["OptimalityTolerance"]
        } else if k == "TolFun" {
            vec!["FunctionTolerance", "OptimalityTolerance"]
        } else {
            vec![k]
        };
        for key in keys {
            let val = Arc::new(p[1].clone().array(&e.cancel)?);
            if let Some(f) = fields.iter_mut().find(|f| {
                f.0.eq_ignore_ascii_case(key)
                    || ocn_extra(&f.0) == Some(key)
                    || onl::canonical(&f.0) == Some(key)
                    || (named_fmincon && fmincon_unimplemented_canonical(&f.0) == Some(key))
                    || named_lp_qp
                        .is_some_and(|who| lp_qp_ineffective_canonical(who, &f.0) == Some(key))
            }) {
                *f = (key.into(), val);
            } else {
                fields.push((key.into(), val));
            }
        }
    }
    let (names, slots) = fields.into_iter().unzip();
    let result = Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?));
    let warn_solver = named_lp_qp.unwrap_or("fmincon");
    for (key, detail) in pending_warnings {
        ocn_warn_option(e, out, warn_solver, key, detail.as_deref())?;
    }
    Ok(Some(result))
}

fn ocn_matrix(v: &Value, who: &str, what: &str) -> Result<DMatrix<f64>> {
    let m = onl::onl_real_vector(v, who, what)?;
    onl::onl_check_finite(who, what, m.data())?;
    if m.len() > MAX_ELEMENTS {
        return Err(format!("{who}: {what} is too large"));
    }
    Ok(DMatrix::from_column_slice(m.rows(), m.cols(), m.data()))
}
fn ocn_vec(v: &Value, who: &str, what: &str) -> Result<DVector<f64>> {
    let m = ocn_matrix(v, who, what)?;
    if !m.is_empty() && m.nrows() != 1 && m.ncols() != 1 {
        return Err(format!("{who}: {what} must be a vector"));
    }
    Ok(DVector::from_column_slice(m.as_slice()))
}
fn ocn_bound(v: Option<&Value>, n: usize, lower: bool, who: &str) -> Result<Vec<f64>> {
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
#[derive(Clone)]
struct Linear {
    g: DMatrix<f64>,
    h: DVector<f64>,
    e: DMatrix<f64>,
    b: DVector<f64>,
    kinds: Vec<(usize, usize)>,
    ni: usize,
    lb: Vec<f64>,
    ub: Vec<f64>,
}
// kinds: 0 inequality, 1 lower, 2 upper, 3 nonlinear inequality.
fn ocn_linear(who: &str, n: usize, args: &[Value]) -> Result<Linear> {
    ocn_variables(who, n)?;
    let pair = |a: Option<&Value>,
                b: Option<&Value>,
                what: &str|
     -> Result<(DMatrix<f64>, DVector<f64>)> {
        let a = a
            .map(|v| ocn_matrix(v, who, what))
            .transpose()?
            .unwrap_or_else(|| DMatrix::zeros(0, n));
        let b = b
            .map(|v| ocn_vec(v, who, what))
            .transpose()?
            .unwrap_or_else(|| DVector::zeros(0));
        if (a.nrows() != 0 && a.ncols() != n) || a.nrows() != b.len() {
            return Err(format!("{who}: {what} dimensions do not agree"));
        }
        if a.is_empty() {
            Ok((DMatrix::zeros(0, n), b))
        } else {
            Ok((a, b))
        }
    };
    let (a, b) = pair(args.first(), args.get(1), "A,b")?;
    let (eq, beq) = pair(args.get(2), args.get(3), "Aeq,beq")?;
    let lb = ocn_bound(args.get(4), n, true, who)?;
    let ub = ocn_bound(args.get(5), n, false, who)?;
    let ni = a.nrows();
    let mut kinds: Vec<_> = (0..ni).map(|i| (0, i)).collect();
    for i in 0..n {
        if lb[i].is_finite() {
            kinds.push((1, i));
        }
        if ub[i].is_finite() {
            kinds.push((2, i));
        }
    }
    ocn_size(n, ni, eq.nrows())?;
    ocn_workspace(n, kinds.len(), eq.nrows())?;
    let mut g = DMatrix::zeros(kinds.len(), n);
    let mut h = DVector::zeros(kinds.len());
    g.view_mut((0, 0), (ni, n)).copy_from(&a);
    h.rows_mut(0, ni).copy_from(&b);
    for (r, &(k, i)) in kinds.iter().enumerate().skip(ni) {
        g[(r, i)] = if k == 1 { -1. } else { 1. };
        h[r] = if k == 1 { -lb[i] } else { ub[i] };
    }
    Ok(Linear {
        g,
        h,
        e: eq,
        b: beq,
        kinds,
        ni,
        lb,
        ub,
    })
}
fn ocn_variables(who: &str, n: usize) -> Result<()> {
    if n == 0 {
        return Err(format!("{who}: the problem has no variables"));
    }
    Ok(())
}
fn ocn_size(n: usize, m: usize, p: usize) -> Result<()> {
    if n > 2000
        || m.saturating_add(p) > 4000
        || m.saturating_mul(n) > MAX_ELEMENTS
        || p.saturating_mul(n) > MAX_ELEMENTS
        || n.saturating_mul(n) > MAX_ELEMENTS
    {
        return Err("constrained optimization: dense problem exceeds the limit (2000 unknowns, 4000 constraint rows, 4000000 entries per dense matrix)".into());
    }
    Ok(())
}
// User limits exclude generated bound rows, residual variables and diagnostic
// slacks. Bound their workspaces separately: at most twice the user width,
// plus diagnostic slacks, and at most 10001 KKT rows (100020001 entries).
fn ocn_workspace(n: usize, m: usize, p: usize) -> Result<()> {
    if n > 4001 || m > 16001 || p > 8000 || n.saturating_add(p) > 10000 {
        return Err("constrained optimization: internal workspace exceeds the limit".into());
    }
    Ok(())
}
fn ocn_inf(v: &DVector<f64>) -> f64 {
    v.iter().fold(0f64, |a, v| a.max(v.abs()))
}
fn ocn_violation(l: &Linear, x: &DVector<f64>) -> f64 {
    (&l.g * x - &l.h)
        .iter()
        .fold(ocn_inf(&(&l.e * x - &l.b)), |a, v| a.max(*v))
}
fn ocn_objective(q: &DMatrix<f64>, c: &DVector<f64>, x: &DVector<f64>) -> f64 {
    0.5 * x.dot(&(q * x)) + c.dot(x)
}
type OcnDirection = (DVector<f64>, DVector<f64>, DVector<f64>, DVector<f64>);

#[derive(Clone)]
struct Solution {
    x: DVector<f64>,
    z: DVector<f64>,
    y: DVector<f64>,
    flag: f64,
    iterations: usize,
    optimality: f64,
    step: f64,
    message: String,
}
#[derive(Clone, Copy)]
struct OcnTolerances {
    optimality: f64,
    feasibility: f64,
}
fn ocn_uniform_tolerance(tol: f64) -> OcnTolerances {
    OcnTolerances {
        optimality: tol,
        feasibility: tol,
    }
}
fn ocn_solve(a: &DMatrix<f64>, b: &DVector<f64>) -> Option<DVector<f64>> {
    // Pivoted LU normally; rank-deficient equalities use a minimum-norm SVD solve.
    if let Some(x) = a.clone().lu().solve(b)
        && x.iter().all(|v| v.is_finite())
        && ocn_inf(&(a * &x - b)) <= 1e-8 * (1. + ocn_inf(b))
    {
        return Some(x);
    }
    a.clone()
        .svd(true, true)
        .solve(b, 1e-12 * a.amax().max(1.))
        .ok()
}
fn ocn_fraction(s: &DVector<f64>, d: &DVector<f64>) -> f64 {
    s.iter()
        .zip(d)
        .filter(|(_, d)| **d < 0.)
        .fold(1f64, |a, (s, d)| a.min(-s / d))
}

// Newton equations after eliminating ds,dz from Gx+s=h, Sz=mu and
// Hx+c+G'z+E'y=0. Factor once for the affine and corrected right sides.
fn ocn_ipm(
    e: &Engine,
    q: &DMatrix<f64>,
    c: &DVector<f64>,
    l: &Linear,
    x0: Option<&DVector<f64>>,
    limit: usize,
    tol: OcnTolerances,
) -> Result<Solution> {
    let (n, m, p) = (c.len(), l.h.len(), l.b.len());
    ocn_workspace(n, m, p)?;
    let mut x = x0.cloned().unwrap_or_else(|| DVector::zeros(n));
    let mut y = DVector::zeros(p);
    let mut z = DVector::from_element(m, 1.);
    let mut s = (&l.h - &l.g * &x).map(|v| v.max(1.));
    let mut result = Solution {
        x: x.clone(),
        z: z.clone(),
        y: y.clone(),
        flag: 0.,
        iterations: 0,
        optimality: f64::INFINITY,
        step: 0.,
        message: "Iteration limit reached".into(),
    };
    let dual_scale = 1. + ocn_inf(c);
    for iteration in 0..=limit {
        e.check_cancel()?;
        let rd = q * &x + c + l.g.transpose() * &z + l.e.transpose() * &y;
        let rp = &l.g * &x + &s - &l.h;
        let re = &l.e * &x - &l.b;
        let mu = if m == 0 { 0. } else { s.dot(&z) / m as f64 };
        let error = ocn_inf(&rd).max(ocn_inf(&rp)).max(ocn_inf(&re)).max(mu);
        result.iterations = iteration;
        result.optimality = ocn_inf(&rd);
        // Conditioning scales must not relax the original-coordinate contract.
        if ocn_inf(&rp).max(ocn_inf(&re)) <= tol.feasibility
            && ocn_violation(l, &x) <= tol.feasibility
            && ocn_inf(&rd).max(mu) <= tol.optimality * dual_scale
        {
            result.flag = 1.;
            result.message = "Primal-dual optimality and feasibility tolerances satisfied".into();
            break;
        }
        if iteration == limit || !error.is_finite() {
            break;
        }
        let ratio = z.component_div(&s);
        let mut weighted = l.g.clone();
        for i in 0..m {
            weighted.row_mut(i).scale_mut(ratio[i]);
        }
        let normal = q + l.g.transpose() * weighted;
        let mut regularized = normal.clone();
        let mut reg = 1e-12 * q.amax().max(1.);
        for _ in 0..8 {
            e.check_cancel()?;
            regularized.copy_from(&normal);
            for i in 0..n {
                regularized[(i, i)] += reg;
            }
            if regularized.clone().cholesky().is_some() {
                break;
            }
            reg *= 10.;
        }
        let mut kkt = DMatrix::zeros(n + p, n + p);
        kkt.view_mut((0, 0), (n, n)).copy_from(&regularized);
        kkt.view_mut((0, n), (n, p)).copy_from(&l.e.transpose());
        kkt.view_mut((n, 0), (p, n)).copy_from(&l.e);
        let lu = kkt.clone().lu();
        let direction = |rc: DVector<f64>| -> Option<OcnDirection> {
            let w = (&rc - z.component_mul(&rp)).component_div(&s);
            let mut rhs = DVector::zeros(n + p);
            rhs.rows_mut(0, n).copy_from(&(-&rd + l.g.transpose() * w));
            rhs.rows_mut(n, p).copy_from(&(-&re));
            let v = lu
                .solve(&rhs)
                .filter(|v| {
                    v.iter().all(|x| x.is_finite())
                        && ocn_inf(&(&kkt * v - &rhs)) < 1e-7 * (1. + ocn_inf(&rhs))
                })
                .or_else(|| ocn_solve(&kkt, &rhs))?;
            let dx = v.rows(0, n).into_owned();
            let dy = v.rows(n, p).into_owned();
            let ds = -&rp - &l.g * &dx;
            let dz = (-rc - z.component_mul(&ds)).component_div(&s);
            Some((dx, dy, ds, dz))
        };
        let Some((_, _, ds, dz)) = direction(s.component_mul(&z)) else {
            break;
        };
        let ap = ocn_fraction(&s, &ds);
        let ad = ocn_fraction(&z, &dz);
        let mua = if m == 0 {
            0.
        } else {
            (&s + ap * &ds).dot(&(&z + ad * &dz)) / m as f64
        };
        let sigma = if mu > 0. {
            (mua / mu).clamp(0., 1.).powi(3)
        } else {
            0.
        };
        let rc = s.component_mul(&z) + ds.component_mul(&dz) - DVector::from_element(m, sigma * mu);
        let Some((dx, dy, ds, dz)) = direction(rc) else {
            break;
        };
        let ap = (0.995 * ocn_fraction(&s, &ds)).min(1.);
        let ad = (0.995 * ocn_fraction(&z, &dz)).min(1.);
        result.step = ap * ocn_inf(&dx);
        x += ap * dx;
        s += ap * ds;
        y += ad * dy;
        z += ad * dz;
        if x.iter()
            .chain(s.iter())
            .chain(z.iter())
            .chain(y.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e30)
        {
            break;
        }
    }
    result.x = x;
    result.z = z;
    result.y = y;
    Ok(result)
}
fn ocn_snap_bound(mut x: f64, lb: f64, ub: f64, tol: f64) -> f64 {
    if (x - lb).abs() <= tol {
        x = lb;
    }
    if (x - ub).abs() <= tol {
        x = ub;
    }
    x
}
fn ocn_empty_linear(
    n: usize,
    g: DMatrix<f64>,
    h: DVector<f64>,
    eq: DMatrix<f64>,
    b: DVector<f64>,
) -> Linear {
    Linear {
        g,
        h,
        e: eq,
        b,
        kinds: vec![],
        ni: 0,
        lb: vec![f64::NEG_INFINITY; n],
        ub: vec![f64::INFINITY; n],
    }
}
/// Minimize one elastic variable to test feasibility. Equalities are represented
/// by their two signed residuals so phase I itself always has a feasible start.
fn ocn_phase(e: &Engine, l: &Linear, tol: f64) -> Result<Solution> {
    let (m, p, n) = (l.h.len(), l.b.len(), l.g.ncols());
    ocn_workspace(n + 1, m + 2 * p + 1, 0)?;
    let mut g = DMatrix::zeros(m + 2 * p + 1, n + 1);
    let mut h = DVector::zeros(g.nrows());
    g.view_mut((0, 0), (m, n)).copy_from(&l.g);
    h.rows_mut(0, m).copy_from(&l.h);
    for i in 0..p {
        g.view_mut((m + 2 * i, 0), (1, n)).copy_from(&l.e.row(i));
        g.view_mut((m + 2 * i + 1, 0), (1, n))
            .copy_from(&(-l.e.row(i)));
        h[m + 2 * i] = l.b[i];
        h[m + 2 * i + 1] = -l.b[i];
    }
    for i in 0..g.nrows() {
        g[(i, n)] = -1.;
    }
    let phase = ocn_empty_linear(n + 1, g, h, DMatrix::zeros(0, n + 1), DVector::zeros(0));
    let mut c = DVector::zeros(n + 1);
    c[n] = 1.;
    ocn_ipm(
        e,
        &DMatrix::zeros(n + 1, n + 1),
        &c,
        &phase,
        None,
        150,
        ocn_uniform_tolerance(tol),
    )
}
fn ocn_qp_raw(
    e: &Engine,
    q: &DMatrix<f64>,
    c: &DVector<f64>,
    l: &Linear,
    x0: Option<&DVector<f64>>,
    limit: usize,
    tol: OcnTolerances,
) -> Result<Solution> {
    if q.amax() == 0. {
        // Objective scaling must not weaken primal feasibility or turn a large
        // finite cost into an apparent recession certificate. Undo it on duals.
        let scale = ocn_inf(c).max(1.);
        let mut r = ocn_hsd(e, &(c / scale), l, limit, tol)?;
        r.z *= scale;
        r.y *= scale;
        r.optimality *= scale;
        return Ok(r);
    }
    let mut r = ocn_ipm(e, q, c, l, x0, limit, tol)?;
    if r.flag > 0. {
        return Ok(r);
    }
    if limit == 0 {
        return Ok(r);
    }
    // Diagnostic certificates keep a tighter residual check than a loose
    // user stopping tolerance; they do not control successful termination.
    let tol = tol.optimality.min(tol.feasibility).min(1e-9);
    let phase = ocn_phase(e, l, tol)?;
    let n = c.len();
    if phase.flag > 0. && phase.x[n] > 100. * tol * (1. + ocn_inf(&l.h) + ocn_inf(&l.b)) {
        r.flag = -2.;
        r.message = "Infeasible constraints (positive phase-I minimum)".into();
        return Ok(r);
    }
    let feasible = phase.x.rows(0, n).into_owned();
    if ocn_violation(l, &feasible) > 100. * tol * (1. + ocn_inf(&l.h) + ocn_inf(&l.b)) {
        return Ok(r);
    }
    // A feasible recession ray satisfying Qd=0 and c'd<=-1 certifies
    // unboundedness for a positive-semidefinite quadratic objective.
    let mut g = DMatrix::zeros(l.g.nrows() + 1, n);
    let mut h = DVector::zeros(g.nrows());
    g.rows_mut(0, l.g.nrows()).copy_from(&l.g);
    g.row_mut(l.g.nrows()).copy_from(&c.transpose());
    h[l.g.nrows()] = -1.;
    let q_nonzero = q.amax() > 0.;
    let p = l.e.nrows() + if q_nonzero { n } else { 0 };
    ocn_workspace(n, g.nrows(), p)?;
    let mut eq = DMatrix::zeros(p, n);
    eq.rows_mut(0, l.e.nrows()).copy_from(&l.e);
    if q_nonzero {
        eq.rows_mut(l.e.nrows(), n).copy_from(q);
    }
    let ray = ocn_empty_linear(n, g, h, eq, DVector::zeros(p));
    let d = ocn_ipm(
        e,
        &DMatrix::identity(n, n),
        &DVector::zeros(n),
        &ray,
        None,
        100,
        ocn_uniform_tolerance(tol),
    )?;
    if ocn_violation(&ray, &d.x) <= 100. * tol && c.dot(&d.x) < -0.5 {
        r.flag = -3.;
        r.x = feasible;
        r.message = "Unbounded objective (verified feasible recession direction)".into();
    }
    Ok(r)
}
fn ocn_psd(q: DMatrix<f64>) -> Result<DMatrix<f64>> {
    ocn_variables("quadprog", q.ncols())?;
    if !q.is_square() {
        return Err("quadprog: H must be square".into());
    }
    ocn_size(q.ncols(), 0, 0)?;
    let q = 0.5 * (&q + q.transpose());
    let spectrum = q.symmetric_eigenvalues();
    let tol = 1e-10 * spectrum.amax().max(1.);
    if spectrum.min() < -tol {
        return Err("quadprog: H must be positive semidefinite for this solver".into());
    }
    Ok(q)
}
fn ocn_limit(o: &ConOptions) -> usize {
    o.base.max_iterations.min(1_000_000.) as usize
}
fn ocn_tolerance(o: &ConOptions) -> OcnTolerances {
    OcnTolerances {
        optimality: o.base.optimality_tol,
        feasibility: o.feasibility,
    }
}
fn ocn_algorithm(o: &ConOptions, who: &str) -> Result<String> {
    let a = o
        .base
        .algorithm
        .as_deref()
        .unwrap_or(if ocn_sqp_family(who) {
            "sqp"
        } else {
            "interior-point"
        });
    let supported = if who == "fmincon" {
        matches!(
            a,
            "interior-point" | "trust-region-reflective" | "sqp" | "sqp-legacy" | "active-set"
        )
    } else if who == "linprog" {
        matches!(
            a,
            "interior-point"
                | "interior-point-legacy"
                | "dual-simplex"
                | "dual-simplex-highs"
                | "dual-simplex-legacy"
        )
    } else if who == "quadprog" {
        matches!(
            a,
            "interior-point"
                | "interior-point-legacy"
                | "dual-simplex"
                | "sqp"
                | "sqp-legacy"
                | "active-set"
                | "interior-point-convex"
                | "trust-region-reflective"
        )
    } else {
        matches!(
            a,
            "interior-point"
                | "interior-point-legacy"
                | "dual-simplex"
                | "sqp"
                | "sqp-legacy"
                | "active-set"
        )
    };
    if !supported {
        return Err(format!("{who}: Algorithm '{a}' is not supported"));
    }
    let actual = if ocn_sqp_family(who) {
        "sqp"
    } else {
        "interior-point"
    };
    Ok(if a != actual {
        format!("; requested {a} is mapped to {actual}")
    } else {
        String::new()
    })
}

fn ocn_report(e: &Engine, r: &Solution, l: &Linear, fc: f64, alg: &str) -> Result<Value> {
    let text = |s: &str| -> Result<Array> {
        Ok(Array::Character(crate::CharacterRow::from_units(
            s.encode_utf16().collect(),
            &e.cancel,
        )?))
    };
    onl::onl_struct(
        e,
        vec![
            (
                "iterations",
                Array::Numeric(Matrix::scalar(r.iterations as f64)),
            ),
            ("funcCount", Array::Numeric(Matrix::scalar(fc))),
            (
                "constrviolation",
                Array::Numeric(Matrix::scalar(ocn_violation(l, &r.x))),
            ),
            ("stepsize", Array::Numeric(Matrix::scalar(r.step))),
            (
                "firstorderopt",
                Array::Numeric(Matrix::scalar(r.optimality)),
            ),
            ("algorithm", text(alg)?),
            ("message", text(&r.message)?),
        ],
    )
}

fn ocn_has_point(r: &Solution, l: &Linear, feasibility: f64) -> bool {
    // Certificates/rays are internal evidence, not optimization results. On a
    // limit or failed search retain only a finite, feasible current iterate.
    matches!(r.flag, 0. | -1. | 1. | 2.)
        && r.x.iter().all(|v| v.is_finite())
        && (r.flag > 0. || ocn_violation(l, &r.x) <= feasibility)
}
fn ocn_empty_value() -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(0, 0, vec![])?))
}
fn ocn_lambda(
    e: &Engine,
    l: &Linear,
    r: &Solution,
    nni: usize,
    nne: usize,
    has_point: bool,
) -> Result<Value> {
    let names = [
        "ineqlin",
        "eqlin",
        "lower",
        "upper",
        "ineqnonlin",
        "eqnonlin",
    ];
    if !has_point {
        return onl::onl_struct(
            e,
            names
                .iter()
                .map(|&name| Ok((name, Array::Numeric(Matrix::from_column_vec(0, 0, vec![])?))))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    let mut fields = vec![
        vec![0.; l.ni],
        r.y.iter()
            .take(r.y.len().saturating_sub(nne))
            .copied()
            .collect(),
        vec![0.; r.x.len()],
        vec![0.; r.x.len()],
        vec![0.; nni],
        r.y.iter()
            .skip(r.y.len().saturating_sub(nne))
            .copied()
            .collect(),
    ];
    for (j, &(k, i)) in l.kinds.iter().enumerate() {
        fields[match k {
            0 => 0,
            1 => 2,
            2 => 3,
            _ => 4,
        }][i] = r.z[j];
    }
    onl::onl_struct(
        e,
        names
            .iter()
            .zip(fields)
            .map(|(&n, v)| Ok((n, Array::Numeric(Matrix::from_column_vec(v.len(), 1, v)?))))
            .collect::<Result<Vec<_>>>()?,
    )
}
fn ocn_problem_form(who: &str, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() != 1 {
        return Ok(args.to_vec());
    }
    let Value::Container(Array::Struct(s)) = &args[0] else {
        return Ok(args.to_vec());
    };
    if s.len() != 1 {
        return Err(format!("{who}: problem must be a scalar structure"));
    }
    let get = |name: &str| -> Result<Value> {
        if let Some(i) = s
            .field_names()
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
        {
            Ok(Value::from(s.get(i, 0)?.as_ref().clone()))
        } else {
            Ok(Value::Matrix(Matrix::from_column_vec(0, 0, vec![])?))
        }
    };
    let names: &[&str] = match who {
        "quadprog" => &[
            "H", "f", "Aineq", "bineq", "Aeq", "beq", "lb", "ub", "x0", "options",
        ],
        "lsqlin" => &[
            "C", "d", "Aineq", "bineq", "Aeq", "beq", "lb", "ub", "x0", "options",
        ],
        "intlinprog" => &[
            "f", "intcon", "Aineq", "bineq", "Aeq", "beq", "lb", "ub", "options",
        ],
        "fmincon" => &[
            "objective",
            "x0",
            "Aineq",
            "bineq",
            "Aeq",
            "beq",
            "lb",
            "ub",
            "nonlcon",
            "options",
        ],
        _ => &["f", "Aineq", "bineq", "Aeq", "beq", "lb", "ub", "options"],
    };
    let mut result = names.iter().map(|n| get(n)).collect::<Result<Vec<_>>>()?;
    let offset = if who == "linprog" { 1 } else { 2 };
    if onl::is_empty_value(&result[offset]) {
        result[offset] = get("A")?;
        result[offset + 1] = get("b")?;
    }
    if who == "fmincon" && onl::is_empty_value(&result[0]) {
        result[0] = get("fun")?;
    }
    Ok(result)
}
pub(super) fn ocn_outputs(
    e: &mut Engine,
    who: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match who {
        "fmincon" => 7,
        "fminimax" | "fgoalattain" => 5,
        "lsqlin" => 6,
        "intlinprog" => 4,
        _ => 5,
    };
    if count > max {
        return Err(format!("{who} does not support {count} outputs"));
    }
    if args.iter().any(|v| matches!(v, Value::QuotedString(_))) {
        return Err("String array storage is not implemented; use char(...) explicitly".into());
    }
    if who == "fminimax" || who == "fgoalattain" {
        return ocn_epigraph(e, who, args, count.max(1), out);
    }
    let expanded = ocn_problem_form(who, args)?;
    let args = expanded.as_slice();
    if who == "fmincon" {
        return ocn_fmincon(e, args, count.max(1), out);
    }

    let offset = if who == "linprog" { 1 } else { 2 };
    let min = if who == "linprog" { 1 } else { 2 };
    let max_args = if who == "linprog" {
        8
    } else if who == "intlinprog" {
        9
    } else {
        10
    };
    if args.len() < min || args.len() > max_args {
        return Err(format!("{who}: incorrect number of input arguments"));
    }
    let (q, c, ls) = if who == "linprog" || who == "intlinprog" {
        let mut c = ocn_vec(&args[0], who, "f")?;
        if c.is_empty() {
            let inferred = args
                .get(offset)
                .map(|a| ocn_matrix(a, who, "A"))
                .transpose()?
                .map_or(0, |a| a.ncols());
            let nb = args
                .get(offset + 4)
                .map(|v| ocn_vec(v, who, "lb"))
                .transpose()?
                .map_or(0, |v| v.len());
            let ne = args
                .get(offset + 2)
                .map(|a| ocn_matrix(a, who, "Aeq"))
                .transpose()?
                .map_or(0, |a| a.ncols());
            let nu = args
                .get(offset + 5)
                .map(|v| ocn_vec(v, who, "ub"))
                .transpose()?
                .map_or(0, |v| v.len());
            c = DVector::zeros(inferred.max(ne).max(nb).max(nu));
        }
        ocn_variables(who, c.len())?;
        ocn_size(c.len(), 0, 0)?;
        (DMatrix::zeros(c.len(), c.len()), c, None)
    } else if who == "quadprog" {
        let q = ocn_psd(ocn_matrix(&args[0], who, "H")?)?;
        let mut c = ocn_vec(&args[1], who, "f")?;
        if c.is_empty() {
            c = DVector::zeros(q.nrows());
        }
        if q.nrows() != c.len() {
            return Err("quadprog: H and f dimensions do not agree".into());
        }
        (q, c, None)
    } else {
        let a = ocn_matrix(&args[0], who, "C")?;
        let d = ocn_vec(&args[1], who, "d")?;
        ocn_variables(who, a.ncols())?;
        ocn_size(a.ncols(), a.nrows(), 0)?;
        if a.nrows() != d.len() {
            return Err("lsqlin: C and d dimensions do not agree".into());
        }
        (
            DMatrix::zeros(a.ncols(), a.ncols()),
            DVector::zeros(a.ncols()),
            Some((a, d)),
        )
    };
    let n = c.len();
    let linear_end = (offset + 6).min(args.len());
    let l = ocn_linear(who, n, &args[offset.min(args.len())..linear_end])?;
    let opt_index = if who == "linprog" {
        7
    } else if who == "intlinprog" {
        8
    } else {
        9
    };
    // QP/LS accept an options structure in the x0 slot when x0 is omitted.
    let short_options = who != "linprog"
        && who != "intlinprog"
        && matches!(args.get(8), Some(Value::Container(Array::Struct(_))));
    let o = ocn_options(e, who, args.get(if short_options { 8 } else { opt_index }))?;
    let note = ocn_algorithm(&o, who)?;
    let x0 = if who != "linprog" && who != "intlinprog" && !short_options {
        args.get(8)
            .filter(|v| !onl::is_empty_value(v))
            .map(|v| ocn_vec(v, who, "x0"))
            .transpose()?
    } else {
        None
    };
    if x0.as_ref().is_some_and(|x| x.len() != n) {
        return Err(format!("{who}: x0 has the wrong length"));
    }
    let mut integer_stats = None;
    let mut r = if let Some((a, d)) = ls.as_ref() {
        ocn_lsq(e, a, d, &l, x0.as_ref(), ocn_limit(&o), ocn_tolerance(&o))?
    } else if who == "intlinprog" {
        let (r, stats) = ocn_integer(e, &c, &l, &args[1], &o)?;
        integer_stats = Some(stats);
        r
    } else {
        ocn_qp(e, &q, &c, &l, x0.as_ref(), ocn_limit(&o), ocn_tolerance(&o))?
    };
    if r.flag > 0.
        && (!r.x.iter().all(|v| v.is_finite()) || ocn_violation(&l, &r.x) > o.feasibility)
    {
        r.flag = 0.;
        r.message = "Returned point does not satisfy ConstraintTolerance".into();
    }
    r.message.push_str(&note);
    let f = ocn_objective(&q, &c, &r.x);
    let mut has_point = ocn_has_point(&r, &l, o.feasibility)
        && f.is_finite()
        && (who != "intlinprog" || r.flag > 0.);
    let mut values = vec![
        Value::Matrix(Matrix::from_column_slice(n, 1, r.x.as_slice())?),
        Value::Matrix(Matrix::scalar(f)),
    ];
    if let Some((a, d)) = ls {
        let residual = &a * &r.x - d;
        let resnorm = residual.norm_squared();
        r.optimality =
            ocn_inf(&(a.transpose() * &residual + l.g.transpose() * &r.z + l.e.transpose() * &r.y));
        has_point &= resnorm.is_finite();
        values[1] = Value::Matrix(Matrix::scalar(resnorm));
        values.push(Value::Matrix(Matrix::from_column_slice(
            residual.len(),
            1,
            residual.as_slice(),
        )?));
    }
    if !has_point {
        for value in &mut values {
            *value = ocn_empty_value()?;
        }
    }
    values.push(Value::Matrix(Matrix::scalar(r.flag)));
    values.push(if let Some(stats) = integer_stats {
        ocn_integer_report(e, &r, &l, &stats)?
    } else {
        ocn_report(e, &r, &l, 0., "primal-dual interior-point")?
    });
    if who != "intlinprog" {
        values.push(ocn_lambda(e, &l, &r, 0, 0, has_point)?);
    }
    onl::onl_final(e, &o.base, r.flag, &r.message, out)?;
    values.truncate(count.max(1));
    Ok(values)
}
#[derive(Clone)]
struct Node {
    l: Linear,
    bound: f64,
}
struct IntegerStats {
    nodes: usize,
    relative: f64,
    absolute: f64,
}
fn ocn_integer(
    e: &Engine,
    c: &DVector<f64>,
    l: &Linear,
    indices: &Value,
    o: &ConOptions,
) -> Result<(Solution, IntegerStats)> {
    let input = ocn_vec(indices, "intlinprog", "intcon")?;
    let mut ints = Vec::new();
    for &i in input.iter() {
        if i < 1. || i.fract() != 0. || i > c.len() as f64 {
            return Err("intlinprog: intcon must contain positive integer indices <= n".into());
        }
        if !ints.contains(&(i as usize - 1)) {
            ints.push(i as usize - 1);
        }
    }
    ints.sort_unstable();
    let mut root = l.clone();
    for &i in &ints {
        root.lb[i] = root.lb[i].ceil();
        root.ub[i] = root.ub[i].floor();
    }
    for (j, &(kind, i)) in root.kinds.iter().enumerate() {
        if kind == 1 {
            root.h[j] = -root.lb[i];
        }
        if kind == 2 {
            root.h[j] = root.ub[i];
        }
    }
    let mut pending = vec![Node {
        l: root,
        bound: f64::NEG_INFINITY,
    }];
    let mut incumbent: Option<Solution> = None;
    let mut best = f64::INFINITY;
    let mut nodes = 0;
    let mut unresolved_bound = f64::INFINITY;
    let q = DMatrix::zeros(c.len(), c.len());
    while !pending.is_empty() && nodes < o.nodes {
        e.check_cancel()?;
        let lower = pending
            .iter()
            .map(|v| v.bound)
            .fold(unresolved_bound, f64::min);
        if best.is_finite()
            && (best - lower).max(0.) <= o.absolute_gap.max(o.relative_gap * best.abs().max(1.))
        {
            break;
        }
        let k = if incumbent.is_none() {
            pending.len() - 1
        } else {
            (0..pending.len())
                .min_by(|&i, &j| pending[i].bound.total_cmp(&pending[j].bound))
                .unwrap()
        };
        let node = pending.swap_remove(k);
        if node.bound >= best {
            continue;
        }
        nodes += 1;
        // A row and the node's box can give a direct Farkas certificate.
        // This also avoids solving an almost-inconsistent child whose LP
        // feasibility tolerance could otherwise accept the same branch again.
        if ocn_box_infeasible(&node.l, o.feasibility.min(1e-9)) {
            continue;
        }
        let mut r = ocn_qp(e, &q, c, &node.l, None, ocn_limit(o), ocn_tolerance(o))?;
        // Both -2 and -5 include a certified primal-infeasibility result.
        if r.flag == -2. || r.flag == -5. {
            continue;
        }
        if r.flag == -3. {
            // An LP ray alone is insufficient for a mixed-integer conclusion.
            // Check an integer feasible base and an integer-scalable ray.
            let feasible = ocn_qp(
                e,
                &q,
                &DVector::zeros(c.len()),
                &node.l,
                None,
                ocn_limit(o),
                ocn_tolerance(o),
            )?;
            let mut base = feasible.x.clone();
            for &i in &ints {
                base[i] = base[i].round();
            }
            let mut ray = r.x.clone();
            let integer_ray = (1..=1000).find(|&scale| {
                ints.iter().all(|&i| {
                    (ray[i] * scale as f64 - (ray[i] * scale as f64).round()).abs() <= o.integer_tol
                })
            });
            if let Some(scale) = integer_ray {
                ray *= scale as f64;
                for &i in &ints {
                    ray[i] = ray[i].round();
                }
            }
            if feasible.flag > 0.
                && ocn_violation(&node.l, &base) <= o.feasibility
                && integer_ray.is_some()
                && c.dot(&ray) < 0.
                && (&node.l.g * &ray).iter().all(|v| *v <= o.feasibility)
                && ocn_inf(&(&node.l.e * &ray)) <= o.feasibility
            {
                r.x = base;
                r.iterations = nodes;
                r.message =
                    "Integer feasible base and scalable recession certificate verified".into();
                return Ok((
                    r,
                    IntegerStats {
                        nodes,
                        relative: f64::INFINITY,
                        absolute: f64::INFINITY,
                    },
                ));
            }
        }
        if r.flag <= 0. {
            unresolved_bound = unresolved_bound.min(node.bound);
            continue;
        }
        let rd = c + node.l.g.transpose() * &r.z + node.l.e.transpose() * &r.y;
        let mut bound = ocn_lp_dual(&node.l, &r.z, &r.y, &rd);
        if bound.is_finite() {
            bound -= 1e-10 * (1. + bound.abs());
        }

        if bound >= best {
            continue;
        }
        let mut rounded = r.x.clone();
        for &i in &ints {
            rounded[i] = rounded[i].round();
        }
        // Integer coordinates stay exact. Continuous cleanup is optional: if
        // it breaks a constraint, check the original continuous coordinates.
        let mut snapped = rounded.clone();
        for i in 0..snapped.len() {
            if ints.binary_search(&i).is_err() {
                snapped[i] = ocn_snap_bound(snapped[i], l.lb[i], l.ub[i], o.feasibility);
            }
        }
        if ocn_violation(&node.l, &snapped) <= o.feasibility {
            rounded = snapped;
        }
        let rounded_feasible = rounded.iter().all(|v| v.is_finite())
            && ocn_violation(&node.l, &rounded) <= o.feasibility;
        if rounded_feasible {
            let objective = c.dot(&rounded);
            if objective.is_finite() && objective < best {
                // Retain verified rounding even when this relaxation must
                // branch. Keep r.x unchanged for fractional choice and bounds.
                let mut candidate = r.clone();
                candidate.x = rounded;
                best = objective;
                incumbent = Some(candidate);
            }
        }
        let mut branch = None;
        // Near-integrality is sufficient only when rounding preserves feasibility.
        let mut fraction = if rounded_feasible { o.integer_tol } else { 0. };
        for &i in &ints {
            let f = (r.x[i] - r.x[i].round()).abs();
            if f > fraction && (branch.is_none() || f > fraction + 1e-12) {
                fraction = f;
                branch = Some(i);
            }
        }
        if let Some(i) = branch {
            if pending.len() + 2 > 100_000
                || (pending.len() + 2).saturating_mul(
                    node.l.g.len() + node.l.e.len() + node.l.h.len() + node.l.b.len() + 2 * c.len(),
                ) > 8 * MAX_ELEMENTS
            {
                unresolved_bound = unresolved_bound.min(bound);
                break;
            }
            // Push the upper branch first: depth-first visits x_i <= floor next.
            for upper in [false, true] {
                let mut child = node.l.clone();
                let m = child.h.len();
                ocn_workspace(c.len(), m + 1, child.b.len())?;
                child.g = child.g.insert_rows(m, 1, 0.);
                child.h = child.h.insert_rows(m, 1, 0.);
                child.g[(m, i)] = if upper { 1. } else { -1. };
                child.h[m] = if upper {
                    r.x[i].floor()
                } else {
                    -r.x[i].ceil()
                };
                if upper {
                    child.ub[i] = child.ub[i].min(r.x[i].floor());
                } else {
                    child.lb[i] = child.lb[i].max(r.x[i].ceil());
                }
                pending.push(Node { l: child, bound });
            }
        } else if !rounded_feasible {
            unresolved_bound = unresolved_bound.min(bound);
        }
    }
    let lower = pending
        .iter()
        .map(|v| v.bound)
        .fold(unresolved_bound, f64::min)
        .min(best);
    let absolute = if best.is_finite() {
        (best - lower).max(0.)
    } else {
        f64::INFINITY
    };
    let relative = if best.is_finite() {
        absolute / best.abs().max(1.)
    } else {
        f64::INFINITY
    };
    let stopped = (!pending.is_empty()
        || unresolved_bound.is_finite()
        || unresolved_bound == f64::NEG_INFINITY)
        && !(best.is_finite()
            && absolute <= o.absolute_gap.max(o.relative_gap * best.abs().max(1.)));
    let mut r = incumbent.unwrap_or_else(|| Solution {
        x: DVector::zeros(c.len()),
        z: DVector::zeros(l.h.len()),
        y: DVector::zeros(l.b.len()),
        flag: if stopped { 0. } else { -2. },
        iterations: nodes,
        optimality: f64::NAN,
        step: 0.,
        message: String::new(),
    });
    if best.is_finite() {
        r.flag = if stopped { 2. } else { 1. };
    }
    r.iterations = nodes;
    r.message = match r.flag {
        1. => "Integer optimality or requested gap tolerance proved",
        2. => "Node/workspace/relaxation limit with feasible integer point",
        -2. => "Certified integer infeasibility",
        _ => "Node/workspace/relaxation limit without feasible integer point",
    }
    .into();
    Ok((
        r,
        IntegerStats {
            nodes,
            relative,
            absolute,
        },
    ))
}
fn ocn_integer_report(e: &Engine, r: &Solution, l: &Linear, s: &IntegerStats) -> Result<Value> {
    let text = crate::CharacterRow::from_units(r.message.encode_utf16().collect(), &e.cancel)?;
    onl::onl_struct(
        e,
        vec![
            ("numnodes", Array::Numeric(Matrix::scalar(s.nodes as f64))),
            ("iterations", Array::Numeric(Matrix::scalar(s.nodes as f64))),
            ("relativegap", Array::Numeric(Matrix::scalar(s.relative))),
            ("absolutegap", Array::Numeric(Matrix::scalar(s.absolute))),
            (
                "constrviolation",
                Array::Numeric(Matrix::scalar(ocn_violation(l, &r.x))),
            ),
            ("message", Array::Character(text)),
        ],
    )
}

struct Nonlinear {
    handle: Option<FunctionHandle>,
    shape: (usize, usize),
    sizes: Option<(usize, usize)>,
    epigraph: Option<Epigraph>,
}
/// Epigraph form of fminimax and fgoalattain over z = [x; t]:
/// F_i(x) - weight_i*t - goal_i <= 0 come first among the nonlinear inequalities.
struct Epigraph {
    who: &'static str,
    fun: FunctionHandle,
    n: usize,
    goal: Vec<f64>,
    weight: Vec<f64>,
    objectives: Option<usize>,
}
impl Epigraph {
    /// F(x) as a real finite vector; the objective count must not change.
    fn ocn_values(
        &mut self,
        e: &mut Engine,
        x: Value,
        out: &mut Execution,
    ) -> Result<(DVector<f64>, (usize, usize))> {
        e.check_cancel()?;
        let who = self.who;
        let v = e.call_handle(&self.fun, vec![x], 1, None, out)?;
        let value = v
            .first()
            .ok_or_else(|| format!("{who}: missing objective output"))?;
        let f = onl::onl_real_vector(value, who, "objective values")?;
        if f.is_empty() {
            return Err(format!("{who}: fun must return at least one objective"));
        }
        if !f.data().iter().all(|v| v.is_finite()) {
            return Err(format!("{who}: objective values must be finite and real"));
        }
        if self.objectives.is_some_and(|m| m != f.len()) {
            return Err(format!("{who}: the number of objectives must not change"));
        }
        if self.objectives.is_none() {
            if who == "fminimax" {
                self.goal = vec![0.; f.len()];
                self.weight = vec![1.; f.len()];
            } else if self.goal.len() != f.len() || self.weight.len() != f.len() {
                return Err(format!(
                    "{who}: goal and weight must have one entry per objective ({})",
                    f.len()
                ));
            }
            self.objectives = Some(f.len());
        }
        Ok((DVector::from_column_slice(f.data()), (f.rows(), f.cols())))
    }
}
struct Point {
    f: f64,
    g: DVector<f64>,
    c: DVector<f64>,
    ce: DVector<f64>,
    j: DMatrix<f64>,
    je: DMatrix<f64>,
}
type ConstraintValues = (
    DVector<f64>,
    DVector<f64>,
    Option<(DMatrix<f64>, DMatrix<f64>)>,
);
impl Nonlinear {
    fn ocn_eval(
        &mut self,
        e: &mut Engine,
        _p: &mut OcnObjective,
        x: &DVector<f64>,
        gradient: bool,
        out: &mut Execution,
    ) -> Result<Option<ConstraintValues>> {
        if self.epigraph.is_some() {
            return self.ocn_epigraph_eval(e, _p, x, out);
        }
        let Some(handle) = &self.handle else {
            return Ok(Some((
                DVector::zeros(0),
                DVector::zeros(0),
                Some((DMatrix::zeros(0, x.len()), DMatrix::zeros(0, x.len()))),
            )));
        };
        e.check_cancel()?;
        let count = if gradient { 4 } else { 2 };
        let v = e.call_handle(
            handle,
            vec![Value::Matrix(Matrix::from_column_slice(
                self.shape.0,
                self.shape.1,
                x.as_slice(),
            )?)],
            count,
            None,
            out,
        )?;
        if v.len() < count {
            return Err(format!("fmincon: nonlcon must return {count} outputs"));
        }
        let c = ocn_vec(&v[0], "fmincon", "nonlinear inequalities")?;
        let ce = ocn_vec(&v[1], "fmincon", "nonlinear equalities")?;
        if let Some(s) = self.sizes
            && s != (c.len(), ce.len())
        {
            return Err("fmincon: nonlinear constraint result length must not change".into());
        }
        self.sizes = Some((c.len(), ce.len()));
        ocn_size(x.len(), c.len(), ce.len())?;
        let j = if gradient {
            let a = ocn_matrix(&v[2], "fmincon", "constraint gradient")?;
            let b = ocn_matrix(&v[3], "fmincon", "equality gradient")?;
            if (!c.is_empty() && a.shape() != (x.len(), c.len()))
                || (!ce.is_empty() && b.shape() != (x.len(), ce.len()))
            {
                return Err(
                    "fmincon: constraint gradient columns must correspond to constraints (n-by-m)"
                        .into(),
                );
            }
            Some((
                if c.is_empty() {
                    DMatrix::zeros(0, x.len())
                } else {
                    a.transpose()
                },
                if ce.is_empty() {
                    DMatrix::zeros(0, x.len())
                } else {
                    b.transpose()
                },
            ))
        } else {
            None
        };
        Ok(Some((c, ce, j)))
    }
    /// Epigraph constraints at z = [x; t], then the user's nonlcon at x.
    /// Each F evaluation counts toward funcCount and MaxFunctionEvaluations.
    fn ocn_epigraph_eval(
        &mut self,
        e: &mut Engine,
        p: &mut OcnObjective,
        z: &DVector<f64>,
        out: &mut Execution,
    ) -> Result<Option<ConstraintValues>> {
        let Some(g) = self.epigraph.as_mut() else {
            unreachable!("epigraph mode")
        };
        if p.evaluations >= p.max_evaluations {
            return Ok(None);
        }
        let who = g.who;
        let n = g.n;
        let xv = Value::Matrix(Matrix::from_column_slice(
            self.shape.0,
            self.shape.1,
            &z.as_slice()[..n],
        )?);
        let (f, _) = g.ocn_values(e, xv.clone(), out)?;
        p.evaluations += 1.;
        let t = z[n];
        let mut c: Vec<f64> = (0..f.len())
            .map(|i| f[i] - g.weight[i] * t - g.goal[i])
            .collect();
        let mut ce = Vec::new();
        if let Some(handle) = &self.handle {
            e.check_cancel()?;
            let v = e.call_handle(handle, vec![xv], 2, None, out)?;
            if v.len() < 2 {
                return Err(format!("{who}: nonlcon must return 2 outputs"));
            }
            c.extend(ocn_vec(&v[0], who, "nonlinear inequalities")?.iter());
            ce.extend(ocn_vec(&v[1], who, "nonlinear equalities")?.iter());
        }
        if let Some(s) = self.sizes
            && s != (c.len(), ce.len())
        {
            return Err(format!(
                "{who}: nonlinear constraint result length must not change"
            ));
        }
        self.sizes = Some((c.len(), ce.len()));
        ocn_size(z.len(), c.len(), ce.len())?;
        Ok(Some((DVector::from_vec(c), DVector::from_vec(ce), None)))
    }
}
#[allow(clippy::too_many_arguments)]
fn ocn_point(
    e: &mut Engine,
    p: &mut OcnObjective,
    nl: &mut Nonlinear,
    x: &DVector<f64>,
    o: &ConOptions,
    l: &Linear,
    out: &mut Execution,
) -> Result<OcnEvaluation<Point>> {
    let (f, given) = match p.eval(e, x.as_slice(), o.base.gradient, out)? {
        OcnEvaluation::Value(value) => value,
        OcnEvaluation::Rejected(reason) => return Ok(OcnEvaluation::Rejected(reason)),
        OcnEvaluation::Limit => return Ok(OcnEvaluation::Limit),
    };
    let g = match p.jacobian(
        e,
        x.as_slice(),
        f,
        given,
        o.base.central,
        Some((&l.lb, &l.ub)),
        out,
    )? {
        OcnEvaluation::Value(value) => value,
        OcnEvaluation::Rejected(reason) => return Ok(OcnEvaluation::Rejected(reason)),
        OcnEvaluation::Limit => return Ok(OcnEvaluation::Limit),
    };
    let Some((c, ce, j)) = nl.ocn_eval(e, p, x, o.constraint_gradient, out)? else {
        return Ok(OcnEvaluation::Limit);
    };
    let (j, je) = if let Some(j) = j {
        j
    } else {
        let mut j = DMatrix::zeros(c.len(), x.len());
        let mut je = DMatrix::zeros(ce.len(), x.len());
        let mut probe = x.clone();
        for k in 0..x.len() {
            if l.lb[k] == l.ub[k] {
                continue;
            }
            let h = if o.base.central {
                f64::EPSILON.cbrt()
            } else {
                0.1 * f64::EPSILON.sqrt()
            } * x[k].abs().max(1.);
            let (step, central) = ocn_fd_step(x[k], l.lb[k], l.ub[k], h, o.base.central);
            if step == 0. {
                continue;
            }
            probe[k] = (x[k] + step).clamp(l.lb[k], l.ub[k]);
            let Some((cp, ep, _)) = nl.ocn_eval(e, p, &probe, false, out)? else {
                return Ok(OcnEvaluation::Limit);
            };
            let (cm, em, den) = if central {
                probe[k] = (x[k] - step).clamp(l.lb[k], l.ub[k]);
                let Some((cm, em, _)) = nl.ocn_eval(e, p, &probe, false, out)? else {
                    return Ok(OcnEvaluation::Limit);
                };
                (cm, em, 2. * step)
            } else {
                (c.clone(), ce.clone(), step)
            };
            j.set_column(k, &((cp - cm) / den));
            je.set_column(k, &((ep - em) / den));
            probe[k] = x[k];
        }
        (j, je)
    };
    Ok(OcnEvaluation::Value(Point {
        f,
        g: DVector::from_column_slice(g.as_slice()),
        c,
        ce,
        j,
        je,
    }))
}
fn ocn_at(l: &Linear, p: &Point, x: &DVector<f64>, step: bool) -> Result<Linear> {
    let n = x.len();
    let m = l.h.len();
    let eq = l.b.len();
    ocn_size(n, l.ni + p.c.len(), eq + p.ce.len())?;
    ocn_workspace(n, m + p.c.len(), eq + p.ce.len())?;
    let mut a = l.clone();
    a.g = a.g.insert_rows(m, p.c.len(), 0.);
    a.h = a.h.insert_rows(m, p.c.len(), 0.);
    a.e = a.e.insert_rows(eq, p.ce.len(), 0.);
    a.b = a.b.insert_rows(eq, p.ce.len(), 0.);
    a.g.rows_mut(m, p.c.len()).copy_from(&p.j);
    a.e.rows_mut(eq, p.ce.len()).copy_from(&p.je);
    if step {
        for i in 0..n {
            a.lb[i] -= x[i];
            a.ub[i] -= x[i];
        }
        a.h.rows_mut(0, m).copy_from(&(&l.h - &l.g * x));
        a.b.rows_mut(0, eq).copy_from(&(&l.b - &l.e * x));
        a.h.rows_mut(m, p.c.len()).copy_from(&(-&p.c));
        a.b.rows_mut(eq, p.ce.len()).copy_from(&(-&p.ce));
    } else {
        a.h.rows_mut(m, p.c.len()).copy_from(&(&p.j * x - &p.c));
        a.b.rows_mut(eq, p.ce.len()).copy_from(&(&p.je * x - &p.ce));
    }
    a.kinds.extend((0..p.c.len()).map(|i| (3, i)));
    Ok(a)
}
fn ocn_l1(l: &Linear, x: &DVector<f64>, p: &Point) -> f64 {
    (&l.g * x - &l.h).iter().map(|v| v.max(0.)).sum::<f64>()
        + (&l.e * x - &l.b).abs().sum()
        + p.c.iter().map(|v| v.max(0.)).sum::<f64>()
        + p.ce.abs().sum()
}
fn ocn_damped_bfgs(b: &mut DMatrix<f64>, s: &DVector<f64>, y: &DVector<f64>) {
    // Powell (1978), damping as in Nocedal & Wright (2006), Algorithm 18.2.
    let bs = &*b * s;
    let sbs = s.dot(&bs);
    let sy = s.dot(y);
    if sbs <= 1e-25 || !sbs.is_finite() {
        return;
    }
    let theta = if sy >= 0.2 * sbs {
        1.
    } else {
        0.8 * sbs / (sbs - sy)
    };
    let r = theta * y + (1. - theta) * &bs;
    let sr = s.dot(&r);
    if sr > 0. {
        *b += (&r * r.transpose()) / sr - (&bs * bs.transpose()) / sbs;
        *b = 0.5 * (&*b + b.transpose());
    }
}
// Nocedal & Wright (2006), ch. 12/18: current-point KKT residual.
// QP multipliers may cancel the current gradient while complementing only
// the proposed step. Use CURRENT slacks for every inequality, including bounds
// and nonlinear constraints represented by ocn_at(..., false).
fn ocn_sqp_optimality(lag: &DVector<f64>, l: &Linear, x: &DVector<f64>, z: &DVector<f64>) -> f64 {
    let residual = &l.g * x - &l.h;
    if lag
        .iter()
        .chain(z.iter())
        .chain(residual.iter())
        .any(|v| !v.is_finite())
    {
        return f64::INFINITY;
    }
    z.iter()
        .zip(residual.iter())
        .fold(ocn_inf(lag), |error, (&dual, &slack)| {
            error.max(-dual).max((dual * slack).abs())
        })
}
fn ocn_sqp_stop(r: &mut Solution, violation: f64, o: &ConOptions, elastic_stop: bool) -> bool {
    let feasible = violation <= o.feasibility;
    let status = if feasible && r.optimality <= o.base.optimality_tol {
        Some((
            1.,
            "SQP first-order optimality and constraint tolerances satisfied",
        ))
    } else if elastic_stop {
        if !feasible {
            Some((-2., "Elastic SQP could not reduce infeasibility"))
        } else if r.step <= o.base.step_tol {
            Some((2., "Feasible SQP step is below StepTolerance"))
        } else {
            Some((
                -1.,
                "Elastic SQP could not find a usable direction at a feasible iterate",
            ))
        }
    } else {
        None
    };
    if let Some((flag, message)) = status {
        r.flag = flag;
        r.message = message.into();
        true
    } else {
        false
    }
}
struct SqpRun {
    r: Solution,
    current: Option<Point>,
    final_l: Linear,
    b: DMatrix<f64>,
}
/// The SQP iteration shared by fmincon and the epigraph solvers (fminimax,
/// fgoalattain). `x` is projected onto the bounds before the first evaluation.
#[allow(clippy::too_many_arguments)]
fn ocn_sqp_run(
    e: &mut Engine,
    who: &str,
    p: &mut OcnObjective,
    nl: &mut Nonlinear,
    l: &Linear,
    o: &ConOptions,
    mut x: DVector<f64>,
    out: &mut Execution,
) -> Result<SqpRun> {
    let n = x.len();
    if l.lb.iter().zip(&l.ub).any(|(a, b)| a > b) {
        return Err(format!("{who}: lb must not exceed ub"));
    }
    for i in 0..n {
        x[i] = x[i].clamp(l.lb[i], l.ub[i]);
    }
    let mut b = DMatrix::identity(n, n);
    let mut penalty: f64 = 10.;
    let mut current = match ocn_point(e, p, nl, &x, o, l, out)? {
        OcnEvaluation::Value(point) => Some(point),
        OcnEvaluation::Rejected(reason) => {
            return Err(format!(
                "{who}: {}",
                reason.replace("evaluated point", "starting point")
            ));
        }
        OcnEvaluation::Limit => None,
    };
    let mut r = Solution {
        x: x.clone(),
        z: DVector::zeros(l.h.len()),
        y: DVector::zeros(l.b.len()),
        flag: 0.,
        iterations: 0,
        optimality: f64::INFINITY,
        step: 0.,
        message: "Iteration or function evaluation limit reached".into(),
    };
    let mut final_l = l.clone();
    let mut elastic_slack = 0.;
    for it in 0..ocn_limit(o) {
        e.check_cancel()?;
        let Some(point) = current.as_ref() else {
            break;
        };
        // SQP model solves need greater accuracy than the outer convergence
        // test, which uses the user's two tolerances independently.
        let model_tol = o.base.optimality_tol.min(o.feasibility).min(1e-9);
        let inner_tol = ocn_uniform_tolerance(if ocn_inf(&point.g) < 1e-3 {
            model_tol.min(1e-12)
        } else {
            model_tol
        });
        let step_l = ocn_at(l, point, &x, true)?;
        let at = ocn_at(l, point, &x, false)?;
        let mut sub = ocn_qp(e, &b, &point.g, &step_l, None, 100, inner_tol)?;
        final_l = at.clone();
        r.iterations = it;
        // An inner iteration limit can still provide an adequate SQP direction;
        // require primal and dual residuals before using that approximate QP.
        if sub.flag == 0. && ocn_violation(&step_l, &sub.x) <= 1e-7 && sub.optimality <= 1e-7 {
            sub.flag = 1.;
        }
        let mut elastic_stop = false;
        elastic_slack = 0.;
        if sub.flag <= 0. {
            // Nocedal & Wright sec. 18.3: elastic l1 QP, retaining the
            // objective model while relaxing inconsistent linearized constraints.
            (sub, elastic_slack) = ocn_elastic(e, &b, &point.g, &step_l, penalty, inner_tol)?;
            elastic_stop = sub.flag <= 0. || ocn_inf(&sub.x) <= o.base.step_tol;
        }
        let lag = &point.g + at.g.transpose() * &sub.z + at.e.transpose() * &sub.y;
        r.optimality = ocn_sqp_optimality(&lag, &at, &x, &sub.z);
        r.z = sub.z.clone();
        r.y = sub.y.clone();
        r.step = ocn_inf(&sub.x);
        if ocn_sqp_stop(&mut r, ocn_violation(&at, &x), o, elastic_stop) {
            break;
        }
        penalty = penalty.max(1. + 1.1 * ocn_inf(&sub.z).max(ocn_inf(&sub.y)));
        let viol = ocn_l1(l, &x, point);
        let merit = point.f + penalty * viol;
        let predicted = point.g.dot(&sub.x) - penalty * viol;
        let mut alpha = 1.;
        let mut accepted = None;
        for _ in 0..40 {
            e.check_cancel()?;
            let trial = ocn_project(&(&x + alpha * &sub.x), l);
            let candidate = match ocn_point(e, p, nl, &trial, o, l, out)? {
                OcnEvaluation::Value(point) => point,
                OcnEvaluation::Rejected(_) => {
                    alpha *= 0.5;
                    continue;
                }
                OcnEvaluation::Limit => break,
            };
            let value = candidate.f + penalty * ocn_l1(l, &trial, &candidate);
            if value <= merit + 1e-4 * alpha * predicted.min(-1e-12 * sub.x.norm_squared()) {
                accepted = Some((trial, candidate));
                break;
            }
            // Second-order correction (Nocedal & Wright sec. 18.3) uses
            // nonlinear residuals at the full trial to avoid the Maratos effect.
            if alpha == 1. && (!point.c.is_empty() || !point.ce.is_empty()) {
                let correction_l = ocn_at(l, &candidate, &trial, true)?;
                let correction = ocn_qp(
                    e,
                    &DMatrix::identity(n, n),
                    &DVector::zeros(n),
                    &correction_l,
                    None,
                    80,
                    inner_tol,
                )?;
                if correction.flag > 0. && ocn_inf(&correction.x) <= 0.5 * ocn_inf(&sub.x).max(1e-8)
                {
                    let corrected = ocn_project(&(&trial + correction.x), l);
                    if let OcnEvaluation::Value(cp) = ocn_point(e, p, nl, &corrected, o, l, out)?
                        && cp.f + penalty * ocn_l1(l, &corrected, &cp)
                            <= merit + 1e-4 * predicted.min(-1e-12 * sub.x.norm_squared())
                    {
                        accepted = Some((corrected, cp));
                        break;
                    }
                }
            }
            alpha *= 0.5;
        }
        let Some((next, next_point)) = accepted else {
            if p.evaluations >= p.max_evaluations {
                break;
            }
            r.flag = -1.;
            r.message = "SQP merit line search could not make progress".into();
            break;
        };
        let next_at = ocn_at(l, &next_point, &next, false)?;
        let new_lag =
            &next_point.g + next_at.g.transpose() * &sub.z + next_at.e.transpose() * &sub.y;
        let s = &next - &x;
        ocn_damped_bfgs(&mut b, &s, &(new_lag - lag));
        r.step = ocn_inf(&s);
        r.iterations = it + 1;
        x = next;
        final_l = next_at;
        current = Some(next_point);
        if r.step <= o.base.step_tol && ocn_violation(&final_l, &x) <= o.feasibility {
            r.flag = 2.;
            r.message = "Feasible SQP step is below StepTolerance".into();
            break;
        }
        if o.base.display == onl::Display::Iter {
            e.emit(
                out,
                format!(
                    "SQP iteration {}: constraint violation {:.4e}, step {:.4e}",
                    it + 1,
                    ocn_violation(&final_l, &x),
                    r.step
                ),
            )?;
        }
    }
    r.x = x;
    if elastic_slack > o.feasibility
        && r.flag == -1.
        && ocn_violation(&final_l, &r.x) > o.feasibility
    {
        r.flag = -2.;
        r.message = "Elastic SQP could not reduce infeasibility".into();
    }
    if let Some(point) = current.as_ref() {
        final_l = ocn_at(l, point, &r.x, false)?;
        if r.z.len() == final_l.h.len() && r.y.len() == final_l.b.len() {
            let lag = &point.g + final_l.g.transpose() * &r.z + final_l.e.transpose() * &r.y;
            r.optimality = ocn_sqp_optimality(&lag, &final_l, &r.x, &r.z);
        }
    }
    Ok(SqpRun {
        r,
        current,
        final_l,
        b,
    })
}
/// Native likelihood SQP subproblem. No parser, option, or public solver changes.
/// All constraint rows are supplied explicitly as c + J*p <= 0.
pub(super) fn ocn_likelihood_qp(
    e: &Engine,
    hessian: &DMatrix<f64>,
    gradient: &DVector<f64>,
    jacobian: &DMatrix<f64>,
    values: &DVector<f64>,
) -> Result<(DVector<f64>, DVector<f64>, bool)> {
    let n = gradient.len();
    ocn_size(n, values.len(), 0)?;
    let linear = ocn_empty_linear(
        n,
        jacobian.clone(),
        -values,
        DMatrix::zeros(0, n),
        DVector::zeros(0),
    );
    let r = ocn_qp(
        e,
        hessian,
        gradient,
        &linear,
        None,
        100,
        ocn_uniform_tolerance(1e-12),
    )?;
    let adequate = r.flag > 0.
        || (r.flag == 0. && ocn_violation(&linear, &r.x) <= 1e-9 && r.optimality <= 1e-8);
    Ok((r.x, r.z, adequate))
}

fn ocn_fmincon(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.len() < 4 || args.len() > 10 {
        return Err("fmincon: expected fun,x0,A,b,Aeq,beq,lb,ub,nonlcon[,options]".into());
    }
    let x0 = onl::onl_real_vector(&args[1], "fmincon", "x0")?;
    onl::onl_check_finite("fmincon", "x0", x0.data())?;
    let n = x0.len();
    let l = ocn_linear("fmincon", n, &args[2..args.len().min(8)])?;
    let o = ocn_options(e, "fmincon", args.get(9))?;
    let note = ocn_algorithm(&o, "fmincon")?;
    let mut p = OcnObjective {
        handle: onl::onl_handle(e, "fmincon", &args[0])?,
        rows: x0.rows(),
        cols: x0.cols(),
        evaluations: 0.,
        max_evaluations: o.base.max_evaluations.unwrap_or(3000.),
        epigraph: None,
    };
    let mut nl = Nonlinear {
        handle: if args.get(8).is_none_or(onl::is_empty_value) {
            None
        } else {
            Some(onl::onl_handle(e, "fmincon", &args[8])?)
        },
        shape: (x0.rows(), x0.cols()),
        sizes: None,
        epigraph: None,
    };
    let SqpRun {
        mut r,
        current,
        final_l,
        b,
    } = ocn_sqp_run(
        e,
        "fmincon",
        &mut p,
        &mut nl,
        &l,
        &o,
        DVector::from_column_slice(x0.data()),
        out,
    )?;
    r.message.push_str(&note);
    let (f, g) = current
        .as_ref()
        .map_or((f64::NAN, DVector::from_element(n, f64::NAN)), |p| {
            (p.f, p.g.clone())
        });
    let (nc, ne) = nl.sizes.unwrap_or((0, 0));
    if r.z.len() != final_l.h.len() {
        r.z = DVector::zeros(final_l.h.len());
        r.y = DVector::zeros(final_l.b.len());
    }
    let has_point =
        current.is_some() && f.is_finite() && ocn_has_point(&r, &final_l, o.feasibility);
    let mut values = vec![
        p.x_value(r.x.as_slice())?,
        Value::Matrix(Matrix::scalar(f)),
        Value::Matrix(Matrix::scalar(r.flag)),
        ocn_report(e, &r, &final_l, p.evaluations, "sqp")?,
        ocn_lambda(e, &final_l, &r, nc, ne, has_point)?,
        Value::Matrix(Matrix::from_column_slice(n, 1, g.as_slice())?),
        Value::Matrix(Matrix::from_column_slice(n, n, b.as_slice())?),
    ];
    if !has_point {
        for i in [0, 1, 5, 6] {
            values[i] = ocn_empty_value()?;
        }
    }
    onl::onl_final(e, &o.base, r.flag, &r.message, out)?;
    values.truncate(count);
    Ok(values)
}

/// z = [x; t]: linear rows get a zero t column, t itself is free.
fn ocn_epigraph_linear(l: &Linear) -> Linear {
    let n = l.g.ncols();
    let mut a = l.clone();
    a.g = a.g.insert_column(n, 0.);
    a.e = a.e.insert_column(n, 0.);
    a.lb.push(f64::NEG_INFINITY);
    a.ub.push(f64::INFINITY);
    a
}
/// fminimax (min max_i F_i) and fgoalattain (Gembicki 1974: min gamma subject
/// to F_i - weight_i*gamma <= goal_i) as min t over z = [x; t] on the SQP path.
fn ocn_epigraph(
    e: &mut Engine,
    who: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let goal_mode = who == "fgoalattain";
    let who: &'static str = if goal_mode { "fgoalattain" } else { "fminimax" };
    let (lo, hi, off) = if goal_mode { (4, 12, 4) } else { (2, 10, 2) };
    if args.len() < lo || args.len() > hi {
        return Err(if goal_mode {
            "fgoalattain: expected fun,x0,goal,weight[,A,b,Aeq,beq,lb,ub,nonlcon,options]".into()
        } else {
            "fminimax: expected fun,x0[,A,b,Aeq,beq,lb,ub,nonlcon,options]".into()
        });
    }
    let fun = onl::onl_handle(e, who, &args[0])?;
    let x0 = onl::onl_real_vector(&args[1], who, "x0")?;
    onl::onl_check_finite(who, "x0", x0.data())?;
    let n = x0.len();
    ocn_variables(who, n)?;
    let (goal, weight) = if goal_mode {
        let goal = ocn_vec(&args[2], who, "goal")?;
        let weight = ocn_vec(&args[3], who, "weight")?;
        if goal.is_empty() || goal.len() != weight.len() {
            return Err(
                "fgoalattain: goal and weight must be nonempty vectors of equal length".into(),
            );
        }
        if weight.iter().all(|w| *w == 0.) {
            return Err("fgoalattain: weight must have at least one nonzero entry".into());
        }
        (goal.as_slice().to_vec(), weight.as_slice().to_vec())
    } else {
        (Vec::new(), Vec::new())
    };
    let base = ocn_linear(who, n, &args[off.min(args.len())..args.len().min(off + 6)])?;
    let o = ocn_options(e, who, args.get(off + 7))?;
    if o.base.gradient {
        return Err(format!(
            "{who}: SpecifyObjectiveGradient is not implemented; objective derivatives use finite differences"
        ));
    }
    let note = ocn_algorithm(&o, who)?;
    if base.lb.iter().zip(&base.ub).any(|(a, b)| a > b) {
        return Err(format!("{who}: lb must not exceed ub"));
    }
    let nonlcon = if args.get(off + 6).is_none_or(onl::is_empty_value) {
        None
    } else {
        Some(onl::onl_handle(e, who, &args[off + 6])?)
    };
    let l = ocn_epigraph_linear(&base);
    let mut g = Epigraph {
        who,
        fun: fun.clone(),
        n,
        goal,
        weight,
        objectives: None,
    };
    let x: Vec<f64> = (0..n)
        .map(|i| x0.data()[i].clamp(base.lb[i], base.ub[i]))
        .collect();
    let x_value = |x: &[f64]| -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_slice(
            x0.rows(),
            x0.cols(),
            x,
        )?))
    };
    // Start on the epigraph boundary: t0 = max over positive weights.
    let (f0, _) = g.ocn_values(e, x_value(&x)?, out)?;
    let t0 = (0..f0.len())
        .filter(|&i| g.weight[i] > 0.)
        .map(|i| (f0[i] - g.goal[i]) / g.weight[i])
        .fold(f64::NEG_INFINITY, f64::max);
    let mut z = DVector::from_vec(x);
    z = z.insert_row(n, if t0.is_finite() { t0 } else { 0. });
    let mut p = OcnObjective {
        handle: fun,
        rows: n + 1,
        cols: 1,
        evaluations: 1.,
        max_evaluations: o.base.max_evaluations.unwrap_or(3000.),
        epigraph: Some(n),
    };
    let mut nl = Nonlinear {
        handle: nonlcon,
        shape: (x0.rows(), x0.cols()),
        sizes: None,
        epigraph: Some(g),
    };
    let SqpRun {
        mut r,
        current,
        final_l,
        ..
    } = ocn_sqp_run(e, who, &mut p, &mut nl, &l, &o, z, out)?;
    r.message.push_str(&note);
    let has_point = current.is_some() && ocn_has_point(&r, &final_l, o.feasibility);
    let xs = &r.x.as_slice()[..n];
    let mut values = vec![x_value(xs)?];
    if has_point {
        let Some(g) = nl.epigraph.as_mut() else {
            unreachable!("epigraph mode")
        };
        let (f, shape) = g.ocn_values(e, x_value(xs)?, out)?;
        p.evaluations += 1.;
        values.push(Value::Matrix(Matrix::from_column_slice(
            shape.0,
            shape.1,
            f.as_slice(),
        )?));
        values.push(Value::Matrix(Matrix::scalar(if goal_mode {
            r.x[n]
        } else {
            f.max()
        })));
    } else {
        values[0] = ocn_empty_value()?;
        values.push(ocn_empty_value()?);
        values.push(ocn_empty_value()?);
    }
    values.push(Value::Matrix(Matrix::scalar(r.flag)));
    let algorithm = if goal_mode {
        "sqp (goal attainment epigraph)"
    } else {
        "sqp (minimax epigraph)"
    };
    values.push(ocn_report(e, &r, &final_l, p.evaluations, algorithm)?);
    onl::onl_final(e, &o.base, r.flag, &r.message, out)?;
    values.truncate(count);
    Ok(values)
}

// Homogeneous self-dual embedding for free primal variables and nonnegative
// slacks/duals: Gx+s=h*tau, Ex=b*tau, G'z+E'y=-c*tau,
// c'x+h'z+b'y=-kappa, s'z+tau*kappa=0 at a solution. Xu, Hung & Ye (1996),
// Wright (1997), ch. 9; Mehrotra affine/corrector centering (1992).
// Free columns are never split. Normal-equation regularisation keeps flat
// directions solvable; certificates are checked in the ORIGINAL equations.
fn ocn_hsd(
    e: &Engine,
    c: &DVector<f64>,
    l: &Linear,
    limit: usize,
    tol: OcnTolerances,
) -> Result<Solution> {
    let (n, m, p) = (c.len(), l.h.len(), l.b.len());
    ocn_workspace(n, m, p)?;
    let mut x = -c / ocn_inf(c).max(1.);
    let mut y = DVector::zeros(p);
    let mut z = DVector::from_element(m, 1.);
    let mut s = DVector::from_element(m, 1.);
    let (mut tau, mut kappa) = (1f64, 1f64);
    let mut r = Solution {
        x: x.clone(),
        y: y.clone(),
        z: z.clone(),
        flag: 0.,
        iterations: 0,
        optimality: f64::INFINITY,
        step: 0.,
        message: "Homogeneous solver iteration limit or numerical breakdown".into(),
    };
    // Scale only the infeasibility certificate, never successful feasibility.
    let primal_scale = 1. + ocn_inf(&l.h).max(ocn_inf(&l.b));
    let dual_scale = 1. + ocn_inf(c);
    for it in 0..=limit {
        e.check_cancel()?;
        r.iterations = it;
        let rd = l.g.transpose() * &z + l.e.transpose() * &y + c * tau;
        let rp = &l.g * &x + &s - &l.h * tau;
        let re = &l.e * &x - &l.b * tau;
        let rg = c.dot(&x) + l.h.dot(&z) + l.b.dot(&y) + kappa;
        let mu = (s.dot(&z) + tau * kappa) / (m + 1) as f64;
        let finite = x
            .iter()
            .chain(y.iter())
            .chain(z.iter())
            .chain(s.iter())
            .all(|v| v.is_finite())
            && tau.is_finite()
            && kappa.is_finite()
            && mu.is_finite()
            && rg.is_finite()
            && rd
                .iter()
                .chain(rp.iter())
                .chain(re.iter())
                .all(|v| v.is_finite());
        if !finite {
            r.flag = -4.;
            r.message = "NaN or nonfinite homogeneous iterate".into();
            break;
        }
        if tau > 1e-12 {
            let xx = &x / tau;
            let zz = &z / tau;
            let yy = &y / tau;
            let stationarity = l.g.transpose() * &zz + l.e.transpose() * &yy + c;
            let dual = ocn_inf(&stationarity);
            let primal_objective = c.dot(&xx);
            let dual_objective = ocn_lp_dual(l, &zz, &yy, &stationarity);
            let gap = (primal_objective - dual_objective).abs();
            if ocn_violation(l, &xx) <= tol.feasibility
                && dual <= tol.optimality * dual_scale
                && dual_objective.is_finite()
                && gap <= tol.optimality * primal_objective.abs().max(dual_objective.abs()).max(1.)
            {
                r.x = xx;
                r.z = zz;
                r.y = yy;
                r.optimality = dual;
                r.flag = 1.;
                r.message =
                    "Homogeneous primal-dual optimality and feasibility tolerances satisfied"
                        .into();
                return Ok(r);
            }
        }
        // Farkas: nonnegative z, G'z+E'y=0, h'z+b'y<0.
        // Recession: Gx<=0, Ex=0, c'x<0. Both => primal/dual infeasible.
        let dc = -(l.h.dot(&z) + l.b.dot(&y));
        let pc = -c.dot(&x);
        let dual_norm = ocn_inf(&z).max(ocn_inf(&y));
        let ray_norm = ocn_inf(&x);
        let certificate_tol = tol.optimality.min(tol.feasibility).min(1e-9);
        let primal_infeasible = dc.is_finite()
            && dc > certificate_tol * primal_scale * dual_norm.max(1e-30)
            && ocn_inf(&(l.g.transpose() * &z + l.e.transpose() * &y))
                <= certificate_tol * dual_norm
            && z.iter().all(|v| *v >= -certificate_tol * dual_norm);
        let dual_infeasible = pc.is_finite()
            && pc > certificate_tol * dual_scale * ray_norm.max(1e-30)
            && (&l.g * &x).iter().all(|v| *v <= certificate_tol * ray_norm)
            && ocn_inf(&(&l.e * &x)) <= certificate_tol * ray_norm;
        if primal_infeasible || dual_infeasible {
            r.flag = if primal_infeasible && dual_infeasible {
                -5.
            } else if primal_infeasible {
                -2.
            } else {
                -3.
            };
            r.message = if r.flag == -5. {
                "Primal and dual infeasibility certificates verified"
            } else if primal_infeasible {
                "Primal infeasibility Farkas certificate verified"
            } else {
                "Unbounded recession certificate verified"
            }
            .into();
            if r.flag == -3. {
                r.x = &x / pc;
                r.z = z;
                r.y = y;
                return Ok(r);
            }
            break;
        }
        if it == limit {
            break;
        }
        let ratio = z.component_div(&s);
        let mut weighted = l.g.clone();
        for i in 0..m {
            weighted.row_mut(i).scale_mut(ratio[i]);
        }
        let normal = l.g.transpose() * weighted;
        let dh = ratio.component_mul(&l.h);
        let gdh = l.g.transpose() * &dh;
        let mut kkt = DMatrix::zeros(n + p + 1, n + p + 1);
        kkt.view_mut((0, 0), (n, n)).copy_from(&normal);
        kkt.view_mut((0, n), (n, p)).copy_from(&l.e.transpose());
        kkt.view_mut((n, 0), (p, n)).copy_from(&l.e);
        kkt.view_mut((0, n + p), (n, 1)).copy_from(&(c - &gdh));
        kkt.view_mut((n, n + p), (p, 1)).copy_from(&(-&l.b));
        kkt.view_mut((n + p, 0), (1, n))
            .copy_from(&(c + &gdh).transpose());
        kkt.view_mut((n + p, n), (1, p)).copy_from(&l.b.transpose());
        kkt[(n + p, n + p)] = -l.h.dot(&dh) - kappa / tau;
        // Scale-aware primal-dual regularisation, retried if factorisation fails.
        let mut reg = 1e-12 * normal.amax().max(1.);
        let mut lu = None;
        for _ in 0..8 {
            e.check_cancel()?;
            let mut a = kkt.clone();
            for i in 0..n {
                a[(i, i)] += reg;
            }
            for i in n..n + p {
                a[(i, i)] -= reg;
            }
            if a.view((0, 0), (n, n)).into_owned().cholesky().is_none() {
                reg *= 10.;
                continue;
            }
            let factor = a.lu();
            if factor.is_invertible() {
                lu = Some(factor);
                break;
            }
            reg *= 10.;
        }
        let Some(lu) = lu else {
            break;
        };
        let direction = |rc: DVector<f64>, rt: f64| {
            let w = (-rc + z.component_mul(&rp)).component_div(&s);
            let mut rhs = DVector::zeros(n + p + 1);
            rhs.rows_mut(0, n).copy_from(&(-&rd - l.g.transpose() * &w));
            rhs.rows_mut(n, p).copy_from(&(-&re));
            rhs[n + p] = -rg - l.h.dot(&w) + rt / tau;
            let mut v = lu.solve(&rhs)?;
            // Iterative refinement against the unregularised Newton equations.
            for _ in 0..3 {
                let residual = &rhs - &kkt * &v;
                if ocn_inf(&residual) <= 1e-10 * (1. + ocn_inf(&rhs)) {
                    break;
                }
                v += lu.solve(&residual)?;
            }
            let dx = v.rows(0, n).into_owned();
            let dy = v.rows(n, p).into_owned();
            let dt = v[n + p];
            let ds = -&rp - &l.g * &dx + &l.h * dt;
            let dz = w + ratio.component_mul(&(&l.g * &dx - &l.h * dt));
            let dk = (-rt - kappa * dt) / tau;
            Some((dx, dy, ds, dz, dt, dk))
        };
        let Some((_, _, ds, dz, dt, dk)) = direction(s.component_mul(&z), tau * kappa) else {
            break;
        };
        let fraction = |v: f64, d: f64| if d < 0. { (-v / d).min(1.) } else { 1. };
        let ap = ocn_fraction(&s, &ds).min(fraction(tau, dt));
        let ad = ocn_fraction(&z, &dz).min(fraction(kappa, dk));
        let mua = ((&s + ap * &ds).dot(&(&z + ad * &dz)) + (tau + ap * dt) * (kappa + ad * dk))
            / (m + 1) as f64;
        let sigma = (mua / mu).clamp(0., 1.).powi(3);
        let rc = s.component_mul(&z) + ds.component_mul(&dz) - DVector::from_element(m, sigma * mu);
        let rt = tau * kappa + dt * dk - sigma * mu;
        let Some((dx, dy, ds, dz, dt, dk)) = direction(rc, rt) else {
            break;
        };
        let alpha = (0.995
            * ocn_fraction(&s, &ds)
                .min(ocn_fraction(&z, &dz))
                .min(fraction(tau, dt))
                .min(fraction(kappa, dk)))
        .min(1.);
        r.step = alpha * ocn_inf(&dx) / tau.max(1e-12);
        x += alpha * dx;
        y += alpha * dy;
        s += alpha * ds;
        z += alpha * dz;
        tau += alpha * dt;
        kappa += alpha * dk;
    }
    if tau > 1e-12 {
        r.x = &x / tau;
        r.y = &y / tau;
        r.z = &z / tau;
    }
    Ok(r)
}

// The minimum of each inequality row over the box is a weighted sum of
// bound inequalities. If it exceeds h, those rows form a Farkas certificate.
fn ocn_box_infeasible(l: &Linear, tol: f64) -> bool {
    (0..l.h.len()).any(|row| {
        let mut minimum = 0.;
        let mut scale = 1. + l.h[row].abs();
        for i in 0..l.g.ncols() {
            let a = l.g[(row, i)];
            if a == 0. {
                continue;
            }
            let term = a * if a > 0. { l.lb[i] } else { l.ub[i] };
            if !term.is_finite() {
                return false;
            }
            minimum += term;
            scale += term.abs();
        }
        minimum - l.h[row] > tol * scale
    })
}

// LP dual objective, including lower/upper bound multipliers in -h'z.
// Absorb stationarity residual into the bound multipliers: rd>=0 adds a
// lower multiplier, rd<0 an upper multiplier. This matters even for tiny rd
// when the corresponding bound is large (Wright 1997, ch. 9, weak duality).
fn ocn_lp_dual(l: &Linear, z: &DVector<f64>, y: &DVector<f64>, rd: &DVector<f64>) -> f64 {
    let mut dual = -l.h.dot(z) - l.b.dot(y);
    for i in 0..rd.len() {
        let endpoint = if rd[i] >= 0. { l.lb[i] } else { l.ub[i] };
        if endpoint.is_finite() {
            dual += rd[i] * endpoint;
        } else if rd[i].abs() > 1e-10 {
            return f64::NEG_INFINITY;
        }
    }
    dual
}

// Fixed-column elimination and rank-revealing QR on E' precede every LP/QP.
// Multipliers are expanded to the original groups after solving.
fn ocn_qp(
    e: &Engine,
    q: &DMatrix<f64>,
    c: &DVector<f64>,
    l: &Linear,
    x0: Option<&DVector<f64>>,
    limit: usize,
    tol: OcnTolerances,
) -> Result<Solution> {
    e.check_cancel()?;
    let n = c.len();
    // A finite box with no coupled rows is separable: each linear term is
    // minimized at its corresponding endpoint. Avoid copying the bound matrix
    // into presolve and cubic factorisations for this common case.
    if limit > 0
        && q.amax() == 0.
        && l.b.is_empty()
        && l.kinds.len() == l.h.len()
        && l.kinds.iter().all(|&(kind, _)| kind == 1 || kind == 2)
        && l.lb
            .iter()
            .zip(&l.ub)
            .all(|(a, b)| a.is_finite() && b.is_finite() && a <= b)
    {
        let x = DVector::from_fn(n, |i, _| if c[i] < 0. { l.ub[i] } else { l.lb[i] });
        let z = DVector::from_iterator(
            l.h.len(),
            l.kinds.iter().map(|&(kind, i)| {
                if kind == 1 {
                    c[i].max(0.)
                } else {
                    (-c[i]).max(0.)
                }
            }),
        );
        return Ok(Solution {
            x,
            z,
            y: DVector::zeros(0),
            flag: 1.,
            iterations: 0,
            optimality: 0.,
            step: 0.,
            message: "Separable finite-box optimum verified".into(),
        });
    }
    let fixed: Vec<_> = (0..n).filter(|&i| l.lb[i] == l.ub[i]).collect();
    let free: Vec<_> = (0..n).filter(|i| !fixed.contains(i)).collect();
    let mut origin = DVector::zeros(n);
    for &i in &fixed {
        origin[i] = l.lb[i];
    }
    let mut reduced = l.clone();
    reduced.g = DMatrix::from_fn(l.g.nrows(), free.len(), |i, j| l.g[(i, free[j])]);
    reduced.e = DMatrix::from_fn(l.e.nrows(), free.len(), |i, j| l.e[(i, free[j])]);
    reduced.h = &l.h - &l.g * &origin;
    reduced.b = &l.b - &l.e * &origin;
    reduced.lb = free.iter().map(|&i| l.lb[i]).collect();
    reduced.ub = free.iter().map(|&i| l.ub[i]).collect();
    let grows: Vec<_> = (0..reduced.h.len())
        .filter(|&i| reduced.g.row(i).norm() > 1e-14)
        .collect();
    let zero_inconsistent = (0..reduced.h.len())
        .any(|i| reduced.g.row(i).norm() <= 1e-14 && reduced.h[i] < -tol.feasibility);
    let original_g = reduced.g.clone();
    let original_h = reduced.h.clone();
    reduced.g = DMatrix::from_fn(grows.len(), free.len(), |i, j| original_g[(grows[i], j)]);
    reduced.h = DVector::from_fn(grows.len(), |i, _| original_h[grows[i]]);
    let mut eqrows: Vec<usize> = vec![];
    if !reduced.e.is_empty() {
        let qr = reduced.e.transpose().col_piv_qr();
        let rr = qr.r();
        let rank = (0..rr.nrows().min(rr.ncols()))
            .take_while(|&i| rr[(i, i)].abs() > 1e-11 * rr.amax().max(1.))
            .count();
        let mut order = DMatrix::from_fn(1, l.b.len(), |_, j| j);
        qr.p().permute_columns(&mut order);
        eqrows = (0..rank).map(|j| order[(0, j)]).collect();
    }
    let original_eq = reduced.e.clone();
    let original_b = reduced.b.clone();
    reduced.e = DMatrix::from_fn(eqrows.len(), free.len(), |i, j| original_eq[(eqrows[i], j)]);
    reduced.b = DVector::from_fn(eqrows.len(), |i, _| original_b[eqrows[i]]);
    let feasible_eq = if free.is_empty() {
        DVector::zeros(0)
    } else if reduced.b.is_empty() {
        DVector::zeros(free.len())
    } else {
        reduced
            .e
            .clone()
            .svd(true, true)
            .solve(&reduced.b, 1e-12)
            .unwrap_or_else(|_| DVector::zeros(free.len()))
    };
    let inconsistent = ocn_inf(&(&original_eq * &feasible_eq - &original_b)) > tol.feasibility
        || l.lb.iter().zip(&l.ub).any(|(a, b)| a > b);
    let mut r = Solution {
        x: origin.clone(),
        z: DVector::zeros(l.h.len()),
        y: DVector::zeros(l.b.len()),
        flag: 0.,
        iterations: 0,
        optimality: f64::INFINITY,
        step: 0.,
        message: String::new(),
    };
    if inconsistent || zero_inconsistent {
        r.flag = -2.;
        r.message = "Presolve certified inconsistent equalities or bounds".into();
        return Ok(r);
    }
    if free.is_empty() {
        r.flag = if ocn_violation(l, &origin) <= tol.feasibility {
            1.
        } else {
            -2.
        };
        r.message = "All variables fixed in presolve".into();
    } else {
        let qq = DMatrix::from_fn(free.len(), free.len(), |i, j| q[(free[i], free[j])]);
        let cc = c + q * &origin;
        let cc = DVector::from_fn(free.len(), |i, _| cc[free[i]]);
        let initial = x0.map(|x| DVector::from_fn(free.len(), |i, _| x[free[i]]));
        let rr = ocn_qp_raw(e, &qq, &cc, &reduced, initial.as_ref(), limit, tol)?;
        r.flag = rr.flag;
        if rr.flag == -3. && q.amax() == 0. {
            r.x.fill(0.);
        }
        r.iterations = rr.iterations;
        r.step = rr.step;
        r.message = rr.message;
        for (j, &i) in free.iter().enumerate() {
            r.x[i] = rr.x[j];
        }
        for (j, &i) in grows.iter().enumerate() {
            r.z[i] = rr.z[j];
        }
        for (j, &i) in eqrows.iter().enumerate() {
            r.y[i] = rr.y[j];
        }
    }
    // At a fixed bound either lower or upper can carry the missing stationarity.
    let gradient = q * &r.x + c + l.g.transpose() * &r.z + l.e.transpose() * &r.y;
    for &i in &fixed {
        for (j, &(kind, k)) in l.kinds.iter().enumerate() {
            if k == i && kind == 1 {
                r.z[j] += gradient[i].max(0.);
            }
            if k == i && kind == 2 {
                r.z[j] += (-gradient[i]).max(0.);
            }
        }
    }
    r.optimality = ocn_inf(&(q * &r.x + c + l.g.transpose() * &r.z + l.e.transpose() * &r.y));
    Ok(r)
}

// Augmented residual form: minimize ||r||^2/2 subject to Cx-r=d.
// Never forms C'C, preserving the conditioning of the least-squares data.
#[allow(clippy::too_many_arguments)]
fn ocn_lsq(
    e: &Engine,
    a: &DMatrix<f64>,
    d: &DVector<f64>,
    l: &Linear,
    x0: Option<&DVector<f64>>,
    limit: usize,
    tol: OcnTolerances,
) -> Result<Solution> {
    let (n, m) = (a.ncols(), a.nrows());
    // Charge the user's data and constraint rows once. For tall data, a thin
    // Householder QR gives ||Cx-d||^2 = ||Rx-(Q'd)[..n]||^2 + constant.
    // Compress before adding residual variables; no C'C or explicit Q is formed.
    ocn_size(n, m.saturating_add(l.ni), l.b.len())?;
    e.check_cancel()?;
    let mut r = if m > n {
        let qr = a.clone().qr();
        let mut rhs = d.clone();
        qr.q_tr_mul(&mut rhs);
        e.check_cancel()?;
        ocn_lsq_augmented(e, &qr.r(), &rhs.rows(0, n).into_owned(), l, x0, limit, tol)?
    } else {
        ocn_lsq_augmented(e, a, d, l, x0, limit, tol)?
    };
    r.optimality = ocn_inf(
        &(a.transpose() * (a * &r.x - d) + l.g.transpose() * &r.z + l.e.transpose() * &r.y),
    );
    Ok(r)
}
#[allow(clippy::too_many_arguments)]
fn ocn_lsq_augmented(
    e: &Engine,
    a: &DMatrix<f64>,
    d: &DVector<f64>,
    l: &Linear,
    x0: Option<&DVector<f64>>,
    limit: usize,
    tol: OcnTolerances,
) -> Result<Solution> {
    let (n, m) = (a.ncols(), a.nrows());
    // With no constraints, the rank-revealing SVD solves the rectangular
    // least-squares problem directly, including square and wide data. For tall
    // data this operates on the QR-compressed matrix supplied by ocn_lsq.
    // No residual unknowns, normal equations, or explicit inverse are needed.
    if limit > 0 && !a.is_empty() && l.h.is_empty() && l.b.is_empty() {
        e.check_cancel()?;
        if let Ok(x) = a.clone().svd(true, true).solve(d, 1e-12 * a.amax().max(1.)) {
            e.check_cancel()?;
            let optimality = ocn_inf(&(a.transpose() * (a * &x - d)));
            if optimality <= tol.optimality * (1. + ocn_inf(&(a.transpose() * d))) {
                return Ok(Solution {
                    x,
                    z: DVector::zeros(0),
                    y: DVector::zeros(0),
                    flag: 1.,
                    iterations: 1,
                    optimality,
                    step: 0.,
                    message: "Unconstrained rank-revealing least-squares optimum verified".into(),
                });
            }
        }
    }
    ocn_workspace(n + m, l.h.len(), l.b.len() + m)?;
    let mut q = DMatrix::zeros(n + m, n + m);
    for i in n..n + m {
        q[(i, i)] = 1.;
    }
    let mut augmented = l.clone();
    augmented.g = augmented.g.insert_columns(n, m, 0.);
    augmented.e = augmented
        .e
        .insert_columns(n, m, 0.)
        .insert_rows(l.b.len(), m, 0.);
    augmented.e.view_mut((l.b.len(), 0), (m, n)).copy_from(a);
    for i in 0..m {
        augmented.e[(l.b.len() + i, n + i)] = -1.;
    }
    augmented.b = augmented.b.insert_rows(l.b.len(), m, 0.);
    augmented.b.rows_mut(l.b.len(), m).copy_from(d);
    augmented.lb.extend(vec![f64::NEG_INFINITY; m]);
    augmented.ub.extend(vec![f64::INFINITY; m]);
    let initial = x0.map(|x| {
        let mut v = DVector::zeros(n + m);
        v.rows_mut(0, n).copy_from(x);
        v.rows_mut(n, m).copy_from(&(a * x - d));
        v
    });
    let mut r = ocn_qp(
        e,
        &q,
        &DVector::zeros(n + m),
        &augmented,
        initial.as_ref(),
        limit,
        tol,
    )?;
    r.x = r.x.rows(0, n).into_owned();
    r.y = r.y.rows(0, l.b.len()).into_owned();
    Ok(r)
}

// A rejected domain trial is recoverable; an exhausted evaluation budget is not.
// Callback errors and invalid output shapes still propagate through Result.
enum OcnEvaluation<T> {
    Value(T),
    Rejected(&'static str),
    Limit,
}
struct OcnObjective {
    handle: FunctionHandle,
    rows: usize,
    cols: usize,
    evaluations: f64,
    max_evaluations: f64,
    /// Epigraph solvers minimise z[n] = t; F is evaluated by Nonlinear.
    epigraph: Option<usize>,
}
impl OcnObjective {
    fn x_value(&self, x: &[f64]) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_slice(
            self.rows, self.cols, x,
        )?))
    }
    fn eval(
        &mut self,
        e: &mut Engine,
        x: &[f64],
        gradient: bool,
        out: &mut Execution,
    ) -> Result<OcnEvaluation<(f64, Option<Matrix>)>> {
        e.check_cancel()?;
        if self.evaluations >= self.max_evaluations {
            return Ok(OcnEvaluation::Limit);
        }
        if let Some(n) = self.epigraph {
            let mut g = vec![0.; x.len()];
            g[n] = 1.;
            return Ok(OcnEvaluation::Value((
                x[n],
                Some(Matrix::from_column_vec(1, x.len(), g)?),
            )));
        }
        let v = e.call_handle(
            &self.handle,
            vec![self.x_value(x)?],
            if gradient { 2 } else { 1 },
            None,
            out,
        )?;
        self.evaluations += 1.;
        let value = v.first().ok_or("fmincon: missing objective output")?;
        if let Value::Complex(f) = value {
            if f.len() != 1 {
                return Err("fmincon: objective must return a scalar".into());
            }
            return Ok(OcnEvaluation::Rejected(
                "objective at the evaluated point must be finite and real",
            ));
        }
        let f = onl::onl_real_vector(value, "fmincon", "objective")?;
        if f.len() != 1 {
            return Err("fmincon: objective must return a scalar".into());
        }
        if !f.data()[0].is_finite() {
            return Ok(OcnEvaluation::Rejected(
                "objective at the evaluated point must be finite and real",
            ));
        }
        let g = if gradient {
            let value = v.get(1).ok_or("fmincon: missing objective gradient")?;
            if let Value::Complex(g) = value {
                if g.len() != x.len() {
                    return Err("fmincon: gradient has wrong length".into());
                }
                return Ok(OcnEvaluation::Rejected(
                    "objective gradient at the evaluated point must be finite and real",
                ));
            }
            let g = onl::onl_real_vector(value, "fmincon", "gradient")?;
            if g.len() != x.len() {
                return Err("fmincon: gradient has wrong length".into());
            }
            if !g.data().iter().all(|v| v.is_finite()) {
                return Ok(OcnEvaluation::Rejected(
                    "objective gradient at the evaluated point must be finite and real",
                ));
            }
            Some(g)
        } else {
            None
        };
        Ok(OcnEvaluation::Value((f.data()[0], g)))
    }
    #[allow(clippy::too_many_arguments)]
    fn jacobian(
        &mut self,
        e: &mut Engine,
        x: &[f64],
        f: f64,
        given: Option<Matrix>,
        central: bool,
        bounds: Option<(&[f64], &[f64])>,
        out: &mut Execution,
    ) -> Result<OcnEvaluation<DMatrix<f64>>> {
        if let Some(g) = given {
            return Ok(OcnEvaluation::Value(DMatrix::from_row_slice(
                1,
                x.len(),
                g.data(),
            )));
        }
        let (lb, ub) = bounds.ok_or("fmincon: missing finite difference bounds")?;
        let mut j = DMatrix::zeros(1, x.len());
        let mut probe = x.to_vec();
        for k in 0..x.len() {
            e.check_cancel()?;
            if lb[k] == ub[k] {
                continue;
            }
            let h = if central {
                f64::EPSILON.cbrt()
            } else {
                0.1 * f64::EPSILON.sqrt()
            } * x[k].abs().max(1.);
            let (mut step, both) = ocn_fd_step(x[k], lb[k], ub[k], h, central);
            if step == 0. {
                continue;
            }
            let mut derivative = None;
            for _ in 0..40 {
                e.check_cancel()?;
                let dp = step.clamp(lb[k] - x[k], ub[k] - x[k]);
                let dm = (-step).clamp(lb[k] - x[k], ub[k] - x[k]);
                let xp = (x[k] + dp).clamp(lb[k], ub[k]);
                let xm = (x[k] + dm).clamp(lb[k], ub[k]);
                if xp == x[k] && xm == x[k] {
                    break;
                }
                let mut fp = None;
                let mut fm = None;
                if xp != x[k] {
                    probe[k] = xp;
                    match self.eval(e, &probe, false, out)? {
                        OcnEvaluation::Value((v, _)) => fp = Some(v),
                        OcnEvaluation::Rejected(_) => {}
                        OcnEvaluation::Limit => return Ok(OcnEvaluation::Limit),
                    }
                }
                // Use the finite side if the other side leaves the objective's
                // domain. At a bound ocn_fd_step prefers the inward direction.
                if (both || fp.is_none()) && xm != x[k] {
                    probe[k] = xm;
                    match self.eval(e, &probe, false, out)? {
                        OcnEvaluation::Value((v, _)) => fm = Some(v),
                        OcnEvaluation::Rejected(_) => {}
                        OcnEvaluation::Limit => return Ok(OcnEvaluation::Limit),
                    }
                }
                derivative = match (fp, fm) {
                    (Some(fp), Some(fm)) => Some((fp - fm) / (dp - dm)),
                    (Some(fp), None) => Some((fp - f) / dp),
                    (None, Some(fm)) => Some((fm - f) / dm),
                    _ => None,
                }
                .filter(|v| v.is_finite());
                if derivative.is_some() {
                    break;
                }
                step *= 0.5;
            }
            let Some(derivative) = derivative else {
                return Ok(OcnEvaluation::Rejected(
                    "no finite objective difference at the finite-difference trial points",
                ));
            };
            j[(0, k)] = derivative;
            probe[k] = x[k];
        }
        Ok(OcnEvaluation::Value(j))
    }
}
fn ocn_fd_step(x: f64, lb: f64, ub: f64, h: f64, central: bool) -> (f64, bool) {
    if central && x - lb >= h && ub - x >= h {
        (h, true)
    } else if ub - x >= x - lb {
        (h.min(ub - x), false)
    } else {
        (-h.min(x - lb), false)
    }
}
fn ocn_project(x: &DVector<f64>, l: &Linear) -> DVector<f64> {
    DVector::from_fn(x.len(), |i, _| x[i].clamp(l.lb[i], l.ub[i]))
}
fn ocn_elastic(
    e: &Engine,
    q: &DMatrix<f64>,
    c: &DVector<f64>,
    l: &Linear,
    penalty: f64,
    tol: OcnTolerances,
) -> Result<(Solution, f64)> {
    let (n, m, p) = (c.len(), l.h.len(), l.b.len());
    ocn_workspace(n + 1, m + 2 * p + 1, 0)?;
    let mut g = DMatrix::zeros(m + 2 * p + 1, n + 1);
    let mut h = DVector::zeros(g.nrows());
    g.view_mut((0, 0), (m, n)).copy_from(&l.g);
    h.rows_mut(0, m).copy_from(&l.h);
    for i in 0..p {
        g.view_mut((m + 2 * i, 0), (1, n)).copy_from(&l.e.row(i));
        h[m + 2 * i] = l.b[i];
        g.view_mut((m + 2 * i + 1, 0), (1, n))
            .copy_from(&(-l.e.row(i)));
        h[m + 2 * i + 1] = -l.b[i];
    }
    for i in 0..m + 2 * p {
        g[(i, n)] = if i < m
            && l.kinds
                .get(i)
                .is_some_and(|&(kind, _)| kind == 1 || kind == 2)
        {
            0.
        } else {
            -1.
        };
    }
    // The shared relaxation is a nonnegative violation, never a reward for
    // increasing the margin of already feasible constraints: -t <= 0.
    g[(m + 2 * p, n)] = -1.;
    let elastic = ocn_empty_linear(n + 1, g, h, DMatrix::zeros(0, n + 1), DVector::zeros(0));
    let mut qq = DMatrix::zeros(n + 1, n + 1);
    qq.view_mut((0, 0), (n, n)).copy_from(q);
    let mut cc = DVector::zeros(n + 1);
    cc.rows_mut(0, n).copy_from(c);
    cc[n] = penalty;
    let mut r = ocn_ipm(e, &qq, &cc, &elastic, None, 120, tol)?;
    let slack = r.x[n];
    r.x = r.x.rows(0, n).into_owned();
    r.y = DVector::from_fn(p, |i, _| r.z[m + 2 * i] - r.z[m + 2 * i + 1]);
    r.z = r.z.rows(0, m).into_owned();
    Ok((r, slack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocn_near_bound_qp_preserves_solver_point_and_diagnostics() {
        // Equality fixes x at a strictly interior point. The old 1e-9 cleanup
        // radius admits a 5e-10 snap, despite the tighter optimality request.
        let e = Engine::default();
        let target = 1. - 5e-10;
        let q = DMatrix::identity(1, 1);
        let c = DVector::from_element(1, -target);
        let mut l = ocn_empty_linear(
            1,
            DMatrix::from_column_slice(2, 1, &[-1., 1.]),
            DVector::from_column_slice(&[2., 1.]),
            DMatrix::identity(1, 1),
            DVector::from_element(1, target),
        );
        l.lb[0] = -2.;
        l.ub[0] = 1.;
        l.kinds = vec![(1, 0), (2, 0)];
        let tol = OcnTolerances {
            feasibility: 1e-9,
            optimality: 1e-14,
        };
        let original = ocn_ipm(&e, &q, &c, &l, None, 200, tol).unwrap();
        assert_eq!(original.flag, 1.);
        assert!((original.x[0] - target).abs() < 1e-13);
        assert!(original.optimality <= 2e-14);
        for (name, r) in [
            ("raw", ocn_qp_raw(&e, &q, &c, &l, None, 200, tol).unwrap()),
            ("presolved", ocn_qp(&e, &q, &c, &l, None, 200, tol).unwrap()),
        ] {
            println!(
                "{name}: original x={}, fo={}; returned x={}, fo={}, flag={}",
                original.x[0], original.optimality, r.x[0], r.optimality, r.flag
            );
            assert_eq!(r.flag, 1.);
            assert!((r.x[0] - target).abs() < 1e-13);
            assert!(r.optimality <= 2e-14);
        }
    }

    // One-dimensional KKT solution (Nocedal & Wright, sec. 16.6):
    // min .5*d^2 + c*d with d <= 1 has d=-c, multiplier zero for these c.
    fn consistent_elastic(c: f64) -> (Engine, Solution, Linear) {
        let e = Engine::default();
        let l = ocn_empty_linear(
            1,
            DMatrix::from_element(1, 1, 1.),
            DVector::from_element(1, 1.),
            DMatrix::zeros(0, 1),
            DVector::zeros(0),
        );
        let q = DMatrix::identity(1, 1);
        let c = DVector::from_element(1, c);
        let tol = ocn_uniform_tolerance(1e-12);
        let sub = ocn_qp(&e, &q, &c, &l, None, 0, tol).unwrap();
        assert_eq!(sub.flag, 0., "consistent QP stopped at its iteration limit");
        assert_eq!(ocn_violation(&l, &sub.x), 0.);
        let (elastic, slack) = ocn_elastic(&e, &q, &c, &l, 10., tol).unwrap();
        assert!(slack.abs() < 1e-10);
        assert_eq!(elastic.flag, 1.);
        (e, elastic, l)
    }

    #[test]
    fn ocn_elastic_consistent_linearization_does_not_maximize_margin() {
        let (_, r, l) = consistent_elastic(-0.25);
        assert!((r.x[0] - 0.25).abs() < 1e-7, "elastic direction {}", r.x[0]);
        assert!(ocn_violation(&l, &r.x) < 1e-10);
        assert!((r.x[0] - 0.25 + r.z[0]).abs() < 1e-9);
    }

    #[test]
    fn ocn_elastic_retains_positive_restoration_slack() {
        // Nocedal & Wright sec. 18.3 elastic model: 0*d=1 requires t>=1.
        // min .5*d^2-4*d+10*t therefore has d=4, t=1; a nonzero
        // objective direction alone does not establish successful restoration.
        let e = Engine::default();
        let l = ocn_empty_linear(
            1,
            DMatrix::zeros(0, 1),
            DVector::zeros(0),
            DMatrix::zeros(1, 1),
            DVector::from_element(1, 1.),
        );
        let (r, slack) = ocn_elastic(
            &e,
            &DMatrix::identity(1, 1),
            &DVector::from_element(1, -4.),
            &l,
            10.,
            ocn_uniform_tolerance(1e-12),
        )
        .unwrap();
        assert_eq!(r.flag, 1.);
        assert!((r.x[0] - 4.).abs() < 1e-9);
        assert!((slack - 1.).abs() < 1e-9);
    }

    #[test]
    fn ocn_sqp_kkt_requires_current_complementarity_and_nonnegative_duals() {
        let l = ocn_empty_linear(
            1,
            DMatrix::from_element(1, 1, 1.),
            DVector::from_element(1, 1.),
            DMatrix::zeros(0, 1),
            DVector::zeros(0),
        );
        let stationary = DVector::zeros(1);
        let z = DVector::from_element(1, 2.);
        // A multiplier complementary at the QP destination x=1 is not
        // complementary at current x=0, even with zero Lagrangian gradient.
        assert_eq!(
            ocn_sqp_optimality(&stationary, &l, &DVector::zeros(1), &z),
            2.
        );
        let active = DVector::from_element(1, 1.);
        assert_eq!(ocn_sqp_optimality(&stationary, &l, &active, &z), 0.);
        assert_eq!(ocn_sqp_optimality(&stationary, &l, &active, &(-&z)), 2.);
        assert_eq!(
            ocn_sqp_optimality(&DVector::from_element(1, 3.), &l, &active, &z),
            3.
        );
        assert!(
            ocn_sqp_optimality(
                &stationary,
                &l,
                &active,
                &DVector::from_element(1, f64::NAN)
            )
            .is_infinite()
        );
    }

    #[test]
    fn ocn_elastic_stop_checks_the_current_iterate() {
        let (e, mut r, l) = consistent_elastic(0.);
        let mut o = ocn_options(&e, "fmincon", None).unwrap();
        let x = DVector::zeros(1);
        r.optimality = ocn_inf(&(l.g.transpose() * &r.z));
        r.step = ocn_inf(&r.x);
        assert!(r.step <= o.base.step_tol, "elastic step {}", r.step);
        assert!(ocn_sqp_stop(&mut r, ocn_violation(&l, &x), &o, true));
        assert_eq!(r.flag, 1., "feasible stationary current point");

        o.base.optimality_tol = 1e-14;
        r.optimality = 1e-3;
        assert!(ocn_sqp_stop(&mut r, 0., &o, true));
        assert_eq!(r.flag, 2., "feasible tiny-step stop");
        r.step = 1.;
        assert!(ocn_sqp_stop(&mut r, 0., &o, true));
        assert_eq!(r.flag, -1., "failed direction at a feasible point");
        assert!(ocn_sqp_stop(&mut r, 1., &o, true));
        assert_eq!(r.flag, -2., "infeasible current point");
    }
}
