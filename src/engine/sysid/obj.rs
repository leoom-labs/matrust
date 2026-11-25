//! Value objects for system identification: `iddata` and the `idpoly`,
//! `idtf`, `idss` and `idproc` models. Like the LTI values they are tagged
//! 1-by-1 structs of immutable shared property slots; the tag field is private
//! and every operation decodes (and validates) the public properties again,
//! so dot-assignment of a property is seen by the next call.
use super::super::{Engine, Value};
use super::lti::{Model, Proc, Structure};
use crate::{Array, CellArray, CharacterRow, Matrix, Result, StructArray};
use nalgebra::DMatrix;
use std::sync::Arc;

pub(super) const TAG: &str = "MatRustSysId";
pub(super) const MAX_SAMPLES: usize = 10_000_000;
pub(super) const MAX_ORDER: usize = 64;

pub(super) fn class(a: &Array) -> Option<&'static str> {
    let Array::Struct(s) = a else { return None };
    if s.len() != 1 {
        return None;
    }
    let Array::Character(c) = s.get(s.field_index(TAG)?, 0).ok()?.as_ref() else {
        return None;
    };
    match String::from_utf16_lossy(c.units()).as_str() {
        "iddata" => Some("iddata"),
        "idpoly" => Some("idpoly"),
        "idtf" => Some("idtf"),
        "idss" => Some("idss"),
        "idproc" => Some("idproc"),
        _ => None,
    }
}
pub(super) fn value_class(v: &Value) -> Option<&'static str> {
    match v {
        Value::Container(a) => class(a),
        _ => None,
    }
}
pub(super) fn is_model(v: &Value) -> bool {
    value_class(v).is_some_and(|c| c != "iddata")
}
pub(super) fn is_data(v: &Value) -> bool {
    value_class(v) == Some("iddata")
}

pub(super) fn chars(e: &Engine, s: &str) -> Result<Arc<Array>> {
    Ok(Arc::new(CharacterRow::from_text(s, &e.cancel)?.into()))
}
pub(super) fn num(m: Matrix) -> Arc<Array> {
    Arc::new(m.into())
}
pub(super) fn scalar(x: f64) -> Arc<Array> {
    num(Matrix::scalar(x))
}
pub(super) fn row(v: &[f64]) -> Result<Arc<Array>> {
    Ok(num(Matrix::from_column_slice(1, v.len(), v)?))
}
pub(super) fn dmat(m: &DMatrix<f64>) -> Result<Arc<Array>> {
    Ok(num(Matrix::from_column_slice(
        m.nrows(),
        m.ncols(),
        m.as_slice(),
    )?))
}
pub(super) fn names_cell(e: &Engine, prefix: &str, n: usize) -> Result<Arc<Array>> {
    let slots = (0..n)
        .map(|i| {
            if prefix.is_empty() {
                chars(e, "")
            } else {
                chars(e, &format!("{prefix}{}", i + 1))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Arc::new(
        CellArray::from_shared(n, 1, slots, &e.cancel)?.into(),
    ))
}
pub(super) fn make_struct(e: &Engine, fields: Vec<(&str, Arc<Array>)>) -> Result<Array> {
    let (names, slots): (Vec<String>, Vec<Arc<Array>>) =
        fields.into_iter().map(|(n, a)| (n.to_string(), a)).unzip();
    Ok(StructArray::from_slots(1, 1, names, slots, &e.cancel)?.into())
}
pub(super) fn get(s: &StructArray, name: &str) -> Option<Arc<Array>> {
    s.get(s.field_index(name)?, 0).ok().cloned()
}
pub(super) fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            match Value::from(Array::Str(s.clone())) {
                Value::Text(t) | Value::QuotedString(t) => Some(t),
                _ => None,
            }
        }
        _ => None,
    }
}
pub(super) fn real_of(v: &Value, what: &str) -> Result<Matrix> {
    match v {
        Value::Matrix(m) => {
            if m.is_nd() {
                return Err(format!("{what} must be a 2-D real double array"));
            }
            Ok(m.clone())
        }
        Value::Logical(l) => Ok(Matrix::from_column_vec(
            l.rows(),
            l.cols(),
            l.data().iter().map(|b| f64::from(*b)).collect(),
        )?),
        _ => Err(format!("{what} must be a real numeric array")),
    }
}
pub(super) fn scalar_of(v: &Value, what: &str) -> Result<f64> {
    let m = real_of(v, what)?;
    if m.len() != 1 {
        return Err(format!("{what} must be a real scalar"));
    }
    Ok(m.data()[0])
}
fn arr_value(a: &Arc<Array>) -> Value {
    Value::from(a.as_ref().clone())
}
fn field_real(s: &StructArray, name: &str, class: &str) -> Result<Matrix> {
    let a = get(s, name).ok_or_else(|| format!("Invalid {class} object: missing {name}"))?;
    let m = real_of(&arr_value(&a), &format!("{class}.{name}"))?;
    let nan_ok = class == "idproc"
        && matches!(
            name,
            "Kp" | "Tp1" | "Tp2" | "Tp3" | "Tw" | "Zeta" | "Td" | "Tz"
        );
    if m.data()
        .iter()
        .any(|x| !x.is_finite() && !(nan_ok && x.is_nan()))
    {
        return Err(format!("{class}.{name} must be finite"));
    }
    Ok(m)
}
fn field_scalar(s: &StructArray, name: &str, class: &str) -> Result<f64> {
    let m = field_real(s, name, class)?;
    if m.len() != 1 {
        return Err(format!("{class}.{name} must be a scalar"));
    }
    Ok(m.data()[0])
}

// ---------------------------------------------------------------- iddata --
#[derive(Clone)]
pub(super) struct IdData {
    pub y: Matrix,
    pub u: Matrix,
    pub ts: f64,
    pub tstart: f64,
    /// Every other public property, in display order.
    pub meta: Vec<(String, Arc<Array>)>,
}
impl IdData {
    pub fn samples(&self) -> usize {
        self.y.rows()
    }
    pub fn ny(&self) -> usize {
        self.y.cols()
    }
    pub fn nu(&self) -> usize {
        self.u.cols()
    }
    pub fn col(m: &Matrix, j: usize) -> Vec<f64> {
        let n = m.rows();
        m.data()[j * n..(j + 1) * n].to_vec()
    }
    /// The SISO output and input columns (input empty for time series).
    pub fn siso(&self, who: &str, need_input: bool) -> Result<(Vec<f64>, Vec<f64>)> {
        if self.ny() != 1 || self.nu() > 1 {
            return Err(format!(
                "{who}: only single-output, single-input data are supported yet"
            ));
        }
        if need_input && self.nu() == 0 {
            return Err(format!("{who}: the data must contain an input channel"));
        }
        let y = Self::col(&self.y, 0);
        let u = if self.nu() == 1 {
            Self::col(&self.u, 0)
        } else {
            vec![0.; y.len()]
        };
        Ok((y, u))
    }
    pub fn defaults(e: &Engine, ny: usize, nu: usize) -> Result<Vec<(String, Arc<Array>)>> {
        Ok(vec![
            ("Domain".into(), chars(e, "Time")?),
            ("Name".into(), chars(e, "")?),
            ("OutputName".into(), names_cell(e, "y", ny)?),
            ("OutputUnit".into(), names_cell(e, "", ny)?),
            ("InputName".into(), names_cell(e, "u", nu)?),
            ("InputUnit".into(), names_cell(e, "", nu)?),
            ("TimeUnit".into(), chars(e, "seconds")?),
            ("InterSample".into(), chars(e, "zoh")?),
        ])
    }
    pub fn meta(&self, name: &str) -> Option<&Arc<Array>> {
        self.meta.iter().find(|(n, _)| n == name).map(|(_, a)| a)
    }
    pub fn encode(&self, e: &Engine) -> Result<Value> {
        let mut fields: Vec<(&str, Arc<Array>)> = vec![(TAG, chars(e, "iddata")?)];
        let meta = |n: &str| self.meta(n).cloned();
        for n in ["Domain", "Name"] {
            if let Some(a) = meta(n) {
                fields.push((n, a));
            }
        }
        fields.push(("OutputData", num(self.y.clone())));
        for n in ["OutputName", "OutputUnit"] {
            if let Some(a) = meta(n) {
                fields.push((n, a));
            }
        }
        fields.push(("InputData", num(self.u.clone())));
        for n in ["InputName", "InputUnit"] {
            if let Some(a) = meta(n) {
                fields.push((n, a));
            }
        }
        fields.push(("Ts", scalar(self.ts)));
        fields.push(("Tstart", scalar(self.tstart)));
        for (n, a) in &self.meta {
            if !matches!(
                n.as_str(),
                "Domain" | "Name" | "OutputName" | "OutputUnit" | "InputName" | "InputUnit"
            ) {
                fields.push((n.as_str(), a.clone()));
            }
        }
        Ok(Value::Container(make_struct(e, fields)?))
    }
    pub fn decode(a: &Array) -> Result<Self> {
        let Array::Struct(s) = a else {
            return Err("Invalid iddata object".into());
        };
        let y = field_real(s, "OutputData", "iddata")?;
        let mut u = field_real(s, "InputData", "iddata")?;
        if u.is_empty() {
            u = Matrix::from_column_vec(y.rows(), 0, Vec::new())?;
        }
        if u.rows() != y.rows() {
            return Err(
                "iddata: OutputData and InputData must have the same number of samples".into(),
            );
        }
        let ts = field_scalar(s, "Ts", "iddata")?;
        if ts.is_nan() || ts <= 0. {
            return Err("iddata: Ts must be a positive finite sample time".into());
        }
        let tstart = field_scalar(s, "Tstart", "iddata")?;
        let meta = s
            .field_names()
            .iter()
            .filter(|n| {
                !matches!(
                    n.as_str(),
                    TAG | "OutputData" | "InputData" | "Ts" | "Tstart"
                )
            })
            .filter_map(|n| get(s, n).map(|a| (n.clone(), a)))
            .collect();
        Ok(IdData {
            y,
            u,
            ts,
            tstart,
            meta,
        })
    }
}

// ----------------------------------------------------------------- models --
#[derive(Clone)]
pub(super) struct IdModel {
    pub model: Model,
    pub io_delay: f64,
    pub input_delay: f64,
    pub meta: Vec<(String, Arc<Array>)>,
}
impl IdModel {
    pub fn class(&self) -> &'static str {
        match self.model.s {
            Structure::Poly { .. } => "idpoly",
            Structure::Tf { .. } => "idtf",
            Structure::Ss { .. } => "idss",
            Structure::Proc(_) => "idproc",
        }
    }
    pub fn new(e: &Engine, model: Model, io_delay: f64, input_delay: f64) -> Result<Self> {
        let (ny, nu) = match &model.s {
            Structure::Ss { b, c, .. } => (c.nrows(), b.ncols()),
            Structure::Poly { b, .. } if b.is_empty() => (1, 0),
            _ => (1, 1),
        };
        let meta = vec![
            ("TimeUnit".into(), chars(e, "seconds")?),
            ("InputName".into(), names_cell(e, "", nu)?),
            ("OutputName".into(), names_cell(e, "", ny)?),
            ("Name".into(), chars(e, "")?),
            (
                "Report".into(),
                Arc::new(report(
                    e,
                    "",
                    "Created by direct construction or transformation. Not estimated.",
                    None,
                )?),
            ),
        ];
        let mut m = IdModel {
            model,
            io_delay,
            input_delay,
            meta,
        };
        m.sync();
        Ok(m)
    }
    fn sync(&mut self) {
        self.model.delay = self.io_delay + self.input_delay;
    }
    pub fn set_meta(&mut self, name: &str, a: Arc<Array>) {
        if let Some(slot) = self.meta.iter_mut().find(|(n, _)| n == name) {
            slot.1 = a;
        } else {
            self.meta.push((name.into(), a));
        }
    }
    pub fn meta(&self, name: &str) -> Option<&Arc<Array>> {
        self.meta.iter().find(|(n, _)| n == name).map(|(_, a)| a)
    }
    pub fn encode(&self, e: &Engine) -> Result<Value> {
        let m = &self.model;
        let mut f: Vec<(&str, Arc<Array>)> = vec![(TAG, chars(e, self.class())?)];
        match &m.s {
            Structure::Poly { a, b, c, d, f: ff } => {
                f.push(("A", row(a)?));
                f.push(("B", row(b)?));
                f.push(("C", row(c)?));
                f.push(("D", row(d)?));
                f.push(("F", row(ff)?));
                f.push(("Variable", chars(e, "z^-1")?));
                f.push(("IODelay", scalar(self.io_delay)));
            }
            Structure::Tf { num, den } => {
                f.push(("Numerator", row(num)?));
                f.push(("Denominator", row(den)?));
                f.push(("Variable", chars(e, if m.ts == 0. { "s" } else { "z" })?));
                f.push(("IODelay", scalar(self.io_delay)));
            }
            Structure::Ss { a, b, c, d, k } => {
                f.push(("A", dmat(a)?));
                f.push(("B", dmat(b)?));
                f.push(("C", dmat(c)?));
                f.push(("D", dmat(d)?));
                f.push(("K", dmat(k)?));
            }
            Structure::Proc(p) => {
                f.push(("Type", chars(e, &p.type_name())?));
                f.push(("Kp", scalar(p.kp)));
                for n in ["Tp1", "Tp2", "Tp3", "Tw", "Zeta", "Td", "Tz"] {
                    f.push((n, scalar(p.get(n))));
                }
            }
        }
        f.push(("NoiseVariance", scalar(m.noise_variance)));
        f.push(("InputDelay", scalar(self.input_delay)));
        f.push(("Ts", scalar(m.ts)));
        for (n, a) in &self.meta {
            f.push((n.as_str(), a.clone()));
        }
        Ok(Value::Container(make_struct(e, f)?))
    }
    pub fn decode(a: &Array) -> Result<Self> {
        let class = class(a).ok_or("Invalid identified model")?;
        let Array::Struct(s) = a else { unreachable!() };
        let ts = field_scalar(s, "Ts", class)?;
        if (ts.is_nan() || ts < 0.) || !ts.is_finite() {
            return Err(format!(
                "{class}: Ts must be 0 (continuous) or a positive sample time"
            ));
        }
        let vec_of = |n: &str| -> Result<Vec<f64>> {
            let m = field_real(s, n, class)?;
            if m.rows() > 1 && m.cols() > 1 {
                return Err(format!("{class}.{n} must be a row vector"));
            }
            if m.len() > MAX_ORDER + 1 + MAX_ORDER {
                return Err(format!("{class}.{n} exceeds the order bound"));
            }
            Ok(m.data().to_vec())
        };
        let monic = |n: &str| -> Result<Vec<f64>> {
            let v = vec_of(n)?;
            if v.first() != Some(&1.) {
                return Err(format!(
                    "{class}.{n} must be a monic polynomial (leading 1)"
                ));
            }
            Ok(v)
        };
        let mat = |n: &str| -> Result<DMatrix<f64>> {
            let m = field_real(s, n, class)?;
            Ok(DMatrix::from_column_slice(m.rows(), m.cols(), m.data()))
        };
        let structure = match class {
            "idpoly" => Structure::Poly {
                a: monic("A")?,
                b: vec_of("B")?,
                c: monic("C")?,
                d: monic("D")?,
                f: monic("F")?,
            },
            "idtf" => {
                let num = vec_of("Numerator")?;
                let den = vec_of("Denominator")?;
                if den.iter().all(|v| *v == 0.) {
                    return Err("idtf: the denominator cannot be zero".into());
                }
                Structure::Tf { num, den }
            }
            "idss" => {
                let (a, b, c, d, k) = (mat("A")?, mat("B")?, mat("C")?, mat("D")?, mat("K")?);
                let n = a.nrows();
                if a.ncols() != n
                    || n > 256
                    || b.nrows() != n
                    || c.ncols() != n
                    || d.nrows() != c.nrows()
                    || d.ncols() != b.ncols()
                    || k.nrows() != n
                    || k.ncols() != c.nrows()
                {
                    return Err("idss: A, B, C, D and K have inconsistent sizes".into());
                }
                Structure::Ss { a, b, c, d, k }
            }
            _ => {
                let t = get(s, "Type").ok_or("idproc: missing Type")?;
                let t = text_of(&arr_value(&t)).ok_or("idproc: Type must be text")?;
                let mut p = Proc::parse(&t)?;
                for n in ["Kp", "Tp1", "Tp2", "Tp3", "Tw", "Zeta", "Td", "Tz"] {
                    p.set(n, field_scalar(s, n, class)?);
                }
                Structure::Proc(p)
            }
        };
        let io_delay = if matches!(class, "idpoly" | "idtf") {
            field_scalar(s, "IODelay", class)?
        } else {
            0.
        };
        let input_delay = field_scalar(s, "InputDelay", class)?;
        for d in [io_delay, input_delay] {
            if (d.is_nan() || d < 0.) || !d.is_finite() || (ts > 0. && d.fract() != 0.) {
                return Err(format!(
                    "{class}: delays must be nonnegative (whole samples for discrete models)"
                ));
            }
        }
        let delay_bound = MAX_SAMPLES as f64;
        if io_delay + input_delay > delay_bound {
            return Err(format!(
                "{class}: the total delay exceeds the bound of {MAX_SAMPLES} (samples or seconds)"
            ));
        }
        let noise_variance = field_scalar(s, "NoiseVariance", class)?;
        let skip = [
            TAG,
            "A",
            "B",
            "C",
            "D",
            "F",
            "K",
            "Variable",
            "IODelay",
            "Numerator",
            "Denominator",
            "Type",
            "Kp",
            "Tp1",
            "Tp2",
            "Tp3",
            "Tw",
            "Zeta",
            "Td",
            "Tz",
            "NoiseVariance",
            "InputDelay",
            "Ts",
        ];
        let meta = s
            .field_names()
            .iter()
            .filter(|n| !skip.contains(&n.as_str()))
            .filter_map(|n| get(s, n).map(|a| (n.clone(), a)))
            .collect();
        let mut m = IdModel {
            model: Model {
                s: structure,
                ts,
                delay: 0.,
                noise_variance,
            },
            io_delay,
            input_delay,
            meta,
        };
        m.sync();
        Ok(m)
    }
}

/// Estimation report: Status, Method, Fit (FitPercent, LossFcn, MSE, FPE,
/// AIC, AICc, nAIC, BIC) and Termination.
pub(super) struct FitStats {
    pub fit: f64,
    pub loss: f64,
    pub mse: f64,
    pub n: usize,
    pub np: usize,
    pub iterations: usize,
    pub why: String,
}
pub(super) fn report(
    e: &Engine,
    method: &str,
    status: &str,
    s: Option<&FitStats>,
) -> Result<Array> {
    let nan = f64::NAN;
    let (fit, loss, mse, fpe, aic, aicc, naic, bic) = match s {
        Some(s) => {
            let n = s.n as f64;
            let d = s.np as f64;
            let v = s.loss.max(1e-300);
            let fpe = if n > d {
                v * (1. + d / n) / (1. - d / n)
            } else {
                f64::INFINITY
            };
            let base = n * (v.ln() + (2. * std::f64::consts::PI).ln() + 1.);
            let aic = base + 2. * d;
            let aicc = if n > d + 1. {
                aic + 2. * d * (d + 1.) / (n - d - 1.)
            } else {
                f64::INFINITY
            };
            let naic = v.ln() + 2. * d / n;
            let bic = base + d * n.ln();
            (s.fit, s.loss, s.mse, fpe, aic, aicc, naic, bic)
        }
        None => (nan, nan, nan, nan, nan, nan, nan, nan),
    };
    let fit_struct = make_struct(
        e,
        vec![
            ("FitPercent", scalar(fit)),
            ("LossFcn", scalar(loss)),
            ("MSE", scalar(mse)),
            ("FPE", scalar(fpe)),
            ("AIC", scalar(aic)),
            ("AICc", scalar(aicc)),
            ("nAIC", scalar(naic)),
            ("BIC", scalar(bic)),
        ],
    )?;
    let term = make_struct(
        e,
        vec![
            ("WhyStop", chars(e, s.map_or("", |s| s.why.as_str()))?),
            ("Iterations", scalar(s.map_or(0., |s| s.iterations as f64))),
        ],
    )?;
    make_struct(
        e,
        vec![
            ("Status", chars(e, status)?),
            ("Method", chars(e, method)?),
            ("Fit", Arc::new(fit_struct)),
            ("Termination", Arc::new(term)),
        ],
    )
}

// ---------------------------------------------------------------- display --
pub(super) fn g(x: f64) -> String {
    if x == 0. {
        return "0".into();
    }
    if !x.is_finite() {
        return if x.is_nan() {
            "NaN".into()
        } else if x > 0. {
            "Inf".into()
        } else {
            "-Inf".into()
        };
    }
    let e = x.abs().log10().floor() as i32;
    if !(-5..6).contains(&e) {
        let s = format!("{:.3e}", x);
        let (m, ex) = s.split_once('e').unwrap();
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let ex: i32 = ex.parse().unwrap_or(0);
        return format!("{m}e{}{:02}", if ex < 0 { '-' } else { '+' }, ex.abs());
    }
    let decimals = (3 - e).max(0) as usize;
    let s = format!("{:.*}", decimals, x);
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}
fn poly_text(p: &[f64], var: &str) -> String {
    let mut out = String::new();
    for (i, &c) in p.iter().enumerate() {
        if c == 0. {
            continue;
        }
        let mag = c.abs();
        let term = if i == 0 {
            g(mag)
        } else if mag == 1. {
            format!("{var}^-{i}")
        } else {
            format!("{} {var}^-{i}", g(mag))
        };
        if out.is_empty() {
            out = if c < 0. { format!("-{term}") } else { term };
        } else {
            out.push_str(if c < 0. { " - " } else { " + " });
            out.push_str(&term);
        }
    }
    if out.is_empty() { "0".into() } else { out }
}
fn desc_text(p: &[f64], var: &str) -> String {
    let p = super::lti::trim(p);
    let n = p.len().saturating_sub(1);
    let mut out = String::new();
    for (i, &c) in p.iter().enumerate() {
        if c == 0. {
            continue;
        }
        let k = n - i;
        let mag = c.abs();
        let pw = match k {
            0 => String::new(),
            1 => var.to_string(),
            _ => format!("{var}^{k}"),
        };
        let term = if k == 0 {
            g(mag)
        } else if mag == 1. {
            pw
        } else {
            format!("{} {pw}", g(mag))
        };
        if out.is_empty() {
            out = if c < 0. { format!("-{term}") } else { term };
        } else {
            out.push_str(if c < 0. { " - " } else { " + " });
            out.push_str(&term);
        }
    }
    if out.is_empty() { "0".into() } else { out }
}
fn status_text(m: &IdModel) -> String {
    let Some(Array::Struct(r)) = m.meta("Report").map(|a| a.as_ref()) else {
        return String::new();
    };
    let status = get(r, "Status")
        .and_then(|a| text_of(&arr_value(&a)))
        .unwrap_or_default();
    let mut s = format!("Status:\n{status}\n");
    if let Some(Array::Struct(fs)) = get(r, "Fit").as_deref()
        && let Some(f) = get(fs, "FitPercent")
        && let Ok(f) = scalar_of(&arr_value(&f), "FitPercent")
        && f.is_finite()
    {
        let fpe = get(fs, "FPE")
            .and_then(|a| scalar_of(&arr_value(&a), "FPE").ok())
            .unwrap_or(f64::NAN);
        let mse = get(fs, "MSE")
            .and_then(|a| scalar_of(&arr_value(&a), "MSE").ok())
            .unwrap_or(f64::NAN);
        s.push_str(&format!(
            "Fit to estimation data: {}%\nFPE: {}, MSE: {}\n",
            g(f),
            g(fpe),
            g(mse)
        ));
    }
    s
}
pub(super) fn display(a: &Array) -> Result<Option<String>> {
    let Some(class) = class(a) else {
        return Ok(None);
    };
    if class == "iddata" {
        let d = IdData::decode(a)?;
        let mut s = format!(
            "Time domain data set with {} samples.\nSample time: {} seconds\n",
            d.samples(),
            g(d.ts)
        );
        let names = |n: &str, k: usize, p: &str| -> Vec<String> {
            let from = d.meta(n).and_then(|a| match a.as_ref() {
                Array::Cell(c) if c.len() == k => (0..k)
                    .map(|i| c.get_linear(i).ok().and_then(|x| text_of(&arr_value(x))))
                    .collect::<Option<Vec<_>>>(),
                _ => None,
            });
            from.unwrap_or_else(|| (1..=k).map(|i| format!("{p}{i}")).collect())
        };
        s.push_str("\nOutputs\n");
        for n in names("OutputName", d.ny(), "y") {
            s.push_str(&format!("   {n}\n"));
        }
        if d.nu() > 0 {
            s.push_str("\nInputs\n");
            for n in names("InputName", d.nu(), "u") {
                s.push_str(&format!("   {n}\n"));
            }
        }
        return Ok(Some(s));
    }
    let m = IdModel::decode(a)?;
    let ts = m.model.ts;
    let mut s = String::new();
    let sample = if ts > 0. {
        format!("Sample time: {} seconds\n", g(ts))
    } else {
        String::new()
    };
    match &m.model.s {
        Structure::Poly { a, b, c, d, f } => {
            let kind = match (a.len() > 1, c.len() > 1, d.len() > 1, f.len() > 1) {
                (_, false, false, false) => "ARX",
                (_, true, false, false) => "ARMAX",
                (false, false, false, true) => "OE",
                (false, _, _, true) => "BJ",
                _ => "polynomial",
            };
            s.push_str(&format!("Discrete-time {kind} model: "));
            s.push_str(match kind {
                "ARX" => "A(z)y(t) = B(z)u(t) + e(t)\n",
                "ARMAX" => "A(z)y(t) = B(z)u(t) + C(z)e(t)\n",
                "OE" => "y(t) = [B(z)/F(z)]u(t) + e(t)\n",
                "BJ" => "y(t) = [B(z)/F(z)]u(t) + [C(z)/D(z)]e(t)\n",
                _ => "A(z)y(t) = [B(z)/F(z)]u(t) + [C(z)/D(z)]e(t)\n",
            });
            for (n, p, show) in [
                (
                    "A",
                    a,
                    a.len() > 1 || kind == "ARX" || kind == "ARMAX" || kind == "polynomial",
                ),
                ("B", b, true),
                ("C", c, c.len() > 1),
                ("D", d, d.len() > 1),
                ("F", f, f.len() > 1),
            ] {
                if show {
                    s.push_str(&format!("  {n}(z) = {}\n", poly_text(p, "z")));
                }
            }
            s.push('\n');
            s.push_str(&sample);
        }
        Structure::Tf { num, den } => {
            let v = if ts > 0. { "z" } else { "s" };
            s.push_str(&format!(
                "{} identified transfer function:\n  {}\n  ----\n  {}\n",
                if ts > 0. {
                    "Discrete-time"
                } else {
                    "Continuous-time"
                },
                desc_text(num, v),
                desc_text(den, v)
            ));
            if m.io_delay > 0. {
                s.push_str(&format!("IO delay: {}\n", g(m.io_delay)));
            }
            s.push_str(&sample);
        }
        Structure::Ss { a, .. } => {
            s.push_str(&format!(
                "{} identified state-space model with {} states:\n  dx/dt = A x(t) + B u(t) + K e(t)\n  y(t) = C x(t) + D u(t) + e(t)\nUse m.A, m.B, m.C, m.D and m.K for the matrices.\n",
                if ts > 0. { "Discrete-time" } else { "Continuous-time" },
                a.nrows()
            ));
            s.push_str(&sample);
        }
        Structure::Proc(p) => {
            s.push_str(&format!(
                "Process model with transfer function ({}):\n",
                p.type_name()
            ));
            for n in p.names() {
                s.push_str(&format!("  {n} = {}\n", g(p.get(n))));
            }
        }
    }
    s.push('\n');
    s.push_str(&status_text(&m));
    Ok(Some(s))
}
