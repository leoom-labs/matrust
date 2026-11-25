//! differentiate and integrate on fit objects: Richardson-extrapolated central
//! differences, and adaptive Gauss-Kronrod (7-15) quadrature between sorted
//! points (interpolant knots are breakpoints, so each piece is smooth).
use super::super::{Engine, Execution, Value};
use super::Fitted;
use crate::{Matrix, Result};

/// Step for point x: a power of two times the larger of |x| and the data span.
fn step(f: &Fitted, x: f64, k: usize) -> f64 {
    let span = f
        .range
        .get(k)
        .map(|(lo, hi)| hi - lo)
        .filter(|s| s.is_finite() && *s > 0.)
        .unwrap_or(1.);
    let scale = f.norm.get(k).map_or(span, |(_, sd)| sd.min(span));
    x.abs().max(scale) * 2f64.powi(-9)
}

fn shaped(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

/// [fx, fxx] = differentiate(cfit, x); [fx, fy, fxx, fxy, fyy] = differentiate(sfit, x, y).
pub(super) fn differentiate(
    e: &mut Engine,
    f: &Fitted,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if rest.is_empty() {
        return Err("differentiate: expected differentiate(fitobject, x)".into());
    }
    let (xs, (rows, cols), used) = super::query_points(e, &f.ft, rest, "differentiate")?;
    if used != rest.len() {
        return Err("differentiate: too many input arguments".into());
    }
    let n = xs[0].len();
    if !f.ft.surface() {
        let x = &xs[0];
        let h: Vec<f64> = x.iter().map(|&v| step(f, v, 0)).collect();
        // Points x, x +- h, x +- h/2 in one evaluation.
        let mut pts = Vec::with_capacity(5 * n);
        pts.extend_from_slice(x);
        for s in [1., -1., 0.5, -0.5] {
            pts.extend(x.iter().zip(&h).map(|(v, h)| v + s * h));
        }
        let v = super::eval_fitted(e, f, &[pts], out)?;
        let at = |k: usize, i: usize| v[k * n + i];
        let mut fx = Vec::with_capacity(n);
        let mut fxx = Vec::with_capacity(n);
        for (i, &hi) in h.iter().enumerate().take(n) {
            let (f0, p1, m1, p2, m2) = (at(0, i), at(1, i), at(2, i), at(3, i), at(4, i));
            let d1 = (p1 - m1) / (2. * hi);
            let d2 = (p2 - m2) / hi;
            fx.push((4. * d2 - d1) / 3.);
            let s1 = (p1 - 2. * f0 + m1) / (h[i] * h[i]);
            let s2 = (p2 - 2. * f0 + m2) / (h[i] * h[i] / 4.);
            fxx.push((4. * s2 - s1) / 3.);
        }
        if nargout > 2 {
            return Err("differentiate: a curve fit has at most two outputs [fx, fxx]".into());
        }
        let mut values = vec![shaped(rows, cols, fx)?];
        if nargout >= 2 {
            values.push(shaped(rows, cols, fxx)?);
        }
        return Ok(values);
    }
    if nargout > 5 {
        return Err(
            "differentiate: a surface fit has at most five outputs [fx, fy, fxx, fxy, fyy]".into(),
        );
    }
    let (x, y) = (&xs[0], &xs[1]);
    let hx: Vec<f64> = x.iter().map(|&v| step(f, v, 0)).collect();
    let hy: Vec<f64> = y.iter().map(|&v| step(f, v, 1)).collect();
    // Offsets (sx, sy) in units of (hx, hy); each derivative is a Richardson
    // combination of the h and h/2 stencils.
    let offsets: Vec<(f64, f64)> = {
        let mut o = vec![(0., 0.)];
        for s in [1., 0.5] {
            o.extend([
                (s, 0.),
                (-s, 0.),
                (0., s),
                (0., -s),
                (s, s),
                (s, -s),
                (-s, s),
                (-s, -s),
            ]);
        }
        o
    };
    let mut px = Vec::with_capacity(offsets.len() * n);
    let mut py = Vec::with_capacity(offsets.len() * n);
    for &(sx, sy) in &offsets {
        px.extend(x.iter().zip(&hx).map(|(v, h)| v + sx * h));
        py.extend(y.iter().zip(&hy).map(|(v, h)| v + sy * h));
    }
    let v = super::eval_fitted(e, f, &[px, py], out)?;
    let at = |k: usize, i: usize| v[k * n + i];
    let mut outs: Vec<Vec<f64>> = (0..5).map(|_| Vec::with_capacity(n)).collect();
    for i in 0..n {
        let f0 = at(0, i);
        let stencil = |base: usize, s: f64| {
            let (hxs, hys) = (hx[i] * s, hy[i] * s);
            let (xp, xm, yp, ym) = (
                at(base, i),
                at(base + 1, i),
                at(base + 2, i),
                at(base + 3, i),
            );
            let (pp, pm, mp, mm) = (
                at(base + 4, i),
                at(base + 5, i),
                at(base + 6, i),
                at(base + 7, i),
            );
            [
                (xp - xm) / (2. * hxs),
                (yp - ym) / (2. * hys),
                (xp - 2. * f0 + xm) / (hxs * hxs),
                (pp - pm - mp + mm) / (4. * hxs * hys),
                (yp - 2. * f0 + ym) / (hys * hys),
            ]
        };
        let (a, b) = (stencil(1, 1.), stencil(9, 0.5));
        for k in 0..5 {
            outs[k].push((4. * b[k] - a[k]) / 3.);
        }
    }
    outs.into_iter()
        .take(nargout.max(1))
        .map(|d| shaped(rows, cols, d))
        .collect()
}

const GK_NODES: [f64; 8] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_4,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_5,
    0.,
];
const GK_WEIGHTS: [f64; 8] = [
    0.022_935_322_010_529_22,
    0.063_092_092_629_978_55,
    0.104_790_010_322_250_2,
    0.140_653_259_715_525_9,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_4,
    0.204_432_940_075_298_9,
    0.209_482_141_084_727_8,
];
const G_WEIGHTS: [f64; 4] = [
    0.129_484_966_168_869_7,
    0.279_705_391_489_276_7,
    0.381_830_050_505_118_9,
    0.417_959_183_673_469_4,
];

/// Integrals over the intervals (a_k, b_k), each by adaptive Gauss-Kronrod;
/// all intervals of one level share one model evaluation.
fn integrals(
    e: &mut Engine,
    f: &Fitted,
    intervals: &[(f64, f64)],
    out: &mut Execution,
) -> Result<Vec<f64>> {
    let mut totals = vec![0.; intervals.len()];
    let mut pending: Vec<(usize, f64, f64, usize)> = intervals
        .iter()
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(k, &(a, b))| (k, a, b, 0))
        .collect();
    while !pending.is_empty() {
        e.check_cancel()?;
        let mut pts = Vec::with_capacity(15 * pending.len());
        for &(_, a, b, _) in &pending {
            let (c, h) = ((a + b) / 2., (b - a) / 2.);
            for &t in &GK_NODES[..7] {
                pts.push(c - h * t);
                pts.push(c + h * t);
            }
            pts.push(c);
        }
        let v = super::eval_fitted(e, f, &[pts], out)?;
        let mut next = Vec::new();
        for (j, &(k, a, b, depth)) in pending.iter().enumerate() {
            let w = &v[15 * j..15 * j + 15];
            let h = (b - a) / 2.;
            let mut kron = GK_WEIGHTS[7] * w[14];
            let mut gauss = G_WEIGHTS[3] * w[14];
            for i in 0..7 {
                let pair = w[2 * i] + w[2 * i + 1];
                kron += GK_WEIGHTS[i] * pair;
                if i % 2 == 1 {
                    gauss += G_WEIGHTS[i / 2] * pair;
                }
            }
            let (kron, gauss) = (kron * h, gauss * h);
            let err = (kron - gauss).abs();
            if err <= 1e-12 * kron.abs().max(1e-300)
                || err < 1e-15 * h.abs()
                || depth >= 30
                || !err.is_finite()
            {
                totals[k] += kron;
            } else {
                let m = (a + b) / 2.;
                next.push((k, a, m, depth + 1));
                next.push((k, m, b, depth + 1));
            }
        }
        pending = next;
        if pending.len() > 100_000 {
            return Err("integrate: the integral needs too many subintervals".into());
        }
    }
    Ok(totals)
}

/// integrate(cfit, x, x0): the integral of the fit from x0 to each x.
pub(super) fn integrate(
    e: &mut Engine,
    f: &Fitted,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    if f.ft.surface() {
        return Err("integrate: only curve fits (cfit) can be integrated".into());
    }
    let [xv, x0v] = rest else {
        return Err("integrate: expected integrate(fitobject, x, x0)".into());
    };
    let (xs, (rows, cols), _) =
        super::query_points(e, &f.ft, std::slice::from_ref(xv), "integrate")?;
    let x0 = super::fit::real_matrix(e, x0v, "integrate", "x0")?;
    if x0.len() != 1 || !x0.data()[0].is_finite() {
        return Err("integrate: x0 must be a finite scalar".into());
    }
    let x0 = x0.data()[0];
    let x = &xs[0];
    if x.iter().any(|v| !v.is_finite()) {
        return Err("integrate: x must be finite".into());
    }
    // -0 and +0 are one point: `dedup` merges them, so the lookups below must not tell them apart.
    let unsigned_zero = |v: f64| if v == 0. { 0. } else { v };
    let x0 = unsigned_zero(x0);
    let mut pts: Vec<f64> = x.iter().map(|&v| unsigned_zero(v)).chain([x0]).collect();
    if let Some(k) = &f.knots {
        // Interpolant breakpoints in data units.
        let (lo, hi) = pts.iter().fold((x0, x0), |(a, b), &v| (a.min(v), b.max(v)));
        pts.extend(
            k.x.iter()
                .map(|&v| unsigned_zero(f.norm.first().map_or(v, |(m, s)| v * s + m)))
                .filter(|v| *v > lo && *v < hi),
        );
    }
    pts.sort_by(f64::total_cmp);
    pts.dedup();
    let intervals: Vec<(f64, f64)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
    let parts = integrals(e, f, &intervals, out)?;
    let mut cumulative = vec![0.; pts.len()];
    for (k, p) in parts.iter().enumerate() {
        cumulative[k + 1] = cumulative[k] + p;
    }
    let origin = cumulative[pts.binary_search_by(|v| v.total_cmp(&x0)).unwrap()];
    let data = x
        .iter()
        .map(|&v| {
            let v = unsigned_zero(v);
            cumulative[pts.binary_search_by(|p| p.total_cmp(&v)).unwrap()] - origin
        })
        .collect();
    shaped(rows, cols, data)
}
