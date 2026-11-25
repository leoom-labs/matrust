//! Communications waveforms. Contracts: public MathWorks reference pages;
//! pulse/CPFSK/OFDM formulae: Proakis, Digital Communications (5th ed.).
//! Independent NumPy/SciPy references: tool/refs/comm/wave.py. No MATLAB.
mod fit;
mod modems;
mod ofdm;
mod plots;
mod pulse;
use super::{Engine, Execution, Value};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub(super) const NAMES: &[&str] = &[
    "rcosdesign",
    "fskmod",
    "fskdemod",
    "dpskmod",
    "dpskdemod",
    "mskmod",
    "mskdemod",
    "ofdmmod",
    "ofdmdemod",
    "scatterplot",
    "eyediagram",
    "berfit",
];
const MAX_SAMPLES: usize = 8_388_608;
const MAX_WORK: usize = 100_000_000;
pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
pub(super) fn is_plot(name: &str) -> bool {
    matches!(name, "scatterplot" | "eyediagram" | "berfit")
}
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match name {
        "mskmod" | "ofdmdemod" | "berfit" => 2,
        "mskdemod" => 3,
        _ => 1,
    };
    if nout > max {
        return Err(format!("{name}: too many output arguments"));
    }
    let values = match name {
        "rcosdesign" => vec![pulse::design(e, a)?],
        "fskmod" | "fskdemod" => vec![modems::fsk(e, a, name == "fskdemod")?],
        "dpskmod" | "dpskdemod" => vec![modems::dpsk(e, a, name == "dpskdemod")?],
        "mskmod" | "mskdemod" => modems::msk(e, a, name == "mskdemod", nout)?,
        "ofdmmod" | "ofdmdemod" => ofdm::call(e, a, name == "ofdmdemod", nout)?,
        "scatterplot" => vec![plots::scatter(e, a, out)?],
        "eyediagram" => vec![plots::eye(e, a, out)?],
        "berfit" => fit::call(e, a, nout, out)?,
        _ => unreachable!(),
    };
    e.charge_function_outputs(&values)?;
    e.check_cancel()?;
    Ok(values)
}
fn arity(a: &[Value], lo: usize, hi: usize, name: &str) -> Result<()> {
    if !(lo..=hi).contains(&a.len()) {
        Err(format!("{name}: expected {lo} to {hi} input arguments"))
    } else {
        Ok(())
    }
}
fn numeric(e: &Engine, v: &Value, name: &str, maxdim: usize) -> Result<Array> {
    let a = v.clone().array(&e.cancel)?;
    if !matches!(a, Array::Numeric(_) | Array::Complex(_) | Array::Logical(_))
        || a.dims().len() > maxdim
    {
        return Err(format!(
            "{name}: expected a numeric array with at most {maxdim} dimensions"
        ));
    }
    for first in (0..a.len()).step_by(4096) {
        e.check_cancel()?;
        if (first..(first + 4096).min(a.len())).any(|i| {
            let z = at(&a, i);
            !z.re.is_finite() || !z.im.is_finite()
        }) {
            return Err(format!("{name}: input values must be finite"));
        }
    }
    Ok(a)
}
fn at(a: &Array, i: usize) -> Complex64 {
    match a {
        Array::Numeric(m) => Complex64::new(m.data()[i], 0.),
        Array::Complex(m) => m.data()[i],
        Array::Logical(m) => Complex64::new(f64::from(m.data()[i]), 0.),
        _ => unreachable!(),
    }
}
fn real(e: &Engine, v: &Value, name: &str) -> Result<Matrix> {
    match numeric(e, v, name, 2)? {
        Array::Numeric(m) => Ok(m),
        _ => Err(format!("{name}: expected real numeric input")),
    }
}
fn vector(e: &Engine, v: &Value, name: &str) -> Result<Matrix> {
    let m = real(e, v, name)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(format!("{name}: expected a vector"));
    }
    Ok(m)
}
fn scalar(e: &Engine, v: &Value, name: &str) -> Result<f64> {
    real(e, v, name)?
        .as_scalar()
        .map_err(|_| format!("{name}: expected a finite real scalar"))
}
fn integer(e: &Engine, v: &Value, lo: usize, hi: usize, name: &str) -> Result<usize> {
    let n = scalar(e, v, name)?;
    if n.fract() != 0. || n < lo as f64 || n > hi as f64 {
        return Err(format!("{name}: expected integer in {lo}..{hi}"));
    }
    Ok(n as usize)
}
fn positive(e: &Engine, v: &Value, name: &str) -> Result<f64> {
    let x = scalar(e, v, name)?;
    if x <= 0. {
        Err(format!("{name}: expected a positive scalar"))
    } else {
        Ok(x)
    }
}
fn text(e: &Engine, v: &Value, name: &str) -> Result<String> {
    super::table_fns::text_scalar(v)
        .ok_or_else(|| format!("{name}: expected scalar text"))
        .or_else(|_| {
            if let Array::Character(c) = v.clone().array(&e.cancel)? {
                c.to_utf8(&e.cancel)
            } else {
                Err(format!("{name}: expected scalar text"))
            }
        })
}
fn empty(v: &Value) -> bool {
    matches!(v, Value::Matrix(m) if m.is_empty())
}
fn choice(
    e: &Engine,
    v: Option<&Value>,
    default: &str,
    choices: &[&str],
    name: &str,
) -> Result<String> {
    let s = match v {
        Some(v) if !empty(v) => text(e, v, name)?.to_ascii_lowercase(),
        _ => default.into(),
    };
    if choices.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(format!("{name}: expected {}", choices.join(" or ")))
    }
}
fn reserve(e: &Engine, units: usize) -> Result<()> {
    e.preflight_elements(units)?;
    if units.saturating_mul(8).saturating_add(e.workspace_bytes()) > e.limits.memory_bytes {
        return Err(
            "comm_wave: scratch and output exceed remaining workspace memory budget".into(),
        );
    }
    Ok(())
}
fn work(e: &mut Engine, n: usize) -> Result<()> {
    if n > MAX_WORK {
        return Err("comm_wave: operation exceeds 100000000 work limit".into());
    }
    e.charge(n)?;
    e.check_cancel()
}
fn zeros<T: Default + Clone>(e: &Engine, n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| "comm_wave: cannot allocate output or scratch")?;
    for first in (0..n).step_by(4096) {
        e.check_cancel()?;
        v.resize((first + 4096).min(n), T::default());
    }
    Ok(v)
}
fn rv(rows: usize, cols: usize, v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, v)?))
}
fn cv(e: &Engine, dims: &[usize], v: Vec<Complex64>) -> Result<Value> {
    Ok(Value::Complex(
        ComplexMatrix::from_column_vec(dims[0], dims[1..].iter().product(), v, &e.cancel)?
            .with_dims(dims)?,
    ))
}
fn unit(p: f64) -> Complex64 {
    Complex64::new(p.cos(), p.sin())
}
// Reflected binary Gray labels on monotonically ordered tones/phase increments.
fn ungray(mut k: usize) -> usize {
    let mut v = k;
    while k > 0 {
        k >>= 1;
        v ^= k;
    }
    v
}
fn label(k: usize, gray: bool) -> usize {
    if gray { k ^ (k >> 1) } else { k }
}
fn symbols(e: &Engine, a: &Array, m: usize, name: &str) -> Result<()> {
    for first in (0..a.len()).step_by(4096) {
        e.check_cancel()?;
        if (first..(first + 4096).min(a.len())).any(|i| {
            let z = at(a, i);
            z.im != 0. || z.re < 0. || z.re >= m as f64 || z.re.fract() != 0.
        }) {
            return Err(format!("{name}: symbols must be integers in 0..M-1"));
        }
    }
    Ok(())
}
fn order(e: &Engine, v: &Value, name: &str) -> Result<usize> {
    let m = integer(e, v, 2, 65536, name)?;
    if !m.is_power_of_two() {
        return Err(format!("{name}: M must be a power of two"));
    }
    Ok(m)
}
// A vector is one channel; only a genuine matrix has multiple channels.
fn lanes(a: &Array) -> (usize, usize) {
    if a.rows() == 1 {
        (a.len(), 1)
    } else {
        (a.rows(), a.cols())
    }
}
