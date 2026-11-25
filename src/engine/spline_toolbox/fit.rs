//! Spline fitting: spaps (Reinsch smoothing to a tolerance), spap2
//! (weighted least squares in B-form), aptknt and augknt.
//!
//! spaps minimises rho E(f) + int |f''|^2 with E(f) = sum_j w_j |y_j - f(x_j)|^2
//! over cubic splines (C. H. Reinsch, Smoothing by spline functions,
//! Numer. Math. 10 (1967) 177-183). That is the csaps objective with p = 1/2
//! and weights rho w, solved by the banded system of poly_interp; rho with
//! E(f) = tol is found by safeguarded regula falsi (Illinois) on
//! log E against log rho (E decreases in rho).
//!
//! spap2 solves the least-squares problem with the banded collocation
//! matrix by Givens rotations (Lawson & Hanson, Solving Least Squares
//! Problems, ch. 27; de Boor, ch. XIV) - no normal equations.
use super::super::poly_interp::{CubicPp, smoothing_data, smoothing_spline};
use super::forms::{Bsp, Pp, clamped_knots};
use crate::Result;

fn to_pp(c: CubicPp) -> Pp {
    Pp {
        breaks: c.breaks,
        coefs: c.coefs.into_iter().flatten().collect(),
        order: 4,
        dim: c.dim,
    }
}

/// The spaps minimiser for one rho (0: weighted line; inf: interpolant).
fn spline_for(x: &[f64], ys: &[Vec<f64>], w: &[f64], rho: f64) -> Result<Pp> {
    let (p, wr): (f64, Vec<f64>) = if rho == 0. {
        (0., w.to_vec())
    } else if rho.is_infinite() {
        (1., w.to_vec())
    } else {
        (0.5, w.iter().map(|v| v * rho).collect())
    };
    let data = smoothing_data("spaps", x, ys, &wr)?;
    Ok(to_pp(smoothing_spline("spaps", &data, Some(p))?.0))
}

fn error_of(pp: &Pp, x: &[f64], ys: &[Vec<f64>], w: &[f64]) -> f64 {
    let mut e = 0.;
    for (i, &xi) in x.iter().enumerate() {
        for (c, y) in ys.iter().enumerate() {
            let r = y[i] - pp.eval(xi, c);
            e += w[i] * r * r;
        }
    }
    e
}

/// spaps for sorted-or-not sites `x`, series `ys`, weights `w` and
/// `tol` (negative: rho = -tol). Returns the spline and rho.
pub(super) fn spaps(
    x: &[f64],
    ys: &[Vec<f64>],
    w: &[f64],
    tol: f64,
    cancel: &dyn Fn() -> Result<()>,
) -> Result<(Pp, f64)> {
    if tol < 0. {
        let rho = -tol;
        return Ok((spline_for(x, ys, w, rho)?, rho));
    }
    let line = spline_for(x, ys, w, 0.)?;
    if error_of(&line, x, ys, w) <= tol {
        return Ok((line, 0.));
    }
    if tol == 0. {
        return Ok((spline_for(x, ys, w, f64::INFINITY)?, f64::INFINITY));
    }
    // phi(L) = log E(e^L) - log tol, decreasing; bracket from L = 0.
    let phi = |l: f64| -> Result<(f64, Pp)> {
        let pp = spline_for(x, ys, w, l.exp())?;
        let e = error_of(&pp, x, ys, w);
        Ok(((e.max(f64::MIN_POSITIVE)).ln() - tol.ln(), pp))
    };
    let (f0, pp0) = phi(0.)?;
    let (mut lo, mut flo, mut hi, mut fhi, mut best);
    if f0 > 0. {
        // E too large at rho = 1: increase rho until E <= tol.
        (lo, flo) = (0., f0);
        loop {
            let l = lo + 2.3;
            let (f, pp) = phi(l)?;
            if f <= 0. {
                (hi, fhi, best) = (l, f, pp);
                break;
            }
            (lo, flo) = (l, f);
            if l > 700. {
                return Ok((spline_for(x, ys, w, f64::INFINITY)?, f64::INFINITY));
            }
            cancel()?;
        }
    } else {
        (hi, fhi, best) = (0., f0, pp0);
        loop {
            let l = hi - 2.3;
            let (f, pp) = phi(l)?;
            if f > 0. {
                (lo, flo) = (l, f);
                break;
            }
            (hi, fhi, best) = (l, f, pp);
            if l < -700. {
                return Ok((line, 0.));
            }
            cancel()?;
        }
    }
    // Illinois regula falsi; keep hi on the E <= tol side.
    let mut side = 0;
    for _ in 0..200 {
        if fhi.abs() < 1e-14 || hi - lo < 1e-14 * (1. + hi.abs()) {
            break;
        }
        let mut m = hi - fhi * (hi - lo) / (fhi - flo);
        if !(m > lo && m < hi) {
            m = 0.5 * (lo + hi);
        }
        let (fm, pp) = phi(m)?;
        if fm <= 0. {
            hi = m;
            fhi = fm;
            best = pp;
            if side == -1 {
                flo /= 2.;
            }
            side = -1;
        } else {
            lo = m;
            flo = fm;
            if side == 1 {
                fhi /= 2.;
            }
            side = 1;
        }
        cancel()?;
    }
    Ok((best, hi.exp()))
}

/// The composite trapezoid weights of sorted copies of the sites, in the
/// original order.
pub(super) fn trapezoid_weights(x: &[f64]) -> Vec<f64> {
    let n = x.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut w = vec![0.; n];
    for p in 0..n - 1 {
        let h = (x[idx[p + 1]] - x[idx[p]]) / 2.;
        w[idx[p]] += h;
        w[idx[p + 1]] += h;
    }
    w
}

/// Weighted least-squares spline of order k on `knots` (dim series).
pub(super) fn spap2(
    knots: Vec<f64>,
    k: usize,
    x: &[f64],
    ys: &[Vec<f64>],
    w: &[f64],
    cancel: &dyn Fn() -> Result<()>,
) -> Result<Bsp> {
    let n = knots.len() - k;
    let dim = ys.len();
    let (a, b) = (knots[0], knots[knots.len() - 1]);
    if x.iter().any(|v| *v < a || *v > b) {
        return Err(format!(
            "spap2: the data sites must lie in the basic interval [{a}, {b}] of the knots"
        ));
    }
    let probe = Bsp {
        knots,
        coefs: vec![0.; n],
        order: k,
        dim: 1,
    };
    // R: n rows, k band entries (R[i][q] = element (i, i+q)); z: n-by-dim.
    let mut r: Vec<f64> = vec![0.; n * k];
    let mut z = vec![0.; n * dim];
    let mut row: Vec<f64> = vec![0.; k];
    let mut rhs = vec![0.; dim];
    for (i, &xi) in x.iter().enumerate() {
        if i % 4096 == 0 {
            cancel()?;
        }
        let sw = w[i].sqrt();
        if sw == 0. {
            continue;
        }
        let Some((mu, basis)) = probe.basis_at_checked(xi, cancel)? else {
            continue;
        };
        // Row entries for columns start..start+k (clip to 0..n).
        let first = mu - k as isize + 1;
        row.fill(0.);
        let mut start = first.max(0) as usize;
        for (q, bq) in basis.iter().enumerate() {
            let j = first + q as isize;
            if j >= 0 && (j as usize) < n {
                row[j as usize - start] = bq * sw;
            }
        }
        for (c, y) in ys.iter().enumerate() {
            rhs[c] = y[i] * sw;
        }
        // Rotate into R column by column.
        while start < n {
            let v = row[0];
            if v != 0. {
                let rr = &mut r[start * k..(start + 1) * k];
                let d = rr[0];
                let rho = d.hypot(v);
                let (cs, sn) = (d / rho, v / rho);
                for q in 0..k {
                    let (p, s) = (rr[q], row[q]);
                    rr[q] = cs * p + sn * s;
                    row[q] = -sn * p + cs * s;
                }
                for c in 0..dim {
                    let (p, s) = (z[start * dim + c], rhs[c]);
                    z[start * dim + c] = cs * p + sn * s;
                    rhs[c] = -sn * p + cs * s;
                }
            }
            // Shift the row one column left.
            row.rotate_left(1);
            row[k - 1] = 0.;
            start += 1;
            if row.iter().all(|v| *v == 0.) {
                break;
            }
        }
    }
    let scale = r.iter().fold(0f64, |m, v| m.max(v.abs()));
    for i in 0..n {
        // NaN pivots fail too.
        if r[i * k].abs() <= scale * 1e-13 || r[i * k].is_nan() {
            return Err("spap2: the data sites do not satisfy the Schoenberg-Whitney conditions for these knots; the least-squares spline is not unique (use fewer knots or more data sites between them)".into());
        }
    }
    // Back substitution.
    let mut a_out = vec![0.; n * dim];
    for i in (0..n).rev() {
        for c in 0..dim {
            let mut s = z[i * dim + c];
            for q in 1..k.min(n - i) {
                s -= r[i * k + q] * a_out[(i + q) * dim + c];
            }
            a_out[i * dim + c] = s / r[i * k];
        }
    }
    Ok(Bsp {
        knots: probe.knots,
        coefs: a_out,
        order: k,
        dim,
    })
}

/// aptknt: k-fold end knots and interior knots aveknt(tau, k); k drops to
/// numel(tau) when tau is shorter.
pub(super) fn aptknt(tau: &[f64], k: usize) -> (Vec<f64>, usize) {
    let n = tau.len();
    let k = k.min(n);
    let mut t = vec![tau[0]; k];
    if k == 1 {
        t.extend(tau.windows(2).map(|w| 0.5 * (w[0] + w[1])));
    } else {
        for j in 0..n - k {
            let s: f64 = tau[j + 1..j + k].iter().sum();
            t.push(s / (k - 1) as f64);
        }
    }
    t.extend(std::iter::repeat_n(tau[n - 1], k));
    (t, k)
}

/// augknt: distinct knots with the ends k-fold and interior multiplicities
/// `mults` (one value, or one per interior break); also the number of knots
/// added on the left.
pub(super) fn augknt(knots: &[f64], k: usize, mults: &[usize]) -> Result<(Vec<f64>, isize)> {
    let mut s = knots.to_vec();
    s.sort_by(f64::total_cmp);
    let mut u = s.clone();
    u.dedup();
    if u.len() < 2 {
        return Err("augknt: at least two distinct knots are needed".into());
    }
    let interior = u.len() - 2;
    if mults.len() != 1 && mults.len() != interior {
        return Err(format!(
            "augknt: mults must be a scalar or have one entry per interior knot ({interior})"
        ));
    }
    let left = s.iter().filter(|v| **v == u[0]).count() as isize;
    let mut out = vec![u[0]; k];
    for (j, v) in u[1..u.len() - 1].iter().enumerate() {
        let m = if mults.len() == 1 { mults[0] } else { mults[j] };
        out.extend(std::iter::repeat_n(*v, m));
    }
    out.extend(std::iter::repeat_n(u[u.len() - 1], k));
    Ok((out, k as isize - left))
}

/// The B-form of a cubic pp on its breaks (k = 4, simple interior knots).
pub(super) fn cubic_b(pp: &Pp) -> Bsp {
    super::forms::pp_to_b(pp, clamped_knots(&pp.breaks, 4), 4)
}
