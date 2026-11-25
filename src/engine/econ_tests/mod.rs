//! Econometric diagnostics, independent of model objects and the desktop.
//! Contracts: MathWorks public function reference pages (links in help).
//! Mathematics: Box-Jenkins, Hamilton, Ljung-Box, Engle, DF, PP and KPSS.
mod correlogram;
mod lagmatrix;
mod portmanteau;
mod regression;
mod tables;
mod unitroot;
use super::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, Result};
use std::collections::BTreeMap;

pub(super) const NAMES: &[&str] = &[
    "autocorr",
    "parcorr",
    "crosscorr",
    "lbqtest",
    "archtest",
    "adftest",
    "kpsstest",
    "pptest",
    "lagmatrix",
];
const MAX_OBS: usize = 1_000_000;
const MAX_LAGS: usize = 2048;
const MAX_WORK: usize = 100_000_000;
const MAX_DESIGN: usize = 2_000_000;
pub(super) fn handles(n: &str) -> bool {
    NAMES.contains(&n)
}
pub(super) fn is_correlogram(n: &str) -> bool {
    matches!(n, "autocorr" | "parcorr" | "crosscorr")
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
            && id.starts_with("econ:")
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
        "lagmatrix" => 2,
        "autocorr" | "parcorr" | "crosscorr" | "lbqtest" | "archtest" => 4,
        _ => 5,
    };
    if nout > max {
        return Err(err(n, "TooManyOutputs", "Too many output arguments."));
    }
    let mut v = match n {
        "autocorr" | "parcorr" | "crosscorr" => correlogram::call(e, n, a, nout, out)?,
        "lagmatrix" => lagmatrix::call(e, a)?,
        "lbqtest" | "archtest" => portmanteau::call(e, n, a, out)?,
        _ => unitroot::call(e, n, a, nout, out)?,
    };
    v.truncate(nout.max(1));
    e.charge_function_outputs(&v)?;
    Ok(v)
}
fn err(n: &str, id: &str, s: &str) -> String {
    format!("econ:{n}:{id}: {s}")
}
fn real(e: &Engine, n: &str, v: &Value) -> Result<Matrix> {
    real_inner(e, n, v, false)
}
fn real_allow_inf(e: &Engine, n: &str, v: &Value) -> Result<Matrix> {
    real_inner(e, n, v, true)
}
fn real_inner(e: &Engine, n: &str, v: &Value, allow_inf: bool) -> Result<Matrix> {
    if v.dims().iter().copied().fold(1usize, usize::saturating_mul) > MAX_OBS {
        return Err(err(
            n,
            "InvalidData",
            "Data must have at most 1000000 elements.",
        ));
    }
    if v.is_nd() || v.is_character() || matches!(v, Value::Text(_) | Value::QuotedString(_)) {
        return Err(err(
            n,
            "InvalidData",
            "Expected a real numeric 2-D array; tables and timetables are not supported.",
        ));
    }
    let x = v.clone().matrix(&e.cancel).map_err(|_| {
        err(
            n,
            "InvalidData",
            "Expected a real numeric array; tables and timetables are not supported.",
        )
    })?;
    if x.len() > MAX_OBS || (!allow_inf && x.data().iter().any(|v| v.is_infinite())) {
        return Err(err(
            n,
            "InvalidData",
            if allow_inf {
                "Data must have at most 1000000 elements."
            } else {
                "Data must have at most 1000000 elements and contain no Inf values."
            },
        ));
    }
    Ok(x)
}
fn vector(e: &Engine, n: &str, v: &Value) -> Result<Matrix> {
    let x = real(e, n, v)?;
    if x.is_empty() || (x.rows() != 1 && x.cols() != 1) {
        return Err(err(
            n,
            "InvalidData",
            "Input must be a nonempty numeric vector.",
        ));
    }
    Ok(x)
}
fn text(v: &Value) -> Option<String> {
    super::table_fns::text_scalar(v)
}
fn value(rows: usize, cols: usize, d: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, d)?))
}
fn scalar(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}
fn shape(row: bool, len: usize) -> (usize, usize) {
    if row { (1, len) } else { (len, 1) }
}
fn vecval(row: bool, d: Vec<f64>) -> Result<Value> {
    let (r, c) = shape(row, d.len());
    value(r, c, d)
}
fn integer(n: &str, x: f64, min: usize, max: usize) -> Result<usize> {
    if !x.is_finite() || x.fract() != 0. || x < min as f64 || x > max as f64 {
        Err(err(
            n,
            "InvalidLags",
            "Lags must be bounded integers compatible with the sample size.",
        ))
    } else {
        Ok(x as usize)
    }
}
fn work(e: &mut Engine, n: &str, size: usize) -> Result<()> {
    if size > MAX_WORK {
        return Err(err(
            n,
            "WorkLimit",
            "Requested calculation exceeds 100000000 work units.",
        ));
    }
    e.charge(size).map_err(|_| {
        err(
            n,
            "WorkLimit",
            "Requested calculation exceeds the host evaluation budget.",
        )
    })?;
    e.check_cancel()
}
fn reserve(e: &Engine, n: &str, units: usize) -> Result<()> {
    if units.saturating_mul(8).saturating_add(e.workspace_bytes()) > e.limits.memory_bytes {
        return Err(err(
            n,
            "WorkspaceLimit",
            "Scratch and output exceed the remaining workspace memory budget.",
        ));
    }
    e.preflight_elements(units)?;
    Ok(())
}
fn design(e: &Engine, n: &str, rows: usize, cols: usize) -> Result<()> {
    let count = rows.saturating_mul(cols);
    if count > MAX_DESIGN {
        return Err(err(
            n,
            "WorkspaceLimit",
            "Regression design exceeds 2000000 elements.",
        ));
    }
    reserve(
        e,
        n,
        count
            .saturating_mul(6)
            .saturating_add(rows.saturating_mul(8))
            .saturating_add(cols.saturating_mul(cols).saturating_mul(6)),
    )
}
fn warning(e: &mut Engine, n: &str, id: &str, msg: &str, out: &mut Execution) -> Result<()> {
    e.lang_utils_call(
        "warning",
        &[
            Value::Text(format!("econ:{n}:{id}")),
            Value::Text(msg.into()),
        ],
        out,
    )?;
    Ok(())
}
struct Options<'a> {
    values: BTreeMap<String, &'a Value>,
    row: bool,
    count: usize,
}
impl<'a> Options<'a> {
    fn new(n: &str, a: &'a [Value], allowed: &[&str]) -> Result<Self> {
        if !a.len().is_multiple_of(2) {
            return Err(err(
                n,
                "InvalidOptions",
                "Options must be name/value pairs.",
            ));
        }
        let mut values = BTreeMap::new();
        for pair in a.as_chunks::<2>().0 {
            let key = text(&pair[0])
                .ok_or_else(|| err(n, "InvalidOptions", "Option names must be scalar text."))?
                .to_ascii_lowercase();
            if !allowed.contains(&key.as_str()) {
                return Err(err(n, "UnknownOption", &format!("Unknown option '{key}'.")));
            }
            if values.insert(key, &pair[1]).is_some() {
                return Err(err(n, "InvalidOptions", "Duplicate option name."));
            }
        }
        Ok(Self {
            values,
            row: false,
            count: 1,
        })
    }
    fn nums(&mut self, e: &Engine, n: &str, key: &str, default: f64) -> Result<Vec<f64>> {
        let Some(v) = self.values.get(key) else {
            return Ok(vec![default]);
        };
        let x = vector(e, n, v)?;
        self.dim(n, x.len(), x.rows() == 1)?;
        Ok(x.data().to_vec())
    }
    fn strings(&mut self, e: &Engine, n: &str, key: &str, default: &str) -> Result<Vec<String>> {
        let Some(v) = self.values.get(key) else {
            return Ok(vec![default.into()]);
        };
        if let Some(s) = text(v) {
            return Ok(vec![s]);
        }
        let a = (*v).clone().array(&e.cancel)?;
        if a.rows() != 1 && a.cols() != 1 {
            return Err(err(n, "InvalidOptions", "Option values must be vectors."));
        }
        let strings = match &a {
            Array::Str(s) => s
                .data()
                .iter()
                .map(|s| {
                    s.as_ref()
                        .map(|s| s.to_string())
                        .ok_or_else(|| err(n, "InvalidOptions", "Missing option value."))
                })
                .collect::<Result<Vec<_>>>()?,
            Array::Cell(c) => (0..c.len())
                .map(|i| {
                    super::table_fns::text_scalar(&Value::from(c.get_linear(i)?.as_ref().clone()))
                        .ok_or_else(|| {
                            err(n, "InvalidOptions", "Expected scalar text in cell vector.")
                        })
                })
                .collect::<Result<Vec<_>>>()?,
            _ => {
                return Err(err(
                    n,
                    "InvalidOptions",
                    "Expected a string or cell vector of names.",
                ));
            }
        };
        self.dim(n, strings.len(), a.rows() == 1)?;
        Ok(strings)
    }
    fn dim(&mut self, n: &str, len: usize, row: bool) -> Result<()> {
        if len == 0 || len > 1024 || (len > 1 && self.count > 1 && len != self.count) {
            return Err(err(
                n,
                "InconsistentOptions",
                "Vector options must have equal lengths (at most 1024 tests).",
            ));
        }
        if len > 1 {
            self.count = len;
            self.row |= row;
        }
        Ok(())
    }
}
fn at<T>(v: &[T], i: usize) -> &T {
    &v[if v.len() == 1 { 0 } else { i }]
}
fn test_values(
    row: bool,
    h: Vec<bool>,
    p: Vec<f64>,
    s: Vec<f64>,
    c: Vec<f64>,
) -> Result<Vec<Value>> {
    let (r, k) = shape(row, h.len());
    Ok(vec![
        Value::Logical(LogicalMatrix::from_canonical_vec(
            r,
            k,
            h.into_iter().map(u8::from).collect(),
        )?),
        vecval(row, p)?,
        vecval(row, s)?,
        vecval(row, c)?,
    ])
}
