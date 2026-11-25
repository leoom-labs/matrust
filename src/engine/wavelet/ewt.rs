//! Empirical wavelet transform (Gilles 2013, IEEE TSP 61(16)). Signals
//! shorter than 64 samples are zero-padded to 64 (outputs cut back to N).
//! 1. Spectrum: sine multitaper estimate (Riedel & Sidorenko 1995) with
//!    K = max(1, round(FrequencyResolution (M+1)) - 1) tapers on M samples;
//!    the default FrequencyResolution 6/(M+1) gives 5 tapers.
//! 2. Peaks: interior local maxima of the one-sided spectrum (of its log
//!    minus its minimum with LogSpectrum). Either the MaxNumPeaks largest, or
//!    those strictly above PeakThresholdPercent (default 70) % of the largest.
//! 3. Boundaries between adjacent peaks: the geometric mean of the two peak
//!    frequencies ('geomean') or the first local minimum of the spectrum
//!    between them ('localmin', else the lowest point).
//! 4. Meyer-type Littlewood-Paley filters with gamma = (1 - 1/M) times
//!    Gilles' bound min (w_{n+1} - w_n) / (w_{n+1} + w_n) over the
//!    boundaries and pi; the squared responses sum to one (tight frame).
//!    cfs_n = ifft(X psi_n), mra_n = ifft(X psi_n^2); columns ordered by
//!    decreasing frequency, the scaling (lowpass) part last.
use super::super::{Engine, Value};
use super::mra::{
    mra_bool, mra_columns, mra_count, mra_fft, mra_options, mra_scalar, mra_signal, mra_text,
};
use crate::Result;
use rustfft::num_complex::Complex;
use std::f64::consts::PI;

#[derive(Clone, Copy, PartialEq)]
enum Segment {
    GeoMean,
    LocalMin,
}

pub(super) fn ewt_builtin(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    const WHO: &str = "ewt";
    let Some(first) = args.first() else {
        return Err("ewt: not enough input arguments".into());
    };
    let x = mra_signal(first, WHO, 2)?;
    const NAMES: [&str; 5] = [
        "MaxNumPeaks",
        "SegmentMethod",
        "FrequencyResolution",
        "PeakThresholdPercent",
        "LogSpectrum",
    ];
    let mut max_peaks = None;
    let mut threshold = None;
    let mut segment = Segment::GeoMean;
    let mut resolution = None;
    let mut log_spectrum = false;
    for (i, v) in mra_options(&args[1..], &NAMES, WHO)? {
        match i {
            0 => max_peaks = Some(mra_count(v, WHO, "MaxNumPeaks", 1_000_000)?),
            1 => {
                let t = mra_text(v).ok_or("ewt: SegmentMethod must be 'geomean' or 'localmin'")?;
                segment = if t.eq_ignore_ascii_case("geomean") {
                    Segment::GeoMean
                } else if t.eq_ignore_ascii_case("localmin") {
                    Segment::LocalMin
                } else {
                    return Err(format!(
                        "ewt: SegmentMethod must be 'geomean' or 'localmin', not '{t}'"
                    ));
                };
            }
            2 => {
                let w = mra_scalar(v, WHO, "FrequencyResolution")?;
                if !(w > 0. && w <= 0.25) {
                    return Err("ewt: FrequencyResolution must be in (0, 0.25]".into());
                }
                resolution = Some(w);
            }
            3 => {
                let p = mra_scalar(v, WHO, "PeakThresholdPercent")?;
                if !(p > 0. && p < 100.) {
                    return Err("ewt: PeakThresholdPercent must be in (0, 100)".into());
                }
                threshold = Some(p);
            }
            _ => log_spectrum = mra_bool(v, WHO, "LogSpectrum")?,
        }
    }
    if max_peaks.is_some() && threshold.is_some() {
        return Err("ewt: specify MaxNumPeaks or PeakThresholdPercent, not both".into());
    }
    let n = x.len();
    let m = n.max(64);
    let tapers = match resolution {
        None => 5,
        Some(w) => ((w * (m as f64 + 1.)).round() as usize)
            .saturating_sub(1)
            .max(1),
    };
    // Complex FFT buffers (multitaper, the signal transform, and the two
    // filter spectra) and the real bank/mra/cfs columns, before any FFT.
    // Without MaxNumPeaks the column count is still unknown, so admit one
    // column here and the real count once the peaks exist.
    let known = max_peaks.unwrap_or(1);
    admit_scratch(e, m, known)?;
    let mut padded = x;
    padded.resize(m, 0.);
    e.check_cancel()?;
    let mut spec = multitaper(e, &padded, tapers)?;
    if log_spectrum {
        let tiny = f64::MIN_POSITIVE;
        for s in &mut spec {
            *s = s.max(tiny).ln();
        }
        let lo = spec.iter().copied().fold(f64::INFINITY, f64::min);
        for s in &mut spec {
            *s -= lo;
        }
    }
    let peaks = select_peaks(&spec, max_peaks, threshold.unwrap_or(70.));
    if max_peaks.is_none() {
        let found = peaks.len().max(1);
        if found != known {
            admit_scratch(e, m, found)?;
        }
    }
    let bounds: Vec<f64> = peaks
        .windows(2)
        .map(|p| boundary_index(&spec, p[0], p[1], segment))
        .map(|b| 2. * PI * b / m as f64)
        .collect();
    let bank = filter_bank(&bounds, m);
    let mut xf: Vec<Complex<f64>> = padded.iter().map(|&v| Complex::new(v, 0.)).collect();
    mra_fft(&mut xf, false);
    let inv = 1. / m as f64;
    let mut cfs = Vec::with_capacity(bank.len());
    let mut mra = Vec::with_capacity(bank.len());
    for psi in bank.iter().rev() {
        e.check_cancel()?;
        let mut c: Vec<Complex<f64>> = xf.iter().zip(psi).map(|(a, p)| a * *p).collect();
        let mut r: Vec<Complex<f64>> = c.iter().zip(psi).map(|(a, p)| a * *p).collect();
        mra_fft(&mut c, true);
        mra_fft(&mut r, true);
        cfs.push(c.iter().map(|v| v.re * inv).collect::<Vec<f64>>());
        mra.push(r.iter().map(|v| v.re * inv).collect::<Vec<f64>>());
    }
    Ok(vec![mra_columns(n, &mra)?, mra_columns(n, &cfs)?])
}

/// Admit every buffer the transform holds at once: three complex FFT vectors
/// (signal spectrum, wavelet coefficient spectrum, squared-filter spectrum),
/// the padded signal, the one-sided multitaper spectrum, and `columns` real
/// filters together with the mra and cfs they produce. Nothing is allocated.
fn admit_scratch(e: &Engine, m: usize, columns: usize) -> Result<()> {
    const WHO: &str = "ewt";
    let columns = columns.max(1);
    // Keeps the matrix element-limit error for a huge bank before the FFT.
    super::mra::mra_admit(e, WHO, columns, m)?;
    let half = m / 2 + 1;
    let complex = m
        .checked_mul(6)
        .ok_or("ewt: the requested size overflows")?;
    let bank = columns
        .checked_mul(m)
        .and_then(|n| n.checked_mul(3))
        .ok_or("ewt: the requested size overflows")?;
    let units = m
        .checked_add(half)
        .and_then(|n| n.checked_add(bank))
        .and_then(|n| n.checked_add(complex))
        .ok_or("ewt: the requested size overflows")?;
    e.preflight_elements(units)
        .map_err(|err| format!("{WHO}: {err}"))
}

/// Sine multitaper power spectrum on the one-sided grid k = 0..floor(M/2).
fn multitaper(e: &Engine, x: &[f64], tapers: usize) -> Result<Vec<f64>> {
    let m = x.len();
    let half = m / 2 + 1;
    let mut spec = vec![0.; half];
    let scale = (2. / (m as f64 + 1.)).sqrt();
    for k in 1..=tapers {
        e.check_cancel()?;
        let mut buf: Vec<Complex<f64>> = (0..m)
            .map(|t| {
                let v = scale * (PI * k as f64 * (t as f64 + 1.) / (m as f64 + 1.)).sin();
                Complex::new(v * x[t], 0.)
            })
            .collect();
        mra_fft(&mut buf, false);
        for (s, b) in spec.iter_mut().zip(&buf) {
            *s += b.norm_sqr() / tapers as f64;
        }
    }
    Ok(spec)
}

/// Peak indices (ascending) of the one-sided spectrum.
fn select_peaks(spec: &[f64], max_peaks: Option<usize>, threshold: f64) -> Vec<usize> {
    let mut peaks: Vec<usize> = (1..spec.len().saturating_sub(1))
        .filter(|&j| spec[j] > spec[j - 1] && spec[j] >= spec[j + 1])
        .collect();
    peaks.sort_by(|&a, &b| spec[b].total_cmp(&spec[a]).then(a.cmp(&b)));
    match max_peaks {
        Some(k) => peaks.truncate(k),
        None => {
            if let Some(&top) = peaks.first() {
                let cut = threshold / 100. * spec[top];
                peaks.retain(|&j| spec[j] > cut);
            }
        }
    }
    peaks.sort_unstable();
    peaks
}

/// Boundary (fractional bin) between peaks at bins a < b.
fn boundary_index(spec: &[f64], a: usize, b: usize, segment: Segment) -> f64 {
    match segment {
        Segment::GeoMean => ((a as f64) * (b as f64)).sqrt(),
        Segment::LocalMin => {
            let first = (a + 1..b).find(|&j| spec[j] < spec[j - 1] && spec[j] <= spec[j + 1]);
            let j = first.unwrap_or_else(|| {
                (a + 1..b)
                    .min_by(|&p, &q| spec[p].total_cmp(&spec[q]))
                    .unwrap_or((a + b) / 2)
            });
            j as f64
        }
    }
}

/// Meyer polynomial beta(x) = x^4 (35 - 84x + 70x^2 - 20x^3) on [0, 1].
fn beta(x: f64) -> f64 {
    let x = x.clamp(0., 1.);
    x.powi(4) * (35. - 84. * x + 70. * x * x - 20. * x * x * x)
}

/// Responses (real, even) on the DFT grid: scaling filter, then one wavelet
/// per band [w_n, w_{n+1}] (the last up to pi).
fn filter_bank(bounds: &[f64], m: usize) -> Vec<Vec<f64>> {
    let omega: Vec<f64> = (0..m)
        .map(|k| 2. * PI * k.min(m - k) as f64 / m as f64)
        .collect();
    if bounds.is_empty() {
        return vec![vec![1.; m]];
    }
    let mut edges = bounds.to_vec();
    edges.push(PI);
    let bound = edges
        .windows(2)
        .map(|w| (w[1] - w[0]) / (w[1] + w[0]))
        .fold(1., f64::min);
    let gamma = (1. - 1. / m as f64) * bound;
    let w1 = bounds[0];
    let mut bank = vec![
        omega
            .iter()
            .map(|&w| {
                if w <= (1. - gamma) * w1 {
                    1.
                } else if w <= (1. + gamma) * w1 {
                    (PI / 2. * beta((w - (1. - gamma) * w1) / (2. * gamma * w1))).cos()
                } else {
                    0.
                }
            })
            .collect(),
    ];
    for (i, &lo) in bounds.iter().enumerate() {
        let hi = bounds.get(i + 1).copied();
        bank.push(
            omega
                .iter()
                .map(|&w| {
                    let up = match hi {
                        None => 1.,
                        Some(hi) => {
                            if w <= (1. - gamma) * hi {
                                1.
                            } else if w <= (1. + gamma) * hi {
                                (PI / 2. * beta((w - (1. - gamma) * hi) / (2. * gamma * hi))).cos()
                            } else {
                                0.
                            }
                        }
                    };
                    let down = if w >= (1. + gamma) * lo {
                        1.
                    } else if w >= (1. - gamma) * lo {
                        (PI / 2. * beta((w - (1. - gamma) * lo) / (2. * gamma * lo))).sin()
                    } else {
                        0.
                    };
                    up * down
                })
                .collect(),
        );
    }
    bank
}
