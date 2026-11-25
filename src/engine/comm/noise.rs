use super::super::random::Generator;
use super::*;
fn seed(e: &Engine, v: &Value, n: &str) -> Result<Generator> {
    let s = count(e, v, n, 0, u32::MAX as usize)?;
    let mut g = Generator::default();
    g.seed(s as u32);
    Ok(g)
}
pub(super) fn call(e: &mut Engine, n: &str, a: &[Value]) -> Result<Vec<Value>> {
    if n == "awgn" { awgn(e, a) } else { wgn(e, a) }
}
fn awgn(e: &mut Engine, a: &[Value]) -> Result<Vec<Value>> {
    let n = "awgn";
    arity(n, a, 2, 5)?;
    let x = numeric(e, &a[0], n)?;
    if !matches!(x, Array::Numeric(_) | Array::Complex(_) | Array::Typed(_)) {
        return Err(err(n, "InvalidInput", "Signal must be floating point."));
    }
    if matches!(&x,Array::Typed(t) if t.is_integer()) {
        return Err(err(n, "InvalidInput", "Signal must be floating point."));
    }
    let mut end = a.len();
    let mut linear = false;
    if a.len() >= 4 && text(e, &a[end - 1]).is_ok() {
        linear = choose(e, n, &a[end - 1], &["db", "linear"])? == "linear";
        end -= 1;
    }
    if end > 4 {
        return Err(err(
            n,
            "InvalidNumInputs",
            "Incorrect number of input arguments.",
        ));
    }
    let ratio = scalar(e, &a[1])?;
    if ratio.is_nan() || (linear && ratio <= 0.) {
        return Err(err(
            n,
            "InvalidSNR",
            "SNR must be real (positive for linear power).",
        ));
    }
    let power = if end >= 3 {
        if text(e, &a[2]).is_ok() {
            choose(e, n, &a[2], &["measured"])?;
            if x.is_empty() {
                return Err(err(
                    n,
                    "InvalidInput",
                    "Cannot measure power of an empty signal.",
                ));
            }
            let mut sum = 0.;
            for i in 0..x.len() {
                tick(e, i)?;
                sum += complex(&x, i)?.norm_sqr();
            }
            sum / x.len() as f64
        } else {
            let p = scalar(e, &a[2])?;
            if linear { p } else { 10f64.powf(p / 10.) }
        }
    } else {
        1.
    };
    let variance = power
        / if linear {
            ratio
        } else {
            10f64.powf(ratio / 10.)
        };
    if !power.is_finite() || power < 0. || !variance.is_finite() || variance < 0. {
        return Err(err(
            n,
            "InvalidPower",
            "Signal and noise powers must be finite and nonnegative.",
        ));
    }
    let local = if end == 4 {
        Some(seed(e, &a[3], n)?)
    } else {
        None
    };
    let dims = x.dims().to_vec();
    let c = matches!(x, Array::Complex(_));
    preflight(e, &dims, if c { 2 } else { 1 })?;
    let v = generate(e, &dims, variance, c, local, Some(&x))?;
    Ok(vec![v, Value::Matrix(Matrix::scalar(variance))])
}
fn wgn(e: &mut Engine, a: &[Value]) -> Result<Vec<Value>> {
    let n = "wgn";
    arity(n, a, 3, 7)?;
    let rows = count(e, &a[0], n, 1, crate::matrix::MAX_ELEMENTS)?;
    let cols = count(e, &a[1], n, 1, crate::matrix::MAX_ELEMENTS)?;
    let p = scalar(e, &a[2])?;
    let mut idx = 3;
    let impedance = if a.get(idx).is_some_and(|v| text(e, v).is_err()) {
        idx += 1;
        scalar(e, &a[idx - 1])?
    } else {
        1.
    };
    if !impedance.is_finite() || impedance <= 0. {
        return Err(err(
            n,
            "InvalidImpedance",
            "Impedance must be finite and positive.",
        ));
    }
    let local = if a.get(idx).is_some_and(|v| text(e, v).is_err()) {
        idx += 1;
        Some(seed(e, &a[idx - 1], n)?)
    } else {
        None
    };
    let mut units = "dbw".to_string();
    let mut c = false;
    let mut saw_units = false;
    let mut saw_type = false;
    for arg in &a[idx..] {
        let s = choose(e, n, arg, &["dbw", "dbm", "linear", "real", "complex"])?;
        if s == "real" || s == "complex" {
            if saw_type {
                return Err(err(n, "InvalidOption", "Output type specified twice."));
            }
            saw_type = true;
            c = s == "complex";
        } else {
            if saw_units {
                return Err(err(n, "InvalidOption", "Power type specified twice."));
            }
            saw_units = true;
            units = s;
        }
    }
    let power = match units.as_str() {
        "linear" => p,
        "dbm" => 10f64.powf((p - 30.) / 10.),
        _ => 10f64.powf(p / 10.),
    };
    let variance = power * impedance;
    if !variance.is_finite() || variance < 0. {
        return Err(err(
            n,
            "InvalidPower",
            "Noise power must be finite and nonnegative.",
        ));
    }
    preflight(e, &[rows, cols], if c { 2 } else { 1 })?;
    Ok(vec![generate(e, &[rows, cols], variance, c, local, None)?])
}
fn generate(
    e: &mut Engine,
    dims: &[usize],
    variance: f64,
    c: bool,
    mut local: Option<Generator>,
    input: Option<&Array>,
) -> Result<Value> {
    let len = preflight(e, dims, if c { 2 } else { 1 })?;
    if c {
        ComplexMatrix::checked_len(dims[0], dims[1..].iter().product())?;
    }
    let scale = (variance / if c { 2. } else { 1. }).sqrt();
    if c {
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            tick(e, i)?;
            let g = local.as_mut().unwrap_or(&mut e.random);
            let z = Complex64::new(g.next_normal() * scale, g.next_normal() * scale);
            data.push(
                z + input
                    .map(|a| complex(a, i))
                    .transpose()?
                    .unwrap_or_default(),
            );
        }
        complex_output(e, dims, data)
    } else {
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            tick(e, i)?;
            let g = local.as_mut().unwrap_or(&mut e.random);
            data.push(
                g.next_normal() * scale + input.map(|a| real(a, i)).transpose()?.unwrap_or(0.),
            );
        }
        output(
            dims,
            data,
            if input.is_some_and(is_single) {
                "single"
            } else {
                "double"
            },
        )
    }
}
