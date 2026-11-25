use super::{Engine, Value};
use crate::{
    Complex64 as C, ComplexMatrix, Matrix, Result,
    dsp::iir::{self, IirBand, IirKind, Zpk},
};

struct EngineElliptic<'a> {
    engine: &'a Engine,
}
impl iir::EllipticMath for EngineElliptic<'_> {
    fn jacobi(&self, u: f64, m: f64) -> Result<(f64, f64, f64)> {
        let args = [
            Value::Matrix(Matrix::scalar(u)),
            Value::Matrix(Matrix::scalar(m)),
        ];
        let out = self.engine.special_fns_outputs("ellipj", &args, 3)?;
        let get = |i: usize| -> Result<f64> {
            match &out[i] {
                Value::Matrix(x) if x.len() == 1 => Ok(x.data()[0]),
                _ => Err("ellipj returned an invalid scalar result".into()),
            }
        };
        Ok((get(0)?, get(1)?, get(2)?))
    }
    fn complete(&self, m: f64) -> Result<(f64, f64)> {
        let args = [Value::Matrix(Matrix::scalar(m))];
        let out = self.engine.special_fns_outputs("ellipke", &args, 2)?;
        let get = |i: usize| -> Result<f64> {
            match &out[i] {
                Value::Matrix(x) if x.len() == 1 => Ok(x.data()[0]),
                _ => Err("ellipke returned an invalid scalar result".into()),
            }
        };
        Ok((get(0)?, get(1)?))
    }
}

pub(super) fn handles(n: &str) -> bool {
    matches!(
        n,
        "butter"
            | "cheby2"
            | "ellip"
            | "besself"
            | "buttord"
            | "cheb1ord"
            | "cheb2ord"
            | "ellipord"
            | "zp2tf"
            | "tf2zp"
            | "zp2sos"
            | "sos2zp"
            | "tf2sos"
            | "sos2tf"
            | "zp2ss"
            | "ss2zp"
            | "tf2ss"
            | "ss2tf"
    )
}
fn scalar(v: &Value, what: &str) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Ok(m.data()[0]),
        _ => Err(format!("{what} must be a real scalar")),
    }
}
fn vec_real(v: &Value, what: &str) -> Result<Vec<f64>> {
    match v {
        Value::Matrix(m) if m.rows() == 1 || m.cols() == 1 => {
            if m.len() > 2 * iir::MAX_DESIGN_ORDER + 1 {
                return Err(format!("{what} vector exceeds the supported order bound"));
            }
            Ok(m.data().to_vec())
        }
        Value::Matrix(m) if m.is_empty() => Ok(Vec::new()),
        _ => Err(format!("{what} must be a real vector")),
    }
}
fn vec_complex(v: &Value, what: &str) -> Result<Vec<C>> {
    match v {
        Value::Complex(m) if m.rows() == 1 || m.cols() == 1 => {
            if m.len() > 2 * iir::MAX_DESIGN_ORDER + 1 {
                return Err(format!("{what} vector exceeds the supported order bound"));
            }
            Ok(m.data().to_vec())
        }
        Value::Matrix(m) if m.rows() == 1 || m.cols() == 1 => {
            if m.len() > 2 * iir::MAX_DESIGN_ORDER + 1 {
                return Err(format!("{what} vector exceeds the supported order bound"));
            }
            Ok(m.data().iter().map(|x| C::new(*x, 0.)).collect())
        }
        Value::Matrix(m) if m.is_empty() => Ok(Vec::new()),
        Value::Complex(m) if m.is_empty() => Ok(Vec::new()),
        _ => Err(format!("{what} must be a vector")),
    }
}
fn opt_text(v: &Value) -> Result<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s),
        _ => Err("IIR options must be quoted text".into()),
    }
}
fn band(v: &Value) -> Result<IirBand> {
    match opt_text(v)?.to_ascii_lowercase().as_str() {
        "low" | "lowpass" => Ok(IirBand::Low),
        "high" | "highpass" => Ok(IirBand::High),
        "bandpass" => Ok(IirBand::Bandpass),
        "stop" | "bandstop" => Ok(IirBand::Stop),
        _ => Err("Filter type must be low, high, bandpass or stop".into()),
    }
}
fn default_band(w: &[f64]) -> IirBand {
    if w.len() == 2 {
        IirBand::Bandpass
    } else {
        IirBand::Low
    }
}
fn order(v: &Value) -> Result<usize> {
    let n = scalar(v, "Filter order")?;
    if !n.is_finite() || n.fract() != 0. || !(1. ..=iir::MAX_DESIGN_ORDER as f64).contains(&n) {
        Err("IIR prototype order must be an integer in 1..100".into())
    } else {
        Ok(n as usize)
    }
}
fn matrix(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}
fn complex(v: &[C], e: &Engine) -> Result<Value> {
    if v.iter().all(|x| x.im == 0.) {
        Ok(Value::Matrix(Matrix::from_column_vec(
            v.len(),
            1,
            v.iter().map(|x| x.re).collect(),
        )?))
    } else {
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            v.len(),
            1,
            v.to_vec(),
            &e.cancel,
        )?))
    }
}
fn zpk_args(a: &[Value], _e: &Engine) -> Result<Zpk> {
    if a.len() != 3 {
        return Err("Expected zero, pole and gain arguments".into());
    }
    Ok(Zpk {
        zeros: vec_complex(&a[0], "Zeros")?,
        poles: vec_complex(&a[1], "Poles")?,
        gain: scalar(&a[2], "Gain")?,
    })
}
fn results_zpk(z: &Zpk, e: &Engine) -> Result<Vec<Value>> {
    Ok(vec![
        complex(&z.zeros, e)?,
        complex(&z.poles, e)?,
        Value::Matrix(Matrix::scalar(z.gain)),
    ])
}
fn results_tf(b: Vec<f64>, a: Vec<f64>) -> Result<Vec<Value>> {
    Ok(vec![matrix(b)?, matrix(a)?])
}
fn results_ss(x: (Matrix, Matrix, Matrix, Matrix)) -> Vec<Value> {
    vec![
        Value::Matrix(x.0),
        Value::Matrix(x.1),
        Value::Matrix(x.2),
        Value::Matrix(x.3),
    ]
}
struct ParsedDesign {
    kind: IirKind,
    order: usize,
    rp: f64,
    rs: f64,
    edges: Vec<f64>,
    band: IirBand,
    analog: bool,
    sos: bool,
}
fn parse_design(name: &str, a: &[Value]) -> Result<ParsedDesign> {
    let (base, rpi, rsi, wi, kind) = match name {
        "butter" => (2, usize::MAX, usize::MAX, 1, IirKind::Butter),
        "cheby2" => (3, usize::MAX, 1, 2, IirKind::Cheby2),
        "ellip" => (4, 1, 2, 3, IirKind::Ellip),
        "besself" => (2, usize::MAX, usize::MAX, 1, IirKind::Bessel),
        _ => unreachable!(),
    };
    if a.len() < base || a.len() > base + 3 {
        return Err(format!("{name} has invalid argument count"));
    }
    let n = order(&a[0])?;
    let rp = if rpi == usize::MAX {
        0.
    } else {
        scalar(&a[rpi], "Passband ripple")?
    };
    let rs = if rsi == usize::MAX {
        0.
    } else {
        scalar(&a[rsi], "Stopband attenuation")?
    };
    let w = vec_real(&a[wi], "Critical frequencies")?;
    let mut b = default_band(&w);
    let mut analog = name == "besself";
    let mut sos = false;
    for v in &a[base..] {
        let t = opt_text(v)?;
        if t.eq_ignore_ascii_case("sos") {
            sos = true
        } else if t.eq_ignore_ascii_case("s") {
            analog = true
        } else {
            b = band(v)?
        }
    }
    if name == "besself" && (a.len() != 2 || !analog) {
        return Err("besself accepts n,Wo and is analog only".into());
    }
    Ok(ParsedDesign {
        kind,
        order: n,
        rp,
        rs,
        edges: w,
        band: b,
        analog,
        sos,
    })
}
/// Digital zpk design for `designfilt`.
pub(super) fn design_zpk(
    e: &Engine,
    kind: IirKind,
    order: usize,
    rs: f64,
    edges: &[f64],
    band: IirBand,
) -> Result<Zpk> {
    let math = EngineElliptic { engine: e };
    iir::design(
        iir::DesignSpec {
            kind,
            order,
            rp: 0.,
            rs,
            edges,
            band,
            analog: false,
        },
        &math,
        &e.cancel,
    )
}
fn design_outputs(name: &str, a: &[Value], e: &Engine, count: usize) -> Result<Vec<Value>> {
    let ParsedDesign {
        kind,
        order,
        rp,
        rs,
        edges,
        band,
        analog,
        sos,
    } = parse_design(name, a)?;
    let math = EngineElliptic { engine: e };
    let z = iir::design(
        iir::DesignSpec {
            kind,
            order,
            rp,
            rs,
            edges: &edges,
            band,
            analog,
        },
        &math,
        &e.cancel,
    )?;
    if sos {
        if analog {
            return Err("Analog SOS design is not supported".into());
        }
        return sos_outputs(&z, count);
    }
    match count {
        1 => {
            let (x, _) = iir::zpk2tf(&z)?;
            Ok(vec![matrix(x)?])
        }
        2 => {
            let (b, a) = iir::zpk2tf(&z)?;
            results_tf(b, a)
        }
        3 => results_zpk(&z, e),
        4 => Ok(results_ss(iir::zpk2ss(&z)?)),
        _ => Err(format!(
            "{name} supports one/two b,a, three z,p,k, or four A,B,C,D outputs"
        )),
    }
}
fn edge_vector(v: &Value) -> Result<Vec<f64>> {
    vec_real(v, "Order design frequencies")
}
fn order_output(name: &str, a: &[Value], count: usize, e: &Engine) -> Result<Vec<Value>> {
    let arity = 4;
    if a.len() < arity || a.len() > arity + 1 {
        return Err(format!("{name} expects Wp, Ws, Rp, Rs and optional 's'"));
    }
    let wp = edge_vector(&a[0])?;
    let ws = edge_vector(&a[1])?;
    let analog = match a.get(arity) {
        None => false,
        Some(v) if opt_text(v)?.eq_ignore_ascii_case("s") => true,
        Some(_) => return Err("The only order-selection option supported is 's'".into()),
    };
    let (n, wn) = iir::select_order(
        iir::OrderSpec {
            name,
            wp: &wp,
            ws: &ws,
            rp: scalar(&a[2], "Rp")?,
            rs: scalar(&a[3], "Rs")?,
            analog,
        },
        &EngineElliptic { engine: e },
        &e.cancel,
    )?;
    let mut out = vec![Value::Matrix(Matrix::scalar(n as f64)), matrix(wn)?];
    if count == 1 {
        out.truncate(1);
    } else if count != 2 {
        return Err(format!("{name} supports one or two outputs"));
    }
    Ok(out)
}
fn call_inner(name: &str, a: &[Value], e: &Engine, count: usize) -> Result<Vec<Value>> {
    if matches!(name, "butter" | "cheby2" | "ellip" | "besself") {
        return design_outputs(name, a, e, count);
    }
    if name.ends_with("ord") {
        return order_output(name, a, count, e);
    }
    let max_outputs = match name {
        "tf2zp" | "sos2zp" | "ss2zp" => 3,
        "zp2ss" | "tf2ss" => 4,
        _ => 2,
    };
    if count == 0 || count > max_outputs {
        return Err(format!("{name} supports one through {max_outputs} outputs"));
    }
    let mut outputs = match name {
        "zp2tf" => {
            let z = zpk_args(a, e)?;
            let (b, d) = iir::zpk2tf(&z)?;
            results_tf(b, d)
        }
        "tf2zp" => {
            if a.len() != 2 {
                return Err("tf2zp requires b,a and three outputs".into());
            }
            results_zpk(
                &iir::tf2zpk(
                    &vec_real(&a[0], "Numerator")?,
                    &vec_real(&a[1], "Denominator")?,
                )?,
                e,
            )
        }
        "zp2sos" => sos_outputs(&zpk_args(a, e)?, count),
        "sos2zp" => {
            if a.len() != 1 {
                return Err("sos2zp requires one SOS input and three outputs".into());
            }
            results_zpk(
                &iir::sos2zpk(match &a[0] {
                    Value::Matrix(x) => x,
                    _ => return Err("SOS input must be real".into()),
                })?,
                e,
            )
        }
        "tf2sos" => {
            if a.len() != 2 {
                return Err("tf2sos requires b,a and one output".into());
            }
            // Shorter numerator: zeros at the origin, not the delay zp2sos sheds.
            sos_outputs(
                &iir::digital_tf_zpk(
                    &vec_real(&a[0], "Numerator")?,
                    &vec_real(&a[1], "Denominator")?,
                )?,
                count,
            )
        }
        "sos2tf" => {
            // MATLAB: the sections multiplied out (conv), b scaled by prod(g), g = 1 by default.
            if a.is_empty() || a.len() > 2 {
                return Err("sos2tf expects an SOS matrix and an optional gain g".into());
            }
            let (mut b, d) = crate::dsp::sos_to_tf(match &a[0] {
                Value::Matrix(x) => x,
                _ => return Err("SOS input must be real".into()),
            })?;
            let g = match a.get(1) {
                None => 1.,
                Some(Value::Matrix(g))
                    if !g.is_empty() && g.data().iter().all(|v| v.is_finite()) =>
                {
                    g.data().iter().product()
                }
                Some(_) => return Err("sos2tf: the gain g must be finite real numbers".into()),
            };
            b.iter_mut().for_each(|v| *v *= g);
            results_tf(b, d)
        }
        "zp2ss" => Ok(results_ss(iir::zpk2ss(&zpk_args(a, e)?)?)),
        "ss2zp" => {
            if a.len() != 4 {
                return Err("ss2zp requires A,B,C,D and three outputs".into());
            }
            let x = match (&a[0], &a[1], &a[2], &a[3]) {
                (Value::Matrix(x), Value::Matrix(y), Value::Matrix(z), Value::Matrix(w)) => {
                    (x, y, z, w)
                }
                _ => return Err("State-space inputs must be real".into()),
            };
            results_zpk(&iir::ss2zpk(x.0, x.1, x.2, x.3)?, e)
        }
        "tf2ss" => {
            if a.len() != 2 {
                return Err("tf2ss returns A,B,C,D".into());
            }
            Ok(results_ss(iir::tf2ss(
                &vec_real(&a[0], "Numerator")?,
                &vec_real(&a[1], "Denominator")?,
            )?))
        }
        "ss2tf" => {
            if a.len() != 4 {
                return Err("ss2tf requires A,B,C,D and returns b,a".into());
            }
            let x = match (&a[0], &a[1], &a[2], &a[3]) {
                (Value::Matrix(x), Value::Matrix(y), Value::Matrix(z), Value::Matrix(w)) => {
                    (x, y, z, w)
                }
                _ => return Err("State-space inputs must be real".into()),
            };
            let (b, d) = iir::ss2tf(x.0, x.1, x.2, x.3)?;
            results_tf(b, d)
        }
        _ => Err("Unknown IIR function".into()),
    }?;
    outputs.truncate(count);
    Ok(outputs)
}
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    count: usize,
    out: &mut super::Execution,
) -> Result<Vec<Value>> {
    let r = call_inner(name, a, e, count)?;
    e.charge_function_outputs(&r)?;
    e.check_cancel()?;
    let _ = out;
    Ok(r)
}

fn sos_outputs(z: &Zpk, count: usize) -> Result<Vec<Value>> {
    match count {
        1 => Ok(vec![Value::Matrix(iir::zpk2sos(z)?)]),
        2 => {
            let unit = Zpk {
                zeros: z.zeros.clone(),
                poles: z.poles.clone(),
                gain: 1.,
            };
            Ok(vec![
                Value::Matrix(iir::zpk2sos(&unit)?),
                Value::Matrix(Matrix::scalar(z.gain)),
            ])
        }
        _ => Err("SOS form supports one or two outputs".into()),
    }
}
