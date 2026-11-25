//! Analytic wavelets and FFT kernels of the continuous wavelet transform and
//! of wavelet synchrosqueezing, from the published definitions:
//!
//! * generalised Morse wavelets, Psi(w) = 2 (w/w0)^beta exp(w0^gamma - w^gamma)
//!   with beta = P^2/gamma and peak frequency w0 = (beta/gamma)^(1/gamma)
//!   (Olhede & Walden 2002; Lilly & Olhede 2009, 2012);
//! * the analytic Morlet (Gabor) wavelet 'amor', Psi(w) = 2 exp(-(w - 6)^2/2);
//! * the bump wavelet, Psi(w) = 2 exp(1 - 1/(1 - ((w - 5)/0.6)^2)) on |w - 5| < 0.6.
//!
//! All are zero for w <= 0 and peak at 2 (L1 normalisation: a real sinusoid of
//! amplitude A has |wt| = A at its scale). The transform is the FFT form of
//! Torrence & Compo (1998); synchrosqueezing follows Daubechies, Lu & Wu
//! (2011) as discretised by Thakur, Brevdo, Fuckar & Wu (2013).
use crate::{Complex64, Result};
use rayon::prelude::*;
use rustfft::FftPlanner;
use std::f64::consts::{LN_2, PI};
use std::sync::atomic::{AtomicBool, Ordering};

/// Complex elements of one result (the 128 MiB complex array limit).
pub(super) const MAX_ELEMENTS: usize = 8_388_608;
/// Rows are transformed on several threads from this many spectrum points
/// (rows x FFT length); measured in docs/tasks/engine-for-apps-batch-7-120802-notes.md.
const PAR_POINTS: usize = 262_144;
const MAX_JOBS: usize = 8;

const AMOR_W0: f64 = 6.;
const BUMP_MU: f64 = 5.;
const BUMP_SIGMA: f64 = 0.6;
/// Coefficients below this fraction of the largest centred sample are not
/// reassigned by the synchrosqueezing (Thakur et al. use 1e-8 for doubles).
const SST_THRESHOLD: f64 = 1e-8;

const ZERO: Complex64 = Complex64::new(0., 0.);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Wavelet {
    Morse { gamma: f64, beta: f64 },
    Amor,
    Bump,
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn buffer(len: usize) -> Result<Vec<Complex64>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| "Cannot allocate the wavelet transform")?;
    v.resize(len, ZERO);
    Ok(v)
}

/// Composite Simpson rule with `n` (even) intervals.
fn simpson(f: impl Fn(f64) -> f64, a: f64, b: f64, n: usize) -> f64 {
    let h = (b - a) / n as f64;
    let mut sum = f(a) + f(b);
    for i in 1..n {
        sum += f(a + h * i as f64) * if i % 2 == 1 { 4. } else { 2. };
    }
    sum * h / 3.
}

/// exp(1 - 1/(1 - u^2)) inside (-1, 1), zero outside.
fn bump_shape(u: f64) -> f64 {
    if u.abs() < 1. {
        (1. - 1. / (1. - u * u)).exp()
    } else {
        0.
    }
}

impl Wavelet {
    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::Morse { .. } => "Morse",
            Self::Amor => "amor",
            Self::Bump => "bump",
        }
    }
    /// Peak (centre) frequency in radians per sample at scale 1.
    pub(super) fn peak(&self) -> f64 {
        match *self {
            Self::Morse { gamma, beta } => (beta / gamma).powf(1. / gamma),
            Self::Amor => AMOR_W0,
            Self::Bump => BUMP_MU,
        }
    }
    /// Frequency response at `w` radians per sample (scale 1).
    pub(super) fn psi(&self, w: f64) -> f64 {
        if w.is_nan() || w <= 0. {
            return 0.;
        }
        match *self {
            Self::Morse { gamma, beta } => {
                2. * (beta * (w / self.peak()).ln() - w.powf(gamma) + beta / gamma).exp()
            }
            Self::Amor => 2. * (-(w - AMOR_W0) * (w - AMOR_W0) / 2.).exp(),
            Self::Bump => 2. * bump_shape((w - BUMP_MU) / BUMP_SIGMA),
        }
    }
    /// Time width that sets the largest scale and the cone of influence:
    /// Morse - the dimensionless Lilly-Olhede time standard deviation
    /// w0 sqrt(int |Psi'|^2 / int |Psi|^2), about sqrt(P^2/2), which reduces to
    /// w0 2^(1/g) sqrt((g (2b - 1) + 1)/4 Gamma((2b - 1)/g) / Gamma((2b + 1)/g));
    /// amor - the e-folding time sqrt(2) of Torrence & Compo; bump - twice the
    /// energy standard deviation (the same convention as sqrt(2) for amor).
    pub(super) fn sigma_t(&self) -> f64 {
        match *self {
            Self::Morse { gamma, beta } => {
                let ratio = (libm::lgamma((2. * beta - 1.) / gamma)
                    - libm::lgamma((2. * beta + 1.) / gamma))
                .exp();
                self.peak()
                    * 2f64.powf(1. / gamma)
                    * ((gamma * (2. * beta - 1.) + 1.) / 4. * ratio).sqrt()
            }
            Self::Amor => std::f64::consts::SQRT_2,
            Self::Bump => {
                // The integrands vanish with all derivatives at +-1: Simpson converges fast.
                let slope = |u: f64| {
                    let d = 1. - u * u;
                    let v = bump_shape(u) * 2. * u / (d * d) / BUMP_SIGMA;
                    if u.abs() < 1. { v * v } else { 0. }
                };
                let energy = |u: f64| bump_shape(u) * bump_shape(u);
                2. * (simpson(slope, -1., 1., 8192) / simpson(energy, -1., 1., 8192)).sqrt()
            }
        }
    }
    /// The response of the smallest scale at the Nyquist frequency, as a
    /// fraction of the peak.
    fn nyquist_fraction(&self) -> f64 {
        match self {
            Self::Morse { .. } => 0.5,
            Self::Amor | Self::Bump => 0.1,
        }
    }
    /// Frequency above the peak where the response falls to the Nyquist fraction.
    pub(super) fn cutoff_frequency(&self) -> f64 {
        let fraction = self.nyquist_fraction();
        match *self {
            Self::Morse { gamma, beta } => {
                let w0 = self.peak();
                let target = fraction.ln();
                let f = |w: f64| beta * (w / w0).ln() - w.powf(gamma) + beta / gamma - target;
                let (mut lo, mut hi) = (w0, 2. * w0);
                while f(hi) > 0. {
                    lo = hi;
                    hi *= 2.;
                }
                for _ in 0..200 {
                    let mid = 0.5 * (lo + hi);
                    if mid <= lo || mid >= hi {
                        break;
                    }
                    if f(mid) > 0. {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                0.5 * (lo + hi)
            }
            Self::Amor => AMOR_W0 + (-2. * fraction.ln()).sqrt(),
            Self::Bump => BUMP_MU + BUMP_SIGMA * (1. - 1. / (1. - fraction.ln())).sqrt(),
        }
    }
    /// Admissibility constant of the single-integral inverse, int Psi(w)/w dw.
    /// The amor Gaussian does not vanish at w = 0: its tail below w = 0.5
    /// (under 3e-7 of the peak) is left out.
    pub(super) fn admissibility(&self) -> f64 {
        match *self {
            Self::Morse { gamma, beta } => {
                2. * (beta / gamma - beta * self.peak().ln() + libm::lgamma(beta / gamma)).exp()
                    / gamma
            }
            Self::Amor => simpson(|w| self.psi(w) / w, 0.5, AMOR_W0 + 8.5, 16384),
            Self::Bump => simpson(
                |w| self.psi(w) / w,
                BUMP_MU - BUMP_SIGMA,
                BUMP_MU + BUMP_SIGMA,
                8192,
            ),
        }
    }
    /// Lowest and highest wavelet centre frequency (cycles per sample) for an
    /// `n`-sample signal: two time widths of the largest wavelet span the
    /// signal; the smallest falls to the Nyquist fraction at pi.
    pub(super) fn bounds(&self, n: usize, nv: usize) -> (f64, f64) {
        let s0 = self.cutoff_frequency() / PI;
        let smax = (n as f64 / (2. * self.sigma_t())).max(s0 * 2f64.powf(1. / nv as f64));
        let w0 = self.peak();
        (w0 / (2. * PI * smax), w0 / (2. * PI * s0))
    }
    /// Scales s0 2^(k/nv) (samples) and their centre frequencies w0/(2 pi s) fs
    /// between the frequency limits `lo` and `hi` (in the units of `fs`).
    ///
    /// The octave count is formed only from a finite positive ratio. A
    /// non-finite `hi/lo` used to saturate the `usize` cast and overflow `+1`,
    /// which released an empty grid.
    pub(super) fn grid(
        &self,
        lo: f64,
        hi: f64,
        fs: f64,
        nv: usize,
    ) -> Result<(Vec<f64>, Vec<f64>)> {
        let w0 = self.peak();
        let ratio = hi / lo;
        // Same octave count as before whenever the ratio is finite, including
        // a non-positive log (the old `floor(...).max(0)`). A non-finite ratio
        // is an error instead of a saturating cast.
        let octaves = if lo.is_finite()
            && hi.is_finite()
            && fs.is_finite()
            && lo > 0.
            && hi > 0.
            && fs > 0.
            && ratio.is_finite()
        {
            ratio.log2()
        } else {
            f64::NAN
        };
        let count_f = if octaves.is_finite() {
            (nv as f64 * octaves + 1e-9).floor().max(0.)
        } else {
            f64::NAN
        };
        // `as usize` saturates past usize::MAX, and the following +1 wraps.
        if !count_f.is_finite() || count_f < 0. || count_f >= usize::MAX as f64 {
            return Err("the frequency range is not representable".into());
        }
        let count = (count_f as usize)
            .checked_add(1)
            .ok_or("the frequency range is not representable")?;
        let s0 = w0 * fs / (2. * PI * hi);
        if !s0.is_finite() || s0 <= 0. {
            return Err("the frequency range is not representable".into());
        }
        let scales: Vec<f64> = (0..count)
            .map(|k| s0 * 2f64.powf(k as f64 / nv as f64))
            .collect();
        let freqs: Vec<f64> = scales.iter().map(|s| w0 / (2. * PI * s) * fs).collect();
        if scales.is_empty() || freqs.is_empty() {
            return Err("the frequency grid is empty".into());
        }
        Ok((scales, freqs))
    }
    /// Cone of influence: the frequency whose wavelet is one time width long
    /// at each sample's distance from the nearer edge, at most `fmax`.
    pub(super) fn coi(&self, n: usize, fs: f64, fmax: f64) -> Vec<f64> {
        let k = self.peak() * self.sigma_t() / (2. * PI) * fs;
        (0..n)
            .map(|i| (k / (i + 1).min(n - i) as f64).min(fmax))
            .collect()
    }
    /// `out[k] = x[k] * gain * Psi(s w_k)` over the positive frequencies
    /// w_k = k dw (k <= m/2), or with `negative` over the mirrored negative
    /// ones (k > m/2, |w_k| = (m - k) dw); every other bin is zero. With
    /// `derivative` the bins are also multiplied by i w_k.
    fn spectrum(&self, s: f64, gain: f64, bins: Bins, x: &[Complex64], out: &mut [Complex64]) {
        match *self {
            Self::Morse { gamma, beta } => {
                let ln_w0 = self.peak().ln();
                let c = beta / gamma;
                if gamma.fract() == 0. && gamma <= 16. {
                    let g = gamma as i32;
                    fill(s, gain, bins, x, out, |w| {
                        2. * (beta * (w.ln() - ln_w0) - w.powi(g) + c).exp()
                    });
                } else {
                    fill(s, gain, bins, x, out, |w| {
                        2. * (beta * (w.ln() - ln_w0) - w.powf(gamma) + c).exp()
                    });
                }
            }
            Self::Amor => fill(s, gain, bins, x, out, |w| {
                2. * (-(w - AMOR_W0) * (w - AMOR_W0) / 2.).exp()
            }),
            Self::Bump => fill(s, gain, bins, x, out, |w| {
                2. * bump_shape((w - BUMP_MU) / BUMP_SIGMA)
            }),
        }
    }
}

/// Which DFT bins a filter covers and whether it also differentiates in time.
#[derive(Clone, Copy)]
struct Bins {
    negative: bool,
    derivative: bool,
}

fn fill(
    s: f64,
    gain: f64,
    bins: Bins,
    x: &[Complex64],
    out: &mut [Complex64],
    psi: impl Fn(f64) -> f64,
) {
    let m = x.len();
    let dw = 2. * PI / m as f64;
    out.fill(ZERO);
    let range = if bins.negative {
        m / 2 + 1..m
    } else {
        1..m / 2 + 1
    };
    for k in range {
        let w = if bins.negative { m - k } else { k } as f64 * dw;
        let g = gain * psi(s * w);
        if g != 0. {
            out[k] = if bins.derivative {
                x[k] * Complex64::new(0., g * w)
            } else {
                x[k] * g
            };
        }
    }
}

/// `[x(pad) .. x(1), x, x(n) .. x(n - pad + 1)]`: reflection about both ends.
fn extended<T: Copy>(x: &[T], pad: usize, lift: impl Fn(T) -> Complex64) -> Result<Vec<Complex64>> {
    let n = x.len();
    let mut v = Vec::new();
    v.try_reserve_exact(n + 2 * pad)
        .map_err(|_| "Cannot allocate the wavelet transform")?;
    v.extend(x[..pad].iter().rev().map(|&a| lift(a)));
    v.extend(x.iter().map(|&a| lift(a)));
    v.extend(x[n - pad..].iter().rev().map(|&a| lift(a)));
    Ok(v)
}

fn forward(mut x: Vec<Complex64>, cancel: &AtomicBool) -> Result<Vec<Complex64>> {
    check(cancel)?;
    FftPlanner::<f64>::new()
        .plan_fft_forward(x.len())
        .process(&mut x);
    Ok(x)
}

/// Rows 0..count: `make(row, spectrum)` fills a length-`m` spectrum, an
/// inverse FFT (unscaled) follows and `store(row, samples)` receives the rows
/// in order. Batches of rows run on the Rayon pool when the work is large.
fn rows(
    count: usize,
    m: usize,
    cancel: &AtomicBool,
    make: impl Fn(usize, &mut [Complex64]) + Sync,
    mut store: impl FnMut(usize, &[Complex64]),
) -> Result<()> {
    check(cancel)?;
    let fft = FftPlanner::<f64>::new().plan_fft_inverse(m);
    let scratch = fft.get_inplace_scratch_len();
    let jobs = if count.saturating_mul(m) >= PAR_POINTS {
        rayon::current_num_threads().min(MAX_JOBS).min(count).max(1)
    } else {
        1
    };
    let mut buffers = Vec::with_capacity(jobs);
    for _ in 0..jobs {
        buffers.push((buffer(m)?, buffer(scratch)?));
    }
    let mut first = 0;
    while first < count {
        check(cancel)?;
        let take = jobs.min(count - first);
        if take == 1 {
            let (b, s) = &mut buffers[0];
            make(first, b);
            fft.process_with_scratch(b, s);
        } else {
            buffers[..take]
                .par_iter_mut()
                .enumerate()
                .for_each(|(i, (b, s))| {
                    make(first + i, b);
                    fft.process_with_scratch(b, s);
                });
        }
        for (i, (b, _)) in buffers[..take].iter().enumerate() {
            store(first + i, b);
        }
        first += take;
    }
    check(cancel)
}

pub(super) enum Samples {
    Real(Vec<f64>),
    Complex(Vec<Complex64>),
}
impl Samples {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Real(v) => v.len(),
            Self::Complex(v) => v.len(),
        }
    }
}

/// Continuous wavelet transform, column-major `scales x n` (real signal) or
/// `scales x n x 2` (complex signal: the analytic part from the positive
/// frequencies, then the anti-analytic part from the negative ones, both
/// halved so that a complex exponential of amplitude A has magnitude A).
pub(super) fn transform(
    x: &Samples,
    wavelet: &Wavelet,
    scales: &[f64],
    extend: bool,
    cancel: &AtomicBool,
) -> Result<Vec<Complex64>> {
    let n = x.len();
    let na = scales.len();
    let pad = if extend { n / 2 } else { 0 };
    let (spectrum, pages) = match x {
        Samples::Real(v) => (extended(v, pad, |a| Complex64::new(a, 0.))?, 1),
        Samples::Complex(v) => (extended(v, pad, |a| a)?, 2),
    };
    let m = spectrum.len();
    let spectrum = forward(spectrum, cancel)?;
    let gain = 1. / (m as f64 * pages as f64);
    let mut out = buffer(na * n * pages)?;
    rows(
        na * pages,
        m,
        cancel,
        |r, b| {
            let bins = Bins {
                negative: r >= na,
                derivative: false,
            };
            wavelet.spectrum(scales[r % na], gain, bins, &spectrum, b);
        },
        |r, row| {
            let base = (r / na) * na * n + r % na;
            for (i, v) in row[pad..pad + n].iter().enumerate() {
                out[base + na * i] = *v;
            }
        },
    )?;
    Ok(out)
}

/// Frequency responses Psi(s w_k) of the filters on an `m`-point DFT grid,
/// column-major `scales x cols`: `cols = m/2 + 1` one-sided, else `m` in DFT
/// order (the negative-frequency half is zero).
pub(super) fn responses(wavelet: &Wavelet, scales: &[f64], m: usize, twosided: bool) -> Vec<f64> {
    let na = scales.len();
    let cols = if twosided { m } else { m / 2 + 1 };
    let dw = 2. * PI / m as f64;
    let mut out = vec![0.; na * cols];
    for k in 0..m / 2 + 1 {
        for (j, s) in scales.iter().enumerate() {
            out[j + na * k] = wavelet.psi(s * k as f64 * dw);
        }
    }
    out
}

/// Time-domain wavelets of the filters on an `n`-point grid, centred so that
/// column floor(n/2) is time zero; column-major `scales x n`.
pub(super) fn wavelets(
    wavelet: &Wavelet,
    scales: &[f64],
    n: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Complex64>> {
    let na = scales.len();
    let ones = vec![Complex64::new(1., 0.); n];
    let mut out = buffer(na * n)?;
    rows(
        na,
        n,
        cancel,
        |j, b| {
            let bins = Bins {
                negative: false,
                derivative: false,
            };
            wavelet.spectrum(scales[j], 1. / n as f64, bins, &ones, b);
        },
        |j, row| {
            for (i, v) in row.iter().enumerate() {
                out[j + na * ((i + n / 2) % n)] = *v;
            }
        },
    )?;
    Ok(out)
}

/// Number of synchrosqueezing scales (and frequency bins) for `n` samples.
pub(super) fn sst_rows(n: usize, nv: usize) -> usize {
    nv * (n.ilog2() as usize - 1)
}

/// Frequency bins of the synchrosqueezed transform: log-spaced from 1/n to
/// 1/2 cycles per sample, times `fs`.
pub(super) fn sst_frequencies(n: usize, na: usize, fs: f64) -> Vec<f64> {
    let lo = (1. / n as f64).log2();
    let step = (-1. - lo) / (na - 1) as f64;
    (0..na)
        .map(|l| {
            if l + 1 == na {
                0.5 * fs
            } else {
                2f64.powf(lo + step * l as f64) * fs
            }
        })
        .collect()
}

/// Wavelet synchrosqueezed transform, column-major `rows x n`: the CWT at the
/// scales 2^(k/nv), k = 1..rows, is reassigned along frequency to the bin
/// nearest (in log2) to the magnitude of the phase transform Im(dW/W)/(2 pi),
/// clamped to the first and last bin so that every coefficient above the
/// threshold is kept, and weighted by log(2)/nv (the scale integral da/a).
pub(super) fn synchrosqueeze(
    x: &[f64],
    wavelet: &Wavelet,
    nv: usize,
    extend: bool,
    cancel: &AtomicBool,
) -> Result<Vec<Complex64>> {
    let n = x.len();
    let na = sst_rows(n, nv);
    let pad = if extend { n / 2 } else { 0 };
    let mean = x.iter().sum::<f64>() / n as f64;
    let threshold = SST_THRESHOLD * x.iter().fold(0f64, |a, v| a.max((v - mean).abs()));
    let mut out = buffer(na * n)?;
    if threshold.is_nan() || threshold <= 0. {
        return Ok(out);
    }
    let spectrum = forward(extended(x, pad, |a| Complex64::new(a, 0.))?, cancel)?;
    let m = spectrum.len();
    let lo = (1. / n as f64).log2();
    let step = (-1. - lo) / (na - 1) as f64;
    let weight = LN_2 / nv as f64;
    let mut held = buffer(n)?;
    rows(
        2 * na,
        m,
        cancel,
        |r, b| {
            let s = 2f64.powf((r / 2 + 1) as f64 / nv as f64);
            let bins = Bins {
                negative: false,
                derivative: r % 2 == 1,
            };
            wavelet.spectrum(s, 1. / m as f64, bins, &spectrum, b);
        },
        |r, row| {
            let row = &row[pad..pad + n];
            if r % 2 == 0 {
                held.copy_from_slice(row);
                return;
            }
            for (i, (w, dw)) in held.iter().zip(row).enumerate() {
                let power = w.norm_sqr();
                if power.sqrt() <= threshold {
                    continue;
                }
                let phase = ((dw.im * w.re - dw.re * w.im) / power / (2. * PI)).abs();
                if phase > 0. && phase.is_finite() {
                    let bin = ((phase.log2() - lo) / step + 0.5).floor();
                    let bin = bin.clamp(0., (na - 1) as f64) as usize;
                    out[bin + na * i] += w * weight;
                }
            }
        },
    )?;
    Ok(out)
}
