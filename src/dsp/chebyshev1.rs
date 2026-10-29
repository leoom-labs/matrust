//! Bounded Chebyshev-I ellipse prototype, genuine frequency transforms and SOS.
//! Small analytic design is serial; filtering uses the existing shared kernels.
use super::{Band, check_cancel};
use crate::{Complex64 as C, ComplexMatrix, Matrix, Result};
use std::{
    f64::consts::{LN_10, PI},
    sync::atomic::AtomicBool,
};

const RANGE: &str = "Chebyshev-I parameters cannot be represented as stable finite f64 sections";
pub const MAX_CHEBY1_ORDER: usize = 20;

#[derive(Clone, Debug)]
struct Section {
    b: [f64; 3],
    a: [f64; 3],
    poles: [C; 2],
    zeros: [C; 2],
    degree: usize,
}

/// At most 20 sections / 40 poles. No retained plan, pool or input payload copy.
#[derive(Clone, Debug)]
pub struct Chebyshev1 {
    sections: Vec<Section>,
    order: usize,
}

fn finite(p: C) -> bool {
    p.re.is_finite() && p.im.is_finite()
}
fn magnitude(p: C) -> f64 {
    p.re.hypot(p.im)
}
fn reciprocal(p: C) -> Result<C> {
    let scale = p.re.abs().max(p.im.abs());
    if !scale.is_finite() || scale == 0. {
        return Err(RANGE.into());
    }
    let q = p / scale;
    let result = (q.conj() / q.norm_sqr()) / scale;
    if !finite(result) {
        return Err(RANGE.into());
    }
    Ok(result)
}
fn digital(p: C) -> Result<C> {
    let scale = p.re.abs().max(p.im.abs()).max(1.);
    let result = (C::new(1. / scale, 0.) + p / scale) / (C::new(1. / scale, 0.) - p / scale);
    if !finite(result) || magnitude(result) >= 1. {
        return Err(RANGE.into());
    }
    Ok(result)
}

// Solve s^2 - width*p*s + center^2 = 0; bandstop uses the genuine
// reciprocal of p (Chebyshev poles do not lie on the Butterworth unit circle).
fn band_roots(p: C, width: f64, center: f64, stop: bool) -> Result<[C; 2]> {
    let p = if stop { reciprocal(p)? } else { p };
    let half = p * (0.5 * width);
    if !finite(half) {
        return Err(RANGE.into());
    }
    let scale = half.re.abs().max(half.im.abs()).max(center);
    let t = half / scale;
    let c = center / scale;
    let d = (t * t - C::new(c * c, 0.)).sqrt();
    let plus = t + d;
    let minus = t - d;
    let first = if magnitude(plus) >= magnitude(minus) {
        plus
    } else {
        minus
    } * scale;
    let second = (reciprocal(first)? * center) * center;
    if !finite(first) || !finite(second) || first.re >= 0. || second.re >= 0. {
        return Err(RANGE.into());
    }
    Ok([first, second])
}
fn sum(values: &[f64]) -> f64 {
    let mut total: f64 = 0.;
    let mut correction = 0.;
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
fn response_magnitude(c: [f64; 3], omega: f64) -> f64 {
    if omega == 0. {
        return sum(&c).abs();
    }
    if omega == PI {
        return sum(&[c[0], -c[1], c[2]]).abs();
    }
    let (s, cosine) = omega.sin_cos();
    let first = c[0] * cosine;
    let last = c[2] * cosine;
    sum(&[
        first,
        c[1],
        last,
        libm::fma(c[0], cosine, -first),
        libm::fma(c[2], cosine, -last),
    ])
    .hypot((c[0] - c[2]) * s)
}

fn section(poles: [C; 2], degree: usize, band: Band, omega: f64) -> Result<Section> {
    let a = [
        1.,
        -(poles[0].re + if degree == 2 { poles[1].re } else { 0. }),
        if degree == 2 {
            libm::fma(poles[0].re, poles[1].re, -(poles[0].im * poles[1].im))
        } else {
            0.
        },
    ];
    if a.iter().any(|v| !v.is_finite())
        || a[2].abs() >= 1.
        || sum(&[1., a[1], a[2]]) <= 0.
        || sum(&[1., -a[1], a[2]]) <= 0.
    {
        return Err(RANGE.into());
    }
    let (template, zeros, anchor) = match band {
        Band::Lowpass => (
            [1., degree as f64, if degree == 2 { 1. } else { 0. }],
            [C::new(-1., 0.); 2],
            0.,
        ),
        Band::Highpass => (
            [1., -(degree as f64), if degree == 2 { 1. } else { 0. }],
            [C::new(1., 0.); 2],
            PI,
        ),
        Band::Bandpass => ([1., 0., -1.], [C::new(1., 0.), C::new(-1., 0.)], omega),
        Band::Bandstop => (
            [1., -2. * omega.cos(), 1.],
            [C::from_polar(1., omega), C::from_polar(1., -omega)],
            0.,
        ),
    };
    let numerator = match band {
        Band::Bandpass => 2. * omega.sin(),
        Band::Bandstop => 4. * (0.5 * omega).sin().powi(2),
        _ => response_magnitude(template, anchor),
    };
    let denominator = response_magnitude(a, anchor);
    let gain = denominator / numerator;
    let b = template.map(|value| value * gain);
    if !gain.is_finite()
        || gain <= 0.
        || b.iter().any(|v| !v.is_finite())
        || !denominator.is_finite()
        || denominator <= 0.
        || (response_magnitude(b, anchor) / denominator - 1.).abs() > 1e-8
    {
        return Err(RANGE.into());
    }
    Ok(Section {
        b,
        a,
        poles,
        zeros,
        degree,
    })
}

/// Digital normalized edges, 1..20 prototype order, positive finite ripple dB.
/// Two-edge designs have twice the prototype order. Unrepresentable extremes
/// fail explicitly; finite transfer-function coefficients do not imply a
/// well-conditioned high-order direct-form realization. Prefer the SOS result.
pub fn cheby1(
    order: usize,
    ripple_db: f64,
    cutoffs: &[f64],
    band: Band,
    cancel: &AtomicBool,
) -> Result<Chebyshev1> {
    check_cancel(cancel)?;
    let paired = matches!(band, Band::Bandpass | Band::Bandstop);
    if !(1..=MAX_CHEBY1_ORDER).contains(&order)
        || !ripple_db.is_finite()
        || ripple_db <= 0.
        || cutoffs.len() != if paired { 2 } else { 1 }
        || cutoffs
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v >= 1.)
        || (paired && cutoffs[0] >= cutoffs[1])
    {
        return Err("cheby1 needs order 1..20, positive finite ripple dB, and one low/high or two increasing bandpass/stop edges strictly between 0 and 1".into());
    }
    let epsilon = (ripple_db * (LN_10 / 10.)).exp_m1().sqrt();
    let mu = (1. / epsilon).asinh() / order as f64;
    let (sinh, cosh) = (mu.sinh(), mu.cosh());
    if !epsilon.is_finite() || epsilon <= 0. || !sinh.is_finite() || sinh <= 0. || !cosh.is_finite()
    {
        return Err(RANGE.into());
    }
    let low = (PI * (0.5 * cutoffs[0])).tan();
    let high = if paired {
        (PI * (0.5 * cutoffs[1])).tan()
    } else {
        low
    };
    let center = low.sqrt() * high.sqrt();
    let width = high - low;
    if !low.is_finite()
        || low <= 0.
        || !center.is_finite()
        || center <= 0.
        || (paired && (!width.is_finite() || width <= 0.))
    {
        return Err(RANGE.into());
    }
    let omega = 2. * center.atan();
    let count = if paired { order } else { order.div_ceil(2) };
    let mut sections = Vec::new();
    sections
        .try_reserve_exact(count)
        .map_err(|_| "Cannot allocate Chebyshev sections")?;
    for j in 0..order.div_ceil(2) {
        check_cancel(cancel)?;
        let single = !order.is_multiple_of(2) && j == order / 2;
        let angle = (2 * j + 1) as f64 * PI / (2 * order) as f64;
        let prototype = if single {
            C::new(-sinh, 0.)
        } else {
            C::new(-sinh * angle.sin(), cosh * angle.cos())
        };
        if paired {
            let analog = band_roots(prototype, width, center, band == Band::Bandstop)?;
            if single {
                let p = digital(analog[0])?;
                let q = if analog[0].im == 0. {
                    digital(analog[1])?
                } else {
                    p.conj()
                };
                sections.push(section([p, q], 2, band, omega)?);
            } else {
                for root in analog {
                    let p = digital(root)?;
                    sections.push(section([p, p.conj()], 2, band, omega)?);
                }
            }
        } else {
            let analog = if band == Band::Highpass {
                reciprocal(prototype)? * low
            } else {
                prototype * low
            };
            let p = digital(analog)?;
            sections.push(section(
                [p, if single { C::new(0., 0.) } else { p.conj() }],
                if single { 1 } else { 2 },
                band,
                omega,
            )?);
        }
    }
    // Normalize every section at the prototype origin, then distribute the
    // even-order -Rp dB anchor gain. Odd-order origin gain is unity.
    let attenuation = if order.is_multiple_of(2) {
        (-ripple_db * (LN_10 / 20.) / count as f64).exp()
    } else {
        1.
    };
    for s in &mut sections {
        s.b = s.b.map(|v| v * attenuation);
        if !s.b[0].is_finite() || s.b[0] <= 0. {
            return Err(RANGE.into());
        }
    }
    sections.sort_by(|a, b| {
        magnitude(a.poles[0])
            .max(magnitude(a.poles[1]))
            .total_cmp(&magnitude(b.poles[0]).max(magnitude(b.poles[1])))
    });
    check_cancel(cancel)?;
    Ok(Chebyshev1 {
        sections,
        order: if paired { 2 * order } else { order },
    })
}

impl Chebyshev1 {
    pub fn order(&self) -> usize {
        self.order
    }
    pub fn sos(&self, cancel: &AtomicBool) -> Result<Matrix> {
        check_cancel(cancel)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(self.sections.len() * 6)
            .map_err(|_| "Cannot allocate Chebyshev SOS")?;
        for col in 0..6 {
            check_cancel(cancel)?;
            for section in &self.sections {
                values.push(if col < 3 {
                    section.b[col]
                } else {
                    section.a[col - 3]
                });
            }
        }
        Matrix::from_column_vec(self.sections.len(), 6, values)
    }
    pub fn transfer_function(&self, cancel: &AtomicBool) -> Result<(Matrix, Matrix)> {
        check_cancel(cancel)?;
        let expand = |numerator: bool| -> Result<Matrix> {
            let mut values = vec![1.];
            for section in &self.sections {
                check_cancel(cancel)?;
                let coefficients = if numerator { section.b } else { section.a };
                let mut next = vec![0.; values.len() + section.degree];
                for (i, value) in values.iter().enumerate() {
                    for j in 0..=section.degree {
                        next[i + j] += value * coefficients[j];
                    }
                }
                if next.iter().any(|v| !v.is_finite()) {
                    return Err(RANGE.into());
                }
                values = next;
            }
            if numerator && values[0] == 0. {
                return Err(RANGE.into());
            }
            Matrix::from_column_vec(1, self.order + 1, values)
        };
        let b = expand(true)?;
        let a = expand(false)?;
        check_cancel(cancel)?;
        Ok((b, a))
    }
    pub fn zero_pole_gain(
        &self,
        cancel: &AtomicBool,
    ) -> Result<(ComplexMatrix, ComplexMatrix, f64)> {
        check_cancel(cancel)?;
        let mut zeros = Vec::new();
        let mut poles = Vec::new();
        zeros
            .try_reserve_exact(self.order)
            .map_err(|_| "Cannot allocate Chebyshev zeros")?;
        poles
            .try_reserve_exact(self.order)
            .map_err(|_| "Cannot allocate Chebyshev poles")?;
        let mut gain = 1.;
        for section in &self.sections {
            check_cancel(cancel)?;
            zeros.extend_from_slice(&section.zeros[..section.degree]);
            poles.extend_from_slice(&section.poles[..section.degree]);
            gain *= section.b[0];
        }
        if !gain.is_finite() || gain <= 0. {
            return Err(RANGE.into());
        }
        Ok((
            ComplexMatrix::from_column_vec(self.order, 1, zeros, cancel)?,
            ComplexMatrix::from_column_vec(self.order, 1, poles, cancel)?,
            gain,
        ))
    }
}
