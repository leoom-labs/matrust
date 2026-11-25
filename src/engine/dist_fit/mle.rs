//! Maximum likelihood fits of the parametric families with right censoring
//! and frequencies, their negative log-likelihoods, covariances (inverse of
//! the observed information) and confidence intervals.
//!
//! Closed forms: Normal/Lognormal (unbiased sigma as normfit/lognfit when not
//! censored), Exponential, Poisson, Binomial, Rayleigh, half normal and
//! inverse Gaussian (uncensored). One-dimensional score equations solved by
//! bisection: Gamma shape (log a - psi(a) = log(mean) - mean(log x)), Weibull
//! shape and extreme-value scale (profile likelihoods, censoring included),
//! negative binomial R. Everything else: Nelder-Mead on log/natural scales
//! followed by Newton steps. Exact intervals as normfit, expfit (chi-square
//! with 2r degrees of freedom), poissfit (Garwood), binofit (Clopper-Pearson)
//! and raylfit; Wald intervals otherwise, on the log scale for positive
//! parameters and the logit scale for probabilities.
use super::super::Engine;
use super::super::special_fns::{erfcx_value, polygamma};
use super::super::stats_dist::families::Family;
use super::families::{bessel_i01_scaled, logit};
use super::optimize::{self, Limits};
use super::sample::Sample;
use crate::Result;
use std::f64::consts::{FRAC_1_SQRT_2, LN_2, PI};

const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

/// How a parameter is estimated and how its Wald interval is formed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    /// Location, measured in units of the scale parameter at this index.
    Loc(usize),
    /// Positive: log scale.
    Pos,
    /// Real shape (GEV, GP k): natural scale.
    Shape,
    /// Probability: logit scale.
    Prob,
    Fixed,
}

/// The families fitdist and mle can fit, with their parameter kinds.
pub(super) fn kinds(f: Family) -> Option<&'static [Kind]> {
    use Kind::*;
    Some(match f {
        Family::Norm | Family::Logn | Family::Ev | Family::Logistic | Family::Loglogistic => {
            &[Loc(1), Pos]
        }
        Family::Exp | Family::Rayl | Family::Poiss => &[Pos],
        Family::Gam
        | Family::Beta
        | Family::Wbl
        | Family::Rician
        | Family::Nakagami
        | Family::InvGauss
        | Family::Bisa => &[Pos, Pos],
        Family::Bino => &[Fixed, Prob],
        Family::Nbin => &[Pos, Prob],
        Family::Gev => &[Shape, Pos, Loc(1)],
        Family::Gp => &[Shape, Pos, Fixed],
        Family::Tls => &[Loc(1), Pos, Pos],
        Family::HalfNorm => &[Fixed, Pos],
        Family::Unif | Family::Chi2 | Family::T | Family::F | Family::Geo => return None,
    })
}

pub(super) const FITTABLE: &str = "Normal, Exponential, Gamma, Weibull, Lognormal, Poisson, Binomial, Beta, Rayleigh, ExtremeValue, NegativeBinomial, Logistic, Loglogistic, Rician, Nakagami, GeneralizedExtremeValue, GeneralizedPareto, tLocationScale, InverseGaussian, BirnbaumSaunders, HalfNormal and Kernel";

/// Fixed parameters and settings of one fit.
#[derive(Clone, Copy, Debug)]
pub(super) struct Options {
    pub ntrials: Option<f64>,
    pub theta: f64,
    pub mu: f64,
    pub limits: Limits,
    /// Normal and lognormal sigma divided by n - 1 (fitdist, normfit,
    /// lognfit) rather than n (mle).
    pub unbiased: bool,
}

impl Options {
    pub fn new(unbiased: bool) -> Self {
        Self {
            ntrials: None,
            theta: 0.,
            mu: 0.,
            limits: Limits::family(3),
            unbiased,
        }
    }
}

// ---------- log densities ----------

fn norm_logsf(z: f64) -> f64 {
    if z < 0. {
        libm::log(0.5 * libm::erfc(z * FRAC_1_SQRT_2))
    } else {
        libm::log(0.5 * erfcx_value(z * FRAC_1_SQRT_2)) - z * z / 2.
    }
}

fn logistic_logpdf(z: f64) -> f64 {
    let a = z.abs();
    -a - 2. * libm::log1p(libm::exp(-a))
}

fn logistic_logsf(z: f64) -> f64 {
    if z > 0. {
        -z - libm::log1p(libm::exp(-z))
    } else {
        -libm::log1p(libm::exp(z))
    }
}

/// log of (x+R-1)!/((R-1)! ) for an integer count x, summed for small x.
fn ln_rising(r: f64, x: f64) -> f64 {
    if x <= 1000. {
        let mut s = 0.;
        let mut j = x - 1.;
        while j >= 0. {
            s += libm::log(r + j);
            j -= 1.;
        }
        s
    } else {
        libm::lgamma(x + r) - libm::lgamma(r)
    }
}

/// log density (or log mass) at x with valid parameters q.
pub(super) fn logpdf(f: Family, x: f64, q: &[f64]) -> f64 {
    let ninf = f64::NEG_INFINITY;
    match f {
        Family::Norm => {
            let z = (x - q[0]) / q[1];
            -0.5 * z * z - libm::log(q[1]) - LN_SQRT_2PI
        }
        Family::Logn => {
            if x <= 0. {
                return ninf;
            }
            let lx = libm::log(x);
            let z = (lx - q[0]) / q[1];
            -0.5 * z * z - libm::log(q[1]) - lx - LN_SQRT_2PI
        }
        Family::HalfNorm => {
            if x < q[0] {
                return ninf;
            }
            let z = (x - q[0]) / q[1];
            LN_2 - 0.5 * z * z - libm::log(q[1]) - LN_SQRT_2PI
        }
        Family::Exp => {
            if x < 0. {
                ninf
            } else {
                -x / q[0] - libm::log(q[0])
            }
        }
        Family::Gam => {
            let (a, b) = (q[0], q[1]);
            if x < 0. {
                return ninf;
            }
            if x == 0. {
                return if a == 1. {
                    -libm::log(b)
                } else if a < 1. {
                    f64::INFINITY
                } else {
                    ninf
                };
            }
            (a - 1.) * libm::log(x) - x / b - libm::lgamma(a) - a * libm::log(b)
        }
        Family::Wbl => {
            let (a, b) = (q[0], q[1]);
            if x <= 0. {
                return if x == 0. && b == 1. {
                    -libm::log(a)
                } else {
                    ninf
                };
            }
            let lz = libm::log(x / a);
            libm::log(b / a) + (b - 1.) * lz - libm::exp(b * lz)
        }
        Family::Rayl => {
            if x <= 0. {
                return ninf;
            }
            libm::log(x) - 2. * libm::log(q[0]) - x * x / (2. * q[0] * q[0])
        }
        Family::Ev => {
            let z = (x - q[0]) / q[1];
            z - libm::exp(z) - libm::log(q[1])
        }
        Family::Beta => {
            let (a, b) = (q[0], q[1]);
            if x <= 0. || x >= 1. {
                return ninf;
            }
            (a - 1.) * libm::log(x) + (b - 1.) * libm::log1p(-x) - libm::lgamma(a) - libm::lgamma(b)
                + libm::lgamma(a + b)
        }
        Family::Logistic => logistic_logpdf((x - q[0]) / q[1]) - libm::log(q[1]),
        Family::Loglogistic => {
            if x <= 0. {
                return ninf;
            }
            let lx = libm::log(x);
            logistic_logpdf((lx - q[0]) / q[1]) - libm::log(q[1]) - lx
        }
        Family::Gev => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            if k == 0. {
                return -z - libm::exp(-z) - libm::log(sigma);
            }
            let kz = k * z;
            if kz <= -1. {
                return ninf;
            }
            let lu = libm::log1p(kz);
            -(1. + 1. / k) * lu - libm::exp(-lu / k) - libm::log(sigma)
        }
        Family::Gp => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            if z < 0. {
                return ninf;
            }
            if k == 0. {
                return -z - libm::log(sigma);
            }
            let kz = k * z;
            if kz <= -1. {
                return ninf;
            }
            -(1. + 1. / k) * libm::log1p(kz) - libm::log(sigma)
        }
        Family::Tls => {
            let (sigma, nu) = (q[1], q[2]);
            let z = (x - q[0]) / sigma;
            libm::lgamma((nu + 1.) / 2.)
                - libm::lgamma(nu / 2.)
                - 0.5 * libm::log(nu * PI)
                - (nu + 1.) / 2. * libm::log1p(z * z / nu)
                - libm::log(sigma)
        }
        Family::InvGauss => {
            let (mu, lambda) = (q[0], q[1]);
            if x <= 0. {
                return ninf;
            }
            0.5 * libm::log(lambda / (2. * PI * x * x * x))
                - lambda * (x - mu) * (x - mu) / (2. * mu * mu * x)
        }
        Family::Bisa => {
            let (b, g) = (q[0], q[1]);
            if x <= 0. {
                return ninf;
            }
            let (r, s) = ((x / b).sqrt(), (b / x).sqrt());
            let z = (r - s) / g;
            libm::log((r + s) / (2. * g * x)) - 0.5 * z * z - LN_SQRT_2PI
        }
        Family::Nakagami => {
            let (mu, omega) = (q[0], q[1]);
            if x <= 0. {
                return ninf;
            }
            LN_2 + mu * libm::log(mu / omega) - libm::lgamma(mu) + (2. * mu - 1.) * libm::log(x)
                - mu * x * x / omega
        }
        Family::Rician => {
            let (s, sigma) = (q[0], q[1]);
            if x <= 0. {
                return ninf;
            }
            let s2 = sigma * sigma;
            let (i0, _) = bessel_i01_scaled(x * s / s2);
            libm::log(x / s2) - (x - s) * (x - s) / (2. * s2) + libm::log(i0)
        }
        Family::Poiss => {
            let l = q[0];
            if x < 0. || x.fract() != 0. {
                return ninf;
            }
            if l == 0. {
                return if x == 0. { 0. } else { ninf };
            }
            x * libm::log(l) - l - libm::lgamma(x + 1.)
        }
        Family::Nbin => {
            let (r, p) = (q[0], q[1]);
            if x < 0. || x.fract() != 0. {
                return ninf;
            }
            ln_rising(r, x) - libm::lgamma(x + 1.) + r * libm::log(p) + x * libm::log1p(-p)
        }
        _ => libm::log(f.pdf(x, q)),
    }
}

/// log of the survival function P(X > x).
pub(super) fn logsf(f: Family, x: f64, q: &[f64]) -> f64 {
    match f {
        Family::Norm => norm_logsf((x - q[0]) / q[1]),
        Family::Logn => {
            if x <= 0. {
                0.
            } else {
                norm_logsf((libm::log(x) - q[0]) / q[1])
            }
        }
        Family::HalfNorm => {
            if x <= q[0] {
                0.
            } else {
                LN_2 + norm_logsf((x - q[0]) / q[1])
            }
        }
        Family::Exp => {
            if x <= 0. {
                0.
            } else {
                -x / q[0]
            }
        }
        Family::Wbl => {
            if x <= 0. {
                0.
            } else {
                -libm::pow(x / q[0], q[1])
            }
        }
        Family::Rayl => {
            if x <= 0. {
                0.
            } else {
                -x * x / (2. * q[0] * q[0])
            }
        }
        Family::Ev => -libm::exp((x - q[0]) / q[1]),
        Family::Logistic => logistic_logsf((x - q[0]) / q[1]),
        Family::Loglogistic => {
            if x <= 0. {
                0.
            } else {
                logistic_logsf((libm::log(x) - q[0]) / q[1])
            }
        }
        Family::Gp => {
            let (k, sigma) = (q[0], q[1]);
            let z = (x - q[2]) / sigma;
            if z <= 0. {
                return 0.;
            }
            if k == 0. {
                return -z;
            }
            let kz = k * z;
            if kz <= -1. {
                f64::NEG_INFINITY
            } else {
                -libm::log1p(kz) / k
            }
        }
        _ => libm::log(f.cdf(x, q, true)),
    }
}

/// Negative log-likelihood of the sample (NaN for invalid parameters).
pub(super) fn nll(f: Family, q: &[f64], s: &Sample) -> f64 {
    if !f.valid(q) {
        return f64::NAN;
    }
    let mut total = 0.;
    for (x, c, w) in s.iter() {
        let l = if c { logsf(f, x, q) } else { logpdf(f, x, q) };
        total -= w * l;
    }
    total
}

// ---------- fits ----------

fn support_error(who: &str, f: Family, what: &str) -> String {
    format!("{who}: {} observations must be {what}", title(f))
}

pub(super) fn title(f: Family) -> &'static str {
    match f {
        Family::Ev => "ExtremeValue",
        Family::Nbin => "NegativeBinomial",
        _ => f.title(),
    }
}

fn check(who: &str, f: Family, s: &Sample, o: &Options) -> Result<()> {
    let all = |p: &dyn Fn(f64) -> bool| s.x.iter().all(|&v| p(v));
    match f {
        Family::Gam
        | Family::Wbl
        | Family::Logn
        | Family::Loglogistic
        | Family::Rician
        | Family::Nakagami
        | Family::InvGauss
        | Family::Bisa
        | Family::Rayl => {
            if !all(&|v| v > 0.) {
                return Err(support_error(who, f, "strictly positive"));
            }
        }
        Family::Exp => {
            if !all(&|v| v >= 0.) {
                return Err(support_error(who, f, "nonnegative"));
            }
        }
        Family::Beta => {
            if !all(&|v| v > 0. && v < 1.) {
                return Err(support_error(who, f, "strictly between 0 and 1"));
            }
        }
        Family::Gp => {
            if !all(&|v| v >= o.theta) {
                return Err(support_error(who, f, "at least the threshold Theta"));
            }
        }
        Family::HalfNorm => {
            if !all(&|v| v >= o.mu) {
                return Err(support_error(who, f, "at least the location mu"));
            }
        }
        Family::Poiss | Family::Nbin | Family::Bino => {
            let top = o.ntrials.unwrap_or(f64::INFINITY);
            if !all(&|v| v >= 0. && v.fract() == 0. && v <= top) {
                return Err(format!(
                    "{who}: observations must be integer counts within the distribution support"
                ));
            }
        }
        _ => {}
    }
    if f.discrete() && s.has_cens() {
        return Err(format!(
            "{who}: censoring is not supported for the {} distribution",
            title(f)
        ));
    }
    if s.uncensored_weight() <= 0. {
        return Err(format!(
            "{who}: at least one uncensored observation is required"
        ));
    }
    Ok(())
}

/// Bisection for an increasing score with a root in (lo, hi]; hi is
/// doubled from 1 until the score is nonnegative.
fn root(e: &Engine, who: &str, score: &mut dyn FnMut(f64) -> Result<f64>) -> Result<f64> {
    let mut lo = 1e-12;
    let mut hi = 1.;
    while score(hi)? < 0. {
        e.check_cancel()?;
        hi *= 2.;
        if hi > 1e16 {
            return Err(format!(
                "{who}: shape estimate exceeds supported numerical range"
            ));
        }
    }
    if score(lo)? > 0. {
        return Err(format!(
            "{who}: shape estimate is below supported numerical range"
        ));
    }
    for _ in 0..200 {
        e.check_cancel()?;
        let mid = lo + (hi - lo) / 2.;
        if score(mid)? < 0. {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= 4. * f64::EPSILON * hi {
            break;
        }
    }
    Ok(lo + (hi - lo) / 2.)
}

/// log(a) - psi(a): recurrence and the Bernoulli asymptotic series at a >= 16.
pub(super) fn log_minus_digamma(a: f64) -> f64 {
    if a >= 16. {
        let r = 1. / a;
        let z = r * r;
        return r / 2.
            + z * (1. / 12.
                + z * (-1. / 120.
                    + z * (1. / 252. + z * (-1. / 240. + z * (1. / 132. - z * 691. / 32760.)))));
    }
    let mut y = a;
    let mut correction = 0.;
    while y < 16. {
        correction += 1. / y;
        y += 1.;
    }
    a.ln() - y.ln() + log_minus_digamma(y) + correction
}

/// Gamma (shape, scale) MLE of an uncensored weighted sample.
fn gamma_profile(e: &Engine, who: &str, s: &Sample) -> Result<(f64, f64)> {
    let w = s.weight();
    let max = s.x.iter().copied().fold(0., f64::max);
    let mean = s.iter().map(|o| o.0 / max * o.2).sum::<f64>() / w * max;
    let avglog = s.iter().map(|o| (o.0.ln() - max.ln()) * o.2).sum::<f64>() / w;
    if s.distinct_uncensored() < 2 {
        return Err(format!(
            "{who}: constant samples have no finite shape estimate"
        ));
    }
    let gap = (mean / max).ln() - avglog;
    if gap <= 0. {
        return Err(format!(
            "{who}: sample spread is too small to estimate the Gamma shape"
        ));
    }
    let shape = root(e, who, &mut |a| Ok(gap - log_minus_digamma(a)))?;
    Ok((shape, mean / shape))
}

/// Weibull (scale, shape) from the profile score of the shape, with right
/// censoring: sum_all w x^b ln x / sum_all w x^b - 1/b - mean_unc(ln x) = 0.
fn weibull_profile(e: &Engine, who: &str, s: &Sample) -> Result<(f64, f64)> {
    if s.distinct_uncensored() < 2 && !s.has_cens() {
        return Err(format!(
            "{who}: constant samples have no finite shape estimate"
        ));
    }
    let max = s.x.iter().copied().fold(0., f64::max);
    let logs: Vec<f64> = s.x.iter().map(|v| v.ln() - max.ln()).collect();
    let r = s.uncensored_weight();
    let avglog = s
        .iter()
        .zip(&logs)
        .filter(|(o, _)| !o.1)
        .map(|(o, l)| l * o.2)
        .sum::<f64>()
        / r;
    let shape = root(e, who, &mut |b| {
        let mut sw = 0.;
        let mut sl = 0.;
        for (i, (l, w)) in logs.iter().zip(&s.freq).enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let t = w * (b * l).exp();
            sw += t;
            sl += t * l;
        }
        Ok(sl / sw - avglog - 1. / b)
    })?;
    let total: f64 = logs
        .iter()
        .zip(&s.freq)
        .map(|(l, w)| w * (shape * l).exp())
        .sum();
    Ok((max * ((total / r).ln() / shape).exp(), shape))
}

/// Extreme value (mu, sigma) from the profile score of sigma with right
/// censoring, on data shifted by the maximum and scaled by the spread.
fn ev_profile(e: &Engine, who: &str, s: &Sample) -> Result<(f64, f64)> {
    if s.distinct_uncensored() < 2 && !s.has_cens() {
        return Err(format!(
            "{who}: constant samples have no finite scale estimate"
        ));
    }
    let c = s.x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut d = s.std();
    if d <= 0. || !d.is_finite() {
        d = 1.;
    }
    let y: Vec<f64> = s.x.iter().map(|v| (v - c) / d).collect();
    let r = s.uncensored_weight();
    let mean_unc = s
        .iter()
        .zip(&y)
        .filter(|(o, _)| !o.1)
        .map(|(o, v)| v * o.2)
        .sum::<f64>()
        / r;
    let sums = |sigma: f64| {
        let (mut a, mut b) = (0., 0.);
        for (v, w) in y.iter().zip(&s.freq) {
            let t = w * (v / sigma).exp();
            a += t;
            b += t * v;
        }
        (a, b)
    };
    let sigma = root(e, who, &mut |sigma| {
        e.check_cancel()?;
        let (a, b) = sums(sigma);
        Ok(sigma + mean_unc - b / a)
    })?;
    let (a, _) = sums(sigma);
    let mu = sigma * (a / r).ln();
    Ok((c + d * mu, d * sigma))
}

/// Negative binomial (R, P): R solves
/// sum w (psi(x+R) - psi(R)) + W log(R/(R+m)) = 0, P = R/(R+m).
fn nbin_profile(e: &Engine, who: &str, s: &Sample) -> Result<(f64, f64)> {
    let w = s.weight();
    let m = s.mean();
    let v = s.iter().map(|o| o.2 * (o.0 - m) * (o.0 - m)).sum::<f64>() / w;
    if v <= m || m <= 0. {
        return Err(format!(
            "{who}: the sample variance must exceed the sample mean (no overdispersion), so the NegativeBinomial fit has no finite R"
        ));
    }
    // Distinct counts with their total frequency.
    let mut counts: Vec<(f64, f64)> = s.iter().map(|o| (o.0, o.2)).collect();
    counts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut grouped: Vec<(f64, f64)> = Vec::new();
    for (x, f) in counts {
        match grouped.last_mut() {
            Some(last) if last.0 == x => last.1 += f,
            _ => grouped.push((x, f)),
        }
    }
    let score = |r: f64| -> f64 {
        let mut total = 0.;
        for &(x, f) in &grouped {
            let d = if x <= 1e5 {
                let mut acc = 0.;
                let mut j = x - 1.;
                while j >= 0. {
                    acc += 1. / (r + j);
                    j -= 1.;
                }
                acc
            } else {
                polygamma(0, x + r) - polygamma(0, r)
            };
            total += f * d;
        }
        total + w * (r / (r + m)).ln()
    };
    let start = m * m / (v - m);
    let (mut lo, mut hi) = (start, start);
    let mut steps = 0;
    while score(lo) < 0. {
        e.check_cancel()?;
        lo /= 2.;
        steps += 1;
        if steps > 2000 {
            return Err(format!("{who}: the NegativeBinomial fit did not converge"));
        }
    }
    while score(hi) > 0. {
        e.check_cancel()?;
        hi *= 2.;
        if hi > 1e15 {
            return Err(format!(
                "{who}: the NegativeBinomial R estimate is unbounded (data not overdispersed)"
            ));
        }
    }
    for _ in 0..200 {
        e.check_cancel()?;
        let mid = (lo * hi).sqrt();
        if mid <= lo || mid >= hi {
            break;
        }
        if score(mid) > 0. {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi / lo - 1. <= 4. * f64::EPSILON {
            break;
        }
    }
    let r = (lo * hi).sqrt();
    Ok((r, r / (r + m)))
}

/// Numerical MLE over the free parameters, starting from `start`.
pub(super) fn numeric(
    e: &Engine,
    who: &str,
    f: Family,
    s: &Sample,
    start: Vec<f64>,
    limits: &Limits,
) -> Result<Vec<f64>> {
    let kinds = kinds(f).ok_or_else(|| format!("{who}: cannot fit {}", f.title()))?;
    let free: Vec<usize> = (0..kinds.len())
        .filter(|&i| kinds[i] != Kind::Fixed)
        .collect();
    let scale = |i: usize| match kinds[i] {
        Kind::Loc(j) => start[j],
        _ => 1.,
    };
    let to_u = |i: usize, v: f64| match kinds[i] {
        Kind::Pos => v.ln(),
        Kind::Prob => logit(v),
        Kind::Loc(_) => v / scale(i),
        _ => v,
    };
    let from_u = |i: usize, u: f64| match kinds[i] {
        Kind::Pos => u.exp(),
        Kind::Prob => 1. / (1. + (-u).exp()),
        Kind::Loc(_) => u * scale(i),
        _ => u,
    };
    let params = |u: &[f64]| {
        let mut q = start.clone();
        for (k, &i) in free.iter().enumerate() {
            q[i] = from_u(i, u[k]);
        }
        q
    };
    let mut objective = |u: &[f64]| -> Result<f64> {
        e.check_cancel()?;
        let v = nll(f, &params(u), s);
        Ok(if v.is_nan() { f64::INFINITY } else { v })
    };
    let u0: Vec<f64> = free.iter().map(|&i| to_u(i, start[i])).collect();
    let step = vec![0.1; u0.len()];
    let (u, _) = optimize::nelder_mead(&mut objective, &u0, &step, limits)
        .map_err(|m| format!("{who}: {m}"))?;
    // One restart guards against a collapsed simplex.
    let (u, fu) = optimize::nelder_mead(&mut objective, &u, &step, limits)
        .map_err(|m| format!("{who}: {m}"))?;
    let h = vec![1e-4; u.len()];
    let (u, _) = optimize::polish(&mut objective, u, fu, &h)?;
    Ok(params(&u))
}

/// Maximum likelihood estimate of the family's parameters.
pub(super) fn fit(e: &Engine, who: &str, f: Family, s: &Sample, o: &Options) -> Result<Vec<f64>> {
    if kinds(f).is_none() {
        return Err(format!(
            "{who}: fitting {} is not supported; supported: {FITTABLE}",
            title(f)
        ));
    }
    check(who, f, s, o)?;
    let w = s.weight();
    let r = s.uncensored_weight();
    let cens = s.has_cens();
    let limits = &o.limits;
    let params = match f {
        Family::Norm | Family::Logn => {
            let t = if f == Family::Logn {
                s.mapped(f64::ln)
            } else {
                s.clone()
            };
            if w < 2. {
                return Err(format!(
                    "{who}: at least two observations are required for Normal and Lognormal"
                ));
            }
            if cens {
                let p = numeric(
                    e,
                    who,
                    Family::Norm,
                    &t,
                    vec![t.mean(), t.std().max(1e-300)],
                    limits,
                )?;
                vec![p[0], p[1]]
            } else {
                let m = t.mean();
                let ss: f64 = t.iter().map(|o| o.2 * (o.0 - m) * (o.0 - m)).sum();
                if ss == 0. {
                    return Err(format!(
                        "{who}: the data have no spread, so sigma would be zero"
                    ));
                }
                vec![m, (ss / if o.unbiased { w - 1. } else { w }).sqrt()]
            }
        }
        Family::Exp => {
            let total: f64 = s.iter().map(|o| o.0 * o.2).sum();
            vec![total / r]
        }
        Family::Poiss => vec![s.iter().map(|o| o.0 * o.2).sum::<f64>() / w],
        Family::Bino => {
            let n = o
                .ntrials
                .ok_or_else(|| format!("{who}: Binomial requires 'NTrials'"))?;
            vec![n, s.iter().map(|o| o.0 * o.2).sum::<f64>() / (n * w)]
        }
        Family::Rayl => {
            let s2: f64 = s.iter().map(|o| o.0 * o.0 * o.2).sum();
            vec![(s2 / (2. * r)).sqrt()]
        }
        Family::HalfNorm => {
            let ss: f64 = s
                .iter()
                .map(|ob| ob.2 * (ob.0 - o.mu) * (ob.0 - o.mu))
                .sum();
            let sigma0 = (ss / w).sqrt();
            if sigma0 == 0. {
                return Err(format!(
                    "{who}: the data have no spread, so sigma would be zero"
                ));
            }
            if cens {
                numeric(e, who, f, s, vec![o.mu, sigma0], limits)?
            } else {
                vec![o.mu, sigma0]
            }
        }
        Family::InvGauss => {
            let m = s.mean();
            let inv: f64 = s.iter().map(|ob| ob.2 * (1. / ob.0 - 1. / m)).sum::<f64>() / w;
            if inv <= 0. {
                return Err(format!(
                    "{who}: constant samples have no finite shape estimate"
                ));
            }
            if cens {
                numeric(e, who, f, s, vec![m, 1. / inv], limits)?
            } else {
                vec![m, 1. / inv]
            }
        }
        Family::Gam => {
            let plain = Sample::from_parts(s.x.clone(), Vec::new(), s.freq.clone());
            let (a, b) = gamma_profile(e, who, &plain)?;
            if cens {
                numeric(e, who, f, s, vec![a, b], limits)?
            } else {
                vec![a, b]
            }
        }
        Family::Nakagami => {
            let squares = Sample::from_parts(
                s.x.iter().map(|v| v * v).collect(),
                Vec::new(),
                s.freq.clone(),
            );
            let (a, b) = gamma_profile(e, who, &squares)?;
            if cens {
                numeric(e, who, f, s, vec![a, a * b], limits)?
            } else {
                vec![a, a * b]
            }
        }
        Family::Wbl => {
            let (a, b) = weibull_profile(e, who, s)?;
            vec![a, b]
        }
        Family::Ev => {
            let (m, sg) = ev_profile(e, who, s)?;
            vec![m, sg]
        }
        Family::Nbin => {
            let (rr, p) = nbin_profile(e, who, s)?;
            vec![rr, p]
        }
        Family::Logistic | Family::Loglogistic => {
            let t = if f == Family::Loglogistic {
                s.mapped(f64::ln)
            } else {
                s.clone()
            };
            spread(who, &t, 2)?;
            let start = vec![t.mean(), t.std() * 3f64.sqrt() / PI];
            numeric(e, who, Family::Logistic, &t, start, limits)?
        }
        Family::Beta => {
            spread(who, s, 2)?;
            let m = s.mean();
            let v = s.iter().map(|o| o.2 * (o.0 - m) * (o.0 - m)).sum::<f64>() / w;
            let mut c = m * (1. - m) / v - 1.;
            if c.is_nan() || c <= 0. || !c.is_finite() {
                c = 1.;
            }
            numeric(e, who, f, s, vec![m * c, (1. - m) * c], limits)?
        }
        Family::Rician => {
            spread(who, s, 2)?;
            let m2 = s.iter().map(|o| o.2 * o.0 * o.0).sum::<f64>() / w;
            let m4 = s.iter().map(|o| o.2 * o.0.powi(4)).sum::<f64>() / w;
            let d = 2. * m2 * m2 - m4;
            let s2 = if d > 0. { d.sqrt() } else { 0.25 * m2 };
            let s2 = s2.min(0.9 * m2);
            numeric(
                e,
                who,
                f,
                s,
                vec![s2.sqrt(), ((m2 - s2) / 2.).sqrt()],
                limits,
            )?
        }
        Family::Bisa => {
            spread(who, s, 2)?;
            let m = s.mean();
            let h = w / s.iter().map(|o| o.2 / o.0).sum::<f64>();
            let g = (2. * ((m / h).sqrt() - 1.)).max(1e-6).sqrt();
            numeric(e, who, f, s, vec![(m * h).sqrt(), g], limits)?
        }
        Family::Gp => {
            spread(who, s, 2)?;
            let y = s.mapped(|v| v - o.theta);
            let m = y.mean();
            let v = y.std().powi(2);
            let (mut k, mut sigma) = (0.5 * (1. - m * m / v), 0.5 * m * (1. + m * m / v));
            let top = y.x.iter().copied().fold(0., f64::max);
            if sigma.is_nan() || sigma <= 0. || (k < 0. && top >= -sigma / k * 0.999) {
                k = 0.;
                sigma = m.max(f64::MIN_POSITIVE);
            }
            numeric(e, who, f, s, vec![k, sigma, o.theta], limits)?
        }
        Family::Gev => {
            spread(who, s, 3)?;
            let sigma = s.std() * 6f64.sqrt() / PI;
            let mu = s.mean() - std::f64::consts::EULER_GAMMA * sigma;
            numeric(e, who, f, s, vec![0., sigma, mu], limits)?
        }
        Family::Tls => {
            spread(who, s, 3)?;
            let mut x: Vec<f64> = s.x.clone();
            x.sort_by(f64::total_cmp);
            let med = x[x.len() / 2];
            let mut dev: Vec<f64> = x.iter().map(|v| (v - med).abs()).collect();
            dev.sort_by(f64::total_cmp);
            let mut sigma = dev[dev.len() / 2] * 1.4826;
            if sigma.is_nan() || sigma <= 0. {
                sigma = s.std();
            }
            numeric(e, who, f, s, vec![med, sigma, 5.], limits)?
        }
        _ => unreachable!(),
    };
    if !f.valid(&params) || params.iter().any(|v| !v.is_finite()) {
        return Err(format!(
            "{who}: the {} fit did not reach valid finite parameters",
            title(f)
        ));
    }
    Ok(params)
}

fn spread(who: &str, s: &Sample, need: usize) -> Result<()> {
    if s.distinct_uncensored() < need {
        return Err(format!(
            "{who}: at least {need} distinct uncensored observations are required (constant samples have no finite estimate)"
        ));
    }
    Ok(())
}

// ---------- covariance and intervals ----------

/// Inverse of the observed information (Hessian of the negative
/// log-likelihood) at q; zero rows and columns for fixed parameters.
pub(super) fn covariance(
    e: &Engine,
    f: Family,
    q: &[f64],
    s: &Sample,
    fixed: &[bool],
) -> Result<Vec<f64>> {
    let k = q.len();
    let mut cov = vec![0.; k * k];
    let w = s.weight();
    let r = s.uncensored_weight();
    let cens = s.has_cens();
    match f {
        // Poisson and binomial: the information at the MLE (their only callers
        // are the fitters; no poisslike/binolike).
        Family::Poiss => {
            cov[0] = q[0] / w;
            return Ok(cov);
        }
        Family::Bino => {
            cov[3] = q[1] * (1. - q[1]) / (q[0] * w);
            return Ok(cov);
        }
        Family::Exp => {
            // NLL = r ln mu + T/mu (T = total of all observations, censored
            // ones included): d2/dmu2 = 2T/mu^3 - r/mu^2 at any mu (explike),
            // which is r/mu^2 at the MLE mu = T/r.
            let mu = q[0];
            let total: f64 = s.iter().map(|o| o.2 * o.0).sum();
            cov[0] = 1. / (2. * total / (mu * mu * mu) - r / (mu * mu));
            return Ok(cov);
        }
        Family::Norm | Family::Logn if !cens => {
            // Analytic Hessian of the normal NLL at (mu, sigma), on log data
            // for the lognormal (the Jacobian term does not depend on q).
            let (mu, sigma) = (q[0], q[1]);
            let t = |v: f64| if f == Family::Logn { v.ln() } else { v };
            let s1: f64 = s.iter().map(|o| o.2 * (t(o.0) - mu)).sum();
            let s2: f64 = s.iter().map(|o| o.2 * (t(o.0) - mu).powi(2)).sum();
            let s3 = sigma * sigma * sigma;
            let h = [
                w / (sigma * sigma),
                2. * s1 / s3,
                2. * s1 / s3,
                3. * s2 / (s3 * sigma) - w / (sigma * sigma),
            ];
            return Ok(optimize::inverse(2, &h).unwrap_or_else(|| vec![f64::NAN; 4]));
        }
        _ => {}
    }
    let kinds = kinds(f).unwrap_or(&[]);
    let free: Vec<usize> = (0..k).filter(|&i| !fixed[i]).collect();
    let step: Vec<f64> = free
        .iter()
        .map(|&i| {
            1e-4 * match kinds.get(i) {
                Some(Kind::Loc(j)) => q[*j],
                Some(Kind::Pos) => q[i].abs(),
                Some(Kind::Prob) => q[i].min(1. - q[i]),
                _ => q[i].abs().max(0.1),
            }
        })
        .collect();
    let mut objective = |u: &[f64]| -> Result<f64> {
        e.check_cancel()?;
        let mut p = q.to_vec();
        for (k, &i) in free.iter().enumerate() {
            p[i] = u[k];
        }
        Ok(nll(f, &p, s))
    };
    let at: Vec<f64> = free.iter().map(|&i| q[i]).collect();
    let h = optimize::hessian(&mut objective, &at, &step)?;
    let m = free.len();
    let inv = optimize::inverse(m, &h).unwrap_or_else(|| vec![f64::NAN; m * m]);
    for (a, &i) in free.iter().enumerate() {
        for (b, &j) in free.iter().enumerate() {
            cov[j * k + i] = inv[b * m + a];
        }
    }
    Ok(cov)
}

fn norm_inv(p: f64) -> f64 {
    Family::Norm.inv(p, &[0., 1.])
}
fn chi2_inv(p: f64, nu: f64) -> f64 {
    Family::Chi2.inv(p, &[nu])
}

/// 100(1-alpha)% intervals [lower, upper] for each parameter.
pub(super) fn param_ci(
    f: Family,
    q: &[f64],
    s: &Sample,
    cov: &[f64],
    fixed: &[bool],
    alpha: f64,
) -> Vec<[f64; 2]> {
    let k = q.len();
    let (lo, hi) = (alpha / 2., 1. - alpha / 2.);
    let w = s.weight();
    let cens = s.has_cens();
    let exact = match f {
        Family::Norm | Family::Logn if !cens && w > 1. => {
            let (mu, sigma) = (q[0], q[1]);
            let nu = w - 1.;
            let half = Family::T.inv(hi, &[nu]) * sigma / w.sqrt();
            Some(vec![
                [mu - half, mu + half],
                [
                    sigma * (nu / chi2_inv(hi, nu)).sqrt(),
                    sigma * (nu / chi2_inv(lo, nu)).sqrt(),
                ],
            ])
        }
        Family::Exp => {
            let total: f64 = s.iter().map(|o| o.0 * o.2).sum();
            let r = s.uncensored_weight();
            Some(vec![[
                2. * total / chi2_inv(hi, 2. * r),
                2. * total / chi2_inv(lo, 2. * r),
            ]])
        }
        Family::Poiss => {
            let total: f64 = s.iter().map(|o| o.0 * o.2).sum();
            let lower = if total == 0. {
                0.
            } else {
                chi2_inv(lo, 2. * total) / (2. * w)
            };
            Some(vec![[lower, chi2_inv(hi, 2. * total + 2.) / (2. * w)]])
        }
        Family::Bino => {
            let n = q[0] * w;
            let x: f64 = s.iter().map(|o| o.0 * o.2).sum();
            let lower = if x == 0. {
                0.
            } else {
                Family::Beta.inv(lo, &[x, n - x + 1.])
            };
            let upper = if x == n {
                1.
            } else {
                Family::Beta.inv(hi, &[x + 1., n - x])
            };
            Some(vec![[q[0], q[0]], [lower, upper]])
        }
        Family::Rayl if !cens => {
            let s2: f64 = s.iter().map(|o| o.0 * o.0 * o.2).sum();
            Some(vec![[
                (s2 / chi2_inv(hi, 2. * w)).sqrt(),
                (s2 / chi2_inv(lo, 2. * w)).sqrt(),
            ]])
        }
        _ => None,
    };
    if let Some(ci) = exact {
        return ci;
    }
    wald(f, q, cov, fixed, alpha, k)
}

/// Wald intervals from the covariance.
pub(super) fn wald(
    f: Family,
    q: &[f64],
    cov: &[f64],
    fixed: &[bool],
    alpha: f64,
    k: usize,
) -> Vec<[f64; 2]> {
    let z = norm_inv(1. - alpha / 2.);
    let kinds = kinds(f).unwrap_or(&[]);
    (0..k)
        .map(|i| {
            let v = q[i];
            if fixed[i] {
                return [v, v];
            }
            let se = cov[i * k + i].sqrt();
            match kinds.get(i) {
                Some(Kind::Pos) => {
                    let t = z * se / v;
                    [v * (-t).exp(), v * t.exp()]
                }
                Some(Kind::Prob) => {
                    let l = logit(v);
                    let t = z * se / (v * (1. - v));
                    let back = |u: f64| 1. / (1. + (-u).exp());
                    [back(l - t), back(l + t)]
                }
                _ => [v - z * se, v + z * se],
            }
        })
        .collect()
}

/// Parameters held fixed in a fit of f.
pub(super) fn fixed(f: Family) -> Vec<bool> {
    kinds(f)
        .map(|k| k.iter().map(|k| *k == Kind::Fixed).collect())
        .unwrap_or_default()
}
