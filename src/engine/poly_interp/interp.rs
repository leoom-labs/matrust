//! interp1, interp2, spline, pchip.
use super::{Engine, Value, eps_of, is_vector, matrix_value, real_matrix, scalar_of, text_of};
use crate::Result;
use crate::engine::Fma;

/// Piecewise cubic: on [x[k], x[k+1]] y = ((c0 t + c1) t + c2) t + c3 with t = q - x[k].
struct Pp {
    x: Vec<f64>,
    c: Vec<[f64; 4]>,
}
impl Pp {
    fn from_slopes(x: &[f64], y: &[f64], s: &[f64]) -> Self {
        let c = (0..x.len() - 1)
            .map(|k| {
                let h = x[k + 1] - x[k];
                let delta = (y[k + 1] - y[k]) / h;
                let zz = (delta - s[k]) / h;
                let zx = (s[k + 1] - delta) / h;
                [(zx - zz) / h, 2. * zz - zx, s[k], y[k]]
            })
            .collect();
        Self { x: x.to_vec(), c }
    }
    /// The parabola through three points, with the cubic coefficient of both
    /// pieces exactly zero (the Hermite form would rebuild it from rounded
    /// differences).
    fn parabola(x: &[f64], y: &[f64]) -> Self {
        Self {
            x: x.to_vec(),
            c: parabola_pieces(x, y).to_vec(),
        }
    }
    fn eval(&self, q: f64) -> f64 {
        if q.is_nan() {
            return f64::NAN;
        }
        let k = interval(&self.x, q);
        let t = q - self.x[k];
        let c = &self.c[k];
        ((c[0] * t + c[1]) * t + c[2]) * t + c[3]
    }
}
/// Index k of the interval [x[k], x[k+1]] holding q (clamped to the end intervals).
fn interval(x: &[f64], q: f64) -> usize {
    x.partition_point(|&v| v <= q)
        .saturating_sub(1)
        .min(x.len() - 2)
}
fn sign(v: f64) -> f64 {
    if v > 0. {
        1.
    } else if v < 0. {
        -1.
    } else {
        0.
    }
}
/// Knot slopes of the shape-preserving piecewise cubic Hermite interpolant
/// (Fritsch & Butland, "A method for constructing local monotone piecewise
/// cubic interpolants", SIAM J. Sci. Stat. Comput. 5, 1984): interior knots
/// take [`harmonic_slope`], the two ends [`pchip_end_slope`]; two points give
/// the line.
fn pchip_slopes(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let d: Vec<f64> = (0..n - 1).map(|k| (y[k + 1] - y[k]) / h[k]).collect();
    if n == 2 {
        return vec![d[0]; 2];
    }
    let mut s = vec![0.; n];
    for k in 1..n - 1 {
        s[k] = harmonic_slope(h[k - 1], h[k], d[k - 1], d[k]);
    }
    s[0] = pchip_end_slope(h[0], h[1], d[0], d[1]);
    s[n - 1] = pchip_end_slope(h[n - 2], h[n - 3], d[n - 2], d[n - 3]);
    s
}
/// Interior slope between the secants `dl` (interval width `hl`) and `dr`
/// (`hr`): zero at a local extremum or flat stretch (the secants differ in
/// sign or one is zero), else the weighted harmonic mean of Fritsch & Butland
/// (1984), 1/s = a/dl + (1 - a)/dr with a = (hl + 2 hr)/(3 (hl + hr)).
/// With m = min(|dl|, |dr|) it is evaluated as m / (a m/|dl| + (1 - a) m/|dr|):
/// the denominator lies in [1/3, 1], so |s| <= 3 m (inside the monotonicity
/// region of Fritsch & Carlson, SIAM J. Numer. Anal. 17, 1980, sec. 4) and
/// nothing overflows; equal secants give s = dl exactly. NaN data give NaN.
fn harmonic_slope(hl: f64, hr: f64, dl: f64, dr: f64) -> f64 {
    if dl.is_nan() || dr.is_nan() {
        return f64::NAN;
    }
    if sign(dl) * sign(dr) <= 0. {
        return 0.;
    }
    let sum = hl + hr;
    let right = if sum.is_finite() {
        hr / sum
    } else {
        (0.5 * hr) / (0.5 * hl + 0.5 * hr)
    };
    let a = (1. + right) / 3.;
    let m = dl.abs().min(dr.abs());
    sign(dl) * (m / (a * (m / dl.abs()) + (1. - a) * (m / dr.abs())))
}
/// End slope (outer interval `h0`, `d0`; neighbour `h1`, `d1`): the slope of
/// the parabola through the three end points ([`parabola_end_slope`]), moved
/// into the Fritsch & Carlson (1980, sec. 4) monotonicity region of the end
/// interval: zero unless it has the sign of d0, and at most 3|d0| in size.
/// The cap can only bite where the data turn (d0, d1 of different sign):
/// otherwise the parabola slope d0 + h0/(h0 + h1) (d0 - d1) is below 2|d0|.
fn pchip_end_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
    let s = parabola_end_slope(h0, h1, d0, d1);
    if s.is_nan() {
        s
    } else if sign(s) != sign(d0) {
        0.
    } else if sign(d0) != sign(d1) && s.abs() > (3. * d0).abs() {
        3. * d0
    } else {
        s
    }
}
/// Slope at an end knot of the parabola through three points: the outer
/// interval has width `h0` and difference `d0`, its neighbour `h1` and `d1`.
/// The product form can overflow although the slope is finite; it is then
/// redone as d0 + w (d0 - d1), w = h0/(h0 + h1), with the difference taken
/// from halves.
fn parabola_end_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
    let sum = h0 + h1;
    let slope = ((2. * h0 + h1) * d0 - h0 * d1) / sum;
    if slope.is_finite() || !(sum.is_finite() && d0.is_finite() && d1.is_finite()) {
        return slope;
    }
    d0 + 2. * (h0 / sum) * (0.5 * d0 - 0.5 * d1)
}
/// d - a b with the rounding error of the product recovered by a fused
/// multiply-add: the slope d - c h0 of a parabola cancels almost completely
/// when the curve starts flat.
fn sub_product(d: f64, a: f64, b: f64) -> f64 {
    let product = a * b;
    (d - product) - a.fma(b, -product)
}
/// The parabola through three points as two pieces in the layout of [`Pp`]:
/// Newton form p = y0 + d0 t + c t (t - h0), c = (d1 - d0)/(h0 + h1) the second
/// divided difference (de Boor, A Practical Guide to Splines, rev. ed. 2001,
/// ch. I), so both pieces share the quadratic coefficient c and the cubic one
/// is zero. Slopes at x[0] and x[1]: d0 - c h0 and (h1 d0 + h0 d1)/(h0 + h1).
fn parabola_pieces(x: &[f64], y: &[f64]) -> [[f64; 4]; 2] {
    let (h0, h1) = (x[1] - x[0], x[2] - x[1]);
    let (d0, d1) = ((y[1] - y[0]) / h0, (y[2] - y[1]) / h1);
    let sum = h0 + h1;
    let finite = d0.is_finite() && d1.is_finite() && sum.is_finite();
    let mut curvature = (d1 - d0) / sum;
    let mut middle = (h1 * d0 + h0 * d1) / sum;
    if finite && !curvature.is_finite() {
        curvature = d1 / sum - d0 / sum;
    }
    if finite && !middle.is_finite() {
        middle = d0 * (h1 / sum) + d1 * (h0 / sum);
    }
    let mut start = sub_product(d0, curvature, h0);
    if !start.is_finite() {
        start = parabola_end_slope(h0, h1, d0, d1);
    }
    [[0., curvature, start, y[0]], [0., curvature, middle, y[1]]]
}
/// LAPACK dgtsv: tridiagonal solve with partial pivoting; false when singular.
fn tridiagonal_solve(dl: &mut [f64], d: &mut [f64], du: &mut [f64], b: &mut [f64]) -> bool {
    let n = d.len();
    let mut du2 = vec![0.; n.saturating_sub(2)];
    for i in 0..n - 1 {
        if d[i].abs() >= dl[i].abs() {
            if d[i] == 0. {
                return false;
            }
            let fact = dl[i] / d[i];
            d[i + 1] -= fact * du[i];
            b[i + 1] -= fact * b[i];
        } else {
            let fact = d[i] / dl[i];
            d[i] = dl[i];
            let temp = d[i + 1];
            d[i + 1] = du[i] - fact * temp;
            if i + 2 < n {
                du2[i] = du[i + 1];
                du[i + 1] = -fact * du2[i];
            }
            du[i] = temp;
            let t = b[i];
            b[i] = b[i + 1];
            b[i + 1] = t - fact * b[i + 1];
        }
    }
    if d[n - 1] == 0. {
        return false;
    }
    b[n - 1] /= d[n - 1];
    if n > 1 {
        b[n - 2] = (b[n - 2] - du[n - 2] * b[n - 1]) / d[n - 2];
    }
    for i in (0..n.saturating_sub(2)).rev() {
        b[i] = (b[i] - du[i] * b[i + 1] - du2[i] * b[i + 2]) / d[i];
    }
    true
}
/// Knot slopes of the not-a-knot cubic spline (de Boor, A Practical Guide to
/// Splines, rev. ed. 2001, ch. IV): row k of the tridiagonal system is
/// h[k] s[k-1] + 2(h[k-1] + h[k]) s[k] + h[k-1] s[k+1] = 3(h[k] d[k-1] + h[k-1] d[k]);
/// the end rows equate the third derivatives across x[1] and x[n-2], with
/// s[2] (s[n-3]) eliminated through the neighbouring row. Two points give
/// the line; three points are the parabola of [`parabola_pieces`], not slopes.
fn spline_slopes(x: &[f64], y: &[f64]) -> Result<Vec<f64>> {
    let n = x.len();
    debug_assert!(n != 3, "three points go through parabola_pieces");
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let del: Vec<f64> = (0..n - 1).map(|k| (y[k + 1] - y[k]) / h[k]).collect();
    if n == 2 {
        return Ok(vec![del[0]; 2]);
    }
    let mut sub = vec![0.; n - 1];
    let mut diag = vec![0.; n];
    let mut sup = vec![0.; n - 1];
    let mut rhs = vec![0.; n];
    for k in 1..n - 1 {
        sub[k - 1] = h[k];
        diag[k] = 2. * (h[k - 1] + h[k]);
        sup[k] = h[k - 1];
        rhs[k] = 3. * (h[k] * del[k - 1] + h[k - 1] * del[k]);
    }
    let first = h[0] + h[1];
    diag[0] = h[1];
    sup[0] = first;
    rhs[0] = ((h[0] + 2. * first) * h[1] * del[0] + h[0] * h[0] * del[1]) / first;
    let last = h[n - 2] + h[n - 3];
    diag[n - 1] = h[n - 3];
    sub[n - 2] = last;
    rhs[n - 1] =
        ((h[n - 2] + 2. * last) * h[n - 3] * del[n - 2] + h[n - 2] * h[n - 2] * del[n - 3]) / last;
    if !tridiagonal_solve(&mut sub, &mut diag, &mut sup, &mut rhs) {
        return Err("spline: the end-condition system is singular".into());
    }
    Ok(rhs)
}
/// Not-a-knot spline pieces: the parabola for three points, else Hermite
/// pieces from [`spline_slopes`].
fn spline_pp(x: &[f64], y: &[f64]) -> Result<Pp> {
    if x.len() == 3 {
        Ok(Pp::parabola(x, y))
    } else {
        Ok(Pp::from_slopes(x, y, &spline_slopes(x, y)?))
    }
}
/// Sort (x, columns of v) ascending; reject NaN, repeated and non-finite abscissae.
fn sorted_grid(x: &[f64]) -> Result<Vec<usize>> {
    if x.iter().any(|v| v.is_nan()) {
        return Err("The grid vectors must not contain NaN.".into());
    }
    let mut order: Vec<usize> = (0..x.len()).collect();
    order.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    if order.windows(2).any(|w| x[w[0]] == x[w[1]]) {
        return Err("The grid vectors are not strictly monotonic increasing.".into());
    }
    Ok(order)
}

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Linear,
    Nearest,
    Next,
    Previous,
    Pchip,
    Spline,
    V5Cubic,
}
#[derive(Clone, Copy)]
enum Outside {
    Fill(f64),
    Extrapolate,
}
fn method_of(text: &str) -> Result<Method> {
    Ok(match text.to_ascii_lowercase().as_str() {
        "linear" => Method::Linear,
        "nearest" => Method::Nearest,
        "next" => Method::Next,
        "previous" => Method::Previous,
        "pchip" | "cubic" => Method::Pchip,
        "spline" => Method::Spline,
        "v5cubic" => Method::V5Cubic,
        _ => return Err(format!("Invalid interpolation method '{text}'.")),
    })
}
fn evaluate(
    method: Method,
    pp: Option<&Pp>,
    x: &[f64],
    y: &[f64],
    q: f64,
    outside: Outside,
) -> f64 {
    let n = x.len();
    if q.is_nan() {
        return f64::NAN;
    }
    if q < x[0] || q > x[n - 1] {
        match outside {
            // The reference's 'v5cubic' gives NaN outside the grid even with 'extrap'.
            Outside::Extrapolate if method == Method::V5Cubic => return f64::NAN,
            Outside::Fill(v) => return v,
            Outside::Extrapolate => {
                // next/previous have no neighbour on the far side
                if (method == Method::Next && q > x[n - 1])
                    || (method == Method::Previous && q < x[0])
                {
                    return f64::NAN;
                }
            }
        }
    }
    match method {
        Method::Linear => {
            let k = interval(x, q);
            if q == x[k] {
                y[k]
            } else if q == x[k + 1] {
                y[k + 1]
            } else {
                y[k] + (q - x[k]) * (y[k + 1] - y[k]) / (x[k + 1] - x[k])
            }
        }
        Method::Nearest => {
            if q <= x[0] {
                y[0]
            } else if q >= x[n - 1] {
                y[n - 1]
            } else {
                let k = interval(x, q);
                if q - x[k] < x[k + 1] - q {
                    y[k]
                } else {
                    y[k + 1]
                }
            }
        }
        Method::Next => y[x.partition_point(|&v| v < q).min(n - 1)],
        Method::Previous => y[x.partition_point(|&v| v <= q).saturating_sub(1)],
        Method::Pchip | Method::Spline => pp.map_or(f64::NAN, |p| p.eval(q)),
        Method::V5Cubic => cubic_convolution(x, y, q),
    }
}
/// MATLAB 5 'v5cubic' on an equally spaced grid of at least 3 points: Keys'
/// cubic convolution (a = -1/2) with the end samples extended by
/// y(0) = 3y(1) - 3y(2) + y(3) and y(n+1) = 3y(n) - 3y(n-1) + y(n-2).
fn cubic_convolution(x: &[f64], y: &[f64], q: f64) -> f64 {
    let n = x.len();
    let h = (x[n - 1] - x[0]) / (n - 1) as f64;
    let s = (q - x[0]) / h;
    let k = (s.floor().max(0.) as usize).min(n - 2);
    let t = s - k as f64;
    let at = |i: isize| -> f64 {
        if i < 0 {
            3. * y[0] - 3. * y[1] + y[2]
        } else if i as usize >= n {
            3. * y[n - 1] - 3. * y[n - 2] + y[n - 3]
        } else {
            y[i as usize]
        }
    };
    let k = k as isize;
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((-t3 + 2. * t2 - t) * at(k - 1)
        + (3. * t3 - 5. * t2 + 2.) * at(k)
        + (-3. * t3 + 4. * t2 + t) * at(k + 1)
        + (t3 - t2) * at(k + 2))
}
fn equally_spaced(x: &[f64]) -> bool {
    let n = x.len();
    let h = (x[n - 1] - x[0]) / (n - 1) as f64;
    let tol = 4. * eps_of(x[0].abs().max(x[n - 1].abs()));
    x.windows(2).all(|w| ((w[1] - w[0]) - h).abs() <= tol)
}

impl Engine {
    pub(super) fn polyopt_interp1(&self, args: &[Value]) -> Result<Value> {
        let simple = matches!(args.len(), 3..=5)
            && args[..3].iter().all(|a| matches!(a, Value::Matrix(_)))
            && args
                .get(3)
                .is_none_or(|a| matches!(text_of(a), Some("linear" | "nearest")))
            && args.get(4).is_none_or(|a| matches!(a, Value::Matrix(_)));
        if simple && let Ok(value) = self.interp1_call(args) {
            return Ok(value);
        }
        let numeric = args.iter().take_while(|a| text_of(a).is_none()).count();
        if !(2..=3).contains(&numeric) {
            return Err(if numeric < 2 {
                "Not enough input arguments."
            } else {
                "interp1: expected (x,v,xq[,method][,'extrap'|extrapval]) or (v,xq[,...])"
            }
            .into());
        }
        let mut method = Method::Linear;
        let mut outside = None;
        let rest = &args[numeric..];
        let mut tail = rest;
        if let Some(first) = rest.first().and_then(text_of) {
            if first.eq_ignore_ascii_case("extrap") {
                return Err(if numeric == 2 {
                    "Wrong number of input arguments."
                } else {
                    "An interpolation method must be specified with 'extrap'."
                }
                .into());
            } else {
                method = method_of(first)?;
                tail = &rest[1..];
                if let Some(second) = tail.first() {
                    outside = Some(match text_of(second) {
                        Some(t) if t.eq_ignore_ascii_case("extrap") => Outside::Extrapolate,
                        Some(_) => {
                            return Err(
                                "interp1: the last option must be 'extrap' or a scalar".into()
                            );
                        }
                        None => match second {
                            Value::Matrix(m) if m.len() == 1 => Outside::Fill(m.data()[0]),
                            _ => {
                                return Err(
                                    "interp1 outside-domain fill must be a real double scalar"
                                        .into(),
                                );
                            }
                        },
                    });
                    tail = &tail[1..];
                }
            }
        }
        if !tail.is_empty() {
            return Err("Too many input arguments.".into());
        }
        let double = |value: &Value| {
            match value {
            Value::Matrix(m) => Ok(m.clone()),
            _ => Err("interp1 requires real double coordinates, samples and queries; logical/complex/text/cell/graphics inputs are unsupported".to_string()),
        }
        };
        let (xv, v, q) = if numeric == 3 {
            (
                Some(double(&args[0])?),
                double(&args[1])?,
                double(&args[2])?,
            )
        } else {
            (None, double(&args[0])?, double(&args[1])?)
        };
        let vector_v = is_vector(&v);
        let n = if vector_v { v.len() } else { v.rows() };
        let columns = if vector_v { 1 } else { v.cols() };
        let x: Vec<f64> = match &xv {
            Some(m) => {
                if !is_vector(m) || m.is_empty() || m.data().iter().any(|v| !v.is_finite()) {
                    return Err("interp1: x must be a vector of finite values".into());
                }
                m.data().to_vec()
            }
            None => (1..=n).map(|i| i as f64).collect(),
        };
        if x.len() != n {
            return Err("interp1: x and v must have the same number of samples.".into());
        }
        if n < 2 {
            return Err("The grid vectors must contain at least 2 points.".into());
        }
        if !vector_v && !is_vector(&q) {
            return Err("interp1: xq must be a vector when v is a matrix".into());
        }
        let (rows, cols) = if vector_v {
            (q.rows(), q.cols())
        } else {
            (q.len(), columns)
        };
        self.polyopt_admit(rows * cols + n * columns)?;
        let order = sorted_grid(&x)?;
        let xs: Vec<f64> = order.iter().map(|&i| x[i]).collect();
        // 'spline' and 'pchip' extrapolate unless a fill value is given.
        let outside = outside.unwrap_or(if matches!(method, Method::Pchip | Method::Spline) {
            Outside::Extrapolate
        } else {
            Outside::Fill(f64::NAN)
        });
        // Reference behavior: 'v5cubic' needs an equally spaced grid and falls back to 'spline'.
        if method == Method::V5Cubic && (n < 3 || !equally_spaced(&xs)) {
            method = Method::Spline;
        }
        let mut result = Vec::with_capacity(rows * cols);
        for col in 0..columns {
            self.check_cancel()?;
            let ys: Vec<f64> = order.iter().map(|&i| v.data()[col * n + i]).collect();
            let pp = match method {
                Method::Pchip => Some(Pp::from_slopes(&xs, &ys, &pchip_slopes(&xs, &ys))),
                Method::Spline => Some(spline_pp(&xs, &ys)?),
                _ => None,
            };
            for (i, &qv) in q.data().iter().enumerate() {
                if i % 65536 == 0 {
                    self.check_cancel()?;
                }
                result.push(evaluate(method, pp.as_ref(), &xs, &ys, qv, outside));
            }
        }
        matrix_value(rows, cols, result)
    }

    pub(super) fn polyopt_spline(&self, name: &str, args: &[Value]) -> Result<Value> {
        match args.len() {
            0 | 1 => return Err("Not enough input arguments.".into()),
            2 => {
                return Err(format!(
                    "{name}(x,y) would return a piecewise polynomial structure, which is not supported; call {name}(x,y,xx)"
                ));
            }
            3 => {}
            _ => return Err("Too many input arguments.".into()),
        }
        let x = real_matrix(&args[0], name)?;
        let y = real_matrix(&args[1], name)?;
        let q = real_matrix(&args[2], name)?;
        let n = x.len();
        if n < 2 {
            return Err("There should be at least two data points.".into());
        }
        if !is_vector(&x) || !is_vector(&y) {
            return Err(format!(
                "{name}: x and y must be vectors; matrix y is not supported yet"
            ));
        }
        if name == "spline" && y.len() == n + 2 {
            return Err(
                "spline: clamped end slopes (y with two extra values) are not supported yet".into(),
            );
        }
        if y.len() != n {
            return Err("Abscissae and ordinate vectors should be of the same length.".into());
        }
        if x.data().iter().any(|v| v.is_infinite()) {
            return Err(format!("{name}: x must contain finite values"));
        }
        // Sorted copies, slopes and coefficients, plus the result.
        self.polyopt_admit(q.len() + 7 * n)?;
        let order = sorted_grid(x.data())?;
        let xs: Vec<f64> = order.iter().map(|&i| x.data()[i]).collect();
        let ys: Vec<f64> = order.iter().map(|&i| y.data()[i]).collect();
        let pp = if name == "spline" {
            spline_pp(&xs, &ys)?
        } else {
            Pp::from_slopes(&xs, &ys, &pchip_slopes(&xs, &ys))
        };
        let mut out = Vec::with_capacity(q.len());
        for (i, &t) in q.data().iter().enumerate() {
            if i % 65536 == 0 {
                self.check_cancel()?;
            }
            out.push(pp.eval(t));
        }
        matrix_value(q.rows(), q.cols(), out)
    }

    pub(super) fn polyopt_interp2(&self, args: &[Value]) -> Result<Value> {
        let numeric = args.iter().take_while(|a| text_of(a).is_none()).count();
        if numeric != 3 && numeric != 5 {
            return Err(if numeric < 3 {
                "Not enough input arguments."
            } else {
                "interp2: expected (V,Xq,Yq[,method][,extrapval]) or (X,Y,V,Xq,Yq[,...])"
            }
            .into());
        }
        let mut nearest = false;
        let mut fill = f64::NAN;
        let mut tail = &args[numeric..];
        if let Some(first) = tail.first() {
            match text_of(first).map(str::to_ascii_lowercase).as_deref() {
                Some("linear") => {}
                Some("nearest") => nearest = true,
                Some("cubic" | "spline" | "makima" | "pchip") => {
                    return Err(
                        "interp2 supports only the 'linear' and 'nearest' methods for now".into(),
                    );
                }
                _ => return Err("interp2: invalid interpolation method".into()),
            }
            tail = &tail[1..];
            if let Some(extra) = tail.first() {
                fill = scalar_of(extra, "interp2", "the extrapolation value")?;
                tail = &tail[1..];
            }
        }
        if !tail.is_empty() {
            return Err("Too many input arguments.".into());
        }
        let off = numeric - 3;
        let v = real_matrix(&args[off], "interp2")?;
        let xq = real_matrix(&args[off + 1], "interp2")?;
        let yq = real_matrix(&args[off + 2], "interp2")?;
        let (m, n) = (v.rows(), v.cols());
        if m < 2 || n < 2 {
            return Err("interp2: V must be at least 2-by-2.".into());
        }
        let axis = |value: &Value, len: usize, along_row: bool| -> Result<Vec<f64>> {
            let a = real_matrix(value, "interp2")?;
            if is_vector(&a) && a.len() == len {
                return Ok(a.data().to_vec());
            }
            if a.rows() == m && a.cols() == n {
                return Ok(if along_row {
                    (0..n).map(|c| a.data()[c * m]).collect()
                } else {
                    a.data()[..m].to_vec()
                });
            }
            Err("interp2: X and Y must be vectors matching the columns/rows of V, or meshgrid matrices".into())
        };
        let (xg, yg) = if off == 2 {
            (axis(&args[0], n, true)?, axis(&args[1], m, false)?)
        } else {
            (
                (1..=n).map(|i| i as f64).collect(),
                (1..=m).map(|i| i as f64).collect(),
            )
        };
        // Strictly monotonic axes; decreasing axes are read back to front.
        let direction = |g: &[f64]| -> Result<Vec<usize>> {
            let up = g.windows(2).all(|w| w[1] > w[0]);
            let down = g.windows(2).all(|w| w[1] < w[0]);
            if up {
                Ok((0..g.len()).collect())
            } else if down {
                Ok((0..g.len()).rev().collect())
            } else {
                Err("interp2: X and Y must be strictly monotonic.".into())
            }
        };
        let (ox, oy) = (direction(&xg)?, direction(&yg)?);
        let xs: Vec<f64> = ox.iter().map(|&i| xg[i]).collect();
        let ys: Vec<f64> = oy.iter().map(|&i| yg[i]).collect();
        let at = |r: usize, c: usize| v.data()[ox[c] * m + oy[r]];
        let grid = (xq.rows() == 1
            && yq.cols() == 1
            && xq.cols() > 0
            && yq.rows() > 0
            && xq.len() * yq.len() != 1)
            || (xq.cols() == 1
                && yq.rows() == 1
                && xq.len() * yq.len() != 1
                && !xq.is_empty()
                && !yq.is_empty());
        let (rows, cols) = if grid {
            (yq.len(), xq.len())
        } else if xq.rows() == yq.rows() && xq.cols() == yq.cols() {
            (xq.rows(), xq.cols())
        } else {
            return Err(
                "interp2: Xq and Yq must be the same size, or a row and a column vector".into(),
            );
        };
        self.polyopt_admit(rows * cols)?;
        let mut out = Vec::with_capacity(rows * cols);
        for idx in 0..rows * cols {
            if idx % 65536 == 0 {
                self.check_cancel()?;
            }
            let (qx, qy) = if grid {
                (xq.data()[idx / rows], yq.data()[idx % rows])
            } else {
                (xq.data()[idx], yq.data()[idx])
            };
            if qx.is_nan()
                || qy.is_nan()
                || qx < xs[0]
                || qx > xs[n - 1]
                || qy < ys[0]
                || qy > ys[m - 1]
            {
                out.push(fill);
                continue;
            }
            let (i, j) = (interval(&xs, qx), interval(&ys, qy));
            out.push(if nearest {
                let ci = if qx - xs[i] < xs[i + 1] - qx {
                    i
                } else {
                    i + 1
                };
                let rj = if qy - ys[j] < ys[j + 1] - qy {
                    j
                } else {
                    j + 1
                };
                at(rj, ci)
            } else {
                let tx = (qx - xs[i]) / (xs[i + 1] - xs[i]);
                let ty = (qy - ys[j]) / (ys[j + 1] - ys[j]);
                let (v00, v01, v10, v11) = (at(j, i), at(j, i + 1), at(j + 1, i), at(j + 1, i + 1));
                let lo = if tx == 0. {
                    v00
                } else {
                    v00 + tx * (v01 - v00)
                };
                let hi = if tx == 0. {
                    v10
                } else {
                    v10 + tx * (v11 - v10)
                };
                if ty == 0. { lo } else { lo + ty * (hi - lo) }
            });
        }
        matrix_value(rows, cols, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_a_knot_slopes_reproduce_a_cubic() {
        let f = |x: f64| ((2. * x - 1.) * x + 3.) * x - 4.;
        let df = |x: f64| (6. * x - 2.) * x + 3.;
        let x = [-1., 0., 0.25, 2., 2.5, 4.];
        let y: Vec<f64> = x.iter().map(|&v| f(v)).collect();
        let s = spline_slopes(&x, &y).unwrap();
        for (got, &x) in s.iter().zip(&x) {
            assert!((got - df(x)).abs() < 1e-11, "{x}: {got} vs {}", df(x));
        }
    }

    fn close(got: f64, want: f64) {
        assert!(((got - want) / want).abs() < 4e-16, "{got:e} vs {want:e}");
    }

    #[test]
    fn three_point_spline_is_an_exact_parabola_at_any_scale() {
        // Tiny spacings: the Hermite form would invent a cubic term of Inf.
        let (x, y) = ([0., 1e-201, 1e-200], [0., 1e-202, 1e-200]);
        let pp = spline_pp(&x, &y).unwrap();
        assert!(pp.c.iter().all(|c| c[0] == 0.));
        close(pp.eval(5e-202), 2.5e-203);
        // Huge slopes: the end slope (3 d0 - d1)/2 overflows in the product form.
        let pp = spline_pp(&[0., 1., 2.], &[0., 1e308, 1e308]).unwrap();
        close(pp.eval(0.5), 6.25e307);
        close(pp.eval(1.5), 1.125e308);
        close(parabola_end_slope(1., 1., 1e308, 0.), 1.5e308);
        close(parabola_end_slope(1., 3., 1e308, -1e308), 1.5e308);
        assert_eq!(parabola_end_slope(1., 1., 1.7e308, -1.7e308), f64::INFINITY);
        // Ordinary data keep the product form: the parabola through (0,1), (1,0), (3,4).
        let pp = spline_pp(&[0., 1., 3.], &[1., 0., 4.]).unwrap();
        for t in [-1., 0.5, 2., 4.] {
            close(pp.eval(t) + 1., (t - 1.) * (t - 1.) + 1.);
        }
    }

    #[test]
    fn pchip_end_slope_survives_overflowing_products() {
        // The end slope (3 d0 - d1)/2 is finite, but its product form is not;
        // the cap 3 d0 overflows too, so it does not bite.
        let s = pchip_slopes(&[0., 1., 2.], &[0., 1e308, 1e308]);
        close(s[0], 1.5e308);
        assert_eq!(s[1..], [0., 0.]);
    }

    #[test]
    fn pchip_slopes_harmonic_mean_and_ends() {
        let s = pchip_slopes(&[0., 1., 3., 4.], &[0., 1., 2., 4.]);
        for (got, want) in s.iter().zip([7. / 6., 9. / 13., 6. / 7., 2.5]) {
            assert!((got - want).abs() < 1e-15, "{got} vs {want}");
        }
        assert_eq!(
            pchip_slopes(&[0., 1., 2., 3.], &[0., 1., 1., 5.]),
            [1.5, 0., 0., 6.]
        );
        assert_eq!(pchip_slopes(&[0., 1., 2.], &[0., 1., -5.])[0], 3.);
        assert_eq!(pchip_slopes(&[0., 1., 2.], &[0., 1., 6.])[0], 0.);
        assert_eq!(pchip_slopes(&[0., 2.], &[1., 5.]), [2., 2.]);
    }
}
