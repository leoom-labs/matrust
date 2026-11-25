//! Filter-design extras: `firls`, `kaiserord`, `firpmord`, `phasedelay`, `iirnotch`, `iirpeak`.
//! firls: least-squares linear-phase FIR (Parks & McClellan 1972 / Rabiner-Gold normal equations).
//! Orders: Kaiser (1974) and Herrmann-Lam-Rabiner (1973) published estimates.
//! Notch/peak: Orfanidis, *Introduction to Signal Processing* (1996), sec. 8.3.
use super::{
    Matrix, check_cancel, group_delay, phase_response, response, sos_group_delay, sos_response,
};
use crate::Result;
use std::{f64::consts::PI, sync::atomic::AtomicBool};

const MAX_FIRLS_ORDER: usize = 2048;
const MAX_DIFF_ORDER: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FirlsKind {
    Symmetric,
    Hilbert,
    Differentiator,
}

/// Gaussian elimination with partial pivoting on a row-major k-by-k system.
fn solve(mut q: Vec<f64>, mut b: Vec<f64>, cancel: &AtomicBool) -> Result<Vec<f64>> {
    let k = b.len();
    let scale = q.iter().fold(0., |m: f64, v| m.max(v.abs()));
    for col in 0..k {
        check_cancel(cancel)?;
        let p = (col..k)
            .max_by(|&x, &y| q[x * k + col].abs().total_cmp(&q[y * k + col].abs()))
            .unwrap();
        if q[p * k + col].abs() <= 1e-14 * scale {
            return Err(
                "firls: the least-squares problem is singular; widen the bands or lower the order"
                    .into(),
            );
        }
        if p != col {
            for j in 0..k {
                q.swap(p * k + j, col * k + j);
            }
            b.swap(p, col);
        }
        let d = q[col * k + col];
        for r in col + 1..k {
            let m = q[r * k + col] / d;
            if m != 0. {
                for j in col..k {
                    q[r * k + j] -= m * q[col * k + j];
                }
                b[r] -= m * b[col];
            }
        }
    }
    for r in (0..k).rev() {
        let s: f64 = (r + 1..k).map(|j| q[r * k + j] * b[j]).sum();
        b[r] = (b[r] - s) / q[r * k + r];
    }
    Ok(b)
}

fn gauss_legendre(n: usize) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let mut x = (PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
            let mut dp = 1.;
            for _ in 0..100 {
                let (mut p0, mut p1) = (1., x);
                for k in 2..=n {
                    let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                    p0 = p1;
                    p1 = p2;
                }
                dp = n as f64 * (x * p1 - p0) / (x * x - 1.);
                let dx = p1 / dp;
                x -= dx;
                if dx.abs() < 1e-16 {
                    break;
                }
            }
            (x, 2. / ((1. - x * x) * dp * dp))
        })
        .collect()
}

/// `firls(n, f, a, w, kind)`: n is the order (n+1 taps), f normalized (Nyquist = 1) in band pairs.
pub fn firls(
    n: usize,
    f: &[f64],
    a: &[f64],
    w: Option<&[f64]>,
    kind: FirlsKind,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    let limit = if kind == FirlsKind::Differentiator {
        MAX_DIFF_ORDER
    } else {
        MAX_FIRLS_ORDER
    };
    if n < 1 || n > limit {
        return Err(format!(
            "firls: the filter order must be an integer in 1..{limit}"
        ));
    }
    if f.len() != a.len() || f.len() < 2 || !f.len().is_multiple_of(2) {
        return Err("firls: F and A must be vectors of the same even length".into());
    }
    if f.iter().chain(a).any(|v| !v.is_finite()) {
        return Err("firls: F and A must be finite".into());
    }
    if f.iter().any(|v| !(0. ..=1.).contains(v)) {
        return Err("firls: frequencies in F must be in the range [0, 1]".into());
    }
    if f.windows(2).any(|p| p[1] < p[0]) {
        return Err("firls: frequencies in F must be nondecreasing".into());
    }
    if f.chunks(2).any(|p| p[1] <= p[0]) {
        return Err("firls: each band edge pair in F must be strictly increasing".into());
    }
    let bands = f.len() / 2;
    let weights = match w {
        Some(w) => {
            if w.len() != bands {
                return Err("firls: W must have one weight per band".into());
            }
            if w.iter().any(|v| !v.is_finite() || *v <= 0.) {
                return Err("firls: weights in W must be positive".into());
            }
            w.to_vec()
        }
        None => vec![1.; bands],
    };
    let antisym = kind != FirlsKind::Symmetric;
    let odd = n % 2 == 1;
    // Basis frequencies m_i of A(w) = sum c_i cos(m_i w) (or sin for antisymmetric).
    let m: Vec<f64> = match (antisym, odd) {
        (false, false) => (0..=n / 2).map(|i| i as f64).collect(),
        (false, true) | (true, true) => (0..=(n - 1) / 2).map(|i| i as f64 + 0.5).collect(),
        (true, false) => (1..=n / 2).map(|i| i as f64).collect(),
    };
    let k = m.len();
    let mut q = vec![0.; k * k];
    let mut b = vec![0.; k];
    for (band, &wt) in weights.iter().enumerate() {
        let (w1, w2) = (PI * f[2 * band], PI * f[2 * band + 1]);
        let (a1, a2) = (a[2 * band], a[2 * band + 1]);
        let slope = (a2 - a1) / (w2 - w1);
        let icept = a1 - slope * w1;
        let weighted = kind == FirlsKind::Differentiator && (a1 != 0. || a2 != 0.);
        check_cancel(cancel)?;
        if weighted {
            if w1 == 0. && icept != 0. {
                return Err(
                    "firls: a differentiator band starting at 0 must have amplitude 0 there".into(),
                );
            }
            // Integrand sin(mi w) sin(mj w) / w^2 (smooth at 0), composite Gauss-Legendre.
            let top = m.last().copied().unwrap_or(1.).max(1.);
            let panels = ((w2 - w1) * top / PI).ceil() as usize + 1;
            let gl = gauss_legendre(24);
            let h = (w2 - w1) / panels as f64;
            let mut pts = Vec::with_capacity(panels * gl.len());
            for p in 0..panels {
                let mid = w1 + (p as f64 + 0.5) * h;
                for &(x, wx) in &gl {
                    pts.push((mid + 0.5 * h * x, 0.5 * h * wx));
                }
            }
            let mut s = vec![0.; k * pts.len()];
            for (i, &mi) in m.iter().enumerate() {
                for (p, &(x, _)) in pts.iter().enumerate() {
                    s[i * pts.len() + p] = if x == 0. { mi } else { (mi * x).sin() / x };
                }
            }
            for i in 0..k {
                check_cancel(cancel)?;
                for j in i..k {
                    let v: f64 = (0..pts.len())
                        .map(|p| s[i * pts.len() + p] * s[j * pts.len() + p] * pts[p].1)
                        .sum();
                    q[i * k + j] += wt * v;
                }
                let v: f64 = (0..pts.len())
                    .map(|p| {
                        let x = pts[p].0;
                        let d = if x == 0. {
                            slope
                        } else {
                            (slope * x + icept) / x
                        };
                        s[i * pts.len() + p] * d * pts[p].1
                    })
                    .sum();
                b[i] += wt * v;
            }
            continue;
        }
        let ic = |kk: f64| -> f64 {
            if kk == 0. {
                w2 - w1
            } else {
                ((kk * w2).sin() - (kk * w1).sin()) / kk
            }
        };
        for i in 0..k {
            for j in i..k {
                let (d, s) = (m[i] - m[j], m[i] + m[j]);
                q[i * k + j] += wt
                    * 0.5
                    * if antisym {
                        ic(d) - ic(s)
                    } else {
                        ic(d) + ic(s)
                    };
            }
            let kk = m[i];
            let t = |x: f64| -> f64 {
                if antisym {
                    slope * ((kk * x).sin() / (kk * kk) - x * (kk * x).cos() / kk)
                        - icept * (kk * x).cos() / kk
                } else if kk == 0. {
                    0.5 * slope * x * x + icept * x
                } else {
                    slope * (x * (kk * x).sin() / kk + (kk * x).cos() / (kk * kk))
                        + icept * (kk * x).sin() / kk
                }
            };
            b[i] += wt * (t(w2) - t(w1));
        }
    }
    for i in 0..k {
        for j in 0..i {
            q[i * k + j] = q[j * k + i];
        }
    }
    let c = solve(q, b, cancel)?;
    let mut h = vec![0.; n + 1];
    match (antisym, odd) {
        (false, false) => {
            let mid = n / 2;
            h[mid] = c[0];
            for i in 1..k {
                h[mid + i] = c[i] / 2.;
                h[mid - i] = c[i] / 2.;
            }
        }
        (false, true) => {
            for i in 0..k {
                h[(n - 1) / 2 - i] = c[i] / 2.;
                h[n.div_ceil(2) + i] = c[i] / 2.;
            }
        }
        (true, false) => {
            let mid = n / 2;
            for i in 0..k {
                h[mid + 1 + i] = c[i] / 2.;
                h[mid - 1 - i] = -c[i] / 2.;
            }
        }
        (true, true) => {
            for i in 0..k {
                h[n.div_ceil(2) + i] = c[i] / 2.;
                h[(n - 1) / 2 - i] = -c[i] / 2.;
            }
        }
    }
    if h.iter().any(|v| !v.is_finite()) {
        return Err("firls: the design produced nonfinite coefficients".into());
    }
    Ok(h)
}

/// Shared checks for kaiserord/firpmord: returns the edges in radians-per-sample fractions of Nyquist.
fn order_spec(f: &[f64], a: &[f64], dev: &[f64], fs: f64, name: &str) -> Result<Vec<f64>> {
    if !fs.is_finite() || fs <= 0. {
        return Err(format!("{name}: the sampling frequency must be positive"));
    }
    if a.len() < 2 || f.len() != 2 * a.len() - 2 {
        return Err(format!(
            "{name}: F must have 2*length(A)-2 elements and A at least 2 elements"
        ));
    }
    if dev.len() != a.len() {
        return Err(format!("{name}: DEV and A must have the same length"));
    }
    if f.iter().chain(a).chain(dev).any(|v| !v.is_finite()) {
        return Err(format!("{name}: inputs must be finite"));
    }
    if dev.iter().any(|v| *v <= 0.) {
        return Err(format!("{name}: deviations in DEV must be positive"));
    }
    let nyq = fs / 2.;
    if f.iter().any(|v| *v <= 0. || *v >= nyq) {
        return Err(format!(
            "{name}: band edges in F must be between 0 and fs/2"
        ));
    }
    if f.windows(2).any(|p| p[1] < p[0]) {
        return Err(format!("{name}: band edges in F must be nondecreasing"));
    }
    if f.chunks(2).any(|p| p[1] == p[0]) {
        return Err(format!("{name}: transition bands must have nonzero width"));
    }
    Ok(f.iter().map(|v| v / nyq).collect())
}

pub struct KaiserOrder {
    pub n: usize,
    pub wn: Vec<f64>,
    pub beta: f64,
    pub ftype: &'static str,
}
/// Kaiser window order estimate (Kaiser 1974): A = -20 log10(min dev), n = ceil((A-7.95)/(2.285 dw)).
pub fn kaiserord(f: &[f64], a: &[f64], dev: &[f64], fs: f64) -> Result<KaiserOrder> {
    let norm = order_spec(f, a, dev, fs, "kaiserord")?;
    let d = dev.iter().cloned().fold(f64::INFINITY, f64::min);
    let att = -20. * d.log10();
    let dw = norm
        .chunks(2)
        .map(|p| PI * (p[1] - p[0]))
        .fold(f64::INFINITY, f64::min);
    let n = ((att - 7.95) / (2.285 * dw)).ceil().max(1.);
    if n > 1e6 {
        return Err("kaiserord: the estimated order is too large".into());
    }
    let beta = if att > 50. {
        0.1102 * (att - 8.7)
    } else if att >= 21. {
        0.5842 * (att - 21.).powf(0.4) + 0.07886 * (att - 21.)
    } else {
        0.
    };
    let wn = norm.chunks(2).map(|p| (p[0] + p[1]) / 2.).collect();
    let on = |i: usize| a[i] != 0.;
    let ftype = match a.len() {
        2 if on(0) && !on(1) => "low",
        2 if !on(0) && on(1) => "high",
        3 if !on(0) && on(1) && !on(2) => "bandpass",
        3 if on(0) && !on(1) && on(2) => "stop",
        _ if on(0) => "DC-1",
        _ => "DC-0",
    };
    // A type II FIR has a zero at Nyquist: a passband there needs an even order.
    let n = if a.last().is_some_and(|v| *v != 0.) && n % 2. == 1. {
        n + 1.
    } else {
        n
    };
    Ok(KaiserOrder {
        n: n as usize,
        wn,
        beta,
        ftype,
    })
}

pub struct FirpmOrder {
    pub n: usize,
    pub fo: Vec<f64>,
    pub ao: Vec<f64>,
    pub w: Vec<f64>,
}
/// Herrmann-Lam-Rabiner estimate for one passband/stopband pair; returns the filter length.
fn hlr_length(d1: f64, d2: f64, df: f64) -> f64 {
    let (l1, l2) = (d1.log10(), d2.log10());
    let dinf = (5.309e-3 * l1 * l1 + 7.114e-2 * l1 - 4.761e-1) * l2
        + (-2.66e-3 * l1 * l1 - 5.941e-1 * l1 - 4.278e-1);
    let fk = 11.01217 + 0.51244 * (l1 - l2);
    dinf / df - fk * df + 1.
}
/// Order estimate for `firpm`; multiband designs take the maximum over adjacent band pairs.
pub fn firpmord(f: &[f64], a: &[f64], dev: &[f64], fs: f64) -> Result<FirpmOrder> {
    let norm = order_spec(f, a, dev, fs, "firpmord")?;
    let mut longest = 0_f64;
    for k in 0..a.len() - 1 {
        // Herrmann's transition width is in cycles/sample (fs = 1); `norm` is Nyquist = 1.
        let df = (norm[2 * k + 1] - norm[2 * k]) / 2.;
        // The band with the larger amplitude is the passband of the pair.
        let (d1, d2) = if a[k].abs() >= a[k + 1].abs() {
            (dev[k], dev[k + 1])
        } else {
            (dev[k + 1], dev[k])
        };
        longest = longest.max(hlr_length(d1, d2, df));
    }
    if !longest.is_finite() || longest > 1e6 {
        return Err("firpmord: the estimated order is too large".into());
    }
    let n = (longest.ceil() - 1.).max(1.) as usize;
    let mut fo = vec![0.];
    fo.extend(&norm);
    fo.push(1.);
    let ao = a.iter().flat_map(|v| [*v, *v]).collect();
    let dm = dev.iter().cloned().fold(0., f64::max);
    Ok(FirpmOrder {
        n,
        fo,
        ao,
        w: dev.iter().map(|d| dm / d).collect(),
    })
}

fn delay_from(h: &[[f64; 2]], f: &[f64], group0: f64) -> Vec<f64> {
    let phase = phase_response(h);
    phase
        .iter()
        .zip(f)
        .enumerate()
        .map(|(i, (p, fr))| if i == 0 { group0 } else { -p / (PI * fr) })
        .collect()
}
fn grid(n: usize) -> Vec<f64> {
    (0..n).map(|i| i as f64 / n as f64).collect()
}
fn check_count(n: usize) -> Result<()> {
    if n == 0 || n > 65536 {
        return Err("phasedelay: the point count must be 1..65536".into());
    }
    Ok(())
}
/// Phase delay in samples at n points over [0, Nyquist); the value at 0 is the group delay there.
pub fn phasedelay(b: &[f64], a: &[f64], n: usize, cancel: &AtomicBool) -> Result<Vec<f64>> {
    check_count(n)?;
    let f = grid(n);
    let h = response(b, a, &f, cancel)?;
    let g0 = group_delay(b, a, &[0.], cancel)?[0];
    Ok(delay_from(&h, &f, g0))
}
pub fn phasedelay_sos(sos: &Matrix, n: usize, cancel: &AtomicBool) -> Result<Vec<f64>> {
    check_count(n)?;
    let f = grid(n);
    let h = sos_response(sos, &f, cancel)?;
    let g0 = sos_group_delay(sos, &[0.], cancel)?[0];
    Ok(delay_from(&h, &f, g0))
}

/// Second-order notch (`peak = false`) or peak (`peak = true`); w0, bw normalized to Nyquist,
/// `ab_db` the (negative) level in dB at the bandwidth.
pub fn notch_peak(w0: f64, bw: f64, ab_db: f64, peak: bool) -> Result<(Vec<f64>, Vec<f64>)> {
    let name = if peak { "iirpeak" } else { "iirnotch" };
    if !w0.is_finite() || w0 <= 0. || w0 >= 1. {
        return Err(format!("{name}: W0 must be between 0 and 1 (exclusive)"));
    }
    if !bw.is_finite() || bw <= 0. || bw >= 1. {
        return Err(format!("{name}: BW must be between 0 and 1 (exclusive)"));
    }
    if !ab_db.is_finite() || ab_db >= 0. {
        return Err(format!("{name}: Ab must be a negative level in dB"));
    }
    let gb = 10f64.powf(ab_db / 20.);
    let t = (PI * bw / 2.).tan();
    let beta = if peak {
        gb / (1. - gb * gb).sqrt() * t
    } else {
        (1. - gb * gb).sqrt() / gb * t
    };
    let gain = 1. / (1. + beta);
    let c = (PI * w0).cos();
    let b = if peak {
        vec![1. - gain, 0., -(1. - gain)]
    } else {
        vec![gain, -2. * gain * c, gain]
    };
    Ok((b, vec![1., -2. * gain * c, 2. * gain - 1.]))
}
