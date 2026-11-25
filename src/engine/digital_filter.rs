//! `designfilt` and the immutable `digitalFilter` objects it returns.
//! FIR responses use the windowed (Hamming) `fir1` design, IIR responses the
//! Butterworth / Chebyshev II designs of `butter` / `cheby2`, as the published
//! designfilt pages list for these specification sets. Coefficients are a row
//! vector (FIR) or an L-by-6 SOS matrix with the gain in the first section (IIR).
//! `dfilt.df1(b, a)` stores the same coefficient object from a numerator and a
//! denominator vector. It does not design from a cutoff.
use super::{Engine, Value};
use crate::dsp::{
    Band, FirOptions, fir1_with_options,
    iir::{IirBand, IirKind},
};
use crate::{Matrix, Result};

#[derive(Clone, Debug)]
pub(super) struct DigitalFilter {
    pub response: &'static str,
    pub fir: bool,
    pub method: &'static str,
    pub order: usize,
    pub sample_rate: Option<f64>,
    /// Specification name/value pairs in the order given (frequencies in the user's units).
    pub specs: Vec<(&'static str, f64)>,
    pub b: Vec<f64>,
    pub a: Vec<f64>,
    pub sos: Option<Matrix>,
}

const RESPONSES: &[&str] = &[
    "lowpassfir",
    "highpassfir",
    "bandpassfir",
    "bandstopfir",
    "lowpassiir",
    "highpassiir",
    "bandpassiir",
    "bandstopiir",
];
const SPECS: &[&str] = &[
    "FilterOrder",
    "CutoffFrequency",
    "CutoffFrequency1",
    "CutoffFrequency2",
    "HalfPowerFrequency",
    "HalfPowerFrequency1",
    "HalfPowerFrequency2",
    "StopbandFrequency",
    "StopbandFrequency1",
    "StopbandFrequency2",
    "StopbandAttenuation",
    "SampleRate",
];
/// Builtins that accept a digitalFilter in place of `b, a`.
pub(super) const FILTER_FUNCTIONS: &[&str] =
    &["filter", "filtfilt", "freqz", "impz", "grpdelay", "phasez"];

impl DigitalFilter {
    pub fn class(&self) -> &'static str {
        "digitalFilter"
    }
    pub fn bytes(&self) -> usize {
        96 + 8 * (self.b.len() + self.a.len() + self.specs.len() * 2)
            + self.sos.as_ref().map_or(0, |s| s.len() * 8)
    }
    fn band_name(&self) -> &'static str {
        &self.response[..self.response.len() - 3]
    }
    pub fn property_names(&self) -> Vec<String> {
        let mut names = vec![
            "FrequencyResponse".to_string(),
            "ImpulseResponse".into(),
            "DesignMethod".into(),
            "Coefficients".into(),
        ];
        names.extend(self.specs.iter().map(|(n, _)| n.to_string()));
        if !self.specs.iter().any(|(n, _)| *n == "SampleRate") {
            names.push("SampleRate".into());
        }
        names
    }
    pub fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        let _ = e;
        let text = |s: &str| -> Result<Value> { Ok(Value::Text(s.into())) };
        match name {
            "FrequencyResponse" => text(self.band_name()),
            "ImpulseResponse" => text(if self.fir { "fir" } else { "iir" }),
            "DesignMethod" => text(self.method),
            "Coefficients" => Ok(match &self.sos {
                Some(s) => Value::Matrix(s.clone()),
                None => Value::Matrix(Matrix::from_column_vec(1, self.b.len(), self.b.clone())?),
            }),
            "SampleRate" => Ok(Value::Matrix(Matrix::scalar(
                self.sample_rate.unwrap_or(2.),
            ))),
            _ => match self.specs.iter().find(|(n, _)| *n == name) {
                Some((_, v)) => Ok(Value::Matrix(Matrix::scalar(*v))),
                None => Err(format!(
                    "Unrecognized property '{name}' for class 'digitalFilter'"
                )),
            },
        }
    }
    pub fn body(&self) -> String {
        let coeffs = match &self.sos {
            Some(s) => format!("[{}\u{d7}6 double]", s.rows()),
            None => format!("[1\u{d7}{} double]", self.b.len()),
        };
        let mut rows: Vec<(String, String)> = vec![("Coefficients".into(), coeffs)];
        rows.push((
            "FrequencyResponse".into(),
            format!("'{}'", self.band_name()),
        ));
        rows.push((
            "ImpulseResponse".into(),
            format!("'{}'", if self.fir { "fir" } else { "iir" }),
        ));
        rows.push(("SampleRate".into(), short(self.sample_rate.unwrap_or(2.))));
        for (n, v) in &self.specs {
            if *n != "SampleRate" {
                rows.push((n.to_string(), short(*v)));
            }
        }
        rows.push(("DesignMethod".into(), format!("'{}'", self.method)));
        let width = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(0) + 4;
        let mut s = String::from("  digitalFilter with properties:\n\n");
        for (n, v) in rows {
            s.push_str(&format!("{n:>width$}: {v}\n"));
        }
        s
    }
}

fn short(v: f64) -> String {
    if v.fract() == 0. && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{:.4}", v).trim_end_matches('0').to_string()
    }
}

fn number(v: &Value, what: &str) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 && m.data()[0].is_finite() => Ok(m.data()[0]),
        _ => Err(format!("designfilt: {what} must be a finite real scalar")),
    }
}

/// Real nonempty row or column. A matrix is rejected, not flattened: a
/// column-major reshape would be a different filter. No default coefficients.
fn coefficient_vector(v: &Value, what: &str) -> Result<Vec<f64>> {
    let Value::Matrix(m) = v else {
        return Err(format!("dfilt.df1: {what} must be a nonempty real vector"));
    };
    if m.is_empty() || m.is_nd() || (m.rows() > 1 && m.cols() > 1) {
        return Err(format!(
            "dfilt.df1: {what} must be a nonempty vector; a matrix is not reshaped into coefficients"
        ));
    }
    if m.data().len() > crate::dsp::MAX_ORDER + 1 {
        return Err(format!(
            "dfilt.df1: {what} is longer than {} coefficients",
            crate::dsp::MAX_ORDER + 1
        ));
    }
    if m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("dfilt.df1: {what} must be finite"));
    }
    Ok(m.data().to_vec())
}

/// Index of the last nonzero coefficient. Exact zeros add no degree.
fn coefficient_degree(c: &[f64]) -> usize {
    c.iter().rposition(|v| *v != 0.).unwrap_or(0)
}

/// True when every denominator coefficient after the leading one is an exact
/// zero. Those trailing zeros are not a feedback term. An SOS cascade is IIR.
pub(super) fn is_fir(f: &DigitalFilter) -> bool {
    f.sos.is_none() && f.a.iter().skip(1).all(|v| *v == 0.)
}

/// `H = dfilt.df1(numerator, denominator)`. Direct form I on the coefficient
/// object `filter` and `freqz` already use. Both arguments are required.
pub(super) fn df1(args: &[Value]) -> Result<DigitalFilter> {
    if args.len() != 2 {
        return Err(if args.len() < 2 {
            "dfilt.df1: numerator and denominator are required".into()
        } else {
            "dfilt.df1: too many input arguments".into()
        });
    }
    let b = coefficient_vector(&args[0], "numerator")?;
    let a = coefficient_vector(&args[1], "denominator")?;
    if a.first().is_none_or(|v| *v == 0.) {
        return Err("dfilt.df1: the leading denominator coefficient must be nonzero".into());
    }
    let fir = a.iter().skip(1).all(|v| *v == 0.);
    Ok(DigitalFilter {
        response: if fir { "df1fir" } else { "df1iir" },
        fir,
        method: "df1",
        order: coefficient_degree(&b).max(coefficient_degree(&a)),
        sample_rate: None,
        specs: Vec::new(),
        b,
        a,
        sos: None,
    })
}

/// `d = designfilt(response, Name, Value, ...)`.
pub(super) fn designfilt(e: &mut Engine, args: &[Value]) -> Result<DigitalFilter> {
    let Some(first) = args.first() else {
        return Err("designfilt: expected a response type such as 'lowpassfir'".into());
    };
    let response = e.text_value(first.clone())?.to_ascii_lowercase();
    let Some(&response) = RESPONSES.iter().find(|r| **r == response) else {
        return Err(format!(
            "designfilt: response '{response}' is not supported; use one of {}",
            RESPONSES.join(", ")
        ));
    };
    if args.len().is_multiple_of(2) {
        return Err("designfilt: name/value arguments must come in pairs".into());
    }
    let mut specs: Vec<(&'static str, f64)> = Vec::new();
    let mut method: Option<String> = None;
    for pair in args[1..].chunks(2) {
        let name = e.text_value(pair[0].clone())?;
        if name.eq_ignore_ascii_case("DesignMethod") {
            method = Some(e.text_value(pair[1].clone())?.to_ascii_lowercase());
            continue;
        }
        let Some(&key) = SPECS.iter().find(|s| s.eq_ignore_ascii_case(&name)) else {
            return Err(format!("designfilt: option '{name}' is not supported"));
        };
        let value = number(&pair[1], key)?;
        specs.retain(|(n, _)| *n != key);
        specs.push((key, value));
    }
    let get = |k: &str| specs.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
    let order = get("FilterOrder").ok_or(
        "designfilt: FilterOrder is required (other specification sets are not supported)",
    )?;
    if order < 1. || order.fract() != 0. || order > 4096. {
        return Err("designfilt: FilterOrder must be an integer from 1 to 4096".into());
    }
    let order = order as usize;
    let fs = get("SampleRate");
    if let Some(fs) = fs
        && fs <= 0.
    {
        return Err("designfilt: SampleRate must be positive".into());
    }
    let nyquist = fs.map_or(1., |f| f / 2.);
    let fir = response.ends_with("fir");
    let bandish = response.starts_with("bandpass") || response.starts_with("bandstop");
    let norm = |keys: &[&str]| -> Result<Vec<f64>> {
        keys.iter()
            .map(|k| {
                let f = get(k)
                    .ok_or_else(|| format!("designfilt: {k} is required for '{response}'"))?;
                let w = f / nyquist;
                if !(w > 0. && w < 1.) {
                    return Err(format!(
                        "designfilt: {k} must lie strictly between 0 and the Nyquist frequency"
                    ));
                }
                Ok(w)
            })
            .collect()
    };
    let allowed = |extra: &[&str]| -> Result<()> {
        for (n, _) in &specs {
            if !matches!(*n, "FilterOrder" | "SampleRate") && !extra.contains(n) {
                return Err(format!(
                    "designfilt: {n} cannot be combined with this '{response}' specification"
                ));
            }
        }
        Ok(())
    };
    let check_method = |want: &str| -> Result<&'static str> {
        let m = method.as_deref().unwrap_or(want);
        let known: &'static str = match m {
            "window" => "window",
            "butter" => "butter",
            "cheby2" => "cheby2",
            _ => {
                return Err(format!(
                    "designfilt: DesignMethod '{m}' is not supported here"
                ));
            }
        };
        if known != want {
            return Err(format!(
                "designfilt: DesignMethod '{m}' does not match this specification set (expected '{want}')"
            ));
        }
        Ok(known)
    };
    if fir {
        let keys: &[&str] = if bandish {
            &["CutoffFrequency1", "CutoffFrequency2"]
        } else {
            &["CutoffFrequency"]
        };
        allowed(keys)?;
        let method = check_method("window")?;
        let w = norm(keys)?;
        if bandish && w[0] >= w[1] {
            return Err("designfilt: CutoffFrequency1 must be below CutoffFrequency2".into());
        }
        let band = match &response[..response.len() - 3] {
            "lowpass" => Band::Lowpass,
            "highpass" => Band::Highpass,
            "bandpass" => Band::Bandpass,
            _ => Band::Bandstop,
        };
        if matches!(band, Band::Highpass | Band::Bandstop) && order % 2 == 1 {
            return Err(format!(
                "designfilt: '{response}' needs an even FilterOrder (type I linear phase)"
            ));
        }
        let b = fir1_with_options(order, &w, band, FirOptions::default(), &e.cancel)?;
        return Ok(DigitalFilter {
            response,
            fir,
            method,
            order,
            sample_rate: fs,
            specs,
            b,
            a: vec![1.],
            sos: None,
        });
    }
    if bandish && order % 2 == 1 {
        return Err(format!(
            "designfilt: '{response}' needs an even FilterOrder"
        ));
    }
    let design_order = if bandish { order / 2 } else { order };
    let (kind, rs, keys, method): (IirKind, f64, &[&str], &'static str) =
        if get("StopbandAttenuation").is_some() {
            let keys: &[&str] = if bandish {
                &[
                    "StopbandFrequency1",
                    "StopbandFrequency2",
                    "StopbandAttenuation",
                ]
            } else {
                &["StopbandFrequency", "StopbandAttenuation"]
            };
            let rs = get("StopbandAttenuation").unwrap_or(0.);
            if rs <= 0. {
                return Err("designfilt: StopbandAttenuation must be positive".into());
            }
            (IirKind::Cheby2, rs, keys, check_method("cheby2")?)
        } else {
            let keys: &[&str] = if bandish {
                &["HalfPowerFrequency1", "HalfPowerFrequency2"]
            } else {
                &["HalfPowerFrequency"]
            };
            (IirKind::Butter, 0., keys, check_method("butter")?)
        };
    allowed(keys)?;
    let freq_keys: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|k| *k != "StopbandAttenuation")
        .collect();
    let w = norm(&freq_keys)?;
    if bandish && w[0] >= w[1] {
        return Err("designfilt: the first band edge must be below the second".into());
    }
    let band = match &response[..response.len() - 3] {
        "lowpass" => IirBand::Low,
        "highpass" => IirBand::High,
        "bandpass" => IirBand::Bandpass,
        _ => IirBand::Stop,
    };
    let z = super::iir::design_zpk(e, kind, design_order, rs, &w, band)?;
    let (b, a) = crate::dsp::iir::zpk2tf(&z)?;
    let sos = crate::dsp::iir::zpk2sos(&z)?;
    Ok(DigitalFilter {
        response,
        fir,
        method,
        order,
        sample_rate: fs,
        specs,
        b,
        a,
        sos: Some(sos),
    })
}

/// `b, a` as row vectors (what `[b, a] = tf(d)` returns).
pub(super) fn expand_args(d: &DigitalFilter, rest: &[Value]) -> Result<Vec<Value>> {
    let row = |v: &[f64]| Matrix::from_column_vec(1, v.len(), v.to_vec()).map(Value::Matrix);
    let mut args = vec![row(&d.b)?, row(&d.a)?];
    args.extend_from_slice(rest);
    Ok(args)
}

/// The builtin and arguments `name(d, rest...)` runs as. IIR filters stay in
/// second-order sections (sosfilt, filtfilt(sos,1,x), sosfreqz, ...): expanding a
/// high-order or narrow-band design to `b, a` is numerically unstable.
/// The analysis builtins work in normalised frequency, so the sample rate is not passed.
pub(super) fn route(
    d: &DigitalFilter,
    name: &str,
    rest: &[Value],
) -> Result<(&'static str, Vec<Value>)> {
    let Some(sos) = &d.sos else {
        let callee = FILTER_FUNCTIONS
            .iter()
            .copied()
            .find(|f| *f == name)
            .ok_or_else(|| format!("{name}: unsupported operation on digitalFilter"))?;
        return Ok((callee, expand_args(d, rest)?));
    };
    let sos = Value::Matrix(sos.clone());
    let (callee, mut args): (&'static str, Vec<Value>) = match name {
        "filter" => ("sosfilt", vec![sos]),
        "filtfilt" => ("filtfilt", vec![sos, Value::Matrix(Matrix::scalar(1.))]),
        "freqz" => ("sosfreqz", vec![sos]),
        "grpdelay" => ("sosgrpdelay", vec![sos]),
        "phasez" => ("sosphasez", vec![sos]),
        "impz" => ("sosimpz", vec![sos]),
        _ => return Err(format!("{name}: unsupported operation on digitalFilter")),
    };
    args.extend_from_slice(rest);
    Ok((callee, args))
}
