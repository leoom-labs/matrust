//! Argument handling for firls, kaiserord, firpmord, phasedelay, iirnotch and iirpeak.
use super::{Engine, Value};
use crate::{
    Array, CellArray, CharacterRow, Matrix, Result,
    dsp::{self, FirlsKind},
    plot::Figure,
};
use std::sync::Arc;

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "firls" | "kaiserord" | "firpmord" | "phasedelay" | "iirnotch" | "iirpeak"
    )
}
fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}
fn vector<'a>(v: &'a Value, what: &str) -> Result<&'a [f64]> {
    match v {
        Value::Matrix(m) if m.rows() == 1 || m.cols() == 1 || m.is_empty() => Ok(m.data()),
        _ => Err(format!("{what} must be a real vector")),
    }
}
fn scalar(v: &Value, what: &str) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Ok(m.data()[0]),
        _ => Err(format!("{what} must be a real scalar")),
    }
}
fn count_arg(v: &Value, name: &str) -> Result<usize> {
    let n = scalar(v, "The point count")?;
    if !n.is_finite() || n < 1. || n.fract() != 0. || n > 65536. {
        return Err(format!(
            "{name}: the point count must be an integer in 1..65536"
        ));
    }
    Ok(n as usize)
}
fn row(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}
fn column(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(v.len(), 1, v)?))
}
fn num(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}
fn cell(slots: Vec<Array>, e: &Engine) -> Result<Value> {
    let n = slots.len();
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        1,
        n,
        slots.into_iter().map(Arc::new).collect(),
        &e.cancel,
    )?)))
}
fn numeric(v: Vec<f64>) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_vec(1, v.len(), v)?))
}

pub(super) fn call(e: &mut Engine, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let max = match name {
        "kaiserord" | "firpmord" => 4,
        "phasedelay" | "iirnotch" | "iirpeak" => 2,
        _ => 1,
    };
    if count > max || (count == 0 && name != "phasedelay") {
        return Err(format!("{name} supports 1..{max} outputs"));
    }
    let mut out = match name {
        "firls" => firls(e, args)?,
        "kaiserord" | "firpmord" => order(e, name, args, count)?,
        "phasedelay" => return phasedelay(e, args, count),
        _ => notch_peak(name, args)?,
    };
    out.truncate(count.max(1));
    e.charge_function_outputs(&out)?;
    e.check_cancel()?;
    Ok(out)
}

fn firls(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    if !(3..=5).contains(&args.len()) {
        return Err(
            "firls expects (n, f, a), (n, f, a, w) and an optional 'hilbert' or 'differentiator'"
                .into(),
        );
    }
    let n = scalar(&args[0], "The filter order")?;
    if !n.is_finite() || n < 1. || n.fract() != 0. {
        return Err("firls: the filter order must be a positive integer".into());
    }
    let mut rest = &args[3..];
    let mut kind = FirlsKind::Symmetric;
    if let Some(t) = rest.last().and_then(text) {
        kind = match t.to_ascii_lowercase().as_str() {
            "hilbert" => FirlsKind::Hilbert,
            "differentiator" => FirlsKind::Differentiator,
            "h" | "d" | "" => {
                return Err("firls: type must be 'hilbert' or 'differentiator'".into());
            }
            _ => return Err("firls: type must be 'hilbert' or 'differentiator'".into()),
        };
        rest = &rest[..rest.len() - 1];
    }
    let w = match rest {
        [] => None,
        [w] => Some(vector(w, "W")?),
        _ => return Err("firls: too many arguments".into()),
    };
    let h = dsp::firls(
        n as usize,
        vector(&args[1], "F")?,
        vector(&args[2], "A")?,
        w,
        kind,
        &e.cancel,
    )?;
    Ok(vec![row(h)?])
}

fn order(e: &Engine, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let mut args = args;
    let mut as_cell = false;
    if let Some(t) = args.last().and_then(text) {
        if !t.eq_ignore_ascii_case("cell") {
            return Err(format!("{name}: the only option is 'cell'"));
        }
        as_cell = true;
        args = &args[..args.len() - 1];
    }
    if !(3..=4).contains(&args.len()) {
        return Err(format!(
            "{name} expects (f, a, dev), (f, a, dev, fs) and an optional 'cell'"
        ));
    }
    let fs = match args.get(3) {
        Some(v) => scalar(v, "FS")?,
        None => 2.,
    };
    let (f, a, dev) = (
        vector(&args[0], "F")?,
        vector(&args[1], "A")?,
        vector(&args[2], "DEV")?,
    );
    if name == "kaiserord" {
        let k = dsp::kaiserord(f, a, dev, fs)?;
        let window = dsp::kaiser(k.n + 1, k.beta, &e.cancel)?;
        if as_cell {
            let c = CharacterRow::from_text;
            return Ok(vec![cell(
                vec![
                    Array::Numeric(Matrix::scalar(k.n as f64)),
                    numeric(k.wn)?,
                    Array::Character(c(k.ftype, &e.cancel)?),
                    Array::Numeric(window),
                    Array::Character(c("noscale", &e.cancel)?),
                ],
                e,
            )?]);
        }
        let wn = if k.wn.len() == 1 {
            num(k.wn[0])
        } else {
            row(k.wn)?
        };
        let mut v = vec![
            num(k.n as f64),
            wn,
            num(k.beta),
            Value::Text(k.ftype.into()),
        ];
        v.truncate(count.max(1));
        Ok(v)
    } else {
        let r = dsp::firpmord(f, a, dev, fs)?;
        if as_cell {
            return Ok(vec![cell(
                vec![
                    Array::Numeric(Matrix::scalar(r.n as f64)),
                    numeric(r.fo)?,
                    numeric(r.ao)?,
                    numeric(r.w)?,
                ],
                e,
            )?]);
        }
        Ok(vec![num(r.n as f64), row(r.fo)?, row(r.ao)?, row(r.w)?])
    }
}

fn phasedelay(e: &mut Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let first = match args.first() {
        Some(Value::Matrix(m)) => m,
        _ => return Err("phasedelay expects numeric (b, a[, n[, fs]]) or (sos[, n[, fs]])".into()),
    };
    // A 6-column matrix is an SOS; a single row is ambiguous with `b` only when a0 == 1 and just n follows.
    let sos = first.cols() == 6
        && (first.rows() > 1 || (first.rows() == 1 && first.data()[3] == 1. && args.len() == 2));
    let (shift, max_args) = if sos { (1, 3) } else { (2, 4) };
    if args.len() < shift || args.len() > max_args {
        return Err("phasedelay expects (b, a[, n[, fs]]) or (sos[, n[, fs]])".into());
    }
    let n = match args.get(shift) {
        Some(v) => count_arg(v, "phasedelay")?,
        // Same default as freqz, grpdelay and phasez (filter_analysis::DEFAULT_POINTS).
        None => 512,
    };
    let fs = match args.get(shift + 1) {
        Some(v) => {
            let fs = scalar(v, "FS")?;
            if !fs.is_finite() || fs <= 0. {
                return Err("phasedelay: the sampling frequency must be positive".into());
            }
            Some(fs)
        }
        None => None,
    };
    let phi = if sos {
        dsp::phasedelay_sos(first, n, &e.cancel)?
    } else {
        dsp::phasedelay(vector(&args[0], "B")?, vector(&args[1], "A")?, n, &e.cancel)?
    };
    let w: Vec<f64> = (0..n)
        .map(|i| i as f64 / n as f64 * fs.map_or(std::f64::consts::PI, |f| f / 2.))
        .collect();
    if count == 0 {
        let x: Vec<f64> = w
            .iter()
            .map(|v| {
                if fs.is_some() {
                    *v
                } else {
                    v / std::f64::consts::PI
                }
            })
            .collect();
        let mut figure = Figure::new(
            &Matrix::from_column_slice(n, 1, &x)?,
            &Matrix::from_column_slice(n, 1, &phi)?,
        )?;
        figure.title = "Phase delay".into();
        figure.xlabel = if fs.is_some() {
            "Frequency (Hz)"
        } else {
            "Normalized frequency (Nyquist = 1)"
        }
        .into();
        figure.ylabel = "Phase delay (samples)".into();
        figure.layout = e.figure.as_ref().and_then(|f| f.layout.clone());
        figure.activate_tile();
        if let Some(previous) = &e.figure {
            figure.identity = previous.identity.clone();
            figure.axes.identity = previous.axes.identity.clone();
        }
        figure.validate()?;
        e.figure = Some(figure);
        return Ok(Vec::new());
    }
    let mut out = vec![column(phi)?, column(w)?];
    out.truncate(count);
    e.charge_function_outputs(&out)?;
    Ok(out)
}

fn notch_peak(name: &str, args: &[Value]) -> Result<Vec<Value>> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!("{name} expects (w0, bw) or (w0, bw, ab)"));
    }
    let ab = match args.get(2) {
        Some(v) => scalar(v, "Ab")?,
        None => -10. * 2f64.log10(),
    };
    let (b, a) = dsp::notch_peak(
        scalar(&args[0], "W0")?,
        scalar(&args[1], "BW")?,
        ab,
        name == "iirpeak",
    )?;
    Ok(vec![row(b)?, row(a)?])
}
