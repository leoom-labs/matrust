//! Bounded Butterworth prototype -> band transform -> real digital SOS.
//! Existing scalar low/high design is deliberately delegated unchanged.
use super::{Band, butter_sos, check_cancel};
use crate::{Matrix, Result};
use nalgebra::Complex;
use std::{f64::consts::PI, sync::atomic::AtomicBool};

type C = Complex<f64>;
const RANGE: &str = "Band edges cannot be represented as stable finite f64 Butterworth SOS";

/// `order` is the prototype order (1..20). Two-edge designs have order 2*order.
/// Small design work is serial; callers use the existing shared-pool SOS kernels.
pub fn butter_sos_with_cutoffs(
    order: usize,
    cutoffs: &[f64],
    band: Band,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    check_cancel(cancel)?;
    let paired = matches!(band, Band::Bandpass | Band::Bandstop);
    if !(1..=20).contains(&order)
        || cutoffs.len() != if paired { 2 } else { 1 }
        || cutoffs
            .iter()
            .any(|x| !x.is_finite() || *x <= 0. || *x >= 1.)
        || (paired && cutoffs[0] >= cutoffs[1])
    {
        return Err("Butterworth prototype order must be 1..20; low/high need one cutoff, bandpass/stop need two increasing cutoffs strictly between 0 and 1".into());
    }
    if !paired {
        let result = butter_sos(order, cutoffs[0], band)?;
        check_cancel(cancel)?;
        return Ok(result);
    }
    let low = (PI * (0.5 * cutoffs[0])).tan();
    let high = (PI * (0.5 * cutoffs[1])).tan();
    let width = high - low;
    // Square roots avoid underflow in low*high for small representable edges.
    let center = low.sqrt() * high.sqrt();
    if !width.is_finite() || width <= 0. || !center.is_finite() || center <= 0. {
        return Err(RANGE.into());
    }
    let omega = 2. * center.atan();
    let mut sections = Vec::with_capacity(order);
    for j in 0..order / 2 {
        check_cancel(cancel)?;
        let angle = (2 * j + 1) as f64 * PI / (2 * order) as f64;
        let pole = C::new(-angle.sin(), angle.cos());
        let transformed = roots(pole, width, center, band)?;
        for analog in transformed {
            let pole = bilinear(analog)?;
            sections.push(section(pole, pole.conj(), omega, band)?);
        }
    }
    if !order.is_multiple_of(2) {
        check_cancel(cancel)?;
        let analog = roots(C::new(-1., 0.), width, center, band)?;
        let first = bilinear(analog[0])?;
        // A real prototype yields either two real poles or one conjugate pair.
        // Enforce conjugacy instead of importing tiny division roundoff into a2.
        let second = if analog[0].im == 0. {
            bilinear(analog[1])?
        } else {
            first.conj()
        };
        sections.push(section(first, second, omega, band)?);
    }
    check_cancel(cancel)?;
    // Lower-radius sections first. Stable deterministic ordering; no global gain
    // product that can underflow before being distributed across sections.
    sections.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut data = Vec::with_capacity(order * 6);
    for column in 0..6 {
        for (_, row) in &sections {
            data.push(row[column]);
        }
    }
    check_cancel(cancel)?;
    Matrix::from_column_vec(order, 6, data)
}

fn finite(z: C) -> bool {
    z.re.is_finite() && z.im.is_finite()
}

/// Solve s² - width*p*s + center² = 0 (bandpass), or replace p by 1/p
/// (bandstop). Prototype poles lie on the unit circle, so 1/p = conjugate(p).
/// Normalize before squaring and obtain the small root from the product of
/// roots. The subtractive quadratic formula loses narrow/complementary roots.
fn roots(pole: C, width: f64, center: f64, band: Band) -> Result<[C; 2]> {
    let half = width * 0.5;
    let scale = half.max(center);
    let prototype = if band == Band::Bandstop {
        pole.conj()
    } else {
        pole
    };
    let t = prototype * (half / scale);
    let ratio = center / scale;
    let d = (t * t - C::new(ratio * ratio, 0.)).sqrt();
    let plus = t + d;
    let minus = t - d;
    let q = if plus.norm_sqr() >= minus.norm_sqr() {
        plus
    } else {
        minus
    };
    let first = q * scale;
    // first is the large root. Dividing before the second multiplication avoids
    // both center² overflow and underflow. If the complex division itself loses
    // representability at an extreme endpoint, finite/stability checks reject
    // the design; this is not a promise of arbitrary-subnormal pole support.
    let second = (C::new(center, 0.) / first) * center;
    if !finite(first) || !finite(second) || first.re >= 0. || second.re >= 0. {
        return Err(RANGE.into());
    }
    Ok([first, second])
}

fn bilinear(pole: C) -> Result<C> {
    let digital = (C::new(1., 0.) + pole) / (C::new(1., 0.) - pole);
    if !finite(digital) || digital.norm_sqr() >= 1. {
        return Err(RANGE.into());
    }
    Ok(digital)
}

// Error-free summation compensation on bounded coefficient terms. In contrast
// to raw a1+(1+a2)*cos(w), preserve the product residual near narrow passbands.
fn sum(values: &[f64]) -> f64 {
    let mut total: f64 = 0.;
    let mut correction: f64 = 0.;
    for &value in values {
        let next = total + value;
        correction += if total.abs() >= value.abs() {
            (total - next) + value
        } else {
            (value - next) + total
        };
        total = next;
    }
    total + correction
}

fn magnitude(coefficients: [f64; 3], omega: f64) -> f64 {
    let (s, c) = omega.sin_cos();
    let product = coefficients[2] * c;
    let real = sum(&[
        coefficients[0] * c,
        coefficients[1],
        product,
        libm::fma(coefficients[2], c, -product),
        libm::fma(coefficients[0], c, -(coefficients[0] * c)),
    ]);
    let imaginary = (coefficients[0] - coefficients[2]) * s;
    real.hypot(imaginary)
}

fn section(p: C, q: C, omega: f64, band: Band) -> Result<(f64, [f64; 6])> {
    let a1 = -(p.re + q.re);
    let a2 = libm::fma(p.re, q.re, -(p.im * q.im));
    let dc = sum(&[1., a1, a2]);
    let nyquist = sum(&[1., -a1, a2]);
    if !a1.is_finite() || !a2.is_finite() || a2.abs() >= 1. || dc <= 0. || nyquist <= 0. {
        return Err(RANGE.into());
    }
    let (template, anchor, denominator) = if band == Band::Bandpass {
        ([1., 0., -1.], omega, magnitude([1., a1, a2], omega))
    } else {
        ([1., -2. * omega.cos(), 1.], 0., dc)
    };
    let numerator = if band == Band::Bandpass {
        2. * omega.sin()
    } else {
        sum(&template)
    };
    let gain = denominator / numerator;
    let b = template.map(|x| x * gain);
    if !gain.is_finite() || gain <= 0. || b.iter().any(|x| !x.is_finite()) {
        return Err(RANGE.into());
    }
    let represented = if band == Band::Bandpass {
        magnitude(b, anchor)
    } else {
        sum(&b)
    };
    // Reject coefficient-level loss of the normalizing response instead of
    // silently emitting a nominally finite but unusable narrow-band section.
    if !represented.is_finite() || (represented / denominator - 1.).abs() > 1e-8 {
        return Err(RANGE.into());
    }
    Ok((
        p.norm_sqr().max(q.norm_sqr()),
        [b[0], b[1], b[2], 1., a1, a2],
    ))
}
