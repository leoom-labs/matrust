//! Single-level and multilevel 1-D discrete wavelet transform kernels: the
//! extension modes `dwtmode` documents, analysis by extension + convolution +
//! dyadic downsampling, synthesis by upsampling + convolution + central part,
//! and the wavedec bookkeeping vector (C = [cA_n cD_n ... cD_1],
//! L = [len(cA_n) len(cD_n) ... len(cD_1) len(x)]).
use crate::Result;

/// Signal extension mode (`dwtmode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::engine) enum Mode {
    Zpd,
    #[default]
    Sym,
    Symw,
    Asym,
    Asymw,
    Sp0,
    Sp1,
    Ppd,
    Per,
}

impl Mode {
    pub(super) fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "zpd" => Self::Zpd,
            "sym" | "symh" => Self::Sym,
            "symw" => Self::Symw,
            "asym" | "asymh" => Self::Asym,
            "asymw" => Self::Asymw,
            "sp0" => Self::Sp0,
            "sp1" | "spd" => Self::Sp1,
            "ppd" => Self::Ppd,
            "per" => Self::Per,
            _ => return None,
        })
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Zpd => "zpd",
            Self::Sym => "sym",
            Self::Symw => "symw",
            Self::Asym => "asym",
            Self::Asymw => "asymw",
            Self::Sp0 => "sp0",
            Self::Sp1 => "sp1",
            Self::Ppd => "ppd",
            Self::Per => "per",
        }
    }
    pub(super) fn describe(self) -> &'static str {
        match self {
            Self::Zpd => "Zero Padding",
            Self::Sym => "Symmetrization (half-point)",
            Self::Symw => "Symmetrization (whole-point)",
            Self::Asym => "Antisymmetrization (half-point)",
            Self::Asymw => "Antisymmetrization (whole-point)",
            Self::Sp0 => "Smooth Padding of order 0",
            Self::Sp1 => "Smooth Padding of order 1",
            Self::Ppd => "Periodized",
            Self::Per => "Periodization",
        }
    }
}

/// `x` extended by `n` samples on each side. `Per` extends periodically
/// (the caller makes the length even first).
pub(super) fn extend(x: &[f64], n: usize, mode: Mode) -> Vec<f64> {
    let len = x.len();
    let mut out = Vec::with_capacity(len + 2 * n);
    if len == 0 {
        out.resize(2 * n, 0.);
        return out;
    }
    if mode == Mode::Asymw {
        return extend_asymw(x, n);
    }
    let l = len as i64;
    let at = |i: i64| -> f64 {
        if (0..l).contains(&i) {
            return x[i as usize];
        }
        match mode {
            Mode::Zpd => 0.,
            Mode::Sp0 => {
                if i < 0 {
                    x[0]
                } else {
                    x[len - 1]
                }
            }
            Mode::Sp1 => {
                if len == 1 {
                    x[0]
                } else if i < 0 {
                    x[0] + i as f64 * (x[1] - x[0])
                } else {
                    x[len - 1] + (i - l + 1) as f64 * (x[len - 1] - x[len - 2])
                }
            }
            Mode::Ppd | Mode::Per => x[i.rem_euclid(l) as usize],
            Mode::Sym | Mode::Asym => {
                let j = i.rem_euclid(2 * l);
                if j < l {
                    x[j as usize]
                } else if mode == Mode::Sym {
                    x[(2 * l - 1 - j) as usize]
                } else {
                    -x[(2 * l - 1 - j) as usize]
                }
            }
            Mode::Symw => {
                if len == 1 {
                    return x[0];
                }
                let p = 2 * l - 2;
                let j = i.rem_euclid(p);
                if j < l {
                    x[j as usize]
                } else {
                    x[(p - j) as usize]
                }
            }
            Mode::Asymw => unreachable!(),
        }
    };
    let n = n as i64;
    out.extend((-n..l + n).map(at));
    out
}

/// Whole-point antisymmetric extension, x(-k) = 2 x(0) - x(k); longer
/// extensions repeat the rule on the already extended signal.
fn extend_asymw(x: &[f64], n: usize) -> Vec<f64> {
    if x.len() == 1 {
        return vec![x[0]; 2 * n + 1];
    }
    let mut cur = x.to_vec();
    let (mut left, mut right) = (0, 0);
    while left < n || right < n {
        let m = cur.len();
        let kl = (n - left).min(m - 1);
        let kr = (n - right).min(m - 1);
        let (first, last) = (cur[0], cur[m - 1]);
        let mut next = Vec::with_capacity(m + kl + kr);
        next.extend((1..=kl).rev().map(|k| 2. * first - cur[k]));
        next.extend_from_slice(&cur);
        next.extend((1..=kr).map(|k| 2. * last - cur[m - 1 - k]));
        cur = next;
        left += kl;
        right += kr;
    }
    cur
}

/// Valid convolution of `y` with `h`, every second output starting at `first`.
fn conv_down(y: &[f64], h: &[f64], first: usize, count: usize) -> Vec<f64> {
    let lf = h.len();
    (0..count)
        .map(|o| {
            let i = first + 2 * o;
            // z[i] = sum_j h[j] y[i + lf - 1 - j]
            let mut s = 0.;
            for (j, &hj) in h.iter().enumerate() {
                s += hj * y[i + lf - 1 - j];
            }
            s
        })
        .collect()
}

/// One analysis step: approximation and detail coefficients.
pub(super) fn dwt(x: &[f64], lo: &[f64], hi: &[f64], mode: Mode) -> (Vec<f64>, Vec<f64>) {
    let lf = lo.len();
    if mode == Mode::Per {
        let mut xe = x.to_vec();
        if xe.len() % 2 == 1 {
            xe.push(xe[xe.len() - 1]);
        }
        let count = xe.len() / 2;
        let y = extend(&xe, lf / 2, Mode::Per);
        return (conv_down(&y, lo, 1, count), conv_down(&y, hi, 1, count));
    }
    let y = extend(x, lf - 1, mode);
    let count = (x.len() + lf - 1) / 2;
    (conv_down(&y, lo, 1, count), conv_down(&y, hi, 1, count))
}

/// Upsample `c` (zeros between samples) and convolve with `f`, adding into `acc`.
fn upconv_add(acc: &mut [f64], c: &[f64], f: &[f64]) {
    for (k, &ck) in c.iter().enumerate() {
        if ck == 0. {
            continue;
        }
        let base = 2 * k;
        for (j, &fj) in f.iter().enumerate() {
            acc[base + j] += ck * fj;
        }
    }
}

/// One synthesis step from `a` and/or `d` (an empty slice is a zero branch).
/// `keep` is the length of the central part returned (default: 2n - lf + 2,
/// 2n for 'per').
pub(super) fn idwt(
    a: &[f64],
    d: &[f64],
    lo_r: &[f64],
    hi_r: &[f64],
    mode: Mode,
    keep: Option<usize>,
) -> Result<Vec<f64>> {
    let n = a.len().max(d.len());
    if !a.is_empty() && !d.is_empty() && a.len() != d.len() {
        return Err("idwt: cA and cD must have the same length".into());
    }
    let lf = lo_r.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    if mode == Mode::Per {
        // Circular synthesis: tap j of coefficient k lands on 2k + j - (lf/2 - 1).
        let m = 2 * n;
        let shift = (lf / 2) as i64 - 1;
        let mut x = vec![0.; m];
        for (c, f) in [(a, lo_r), (d, hi_r)] {
            for (k, &ck) in c.iter().enumerate() {
                for (j, &fj) in f.iter().enumerate() {
                    let at = (2 * k + j) as i64 - shift;
                    x[at.rem_euclid(m as i64) as usize] += ck * fj;
                }
            }
        }
        return central(x, keep.unwrap_or(m), "idwt");
    }
    // dyadup(c, 0) has 2n - 1 samples; the full convolution 2n + lf - 2.
    let full_len = 2 * n + lf - 2;
    let mut full = vec![0.; full_len];
    upconv_add(&mut full, a, lo_r);
    upconv_add(&mut full, d, hi_r);
    let default = (2 * n + 2)
        .checked_sub(lf)
        .filter(|&k| k > 0)
        .ok_or("idwt: the coefficient vectors are too short for the reconstruction filters")?;
    central(full, keep.unwrap_or(default), "idwt")
}

/// The length-`len` central part of `x` (one extra sample drops at the end).
fn central(x: Vec<f64>, len: usize, who: &str) -> Result<Vec<f64>> {
    if len > x.len() {
        return Err(format!(
            "{who}: the requested length {len} exceeds the {} samples the coefficients give",
            x.len()
        ));
    }
    let first = (x.len() - len) / 2;
    Ok(x[first..first + len].to_vec())
}

/// Multilevel decomposition; `check` runs between levels (cancellation).
pub(super) fn wavedec(
    x: &[f64],
    levels: usize,
    lo: &[f64],
    hi: &[f64],
    mode: Mode,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<(Vec<f64>, Vec<usize>)> {
    let mut a = x.to_vec();
    let mut details: Vec<Vec<f64>> = Vec::with_capacity(levels);
    for _ in 0..levels {
        check()?;
        let (ca, cd) = dwt(&a, lo, hi, mode);
        details.push(cd);
        a = ca;
    }
    let mut l = Vec::with_capacity(levels + 2);
    l.push(a.len());
    let mut c = a;
    for d in details.iter().rev() {
        l.push(d.len());
        c.extend_from_slice(d);
    }
    l.push(x.len());
    Ok((c, l))
}

/// Checks C against the bookkeeping vector; returns the number of levels.
///
/// Callers cast each entry with `as usize`, which saturates a finite value
/// that does not fit to `usize::MAX`. Those entries, and any sum that wraps,
/// are an invalid bookkeeping vector (waverec, appcoef, detcoef, wrcoef, wden).
pub(super) fn check_book(c: &[f64], l: &[usize], who: &str) -> Result<usize> {
    if l.len() < 2 || l.contains(&usize::MAX) {
        return Err(format!("{who}: invalid bookkeeping vector L"));
    }
    let total = l[..l.len() - 1]
        .iter()
        .try_fold(0usize, |acc, &n| acc.checked_add(n))
        .ok_or_else(|| format!("{who}: invalid bookkeeping vector L"))?;
    if total != c.len() {
        return Err(format!(
            "{who}: the bookkeeping vector L does not match C (sum(L(1:end-1)) must equal numel(C))"
        ));
    }
    Ok(l.len() - 2)
}

/// Detail coefficients of level `k` (1 = finest).
pub(super) fn detail(c: &[f64], l: &[usize], k: usize) -> Vec<f64> {
    let levels = l.len() - 2;
    // Position of cD_k in C: after cA_n and cD_n .. cD_{k+1}.
    let start: usize = l[..=levels - k].iter().sum();
    c[start..start + l[levels - k + 1]].to_vec()
}

/// Approximation at level `n` (0 = the reconstructed signal).
pub(super) fn approx(
    c: &[f64],
    l: &[usize],
    lo_r: &[f64],
    hi_r: &[f64],
    n: usize,
    mode: Mode,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<f64>> {
    let levels = l.len() - 2;
    let n0 = l.first().copied().unwrap_or(0);
    if n0 > c.len() {
        return Err("invalid bookkeeping vector L".into());
    }
    let mut a = c[..n0].to_vec();
    for k in (n + 1..=levels).rev() {
        check()?;
        let d = detail(c, l, k);
        a = idwt(&a, &d, lo_r, hi_r, mode, Some(l[levels - k + 2]))?;
    }
    Ok(a)
}

/// Single-branch reconstruction (`wrcoef`): the other coefficients set to
/// zero, so that wrcoef('a', n) + sum of wrcoef('d', k <= n) gives x.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_branch(
    c: &[f64],
    l: &[usize],
    lo_r: &[f64],
    hi_r: &[f64],
    approx_branch: bool,
    n: usize,
    mode: Mode,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<f64>> {
    let levels = l.len() - 2;
    let mut x = if approx_branch {
        approx(c, l, lo_r, hi_r, n, mode, check)?
    } else {
        detail(c, l, n)
    };
    if n == 0 {
        return Ok(x);
    }
    // First step uses the branch filter, the rest the lowpass filter.
    let first = levels - n + 2;
    x = if approx_branch {
        idwt(&x, &[], lo_r, hi_r, mode, Some(l[first]))?
    } else {
        idwt(&[], &x, lo_r, hi_r, mode, Some(l[first]))?
    };
    for &len in &l[first + 1..] {
        check()?;
        x = idwt(&x, &[], lo_r, hi_r, mode, Some(len))?;
    }
    Ok(x)
}

/// Maximum useful level: fix(log2(n / (lf - 1))), 0 when below one.
pub(super) fn max_level(n: usize, lf: usize) -> usize {
    if lf < 2 || n < lf - 1 {
        return 0;
    }
    (n as f64 / (lf - 1) as f64).log2().floor().max(0.) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_modes_on_a_short_signal() {
        let x = [1., 2., 4.];
        assert_eq!(extend(&x, 2, Mode::Sym), [2., 1., 1., 2., 4., 4., 2.]);
        assert_eq!(extend(&x, 2, Mode::Symw), [4., 2., 1., 2., 4., 2., 1.]);
        assert_eq!(extend(&x, 2, Mode::Asym), [-2., -1., 1., 2., 4., -4., -2.]);
        assert_eq!(extend(&x, 2, Mode::Asymw), [-2., 0., 1., 2., 4., 6., 7.]);
        assert_eq!(extend(&x, 2, Mode::Sp1), [-1., 0., 1., 2., 4., 6., 8.]);
        assert_eq!(extend(&x, 2, Mode::Ppd), [2., 4., 1., 2., 4., 1., 2.]);
        assert_eq!(extend(&x, 1, Mode::Zpd), [0., 1., 2., 4., 0.]);
    }

    #[test]
    fn max_level_follows_the_filter_length() {
        assert_eq!(max_level(1024, 8), 7);
        assert_eq!(max_level(5, 20), 0);
        assert_eq!(max_level(64, 2), 6);
    }
}
