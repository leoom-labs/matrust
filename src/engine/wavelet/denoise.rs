//! Threshold selection and wavelet denoising from the published methods:
//! Donoho & Johnstone (1994, 1995) for 'sqtwolog', 'rigrsure' (SURE),
//! 'heursure' and 'minimaxi'; Johnstone & Silverman (2005, empirical Bayes
//! with a Laplace prior of scale a = 0.5, mixing weight by marginal maximum
//! likelihood) for 'Bayes'; Abramovich, Benjamini, Donoho & Johnstone (2006)
//! for 'FDR'; Cai (1999) block James-Stein for 'BlockJS'.
use super::super::number_theory::erfcinv_fn;
use super::super::special_fns::erfcx_value;
use crate::Result;

/// Threshold selection rule of `thselect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rule {
    Rigrsure,
    Heursure,
    Sqtwolog,
    Minimaxi,
}

impl Rule {
    pub(super) fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "rigrsure" => Self::Rigrsure,
            "heursure" => Self::Heursure,
            "sqtwolog" => Self::Sqtwolog,
            "minimaxi" => Self::Minimaxi,
            _ => return None,
        })
    }
}

/// Threshold for data assumed to have unit noise level.
pub(super) fn thselect(x: &[f64], rule: Rule) -> f64 {
    let n = x.len();
    if n == 0 {
        return 0.;
    }
    let nf = n as f64;
    match rule {
        Rule::Sqtwolog => (2. * nf.ln()).sqrt(),
        Rule::Minimaxi => minimaxi(n),
        Rule::Rigrsure => rigrsure(x),
        Rule::Heursure => {
            let hthr = (2. * nf.ln()).sqrt();
            let eta = (x.iter().map(|v| v * v).sum::<f64>() - nf) / nf;
            let crit = nf.log2().powf(1.5) / nf.sqrt();
            if eta < crit {
                hthr
            } else {
                rigrsure(x).min(hthr)
            }
        }
    }
}

fn minimaxi(n: usize) -> f64 {
    if n <= 32 {
        0.
    } else {
        0.3936 + 0.1829 * (n as f64).log2()
    }
}

/// Stein's unbiased risk estimate minimiser over the candidate thresholds |x_i|.
fn rigrsure(x: &[f64]) -> f64 {
    let n = x.len();
    let mut sx2: Vec<f64> = x.iter().map(|v| v * v).collect();
    sx2.sort_by(f64::total_cmp);
    let nf = n as f64;
    let mut cum = 0.;
    let (mut best, mut best_risk) = (0, f64::INFINITY);
    for (i, &s) in sx2.iter().enumerate() {
        cum += s;
        let risk = (nf - 2. * (i + 1) as f64 + cum + (n - 1 - i) as f64 * s) / nf;
        if risk < best_risk {
            best_risk = risk;
            best = i;
        }
    }
    sx2[best].sqrt()
}

/// Soft (`soft = true`) or hard thresholding.
pub(super) fn wthresh(v: f64, soft: bool, t: f64) -> f64 {
    if soft {
        let m = v.abs() - t;
        if m > 0. { m.copysign(v) } else { 0. }
    } else if v.abs() > t {
        v
    } else {
        0.
    }
}

/// median(|d|) / 0.6745 (`wnoisest`).
pub(super) fn noise_sigma(d: &[f64]) -> f64 {
    if d.is_empty() {
        return 0.;
    }
    let mut a: Vec<f64> = d.iter().map(|v| v.abs()).collect();
    a.sort_by(f64::total_cmp);
    let n = a.len();
    let med = if n % 2 == 1 {
        a[n / 2]
    } else {
        (a[n / 2 - 1] + a[n / 2]) / 2.
    };
    med / 0.6745
}

// ---------- empirical Bayes (Laplace prior) ----------

const A: f64 = 0.5;
const SQRT_2: f64 = std::f64::consts::SQRT_2;

/// Mills ratio (1 - Phi(z)) / phi(z).
fn mills(z: f64) -> f64 {
    (std::f64::consts::PI / 2.).sqrt() * erfcx_value(z / SQRT_2)
}
fn pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / (2. * std::f64::consts::PI).sqrt()
}
fn cdf(z: f64) -> f64 {
    0.5 * libm::erfc(-z / SQRT_2)
}
fn cdf_inv(p: f64) -> f64 {
    -SQRT_2 * erfcinv_fn(2. * p)
}

/// beta(x) = g(x)/phi(x) - 1 for the Laplace density convolved with N(0,1).
fn beta(x: f64) -> f64 {
    let x = x.abs();
    A / 2. * (mills(x + A) + mills(A - x)) - 1.
}

/// Mixing weight whose posterior-median threshold is `t`.
fn w_from_t(t: f64) -> f64 {
    1. / (A / 2. * mills(A - t) - A / 2. * mills(t + A) + 1.)
}

/// Posterior-median threshold of the weight `w` (bisection to full precision).
pub(super) fn t_from_w(w: f64) -> f64 {
    let (mut lo, mut hi) = (0., 20.);
    for _ in 0..300 {
        let mid = (lo + hi) / 2.;
        if mid == lo || mid == hi {
            break;
        }
        if A / 2. * mills(A - mid) - A / 2. * mills(mid + A) + 1. < 1. / w {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.
}

/// Marginal maximum likelihood weight in [w(sqrt(2 log n)), 1].
pub(super) fn w_from_x(x: &[f64]) -> f64 {
    let b: Vec<f64> = x.iter().map(|&v| beta(v)).collect();
    let wlo = w_from_t((2. * (x.len() as f64).ln()).sqrt());
    let score = |w: f64| -> f64 {
        b.iter()
            .map(|&bi| {
                if bi.is_infinite() {
                    1. / w
                } else {
                    bi / (1. + w * bi)
                }
            })
            .sum()
    };
    if score(1.) >= 0. {
        return 1.;
    }
    if score(wlo) <= 0. {
        return wlo;
    }
    let (mut lo, mut hi) = (wlo, 1.);
    for _ in 0..300 {
        let mid = (lo + hi) / 2.;
        if mid == lo || mid == hi {
            break;
        }
        if score(mid) > 0. {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.
}

/// Posterior median of mu given x (unit noise).
fn post_median(x: f64, w: f64) -> f64 {
    let ax = x.abs();
    let xma = ax - A;
    let cor = if xma > 25. {
        0.
    } else {
        // (1/a) phi(x-a) (1/w + beta(x)), with phi(x-a) * Phi(x-a)/phi(x-a) folded.
        let p = pdf(xma);
        let zz = (p / w + A / 2. * p * mills(ax + A) + A / 2. * cdf(xma) - p) / A;
        cdf_inv(zz.min(1.))
    };
    let m = (xma - cor).max(0.);
    if x < 0. { -m } else { m }
}

/// Posterior mean of mu given x (unit noise).
fn post_mean(x: f64, w: f64) -> f64 {
    let ax = x.abs();
    let b = beta(ax);
    let wpost = 1. - (1. - w) / (1. + w * b);
    let xma = ax - A;
    let r = pdf(xma) * mills(ax + A) / cdf(xma);
    let cond = ax - A * (1. - r) / (1. + r);
    let m = wpost * cond;
    if x < 0. { -m } else { m }
}

// ---------- wdenoise methods ----------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Method {
    Bayes,
    Sure,
    Universal,
    Minimax,
    Fdr(f64),
    BlockJs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ThRule {
    Median,
    Mean,
    Soft,
    Hard,
    JamesStein,
}

/// Cai (1999): root of lambda - log(lambda) - 3 = 0.
const LAMBDA_JS: f64 = 4.50524;

/// Denoised detail coefficients of one level; `sigma` is the noise level and
/// `n_signal` the number of samples of the signal.
pub(super) fn denoise_level(
    d: &[f64],
    sigma: f64,
    method: Method,
    rule: ThRule,
    n_signal: usize,
) -> Result<Vec<f64>> {
    if sigma <= 0. || d.is_empty() {
        return Ok(d.to_vec());
    }
    let soft = rule == ThRule::Soft;
    let apply = |t: f64| d.iter().map(|&v| wthresh(v, soft, t)).collect();
    Ok(match method {
        Method::Bayes => {
            let z: Vec<f64> = d.iter().map(|v| v / sigma).collect();
            let w = w_from_x(&z);
            match rule {
                ThRule::Median => z.iter().map(|&v| post_median(v, w) * sigma).collect(),
                ThRule::Mean => z.iter().map(|&v| post_mean(v, w) * sigma).collect(),
                _ => {
                    let t = t_from_w(w);
                    z.iter().map(|&v| wthresh(v, soft, t) * sigma).collect()
                }
            }
        }
        Method::Universal => apply(sigma * (2. * (n_signal as f64).ln()).sqrt()),
        Method::Minimax => apply(sigma * minimaxi(n_signal)),
        Method::Sure => {
            let z: Vec<f64> = d.iter().map(|v| v / sigma).collect();
            apply(sigma * rigrsure(&z))
        }
        Method::Fdr(q) => {
            let m = d.len();
            let mut p: Vec<f64> = d
                .iter()
                .map(|v| libm::erfc(v.abs() / (sigma * SQRT_2)))
                .collect();
            p.sort_by(f64::total_cmp);
            let k = (0..m)
                .rev()
                .find(|&i| p[i] <= q * (i + 1) as f64 / m as f64);
            let t = match k {
                Some(i) => sigma * SQRT_2 * erfcinv_fn(q * (i + 1) as f64 / m as f64),
                None => d.iter().fold(0., |acc: f64, v| acc.max(v.abs())),
            };
            d.iter().map(|&v| wthresh(v, false, t)).collect()
        }
        Method::BlockJs => {
            let block = ((n_signal as f64).ln().floor() as usize).max(1);
            let mut y = d.to_vec();
            for chunk in y.chunks_mut(block) {
                let s2: f64 = chunk.iter().map(|v| v * v).sum();
                let f = if s2 == 0. {
                    0.
                } else {
                    (1. - LAMBDA_JS * chunk.len() as f64 * sigma * sigma / s2).max(0.)
                };
                chunk.iter_mut().for_each(|v| *v *= f);
            }
            y
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_and_hard_thresholds() {
        assert_eq!(wthresh(-3., true, 1.), -2.);
        assert_eq!(wthresh(0.5, true, 1.), 0.);
        assert_eq!(wthresh(1., false, 1.), 0.);
        assert_eq!(wthresh(-1.5, false, 1.), -1.5);
    }

    #[test]
    fn bayes_threshold_is_zero_for_weight_one() {
        assert!(t_from_w(1.) < 1e-12);
        assert!((w_from_t(t_from_w(0.2)) - 0.2).abs() < 1e-12);
    }
}
