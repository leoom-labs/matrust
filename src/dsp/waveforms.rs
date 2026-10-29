//! Real f64 signal generation with bounded shared-pool parallel execution.
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::{
    f64::consts::{PI, TAU},
    sync::atomic::AtomicBool,
};

// Phase reduction follows f64 TAU modulus; large phases lose accuracy against exact 2*pi.
const CHUNK: usize = 4096;
// One/eight-worker release measurements support this threshold on the tested host.
// See research/signal-waveforms/benchmark-results.json; smaller inputs stay serial.
const PARALLEL_ELEMENTS: usize = 65_536;

fn map(
    input: &Matrix,
    cancel: &AtomicBool,
    sample: impl Fn(f64) -> Result<f64> + Sync,
) -> Result<Matrix> {
    super::check_cancel(cancel)?;
    if input.len() > crate::matrix::MAX_ELEMENTS {
        return Err("Signal array exceeds matrix element limit".into());
    }
    let mut output = vec![0.; input.len()];
    let fill = |(index, chunk): (usize, &mut [f64])| -> Result<()> {
        super::check_cancel(cancel)?;
        let start = index * CHUNK;
        let end = start + chunk.len();
        for (value, &source) in chunk.iter_mut().zip(&input.data()[start..end]) {
            *value = sample(source)?;
        }
        Ok(())
    };
    if input.len() >= PARALLEL_ELEMENTS && rayon::current_num_threads() > 1 {
        output
            .par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(fill)?;
    } else {
        for chunk in output.chunks_mut(CHUNK).enumerate() {
            fill(chunk)?;
        }
    }
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(input.rows(), input.cols(), output)
}

/// Normalized sin(pi*x)/(pi*x). +/-0 -> 1; finite nonzero integers -> +0;
/// NaN and +/-Inf -> NaN. Output shape equals input shape.
pub fn sinc(input: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    map(input, cancel, |x| Ok(sinc_sample(x)))
}

fn sinc_sample(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let x = x.abs();
    if x < 1e-4 {
        // Also handles subnormal x: multiplying by pi and dividing back would lose precision.
        let z = PI * x;
        let z2 = z * z;
        return 1. + z2 * (-1. / 6. + z2 / 120.);
    }
    let whole = x.floor();
    let fraction = x - whole;
    if fraction == 0. {
        return 0.;
    }
    // Every noninteger f64 has magnitude < 2^52, so whole's parity is representable.
    let sine = libm::sin(PI * fraction.min(1. - fraction));
    let signed = if whole % 2. == 0. { sine } else { -sine };
    // Never form pi*x, which can overflow for otherwise finite input.
    signed / x / PI
}

fn phase(x: f64) -> Result<f64> {
    if !x.is_finite() {
        return Err("square/sawtooth require finite phases".into());
    }
    let remainder = x % TAU;
    // A tiny negative remainder can round to TAU on addition. Keep that left-limit
    // endpoint; changing it to zero would move a sample across a discontinuity.
    Ok(if remainder < 0. {
        remainder + TAU
    } else {
        remainder
    })
}

/// Square wave with scalar duty in percent, 0..=100; usual default is 50.
/// The positive half-open part starts at each multiple of representable TAU.
pub fn square(input: &Matrix, duty: f64, cancel: &AtomicBool) -> Result<Matrix> {
    if !duty.is_finite() || !(0. ..=100.).contains(&duty) {
        return Err("square duty must be a finite scalar between 0 and 100 percent".into());
    }
    let threshold = TAU * duty / 100.;
    map(input, cancel, |x| {
        let p = phase(x)?;
        let positive = if duty == 100. {
            true
        } else if duty == 0. {
            false
        } else if threshold < f64::MIN_POSITIVE {
            // Avoid rounding a tiny positive duty to an empty interval.
            p / duty < TAU / 100.
        } else {
            p < threshold
        };
        Ok(if positive { 1. } else { -1. })
    })
}

/// Sawtooth/triangle wave with scalar width in 0..=1; usual default is 1.
/// Width 0 is descending, width 1 ascending, width 0.5 triangular.
pub fn sawtooth(input: &Matrix, width: f64, cancel: &AtomicBool) -> Result<Matrix> {
    if !width.is_finite() || !(0. ..=1.).contains(&width) {
        return Err("sawtooth width must be a finite scalar between 0 and 1".into());
    }
    let peak = TAU * width;
    map(input, cancel, |x| {
        let p = phase(x)?;
        let value = if width == 0. {
            1. - 2. * (p / TAU)
        } else if width == 1. {
            2. * (p / TAU) - 1.
        } else if peak < f64::MIN_POSITIVE {
            let scaled = p / width;
            if scaled < TAU {
                2. * (scaled / TAU) - 1.
            } else {
                2. * ((TAU - p) / (1. - width) / TAU) - 1.
            }
        } else if p <= peak {
            2. * (p / peak) - 1.
        }
        // Use the same rounded breakpoint in both ramps to keep narrow tails bounded.
        else {
            2. * ((TAU - p) / (TAU - peak)) - 1.
        };
        Ok(value)
    })
}
