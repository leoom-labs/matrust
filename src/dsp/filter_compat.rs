//! Support for the MATLAB-compatible filter analysis calls: the automatic
//! impulse-response length of `impz`/`stepz`, the bilinear transformation and
//! second-order sections multiplied out into one transfer function.
//!
//! `impulse_length` follows the published rule of `impzlength` (MATLAB documentation;
//! GNU Octave signal 1.4.8 implements the same rule): FIR filters use length(b); stable
//! IIR filters run until the slowest pole has decayed to 5e-5, i.e.
//! floor(m * log10(5e-5) / log10(max|p|)) + delay, where m is the multiplicity of that pole
//! and delay the number of leading zeros of b; unstable filters (max|p| > 1 + 1e-4) use
//! floor(6 / log10(max|p|)); poles on the unit circle (within 1e-5) use five periods of
//! the slowest oscillation (at least 10 samples) or the damped poles' length, whichever is
//! longer, plus the delay. Deviations from Octave 1.4.8, both for inputs it mishandles:
//! a denominator whose a(2:end) are all zero is FIR (Octave returns the delay only), and
//! the dominant pole's multiplicity is the largest among the poles of maximal magnitude
//! (Octave uses whichever of them `roots` lists first). The length is at least 1.
use crate::{Complex64 as C, Matrix, Result};
use std::f64::consts::PI;

/// Decay level of the automatic impulse-response length (MATLAB/Octave `impzlength`).
pub const IMPULSE_LENGTH_TOLERANCE: f64 = 5e-5;
/// Largest automatic or explicit impulse/step response length.
pub const MAX_IMPULSE_LENGTH: usize = 65536;
const MAX_ROOT_DEGREE: usize = 256;
const MAX_SECTIONS: usize = super::MAX_SOS_SECTIONS;
const MAX_BILINEAR_DEGREE: usize = 40;

fn finite_coefficients(b: &[f64], a: &[f64]) -> Result<()> {
    if b.is_empty() || a.is_empty() || a[0] == 0. || b.iter().chain(a).any(|v| !v.is_finite()) {
        return Err("Filter coefficients must be finite nonempty vectors with nonzero a(1)".into());
    }
    Ok(())
}

fn convolve(x: &[f64], y: &[f64]) -> Vec<f64> {
    let mut out = vec![0.; x.len() + y.len() - 1];
    for (i, &u) in x.iter().enumerate() {
        for (j, &v) in y.iter().enumerate() {
            out[i + j] += u * v;
        }
    }
    out
}

/// The sections of an L-by-6 SOS matrix multiplied out: b = conv of rows (:, 1:3) and
/// a = conv of rows (:, 4:6), each of length 2L + 1, without normalization (MATLAB `sos2tf`).
pub fn sos_to_tf(sos: &Matrix) -> Result<(Vec<f64>, Vec<f64>)> {
    if sos.cols() != 6 || sos.rows() == 0 || sos.rows() > MAX_SECTIONS {
        return Err(format!(
            "SOS must have 1..{MAX_SECTIONS} rows and six columns [b0 b1 b2 a0 a1 a2]"
        ));
    }
    if sos.data().iter().any(|v| !v.is_finite()) {
        return Err("SOS coefficients must be finite".into());
    }
    let (mut b, mut a) = (vec![1.], vec![1.]);
    for r in 0..sos.rows() {
        let row: Vec<f64> = (0..6).map(|c| sos.get(r, c)).collect();
        b = convolve(&b, &row[..3]);
        a = convolve(&a, &row[3..]);
    }
    Ok((b, a))
}

/// Multiplicity of the dominant pole: among the roots of the given magnitude (within the
/// tolerance), the largest number of roots within the tolerance of one of them.
fn multiplicity(roots: &[C], magnitude: f64) -> usize {
    roots
        .iter()
        .filter(|r| (r.norm() - magnitude).abs() < IMPULSE_LENGTH_TOLERANCE)
        .map(|c| {
            roots
                .iter()
                .filter(|r| (*r - c).norm() < IMPULSE_LENGTH_TOLERANCE)
                .count()
        })
        .max()
        .unwrap_or(1)
        .max(1)
}

fn bounded_length(len: f64) -> Result<usize> {
    let len = if len.is_nan() { 1. } else { len.max(1.) };
    if len > MAX_IMPULSE_LENGTH as f64 {
        return Err(format!(
            "The automatic response length ({len}) exceeds {MAX_IMPULSE_LENGTH} samples; pass n explicitly"
        ));
    }
    Ok(len as usize)
}

/// Default number of samples of `impz(b, a)` / `stepz(b, a)`; see the module notes.
pub fn impulse_length(b: &[f64], a: &[f64]) -> Result<usize> {
    finite_coefficients(b, a)?;
    if a[1..].iter().all(|v| *v == 0.) {
        return bounded_length(b.len() as f64);
    }
    let delay = b.iter().position(|v| *v != 0.).unwrap_or(0) as f64;
    let coefficients: Vec<C> = a.iter().map(|&v| C::new(v, 0.)).collect();
    let roots = super::iir::roots_up_to(&coefficients, MAX_ROOT_DEGREE).map_err(|_| {
        format!(
            "The automatic response length needs a denominator of degree at most {MAX_ROOT_DEGREE}; pass n explicitly"
        )
    })?;
    if roots.iter().any(|r| !r.re.is_finite() || !r.im.is_finite()) {
        return Err("The filter poles are not finite".into());
    }
    let largest = |set: &[C]| set.iter().map(|r| r.norm()).fold(0., f64::max);
    let log10 = libm::log10;
    let tolerance = log10(IMPULSE_LENGTH_TOLERANCE);
    let max_pole = largest(&roots);
    let len = if max_pole > 1. + 1e-4 {
        // Unstable: growth to 1e6 (120 dB).
        (6. / log10(max_pole)).floor()
    } else if max_pole < 1. - 1e-5 {
        (multiplicity(&roots, max_pole) as f64 * tolerance / log10(max_pole)).floor() + delay
    } else {
        // Poles at z = 1 count as period 2 (as Octave), so they do not have arg 0.
        let min_angle = roots
            .iter()
            .filter(|r| (1. - 1e-5..=1. + 1e-4).contains(&r.norm()))
            .map(|&r| {
                if (r - C::new(1., 0.)).norm() < 1e-5 {
                    -r
                } else {
                    r
                }
            })
            .map(|r| libm::atan2(r.im, r.re).abs())
            .filter(|angle| *angle > 0.)
            .fold(f64::INFINITY, f64::min);
        let periodic = if min_angle.is_finite() {
            (5. * 2. * PI / min_angle).floor()
        } else {
            10.
        };
        let damped: Vec<C> = roots
            .iter()
            .copied()
            .filter(|r| r.norm() < 1. - 1e-5)
            .collect();
        let damped_len = if damped.is_empty() {
            0.
        } else {
            let max_damped = largest(&damped);
            (multiplicity(&damped, max_damped) as f64 * tolerance / log10(max_damped)).floor()
        };
        periodic.max(damped_len) + delay
    };
    bounded_length(len)
}

/// Default response length for second-order sections: `impulse_length` of the cascade
/// multiplied out, after removing trailing (then leading) coefficient pairs that are zero
/// in both b and a, as Octave's `sos2tf` does before `impzlength`.
pub fn sos_impulse_length(sos: &Matrix) -> Result<usize> {
    let (mut b, mut a) = sos_to_tf(sos)?;
    while b.len() > 1 && b.last() == Some(&0.) && a.last() == Some(&0.) {
        b.pop();
        a.pop();
    }
    let lead = b
        .iter()
        .zip(&a)
        .take_while(|(x, y)| **x == 0. && **y == 0.)
        .count()
        .min(b.len() - 1);
    impulse_length(&b[lead..], &a[lead..])
}

/// Scale K of the bilinear substitution s = K (z - 1)/(z + 1): K = 2 fs, or with
/// prewarping at fp Hz K = 2 pi fp / tan(pi fp / fs), so that the analog response at
/// fp Hz equals the digital response at fp Hz (MATLAB `bilinear(..., fs, fp)`).
pub fn bilinear_scale(fs: f64, fp: Option<f64>) -> Result<f64> {
    if !fs.is_finite() || fs <= 0. {
        return Err("bilinear: the sample rate fs must be a positive finite scalar".into());
    }
    match fp {
        None => Ok(2. * fs),
        Some(fp) if fp.is_finite() && fp > 0. && fp < fs / 2. => {
            let w = 2. * PI * fp;
            Ok(w / libm::tan(w / fs / 2.))
        }
        Some(_) => {
            Err("bilinear: the match frequency fp must lie strictly between 0 and fs/2".into())
        }
    }
}

/// Coefficients (powers of z^-1) of (1 - z^-1)^minus (1 + z^-1)^plus.
fn bilinear_basis(minus: usize, plus: usize) -> Vec<f64> {
    let mut p = vec![1.];
    for _ in 0..minus {
        p = convolve(&p, &[1., -1.]);
    }
    for _ in 0..plus {
        p = convolve(&p, &[1., 1.]);
    }
    p
}

/// Analog transfer function b(s)/a(s) (descending powers of s) to the digital b(z)/a(z)
/// (powers of z^-1) with s = k (z - 1)/(z + 1), normalized so that a(1) = 1. Leading zeros
/// are ignored; the numerator degree must not exceed the denominator degree.
pub fn bilinear_tf(b: &[f64], a: &[f64], k: f64) -> Result<(Vec<f64>, Vec<f64>)> {
    if b.is_empty() || a.is_empty() || b.iter().chain(a).any(|v| !v.is_finite()) {
        return Err("bilinear: num and den must be finite nonempty vectors".into());
    }
    let strip = |c: &[f64]| c[c.iter().position(|v| *v != 0.).unwrap_or(c.len())..].to_vec();
    let a = strip(a);
    if a.is_empty() {
        return Err("bilinear: the denominator must not be zero".into());
    }
    let mut b = strip(b);
    if b.is_empty() {
        b.push(0.);
    }
    if b.len() > a.len() {
        return Err("bilinear: the numerator degree must not exceed the denominator degree".into());
    }
    let order = a.len() - 1;
    if order > MAX_BILINEAR_DEGREE {
        return Err(format!(
            "bilinear: the denominator degree must be at most {MAX_BILINEAR_DEGREE}"
        ));
    }
    let expand = |c: &[f64]| {
        let degree = c.len() - 1;
        let mut out = vec![0.; order + 1];
        for (j, &value) in c.iter().enumerate() {
            let power = degree - j;
            let scale = value * k.powi(power as i32);
            for (o, v) in out.iter_mut().zip(bilinear_basis(power, order - power)) {
                *o += scale * v;
            }
        }
        out
    };
    let mut num = expand(&b);
    let mut den = expand(&a);
    let lead = den[0];
    if lead == 0. || !lead.is_finite() {
        return Err(
            "bilinear: a pole at s = 2*fs (or the prewarped scale) maps to infinity".into(),
        );
    }
    for v in num.iter_mut().chain(den.iter_mut()) {
        *v /= lead;
    }
    if num.iter().chain(&den).any(|v| !v.is_finite()) {
        return Err("bilinear: the transformed coefficients are not finite".into());
    }
    Ok((num, den))
}

/// Zeros, poles and gain mapped with s = k (z - 1)/(z + 1): z -> (k + z)/(k - z), extra
/// zeros at -1 up to the number of poles, gain * real(prod(k - z) / prod(k - p)).
pub fn bilinear_zpk(z: &[C], p: &[C], gain: f64, k: f64) -> Result<(Vec<C>, Vec<C>, f64)> {
    if z.len().max(p.len()) > 2 * MAX_SECTIONS {
        return Err("bilinear: at most 128 zeros and poles are supported".into());
    }
    if !gain.is_finite()
        || z.iter()
            .chain(p)
            .any(|r| !r.re.is_finite() || !r.im.is_finite())
    {
        return Err("bilinear: zeros, poles and gain must be finite".into());
    }
    if z.len() > p.len() {
        return Err("bilinear: more zeros than poles (the analog filter must be proper)".into());
    }
    let kc = C::new(k, 0.);
    let map = |r: &C| (kc + r) / (kc - r);
    let mut zd: Vec<C> = z.iter().map(map).collect();
    let pd: Vec<C> = p.iter().map(map).collect();
    zd.extend(std::iter::repeat_n(
        C::new(-1., 0.),
        p.len().saturating_sub(z.len()),
    ));
    let mut ratio = C::new(1., 0.);
    for i in 0..z.len().max(p.len()) {
        let num = z.get(i).map_or(C::new(1., 0.), |r| kc - r);
        let den = p.get(i).map_or(C::new(1., 0.), |r| kc - r);
        ratio *= num / den;
    }
    let kd = gain * ratio.re;
    if zd
        .iter()
        .chain(&pd)
        .any(|r| !r.re.is_finite() || !r.im.is_finite())
        || !kd.is_finite()
    {
        return Err(
            "bilinear: a pole or zero at s = 2*fs (or the prewarped scale) maps to infinity".into(),
        );
    }
    Ok((zd, pd, kd))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() <= 1e-12 * (1. + y.abs()), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn impulse_length_matches_the_published_impzlength_cases() {
        // Expected values: GNU Octave signal 1.4.8 impzlength test block (and Octave runs).
        let conv = |x: &[f64], y: &[f64]| convolve(x, y);
        assert_eq!(impulse_length(&[1., 2., 3., 4., 5.], &[1.]).unwrap(), 5);
        assert_eq!(impulse_length(&[1.], &[1., -0.95]).unwrap(), 193);
        assert_eq!(impulse_length(&[1.], &[1., -1.1]).unwrap(), 144);
        assert_eq!(impulse_length(&[1.], &[1., 0.5, 1.]).unwrap(), 17);
        assert_eq!(
            impulse_length(&[1.], &conv(&[1., -0.9], &[1., -0.9])).unwrap(),
            187
        );
        assert_eq!(
            impulse_length(&[1.], &conv(&[1., 0.5, 1.], &[1., -0.1])).unwrap(),
            17
        );
        assert_eq!(
            impulse_length(&[1.], &conv(&[1., 0.5, 1.], &[1., -0.9])).unwrap(),
            93
        );
        assert_eq!(impulse_length(&[0., 0., 1.], &[1., -0.95]).unwrap(), 195);
        assert_eq!(impulse_length(&[1.], &[1., -1.]).unwrap(), 10);
        // All poles at the origin: FIR.
        assert_eq!(impulse_length(&[1., 2., 3.], &[1., 0., 0.]).unwrap(), 3);
        // A pole within 1e-5 of the unit circle counts as marginal (10 samples); 0.9999 decays
        // past 65536 samples.
        assert_eq!(impulse_length(&[1.], &[1., -0.999_999_9]).unwrap(), 10);
        assert!(impulse_length(&[1.], &[1., -0.9999]).is_err());
        assert!(impulse_length(&[1.], &[0., 1.]).is_err());
    }

    #[test]
    fn sos_cascade_multiplies_out_without_normalizing() {
        let sos = Matrix::from_column_vec(
            2,
            6,
            vec![1., 0., 2., 1., 1., 0., 2., 1., 0.5, -0.5, 0.1, 0.],
        )
        .unwrap();
        let (b, a) = sos_to_tf(&sos).unwrap();
        close(&b, &conv_ref(&[1., 2., 1.], &[0., 1., 0.]));
        close(&a, &conv_ref(&[2., 0.5, 0.1], &[1., -0.5, 0.]));
        assert!(sos_to_tf(&Matrix::from_column_vec(1, 5, vec![1.; 5]).unwrap()).is_err());
        // The trailing zero pair is removed before the length rule.
        assert_eq!(
            sos_impulse_length(&sos).unwrap(),
            impulse_length(&[0., 1., 2., 1.], &[2., -0.5, -0.15, -0.05]).unwrap()
        );
    }

    fn conv_ref(x: &[f64], y: &[f64]) -> Vec<f64> {
        (0..x.len() + y.len() - 1)
            .map(|n| {
                (0..x.len())
                    .filter(|&k| n >= k && n - k < y.len())
                    .map(|k| x[k] * y[n - k])
                    .sum()
            })
            .collect()
    }

    #[test]
    fn bilinear_first_order_closed_form_and_prewarp_match() {
        // 1/(s + 1) with K = 2 fs = 20: (1 + q) / (21 - 19 q).
        let (b, a) = bilinear_tf(&[1.], &[1., 1.], bilinear_scale(10., None).unwrap()).unwrap();
        close(&b, &[1. / 21., 1. / 21.]);
        close(&a, &[1., -19. / 21.]);
        let (z, p, k) = bilinear_zpk(&[], &[C::new(-1., 0.)], 1., 20.).unwrap();
        assert_eq!(z, vec![C::new(-1., 0.)]);
        close(&[p[0].re, p[0].im, k], &[19. / 21., 0., 1. / 21.]);
        // Prewarping: H(j 2 pi fp) = H_d(exp(j 2 pi fp / fs)).
        let (fs, fp) = (100., 30.);
        let (b, a) = bilinear_tf(
            &[2., 0.],
            &[1., 3., 50.],
            bilinear_scale(fs, Some(fp)).unwrap(),
        )
        .unwrap();
        let s = C::new(0., 2. * PI * fp);
        let analog = (s * 2.) / (s * s + s * 3. + 50.);
        let q = C::new(0., -2. * PI * fp / fs).exp();
        let poly = |c: &[f64]| c.iter().rev().fold(C::new(0., 0.), |acc, &v| acc * q + v);
        let digital = poly(&b) / poly(&a);
        assert!((analog - digital).norm() < 1e-12 * analog.norm());
        assert!(bilinear_scale(100., Some(50.)).is_err());
        assert!(bilinear_scale(0., None).is_err());
        assert!(bilinear_tf(&[1., 0., 0.], &[1., 1.], 20.).is_err());
        assert!(bilinear_zpk(&[], &[C::new(20., 0.)], 1., 20.).is_err());
    }
}
