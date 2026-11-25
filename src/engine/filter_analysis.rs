//! MATLAB-compatible filter analysis calls: `freqz`, `grpdelay`, `phasez`, `impz` and
//! `stepz` with one or two outputs (no output plots), and `bilinear`.
//!
//! Filters are `b, a` vectors (a defaults to 1), an L-by-6 SOS matrix with L >= 2 (a
//! single row is a numerator, as in MATLAB), or a digitalFilter (IIR designs stay in
//! sections). Frequency forms: `n` points over [0, pi) (default 512) or [0, 2 pi) with
//! 'whole', or a vector `w` in rad/sample; with a sample rate `fs` the grid and `w` are in
//! Hz. Grid outputs are columns; a frequency vector keeps its orientation. The
//! engine-specific names (`sosfreqz`, `freqzmag`, `sosimpz`, ...) stay in engine.rs.
use super::{Engine, Execution, Value, string_class};
use crate::{
    Complex64 as C, ComplexMatrix, Matrix, Result,
    dsp::{self, Analysis},
    plot::Figure,
    syntax::{Evaluation, Expr},
};
use std::f64::consts::PI;

const MAX_POINTS: usize = 65536;
const DEFAULT_POINTS: usize = 512;
const MAX_ROOTS: usize = 128;

pub(super) fn handles(name: &str) -> bool {
    plots(name) || name == "bilinear"
}
fn plots(name: &str) -> bool {
    matches!(name, "freqz" | "grpdelay" | "phasez" | "impz" | "stepz")
}

/// A call without outputs, e.g. `freqz(b, a)`, plots. Runs before object statements so
/// that `freqz(d, ...)` reaches the digitalFilter form; other object or container first
/// arguments (class methods) are left to the usual dispatch.
pub(super) fn statement(e: &mut Engine, st: &Evaluation, out: &mut Execution) -> Result<bool> {
    if !st.targets.is_empty()
        || st.indexed_target.is_some()
        || st.path_target.is_some()
        || st.list_target.is_some()
    {
        return Ok(false);
    }
    let Expr::Call(name, args) = &st.expr else {
        return Ok(false);
    };
    if !plots(name) || args.is_empty() || e.contains_array(name) {
        return Ok(false);
    }
    if let Some(Expr::Name(first)) = args.first()
        && e.container_variables.contains_key(first)
        && e.filter_var(first).is_none()
    {
        return Ok(false);
    }
    if e.resolve_function(name)?.is_some() {
        return Ok(false);
    }
    let values = string_class::arguments(e, name, args, out)?;
    call(e, name, &values, 0, out)?;
    Ok(true)
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let values = match name {
        "bilinear" => bilinear(e, args, count)?,
        "impz" | "stepz" => time_response(e, name, args, count)?,
        _ => frequency_analysis(e, name, args, count)?,
    };
    e.charge_function_outputs(&values)?;
    e.check_cancel()?;
    let _ = out;
    Ok(values)
}

enum System {
    Tf(Vec<f64>, Vec<f64>),
    Sos(Matrix),
}

fn coefficients(name: &str, v: &Value, what: &str) -> Result<Vec<f64>> {
    match v {
        Value::Matrix(m) if !m.is_empty() && (m.rows() == 1 || m.cols() == 1) => {
            Ok(m.data().to_vec())
        }
        Value::Matrix(_) => Err(format!(
            "{name}: {what} must be a nonempty vector (second-order sections need an L-by-6 matrix with L >= 2)"
        )),
        _ => Err(format!("{name}: {what} must be real double coefficients")),
    }
}

/// The filter and the arguments after it.
fn system<'a>(e: &Engine, name: &str, args: &'a [Value]) -> Result<(System, &'a [Value])> {
    let Some(first) = args.first() else {
        return Err(format!("{name}: not enough input arguments"));
    };
    if let Some(f) = super::objects::filter_value(e, first) {
        let system = match &f.sos {
            Some(sos) => System::Sos(sos.clone()),
            None => System::Tf(f.b.clone(), f.a.clone()),
        };
        return Ok((system, &args[1..]));
    }
    if let Value::Matrix(m) = first
        && m.cols() == 6
        && m.rows() >= 2
    {
        return Ok((System::Sos(m.clone()), &args[1..]));
    }
    let b = coefficients(name, first, "b")?;
    match args.get(1) {
        None => Ok((System::Tf(b, vec![1.]), &args[1..])),
        Some(a) => Ok((System::Tf(b, coefficients(name, a, "a")?), &args[2..])),
    }
}

fn positive_scalar(name: &str, v: &Value, what: &str) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 && m.data()[0].is_finite() && m.data()[0] > 0. => {
            Ok(m.data()[0])
        }
        _ => Err(format!("{name}: {what} must be a positive finite scalar")),
    }
}

fn point_count(name: &str, value: f64) -> Result<usize> {
    if value.fract() != 0. || !(1. ..=MAX_POINTS as f64).contains(&value) {
        return Err(format!("{name}: n must be an integer in 1..{MAX_POINTS}"));
    }
    Ok(value as usize)
}

enum Grid {
    Uniform { n: usize, whole: bool },
    Points(Matrix),
}

/// `[n | w | f | []] ['whole' | 'half'] [fs]` after the filter.
fn frequency_spec(name: &str, rest: &[Value]) -> Result<(Grid, Option<f64>)> {
    let mut grid = None;
    let mut whole = false;
    let mut fs = None;
    for (i, v) in rest.iter().enumerate() {
        match v {
            Value::Text(s) | Value::QuotedString(s) => match s.to_ascii_lowercase().as_str() {
                "whole" => whole = true,
                "half" => whole = false,
                _ => {
                    return Err(format!(
                        "{name}: unknown option '{s}'; use 'whole' or 'half'"
                    ));
                }
            },
            Value::Matrix(m) if i == 0 => {
                grid = Some(if m.is_empty() {
                    None
                } else if m.len() == 1 {
                    Some(Grid::Uniform {
                        n: point_count(name, m.data()[0])?,
                        whole: false,
                    })
                } else if m.rows() == 1 || m.cols() == 1 {
                    if m.len() > MAX_POINTS || m.data().iter().any(|v| !v.is_finite()) {
                        return Err(format!(
                            "{name}: the frequency vector must hold at most {MAX_POINTS} finite values"
                        ));
                    }
                    Some(Grid::Points(m.clone()))
                } else {
                    return Err(format!(
                        "{name}: the frequencies must be a point count n or a vector"
                    ));
                });
            }
            Value::Matrix(_) if fs.is_none() => fs = Some(positive_scalar(name, v, "fs")?),
            _ => {
                return Err(format!(
                    "{name}: expected the filter, then n or a frequency vector, 'whole' and fs"
                ));
            }
        }
    }
    let grid = match grid.flatten() {
        Some(Grid::Points(_)) if whole => {
            return Err(format!(
                "{name}: 'whole' applies to a point count n, not a frequency vector"
            ));
        }
        Some(Grid::Uniform { n, .. }) => Grid::Uniform { n, whole },
        Some(points) => points,
        None => Grid::Uniform {
            n: DEFAULT_POINTS,
            whole,
        },
    };
    Ok((grid, fs))
}

/// Normalized frequencies (Nyquist = 1) to evaluate, the output frequencies and their shape.
fn frequencies(grid: &Grid, fs: Option<f64>) -> (Vec<f64>, Vec<f64>, (usize, usize)) {
    match grid {
        Grid::Uniform { n, whole } => {
            let span = if *whole { 2. } else { 1. };
            let f: Vec<f64> = (0..*n).map(|k| span * k as f64 / *n as f64).collect();
            let w = f
                .iter()
                .map(|v| match fs {
                    Some(fs) => v * fs / 2.,
                    None => PI * v,
                })
                .collect();
            (f, w, (*n, 1))
        }
        Grid::Points(m) => {
            let f = m
                .data()
                .iter()
                .map(|v| match fs {
                    Some(fs) => 2. * v / fs,
                    None => v / PI,
                })
                .collect();
            (f, m.data().to_vec(), (m.rows(), m.cols()))
        }
    }
}

fn response(e: &Engine, system: &System, f: &[f64]) -> Result<Vec<[f64; 2]>> {
    match system {
        System::Tf(b, a) => dsp::response_at(b, a, f, &e.cancel),
        System::Sos(sos) => dsp::sos_response_at(sos, f, &e.cancel),
    }
}

fn frequency_analysis(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    if count > 2 {
        return Err(format!("{name} supports at most two outputs"));
    }
    let (system, rest) = system(e, name, args)?;
    let (grid, fs) = frequency_spec(name, rest)?;
    let (f, w, (rows, cols)) = frequencies(&grid, fs);
    let kind = match name {
        "freqz" => Analysis::Magnitude,
        "phasez" => Analysis::Phase,
        _ => Analysis::GroupDelay,
    };
    let (first, plotted) = match kind {
        Analysis::Magnitude => {
            let h = response(e, &system, &f)?;
            // Magnitude in dB is only needed for the bare-call plot.
            let db = if count == 0 {
                h.iter()
                    .map(|v| 20. * v[0].hypot(v[1]).max(1e-15).log10())
                    .collect()
            } else {
                Vec::new()
            };
            let data = h.iter().map(|v| C::new(v[0], v[1])).collect();
            let value =
                Value::Complex(ComplexMatrix::from_column_vec(rows, cols, data, &e.cancel)?);
            (value, db)
        }
        Analysis::Phase => {
            let phi = dsp::phase_response(&response(e, &system, &f)?);
            (
                Value::Matrix(Matrix::from_column_slice(rows, cols, &phi)?),
                phi,
            )
        }
        _ => {
            let gd = match &system {
                System::Tf(b, a) => dsp::group_delay_at(b, a, &f, &e.cancel)?,
                System::Sos(sos) => dsp::sos_group_delay_at(sos, &f, &e.cancel)?,
            };
            (
                Value::Matrix(Matrix::from_column_slice(rows, cols, &gd)?),
                gd,
            )
        }
    };
    if count == 0 {
        let (x, xlabel) = match fs {
            Some(_) => (w, "Frequency (Hz)"),
            None => (f, "Normalized frequency (Nyquist = 1)"),
        };
        let (title, ylabel) = match kind {
            Analysis::Magnitude => ("Frequency response", "Magnitude (dB)"),
            _ => (kind.label(), kind.label()),
        };
        plot(e, &x, &plotted, [title, xlabel, ylabel])?;
        return Ok(vec![]);
    }
    let mut values = vec![first];
    if count == 2 {
        values.push(Value::Matrix(Matrix::from_column_vec(rows, cols, w)?));
    }
    Ok(values)
}

/// `[n | []] [fs]` after the filter; n defaults to the automatic impulse-response length.
fn time_response(e: &mut Engine, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if count > 2 {
        return Err(format!("{name} supports at most two outputs"));
    }
    let (system, rest) = system(e, name, args)?;
    if rest.len() > 2 {
        return Err(format!("{name}: expected the filter, then n and fs"));
    }
    let n = match rest.first() {
        None => None,
        Some(Value::Matrix(m)) if m.is_empty() => None,
        Some(Value::Matrix(m)) if m.len() == 1 => Some(point_count(name, m.data()[0])?),
        Some(Value::Matrix(_)) => {
            return Err(format!(
                "{name}: a vector of sample indices is not supported; pass the number of samples n"
            ));
        }
        Some(_) => return Err(format!("{name}: n must be a positive integer")),
    };
    let fs = match rest.get(1) {
        Some(v) => Some(positive_scalar(name, v, "fs")?),
        None => None,
    };
    let n = match (n, &system) {
        (Some(n), _) => n,
        (None, System::Tf(b, a)) => dsp::impulse_length(b, a)?,
        (None, System::Sos(sos)) => dsp::sos_impulse_length(sos)?,
    };
    let kind = if name == "impz" {
        Analysis::Impulse
    } else {
        Analysis::Step
    };
    let y = match &system {
        System::Tf(b, a) => dsp::analyze(kind, b, a, n, &e.cancel)?,
        System::Sos(sos) => dsp::analyze_sos(kind, sos, n, &e.cancel)?,
    };
    let t: Vec<f64> = (0..n).map(|k| k as f64 / fs.unwrap_or(1.)).collect();
    if count == 0 {
        let xlabel = if fs.is_some() { "Time (s)" } else { "Sample" };
        plot(e, &t, &y, [kind.label(), xlabel, "Amplitude"])?;
        return Ok(vec![]);
    }
    let mut values = vec![Value::Matrix(Matrix::from_column_vec(n, 1, y)?)];
    if count == 2 {
        values.push(Value::Matrix(Matrix::from_column_vec(n, 1, t)?));
    }
    Ok(values)
}

/// One line in the current axes (or layout tile); the figure is unchanged on error.
fn plot(e: &mut Engine, x: &[f64], y: &[f64], [title, xlabel, ylabel]: [&str; 3]) -> Result<()> {
    let n = x.len();
    let mut figure = Figure::new(
        &Matrix::from_column_slice(n, 1, x)?,
        &Matrix::from_column_slice(n, 1, y)?,
    )?;
    figure.title = title.into();
    figure.xlabel = xlabel.into();
    figure.ylabel = ylabel.into();
    figure.layout = e.figure.as_ref().and_then(|f| f.layout.clone());
    figure.activate_tile();
    if let Some(previous) = &e.figure {
        figure.identity = previous.identity.clone();
        figure.axes.identity = previous.axes.identity.clone();
    }
    figure.validate()?;
    e.figure = Some(figure);
    Ok(())
}

fn roots_vector(v: &Value, what: &str) -> Result<Vec<C>> {
    let (rows, cols, data): (usize, usize, Vec<C>) = match v {
        Value::Matrix(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|&x| C::new(x, 0.)).collect(),
        ),
        Value::Complex(m) => (m.rows(), m.cols(), m.data().to_vec()),
        _ => return Err(format!("bilinear: {what} must be a numeric vector")),
    };
    if !data.is_empty() && rows != 1 && cols != 1 {
        return Err(format!(
            "bilinear: {what} must be a vector (state-space input A, B, C, D is not supported)"
        ));
    }
    if data.len() > MAX_ROOTS {
        return Err(format!(
            "bilinear: {what} must have at most {MAX_ROOTS} entries"
        ));
    }
    Ok(data)
}

fn roots_column(e: &Engine, roots: &[C]) -> Result<Value> {
    Ok(if roots.iter().all(|r| r.im == 0.) {
        Value::Matrix(Matrix::from_column_vec(
            roots.len(),
            1,
            roots.iter().map(|r| r.re).collect(),
        )?)
    } else {
        Value::Complex(ComplexMatrix::from_column_vec(
            roots.len(),
            1,
            roots.to_vec(),
            &e.cancel,
        )?)
    })
}

/// `[bz, az] = bilinear(b, a, fs[, fp])` or `[zd, pd, kd] = bilinear(z, p, k, fs[, fp])`.
/// As MATLAB, four arguments are zero-pole-gain when p is a column (or scalar) and z has
/// at most one column; otherwise b, a, fs, fp.
fn bilinear(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let columns = |v: &Value| match v {
        Value::Matrix(m) => m.cols(),
        Value::Complex(m) => m.cols(),
        _ => usize::MAX,
    };
    let zpk = match args.len() {
        3 => false,
        4 => columns(&args[1]) == 1 && columns(&args[0]) < 2,
        5 => true,
        _ => {
            return Err(
                "bilinear expects (b, a, fs), (b, a, fs, fp), (z, p, k, fs) or (z, p, k, fs, fp)"
                    .into(),
            );
        }
    };
    let real_scalar = |v: &Value, what: &str| match v {
        Value::Matrix(m) if m.len() == 1 => Ok(m.data()[0]),
        _ => Err(format!("bilinear: {what} must be a real scalar")),
    };
    if zpk {
        if count > 3 {
            return Err(
                "bilinear returns at most three outputs (zd, pd, kd) for zero-pole-gain input"
                    .into(),
            );
        }
        let z = roots_vector(&args[0], "z")?;
        let p = roots_vector(&args[1], "p")?;
        let k = real_scalar(&args[2], "k")?;
        let fs = real_scalar(&args[3], "fs")?;
        let fp = args.get(4).map(|v| real_scalar(v, "fp")).transpose()?;
        let (zd, pd, kd) = dsp::bilinear_zpk(&z, &p, k, dsp::bilinear_scale(fs, fp)?)?;
        let mut values = vec![
            roots_column(e, &zd)?,
            roots_column(e, &pd)?,
            Value::Matrix(Matrix::scalar(kd)),
        ];
        values.truncate(count.max(1));
        return Ok(values);
    }
    if count > 2 {
        return Err(
            "bilinear returns at most two outputs (numd, dend) for transfer-function input".into(),
        );
    }
    let b = coefficients("bilinear", &args[0], "num")?;
    let a = coefficients("bilinear", &args[1], "den")?;
    let fs = real_scalar(&args[2], "fs")?;
    let fp = args.get(3).map(|v| real_scalar(v, "fp")).transpose()?;
    let (bd, ad) = dsp::bilinear_tf(&b, &a, dsp::bilinear_scale(fs, fp)?)?;
    let row = |v: Vec<f64>| Matrix::from_column_vec(1, v.len(), v).map(Value::Matrix);
    let mut values = vec![row(bd)?, row(ad)?];
    values.truncate(count.max(1));
    Ok(values)
}
