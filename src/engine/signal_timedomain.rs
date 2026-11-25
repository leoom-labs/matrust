//! Argument handling for the independent time-domain DSP module.
use super::{Engine, Value};
use crate::{
    Array, Matrix, Result, StructArray,
    dsp::{Band, timedomain as td},
};
use std::sync::Arc;

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "findpeaks"
            | "eqtflength"
            | "resample"
            | "decimate"
            | "interp"
            | "hilbert"
            | "envelope"
            | "medfilt1"
            | "sgolayfilt"
            | "sgolay"
            | "lowpass"
            | "highpass"
            | "bandpass"
            | "bandstop"
            | "fir2"
            | "firpm"
    )
}
fn matrix(v: &Value) -> Result<&Matrix> {
    match v {
        Value::Matrix(m) => Ok(m),
        _ => Err("Signal arguments must be real double arrays".into()),
    }
}
fn scalar(v: &Value) -> Result<f64> {
    matrix(v)?.as_scalar()
}
fn integer(v: &Value) -> Result<usize> {
    let n = scalar(v)?;
    if !n.is_finite() || n < 0. || n.fract() != 0. || n > 1_048_576. {
        Err("Expected a bounded nonnegative integer".into())
    } else {
        Ok(n as usize)
    }
}
fn text(v: &Value) -> Result<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s),
        _ => Err("Expected a quoted option name".into()),
    }
}
fn is_text(v: &Value) -> bool {
    matches!(v, Value::Text(_) | Value::QuotedString(_))
}
fn vector(v: &Value) -> Result<&[f64]> {
    crate::dsp::vector(matrix(v)?)
}
fn row(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}
fn arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if (min..=max).contains(&args.len()) {
        Ok(())
    } else {
        Err(format!("Expected {min}..{max} arguments"))
    }
}

fn coefficient_length<T: Copy + Default + PartialEq>(e: &Engine, data: &[T]) -> Result<usize> {
    let mut end = data.len();
    for chunk in data.rchunks(8192) {
        e.check_cancel()?;
        if let Some(last) = chunk.iter().rposition(|&v| v != T::default()) {
            return Ok(end - chunk.len() + last + 1);
        }
        end -= chunk.len();
    }
    Ok(0)
}

fn resize_coefficients<T: Copy + Default>(e: &Engine, data: &[T], len: usize) -> Result<Vec<T>> {
    let mut row = Vec::with_capacity(len);
    for chunk in data[..data.len().min(len)].chunks(8192) {
        e.check_cancel()?;
        row.extend_from_slice(chunk);
    }
    while row.len() < len {
        e.check_cancel()?;
        row.resize((row.len() + 8192).min(len), T::default());
    }
    Ok(row)
}

fn eqtflength(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let [num, den] = args else {
        return Err("Use eqtflength(num,den)".into());
    };
    let coefficients = [num, den];
    let vector_shape = |rows, cols| rows == 1 || cols == 1 || (rows == 0 && cols == 0);
    let mut len = 0;
    for value in coefficients {
        let used = match value {
            Value::Matrix(m) if m.ndims() == 2 && vector_shape(m.rows(), m.cols()) => {
                coefficient_length(e, m.data())?
            }
            Value::Complex(m) if m.ndims() == 2 && vector_shape(m.rows(), m.cols()) => {
                coefficient_length(e, m.data())?
            }
            _ => return Err("eqtflength: num and den must be double vectors".into()),
        };
        len = len.max(used);
    }
    let units = coefficients[..count]
        .iter()
        .map(|value| {
            if matches!(value, Value::Complex(_)) {
                2
            } else {
                1
            }
        })
        .sum::<usize>();
    e.preflight_elements(len * units)?;
    coefficients[..count]
        .iter()
        .map(|value| {
            Ok(match value {
                Value::Matrix(m) if m.len() == len => Value::Matrix(m.reshape(1, len)?),
                Value::Matrix(m) => row(resize_coefficients(e, m.data(), len)?)?,
                Value::Complex(m) if m.len() == len => {
                    Value::Complex(m.reshape(1, len, &e.cancel)?)
                }
                Value::Complex(m) => Value::Complex(crate::ComplexMatrix::from_column_vec(
                    1,
                    len,
                    resize_coefficients(e, m.data(), len)?,
                    &e.cancel,
                )?),
                _ => unreachable!(),
            })
        })
        .collect()
}
fn parse_peaks(args: &[Value]) -> Result<(Option<&[f64]>, td::PeakOptions)> {
    arity(args, 1, 22)?;
    let mut i = 1;
    let axis = if args.get(i).is_some_and(|v| !is_text(v)) {
        i += 1;
        Some(vector(&args[1])?)
    } else {
        None
    };
    let mut o = td::PeakOptions::default();
    if !(args.len() - i).is_multiple_of(2) {
        return Err("findpeaks options require Name,Value pairs".into());
    }
    while i < args.len() {
        let v = &args[i + 1];
        match text(&args[i])?.to_ascii_lowercase().as_str() {
            "minpeakheight" => o.min_height = scalar(v)?,
            "minpeakprominence" => o.min_prominence = scalar(v)?,
            "minpeakdistance" => o.min_distance = scalar(v)?,
            "threshold" => o.threshold = scalar(v)?,
            "npeaks" => o.count = integer(v)?,
            "minpeakwidth" => o.min_width = scalar(v)?,
            "maxpeakwidth" => o.max_width = scalar(v)?,
            "sortstr" => {
                o.sort = match text(v)?.to_ascii_lowercase().as_str() {
                    "none" => td::PeakSort::None,
                    "ascend" => td::PeakSort::Ascend,
                    "descend" => td::PeakSort::Descend,
                    _ => return Err("SortStr must be none, ascend or descend".into()),
                }
            }
            "widthreference" => {
                o.half_height = match text(v)?.to_ascii_lowercase().as_str() {
                    "halfprom" => false,
                    "halfheight" => true,
                    _ => return Err("WidthReference must be halfprom or halfheight".into()),
                }
            }
            _ => return Err(format!("Unsupported findpeaks option {}", text(&args[i])?)),
        }
        i += 2;
    }
    Ok((axis, o))
}

pub(super) fn call(e: &mut Engine, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let max = match name {
        "findpeaks" => 4,
        "eqtflength" | "envelope" | "sgolay" | "lowpass" | "highpass" | "bandpass" | "bandstop" => {
            2
        }
        _ => 1,
    };
    if count > max || (count == 0 && name != "findpeaks") {
        return Err(format!("{name} supports 1..{max} outputs"));
    }
    if name == "eqtflength" {
        return eqtflength(e, args, count);
    }
    let c = &e.cancel;
    let mut outputs = match name {
        "findpeaks" => {
            let (axis, o) = parse_peaks(args)?;
            let y = matrix(&args[0])?;
            let p = td::findpeaks(y, axis, &o, c)?;
            if count == 0 {
                let x = match axis {
                    Some(x) => Matrix::from_column_slice(1, x.len(), x)?,
                    None => Matrix::from_column_vec(
                        1,
                        y.len(),
                        (1..=y.len()).map(|j| j as f64).collect(),
                    )?,
                };
                let mut fig = crate::plot::Figure::new(&x, y)?;
                if !p.heights.is_empty() {
                    let px = Matrix::from_column_vec(1, p.locations.len(), p.locations)?;
                    let py = Matrix::from_column_vec(1, p.heights.len(), p.heights)?;
                    let mut markers = crate::plot::series_from_matrices(
                        Some(&px),
                        &py,
                        crate::plot::PlotKind::Line,
                    )?;
                    for s in &mut markers {
                        s.line_spec("ro")?;
                    }
                    fig.add(markers)?;
                }
                fig.title = "Signal peaks".into();
                fig.layout = e.figure.as_ref().and_then(|f| f.layout.clone());
                fig.activate_tile();
                if let Some(previous) = &e.figure {
                    fig.identity = previous.identity.clone();
                    fig.axes.identity = previous.axes.identity.clone();
                }
                fig.validate()?;
                e.check_cancel()?;
                e.figure = Some(fig);
                return Ok(Vec::new());
            }
            vec![
                row(p.heights)?,
                row(p.locations)?,
                row(p.widths)?,
                row(p.prominences)?,
            ]
        }
        "resample" => {
            arity(args, 3, 5)?;
            let n = args.get(3).map(integer).transpose()?.unwrap_or(10);
            let beta = args.get(4).map(scalar).transpose()?.unwrap_or(5.);
            vec![Value::Matrix(td::resample(
                matrix(&args[0])?,
                integer(&args[1])?,
                integer(&args[2])?,
                n,
                beta,
                c,
            )?)]
        }
        "decimate" => {
            arity(args, 2, 4)?;
            let mut n = None;
            let mut fir = false;
            if let Some(v) = args.get(2) {
                let opt = if is_text(v) {
                    if args.len() == 4 {
                        return Err("decimate order must precede the filter type".into());
                    }
                    Some(v)
                } else {
                    n = Some(integer(v)?);
                    args.get(3)
                };
                if let Some(v) = opt {
                    fir = match text(v)?.to_ascii_lowercase().as_str() {
                        "fir" => true,
                        "iir" => false,
                        _ => return Err("decimate type must be fir or iir".into()),
                    };
                }
            }
            vec![Value::Matrix(td::decimate(
                matrix(&args[0])?,
                integer(&args[1])?,
                n,
                fir,
                c,
            )?)]
        }
        "interp" => {
            arity(args, 2, 4)?;
            vec![Value::Matrix(td::interp(
                matrix(&args[0])?,
                integer(&args[1])?,
                args.get(2).map(integer).transpose()?.unwrap_or(4),
                args.get(3).map(scalar).transpose()?.unwrap_or(0.5),
                c,
            )?)]
        }
        "hilbert" => {
            arity(args, 1, 1)?;
            vec![Value::Complex(td::hilbert(matrix(&args[0])?, c)?)]
        }
        "envelope" => {
            arity(args, 1, 3)?;
            let n = args.get(1).map(integer).transpose()?.unwrap_or(3);
            let kind = match args
                .get(2)
                .map(text)
                .transpose()?
                .unwrap_or("analytic")
                .to_ascii_lowercase()
                .as_str()
            {
                "analytic" => td::EnvelopeKind::Analytic,
                "rms" => td::EnvelopeKind::Rms,
                "peak" => td::EnvelopeKind::Peak,
                _ => return Err("Envelope method must be analytic, rms or peak".into()),
            };
            let (up, lo) = td::envelope(matrix(&args[0])?, n, kind, c)?;
            vec![Value::Matrix(up), Value::Matrix(lo)]
        }
        "medfilt1" => {
            arity(args, 1, 3)?;
            let mut n = 3;
            let mut truncate = false;
            if let Some(v) = args.get(1) {
                let opt = if is_text(v) {
                    if args.len() == 3 {
                        return Err("Median window must precede zeropad or truncate".into());
                    }
                    Some(v)
                } else {
                    n = integer(v)?;
                    args.get(2)
                };
                if let Some(v) = opt {
                    truncate = match text(v)?.to_ascii_lowercase().as_str() {
                        "zeropad" => false,
                        "truncate" => true,
                        _ => return Err("medfilt1 padding must be zeropad or truncate".into()),
                    };
                }
            }
            vec![Value::Matrix(td::medfilt1(
                matrix(&args[0])?,
                n,
                truncate,
                c,
            )?)]
        }
        "sgolay" => {
            arity(args, 2, 2)?;
            let (b, g) = td::sgolay(integer(&args[0])?, integer(&args[1])?, c)?;
            vec![Value::Matrix(b), Value::Matrix(g)]
        }
        "sgolayfilt" => {
            arity(args, 3, 3)?;
            vec![Value::Matrix(td::sgolayfilt(
                matrix(&args[0])?,
                integer(&args[1])?,
                integer(&args[2])?,
                c,
            )?)]
        }
        "lowpass" | "highpass" | "bandpass" | "bandstop" => {
            arity(args, 2, 7)?;
            let mut i = 2;
            let fs = if args.get(i).is_some_and(|v| !is_text(v)) {
                i += 1;
                Some(scalar(&args[2])?)
            } else {
                None
            };
            if !(args.len() - i).is_multiple_of(2) {
                return Err("Filter options require Name,Value pairs".into());
            }
            let (mut s, mut a) = (0.85, 60.);
            while i < args.len() {
                match text(&args[i])?.to_ascii_lowercase().as_str() {
                    "steepness" => s = scalar(&args[i + 1])?,
                    "stopbandattenuation" => a = scalar(&args[i + 1])?,
                    _ => return Err("Unsupported FIR filter option".into()),
                }
                i += 2;
            }
            let kind = Band::parse(name)?;
            let (y, h) =
                td::convenience_filter(matrix(&args[0])?, vector(&args[1])?, fs, kind, s, a, c)?;
            let mut values = vec![Value::Matrix(y)];
            if count > 1 {
                let fields = vec!["order".into(), "coefficients".into(), "type".into()];
                let slots = vec![
                    Arc::new(Array::Numeric(Matrix::scalar((h.len() - 1) as f64))),
                    Arc::new(Array::Numeric(Matrix::from_column_vec(1, h.len(), h)?)),
                    Arc::new(Array::Character(crate::CharacterRow::from_text(name, c)?)),
                ];
                values.push(Value::Container(Array::Struct(StructArray::from_slots(
                    1, 1, fields, slots, c,
                )?)));
            }
            values
        }
        "fir2" => {
            arity(args, 3, 6)?;
            let mut mesh = None;
            let mut lap = 0;
            let mut window = None;
            let syntax = "fir2 syntax: fir2(n,f,m[,npt[,lap]][,window]); npt/lap must be integer scalars and the window a final real vector";
            for (i, v) in args.iter().enumerate().skip(3) {
                if i < 5 && matches!(v, Value::Matrix(m) if m.len() == 1) {
                    let number = integer(v).map_err(|_| syntax.to_string())?;
                    if i == 3 {
                        mesh = Some(number);
                    } else {
                        lap = number;
                    }
                } else if i + 1 == args.len() {
                    window = Some(vector(v).map_err(|_| syntax.to_string())?);
                } else {
                    return Err(syntax.into());
                }
            }
            vec![row(td::fir2(
                integer(&args[0])?,
                vector(&args[1])?,
                vector(&args[2])?,
                mesh,
                lap,
                window,
                c,
            )?)?]
        }
        "firpm" => {
            arity(args, 3, 5)?;
            let mut weights = None;
            let mut kind = td::RemezKind::Symmetric;
            let opt = if let Some(v) = args.get(3) {
                if is_text(v) {
                    if args.len() == 5 {
                        return Err("firpm weights must precede type".into());
                    }
                    Some(v)
                } else {
                    weights = Some(vector(v)?);
                    args.get(4)
                }
            } else {
                None
            };
            if let Some(v) = opt {
                kind = match text(v)?.to_ascii_lowercase().as_str() {
                    "hilbert" => td::RemezKind::Hilbert,
                    "differentiator" => td::RemezKind::Differentiator,
                    "bandpass" => td::RemezKind::Symmetric,
                    _ => {
                        return Err("firpm type must be bandpass, hilbert or differentiator".into());
                    }
                };
            }
            vec![row(td::firpm(
                integer(&args[0])?,
                vector(&args[1])?,
                vector(&args[2])?,
                weights,
                kind,
                c,
            )?)?]
        }
        _ => return Err("Unknown time-domain function".into()),
    };
    outputs.truncate(count);
    e.charge_function_outputs(&outputs)?;
    e.check_cancel()?;
    Ok(outputs)
}
