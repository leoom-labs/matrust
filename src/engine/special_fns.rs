//! Special functions: Bessel (`besselj`, `bessely`, `besseli`,
//! `besselk`, `besselh`), `airy`, incomplete gamma and beta functions and their
//! inverses, `expint`, `psi`, `legendre`, `ellipke`, `ellipj` and `erfcx`.
//!
//! Sources, cited again at each kernel:
//! - Real-argument Bessel functions: Temme, "On the numerical evaluation of
//!   the ordinary Bessel function of the second kind" (1976), section 2,
//!   and "... modified Bessel function of the third kind" (1975), sections
//!   2-3; continued fractions and normalization (DLMF 3.10, 10.74; Barnett,
//!   "The Calculation of Spherical Bessel Functions and Coulomb Functions"
//!   (1996), section 6); Hankel's expansions (DLMF 10.17, 10.40).
//! - Complex arguments: power series (DLMF 10.2.2, 10.25.2, 10.8.1, 10.31.1),
//!   the analytic continuations DLMF 10.11, 10.34 and 10.27.
//! - Incomplete gamma/beta and `expint`: the series and continued fractions of
//!   DLMF 8.7.1, 8.9.2, 8.17.22, 6.6 and 6.9, evaluated by Lentz's method
//!   (Thompson and Barnett, J. Comput. Phys. 64 (1986)), with the region choice
//!   of DiDonato and Morris (ACM TOMS 12 (1986)) and Temme's prefactor for large
//!   parameters (DLMF 8.12); the inverses refine DLMF 8.12 / Abramowitz and
//!   Stegun 26.5.22 starting values by Halley's method.
//! - `ellipke` and `ellipj` use arithmetic-geometric mean iterations;
//!   the tolerance controls when the approximation stops.
//!
//! Real or complex double arguments; `erfcx` also keeps `single`.
use super::{Engine, Value};
use crate::engine::Fma;
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

type C = Complex64;

const NAMES: &[&str] = &[
    "besselj",
    "bessely",
    "besseli",
    "besselk",
    "besselh",
    "airy",
    "gammainc",
    "gammaincinv",
    "betainc",
    "betaincinv",
    "expint",
    "psi",
    "legendre",
    "ellipke",
    "ellipj",
    "erfcx",
];

const CHUNK: usize = 256;
const EVALUATION_UNITS: usize = 32 * 1024 * 1024;
const EPS: f64 = 1e-16;
const FPMIN: f64 = 1e-300;
const MAXIT: usize = 100_000;
const EULER: f64 = std::f64::consts::EULER_GAMMA;
/// Largest `legendre` degree (the output has n+1 rows per point).
const LEGENDRE_MAX_DEGREE: f64 = 10_000.;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn c(re: f64, im: f64) -> C {
    C::new(re, im)
}

/// One numeric argument as column-major parts.
struct Arg {
    rows: usize,
    cols: usize,
    re: Vec<f64>,
    im: Option<Vec<f64>>,
}

impl Arg {
    fn len(&self) -> usize {
        self.re.len()
    }
    fn scalar(&self) -> bool {
        self.re.len() == 1
    }
    fn is_complex(&self) -> bool {
        self.im.is_some()
    }
    fn re_at(&self, i: usize) -> f64 {
        self.re[if self.scalar() { 0 } else { i }]
    }
    fn at(&self, i: usize) -> C {
        let j = if self.scalar() { 0 } else { i };
        c(self.re[j], self.im.as_ref().map_or(0., |v| v[j]))
    }
    fn is_vector(&self) -> bool {
        self.rows == 1 || self.cols == 1
    }
}

/// Common size of element-wise arguments: equal sizes, scalars expand.
fn common_shape(args: &[&Arg], error: &str) -> Result<(usize, usize)> {
    let mut shape = None;
    for a in args {
        if a.scalar() {
            continue;
        }
        match shape {
            None => shape = Some((a.rows, a.cols)),
            Some(s) if s == (a.rows, a.cols) => {}
            Some(_) => return Err(error.into()),
        }
    }
    Ok(shape.unwrap_or((1, 1)))
}

fn arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(chars)) if chars.rows() <= 1 => {
            Some(String::from_utf16_lossy(chars.units()))
        }
        _ => None,
    }
}

// ---------- elementary helpers ----------

/// sin(pi x), exact at multiples of 1/2.
fn sinpi(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let mut r = x % 2.;
    if r < 0. {
        r += 2.;
    }
    let (r, sign) = if r >= 1. { (r - 1., -1.) } else { (r, 1.) };
    let value = if r == 0. {
        0.
    } else if r == 0.5 {
        1.
    } else if r < 0.5 {
        libm::sin(PI * r)
    } else {
        libm::sin(PI * (1. - r))
    };
    sign * value
}

/// cos(pi x), exact at multiples of 1/2.
fn cospi(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let r = x.abs() % 2.;
    let (r, sign) = if r >= 1. { (r - 1., -1.) } else { (r, 1.) };
    let value = if r == 0. {
        1.
    } else if r == 0.5 {
        0.
    } else if r < 0.5 {
        libm::cos(PI * r)
    } else {
        -libm::cos(PI * (1. - r))
    };
    sign * value
}

/// e^{i pi t}.
fn cis_pi(t: f64) -> C {
    c(cospi(t), sinpi(t))
}

fn cexp(z: C) -> C {
    let m = libm::exp(z.re);
    if z.im == 0. {
        return c(m, 0.);
    }
    c(m * libm::cos(z.im), m * libm::sin(z.im))
}

/// z * f without turning a zero imaginary part into NaN when f is infinite.
fn scale(z: C, f: f64) -> C {
    c(z.re * f, if z.im == 0. { 0. } else { z.im * f })
}

fn is_integer(x: f64) -> bool {
    x.is_finite() && x == x.trunc()
}

/// Taylor coefficients of 1/Gamma(1+x), generated from DLMF 5.7.1-5.7.2.
/// `tool/cleanroom-bessel/reference.py` reproduces these rounded doubles.
const RECIPROCAL_GAMMA: [f64; 19] = [
    1.0,
    EULER,
    -0.655_878_071_520_253_9,
    -0.042_002_635_034_095_24,
    0.166_538_611_382_291_48,
    -0.042_197_734_555_544_33,
    -0.009_621_971_527_876_973,
    0.007_218_943_246_663_1,
    -0.001_165_167_591_859_065_2,
    -0.000_215_241_674_114_950_98,
    0.000_128_050_282_388_116_2,
    -2.013_485_478_078_824e-5,
    -1.250_493_482_142_670_6e-6,
    1.133_027_231_981_696e-6,
    -2.056_338_416_977_607e-7,
    6.116_095_104_481_416e-9,
    5.002_007_644_469_223e-9,
    -1.181_274_570_487_02e-9,
    1.043_426_711_691_100_5e-10,
];

/// Temme (1975), section 2, eq. (2.11): odd/even reciprocal-gamma parts
/// and 1/Gamma(1+mu), 1/Gamma(1-mu), for |mu| <= 1/2.
fn temme_gammas(mu: f64) -> (f64, f64, f64, f64) {
    let square = mu * mu;
    let mut even = 0.;
    let mut odd = 0.;
    for n in (0..RECIPROCAL_GAMMA.len()).rev() {
        if n % 2 == 0 {
            even = even * square + RECIPROCAL_GAMMA[n];
        } else {
            odd = odd * square + RECIPROCAL_GAMMA[n];
        }
    }
    (-odd, even, even + mu * odd, even - mu * odd)
}

// ---------- Bessel functions of real argument ----------

/// Hankel's expansion terms a_k(nu)/x^k (DLMF 10.17.1), until the smallest.
fn hankel_terms(nu: f64, x: f64) -> Vec<f64> {
    let mu = 4. * nu * nu;
    let mut terms = vec![1.];
    let mut term: f64 = 1.;
    for k in 1..200 {
        let odd = (2 * k - 1) as f64;
        let next = term * (mu - odd * odd) / (k as f64 * 8. * x);
        if next == 0. || next.abs() >= term.abs() || next.abs() < 1e-18 {
            if next != 0. && next.abs() < term.abs() {
                terms.push(next);
            }
            break;
        }
        terms.push(next);
        term = next;
    }
    terms
}

/// J and Y for large x (DLMF 10.17.3/10.17.4).
fn jy_asymptotic(nu: f64, x: f64) -> (f64, f64) {
    let terms = hankel_terms(nu, x);
    let (mut p, mut q) = (0., 0.);
    for (k, t) in terms.iter().enumerate() {
        let sign = if (k / 2) % 2 == 0 { 1. } else { -1. };
        if k % 2 == 0 {
            p += sign * t;
        } else {
            q += sign * t;
        }
    }
    // chi = x - phi with phi = (nu/2 + 1/4) pi, expanded so that x is reduced exactly.
    let (sx, cx) = (libm::sin(x), libm::cos(x));
    let (sp, cp) = (sinpi(nu / 2. + 0.25), cospi(nu / 2. + 0.25));
    let cos_chi = cx * cp + sx * sp;
    let sin_chi = sx * cp - cx * sp;
    let factor = (2. / (PI * x)).sqrt();
    (
        factor * (p * cos_chi - q * sin_chi),
        factor * (p * sin_chi + q * cos_chi),
    )
}

/// J_nu and Y_nu, nu >= 0, x > 0. Temme (1976), section 2, and
/// Steed normalization: Barnett, "The Calculation of Spherical Bessel
/// Functions and Coulomb Functions" (1996), section 6, eqs. (31)-(34).
fn jy_real(nu: f64, x: f64) -> (f64, f64) {
    if x > f64::MAX / PI {
        // DLMF 10.17.3-10.17.4: at these x and admitted orders the leading
        // term is sufficient. Form its amplitude without overflowing pi*x.
        let amplitude = (2. / PI).sqrt() / x.sqrt();
        let phase = nu / 2. + 0.25;
        let (s, co) = (libm::sin(x), libm::cos(x));
        let (sp, cp) = (sinpi(phase), cospi(phase));
        return (
            amplitude * (co * cp + s * sp),
            amplitude * (s * cp - co * sp),
        );
    }
    if x >= 32. + 0.5 * nu * nu {
        return jy_asymptotic(nu, x);
    }
    let steps = (nu + 0.5).floor() as usize;
    let mu = nu - steps as f64;
    if x >= 32. && nu < x {
        // In the oscillatory region both forward recurrences are stable
        // (DLMF 10.74(iv)). Small-order Hankel seeds avoid a CF1 depth of O(x)
        // when x is large but not yet large relative to nu squared.
        let (mut j, mut y) = jy_asymptotic(mu, x);
        let (mut j_next, mut y_next) = jy_asymptotic(mu + 1., x);
        for n in 1..=steps {
            let factor = 2. * (mu + n as f64) / x;
            (j, j_next) = (j_next, factor * j_next - j);
            (y, y_next) = (y_next, factor * y_next - y);
        }
        return (j, y);
    }
    let (j, mut y, mut y_next) = if x < 2. {
        let (y, next) = temme_second_pair(mu, x, -1.);
        (real_first_series(nu, x, -1.), y, next)
    } else {
        let (derivative, sign) = bessel_log_derivative(nu, x, -1.);
        // Carry an arbitrary J normalization downwards; retain the value at
        // the requested order while rescaling to avoid intermediate overflow.
        let mut target = sign;
        let mut lower = sign;
        let mut upper = sign * (nu / x - derivative);
        for n in (1..=steps).rev() {
            let previous = 2. * (mu + n as f64) / x * lower - upper;
            upper = lower;
            lower = previous;
            if lower.abs().max(upper.abs()) > 1e100 {
                lower *= 1e-100;
                upper *= 1e-100;
                target *= 1e-100;
            }
        }
        let h = hankel_log_derivative(mu, x);
        let slope = mu / x * lower - upper;
        let irregular = (h.re * lower - slope) / h.im;
        let normalization = ((2. / PI / x) / h.im).sqrt() / lower.hypot(irregular);
        let j_mu = lower * normalization;
        let y_mu = irregular * normalization;
        let y_next = (mu / x - h.re) * y_mu - h.im * j_mu;
        (target * normalization, y_mu, y_next)
    };
    for n in 1..=steps {
        let next = 2. * (mu + n as f64) / x * y_next - y;
        y = y_next;
        y_next = next;
        if y.is_infinite() {
            break;
        }
    }
    (j, y)
}

/// Exponentially scaled I_nu and K_nu. Temme (1975), sections 2-3,
/// DLMF 10.33.1 (I ratio) and 10.28.2 (Wronskian normalization).
fn ik_real(nu: f64, x: f64) -> (f64, f64) {
    if x >= 32. + 0.5 * nu * nu {
        let terms = hankel_terms(nu, x);
        let mut i = 0.;
        let mut k = 0.;
        for (n, term) in terms.iter().enumerate() {
            k += term;
            i += if n % 2 == 0 { *term } else { -term };
        }
        return (
            i / (2. * PI).sqrt() / x.sqrt(),
            k * (PI / 2.).sqrt() / x.sqrt(),
        );
    }
    let steps = (nu + 0.5).floor() as usize;
    let mu = nu - steps as f64;
    let (mut k, mut next) = if x < 2. {
        let (k, next) = temme_second_pair(mu, x, 1.);
        let exponential = libm::exp(x);
        (k * exponential, next * exponential)
    } else {
        temme_k_scaled(mu, x)
    };
    let mut rescalings = 0;
    for n in 1..=steps {
        if next.abs() > 1e200 {
            k *= 1e-200;
            next *= 1e-200;
            rescalings += 1;
        }
        let higher = k + 2. * (mu + n as f64) / x * next;
        k = next;
        next = higher;
    }
    let mut i = if x < 2. {
        real_first_series(nu, x, 1.) * libm::exp(-x)
    } else {
        let (derivative, _) = bessel_log_derivative(nu, x, 1.);
        1. / (next + (derivative - nu / x) * k) / x
    };
    for _ in 0..rescalings {
        k *= 1e200;
        if x >= 2. {
            i *= 1e-200;
        }
    }
    (i, k)
}

/// (x/2)^nu / Gamma(nu + 1), the leading factor of the small-x J/I power
/// series (DLMF 10.2.2, 10.25.2), for 0 < x < 2 and nu >= 0. With nu = n + f
/// the recurrence Gamma(z + 1) = z Gamma(z) (DLMF 5.5.1) gives
/// Gamma(nu + 1) = Gamma(1 + f) prod_{k=1..n} (f + k). Each f + k equals
/// nu - (n - k) and is exact; the product is carried in double-double
/// (error-free products, Knuth 1997, section 4.2.2) and the power comes from
/// pow, so no rounding error grows with the order. The earlier
/// exp(nu ln(x/2) - ln Gamma(nu + 1)) subtracts two logarithms of size O(nu
/// ln nu) and loses about nu ln(nu) eps in relative accuracy.
fn real_series_lead(nu: f64, x: f64) -> f64 {
    let half = 0.5 * x;
    let whole = nu.floor();
    let mut hi = libm::tgamma(1. + (nu - whole));
    let mut lo = 0.;
    let mut k = 1.;
    while k <= whole {
        let (product, error) = two_prod(hi, nu - (whole - k));
        if !product.is_finite() {
            // Gamma(nu + 1) overflows, so the factor is below 2e-308: only a
            // subnormal or zero result remains, for which the logarithmic
            // form is accurate enough.
            return libm::exp(nu * libm::log(half) - libm::lgamma(nu + 1.));
        }
        (hi, lo) = (product, lo * (nu - (whole - k)) + error);
        k += 1.;
    }
    libm::pow(half, nu) / (hi + lo)
}

/// Small-x J/I power series (DLMF 10.2.2, 10.25.2).
fn real_first_series(nu: f64, x: f64, sign: f64) -> f64 {
    let lead = real_series_lead(nu, x);
    let mut term = 1.;
    let mut sum = 1.;
    for n in 1..MAXIT {
        let n = n as f64;
        term *= sign * (x * 0.5) * (x * 0.5) / (n * (nu + n));
        sum += term;
        if term.abs() <= EPS * sum.abs() {
            break;
        }
    }
    lead * sum
}

/// Y_mu,Y_{mu+1} (sign=-1) or K_mu,K_{mu+1} (sign=+1), |mu|<=1/2.
/// Temme (1976), section 2, eqs. (2.2)-(2.3); Temme (1975), section 2,
/// eqs. (2.1)-(2.11). The odd gamma part and sinh(t)/t avoid cancellation.
fn temme_second_pair(mu: f64, x: f64, sign: f64) -> (f64, f64) {
    let (odd, even, plus, minus) = temme_gammas(mu);
    let logarithm = std::f64::consts::LN_2 - libm::log(x);
    let t = mu * logarithm;
    let sinc = if mu == 0. { 1. } else { PI * mu / sinpi(mu) };
    let sinhc = if t == 0. { 1. } else { libm::sinh(t) / t };
    let mut f = sinc * (odd * libm::cosh(t) + even * logarithm * sinhc);
    let mut p = 0.5 * libm::exp(t) / plus;
    let mut q = 0.5 * libm::exp(-t) / minus;
    let correction = if sign < 0. && mu != 0. {
        2. * sinpi(mu * 0.5).powi(2) / mu
    } else {
        0.
    };
    let mut weight = 1.;
    let mut value = f + correction * q;
    let mut adjacent = p;
    for n in 1..MAXIT {
        let n = n as f64;
        f = (n * f + p + q) / ((n - mu) * (n + mu));
        p /= n - mu;
        q /= n + mu;
        weight *= sign * (x * 0.5) * (x * 0.5) / n;
        let g = f + correction * q;
        let increment = weight * g;
        let adjacent_increment = weight * (p - n * g);
        value += increment;
        adjacent += adjacent_increment;
        if increment.abs() <= EPS * value.abs() && adjacent_increment.abs() <= EPS * adjacent.abs()
        {
            break;
        }
    }
    let factor = if sign < 0. { -2. / PI } else { 1. };
    (factor * value, (factor * adjacent) / x * 2.)
}

/// CF1 for J'/J or I'/I, from DLMF 10.10.1 and 10.33.1.
/// Modified Lentz evaluation (DLMF 3.10(iii)); the denominator signs give
/// the sign of J, since the sufficiently high-order minimal solution is positive.
fn bessel_log_derivative(nu: f64, x: f64, sign: f64) -> (f64, f64) {
    let protect = |v: f64| {
        if v.abs() < FPMIN {
            FPMIN.copysign(v)
        } else {
            v
        }
    };
    let mut value = protect(nu / x);
    let mut numerator = value;
    let mut denominator = 0.;
    let mut orientation = 1.;
    for n in 1..=MAXIT {
        let b = 2. * (nu + n as f64) / x;
        denominator = 1. / protect(b + sign * denominator);
        numerator = protect(b + sign / numerator);
        let ratio = numerator * denominator;
        value *= ratio;
        if denominator < 0. {
            orientation = -orientation;
        }
        if (ratio - 1.).abs() <= 2. * f64::EPSILON {
            break;
        }
    }
    (value, orientation)
}

/// CF2 for H'/H. Barnett, "The Calculation of Spherical Bessel Functions
/// and Coulomb Functions" (1996), section 6, eq. (34), eta=0, L=mu-1/2.
/// The -1/(2x) term converts Riccati-Bessel to cylindrical functions.
/// Evaluate the convergent differences by DLMF 3.10.16-3.10.17 (Steed).
fn hankel_log_derivative(mu: f64, x: f64) -> C {
    let mut denominator = c(2. * x, 2.).inv();
    let mut increment = denominator * (0.25 - mu * mu);
    let mut fraction = increment;
    for n in 2..=MAXIT {
        let n = n as f64;
        let a = (n - 0.5) * (n - 0.5) - mu * mu;
        let b = c(2. * x, 2. * n);
        denominator = (b + denominator * a).inv();
        increment *= b * denominator - 1.;
        fraction += increment;
        if increment.norm() <= EPS * (1. + fraction.norm()) {
            break;
        }
    }
    c(-0.5 / x - fraction.im / x, 1. + fraction.re / x)
}

/// Temme (1975), section 3: minimal solution of eqs. (3.5)-(3.6),
/// normalized by sum(k_n)=(2x)^(-mu-1/2), eq. (3.13). Backward ratios
/// and a nested sum need constant storage and avoid large intermediate k_n.
/// Compare successive truncations to choose the depth (DLMF 3.10.12).
fn temme_k_scaled(mu: f64, x: f64) -> (f64, f64) {
    let root = (PI / 2.).sqrt() / x.sqrt();
    let mut previous = (0., 0.);
    let mut depth = 16;
    loop {
        let mut ratio = 0.;
        let mut sum = 1.;
        for n in (1..=depth).rev() {
            let n = n as f64;
            let a = ((n - 0.5) * (n - 0.5) - mu * mu) / (n * (n + 1.));
            let b = 2. * (n + x) / (n + 1.);
            ratio = a / (b - ratio);
            sum = 1. + ratio * sum;
        }
        let value = root / sum;
        let adjacent = value * (1. + (mu + 0.5 - ratio) / x);
        if ((value - previous.0).abs() <= 2. * f64::EPSILON * value
            && (adjacent - previous.1).abs() <= 2. * f64::EPSILON * adjacent)
            || depth == 1024
        {
            return (value, adjacent);
        }
        previous = (value, adjacent);
        depth *= 2;
    }
}

// ---------- Bessel functions of complex argument ----------

/// Leading factor (z/2)^nu / Gamma(nu+1), in logarithms so large orders work.
fn series_lead(nu: f64, half: C) -> C {
    if nu == 0. {
        return c(1., 0.);
    }
    if is_integer(nu) && nu < 0. {
        return c(0., 0.);
    }
    let (lg, sign) = libm::lgamma_r(nu + 1.);
    cexp(half.ln() * nu - lg) * f64::from(sign)
}

/// sum_k (s z^2/4)^k / (k! Gamma(nu+k+1)) (z/2)^nu: J for s = -1, I for s = +1
/// (DLMF 10.2.2, 10.25.2).
fn series_ji(nu: f64, z: C, s: f64) -> C {
    let half = z * 0.5;
    let q = half * half * s;
    let mut term = series_lead(nu, half);
    if term == c(0., 0.) && nu < 0. {
        // J_{-n} = (-1)^n J_n for integer n.
        let n = -nu;
        let sign = if s < 0. && (n as i64) % 2 == 1 {
            -1.
        } else {
            1.
        };
        return series_ji(n, z, s) * sign;
    }
    let mut sum = term;
    for k in 1..MAXIT {
        let fk = k as f64;
        term *= q / (fk * (nu + fk));
        sum += term;
        if term.norm() <= EPS * sum.norm() && fk * fk > q.norm() {
            break;
        }
    }
    sum
}

/// Harmonic number H_k.
fn harmonic(k: usize) -> f64 {
    (1..=k).map(|j| 1. / j as f64).sum()
}

/// Y_n (s = -1, DLMF 10.8.1) or K_n (s = +1, DLMF 10.31.1) for integer n >= 0.
fn series_integer_yk(n: usize, z: C, s: f64) -> C {
    let half = z * 0.5;
    let q = half * half;
    let log_half = half.ln();
    // Finite sum: sum_{k<n} (n-k-1)!/k! (-s q)^k.
    let mut finite = c(0., 0.);
    if n > 0 {
        let mut t = c((1..n).map(|j| j as f64).product::<f64>(), 0.);
        finite = t;
        for k in 1..n {
            t *= q * (-s) / (k as f64 * (n - k) as f64);
            finite += t;
        }
        finite *= cexp(-log_half * n as f64);
    }
    // Infinite sum: sum_k (psi(k+1) + psi(n+k+1)) (s q)^k / (k! (n+k)!).
    let mut u = c(1. / (1..=n).map(|j| j as f64).product::<f64>(), 0.);
    let mut hk = 0.;
    let mut hnk = harmonic(n);
    let mut infinite = u * (hk + hnk - 2. * EULER);
    for k in 1..MAXIT {
        let fk = k as f64;
        u *= q * s / (fk * (n as f64 + fk));
        hk += 1. / fk;
        hnk += 1. / (n as f64 + fk);
        let term = u * (hk + hnk - 2. * EULER);
        infinite += term;
        if term.norm() <= EPS * infinite.norm() && fk * fk > q.norm() {
            break;
        }
    }
    let lead = cexp(log_half * n as f64);
    let ji = series_ji(n as f64, z, s);
    if s < 0. {
        -finite / PI + log_half * ji * (2. / PI) - lead * infinite / PI
    } else {
        let sign = if n.is_multiple_of(2) { 1. } else { -1. };
        finite * 0.5 - log_half * ji * sign + lead * infinite * (0.5 * sign)
    }
}

/// J_nu(z), Y_nu(z) (s = -1) or I_nu(z), K_nu(z) (s = +1) by power series, nu >= 0.
fn series_pair(nu: f64, z: C, s: f64) -> (C, C) {
    let first = series_ji(nu, z, s);
    let second = if is_integer(nu) {
        series_integer_yk(nu as usize, z, s)
    } else {
        let minus = series_ji(-nu, z, s);
        if s < 0. {
            (first * cospi(nu) - minus) / sinpi(nu)
        } else {
            (minus - first) * (FRAC_PI_2 / sinpi(nu))
        }
    };
    (first, second)
}

fn asymptotic_sums(nu: f64, z: C) -> Vec<C> {
    let mu = 4. * nu * nu;
    let mut terms = vec![c(1., 0.)];
    let mut term = c(1., 0.);
    let inv = (z * 8.).inv();
    for k in 1..200 {
        let odd = (2 * k - 1) as f64;
        let next = term * inv * ((mu - odd * odd) / k as f64);
        if next.norm() >= term.norm() || next.norm() < 1e-18 {
            if next.norm() < term.norm() {
                terms.push(next);
            }
            break;
        }
        terms.push(next);
        term = next;
    }
    terms
}

/// Whether the complex argument is large enough for Hankel's expansions.
fn large(nu: f64, z: C) -> bool {
    z.norm() >= 17. + 0.5 * nu * nu
}

/// J_nu(z) e^{-|Im z|} and Y_nu(z) e^{-|Im z|} for nu >= 0 (principal branches).
fn jy_scaled(nu: f64, z: C) -> (C, C) {
    if z.im == 0. && z.re > 0. {
        let (j, y) = jy_real(nu, z.re);
        return (c(j, 0.), c(y, 0.));
    }
    if z.re < 0. {
        // DLMF 10.11.1/10.11.2 with m = +-1.
        let m = if z.im >= 0. { 1. } else { -1. };
        let (jw, yw) = jy_scaled(nu, -z);
        let j = cis_pi(m * nu) * jw;
        let y = cis_pi(-m * nu) * yw + c(0., 2. * m * cospi(nu)) * jw;
        return (j, y);
    }
    if z.re == 0. {
        // DLMF 10.27.6 and 10.27.8: J and Y on the imaginary axis from I and K.
        let y = z.im.abs();
        let (is, ks) = ik_real(nu, y);
        let j = cis_pi(nu / 2.) * is;
        let yv =
            cis_pi((nu + 1.) / 2.) * is - cis_pi(-nu / 2.) * (ks * libm::exp(-2. * y) * 2. / PI);
        return if z.im < 0. {
            (j.conj(), yv.conj())
        } else {
            (j, yv)
        };
    }
    if large(nu, z) {
        // DLMF 10.17.5/10.17.6.
        let terms = asymptotic_sums(nu, z);
        let (mut s1, mut s2) = (c(0., 0.), c(0., 0.));
        let mut ik = c(1., 0.);
        for t in &terms {
            s1 += ik * t;
            s2 += ik.conj() * t;
            ik *= c(0., 1.);
        }
        let root = (z * (PI / 2.)).inv().sqrt();
        let phase = nu / 2. + 0.25;
        let h1 = root * cis_pi(-phase) * s1 * cexp(c(-z.im - z.im.abs(), z.re));
        let h2 = root * cis_pi(phase) * s2 * cexp(c(z.im - z.im.abs(), -z.re));
        return ((h1 + h2) * 0.5, (h1 - h2) * c(0., -0.5));
    }
    let (j, y) = series_pair(nu, z, -1.);
    let scale = libm::exp(-z.im.abs());
    (j * scale, y * scale)
}

/// I_nu(z) e^{-|Re z|} and K_nu(z) e^{z} for nu >= 0 (principal branches).
fn ik_scaled(nu: f64, z: C) -> (C, C) {
    if z.im == 0. && z.re > 0. {
        let (i, k) = ik_real(nu, z.re);
        return (c(i, 0.), c(k, 0.));
    }
    if z.re < 0. {
        // DLMF 10.34.1/10.34.2 with m = +-1.
        let m = if z.im >= 0. { 1. } else { -1. };
        let w = -z;
        let (iw, kw) = ik_scaled(nu, w);
        let i = cis_pi(m * nu) * iw;
        let k = cis_pi(-m * nu) * kw * cexp(w * -2.) - c(0., m * PI) * iw * cexp(c(0., -w.im));
        return (i, k);
    }
    if z.re == 0. {
        // DLMF 10.27.6 and 10.27.8 on the imaginary axis.
        let y = z.im.abs();
        let (j, yv) = jy_real(nu, y);
        let i = cis_pi(nu / 2.) * c(j, 0.);
        let k = cis_pi(-(nu + 1.) / 2.) * c(j, -yv) * cexp(c(0., y)) * FRAC_PI_2;
        return if z.im < 0. {
            (i.conj(), k.conj())
        } else {
            (i, k)
        };
    }
    if large(nu, z) {
        // DLMF 10.40.2 and 10.40.5.
        let terms = asymptotic_sums(nu, z);
        let (mut sk, mut si) = (c(0., 0.), c(0., 0.));
        for (k, t) in terms.iter().enumerate() {
            sk += t;
            si += if k % 2 == 0 { *t } else { -t };
        }
        let k = (z * (2. / PI)).inv().sqrt() * sk;
        let root = (z * (2. * PI)).inv().sqrt();
        let side = if z.im >= 0. { 1. } else { -1. };
        let i = root * si * cexp(c(0., z.im))
            + root * sk * c(0., side) * cis_pi(side * nu) * cexp(c(-2. * z.re, -z.im));
        return (i, k);
    }
    let (i, k) = series_pair(nu, z, 1.);
    (i * libm::exp(-z.re.abs()), k * cexp(z))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    J,
    Y,
    I,
    K,
    H1,
    H2,
}

fn nan() -> C {
    c(f64::NAN, 0.)
}

/// Largest order the recurrences run for; beyond it the work per element
/// would be unbounded (the downward recurrence takes about `nu` steps).
const ORDER_LIMIT: f64 = 1e4;

/// Orders above `ORDER_LIMIT`. For |z| <= nu/2 the leading term
/// (z/2)^nu / Gamma(nu+1) (DLMF 10.7.3, 10.25.2) is far below the smallest
/// double (the exponent is under -3900), so J and I are 0 and Y and K overflow,
/// as the reference returns (`besselj(1e5,1) = 0`, `besselj(Inf,1) = 0`). Larger
/// arguments would need the uniform expansions and give NaN.
fn bessel_huge_order(kind: Kind, nu: f64, z: C) -> C {
    if z.norm() > 0.5 * nu.abs() {
        return nan();
    }
    let real = z.im == 0. && z.re > 0.;
    match kind {
        Kind::J | Kind::I => c(0., 0.),
        Kind::K if real => c(f64::INFINITY, 0.),
        Kind::Y if real && nu > 0. => c(f64::NEG_INFINITY, 0.),
        _ => nan(),
    }
}

/// One Bessel function value with MATLAB's scaling option.
fn bessel(kind: Kind, nu: f64, z: C, scaled: bool) -> C {
    if nu.is_nan() || z.re.is_nan() || z.im.is_nan() {
        return nan();
    }
    if !(z.re.is_finite() && z.im.is_finite()) {
        return match kind {
            Kind::I if z.im == 0. && z.re == f64::INFINITY => {
                c(if scaled { 0. } else { f64::INFINITY }, 0.)
            }
            Kind::K if z.im == 0. && z.re == f64::INFINITY => c(0., 0.),
            _ => nan(),
        };
    }
    let a = nu.abs();
    if z == c(0., 0.) {
        let at_zero = if nu == 0. {
            1.
        } else if nu > 0. || is_integer(nu) {
            0.
        } else {
            f64::INFINITY
        };
        return match kind {
            Kind::J | Kind::I => c(at_zero, 0.),
            Kind::Y => c(f64::NEG_INFINITY, 0.),
            Kind::K => c(f64::INFINITY, 0.),
            Kind::H1 => c(at_zero, f64::NEG_INFINITY),
            Kind::H2 => c(at_zero, f64::INFINITY),
        };
    }
    if a > ORDER_LIMIT {
        return bessel_huge_order(kind, nu, z);
    }
    match kind {
        Kind::I | Kind::K => {
            let (is, ks) = ik_scaled(a, z);
            if kind == Kind::K {
                return if scaled {
                    ks
                } else if z.im == 0. {
                    scale(ks, libm::exp(-z.re))
                } else {
                    ks * cexp(-z)
                };
            }
            // DLMF 10.27.2: I_{-nu} = I_nu + (2/pi) sin(nu pi) K_nu.
            let s = sinpi(a);
            let i = if nu < 0. && s != 0. {
                is + ks * cexp(-z - z.re.abs()) * (2. / PI * s)
            } else {
                is
            };
            if scaled {
                i
            } else {
                scale(i, libm::exp(z.re.abs()))
            }
        }
        _ => {
            let (js, ys) = jy_scaled(a, z);
            // DLMF 10.4.7/10.4.8 for negative orders.
            let (j, y) = if nu < 0. {
                let (cs, sn) = (cospi(a), sinpi(a));
                let j = if sn == 0. { js * cs } else { js * cs - ys * sn };
                let y = if sn == 0. { ys * cs } else { js * sn + ys * cs };
                (j, y)
            } else {
                (js, ys)
            };
            let up = z.im.abs();
            match kind {
                Kind::J if scaled => j,
                Kind::Y if scaled => y,
                Kind::J => scale(j, libm::exp(up)),
                Kind::Y => scale(y, libm::exp(up)),
                Kind::H1 => {
                    let h = j + c(0., 1.) * y;
                    if scaled {
                        h * cexp(c(up + z.im, -z.re))
                    } else {
                        h * libm::exp(up)
                    }
                }
                _ => {
                    let h = j - c(0., 1.) * y;
                    if scaled {
                        h * cexp(c(up - z.im, z.re))
                    } else {
                        h * libm::exp(up)
                    }
                }
            }
        }
    }
}

// ---------- Airy functions ----------

const AI0: f64 = 0.355_028_053_887_817_2;
const AIP0: f64 = 0.258_819_403_792_806_8;

/// Maclaurin series (DLMF 9.4.1-9.4.4): Ai, Ai', Bi, Bi'.
fn airy_series(z: C) -> [C; 4] {
    let z3 = z * z * z;
    let mut f = c(1., 0.);
    let mut g = z;
    let mut fp = z * z * 0.5;
    let mut gp = c(1., 0.);
    let (mut tf, mut tg, mut tfp, mut tgp) = (f, g, fp, gp);
    for k in 1..200 {
        let k3 = 3. * k as f64;
        tf *= z3 / (k3 * (k3 - 1.));
        tg *= z3 / ((k3 + 1.) * k3);
        f += tf;
        g += tg;
        if k > 1 {
            tfp *= z3 / ((k3 - 3.) * (k3 - 1.));
            fp += tfp;
        }
        tgp *= z3 / ((k3 - 2.) * k3);
        gp += tgp;
        if tf.norm() + tg.norm() + tfp.norm() + tgp.norm() < 1e-18 * (f.norm() + g.norm()) {
            break;
        }
    }
    let s3 = 3_f64.sqrt();
    [
        f * AI0 - g * AIP0,
        fp * AI0 - gp * AIP0,
        (f * AI0 + g * AIP0) * s3,
        (fp * AI0 + gp * AIP0) * s3,
    ]
}

/// Exact sum a + b = s + e.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// Exact product a * b = p + e.
fn two_prod(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    (p, a.fma(b, -p))
}

/// Ai, Ai', Bi, Bi' at -x for x >= 9.5 from the large-argument expansions
/// DLMF 9.7.9 and 9.7.10. The truncation error is below exp(-4 x^{3/2}/3), under
/// 1e-16 here; the phase zeta - pi/4 (zeta = 2/3 x^{3/2}, up to 1e3 or more) is
/// formed in double-double so the cosine and sine keep full precision.
fn airy_negative_asymptotic(k: usize, x: f64) -> f64 {
    const TWO_THIRDS_LO: f64 = 3.700_743_415_417_188e-17;
    const TWO_PI_HI: f64 = 2. * PI;
    const TWO_PI_LO: f64 = 2.449_293_598_294_706_4e-16;
    const QUARTER_PI_LO: f64 = 3.061_616_997_868_383e-17;
    let root = x.sqrt();
    let root_lo = root.fma(-root, x) / (2. * root);
    let (p, pe) = two_prod(x, root);
    let pe = pe + x * root_lo;
    let (z1, z2) = two_prod(p, 2. / 3.);
    let z2 = z2 + p * TWO_THIRDS_LO + pe * (2. / 3.);
    let zeta = z1 + z2;
    let n = (z1 / TWO_PI_HI).round();
    let (a, ae) = two_prod(n, TWO_PI_HI);
    let lo = z2 - ae - n * TWO_PI_LO - QUARTER_PI_LO;
    let (theta, theta_e) = two_sum(z1 - a, -FRAC_PI_4);
    let theta_lo = theta_e + lo;
    let (sin_t, cos_t) = (libm::sin(theta), libm::cos(theta));
    let sin_p = sin_t + theta_lo * cos_t;
    let cos_p = cos_t - theta_lo * sin_t;
    // u_j and v_j (DLMF 9.7.2, 9.7.3), summed to the smallest term.
    let (mut even_u, mut odd_u, mut even_v, mut odd_v) = (0., 0., 0., 0.);
    let mut u = 1.;
    let mut power = 1.;
    let mut previous = f64::INFINITY;
    for j in 0..60 {
        let jf = j as f64;
        if j > 0 {
            u *= (6. * jf - 5.) * (6. * jf - 3.) * (6. * jf - 1.) / ((2. * jf - 1.) * 216. * jf);
            power /= zeta;
        }
        let v = -(6. * jf + 1.) / (6. * jf - 1.) * u;
        let (tu, tv) = (u * power, v * power);
        if tu.abs() > previous {
            break;
        }
        previous = tu.abs();
        let sign = if (j / 2) % 2 == 0 { 1. } else { -1. };
        if j % 2 == 0 {
            even_u += sign * tu;
            even_v += sign * tv;
        } else {
            odd_u += sign * tu;
            odd_v += sign * tv;
        }
        if tu.abs() < 1e-18 {
            break;
        }
    }
    let amplitude = 1. / (PI.sqrt() * root.sqrt());
    let slope = root.sqrt() / PI.sqrt();
    match k {
        0 => amplitude * (cos_p * even_u + sin_p * odd_u),
        1 => slope * (sin_p * even_v - cos_p * odd_v),
        2 => amplitude * (-sin_p * even_u + cos_p * odd_u),
        _ => slope * (cos_p * even_v + sin_p * odd_v),
    }
}

/// airy(k, z, scale) through DLMF 9.6.1-9.6.9 away from the origin.
fn airy(k: usize, z: C, scaled: bool) -> C {
    if z.re.is_nan() || z.im.is_nan() {
        return nan();
    }
    let zeta = z * z.sqrt() * (2. / 3.);
    let factor = |kind_ai: bool| -> C {
        if !scaled {
            c(1., 0.)
        } else if kind_ai {
            cexp(zeta)
        } else {
            c(libm::exp(-zeta.re.abs()), 0.)
        }
    };
    if !(z.re.is_finite() && z.im.is_finite()) {
        return nan();
    }
    let use_series = z.norm() <= 1. || (z.im != 0. && z.norm() <= 2.);
    if use_series {
        return airy_series(z)[k] * factor(k < 2);
    }
    let s3 = 3_f64.sqrt();
    if z.re >= 0. {
        // Ai, Ai' from K_{1/3}, K_{2/3}; Bi, Bi' from I_{+-1/3}, I_{+-2/3}; the
        // Bessel scaling matches airy's.
        let value = match k {
            0 => (z / 3.).sqrt() * bessel(Kind::K, 1. / 3., zeta, true) / PI,
            1 => -z / (PI * s3) * bessel(Kind::K, 2. / 3., zeta, true),
            2 => {
                (z / 3.).sqrt()
                    * (bessel(Kind::I, -1. / 3., zeta, true) + bessel(Kind::I, 1. / 3., zeta, true))
            }
            _ => {
                z / s3
                    * (bessel(Kind::I, -2. / 3., zeta, true) + bessel(Kind::I, 2. / 3., zeta, true))
            }
        };
        return if scaled {
            value
        } else {
            value
                * if k < 2 {
                    cexp(-zeta)
                } else {
                    c(libm::exp(zeta.re.abs()), 0.)
                }
        };
    }
    if z.im == 0. && z.re <= -9.5 {
        return c(airy_negative_asymptotic(k, -z.re), 0.) * factor(k < 2);
    }
    // Negative real part: w = -z and DLMF 9.6.6-9.6.9 with J of zeta_w.
    let w = -z;
    let zw = w * w.sqrt() * (2. / 3.);
    let j = |nu: f64| bessel(Kind::J, nu, zw, false);
    let value = match k {
        0 => w.sqrt() / 3. * (j(1. / 3.) + j(-1. / 3.)),
        1 => w / 3. * (j(2. / 3.) - j(-2. / 3.)),
        2 => (w / 3.).sqrt() * (j(-1. / 3.) - j(1. / 3.)),
        _ => w / s3 * (j(-2. / 3.) + j(2. / 3.)),
    };
    value * factor(k < 2)
}

// ---------- incomplete gamma ----------

/// Stirling's correction ln Gamma*(a) = ln Gamma(a) - (a-1/2) ln a + a - ln sqrt(2 pi)
/// for a >= 10 (DLMF 5.11.1).
fn stirling_correction(a: f64) -> f64 {
    let a2 = a * a;
    (1. / 12. - (1. / 360. - (1. / 1260. - (1. / 1680. - 1. / (1188. * a2)) / a2) / a2) / a2) / a
}

/// x^a e^{-x} / Gamma(a+1) for x > 0, a > 0; Temme's form for large a (DLMF 8.12).
fn gamma_prefix(a: f64, x: f64) -> f64 {
    if a >= 20. {
        let r = (x - a) / a;
        // log1p(r) loses the digits of log(x/a) once x is small against a.
        let phi = if x < 0.5 * a {
            r - libm::log(x / a)
        } else {
            r - libm::log1p(r)
        };
        let correction = stirling_correction(a);
        return libm::exp(-a * phi - correction) / (2. * PI * a).sqrt();
    }
    let direct = x.powf(a) * libm::exp(-x) / libm::tgamma(a + 1.);
    if direct.is_finite() && direct > 1e-290 {
        direct
    } else {
        libm::exp(a * libm::log(x) - x - libm::lgamma(a + 1.))
    }
}

/// Modified Lentz algorithm (Thompson and Barnett, J. Comput. Phys. 64 (1986),
/// section 3) for b0 + a1/(b1 + a2/(b2 + ...)), `term(n) = (a_n, b_n)` for n >= 1.
/// Vanishing denominators are replaced by a tiny number; the loop stops once a
/// step changes the value by at most one rounding, and gives NaN if `limit`
/// terms do not get there.
fn lentz(b0: f64, limit: usize, term: impl Fn(f64) -> (f64, f64)) -> f64 {
    let mut f = if b0 == 0. { FPMIN } else { b0 };
    let (mut cc, mut d) = (f, 0.);
    for n in 1..limit {
        let (an, bn) = term(n as f64);
        d = bn + an * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        cc = bn + an / cc;
        if cc.abs() < FPMIN {
            cc = FPMIN;
        }
        d = 1. / d;
        let delta = cc * d;
        f *= delta;
        if (delta - 1.).abs() <= f64::EPSILON {
            return f;
        }
    }
    f64::NAN
}

/// sum_{k>=0} x^k / ((a+1)(a+2)...(a+k)) = Gamma(a+1) e^x x^-a P(a, x): the
/// first series of DLMF 8.7.1 with DLMF 8.2.7. Converges for every real x.
fn gamma_series(a: f64, x: f64) -> f64 {
    let (mut term, mut sum, mut k) = (1., 1., 1.);
    for _ in 0..MAXIT {
        term *= x / (a + k);
        sum += term;
        if term.abs() <= 0.5 * f64::EPSILON * sum.abs() {
            break;
        }
        k += 1.;
    }
    sum
}

/// x^-a e^x Gamma(a, x) for x > 0 by Legendre's continued fraction, the even
/// part of DLMF 8.9.2: 1/(x+1-a - 1(1-a)/(x+3-a - 2(2-a)/(x+5-a - ...))).
fn gamma_fraction(a: f64, x: f64) -> f64 {
    1. / lentz(x + 1. - a, MAXIT, |n| (-n * (n - a), x + 2. * n + 1. - a))
}

/// Largest integer A whose upper tail is the finite sum of DLMF 8.4.10.
const GAMMA_FINITE_SUM: f64 = 30.;

/// e_{n-1}(x) = sum_{k<n} x^k/k! and n!/x^n for integer n >= 1: Q(n, x) =
/// e^-x e_{n-1}(x) (DLMF 8.4.10), so the scaled upper tail is their product.
fn gamma_finite_sum(n: f64, x: f64) -> (f64, f64) {
    let (mut term, mut sum, mut scale) = (1., 1., 1. / x);
    let mut k = 1.;
    while k < n {
        term *= x / k;
        sum += term;
        k += 1.;
        scale *= k / x;
    }
    (sum, scale)
}

/// 1/Gamma(1+a) - 1 for 0 <= a < 1, by the Taylor series of 1/Gamma
/// (Abramowitz and Stegun 6.1.34) below 1/2 where the difference cancels.
fn reciprocal_gamma_minus_one(a: f64) -> f64 {
    if a < 0.5 {
        a * RECIPROCAL_GAMMA[1..]
            .iter()
            .rev()
            .fold(0., |acc, &k| acc * a + k)
    } else {
        1. / libm::tgamma(1. + a) - 1.
    }
}

/// Q(a, x) for 0 < a < 1, 0 < x <= 1.5 from the second series of DLMF 8.7.1:
/// Q = 1 - x^a/Gamma(a+1) - (a x^a/Gamma(a+1)) sum_{k>=1} (-x)^k/(k!(a+k)),
/// with 1 - x^a/Gamma(a+1) formed from 1/Gamma(1+a) - 1 and expm1(a ln x) so a
/// small Q keeps its relative accuracy (the small-a treatment of DiDonato and
/// Morris, ACM TOMS 12 (1986)).
fn gamma_upper_small_a(a: f64, x: f64) -> f64 {
    let rm1 = reciprocal_gamma_minus_one(a);
    let r = 1. + rm1;
    let lx = a * libm::log(x);
    let head = -rm1 - r * libm::expm1(lx);
    let (mut term, mut sum) = (1., 0.);
    for k in 1..MAXIT {
        let k = k as f64;
        term *= -x / k;
        let t = term / (a + k);
        sum += t;
        if t.abs() <= 0.25 * f64::EPSILON * sum.abs() {
            break;
        }
    }
    head - a * r * libm::exp(lx) * sum
}

#[derive(Clone, Copy, PartialEq)]
enum Tail {
    Lower,
    Upper,
    ScaledLower,
    ScaledUpper,
}

/// x^a e^-x / Gamma(a+1) from its three factors while each is a normal number
/// (each is then correct to about one rounding, also far from x = a, where the
/// exponential of Temme's form magnifies the error of its argument), else
/// `gamma_prefix`.
fn power_prefix(a: f64, x: f64) -> f64 {
    if a <= 170. && x < 700. {
        let (power, gamma) = (libm::pow(x, a), libm::tgamma(a + 1.));
        let direct = power * libm::exp(-x) / gamma;
        if power.is_normal() && gamma.is_finite() && direct.is_normal() {
            return direct;
        }
    }
    gamma_prefix(a, x)
}

/// Smallest a for Temme's uniform expansion near the transition x = a.
const GAMMA_UNIFORM: f64 = 1e5;

/// (a eta^2/2, t, w) of Temme's uniform expansion for large a and |x/a - 1| <= 0.3
/// (DLMF 8.12.3, 8.12.4, 8.12.7): with t = eta sqrt(a/2), so t^2 = a eta^2/2,
/// Q = erfc(t)/2 + R, P = erfc(-t)/2 - R, R = e^(-t^2) w and
/// w = (c0 + c1/a)/sqrt(2 pi a), with c0, c1 from DLMF 8.12.8 or, near eta = 0,
/// their Taylor series. The neglected c2/a^2 term is below 1e-12 of R for a >= 1e5.
fn uniform_terms(a: f64, x: f64) -> (f64, f64, f64) {
    // d = lambda - 1 and eta^2/2 = d - ln(1+d) (DLMF 8.12.1), the latter by its
    // series for small d, where the difference cancels.
    let d = (x - a) / a;
    let half = if d.abs() < 0.1 {
        let (mut power, mut sum) = (d * d, 0.);
        for k in 2..40 {
            let t = power / k as f64;
            sum += t;
            if t.abs() <= 0.25 * f64::EPSILON * sum {
                break;
            }
            power *= -d;
        }
        sum
    } else {
        d - libm::log1p(d)
    };
    let eta = (2. * half).sqrt().copysign(d);
    let (c0, c1) = if d.abs() < 1e-3 {
        (
            -1. / 3. + eta * (1. / 12. + eta * (-2. / 135. + eta * (1. / 864. + eta / 2835.))),
            -1. / 540. + eta * (-1. / 288. + eta / 378.),
        )
    } else {
        (
            1. / d - 1. / eta,
            1. / (eta * eta * eta) - 1. / (d * d * d) - 1. / (d * d) - 1. / (12. * d),
        )
    };
    (
        a * half,
        eta * (0.5 * a).sqrt(),
        (c0 + c1 / a) / (2. * PI * a).sqrt(),
    )
}

/// (P(a, x), Q(a, x)) from `uniform_terms`.
fn gamma_uniform(a: f64, x: f64) -> (f64, f64) {
    let (exponent, t, w) = uniform_terms(a, x);
    let r = libm::exp(-exponent) * w;
    (0.5 * libm::erfc(-t) - r, 0.5 * libm::erfc(t) + r)
}

/// The two tails of `gamma_uniform` divided by x^a e^-x / Gamma(a+1) =
/// e^(-a eta^2/2 - c)/sqrt(2 pi a) (the `gamma_prefix` form, c Stirling's
/// correction). The tail that is small (Q for x > a, P for x < a) is
/// e^(-t^2) (erfcx(|t|)/2 +- w) with erfcx(t) = e^(t^2) erfc(t), so the
/// exponential cancels against the prefix before it can underflow: the scaled
/// tails stay finite where the tails themselves are 0. The other tail is about 1
/// and is divided by the prefix as it is, which overflows only when the result does.
fn gamma_uniform_scaled(a: f64, x: f64) -> (f64, f64) {
    let (exponent, t, w) = uniform_terms(a, x);
    let k = (2. * PI * a).sqrt() * libm::exp(stirling_correction(a));
    if t >= 0. {
        let p = 0.5 * libm::erfc(-t) - libm::exp(-exponent) * w;
        (p * libm::exp(exponent) * k, (0.5 * erfcx_value(t) + w) * k)
    } else {
        let q = 0.5 * libm::erfc(t) + libm::exp(-exponent) * w;
        ((0.5 * erfcx_value(-t) - w) * k, q * libm::exp(exponent) * k)
    }
}

/// One tail of P(a, x) for x >= 0, a >= 0 (DLMF 8.2.4). The region choice follows
/// DiDonato and Morris (ACM TOMS 12 (1986)): Temme's uniform expansion for large
/// a near x = a, else the series of DLMF 8.7.1 below x = a + 1 (x = 1.5 for
/// a < 1) and Legendre's continued fraction above it, the finite sum DLMF 8.4.10
/// for the upper tail of small integer a, and for a < 1 the upper tail without
/// the cancellation in 1 - P.
fn gammainc_real(x: f64, a: f64, tail: Tail) -> f64 {
    use Tail::{Lower, ScaledLower, ScaledUpper, Upper};
    if x.is_nan() || a.is_nan() {
        return f64::NAN;
    }
    // The limits a -> 0+ (DLMF 8.2.4, 8.2.7; `gammainc_negative` agrees for x < 0):
    // P(a, x) -> 1 and Q(a, x) -> 0, since Gamma(a) grows while Gamma(a, x) stays
    // finite; the scaled lower tail Gamma(a+1) e^x x^-a P(a, x) -> e^x; the scaled
    // upper tail a Gamma(a, x) e^x x^-a -> 0.
    if a == 0. {
        return match tail {
            Lower => 1.,
            Upper | ScaledUpper => 0.,
            ScaledLower => libm::exp(x),
        };
    }
    if x == 0. {
        return match tail {
            Lower => 0.,
            Upper | ScaledLower => 1.,
            ScaledUpper => f64::INFINITY,
        };
    }
    if x.is_infinite() {
        return match tail {
            Lower => 1.,
            Upper | ScaledUpper => 0.,
            ScaledLower => f64::INFINITY,
        };
    }
    if a.is_infinite() {
        return match tail {
            Lower => 0.,
            Upper | ScaledLower => 1.,
            ScaledUpper => f64::INFINITY,
        };
    }
    // x^a e^-x / Gamma(a+1): the scaled tails are the tails divided by it.
    let prefix = power_prefix(a, x);
    let unscale = |t: f64| {
        if prefix > 1e-290 && prefix.is_finite() {
            t / prefix
        } else {
            libm::exp(libm::log(t) + libm::lgamma(a + 1.) + x - a * libm::log(x))
        }
    };
    if a >= GAMMA_UNIFORM && (x - a).abs() <= 0.3 * a {
        return match tail {
            Lower => gamma_uniform(a, x).0,
            Upper => gamma_uniform(a, x).1,
            ScaledLower => gamma_uniform_scaled(a, x).0,
            ScaledUpper => gamma_uniform_scaled(a, x).1,
        };
    }
    let finite = a <= GAMMA_FINITE_SUM && a == a.trunc() && x < 700.;
    let upper_sum = || {
        let (sum, scale) = gamma_finite_sum(a, x);
        (libm::exp(-x) * sum, sum * scale)
    };
    let series = if a < 1. { x <= 1.5 } else { x < a + 1. };
    if series {
        let s = gamma_series(a, x);
        let p = prefix * s;
        let upper = || {
            if finite {
                upper_sum()
            } else {
                let q = if a < 1. {
                    gamma_upper_small_a(a, x)
                } else {
                    1. - p
                };
                (q, unscale(q))
            }
        };
        return match tail {
            Lower => p,
            ScaledLower => s,
            Upper => upper().0,
            ScaledUpper => upper().1,
        };
    }
    let (q, scaled) = if finite {
        upper_sum()
    } else {
        let f = gamma_fraction(a, x);
        (prefix * a * f, a * f)
    };
    match tail {
        Lower => 1. - q,
        ScaledLower => unscale(1. - q),
        Upper => q,
        ScaledUpper => scaled,
    }
}

/// Negative x: x^a through the complex logarithm, as the reference returns a complex value.
fn gammainc_negative(x: f64, a: f64, tail: Tail) -> C {
    // The limits a -> 0+ as in `gammainc_real`.
    if a == 0. {
        return c(
            match tail {
                Tail::Lower => 1.,
                Tail::Upper | Tail::ScaledUpper => 0.,
                Tail::ScaledLower => libm::exp(x),
            },
            0.,
        );
    }
    let power = cexp(c(libm::log(-x), PI) * a);
    let s = gamma_series(a, x) / libm::tgamma(a + 1.);
    let p = power * libm::exp(-x) * s;
    match tail {
        Tail::Lower => p,
        Tail::Upper => c(1., 0.) - p,
        Tail::ScaledLower => c(gamma_series(a, x), 0.),
        Tail::ScaledUpper => (c(1., 0.) - p) / (power * libm::exp(-x) / libm::tgamma(a + 1.)),
    }
}

/// t with upper normal tail probability q, given p = 1 - q as well: the rational
/// approximation Abramowitz and Stegun 26.2.23 (error below 4.5e-4), a start only.
fn normal_upper_quantile(p: f64, q: f64) -> f64 {
    let tail = |r: f64| {
        let t = (-2. * libm::log(r)).sqrt();
        t - (2.515_517 + t * (0.802_853 + t * 0.010_328))
            / (1. + t * (1.432_788 + t * (0.189_269 + t * 0.001_308)))
    };
    if q <= 0.5 { tail(q) } else { -tail(p) }
}

/// lambda > 0 with lambda - 1 - ln lambda = eta^2/2 and sign(lambda - 1) = sign(eta),
/// the variable of Temme's uniform expansion (DLMF 8.12.1), by Newton's method.
fn temme_lambda(eta: f64) -> f64 {
    let s = 0.5 * eta * eta;
    if eta.abs() < 1e-3 {
        // the start of the inverse series, enough for a starting value
        return 1. + eta + eta * eta / 3.;
    }
    if eta > 0. {
        let mut l = 1. + eta + eta * eta / 3.;
        for _ in 0..50 {
            let step = (l - 1. - libm::log(l) - s) / (1. - 1. / l);
            l -= step;
            if step.is_nan() || step.abs() <= 1e-15 * l {
                break;
            }
        }
        l
    } else {
        // In m = ln lambda: e^m - 1 - m = s, increasing from e^m > 0 below the root.
        let mut m = if s > 1. { -1. - s } else { eta };
        for _ in 0..50 {
            let step = (libm::exp(m) - 1. - m - s) / libm::expm1(m);
            m -= step;
            if step.is_nan() || step.abs() <= 1e-15 * m.abs() {
                break;
            }
        }
        libm::exp(m)
    }
}

/// x in (lo, hi) with tail(x) = target for a monotone tail; `f(x)` returns the
/// tail and its derivative, `curvature(x)` tail''/tail', `rising` the direction.
/// Halley's method runs on g = ln(tail/target): far out the tails decay about
/// exponentially, where steps on g stay long while steps on the tail itself
/// shrink to a constant size. A step that leaves the bracket bisects it instead
/// (geometrically where it spans orders of magnitude). A NaN tail (a continued
/// fraction that did not converge) is not convergence and says nothing about the
/// side of the root: the root lies on the side of the smaller tail, so `x`
/// retreats by half the distance to that end of the bracket, and NaN is returned
/// when 20 evaluations find no value, rather than a root that was never checked.
fn tail_root(
    mut x: f64,
    (mut lo, mut hi): (f64, f64),
    rising: bool,
    target: f64,
    f: impl Fn(f64) -> (f64, f64),
    curvature: impl Fn(f64) -> f64,
) -> f64 {
    let (mut unusable, mut last_usable) = (0, true);
    for _ in 0..400 {
        let (value, slope) = f(x);
        last_usable = !value.is_nan();
        if !last_usable {
            unusable += 1;
            if unusable > 20 {
                return f64::NAN;
            }
            let end = if rising { lo } else { hi };
            x = if end.is_infinite() {
                2. * x
            } else {
                0.5 * (x + end)
            };
            continue;
        }
        if value == target {
            return x;
        }
        if (value > target) == rising {
            hi = x;
        } else {
            lo = x;
        }
        let ratio = value / target;
        let g = if ratio.is_normal() {
            libm::log(ratio)
        } else {
            libm::log(value) - libm::log(target)
        };
        let dg = slope / value;
        let newton = g / dg;
        let denominator = 1. - 0.5 * newton * (curvature(x) - dg);
        let mut next = if denominator > 0.5 {
            x - newton / denominator
        } else {
            x - newton
        };
        if (next - x).abs() <= 2. * f64::EPSILON * x {
            return next;
        }
        if next.is_nan() || next <= lo || next >= hi {
            next = if hi.is_infinite() {
                4. * x
            } else if lo == 0. {
                hi / 16.
            } else if hi > 4. * lo {
                (lo * hi).sqrt()
            } else {
                0.5 * (lo + hi)
            };
        }
        if hi - lo <= 2. * f64::EPSILON * x {
            return next;
        }
        x = next;
    }
    if last_usable { x } else { f64::NAN }
}

/// x with gammainc(x, a, tail) = y. The start inverts the leading term of the
/// series for small x, otherwise the leading term of Temme's expansion
/// Q(a, x) ~ erfc(eta sqrt(a/2))/2 (DLMF 8.12.3, 8.12.1); `tail_root` finishes.
fn gammaincinv_real(y: f64, a: f64, upper: bool) -> f64 {
    if y.is_nan() || a.is_nan() {
        return f64::NAN;
    }
    let (p, q) = if upper { (1. - y, y) } else { (y, 1. - y) };
    if p == 0. {
        return 0.;
    }
    if q == 0. || a.is_infinite() {
        return f64::INFINITY;
    }
    // P(0, x) = 1 for every x >= 0: the limit a -> 0+ of the solution is 0.
    if a == 0. {
        return 0.;
    }
    let small = libm::exp((libm::log(p) + libm::lgamma(a + 1.)) / a);
    let start = if small < 0.2 * (a + 1.) {
        small
    } else {
        a * temme_lambda(normal_upper_quantile(p, q) / a.sqrt())
    };
    if start == 0. {
        // x^a/Gamma(a+1) ~ p only below the smallest double.
        return 0.;
    }
    // Iterate on the smaller tail (1 - y is exact for y >= 1/2).
    let (lower, target) = if p <= q { (true, p) } else { (false, q) };
    let tail = if lower { Tail::Lower } else { Tail::Upper };
    tail_root(
        start,
        (0., f64::INFINITY),
        lower,
        target,
        |x| {
            // d/dx P(a, x) = x^(a-1) e^-x / Gamma(a)
            let density = power_prefix(a, x) * a / x;
            let value = gammainc_real(x, a, tail);
            (value, if lower { density } else { -density })
        },
        |x| (a - 1.) / x - 1.,
    )
}

// ---------- incomplete beta ----------

/// x^a (1-x)^b / B(a, b) for 0 < x < 1.
fn beta_prefix(x: f64, a: f64, b: f64) -> f64 {
    if a >= 10. && b >= 10. {
        // Stirling forms of the three gamma functions; the powers are taken of
        // ratios near 1 so large parameters keep full precision.
        let s = a + b;
        let la = a * libm::log1p((x * s - a) / a);
        let lb = b * libm::log1p(((1. - x) * s - b) / b);
        let corr = stirling_correction(s) - stirling_correction(a) - stirling_correction(b);
        return libm::exp(la + lb + corr) * (a * b / (2. * PI * s)).sqrt();
    }
    if a + b < 150. {
        let direct = libm::tgamma(a + b) / (libm::tgamma(a) * libm::tgamma(b))
            * x.powf(a)
            * (1. - x).powf(b);
        if direct.is_finite() && direct > 1e-290 {
            return direct;
        }
    }
    libm::exp(
        libm::lgamma(a + b) - libm::lgamma(a) - libm::lgamma(b)
            + a * libm::log(x)
            + b * libm::log1p(-x),
    )
}

/// Terms allowed to the continued fraction of `beta_fraction`. At x = a/(a+b),
/// where it is slowest, a = b needs about 1000 terms for 1e6, 20000 for 1e10 and
/// 410000 for 1e14 (the count grows like a^0.3); 2e6 reaches a = b = 1e16, past
/// which the fraction gives NaN. Away from that point it needs a few dozen.
const BETA_MAXIT: usize = 2_000_000;

/// 1/(1 + d1/(1 + d2/(1 + ...))) of DLMF 8.17.22, so that
/// I_x(a, b) = x^a (1-x)^b / (a B(a, b)) times it; with d_{2m} =
/// m(b-m)x/((a+2m-1)(a+2m)) and d_{2m+1} = -(a+m)(a+b+m)x/((a+2m)(a+2m+1)).
/// Converges quickly for x < (a+1)/(a+b+2).
fn beta_fraction(a: f64, b: f64, x: f64) -> f64 {
    1. / lentz(1., BETA_MAXIT, |n| {
        let m = (n / 2.).floor();
        let d = if n % 2. == 0. {
            m * (b - m) * x / ((a + 2. * m - 1.) * (a + 2. * m))
        } else {
            -(a + m) * (a + b + m) * x / ((a + 2. * m) * (a + 2. * m + 1.))
        };
        (d, 1.)
    })
}

/// (I_x(a, b), 1 - I_x(a, b)) for x in [0, 1], a, b >= 0. The continued fraction
/// DLMF 8.17.22 gives whichever tail lies on its fast side, the symmetry
/// I_x(a, b) = 1 - I_{1-x}(b, a) (DLMF 8.17.4) the other; each tail is computed
/// directly where it is the small one.
fn betainc_pair(x: f64, a: f64, b: f64) -> (f64, f64) {
    if x.is_nan() || a.is_nan() || b.is_nan() {
        return (f64::NAN, f64::NAN);
    }
    if x == 0. {
        return (0., 1.);
    }
    if x == 1. {
        return (1., 0.);
    }
    // Limits with all the mass at 0 (a -> 0 or b -> Inf) or at 1.
    let at_zero = a == 0. || b.is_infinite();
    let at_one = b == 0. || a.is_infinite();
    if at_zero && at_one {
        return (f64::NAN, f64::NAN);
    }
    if at_zero {
        return (1., 0.);
    }
    if at_one {
        return (0., 1.);
    }
    if x <= (a + 1.) / (a + b + 2.) {
        let lower = beta_prefix(x, a, b) / a * beta_fraction(a, b, x);
        (lower, 1. - lower)
    } else {
        let upper = beta_prefix(1. - x, b, a) / b * beta_fraction(b, a, 1. - x);
        (1. - upper, upper)
    }
}

/// x with betainc(x, a, b, tail) = y. The start inverts the leading term of
/// DLMF 8.17.22 near either end, x^a/(a B(a, b)) or (1-x)^b/(b B(a, b)), or else
/// takes the normal approximation Abramowitz and Stegun 26.5.22; `tail_root`
/// finishes.
fn betaincinv_real(y: f64, a: f64, b: f64, upper: bool) -> f64 {
    if y.is_nan() || a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let (p, q) = if upper { (1. - y, y) } else { (y, 1. - y) };
    if p == 0. {
        return 0.;
    }
    if q == 0. {
        return 1.;
    }
    let at_zero = a == 0. || b.is_infinite();
    let at_one = b == 0. || a.is_infinite();
    if at_zero && at_one {
        return f64::NAN;
    }
    if at_zero {
        return 0.;
    }
    if at_one {
        return 1.;
    }
    let ln_beta = libm::lgamma(a) + libm::lgamma(b) - libm::lgamma(a + b);
    let mean = a / (a + b);
    let near_zero = libm::exp((libm::log(p * a) + ln_beta) / a);
    let near_one = libm::exp((libm::log(q * b) + ln_beta) / b);
    let start = if near_zero < 0.5 * mean {
        if near_zero == 0. {
            // x^a/(a B(a, b)) ~ p only below the smallest double.
            return 0.;
        }
        near_zero
    } else if near_one < 0.5 * (1. - mean) {
        (1. - near_one).min(1. - 0.5 * f64::EPSILON)
    } else if a > 1. && b > 1. {
        let t = normal_upper_quantile(q, p);
        let lambda = (t * t - 3.) / 6.;
        let (ra, rb) = (1. / (2. * a - 1.), 1. / (2. * b - 1.));
        let h = 2. / (ra + rb);
        let w = t * (h + lambda).sqrt() / h - (rb - ra) * (lambda + 5. / 6. - 2. / (3. * h));
        a / (a + b * libm::exp(2. * w))
    } else {
        mean
    };
    let start = if start > 0. && start < 1. {
        start
    } else {
        mean
    };
    // Iterate on the smaller tail (1 - y is exact for y >= 1/2).
    let (lower, target) = if p <= q { (true, p) } else { (false, q) };
    tail_root(
        start,
        (0., 1.),
        lower,
        target,
        |x| {
            let (low, up) = betainc_pair(x, a, b);
            // d/dx I_x(a, b) = x^(a-1) (1-x)^(b-1) / B(a, b)
            let density = beta_prefix(x, a, b) / (x * (1. - x));
            if lower {
                (low, density)
            } else {
                (up, -density)
            }
        },
        |x| (a - 1.) / x - (b - 1.) / (1. - x),
    )
}

// ---------- expint, psi, erfcx ----------

/// Sum of `terms` with Neumaier's compensation (the error of each addition is
/// carried separately), for series whose partial sums cancel.
fn compensated_sum(terms: impl IntoIterator<Item = f64>) -> f64 {
    let (mut sum, mut carry) = (0., 0.);
    for t in terms {
        let s = sum + t;
        carry += if sum.abs() >= t.abs() {
            (sum - s) + t
        } else {
            (t - s) + sum
        };
        sum = s;
    }
    sum + carry
}

/// |z| beyond which the series for E1 start from `series_scale` instead of 1: the
/// terms of sum z^k/(k k!) peak near e^|z|/sqrt(2 pi |z|), which would overflow
/// a little before the sum itself does.
const SERIES_BIG: f64 = 500.;

/// 2^-1000 for |z| > `SERIES_BIG`, else 1: an exact power of two, so scaling a
/// series by it and dividing the sum by it again changes no digit.
fn series_scale(size: f64) -> f64 {
    if size > SERIES_BIG {
        libm::ldexp(1., -1000)
    } else {
        1.
    }
}

/// scale * sum_{k>=1} x^k/(k k!) for real x until the terms stop mattering
/// (DLMF 6.6.1 and 6.6.2 differ only in the sign of x).
fn exponential_series(x: f64, scale: f64) -> Vec<f64> {
    let mut terms = Vec::new();
    let (mut power, mut total) = (scale, 0f64);
    for k in 1..MAXIT {
        let k = k as f64;
        power *= x / k;
        let t = power / k;
        terms.push(t);
        total = total.max(t.abs());
        if t.abs() <= 0.25 * f64::EPSILON * total && k > x.abs() {
            break;
        }
    }
    terms
}

/// Principal ln z from the libm crate, so every platform rounds alike.
fn complex_log(z: C) -> C {
    c(libm::log(libm::hypot(z.re, z.im)), libm::atan2(z.im, z.re))
}

/// E1(z) for complex z (|arg z| < pi, z on the negative axis taken from above):
/// the series DLMF 6.6.2, E1(z) = -gamma - ln z - sum_{k>=1} (-z)^k/(k k!),
/// where its terms do not cancel much, else the continued fraction DLMF 6.9.1 in
/// its even form e^-z/(z+1 - 1/(z+3 - 4/(z+5 - ...))), evaluated by Lentz's method.
/// Real z uses real arithmetic: E1(x) = e^-x times `gamma_fraction(0, x)` for
/// x > 1, and E1(-x) = -Ei(x) - i pi (DLMF 6.2.5, 6.2.6) with Ei from DLMF 6.6.1.
fn expint_value(z: C) -> C {
    // As the reference returns: expint(NaN) = 0, expint(Inf) = NaN.
    if z.re.is_nan() || z.im.is_nan() {
        return c(0., 0.);
    }
    if z.re.is_infinite() || z.im.is_infinite() {
        return nan();
    }
    if z.im == 0. {
        let x = z.re;
        if x == 0. {
            return c(f64::INFINITY, 0.);
        }
        if x > 1. {
            return c(libm::exp(-x) * gamma_fraction(0., x), 0.);
        }
        if x > 0. {
            let terms = [-EULER, -libm::log(x)]
                .into_iter()
                .chain(exponential_series(-x, 1.).into_iter().map(|t| -t));
            return c(compensated_sum(terms), 0.);
        }
        let scale = series_scale(-x);
        let terms = [EULER, libm::log(-x)]
            .map(|t| scale * t)
            .into_iter()
            .chain(exponential_series(-x, scale));
        return c(-compensated_sum(terms) / scale, -PI);
    }
    let r = z.norm();
    if r <= 1. || r + z.re <= 3. {
        let w = -z;
        let scale = series_scale(r);
        let (mut power, mut sum, mut total) = (c(scale, 0.), c(0., 0.), 0f64);
        for k in 1..MAXIT {
            let k = k as f64;
            // w/k first: power * w alone overflows before the division by k.
            power *= w / k;
            let t = power / k;
            sum += t;
            total = total.max(t.norm());
            if t.norm() <= 0.25 * f64::EPSILON * total && k > r {
                break;
            }
        }
        return c(-EULER, 0.) - complex_log(z) - sum / scale;
    }
    // Lentz's method for G = z+1 + K(-n^2 / (z+2n+1)); E1 = e^-z / G.
    let tiny = c(FPMIN, 0.);
    let mut g = z + 1.;
    let (mut cc, mut d) = (g, c(0., 0.));
    for n in 1..MAXIT {
        let n = n as f64;
        let (an, bn) = (-n * n, z + (2. * n + 1.));
        d = bn + d * an;
        if d.norm() < FPMIN {
            d = tiny;
        }
        cc = bn + c(an, 0.) / cc;
        if cc.norm() < FPMIN {
            cc = tiny;
        }
        d = c(1., 0.) / d;
        let delta = cc * d;
        g *= delta;
        if (delta - 1.).norm() <= f64::EPSILON {
            break;
        }
    }
    if z.re < -700. {
        // e^-z alone would overflow before the division.
        return cexp(-z - complex_log(g));
    }
    cexp(-z) / g
}

/// Bernoulli numbers B_2 .. B_16.
const BERNOULLI: [f64; 8] = [
    1. / 6.,
    -1. / 30.,
    1. / 42.,
    -1. / 30.,
    5. / 66.,
    -691. / 2730.,
    7. / 6.,
    -3617. / 510.,
];

/// psi^(n)(x) for x >= 0: recurrence up to x >= 15 + n, then the asymptotic
/// series (DLMF 5.11.2 and 5.15.8).
fn polygamma(n: usize, x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == 0. {
        return if n.is_multiple_of(2) {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if x == f64::INFINITY {
        return if n == 0 { f64::INFINITY } else { 0. };
    }
    let limit = 15. + n as f64;
    let mut shift = 0.;
    let mut x = x;
    if n == 0 {
        while x < limit {
            shift -= 1. / x;
            x += 1.;
        }
        let x2 = x * x;
        let mut tail = 0.;
        let mut power = x2;
        for (k, b) in BERNOULLI.iter().enumerate() {
            tail += b / (2. * (k + 1) as f64 * power);
            power *= x2;
        }
        return shift + libm::log(x) - 0.5 / x - tail;
    }
    let np1 = (n + 1) as i32;
    while x < limit {
        shift += x.powi(-np1);
        x += 1.;
    }
    let factorial = |m: usize| (1..=m).map(|j| j as f64).product::<f64>();
    let mut sum = factorial(n - 1) / x.powi(n as i32) + factorial(n) / (2. * x.powi(np1));
    for (k, b) in BERNOULLI.iter().enumerate() {
        let two_k = 2 * (k + 1);
        // B_2k (2k+n-1)! / ((2k)! x^(2k+n))
        let ratio: f64 = ((two_k + 1)..=(two_k + n - 1)).map(|j| j as f64).product();
        sum += b * ratio / x.powi((two_k + n) as i32);
    }
    let sign = if n % 2 == 1 { 1. } else { -1. };
    let value = sign * (factorial(n) * shift + sum);
    // The reference returns 0 where n! and x^n overflow together (`psi(171, 1000)`);
    // the quotient of the two infinities is NaN here, so match it.
    if value.is_nan() && x > 1. { 0. } else { value }
}

/// e^{x^2} erfc(x): the product for x < 2, the continued fraction
/// erfc x = e^{-x^2}/sqrt(pi) / (x + (1/2)/(x + 1/(x + (3/2)/(x + ...))))
/// (DLMF 7.9.2, Lentz) for larger x, and 2 e^{x^2} - erfcx(-x) for x < 0.
fn erfcx_value(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x < 0. {
        if x < -26.7 {
            return f64::INFINITY;
        }
        return 2. * libm::exp(x * x) - erfcx_value(-x);
    }
    if x == f64::INFINITY {
        return 0.;
    }
    if x < 2. {
        return libm::exp(x * x) * libm::erfc(x);
    }
    let mut f = x;
    let mut cc = x;
    let mut d = 0.;
    for k in 1..MAXIT {
        let a = k as f64 / 2.;
        d = x + a * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        cc = x + a / cc;
        if cc.abs() < FPMIN {
            cc = FPMIN;
        }
        d = 1. / d;
        let del = cc * d;
        f *= del;
        if (del - 1.).abs() < EPS {
            break;
        }
    }
    1. / (PI.sqrt() * f)
}

// ---------- legendre ----------

#[derive(Clone, Copy, PartialEq)]
enum Normalization {
    Unnormalized,
    Schmidt,
    Full,
}

/// P_n^m(x) for m = 0..n with the Condon-Shortley phase, as legendre returns them.
fn legendre_column(n: usize, x: f64, norm: Normalization, out: &mut Vec<f64>) {
    let s = (1. - x * x).max(0.).sqrt();
    if norm == Normalization::Unnormalized {
        // P_m^m = (-1)^m (2m-1)!! s^m, P_{m+1}^m = x (2m+1) P_m^m, then upward in l
        // (DLMF 14.10.3 / 14.7.15).
        let mut pmm = 1.;
        for m in 0..=n {
            if m > 0 {
                pmm *= -((2 * m - 1) as f64) * s;
            }
            let mut prev = pmm;
            if m == n {
                out.push(pmm + 0.);
                continue;
            }
            let mut cur = x * (2 * m + 1) as f64 * pmm;
            for l in (m + 2)..=n {
                let next =
                    ((2 * l - 1) as f64 * x * cur - (l + m - 1) as f64 * prev) / (l - m) as f64;
                prev = cur;
                cur = next;
            }
            out.push(cur + 0.);
        }
        return;
    }
    // Fully normalized N_l^m = sqrt((l+1/2)(l-m)!/(l+m)!) P_l^m by the stable recurrence.
    let mut nmm = std::f64::consts::FRAC_1_SQRT_2;
    let schmidt = |m: usize, v: f64| {
        let base = v * (2. / (2 * n + 1) as f64).sqrt();
        if m == 0 {
            base
        } else {
            let sign = if m % 2 == 1 { -1. } else { 1. };
            base * std::f64::consts::SQRT_2 * sign
        }
    };
    for m in 0..=n {
        if m > 0 {
            nmm *= -((2 * m + 1) as f64 / (2 * m) as f64).sqrt() * s;
        }
        let value = if m == n {
            nmm
        } else {
            let mut prev = nmm;
            let mut cur = ((2 * m + 3) as f64).sqrt() * x * nmm;
            for l in (m + 2)..=n {
                let (lf, mf) = (l as f64, m as f64);
                let alm = ((4. * lf * lf - 1.) / (lf * lf - mf * mf)).sqrt();
                let l1 = lf - 1.;
                let alm1 = ((4. * l1 * l1 - 1.) / (l1 * l1 - mf * mf)).sqrt();
                let next = alm * (x * cur - prev / alm1);
                prev = cur;
                cur = next;
            }
            cur
        };
        // Both normalized forms drop the Condon-Shortley phase again.
        let value = match norm {
            Normalization::Schmidt => schmidt(m, value),
            _ if m % 2 == 1 => -value,
            _ => value,
        };
        out.push(value + 0.);
    }
}

// ---------- ellipke, ellipj ----------
//
// Both use the arithmetic-geometric mean of 1 and k' = sqrt(1 - m): DLMF 19.8(i)
// and Abramowitz & Stegun 17.6 for K and E, the descending Landen transformation
// of DLMF 22.20(ii) and A&S 16.4 for sn, cn and dn. A&S 17.6 stop at step N when
// a_N = b_N, that is c_N = 0, to the accuracy wanted, which leaves one step open:
// `ellipke` stops at the first a_N, b_N with (a_N - b_N)/2 = c_{N+1} <= tol, `ellipj`
// at the first c_N <= tol. A vector argument runs every element for the same number
// of steps, the largest any element needs.

/// Elements from which `ellipj` splits the work over the worker pool. The
/// `ellipj_timing` test (i7-12700KF under load) had a 4-worker pool level with the
/// serial loop at 16384 elements and 1.4x faster at 65536.
const ELLIPJ_PARALLEL_AT: usize = 16_384;

/// Most AGM steps. Double precision needs at most 9 (k' >= 2^-26.5 when m < 1);
/// the cap only bounds a tolerance that can never be met (negative or NaN).
const AGM_MAX_STEPS: usize = 64;

/// One state (a_n, b_n, c_n) of the AGM of 1 and sqrt(1 - m), with
/// c_n^2 = a_n^2 - b_n^2 (DLMF 19.8.1-2, 22.20.1).
#[derive(Clone, Copy)]
struct Agm {
    a: f64,
    b: f64,
    c: f64,
}

impl Agm {
    fn new(m: f64) -> Self {
        Agm {
            a: 1.,
            b: (1. - m).sqrt(),
            c: m.sqrt(),
        }
    }

    /// c_{n+1} = (a_n - b_n)/2, computed as c_n^2 / (4 a_{n+1}) to avoid the cancellation.
    fn next_c(&self) -> f64 {
        self.c * self.c / (2. * (self.a + self.b))
    }

    fn step(&mut self) {
        *self = Agm {
            a: (self.a + self.b) / 2.,
            b: (self.a * self.b).sqrt(),
            c: self.next_c(),
        };
    }
}

/// AGM steps `ellipke` needs for 0 <= m < 1: the first N with c_{N+1} <= tol.
fn ellipke_steps(m: f64, tol: f64) -> usize {
    let mut agm = Agm::new(m);
    for n in 0..AGM_MAX_STEPS {
        let next = agm.next_c();
        if next <= tol || next == 0. {
            return n;
        }
        agm.step();
    }
    AGM_MAX_STEPS
}

/// K(m) and E(m) after `steps` AGM steps: K = pi / (2 a_N) and
/// E = K (1 - sum_{n=0}^N 2^(n-1) c_n^2) (DLMF 19.8.5-6, A&S 17.6.3-4).
fn ellipke_value(m: f64, steps: usize) -> (f64, f64) {
    if m == 1. {
        return (f64::INFINITY, 1.);
    }
    if m.is_nan() {
        return (f64::NAN, f64::NAN);
    }
    let mut agm = Agm::new(m);
    let (mut weight, mut sum) = (0.5, 0.5 * m);
    for _ in 0..steps {
        agm.step();
        weight *= 2.;
        sum += weight * agm.c * agm.c;
    }
    let k = FRAC_PI_2 / agm.a;
    (k, k * (1. - sum))
}

/// Descending Landen steps `ellipj` needs for 0 <= m < 1: the first N with c_N <= tol.
fn landen_steps(m: f64, tol: f64) -> usize {
    let mut agm = Agm::new(m);
    for n in 0..AGM_MAX_STEPS {
        if agm.c <= tol || agm.c == 0. {
            return n;
        }
        agm.step();
    }
    AGM_MAX_STEPS
}

/// The ratios c_n / a_n for n = 1..=steps of parameter m, and a_steps.
struct Landen {
    ratios: [f64; AGM_MAX_STEPS],
    steps: usize,
    a: f64,
}

impl Landen {
    fn new(m: f64, steps: usize) -> Self {
        let mut ratios = [0.; AGM_MAX_STEPS];
        let mut agm = Agm::new(m);
        for ratio in &mut ratios[..steps] {
            agm.step();
            *ratio = agm.c / agm.a;
        }
        Landen {
            ratios,
            steps,
            a: agm.a,
        }
    }
}

/// (sn, cn, dn)(u | m) for 0 <= m <= 1 with `steps` Landen steps; `shared`, when
/// given, was built for the same m.
///
/// phi_N = 2^N a_N u, then phi_{n-1} = (phi_n + asin((c_n/a_n) sin phi_n)) / 2 down to
/// phi_0; sn = sin phi_0, cn = cos phi_0 (DLMF 22.20.2-3, A&S 16.4). dn comes from
/// dn^2 = cn^2 + (1 - m) sn^2 (DLMF 22.6.1), a sum of two non-negative terms.
/// m = 0 is sn = sin, cn = cos, dn = 1 and m = 1 the limit sn = tanh, cn = dn = sech
/// (DLMF 22.5(ii)). m = 0 skips the 2^N scaling, which `steps`, shared with other
/// elements, could push past the double range for a huge u.
fn jacobi(u: f64, m: f64, steps: usize, shared: Option<&Landen>) -> (f64, f64, f64) {
    if m == 0. {
        return (libm::sin(u), libm::cos(u), 1.);
    }
    if m == 1. {
        let sech = 1. / libm::cosh(u);
        return (libm::tanh(u), sech, sech);
    }
    if m.is_nan() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let own;
    let landen = match shared {
        Some(landen) => landen,
        None => {
            own = Landen::new(m, steps);
            &own
        }
    };
    let mut phi = libm::scalbn(landen.a * u, landen.steps as i32);
    for &ratio in landen.ratios[..landen.steps].iter().rev() {
        phi = (phi + libm::asin(ratio * libm::sin(phi))) / 2.;
    }
    let (sn, cn) = (libm::sin(phi), libm::cos(phi));
    let dn = (cn * cn + (1. - m) * sn * sn).sqrt();
    (sn, cn, dn)
}

#[cfg(test)]
mod elliptic_tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    /// (m, K, E) from mpmath at 40 digits (tool/ellip-mpmath-reference.py).
    #[allow(clippy::excessive_precision)]
    const KE: &[(f64, f64, f64)] = &[
        (1e-10, 1.5707963268341665, 1.5707963267556266),
        (0.1, 1.6124413487202194, 1.5307576368977631),
        (0.3, 1.713889448178791, 1.4453630644126654),
        (0.5, 1.8540746773013719, 1.3506438810476755),
        (0.7, 2.075363135292469, 1.2416705679458229),
        (0.9, 2.5780921133481733, 1.1047747327040733),
        (0.99, 3.695637362989874, 1.015993545025224),
        (0.999999, 8.294051463601063, 1.0000038970261722),
        (0.999999999999, 15.201815980070121, 1.0000000000073508),
        (0.9999999999999998, 19.40812105567847, 1.000000000000002),
    ];

    /// (u, m, sn, cn, dn) from the same script.
    #[allow(clippy::excessive_precision)]
    const SN_CN_DN: &[(f64, f64, f64, f64, f64)] = &[
        (
            0.5,
            0.3,
            0.4742156227118206,
            0.8804087364264624,
            0.9656789647459512,
        ),
        (
            2.0,
            0.5,
            0.9946623253580177,
            -0.10318361552776183,
            0.7108610477840873,
        ),
        (
            -3.7,
            0.9,
            -0.911681826495138,
            -0.41089688151468035,
            0.5019488245973291,
        ),
        (
            10.0,
            0.2,
            -0.04224331986204232,
            -0.999107352553785,
            0.9998215342676945,
        ),
        (
            1.8540746773013719,
            0.5,
            1.0,
            2.98456382067177e-17,
            FRAC_1_SQRT_2,
        ),
        (
            25.0,
            0.999,
            0.999612433151943,
            -0.027838525249234494,
            0.04212135449583993,
        ),
        (1e-08, 0.7, 1e-08, 1.0, 1.0),
        (
            5.0,
            0.999999999,
            0.9999092045123453,
            0.013475263689029354,
            0.013475300787264898,
        ),
        (
            100.0,
            0.5,
            0.11960192815774341,
            -0.9928219270246553,
            0.9964174272815962,
        ),
        (
            0.3,
            1e-12,
            0.2955202066613353,
            0.9553364891256073,
            0.9999999999999564,
        ),
    ];

    fn relative(got: f64, want: f64) -> f64 {
        (got - want).abs() / want.abs()
    }

    #[test]
    fn complete_integrals_match_high_precision() {
        for &(m, k, e) in KE {
            let (gk, ge) = ellipke_value(m, ellipke_steps(m, f64::EPSILON));
            assert!(relative(gk, k) < 4e-16, "K({m}) = {gk:e}, want {k:e}");
            assert!(relative(ge, e) < 2e-15, "E({m}) = {ge:e}, want {e:e}");
        }
        assert_eq!(ellipke_value(0., 0), (FRAC_PI_2, FRAC_PI_2));
        assert_eq!(ellipke_value(1., 0), (f64::INFINITY, 1.));
    }

    #[test]
    fn legendre_relation_holds() {
        // E K' + E' K - K K' = pi/2 (DLMF 19.7.1), with K' = K(1 - m).
        for &m in &[0.01, 0.2, 0.5, 0.8, 0.95] {
            let (k, e) = ellipke_value(m, ellipke_steps(m, f64::EPSILON));
            let (kp, ep) = ellipke_value(1. - m, ellipke_steps(1. - m, f64::EPSILON));
            assert!((e * kp + ep * k - k * kp - FRAC_PI_2).abs() < 4e-15, "{m}");
        }
    }

    #[test]
    fn jacobi_functions_match_high_precision() {
        for &(u, m, sn, cn, dn) in SN_CN_DN {
            let steps = landen_steps(m, f64::EPSILON);
            let (gs, gc, gd) = jacobi(u, m, steps, None);
            // phi_0 carries the rounding of phi_N = 2^N a_N u, a few ulps of u; near m = 1
            // the asin of the last back-substitution steps (c_1/a_1 near 1) adds about
            // 20 ulps more (5.2e-15 at u = 5, m = 1 - 1e-9).
            let tol = 2e-15 * u.abs().max(1.);
            for (got, want, name) in [(gs, sn, "sn"), (gc, cn, "cn"), (gd, dn, "dn")] {
                assert!(
                    (got - want).abs() < tol,
                    "{name}({u}|{m}) = {got:e}, want {want:e}"
                );
            }
            assert_eq!(
                (gs, gc, gd),
                jacobi(u, m, steps, Some(&Landen::new(m, steps)))
            );
        }
        let sech = 1. / libm::cosh(2.);
        assert_eq!(jacobi(2., 1., 0, None), (libm::tanh(2.), sech, sech));
        assert_eq!(jacobi(1., 0., 0, None), (libm::sin(1.), libm::cos(1.), 1.));
    }

    #[test]
    fn stopping_rules_and_bounds() {
        // c_3 = 1.2e-5 is the first c_{N+1} below 1e-3, so K uses a_2.
        assert_eq!(ellipke_steps(0.5, 1e-3), 2);
        // c_2 = 1.8e-3, c_3 = 9.0e-7: sn, cn, dn use three steps.
        assert_eq!(landen_steps(0.3, 1e-3), 3);
        assert_eq!(ellipke_steps(0., f64::EPSILON), 0);
        assert_eq!(landen_steps(0., f64::EPSILON), 0);
        let near_one = 1. - f64::EPSILON / 2.;
        assert!(ellipke_steps(near_one, f64::EPSILON) <= 9);
        assert!(landen_steps(near_one, f64::EPSILON) <= 9);
        // A tolerance that can never be met stops once c underflows to zero.
        for tol in [-1., f64::NAN, 0.] {
            assert!(ellipke_steps(near_one, tol) < AGM_MAX_STEPS);
            assert!(landen_steps(near_one, tol) < AGM_MAX_STEPS);
        }
    }

    #[test]
    fn vector_arguments_share_the_largest_step_count() {
        let e = Engine::default();
        let m = Value::Matrix(Matrix::from_column_vec(1, 2, vec![0.1, 0.9]).unwrap());
        let tol = Value::Matrix(Matrix::from_column_vec(1, 1, vec![1e-3]).unwrap());
        let Value::Matrix(k) = e
            .special_fns_outputs("ellipke", &[m, tol], 1)
            .unwrap()
            .remove(0)
        else {
            panic!("ellipke returns a real matrix");
        };
        assert_eq!(ellipke_steps(0.1, 1e-3), 1);
        assert_eq!(ellipke_steps(0.9, 1e-3), 2);
        assert_eq!(k.data()[0], ellipke_value(0.1, 2).0);
        assert_eq!(k.data()[1], ellipke_value(0.9, 2).0);
    }

    fn ellipj_outputs(e: &Engine, len: usize, m: Value) -> Vec<Value> {
        let u = (0..len).map(|i| i as f64 * 1e-3 - 7.).collect();
        let u = Value::Matrix(Matrix::from_column_vec(1, len, u).unwrap());
        e.special_fns_outputs("ellipj", &[u, m], 3).unwrap()
    }

    fn real(value: &Value) -> &[f64] {
        match value {
            Value::Matrix(m) => m.data(),
            _ => panic!("ellipj returns real matrices"),
        }
    }

    #[test]
    fn parallel_ellipj_matches_serial() {
        let e = Engine::default();
        let len = ELLIPJ_PARALLEL_AT + 1000;
        let m = Value::Matrix(Matrix::from_column_vec(1, 1, vec![0.7]).unwrap());
        let serial = ellipj_outputs(&e, len, m.clone());
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let parallel = pool.install(|| ellipj_outputs(&e, len, m));
        for (a, b) in serial.iter().zip(&parallel) {
            assert_eq!(real(a), real(b));
        }
    }

    fn row(data: Vec<f64>) -> Value {
        Value::Matrix(Matrix::from_column_vec(1, data.len(), data).unwrap())
    }

    #[test]
    fn zero_parameter_ignores_the_shared_step_count() {
        let e = Engine::default();
        // The m = 0.5 element needs steps; scaling 1e308 by 2^N would overflow.
        let out = e
            .special_fns_outputs("ellipj", &[row(vec![1e308, 1.]), row(vec![0., 0.5])], 3)
            .unwrap();
        assert_eq!(real(&out[0])[0], libm::sin(1e308));
        assert_eq!(real(&out[1])[0], libm::cos(1e308));
        assert_eq!(real(&out[2])[0], 1.);
        let alone = e
            .special_fns_outputs("ellipj", &[row(vec![1.]), row(vec![0.5])], 3)
            .unwrap();
        for (all, one) in out.iter().zip(&alone) {
            assert_eq!(real(all)[1], real(one)[0]);
        }
    }

    /// The statement budget charges the buffers `ellipj` and `ellipke` really hold at
    /// once, in 8-byte units: U, M, the triples or pairs, and the outputs.
    #[test]
    fn budget_counts_the_peak_buffers() {
        let with_room = |units: usize| Engine {
            evaluated_elements: EVALUATION_UNITS - units,
            ..Engine::default()
        };
        let (u, m) = (row(vec![1.]), row(vec![0.5; 10]));
        for (count, units) in [(1, 1 + 10 + 40), (2, 1 + 10 + 50), (3, 1 + 10 + 60)] {
            let call = |room| {
                with_room(room).special_fns_outputs("ellipj", &[u.clone(), m.clone()], count)
            };
            assert!(call(units - 1).unwrap_err().contains("evaluation budget"));
            assert_eq!(call(units).unwrap().len(), count);
        }
        for (count, units) in [(1, 20), (2, 50)] {
            let call = |room| {
                with_room(room).special_fns_outputs("ellipke", std::slice::from_ref(&m), count)
            };
            assert!(call(units - 1).unwrap_err().contains("evaluation budget"));
            assert_eq!(call(units).unwrap().len(), count);
        }
    }

    /// `m = 0.5*ones(1, 8e6); [s, c, d] = ellipj(1, m)` holds 7 doubles per element at
    /// its peak (448 MB), well past the 256 MiB budget; it must fail before allocating.
    #[test]
    fn eight_million_elements_exceed_the_budget() {
        let e = Engine::default();
        let m = row(vec![0.5; 8_000_000]);
        let error = e
            .special_fns_outputs("ellipj", &[row(vec![1.]), m.clone()], 3)
            .unwrap_err();
        assert!(error.starts_with("Statement exceeds its 256 MiB evaluation budget"));
        let error = e.special_fns_outputs("ellipke", &[m], 2).unwrap_err();
        assert!(error.starts_with("Statement exceeds its 256 MiB evaluation budget"));
    }

    /// Release timing with one and four workers (AGENTS.md), and the serial against the
    /// pooled path at small sizes for `ELLIPJ_PARALLEL_AT`:
    /// `cargo test --release --features desktop --lib ellipj_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ellipj_timing() {
        fn best(run: impl Fn()) -> f64 {
            (0..7)
                .map(|_| {
                    let start = std::time::Instant::now();
                    run();
                    start.elapsed().as_secs_f64() * 1e3
                })
                .fold(f64::INFINITY, f64::min)
        }
        let e = Engine::default();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let landen = Landen::new(0.7, landen_steps(0.7, f64::EPSILON));
        let value = |i: usize| jacobi(i as f64 * 1e-3, 0.7, landen.steps, Some(&landen));
        for len in [512, 1_024, 2_048, 4_096, 16_384] {
            let serial = best(|| drop(e.spf_map(len, value).unwrap()));
            let pooled = best(|| pool.install(|| drop(e.spf_par_map(len, value).unwrap())));
            println!("n={len}: serial {serial:.3} ms, 4-worker pool {pooled:.3} ms");
        }
        for len in [65_536, 1_000_000] {
            let scalar = Value::Matrix(Matrix::from_column_vec(1, 1, vec![0.7]).unwrap());
            let m = (0..len).map(|i| (i % 1000) as f64 / 1000.).collect();
            let vector = Value::Matrix(Matrix::from_column_vec(1, len, m).unwrap());
            for (label, m) in [("scalar m", scalar), ("vector m", vector)] {
                for workers in [1, 4] {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(workers)
                        .build()
                        .unwrap();
                    let time = best(|| drop(pool.install(|| ellipj_outputs(&e, len, m.clone()))));
                    println!("ellipj n={len} {label}, {workers} worker(s): best of 7 {time:.3} ms");
                }
            }
        }
    }
}

// ---------- engine glue ----------

impl Engine {
    /// One-output calls of every function in this module; `None` for other names.
    pub(super) fn special_fns_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        Ok(Some(self.special_fns_outputs(name, args, 1)?.remove(0)))
    }

    /// Every function of this module with `count` outputs.
    pub(super) fn special_fns_outputs(
        &self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max_outputs = match name {
            "ellipke" => 2,
            "ellipj" => 3,
            _ => 1,
        };
        if count > max_outputs {
            return Err("Too many output arguments.".into());
        }
        match name {
            "besselj" | "bessely" | "besseli" | "besselk" => {
                let kind = match name {
                    "besselj" => Kind::J,
                    "bessely" => Kind::Y,
                    "besseli" => Kind::I,
                    _ => Kind::K,
                };
                arity(args, 2, 3)?;
                let scaled = self.spf_scale_flag(args.get(2))?;
                Ok(vec![
                    self.spf_bessel_call(kind, &args[0], &args[1], scaled)?,
                ])
            }
            "besselh" => {
                arity(args, 2, 4)?;
                let (k, z, scale) = match args.len() {
                    2 => (None, &args[1], None),
                    _ => (Some(&args[1]), &args[2], args.get(3)),
                };
                let kind = match k {
                    None => Kind::H1,
                    Some(value) => {
                        let k = self.spf_arg(value)?;
                        match (k.scalar(), k.is_complex(), k.re.first().copied()) {
                            (true, false, Some(1.)) => Kind::H1,
                            (true, false, Some(2.)) => Kind::H2,
                            _ => return Err("K must be 1 or 2.".into()),
                        }
                    }
                };
                let scaled = self.spf_scale_flag(scale)?;
                Ok(vec![self.spf_bessel_call(kind, &args[0], z, scaled)?])
            }
            "airy" => Ok(vec![self.spf_airy(args)?]),
            "gammainc" => Ok(vec![self.spf_gammainc(args)?]),
            "gammaincinv" => Ok(vec![self.spf_gammaincinv(args)?]),
            "betainc" => Ok(vec![self.spf_betainc(args)?]),
            "betaincinv" => Ok(vec![self.spf_betaincinv(args)?]),
            "expint" => Ok(vec![self.spf_expint(args)?]),
            "psi" => Ok(vec![self.spf_psi(args)?]),
            "legendre" => Ok(vec![self.spf_legendre(args)?]),
            "ellipke" => self.spf_ellipke(args, count),
            "ellipj" => self.spf_ellipj(args, count),
            _ => Ok(vec![self.spf_erfcx(args)?]),
        }
    }

    fn spf_budget(&self, units: usize) -> Result<()> {
        if self
            .evaluated_elements
            .checked_add(units)
            .is_none_or(|n| n > EVALUATION_UNITS)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    fn spf_arg(&self, value: &Value) -> Result<Arg> {
        let complex = match value {
            Value::Complex(z) | Value::Container(Array::Complex(z)) => Some(z.clone()),
            _ => None,
        };
        if let Some(z) = complex {
            return Ok(Arg {
                rows: z.rows(),
                cols: z.cols(),
                re: z.data().iter().map(|v| v.re).collect(),
                im: Some(z.data().iter().map(|v| v.im).collect()),
            });
        }
        let matrix = match value {
            Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
            Value::Logical(_) | Value::Container(Array::Logical(_) | Array::Typed(_)) => {
                value.clone().matrix(&self.cancel)?
            }
            Value::Container(Array::Character(_)) | Value::Text(_) => {
                value.clone().matrix(&self.cancel)?
            }
            _ => return Err("Inputs must be numeric.".into()),
        };
        Ok(Arg {
            rows: matrix.rows(),
            cols: matrix.cols(),
            re: matrix.data().to_vec(),
            im: None,
        })
    }

    fn spf_scale_flag(&self, value: Option<&Value>) -> Result<bool> {
        let Some(value) = value else {
            return Ok(false);
        };
        let scale = self.spf_arg(value)?;
        match (scale.scalar(), scale.is_complex(), scale.re.first()) {
            (true, false, Some(&v)) if v == 0. || v == 1. => Ok(v == 1.),
            _ => Err("SCALE must be 0 or 1.".into()),
        }
    }

    /// Applies `value` to every element, checking cancellation per chunk.
    fn spf_map<T>(&self, len: usize, value: impl Fn(usize) -> T) -> Result<Vec<T>> {
        self.spf_budget(len.saturating_mul(2))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| "Cannot allocate special function output")?;
        for first in (0..len).step_by(CHUNK) {
            self.check_cancel()?;
            out.extend((first..(first + CHUNK).min(len)).map(&value));
        }
        Ok(out)
    }

    /// A real result when every imaginary part is zero, unless `keep_complex`.
    fn spf_finish(
        &self,
        rows: usize,
        cols: usize,
        data: Vec<C>,
        keep_complex: bool,
    ) -> Result<Value> {
        if !keep_complex && data.iter().all(|z| z.im == 0.) {
            return self.spf_real(rows, cols, data.iter().map(|z| z.re).collect());
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            rows,
            cols,
            data,
            &self.cancel,
        )?))
    }

    fn spf_real(&self, rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
    }

    fn spf_bessel_call(&self, kind: Kind, nu: &Value, z: &Value, scaled: bool) -> Result<Value> {
        let nu = self.spf_arg(nu)?;
        let z = self.spf_arg(z)?;
        if nu.is_complex() {
            return Err("NU must be real.".into());
        }
        // A row of orders against a column of arguments gives the reference's table.
        if nu.rows == 1 && nu.cols > 1 && z.cols == 1 && z.rows > 1 {
            let (rows, cols) = (z.rows, nu.cols);
            let data = self.spf_map(rows * cols, |i| {
                bessel(kind, nu.re[i / rows], z.at(i % rows), scaled)
            })?;
            return self.spf_finish(rows, cols, data, false);
        }
        let (rows, cols) = common_shape(
            &[&nu, &z],
            "NU and Z must be the same size or one must be a scalar.",
        )?;
        let data = self.spf_map(rows * cols, |i| bessel(kind, nu.re_at(i), z.at(i), scaled))?;
        self.spf_finish(rows, cols, data, false)
    }

    fn spf_airy(&self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 3)?;
        let (k, z) = if args.len() == 1 {
            (0, &args[0])
        } else {
            let k = self.spf_arg(&args[0])?;
            let k = match (k.scalar(), k.is_complex(), k.re.first()) {
                (true, false, Some(&v)) if [0., 1., 2., 3.].contains(&v) => v as usize,
                _ => return Err("K must be 0, 1, 2, or 3.".into()),
            };
            (k, &args[1])
        };
        let scaled = self.spf_scale_flag(args.get(2))?;
        let z = self.spf_arg(z)?;
        let data = self.spf_map(z.len(), |i| airy(k, z.at(i), scaled))?;
        self.spf_finish(z.rows, z.cols, data, false)
    }

    fn spf_tail(&self, value: Option<&Value>, scaled: bool) -> Result<Tail> {
        let Some(value) = value else {
            return Ok(Tail::Lower);
        };
        let tail = text(value).map(|t| t.to_ascii_lowercase());
        match tail.as_deref() {
            Some("lower") => Ok(Tail::Lower),
            Some("upper") => Ok(Tail::Upper),
            Some("scaledlower") if scaled => Ok(Tail::ScaledLower),
            Some("scaledupper") if scaled => Ok(Tail::ScaledUpper),
            _ if scaled => {
                Err("TAIL must be 'lower', 'upper', 'scaledlower', or 'scaledupper'.".into())
            }
            _ => Err("TAIL must be 'lower' or 'upper'.".into()),
        }
    }

    fn spf_gammainc(&self, args: &[Value]) -> Result<Value> {
        arity(args, 2, 3)?;
        let tail = self.spf_tail(args.get(2), true)?;
        let x = self.spf_arg(&args[0])?;
        let a = self.spf_arg(&args[1])?;
        if a.is_complex() || a.re.iter().any(|&v| v < 0.) {
            return Err("A must be real and non-negative.".into());
        }
        if x.is_complex() {
            return Err("X must be real.".into());
        }
        let (rows, cols) = common_shape(
            &[&x, &a],
            "X and A must be the same size, or either can be a scalar.",
        )?;
        let n = rows * cols;
        if x.re.iter().any(|&v| v < 0.) {
            let data = self.spf_map(n, |i| {
                let (xv, av) = (x.re_at(i), a.re_at(i));
                if xv < 0. {
                    gammainc_negative(xv, av, tail)
                } else {
                    c(gammainc_real(xv, av, tail), 0.)
                }
            })?;
            return self.spf_finish(rows, cols, data, true);
        }
        let data = self.spf_map(n, |i| gammainc_real(x.re_at(i), a.re_at(i), tail))?;
        self.spf_real(rows, cols, data)
    }

    fn spf_gammaincinv(&self, args: &[Value]) -> Result<Value> {
        arity(args, 2, 3)?;
        let upper = self.spf_tail(args.get(2), false)? == Tail::Upper;
        let y = self.spf_arg(&args[0])?;
        let a = self.spf_arg(&args[1])?;
        if y.is_complex()
            || y.re
                .iter()
                .any(|&v| !(0. ..=1.).contains(&v) && !v.is_nan())
        {
            return Err("Y must be in the interval [0,1].".into());
        }
        if a.is_complex() || a.re.iter().any(|&v| v < 0.) {
            return Err("A must be real and non-negative.".into());
        }
        let (rows, cols) = common_shape(
            &[&y, &a],
            "Y and A must be the same size, or either can be a scalar.",
        )?;
        let data = self.spf_map(rows * cols, |i| {
            gammaincinv_real(y.re_at(i), a.re_at(i), upper)
        })?;
        self.spf_real(rows, cols, data)
    }

    fn spf_beta_args(
        &self,
        args: &[Value],
        first: &str,
    ) -> Result<(Arg, Arg, Arg, bool, usize, usize)> {
        arity(args, 3, 4)?;
        let upper = self.spf_tail(args.get(3), false)? == Tail::Upper;
        let x = self.spf_arg(&args[0])?;
        let a = self.spf_arg(&args[1])?;
        let b = self.spf_arg(&args[2])?;
        if x.is_complex() || x.re.iter().any(|&v| !(0. ..=1.).contains(&v)) {
            return Err(format!("{first} must be in the interval [0,1]."));
        }
        if a.is_complex() || a.re.iter().any(|&v| v < 0.) {
            return Err("Z must be real and non-negative.".into());
        }
        if b.is_complex() || b.re.iter().any(|&v| v < 0.) {
            return Err("W must be real and non-negative.".into());
        }
        let (rows, cols) = common_shape(
            &[&x, &a, &b],
            &format!("{first}, Z and W must all be the same size (or any of them can be scalar)."),
        )?;
        Ok((x, a, b, upper, rows, cols))
    }

    fn spf_betainc(&self, args: &[Value]) -> Result<Value> {
        let (x, a, b, upper, rows, cols) = self.spf_beta_args(args, "X")?;
        let data = self.spf_map(rows * cols, |i| {
            let (lower, up) = betainc_pair(x.re_at(i), a.re_at(i), b.re_at(i));
            if upper { up } else { lower }
        })?;
        self.spf_real(rows, cols, data)
    }

    fn spf_betaincinv(&self, args: &[Value]) -> Result<Value> {
        let (y, a, b, upper, rows, cols) = self.spf_beta_args(args, "Y")?;
        let data = self.spf_map(rows * cols, |i| {
            betaincinv_real(y.re_at(i), a.re_at(i), b.re_at(i), upper)
        })?;
        self.spf_real(rows, cols, data)
    }

    fn spf_expint(&self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 1)?;
        let x = self.spf_arg(&args[0])?;
        let data = self.spf_map(x.len(), |i| expint_value(x.at(i)))?;
        self.spf_finish(x.rows, x.cols, data, x.is_complex())
    }

    fn spf_psi(&self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 2)?;
        let (k, x) = if args.len() == 1 {
            (0, &args[0])
        } else {
            let k = self.spf_arg(&args[0])?;
            let k = match (k.scalar(), k.is_complex(), k.re.first()) {
                (true, false, Some(&v)) if is_integer(v) && (0. ..=1000.).contains(&v) => {
                    v as usize
                }
                _ => return Err("K must be a nonnegative integer.".into()),
            };
            (k, &args[1])
        };
        let x = self.spf_arg(x)?;
        if x.is_complex() {
            return Err("Input must be real.".into());
        }
        if x.re.iter().any(|&v| v < 0.) {
            return Err("X must be nonnegative.".into());
        }
        let data = self.spf_map(x.len(), |i| polygamma(k, x.re[i]))?;
        self.spf_real(x.rows, x.cols, data)
    }

    fn spf_legendre(&self, args: &[Value]) -> Result<Value> {
        arity(args, 2, 3)?;
        let n = self.spf_arg(&args[0])?;
        let n = match (n.scalar(), n.is_complex(), n.re.first()) {
            (true, false, Some(&v)) if is_integer(v) && v >= 0. => v,
            _ => return Err("N must be a positive scalar integer.".into()),
        };
        let norm = match args.get(2) {
            None => Normalization::Unnormalized,
            Some(value) => match text(value).map(|t| t.to_ascii_lowercase()).as_deref() {
                Some("unnorm") => Normalization::Unnormalized,
                Some("sch") => Normalization::Schmidt,
                Some("norm") => Normalization::Full,
                _ => {
                    return Err("NORMALIZE must be 'unnorm', 'sch', or 'norm'.".into());
                }
            },
        };
        let x = self.spf_arg(&args[1])?;
        if x.is_complex()
            || x.re
                .iter()
                .any(|&v| !(-1. ..=1.).contains(&v) && !v.is_nan())
        {
            return Err("X must be real and in the range (-1,1).".into());
        }
        if !x.is_vector() && x.len() > 0 {
            return Err(
                "legendre of a matrix X returns a 3-D array; N-D arrays are not supported yet"
                    .into(),
            );
        }
        if n > LEGENDRE_MAX_DEGREE {
            return Err(format!(
                "legendre supports degrees up to {LEGENDRE_MAX_DEGREE}."
            ));
        }
        let n = n as usize;
        let total = (n + 1)
            .checked_mul(x.len())
            .ok_or("legendre output is too large")?;
        self.spf_budget(total.saturating_add(x.len() * n))?;
        let mut data = Vec::new();
        data.try_reserve_exact(total)
            .map_err(|_| "Cannot allocate legendre output")?;
        for (i, &v) in x.re.iter().enumerate() {
            if i % 64 == 0 {
                self.check_cancel()?;
            }
            legendre_column(n, v, norm, &mut data);
        }
        self.spf_real(n + 1, x.len(), data)
    }

    fn spf_tolerance(&self, value: Option<&Value>) -> Result<f64> {
        match value {
            None => Ok(f64::EPSILON),
            Some(value) => {
                let tol = self.spf_arg(value)?;
                if tol.is_complex() || !tol.scalar() {
                    return Err("TOL must be a real scalar.".into());
                }
                Ok(tol.re[0])
            }
        }
    }

    /// The parameter M of `ellipke` and `ellipj`: real, in [0, 1] or NaN.
    fn spf_elliptic_parameter(&self, value: &Value) -> Result<Arg> {
        let m = self.spf_arg(value)?;
        if m.is_complex() {
            return Err("Input arguments must be real.".into());
        }
        if m.re.iter().any(|&v| v < 0. || v > 1.) {
            return Err("M must be in the range 0 <= M <= 1.".into());
        }
        Ok(m)
    }

    /// The most AGM steps any element of `m` in [0, 1) needs.
    fn spf_agm_steps(&self, m: &[f64], steps: impl Fn(f64) -> usize) -> Result<usize> {
        let mut most = 0;
        for chunk in m.chunks(CHUNK) {
            self.check_cancel()?;
            for &v in chunk.iter().filter(|v| (0. ..1.).contains(*v)) {
                most = most.max(steps(v));
            }
        }
        Ok(most)
    }

    /// `spf_map` over the current worker pool, in chunks of `CHUNK` elements.
    fn spf_par_map<T: Copy + Default + Send>(
        &self,
        len: usize,
        value: impl Fn(usize) -> T + Sync,
    ) -> Result<Vec<T>> {
        use rayon::prelude::*;
        use std::sync::atomic::Ordering;
        self.spf_budget(len.saturating_mul(2))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| "Cannot allocate special function output")?;
        out.resize(len, T::default());
        let cancel = &*self.cancel;
        out.par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(|(block, chunk)| -> Result<()> {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Calculation cancelled".into());
                }
                for (j, slot) in chunk.iter_mut().enumerate() {
                    *slot = value(block * CHUNK + j);
                }
                Ok(())
            })?;
        self.check_cancel()?;
        Ok(out)
    }

    fn spf_ellipke(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        arity(args, 1, 2)?;
        let m = self.spf_elliptic_parameter(&args[0])?;
        let tol = self.spf_tolerance(args.get(1))?;
        let steps = self.spf_agm_steps(&m.re, |v| ellipke_steps(v, tol))?;
        let len = m.len();
        let value = |i: usize| ellipke_value(m.re[i], steps);
        if count <= 1 {
            let k = self.spf_map(len, |i| value(i).0)?;
            return Ok(vec![self.spf_real(m.rows, m.cols, k)?]);
        }
        // Peak: M, the (K, E) pairs and the two vectors unzipped from them.
        self.spf_budget(len.saturating_mul(5))?;
        let (k, e): (Vec<f64>, Vec<f64>) = self.spf_map(len, value)?.into_iter().unzip();
        Ok(vec![
            self.spf_real(m.rows, m.cols, k)?,
            self.spf_real(m.rows, m.cols, e)?,
        ])
    }

    fn spf_ellipj(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        arity(args, 2, 3)?;
        let u = self.spf_arg(&args[0])?;
        if u.is_complex() {
            return Err("Input arguments must be real.".into());
        }
        let m = self.spf_elliptic_parameter(&args[1])?;
        let tol = self.spf_tolerance(args.get(2))?;
        let (rows, cols) = common_shape(&[&u, &m], "U and M must be the same size.")?;
        let len = rows * cols;
        let count = count.clamp(1, 3);
        // Peak: U and M, the (sn, cn, dn) triples and the `count` output vectors.
        self.spf_budget(
            u.len()
                .saturating_add(m.len())
                .saturating_add(len.saturating_mul(3 + count)),
        )?;
        let steps = self.spf_agm_steps(&m.re, |v| landen_steps(v, tol))?;
        // A scalar M (the common case) shares one Landen sequence.
        let shared = m.scalar().then(|| Landen::new(m.re[0], steps));
        let value = |i| jacobi(u.re_at(i), m.re_at(i), steps, shared.as_ref());
        let values = if len >= ELLIPJ_PARALLEL_AT && crate::parallel::current_worker_count() > 1 {
            self.spf_par_map(len, value)?
        } else {
            self.spf_map(len, value)?
        };
        let mut out = Vec::new();
        for k in 0..count {
            let data = values.iter().map(|v| [v.0, v.1, v.2][k]).collect();
            out.push(self.spf_real(rows, cols, data)?);
        }
        Ok(out)
    }

    fn spf_erfcx(&self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 1)?;
        let x = self.spf_arg(&args[0])?;
        if x.is_complex() {
            return Err("Input must be real and full.".into());
        }
        let data = self.spf_map(x.len(), |i| erfcx_value(x.re[i]))?;
        self.spf_real(x.rows, x.cols, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reciprocal_gamma_series_matches_tgamma() {
        for &mu in &[-0.099, -0.05, 0.0001, 0.07, 0.0999] {
            let (gam1, gam2, gampl, gammi) = temme_gammas(mu);
            let exact_pl = 1. / libm::tgamma(1. + mu);
            let exact_mi = 1. / libm::tgamma(1. - mu);
            assert!((gampl - exact_pl).abs() < 1e-15, "{mu}");
            assert!((gammi - exact_mi).abs() < 1e-15, "{mu}");
            assert!((gam2 - (exact_pl + exact_mi) / 2.).abs() < 1e-15, "{mu}");
            assert!(
                (gam1 - (exact_mi - exact_pl) / (2. * mu)).abs() < 1e-11,
                "{mu}"
            );
        }
    }

    fn close(got: f64, want: f64, tolerance: f64) -> bool {
        (got - want).abs() <= tolerance * want.abs()
    }

    #[test]
    fn incomplete_gamma_matches_closed_forms() {
        // P(1, x) = 1 - e^-x, P(1/2, x) = erf(sqrt x), Q(3, x) = e^-x (1 + x + x^2/2)
        // (DLMF 8.4.10, 8.4.1), across the series, small-a and fraction regions.
        for &x in &[0.01, 0.5, 1.4, 1.6, 3., 10., 50.] {
            let p1 = -libm::expm1(-x);
            assert!(close(gammainc_real(x, 1., Tail::Lower), p1, 4e-15), "{x}");
            let r = x.sqrt();
            assert!(
                close(gammainc_real(x, 0.5, Tail::Lower), libm::erf(r), 4e-15),
                "{x}"
            );
            assert!(
                close(gammainc_real(x, 0.5, Tail::Upper), libm::erfc(r), 1e-14),
                "{x}"
            );
            let q3 = libm::exp(-x) * (1. + x + x * x / 2.);
            assert!(close(gammainc_real(x, 3., Tail::Upper), q3, 4e-15), "{x}");
            let scaled = q3 * 6. * libm::exp(x) / x.powi(3);
            assert!(
                close(gammainc_real(x, 3., Tail::ScaledUpper), scaled, 4e-15),
                "{x}"
            );
        }
        // Small a keeps the relative accuracy of Q ~ a E1(x).
        let q = gammainc_real(0.5, 1e-10, Tail::Upper);
        assert!(close(q, 1e-10 * 0.559_773_594_776_160_8, 1e-9), "{q}");
    }

    #[test]
    fn uniform_expansion_agrees_with_series_and_fraction() {
        let a = 1e5;
        for &f in &[0.99, 0.999, 1., 1.001, 1.01] {
            let x = a * f;
            let (p, q) = gamma_uniform(a, x);
            let s = power_prefix(a, x) * gamma_series(a, x);
            let fraction = power_prefix(a, x) * a * gamma_fraction(a, x);
            if f <= 1. {
                assert!(close(p, s, 1e-13), "{f}: {p} {s}");
            } else {
                assert!(close(q, fraction, 1e-13), "{f}: {q} {fraction}");
            }
        }
        // P(a, a) = 1/2 + 1/(3 sqrt(2 pi a)) + O(1/a) beyond the series' reach.
        let p = gammainc_real(1e10, 1e10, Tail::Lower);
        assert!(
            close(p, 0.5 + 1. / (3. * (2. * PI * 1e10).sqrt()), 1e-10),
            "{p}"
        );
    }

    #[test]
    fn incomplete_beta_matches_closed_forms() {
        // I_x(a, 1) = x^a, I_x(1, b) = 1 - (1-x)^b, I_x(1/2, 1/2) = (2/pi) asin(sqrt x).
        for &x in &[1e-6, 0.1, 0.45, 0.5, 0.8, 0.999] {
            let (lower, upper) = betainc_pair(x, 2.5, 1.);
            assert!(close(lower, x.powf(2.5), 4e-15), "{x}");
            assert!(close(upper, -libm::expm1(2.5 * libm::log(x)), 4e-14), "{x}");
            let (lower, _) = betainc_pair(x, 1., 3.5);
            assert!(
                close(lower, -libm::expm1(3.5 * libm::log1p(-x)), 4e-15),
                "{x}"
            );
            let (lower, _) = betainc_pair(x, 0.5, 0.5);
            assert!(close(lower, 2. / PI * libm::asin(x.sqrt()), 4e-15), "{x}");
        }
    }

    #[test]
    fn inverses_round_trip() {
        for &a in &[1e-3, 0.5, 1., 3., 40., 1e4, 1e6] {
            for &y in &[1e-300, 1e-12, 0.05, 0.5, 0.9, 1. - 1e-12] {
                for &upper in &[false, true] {
                    let x = gammaincinv_real(y, a, upper);
                    let tail = if upper { Tail::Upper } else { Tail::Lower };
                    let back = gammainc_real(x, a, tail);
                    // residual within a few roundings of x or of y
                    let slack = power_prefix(a, x) * a * 8. * f64::EPSILON;
                    let ok = (back - y).abs() <= slack.max(8. * f64::EPSILON * y);
                    assert!(
                        ok || x == 0.,
                        "gammaincinv({y}, {a}, {upper}) = {x}: {back}"
                    );
                }
            }
        }
        for &(a, b) in &[(0.1, 5.), (0.5, 0.5), (2., 3.), (30., 40.), (1., 1e3)] {
            for &y in &[1e-300, 1e-12, 0.05, 0.5, 0.9, 1. - 1e-12] {
                for &upper in &[false, true] {
                    let x = betaincinv_real(y, a, b, upper);
                    let (lower, up) = betainc_pair(x, a, b);
                    let back = if upper { up } else { lower };
                    let slack =
                        beta_prefix(x, a, b) / x.min(1. - x).max(1e-300) * 8. * f64::EPSILON;
                    let ok = (back - y).abs() <= slack.max(8. * f64::EPSILON * y);
                    assert!(
                        ok || x == 0. || x == 1.,
                        "betaincinv({y}, {a}, {b}) = {x}: {back}"
                    );
                }
            }
        }
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn expint_matches_high_precision_values() {
        // 150-digit evaluations of the series DLMF 6.6.2 (tool/special-fns-checks).
        let cases = [
            (
                (0.5, 2.),
                (-0.238_126_937_892_671_86, -0.025_877_115_590_053_963),
            ),
            (
                (-3., 0.5),
                (-9.383_603_509_330_943, 0.129_212_970_084_629_76),
            ),
            (
                (5., -7.),
                (-1.346_510_875_229_199_6e-5, 7.303_617_272_646_524e-4),
            ),
            ((-20., 3.), (24_171_587.218_951_08, 7_456_212.526_029_36)),
            ((0.3, 0.), (0.905_676_651_675_846_8, 0.)),
            ((3., 0.), (0.013_048_381_094_197_037, 0.)),
            ((-5., 0.), (-40.185_275_355_803_18, -PI)),
        ];
        for ((x, y), (re, im)) in cases {
            let e = expint_value(c(x, y));
            let error = (e - c(re, im)).norm() / c(re, im).norm();
            assert!(error < 1e-14, "E1({x}+{y}i) = {e}: {error:e}");
        }
    }

    #[test]
    fn incomplete_gamma_at_a_zero_follows_the_limits() {
        // a -> 0+: P -> 1, Q -> 0, the scaled lower tail -> e^x (the series sum
        // x^k/k!), the scaled upper tail -> 0 (DLMF 8.2.4, 8.2.7).
        for &x in &[0.5, 1., 30.] {
            assert_eq!(gammainc_real(x, 0., Tail::Lower), 1.);
            assert_eq!(gammainc_real(x, 0., Tail::Upper), 0.);
            assert_eq!(gammainc_real(x, 0., Tail::ScaledUpper), 0.);
            let scaled = gammainc_real(x, 0., Tail::ScaledLower);
            assert!(close(scaled, libm::exp(x), 1e-15), "{x}: {scaled}");
            // the same limits for negative x, and the series at a tiny a
            let negative = gammainc_negative(-x, 0., Tail::ScaledLower);
            assert_eq!(negative, c(libm::exp(-x), 0.));
            let near = gammainc_real(x, 1e-9, Tail::ScaledLower);
            assert!(close(near, libm::exp(x), 1e-7), "{x}: {near}");
        }
        let e = gammainc_real(1., 0., Tail::ScaledLower);
        assert!(close(e, std::f64::consts::E, 1e-15), "{e}");
        assert_eq!(
            gammainc_real(f64::INFINITY, 0., Tail::ScaledLower),
            f64::INFINITY
        );
    }

    #[test]
    fn uniform_scaled_tails_survive_underflow() {
        // At x = 0.8 a the lower tail is about e^-2314 and underflows, but its ratio
        // to x^a e^-x/Gamma(a+1), sum x^k/((a+1)...(a+k)), is about 1/(1 - x/a) = 5;
        // likewise Q(a, 1.2 a) divided by the prefix is a/(x - a) = 5.
        let a = 1e5;
        let lower = gammainc_real(8e4, a, Tail::ScaledLower);
        assert!(close(lower, gamma_series(a, 8e4), 1e-11), "{lower}");
        assert!((lower - 5.).abs() < 0.01, "{lower}");
        let upper = gammainc_real(1.2e5, a, Tail::ScaledUpper);
        assert!(close(upper, a * gamma_fraction(a, 1.2e5), 1e-11), "{upper}");
        assert!((upper - 5.).abs() < 0.01, "{upper}");
        // across the region, both sides of x = a, against the series and fraction and,
        // where the prefix is a normal number, the unscaled tails
        for &f in &[0.7, 0.9, 0.99, 0.999, 1., 1.001, 1.01, 1.1, 1.3] {
            let x = a * f;
            let (p, q) = gamma_uniform(a, x);
            let (sp, sq) = gamma_uniform_scaled(a, x);
            let prefix = power_prefix(a, x);
            if (0.9..=1.1).contains(&f) {
                assert!(close(sp * prefix, p, 1e-9), "{f}: {sp} {p}");
                assert!(close(sq * prefix, q, 1e-9), "{f}: {sq} {q}");
            }
            if f <= 1.01 {
                assert!(close(sp, gamma_series(a, x), 1e-11), "{f}: {sp}");
            }
            if f >= 1. {
                assert!(close(sq, a * gamma_fraction(a, x), 1e-11), "{f}: {sq}");
            }
        }
        // the large tail divided by a prefix below 1e-308 overflows with the result
        assert_eq!(gammainc_real(1.2e5, a, Tail::ScaledLower), f64::INFINITY);
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn expint_series_survives_large_negative_arguments() {
        // 150-digit values of the series DLMF 6.6.2 (tool/special-fns-checks/e1-reference.py).
        // The terms of E1(-708+i) peak near 4.5e305, so power * w alone reached 3e308.
        let cases = [
            (
                (-708., 1.),
                (-2.315_614_338_004_725_2e304, 3.595_160_781_355_368e304),
            ),
            (
                (-715., 1.),
                (-2.514_429_402_409_967_5e307, 3.903_954_642_229_246e307),
            ),
            ((-715., 0.), (-4.643_625_670_370_571e307, -PI)),
            ((-710., 0.), (-3.150_915_688_206_201_4e305, -PI)),
        ];
        for ((x, y), (re, im)) in cases {
            let e = expint_value(c(x, y));
            let error = (e - c(re, im)).norm() / c(re, im).norm();
            assert!(error < 1e-13, "E1({x}+{y}i) = {e}: {error:e}");
        }
        // Past the largest double the real part is -Inf, not NaN.
        let e = expint_value(c(-800., 0.));
        assert!(e.re == f64::NEG_INFINITY && close(e.im, -PI, 1e-15), "{e}");
    }

    #[test]
    fn tail_root_does_not_take_nan_for_convergence() {
        let curvature = |x: f64| 1. / x;
        // x^2 rising, not evaluable on [0.45, 0.55), which holds the start: the
        // retreat toward the lower end finds the root 0.3 outside it.
        let hole = |x: f64| (0.45..0.55).contains(&x);
        let rising = |x: f64| {
            if hole(x) {
                (f64::NAN, f64::NAN)
            } else {
                (x * x, 2. * x)
            }
        };
        let root = tail_root(0.5, (0., 1.), true, 0.09, rising, curvature);
        assert!(close(root, 0.3, 1e-14), "{root}");
        // 1 - x^2 falling: the retreat goes toward the upper end, root 0.9.
        let falling = |x: f64| {
            if hole(x) {
                (f64::NAN, f64::NAN)
            } else {
                (1. - x * x, -2. * x)
            }
        };
        let root = tail_root(0.5, (0., 1.), false, 0.19, falling, curvature);
        assert!(close(root, 0.9, 1e-14), "{root}");
        // A root inside the hole (0.5), or no evaluable point at all, is NaN.
        assert!(tail_root(0.5, (0., 1.), true, 0.25, rising, curvature).is_nan());
        let nothing = |_: f64| (f64::NAN, f64::NAN);
        assert!(tail_root(0.5, (0., 1.), true, 0.09, nothing, curvature).is_nan());
    }

    #[test]
    fn betaincinv_with_huge_parameters_gives_a_root_of_betainc() {
        // At x = a/(a+b) the fraction of DLMF 8.17.22 needs about 4e5 terms for
        // a = b = 1e14, past the 1e5 that `lentz` allows `gamma_fraction`: it gave
        // NaN, which `tail_root` took for convergence, so the start came back.
        let (lower, upper) = betainc_pair(0.5, 1e14, 1e14);
        assert!(
            close(lower, 0.5, 1e-7) && close(upper, 0.5, 1e-7),
            "{lower}"
        );
        for &upper_tail in &[false, true] {
            let x = betaincinv_real(0.5, 1e14, 1e14, upper_tail);
            // the standard deviation of the beta distribution here is 3.5e-8
            assert!((x - 0.5).abs() < 1e-9, "{x}");
            let (lower, upper) = betainc_pair(x, 1e14, 1e14);
            let back = if upper_tail { upper } else { lower };
            assert!((back - 0.5).abs() < 1e-7, "{back}");
        }
    }

    #[test]
    fn half_integer_orders_have_closed_forms() {
        // J_{1/2}(x) = sqrt(2/(pi x)) sin x, Y_{1/2}(x) = -sqrt(2/(pi x)) cos x.
        for &x in &[0.3, 1.5, 4., 30.] {
            let (j, y) = jy_real(0.5, x);
            let f = (2. / (PI * x)).sqrt();
            assert!((j - f * x.sin()).abs() < 1e-14 * f, "{x}");
            assert!((y + f * x.cos()).abs() < 1e-14 * f, "{x}");
        }
    }

    #[test]
    fn real_bessel_pairs_match_independent_references() {
        // mpmath 1.3.0, 80 decimal digits, rounded to binary64; not MATLAB
        // oracle evidence. Reproduce with tool/cleanroom-bessel/unit_values.py.
        // Covers the Temme/CF boundary, forward and backward order recurrence,
        // the turning point, and the cancellation-sensitive near-integer case.
        let cases = [
            (
                0.,
                2.,
                [
                    0.223_890_779_141_235_67,
                    0.510_375_672_649_745_1,
                    0.308_508_322_553_671_05,
                    0.841_568_215_070_771_4,
                ],
            ),
            (
                1.,
                10.,
                [
                    0.043_472_746_168_861_44,
                    0.249_015_424_206_953_88,
                    0.121_262_681_384_455_52,
                    0.410_766_570_595_788_75,
                ],
            ),
            (
                1.5,
                2.,
                [
                    0.491_293_778_687_162_34,
                    -0.395_623_281_358_703_5,
                    0.148_797_515_394_723_58,
                    1.329_340_388_179_137,
                ],
            ),
            (
                2.7,
                0.3,
                [
                    0.001_421_018_648_372_153,
                    -83.569_626_864_374_43,
                    0.001_065_598_001_995_147_6,
                    172.564_792_746_247_86,
                ],
            ),
            (
                1. / 3.,
                7.5,
                [
                    0.289_676_596_292_362_53,
                    -0.029_463_011_809_224_77,
                    0.147_135_010_921_375_9,
                    0.453_675_698_972_991_54,
                ],
            ),
            (
                1.000_000_000_001,
                2.,
                [
                    0.576_724_807_756_817_2,
                    -0.107_032_431_541_588_34,
                    0.215_269_289_248_802_31,
                    1.033_476_847_069_109_4,
                ],
            ),
            (
                0.,
                32.,
                [
                    0.138_079_009_746_555_93,
                    -0.028_742_484_654_433_298,
                    0.070_804_189_311_245_6,
                    0.220_706_015_562_304_43,
                ],
            ),
            (
                50.,
                10.,
                [
                    1.784_513_607_871_595_3e-30,
                    -3.641_066_501_800_740_4e27,
                    2.159_626_789_445_448e-34,
                    4.540_477_895_392_307e31,
                ],
            ),
            (
                100.,
                100.,
                [
                    0.096_366_673_295_861_56,
                    -0.166_921_411_417_576_5,
                    1.726_686_262_816_769_7e-22,
                    2.047_573_673_116_675_7e19,
                ],
            ),
            (
                200.,
                100.,
                [
                    2.059_442_493_941_168e-41,
                    -8.923_664_812_553_052e37,
                    5.191_015_816_987_435e-74,
                    4.307_566_577_084_336e70,
                ],
            ),
        ];
        for (nu, x, expected) in cases {
            let (j, y) = jy_real(nu, x);
            let (i, k) = ik_real(nu, x);
            for (got, want) in [j, y, i, k].into_iter().zip(expected) {
                assert!(
                    (got - want).abs() <= 5e-14 * want.abs(),
                    "nu={nu}, x={x}: {got:e}, expected {want:e}"
                );
            }
        }
    }

    #[test]
    fn power_series_leading_factor_is_accurate_at_high_orders() {
        // J and I below x = 2 are the leading factor (x/2)^nu / Gamma(nu + 1)
        // times a short sum, so their relative error is that factor's. The
        // reference values are mpmath 1.3.0 at 80 digits (reproduce with
        // tool/cleanroom-bessel/tight.py, which scans the whole region). A
        // log-difference exp(nu ln(x/2) - ln Gamma(nu + 1)) is off by 5e-14 to
        // 1.4e-13 on the first three.
        let cases = [
            (Kind::J, 100., 1., 8.431_828_789_626_709e-189),
            (Kind::J, 150., 1.99, 8.198_250_920_221_38e-264),
            (Kind::I, 150., 1.99, 8.306_461_808_337_775e-264),
            (Kind::J, 50., 0.1, 2.920_142_569_099_643_7e-130),
            (Kind::I, 75., 1.5, 1.730_602_100_288_172_4e-119),
        ];
        for (kind, nu, x, want) in cases {
            let got = bessel(kind, nu, c(x, 0.), false).re;
            assert!(
                (got - want).abs() <= 1e-14 * want,
                "order {nu}, x={x}: {got:e}, expected {want:e}"
            );
        }
    }

    #[test]
    fn modified_half_integer_scaling_and_extreme_orders() {
        // DLMF 10.39.1-10.39.2, with expm1 retaining the small-x difference.
        for x in [1e-100, 0.3, 1.999_999_999, 2., 4., 32., 1000.] {
            let (i, k) = ik_real(0.5, x);
            let expected_i = -libm::expm1(-2. * x) / (2. * PI * x).sqrt();
            let expected_k = (PI / (2. * x)).sqrt();
            assert!((i - expected_i).abs() < 5e-14 * expected_i, "{x}: I");
            assert!((k - expected_k).abs() < 5e-14 * expected_k, "{x}: K");
        }
        for x in [1e-300, 1., 2., 100.] {
            assert_eq!(jy_real(10_000., x), (0., f64::NEG_INFINITY));
            assert_eq!(ik_real(10_000., x), (0., f64::INFINITY));
        }
        let x = 1e308;
        let (j, y) = jy_real(0.5, x);
        let amplitude = (2. / PI).sqrt() / x.sqrt();
        assert!((j - amplitude * libm::sin(x)).abs() <= 1e-15 * amplitude);
        assert!((y + amplitude * libm::cos(x)).abs() <= 1e-15 * amplitude);
        let (i, k) = ik_real(0.5, x);
        assert!((i * x.sqrt() - 1. / (2. * PI).sqrt()).abs() < 1e-15);
        assert!((k * x.sqrt() - (PI / 2.).sqrt()).abs() < 1e-15);
    }

    #[test]
    fn large_oscillatory_arguments_use_bounded_order_recurrence() {
        // Independent 80-digit mpmath values. CF1 would need O(x) steps here,
        // exceeding MAXIT at x=1e6; forward recurrence needs at most nu steps.
        for (nu, x, expected_j, expected_y) in [
            (
                1000.,
                1e5,
                0.001_283_178_112_502_480_4,
                0.002_172_549_191_376_840_4,
            ),
            (
                10_000.,
                1e6,
                0.000_129_300_688_776_859_8,
                -0.000_787_358_201_599_166,
            ),
        ] {
            let (j, y) = jy_real(nu, x);
            assert!((j - expected_j).abs() < 5e-13 * expected_j.abs());
            assert!((y - expected_y).abs() < 5e-13 * expected_y.abs());
        }
    }

    #[test]
    fn temme_gamma_parts_are_regular_at_zero_and_half_orders() {
        let (odd, even, plus, minus) = temme_gammas(0.);
        assert_eq!(odd, -EULER);
        assert_eq!((even, plus, minus), (1., 1., 1.));
        for mu in [-0.5, -0.499_999_999, -1e-12, 1e-12, 0.499_999_999, 0.5] {
            let (odd, even, plus, minus) = temme_gammas(mu);
            let reflected = temme_gammas(-mu);
            assert_eq!(
                (odd, even, plus, minus),
                (reflected.0, reflected.1, reflected.3, reflected.2)
            );
            assert!((plus - 1. / libm::tgamma(1. + mu)).abs() < 8e-16);
            assert!((minus - 1. / libm::tgamma(1. - mu)).abs() < 8e-16);
        }
    }
}
