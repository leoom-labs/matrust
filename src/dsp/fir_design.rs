//! Bounded real FIR design, including explicit windows and optional normalization.
//! Analytic ideal-band integration followed by a supplied or default Hamming window.
use super::{Band, MAX_ORDER, check_cancel};
use crate::Result;
use std::{f64::consts::PI, sync::atomic::AtomicBool};

// Preserve the rounding residue of a midpoint/product until phase reduction.
// This keeps exact cosine zeros exact and resolves which side of a zero a
// nearby representable cutoff lies on before multiplying by pi.
fn sum_with_error(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let virtual_b = sum - a;
    (sum, (a - (sum - virtual_b)) + (b - virtual_b))
}
fn midpoint(a: f64, b: f64) -> (f64, f64) {
    let (sum, error) = sum_with_error(a, b);
    (sum / 2., error / 2.)
}
fn phase_cos(x: f64, error: f64) -> f64 {
    let integer = x.round();
    let fraction = x - integer;
    let sign = if (integer as i64) % 2 == 0 { 1. } else { -1. };
    if fraction.abs() <= 0.25 {
        sign * (PI * (fraction + error)).cos()
    } else {
        let distance = (0.5 - fraction.abs()) - if fraction > 0. { error } else { -error };
        sign * (PI * distance).sin()
    }
}
fn phase_sin(x: f64, error: f64) -> f64 {
    let integer = x.round();
    let fraction = x - integer;
    let sign = if (integer as i64) % 2 == 0 { 1. } else { -1. };
    if fraction.abs() <= 0.25 {
        sign * (PI * (fraction + error)).sin()
    } else {
        let direction = if fraction > 0. { 1. } else { -1. };
        let distance = (0.5 - fraction.abs()) - direction * error;
        sign * direction * (PI * distance).cos()
    }
}
fn phase_sin_cos(x: f64, error: f64) -> (f64, f64) {
    let integer = x.round();
    let fraction = x - integer;
    let sign = if (integer as i64) % 2 == 0 { 1. } else { -1. };
    if fraction.abs() <= 0.25 {
        let (s, c) = (PI * (fraction + error)).sin_cos();
        (sign * s, sign * c)
    } else {
        let direction = if fraction > 0. { 1. } else { -1. };
        let distance = (0.5 - fraction.abs()) - direction * error;
        let (s, c) = (PI * distance).sin_cos();
        (sign * direction * c, sign * s)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FirOptions<'a> {
    /// None or an empty vector selects the default Hamming window.
    pub window: Option<&'a [f64]>,
    /// Normalize the passband reference gain; false preserves window amplitude.
    pub scale: bool,
}
impl Default for FirOptions<'_> {
    fn default() -> Self {
        Self {
            window: None,
            scale: true,
        }
    }
}

/// Backward-compatible default Hamming design API used by existing desktop callers.
pub fn fir1(order: usize, cutoffs: &[f64], band: Band) -> Result<Vec<f64>> {
    fir1_with_options(
        order,
        cutoffs,
        band,
        FirOptions::default(),
        &AtomicBool::new(false),
    )
}

/// Real low/high/bandpass/stop design with an optional real window.
/// Odd high/stop orders are promoted before the window length is checked.
/// This bounded design stays serial; at most 4097 coefficients are generated.
pub fn fir1_with_options(
    mut order: usize,
    cutoffs: &[f64],
    band: Band,
    options: FirOptions<'_>,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    check_cancel(cancel)?;
    if order == 0 || order > MAX_ORDER {
        return Err(format!("FIR order must be 1..{MAX_ORDER}"));
    }
    let count = if matches!(band, Band::Bandpass | Band::Bandstop) {
        2
    } else {
        1
    };
    if cutoffs.len() != count
        || cutoffs
            .iter()
            .any(|f| !f.is_finite() || *f <= 0. || *f >= 1.)
        || (count == 2 && cutoffs[0] >= cutoffs[1])
    {
        return Err("Cutoffs must be increasing, strictly between 0 and 1 (Nyquist)".into());
    }
    if matches!(band, Band::Highpass | Band::Bandstop) && !order.is_multiple_of(2) {
        order += 1;
    }
    if order > MAX_ORDER {
        return Err(format!("Promoted FIR order exceeds {MAX_ORDER}"));
    }
    let window = options.window.filter(|w| !w.is_empty());
    if let Some(w) = window {
        if w.len() != order + 1 {
            return Err(format!(
                "Window must contain {} elements for the effective FIR order {order}",
                order + 1
            ));
        }
        if w.iter().any(|v| !v.is_finite()) {
            return Err("FIR window must contain only finite real values".into());
        }
    }
    // Rescale explicit windows before normalized design, preventing overflow or
    // underflow from arbitrary common window amplitudes. Noscale preserves them.
    let window_scale = if options.scale {
        window.map_or(1., |w| w.iter().map(|v| v.abs()).fold(0., f64::max))
    } else {
        1.
    };
    if window_scale == 0. {
        return Err("Cannot normalize an all-zero FIR window".into());
    }
    let symmetric = window.is_none_or(|w| w.iter().zip(w.iter().rev()).all(|(a, b)| a == b));
    let middle = order as f64 / 2.;
    let sinc = |frequency: f64, frequency_error: f64, t: f64| {
        let value = frequency * t;
        let error = libm::fma(frequency, t, -value) + frequency_error * t;
        let argument = value + error;
        if argument == 0. {
            1.
        } else {
            phase_sin(value, error) / (PI * argument)
        }
    };
    let (high_width, high_error) = sum_with_error(1., -cutoffs[count - 1]);
    let (band_width, band_error) = if count == 2 {
        sum_with_error(cutoffs[1], -cutoffs[0])
    } else {
        (cutoffs[0], 0.)
    };
    let (center, center_error) = if count == 2 {
        midpoint(cutoffs[0], cutoffs[1])
    } else {
        (0., 0.)
    };
    // Near Nyquist, evaluating cos(pi * center * half_integer) loses all
    // relative accuracy as center approaches 1. Keep the small complementary
    // distance before rounding the midpoint, and use exact integer/half-integer
    // phase identities. This matters for odd-order asymmetric custom windows.
    let (center_complement, complement_error) = if count == 2 {
        midpoint(1. - cutoffs[0], 1. - cutoffs[1])
    } else {
        (0., 0.)
    };
    // Both subtractions are exact in this range (Sterbenz's lemma).
    let use_complement = count == 2 && cutoffs[0] >= 0.5;
    let phase_arguments = |i: usize, frequency: f64, error: f64| {
        let t = i as f64 - middle;
        let value = frequency * t;
        (value, libm::fma(frequency, t, -value) + error * t)
    };
    let center_cos = |i: usize| {
        if use_complement {
            let (value, error) = phase_arguments(i, center_complement, complement_error);
            let sign = if (i + order / 2).is_multiple_of(2) {
                1.
            } else {
                -1.
            };
            if order.is_multiple_of(2) {
                sign * phase_cos(value, error)
            } else {
                -sign * phase_sin(value, error)
            }
        } else {
            let (value, error) = phase_arguments(i, center, center_error);
            phase_cos(value, error)
        }
    };
    let center_phase = |i: usize| {
        if use_complement {
            let (value, error) = phase_arguments(i, center_complement, complement_error);
            let (s, c) = phase_sin_cos(value, error);
            let sign = if (i + order / 2).is_multiple_of(2) {
                1.
            } else {
                -1.
            };
            if order.is_multiple_of(2) {
                (-sign * s, sign * c)
            } else {
                (-sign * c, -sign * s)
            }
        } else {
            let (value, error) = phase_arguments(i, center, center_error);
            phase_sin_cos(value, error)
        }
    };
    let total_stop_pass = if band == Band::Bandstop {
        cutoffs[0] + high_width
    } else {
        1.
    };
    // Reuse the ideal-band phase when accumulating the reference response.
    // No phase cache, extra output buffer or parallel reduction is required.
    let evaluate = |i: usize| {
        let t = i as f64 - middle;
        let ideal_cos = if band == Band::Bandpass {
            center_cos(i)
        } else {
            1.
        };
        // Preserve the asymmetric coefficient's original cosine-only path:
        // sin_cos may round its cosine differently on some target libraries.
        let (s, c) = if band == Band::Bandpass && options.scale && !symmetric {
            center_phase(i)
        } else {
            (0., ideal_cos)
        };
        let alternating = if (i + order / 2).is_multiple_of(2) {
            1.
        } else {
            -1.
        };
        let ideal = match band {
            Band::Lowpass => {
                let amplitude = if options.scale { 1. } else { cutoffs[0] };
                amplitude * sinc(cutoffs[0], 0., t)
            }
            Band::Highpass => {
                let amplitude = if options.scale { 1. } else { high_width };
                amplitude * alternating * sinc(high_width, high_error, t)
            }
            Band::Bandpass => {
                let amplitude = if options.scale { 1. } else { band_width };
                amplitude * ideal_cos * sinc(band_width / 2., band_error / 2., t)
            }
            Band::Bandstop => {
                let divisor = if options.scale { total_stop_pass } else { 1. };
                (cutoffs[0] / divisor) * sinc(cutoffs[0], 0., t)
                    + (high_width / divisor) * alternating * sinc(high_width, high_error, t)
            }
        };
        (ideal, s, c)
    };
    let weight = |i: usize| {
        window.map_or_else(
            || 0.54 - 0.46 * (2. * PI * i as f64 / order as f64).cos(),
            |w| w[i],
        ) / window_scale
    };
    let (mut real, mut imag, mut real_error, mut imag_error) = (0., 0., 0., 0.);
    let mut absolute_sum = 0.;
    let mut accumulate = |i: usize, value: f64, s: f64, c: f64| {
        if !options.scale {
            return;
        }
        let (re, im) = match band {
            Band::Lowpass | Band::Bandstop => (value, 0.),
            Band::Highpass => (if i.is_multiple_of(2) { value } else { -value }, 0.),
            Band::Bandpass => (value * c, if symmetric { 0. } else { value * s }),
        };
        let corrected = re - real_error;
        let next = real + corrected;
        real_error = (next - real) - corrected;
        real = next;
        let corrected = im - imag_error;
        let next = imag + corrected;
        imag_error = (next - imag) - corrected;
        imag = next;
        absolute_sum += value.abs();
    };
    let mut coefficients = Vec::with_capacity(order + 1);
    for i in 0..=order {
        if i.is_multiple_of(128) {
            check_cancel(cancel)?;
        }
        let (ideal, s, c) = evaluate(i);
        let value = ideal * weight(i);
        coefficients.push(value);
        accumulate(i, value, s, c);
    }
    if options.scale {
        let gain = if matches!(band, Band::Lowpass | Band::Bandstop) {
            real
        } else {
            real.hypot(imag)
        };
        if !gain.is_finite() || gain.abs() <= 16. * f64::EPSILON * absolute_sum {
            return Err("Degenerate FIR normalization; change the window, increase order or widen the passband".into());
        }
        for value in &mut coefficients {
            *value /= gain;
        }
    }
    if coefficients.iter().any(|v| !v.is_finite()) {
        return Err("FIR coefficient calculation overflowed".into());
    }
    check_cancel(cancel)?;
    Ok(coefficients)
}
