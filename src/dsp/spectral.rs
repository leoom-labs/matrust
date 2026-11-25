//! Windowed FFT analysis, independent of the interpreter and desktop.
//! Welch (1967), The use of FFT for the estimation of power spectra, pp. 70-73;
//! Harris (1978), On the use of windows for harmonic analysis, section II;
//! Allen (1977), Short term spectral analysis, synthesis, and modification by DFT.
//! No detrending. Cross spectra use conj(X)*Y; H1 is Pxy/Pxx.
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use rayon::prelude::*;
use rustfft::{Fft, FftPlanner};

// Benchmarked by examples/benchmark_spectra.rs. Only independent columns
// parallelize, using the existing pool with at most eight jobs.
const PAR_WORK: usize = 1_000_000;
const MAX_JOBS: usize = 8;
use std::{
    f64::consts::TAU,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    OneSided,
    TwoSided,
    Centered,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Estimate {
    Periodogram,
    Welch,
    Cross,
    Coherence,
    Transfer,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub window: Vec<f64>,
    pub overlap: usize,
    pub nfft: usize,
    /// None gives radians/sample and density per radian/sample.
    pub fs: Option<f64>,
    pub range: Range,
    pub power: bool,
}
impl Options {
    fn validate(&self) -> Result<(f64, f64)> {
        let l = self.window.len();
        if l == 0 || l > super::MAX_FFT_LENGTH || self.window.iter().any(|x| !x.is_finite()) {
            return Err(
                "Spectral window must be a finite nonempty vector of at most 1048576 samples"
                    .into(),
            );
        }
        if self.overlap >= l {
            return Err("Spectral noverlap must be less than window length".into());
        }
        if self.nfft < l || self.nfft > super::MAX_FFT_LENGTH {
            return Err(
                "Spectral FFT length must be at least window length and at most 1048576".into(),
            );
        }
        let fs = self.fs.unwrap_or(TAU);
        if !fs.is_finite() || fs <= 0. {
            return Err("Spectral sample rate must be finite and positive".into());
        }
        let energy: f64 = self.window.iter().map(|v| v * v).sum();
        let sum: f64 = self.window.iter().sum();
        let denominator = if self.power { sum * sum } else { energy * fs };
        if !energy.is_finite() || energy <= 0. || !denominator.is_finite() || denominator <= 0. {
            return Err("Spectral window has zero or unrepresentable normalization".into());
        }
        Ok((fs, denominator))
    }
    pub fn bins(&self) -> usize {
        if self.range == Range::OneSided {
            self.nfft / 2 + 1
        } else {
            self.nfft
        }
    }
    fn index(&self, bin: usize) -> usize {
        if self.range == Range::Centered {
            (bin + self.nfft / 2 + 1) % self.nfft
        } else {
            bin
        }
    }
    fn doubling(&self, bin: usize) -> f64 {
        if self.range == Range::OneSided
            && bin > 0
            && !(self.nfft.is_multiple_of(2) && bin == self.nfft / 2)
        {
            2.
        } else {
            1.
        }
    }
    fn frequencies(&self, fs: f64) -> Result<Matrix> {
        Matrix::from_column_vec(
            self.bins(),
            1,
            (0..self.bins())
                .map(|k| {
                    let i = if self.range == Range::Centered {
                        k as f64 - ((self.nfft - 1) / 2) as f64
                    } else {
                        k as f64
                    };
                    (i / self.nfft as f64) * fs
                })
                .collect(),
        )
    }
}

fn shape(x: &Array) -> Result<(usize, usize)> {
    x.require_numeric_array()?;
    if x.is_empty() {
        return Err("Spectral input must be nonempty".into());
    }
    Ok(if x.rows() == 1 {
        (x.cols(), 1)
    } else {
        (x.rows(), x.cols())
    })
}
fn sample(x: &Array, i: usize) -> Complex64 {
    match x {
        Array::Numeric(a) => Complex64::new(a.data()[i], 0.),
        Array::Complex(a) => a.data()[i],
        Array::Logical(a) => Complex64::new(f64::from(a.data()[i]), 0.),
        _ => unreachable!("validated numeric input"),
    }
}
fn admit_input(x: &Array, range: Range, cancel: &AtomicBool) -> Result<bool> {
    let mut real = true;
    for i in 0..x.len() {
        if i.is_multiple_of(4096) {
            super::check_cancel(cancel)?;
        }
        let z = sample(x, i);
        if !z.re.is_finite() || !z.im.is_finite() {
            return Err("Spectral input must be finite".into());
        }
        real &= z.im == 0.;
        if range == Range::OneSided && z.im != 0. {
            return Err("One-sided spectra require real input".into());
        }
    }
    Ok(real)
}

/// Conservative temporary/output/planner admission, before planning or allocation.
fn budget(
    o: &Options,
    output: usize,
    transforms: usize,
    jobs: usize,
    available: usize,
    fft_bytes: usize,
) -> Result<()> {
    let work = transforms
        .checked_mul(o.nfft)
        .and_then(|n| n.checked_mul(o.nfft.ilog2().max(1) as usize));
    if work.is_none_or(|n| n > 256_000_000) {
        return Err("Spectral operation exceeds work limit".into());
    }
    let planning = o
        .nfft
        .checked_mul(if o.nfft.is_power_of_two() { 128 } else { 512 });
    let bytes = planning.and_then(|p| p.checked_add(output)).and_then(|p| {
        p.checked_add((o.nfft * 128).max(fft_bytes) * jobs + o.window.len() * 8 + 1024 * 1024)
    });
    if bytes.is_none_or(|n| n > available) {
        return Err("Spectral operation exceeds remaining evaluation budget".into());
    }
    if bytes.is_none_or(|n| n > 256 * 1024 * 1024) {
        return Err("Spectral operation exceeds temporary memory limit".into());
    }
    Ok(())
}
fn admitted_jobs(
    o: &Options,
    output: usize,
    transforms: usize,
    lanes: usize,
    available: usize,
    fft_bytes: usize,
) -> Result<usize> {
    let work = transforms
        .saturating_mul(o.nfft)
        .saturating_mul(o.nfft.ilog2().max(1) as usize);
    let mut jobs = if work >= PAR_WORK {
        lanes.min(MAX_JOBS).min(rayon::current_num_threads())
    } else {
        1
    };
    // Reduce concurrency to fit the same statement budget rather than rejecting
    // a supported operation just because more workers are available.
    while jobs > 1 && budget(o, output, transforms, jobs, available, fft_bytes).is_err() {
        jobs -= 1;
    }
    budget(o, output, transforms, jobs, available, fft_bytes)?;
    Ok(jobs)
}

struct SegmentFft {
    fft: Arc<dyn Fft<f64>>,
    buffer: Vec<Complex64>,
    scratch: Vec<Complex64>,
    real_input: bool,
}
fn spectral_plan(n: usize, inverse: bool, cancel: &AtomicBool) -> Result<Arc<dyn Fft<f64>>> {
    super::check_cancel(cancel)?;
    let mut planner = FftPlanner::new();
    let fft = if inverse {
        planner.plan_fft_inverse(n)
    } else {
        planner.plan_fft_forward(n)
    };
    super::check_cancel(cancel)?;
    Ok(fft)
}
impl SegmentFft {
    fn with_plan(fft: Arc<dyn Fft<f64>>) -> Self {
        Self {
            real_input: false,
            scratch: vec![Complex64::default(); fft.get_inplace_scratch_len()],
            buffer: vec![Complex64::default(); fft.len()],
            fft,
        }
    }
    /// The common framing/windowing/FFT path: exactly one FFT per input segment.
    fn forward(
        &mut self,
        x: &Array,
        column: usize,
        rows: usize,
        start: isize,
        o: &Options,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.buffer.fill(Complex64::default());
        for (j, &w) in o.window.iter().enumerate() {
            if j.is_multiple_of(4096) {
                super::check_cancel(cancel)?;
            }
            let i = start + j as isize;
            if i >= 0 && (i as usize) < rows {
                self.buffer[j] = sample(x, column * rows + i as usize) * w;
            }
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        if self.real_input {
            // Enforce the exact Hermitian identity for real data. This permits
            // exact inverse type admission without discarding small complex
            // information based on a tolerance.
            self.buffer[0].im = 0.;
            if self.buffer.len().is_multiple_of(2) {
                let nyquist = self.buffer.len() / 2;
                self.buffer[nyquist].im = 0.;
            }
            for k in 1..self.buffer.len().div_ceil(2) {
                if k.is_multiple_of(4096) {
                    super::check_cancel(cancel)?;
                }
                let mirror = self.buffer.len() - k;
                self.buffer[mirror] = self.buffer[k].conj();
            }
        }
        super::check_cancel(cancel)
    }
}

/// Averaged modified periodograms. Columns are independent channels; tails that
/// do not fill a segment are discarded. Undefined coherence/H1 bins are NaN.
pub fn estimate(
    x: &Array,
    y: Option<&Array>,
    kind: Estimate,
    o: &Options,
    available: usize,
    cancel: &AtomicBool,
) -> Result<(Array, Matrix)> {
    let (rows, cols) = shape(x)?;
    let (fs, norm) = o.validate()?;
    if rows < o.window.len() {
        return Err("Spectral window longer than x".into());
    }
    if kind == Estimate::Periodogram && rows != o.window.len() {
        return Err("periodogram window length must equal x length".into());
    }
    let cross = matches!(
        kind,
        Estimate::Cross | Estimate::Coherence | Estimate::Transfer
    );
    if cross && y.is_none() {
        return Err("Cross spectral estimates require y".into());
    }
    if let Some(y) = y
        && shape(y)? != (rows, cols)
    {
        return Err("Cross spectral inputs must have matching channel shapes".into());
    }
    let segments = if kind == Estimate::Periodogram {
        1
    } else {
        1 + (rows - o.window.len()) / (o.window.len() - o.overlap)
    };
    let count = Matrix::checked_len(o.bins(), cols)?;
    let transforms = segments
        .saturating_mul(cols)
        .saturating_mul(if cross { 2 } else { 1 });
    admitted_jobs(o, count * 48 + o.bins() * 8, transforms, cols, available, 0)?;
    let real_x = admit_input(x, o.range, cancel)?;
    let real_y = y
        .map(|y| admit_input(y, o.range, cancel))
        .transpose()?
        .unwrap_or(false);
    let plan = spectral_plan(o.nfft, false, cancel)?;
    // Admit actual RustFFT scratch for both inputs and all concurrent jobs.
    let fft_bytes =
        (plan.get_inplace_scratch_len() + o.nfft).saturating_mul(if cross { 32 } else { 16 });
    let jobs = admitted_jobs(
        o,
        count * 48 + o.bins() * 8,
        transforms,
        cols,
        available,
        fft_bytes,
    )?;
    let mut output = vec![Complex64::default(); count];
    let run = |job: usize, output: &mut [Complex64]| -> Result<()> {
        super::check_cancel(cancel)?;
        let mut fft = SegmentFft::with_plan(plan.clone());
        fft.real_input = real_x;
        let mut other = if cross {
            let mut other = SegmentFft::with_plan(plan.clone());
            other.real_input = real_y;
            Some(other)
        } else {
            None
        };
        for (local, output) in output.chunks_mut(o.bins()).enumerate() {
            let column = job * cols.div_ceil(jobs) + local;
            let mut xx = vec![0.; o.bins()];
            let mut yy = vec![0.; o.bins()];
            let mut xy = vec![Complex64::default(); o.bins()];
            for segment in 0..segments {
                let start = (segment * (o.window.len() - o.overlap)) as isize;
                fft.forward(x, column, rows, start, o, cancel)?;
                if let (Some(other), Some(y)) = (&mut other, y) {
                    other.forward(y, column, rows, start, o, cancel)?;
                }
                for k in 0..o.bins() {
                    if k.is_multiple_of(4096) {
                        super::check_cancel(cancel)?;
                    }
                    let i = o.index(k);
                    let a = fft.buffer[i];
                    xx[k] += a.norm_sqr();
                    if let Some(other) = &other {
                        let b = other.buffer[i];
                        yy[k] += b.norm_sqr();
                        xy[k] += a.conj() * b;
                    }
                }
            }
            for k in 0..o.bins() {
                if k.is_multiple_of(4096) {
                    super::check_cancel(cancel)?;
                }
                if !xx[k].is_finite()
                    || !yy[k].is_finite()
                    || !xy[k].re.is_finite()
                    || !xy[k].im.is_finite()
                {
                    return Err("Spectral accumulation overflowed; scale the input signal".into());
                }
                let scale = o.doubling(k) / (norm * segments as f64);
                output[k] = match kind {
                    Estimate::Periodogram | Estimate::Welch => Complex64::new(xx[k] * scale, 0.),
                    Estimate::Cross => xy[k] * scale,
                    Estimate::Coherence => Complex64::new(
                        if xx[k] == 0. || yy[k] == 0. {
                            f64::NAN
                        } else {
                            ((xy[k].norm() / xx[k].sqrt()) / yy[k].sqrt())
                                .powi(2)
                                .min(1.)
                        },
                        0.,
                    ),
                    Estimate::Transfer => xy[k] / xx[k],
                };
            }
        }
        Ok(())
    };
    let chunk = cols.div_ceil(jobs) * o.bins();
    if jobs > 1 {
        output
            .par_chunks_mut(chunk)
            .enumerate()
            .try_for_each(|(job, output)| run(job, output))?;
    } else {
        run(0, &mut output)?;
    }
    let result = if matches!(kind, Estimate::Cross | Estimate::Transfer) {
        ComplexMatrix::from_column_vec(o.bins(), cols, output, cancel)?.into()
    } else {
        Matrix::from_column_vec(o.bins(), cols, output.into_iter().map(|z| z.re).collect())?.into()
    };
    super::check_cancel(cancel)?;
    Ok((result, o.frequencies(fs)?))
}

pub struct ShortTime {
    pub spectrum: ComplexMatrix,
    pub frequencies: Matrix,
    pub times: Matrix,
    pub power: Option<Matrix>,
}
/// Raw windowed DFTs. Spectrogram uses complete frames; STFT pads L/2 zeros
/// at each boundary and pads the tail to a whole hop (SciPy boundary='zeros').
pub fn short_time(
    x: &Array,
    o: &Options,
    padded: bool,
    power: bool,
    available: usize,
    cancel: &AtomicBool,
) -> Result<ShortTime> {
    let (rows, cols) = shape(x)?;
    let (fs, norm) = o.validate()?;
    if cols != 1 {
        return Err("STFT/spectrogram currently require a vector".into());
    }
    if rows < o.window.len() {
        return Err("Spectral window longer than x".into());
    }
    let hop = o.window.len() - o.overlap;
    let pad = if padded { o.window.len() / 2 } else { 0 };
    let frames = if padded {
        1 + (rows + 2 * pad - o.window.len()).div_ceil(hop)
    } else {
        1 + (rows - o.window.len()) / hop
    };
    let last_time = ((frames - 1) as f64 * hop as f64
        + if padded {
            0.
        } else {
            o.window.len() as f64 / 2.
        })
        / o.fs.unwrap_or(1.);
    if !last_time.is_finite() {
        return Err("STFT time coordinates overflow at this sample rate".into());
    }
    let count = ComplexMatrix::checked_len(o.bins(), frames)?;
    let output_bytes = count * (if power { 24 } else { 16 }) + (frames + o.bins()) * 8;
    admitted_jobs(o, output_bytes, frames, frames, available, 0)?;
    let real = admit_input(x, o.range, cancel)?;
    let plan = spectral_plan(o.nfft, false, cancel)?;
    let fft_bytes = (plan.get_inplace_scratch_len() + o.nfft).saturating_mul(16);
    let jobs = admitted_jobs(o, output_bytes, frames, frames, available, fft_bytes)?;
    let mut spectrum = vec![Complex64::default(); count];
    let frames_per_job = frames.div_ceil(jobs);
    let run = |job: usize, output: &mut [Complex64]| -> Result<()> {
        super::check_cancel(cancel)?;
        let mut fft = SegmentFft::with_plan(plan.clone());
        fft.real_input = real;
        for (local, output) in output.chunks_mut(o.bins()).enumerate() {
            let frame = job * frames_per_job + local;
            fft.forward(x, 0, rows, (frame * hop) as isize - pad as isize, o, cancel)?;
            for (k, target) in output.iter_mut().enumerate() {
                if k.is_multiple_of(4096) {
                    super::check_cancel(cancel)?;
                }
                let z = fft.buffer[o.index(k)];
                if !z.re.is_finite() || !z.im.is_finite() {
                    return Err("STFT overflowed; scale the input signal or window".into());
                }
                *target = z;
            }
        }
        Ok(())
    };
    if jobs > 1 {
        spectrum
            .par_chunks_mut(frames_per_job * o.bins())
            .enumerate()
            .try_for_each(|(job, output)| run(job, output))?;
    } else {
        run(0, &mut spectrum)?;
    }
    // Derive all requested powers from the retained transform; no second FFT.
    let ps = if power {
        let mut ps = Vec::with_capacity(count);
        for (i, z) in spectrum.iter().enumerate() {
            if i.is_multiple_of(4096) {
                super::check_cancel(cancel)?;
            }
            let p = z.norm_sqr() * o.doubling(i % o.bins()) / norm;
            if !p.is_finite() {
                return Err("Spectrogram power overflowed; scale the input signal".into());
            }
            ps.push(p);
        }
        Some(ps)
    } else {
        None
    };
    Ok(ShortTime {
        spectrum: ComplexMatrix::from_column_vec(o.bins(), frames, spectrum, cancel)?,
        frequencies: o.frequencies(fs)?,
        times: Matrix::from_column_vec(
            1,
            frames,
            (0..frames)
                .map(|j| {
                    (j as f64 * hop as f64
                        + if padded {
                            0.
                        } else {
                            o.window.len() as f64 / 2.
                        })
                        / o.fs.unwrap_or(1.)
                })
                .collect(),
        )?,
        power: ps
            .map(|v| Matrix::from_column_vec(o.bins(), frames, v))
            .transpose()?,
    })
}

/// Weighted overlap-add: inverse DFT * synthesis window / sum(window^2).
/// Allen (1977), synthesis by overlap-add; weighted NOLA normalization as
/// described in SciPy's STFT notes. Reject uncovered output samples.
pub fn inverse(
    s: &ComplexMatrix,
    o: &Options,
    output_length: Option<usize>,
    available: usize,
    cancel: &AtomicBool,
) -> Result<(Array, Matrix)> {
    o.validate()?;
    if s.rows() != o.bins() || s.cols() == 0 {
        return Err("ISTFT spectrum dimensions do not match FFTLength/FrequencyRange".into());
    }
    let hop = o.window.len() - o.overlap;
    let full = (s.cols() - 1)
        .checked_mul(hop)
        .and_then(|n| n.checked_add(o.window.len()))
        .ok_or("ISTFT size overflow")?;
    let pad = o.window.len() / 2;
    let length = full - 2 * pad;
    let length = output_length.unwrap_or(length);
    if length == 0 || length > full - 2 * pad {
        return Err("ISTFT OutputLength exceeds reconstructed signal length".into());
    }
    Matrix::checked_len(length, 1)?;
    if !((length - 1) as f64 / o.fs.unwrap_or(1.)).is_finite() {
        return Err("ISTFT time coordinates overflow at this sample rate".into());
    }

    budget(
        o,
        full.saturating_mul(48) + length * 24,
        s.cols(),
        1,
        available,
        0,
    )?;
    let plan = spectral_plan(o.nfft, true, cancel)?;
    budget(
        o,
        full.saturating_mul(48) + length * 24,
        s.cols(),
        1,
        available,
        (plan.get_inplace_scratch_len() + o.nfft).saturating_mul(16),
    )?;
    let mut fft = SegmentFft::with_plan(plan);
    let mut values = vec![Complex64::default(); full];
    let mut weights = vec![0.; full];
    let mut real = true;
    for frame in 0..s.cols() {
        super::check_cancel(cancel)?;
        fft.buffer.fill(Complex64::default());
        for k in 0..o.bins() {
            if k.is_multiple_of(4096) {
                super::check_cancel(cancel)?;
            }
            let z = s.data()[frame * o.bins() + k];
            if !z.re.is_finite() || !z.im.is_finite() {
                return Err("ISTFT spectrum must be finite".into());
            }
            fft.buffer[o.index(k)] = z;
        }
        if o.range == Range::OneSided {
            if fft.buffer[0].im != 0.
                || (o.nfft.is_multiple_of(2) && fft.buffer[o.nfft / 2].im != 0.)
            {
                return Err("One-sided ISTFT DC/Nyquist bins must be real".into());
            }
            for k in 1..o.nfft.div_ceil(2) {
                fft.buffer[o.nfft - k] = fft.buffer[k].conj();
            }
        }
        // A full complex spectrum must be exactly Hermitian to produce a real
        // result. Never remove genuine small imaginary signals by tolerance.
        real &= fft.buffer[0].im == 0.;
        if o.nfft.is_multiple_of(2) {
            real &= fft.buffer[o.nfft / 2].im == 0.;
        }
        for k in 1..o.nfft.div_ceil(2) {
            if k.is_multiple_of(4096) {
                super::check_cancel(cancel)?;
            }
            real &= fft.buffer[k] == fft.buffer[o.nfft - k].conj();
        }
        fft.fft
            .process_with_scratch(&mut fft.buffer, &mut fft.scratch);
        for (j, &w) in o.window.iter().enumerate() {
            let index = frame * hop + j;
            values[index] += fft.buffer[j] * (w / o.nfft as f64);
            weights[index] += w * w;
        }
    }
    let mut output = Vec::with_capacity(length);
    for i in pad..pad + length {
        if i.is_multiple_of(4096) {
            super::check_cancel(cancel)?;
        }
        if weights[i] <= 0. || !weights[i].is_finite() {
            return Err("ISTFT window/overlap leaves uncovered samples (NOLA failure)".into());
        }
        let z = values[i] / weights[i];
        if !z.re.is_finite() || !z.im.is_finite() {
            return Err("ISTFT reconstruction overflowed; scale the spectrum or window".into());
        }
        output.push(z);
    }
    let result = if real {
        Matrix::from_column_vec(length, 1, output.into_iter().map(|z| z.re).collect())?.into()
    } else {
        ComplexMatrix::from_column_vec(length, 1, output, cancel)?.into()
    };
    Ok((
        result,
        Matrix::from_column_vec(
            length,
            1,
            (0..length).map(|j| j as f64 / o.fs.unwrap_or(1.)).collect(),
        )?,
    ))
}
