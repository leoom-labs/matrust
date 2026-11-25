//! Deterministic start points of the nonlinear library models, computed from
//! the data: log-linear and variable-projection searches (exp, power), the
//! integral-equation method for sums of two exponentials, peak peeling for
//! Gaussians, a least-squares frequency search for sine and Fourier series,
//! and Levy's linearization for rational models. Custom equations without a
//! StartPoint get a fixed pseudo-random start.
use super::library::Lib;
use super::solve::lstsq;
use nalgebra::DMatrix;

/// Points used by the start heuristics: at most this many, evenly spread.
const MAX_POINTS: usize = 2000;
/// Frequencies tried by the sine/Fourier search.
const MAX_FREQUENCIES: usize = 4096;

/// Data sorted by x, thinned to at most MAX_POINTS, without non-finite pairs.
fn prepare(x: &[f64], y: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut idx: Vec<usize> = (0..x.len())
        .filter(|&i| x[i].is_finite() && y[i].is_finite())
        .collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    if idx.len() > MAX_POINTS {
        let n = idx.len();
        idx = (0..MAX_POINTS)
            .map(|k| idx[k * (n - 1) / (MAX_POINTS - 1)])
            .collect();
    }
    (
        idx.iter().map(|&i| x[i]).collect(),
        idx.iter().map(|&i| y[i]).collect(),
    )
}

/// Least-squares coefficients and SSE of y on the given columns.
fn ls(cols: &[Vec<f64>], y: &[f64]) -> Option<(Vec<f64>, f64)> {
    let n = y.len();
    if cols.iter().any(|c| c.iter().any(|v| !v.is_finite())) || n == 0 {
        return None;
    }
    let a = DMatrix::from_fn(n, cols.len(), |i, k| cols[k][i]);
    let (c, _) = lstsq(&a, y);
    let sse = (0..n)
        .map(|i| {
            let f: f64 = (0..cols.len()).map(|k| c[k] * cols[k][i]).sum();
            (y[i] - f).powi(2)
        })
        .sum::<f64>();
    (sse.is_finite() && c.iter().all(|v| v.is_finite())).then_some((c, sse))
}

/// Minimizer of f on [lo, hi] by golden-section search.
fn golden(f: &mut dyn FnMut(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    let g = (5f64.sqrt() - 1.) / 2.;
    let mut c = hi - g * (hi - lo);
    let mut d = lo + g * (hi - lo);
    let (mut fc, mut fd) = (f(c), f(d));
    for _ in 0..60 {
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - g * (hi - lo);
            fc = f(c);
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + g * (hi - lo);
            fd = f(d);
        }
    }
    if fc < fd { c } else { d }
}

fn span(x: &[f64]) -> f64 {
    let s = x.last().copied().unwrap_or(0.) - x.first().copied().unwrap_or(0.);
    if s > 0. && s.is_finite() { s } else { 1. }
}
fn mid(x: &[f64]) -> f64 {
    (x.first().copied().unwrap_or(0.) + x.last().copied().unwrap_or(0.)) / 2.
}

/// One nonlinear parameter: grid search plus golden refinement of the
/// variable-projection SSE. Returns the parameter and its linear coefficients.
fn project(
    grid: &[f64],
    step: f64,
    y: &[f64],
    basis: &dyn Fn(f64) -> Vec<Vec<f64>>,
) -> Option<(f64, Vec<f64>)> {
    let sse = |t: f64| ls(&basis(t), y).map_or(f64::INFINITY, |(_, s)| s);
    let mut best = None;
    for &t in grid {
        let s = sse(t);
        if s.is_finite() && best.is_none_or(|(_, b)| s < b) {
            best = Some((t, s));
        }
    }
    let (t, _) = best?;
    let mut f = |t: f64| sse(t);
    let refined = golden(&mut f, t - step, t + step);
    let t = if sse(refined) <= sse(t) { refined } else { t };
    ls(&basis(t), y).map(|(c, _)| (t, c))
}

pub(super) fn start_point(lib: Lib, x: &[f64], y: &[f64]) -> Vec<f64> {
    let p = lib.coeffs().len();
    let (x, y) = prepare(x, y);
    if x.is_empty() {
        return vec![1.; p];
    }
    let mut s = match lib {
        Lib::Exp(1) => exp1(&x, &y),
        Lib::Exp(_) => exp2(&x, &y),
        Lib::Fourier(n) => fourier(&x, &y, n),
        Lib::Gauss(n) => gauss(&x, &y, n),
        Lib::Power(n) => power(&x, &y, n),
        Lib::Rat(m, n) => rat(&x, &y, m, n),
        Lib::Sin(n) => sines(&x, &y, n),
        Lib::Weibull => weibull(&x, &y),
        Lib::Poly(_) | Lib::PolySurf(..) => vec![0.; p],
    };
    s.resize(p, 1.);
    for v in &mut s {
        if !v.is_finite() {
            *v = 1.;
        }
    }
    s
}

fn exp1(x: &[f64], y: &[f64]) -> Vec<f64> {
    let (l, m) = (span(x), mid(x));
    let mut grid: Vec<f64> = (-40..=40).map(|k| k as f64 * 0.25 / l).collect();
    let sign = y[0].signum();
    if y.iter().all(|v| v.signum() == sign && *v != 0.) {
        let logs: Vec<f64> = y.iter().map(|v| v.abs().ln()).collect();
        if let Some((c, _)) = ls(&[vec![1.; x.len()], x.to_vec()], &logs) {
            grid.push(c[1]);
        }
    }
    let basis = |b: f64| vec![x.iter().map(|t| (b * (t - m)).exp()).collect()];
    match project(&grid, 0.25 / l, y, &basis) {
        Some((b, c)) => vec![c[0] * (-b * m).exp(), b],
        None => vec![y.iter().sum::<f64>() / y.len() as f64, 0.],
    }
}

fn sse_of(lib: Lib, c: &[f64], x: &[f64], y: &[f64]) -> f64 {
    let s: f64 = x
        .iter()
        .zip(y)
        .map(|(&t, &v)| (lib.value(c, t, 0.) - v).powi(2))
        .sum();
    if s.is_finite() { s } else { f64::INFINITY }
}

/// Sum of two exponentials: y'' = (b+d) y' - b d y, integrated twice, is linear
/// in the running integrals S and SS of y; their regression gives b and d.
fn exp2(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let m = mid(x);
    let mut candidates = Vec::new();
    if n >= 4 {
        let mut s = vec![0.; n];
        let mut ss = vec![0.; n];
        for k in 1..n {
            let dx = x[k] - x[k - 1];
            s[k] = s[k - 1] + 0.5 * (y[k] + y[k - 1]) * dx;
            ss[k] = ss[k - 1] + 0.5 * (s[k] + s[k - 1]) * dx;
        }
        if let Some((c, _)) = ls(&[ss, s, x.to_vec(), vec![1.; n]], y) {
            let (q, p) = (c[0], c[1]);
            let disc = p * p + 4. * q;
            if disc > 0. {
                let root = disc.sqrt();
                let (b, d) = ((p + root) / 2., (p - root) / 2.);
                let cols = vec![
                    x.iter().map(|t| (b * (t - m)).exp()).collect(),
                    x.iter().map(|t| (d * (t - m)).exp()).collect(),
                ];
                if let Some((a, _)) = ls(&cols, y) {
                    candidates.push(vec![a[0] * (-b * m).exp(), b, a[1] * (-d * m).exp(), d]);
                }
            }
        }
    }
    // Peel one exponential, then fit the remainder.
    let first = exp1(x, y);
    let rest: Vec<f64> = x
        .iter()
        .zip(y)
        .map(|(&t, &v)| v - first[0] * (first[1] * t).exp())
        .collect();
    let mut second = exp1(x, &rest);
    if (second[1] - first[1]).abs() < 1e-3 / span(x) {
        second[1] = first[1] / 2.;
    }
    candidates.push(vec![first[0], first[1], second[0], second[1]]);
    candidates
        .into_iter()
        .min_by(|a, b| sse_of(Lib::Exp(2), a, x, y).total_cmp(&sse_of(Lib::Exp(2), b, x, y)))
        .unwrap()
}

fn gauss(x: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let len = x.len();
    let l = span(x);
    let spacing = if len > 1 { l / (len - 1) as f64 } else { 1. };
    let mut r = y.to_vec();
    let mut out = Vec::with_capacity(3 * n);
    for _ in 0..n {
        let i = (0..len)
            .max_by(|&a, &b| r[a].abs().total_cmp(&r[b].abs()))
            .unwrap();
        let (a, b) = (r[i], x[i]);
        let half = a.abs() / 2.;
        let above = |k: usize| r[k] * a.signum() > half;
        let crossing = |k0: usize, k1: usize| {
            // Linear interpolation of the half-height crossing between k0 (above) and k1.
            let (v0, v1) = (r[k0] * a.signum(), r[k1] * a.signum());
            if v0 == v1 {
                x[k1]
            } else {
                x[k0] + (half - v0) / (v1 - v0) * (x[k1] - x[k0])
            }
        };
        let mut left = None;
        let mut k = i;
        while k > 0 {
            if !above(k - 1) {
                left = Some(crossing(k, k - 1));
                break;
            }
            k -= 1;
        }
        let mut right = None;
        let mut k = i;
        while k + 1 < len {
            if !above(k + 1) {
                right = Some(crossing(k, k + 1));
                break;
            }
            k += 1;
        }
        let hw = match (left, right) {
            (Some(lo), Some(hi)) => (hi - lo) / 2.,
            (Some(lo), None) => b - lo,
            (None, Some(hi)) => hi - b,
            (None, None) => l / (2 * n) as f64,
        };
        let c = (hw / 2f64.ln().sqrt()).max(spacing);
        out.extend([a, b, c]);
        for k in 0..len {
            let u = (x[k] - b) / c;
            r[k] -= a * (-u * u).exp();
        }
    }
    let cols: Vec<Vec<f64>> = (0..n)
        .map(|k| {
            x.iter()
                .map(|t| {
                    let u = (t - out[3 * k + 1]) / out[3 * k + 2];
                    (-u * u).exp()
                })
                .collect()
        })
        .collect();
    if let Some((a, _)) = ls(&cols, y) {
        for k in 0..n {
            out[3 * k] = a[k];
        }
    }
    out
}

/// Angular frequency whose sine/cosine pair (plus a constant if asked) best
/// fits r by least squares, over a 4x oversampled grid up to the Nyquist
/// frequency of the median spacing, refined by golden section.
fn best_frequency(x: &[f64], r: &[f64], constant: bool) -> f64 {
    let n = x.len();
    let l = span(x);
    let mut gaps: Vec<f64> = x
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.)
        .collect();
    gaps.sort_by(f64::total_cmp);
    let h = gaps
        .get(gaps.len() / 2)
        .copied()
        .unwrap_or(l / n.max(2) as f64);
    let delta = std::f64::consts::PI / (2. * l);
    let top = (std::f64::consts::PI / h / delta)
        .ceil()
        .clamp(1., MAX_FREQUENCIES as f64) as usize;
    let sse = |w: f64| {
        let mut cols = vec![
            x.iter().map(|t| (w * t).sin()).collect::<Vec<f64>>(),
            x.iter().map(|t| (w * t).cos()).collect(),
        ];
        if constant {
            cols.push(vec![1.; n]);
        }
        ls(&cols, r).map_or(f64::INFINITY, |(_, s)| s)
    };
    let mut best = (delta, f64::INFINITY);
    for k in 1..=top {
        let w = k as f64 * delta;
        let s = sse(w);
        if s < best.1 {
            best = (w, s);
        }
    }
    let mut f = |w: f64| sse(w);
    let w = golden(&mut f, (best.0 - delta).max(delta / 4.), best.0 + delta);
    if sse(w) <= best.1 { w } else { best.0 }
}

fn sines(x: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let mut r = y.to_vec();
    let mut freqs = Vec::with_capacity(n);
    for _ in 0..n {
        let w = best_frequency(x, &r, false);
        let cols = vec![
            x.iter().map(|t| (w * t).sin()).collect::<Vec<f64>>(),
            x.iter().map(|t| (w * t).cos()).collect(),
        ];
        if let Some((c, _)) = ls(&cols, &r) {
            for (k, t) in x.iter().enumerate() {
                r[k] -= c[0] * (w * t).sin() + c[1] * (w * t).cos();
            }
        }
        freqs.push(w);
    }
    let cols: Vec<Vec<f64>> = freqs
        .iter()
        .flat_map(|&w| {
            [
                x.iter().map(|t| (w * t).sin()).collect::<Vec<f64>>(),
                x.iter().map(|t| (w * t).cos()).collect(),
            ]
        })
        .collect();
    let c = ls(&cols, y).map_or(vec![1.; 2 * n], |(c, _)| c);
    let mut out = Vec::with_capacity(3 * n);
    for (k, &w) in freqs.iter().enumerate() {
        // a sin(w x + c) = a cos c sin(w x) + a sin c cos(w x)
        let (s, co) = (c[2 * k], c[2 * k + 1]);
        out.extend([s.hypot(co), w, co.atan2(s)]);
    }
    out
}

fn fourier_basis(x: &[f64], w: f64, n: usize) -> Vec<Vec<f64>> {
    let mut cols = vec![vec![1.; x.len()]];
    for k in 1..=n {
        let kw = k as f64 * w;
        cols.push(x.iter().map(|t| (kw * t).cos()).collect());
        cols.push(x.iter().map(|t| (kw * t).sin()).collect());
    }
    cols
}

fn fourier(x: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let mean = y.iter().sum::<f64>() / y.len() as f64;
    let centred: Vec<f64> = y.iter().map(|v| v - mean).collect();
    let peak = best_frequency(x, &centred, false);
    let delta = std::f64::consts::PI / (2. * span(x));
    // The strongest line may be a harmonic of the fundamental.
    let grid: Vec<f64> = (1..=n).map(|k| peak / k as f64).collect();
    let basis = |w: f64| fourier_basis(x, w, n);
    match project(&grid, delta / n as f64, y, &basis) {
        Some((w, mut c)) => {
            c.push(w);
            c
        }
        None => {
            let mut c = vec![0.; 2 * n + 1];
            c[0] = mean;
            c.push(peak);
            c
        }
    }
}

fn power(x: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let keep: Vec<usize> = (0..x.len()).filter(|&i| x[i] > 0.).collect();
    if keep.is_empty() {
        return vec![1.; n + 1];
    }
    let xp: Vec<f64> = keep.iter().map(|&i| x[i]).collect();
    let yp: Vec<f64> = keep.iter().map(|&i| y[i]).collect();
    let mut grid: Vec<f64> = (-40..=40).map(|k| k as f64 * 0.1).collect();
    let sign = yp[0].signum();
    if n == 1 && yp.iter().all(|v| v.signum() == sign && *v != 0.) {
        let lx: Vec<f64> = xp.iter().map(|v| v.ln()).collect();
        let ly: Vec<f64> = yp.iter().map(|v| v.abs().ln()).collect();
        if let Some((c, _)) = ls(&[vec![1.; lx.len()], lx], &ly) {
            grid.push(c[1]);
        }
    }
    let basis = |b: f64| {
        let mut cols = vec![xp.iter().map(|t| t.powf(b)).collect::<Vec<f64>>()];
        if n == 2 {
            cols.push(vec![1.; xp.len()]);
        }
        cols
    };
    match project(&grid, 0.1, &yp, &basis) {
        Some((b, c)) => {
            let mut s = vec![c[0], b];
            if n == 2 {
                s.push(c[1]);
            }
            s
        }
        None => vec![1.; n + 1],
    }
}

/// Levy: y*Q(x) = P(x) is linear in the coefficients of P and of the monic Q.
fn rat(x: &[f64], y: &[f64], m: usize, n: usize) -> Vec<f64> {
    let mut cols: Vec<Vec<f64>> = (0..=m)
        .map(|k| x.iter().map(|t| t.powi((m - k) as i32)).collect())
        .collect();
    for k in 1..=n {
        cols.push(
            x.iter()
                .zip(y)
                .map(|(t, v)| -v * t.powi((n - k) as i32))
                .collect(),
        );
    }
    let rhs: Vec<f64> = x.iter().zip(y).map(|(t, v)| v * t.powi(n as i32)).collect();
    match ls(&cols, &rhs) {
        Some((c, _)) => c,
        None => {
            let mut c = vec![0.; m + 1 + n];
            c[m] = y.iter().sum::<f64>() / y.len() as f64;
            c[m + n] = 1.;
            c
        }
    }
}

/// ln y = ln(a b) + (b-1) ln x - a x^b is linear in (ln(a b), a) for fixed b.
fn weibull(x: &[f64], y: &[f64]) -> Vec<f64> {
    let keep: Vec<usize> = (0..x.len()).filter(|&i| x[i] > 0. && y[i] > 0.).collect();
    if keep.len() < 2 {
        return vec![1., 1.];
    }
    let lx: Vec<f64> = keep.iter().map(|&i| x[i].ln()).collect();
    let ly: Vec<f64> = keep.iter().map(|&i| y[i].ln()).collect();
    let xs: Vec<f64> = keep.iter().map(|&i| x[i]).collect();
    let mut best = (vec![1., 1.], sse_of(Lib::Weibull, &[1., 1.], x, y));
    for k in 1..=60 {
        let b = k as f64 * 0.1;
        let rhs: Vec<f64> = ly.iter().zip(&lx).map(|(v, l)| v - (b - 1.) * l).collect();
        let cols = vec![vec![1.; xs.len()], xs.iter().map(|t| -t.powf(b)).collect()];
        if let Some((c, _)) = ls(&cols, &rhs)
            && c[1] > 0.
        {
            let s = sse_of(Lib::Weibull, &[c[1], b], x, y);
            if s < best.1 {
                best = (vec![c[1], b], s);
            }
        }
    }
    best.0
}

/// MATLAB picks a random start for custom equations; this is a fixed
/// sequence (splitmix64) mapped into the bounds, so fits are reproducible.
pub(super) fn seeded_start(lower: &[f64], upper: &[f64]) -> Vec<f64> {
    let mut state: u64 = 0x5EED_CF17_2026_0001;
    lower
        .iter()
        .zip(upper)
        .map(|(&lo, &hi)| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            let u = ((z >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
            match (lo.is_finite(), hi.is_finite()) {
                (true, true) => lo + u * (hi - lo),
                (true, false) => lo + u,
                (false, true) => hi - u,
                (false, false) => u,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_models_are_found_from_the_data() {
        let x: Vec<f64> = (0..60).map(|i| i as f64 * 0.1).collect();
        let y: Vec<f64> = x.iter().map(|t| 3. * (-0.8 * t).exp()).collect();
        let s = start_point(Lib::Exp(1), &x, &y);
        assert!(
            (s[0] - 3.).abs() < 1e-6 && (s[1] + 0.8).abs() < 1e-6,
            "{s:?}"
        );
        let y: Vec<f64> = x.iter().map(|t| 2. * (1.3 * t + 0.4).sin()).collect();
        let s = start_point(Lib::Sin(1), &x, &y);
        assert!((s[1] - 1.3).abs() < 1e-3, "{s:?}");
        let y: Vec<f64> = x
            .iter()
            .map(|t| 2. * (-((t - 2.) / 0.7).powi(2)).exp())
            .collect();
        let s = start_point(Lib::Gauss(1), &x, &y);
        assert!(
            (s[1] - 2.).abs() < 0.11 && (s[2] - 0.7).abs() < 0.1,
            "{s:?}"
        );
        let y: Vec<f64> = x
            .iter()
            .map(|t| 2. * (0.3 * t).exp() + 5. * (-1.5 * t).exp())
            .collect();
        let s = start_point(Lib::Exp(2), &x, &y);
        assert!(sse_of(Lib::Exp(2), &s, &x, &y) < 1e-3, "{s:?}");
    }

    #[test]
    fn seeded_starts_are_fixed_and_inside_bounds() {
        let a = seeded_start(&[0., f64::NEG_INFINITY], &[2., f64::INFINITY]);
        assert_eq!(
            a,
            seeded_start(&[0., f64::NEG_INFINITY], &[2., f64::INFINITY])
        );
        assert!((0. ..=2.).contains(&a[0]) && (0. ..1.).contains(&a[1]));
    }
}
