//! Cubic spline interpolation with end conditions (csapi: not-a-knot;
//! csape: complete/clamped, second, variational, periodic, not-a-knot and
//! the default Lagrange condition). The slopes s_i solve the C2 equations
//!   h_i s_{i-1} + 2(h_{i-1} + h_i) s_i + h_{i-1} s_{i+1} = 3(h_i d_{i-1} + h_{i-1} d_i)
//! (d_i the divided differences) with one row per end condition
//! (de Boor, A Practical Guide to Splines, ch. IV), a tridiagonal system
//! solved by Gaussian elimination with partial pivoting (LAPACK dgtsv); the
//! periodic system is cyclic and uses Sherman-Morrison on top.
use super::forms::Pp;
use crate::Result;

#[derive(Clone, Debug)]
pub(super) enum End {
    NotAKnot,
    /// Slope per component.
    Slope(Vec<f64>),
    /// Second derivative per component.
    Second(Vec<f64>),
    /// Slope of the cubic through the four data nearest the end.
    Lagrange,
    Periodic,
}

/// Tridiagonal solve with partial pivoting (dgtsv) for several right-hand
/// sides; false when singular.
fn gtsv(dl: &mut [f64], d: &mut [f64], du: &mut [f64], b: &mut [Vec<f64>]) -> bool {
    let n = d.len();
    if n == 1 {
        if d[0] == 0. {
            return false;
        }
        for r in b.iter_mut() {
            r[0] /= d[0];
        }
        return true;
    }
    for i in 0..n - 1 {
        if d[i].abs() >= dl[i].abs() {
            if d[i] == 0. {
                return false;
            }
            let fact = dl[i] / d[i];
            d[i + 1] -= fact * du[i];
            for r in b.iter_mut() {
                r[i + 1] -= fact * r[i];
            }
            dl[i] = 0.;
        } else {
            let fact = d[i] / dl[i];
            d[i] = dl[i];
            let temp = d[i + 1];
            d[i + 1] = du[i] - fact * temp;
            if i + 2 < n {
                dl[i] = du[i + 1];
                du[i + 1] = -fact * dl[i];
            }
            du[i] = temp;
            for r in b.iter_mut() {
                let t = r[i];
                r[i] = r[i + 1];
                r[i + 1] = t - fact * r[i + 1];
            }
        }
    }
    if d[n - 1] == 0. || !d[n - 1].is_finite() {
        return false;
    }
    for r in b.iter_mut() {
        r[n - 1] /= d[n - 1];
        r[n - 2] = (r[n - 2] - du[n - 2] * r[n - 1]) / d[n - 2];
        for i in (0..n.saturating_sub(2)).rev() {
            r[i] = (r[i] - du[i] * r[i + 1] - dl[i] * r[i + 2]) / d[i];
        }
    }
    true
}

/// Derivative at t[0] of the polynomial interpolating (t, v) (Newton form).
fn end_slope(t: &[f64], v: &[f64]) -> f64 {
    let m = t.len();
    let mut dd = v.to_vec();
    let mut slope = 0.;
    let mut prod = 1.;
    for j in 1..m {
        for i in (j..m).rev() {
            dd[i] = (dd[i] - dd[i - 1]) / (t[i] - t[i - j]);
        }
        slope += dd[j] * prod;
        prod *= t[0] - t[j];
    }
    slope
}

/// The cubic interpolant of sorted distinct sites `x` (n >= 2) and
/// component series `ys`.
pub(super) fn cubic_interpolant(
    who: &str,
    x: &[f64],
    ys: &[Vec<f64>],
    left: &End,
    right: &End,
) -> Result<Pp> {
    let n = x.len();
    let dim = ys.len();
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    if h.iter().any(|v| !(v.is_finite() && *v > 0.)) {
        return Err(format!("{who}: the data sites must be distinct and finite"));
    }
    let del: Vec<Vec<f64>> = ys
        .iter()
        .map(|y| (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect())
        .collect();
    let periodic = matches!(left, End::Periodic);
    let nak = matches!(left, End::NotAKnot) && matches!(right, End::NotAKnot);
    let slopes: Vec<Vec<f64>> = if n == 2 && (nak || periodic) {
        // A line (periodic: equal end values give the constant).
        del.iter()
            .map(|d| if periodic { vec![0.; 2] } else { vec![d[0]; 2] })
            .collect()
    } else if n == 3 && nak {
        // Not-a-knot at both ends of three sites: the parabola.
        (0..dim)
            .map(|c| {
                let c2 = (del[c][1] - del[c][0]) / (x[2] - x[0]);
                (0..3)
                    .map(|i| del[c][0] + c2 * (2. * x[i] - x[0] - x[1]))
                    .collect()
            })
            .collect()
    } else if periodic {
        periodic_slopes(who, &h, &del)?
    } else {
        let mut dl = vec![0.; n - 1];
        let mut d = vec![0.; n];
        let mut du = vec![0.; n - 1];
        let mut b = vec![vec![0.; n]; dim];
        for i in 1..n - 1 {
            dl[i - 1] = h[i];
            d[i] = 2. * (h[i - 1] + h[i]);
            du[i] = h[i - 1];
            for c in 0..dim {
                b[c][i] = 3. * (h[i] * del[c][i - 1] + h[i - 1] * del[c][i]);
            }
        }
        let lag = |c: usize, rev: bool| {
            let m = n.min(4);
            if rev {
                let t: Vec<f64> = x[n - m..].iter().rev().copied().collect();
                let v: Vec<f64> = ys[c][n - m..].iter().rev().copied().collect();
                end_slope(&t, &v)
            } else {
                end_slope(&x[..m], &ys[c][..m])
            }
        };
        match left {
            End::Slope(v) => {
                d[0] = 1.;
                for c in 0..dim {
                    b[c][0] = v[c];
                }
            }
            End::Lagrange => {
                d[0] = 1.;
                for (c, bc) in b.iter_mut().enumerate() {
                    bc[0] = lag(c, false);
                }
            }
            End::Second(v) => {
                d[0] = 2.;
                du[0] = 1.;
                for c in 0..dim {
                    b[c][0] = 3. * del[c][0] - v[c] * h[0] / 2.;
                }
            }
            End::NotAKnot if n >= 3 => {
                d[0] = h[1];
                du[0] = h[0] + h[1];
                for c in 0..dim {
                    b[c][0] = ((h[0] + 2. * (h[0] + h[1])) * h[1] * del[c][0]
                        + h[0] * h[0] * del[c][1])
                        / (h[0] + h[1]);
                }
            }
            End::NotAKnot | End::Periodic => {
                d[0] = 1.;
                for c in 0..dim {
                    b[c][0] = del[c][0];
                }
            }
        }
        let l = n - 1;
        match right {
            End::Slope(v) => {
                d[l] = 1.;
                for c in 0..dim {
                    b[c][l] = v[c];
                }
            }
            End::Lagrange => {
                d[l] = 1.;
                for (c, bc) in b.iter_mut().enumerate() {
                    bc[l] = lag(c, true);
                }
            }
            End::Second(v) => {
                dl[l - 1] = 1.;
                d[l] = 2.;
                for c in 0..dim {
                    b[c][l] = 3. * del[c][l - 1] + v[c] * h[l - 1] / 2.;
                }
            }
            End::NotAKnot if n >= 3 => {
                let (ha, hb) = (h[l - 2], h[l - 1]);
                dl[l - 1] = ha + hb;
                d[l] = ha;
                for c in 0..dim {
                    b[c][l] = (hb * hb * del[c][l - 2]
                        + (2. * (ha + hb) + hb) * ha * del[c][l - 1])
                        / (ha + hb);
                }
            }
            End::NotAKnot | End::Periodic => {
                d[l] = 1.;
                for c in 0..dim {
                    b[c][l] = del[c][l - 1];
                }
            }
        }
        if !gtsv(&mut dl, &mut d, &mut du, &mut b) {
            return Err(format!("{who}: the end conditions give a singular system"));
        }
        b
    };
    Ok(hermite(x, ys, &slopes, &h, &del))
}

/// Cubic Hermite pieces from values and slopes.
fn hermite(x: &[f64], ys: &[Vec<f64>], s: &[Vec<f64>], h: &[f64], del: &[Vec<f64>]) -> Pp {
    let n = x.len();
    let dim = ys.len();
    let mut coefs = vec![0.; (n - 1) * dim * 4];
    for i in 0..n - 1 {
        for c in 0..dim {
            let (s0, s1, dd, hi) = (s[c][i], s[c][i + 1], del[c][i], h[i]);
            let r = (i * dim + c) * 4;
            coefs[r] = (s0 + s1 - 2. * dd) / (hi * hi);
            coefs[r + 1] = (3. * dd - 2. * s0 - s1) / hi;
            coefs[r + 2] = s0;
            coefs[r + 3] = ys[c][i];
        }
    }
    Pp {
        breaks: x.to_vec(),
        coefs,
        order: 4,
        dim,
    }
}

/// Periodic slopes s_0..s_{n-2} (s_{n-1} = s_0): cyclic tridiagonal.
fn periodic_slopes(who: &str, h: &[f64], del: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
    let m = h.len(); // unknowns
    let dim = del.len();
    // Row i: a_i s_{i-1} + d_i s_i + c_i s_{i+1} (indices mod m).
    let hp = |i: usize| h[(i + m - 1) % m];
    let a: Vec<f64> = (0..m).map(|i| h[i]).collect();
    let dg: Vec<f64> = (0..m).map(|i| 2. * (hp(i) + h[i])).collect();
    let cc: Vec<f64> = (0..m).map(hp).collect();
    let rhs: Vec<Vec<f64>> = (0..dim)
        .map(|c| {
            (0..m)
                .map(|i| 3. * (h[i] * del[c][(i + m - 1) % m] + hp(i) * del[c][i]))
                .collect()
        })
        .collect();
    let mut out: Vec<Vec<f64>>;
    if m == 2 {
        // s_{i-1} and s_{i+1} coincide.
        let (a00, a01, a10, a11) = (dg[0], a[0] + cc[0], a[1] + cc[1], dg[1]);
        let det = a00 * a11 - a01 * a10;
        if det == 0. {
            return Err(format!("{who}: the periodic system is singular"));
        }
        out = rhs
            .iter()
            .map(|r| {
                vec![
                    (r[0] * a11 - a01 * r[1]) / det,
                    (a00 * r[1] - a10 * r[0]) / det,
                ]
            })
            .collect();
    } else {
        // Sherman-Morrison: A = T + u v', u = (g, 0.., c_{m-1}), v = (1, 0.., a_0/g).
        let g = -dg[0];
        let mut d = dg.clone();
        d[0] -= g;
        d[m - 1] -= a[0] * cc[m - 1] / g;
        let mut sys = rhs.clone();
        let mut u = vec![0.; m];
        u[0] = g;
        u[m - 1] = cc[m - 1];
        sys.push(u);
        let mut dl: Vec<f64> = (1..m).map(|i| a[i]).collect();
        let mut du: Vec<f64> = (0..m - 1).map(|i| cc[i]).collect();
        if !gtsv(&mut dl, &mut d, &mut du, &mut sys) {
            return Err(format!("{who}: the periodic system is singular"));
        }
        let z = sys.pop().unwrap();
        let denom = 1. + z[0] + a[0] * z[m - 1] / g;
        out = sys
            .into_iter()
            .map(|mut x| {
                let f = (x[0] + a[0] * x[m - 1] / g) / denom;
                for (xi, zi) in x.iter_mut().zip(&z) {
                    *xi -= f * zi;
                }
                x
            })
            .collect();
    }
    for s in out.iter_mut() {
        let first = s[0];
        s.push(first);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_and_clamped_reproduce_a_cubic() {
        // f = x^3 - 2x: clamped with exact slopes reproduces it.
        let x = [0., 0.5, 1.7, 2., 3.1];
        let f = |t: f64| t * t * t - 2. * t;
        let y = vec![x.iter().map(|&t| f(t)).collect::<Vec<_>>()];
        let pp = cubic_interpolant(
            "t",
            &x,
            &y,
            &End::Slope(vec![-2.]),
            &End::Slope(vec![3. * 3.1 * 3.1 - 2.]),
        )
        .unwrap();
        for t in [0.2, 1., 2.5, 3.] {
            assert!((pp.eval(t, 0) - f(t)).abs() < 1e-12);
        }
        let pp = cubic_interpolant("t", &x, &y, &End::NotAKnot, &End::NotAKnot).unwrap();
        for t in [0.2, 1., 2.5, 3.] {
            assert!((pp.eval(t, 0) - f(t)).abs() < 1e-12);
        }
        let pp = cubic_interpolant("t", &x, &y, &End::Lagrange, &End::Lagrange).unwrap();
        for t in [0.2, 1., 2.5, 3.] {
            assert!((pp.eval(t, 0) - f(t)).abs() < 1e-12);
        }
    }
}
