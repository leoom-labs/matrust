//! The `cwtfilterbank` value object and its methods (wt, freqz, wavelets,
//! scales, centerFrequencies, centerPeriods). It is a tagged 1-by-1 struct of
//! read-only public properties: every method decodes and validates the
//! properties again and derives the scales from them.
use super::super::{Engine, Value, lang_more};
use super::cwt::{self, Config};
use super::cwt_kernel::{self as kernel, Wavelet};
use crate::{Array, CharacterRow, Matrix, Result, StructArray};
use std::sync::Arc;

/// Value of the language-object tag field (named by lang_more::lng_tag).
pub(in crate::engine) const TAG: f64 = 71.;

pub(super) const METHODS: &[&str] = &[
    "wt",
    "freqz",
    "wavelets",
    "scales",
    "centerFrequencies",
    "centerPeriods",
];

pub(super) fn is_bank_array(a: &Array) -> bool {
    let Array::Struct(s) = a else { return false };
    s.len() == 1
        && s.field_index(lang_more::TAG)
            .and_then(|f| s.get(f, 0).ok())
            .is_some_and(|v| matches!(v.as_ref(), Array::Numeric(m) if m.data() == [TAG]))
}

pub(super) fn is_bank(v: &Value) -> bool {
    matches!(v, Value::Container(a) if is_bank_array(a))
}

/// A method call: one of the method names with a filter bank first.
pub(super) fn is_method(name: &str, args: &[Value]) -> bool {
    METHODS.contains(&name) && args.first().is_some_and(is_bank)
}

fn chars(e: &Engine, s: &str) -> Result<Arc<Array>> {
    Ok(Arc::new(CharacterRow::from_text(s, &e.cancel)?.into()))
}

fn numbers(v: &[f64]) -> Result<Arc<Array>> {
    let rows = usize::from(!v.is_empty());
    Ok(Arc::new(
        Matrix::from_column_slice(rows, v.len(), v)?.into(),
    ))
}

pub(super) fn encode(e: &Engine, c: &Config) -> Result<Value> {
    let (time_bandwidth, parameters) = match c.wavelet {
        Wavelet::Morse { gamma, beta } if c.parameters => (gamma * beta, vec![gamma, gamma * beta]),
        Wavelet::Morse { gamma, beta } => (gamma * beta, Vec::new()),
        _ => (60., Vec::new()),
    };
    let limits = c.limits.map(|(a, b)| vec![a, b]).unwrap_or_default();
    let fields = [
        ("VoicesPerOctave", numbers(&[c.nv as f64])?),
        ("Wavelet", chars(e, c.wavelet.name())?),
        ("SamplingFrequency", numbers(&[c.fs])?),
        ("SamplingPeriod", numbers(&[])?),
        ("PeriodLimits", numbers(&[])?),
        ("SignalLength", numbers(&[c.n as f64])?),
        ("FrequencyLimits", numbers(&limits)?),
        ("TimeBandwidth", numbers(&[time_bandwidth])?),
        ("WaveletParameters", numbers(&parameters)?),
        (
            "Boundary",
            chars(e, if c.extend { "reflection" } else { "periodic" })?,
        ),
        (lang_more::TAG, numbers(&[TAG])?),
    ];
    let (names, slots) = fields.into_iter().map(|(n, a)| (n.to_string(), a)).unzip();
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?)))
}

pub(super) fn decode(v: &Value, who: &str) -> Result<Config> {
    let Value::Container(a @ Array::Struct(s)) = v else {
        return Err(format!("{who}: expected a cwtfilterbank object"));
    };
    if !is_bank_array(a) {
        return Err(format!("{who}: expected a cwtfilterbank object"));
    }
    let field = |name: &str| -> Result<Value> {
        let index = s
            .field_index(name)
            .ok_or_else(|| format!("{who}: invalid cwtfilterbank object (no {name})"))?;
        Ok(Value::from(s.get(index, 0)?.as_ref().clone()))
    };
    let empty = |v: &Value| cwt::reals(v).is_some_and(|r| r.0.is_empty());
    let text = |name: &str| -> Result<String> {
        match field(name)? {
            Value::Container(Array::Character(c)) => Ok(String::from_utf16_lossy(c.units())),
            Value::Text(t) | Value::QuotedString(t) => Ok(t),
            _ => Err(format!("{who}: invalid cwtfilterbank object ({name})")),
        }
    };
    let name = text("Wavelet")?;
    let parameters = field("WaveletParameters")?;
    let morse = name.eq_ignore_ascii_case("morse");
    let (tb, params) = match (morse, empty(&parameters)) {
        (false, _) => (None, None),
        (true, true) => (
            Some(cwt::time_bandwidth(&field("TimeBandwidth")?, who)?),
            None,
        ),
        (true, false) => (None, Some(cwt::wavelet_parameters(&parameters, who)?)),
    };
    let limits = field("FrequencyLimits")?;
    Ok(Config {
        n: signal_length(&field("SignalLength")?, who)?,
        wavelet: cwt::wavelet(Some(&name), tb, params, who)?,
        nv: cwt::voices(&field("VoicesPerOctave")?, who)?,
        fs: cwt::positive(&field("SamplingFrequency")?, who, "SamplingFrequency")?,
        limits: if empty(&limits) {
            None
        } else {
            Some(cwt::frequency_limits(&limits, who)?)
        },
        extend: boundary(&text("Boundary")?, who)?,
        parameters: params.is_some(),
    })
}

fn signal_length(v: &Value, who: &str) -> Result<usize> {
    let max = crate::matrix::MAX_ELEMENTS;
    match cwt::scalar(v) {
        Some(x) if x >= 4. && x.fract() == 0. && x <= max as f64 => Ok(x as usize),
        _ => Err(format!(
            "{who}: SignalLength must be an integer between 4 and {max}"
        )),
    }
}

fn boundary(name: &str, who: &str) -> Result<bool> {
    match name.to_ascii_lowercase().as_str() {
        "reflection" => Ok(true),
        "periodic" => Ok(false),
        _ => Err(format!(
            "{who}: Boundary must be 'reflection' or 'periodic'"
        )),
    }
}

/// `fb = cwtfilterbank(Name, Value, ...)`.
pub(super) fn create(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "cwtfilterbank";
    cwt::too_many(nargout, 1, WHO)?;
    let (mut n, mut name, mut nv, mut fs) = (1024, None, 10, 1.);
    let (mut limits, mut tb, mut params, mut extend) = (None, None, None, true);
    for (key, written, value) in cwt::pairs(args, WHO)? {
        match key.as_str() {
            "signallength" => n = signal_length(value, WHO)?,
            "wavelet" => {
                name = Some(
                    cwt::text(value)
                        .ok_or("cwtfilterbank: Wavelet must be 'Morse', 'amor' or 'bump'")?,
                );
            }
            "voicesperoctave" => nv = cwt::voices(value, WHO)?,
            "samplingfrequency" => fs = cwt::positive(value, WHO, "SamplingFrequency")?,
            "frequencylimits" => limits = Some(cwt::frequency_limits(value, WHO)?),
            "timebandwidth" => tb = Some(cwt::time_bandwidth(value, WHO)?),
            "waveletparameters" => params = Some(cwt::wavelet_parameters(value, WHO)?),
            "boundary" => {
                extend = boundary(
                    cwt::text(value)
                        .ok_or("cwtfilterbank: Boundary must be 'reflection' or 'periodic'")?,
                    WHO,
                )?;
            }
            "samplingperiod" | "periodlimits" => {
                return Err(format!(
                    "cwtfilterbank: {written} (duration) is not supported yet; use \
                     SamplingFrequency and FrequencyLimits"
                ));
            }
            _ => return Err(format!("cwtfilterbank: unknown option '{written}'")),
        }
    }
    let config = Config {
        n,
        wavelet: cwt::wavelet(name, tb, params, WHO)?,
        nv,
        fs,
        limits,
        extend,
        parameters: params.is_some(),
    };
    let (scales, _) = config.grid(WHO)?;
    // Admit the requested rows-by-signal-length filter matrix even though
    // this value object stores only its configuration and builds filters lazily.
    // Otherwise an impossible bank can be constructed and fail only on use.
    let elements = Matrix::checked_len(scales.len(), config.n)?;
    e.preflight_elements(elements)?;
    Ok(vec![encode(e, &config)?])
}

fn no_more(args: &[Value], who: &str) -> Result<()> {
    if args.len() > 1 {
        Err(format!("{who}: too many input arguments"))
    } else {
        Ok(())
    }
}

pub(super) fn method(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    let config = decode(&args[0], name)?;
    match name {
        "wt" => {
            if nargout == 4 {
                return Err("wt: the scaling coefficients output is not supported yet".into());
            }
            cwt::too_many(nargout, 3, name)?;
            let [_, x] = args else {
                return Err("wt: expected wt(fb, x)".into());
            };
            let x = cwt::signal(x, name)?;
            if x.len() != config.n {
                return Err(format!(
                    "wt: the signal length must equal the SignalLength of the filter bank ({})",
                    config.n
                ));
            }
            let (wt, freqs, coi) = config.transform(e, &x, name)?;
            let mut out = vec![wt];
            if nargout > 1 {
                out.push(cwt::column(freqs)?);
            }
            if nargout > 2 {
                out.push(cwt::column(coi)?);
            }
            Ok(out)
        }
        "freqz" => {
            cwt::too_many(nargout, 2, name)?;
            let mut twosided = false;
            for (key, written, value) in cwt::pairs(&args[1..], name)? {
                match key.as_str() {
                    "frequencyrange" => {
                        twosided = match cwt::text(value).map(str::to_ascii_lowercase).as_deref() {
                            Some("onesided") => false,
                            Some("twosided") => true,
                            _ => {
                                return Err(
                                    "freqz: FrequencyRange must be 'onesided' or 'twosided'".into(),
                                );
                            }
                        }
                    }
                    "includelowpass" => {
                        if cwt::flag(value, name, "IncludeLowpass")? {
                            return Err(
                                "freqz: the lowpass (scaling) filter is not supported yet".into()
                            );
                        }
                    }
                    _ => return Err(format!("freqz: unknown option '{written}'")),
                }
            }
            let (scales, _) = config.grid(name)?;
            // The DFT grid the transform filters on (the reflected signal).
            let m = cwt::reflected_len(config.n, config.extend);
            let cols = if twosided { m } else { m / 2 + 1 };
            let elems = Matrix::checked_len(scales.len(), cols)?;
            e.preflight_elements(elems)?;
            e.check_cancel()?;
            let data = kernel::responses(&config.wavelet, &scales, m, twosided);
            let mut out = vec![Value::Matrix(Matrix::from_column_vec(
                scales.len(),
                cols,
                data,
            )?)];
            if nargout > 1 {
                out.push(cwt::row(
                    (0..cols).map(|k| k as f64 * config.fs / m as f64).collect(),
                )?);
            }
            Ok(out)
        }
        "wavelets" => {
            cwt::too_many(nargout, 2, name)?;
            no_more(args, name)?;
            let (scales, _) = config.grid(name)?;
            let (na, n) = (scales.len(), config.n);
            let elems = na
                .checked_mul(n)
                .ok_or("wavelets: the result size overflows")?;
            // The result plus the length-n spectrum the inverse DFT starts from.
            let units = elems
                .checked_mul(2)
                .and_then(|units| {
                    n.checked_mul(2)
                        .and_then(|spectrum| units.checked_add(spectrum))
                })
                .ok_or("wavelets: the result size overflows")?;
            e.preflight_elements(units)?;
            if elems > kernel::MAX_ELEMENTS {
                return Err(format!(
                    "wavelets: the {na}-by-{n} result exceeds {} elements",
                    kernel::MAX_ELEMENTS
                ));
            }
            let data = kernel::wavelets(&config.wavelet, &scales, n, &e.cancel)?;
            let mut out = vec![Value::Complex(cwt::complex(e, na, n, data)?)];
            if nargout > 1 {
                out.push(cwt::row(
                    (0..n)
                        .map(|i| (i as f64 - (n / 2) as f64) / config.fs)
                        .collect(),
                )?);
            }
            Ok(out)
        }
        _ => {
            cwt::too_many(nargout, 1, name)?;
            no_more(args, name)?;
            let (scales, freqs) = config.grid(name)?;
            Ok(vec![match name {
                "scales" => cwt::row(scales)?,
                "centerFrequencies" => cwt::column(freqs)?,
                _ => cwt::column(freqs.iter().map(|f| 1. / f).collect())?,
            }])
        }
    }
}
