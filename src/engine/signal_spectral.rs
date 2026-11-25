//! Interpreter adapter only; numerical work stays in dsp::spectral.
use super::{Engine, Value};
use crate::{
    Array, Matrix, Result,
    dsp::{
        self,
        spectral::{self, Estimate, Options, Range},
    },
    plot::Figure,
};

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "periodogram"
            | "pwelch"
            | "cpsd"
            | "mscohere"
            | "tfestimate"
            | "spectrogram"
            | "stft"
            | "istft"
    )
}
pub(super) fn plots(name: &str) -> bool {
    handles(name) && name != "istft"
}
fn scalar(v: &Value) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => m.as_scalar(),
        _ => Err("Spectral parameter must be a real scalar".into()),
    }
}
fn integer(v: &Value) -> Result<usize> {
    let n = scalar(v)?;
    if !n.is_finite() || n < 0. || n.fract() != 0. || n > dsp::MAX_FFT_LENGTH as f64 {
        return Err("Spectral lengths must be integers between 0 and 1048576".into());
    }
    Ok(n as usize)
}
fn text(v: &Value) -> Result<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s),
        _ => Err("Spectral option must be text".into()),
    }
}
fn empty(v: &Value) -> bool {
    matches!(v, Value::Matrix(m) if m.is_empty())
}
fn input(v: &Value) -> Result<Array> {
    match v {
        Value::Matrix(m) => Ok(m.clone().into()),
        Value::Complex(m) => Ok(m.clone().into()),
        Value::Logical(m) => Ok(m.clone().into()),
        _ => Err("Spectral input must be a real, logical or complex array".into()),
    }
}
fn window(
    v: Option<&Value>,
    default: usize,
    kind: dsp::WindowKind,
    engine: &Engine,
) -> Result<Vec<f64>> {
    let n = match v.filter(|v| !empty(v)) {
        Some(Value::Matrix(m)) if m.len() > 1 => {
            if m.len() > dsp::MAX_FFT_LENGTH {
                return Err("Spectral window length exceeds 1048576 samples".into());
            }
            engine.check_cancel()?;
            return Ok(dsp::vector(m)?.to_vec());
        }
        Some(v) => integer(v)?,
        None => default,
    };
    if n == 0 || n > dsp::MAX_FFT_LENGTH {
        return Err("Spectral window length must be 1..1048576".into());
    }
    Ok(dsp::window(kind, n, false, &engine.cancel)?.data().to_vec())
}
fn range(v: &str) -> Result<Range> {
    match v.to_ascii_lowercase().as_str() {
        "onesided" => Ok(Range::OneSided),
        "twosided" => Ok(Range::TwoSided),
        "centered" => Ok(Range::Centered),
        _ => Err("Spectral range must be onesided, twosided or centered".into()),
    }
}
fn default_range(x: &Array) -> Range {
    if matches!(x, Array::Complex(_)) {
        Range::TwoSided
    } else {
        Range::OneSided
    }
}
fn optional_integer(args: &[Value], i: usize, default: usize) -> Result<usize> {
    args.get(i)
        .filter(|v| !empty(v))
        .map(integer)
        .unwrap_or(Ok(default))
}
fn positional(
    engine: &Engine,
    name: &str,
    args: &[Value],
    x: &Array,
    y: Option<&Array>,
) -> Result<Options> {
    let start = if y.is_some() { 2 } else { 1 };
    let period = name == "periodogram";
    let spectrogram = name == "spectrogram";
    let rows = if x.rows() == 1 { x.cols() } else { x.rows() };
    let numeric = args[start..]
        .iter()
        .position(|v| matches!(v, Value::Text(_) | Value::QuotedString(_)))
        .map_or(args.len(), |j| j + start);
    let max = start + if period { 3 } else { 4 };
    if numeric > max || (spectrogram && numeric < start + 3) {
        return Err(format!(
            "{name}: expected window, {}nfft, optional fs, then range/scaling",
            if period { "" } else { "noverlap, " }
        ));
    }
    let default = if period { rows } else { (2 * rows / 9).max(1) };
    if default > dsp::MAX_FFT_LENGTH && args.get(start).is_none_or(empty) {
        return Err(
            "Default spectral window exceeds 1048576 samples; specify a shorter window".into(),
        );
    }
    let w = window(
        args.get(start).filter(|_| start < numeric),
        default,
        if period {
            dsp::WindowKind::Rectangular
        } else {
            dsp::WindowKind::Hamming
        },
        engine,
    )?;
    if period && w.len() != rows {
        return Err("periodogram window length must equal x length".into());
    }
    let overlap = if period {
        0
    } else {
        optional_integer(&args[..numeric], start + 1, w.len() / 2)?
    };
    let nindex = start + if period { 1 } else { 2 };
    let nfft = optional_integer(
        &args[..numeric],
        nindex,
        w.len().next_power_of_two().max(256),
    )?;
    let fs = args
        .get(nindex + 1)
        .filter(|_| nindex + 1 < numeric)
        .filter(|v| !empty(v))
        .map(scalar)
        .transpose()?;
    let mut o = Options {
        window: w,
        overlap,
        nfft,
        fs,
        range: default_range(x),
        power: false,
    };
    if y.is_some_and(|y| matches!(y, Array::Complex(_))) {
        o.range = Range::TwoSided;
    }
    let mut seen_range = false;
    let mut seen_scale = false;
    for v in &args[numeric..] {
        let s = text(v)?.to_ascii_lowercase();
        match s.as_str() {
            "psd" | "power" if !seen_scale => {
                o.power = s == "power";
                seen_scale = true;
            }
            "onesided" | "twosided" | "centered" if !seen_range => {
                o.range = range(&s)?;
                seen_range = true;
            }
            _ => {
                return Err(format!(
                    "{name}: unsupported or duplicate spectral option '{s}'"
                ));
            }
        }
    }
    Ok(o)
}
fn named(
    engine: &Engine,
    name: &str,
    args: &[Value],
    x: &Array,
) -> Result<(Options, Option<usize>)> {
    let inverse = name == "istft";
    let mut i = 1;
    let fs = if args
        .get(i)
        .is_some_and(|v| !matches!(v, Value::Text(_) | Value::QuotedString(_)))
    {
        i += 1;
        Some(scalar(&args[i - 1])?)
    } else {
        None
    };
    if !(args.len() - i).is_multiple_of(2) {
        return Err(format!("{name}: expected Name,Value pairs"));
    }
    let mut w = None;
    let mut overlap = None;
    let mut nfft = None;
    let mut r = Range::Centered;
    let mut length = None;
    let mut seen = std::collections::BTreeSet::new();
    while i < args.len() {
        let key = text(&args[i])?.to_ascii_lowercase();
        if !seen.insert(key.clone()) {
            return Err(format!("{name}: duplicate option '{key}'"));
        }
        let value = &args[i + 1];
        match key.as_str() {
            "window" => w = Some(value),
            "overlaplength" => overlap = Some(integer(value)?),
            "fftlength" => nfft = Some(integer(value)?),
            "frequencyrange" => r = range(text(value)?)?,
            "outputlength" if inverse => length = Some(integer(value)?),
            _ => return Err(format!("{name}: unsupported option '{key}'")),
        }
        i += 2;
    }
    let rows = if inverse {
        x.rows()
    } else if x.rows() == 1 {
        x.cols()
    } else {
        x.rows()
    };
    let inferred_nfft = if inverse {
        if r == Range::OneSided {
            (x.rows() - 1) * 2
        } else {
            x.rows()
        }
    } else {
        rows
    };
    let default_window = if inverse {
        nfft.unwrap_or(inferred_nfft)
    } else {
        rows
    };
    let w = window(w, default_window.min(128), dsp::WindowKind::Hann, engine)?;
    let nfft = nfft.unwrap_or(if inverse { inferred_nfft } else { w.len() });
    let overlap = overlap.unwrap_or(3 * w.len() / 4);
    Ok((
        Options {
            window: w,
            overlap,
            nfft,
            fs,
            range: r,
            power: false,
        },
        length,
    ))
}

pub(super) fn call(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    let max = if matches!(name, "spectrogram" | "stft") {
        4
    } else {
        2
    };
    if count > max {
        return Err(format!("{name} supports at most {max} outputs"));
    }
    if count == 0 && matches!(name, "spectrogram" | "stft") {
        return Err(format!(
            "{name} plots need image plots, which are not available yet; request outputs, e.g. [s,f,t] = {name}(...)"
        ));
    }
    if args.is_empty() {
        return Err(format!("{name} requires input samples"));
    }
    let args = engine.text_arguments(args.to_vec())?;
    let x = input(&args[0])?;
    if x.is_empty() {
        return Err("Spectral input must be nonempty".into());
    }
    engine.check_cancel()?;
    let cross = matches!(name, "cpsd" | "mscohere" | "tfestimate");
    let y = if cross {
        Some(input(
            args.get(1)
                .ok_or("Cross spectral estimate requires x and y")?,
        )?)
    } else {
        None
    };
    let available = (32usize * 1024 * 1024).saturating_sub(engine.evaluated_elements) * 8;
    let mut outputs = if matches!(name, "stft" | "istft") {
        let (o, length) = named(engine, name, &args, &x)?;
        if name == "istft" {
            let s = match &x {
                Array::Complex(s) => s.clone(),
                Array::Numeric(s) => crate::ComplexMatrix::from_real(s, &engine.cancel)?,
                _ => return Err("istft requires a real or complex spectrum".into()),
            };
            let (x, t) = spectral::inverse(&s, &o, length, available, &engine.cancel)?;
            vec![x.into(), Value::Matrix(t)]
        } else {
            let s = spectral::short_time(&x, &o, true, count == 4, available, &engine.cancel)?;
            let mut v = vec![
                Value::Complex(s.spectrum),
                Value::Matrix(s.frequencies),
                Value::Matrix(s.times),
            ];
            if let Some(p) = s.power {
                v.push(Value::Matrix(p));
            }
            v
        }
    } else {
        let o = positional(engine, name, &args, &x, y.as_ref())?;
        if name == "spectrogram" {
            let s = spectral::short_time(&x, &o, false, count == 4, available, &engine.cancel)?;
            let mut v = vec![
                Value::Complex(s.spectrum),
                Value::Matrix(s.frequencies),
                Value::Matrix(s.times),
            ];
            if let Some(p) = s.power {
                v.push(Value::Matrix(p));
            }
            v
        } else {
            let kind = match name {
                "periodogram" => Estimate::Periodogram,
                "pwelch" => Estimate::Welch,
                "cpsd" => Estimate::Cross,
                "mscohere" => Estimate::Coherence,
                "tfestimate" => Estimate::Transfer,
                _ => unreachable!(),
            };
            let (p, f) = spectral::estimate(&x, y.as_ref(), kind, &o, available, &engine.cancel)?;
            if count == 0 {
                let (rows, cols) = (p.rows(), p.cols());
                let data = (0..p.len())
                    .map(|i| {
                        let magnitude = match &p {
                            Array::Numeric(a) => a.data()[i],
                            Array::Complex(a) => a.data()[i].norm(),
                            _ => unreachable!(),
                        };
                        if kind == Estimate::Coherence {
                            magnitude
                        } else {
                            (if kind == Estimate::Transfer { 20. } else { 10. })
                                * magnitude.max(1e-30).log10()
                        }
                    })
                    .collect();
                let db = Matrix::from_column_vec(rows, cols, data)?;
                let mut figure = Figure::new(&f, &db)?;
                figure.title = name.into();
                figure.xlabel = if o.fs.is_some() {
                    "Frequency (Hz)"
                } else {
                    "Frequency (rad/sample)"
                }
                .into();
                figure.ylabel = if kind == Estimate::Coherence {
                    "Coherence"
                } else {
                    "Magnitude (dB)"
                }
                .into();
                figure.layout = engine.figure.as_ref().and_then(|f| f.layout.clone());
                figure.activate_tile();
                if let Some(previous) = &engine.figure {
                    figure.identity = previous.identity.clone();
                    figure.axes.identity = previous.axes.identity.clone();
                }
                figure.validate()?;
                engine.figure = Some(figure);
                return Ok(Vec::new());
            }
            vec![p.into(), Value::Matrix(f)]
        }
    };
    outputs.truncate(count);
    engine.charge_function_outputs(&outputs)?;
    engine.check_cancel()?;
    Ok(outputs)
}
