//! MPC numerical support, independent of the optional desktop front end.
//! QPs follow the public mpcActiveSetSolver contract: A*x <= b.
mod display;
pub(in crate::engine) mod estimator;
mod r#move;
pub(super) mod object;
pub(in crate::engine) mod prediction;
pub(crate) mod qp;
mod review;
mod sim;
pub(in crate::engine) mod state;

use super::{Engine, Execution, Value};
use crate::typed_array::{NumericClass, TypedArray};
use crate::{Array, CharacterRow, LogicalMatrix, Matrix, Result, StructArray};
pub(super) use display::body;
pub(super) use object::{
    MpcObject, assign, equal, get_method, property, property_names, set_method,
};
use qp::{QpOptions, solve, workspace_bytes};
use std::sync::Arc;

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "mpcActiveSetSolver"
            | "mpcActiveSetOptions"
            | "mpc"
            | "getEstimator"
            | "setEstimator"
            | "getindist"
            | "setindist"
            | "getoutdist"
            | "setoutdist"
            | "review"
            | "mpcmove"
            | "mpcmoveopt"
            | "mpcsimopt"
            | "mpcstate"
            | "setmpcsignals"
    )
}
fn text(e: &Engine, v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) => c.to_utf8(&e.cancel).ok(),
        _ => None,
    }
}
fn numeric(v: &Value, single: bool, name: &str) -> Result<Matrix> {
    match v {
        Value::Matrix(m) if !single && !m.is_nd() => Ok(m.clone()),
        Value::Container(Array::Typed(t))
            if single && t.class() == NumericClass::Single && t.ndims() <= 2 =>
        {
            if t.len() > qp::MAX_M * qp::MAX_N {
                return Err("mpcActiveSetSolver: input exceeds dimension limits".into());
            }
            t.to_matrix()
        }
        _ => Err(format!(
            "mpcActiveSetSolver: {name} must be a real {} matrix",
            if single { "single" } else { "double" }
        )),
    }
}
fn real_scalar(v: &Value) -> Option<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        Value::Container(Array::Typed(t)) if t.len() == 1 && t.class() == NumericClass::Single => {
            Some(t.real(0))
        }
        _ => None,
    }
}
fn logical(v: &Value, name: &str) -> Result<bool> {
    match v {
        Value::Logical(m) if m.len() == 1 => Ok(m.data()[0] != 0),
        _ => Err(format!(
            "mpcActiveSetSolver: {name} must be a logical scalar"
        )),
    }
}
fn array(m: Matrix, single: bool) -> Result<Array> {
    if single {
        Ok(Array::Typed(TypedArray::from_matrix(
            NumericClass::Single,
            &m,
        )?))
    } else {
        Ok(Array::Numeric(m))
    }
}
fn structure(e: &Engine, fields: Vec<(&str, Array)>) -> Result<Value> {
    let (names, slots) = fields
        .into_iter()
        .map(|(k, v)| (k.to_string(), Arc::new(v)))
        .unzip();
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?)))
}
fn options_value(e: &Engine, single: bool) -> Result<Value> {
    structure(
        e,
        vec![
            (
                "DataType",
                Array::Character(CharacterRow::from_units(
                    if single { "single" } else { "double" }
                        .encode_utf16()
                        .collect(),
                    &e.cancel,
                )?),
            ),
            ("MaxIterations", array(Matrix::scalar(200.), single)?),
            ("ConstraintTolerance", array(Matrix::scalar(1e-6), single)?),
            (
                "UseHessianAsInput",
                Array::Logical(LogicalMatrix::scalar(true)),
            ),
            (
                "IntegrityChecks",
                Array::Logical(LogicalMatrix::scalar(true)),
            ),
        ],
    )
}
fn options<'a>(e: &'a Engine, v: &Value) -> Result<(QpOptions<'a>, bool)> {
    let Value::Container(Array::Struct(s)) = v else {
        return Err(
            "mpcActiveSetSolver: options must be a scalar structure from mpcActiveSetOptions"
                .into(),
        );
    };
    if s.len() != 1 {
        return Err("mpcActiveSetSolver: options must be a scalar structure".into());
    }
    let mut opts = QpOptions {
        cancel: Some(&e.cancel),
        ..QpOptions::default()
    };
    let mut single = false;
    for (i, name) in s.field_names().iter().enumerate() {
        let v = Value::from(s.get(i, 0)?.as_ref().clone());
        match name.as_str() {
            "DataType" => {
                single = match text(e, &v).as_deref() {
                    Some("single") => true,
                    Some("double") => false,
                    _ => {
                        return Err(
                            "mpcActiveSetSolver: DataType must be 'double' or 'single'".into()
                        );
                    }
                }
            }
            "MaxIterations" => {
                let x = real_scalar(&v)
                    .filter(|x| x.is_finite() && *x >= 1. && *x <= 1_000_000. && x.fract() == 0.)
                    .ok_or(
                        "mpcActiveSetSolver: MaxIterations must be an integer from 1 to 1000000",
                    )?;
                opts.max_iterations = x as usize;
            }
            "ConstraintTolerance" => {
                opts.constraint_tolerance = real_scalar(&v)
                    .filter(|x| x.is_finite() && *x > 0.)
                    .ok_or("mpcActiveSetSolver: ConstraintTolerance must be finite and positive")?
            }
            "UseHessianAsInput" => opts.use_hessian_as_input = logical(&v, name)?,
            "IntegrityChecks" => opts.integrity_checks = logical(&v, name)?,
            _ => return Err(format!("mpcActiveSetSolver: unsupported option '{name}'")),
        }
    }
    Ok((opts, single))
}
fn constraint_matrix(m: &Matrix, n: usize, name: &str) -> Result<Matrix> {
    if m.is_empty() && (m.rows(), m.cols()) == (0, 0) {
        return Matrix::filled(0, n, 0.);
    }
    if m.cols() != n {
        return Err(format!("mpcActiveSetSolver: {name} must have n columns"));
    }
    Ok(m.clone())
}
fn column(m: &Matrix, n: usize, name: &str) -> Result<()> {
    if m.len() != n || (m.cols() != 1 && !(n == 0 && (m.rows(), m.cols()) == (0, 0))) {
        return Err(format!(
            "mpcActiveSetSolver: {name} must be a column vector of length {n}"
        ));
    }
    Ok(())
}
pub(super) fn outputs(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if name == "review" {
        return review::outputs(e, args, count, out);
    }
    if name == "mpcsimopt" {
        return sim::options(e, args, count);
    }
    if name == "sim" {
        return sim::outputs(e, args, count, out);
    }
    if name == "mpcmove" {
        return r#move::outputs(e, args, count, out);
    }
    if name == "mpcmoveopt" {
        return r#move::options(e, args, count);
    }
    if name == "setmpcsignals" {
        return object::set_signals(e, args, count, out);
    }
    if name == "mpc" {
        return object::construct(e, args, count, out);
    }
    if !matches!(name, "mpcActiveSetOptions" | "mpcActiveSetSolver") {
        return estimator::outputs(e, name, args, count, out);
    }
    if name == "mpcActiveSetOptions" {
        if args.len() > 1 || count > 1 {
            return Err("mpcActiveSetOptions expects zero or one input and one output".into());
        }
        let single = match args.first().map(|v| text(e, v)) {
            None => false,
            Some(Some(t)) if t == "double" => false,
            Some(Some(t)) if t == "single" => true,
            _ => return Err("mpcActiveSetOptions: type must be 'double' or 'single'".into()),
        };
        return Ok(vec![options_value(e, single)?]);
    }
    if args.len() != 8 || count > 4 {
        return Err("mpcActiveSetSolver expects eight inputs and at most four outputs".into());
    }
    let (opts, single) = options(e, &args[7])?;
    let h = numeric(&args[0], single, "H/Linv")?;
    let f = numeric(&args[1], single, "f")?;
    let a = constraint_matrix(&numeric(&args[2], single, "A")?, f.len(), "A")?;
    let b = numeric(&args[3], single, "b")?;
    let eq = constraint_matrix(&numeric(&args[4], single, "Aeq")?, f.len(), "Aeq")?;
    let beq = numeric(&args[5], single, "beq")?;
    column(&f, f.len(), "f")?;
    column(&b, a.rows(), "b")?;
    column(&beq, eq.rows(), "beq")?;
    let Value::Logical(warm) = &args[6] else {
        return Err("mpcActiveSetSolver: iA0 must be a logical vector".into());
    };
    if warm.ndims() > 2
        || warm.len() != a.rows()
        || (warm.rows() != 1 && warm.cols() != 1 && !warm.is_empty())
    {
        return Err("mpcActiveSetSolver: iA0 must be a logical vector of length m".into());
    }
    let bytes = workspace_bytes(f.len(), a.rows(), eq.rows())?;
    if bytes > e.limits.memory_bytes {
        return Err("mpcActiveSetSolver: workspace exceeds memory limit".into());
    }
    let warm: Vec<bool> = warm.data().iter().map(|&b| b != 0).collect();
    let result = solve(&h, f.data(), &a, b.data(), &eq, beq.data(), &warm, &opts)?;
    let flag = result.exitflag();
    let mut values = vec![Value::from(array(
        Matrix::from_column_vec(result.x.len(), 1, result.x)?,
        single,
    )?)];
    if count > 1 {
        values.push(Value::Matrix(Matrix::scalar(flag)));
    }
    if count > 2 {
        values.push(Value::Logical(LogicalMatrix::from_column_vec(
            result.active.len(),
            1,
            result.active.into_iter().map(u8::from).collect(),
            &e.cancel,
        )?));
    }
    if count > 3 {
        values.push(structure(
            e,
            vec![
                (
                    "ineqlin",
                    array(
                        Matrix::from_column_vec(
                            result.lambda.ineqlin.len(),
                            1,
                            result.lambda.ineqlin,
                        )?,
                        single,
                    )?,
                ),
                (
                    "eqlin",
                    array(
                        Matrix::from_column_vec(result.lambda.eqlin.len(), 1, result.lambda.eqlin)?,
                        single,
                    )?,
                ),
            ],
        )?);
    }
    Ok(values)
}
