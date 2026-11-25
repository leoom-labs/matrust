//! Continuous wavelet transform builtins: cwt, icwt, cwtfilterbank (with its
//! wt, freqz, wavelets and scales methods), wsst and iwsst. Argument handling
//! and output shapes follow the public MathWorks function documentation; the
//! numerics are in cwt_kernel.rs. Expected values in tests/wavelet_cwt.rs come
//! from the NumPy reference tool/refs/wavelet_cwt/wavelet_cwt.py.
use super::super::{Engine, Value};
use super::cwt_bank;
use super::cwt_kernel::{self as kernel, Samples, Wavelet};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use std::f64::consts::LN_2;

pub(super) const NAMES: &[&str] = &["cwt", "icwt", "cwtfilterbank", "wsst", "iwsst"];

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    if cwt_bank::is_method(name, args) {
        return cwt_bank::method(e, name, args, nargout);
    }
    match name {
        "cwt" => cwt(e, args, nargout),
        "icwt" => icwt(e, args, nargout),
        "cwtfilterbank" => cwt_bank::create(e, args, nargout),
        "wsst" => wsst(e, args, nargout),
        "iwsst" => iwsst(e, args, nargout),
        _ => Err(format!("Undefined function '{name}'")),
    }
}

// ---------- argument helpers ----------

pub(super) fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

/// A real double or logical array as (values, rows, columns); None for
/// anything else (text, complex, N-D, containers).
pub(super) fn reals(v: &Value) -> Option<(Vec<f64>, usize, usize)> {
    match v {
        Value::Matrix(m) if !m.is_nd() => Some((m.data().to_vec(), m.rows(), m.cols())),
        Value::Logical(l) if !l.is_nd() => Some((
            l.data().iter().map(|b| f64::from(*b)).collect(),
            l.rows(),
            l.cols(),
        )),
        _ => None,
    }
}

pub(super) fn scalar(v: &Value) -> Option<f64> {
    match reals(v)?.0.as_slice() {
        [x] => Some(*x),
        _ => None,
    }
}

pub(super) fn is_duration(v: &Value) -> bool {
    matches!(v, Value::Container(Array::Duration(_)))
}

pub(super) fn too_many(nargout: usize, max: usize, who: &str) -> Result<()> {
    if nargout > max {
        Err(format!("{who}: too many output arguments"))
    } else {
        Ok(())
    }
}

/// Trailing Name, Value pairs as (lower-case name, name as written, value).
pub(super) fn pairs<'a>(rest: &'a [Value], who: &str) -> Result<Vec<(String, &'a str, &'a Value)>> {
    let mut found = Vec::with_capacity(rest.len() / 2);
    let mut i = 0;
    while i < rest.len() {
        let Some(name) = text(&rest[i]) else {
            return Err(format!(
                "{who}: expected an option name, got a {} argument",
                kind(&rest[i])
            ));
        };
        let Some(value) = rest.get(i + 1) else {
            return Err(format!("{who}: option '{name}' needs a value"));
        };
        found.push((name.to_ascii_lowercase(), name, value));
        i += 2;
    }
    Ok(found)
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Matrix(_) => "numeric",
        Value::Logical(_) => "logical",
        Value::Complex(_) => "complex",
        Value::Text(_) | Value::QuotedString(_) => "text",
        _ => "non-numeric",
    }
}

pub(super) fn flag(v: &Value, who: &str, what: &str) -> Result<bool> {
    match scalar(v) {
        Some(x) if x == 0. || x == 1. => Ok(x == 1.),
        _ => Err(format!("{who}: {what} must be true or false")),
    }
}

pub(super) fn positive(v: &Value, who: &str, what: &str) -> Result<f64> {
    match scalar(v) {
        Some(x) if x > 0. && x.is_finite() => Ok(x),
        _ => Err(format!("{who}: {what} must be a positive scalar")),
    }
}

pub(super) fn voices(v: &Value, who: &str) -> Result<usize> {
    match scalar(v) {
        Some(x) if (1. ..=48.).contains(&x) && x.fract() == 0. => Ok(x as usize),
        _ => Err(format!(
            "{who}: VoicesPerOctave must be an integer between 1 and 48"
        )),
    }
}

pub(super) fn time_bandwidth(v: &Value, who: &str) -> Result<f64> {
    match scalar(v) {
        Some(x) if (3. ..=120.).contains(&x) => Ok(x),
        _ => Err(format!(
            "{who}: TimeBandwidth must be a scalar between 3 and 120"
        )),
    }
}

pub(super) fn wavelet_parameters(v: &Value, who: &str) -> Result<(f64, f64)> {
    match reals(v).as_ref().map(|r| r.0.as_slice()) {
        Some([g, p]) if *g >= 1. && p >= g && p / g <= 40. && p.is_finite() => Ok((*g, *p)),
        _ => Err(format!(
            "{who}: WaveletParameters must be [gamma, P2] with gamma >= 1, P2 >= gamma and \
             P2/gamma <= 40"
        )),
    }
}

pub(super) fn frequency_limits(v: &Value, who: &str) -> Result<(f64, f64)> {
    match reals(v).as_ref().map(|r| r.0.as_slice()) {
        Some([a, b]) if *a > 0. && b > a && b.is_finite() => Ok((*a, *b)),
        _ => Err(format!(
            "{who}: FrequencyLimits must be two positive, strictly increasing frequencies"
        )),
    }
}

/// `f(x, 'name')` with a first text argument that is neither a wavelet nor
/// one of the options: say so before the Name, Value pairing complains.
fn first_text(args: &[Value], i: usize, options: &[&str], who: &str) -> Result<()> {
    match args.get(i).and_then(text) {
        Some(t) if i == 1 && !options.iter().any(|o| o.eq_ignore_ascii_case(t)) => Err(format!(
            "{who}: unknown wavelet or option '{t}' (wavelets are 'morse', 'amor' and 'bump')"
        )),
        _ => Ok(()),
    }
}

pub(super) fn is_wavelet_name(name: &str) -> bool {
    ["morse", "amor", "bump"]
        .iter()
        .any(|w| w.eq_ignore_ascii_case(name))
}

/// The wavelet from its name and the Morse parameters.
pub(super) fn wavelet(
    name: Option<&str>,
    tb: Option<f64>,
    params: Option<(f64, f64)>,
    who: &str,
) -> Result<Wavelet> {
    let name = name.unwrap_or("morse").to_ascii_lowercase();
    match name.as_str() {
        "morse" => match (tb, params) {
            (Some(_), Some(_)) => Err(format!(
                "{who}: specify TimeBandwidth or WaveletParameters, not both"
            )),
            (Some(p2), None) => Ok(Wavelet::Morse {
                gamma: 3.,
                beta: p2 / 3.,
            }),
            (None, Some((gamma, p2))) => Ok(Wavelet::Morse {
                gamma,
                beta: p2 / gamma,
            }),
            (None, None) => Ok(Wavelet::Morse {
                gamma: 3.,
                beta: 20.,
            }),
        },
        "amor" | "bump" if tb.is_some() || params.is_some() => Err(format!(
            "{who}: TimeBandwidth and WaveletParameters apply only to the Morse wavelet"
        )),
        "amor" => Ok(Wavelet::Amor),
        "bump" => Ok(Wavelet::Bump),
        _ => Err(format!(
            "{who}: unknown wavelet '{name}' (expected 'morse', 'amor' or 'bump')"
        )),
    }
}

/// A real or complex signal vector of at least four finite samples.
pub(super) fn signal(v: &Value, who: &str) -> Result<Samples> {
    let vector = |rows: usize, cols: usize, nd: bool| -> Result<()> {
        if nd || (rows != 1 && cols != 1) || rows * cols < 4 {
            Err(format!(
                "{who}: the signal must be a vector with at least 4 samples"
            ))
        } else {
            Ok(())
        }
    };
    let finite = |ok: bool| -> Result<()> {
        if ok {
            Ok(())
        } else {
            Err(format!("{who}: the signal must contain finite values"))
        }
    };
    match v {
        Value::Matrix(m) => {
            vector(m.rows(), m.cols(), m.is_nd())?;
            finite(m.data().iter().all(|x| x.is_finite()))?;
            Ok(Samples::Real(m.data().to_vec()))
        }
        Value::Complex(c) => {
            vector(c.rows(), c.cols(), c.is_nd())?;
            finite(
                c.data()
                    .iter()
                    .all(|x| x.re.is_finite() && x.im.is_finite()),
            )?;
            Ok(Samples::Complex(c.data().to_vec()))
        }
        _ => Err(format!(
            "{who}: the signal must be a real or complex double vector"
        )),
    }
}

// ---------- outputs ----------

pub(super) fn column(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(v.len(), 1, v)?))
}

pub(super) fn row(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}

pub(super) fn complex(
    e: &Engine,
    rows: usize,
    cols: usize,
    data: Vec<Complex64>,
) -> Result<ComplexMatrix> {
    ComplexMatrix::from_column_vec(rows, cols, data, &e.cancel)
}

fn size_overflow(who: &str) -> String {
    format!("{who}: the requested size overflows")
}

fn mul(a: usize, b: usize, who: &str) -> Result<usize> {
    a.checked_mul(b).ok_or_else(|| size_overflow(who))
}

/// Samples of the signal the filters see: `n`, or `n + 2 floor(n/2)` when it
/// is reflected about both ends.
pub(super) fn reflected_len(n: usize, extend: bool) -> usize {
    if extend { n + 2 * (n / 2) } else { n }
}

/// Admit the complex coefficient matrix (`rows` by `cols`, `pages` of them)
/// together with the reflected signal and `extra` further complex samples.
/// Returns the coefficient count. Nothing is allocated.
pub(super) fn admit_complex_buffers(
    e: &Engine,
    who: &str,
    rows: usize,
    cols: usize,
    pages: usize,
    extend: bool,
    extra: usize,
) -> Result<usize> {
    let coeffs = mul(mul(rows, cols, who)?, pages, who)?;
    // A complex sample is two 8-byte units. The buffers coexist.
    let reflected = mul(reflected_len(cols, extend), 2, who)?;
    let extra = mul(extra, 2, who)?;
    let units = mul(coeffs, 2, who)?
        .checked_add(reflected)
        .and_then(|units| units.checked_add(extra))
        .ok_or_else(|| size_overflow(who))?;
    e.preflight_elements(units)?;
    Ok(coeffs)
}

// ---------- the transform configuration shared by cwt and cwtfilterbank ----------

#[derive(Clone, Debug)]
pub(super) struct Config {
    pub n: usize,
    pub wavelet: Wavelet,
    pub nv: usize,
    pub fs: f64,
    /// FrequencyLimits as given (in the units of `fs`).
    pub limits: Option<(f64, f64)>,
    /// Reflect the signal at both ends (else it is taken as periodic).
    pub extend: bool,
    /// WaveletParameters was given (shown by the filter bank object).
    pub parameters: bool,
}

impl Config {
    /// Scales (samples) and centre frequencies (units of `fs`), high to low.
    pub(super) fn grid(&self, who: &str) -> Result<(Vec<f64>, Vec<f64>)> {
        let (lo_n, hi_n) = self.wavelet.bounds(self.n, self.nv);
        let lo_hz = lo_n * self.fs;
        let hi_hz = hi_n * self.fs;
        // Hertz bounds lose the ratio when `fs` underflows or overflows them
        // (cwt(zeros(1,128), 1e-323) scaled the defaults to [0, 5e-324]).
        // The scale grid depends only on the normalized ratio.
        let normalized = !(lo_hz.is_finite() && hi_hz.is_finite() && lo_hz > 0. && hi_hz > lo_hz);
        let (lo, hi, fs) = if normalized {
            (lo_n, hi_n, 1.)
        } else {
            (lo_hz, hi_hz, self.fs)
        };
        let (a, b) = match self.limits {
            Some((fa, fb)) if normalized => {
                let fa = fa / self.fs;
                let fb = fb / self.fs;
                if !(fa.is_finite() && fb.is_finite() && fa > 0.) {
                    return Err(format!(
                        "{who}: FrequencyLimits are not representable at this sampling frequency"
                    ));
                }
                (fa.max(lo), fb.min(hi))
            }
            Some((fa, fb)) => (fa.max(lo), fb.min(hi)),
            None => (lo, hi),
        };
        if self.limits.is_some() && !(b > a && (b / a).log2() >= 1. / self.nv as f64 - 1e-9) {
            return Err(format!(
                "{who}: FrequencyLimits must cover at least 1/VoicesPerOctave octaves between \
                 {lo_hz:.6} and {hi_hz:.6}"
            ));
        }
        let (scales, freqs) = self
            .wavelet
            .grid(a, b, fs, self.nv)
            .map_err(|err| format!("{who}: {err}"))?;
        let freqs = if normalized {
            freqs.into_iter().map(|f| f * self.fs).collect()
        } else {
            freqs
        };
        if freqs.is_empty() {
            return Err(format!("{who}: the frequency grid is empty"));
        }
        Ok((scales, freqs))
    }
    /// The transform of `x`: wt (scales x n, or scales x n x 2 for complex
    /// input), the frequencies and the cone of influence.
    pub(super) fn transform(
        &self,
        e: &Engine,
        x: &Samples,
        who: &str,
    ) -> Result<(Value, Vec<f64>, Vec<f64>)> {
        let (scales, freqs) = self.grid(who)?;
        let pages = match x {
            Samples::Real(_) => 1,
            Samples::Complex(_) => 2,
        };
        let na = scales.len();
        let coeffs = admit_complex_buffers(e, who, na, self.n, pages, self.extend, 0)?;
        if coeffs > kernel::MAX_ELEMENTS {
            return Err(format!(
                "{who}: the {na}-by-{} transform exceeds {} elements; shorten the signal, narrow \
                 FrequencyLimits or lower VoicesPerOctave",
                self.n,
                kernel::MAX_ELEMENTS
            ));
        }
        let data = kernel::transform(x, &self.wavelet, &scales, self.extend, &e.cancel)?;
        let mut wt = complex(e, na, self.n * pages, data)?;
        if pages == 2 {
            wt = wt.with_dims(&[na, self.n, 2])?;
        }
        let fmax = freqs
            .first()
            .copied()
            .ok_or_else(|| format!("{who}: the frequency grid is empty"))?;
        let coi = self.wavelet.coi(self.n, self.fs, fmax);
        Ok((Value::Complex(wt), freqs, coi))
    }
}

// ---------- cwt ----------

fn cwt(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "cwt";
    if nargout == 5 {
        return Err("cwt: the scaling coefficients output is not supported yet".into());
    }
    too_many(nargout, 4, WHO)?;
    let Some(first) = args.first() else {
        return Err("cwt: not enough input arguments".into());
    };
    let x = signal(first, WHO)?;
    let mut i = 1;
    let mut name = None;
    let mut fs = None;
    if let Some(t) = args.get(i).and_then(text)
        && is_wavelet_name(t)
    {
        name = Some(t);
        i += 1;
    }
    if let Some(v) = args.get(i) {
        if is_duration(v) {
            return Err(
                "cwt: duration sampling periods are not supported yet; pass the sampling \
                 frequency in hertz"
                    .into(),
            );
        }
        if text(v).is_none() {
            fs = Some(positive(v, WHO, "the sampling frequency")?);
            i += 1;
        }
    }
    const OPTIONS: &[&str] = &[
        "VoicesPerOctave",
        "FrequencyLimits",
        "TimeBandwidth",
        "WaveletParameters",
        "ExtendSignal",
        "FilterBank",
        "PeriodLimits",
        "NumOctaves",
    ];
    first_text(args, i, OPTIONS, WHO)?;
    let options = pairs(&args[i..], WHO)?;
    let (mut nv, mut limits, mut tb, mut params, mut extend, mut bank) =
        (None, None, None, None, None, None);
    for (key, written, value) in &options {
        match key.as_str() {
            "voicesperoctave" => nv = Some(voices(value, WHO)?),
            "frequencylimits" => limits = Some(frequency_limits(value, WHO)?),
            "timebandwidth" => tb = Some(time_bandwidth(value, WHO)?),
            "waveletparameters" => params = Some(wavelet_parameters(value, WHO)?),
            "extendsignal" => extend = Some(flag(value, WHO, "ExtendSignal")?),
            "filterbank" => bank = Some(cwt_bank::decode(value, WHO)?),
            "periodlimits" => {
                return Err(
                    "cwt: PeriodLimits needs a duration sampling period, which is not supported \
                     yet; use FrequencyLimits"
                        .into(),
                );
            }
            "numoctaves" => {
                return Err("cwt: NumOctaves is not supported; use FrequencyLimits".into());
            }
            _ => return Err(format!("cwt: unknown option '{written}'")),
        }
    }
    let config = if let Some(bank) = bank {
        if options.len() > 1 || name.is_some() || fs.is_some() {
            return Err(
                "cwt: FilterBank cannot be combined with a wavelet, sampling frequency or other \
                 options"
                    .into(),
            );
        }
        if bank.n != x.len() {
            return Err(format!(
                "cwt: the signal length must equal the SignalLength of the filter bank ({})",
                bank.n
            ));
        }
        bank
    } else {
        Config {
            n: x.len(),
            wavelet: wavelet(name, tb, params, WHO)?,
            nv: nv.unwrap_or(10),
            fs: fs.unwrap_or(1.),
            limits,
            extend: extend.unwrap_or(true),
            parameters: params.is_some(),
        }
    };
    let (wt, freqs, coi) = config.transform(e, &x, WHO)?;
    let mut out = vec![wt];
    if nargout > 1 {
        out.push(column(freqs)?);
    }
    if nargout > 2 {
        out.push(column(coi)?);
    }
    if nargout > 3 {
        out.push(cwt_bank::encode(e, &config)?);
    }
    Ok(out)
}

// ---------- icwt ----------

/// Coefficients as (data, rows, columns, pages).
fn coefficients(v: &Value, who: &str) -> Result<(Vec<Complex64>, usize, usize, usize)> {
    let bad = || format!("{who}: the coefficients must be a matrix or a 3-D array with two pages");
    let (data, dims): (Vec<Complex64>, Vec<usize>) = match v {
        Value::Complex(c) => (c.data().to_vec(), c.dims().to_vec()),
        Value::Matrix(m) => (
            m.data().iter().map(|x| Complex64::new(*x, 0.)).collect(),
            m.dims().to_vec(),
        ),
        _ => return Err(bad()),
    };
    let pages = match dims.as_slice() {
        [_, _] => 1,
        [_, _, 2] => 2,
        _ => return Err(bad()),
    };
    if dims[0] == 0 || dims[1] == 0 {
        return Err(format!("{who}: the coefficients must not be empty"));
    }
    if data.iter().any(|x| !(x.re.is_finite() && x.im.is_finite())) {
        return Err(format!("{who}: the coefficients must be finite"));
    }
    Ok((data, dims[0], dims[1], pages))
}

/// Rows whose frequency lies inside `range` (all rows without a range).
fn selected_rows(freqs: Option<(&Value, &Value)>, na: usize, who: &str) -> Result<Vec<bool>> {
    let Some((f, range)) = freqs else {
        return Ok(vec![true; na]);
    };
    let f = reals(f).map(|r| r.0).unwrap_or_default();
    if f.len() != na {
        return Err(format!(
            "{who}: the frequency vector must have one element per row of the coefficients"
        ));
    }
    let (lo, hi) = match reals(range).as_ref().map(|r| r.0.as_slice()) {
        Some([a, b]) if a < b => (*a, *b),
        _ => {
            return Err(format!(
                "{who}: the frequency range must be two increasing values"
            ));
        }
    };
    let keep: Vec<bool> = f.iter().map(|v| (lo..=hi).contains(v)).collect();
    if !keep.contains(&true) {
        return Err(format!(
            "{who}: no frequency lies inside the frequency range"
        ));
    }
    Ok(keep)
}

fn icwt(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "icwt";
    too_many(nargout, 1, WHO)?;
    let Some(first) = args.first() else {
        return Err("icwt: not enough input arguments".into());
    };
    let (data, na, n, pages) = coefficients(first, WHO)?;
    let mut i = 1;
    let mut name = None;
    if let Some(t) = args.get(i).and_then(text)
        && is_wavelet_name(t)
    {
        name = Some(t);
        i += 1;
    }
    let mut freqs = None;
    if let Some(v) = args.get(i)
        && text(v).is_none()
    {
        if is_duration(v) {
            return Err("icwt: duration periods are not supported yet; pass frequencies".into());
        }
        let Some(range) = args.get(i + 1).filter(|r| text(r).is_none()) else {
            return Err("icwt: a frequency vector needs a frequency range [fmin fmax]".into());
        };
        freqs = Some((v, range));
        i += 2;
    }
    const OPTIONS: &[&str] = &[
        "VoicesPerOctave",
        "TimeBandwidth",
        "WaveletParameters",
        "AnalysisFilterBank",
        "SignalMean",
        "ScalingCoefficients",
    ];
    first_text(args, i, OPTIONS, WHO)?;
    let (mut nv, mut tb, mut params, mut bank) = (None, None, None, None);
    let mut mean = Complex64::new(0., 0.);
    for (key, written, value) in pairs(&args[i..], WHO)? {
        match key.as_str() {
            "voicesperoctave" => nv = Some(voices(value, WHO)?),
            "timebandwidth" => tb = Some(time_bandwidth(value, WHO)?),
            "waveletparameters" => params = Some(wavelet_parameters(value, WHO)?),
            "analysisfilterbank" => bank = Some(cwt_bank::decode(value, WHO)?),
            "signalmean" => {
                mean = match value {
                    Value::Complex(c) if c.len() == 1 && pages == 2 => c.data()[0],
                    v => match scalar(v) {
                        Some(x) if x.is_finite() => Complex64::new(x, 0.),
                        _ => return Err("icwt: SignalMean must be a finite scalar".into()),
                    },
                };
            }
            "scalingcoefficients" => {
                return Err("icwt: ScalingCoefficients is not supported yet".into());
            }
            _ => return Err(format!("icwt: unknown option '{written}'")),
        }
    }
    let (wav, nv) = if let Some(bank) = bank {
        if name.is_some() || nv.is_some() || tb.is_some() || params.is_some() {
            return Err(
                "icwt: AnalysisFilterBank cannot be combined with a wavelet or its parameters"
                    .into(),
            );
        }
        (bank.wavelet, bank.nv)
    } else {
        (wavelet(name, tb, params, WHO)?, nv.unwrap_or(10))
    };
    let keep = selected_rows(freqs, na, WHO)?;
    e.check_cancel()?;
    let gain = 2. * LN_2 / (nv as f64 * wav.admissibility());
    let mut sums = vec![Complex64::new(0., 0.); n];
    for p in 0..pages {
        for (b, sum) in sums.iter_mut().enumerate() {
            let start = p * na * n + na * b;
            for (v, _) in data[start..start + na]
                .iter()
                .zip(&keep)
                .filter(|(_, k)| **k)
            {
                *sum += v;
            }
        }
    }
    if pages == 1 {
        return Ok(vec![row(sums
            .iter()
            .map(|s| gain * s.re + mean.re)
            .collect())?]);
    }
    let values = sums.iter().map(|s| s * gain + mean).collect();
    Ok(vec![Value::Complex(complex(e, 1, n, values)?)])
}

// ---------- wsst / iwsst ----------

fn sst_wavelet(name: &str, who: &str) -> Result<Wavelet> {
    match name.to_ascii_lowercase().as_str() {
        "amor" => Ok(Wavelet::Amor),
        "bump" => Ok(Wavelet::Bump),
        _ => Err(format!(
            "{who}: unknown wavelet '{name}' (expected 'amor' or 'bump')"
        )),
    }
}

fn is_sst_wavelet_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("amor") || name.eq_ignore_ascii_case("bump")
}

fn wsst(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "wsst";
    too_many(nargout, 2, WHO)?;
    let Some(first) = args.first() else {
        return Err("wsst: not enough input arguments".into());
    };
    let Samples::Real(x) = signal(first, WHO)? else {
        return Err("wsst: the signal must be real".into());
    };
    let n = x.len();
    let mut i = 1;
    let mut wav = Wavelet::Amor;
    let mut fs = 1.;
    for slot in 0..2 {
        let Some(v) = args.get(i) else { break };
        if let Some(t) = text(v) {
            if !is_sst_wavelet_name(t) {
                // An option name, or a wavelet that wsst does not have.
                if slot == 0 && args.len() == 2 || is_wavelet_name(t) {
                    sst_wavelet(t, WHO)?;
                }
                break;
            }
            wav = sst_wavelet(t, WHO)?;
        } else if is_duration(v) {
            return Err(
                "wsst: duration sampling periods are not supported yet; pass the sampling \
                 frequency in hertz"
                    .into(),
            );
        } else {
            fs = positive(v, WHO, "the sampling frequency")?;
        }
        i += 1;
    }
    let mut nv = 32;
    let mut extend = false;
    for (key, written, value) in pairs(&args[i..], WHO)? {
        match key.as_str() {
            "voicesperoctave" => {
                nv = match scalar(value) {
                    Some(x) if (10. ..=48.).contains(&x) && x % 2. == 0. => x as usize,
                    _ => {
                        return Err(
                            "wsst: VoicesPerOctave must be an even integer between 10 and 48"
                                .into(),
                        );
                    }
                }
            }
            "extendsignal" => extend = flag(value, WHO, "ExtendSignal")?,
            _ => return Err(format!("wsst: unknown option '{written}'")),
        }
    }
    let na = kernel::sst_rows(n, nv);
    // `held` keeps one scale's coefficients (n complex samples) beside the result.
    let coeffs = admit_complex_buffers(e, WHO, na, n, 1, extend, n)?;
    if coeffs > kernel::MAX_ELEMENTS {
        return Err(format!(
            "wsst: the {na}-by-{n} transform exceeds {} elements; shorten the signal or lower \
             VoicesPerOctave",
            kernel::MAX_ELEMENTS
        ));
    }
    let data = kernel::synchrosqueeze(&x, &wav, nv, extend, &e.cancel)?;
    let mut out = vec![Value::Complex(complex(e, na, n, data)?)];
    if nargout > 1 {
        out.push(row(kernel::sst_frequencies(n, na, fs))?);
    }
    Ok(out)
}

fn iwsst(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "iwsst";
    too_many(nargout, 1, WHO)?;
    let Some(first) = args.first() else {
        return Err("iwsst: not enough input arguments".into());
    };
    let (data, na, n, pages) = coefficients(first, WHO)?;
    if pages != 1 {
        return Err("iwsst: the synchrosqueezed transform must be a matrix".into());
    }
    // Positional arguments: (f, freqrange) or iridge, then the wavelet.
    let mut i = 1;
    let mut numeric = Vec::new();
    while let Some(v) = args.get(i).filter(|v| text(v).is_none()) {
        numeric.push(v);
        i += 1;
    }
    let mut wav = Wavelet::Amor;
    if let Some(t) = args.get(i).and_then(text)
        && (is_wavelet_name(t) || args.len() == i + 1)
    {
        wav = sst_wavelet(t, WHO)?;
        i += 1;
    }
    let mut bins = 16;
    let mut bins_given = false;
    for (key, written, value) in pairs(&args[i..], WHO)? {
        match key.as_str() {
            "numfrequencybins" => {
                bins = match scalar(value) {
                    Some(x) if x >= 1. && x.fract() == 0. && x < 1e9 => x as usize,
                    _ => {
                        return Err("iwsst: NumFrequencyBins must be a positive integer".into());
                    }
                };
                bins_given = true;
            }
            _ => return Err(format!("iwsst: unknown option '{written}'")),
        }
    }
    e.check_cancel()?;
    let gain = 2. / wav.admissibility();
    let at = |l: usize, b: usize| data[l + na * b].re;
    let (rows, cols, values) = match numeric.as_slice() {
        [] | [_, _] => {
            if bins_given {
                return Err("iwsst: NumFrequencyBins needs ridge indices".into());
            }
            let freqs = match numeric.as_slice() {
                [f, range] => Some((*f, *range)),
                _ => None,
            };
            let keep = selected_rows(freqs, na, WHO)?;
            let values = (0..n)
                .map(|b| gain * (0..na).filter(|l| keep[*l]).map(|l| at(l, b)).sum::<f64>())
                .collect();
            (n, 1, values)
        }
        [ridge] => {
            let Some((idx, r, c)) = reals(ridge) else {
                return Err("iwsst: the ridge indices must be a real array".into());
            };
            // One ridge per column; a row vector is one ridge.
            let (r, c) = if r == 1 { (c, 1) } else { (r, c) };
            if r != n {
                return Err(format!(
                    "iwsst: the ridge indices need one row per column of the transform ({n})"
                ));
            }
            if idx
                .iter()
                .any(|v| !(*v >= 1. && *v <= na as f64 && v.fract() == 0.))
            {
                return Err(format!(
                    "iwsst: the ridge indices must be integers between 1 and {na}"
                ));
            }
            // Each ridge sample sums about `bins` rows. The coefficient matrix
            // is finite, but rows x columns x bins is not, so admit that
            // product before the reconstruction allocates its output.
            let work = mul(mul(r, c, WHO)?, bins, WHO)?;
            e.preflight_elements(work)
                .map_err(|err| format!("{WHO}: {err}"))?;
            let mut values = Vec::with_capacity(idx.len());
            let mut steps = 0usize;
            for (k, v) in idx.iter().enumerate() {
                let (b, centre) = (k % n, *v as usize - 1);
                let start = centre.saturating_sub(bins);
                let end = centre.saturating_add(bins).min(na.saturating_sub(1));
                let mut acc = 0.;
                for l in start..=end {
                    if steps.is_multiple_of(4096) {
                        e.check_cancel()?;
                    }
                    steps += 1;
                    acc += at(l, b);
                }
                values.push(gain * acc);
            }
            (n, c, values)
        }
        _ => return Err("iwsst: too many input arguments".into()),
    };
    Ok(vec![Value::Matrix(Matrix::from_column_vec(
        rows, cols, values,
    )?)])
}
