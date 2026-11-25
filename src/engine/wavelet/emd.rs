//! Empirical mode decomposition (Huang et al. 1998). Each IMF is sifted:
//! the mean of the upper and lower envelopes (cubic interpolation of the
//! local maxima / minima) is subtracted until the Cauchy criterion
//! ||h_prev - h||^2 / ||h_prev||^2 drops below SiftRelativeTolerance or
//! SiftMaxIterations is reached. Envelopes use the 'spline' (not-a-knot,
//! de Boor) or 'pchip' (Fritsch-Carlson) interpolant through the extrema plus
//! the two extrema nearest each end mirrored about the end samples.
//! Decomposition stops at MaxNumIMF IMFs, when the residual has fewer than
//! MaxNumExtrema extrema (or no maximum / minimum) or when
//! 10 log10(||x||^2 / ||r||^2) exceeds MaxEnergyRatio.
use super::super::{Engine, Value};
use super::mra::{
    mra_bool, mra_column, mra_columns, mra_count, mra_options, mra_scalar, mra_signal, mra_struct,
    mra_text,
};
use crate::Result;

#[derive(Clone, Copy, PartialEq)]
enum Interp {
    Spline,
    Pchip,
}

struct Opts {
    sift_tol: f64,
    sift_max: usize,
    max_imf: usize,
    max_extrema: usize,
    max_energy_ratio: f64,
    interp: Interp,
}

pub(super) fn emd_builtin(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "emd";
    let Some(first) = args.first() else {
        return Err("emd: not enough input arguments".into());
    };
    let x = mra_signal(first, WHO, 1)?;
    let mut o = Opts {
        sift_tol: 0.2,
        sift_max: 100,
        max_imf: 10,
        max_extrema: 1,
        max_energy_ratio: 20.,
        interp: Interp::Spline,
    };
    const NAMES: [&str; 7] = [
        "SiftRelativeTolerance",
        "SiftMaxIterations",
        "MaxNumIMF",
        "MaxNumExtrema",
        "MaxEnergyRatio",
        "Interpolation",
        "Display",
    ];
    const BIG: usize = 1_000_000;
    for (i, v) in mra_options(&args[1..], &NAMES, WHO)? {
        match i {
            0 => {
                o.sift_tol = mra_scalar(v, WHO, "SiftRelativeTolerance")?;
                if o.sift_tol <= 0. {
                    return Err("emd: SiftRelativeTolerance must be positive".into());
                }
            }
            1 => o.sift_max = mra_count(v, WHO, "SiftMaxIterations", BIG)?,
            2 => o.max_imf = mra_count(v, WHO, "MaxNumIMF", BIG)?,
            3 => o.max_extrema = mra_count(v, WHO, "MaxNumExtrema", BIG)?,
            4 => o.max_energy_ratio = mra_scalar(v, WHO, "MaxEnergyRatio")?,
            5 => {
                let t = mra_text(v).ok_or("emd: Interpolation must be 'spline' or 'pchip'")?;
                o.interp = if t.eq_ignore_ascii_case("spline") {
                    Interp::Spline
                } else if t.eq_ignore_ascii_case("pchip") {
                    Interp::Pchip
                } else {
                    return Err(format!(
                        "emd: Interpolation must be 'spline' or 'pchip', not '{t}'"
                    ));
                };
            }
            _ => {
                if mra_bool(v, WHO, "Display")? {
                    return Err("emd: Display=1 is not supported yet".into());
                }
            }
        }
    }
    let n = x.len();
    let r = emd(e, &x, &o)?;
    let mut outs = vec![mra_columns(n, &r.imfs)?, mra_column(r.residual)?];
    if nargout > 2 {
        let col = |v: Vec<f64>| mra_column(v);
        let k = r.imfs.len();
        outs.push(mra_struct(
            e,
            vec![
                ("NumIMF", col((1..=k).map(|i| i as f64).collect())?),
                ("NumExtrema", col(r.extrema)?),
                ("NumZerocrossing", col(r.zero_crossings)?),
                ("NumSifting", col(r.siftings)?),
                ("MeanEnvelopeEnergy", col(r.envelope_energy)?),
                ("RelativeTolerance", col(r.tolerance)?),
            ],
        )?);
    }
    Ok(outs)
}

struct EmdResult {
    imfs: Vec<Vec<f64>>,
    residual: Vec<f64>,
    extrema: Vec<f64>,
    zero_crossings: Vec<f64>,
    siftings: Vec<f64>,
    envelope_energy: Vec<f64>,
    tolerance: Vec<f64>,
}

fn energy(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

fn emd(e: &Engine, x: &[f64], o: &Opts) -> Result<EmdResult> {
    let n = x.len();
    // Up to MaxNumIMF columns, before the sifting loop allocates any of them.
    super::mra::mra_admit(e, "emd", o.max_imf, n)?;
    let ex = energy(x);
    let mut out = EmdResult {
        imfs: Vec::new(),
        residual: x.to_vec(),
        extrema: Vec::new(),
        zero_crossings: Vec::new(),
        siftings: Vec::new(),
        envelope_energy: Vec::new(),
        tolerance: Vec::new(),
    };
    while out.imfs.len() < o.max_imf {
        let r = &out.residual;
        let (maxima, minima) = extrema(r);
        if maxima.len() + minima.len() < o.max_extrema || maxima.is_empty() || minima.is_empty() {
            break;
        }
        let er = energy(r);
        if er == 0. || 10. * (ex / er).log10() > o.max_energy_ratio {
            break;
        }
        let mut h = r.clone();
        let mut sifts = 0;
        let mut tol = f64::INFINITY;
        let mut mean_energy = 0.;
        while sifts < o.sift_max {
            e.check_cancel()?;
            let (mx, mn) = extrema(&h);
            if mx.is_empty() || mn.is_empty() {
                break;
            }
            let upper = envelope(&h, &mx, o.interp);
            let lower = envelope(&h, &mn, o.interp);
            let m: Vec<f64> = upper
                .iter()
                .zip(&lower)
                .map(|(u, l)| (u + l) / 2.)
                .collect();
            let eh = energy(&h);
            let em = energy(&m);
            for (hv, mv) in h.iter_mut().zip(&m) {
                *hv -= mv;
            }
            sifts += 1;
            mean_energy = em / n as f64;
            tol = if eh > 0. { em / eh } else { 0. };
            if tol < o.sift_tol {
                break;
            }
        }
        let (mx, mn) = extrema(&h);
        out.extrema.push((mx.len() + mn.len()) as f64);
        out.zero_crossings.push(zero_crossings(&h) as f64);
        out.siftings.push(sifts as f64);
        out.envelope_energy.push(mean_energy);
        out.tolerance.push(tol);
        for (rv, hv) in out.residual.iter_mut().zip(&h) {
            *rv -= hv;
        }
        out.imfs.push(h);
    }
    // IMFs + residual reproduce x: put the rounding left over into the residual.
    for t in 0..n {
        let s: f64 = out.imfs.iter().map(|c| c[t]).sum();
        out.residual[t] = x[t] - s;
    }
    Ok(out)
}

/// Interior local maxima and minima (a plateau counts once, at its start).
fn extrema(v: &[f64]) -> (Vec<usize>, Vec<usize>) {
    let mut mx = Vec::new();
    let mut mn = Vec::new();
    let n = v.len();
    let mut i = 1;
    while i + 1 < n {
        if v[i] != v[i - 1] {
            // Walk over a plateau starting here.
            let mut j = i;
            while j + 1 < n && v[j + 1] == v[i] {
                j += 1;
            }
            if j + 1 < n {
                if v[i] > v[i - 1] && v[i] > v[j + 1] {
                    mx.push(i);
                } else if v[i] < v[i - 1] && v[i] < v[j + 1] {
                    mn.push(i);
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    (mx, mn)
}

fn zero_crossings(v: &[f64]) -> usize {
    v.windows(2).filter(|w| (w[0] < 0.) != (w[1] < 0.)).count()
}

/// Envelope through v at `idx` plus up to two extrema mirrored about each end.
fn envelope(v: &[f64], idx: &[usize], interp: Interp) -> Vec<f64> {
    let n = v.len();
    let last = (n - 1) as f64;
    let mut xs = Vec::with_capacity(idx.len() + 4);
    let mut ys = Vec::with_capacity(idx.len() + 4);
    for &i in idx.iter().take(2).rev() {
        xs.push(-(i as f64));
        ys.push(v[i]);
    }
    for &i in idx {
        xs.push(i as f64);
        ys.push(v[i]);
    }
    for &i in idx.iter().rev().take(2) {
        xs.push(2. * last - i as f64);
        ys.push(v[i]);
    }
    let slopes = match interp {
        Interp::Spline => spline_slopes(&xs, &ys),
        Interp::Pchip => pchip_slopes(&xs, &ys),
    };
    hermite(&xs, &ys, &slopes, n)
}

/// Evaluates the cubic Hermite interpolant at 0, 1, ..., n-1.
fn hermite(xs: &[f64], ys: &[f64], d: &[f64], n: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(n);
    let mut k = 0;
    for t in 0..n {
        let t = t as f64;
        while k + 2 < xs.len() && t > xs[k + 1] {
            k += 1;
        }
        let h = xs[k + 1] - xs[k];
        let s = t - xs[k];
        let delta = (ys[k + 1] - ys[k]) / h;
        let c2 = (3. * delta - 2. * d[k] - d[k + 1]) / h;
        let c3 = (d[k] - 2. * delta + d[k + 1]) / (h * h);
        out.push(ys[k] + s * (d[k] + s * (c2 + s * c3)));
    }
    out
}

/// Slopes of the not-a-knot cubic spline (de Boor, A Practical Guide to
/// Splines, ch. IV); 2 points: the line, 3 points: the parabola.
fn spline_slopes(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let dx: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let del: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / dx[i]).collect();
    if n == 2 {
        return vec![del[0]; 2];
    }
    if n == 3 {
        // Parabola through the three points.
        let c = (del[1] - del[0]) / (x[2] - x[0]);
        return vec![del[0] - c * dx[0], del[0] + c * dx[0], del[1] + c * dx[1]];
    }
    // Tridiagonal system: sub a, diag b, super c, rhs r.
    let mut a = vec![0.; n];
    let mut b = vec![0.; n];
    let mut c = vec![0.; n];
    let mut r = vec![0.; n];
    let x31 = x[2] - x[0];
    b[0] = dx[1];
    c[0] = x31;
    r[0] = ((dx[0] + 2. * x31) * dx[1] * del[0] + dx[0] * dx[0] * del[1]) / x31;
    for i in 1..n - 1 {
        a[i] = dx[i];
        b[i] = 2. * (dx[i - 1] + dx[i]);
        c[i] = dx[i - 1];
        r[i] = 3. * (dx[i] * del[i - 1] + dx[i - 1] * del[i]);
    }
    let xn = x[n - 1] - x[n - 3];
    a[n - 1] = xn;
    b[n - 1] = dx[n - 3];
    r[n - 1] =
        (dx[n - 2] * dx[n - 2] * del[n - 3] + (2. * xn + dx[n - 2]) * dx[n - 3] * del[n - 2]) / xn;
    tridiagonal_solve(a, b, c, r)
}

/// Gaussian elimination with partial pivoting on a tridiagonal system
/// (sub-diagonal a[1..], diagonal b, super-diagonal c[..n-1]).
fn tridiagonal_solve(a: Vec<f64>, mut b: Vec<f64>, mut c: Vec<f64>, mut r: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    let mut a = a;
    let mut c2 = vec![0.; n]; // second super-diagonal created by row swaps
    for i in 0..n - 1 {
        if a[i + 1].abs() > b[i].abs() {
            // Swap rows i and i+1.
            let (bi, ci, c2i, ri) = (b[i], c[i], c2[i], r[i]);
            b[i] = a[i + 1];
            c[i] = b[i + 1];
            c2[i] = if i + 1 < n - 1 { c[i + 1] } else { 0. };
            r[i] = r[i + 1];
            a[i + 1] = bi;
            b[i + 1] = ci;
            if i + 1 < n - 1 {
                c[i + 1] = c2i;
            }
            r[i + 1] = ri;
        }
        let m = a[i + 1] / b[i];
        b[i + 1] -= m * c[i];
        if i + 1 < n - 1 {
            c[i + 1] -= m * c2[i];
        }
        r[i + 1] -= m * r[i];
        a[i + 1] = 0.;
    }
    let mut s = vec![0.; n];
    for i in (0..n).rev() {
        let mut v = r[i];
        if i + 1 < n {
            v -= c[i] * s[i + 1];
        }
        if i + 2 < n {
            v -= c2[i] * s[i + 2];
        }
        s[i] = v / b[i];
    }
    s
}

/// Shape-preserving piecewise cubic slopes (Fritsch & Carlson 1980, with the
/// three-point end formula of Moler, Numerical Computing with MATLAB 3.6).
fn pchip_slopes(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let del: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
    if n == 2 {
        return vec![del[0]; 2];
    }
    let mut d = vec![0.; n];
    for k in 1..n - 1 {
        if del[k - 1] * del[k] > 0. {
            let w1 = 2. * h[k] + h[k - 1];
            let w2 = h[k] + 2. * h[k - 1];
            d[k] = (w1 + w2) / (w1 / del[k - 1] + w2 / del[k]);
        }
    }
    let end = |h0: f64, h1: f64, d0: f64, d1: f64| {
        let mut s = ((2. * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
        if s.signum() != d0.signum() || d0 == 0. {
            s = 0.;
        } else if d0.signum() != d1.signum() && s.abs() > (3. * d0).abs() {
            s = 3. * d0;
        }
        s
    };
    d[0] = end(h[0], h[1], del[0], del[1]);
    d[n - 1] = end(h[n - 2], h[n - 3], del[n - 2], del[n - 3]);
    d
}
