//! Scalar kernels of the probability distributions: density (or probability
//! mass), distribution function in either tail, inverse and random draws.
//!
//! Sources (published mathematics only):
//! * DLMF chapters 5, 7, 8 (gamma, error and incomplete gamma/beta functions);
//!   the distribution functions are the standard ones of Johnson, Kotz and
//!   Balakrishnan, "Continuous Univariate Distributions" (2nd ed., 1994/95) and
//!   Johnson, Kemp and Kotz, "Univariate Discrete Distributions" (3rd ed., 2005).
//! * Binomial and Poisson probabilities: C. Loader, "Fast and accurate
//!   computation of binomial probabilities" (2000), the saddle-point form with
//!   the deviance term bd0 and Stirling's remainder.
//! * Gamma variates: G. Marsaglia and W. W. Tsang, "A simple method for
//!   generating gamma variables", ACM TOMS 26 (2000), with the a < 1 boost
//!   G(a) = G(a+1) U^(1/a) of the same paper.
use super::super::number_theory::erfcinv_fn;
use super::super::random::Generator;
use super::super::special_fns::{
    Tail, beta_fraction, betaincinv_real, gammainc_real, gammaincinv_real, power_prefix,
    stirling_correction,
};
use crate::engine::Fma;
use std::f64::consts::{FRAC_1_SQRT_2, PI, SQRT_2};

/// 1/sqrt(2 pi)
const INV_SQRT_2PI: f64 = 0.398_942_280_401_432_7;
/// ln(2 pi)
const LN_2PI: f64 = 1.837_877_066_409_345_5;
/// ln(sqrt(2 pi))
const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;
/// Largest double below which every integer is exact.
const FLINT: f64 = 9_007_199_254_740_992.;
/// Terms summed directly in a discrete distribution function before the
/// incomplete beta/gamma form takes over.
const DIRECT_TERMS: f64 = 64.;
/// Steps of the mode walk of a discrete draw before bisection takes over.
const WALK_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum Family {
    Norm,
    Unif,
    Exp,
    Gam,
    Beta,
    Chi2,
    T,
    F,
    Logn,
    Wbl,
    Rayl,
    Ev,
    Bino,
    Poiss,
    Geo,
    Nbin,
    // Families served by the distribution objects only (no <prefix>pdf names).
    Logistic,
    Loglogistic,
    Rician,
    Nakagami,
    Gev,
    Gp,
    Tls,
    InvGauss,
    Bisa,
    HalfNorm,
}

pub(in crate::engine) const ALL: [Family; 16] = [
    Family::Norm,
    Family::Unif,
    Family::Exp,
    Family::Gam,
    Family::Beta,
    Family::Chi2,
    Family::T,
    Family::F,
    Family::Logn,
    Family::Wbl,
    Family::Rayl,
    Family::Ev,
    Family::Bino,
    Family::Poiss,
    Family::Geo,
    Family::Nbin,
];

/// Families reached by name (makedist, fitdist, pdf('Logistic', ...)) only.
pub(in crate::engine) const OBJECT_ONLY: [Family; 10] = [
    Family::Logistic,
    Family::Loglogistic,
    Family::Rician,
    Family::Nakagami,
    Family::Gev,
    Family::Gp,
    Family::Tls,
    Family::InvGauss,
    Family::Bisa,
    Family::HalfNorm,
];

impl Family {
    /// The prefix of the function names (`normpdf`, `normcdf`, ...).
    pub(in crate::engine) fn prefix(self) -> &'static str {
        match self {
            Self::Norm => "norm",
            Self::Unif => "unif",
            Self::Exp => "exp",
            Self::Gam => "gam",
            Self::Beta => "beta",
            Self::Chi2 => "chi2",
            Self::T => "t",
            Self::F => "f",
            Self::Logn => "logn",
            Self::Wbl => "wbl",
            Self::Rayl => "rayl",
            Self::Ev => "ev",
            Self::Bino => "bino",
            Self::Poiss => "poiss",
            Self::Geo => "geo",
            Self::Nbin => "nbin",
            Self::Logistic => "logistic",
            Self::Loglogistic => "loglogistic",
            Self::Rician => "rician",
            Self::Nakagami => "nakagami",
            Self::Gev => "gev",
            Self::Gp => "gp",
            Self::Tls => "tlocationscale",
            Self::InvGauss => "inversegaussian",
            Self::Bisa => "birnbaumsaunders",
            Self::HalfNorm => "halfnormal",
        }
    }

    /// The long name used by pdf/cdf/icdf/random.
    pub(in crate::engine) fn title(self) -> &'static str {
        match self {
            Self::Norm => "Normal",
            Self::Unif => "Uniform",
            Self::Exp => "Exponential",
            Self::Gam => "Gamma",
            Self::Beta => "Beta",
            Self::Chi2 => "Chisquare",
            Self::T => "T",
            Self::F => "F",
            Self::Logn => "Lognormal",
            Self::Wbl => "Weibull",
            Self::Rayl => "Rayleigh",
            Self::Ev => "Extreme Value",
            Self::Bino => "Binomial",
            Self::Poiss => "Poisson",
            Self::Geo => "Geometric",
            Self::Nbin => "Negative Binomial",
            Self::Logistic => "Logistic",
            Self::Loglogistic => "Loglogistic",
            Self::Rician => "Rician",
            Self::Nakagami => "Nakagami",
            Self::Gev => "GeneralizedExtremeValue",
            Self::Gp => "GeneralizedPareto",
            Self::Tls => "tLocationScale",
            Self::InvGauss => "InverseGaussian",
            Self::Bisa => "BirnbaumSaunders",
            Self::HalfNorm => "HalfNormal",
        }
    }

    /// Defaults of the parameters in order; `None` marks a required one.
    pub(in crate::engine) fn defaults(self) -> &'static [Option<f64>] {
        match self {
            Self::Norm | Self::Logn | Self::Ev => &[Some(0.), Some(1.)],
            Self::Unif => &[Some(0.), Some(1.)],
            Self::Exp | Self::Rayl => &[Some(1.)],
            Self::Wbl => &[Some(1.), Some(1.)],
            Self::Gam => &[None, Some(1.)],
            Self::Beta | Self::F | Self::Bino | Self::Nbin => &[None, None],
            Self::Chi2 | Self::T | Self::Poiss | Self::Geo => &[None],
            Self::Logistic | Self::Loglogistic | Self::HalfNorm => &[Some(0.), Some(1.)],
            Self::Rician | Self::Nakagami | Self::InvGauss | Self::Bisa => &[Some(1.), Some(1.)],
            Self::Gev => &[Some(0.), Some(1.), Some(0.)],
            Self::Gp => &[Some(1.), Some(1.), Some(1.)],
            Self::Tls => &[Some(0.), Some(1.), Some(5.)],
        }
    }

    pub(in crate::engine) fn discrete(self) -> bool {
        matches!(self, Self::Bino | Self::Poiss | Self::Geo | Self::Nbin)
    }

    /// Whether the parameters lie in the family's domain.
    pub(in crate::engine) fn valid(self, q: &[f64]) -> bool {
        let pos = |v: f64| v > 0.;
        let prob = |v: f64| (0. ..=1.).contains(&v);
        if q.iter().any(|v| v.is_nan()) {
            return false;
        }
        match self {
            Self::Norm | Self::Logn | Self::Ev => q[0].is_finite() && pos(q[1]),
            Self::Unif => q[0].is_finite() && q[1].is_finite() && q[0] < q[1],
            Self::Exp | Self::Rayl | Self::Chi2 => pos(q[0]) && q[0].is_finite(),
            Self::T => pos(q[0]),
            Self::Gam | Self::Wbl | Self::Beta | Self::F => {
                pos(q[0]) && pos(q[1]) && q[0].is_finite() && q[1].is_finite()
            }
            Self::Bino => q[0] >= 0. && q[0] < FLINT && q[0].fract() == 0. && prob(q[1]),
            Self::Poiss => q[0] >= 0. && q[0].is_finite(),
            // p = 0: success never comes, all the mass sits at +Inf.
            Self::Geo => prob(q[0]),
            Self::Nbin => pos(q[0]) && q[0].is_finite() && prob(q[1]),
            Self::Logistic | Self::Loglogistic | Self::HalfNorm => {
                q[0].is_finite() && pos(q[1]) && q[1].is_finite()
            }
            Self::Rician => q[0] >= 0. && q[0].is_finite() && pos(q[1]) && q[1].is_finite(),
            Self::Nakagami | Self::InvGauss | Self::Bisa => {
                pos(q[0]) && pos(q[1]) && q[0].is_finite() && q[1].is_finite()
            }
            Self::Gev | Self::Gp => {
                q[0].is_finite() && pos(q[1]) && q[1].is_finite() && q[2].is_finite()
            }
            Self::Tls => q[0].is_finite() && pos(q[1]) && q[1].is_finite() && pos(q[2]),
        }
    }

    /// Density, or probability mass for the discrete families.
    pub(in crate::engine) fn pdf(self, x: f64, q: &[f64]) -> f64 {
        if x.is_nan() || !self.valid(q) {
            return f64::NAN;
        }
        match self {
            Self::Norm => norm_pdf((x - q[0]) / q[1]) / q[1],
            Self::Unif => {
                if x >= q[0] && x <= q[1] {
                    1. / (q[1] - q[0])
                } else {
                    0.
                }
            }
            Self::Exp => {
                if x < 0. {
                    0.
                } else {
                    libm::exp(-x / q[0]) / q[0]
                }
            }
            Self::Gam => gam_pdf(x / q[1], q[0]) / q[1],
            Self::Chi2 => gam_pdf(x / 2., q[0] / 2.) / 2.,
            Self::Beta => beta_pdf(x, q[0], q[1]),
            Self::T => t_pdf(x, q[0]),
            Self::F => f_pdf(x, q[0], q[1]),
            Self::Logn => {
                if x <= 0. || x.is_infinite() {
                    0.
                } else {
                    let d = norm_pdf((libm::log(x) - q[0]) / q[1]);
                    // SIGMA x may underflow to 0 where the density is 0 (0/0).
                    if d == 0. { 0. } else { d / (q[1] * x) }
                }
            }
            Self::Wbl => wbl_pdf(x / q[0], q[1]) / q[0],
            Self::Rayl => {
                let z = x / q[0];
                // z is infinite also for finite x and a tiny scale (Inf * 0).
                if x < 0. || z.is_infinite() {
                    0.
                } else {
                    z * exp_half_square(z) / q[0]
                }
            }
            Self::Ev => {
                let z = (x - q[0]) / q[1];
                if z.is_infinite() {
                    0.
                } else {
                    libm::exp(z - libm::exp(z)) / q[1]
                }
            }
            Self::Bino | Self::Poiss | Self::Geo | Self::Nbin => {
                if x < 0. || x.fract() != 0. || x.is_infinite() {
                    0.
                } else {
                    self.pmf(x, q)
                }
            }
            Self::Tls => t_pdf((x - q[0]) / q[1], q[2]) / q[1],
            Self::Nakagami => {
                if x <= 0. || x.is_infinite() {
                    0.
                } else {
                    let s = q[1] / q[0];
                    2. * x * gam_pdf(x * x / s, q[0]) / s
                }
            }
            Self::HalfNorm => {
                if x < q[0] {
                    0.
                } else {
                    2. * norm_pdf((x - q[0]) / q[1]) / q[1]
                }
            }
            Self::Bisa => {
                if x <= 0. || x.is_infinite() {
                    0.
                } else {
                    let (r, s) = ((x / q[0]).sqrt(), (q[0] / x).sqrt());
                    (r + s) / (2. * q[1] * x) * norm_pdf((r - s) / q[1])
                }
            }
            _ => crate::engine::dist_fit::families::pdf(self, x, q),
        }
    }

    /// Probability mass at the integer k >= 0 (valid parameters).
    fn pmf(self, k: f64, q: &[f64]) -> f64 {
        match self {
            Self::Bino => binomial_mass(k, q[0], q[1], 1. - q[1]),
            Self::Poiss => poisson_mass(k, q[0]),
            Self::Geo => {
                if q[0] == 1. {
                    if k == 0. { 1. } else { 0. }
                } else {
                    q[0] * libm::exp(k * libm::log1p(-q[0]))
                }
            }
            Self::Nbin => nbin_mass(k, q[0], q[1]),
            _ => f64::NAN,
        }
    }

    /// The distribution function, or with `upper` its complement, each
    /// computed directly in its own tail.
    pub(in crate::engine) fn cdf(self, x: f64, q: &[f64], upper: bool) -> f64 {
        if x.is_nan() || !self.valid(q) {
            return f64::NAN;
        }
        let pick = |(lower, up): (f64, f64)| if upper { up } else { lower };
        match self {
            Self::Norm => norm_cdf((x - q[0]) / q[1], upper),
            Self::Unif => {
                let (a, b) = (q[0], q[1]);
                let lower = if x <= a {
                    (0., 1.)
                } else if x >= b {
                    (1., 0.)
                } else {
                    ((x - a) / (b - a), (b - x) / (b - a))
                };
                pick(lower)
            }
            Self::Exp => pick(exp_tails(x / q[0])),
            Self::Gam => pick(gam_tails(x / q[1], q[0])),
            Self::Chi2 => pick(gam_tails(x / 2., q[0] / 2.)),
            Self::Beta => {
                if x <= 0. {
                    pick((0., 1.))
                } else if x >= 1. {
                    pick((1., 0.))
                } else {
                    pick(beta_tails(x, 1. - x, q[0], q[1]))
                }
            }
            Self::T => pick(t_tails(x, q[0])),
            Self::F => pick(f_tails(x, q[0], q[1])),
            Self::Logn => {
                if x <= 0. {
                    pick((0., 1.))
                } else {
                    norm_cdf((libm::log(x) - q[0]) / q[1], upper)
                }
            }
            Self::Wbl => {
                if x <= 0. {
                    pick((0., 1.))
                } else {
                    let s = libm::pow(x / q[0], q[1]);
                    pick((-libm::expm1(-s), libm::exp(-s)))
                }
            }
            Self::Rayl => {
                if x <= 0. {
                    pick((0., 1.))
                } else {
                    let z = x / q[0];
                    let e = exp_half_square(z);
                    pick((-expm1_half_square(z), e))
                }
            }
            Self::Ev => {
                let s = libm::exp((x - q[0]) / q[1]);
                pick((-libm::expm1(-s), libm::exp(-s)))
            }
            Self::Bino | Self::Poiss | Self::Geo | Self::Nbin => {
                let k = x.floor();
                if k < 0. {
                    return pick((0., 1.));
                }
                pick(self.discrete_tails(k, q))
            }
            Self::Tls => pick(t_tails((x - q[0]) / q[1], q[2])),
            Self::Nakagami => {
                if x <= 0. {
                    pick((0., 1.))
                } else {
                    pick(gam_tails(x * x * q[0] / q[1], q[0]))
                }
            }
            Self::HalfNorm => {
                if x <= q[0] {
                    pick((0., 1.))
                } else {
                    let z = (x - q[0]) / q[1];
                    // P(|Z| <= z) = 1 - 2 Q(z); the upper tail is 2 Q(z).
                    let up = 2. * norm_cdf(z, true);
                    pick((
                        if up > 0.5 {
                            1. - up
                        } else {
                            libm::erf(z * FRAC_1_SQRT_2)
                        },
                        up,
                    ))
                }
            }
            Self::Bisa => {
                if x <= 0. {
                    pick((0., 1.))
                } else {
                    let z = ((x / q[0]).sqrt() - (q[0] / x).sqrt()) / q[1];
                    norm_cdf(z, upper)
                }
            }
            _ => pick(crate::engine::dist_fit::families::tails(self, x, q)),
        }
    }

    /// (P(X <= k), P(X > k)) for an integer k >= 0.
    fn discrete_tails(self, k: f64, q: &[f64]) -> (f64, f64) {
        if k == f64::INFINITY {
            return (1., 0.);
        }
        match self {
            Self::Bino => {
                let (n, p) = (q[0], q[1]);
                if k >= n || p == 0. {
                    return (1., 0.);
                }
                if p == 1. {
                    return (0., 1.);
                }
                let mean = n * p;
                if k < mean && k < DIRECT_TERMS {
                    complement_of(self.mass_sum(0., k, q))
                } else if k >= mean && n - k <= DIRECT_TERMS {
                    swap(complement_of(self.mass_sum(k + 1., n, q)))
                } else {
                    // P(X > k) = I_p(k+1, n-k) (DLMF 8.17.5).
                    swap(beta_tails(p, 1. - p, k + 1., n - k))
                }
            }
            Self::Poiss => {
                let lambda = q[0];
                if lambda == 0. {
                    return (1., 0.);
                }
                if k < lambda && k < DIRECT_TERMS {
                    complement_of(self.mass_sum(0., k, q))
                } else {
                    // P(X <= k) = Q(k+1, lambda) (DLMF 8.4.10).
                    (
                        gammainc_real(lambda, k + 1., Tail::Upper),
                        gammainc_real(lambda, k + 1., Tail::Lower),
                    )
                }
            }
            Self::Geo => {
                let p = q[0];
                if p == 1. {
                    return (1., 0.);
                }
                let s = (k + 1.) * libm::log1p(-p);
                (-libm::expm1(s), libm::exp(s))
            }
            Self::Nbin => {
                let (r, p) = (q[0], q[1]);
                if p == 1. {
                    return (1., 0.);
                }
                let mean = r * (1. - p) / p;
                if k < mean && k < DIRECT_TERMS {
                    complement_of(self.mass_sum(0., k, q))
                } else {
                    // P(X <= k) = I_p(r, k+1).
                    beta_tails(p, 1. - p, r, k + 1.)
                }
            }
            _ => (f64::NAN, f64::NAN),
        }
    }

    /// Sum of the masses at from..=to, each computed on its own (no recurrence).
    fn mass_sum(self, from: f64, to: f64, q: &[f64]) -> f64 {
        let mut terms = Vec::with_capacity((to - from + 1.) as usize);
        let mut k = from;
        while k <= to {
            terms.push(self.pmf(k, q));
            k += 1.;
        }
        // Small terms first, then Neumaier's compensated sum.
        terms.sort_by(f64::total_cmp);
        let (mut sum, mut carry) = (0., 0.);
        for t in terms {
            let s = sum + t;
            carry += if f64::abs(sum) >= f64::abs(t) {
                (sum - s) + t
            } else {
                (t - s) + sum
            };
            sum = s;
        }
        sum + carry
    }

    /// Inverse distribution function: for the discrete families the smallest
    /// integer x with cdf(x) >= p.
    pub(in crate::engine) fn inv(self, p: f64, q: &[f64]) -> f64 {
        if p.is_nan() || !(0. ..=1.).contains(&p) || !self.valid(q) {
            return f64::NAN;
        }
        if self.discrete() {
            return self.discrete_inv(p, q);
        }
        let start = match self {
            Self::Norm => q[0] + q[1] * norm_quantile(p),
            Self::Unif => {
                if p == 1. {
                    q[1]
                } else {
                    q[0] + p * (q[1] - q[0])
                }
            }
            Self::Exp => -q[0] * libm::log1p(-p),
            Self::Gam => q[1] * gammaincinv_real(p, q[0], false),
            Self::Chi2 => 2. * gammaincinv_real(p, q[0] / 2., false),
            Self::Beta => betaincinv_real(p, q[0], q[1], false),
            Self::T => t_quantile(p, q[0]),
            Self::F => f_quantile(p, q[0], q[1]),
            Self::Logn => libm::exp(q[0] + q[1] * norm_quantile(p)),
            Self::Wbl => q[0] * libm::pow(-libm::log1p(-p), 1. / q[1]),
            Self::Rayl => q[0] * (-2. * libm::log1p(-p)).sqrt(),
            Self::Ev => q[0] + q[1] * libm::log(-libm::log1p(-p)),
            Self::Tls => q[0] + q[1] * t_quantile(p, q[2]),
            Self::Nakagami => (gammaincinv_real(p, q[0], false) * q[1] / q[0]).sqrt(),
            Self::HalfNorm => q[0] + q[1] * norm_quantile(0.5 + p / 2.),
            Self::Bisa => q[0] * bisa_root(q[1] * norm_quantile(p) / 2.),
            _ => crate::engine::dist_fit::families::quantile(self, p, q),
        };
        if p == 0. || p == 1. || matches!(self, Self::Unif) {
            return start;
        }
        let x = self.polish(start, p, q);
        let upper = p > 0.5;
        let target = if upper { 1. - p } else { p };
        let r = (self.cdf(x, q, upper) - target).abs();
        if x.is_finite() && r <= 1e-9 * target {
            return x;
        }
        // The start was poor (or the steps stalled): bisect over the doubles.
        self.polish(self.bisect(p, q), p, q)
    }

    /// The double where cdf crosses p, by bisection on the ordered bit
    /// patterns of the doubles of the support (at most 64 cdf calls).
    fn bisect(self, p: f64, q: &[f64]) -> f64 {
        let (lo, hi) = match self {
            Self::Norm
            | Self::T
            | Self::Ev
            | Self::Logistic
            | Self::Gev
            | Self::Gp
            | Self::Tls
            | Self::HalfNorm => (-f64::MAX, f64::MAX),
            Self::Beta => (0., 1.),
            _ => (0., f64::MAX),
        };
        let upper = p > 0.5;
        let target = if upper { 1. - p } else { p };
        // below(x): cdf(x) < p, compared on the smaller tail
        let below = |x: f64| {
            if upper {
                self.cdf(x, q, true) > target
            } else {
                self.cdf(x, q, false) < target
            }
        };
        // Order-preserving map of the doubles to integers (i128 so that the
        // width of the whole real line does not overflow).
        let key = |x: f64| {
            let b = i128::from(x.to_bits() as i64);
            if b < 0 { i128::from(i64::MIN) - b } else { b }
        };
        let unkey = |k: i128| {
            let b = if k < 0 { i128::from(i64::MIN) - k } else { k };
            f64::from_bits(b as i64 as u64)
        };
        // A quantile beyond the largest double (cdf(MAX) still below p, or
        // cdf(-MAX) already at p) is an overflow: +-Inf, not +-MAX.
        if below(hi) {
            return f64::INFINITY;
        }
        if lo == -f64::MAX && !below(lo) {
            return f64::NEG_INFINITY;
        }
        let (mut a, mut b) = (key(lo), key(hi));
        while b - a > 1 {
            let mid = a + (b - a) / 2;
            if below(unkey(mid)) {
                a = mid;
            } else {
                b = mid;
            }
        }
        unkey(b)
    }

    /// Newton steps on the smaller tail, kept only while they shrink the
    /// residual; the closed forms and the special-function inverses are
    /// already close, this removes their last few roundings.
    fn polish(self, mut x: f64, p: f64, q: &[f64]) -> f64 {
        if !x.is_finite() {
            return x;
        }
        let upper = p > 0.5;
        let target = if upper { 1. - p } else { p };
        let residual = |x: f64| self.cdf(x, q, upper) - target;
        let mut r = residual(x);
        for _ in 0..4 {
            if r == 0. {
                break;
            }
            let d = self.pdf(x, q);
            if d.is_nan() || d <= 0. || d.is_infinite() {
                break;
            }
            let step = if upper { -r / d } else { r / d };
            let next = x - step;
            if !next.is_finite() || next == x {
                break;
            }
            let rn = residual(next);
            if rn.abs() < r.abs() {
                x = next;
                r = rn;
            } else {
                break;
            }
        }
        x
    }

    /// Smallest integer k with cdf(k) >= p (p in [0,1], valid parameters).
    fn discrete_inv(self, p: f64, q: &[f64]) -> f64 {
        let top = match self {
            Self::Bino => q[0],
            _ => f64::INFINITY,
        };
        if p == 0. {
            return 0.;
        }
        // A degenerate law has cdf(0) = 1, so even p = 1 is reached at 0
        // (checked before the p = 1 shortcut below).
        let trivial = match self {
            Self::Bino => q[1] == 0. || q[0] == 0.,
            Self::Poiss => q[0] == 0.,
            Self::Geo | Self::Nbin => q[q.len() - 1] == 1.,
            _ => false,
        };
        if trivial {
            return 0.;
        }
        // Success probability 0: cdf(k) = 0 at every integer k.
        if matches!(self, Self::Geo | Self::Nbin) && q[q.len() - 1] == 0. {
            return f64::INFINITY;
        }
        if p == 1. {
            return top;
        }
        if matches!(self, Self::Bino) && q[1] == 1. {
            return q[0];
        }
        // cdf(k) >= p, compared on the smaller tail.
        let reached = |k: f64| {
            if k >= top {
                return true;
            }
            let (lower, up) = self.discrete_tails(k, q);
            if p > 0.5 { up <= 1. - p } else { lower >= p }
        };
        let (mean, sd) = self.moments(q);
        let guess = (mean + sd * norm_quantile(p)).floor().clamp(0., top);
        let guess = if guess.is_finite() { guess } else { 0. };
        // Bracket (lo, hi]: cdf(lo) < p <= cdf(hi), lo = -1 standing for "below 0".
        let (mut lo, mut hi);
        if reached(guess) {
            hi = guess;
            // Bounded: the step doubles, so hi - step is below 0 within 1025 rounds.
            let mut step = 1f64;
            loop {
                let probe = hi - step;
                if probe < 0. {
                    lo = -1.;
                    break;
                }
                if reached(probe) {
                    hi = probe;
                    step *= 2.;
                } else {
                    lo = probe;
                    break;
                }
            }
        } else {
            lo = guess;
            // The step doubles every round, so within 1025 rounds the probe
            // is infinite (or `top`) and counts as reached: the loop is bounded.
            let mut step = 1f64;
            loop {
                let probe = (lo + step).min(top);
                if reached(probe) {
                    hi = probe;
                    break;
                }
                lo = probe;
                step *= 2.;
            }
            if hi.is_infinite() {
                // The quantile is beyond the largest double.
                return hi;
            }
        }
        while hi - lo > 1. {
            let mid = (lo + (hi - lo) / 2.).floor();
            if mid <= lo || mid >= hi {
                // Above 2^53 the doubles are more than 1 apart: lo and hi are
                // adjacent, no integer lies between them, and hi is the answer.
                break;
            }
            if reached(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    fn moments(self, q: &[f64]) -> (f64, f64) {
        match self {
            Self::Bino => (q[0] * q[1], (q[0] * q[1] * (1. - q[1])).sqrt()),
            Self::Poiss => (q[0], q[0].sqrt()),
            Self::Geo => {
                let r = 1. - q[0];
                (r / q[0], r.sqrt() / q[0])
            }
            Self::Nbin => {
                let r = 1. - q[1];
                (q[0] * r / q[1], (q[0] * r).sqrt() / q[1])
            }
            _ => (f64::NAN, f64::NAN),
        }
    }

    /// One random draw with these parameters; `cache` keeps the mode data of
    /// the discrete inversion between draws with the same parameters.
    pub(in crate::engine) fn rnd(self, q: &[f64], g: &mut Generator, cache: &mut WalkCache) -> f64 {
        if !self.valid(q) {
            return f64::NAN;
        }
        match self {
            Self::Norm => q[0] + q[1] * g.next_normal(),
            Self::Unif => q[0] + (q[1] - q[0]) * g.next_f64(),
            Self::Exp => -q[0] * libm::log(g.next_f64()),
            Self::Gam => q[1] * gamma_draw(q[0], g),
            Self::Chi2 => 2. * gamma_draw(q[0] / 2., g),
            Self::Beta => beta_draw(q[0], q[1], g),
            Self::T => {
                let z = g.next_normal();
                if q[0].is_infinite() {
                    z
                } else {
                    z / (2. * gamma_draw(q[0] / 2., g) / q[0]).sqrt()
                }
            }
            Self::F => {
                let a = 2. * gamma_draw(q[0] / 2., g) / q[0];
                let b = 2. * gamma_draw(q[1] / 2., g) / q[1];
                a / b
            }
            Self::Logn => libm::exp(q[0] + q[1] * g.next_normal()),
            Self::Wbl => q[0] * libm::pow(-libm::log(g.next_f64()), 1. / q[1]),
            Self::Rayl => q[0] * (-2. * libm::log(g.next_f64())).sqrt(),
            Self::Ev => q[0] + q[1] * libm::log(-libm::log(g.next_f64())),
            Self::Geo => {
                if q[0] == 1. {
                    0.
                } else if q[0] == 0. {
                    f64::INFINITY
                } else {
                    (libm::log(g.next_f64()) / libm::log1p(-q[0])).floor() + 0.
                }
            }
            Self::Bino if q[0] <= 32. => {
                let mut count = 0.;
                for _ in 0..q[0] as usize {
                    if g.next_f64() < q[1] {
                        count += 1.;
                    }
                }
                count
            }
            Self::Bino | Self::Poiss | Self::Nbin => {
                let u = g.next_f64();
                self.walk_inv(u, q, cache)
            }
            Self::Tls => {
                let z = g.next_normal();
                let t = if q[2].is_infinite() {
                    z
                } else {
                    z / (2. * gamma_draw(q[2] / 2., g) / q[2]).sqrt()
                };
                q[0] + q[1] * t
            }
            Self::Nakagami => (gamma_draw(q[0], g) * q[1] / q[0]).sqrt(),
            Self::HalfNorm => q[0] + q[1] * g.next_normal().abs(),
            Self::Rician => {
                let (a, b) = (g.next_normal(), g.next_normal());
                (q[1] * a + q[0]).hypot(q[1] * b)
            }
            Self::Bisa => q[0] * bisa_root(q[1] * g.next_normal() / 2.),
            Self::InvGauss => crate::engine::dist_fit::families::inverse_gaussian_draw(
                q[0],
                q[1],
                g.next_normal(),
                g.next_f64(),
            ),
            _ => {
                let u = g.next_f64();
                crate::engine::dist_fit::families::quantile(self, u, q)
            }
        }
    }

    /// Discrete inversion of u by a walk from the mode with the mass
    /// recurrence; long walks fall back to the bisection of `discrete_inv`.
    fn walk_inv(self, u: f64, q: &[f64], cache: &mut WalkCache) -> f64 {
        if cache.family != Some(self) || cache.params[..q.len()] != *q {
            let mode = match self {
                Self::Bino => ((q[0] + 1.) * q[1]).floor().min(q[0]),
                Self::Poiss => q[0].floor(),
                Self::Nbin if q[0] > 1. => ((q[0] - 1.) * (1. - q[1]) / q[1]).floor(),
                _ => 0.,
            };
            *cache = WalkCache {
                family: Some(self),
                params: [q[0], q.get(1).copied().unwrap_or(0.)],
                mode,
                mass: self.pmf(mode, q),
                cdf: self.discrete_tails(mode, q).0,
            };
        }
        // Above 2^53 a step of 1 does not move k: the walk is meaningless.
        if cache.mass.is_nan() || cache.mass <= 0. || cache.mode >= 9_007_199_254_740_992. {
            return self.discrete_inv(u, q);
        }
        // pmf(k+1) / pmf(k)
        let ratio = |k: f64| match self {
            Self::Bino => (q[0] - k) / (k + 1.) * (q[1] / (1. - q[1])),
            Self::Poiss => q[0] / (k + 1.),
            Self::Nbin => (q[0] + k) / (k + 1.) * (1. - q[1]),
            _ => 0.,
        };
        let (mut k, mut mass, mut cdf) = (cache.mode, cache.mass, cache.cdf);
        if u <= cdf {
            for _ in 0..WALK_LIMIT {
                // cdf(k-1) = cdf(k) - pmf(k)
                if k == 0. || cdf - mass < u {
                    return k;
                }
                cdf -= mass;
                mass /= ratio(k - 1.);
                k -= 1.;
            }
        } else {
            for _ in 0..WALK_LIMIT {
                mass *= ratio(k);
                k += 1.;
                cdf += mass;
                if cdf >= u {
                    return k;
                }
                if mass == 0. {
                    break;
                }
            }
        }
        self.discrete_inv(u, q)
    }
}

/// Mode data of the last discrete parameter set drawn from.
#[derive(Default)]
pub(in crate::engine) struct WalkCache {
    family: Option<Family>,
    params: [f64; 2],
    mode: f64,
    mass: f64,
    cdf: f64,
}

fn swap((a, b): (f64, f64)) -> (f64, f64) {
    (b, a)
}

/// (s, 1 - s) for a probability s.
fn complement_of(s: f64) -> (f64, f64) {
    let s = s.min(1.);
    (s, 1. - s)
}

// ---------- normal ----------

/// exp(-z^2/2) with z^2 split exactly into s + e (s = fl(z^2)), so that the
/// rounding of the square does not reach the result in the far tail.
fn exp_half_square(z: f64) -> f64 {
    let s = z * z;
    if !s.is_finite() {
        return 0.;
    }
    let e = z.fma(z, -s);
    libm::exp(-0.5 * s) * (1. - 0.5 * e)
}

/// expm1(-z^2/2) with the same exact square.
fn expm1_half_square(z: f64) -> f64 {
    let s = z * z;
    if !s.is_finite() {
        return -1.;
    }
    let e = z.fma(z, -s);
    let m = libm::expm1(-0.5 * s);
    // expm1(-(s+e)/2) ~ m - (1+m) e/2
    m - (1. + m) * 0.5 * e
}

fn norm_pdf(z: f64) -> f64 {
    if z.is_infinite() {
        return 0.;
    }
    INV_SQRT_2PI * exp_half_square(z)
}

/// Phi(z) = erfc(-z/sqrt 2)/2 (DLMF 7.18.2); the upper tail is Phi(-z).
fn norm_cdf(z: f64, upper: bool) -> f64 {
    let z = if upper { -z } else { z };
    if z.is_nan() {
        return f64::NAN;
    }
    0.5 * libm::erfc(-z * FRAC_1_SQRT_2)
}

/// Phi^-1(p) = -sqrt 2 erfcinv(2p) (DLMF 7.17.2); 2p and 2 - 2p are exact.
/// (w + sqrt(w^2 + 1))^2 without cancellation for w < 0 or overflow for
/// large |w| (the Birnbaum-Saunders quantile in units of beta).
fn bisa_root(w: f64) -> f64 {
    let r = if w < 0. {
        1. / (w.hypot(1.) - w)
    } else {
        w + w.hypot(1.)
    };
    r * r
}

fn norm_quantile(p: f64) -> f64 {
    -SQRT_2 * erfcinv_fn(2. * p)
}

// ---------- exponential, gamma, beta ----------

fn exp_tails(z: f64) -> (f64, f64) {
    if z <= 0. {
        (0., 1.)
    } else {
        (-libm::expm1(-z), libm::exp(-z))
    }
}

/// Density of the standard gamma distribution with shape a at z >= 0:
/// z^(a-1) e^-z / Gamma(a) = a/z * z^a e^-z / Gamma(a+1).
fn gam_pdf(z: f64, a: f64) -> f64 {
    if z < 0. || z.is_infinite() {
        return 0.;
    }
    if z == 0. {
        return if a < 1. {
            f64::INFINITY
        } else if a == 1. {
            1.
        } else {
            0.
        };
    }
    power_prefix(a, z) * (a / z)
}

fn gam_tails(z: f64, a: f64) -> (f64, f64) {
    if z <= 0. {
        return (0., 1.);
    }
    if z.is_infinite() {
        return (1., 0.);
    }
    (
        gammainc_real(z, a, Tail::Lower),
        gammainc_real(z, a, Tail::Upper),
    )
}

/// x^a y^b / B(a, b) for x + y = 1, with x and y each given to full relative
/// precision (when they come from separate quotients). Only the smaller of the
/// two is used: ln of the larger is log1p(-smaller), and the deviation from the
/// mean, x (a+b) - a = -(y (a+b) - b), comes from one fma, so neither 1 - x
/// nor a difference of large logarithms loses digits. For a, b >= 10 the
/// gamma functions are in Stirling's form (DLMF 5.11.1), the powers taken of
/// ratios near 1.
fn beta_kernel(x: f64, y: f64, a: f64, b: f64) -> f64 {
    if x == 0. || y == 0. {
        return 0.;
    }
    let s = a + b;
    let (ln_x, ln_y) = if x <= y {
        (libm::log(x), libm::log1p(-x))
    } else {
        (libm::log1p(-y), libm::log(y))
    };
    if a >= 10. && b >= 10. {
        let (e, dev) = exact_deviation(x, y, a, b);
        let corr = stirling_correction(s) - stirling_correction(a) - stirling_correction(b);
        let scale = a.sqrt() * (b / (2. * PI * s)).sqrt();
        if dev.abs() < 0.5 * a.min(b) {
            return libm::exp(ln_kernel(a, b, dev) + corr) * scale;
        }
        // x t / a = 1 + dev/a and y t / b = 1 - dev/b; log1p near the mean,
        // the plain logarithm of the ratio away from it (where log1p's
        // argument nears -1 and its rounding would be magnified).
        let ln_ratio = |d: f64, shape: f64, v: f64| {
            if d.abs() < 0.5 * shape {
                libm::log1p(d / shape)
            } else {
                libm::log(v * s / shape) + e / s
            }
        };
        let la = a * ln_ratio(dev, a, x);
        let lb = b * ln_ratio(-dev, b, y);
        return libm::exp(la + lb + corr) * scale;
    }
    if s < 150. {
        let ratio = libm::tgamma(s) / (libm::tgamma(a) * libm::tgamma(b));
        let powers = if x <= y {
            libm::pow(x, a) * libm::exp(b * ln_y)
        } else {
            libm::exp(a * ln_x) * libm::pow(y, b)
        };
        let direct = ratio * powers;
        if direct.is_finite() && direct > 1e-290 && ratio.is_normal() {
            return direct;
        }
    }
    let big = a.max(b);
    let small = a.min(b);
    // ln Gamma(a+b) - ln Gamma(big) without the cancellation of two large
    // logarithms once big >= 10 (Stirling for both, DLMF 5.11.1).
    let ratio = if big >= 10. {
        (big - 0.5) * libm::log1p(small / big) + small * libm::log(s) - small
            + stirling_correction(s)
            - stirling_correction(big)
    } else {
        libm::lgamma(s) - libm::lgamma(big)
    };
    libm::exp(ratio - libm::lgamma(small) + a * ln_x + b * ln_y)
}

/// (e, dev): e = (a + b) - s exactly for s = fl(a + b) (TwoSum), and the
/// deviation dev = x t - a = -(y t - b) of the exact sum t = s + e from the
/// smaller of x and y = 1 - x, with one rounding (fma). Ignoring e would cost
/// a relative error e in the kernel (up to 1e-7 for non-integer shapes near 1e9).
fn exact_deviation(x: f64, y: f64, a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    let dev = if x <= y {
        x.fma(s, -a) + x * e
    } else {
        -(y.fma(s, -b) + y * e)
    };
    (e, dev)
}

/// a ln(1 + dev/a) + b ln(1 - dev/b) (= ln of x^a y^b (a+b)^(a+b) / (a^a b^b))
/// for |dev| < min(a, b)/2, with the linear terms +-dev cancelled exactly: the
/// rounding is then relative to this (small) exponent, not to |dev|, which
/// grows like the square root of the shapes.
fn ln_kernel(a: f64, b: f64, dev: f64) -> f64 {
    a * log1pmx(dev / a) + b * log1pmx(-dev / b)
}

/// ln(1 + u) - u for |u| < 1/2 without cancellation: with v = u/(2+u),
/// ln(1 + u) = 2 (v + v^3/3 + v^5/5 + ...) (DLMF 4.6.4) and u - 2v = u v.
fn log1pmx(u: f64) -> f64 {
    let v = u / (2. + u);
    let v2 = v * v;
    let mut term = 2. * v * v2;
    let mut sum = term / 3.;
    let mut j = 5.;
    // |v| <= 1/3, so each term is at most 1/9 of the one before.
    for _ in 0..40 {
        term *= v2;
        let next = sum + term / j;
        if next == sum {
            break;
        }
        sum = next;
        j += 2.;
    }
    sum - u * v
}

/// (I_x(a, b), 1 - I_x(a, b)) for 0 < x < 1 with y = 1 - x given separately,
/// a, b > 0 finite: the continued fraction DLMF 8.17.22 on its fast side
/// (x <= (a+1)/(a+b+2)), the symmetry DLMF 8.17.4 on the other, each tail
/// computed directly where it is the small one.
fn beta_tails(x: f64, y: f64, a: f64, b: f64) -> (f64, f64) {
    if x <= 0. {
        return (0., 1.);
    }
    if y <= 0. {
        return (1., 0.);
    }
    // One parameter large, the other small, and the argument of the large
    // one near 1: the continued fraction there loses about a*eps, so that
    // side comes from the expansion of `beta_large_shape` and the other side
    // from the fraction (well conditioned there); the smaller is kept.
    if a >= LARGE_SHAPE && b <= SMALL_SHAPE && a >= 20. * b && y < 0.2 {
        let s0 = -libm::log1p(-y);
        let lower = beta_large_shape(a, b, s0);
        let upper = beta_kernel(y, x, b, a) / b * beta_fraction(b, a, y);
        // Q(b, a s0) <= 1/2 roughly from the gamma median b on.
        return if a * s0 >= b {
            (lower, 1. - lower)
        } else {
            (1. - upper, upper)
        };
    }
    if b >= LARGE_SHAPE && a <= SMALL_SHAPE && b >= 20. * a && x < 0.2 {
        let s0 = -libm::log1p(-x);
        let upper = beta_large_shape(b, a, s0);
        let lower = beta_kernel(x, y, a, b) / a * beta_fraction(a, b, x);
        return if b * s0 < a {
            (lower, 1. - lower)
        } else {
            (1. - upper, upper)
        };
    }
    if a.min(b) >= TEMME_SHAPE
        && let Some(tails) = beta_temme(x, y, a, b)
    {
        return tails;
    }
    if x <= (a + 1.) / (a + b + 2.) {
        let lower = beta_kernel(x, y, a, b) / a * beta_fraction(a, b, x);
        (lower, 1. - lower)
    } else {
        let upper = beta_kernel(y, x, b, a) / b * beta_fraction(b, a, y);
        (1. - upper, upper)
    }
}

/// Smallest shapes for which `beta_temme` replaces the continued fraction.
/// Near the mean the fraction's value comes from the cancellation of terms
/// close to 1 and loses about eps sqrt(a + b) (1e-11 at a = b = 1e9).
const TEMME_SHAPE: f64 = 5e3;
/// Half-width, in standard deviations, of the core around the mean where
/// `beta_temme` integrates the density instead of using the expansion.
const TEMME_CORE: f64 = 2.;

/// Kronrod's 15-point extension of the 7-point Gauss rule on [-1, 1]
/// (positive nodes, then their weights, the centre last).
const KRONROD_X: [f64; 7] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_4,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_5,
];
const KRONROD_W: [f64; 8] = [
    0.022_935_322_010_529_225,
    0.063_092_092_629_978_55,
    0.104_790_010_322_250_18,
    0.140_653_259_715_525_92,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_4,
    0.204_432_940_075_298_9,
    0.209_482_141_084_727_83,
];

/// The uniform asymptotic expansion of I_x(a, b) for large a and b (N. M.
/// Temme, SIAM J. Math. Anal. 18 (1987); DLMF 8.18(ii)), derived here as
/// follows. With s = a + b, x0 = a/s, y0 = b/s, r = sqrt(x0 y0) and
/// -xi^2/2 = x0 ln(t/x0) + y0 ln((1-t)/y0) (xi of the sign of t - x0),
/// t^(a-1) (1-t)^(b-1) dt = x0^a y0^b e^(-s xi^2/2) f(xi) dxi, f = xi/(t - x0).
/// Integrating by parts with G_k = (F_k(xi) - F_k(0))/xi, F_0 = f,
/// F_(k+1) = G_k', gives
///   I_x(a, b) = erfc(-xi sqrt(s/2))/2 - x^a y^b / (s B(a, b)) sum_k G_k(xi) / s^k,
///   G_0 = 1/u - 1/(r xi),  u = x - x0,
///   G_1 = -t(1-t)/u^3 + 1/(r xi^3) - F_1(0)/xi,
///   G_2 = (F_2(xi) - F_2(0))/xi,
///   F_2 = -xi t(1-t)(1-2t)/u^4 + 3 xi t^2 (1-t)^2/u^5 - 3/(r xi^4) + F_1(0)/xi^2,
/// all at t = x. The values F_k(0) follow from x -> 1, where the sum must
/// reduce to 1: r e^c (1 + r F_1(0)/s + r F_2(0)/s^2 + ...) = 1 with
/// c = c1/s + O(s^-3) the Stirling remainder of Gamma(s)/(Gamma(a) Gamma(b)),
/// c1 = (1 - 1/x0 - 1/y0)/12 (DLMF 5.11.1), so F_1(0) = -c1/r and
/// F_2(0) = c1^2/(2r). The neglected term is about (1/min(a, b))^3 times
/// the leading one.
///
/// Evaluated in standardised variables, so that no power under- or
/// overflows for any shapes: sd = sqrt(a b / s) (the standard deviation of
/// x s), v = dev/sd with dev = x s - a (`exact_deviation`, never x - x0),
/// z = xi sqrt(s) = sqrt(-2 ln_kernel), tau = t(1-t)/r^2 = (1 + dev/a)(1 - dev/b),
/// k1 = -c1/s = (1/a + 1/b - 1/s)/12, k2 = k1^2/2. Then
///   sum_k G_k / s^k = (sqrt(s)/r) [ (1/v - 1/z) + (1/z^3 - tau/v^3) - k1/z
///       + 3 tau^2/v^5 - tau (1-2t)/(v^4 sd) - 3/z^5 + k1/z^3 - k2/z ]
/// and x^a y^b / (B(a, b) r sqrt(s)) = e^(ln_kernel + c) / sqrt(2 pi).
/// The closed forms cancel near the mean (G_k like z^-(2k+1)), so within
/// `TEMME_CORE` standard deviations of it the density is integrated
/// (Kronrod 15-point rule, one piece per standard deviation) from the edge
/// of the core instead.
struct Temme {
    a: f64,
    b: f64,
    s: f64,
    sd: f64,
    /// e^c / sqrt(2 pi), c the Stirling remainder above.
    scale: f64,
    k1: f64,
}

/// (I_x(a, b), 1 - I_x(a, b)) by `Temme`, or None outside the range of
/// `ln_kernel` (there the tails are below 1e-300 or the fraction is fine).
// Out of line, so the small-shape path through beta_tails stays compact.
#[inline(never)]
fn beta_temme(x: f64, y: f64, a: f64, b: f64) -> Option<(f64, f64)> {
    beta_temme_at(a, b, exact_deviation(x, y, a, b).1)
}

/// `beta_temme` from the deviation dev = x (a + b) - a itself.
// Out of line, so the small-shape path through beta_tails stays compact.
#[inline(never)]
fn beta_temme_at(a: f64, b: f64, dev: f64) -> Option<(f64, f64)> {
    let s = a + b;
    if dev.is_nan() || dev.abs() >= 0.5 * a.min(b) || !s.is_finite() {
        return None;
    }
    let corr = stirling_correction(s) - stirling_correction(a) - stirling_correction(b);
    let t = Temme {
        a,
        b,
        s,
        sd: a.sqrt() * (b / s).sqrt(),
        scale: libm::exp(corr) * INV_SQRT_2PI,
        k1: (1. / a + 1. / b - 1. / s) / 12.,
    };
    let core = TEMME_CORE * t.sd;
    Some(if dev.abs() >= core {
        t.tails(dev)
    } else if dev <= 0. {
        let lower = t.tails(-core).0 + t.integral(-core, dev);
        (lower, 1. - lower)
    } else {
        let upper = t.tails(core).1 + t.integral(dev, core);
        (1. - upper, upper)
    })
}

impl Temme {
    /// The expansion at the deviation dev (|dev| >= the core).
    fn tails(&self, dev: f64) -> (f64, f64) {
        let (a, b, k1) = (self.a, self.b, self.k1);
        let ln = ln_kernel(a, b, dev);
        let z = (-2. * ln).sqrt().copysign(dev);
        let v = dev / self.sd;
        let tau = (1. + dev / a) * (1. - dev / b);
        let one_minus_2t = (b - a - 2. * dev) / self.s;
        let (v2, z2) = (v * v, z * z);
        let (v3, z3) = (v2 * v, z2 * z);
        let terms = (1. / v - 1. / z) + (1. / z3 - tau / v3) - k1 / z
            + (3. * tau * tau / (v3 * v2) - tau * one_minus_2t / (v2 * v2 * self.sd))
            - 3. / (z3 * z2)
            + k1 / z3
            - 0.5 * k1 * k1 / z;
        let sum = libm::exp(ln) * self.scale * terms;
        let w = z * FRAC_1_SQRT_2;
        (0.5 * libm::erfc(-w) - sum, 0.5 * libm::erfc(w) + sum)
    }

    /// Density of t in the variable dev (t = (a + dev)/s, dt = ddev/s):
    /// x^a y^b / (B(a, b) s t (1-t)) = e^(ln_kernel + c) / (sqrt(2 pi) sd tau).
    fn density(&self, dev: f64) -> f64 {
        let (a, b) = (self.a, self.b);
        let tau = (1. + dev / a) * (1. - dev / b);
        libm::exp(ln_kernel(a, b, dev)) * self.scale / (self.sd * tau)
    }

    /// Integral of the density over dev in [lo, hi] (at most a few standard
    /// deviations: one Kronrod piece per standard deviation).
    fn integral(&self, lo: f64, hi: f64) -> f64 {
        let pieces = ((hi - lo) / self.sd).ceil().clamp(1., 8.);
        let half = 0.5 * (hi - lo) / pieces;
        let mut sum = 0.;
        for i in 0..pieces as usize {
            let mid = lo + (2 * i + 1) as f64 * half;
            let mut piece = KRONROD_W[7] * self.density(mid);
            for (xk, wk) in KRONROD_X.iter().zip(KRONROD_W) {
                piece += wk * (self.density(mid - half * xk) + self.density(mid + half * xk));
            }
            sum += piece * half;
        }
        sum
    }
}

fn beta_pdf(x: f64, a: f64, b: f64) -> f64 {
    if !(0. ..=1.).contains(&x) {
        return 0.;
    }
    let edge = |shape: f64, other: f64| {
        if shape < 1. {
            f64::INFINITY
        } else if shape == 1. {
            // 1/B(1, other) = other
            other
        } else {
            0.
        }
    };
    if x == 0. {
        return edge(a, b);
    }
    if x == 1. {
        return edge(b, a);
    }
    let y = 1. - x;
    beta_kernel(x, y, a, b) / (x * y)
}

const LARGE_SHAPE: f64 = 100.;
const SMALL_SHAPE: f64 = 10.;

/// Bernoulli numbers B_2, B_4, ..., B_20 (DLMF table 24.2.1).
const BERNOULLI: [f64; 10] = [
    1. / 6.,
    -1. / 30.,
    1. / 42.,
    -1. / 30.,
    5. / 66.,
    -691. / 2730.,
    7. / 6.,
    -3617. / 510.,
    43867. / 798.,
    -174611. / 330.,
];

/// I_x(a, b) for large a and small b, x = e^-s0 near 1 (s0 = -ln x given
/// accurately). With t = e^-s in the integral of the incomplete beta function,
/// (1 - e^-s)^(b-1) = s^(b-1) f(s), ln f(s) = (b-1) (-s/2 + sum B_2n s^2n /
/// (2n (2n)!)) (the series of ln(sinh(s/2)/(s/2)), DLMF 4.33 with 24.2), so
/// with f = sum c_k s^k, termwise integration gives
/// I_x(a, b) = Gamma(a+b) / (Gamma(a) Gamma(b) a^b)
///             * sum_k c_k (b)_k a^-k Q(b+k, a s0),
/// every term positive for the shapes used here, Q the upper regularised
/// incomplete gamma function (DLMF 8.2.4).
fn beta_large_shape(a: f64, b: f64, s0: f64) -> f64 {
    const TERMS: usize = 24;
    // g_j: coefficients of ln f.
    let mut g = [0.; TERMS];
    g[1] = -(b - 1.) / 2.;
    let mut factorial = 2.; // (2n)!
    for (n, bern) in BERNOULLI.iter().enumerate() {
        let k = 2 * (n + 1);
        if k >= TERMS {
            break;
        }
        if n > 0 {
            factorial *= ((k - 1) * k) as f64;
        }
        g[k] = (b - 1.) * bern / (k as f64 * factorial);
    }
    // c_k of f = exp(ln f): k c_k = sum_j j g_j c_(k-j).
    let mut c = [0.; TERMS];
    c[0] = 1.;
    for k in 1..TERMS {
        let mut acc = 0.;
        for j in 1..=k {
            acc += j as f64 * g[j] * c[k - j];
        }
        c[k] = acc / k as f64;
    }
    let z = a * s0;
    // Gamma(a+b) / (Gamma(a) a^b) by Stirling's series (DLMF 5.11.1, a >= 10).
    let ratio = libm::exp(
        (a + b - 0.5) * libm::log1p(b / a) - b + stirling_correction(a + b)
            - stirling_correction(a),
    );
    let mut sum = 0.;
    let mut rising = 1.; // (b)_k / a^k
    for (k, ck) in c.iter().enumerate() {
        let term = ck * rising * gammainc_real(z, b + k as f64, Tail::Upper);
        sum += term;
        if term.abs() <= 1e-17 * sum.abs() && k > 2 {
            break;
        }
        rising *= (b + k as f64) / a;
    }
    ratio * sum
}

// ---------- Student t ----------

/// Gamma((nu+1)/2) / (sqrt(nu pi) Gamma(nu/2)).
fn t_constant(nu: f64) -> f64 {
    let z = nu / 2.;
    if z >= 10. {
        // Gamma(z+1/2)/Gamma(z) = sqrt(z) exp(z log1p(1/(2z)) - 1/2 + dc)
        // from Stirling's series of both (DLMF 5.11.1).
        let e =
            z * libm::log1p(0.5 / z) - 0.5 + stirling_correction(z + 0.5) - stirling_correction(z);
        // sqrt(z) / sqrt(nu pi) = 1/sqrt(2 pi)
        INV_SQRT_2PI * libm::exp(e)
    } else {
        libm::tgamma(z + 0.5) / (libm::tgamma(z) * (nu * PI).sqrt())
    }
}

fn t_pdf(x: f64, nu: f64) -> f64 {
    if nu.is_infinite() {
        return norm_pdf(x);
    }
    if x.is_infinite() {
        return 0.;
    }
    if t_far(x, nu) {
        // C (x^2/nu)^(-(nu+1)/2) to a relative 1e-17, without squaring x
        return libm::exp(
            libm::log(t_constant(nu)) + 0.5 * (nu + 1.) * libm::log(nu)
                - (nu + 1.) * libm::log(x.abs()),
        );
    }
    // (1 + x^2/nu)^(-(nu+1)/2)
    let power = libm::exp(-0.5 * (nu + 1.) * libm::log1p(x * x / nu));
    t_constant(nu) * power
}

/// |x| so large that nu/x^2 < 1e-17: the leading terms of the density and of
/// the tail (C nu^((nu+1)/2) |x|^-(nu+1) and that times |x|/nu) are then exact
/// to working precision, and they need no x^2 (which may overflow).
fn t_far(x: f64, nu: f64) -> bool {
    x.abs() > 3.2e8 * nu.sqrt()
}

/// ln of the tail P(T > |x|) ~ C nu^((nu-1)/2) |x|^-nu for `t_far` arguments.
fn t_far_ln_tail(ax_ln: f64, nu: f64) -> f64 {
    libm::log(t_constant(nu)) + 0.5 * (nu - 1.) * libm::log(nu) - nu * ax_ln
}

/// (P(T <= x), P(T > x)): the tail beyond |x| is I_{nu/(nu+x^2)}(nu/2, 1/2)/2
/// (DLMF 8.17 with Abramowitz and Stegun 26.7.1), its complement
/// I_{x^2/(nu+x^2)}(1/2, nu/2)/2; each is evaluated from its own argument.
fn t_tails(x: f64, nu: f64) -> (f64, f64) {
    if nu.is_infinite() {
        return (norm_cdf(x, false), norm_cdf(x, true));
    }
    if x.is_infinite() {
        return if x > 0. { (1., 0.) } else { (0., 1.) };
    }
    if x == 0. {
        return (0.5, 0.5);
    }
    if t_far(x, nu) {
        let tail = libm::exp(t_far_ln_tail(libm::log(x.abs()), nu));
        return if x < 0. {
            (tail, 1. - tail)
        } else {
            (1. - tail, tail)
        };
    }
    let x2 = x * x;
    let (tail, centre) = if x2 < nu {
        let w = x2 / (nu + x2);
        let (c, t) = beta_tails(w, nu / (nu + x2), 0.5, nu / 2.);
        (0.5 * t, 0.5 * c)
    } else {
        let w = nu / (nu + x2);
        let (t, c) = beta_tails(w, x2 / (nu + x2), nu / 2., 0.5);
        (0.5 * t, 0.5 * c)
    };
    if x < 0. {
        (tail, 0.5 + centre)
    } else {
        (0.5 + centre, tail)
    }
}

pub(in crate::engine) fn t_quantile(p: f64, nu: f64) -> f64 {
    if p == 0. {
        return f64::NEG_INFINITY;
    }
    if p == 1. {
        return f64::INFINITY;
    }
    if p == 0.5 {
        return 0.;
    }
    if nu.is_infinite() {
        return norm_quantile(p);
    }
    if p > 0.5 {
        return -t_quantile(1. - p, nu);
    }
    // Far tail: invert the leading term, kept when it lands where it is exact.
    let far = -libm::exp(
        (libm::log(t_constant(nu)) + 0.5 * (nu - 1.) * libm::log(nu) - libm::log(p)) / nu,
    );
    if t_far(far, nu) {
        return far;
    }
    // Tail 2p = I_{w1}(nu/2, 1/2), w1 = nu/(nu+x^2), w2 = 1 - w1 = x^2/(nu+x^2).
    let w2 = betaincinv_real(2. * p, 0.5, nu / 2., true);
    if w2 < 0.5 {
        -(nu * w2 / (1. - w2)).sqrt()
    } else {
        let w1 = betaincinv_real(2. * p, nu / 2., 0.5, false);
        -(nu * (1. - w1) / w1).sqrt()
    }
}

// ---------- F ----------

/// (y, 1 - y) with y = nu1 x / (nu1 x + nu2), both from their own quotients.
fn f_split(x: f64, n1: f64, n2: f64) -> (f64, f64) {
    let a = n1 * x;
    let d = a + n2;
    if a.is_infinite() {
        return (1., n2 / a);
    }
    (a / d, n2 / d)
}

/// With y as in `f_split`, the density is x^-1 y^(n1/2) (1-y)^(n2/2) / B(n1/2, n2/2).
fn f_pdf(x: f64, n1: f64, n2: f64) -> f64 {
    if x < 0. || x.is_infinite() {
        return 0.;
    }
    let a = n1 / 2.;
    if x == 0. {
        return if a < 1. {
            f64::INFINITY
        } else if a == 1. {
            1.
        } else {
            0.
        };
    }
    let (y, w) = f_split(x, n1, n2);
    beta_kernel(y, w, a, n2 / 2.) / x
}

fn f_tails(x: f64, n1: f64, n2: f64) -> (f64, f64) {
    if x <= 0. {
        return (0., 1.);
    }
    if x.is_infinite() {
        return (1., 0.);
    }
    let (a, b) = (n1 / 2., n2 / 2.);
    // n1 x overflowing would give a zero deviation (the centre); f_split handles it.
    if a.min(b) >= TEMME_SHAPE && (n1 * x + n2).is_finite() {
        // The deviation y (a+b) - a of y = n1 x/(n1 x + n2), straight from
        // x - 1: rounding y itself would cost eps times the condition number
        // of the cdf near the mean (4e4 for n1 = n2 = 2e9).
        let dev = a * (n2 / (n1 * x + n2)) * (x - 1.);
        if let Some(tails) = beta_temme_at(a, b, dev) {
            return tails;
        }
    }
    let (y, w) = f_split(x, n1, n2);
    beta_tails(y, w, a, b)
}

pub(in crate::engine) fn f_quantile(p: f64, n1: f64, n2: f64) -> f64 {
    if p == 0. {
        return 0.;
    }
    if p == 1. {
        return f64::INFINITY;
    }
    let (a, b) = (n1 / 2., n2 / 2.);
    let y = betaincinv_real(p, a, b, false);
    if y <= 0.5 {
        n2 * y / (n1 * (1. - y))
    } else {
        // 1 - y solves I_w(b, a) = 1 - p, i.e. the upper tail of I_w(b, a) is p.
        let w = betaincinv_real(p, b, a, true);
        n2 * (1. - w) / (n1 * w)
    }
}

// ---------- Weibull ----------

fn wbl_pdf(z: f64, b: f64) -> f64 {
    if z < 0. || z.is_infinite() {
        return 0.;
    }
    if z == 0. {
        return if b < 1. {
            f64::INFINITY
        } else if b == 1. {
            1.
        } else {
            0.
        };
    }
    let s = libm::pow(z, b);
    if s.is_infinite() {
        // z^b beyond the doubles: the factor exp(-z^b) underflowed long
        // before, and b (z^b / z) exp(-z^b) would be Inf * 0.
        return 0.;
    }
    // z^(b-1) as z^b / z, or directly where z^b underflowed (b - 1 is then
    // exact for b >= 1/2, and z^b / z would lose every digit).
    let power = if s >= f64::MIN_POSITIVE {
        s / z
    } else {
        libm::pow(z, b - 1.)
    };
    b * power * libm::exp(-s)
}

// ---------- binomial, Poisson, negative binomial (Loader 2000) ----------

/// ln(n!) for n = 0..=22, from the exact factorials (each correctly rounded).
fn ln_factorial_small(n: usize) -> f64 {
    let mut f = 1f64;
    for i in 2..=n {
        f *= i as f64;
    }
    libm::log(f)
}

/// Stirling's remainder ln Gamma(n+1) - (n+1/2) ln n + n - ln sqrt(2 pi).
fn stirlerr(n: f64) -> f64 {
    if n > 15. {
        // stirling_correction(n) is the same series in 1/n (DLMF 5.11.1).
        return stirling_correction(n);
    }
    let lg = if n.fract() == 0. && n <= 22. {
        ln_factorial_small(n as usize)
    } else {
        libm::lgamma(n + 1.)
    };
    lg - (n + 0.5) * libm::log(n) + n - LN_SQRT_2PI
}

/// The deviance term x ln(x/m) + m - x, by the series in v = (x-m)/(x+m) when x
/// is near m: x ln(x/m) + m - x = (x-m) v + 2x (v^3/3 + v^5/5 + ...).
fn bd0(x: f64, m: f64) -> f64 {
    if (x - m).abs() < 0.1 * (x + m) {
        let v = (x - m) / (x + m);
        let mut s = (x - m) * v;
        let v2 = v * v;
        let mut ej = 2. * x * v;
        for j in 1..1000 {
            ej *= v2;
            let next = s + ej / f64::from(2 * j + 1);
            if next == s {
                return next;
            }
            s = next;
        }
        return s;
    }
    x * libm::log(x / m) + m - x
}

/// C(n, x) p^x q^(n-x) for real n >= x >= 0 (q = 1 - p given separately).
fn binomial_mass(x: f64, n: f64, p: f64, q: f64) -> f64 {
    if x > n {
        return 0.;
    }
    if p == 0. {
        return if x == 0. { 1. } else { 0. };
    }
    if q == 0. {
        return if x == n { 1. } else { 0. };
    }
    if x == 0. {
        if n == 0. {
            return 1.;
        }
        return libm::exp(n * libm::log1p(-p));
    }
    if x == n {
        return libm::exp(n * libm::log(p));
    }
    let lc = stirlerr(n) - stirlerr(x) - stirlerr(n - x) - bd0(x, n * p) - bd0(n - x, n * q);
    let lf = LN_2PI + libm::log(x) + libm::log1p(-x / n);
    libm::exp(lc - 0.5 * lf)
}

fn poisson_mass(x: f64, lambda: f64) -> f64 {
    if lambda == 0. {
        return if x == 0. { 1. } else { 0. };
    }
    if x == 0. {
        return libm::exp(-lambda);
    }
    libm::exp(-stirlerr(x) - bd0(x, lambda)) / (2. * PI * x).sqrt()
}

/// Gamma(r+k)/(Gamma(r) k!) p^r q^k = r/(r+k) * C(r+k, r) p^r q^k.
fn nbin_mass(k: f64, r: f64, p: f64) -> f64 {
    if p == 1. {
        return if k == 0. { 1. } else { 0. };
    }
    let n = r + k;
    let mass = binomial_mass(r, n, p, 1. - p);
    if k == 0. { mass } else { mass * (r / n) }
}

// ---------- random draws ----------

/// Standard gamma variate with shape a > 0 (Marsaglia and Tsang 2000).
fn gamma_draw(a: f64, g: &mut Generator) -> f64 {
    if a < 1. {
        let boost = libm::pow(g.next_f64(), 1. / a);
        return gamma_draw(a + 1., g) * boost;
    }
    let d = a - 1. / 3.;
    let c = 1. / (9. * d).sqrt();
    loop {
        let (x, v) = loop {
            let x = g.next_normal();
            let v = 1. + c * x;
            if v > 0. {
                break (x, v * v * v);
            }
        };
        let u = g.next_f64();
        let x2 = x * x;
        if u < 1. - 0.0331 * x2 * x2 {
            return d * v;
        }
        if libm::log(u) < 0.5 * x2 + d * (1. - v + libm::log(v)) {
            return d * v;
        }
    }
}

/// Beta variate as X/(X+Y) of two gamma variates.
fn beta_draw(a: f64, b: f64, g: &mut Generator) -> f64 {
    for _ in 0..1000 {
        let x = gamma_draw(a, g);
        let y = gamma_draw(b, g);
        let s = x + y;
        if s > 0. {
            return x / s;
        }
    }
    // Both shapes so small that every draw underflowed: the mass sits at the ends.
    if g.next_f64() < a / (a + b) { 1. } else { 0. }
}
