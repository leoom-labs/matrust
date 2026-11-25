//! Multiresolution-analysis builtins: modwt, imodwt, modwtmra, emd, vmd and
//! ewt. Argument handling shared by the four algorithm files; outputs follow
//! the MATLAB shapes (modwt rows per level, emd/vmd/ewt one column per mode).
use super::super::poly_interp::real_matrix;
use super::super::{Engine, Value};
use crate::{Array, Matrix, Result, StructArray};

pub(super) const NAMES: &[&str] = &["modwt", "imodwt", "modwtmra", "emd", "vmd", "ewt"];

/// Longest signal accepted (keeps the O(N log N) work and N-by-K outputs bounded).
pub(super) const MAX_SAMPLES: usize = 1 << 24;

fn max_outputs(name: &str) -> usize {
    match name {
        "emd" | "vmd" => 3,
        "ewt" => 2,
        _ => 1,
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    if nargout > max_outputs(name) {
        if name == "ewt" {
            return Err("ewt: the wfb and info outputs are not supported yet".into());
        }
        return Err(format!("{name}: too many output arguments"));
    }
    match name {
        "modwt" => super::modwt::modwt_builtin(e, args),
        "imodwt" => super::modwt::imodwt_builtin(e, args),
        "modwtmra" => super::modwt::modwtmra_builtin(e, args),
        "emd" => super::emd::emd_builtin(e, args, nargout),
        "vmd" => super::vmd::vmd_builtin(e, args, nargout),
        "ewt" => super::ewt::ewt_builtin(e, args),
        _ => Err(format!("Undefined function '{name}'")),
    }
}

pub(super) fn mra_text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

/// A real finite nonempty vector (row or column); `min` samples at least.
pub(super) fn mra_signal(v: &Value, who: &str, min: usize) -> Result<Vec<f64>> {
    let m = real_matrix(v, who)?;
    if m.is_nd() || (m.rows() > 1 && m.cols() > 1) {
        return Err(format!("{who}: X must be a real vector"));
    }
    if m.len() < min {
        return Err(format!("{who}: X must have at least {min} samples"));
    }
    if m.len() > MAX_SAMPLES {
        return Err(format!("{who}: X has more than {MAX_SAMPLES} samples"));
    }
    if m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("{who}: X must contain finite values"));
    }
    Ok(m.data().to_vec())
}

pub(super) fn mra_scalar(v: &Value, who: &str, what: &str) -> Result<f64> {
    let m = real_matrix(v, who).map_err(|_| format!("{who}: {what} must be a real scalar"))?;
    match m.data() {
        [x] if x.is_finite() => Ok(*x),
        _ => Err(format!("{who}: {what} must be a real finite scalar")),
    }
}

/// A positive integer no larger than `max`.
pub(super) fn mra_count(v: &Value, who: &str, what: &str, max: usize) -> Result<usize> {
    let x = mra_scalar(v, who, what)?;
    if x < 1. || x.fract() != 0. || x > max as f64 {
        return Err(format!(
            "{who}: {what} must be a positive integer (at most {max})"
        ));
    }
    Ok(x as usize)
}

pub(super) fn mra_bool(v: &Value, who: &str, what: &str) -> Result<bool> {
    match v {
        Value::Logical(l) if l.len() == 1 => Ok(l.data()[0] != 0),
        _ => {
            let x = mra_scalar(v, who, what)?;
            if x == 0. || x == 1. {
                Ok(x == 1.)
            } else {
                Err(format!("{who}: {what} must be true or false"))
            }
        }
    }
}

/// Name-value pairs: each name matched case-insensitively as a unique prefix
/// of one of `names` (MATLAB's partial matching). Returns (index, value).
pub(super) fn mra_options<'a>(
    rest: &'a [Value],
    names: &[&str],
    who: &str,
) -> Result<Vec<(usize, &'a Value)>> {
    if rest.len() % 2 == 1 {
        return Err(format!("{who}: name-value arguments must come in pairs"));
    }
    let mut out = Vec::new();
    for pair in rest.chunks(2) {
        let key = mra_text(&pair[0])
            .ok_or_else(|| format!("{who}: option names must be character vectors"))?;
        let lower = key.to_ascii_lowercase();
        let exact = names.iter().position(|n| n.eq_ignore_ascii_case(key));
        let hits: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, n)| !lower.is_empty() && n.to_ascii_lowercase().starts_with(&lower))
            .map(|(i, _)| i)
            .collect();
        let i = match (exact, hits.as_slice()) {
            (Some(i), _) => i,
            (None, [i]) => *i,
            (None, []) => return Err(format!("{who}: unknown option '{key}'")),
            _ => return Err(format!("{who}: ambiguous option name '{key}'")),
        };
        out.push((i, &pair[1]));
    }
    Ok(out)
}

/// Admit `rows * cols` real elements before a user-sized allocation.
/// `Matrix::checked_len` is the dense element limit (checked multiply);
/// `preflight_elements` is the statement memory and evaluation budget.
pub(super) fn mra_admit(e: &Engine, who: &str, rows: usize, cols: usize) -> Result<()> {
    let n = Matrix::checked_len(rows, cols).map_err(|err| format!("{who}: {err}"))?;
    e.preflight_elements(n)
        .map_err(|err| format!("{who}: {err}"))
}

/// Admit `rows * cols` complex samples (two 8-byte units each) before allocation.
pub(super) fn mra_admit_complex(e: &Engine, who: &str, rows: usize, cols: usize) -> Result<()> {
    let n = rows.checked_mul(cols).ok_or("Matrix dimensions overflow")?;
    let units = n.checked_mul(2).ok_or("Matrix dimensions overflow")?;
    if rows > crate::matrix::MAX_ELEMENTS
        || cols > crate::matrix::MAX_ELEMENTS
        || units > crate::matrix::MAX_ELEMENTS
    {
        return Err(format!(
            "{who}: Matrix exceeds the {}-element limit",
            crate::matrix::MAX_ELEMENTS
        ));
    }
    e.preflight_elements(units)
        .map_err(|err| format!("{who}: {err}"))
}

pub(super) fn mra_matrix(rows: usize, cols: usize, column_major: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        rows,
        cols,
        column_major,
    )?))
}

pub(super) fn mra_column(v: Vec<f64>) -> Result<Value> {
    let n = v.len();
    mra_matrix(n, 1, v)
}

/// N-by-K matrix from K columns of length N.
pub(super) fn mra_columns(n: usize, cols: &[Vec<f64>]) -> Result<Value> {
    let len = Matrix::checked_len(n, cols.len())?;
    let mut data = Vec::with_capacity(len);
    for c in cols {
        data.extend_from_slice(&c[..n]);
    }
    mra_matrix(n, cols.len(), data)
}

pub(super) fn mra_struct(e: &Engine, fields: Vec<(&str, Value)>) -> Result<Value> {
    let mut s = StructArray::scalar_empty(&e.cancel)?;
    for (n, v) in fields {
        s = s.with_field(n, v.array(&e.cancel)?, &e.cancel)?;
    }
    Ok(Value::Container(Array::Struct(s)))
}

/// In-place complex FFT (forward or inverse, unnormalised) of `buf`.
pub(super) fn mra_fft(buf: &mut [rustfft::num_complex::Complex<f64>], inverse: bool) {
    let mut planner = rustfft::FftPlanner::<f64>::new();
    let plan = if inverse {
        planner.plan_fft_inverse(buf.len())
    } else {
        planner.plan_fft_forward(buf.len())
    };
    plan.process(buf);
}
