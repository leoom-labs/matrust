//! Cubic smoothing spline (csaps), fnval, and the pp structs that
//! spline/pchip/makima(x, y) return.
//!
//! csaps minimises p sum_i w_i |y_i - f(x_i)|^2 + (1 - p) int |f''(t)|^2 dt over
//! natural cubic splines with knots at the data sites (Reinsch 1967; de Boor,
//! A Practical Guide to Splines, rev. ed. 2001, ch. XIV). With h_i = x_{i+1} - x_i,
//! Q the n-by-(n-2) second-divided-difference matrix (column j: 1/h_{j-1},
//! -(1/h_{j-1} + 1/h_j), 1/h_j), R the tridiagonal (n-2)-by-(n-2) matrix with
//! diagonal 2(h_{j-1} + h_j) and off-diagonal h_j, and W = diag(w):
//!   (6(1-p) Q'W^-1 Q + p R) u = Q'y,   f(x) = y - 6(1-p) W^-1 Q u,   f''/6 = p u,
//! the system is symmetric positive definite and pentadiagonal (banded LDL', O(n)).
//! Default p (the rule of the MIT `csaps` Python package, inside the public
//! doc's "near 1/(1+h^3/6)" range): p = 1/(1 + trace(R)/(6 trace(Q'W^-1 Q))).
use super::{Engine, Execution, Value, real_matrix, text_of};
use crate::{Array, CharacterRow, Matrix, Result, StructArray};
use std::sync::Arc;

/// A cubic piecewise polynomial with `dim` components: piece k, component j
/// has `coefs[k * dim + j] = [c3, c2, c1, c0]` in t = x - breaks[k] (the row
/// layout of mkpp).
#[derive(Clone)]
pub(in crate::engine) struct CubicPp {
    pub breaks: Vec<f64>,
    pub dim: usize,
    pub coefs: Vec<[f64; 4]>,
}

impl CubicPp {
    /// Component `j` at `q`; the end pieces extend outside the breaks.
    pub(in crate::engine) fn eval(&self, q: f64, j: usize) -> f64 {
        if q.is_nan() {
            return f64::NAN;
        }
        let k = self
            .breaks
            .partition_point(|&v| v <= q)
            .saturating_sub(1)
            .min(self.breaks.len() - 2);
        let t = q - self.breaks[k];
        let c = &self.coefs[k * self.dim + j];
        ((c[0] * t + c[1]) * t + c[2]) * t + c[3]
    }
    /// Values at `q` in ppval's shape: the shape of q for one component, else
    /// dim-by-numel(q) (q must then be a vector).
    pub(in crate::engine) fn values(&self, e: &Engine, q: &Matrix) -> Result<Value> {
        if self.dim > 1 && q.rows() > 1 && q.cols() > 1 {
            return Err(
                "Vector-valued ppval with a matrix query requires unsupported N-D output.".into(),
            );
        }
        e.polyopt_admit(q.len() * self.dim)?;
        let mut data = Vec::with_capacity(q.len() * self.dim);
        for (i, &v) in q.data().iter().enumerate() {
            if i % 65536 == 0 {
                e.check_cancel()?;
            }
            for j in 0..self.dim {
                data.push(self.eval(v, j));
            }
        }
        let (rows, cols) = if self.dim == 1 {
            (q.rows(), q.cols())
        } else {
            (self.dim, q.len())
        };
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
    }
    /// The struct mkpp(breaks, coefs, dim) returns (form, breaks, coefs,
    /// pieces, order, dim), order 4.
    pub(in crate::engine) fn to_value(&self, e: &Engine) -> Result<Value> {
        let pieces = self.breaks.len() - 1;
        let rows = pieces * self.dim;
        let mut data = vec![0.; rows * 4];
        for (r, c) in self.coefs.iter().enumerate() {
            for (k, v) in c.iter().enumerate() {
                data[k * rows + r] = *v;
            }
        }
        let fields: Vec<(&str, Array)> = vec![
            (
                "form",
                Array::Character(CharacterRow::from_text("pp", &e.cancel)?),
            ),
            (
                "breaks",
                Array::Numeric(Matrix::from_column_vec(
                    1,
                    self.breaks.len(),
                    self.breaks.clone(),
                )?),
            ),
            (
                "coefs",
                Array::Numeric(Matrix::from_column_vec(rows, 4, data)?),
            ),
            ("pieces", Array::Numeric(Matrix::scalar(pieces as f64))),
            ("order", Array::Numeric(Matrix::scalar(4.))),
            ("dim", Array::Numeric(Matrix::scalar(self.dim as f64))),
        ];
        let (names, slots) = fields
            .into_iter()
            .map(|(n, a)| (n.to_owned(), Arc::new(a)))
            .unzip();
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1, 1, names, slots, &e.cancel,
        )?)))
    }
}

/// Smoothing data: sorted distinct sites, their weighted-mean values (one
/// series of length n per component) and summed weights.
pub(in crate::engine) struct SmoothData {
    pub x: Vec<f64>,
    pub y: Vec<Vec<f64>>,
    pub w: Vec<f64>,
}

/// Sort the sites, drop points whose x or any y component is NaN, average
/// repeated sites (weighted, weights summed) and drop sites of zero total
/// weight (they do not change the minimiser). `y` holds `dim` series of
/// length x.len(); `w` is empty (all ones) or one weight per site.
pub(in crate::engine) fn smoothing_data(
    who: &str,
    x: &[f64],
    y: &[Vec<f64>],
    w: &[f64],
) -> Result<SmoothData> {
    let n = x.len();
    if !w.is_empty() && w.len() != n {
        return Err(format!(
            "{who}: the weights must have one value per data site ({n}), not {}",
            w.len()
        ));
    }
    if w.iter().any(|v| !v.is_finite() || *v < 0.) {
        return Err(format!("{who}: the weights must be finite and nonnegative"));
    }
    let weight = |i: usize| if w.is_empty() { 1. } else { w[i] };
    let mut idx: Vec<usize> = (0..n)
        .filter(|&i| !x[i].is_nan() && y.iter().all(|s| !s[i].is_nan()))
        .collect();
    if idx
        .iter()
        .any(|&i| x[i].is_infinite() || y.iter().any(|s| s[i].is_infinite()))
    {
        return Err(format!(
            "{who}: x and y must be finite (NaN points are ignored)"
        ));
    }
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let dim = y.len();
    let mut out = SmoothData {
        x: Vec::new(),
        y: vec![Vec::new(); dim],
        w: Vec::new(),
    };
    // Weighted sums per site, divided by the summed weight at the end.
    let mut k = 0;
    while k < idx.len() {
        let site = x[idx[k]];
        let mut total = 0.;
        let mut sums = vec![0.; dim];
        let mut plain = vec![0.; dim];
        let mut count = 0.;
        while k < idx.len() && x[idx[k]] == site {
            let i = idx[k];
            let wi = weight(i);
            total += wi;
            count += 1.;
            for (j, s) in y.iter().enumerate() {
                sums[j] += wi * s[i];
                plain[j] += s[i];
            }
            k += 1;
        }
        if total > 0. {
            out.x.push(site);
            for j in 0..dim {
                out.y[j].push(if count == 1. {
                    plain[j]
                } else {
                    sums[j] / total
                });
            }
            out.w.push(total);
        }
    }
    if out.x.len() < 2 {
        return Err(format!(
            "{who}: at least two distinct data sites (with finite values and positive weight) are needed"
        ));
    }
    Ok(out)
}

/// Pentadiagonal symmetric matrix by its diagonal and first two superdiagonals.
struct Banded {
    d0: Vec<f64>,
    d1: Vec<f64>,
    d2: Vec<f64>,
}

/// The blocks of the smoothing system: R (diagonal, superdiagonal) and
/// Q'W^-1 Q (pentadiagonal), both of size n-2.
fn blocks(x: &[f64], w: &[f64]) -> (Vec<f64>, Vec<f64>, Banded) {
    let n = x.len();
    let m = n - 2;
    let h: Vec<f64> = x.windows(2).map(|v| v[1] - v[0]).collect();
    // Column j of Q (interior site j+1): rows j, j+1, j+2.
    let a: Vec<f64> = (0..m).map(|j| 1. / h[j]).collect();
    let b: Vec<f64> = (0..m).map(|j| -(1. / h[j] + 1. / h[j + 1])).collect();
    let c: Vec<f64> = (0..m).map(|j| 1. / h[j + 1]).collect();
    let r0 = (0..m).map(|j| 2. * (h[j] + h[j + 1])).collect();
    let r1 = (0..m.saturating_sub(1)).map(|j| h[j + 1]).collect();
    let q = Banded {
        d0: (0..m)
            .map(|j| a[j] * a[j] / w[j] + b[j] * b[j] / w[j + 1] + c[j] * c[j] / w[j + 2])
            .collect(),
        d1: (0..m.saturating_sub(1))
            .map(|j| b[j] * a[j + 1] / w[j + 1] + c[j] * b[j + 1] / w[j + 2])
            .collect(),
        d2: (0..m.saturating_sub(2))
            .map(|j| c[j] * a[j + 2] / w[j + 2])
            .collect(),
    };
    (r0, r1, q)
}

/// Default smoothing parameter 1/(1 + trace(R)/(6 trace(Q'W^-1 Q))); 1 for two sites.
pub(in crate::engine) fn default_p(x: &[f64], w: &[f64]) -> f64 {
    if x.len() < 3 {
        return 1.;
    }
    let (r0, _, q) = blocks(x, w);
    let tr: f64 = r0.iter().sum();
    let tq: f64 = q.d0.iter().sum();
    1. / (1. + tr / (6. * tq))
}

/// LDL' factorization of a symmetric positive definite pentadiagonal matrix
/// (the banded Cholesky of LAPACK dpbtrf without square roots); None when a
/// pivot is not positive.
struct Ldl {
    d: Vec<f64>,
    l1: Vec<f64>,
    l2: Vec<f64>,
}
impl Ldl {
    fn new(a: &Banded) -> Option<Self> {
        let m = a.d0.len();
        let mut d = vec![0.; m];
        let mut l1 = vec![0.; m.saturating_sub(1)];
        let mut l2 = vec![0.; m.saturating_sub(2)];
        for j in 0..m {
            let mut dj = a.d0[j];
            if j >= 1 {
                dj -= l1[j - 1] * l1[j - 1] * d[j - 1];
            }
            if j >= 2 {
                dj -= l2[j - 2] * l2[j - 2] * d[j - 2];
            }
            if dj.is_nan() || dj <= 0. || dj.is_infinite() {
                return None;
            }
            d[j] = dj;
            if j + 1 < m {
                let mut v = a.d1[j];
                if j >= 1 {
                    v -= l2[j - 1] * d[j - 1] * l1[j - 1];
                }
                l1[j] = v / dj;
            }
            if j + 2 < m {
                l2[j] = a.d2[j] / dj;
            }
        }
        Some(Self { d, l1, l2 })
    }
    fn solve(&self, b: &mut [f64]) {
        let m = b.len();
        for j in 1..m {
            b[j] -= self.l1[j - 1] * b[j - 1];
            if j >= 2 {
                b[j] -= self.l2[j - 2] * b[j - 2];
            }
        }
        for (bj, dj) in b.iter_mut().zip(&self.d) {
            *bj /= dj;
        }
        for j in (0..m.saturating_sub(1)).rev() {
            b[j] -= self.l1[j] * b[j + 1];
            if j + 2 < m {
                b[j] -= self.l2[j] * b[j + 2];
            }
        }
    }
}

/// The smoothing spline of prepared data and the smoothing parameter used
/// (`p` None = the default rule).
pub(in crate::engine) fn smoothing_spline(
    who: &str,
    data: &SmoothData,
    p: Option<f64>,
) -> Result<(CubicPp, f64)> {
    let x = &data.x;
    let n = x.len();
    let dim = data.y.len();
    let p = match p {
        Some(p) if (0. ..=1.).contains(&p) => p,
        Some(_) => {
            return Err(format!(
                "{who}: the smoothing parameter p must be between 0 and 1"
            ));
        }
        None => default_p(x, &data.w),
    };
    let h: Vec<f64> = x.windows(2).map(|v| v[1] - v[0]).collect();
    if h.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err(format!(
            "{who}: the data sites are too far apart or too close"
        ));
    }
    let mut coefs = vec![[0.; 4]; (n - 1) * dim];
    if n == 2 {
        for (j, y) in data.y.iter().enumerate() {
            coefs[j] = [0., 0., (y[1] - y[0]) / h[0], y[0]];
        }
        return Ok((
            CubicPp {
                breaks: x.clone(),
                dim,
                coefs,
            },
            p,
        ));
    }
    let m = n - 2;
    let (r0, r1, q) = blocks(x, &data.w);
    let s = 6. * (1. - p);
    let a = Banded {
        d0: (0..m).map(|j| s * q.d0[j] + p * r0[j]).collect(),
        d1: (0..m.saturating_sub(1))
            .map(|j| s * q.d1[j] + p * r1[j])
            .collect(),
        d2: q.d2.iter().map(|v| s * v).collect(),
    };
    // The system loses accuracy when sites nearly coincide (entries ~ 1/h^2)
    // and p is far below the default; a failed factorization is reported.
    let ldl = Ldl::new(&a).ok_or_else(|| {
        format!(
            "{who}: the smoothing system is numerically singular for p = {p}: some data sites are too close together (or weights too small) for this much smoothing; use p nearer 1 or the default p, or merge nearly equal x values"
        )
    })?;
    for (j, y) in data.y.iter().enumerate() {
        let slope: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
        let mut u: Vec<f64> = (0..m).map(|i| slope[i + 1] - slope[i]).collect();
        ldl.solve(&mut u);
        // u padded with the natural end conditions u_0 = u_{n-1} = 0.
        let up = |i: usize| if i == 0 || i == n - 1 { 0. } else { u[i - 1] };
        // Smoothed values f = y - 6(1-p) W^-1 Q u, (Q u)_i = d1_i - d1_{i-1}
        // with d1_i = (u_{i+1} - u_i)/h_i and d1_{-1} = d1_{n-1} = 0.
        let d1: Vec<f64> = (0..n - 1).map(|i| (up(i + 1) - up(i)) / h[i]).collect();
        let f: Vec<f64> = (0..n)
            .map(|i| {
                let right = if i < n - 1 { d1[i] } else { 0. };
                let left = if i > 0 { d1[i - 1] } else { 0. };
                y[i] - s * (right - left) / data.w[i]
            })
            .collect();
        for i in 0..n - 1 {
            let (c0, c1) = (p * up(i), p * up(i + 1));
            coefs[i * dim + j] = [
                (c1 - c0) / h[i],
                3. * c0,
                (f[i + 1] - f[i]) / h[i] - h[i] * (2. * c0 + c1),
                f[i],
            ];
        }
    }
    Ok((
        CubicPp {
            breaks: x.clone(),
            dim,
            coefs,
        },
        p,
    ))
}

fn optional_matrix(v: Option<&Value>, who: &str) -> Result<Option<Matrix>> {
    match v {
        None => Ok(None),
        Some(v) => {
            let m = real_matrix(v, who)?;
            Ok(if m.is_empty() { None } else { Some(m) })
        }
    }
}

impl Engine {
    /// [pp, p] = csaps(x, y, p, xx, w) in its documented forms.
    pub(super) fn polyopt_csaps(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 5 {
            return Err("Too many input arguments.".into());
        }
        if matches!(&args[0], Value::Container(Array::Cell(_))) {
            return Err("csaps: gridded data (x as a cell array) is not supported yet".into());
        }
        let x = real_matrix(&args[0], "csaps")?;
        let y = real_matrix(&args[1], "csaps")?;
        if x.rows() != 1 && x.cols() != 1 {
            return Err("csaps: x must be a vector of data sites".into());
        }
        let n = x.len();
        let series: Vec<Vec<f64>> = if y.rows() == 1 || y.cols() == 1 {
            if y.len() != n {
                return Err(format!(
                    "csaps: x and y must have the same number of data sites ({n} sites, {} values)",
                    y.len()
                ));
            }
            vec![y.data().to_vec()]
        } else {
            if y.cols() != n {
                return Err(format!(
                    "csaps: y must have one column per data site ({n}), one row per component; it is {}-by-{}",
                    y.rows(),
                    y.cols()
                ));
            }
            (0..y.rows())
                .map(|r| (0..n).map(|c| y.get(r, c)).collect())
                .collect()
        };
        self.polyopt_admit(n * (series.len() + 12))?;
        let p = match optional_matrix(args.get(2), "csaps")? {
            None => None,
            Some(m) if m.len() == 1 && !m.data()[0].is_nan() => Some(m.data()[0]),
            Some(_) => {
                return Err(
                    "csaps: the smoothing parameter p must be a scalar between 0 and 1".into(),
                );
            }
        };
        let w = optional_matrix(args.get(4), "csaps")?;
        if let Some(w) = &w
            && w.rows() != 1
            && w.cols() != 1
        {
            return Err("csaps: the weights must be a vector".into());
        }
        let w = w.map(|m| m.data().to_vec()).unwrap_or_default();
        let data = smoothing_data("csaps", x.data(), &series, &w)?;
        self.check_cancel()?;
        let (pp, used) = smoothing_spline("csaps", &data, p)?;
        let first = match optional_matrix(args.get(3), "csaps")? {
            Some(xx) => pp.values(self, &xx)?,
            None => pp.to_value(self)?,
        };
        let mut out = vec![first];
        if nout >= 2 {
            out.push(Value::Matrix(Matrix::scalar(used)));
        }
        Ok(out)
    }

    /// fnval(f, x) for pp structures (fnval(x, f) is accepted too).
    pub(super) fn polyopt_fnval(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let [a, b] = args else {
            return Err(if args.len() < 2 {
                "Not enough input arguments."
            } else {
                "fnval: the left-limit form fnval(f, x, 'l') is not supported yet"
            }
            .into());
        };
        let is_struct = |v: &Value| matches!(v, Value::Container(Array::Struct(_)));
        let (f, x) = if !is_struct(a) && is_struct(b) {
            (b, a)
        } else {
            (a, b)
        };
        if !is_struct(f) {
            return Err("fnval: the function must be a piecewise polynomial (pp) structure".into());
        }
        if text_of(x).is_some() {
            return Err("fnval: x must be numeric".into());
        }
        let mut values = self
            .lax_outputs("ppval", &[f.clone(), x.clone()], 1, out)?
            .unwrap_or_default();
        if values.is_empty() {
            return Err("fnval: could not evaluate the function".into());
        }
        Ok(values.swap_remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dense_solve(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
        let m = b.len();
        let mut a: Vec<Vec<f64>> = a.to_vec();
        let mut b = b.to_vec();
        for k in 0..m {
            let pivot = a[k].clone();
            for i in k + 1..m {
                let f = a[i][k] / pivot[k];
                for (aij, akj) in a[i][k..].iter_mut().zip(&pivot[k..]) {
                    *aij -= f * akj;
                }
                b[i] -= f * b[k];
            }
        }
        for k in (0..m).rev() {
            let s: f64 = (k + 1..m).map(|j| a[k][j] * b[j]).sum();
            b[k] = (b[k] - s) / a[k][k];
        }
        b
    }

    #[test]
    fn banded_ldl_matches_a_dense_solve() {
        let band = Banded {
            d0: vec![6., 7., 8., 9., 10.],
            d1: vec![1., -2., 0.5, 1.5],
            d2: vec![0.3, -0.4, 0.2],
        };
        let m = 5;
        let mut dense = vec![vec![0.; m]; m];
        for j in 0..m {
            dense[j][j] = band.d0[j];
            if j + 1 < m {
                dense[j][j + 1] = band.d1[j];
                dense[j + 1][j] = band.d1[j];
            }
            if j + 2 < m {
                dense[j][j + 2] = band.d2[j];
                dense[j + 2][j] = band.d2[j];
            }
        }
        let b = [1., -2., 3., 0.5, 4.];
        let want = dense_solve(&dense, &b);
        let mut got = b.to_vec();
        Ldl::new(&band).unwrap().solve(&mut got);
        for (g, w) in got.iter().zip(&want) {
            assert!((g - w).abs() < 1e-14, "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn natural_interpolant_and_line_at_the_ends_of_p() {
        let x = vec![0., 1., 2.5, 3., 4.5];
        let y = vec![vec![1., 3., 2., 5., 4.]];
        let data = smoothing_data("t", &x, &y, &[]).unwrap();
        let (pp, _) = smoothing_spline("t", &data, Some(1.)).unwrap();
        for (xi, yi) in x.iter().zip(&y[0]) {
            assert!((pp.eval(*xi, 0) - yi).abs() < 1e-13);
        }
        let (line, _) = smoothing_spline("t", &data, Some(0.)).unwrap();
        assert!(line.coefs.iter().all(|c| c[0] == 0. && c[1] == 0.));
        let slope = line.coefs[0][2];
        assert!(line.coefs.iter().all(|c| (c[2] - slope).abs() < 1e-13));
    }
}
