//! Scalar kernels of the families added for the distribution objects
//! (logistic, log-logistic, Rician, generalized extreme value, generalized
//! Pareto, inverse Gaussian), and their moments. The simpler new families
//! (t location-scale, Nakagami, half normal, Birnbaum-Saunders) reuse the
//! kernels of `stats_dist/families.rs` directly in its match arms.
//!
//! Sources (published mathematics only): Johnson, Kotz and Balakrishnan,
//! "Continuous Univariate Distributions" (2nd ed.), chapters 15 (inverse
//! Gaussian), 22 (extreme value), 23 (logistic); the Rician distribution
//! function as the Poisson mixture of noncentral chi-square laws with two
//! degrees of freedom (JKB ch. 29), and for s/sigma >= 1000 its expansion in
//! sigma/s from the Bessel asymptotics DLMF 10.40.1; DLMF 10.25 (modified
//! Bessel functions);
//! J. R. Michael, W. R. Schucany and R. W. Haas, "Generating random variates
//! using transformations with multiple roots", Amer. Statist. 30 (1976).
use super::super::special_fns::{Tail, erfcx_value, gammainc_real, ik_real};
use super::super::stats_dist::families::Family;
use std::f64::consts::{FRAC_1_SQRT_2, PI};

/// e^-x I0(x) and e^-x I1(x) for x >= 0.
pub(in crate::engine) fn bessel_i01_scaled(x: f64) -> (f64, f64) {
    if x == 0. {
        return (1., 0.);
    }
    if x.is_infinite() {
        return (0., 0.);
    }
    (ik_real(0., x).0, ik_real(1., x).0)
}

fn logistic_pdf(z: f64) -> f64 {
    let e = libm::exp(-z.abs());
    e / ((1. + e) * (1. + e))
}

/// (lower, upper) tails of the standard logistic law.
fn logistic_tails(z: f64) -> (f64, f64) {
    (1. / (1. + libm::exp(-z)), 1. / (1. + libm::exp(z)))
}

/// log(p / (1 - p)) for p in [0, 1].
pub(in crate::engine) fn logit(p: f64) -> f64 {
    libm::log(p) - libm::log1p(-p)
}

/// Density at x for the families without an arm in `Family::pdf`.
pub(in crate::engine) fn pdf(f: Family, x: f64, q: &[f64]) -> f64 {
    match f {
        Family::Logistic => logistic_pdf((x - q[0]) / q[1]) / q[1],
        Family::Loglogistic => {
            if x <= 0. || x.is_infinite() {
                0.
            } else {
                logistic_pdf((libm::log(x) - q[0]) / q[1]) / (q[1] * x)
            }
        }
        Family::Rician => {
            if x <= 0. || x.is_infinite() {
                0.
            } else {
                let s2 = q[1] * q[1];
                let (i0, _) = bessel_i01_scaled(x * q[0] / s2);
                x / s2 * libm::exp(-(x - q[0]) * (x - q[0]) / (2. * s2)) * i0
            }
        }
        Family::Gev => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            if k == 0. {
                if z.is_infinite() {
                    return 0.;
                }
                return libm::exp(-z - libm::exp(-z)) / sigma;
            }
            let kz = k * z;
            if kz <= -1. || z.is_infinite() {
                return 0.;
            }
            let lu = libm::log1p(kz);
            libm::exp(-(1. + 1. / k) * lu - libm::exp(-lu / k)) / sigma
        }
        Family::Gp => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            if z < 0. || z.is_infinite() {
                return 0.;
            }
            if k == 0. {
                return libm::exp(-z) / sigma;
            }
            let kz = k * z;
            if kz <= -1. {
                return 0.;
            }
            libm::exp(-(1. + 1. / k) * libm::log1p(kz)) / sigma
        }
        Family::InvGauss => {
            let (mu, lambda) = (q[0], q[1]);
            if x <= 0. || x.is_infinite() {
                return 0.;
            }
            (lambda / (2. * PI * x * x * x)).sqrt()
                * libm::exp(-lambda * (x - mu) * (x - mu) / (2. * mu * mu * x))
        }
        _ => f64::NAN,
    }
}

/// (P(X <= x), P(X > x)) for the families without an arm in `Family::cdf`.
pub(in crate::engine) fn tails(f: Family, x: f64, q: &[f64]) -> (f64, f64) {
    match f {
        Family::Logistic => logistic_tails((x - q[0]) / q[1]),
        Family::Loglogistic => {
            if x <= 0. {
                (0., 1.)
            } else {
                logistic_tails((libm::log(x) - q[0]) / q[1])
            }
        }
        Family::Rician => rician_tails(x, q[0], q[1]),
        Family::Gev => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            let t = if k == 0. {
                libm::exp(-z)
            } else {
                let kz = k * z;
                if kz <= -1. {
                    // Outside the support: below it for k > 0, above for k < 0.
                    return if k > 0. { (0., 1.) } else { (1., 0.) };
                }
                libm::exp(-libm::log1p(kz) / k)
            };
            (libm::exp(-t), -libm::expm1(-t))
        }
        Family::Gp => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            if z <= 0. {
                return (0., 1.);
            }
            let s = if k == 0. {
                -z
            } else {
                let kz = k * z;
                if kz <= -1. {
                    return (1., 0.);
                }
                -libm::log1p(kz) / k
            };
            (-libm::expm1(s), libm::exp(s))
        }
        Family::InvGauss => {
            let (mu, lambda) = (q[0], q[1]);
            if x <= 0. {
                return (0., 1.);
            }
            if x.is_infinite() {
                return (1., 0.);
            }
            let a = (lambda / x).sqrt();
            let z1 = a * (x / mu - 1.);
            let z2 = a * (x / mu + 1.);
            // e^(2 lambda/mu) Phi(-z2) = erfcx(z2/sqrt2)/2 e^(-z1^2/2) (no overflow).
            let half_square = lambda * (x - mu) * (x - mu) / (2. * mu * mu * x);
            let decay = libm::exp(-half_square);
            if z1 <= 0. {
                let lower = 0.5 * libm::erfc(-z1 * FRAC_1_SQRT_2)
                    + 0.5 * erfcx_value(z2 * FRAC_1_SQRT_2) * decay;
                (lower, 1. - lower)
            } else {
                let upper = 0.5
                    * decay
                    * (erfcx_value(z1 * FRAC_1_SQRT_2) - erfcx_value(z2 * FRAC_1_SQRT_2));
                (1. - upper, upper)
            }
        }
        _ => (f64::NAN, f64::NAN),
    }
}

/// Rician tails: (X/sigma)^2 is noncentral chi-square with 2 degrees of
/// freedom and noncentrality (s/sigma)^2, a Poisson(m = s^2/(2 sigma^2))
/// mixture of Gamma(1 + j) laws evaluated at x^2/(2 sigma^2). The sum starts at
/// the Poisson mode and walks out both ways until the weights are negligible.
/// From s/sigma >= 1000 (Poisson mode >= 5e5) that walk would need ~9 sqrt(m)
/// terms each way, so the uniform large-noncentrality expansion takes over.
fn rician_tails(x: f64, s: f64, sigma: f64) -> (f64, f64) {
    if x <= 0. {
        return (0., 1.);
    }
    if x.is_infinite() {
        return (1., 0.);
    }
    let a = s.abs() / sigma;
    if a >= RICIAN_ASYMPTOTIC_SNR {
        return rician_tails_large(x / sigma, a);
    }
    let y = x * x / (2. * sigma * sigma);
    let m = s * s / (2. * sigma * sigma);
    if m == 0. {
        return (-libm::expm1(-y), libm::exp(-y));
    }
    let weight = |j: f64| libm::exp(-m + j * libm::log(m) - libm::lgamma(j + 1.));
    let mode = m.floor();
    let (mut lower, mut upper) = (0., 0.);
    let mut add = |j: f64| {
        let w = weight(j);
        lower += w * gammainc_real(y, j + 1., Tail::Lower);
        upper += w * gammainc_real(y, j + 1., Tail::Upper);
        w
    };
    let spread = 40. + 40. * m.sqrt();
    let mut j = mode;
    while j <= mode + spread {
        let w = add(j);
        if w < 1e-18 && j > mode + 2. {
            break;
        }
        j += 1.;
    }
    let mut j = mode - 1.;
    while j >= 0. && j >= mode - spread {
        let w = add(j);
        if w < 1e-18 {
            break;
        }
        j -= 1.;
    }
    (lower.min(1.), upper.min(1.))
}

/// s/sigma from which `rician_tails` uses `rician_tails_large`; there the
/// truncation error is below 1e-13 absolute (1e-7 relative in the tails) against
/// mpmath quadrature of the pdf (tool/refs/stats/rician_large.py).
const RICIAN_ASYMPTOTIC_SNR: f64 = 1000.;

/// Rician tails at u = x/sigma for noncentrality a = s/sigma >> 1. With
/// I0(z) e^-z = (2 pi z)^-1/2 (1 + 1/(8z) + ...) the density of u is
/// sqrt(u/a) phi(u - a) (1 + 1/(8au) + ...); expanding in 1/a and integrating
/// term by term against phi gives, with t = u - a,
/// F(u) = Phi(t) + phi(t) (-1/(2a) + t/(8a^2) - (t^2 + 1)/(16a^3)) + O(a^-4).
fn rician_tails_large(u: f64, a: f64) -> (f64, f64) {
    let t = u - a;
    let pdf = libm::exp(-0.5 * t * t) / (2. * PI).sqrt();
    let c = if pdf > 0. {
        pdf * (-0.5 / a + t / (8. * a * a) - (t * t + 1.) / (16. * a * a * a))
    } else {
        0.
    };
    let lower = 0.5 * libm::erfc(-t * FRAC_1_SQRT_2) + c;
    let upper = 0.5 * libm::erfc(t * FRAC_1_SQRT_2) - c;
    (lower.clamp(0., 1.), upper.clamp(0., 1.))
}

/// Quantile start (or the exact quantile) for the families without an arm in
/// `Family::inv`; NaN asks the caller to bisect.
pub(in crate::engine) fn quantile(f: Family, p: f64, q: &[f64]) -> f64 {
    match f {
        Family::Logistic => q[0] + q[1] * logit(p),
        Family::Loglogistic => libm::exp(q[0] + q[1] * logit(p)),
        Family::Gev => {
            let (k, sigma, mu) = (q[0], q[1], q[2]);
            let l = libm::log(-libm::log(p));
            if k == 0. {
                mu - sigma * l
            } else {
                mu + sigma * libm::expm1(-k * l) / k
            }
        }
        Family::Gp => {
            let (k, sigma, theta) = (q[0], q[1], q[2]);
            let l = libm::log1p(-p);
            if k == 0. {
                theta - sigma * l
            } else {
                theta + sigma * libm::expm1(-k * l) / k
            }
        }
        Family::Rician | Family::InvGauss => {
            if p == 0. {
                0.
            } else if p == 1. {
                f64::INFINITY
            } else {
                f64::NAN
            }
        }
        _ => f64::NAN,
    }
}

/// One inverse Gaussian variate from a standard normal n and a uniform u
/// (Michael, Schucany and Haas 1976).
pub(in crate::engine) fn inverse_gaussian_draw(mu: f64, lambda: f64, n: f64, u: f64) -> f64 {
    let v = n * n;
    let y = mu + mu * mu * v / (2. * lambda)
        - mu / (2. * lambda) * (4. * mu * lambda * v + mu * mu * v * v).sqrt();
    if u <= mu / (mu + y) { y } else { mu * mu / y }
}

/// (mean, variance) of the families added for the distribution objects.
pub(in crate::engine) fn moments(f: Family, q: &[f64]) -> (f64, f64) {
    let inf = f64::INFINITY;
    match f {
        Family::Logistic => (q[0], (PI * q[1]).powi(2) / 3.),
        Family::Loglogistic => {
            let (mu, s) = (q[0], q[1]);
            let e = libm::exp(mu);
            let m = if s < 1. {
                e * PI * s / libm::sin(PI * s)
            } else {
                inf
            };
            let v = if s < 0.5 {
                let b = PI * s / libm::sin(PI * s);
                e * e * (2. * PI * s / libm::sin(2. * PI * s) - b * b)
            } else {
                inf
            };
            (m, v)
        }
        Family::Rician => {
            let (s, sigma) = (q[0], q[1]);
            let nu = s * s / (2. * sigma * sigma);
            // Laguerre L_{1/2}(-nu) = (1 + nu) I0e(nu/2) + nu I1e(nu/2).
            let (i0, i1) = bessel_i01_scaled(nu / 2.);
            let mean = sigma * (PI / 2.).sqrt() * ((1. + nu) * i0 + nu * i1);
            (mean, 2. * sigma * sigma + s * s - mean * mean)
        }
        Family::Nakagami => {
            let (mu, omega) = (q[0], q[1]);
            let r = libm::exp(libm::lgamma(mu + 0.5) - libm::lgamma(mu));
            let mean = r * (omega / mu).sqrt();
            (mean, omega * (1. - r * r / mu))
        }
        Family::Gev => {
            let (k, sigma, mu) = (q[0], q[1], q[2]);
            let g = |a: f64| libm::tgamma(a);
            let mean = if k == 0. {
                mu + sigma * std::f64::consts::EULER_GAMMA
            } else if k < 1. {
                mu + sigma * (g(1. - k) - 1.) / k
            } else {
                inf
            };
            let var = if k == 0. {
                (PI * sigma).powi(2) / 6.
            } else if k < 0.5 {
                let g1 = g(1. - k);
                sigma * sigma * (g(1. - 2. * k) - g1 * g1) / (k * k)
            } else {
                inf
            };
            (mean, var)
        }
        Family::Gp => {
            let (k, sigma, theta) = (q[0], q[1], q[2]);
            let mean = if k < 1. {
                theta + sigma / (1. - k)
            } else {
                inf
            };
            let var = if k < 0.5 {
                sigma * sigma / ((1. - k) * (1. - k) * (1. - 2. * k))
            } else {
                inf
            };
            (mean, var)
        }
        Family::Tls => {
            let (mu, sigma, nu) = (q[0], q[1], q[2]);
            let mean = if nu > 1. { mu } else { f64::NAN };
            let var = if nu > 2. {
                sigma * sigma * nu / (nu - 2.)
            } else if nu > 1. {
                inf
            } else {
                f64::NAN
            };
            (mean, var)
        }
        Family::InvGauss => (q[0], q[0].powi(3) / q[1]),
        Family::Bisa => {
            let (b, g) = (q[0], q[1]);
            (b * (1. + g * g / 2.), (b * g).powi(2) * (1. + 1.25 * g * g))
        }
        Family::HalfNorm => (q[0] + q[1] * (2. / PI).sqrt(), q[1] * q[1] * (1. - 2. / PI)),
        _ => (f64::NAN, f64::NAN),
    }
}
