//! Real-valued signal processing. Frequencies are normalized to Nyquist.
//! Recursive samples stay sequential; independent columns can use the shared pool.
use crate::{Matrix, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::PI,
    sync::atomic::{AtomicBool, Ordering},
};

mod fft;
pub use fft::{FftOptions, MAX_FFT_LENGTH, TransformDirection, fourier, fourier_shape};
mod analysis;
mod butter_bands;
mod chebyshev1;
pub use chebyshev1::{Chebyshev1, MAX_CHEBY1_ORDER, cheby1};
mod chirp;
mod filtfilt;
mod filtic;
mod fir_design;
mod kaiser;
mod rate_conversion;
mod small_filter;
mod sos_filtfilt;
mod sos_pipeline;
mod state;
mod waveforms;
mod windows;
pub use analysis::{Analysis, analyze, analyze_sos, group_delay, phase_response, sos_group_delay};
pub use butter_bands::butter_sos_with_cutoffs;
pub use chirp::{ChirpMethod, ChirpOptions, ChirpShape, chirp};
pub use filtfilt::filtfilt;
pub use filtic::filtic;
pub use fir_design::{FirOptions, fir1, fir1_with_options};
pub use kaiser::kaiser;
pub use rate_conversion::{downsample, rate_output_shape, upfirdn, upfirdn_output_shape, upsample};
pub use sos_filtfilt::filtfilt_sos;
pub use state::{FilterOptions, filter_with_options, filter_with_state};
pub use waveforms::{sawtooth, sinc, square};
pub use windows::{WindowKind, window, window_length};

pub const MAX_ORDER: usize = 4096;
const MAX_WORK: usize = 256_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Band {
    Lowpass,
    Highpass,
    Bandpass,
    Bandstop,
}
impl Band {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "low" | "lowpass" => Ok(Self::Lowpass),
            "high" | "highpass" => Ok(Self::Highpass),
            "bandpass" => Ok(Self::Bandpass),
            "stop" | "bandstop" => Ok(Self::Bandstop),
            _ => Err("Filter type must be low, high, bandpass or stop".into()),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Lowpass => "low",
            Self::Highpass => "high",
            Self::Bandpass => "bandpass",
            Self::Bandstop => "stop",
        }
    }
}

/// Butterworth low/highpass design in normalized biquad sections [b0 b1 b2 a0 a1 a2].
/// Bilinear transform with prewarping, each section scaled at DC or Nyquist.
pub fn butter_sos(order: usize, cutoff: f64, band: Band) -> Result<Matrix> {
    if !(1..=20).contains(&order) || !cutoff.is_finite() || cutoff <= 0. || cutoff >= 1. {
        return Err("Butterworth order must be 1..20 and cutoff strictly between 0 and 1".into());
    }
    if !matches!(band, Band::Lowpass | Band::Highpass) {
        return Err("Butterworth SOS currently supports lowpass and highpass".into());
    }
    let high = band == Band::Highpass;
    // Reciprocal prewarping keeps intermediates bounded close to Nyquist.
    let k = (PI * cutoff / 2.).tan();
    let mut sections = Vec::new();
    if order % 2 == 1 {
        let norm = 1. / (1. + k);
        let b = if high { norm } else { k * norm };
        sections.push([b, if high { -b } else { b }, 0., 1., (k - 1.) * norm, 0.]);
    }
    for j in 0..order / 2 {
        let damping = 2. * ((2 * j + 1) as f64 * PI / (2 * order) as f64).sin();
        let norm = 1. / (1. + damping * k + k * k);
        let b = if high { norm } else { k * k * norm };
        sections.push([
            b,
            if high { -2. * b } else { 2. * b },
            b,
            1.,
            2. * (k * k - 1.) * norm,
            (1. - damping * k + k * k) * norm,
        ]);
    }
    // Low-Q sections first reduce intermediate peaks in the cascade.
    sections.reverse();
    if sections.iter().any(|s| {
        s.iter().any(|v| !v.is_finite())
            || s[5].abs() >= 1.
            || 1. + s[4] + s[5] <= 0.
            || 1. - s[4] + s[5] <= 0.
    }) {
        return Err(
            "Cutoff is too close to DC or Nyquist for a stable f64 Butterworth design".into(),
        );
    }
    let data: Vec<_> = (0..6)
        .flat_map(|c| sections.iter().map(move |s| s[c]))
        .collect();
    Matrix::from_column_slice(sections.len(), 6, &data)
}

#[derive(Clone, Debug)]
pub struct Filter {
    b: std::sync::Arc<[f64]>,
    a: std::sync::Arc<[f64]>,
    state: Vec<f64>,
    feedback: bool,
}
impl Filter {
    pub fn new(b: &[f64], a: &[f64]) -> Result<Self> {
        if b.is_empty()
            || a.is_empty()
            || b.len().max(a.len()) > MAX_ORDER + 1
            || a[0] == 0.
            || b.iter().chain(a).any(|v| !v.is_finite())
        {
            return Err("Filter coefficients must be finite nonempty vectors with nonzero a(1), order at most 4096".into());
        }
        let n = b.len().max(a.len());
        let mut numerator = vec![0.; n];
        let mut denominator = vec![0.; n];
        for (to, from) in numerator.iter_mut().zip(b) {
            *to = *from / a[0];
        }
        for (to, from) in denominator.iter_mut().zip(a) {
            *to = *from / a[0];
        }
        if numerator.iter().chain(&denominator).any(|v| !v.is_finite()) {
            return Err("Normalizing filter coefficients overflowed".into());
        }
        Ok(Self {
            feedback: denominator[1..].iter().any(|v| *v != 0.),
            b: numerator.into(),
            a: denominator.into(),
            state: vec![0.; n - 1],
        })
    }
    pub fn state(&self) -> &[f64] {
        &self.state
    }
    pub fn set_state(&mut self, state: &[f64]) -> Result<()> {
        if state.len() != self.state.len() || state.iter().any(|v| !v.is_finite()) {
            return Err("Invalid filter initial state".into());
        }
        self.state.copy_from_slice(state);
        Ok(())
    }
    pub fn reset(&mut self) {
        self.state.fill(0.);
    }
    #[inline]
    pub fn sample(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.state.first().copied().unwrap_or(0.);
        let n = self.state.len();
        if n > 0 {
            if self.feedback {
                for j in 0..n - 1 {
                    self.state[j] = self.state[j + 1] + self.b[j + 1] * x - self.a[j + 1] * y;
                }
                self.state[n - 1] = self.b[n] * x - self.a[n] * y;
            } else {
                // FIR delays contain contributions from inputs only. A padded
                // zero feedback term would incorrectly feed 0*NaN back forever.
                for j in 0..n - 1 {
                    self.state[j] = self.state[j + 1] + self.b[j + 1] * x;
                }
                self.state[n - 1] = self.b[n] * x;
            }
        }
        y
    }
    pub fn process(&mut self, data: &mut [f64], cancel: &AtomicBool) -> Result<()> {
        // Fixed-size recurrences keep coefficients and delays in registers.
        // Samples remain ordered; only independent channels parallelize.
        match (self.state.len(), self.feedback) {
            (1, false) => return self.process_small::<1, false>(data, cancel),
            (1, true) => return self.process_small::<1, true>(data, cancel),
            (2, false) => return self.process_small::<2, false>(data, cancel),
            (2, true) => return self.process_small::<2, true>(data, cancel),
            _ => {}
        }
        for chunk in data.chunks_mut(1024) {
            check_cancel(cancel)?;
            for x in chunk {
                *x = self.sample(*x);
            }
        }
        Ok(())
    }
}

pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}
pub fn vector(m: &Matrix) -> Result<&[f64]> {
    if m.is_empty() || (m.rows() > 1 && m.cols() > 1) {
        Err("Expected a nonempty coefficient vector".into())
    } else {
        Ok(m.data())
    }
}

fn channels<F>(x: &Matrix, cost: usize, cancel: &AtomicBool, process: F) -> Result<Matrix>
where
    F: Fn(&mut [f64]) -> Result<()> + Sync,
{
    if x.len().saturating_mul(cost) > MAX_WORK {
        return Err("Filter operation exceeds work limit; process shorter chunks".into());
    }
    check_cancel(cancel)?;
    if x.is_empty() {
        return Ok(x.clone());
    }
    let mut data = x.data().to_vec();
    let rows = if x.rows() == 1 { x.len() } else { x.rows() };
    // The threshold is benchmarked in benches/dsp.rs; only independent channels parallelize.
    if x.cols() > 1 && x.rows() > 1 && x.len().saturating_mul(cost) >= 1_000_000 {
        data.par_chunks_mut(rows).try_for_each(&process)?;
    } else {
        for channel in data.chunks_mut(rows) {
            process(channel)?;
        }
    }
    Matrix::from_column_vec(x.rows(), x.cols(), data)
}
pub fn filter(b: &[f64], a: &[f64], x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    filter_with_options(b, a, x, FilterOptions::default(), cancel).map(|(y, _)| y)
}
pub fn sos_sections(sos: &Matrix) -> Result<Vec<Filter>> {
    if sos.cols() != 6 || sos.rows() == 0 || sos.rows() > 64 {
        return Err("SOS must have 1..64 rows and six columns [b0 b1 b2 a0 a1 a2]".into());
    }
    (0..sos.rows())
        .map(|r| {
            let v: Vec<_> = (0..6).map(|c| sos.data()[c * sos.rows() + r]).collect();
            Filter::new(&v[..3], &v[3..])
        })
        .collect()
}
pub fn sosfilt(sos: &Matrix, x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    let sections = sos_sections(sos)?;
    channels(x, sections.len() * 3, cancel, |channel| {
        sos_pipeline::process(&sections, channel, cancel)
    })
}

pub fn sos_response(
    sos: &Matrix,
    frequencies: &[f64],
    cancel: &AtomicBool,
) -> Result<Vec<[f64; 2]>> {
    let sections = sos_sections(sos)?;
    if frequencies.len().saturating_mul(sections.len() * 3) > MAX_WORK {
        return Err("Frequency response exceeds work limit".into());
    }
    let mut result = vec![[1., 0.]; frequencies.len()];
    for section in sections {
        let h = response(&section.b, &section.a, frequencies, cancel)?;
        for (product, v) in result.iter_mut().zip(h) {
            *product = [
                product[0] * v[0] - product[1] * v[1],
                product[0] * v[1] + product[1] * v[0],
            ];
        }
    }
    Ok(result)
}

/// Real and imaginary components evaluated using complex Horner, without extra dependencies.
pub fn response(
    b: &[f64],
    a: &[f64],
    frequencies: &[f64],
    cancel: &AtomicBool,
) -> Result<Vec<[f64; 2]>> {
    Filter::new(b, a)?;
    if frequencies.len().saturating_mul(b.len().max(a.len())) > MAX_WORK {
        return Err("Frequency response exceeds work limit".into());
    }
    if frequencies
        .iter()
        .any(|f| !f.is_finite() || *f < 0. || *f > 1.)
    {
        return Err("Response frequencies must be between 0 and 1 (Nyquist)".into());
    }
    let mut out = Vec::with_capacity(frequencies.len());
    for (index, &f) in frequencies.iter().enumerate() {
        if index % 256 == 0 {
            check_cancel(cancel)?;
        }
        let (zi, zr) = (-PI * f).sin_cos();
        let eval = |coeff: &[f64]| {
            coeff.iter().rev().fold((0., 0.), |(r, i), &v| {
                (v + r * zr - i * zi, r * zi + i * zr)
            })
        };
        let (br, bi) = eval(b);
        let (ar, ai) = eval(a);
        // Smith's division avoids squaring a potentially large denominator.
        let (re, im) = if ar.abs() >= ai.abs() {
            let r = ai / ar;
            let d = ar + ai * r;
            ((br + bi * r) / d, (bi - br * r) / d)
        } else {
            let r = ar / ai;
            let d = ai + ar * r;
            ((br * r + bi) / d, (bi * r - br) / d)
        };
        out.push([re, im]);
    }
    Ok(out)
}
