//! GARCH, EGARCH and GJR volatility models (Bollerslev 1986/1987, Nelson 1991,
//! Glosten-Jagannathan-Runkle 1993; MathWorks public object/method references).
//! Integration API for 5d: `Model`, `Kind`, `is_model`, `model_value`,
//! `conditional_variance(model, innovations, E0, V0, cancel)` and
//! `one_step_forecast(model, innovations, variances)`. These helpers take centered
//! innovations, chronological presamples (latest last), and return variances.
//! `conditional_variance` derives omitted presamples from the innovations' second
//! moment; callers needing other initialization pass explicit slices. `forecast_path`
//! takes aligned innovation/variance history; EGARCH exponentiates expected logs
//! (the documented biased forecast). All kernels are serial and cancellable.
//! Limits: degree 128, estimated parameters 128, 1e7 observations across paths,
//! additionally subject to the engine's workspace/evaluation/output budgets.
mod display;
mod estimate;
mod model;
mod recursion;
use super::{Engine, Execution, Value};
use crate::{Array, CellArray, Matrix, Result, StringArray, StructArray};
pub(super) use estimate::{initial_model as estimation_start, initial_values, opg_covariance};
pub use model::{Kind, Model};
pub use recursion::{conditional_variance, forecast_path, one_step_forecast};
pub(super) use recursion::{estimation_variance, likelihood as likelihood_contributions};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
const MAX_LAG: usize = 128;
const MAX_POINTS: usize = 10_000_000;
pub(super) const NAMES: &[&str] = &[
    "garch",
    "egarch",
    "gjr",
    "estimate",
    "forecast",
    "infer",
    "simulate",
    "summarize",
];
pub(super) fn handles(n: &str) -> bool {
    matches!(n, "garch" | "egarch" | "gjr")
}
pub fn is_model(e: &Engine, v: &Value) -> bool {
    model_value(e, v).is_some()
}
pub fn model_value(e: &Engine, v: &Value) -> Option<Arc<Model>> {
    super::objects::econ_vol_value(e, v)
}
pub(super) fn method(e: &Engine, n: &str, a: &[Value]) -> bool {
    NAMES.contains(&n) && !handles(n) && a.first().is_some_and(|v| is_model(e, v))
}
fn err(n: &str, id: &str, msg: &str) -> String {
    format!("econ:{n}:{id}: {msg}")
}
fn number(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}
fn string_array(s: &str) -> Result<Array> {
    Ok(Array::Str(StringArray::scalar(s)?))
}
fn string(s: &str) -> Result<Value> {
    Ok(Value::from(string_array(s)?))
}
fn text(v: &Value) -> Result<String> {
    super::table_fns::text_scalar(v)
        .ok_or_else(|| err("model", "InvalidText", "Expected scalar text."))
}
fn matrix(e: &Engine, v: &Value) -> Result<Matrix> {
    let a = v.clone().array(&e.cancel)?;
    match a {
        Array::Numeric(m) => Ok(m),
        _ => Err(err(
            "model",
            "InvalidData",
            "Expected a real double matrix.",
        )),
    }
}
fn scalar(e: &Engine, v: &Value) -> Result<f64> {
    let m = matrix(e, v)?;
    if m.len() != 1 {
        return Err(err("model", "InvalidScalar", "Expected a real scalar."));
    }
    Ok(m.data()[0])
}
fn order(e: &Engine, v: &Value, what: &str, zero: bool, max: usize) -> Result<usize> {
    let x = scalar(e, v)?;
    if !x.is_finite() || x.fract() != 0. || x < if zero { 0. } else { 1. } || x > max as f64 {
        return Err(err(
            "model",
            "InvalidOrder",
            &format!(
                "{what} must be {} integer at most {max}.",
                if zero { "a nonnegative" } else { "a positive" }
            ),
        ));
    }
    Ok(x as usize)
}
fn structure(f: Vec<(&str, Array)>) -> Result<Value> {
    let names = f.iter().map(|(n, _)| n.to_string()).collect();
    let fields = f.into_iter().map(|(_, v)| Arc::new(v)).collect();
    Ok(Value::from(Array::Struct(StructArray::from_slots(
        1,
        1,
        names,
        fields,
        &AtomicBool::new(false),
    )?)))
}
fn pairs<'a>(a: &'a [Value], allowed: &[&str]) -> Result<BTreeMap<String, &'a Value>> {
    if !a.len().is_multiple_of(2) {
        return Err(err(
            "model",
            "InvalidNameValue",
            "Expected Name=Value pairs.",
        ));
    }
    let mut m = BTreeMap::new();
    for p in a.as_chunks::<2>().0 {
        let name = text(&p[0])?;
        let n = name.to_lowercase();
        if !allowed.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
            return Err(err(
                "model",
                "InvalidNameValue",
                &format!("Unrecognized option '{name}'."),
            ));
        }
        if m.insert(n, &p[1]).is_some() {
            return Err(err(
                "model",
                "InvalidNameValue",
                "Duplicate name-value option.",
            ));
        }
    }
    Ok(m)
}
fn cancel(c: &AtomicBool) -> Result<()> {
    if c.load(Ordering::Relaxed) {
        Err("Execution cancelled".into())
    } else {
        Ok(())
    }
}
pub(super) fn call(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let result = call_inner(e, n, a, nout, out);
    result.map_err(|msg| {
        if msg.starts_with("econ:") {
            let id = msg
                .rsplit_once(": ")
                .map_or("econ:InvalidInput", |(id, _)| id)
                .to_owned();
            e.raise(msg, id)
        } else {
            msg
        }
    })
}
fn call_inner(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match n {
        "estimate" => 4,
        "infer" | "simulate" => 2,
        _ => 1,
    };
    if nout > max {
        return Err(err(n, "TooManyOutputs", "Too many output arguments."));
    }
    let v = if handles(n) {
        vec![super::objects::econ_vol_new(e, model::construct(e, n, a)?)?]
    } else {
        let m = model_value(e, a.first().ok_or("Not enough input arguments.")?)
            .ok_or("Expected a conditional variance model")?;
        match n {
            "estimate" => estimate::call(e, &m, &a[1..], nout, out)?,
            "summarize" => {
                if a.len() != 1 {
                    return Err("Too many input arguments.".into());
                }
                vec![display::summarize(e, &m, &a[0], nout, out)?]
            }
            _ => {
                m.complete()?;
                recursion::call(e, n, &m, &a[1..])?
            }
        }
    };
    e.charge_function_outputs(&v)?;
    Ok(v.into_iter().take(nout.max(1)).collect())
}
pub(super) use display::body as model_body;
pub(super) use model::property as model_property;
pub(super) fn model_assign(
    e: &mut Engine,
    m: &Model,
    steps: &[crate::syntax::PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<Model> {
    let result = model::assign(e, m, steps, value, out);
    result.map_err(|msg| {
        if let Some((id, _)) = msg.rsplit_once(": ")
            && id.starts_with("econ:")
        {
            let id = id.to_owned();
            e.raise(msg, id)
        } else {
            msg
        }
    })
}

pub(super) const METHODS: &[&str] = &["estimate", "forecast", "infer", "simulate", "summarize"];
pub(super) fn property_names(m: &Model) -> Vec<String> {
    let mut n = [
        "Description",
        "SeriesName",
        "Distribution",
        "P",
        "Q",
        "Constant",
        "GARCH",
        "ARCH",
        "Offset",
        "UnconditionalVariance",
    ]
    .map(String::from)
    .to_vec();
    if m.kind != Kind::Garch {
        n.push("Leverage".into());
    }
    n
}
/// summarize without outputs prints its report, including with a semicolon.
pub(super) fn statement(
    e: &mut Engine,
    st: &crate::syntax::Evaluation,
    out: &mut Execution,
) -> Result<bool> {
    use crate::syntax::Expr;
    if !st.targets.is_empty()
        || st.path_target.is_some()
        || st.indexed_target.is_some()
        || st.list_target.is_some()
    {
        return Ok(false);
    }
    let Expr::Call(n, args) = &st.expr else {
        return Ok(false);
    };
    if n != "summarize"
        || e.contains_array(n)
        || e.resolve_function(n)?.is_some()
        || !expression_method(e, n, args)?
    {
        return Ok(false);
    }
    let a = super::string_class::arguments(e, n, args, out)?;
    if !method(e, n, &a) {
        return Ok(false);
    }
    call(e, n, &a, 0, out)?;
    Ok(true)
}

pub(super) fn undefined_method(name: &str) -> String {
    format!("Undefined function or variable '{name}'.")
}

/// Scope expression dispatch before evaluating arguments: shared method names must
/// not evaluate another model family's constructors or random arguments twice.
pub(super) fn expression_method(
    e: &mut Engine,
    n: &str,
    a: &[crate::syntax::Expr],
) -> Result<bool> {
    use crate::syntax::Expr;
    if !METHODS.contains(&n) {
        return Ok(false);
    }
    match a.first() {
        Some(Expr::Name(n)) => Ok(e.array(n).is_some_and(|v| is_model(e, &Value::from(v)))),
        Some(Expr::Call(n, a)) => Ok(!e.contains_array(n)
            && e.resolve_function(n)?.is_none()
            && (handles(n) || (n == "estimate" && expression_method(e, n, a)?))),
        _ => Ok(false),
    }
}
