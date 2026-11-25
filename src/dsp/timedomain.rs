//! Time-domain DSP derived from published signal mathematics, not vendor source.
//! Original methods: Savitzky & Golay, "Smoothing and Differentiation of Data
//! by Simplified Least Squares Procedures" (1964), polynomial least-squares fit;
//! McClellan, Parks & Rabiner, "A Computer Program for Designing Optimum FIR
//! Linear Phase Digital Filters" (1973), alternating-error exchange.
//! Accessible mathematical definitions: SciPy contributors, SciPy Signal API
//! Reference, peak_prominences/peak_widths/firwin2 Notes, remez and savgol_coeffs
//! (accessed 2026-10-30). Kaiser order and beta formulas: kaiserord/kaiser_beta.
//! No paper implementation, vendor source or Numerical Recipes code was used.
use super::{Band, FirOptions, check_cancel};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use nalgebra::DMatrix;
use rayon::prelude::*;
use std::{f64::consts::PI, sync::atomic::AtomicBool};

// Admit common windows/FIRs at the full sample limit; retain a guard against
// genuinely huge products (e.g. a 4097-point median over a million samples).
const MAX_WORK: usize = 2_000_000_000;
const MAX_REMEZ_WORK: usize = 256_000_000;
const MAX_TAPS: usize = 4097;
const MAX_SAMPLES: usize = 1_048_576;

fn finite(x: &Matrix) -> Result<()> {
    if x.len() > MAX_SAMPLES || x.data().iter().any(|v| !v.is_finite()) {
        Err("Signal must be finite and contain at most 1048576 elements".into())
    } else {
        Ok(())
    }
}
fn shape(x: &Matrix) -> (usize, usize) {
    if x.rows() == 1 {
        (x.cols(), 1)
    } else {
        (x.rows(), x.cols())
    }
}
fn output(
    x: &Matrix,
    samples: usize,
    channels: usize,
    data: Vec<f64>,
    c: &AtomicBool,
) -> Result<Matrix> {
    check_cancel(c)?;
    if data.iter().any(|v| !v.is_finite()) {
        return Err("Signal computation overflowed".into());
    }
    if x.rows() == 1 {
        Matrix::from_column_vec(1, samples, data)
    } else {
        Matrix::from_column_vec(samples, channels, data)
    }
}
fn work(n: usize, cost: usize, c: &AtomicBool) -> Result<()> {
    check_cancel(c)?;
    if n.saturating_mul(cost) > MAX_WORK {
        Err("Signal operation exceeds work limit".into())
    } else {
        Ok(())
    }
}

// Conservative admission based on release measurements in the task notes.
// At most eight jobs; all callers reuse their pool and all kernels stay serial
// inside a job. A unit is one sample, except for stateful IIRs (one column).
fn process_chunks(
    data: &mut [f64],
    unit: usize,
    cost: usize,
    c: &AtomicBool,
    process: impl Fn(usize, &mut [f64]) -> Result<()> + Sync,
) -> Result<()> {
    check_cancel(c)?;
    if crate::parallel::current_worker_count() > 1
        && data.len().saturating_mul(cost) >= 1_000_000
        && data.len() / unit > 1
    {
        let chunk = (data.len() / unit).div_ceil(8) * unit;
        data.par_chunks_mut(chunk)
            .enumerate()
            .try_for_each(|(j, part)| process(j * chunk, part))?;
    } else {
        process(0, data)?;
    }
    check_cancel(c)
}

// Split a job at column boundaries once, keeping integer division out of hot
// per-sample loops. Input and output lanes may have different sample counts.
fn output_lanes(
    x: &Matrix,
    out_samples: usize,
    first: usize,
    data: &mut [f64],
    mut process: impl FnMut(&[f64], usize, &mut [f64]) -> Result<()>,
) -> Result<()> {
    let samples = shape(x).0;
    let mut offset = 0;
    while offset < data.len() {
        let index = first + offset;
        let lane = index / out_samples;
        let begin = index % out_samples;
        let count = (out_samples - begin).min(data.len() - offset);
        process(
            &x.data()[lane * samples..(lane + 1) * samples],
            begin,
            &mut data[offset..offset + count],
        )?;
        offset += count;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct PeakOptions {
    pub min_height: f64,
    pub min_prominence: f64,
    pub min_distance: f64,
    pub threshold: f64,
    pub count: usize,
    pub sort: PeakSort,
    pub half_height: bool,
    pub min_width: f64,
    pub max_width: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeakSort {
    None,
    Ascend,
    Descend,
}
impl Default for PeakOptions {
    fn default() -> Self {
        Self {
            min_height: f64::NEG_INFINITY,
            min_prominence: 0.,
            min_distance: 0.,
            threshold: 0.,
            count: usize::MAX,
            sort: PeakSort::None,
            half_height: false,
            min_width: 0.,
            max_width: f64::INFINITY,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Peaks {
    pub heights: Vec<f64>,
    pub locations: Vec<f64>,
    pub widths: Vec<f64>,
    pub prominences: Vec<f64>,
}
// Two linear-space segment trees bound both topographic base and crossing
// queries to O(log N). This avoids quadratic scans on repeating equal peaks.
struct PeakTree {
    size: usize,
    minima: Vec<f64>,
    maxima: Vec<f64>,
}
impl PeakTree {
    fn new(a: &[f64], c: &AtomicBool) -> Result<Self> {
        let size = a.len().max(1).next_power_of_two();
        let mut minima = vec![f64::INFINITY; 2 * size];
        let mut maxima = vec![f64::NEG_INFINITY; 2 * size];
        minima[size..size + a.len()].copy_from_slice(a);
        maxima[size..size + a.len()].copy_from_slice(a);
        for j in (1..size).rev() {
            if j.is_multiple_of(4096) {
                check_cancel(c)?;
            }
            minima[j] = minima[2 * j].min(minima[2 * j + 1]);
            maxima[j] = maxima[2 * j].max(maxima[2 * j + 1]);
        }
        Ok(Self {
            size,
            minima,
            maxima,
        })
    }
    fn minimum(&self, mut lo: usize, mut hi: usize) -> f64 {
        lo += self.size;
        hi += self.size;
        let mut value = f64::INFINITY;
        while lo < hi {
            if lo % 2 == 1 {
                value = value.min(self.minima[lo]);
                lo += 1;
            }
            if hi % 2 == 1 {
                hi -= 1;
                value = value.min(self.minima[hi]);
            }
            lo /= 2;
            hi /= 2;
        }
        value
    }
    // Half-open query interval. Search the nearest sample, in either direction,
    // whose value is strictly higher (above) or at/below the threshold.
    fn search(
        &self,
        lo: usize,
        hi: usize,
        level: f64,
        above: bool,
        reverse: bool,
    ) -> Option<usize> {
        struct Query {
            lo: usize,
            hi: usize,
            level: f64,
            above: bool,
            reverse: bool,
        }
        fn visit(t: &PeakTree, node: usize, first: usize, last: usize, q: &Query) -> Option<usize> {
            if first >= q.hi
                || last <= q.lo
                || if q.above {
                    t.maxima[node] <= q.level
                } else {
                    t.minima[node] > q.level
                }
            {
                return None;
            }
            if last - first == 1 {
                return Some(first);
            }
            let mid = (first + last) / 2;
            if q.reverse {
                visit(t, 2 * node + 1, mid, last, q).or_else(|| visit(t, 2 * node, first, mid, q))
            } else {
                visit(t, 2 * node, first, mid, q).or_else(|| visit(t, 2 * node + 1, mid, last, q))
            }
        }
        visit(
            self,
            1,
            0,
            self.size,
            &Query {
                lo,
                hi,
                level,
                above,
                reverse,
            },
        )
    }
}
/// Endpoint samples are excluded; equal-height plateaus use their first sample.
/// Bases stop at the boundary or a strictly higher sample; equal peaks are ignored.
pub fn findpeaks(
    y: &Matrix,
    axis: Option<&[f64]>,
    o: &PeakOptions,
    c: &AtomicBool,
) -> Result<Peaks> {
    finite(y)?;
    if y.rows() > 1 && y.cols() > 1 {
        return Err("findpeaks requires a vector".into());
    }
    let a = y.data();
    if let Some(x) = axis
        && (x.len() != a.len()
            || x.iter().any(|v| !v.is_finite())
            || x.windows(2).any(|p| p[0] >= p[1]))
    {
        return Err(
            "Peak coordinates must be finite, strictly increasing and match the signal".into(),
        );
    }
    if o.min_height.is_nan()
        || o.min_height == f64::INFINITY
        || [o.min_prominence, o.min_distance, o.threshold, o.min_width]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.)
        || o.max_width.is_nan()
        || o.max_width < o.min_width
    {
        return Err("Invalid findpeaks option value".into());
    }
    let coord = |i: usize| axis.map_or(i as f64 + 1., |x| x[i]);
    let mut peaks = Vec::new();
    let mut i = 1;
    check_cancel(c)?;
    let mut tree = None;
    while i + 1 < a.len() {
        if i.is_multiple_of(1024) {
            check_cancel(c)?;
        }
        let start = i;
        while i + 1 < a.len() && a[i + 1] == a[start] {
            i += 1;
        }
        let end = i;
        i += 1;
        if end + 1 == a.len()
            || a[start] <= a[start - 1]
            || a[end] <= a[end + 1]
            || a[start] <= o.min_height
            || (a[start] - a[start - 1]).min(a[end] - a[end + 1]) < o.threshold
        {
            continue;
        }
        if tree.is_none() {
            tree = Some(PeakTree::new(a, c)?);
        }
        let tree = tree.as_ref().unwrap();
        let h = a[start];
        let lb = tree.search(0, start, h, true, true).map_or(0, |j| j + 1);
        let rb = tree
            .search(end + 1, a.len(), h, true, false)
            .map_or(a.len() - 1, |j| j - 1);
        let p = h - tree.minimum(lb, start + 1).max(tree.minimum(end, rb + 1));
        if !p.is_finite() {
            return Err("Peak prominence overflowed".into());
        }
        if p < o.min_prominence {
            continue;
        }
        let level = if o.half_height { h / 2. } else { h - p / 2. };
        let left_bound = if o.half_height { 0 } else { lb };
        let right_bound = if o.half_height { a.len() - 1 } else { rb };
        let l = tree
            .search(left_bound, start + 1, level, false, true)
            .unwrap_or(left_bound);
        let r = tree
            .search(end, right_bound + 1, level, false, false)
            .unwrap_or(right_bound);
        let mut lx = coord(l);
        let mut rx = coord(r);
        if l < start && a[l] < level {
            lx += (level - a[l]) / (a[l + 1] - a[l]) * (coord(l + 1) - coord(l));
        }
        if r > end && a[r] < level {
            rx -= (level - a[r]) / (a[r - 1] - a[r]) * (coord(r) - coord(r - 1));
        }
        let w = if level >= h { 0. } else { rx - lx };
        if !w.is_finite() {
            return Err("Peak width overflowed".into());
        }
        if w >= o.min_width && w <= o.max_width {
            peaks.push((start, h, w, p));
        }
    }
    // Deterministic greedy suppression, tallest first; ties choose the first sample.
    if o.min_distance > 0. || o.sort == PeakSort::Descend {
        peaks.sort_by(|a, b| {
            if a.1 == b.1 {
                a.0.cmp(&b.0)
            } else {
                b.1.total_cmp(&a.1)
            }
        });
    }
    let mut kept: Vec<(usize, f64, f64, f64)> = Vec::new();
    let mut indices = std::collections::BTreeSet::new();
    if o.min_distance == 0. {
        kept = peaks;
    } else {
        for (j, p) in peaks.into_iter().enumerate() {
            if j.is_multiple_of(1024) {
                check_cancel(c)?;
            }
            let left = indices.range(..p.0).next_back().copied();
            let right = indices.range(p.0..).next().copied();
            if [left, right]
                .into_iter()
                .flatten()
                .all(|q| (coord(p.0) - coord(q)).abs() >= o.min_distance)
            {
                indices.insert(p.0);
                kept.push(p);
            }
        }
    }
    match o.sort {
        PeakSort::None if o.min_distance > 0. => kept.sort_by_key(|p| p.0),
        PeakSort::Ascend => kept.sort_by(|a, b| {
            if a.1 == b.1 {
                a.0.cmp(&b.0)
            } else {
                a.1.total_cmp(&b.1)
            }
        }),
        _ => {}
    }
    kept.truncate(o.count);
    check_cancel(c)?;
    Ok(Peaks {
        heights: kept.iter().map(|p| p.1).collect(),
        locations: kept.iter().map(|p| coord(p.0)).collect(),
        widths: kept.iter().map(|p| p.2).collect(),
        prominences: kept.iter().map(|p| p.3).collect(),
    })
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn beta(a: f64) -> f64 {
    if a > 50. {
        0.1102 * (a - 8.7)
    } else if a >= 21. {
        0.5842 * (a - 21.).powf(0.4) + 0.07886 * (a - 21.)
    } else {
        0.
    }
}
/// Centered polyphase convolution: evaluates only the requested output phase.
/// Implicit zero extension, no explicit zero-inserted array; delay removed exactly.
fn polyphase(x: &Matrix, h: &[f64], p: usize, q: usize, c: &AtomicBool) -> Result<Matrix> {
    finite(x)?;
    let (n, channels) = shape(x);
    let count = n
        .checked_mul(p)
        .ok_or("Resampling dimensions overflow")?
        .div_ceil(q);
    if count.saturating_mul(channels) > MAX_SAMPLES {
        return Err("Resampling output exceeds sample limit".into());
    }
    work(count.saturating_mul(channels), h.len().div_ceil(p), c)?;
    if x.is_empty() {
        return Ok(x.clone());
    }
    let center = h.len() / 2;
    let mut out = vec![0.; count * channels];
    let process = |first: usize, chunk: &mut [f64]| -> Result<()> {
        output_lanes(x, count, first, chunk, |lane, begin, result| {
            for (offset, y) in result.iter_mut().enumerate() {
                if offset.is_multiple_of(1024) {
                    check_cancel(c)?;
                }
                let j = begin + offset;
                let t = center + j * q;
                let first = t.saturating_sub(h.len() - 1).div_ceil(p);
                let last = (t / p).min(n - 1);
                if first <= last {
                    *y = (first..=last).map(|k| lane[k] * h[t - k * p]).sum();
                }
            }
            Ok(())
        })
    };
    process_chunks(&mut out, 1, h.len().div_ceil(p), c, process)?;
    output(x, count, channels, out, c)
}
fn resampling_fir(p: usize, q: usize, n: usize, b: f64, c: &AtomicBool) -> Result<Vec<f64>> {
    let rate = p.max(q);
    let order = n
        .checked_mul(2)
        .and_then(|x| x.checked_mul(rate))
        .ok_or("Resampling order overflow")?;
    if order == 0 || order >= MAX_TAPS {
        return Err("Resampling FIR order must be 1..4096".into());
    }
    let w = super::kaiser(order + 1, b, c)?;
    // fir1 excludes Nyquist; identity-rate conversion is handled by the caller.
    let mut h = super::fir1_with_options(
        order,
        &[1. / rate as f64],
        Band::Lowpass,
        FirOptions {
            window: Some(w.data()),
            scale: true,
        },
        c,
    )?;
    h.iter_mut().for_each(|v| *v *= p as f64);
    Ok(h)
}
pub fn resample(
    x: &Matrix,
    p: usize,
    q: usize,
    n: usize,
    b: f64,
    c: &AtomicBool,
) -> Result<Matrix> {
    if p == 0
        || q == 0
        || p > 65536
        || q > 65536
        || n == 0
        || n > 2048
        || !b.is_finite()
        || !(0. ..=100.).contains(&b)
    {
        return Err("resample needs bounded positive p, q, n and beta in 0..100".into());
    }
    finite(x)?;
    check_cancel(c)?;
    let d = gcd(p, q);
    let (p, q) = (p / d, q / d);
    if p == q {
        return Ok(x.clone());
    }
    let h = resampling_fir(p, q, n, b, c)?;
    polyphase(x, &h, p, q, c)
}
pub fn decimate(
    x: &Matrix,
    r: usize,
    order: Option<usize>,
    fir: bool,
    c: &AtomicBool,
) -> Result<Matrix> {
    if !(2..=1024).contains(&r) {
        return Err("decimate ratio must be 2..1024".into());
    }
    if fir {
        let n = order.unwrap_or(30);
        let h =
            super::fir1_with_options(n, &[1. / r as f64], Band::Lowpass, FirOptions::default(), c)?;
        // Centered convolution requires an integer group delay.
        if !n.is_multiple_of(2) {
            return Err("FIR decimation requires an even order".into());
        }
        polyphase(x, &h, 1, r, c)
    } else {
        let design = super::cheby1(
            order.unwrap_or(8),
            0.05,
            &[0.8 / r as f64],
            Band::Lowpass,
            c,
        )?;
        let sos = design.sos(c)?;
        let y = decimate_zero_phase(&sos, x, 3 * (order.unwrap_or(8) + 1), c)?;
        super::downsample(&y, r, 0, c)
    }
}
// SOS forward/backward filtering with odd reflection and the decimation
// convention 3*(order+1). Existing filtfilt/filtfilt_sos behavior is untouched.
fn decimate_zero_phase(sos: &Matrix, x: &Matrix, pad: usize, c: &AtomicBool) -> Result<Matrix> {
    finite(x)?;
    let (samples, channels) = shape(x);
    if x.is_empty() {
        check_cancel(c)?;
        return Ok(x.clone());
    }
    if samples <= pad {
        return Err(format!(
            "IIR decimation requires more than {pad} samples per channel"
        ));
    }
    work(
        (samples + 2 * pad).saturating_mul(channels),
        4 * sos.rows(),
        c,
    )?;
    let mut out = vec![0.; x.len()];
    process_chunks(&mut out, samples, 4 * sos.rows(), c, |first, result| {
        for (channel, dest) in result.chunks_mut(samples).enumerate() {
            check_cancel(c)?;
            let begin = first + channel * samples;
            let lane = &x.data()[begin..begin + samples];
            let mut values = Vec::with_capacity(samples + 2 * pad);
            values.extend((1..=pad).rev().map(|j| libm::fma(lane[0], 2., -lane[j])));
            values.extend_from_slice(lane);
            values.extend(
                (1..=pad).map(|j| libm::fma(lane[samples - 1], 2., -lane[samples - 1 - j])),
            );
            if values.iter().any(|v| !v.is_finite()) {
                return Err("IIR reflection overflowed".into());
            }
            for _ in 0..2 {
                for section in 0..sos.rows() {
                    let b: Vec<_> = (0..3).map(|j| sos.get(section, j)).collect();
                    let a: Vec<_> = (3..6).map(|j| sos.get(section, j)).collect();
                    let gain = b.iter().sum::<f64>() / a.iter().sum::<f64>();
                    let endpoint = values[0];
                    let mut filter = super::Filter::new(&b, &a)?;
                    filter.set_state(&[
                        (gain - b[0] / a[0]) * endpoint,
                        (b[2] - gain * a[2]) / a[0] * endpoint,
                    ])?;
                    filter.process(&mut values, c)?;
                }
                values.reverse();
            }
            dest.copy_from_slice(&values[pad..pad + samples]);
        }
        Ok(())
    })?;
    output(x, samples, channels, out, c)
}
/// Cardinal windowed-sinc interpolation: 2*l*r+1 taps, cutoff 1/r, beta=5.
/// alpha declares the input band of interest; this fixed-window design keeps
/// the cutoff at input Nyquist. It does not rescale the cardinal sinc for DC.
pub fn interp(x: &Matrix, r: usize, l: usize, alpha: f64, c: &AtomicBool) -> Result<Matrix> {
    if !(1..=1024).contains(&r)
        || !(1..=2048).contains(&l)
        || !alpha.is_finite()
        || alpha <= 0.
        || alpha > 1.
    {
        return Err("interp needs positive bounded r, l and alpha in (0,1]".into());
    }
    if r == 1 {
        finite(x)?;
        check_cancel(c)?;
        return Ok(x.clone());
    }
    let n = 2 * l * r;
    if n >= MAX_TAPS {
        return Err("Interpolation order exceeds 4096".into());
    }
    let window = super::kaiser(n + 1, 5., c)?;
    let mut h = super::fir1_with_options(
        n,
        &[1. / r as f64],
        Band::Lowpass,
        FirOptions {
            window: Some(window.data()),
            scale: false,
        },
        c,
    )?;
    h.iter_mut().for_each(|v| *v *= r as f64);
    polyphase(x, &h, r, 1, c)
}

pub fn hilbert(x: &Matrix, c: &AtomicBool) -> Result<ComplexMatrix> {
    finite(x)?;
    if x.is_empty() {
        return ComplexMatrix::from_column_vec(x.rows(), x.cols(), Vec::new(), c);
    }
    let a = super::fourier(
        &Array::Numeric(x.clone()),
        super::FftOptions::default(),
        super::TransformDirection::Forward,
        256 * 1024 * 1024,
        c,
    )?;
    let Array::Complex(z) = a else {
        return Err("FFT returned noncomplex data".into());
    };
    let (n, _) = shape(x);
    if n == 0 {
        return Ok(z);
    }
    let mut spectrum = z.data().to_vec();
    for lane in spectrum.chunks_mut(n) {
        for (k, v) in lane.iter_mut().enumerate() {
            if k.is_multiple_of(4096) {
                check_cancel(c)?;
            }
            let gain = if k == 0 || (n.is_multiple_of(2) && k == n / 2) {
                1.
            } else if k <= (n - 1) / 2 {
                2.
            } else {
                0.
            };
            *v *= gain;
        }
    }
    let a = Array::Complex(ComplexMatrix::from_column_vec(
        x.rows(),
        x.cols(),
        spectrum,
        c,
    )?);
    match super::fourier(
        &a,
        super::FftOptions::default(),
        super::TransformDirection::Inverse,
        256 * 1024 * 1024,
        c,
    )? {
        Array::Complex(z)
            if z.data()
                .iter()
                .all(|v| v.re.is_finite() && v.im.is_finite()) =>
        {
            Ok(z)
        }
        Array::Numeric(m) => {
            finite(&m)?;
            ComplexMatrix::from_real(&m, c)
        }
        _ => Err("Analytic signal overflowed".into()),
    }
}
#[derive(Clone, Copy, Debug)]
pub enum EnvelopeKind {
    Analytic,
    Rms,
    Peak,
}
pub fn envelope(
    x: &Matrix,
    n: usize,
    kind: EnvelopeKind,
    c: &AtomicBool,
) -> Result<(Matrix, Matrix)> {
    check_cancel(c)?;
    finite(x)?;
    if n == 0 || n > MAX_TAPS {
        return Err("Envelope window must be 1..4097".into());
    }
    let (samples, channels) = shape(x);
    if x.is_empty() {
        return Ok((x.clone(), x.clone()));
    }
    work(
        x.len(),
        if matches!(kind, EnvelopeKind::Rms) {
            2 * n
        } else {
            1
        },
        c,
    )?;
    let mut upper = vec![0.; x.len()];
    let mut lower = upper.clone();
    match kind {
        EnvelopeKind::Analytic => {
            // Remove and restore each column's mean. n is accepted but no FIR approximation is used.
            let mut centered = x.data().to_vec();
            let means: Vec<_> = centered
                .chunks_mut(samples)
                .map(|lane| {
                    let m = lane.iter().map(|v| *v / samples as f64).sum::<f64>();
                    lane.iter_mut().for_each(|v| *v -= m);
                    m
                })
                .collect();
            let z = hilbert(&output(x, samples, channels, centered, c)?, c)?;
            for (j, v) in z.data().iter().enumerate() {
                upper[j] = means[j / samples] + v.norm();
                lower[j] = means[j / samples] - v.norm();
            }
        }
        EnvelopeKind::Rms => {
            process_chunks(&mut upper, 1, 2 * n, c, |first, result| {
                output_lanes(x, samples, first, result, |lane, begin, result| {
                    for (offset, y) in result.iter_mut().enumerate() {
                        if offset.is_multiple_of(1024) {
                            check_cancel(c)?;
                        }
                        let i = begin + offset;
                        let start = i.saturating_sub(n / 2);
                        let end = (i + (n - 1) / 2 + 1).min(samples);
                        let scale = lane[start..end].iter().fold(0_f64, |m, v| m.max(v.abs()));
                        *y = if scale == 0. {
                            0.
                        } else {
                            scale
                                * (lane[start..end]
                                    .iter()
                                    .map(|v| (v / scale).powi(2))
                                    .sum::<f64>()
                                    / (end - start) as f64)
                                    .sqrt()
                        };
                    }
                    Ok(())
                })
            })?;
            for (j, (lo, up)) in lower.iter_mut().zip(&upper).enumerate() {
                if j.is_multiple_of(4096) {
                    check_cancel(c)?;
                }
                *lo = -*up;
            }
        }
        EnvelopeKind::Peak => {
            for (channel, lane) in x.data().chunks(samples).enumerate() {
                for sign in [1., -1.] {
                    let mut knots = vec![0];
                    for i in 1..samples.saturating_sub(1) {
                        if i.is_multiple_of(1024) {
                            check_cancel(c)?;
                        }
                        if sign * lane[i] > sign * lane[i - 1]
                            && sign * lane[i] >= sign * lane[i + 1]
                        {
                            if knots.len() > 1 && i - knots[knots.len() - 1] < n {
                                if sign * lane[i] > sign * lane[*knots.last().unwrap()] {
                                    *knots.last_mut().unwrap() = i;
                                }
                            } else {
                                knots.push(i);
                            }
                        }
                    }
                    if samples > 1 {
                        knots.push(samples - 1);
                    }
                    let dest = if sign > 0. { &mut upper } else { &mut lower };
                    if samples == 1 {
                        dest[channel * samples] = lane[0];
                    }
                    for pair in knots.windows(2) {
                        let (a, b) = (pair[0], pair[1]);
                        for i in a..=b {
                            dest[channel * samples + i] =
                                lane[a] + (lane[b] - lane[a]) * ((i - a) as f64 / (b - a) as f64);
                        }
                    }
                }
            }
        }
    }
    Ok((
        output(x, samples, channels, upper, c)?,
        output(x, samples, channels, lower, c)?,
    ))
}

pub fn medfilt1(x: &Matrix, n: usize, truncate: bool, c: &AtomicBool) -> Result<Matrix> {
    finite(x)?;
    if n == 0 || n > MAX_TAPS {
        return Err("Median window must be 1..4097".into());
    }
    work(x.len(), n.saturating_mul(n.ilog2() as usize + 1), c)?;
    if x.is_empty() {
        return Ok(x.clone());
    }
    let (samples, channels) = shape(x);
    let mut out = vec![0.; x.len()];
    process_chunks(
        &mut out,
        1,
        n.saturating_mul(n.ilog2() as usize + 1),
        c,
        |first, result| {
            let mut scratch = Vec::with_capacity(n);
            output_lanes(x, samples, first, result, |lane, begin, result| {
                for (offset, y) in result.iter_mut().enumerate() {
                    if offset.is_multiple_of(256) {
                        check_cancel(c)?;
                    }
                    let i = begin + offset;
                    scratch.clear();
                    for j in 0..n {
                        let k = i as isize + j as isize - (n / 2) as isize;
                        if k >= 0 && k < samples as isize {
                            scratch.push(lane[k as usize]);
                        } else if !truncate {
                            scratch.push(0.);
                        }
                    }
                    scratch.sort_unstable_by(f64::total_cmp);
                    let m = scratch.len() / 2;
                    *y = if scratch.len().is_multiple_of(2) {
                        scratch[m - 1] * 0.5 + scratch[m] * 0.5
                    } else {
                        scratch[m]
                    };
                }
                Ok(())
            })
        },
    )?;
    output(x, samples, channels, out, c)
}
/// Polynomial least squares with a scaled Vandermonde and SVD, never normal equations.
/// B maps one frame to fitted values; G(:,k+1) estimates the kth coefficient
/// in powers of the sample offset (multiply by k! for derivatives).
pub fn sgolay(order: usize, frame: usize, c: &AtomicBool) -> Result<(Matrix, Matrix)> {
    check_cancel(c)?;
    if frame == 0 || frame.is_multiple_of(2) || frame > 255 || order >= frame || order > 20 {
        return Err("sgolay requires odd frame <=255 and order < frame, at most 20".into());
    }
    let m = (frame / 2) as f64;
    let scale = m.max(1.);
    let v = DMatrix::from_fn(frame, order + 1, |r, k| {
        ((r as f64 - m) / scale).powi(k as i32)
    });
    let svd = v.clone().svd(true, true);
    let pinv = svd
        .pseudo_inverse(1e-14)
        .map_err(|_| "Savitzky-Golay least-squares solve failed")?;
    check_cancel(c)?;
    let b = &v * &pinv;
    let g = DMatrix::from_fn(frame, order + 1, |r, k| pinv[(k, r)] / scale.powi(k as i32));
    Ok((
        Matrix::from_column_slice(frame, frame, b.as_slice())?,
        Matrix::from_column_slice(frame, order + 1, g.as_slice())?,
    ))
}
pub fn sgolayfilt(x: &Matrix, order: usize, frame: usize, c: &AtomicBool) -> Result<Matrix> {
    finite(x)?;
    let (b, _) = sgolay(order, frame, c)?;
    let (samples, channels) = shape(x);
    if samples < frame {
        return Err("Signal must contain at least one Savitzky-Golay frame".into());
    }
    work(x.len(), frame, c)?;
    let mut out = vec![0.; x.len()];
    process_chunks(&mut out, 1, frame, c, |first, result| {
        output_lanes(x, samples, first, result, |lane, begin, result| {
            for (offset, y) in result.iter_mut().enumerate() {
                if offset.is_multiple_of(1024) {
                    check_cancel(c)?;
                }
                let i = begin + offset;
                let start = i.saturating_sub(frame / 2).min(samples - frame);
                let row = i - start;
                *y = (0..frame).map(|j| b.get(row, j) * lane[start + j]).sum();
            }
            Ok(())
        })
    })?;
    output(x, samples, channels, out, c)
}

/// Kaiser FIR: transition extends away from each passband edge by
/// (1-steepness) times the adjacent available stopband. Zero extension at ends.
pub fn convenience_filter(
    x: &Matrix,
    edges: &[f64],
    fs: Option<f64>,
    kind: Band,
    steepness: f64,
    attenuation: f64,
    c: &AtomicBool,
) -> Result<(Matrix, Vec<f64>)> {
    finite(x)?;
    if !steepness.is_finite()
        || steepness <= 0.
        || steepness >= 1.
        || !attenuation.is_finite()
        || !(21. ..=160.).contains(&attenuation)
    {
        return Err("Steepness must be in (0,1); StopbandAttenuation must be 21..160 dB".into());
    }
    let fs = fs.unwrap_or(2.);
    if !fs.is_finite() || fs <= 0. {
        return Err("Sample rate must be positive and finite".into());
    }
    let pair = matches!(kind, Band::Bandpass | Band::Bandstop);
    if edges.len() != if pair { 2 } else { 1 } {
        return Err("Passband edges must be increasing and strictly inside Nyquist".into());
    }
    let cuts: Vec<_> = edges.iter().map(|f| 2. * f / fs).collect();
    if cuts.iter().any(|f| !f.is_finite() || *f <= 0. || *f >= 1.) || (pair && cuts[0] >= cuts[1]) {
        return Err("Passband edges must be increasing and strictly inside Nyquist".into());
    }
    let width = match kind {
        Band::Lowpass => 1. - cuts[0],
        Band::Highpass => cuts[0],
        Band::Bandpass => cuts[0].min(1. - cuts[1]),
        Band::Bandstop => (cuts[1] - cuts[0]) / 2.,
    } * (1. - steepness);
    let middle = match kind {
        Band::Lowpass => vec![cuts[0] + width / 2.],
        Band::Highpass => vec![cuts[0] - width / 2.],
        Band::Bandpass => vec![cuts[0] - width / 2., cuts[1] + width / 2.],
        Band::Bandstop => vec![cuts[0] + width / 2., cuts[1] - width / 2.],
    };
    let mut order = ((attenuation - 8.) / (2.285 * PI * width)).ceil().max(2.) as usize;
    order += order % 2;
    if order >= MAX_TAPS {
        return Err("Requested transition requires more than 4096 FIR taps".into());
    }
    let window = super::kaiser(order + 1, beta(attenuation), c)?;
    let h = super::fir1_with_options(
        order,
        &middle,
        kind,
        FirOptions {
            window: Some(window.data()),
            scale: true,
        },
        c,
    )?;
    Ok((polyphase(x, &h, 1, 1, c)?, h))
}

/// Frequency-sampling FIR: inverse real DFT on a uniform mesh, Hamming by default.
/// Repeated interior breakpoints are spread by lap mesh intervals (zero: epsilon).
pub fn fir2(
    order: usize,
    f: &[f64],
    gains: &[f64],
    npt: Option<usize>,
    lap: usize,
    window: Option<&[f64]>,
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    check_cancel(c)?;
    if order == 0
        || order >= MAX_TAPS
        || f.len() < 2
        || f.len() > 8192
        || f.len() != gains.len()
        || f[0] != 0.
        || *f.last().unwrap() != 1.
        || f.iter().chain(gains).any(|v| !v.is_finite())
        || f.windows(2).any(|p| p[0] > p[1])
    {
        return Err("fir2 needs order 1..4096, matching finite f/m vectors from 0 to 1".into());
    }
    if order % 2 == 1 && gains[gains.len() - 1] != 0. {
        return Err("Odd-order symmetric FIR must have zero Nyquist gain".into());
    }
    let mesh = npt.unwrap_or((order + 1).next_power_of_two().max(512));
    if mesh <= order || mesh > 65536 || lap > mesh / 2 {
        return Err("fir2 mesh must exceed order, at most 65536; lap at most half the mesh".into());
    }
    work(mesh, (2 * mesh).ilog2() as usize * 16, c)?;
    let mut freq = f.to_vec();
    for i in 1..f.len() {
        if f[i] == f[i - 1] {
            if i == 1 || i + 1 == f.len() || f.get(i + 1) == Some(&f[i]) {
                return Err("Only paired interior frequency repeats are supported".into());
            }
            let delta = if lap == 0 {
                f64::EPSILON
            } else {
                lap as f64 / (2. * mesh as f64)
            };
            freq[i - 1] -= delta;
            freq[i] += delta;
        }
    }
    if freq.windows(2).any(|p| p[0] >= p[1]) {
        return Err("fir2 lap overlaps adjacent breakpoints".into());
    }
    let w = match window {
        Some(w) if w.len() == order + 1 && w.iter().all(|v| v.is_finite()) => w.to_vec(),
        Some(_) => return Err("fir2 window must have order+1 finite elements".into()),
        None => (0..=order)
            .map(|j| 0.54 - 0.46 * (2. * PI * j as f64 / order as f64).cos())
            .collect(),
    };
    let mut spectrum = Vec::with_capacity(mesh + 1);
    let mut band = 0;
    for k in 0..=mesh {
        let x = k as f64 / mesh as f64;
        while band + 2 < freq.len() && x > freq[band + 1] {
            band += 1;
        }
        let t = (x - freq[band]) / (freq[band + 1] - freq[band]);
        spectrum.push(gains[band] + t * (gains[band + 1] - gains[band]));
    }
    let mut full = vec![Complex64::new(0., 0.); 2 * mesh];
    for (k, &gain) in spectrum.iter().enumerate() {
        if k.is_multiple_of(4096) {
            check_cancel(c)?;
        }
        let angle = -PI * k as f64 * order as f64 / (2 * mesh) as f64;
        full[k] = Complex64::new(gain * angle.cos(), gain * angle.sin());
        if k == mesh {
            full[k].im = 0.;
        }
        if k > 0 && k < mesh {
            full[2 * mesh - k] = full[k].conj();
        }
    }
    let input = Array::Complex(ComplexMatrix::from_column_vec(2 * mesh, 1, full, c)?);
    let transformed = super::fourier(
        &input,
        super::FftOptions::default(),
        super::TransformDirection::Inverse,
        256 * 1024 * 1024,
        c,
    )?;
    let h: Vec<_> = match transformed {
        Array::Complex(z) => (0..=order).map(|j| z.data()[j].re * w[j]).collect(),
        Array::Numeric(z) => (0..=order).map(|j| z.data()[j] * w[j]).collect(),
        _ => return Err("fir2 inverse FFT returned nonnumeric data".into()),
    };
    if h.iter().any(|v| !v.is_finite()) {
        return Err("fir2 coefficients overflowed".into());
    }

    Ok(h)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemezKind {
    Symmetric,
    Hilbert,
    Differentiator,
}
#[derive(Clone, Copy)]
struct GridPoint {
    x: f64,
    desired: f64,
    weight: f64,
    edge: bool,
}
// Reduce types II-IV to a cosine polynomial before applying the exchange.
fn remez_factor(f: f64, odd: bool, anti: bool) -> f64 {
    match (odd, anti) {
        (true, false) => 1.,
        (false, false) => (PI * f / 2.).cos(),
        (true, true) => (PI * f).sin(),
        (false, true) => (PI * f / 2.).sin(),
    }
}
fn remez_basis(k: usize, f: f64, odd_taps: bool, anti: bool) -> f64 {
    let shift = if odd_taps {
        if anti { 1. } else { 0. }
    } else {
        0.5
    };
    let theta = PI * f * (k as f64 + shift);
    if anti { theta.sin() } else { theta.cos() }
}
// McClellan-Parks-Rabiner barycentric error evaluation. Mathematical formulas:
// S. Filip, "The Parks McClellan algorithm: a scalable approach for designing
// FIR filters", PEQUAN seminar (2015), slides 16-17 (LIP6 author's slides).
// Common scaling of the weights cancels in both delta and interpolation.
struct RemezInterpolant {
    nodes: Vec<f64>,
    weights: Vec<f64>,
    weight_scale: f64,
    desired: Vec<f64>,
    corrections: Vec<f64>,
}
fn remez_sum(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut correction) = (0., 0.);
    for value in values {
        let next = sum + value;
        correction += if sum.abs() >= value.abs() {
            (sum - next) + value
        } else {
            (value - next) + sum
        };
        sum = next;
    }
    sum + correction
}
impl RemezInterpolant {
    fn new(grid: &[GridPoint], extrema: &[usize]) -> Result<Self> {
        let nodes: Vec<_> = extrema.iter().map(|&j| grid[j].x).collect();
        let mut weights: Vec<_> = nodes
            .iter()
            .enumerate()
            .map(|(k, &x)| {
                1. / nodes
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != k)
                    .map(|(_, &y)| 2. * (x - y))
                    .product::<f64>()
            })
            .collect();
        let scale = weights.iter().map(|v| v.abs()).fold(0., f64::max);
        if !scale.is_finite() || scale == 0. {
            return Err(
                "firpm interpolation nodes are ill-conditioned; change bands or order".into(),
            );
        }
        weights.iter_mut().for_each(|v| *v /= scale);
        // Sum(lambda)=0 for this degree. Subtract a common reference BEFORE
        // summation so a tiny initial ripple isn't lost by subtracting nearly
        // equal passband terms. This matters even for ordinary orders > 160.
        let reference = grid[extrema[0]].desired;
        let numerator = remez_sum(
            extrema
                .iter()
                .zip(&weights)
                .map(|(&j, &w)| w * (grid[j].desired - reference)),
        );
        let denominator = remez_sum(extrema.iter().zip(&weights).enumerate().map(
            |(k, (&j, &w))| {
                if k % 2 == 0 {
                    w / grid[j].weight
                } else {
                    -w / grid[j].weight
                }
            },
        ));
        let delta = numerator / denominator;
        let desired: Vec<_> = extrema.iter().map(|&j| grid[j].desired).collect();
        let corrections: Vec<_> = extrema
            .iter()
            .enumerate()
            .map(|(k, &j)| if k % 2 == 0 { -delta } else { delta } / grid[j].weight)
            .collect();
        if corrections.iter().any(|v| !v.is_finite()) {
            return Err("firpm interpolation produced nonfinite values".into());
        }
        Ok(Self {
            nodes,
            weights,
            weight_scale: scale,
            desired,
            corrections,
        })
    }
    fn evaluate(&self, x: f64) -> f64 {
        self.difference(x, 0.)
    }
    fn difference(&self, x: f64, desired: f64) -> f64 {
        // Interpolate the residual directly. Forming P(x)-D(x) after evaluating
        // P would erase alternation when the ripple is below a passband ulp.
        // Store D_j and its ripple correction separately for the same reason.
        if let Some(j) = self.nodes.iter().position(|&v| v == x) {
            return (self.desired[j] - desired) + self.corrections[j];
        }
        // First barycentric form: the reciprocal denominator is known from
        // the node product. Summing it as lambda/(x-x_j) can cancel almost
        // completely during initialization on disjoint bands, creating false
        // poles and extrema. The factors of two match weight construction.
        let reciprocal_denominator =
            self.nodes.iter().map(|&v| 2. * (x - v)).product::<f64>() * (self.weight_scale / 2.);
        let numerator = remez_sum(
            self.nodes
                .iter()
                .zip(&self.weights)
                .zip(self.desired.iter().zip(&self.corrections))
                .map(|((&v, &w), (&y, &correction))| w / (x - v) * ((y - desired) + correction)),
        );
        numerator * reciprocal_denominator
    }
}

fn remez_extrema(grid: &[GridPoint], errors: &[f64], required: usize) -> Vec<usize> {
    let mut candidates: Vec<usize> = Vec::new();
    for (j, &e) in errors.iter().enumerate() {
        // Always offer BOTH edges of every band, even when its gap is narrower
        // than a grid step. Between edges, seek signed local error extrema.
        if grid[j].edge
            || ((e - errors[j - 1]) * e.signum() >= 0. && (e - errors[j + 1]) * e.signum() >= 0.)
        {
            if let Some(&last) = candidates.last()
                && errors[last].is_sign_positive() == e.is_sign_positive()
            {
                if e.abs() > errors[last].abs() {
                    *candidates.last_mut().unwrap() = j;
                }
            } else {
                candidates.push(j);
            }
        }
    }
    // Discard the weakest extremum, merging its same-sign neighbors by keeping
    // the stronger one. With only one excess, only an endpoint can be removed
    // without losing alternation. This retains the largest alternating errors.
    while candidates.len() > required {
        let last = candidates.len() - 1;
        let k = if candidates.len() == required + 1 {
            if errors[candidates[0]].abs() <= errors[candidates[last]].abs() {
                0
            } else {
                last
            }
        } else {
            (0..candidates.len())
                .min_by(|&a, &b| {
                    errors[candidates[a]]
                        .abs()
                        .total_cmp(&errors[candidates[b]].abs())
                })
                .unwrap()
        };
        candidates.remove(k);
        if k > 0 && k < last {
            let weaker = if errors[candidates[k - 1]].abs() <= errors[candidates[k]].abs() {
                k - 1
            } else {
                k
            };
            candidates.remove(weaker);
        }
    }
    candidates
}

// Reference scaling (Filip 2015, slides 10-12): refine a converged lower-degree
// reference instead of starting a high degree from ill-conditioned uniform
// nodes. Keep every old extremum, splitting the widest available grid gaps.
fn remez_refine(extrema: &[usize], required: usize) -> Result<Vec<usize>> {
    let mut gaps: Vec<_> = extrema.windows(2).filter(|p| p[1] > p[0] + 1).collect();
    gaps.sort_by_key(|p| std::cmp::Reverse(p[1] - p[0]));
    let mut refined = extrema.to_vec();
    refined.extend(
        gaps.iter()
            .take(required - extrema.len())
            .map(|p| (p[0] + p[1]) / 2),
    );
    refined.sort_unstable();
    while refined.len() < required {
        let k = (0..refined.len() - 1)
            .max_by_key(|&k| refined[k + 1] - refined[k])
            .unwrap();
        if refined[k + 1] <= refined[k] + 1 {
            return Err("firpm reference scaling needs more distinct grid points".into());
        }
        refined.insert(k + 1, (refined[k] + refined[k + 1]) / 2);
    }
    Ok(refined)
}

/// Bounded dense-grid Remez exchange (McClellan, Parks & Rabiner, 1973).
/// All four real linear-phase symmetry classes use barycentric interpolation.
/// Failure to converge is an error, never a fallback to a different design.
pub fn firpm(
    order: usize,
    bands: &[f64],
    amplitudes: &[f64],
    weights: Option<&[f64]>,
    kind: RemezKind,
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    check_cancel(c)?;
    if order == 0
        || order > 256
        || bands.len() < 2
        || bands.len() > 64
        || !bands.len().is_multiple_of(2)
        || bands.len() != amplitudes.len()
        || bands.iter().chain(amplitudes).any(|x| !x.is_finite())
        || bands[0] < 0.
        || bands[bands.len() - 1] > 1.
        || bands.windows(2).any(|p| p[0] >= p[1])
    {
        return Err(
            "firpm needs order 1..256 and paired increasing bands/amplitudes in [0,1]".into(),
        );
    }
    let nb = bands.len() / 2;
    let defaults = [1.; 32];
    let w = weights.unwrap_or(&defaults[..nb]);
    if w.len() != nb || w.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err("firpm needs one finite positive weight per band".into());
    }
    let taps = order + 1;
    let odd = taps % 2 == 1;
    let anti = kind != RemezKind::Symmetric;
    let nc = if anti { taps / 2 } else { taps.div_ceil(2) };
    let mut required = if nc > 64 { nc.div_ceil(2) + 1 } else { nc + 1 };
    if (!anti && !odd && bands[bands.len() - 1] == 1. && amplitudes[amplitudes.len() - 1] != 0.)
        || (anti && bands[0] == 0. && amplitudes[0] != 0.)
        || (anti && odd && bands[bands.len() - 1] == 1. && amplitudes[amplitudes.len() - 1] != 0.)
    {
        return Err("Desired endpoint gain conflicts with FIR symmetry".into());
    }
    if odd && !anti && amplitudes.iter().all(|v| *v == amplitudes[0]) {
        let mut h = vec![0.; taps];
        h[taps / 2] = amplitudes[0];
        return Ok(h);
    }
    if amplitudes.iter().all(|v| *v == 0.) {
        return Ok(vec![0.; taps]);
    }
    // 128 grid samples per independent coefficient over Nyquist. Exact band
    // endpoints are included; forced-zero endpoints are excluded from exchange.
    let spacing = 1. / (128 * nc) as f64;
    let mut grid = Vec::new();
    for band in 0..nb {
        let (lo, hi) = (bands[2 * band], bands[2 * band + 1]);
        let steps = ((hi - lo) / spacing).ceil() as usize;
        let start = grid.len();
        for j in 0..=steps {
            // Uniform steps within each band, including its exact upper edge.
            // Avoid rescaling the whole mesh when a width is an almost-integer
            // number of steps (notably symmetric multiband specifications).
            let f = (lo + spacing * j as f64).min(hi);
            if grid
                .last()
                .is_some_and(|p: &GridPoint| p.x == (PI * f).cos())
            {
                continue;
            }
            if (anti && f == 0.) || (f == 1. && ((anti && odd) || (!anti && !odd))) {
                continue;
            }
            let a = amplitudes[2 * band]
                + (amplitudes[2 * band + 1] - amplitudes[2 * band]) * (f - lo) / (hi - lo);
            let factor = remez_factor(f, odd, anti);
            let desired = a / factor;
            // a(f) is the desired amplitude, not a slope multiplier. Relative
            // error weighting applies throughout each nonzero differentiator band.
            let nonzero_band = amplitudes[2 * band] != 0. || amplitudes[2 * band + 1] != 0.;
            let weight = if kind == RemezKind::Differentiator && nonzero_band {
                w[band] * factor / f
            } else {
                w[band] * factor
            };
            if !desired.is_finite() || !weight.is_finite() || weight <= 0. {
                return Err("firpm desired response or error weight overflowed".into());
            }
            grid.push(GridPoint {
                x: (PI * f).cos(),
                desired,
                weight,
                edge: false,
            });
        }
        if grid.len() > start {
            grid[start].edge = true;
            grid.last_mut().unwrap().edge = true;
        }
    }
    if grid.len() < nc + 1 {
        return Err("firpm bands contain too few grid points".into());
    }
    let mut extrema: Vec<_> = (0..required)
        .map(|k| k * (grid.len() - 1) / (required - 1))
        .collect();
    let mut converged = None;
    let mut work_used = 0usize;
    let error_scale = grid
        .iter()
        .map(|p| (p.desired * p.weight).abs())
        .fold(0., f64::max);
    if !error_scale.is_finite() {
        return Err("firpm weighted desired response overflowed".into());
    }
    for _ in 0..100 {
        check_cancel(c)?;
        work_used = work_used.saturating_add(grid.len() * required + required * required);
        if work_used > MAX_REMEZ_WORK {
            return Err("firpm exchange exceeds bounded work limit; reduce order".into());
        }
        let interpolant = RemezInterpolant::new(&grid, &extrema)?;
        let mut errors = Vec::with_capacity(grid.len());
        for (j, p) in grid.iter().enumerate() {
            if j.is_multiple_of(1024) {
                check_cancel(c)?;
            }
            errors.push(interpolant.difference(p.x, p.desired) * p.weight);
        }
        if errors.iter().any(|v| !v.is_finite()) {
            return Err("firpm exchange produced nonfinite error".into());
        }
        let candidates = remez_extrema(&grid, &errors, required);
        if candidates.len() != required {
            return Err("firpm could not find enough alternating extrema".into());
        }
        let min = candidates
            .iter()
            .map(|&j| errors[j].abs())
            .fold(f64::INFINITY, f64::min);
        let max = errors.iter().map(|v| v.abs()).fold(0_f64, f64::max);
        // At high orders the ripple approaches rounding noise: an absolute
        // floor scaled by the weighted desired response avoids endless swaps.
        if candidates == extrema || max - min <= 1e-7 * max + 2e-14 * error_scale {
            if required != nc + 1 {
                required = nc + 1;
                extrema = remez_refine(&candidates, required)?;
                continue;
            }
            converged = Some(interpolant);
            break;
        }
        extrema = candidates;
    }
    let interpolant = converged.ok_or("firpm Remez exchange did not converge in 100 iterations")?;
    // Recover the original sine/cosine coefficients by orthogonal midpoint
    // quadrature (exact for these trigonometric polynomials), never a solve.
    let response: Vec<_> = (0..taps)
        .map(|j| {
            let f = (j as f64 + 0.5) / taps as f64;
            interpolant.evaluate((PI * f).cos()) * remez_factor(f, odd, anti)
        })
        .collect();
    let coefficients: Vec<_> =
        (0..nc)
            .map(|k| {
                let sum =
                    remez_sum(response.iter().enumerate().map(|(j, &y)| {
                        y * remez_basis(k, (j as f64 + 0.5) / taps as f64, odd, anti)
                    }));
                sum * if odd && !anti && k == 0 { 1. } else { 2. } / taps as f64
            })
            .collect();
    let mut h = vec![0.; taps];
    for (k, &coefficient) in coefficients.iter().enumerate() {
        if odd && !anti && k == 0 {
            h[taps / 2] = coefficient;
            continue;
        }
        let left = if odd {
            taps / 2 - k - usize::from(anti)
        } else {
            taps / 2 - k - 1
        };
        let right = taps - 1 - left;
        h[left] = coefficient / 2.;
        h[right] = if anti {
            -coefficient / 2.
        } else {
            coefficient / 2.
        };
    }
    check_cancel(c)?;
    if h.iter().any(|v| !v.is_finite()) {
        return Err("firpm coefficient reconstruction overflowed".into());
    }
    Ok(h)
}
