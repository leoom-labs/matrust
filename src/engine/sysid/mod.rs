//! System identification: iddata, idpoly/idtf/idss/idproc models, the
//! arx/armax/oe/bj/tfest/ssest/n4sid/procest estimators and the
//! compare/resid/predict/sim analyses. Built from MathWorks' public function
//! documentation (calling forms, properties, output shapes) and the published
//! algorithms cited in each file; see tool/refs/system_ident/.
mod analysis;
mod est;
mod lti;
mod num;
mod obj;

use super::{Engine, Execution, Value};
use crate::syntax::Expr;
use crate::{Array, LogicalMatrix, Result};

/// Builtins this module owns.
pub(super) const NAMES: &[&str] = &[
    "iddata", "idpoly", "idtf", "idss", "idproc", "arx", "armax", "oe", "bj", "tfest", "ssest",
    "n4sid", "procest", "compare", "resid",
];
pub(super) fn owns(name: &str) -> bool {
    NAMES.contains(&name)
}
/// Class name of an iddata or identified-model value.
pub(super) fn class(a: &Array) -> Option<&'static str> {
    obj::class(a)
}
pub(super) fn is_object(v: &Value) -> bool {
    obj::value_class(v).is_some()
}
/// `name = value` display text.
pub(super) fn display(a: &Array) -> Result<Option<String>> {
    obj::display(a)
}
/// Identified models among the values become the engine's tf/ss values.
pub(super) fn lti_args(
    e: &mut Engine,
    values: Vec<Value>,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    analysis::lti_args(e, values, out)
}
/// `data(samples, outputs, inputs)` for an iddata variable.
pub(super) fn index(
    e: &mut Engine,
    a: &Array,
    args: &[Expr],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if obj::class(a) != Some("iddata") {
        return Ok(None);
    }
    analysis::index(e, a, args, out).map(Some)
}
fn lti_consumer(name: &str) -> bool {
    super::control_lti::NAMES.contains(&name)
        || super::control_response::handles(name)
        || super::control_design::cds_handles(name)
}
/// Calls answered here: owned names always; shared names (predict, sim,
/// detrend, class, isa, size, ...) when an argument is one of our objects;
/// LTI functions with an identified model, converted to tf/ss first.
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    let ours = args.iter().any(is_object);
    if !owns(name) && !ours {
        return Ok(None);
    }
    let one = |v: Value| Ok(Some(vec![v]));
    let max_out = |n: usize| -> Result<()> {
        if count > n {
            Err(format!("{name}: too many output arguments"))
        } else {
            Ok(())
        }
    };
    match name {
        "iddata" => {
            max_out(1)?;
            return one(analysis::iddata(e, args)?);
        }
        "idpoly" | "idtf" | "idss" | "idproc" => {
            max_out(1)?;
            return one(analysis::construct(e, name, args, out)?);
        }
        "arx" | "armax" | "oe" | "bj" | "tfest" | "ssest" | "n4sid" | "procest" => {
            max_out(1)?;
            let data = analysis::data_of(
                args.first()
                    .ok_or_else(|| format!("{name}: expected estimation data"))?,
                name,
            )?;
            let rest = &args[1..];
            let v = match name {
                "tfest" => est::tfest(e, &data, rest)?,
                "ssest" | "n4sid" => est::state_space(e, name, &data, rest)?,
                "procest" => est::procest(e, &data, rest)?,
                _ => est::polynomial(e, name, &data, rest)?,
            };
            return one(v);
        }
        "compare" => {
            max_out(3)?;
            let mut v = analysis::compare(e, args)?;
            v.truncate(count.max(1));
            return Ok(Some(v));
        }
        "resid" => {
            max_out(3)?;
            let mut v = analysis::resid(e, args)?;
            v.truncate(count.max(1));
            return Ok(Some(v));
        }
        _ => {}
    }
    let first_ours = args.first().is_some_and(is_object);
    match name {
        "predict" if args.iter().any(obj::is_model) => {
            max_out(1)?;
            return one(analysis::predict(e, args)?);
        }
        "sim" if args.first().is_some_and(obj::is_model) => {
            max_out(1)?;
            return one(analysis::sim(e, args)?);
        }
        "detrend" if first_ours => {
            max_out(1)?;
            return one(analysis::detrend(e, args)?);
        }
        "size" if first_ours && args.len() <= 2 => {
            max_out(1)?;
            return one(analysis::size(e, args)?);
        }
        "class" if first_ours && args.len() == 1 => {
            return one(Value::Text(
                analysis::class_name(&args[0]).unwrap_or("struct").into(),
            ));
        }
        "isa" if first_ours && args.len() == 2 => {
            let c = analysis::class_name(&args[0]).unwrap_or("struct");
            let t = obj::text_of(&args[1]).ok_or("isa: the class name must be text")?;
            let yes = t == c
                || (c != "iddata"
                    && matches!(
                        t.as_str(),
                        "idlti" | "idParametric" | "lti" | "DynamicSystem"
                    ));
            return one(Value::Logical(LogicalMatrix::scalar(yes)));
        }
        "isstruct" if first_ours && args.len() == 1 => {
            return one(Value::Logical(LogicalMatrix::scalar(false)));
        }
        "disp" | "display" if first_ours && args.len() == 1 => {
            let Value::Container(a) = &args[0] else {
                unreachable!()
            };
            let text = obj::display(a)?.unwrap_or_default();
            e.emit(out, text)?;
            return Ok(Some(Vec::new()));
        }
        _ => {}
    }
    if args.iter().any(obj::is_model) && lti_consumer(name) {
        let conv = analysis::lti_args(e, args.to_vec(), out)?;
        if count > 1 {
            return Err(format!(
                "{name}: use tf(sys) or ss(sys) first for several outputs"
            ));
        }
        return Ok(Some(vec![e.call(name, conv, out)?]));
    }
    Ok(None)
}
