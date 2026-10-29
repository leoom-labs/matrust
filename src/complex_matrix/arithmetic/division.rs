//! Scalar complex division adapted from LAPACK DLADIV/DLADIV1/DLADIV2
//! (Baudin/Smith), source hash and full license in the research provenance.
//! Distribution must retain THIRD_PARTY_LAPACK_LICENSE.txt.
//! The finite general path preserves the published operation order. Pure real
//! and imaginary denominators avoid needless cross-products. Nonfinite/zero
//! denominators use the existing num-complex IEEE behavior; MATLAB classification
//! parity for these cases has not been established by an installed runtime.
use super::Complex64;

pub(super) fn divide(x: Complex64, y: Complex64) -> Complex64 {
    if !x.re.is_finite()
        || !x.im.is_finite()
        || !y.re.is_finite()
        || !y.im.is_finite()
        || (y.re == 0. && y.im == 0.)
    {
        return x / y;
    }
    if y.im == 0. {
        return Complex64::new(x.re / y.re, x.im / y.re);
    }
    if y.re == 0. {
        return Complex64::new(x.im / y.im, -x.re / y.im);
    }
    let (mut a, mut b, mut c, mut d) = (x.re, x.im, y.re, y.im);
    let ab = a.abs().max(b.abs());
    let cd = c.abs().max(d.abs());
    let mut scale = 1.;
    // DLAMCH('Epsilon') is half Rust's distance from1 to its successor for
    // IEEE round-to-nearest. Safe minimum is binary64's minimum normal value.
    const EPS: f64 = f64::EPSILON * 0.5;
    const BOOST: f64 = 2. / (EPS * EPS);
    if ab >= f64::MAX * 0.5 {
        a *= 0.5;
        b *= 0.5;
        scale *= 2.;
    }
    if cd >= f64::MAX * 0.5 {
        c *= 0.5;
        d *= 0.5;
        scale *= 0.5;
    }
    if ab <= f64::MIN_POSITIVE * 2. / EPS {
        a *= BOOST;
        b *= BOOST;
        scale /= BOOST;
    }
    if cd <= f64::MIN_POSITIVE * 2. / EPS {
        c *= BOOST;
        d *= BOOST;
        scale *= BOOST;
    }
    let (p, q) = if y.im.abs() <= y.re.abs() {
        divide_pair(a, b, c, d)
    } else {
        let (p, q) = divide_pair(b, a, d, c);
        (p, -q)
    };
    Complex64::new(p * scale, q * scale)
}
fn divide_pair(a: f64, b: f64, c: f64, d: f64) -> (f64, f64) {
    let ratio = d / c;
    let inverse = 1. / (c + d * ratio);
    (
        component(a, b, c, d, ratio, inverse),
        component(b, -a, c, d, ratio, inverse),
    )
}
fn component(a: f64, b: f64, c: f64, d: f64, ratio: f64, inverse: f64) -> f64 {
    if ratio != 0. {
        let product = b * ratio;
        if product != 0. {
            (a + product) * inverse
        } else {
            a * inverse + (b * inverse) * ratio
        }
    } else {
        (a + d * (b / c)) * inverse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaled_division_handles_extremes_and_exact_component_controls() {
        for z in [
            Complex64::new(f64::MAX, f64::MAX),
            Complex64::new(f64::MIN_POSITIVE, f64::MIN_POSITIVE),
            Complex64::new(f64::from_bits(1), f64::from_bits(1)),
            Complex64::new(1e300, -1e300),
        ] {
            let answer = divide(z, z);
            assert!((answer.re - 1.).abs() <= 4. * f64::EPSILON);
            assert_eq!(answer.im, 0.);
        }
        assert_eq!(
            divide(Complex64::new(3., 4.), Complex64::new(1., -2.)),
            Complex64::new(-1., 2.)
        );
        let extreme = divide(
            Complex64::new(f64::MAX, f64::from_bits(1)),
            Complex64::new(f64::from_bits(1), 0.),
        );
        assert!(extreme.re.is_infinite());
        assert_eq!(extreme.im, 1.);
        let value = divide(Complex64::new(0., -0.), Complex64::new(2., 0.));
        assert_eq!(value.re.to_bits(), 0);
        assert_eq!(value.im.to_bits(), (-0f64).to_bits());
    }
}
