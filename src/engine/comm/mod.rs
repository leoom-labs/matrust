//! Communications core. Public specifications: MathWorks Communications Toolbox
//! reference pages; theory: Proakis and Simon & Alouini. Independent fixtures:
//! tool/refs/comm/core.py. Serial, chunk-cancellable kernels share session storage.
mod bits;
mod errors;
mod modem;
mod noise;
mod theory;
use super::{Engine, Execution, Value};
use crate::{
    Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix, NumericClass, Result, TypedArray,
};
use std::collections::BTreeMap;
pub(super) const NAMES: &[&str] = &[
    "int2bit",
    "bit2int",
    "de2bi",
    "bi2de",
    "pskmod",
    "pskdemod",
    "qammod",
    "qamdemod",
    "pammod",
    "pamdemod",
    "genqammod",
    "genqamdemod",
    "awgn",
    "wgn",
    "biterr",
    "symerr",
    "berawgn",
    "qfunc",
    "qfuncinv",
];
pub(super) fn handles(n: &str) -> bool {
    NAMES.contains(&n)
}
pub(super) fn call(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let result = call_inner(e, n, a, nout, out);
    result.map_err(|message| {
        if let Some((id, _)) = message.split_once(": ")
            && id.starts_with("comm:")
        {
            let id = id.to_owned();
            e.raise(message, id)
        } else {
            message
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
        "biterr" | "symerr" => 3,
        "awgn" | "berawgn" => 2,
        _ => 1,
    };
    if nout > max {
        return Err(err(n, "TooManyOutputs", "Too many output arguments."));
    }
    let v = match n {
        "int2bit" | "bit2int" | "de2bi" | "bi2de" => vec![bits::call(e, n, a)?],
        "awgn" | "wgn" => noise::call(e, n, a)?,
        "biterr" | "symerr" => errors::call(e, n, a)?,
        "qfunc" | "qfuncinv" | "berawgn" => theory::call(e, n, a)?,
        _ => vec![modem::call(e, n, a, out)?],
    };
    e.charge_function_outputs(&v)?;
    Ok(v.into_iter().take(nout.max(1)).collect())
}
fn err(n: &str, id: &str, msg: &str) -> String {
    format!("comm:{n}:{id}: {msg}")
}
fn arity(n: &str, a: &[Value], lo: usize, hi: usize) -> Result<()> {
    if a.len() < lo || a.len() > hi {
        Err(err(
            n,
            "InvalidNumInputs",
            "Incorrect number of input arguments.",
        ))
    } else {
        Ok(())
    }
}
fn array(e: &Engine, v: &Value) -> Result<Array> {
    v.clone().array(&e.cancel)
}
fn text(e: &Engine, v: &Value) -> Result<String> {
    super::table_fns::text_scalar(v)
        .or_else(|| {
            array(e, v)
                .ok()
                .and_then(|a| super::table_fns::text_scalar(&Value::from(a)))
        })
        .ok_or_else(|| "Expected scalar text".into())
}
fn real(a: &Array, i: usize) -> Result<f64> {
    match a {
        Array::Numeric(m) => Ok(m.data()[i]),
        Array::Logical(m) => Ok(f64::from(m.data()[i])),
        Array::Typed(m) => Ok(m.real(i)),
        _ => Err("Expected a real numeric array".into()),
    }
}
fn integer(a: &Array, i: usize, n: &str, negative: bool) -> Result<i128> {
    let v = match a {
        Array::Typed(m) if m.is_integer() => m.int(i),
        _ => {
            let v = real(a, i)?;
            if !v.is_finite() || v.fract() != 0. || v.abs() > 9_007_199_254_740_992. {
                return Err(err(
                    n,
                    "InvalidInput",
                    "Input must contain finite integers (floating values at most flintmax).",
                ));
            }
            v as i128
        }
    };
    if !negative && v < 0 {
        return Err(err(
            n,
            "InvalidInput",
            "Input must contain nonnegative integers.",
        ));
    }
    Ok(v)
}
fn scalar(e: &Engine, v: &Value) -> Result<f64> {
    let a = array(e, v)?;
    if a.len() != 1 {
        return Err("Expected a real numeric scalar".into());
    }
    real(&a, 0)
}
fn count(e: &Engine, v: &Value, n: &str, lo: usize, hi: usize) -> Result<usize> {
    let x = scalar(e, v)?;
    if !x.is_finite() || x.fract() != 0. || x < lo as f64 || x > hi as f64 {
        return Err(err(
            n,
            "InvalidInteger",
            "Expected an integer within the supported bounds.",
        ));
    }
    Ok(x as usize)
}
fn boolean(e: &Engine, v: &Value) -> Result<bool> {
    match scalar(e, v)? {
        0. => Ok(false),
        1. => Ok(true),
        _ => Err("Expected a logical scalar (0 or 1)".into()),
    }
}
fn complex(a: &Array, i: usize) -> Result<Complex64> {
    match a {
        Array::Complex(m) => Ok(m.data()[i]),
        _ => Ok(Complex64::new(real(a, i)?, 0.)),
    }
}
fn is_single(a: &Array) -> bool {
    matches!(a,Array::Typed(t) if t.class()==NumericClass::Single)
}
fn numeric(e: &Engine, v: &Value, n: &str) -> Result<Array> {
    let a = array(e, v)?;
    if !matches!(
        a,
        Array::Numeric(_) | Array::Complex(_) | Array::Logical(_) | Array::Typed(_)
    ) {
        return Err(err(n, "InvalidInput", "Expected a numeric array."));
    }
    Ok(a)
}
fn output(dims: &[usize], data: Vec<f64>, class: &str) -> Result<Value> {
    let cols = dims[1..].iter().product();
    if class == "logical" {
        return Ok(Value::Logical(
            LogicalMatrix::from_column_vec(
                dims[0],
                cols,
                data.into_iter().map(|v| u8::from(v != 0.)).collect(),
                &std::sync::atomic::AtomicBool::new(false),
            )?
            .with_dims(dims)?,
        ));
    }
    let m = Matrix::from_column_vec(dims[0], cols, data)?.with_dims(dims)?;
    if class == "double" {
        Ok(Value::Matrix(m))
    } else {
        let c = NumericClass::parse(class).ok_or("Unsupported OutputDataType")?;
        Ok(Value::from(Array::Typed(
            TypedArray::from_matrix(c, &m)?.with_dims(dims)?,
        )))
    }
}
fn complex_output(e: &Engine, dims: &[usize], data: Vec<Complex64>) -> Result<Value> {
    Ok(Value::Complex(
        ComplexMatrix::from_column_vec(dims[0], dims[1..].iter().product(), data, &e.cancel)?
            .with_dims(dims)?,
    ))
}
fn options(e: &Engine, n: &str, a: &[Value], allowed: &[&str]) -> Result<BTreeMap<String, Value>> {
    if !a.len().is_multiple_of(2) {
        return Err(err(n, "InvalidNameValue", "Expected Name,Value pairs."));
    }
    let mut o = BTreeMap::new();
    for p in a.as_chunks::<2>().0 {
        let k = text(e, &p[0])?.to_ascii_lowercase();
        if !allowed.contains(&k.as_str()) {
            return Err(err(
                n,
                "InvalidParameter",
                &format!("Unrecognized parameter name '{k}'."),
            ));
        }
        if o.insert(k.clone(), p[1].clone()).is_some() {
            return Err(err(
                n,
                "InvalidParameter",
                &format!("Duplicate parameter '{k}'."),
            ));
        }
    }
    Ok(o)
}
fn choose(e: &Engine, n: &str, v: &Value, allowed: &[&str]) -> Result<String> {
    let s = text(e, v)?.to_ascii_lowercase();
    if !allowed.contains(&s.as_str()) {
        return Err(err(n, "InvalidOption", &format!("Unknown option '{s}'.")));
    }
    Ok(s)
}
fn preflight(e: &Engine, dims: &[usize], factor: usize) -> Result<usize> {
    let cols = dims[1..].iter().try_fold(1usize, |a, &b| {
        a.checked_mul(b).ok_or("Dimensions overflow")
    })?;
    let len = Matrix::checked_len(dims[0], cols)?;
    e.preflight_elements(len.saturating_mul(factor))?;
    Ok(len)
}
fn tick(e: &Engine, i: usize) -> Result<()> {
    if i.is_multiple_of(4096) {
        e.check_cancel()?;
    }
    Ok(())
}
