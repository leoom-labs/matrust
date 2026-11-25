//! Numerical kernels for system identification: polynomial filters in the
//! delay operator q^-1, least squares by Householder QR followed by an SVD of
//! the triangular factor (minimum-norm on rank deficiency, no normal
//! equations, no inverse), polynomial roots from companion eigenvalues, and a
//! Levenberg-Marquardt loop (Marquardt scaling; Ljung, System Identification,
//! 2nd ed., sec. 10.2; Nocedal & Wright, Numerical Optimization, sec. 10.3).
use crate::Result;
use nalgebra::{Complex, DMatrix, DVector};

pub(super) type C = Complex<f64>;
pub(super) type JacFn<'a> = &'a mut dyn FnMut(&[f64], &[f64]) -> DMatrix<f64>;

/// `y = filter(b, a, x)` with zero initial conditions (direct form II
/// transposed); `a[0]` must be nonzero.
pub(super) fn lfilter(b: &[f64], a: &[f64], x: &[f64]) -> Vec<f64> {
    let a0 = a[0];
    let n = a.len().max(b.len());
    let mut bb = vec![0.; n];
    let mut aa = vec![0.; n];
    for (i, v) in b.iter().enumerate() {
        bb[i] = v / a0;
    }
    for (i, v) in a.iter().enumerate() {
        aa[i] = v / a0;
    }
    let mut z = vec![0.; n];
    let mut y = Vec::with_capacity(x.len());
    for &xt in x {
        let yt = bb[0] * xt + z[0];
        for i in 1..n {
            z[i - 1] = bb[i] * xt + z[i] - aa[i] * yt;
        }
        y.push(yt);
    }
    y
}
/// x delayed by k samples (zeros enter at the start).
pub(super) fn shift(x: &[f64], k: usize) -> Vec<f64> {
    let n = x.len();
    let mut y = vec![0.; n];
    if k < n {
        y[k..].copy_from_slice(&x[..n - k]);
    }
    y
}
pub(super) fn conv(a: &[f64], b: &[f64]) -> Vec<f64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut c = vec![0.; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            c[i + j] += x * y;
        }
    }
    c
}
/// Roots of a polynomial given in descending powers (leading zeros ignored).
pub(super) fn roots(p: &[f64]) -> Vec<C> {
    let Some(i) = p.iter().position(|v| *v != 0.) else {
        return Vec::new();
    };
    let p = &p[i..];
    let n = p.len() - 1;
    if n == 0 {
        return Vec::new();
    }
    let mut m = DMatrix::<f64>::zeros(n, n);
    for j in 0..n {
        m[(0, j)] = -p[j + 1] / p[0];
    }
    for j in 1..n {
        m[(j, j - 1)] = 1.;
    }
    m.complex_eigenvalues().iter().copied().collect()
}
/// Monic real polynomial (descending powers) with the given roots; complex
/// roots are expected in conjugate pairs and the imaginary residue dropped.
pub(super) fn poly(r: &[C]) -> Vec<f64> {
    let mut c = vec![C::new(1., 0.)];
    for &z in r {
        let mut next = vec![C::new(0., 0.); c.len() + 1];
        for (i, v) in c.iter().enumerate() {
            next[i] += v;
            next[i + 1] -= v * z;
        }
        c = next;
    }
    c.iter().map(|v| v.re).collect()
}
/// Every root of the monic q^-1 polynomial strictly inside the unit circle.
pub(super) fn stable_q(p: &[f64]) -> bool {
    p.len() <= 1 || roots(p).iter().all(|r| r.norm() < 1. - 1e-10)
}
/// Every root in the open left half plane (continuous denominators).
pub(super) fn hurwitz(p: &[f64]) -> bool {
    p.len() <= 1 || roots(p).iter().all(|r| r.re < -1e-12)
}
/// Reflect roots outside the unit circle to 1/conj(r) (same spectrum shape).
pub(super) fn reflect(p: &[f64]) -> Vec<f64> {
    if stable_q(p) {
        return p.to_vec();
    }
    let r: Vec<C> = roots(p)
        .into_iter()
        .map(|r| {
            let m = r.norm();
            if m > 1. {
                r / (m * m)
            } else if m >= 1. - 1e-10 {
                r * 0.999
            } else {
                r
            }
        })
        .collect();
    poly(&r)
}
/// Least squares min |A x - b| through QR and an SVD of R; rank deficient
/// columns receive the minimum-norm solution.
pub(super) fn lstsq(a: &DMatrix<f64>, b: &DVector<f64>) -> Result<DVector<f64>> {
    let (m, n) = a.shape();
    if n == 0 {
        return Ok(DVector::zeros(0));
    }
    if a.iter().chain(b.iter()).any(|v| !v.is_finite()) {
        return Err("Regression data contains NaN or Inf".into());
    }
    let (r, rhs) = if m > n {
        let qr = a.clone().qr();
        let mut qtb = b.clone();
        qr.q_tr_mul(&mut qtb);
        (qr.r(), qtb.rows(0, n).into_owned())
    } else {
        (a.clone(), b.clone())
    };
    let svd = r.svd(true, true);
    let smax = svd.singular_values.iter().cloned().fold(0., f64::max);
    let tol = smax * (m.max(n) as f64) * f64::EPSILON;
    svd.solve(&rhs, tol)
        .map_err(|e| format!("Least squares failed: {e}"))
}
pub(super) fn sumsq(x: &[f64]) -> f64 {
    x.iter().map(|v| v * v).sum()
}

/// Outcome of a Levenberg-Marquardt minimisation.
pub(super) struct Lm {
    pub theta: Vec<f64>,
    pub iterations: usize,
    pub why: &'static str,
}
/// Minimise |r(theta)|^2. `r` returns None for an inadmissible parameter
/// (unstable predictor); `jac` gives the N-by-p Jacobian dr/dtheta.
pub(super) fn levenberg_marquardt(
    theta: Vec<f64>,
    max_iter: usize,
    cancel: &dyn Fn() -> Result<()>,
    r: &mut dyn FnMut(&[f64]) -> Option<Vec<f64>>,
    jac: JacFn<'_>,
) -> Result<Lm> {
    let mut theta = theta;
    let p = theta.len();
    let Some(mut res) = r(&theta) else {
        return Err("The initial model has an unstable predictor".into());
    };
    let mut v = sumsq(&res);
    let scale = v.max(1e-300);
    let mut lambda: f64 = 1e-4;
    let mut why = "Maximum number of iterations reached.";
    let mut iterations = 0;
    if p == 0 || v <= 1e-30 * scale.max(1.) {
        return Ok(Lm {
            theta,
            iterations,
            why: "Near (local) minimum, (norm(g) < tol).",
        });
    }
    'outer: for it in 0..max_iter {
        cancel()?;
        iterations = it + 1;
        let j = jac(&theta, &res);
        let n = res.len();
        let dscale: Vec<f64> = (0..p)
            .map(|c| j.column(c).norm().max(1e-12 * (1. + v.sqrt())))
            .collect();
        loop {
            let mut aug = DMatrix::<f64>::zeros(n + p, p);
            aug.view_mut((0, 0), (n, p)).copy_from(&j);
            for c in 0..p {
                aug[(n + c, c)] = lambda.sqrt() * dscale[c];
            }
            let mut rhs = DVector::<f64>::zeros(n + p);
            for i in 0..n {
                rhs[i] = -res[i];
            }
            let delta = lstsq(&aug, &rhs)?;
            let trial: Vec<f64> = theta.iter().zip(delta.iter()).map(|(a, b)| a + b).collect();
            let accepted = r(&trial).map(|rt| (sumsq(&rt), rt));
            match accepted {
                Some((vt, rt)) if vt < v => {
                    let gain = v - vt;
                    let step = delta.norm();
                    theta = trial;
                    res = rt;
                    v = vt;
                    lambda = (lambda / 10.).max(1e-12);
                    let size = theta.iter().map(|x| x * x).sum::<f64>().sqrt();
                    if gain <= 1e-13 * (v + gain) || step <= 1e-12 * (size + 1e-12) {
                        why = "Near (local) minimum, (norm(g) < tol).";
                        break 'outer;
                    }
                    if v <= 1e-30 * scale.max(1.) {
                        why = "Near (local) minimum, (norm(g) < tol).";
                        break 'outer;
                    }
                    break;
                }
                _ => {
                    lambda *= 10.;
                    if lambda > 1e14 {
                        why = "No improvement along the search direction.";
                        break 'outer;
                    }
                }
            }
        }
    }
    Ok(Lm {
        theta,
        iterations,
        why,
    })
}
