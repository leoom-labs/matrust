//! fftn ifftn convn interp3 interpn griddata griddatan delaunay delaunayn
//! convhull convhulln dsearchn tsearchn voronoin quad2d quadl quadv freqspace
//! iqr: their two-dimensional (and one-dimensional) forms. The
//! corpus is tests/fixtures/fn/sci2-expected.txt. MatRust has no N-D arrays, so
//! three-dimensional data is rejected explicitly.
use super::{Engine, Execution, Fma, Value};
use crate::{Array, Complex64, ComplexMatrix, Matrix, NumericClass, Result, TypedArray};
use std::cmp::Ordering;

type C = Complex64;

const NAMES: &[&str] = &[
    "fftn",
    "ifftn",
    "convn",
    "interp3",
    "interpn",
    "griddata",
    "griddatan",
    "delaunay",
    "delaunayn",
    "convhull",
    "convhulln",
    "dsearchn",
    "tsearchn",
    "voronoin",
    "quad2d",
    "quadl",
    "quadv",
    "freqspace",
    "iqr",
];

/// Scattered points accepted by the triangulation functions.
const MAX_POINTS: usize = 200_000;
/// Query points times data points for the O(m*n) searches.
const MAX_PAIRS: usize = 64 * 1024 * 1024;
const NOT_ENOUGH: &str = "Not enough input arguments.";
const NO_ND: &str = "MatRust has no N-D arrays";
const EMPTY_DATA: &str = "Interpolation data must not be empty.";

/// Functions with more than one output.
pub(super) fn sc2_has_outputs(name: &str) -> bool {
    matches!(
        name,
        "griddata"
            | "convhull"
            | "convhulln"
            | "dsearchn"
            | "tsearchn"
            | "voronoin"
            | "quad2d"
            | "quadl"
            | "quadv"
            | "freqspace"
    )
}

fn sc2_max_outputs(name: &str) -> usize {
    match name {
        "griddata" => 3,
        n if sc2_has_outputs(n) => 2,
        _ => 1,
    }
}

fn sc2_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}

fn sc2_num(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

fn sc2_class(value: &Value) -> Option<NumericClass> {
    match value {
        Value::Container(Array::Typed(t)) => Some(t.class()),
        _ => None,
    }
}

fn sc2_single(value: Value) -> Result<Value> {
    match value {
        Value::Matrix(m) => Ok(Value::Container(Array::Typed(TypedArray::from_matrix(
            NumericClass::Single,
            &m,
        )?))),
        Value::Complex(_) => Err("Complex single values are not supported yet".into()),
        other => Ok(other),
    }
}

fn sc2_is_empty(value: &Value) -> bool {
    match value {
        Value::Matrix(m) => m.is_empty(),
        _ => false,
    }
}

fn sc2_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        Value::Logical(m) if m.len() == 1 => Some(f64::from(m.data()[0])),
        Value::Container(Array::Typed(t)) if t.len() == 1 => Some(t.real(0)),
        _ => None,
    }
}

/// Real column-major data with its shape.
#[derive(Clone)]
struct Grid {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl Grid {
    fn at(&self, r: usize, c: usize) -> f64 {
        self.data[c * self.rows + r]
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn is_vector(&self) -> bool {
        self.rows == 1 || self.cols == 1
    }
    fn value(self) -> Result<Value> {
        sc2_num(self.rows, self.cols, self.data)
    }
}

// ---------------------------------------------------------------------------
// Small Fourier kernels. Lengths 1 to 4 use FFTW's exact-constant codelets so
// that rounding (and the sign of zero parts) matches the reference; longer lengths
// use the engine's fft.

const KP866: f64 = 0.866_025_403_784_438_6;
const KP1_732: f64 = 1.732_050_807_568_877_2;

fn sc2_swap(z: C) -> C {
    C::new(z.im, z.re)
}

/// Forward complex DFT of length 1 to 4 (FFTW n1_N).
fn sc2_c2c_forward(x: &[C]) -> Vec<C> {
    match x.len() {
        2 => vec![x[0] + x[1], x[0] - x[1]],
        3 => {
            let t4 = x[1].re + x[2].re;
            let t9 = KP866 * (x[2].re - x[1].re);
            let t8 = KP866 * (x[1].im - x[2].im);
            let tb = x[1].im + x[2].im;
            let t5 = x[0].re - 0.5 * t4;
            let tc = x[0].im - 0.5 * tb;
            vec![
                C::new(x[0].re + t4, x[0].im + tb),
                C::new(t5 + t8, t9 + tc),
                C::new(t5 - t8, tc - t9),
            ]
        }
        4 => {
            let t3 = x[0].re + x[2].re;
            let tb = x[0].re - x[2].re;
            let t9 = x[0].im + x[2].im;
            let tj = x[0].im - x[2].im;
            let t6 = x[1].re + x[3].re;
            let tc = x[1].re - x[3].re;
            let ta = x[1].im + x[3].im;
            let ti = x[1].im - x[3].im;
            vec![
                C::new(t3 + t6, t9 + ta),
                C::new(tb + ti, tj - tc),
                C::new(t3 - t6, t9 - ta),
                C::new(tb - ti, tc + tj),
            ]
        }
        _ => x.to_vec(),
    }
}

fn sc2_c2c(x: &[C], forward: bool) -> Vec<C> {
    if forward {
        sc2_c2c_forward(x)
    } else {
        let swapped: Vec<C> = x.iter().map(|z| sc2_swap(*z)).collect();
        sc2_c2c_forward(&swapped)
            .into_iter()
            .map(sc2_swap)
            .collect()
    }
}

/// Forward real-to-complex DFT, the first n/2+1 outputs (FFTW r2cf_N).
fn sc2_r2c(x: &[f64]) -> Vec<C> {
    match x.len() {
        1 => vec![C::new(x[0], 0.)],
        2 => vec![C::new(x[0] + x[1], 0.), C::new(x[0] - x[1], 0.)],
        3 => {
            let t4 = x[1] + x[2];
            vec![
                C::new(x[0] + t4, 0.),
                C::new(x[0] - 0.5 * t4, KP866 * (x[2] - x[1])),
            ]
        }
        _ => {
            let t5 = x[0] + x[2];
            let t6 = x[1] + x[3];
            vec![
                C::new(t5 + t6, 0.),
                C::new(x[0] - x[2], x[3] - x[1]),
                C::new(t5 - t6, 0.),
            ]
        }
    }
}

/// Backward complex-to-real DFT from the first n/2+1 values (FFTW r2cb_N).
fn sc2_c2r(h: &[C], n: usize) -> Vec<f64> {
    match n {
        1 => vec![h[0].re],
        2 => vec![h[0].re + h[1].re, h[0].re - h[1].re],
        3 => {
            let t3 = h[0].re - h[1].re;
            let t5 = KP1_732 * h[1].im;
            vec![h[0].re + 2. * h[1].re, t3 - t5, t3 + t5]
        }
        _ => {
            let t3 = h[0].re + h[2].re;
            let t6 = h[0].re - h[2].re;
            vec![
                t3 + 2. * h[1].re,
                t6 - 2. * h[1].im,
                t3 - 2. * h[1].re,
                t6 + 2. * h[1].im,
            ]
        }
    }
}

/// `data` (rows x cols, column-major) is Hermitian in two dimensions.
fn sc2_hermitian(data: &[C], rows: usize, cols: usize) -> bool {
    (0..cols).all(|c| {
        (0..rows).all(|r| {
            let mirror = data[((cols - c) % cols) * rows + (rows - r) % rows];
            data[c * rows + r] == mirror.conj()
        })
    })
}

/// Two-dimensional transforms with the small kernels (both lengths <= 4).
/// With `symmetric`, an inverse transform reads only the first half of the
/// spectrum along dimension 1 (FFTW's c2r) and rebuilds the rest by Hermitian
/// symmetry, whatever the data holds.
fn sc2_small_fft(
    data: &[C],
    real: bool,
    rows: usize,
    cols: usize,
    inverse: bool,
    symmetric: bool,
) -> Vec<C> {
    let column = |c: usize| -> Vec<C> { data[c * rows..(c + 1) * rows].to_vec() };
    if !inverse && real {
        // r2c along dimension 1, c2c along dimension 2 for the first half,
        // the rest by Hermitian symmetry (FFTW's rank-2 r2c).
        let half = rows / 2 + 1;
        let mut out = vec![C::new(0., 0.); rows * cols];
        let mut partial = vec![C::new(0., 0.); half * cols];
        for c in 0..cols {
            let x: Vec<f64> = column(c).iter().map(|z| z.re).collect();
            for (r, z) in sc2_r2c(&x).into_iter().enumerate() {
                partial[c * half + r] = z;
            }
        }
        for r in 0..half {
            let lane: Vec<C> = (0..cols).map(|c| partial[c * half + r]).collect();
            for (c, z) in sc2_c2c(&lane, true).into_iter().enumerate() {
                out[c * rows + r] = z;
            }
        }
        for r in half..rows {
            for c in 0..cols {
                out[c * rows + r] = out[((cols - c) % cols) * rows + rows - r].conj();
            }
        }
        return out;
    }
    if inverse && (symmetric || sc2_hermitian(data, rows, cols)) {
        // c2c along dimension 2 for the first half, c2r along dimension 1.
        let half = rows / 2 + 1;
        let mut partial = vec![C::new(0., 0.); half * cols];
        for r in 0..half {
            let lane: Vec<C> = (0..cols).map(|c| data[c * rows + r]).collect();
            for (c, z) in sc2_c2c(&lane, false).into_iter().enumerate() {
                partial[c * half + r] = z;
            }
        }
        let mut out = Vec::with_capacity(rows * cols);
        for c in 0..cols {
            let x = sc2_c2r(&partial[c * half..(c + 1) * half], rows);
            out.extend(x.into_iter().map(|v| C::new(v, 0.)));
        }
        return out;
    }
    let mut out = data.to_vec();
    for c in 0..cols {
        let lane = sc2_c2c(&out[c * rows..(c + 1) * rows], !inverse);
        out[c * rows..(c + 1) * rows].copy_from_slice(&lane);
    }
    for r in 0..rows {
        let lane: Vec<C> = (0..cols).map(|c| out[c * rows + r]).collect();
        for (c, z) in sc2_c2c(&lane, !inverse).into_iter().enumerate() {
            out[c * rows + r] = z;
        }
    }
    out
}

/// The spectrum a symmetric inverse transform uses: the first rows/2 + 1 rows
/// of `data` (rows x cols), the rest the conjugate mirror of those.
fn sc2_conjugate_half(data: &[C], rows: usize, cols: usize) -> Vec<C> {
    let mut out = data.to_vec();
    for c in 0..cols {
        for r in rows / 2 + 1..rows {
            out[c * rows + r] = data[((cols - c) % cols) * rows + rows - r].conj();
        }
    }
    out
}

/// Pads with zeros or truncates `data` (rows x cols) to m x n.
fn sc2_resize<T: Copy>(data: &[T], rows: usize, m: usize, n: usize, zero: T) -> Vec<T> {
    let cols = data.len().checked_div(rows).unwrap_or(0);
    let mut out = vec![zero; m * n];
    for c in 0..n.min(cols) {
        for r in 0..m.min(rows) {
            out[c * m + r] = data[c * rows + r];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// One-dimensional kernels for gridded interpolation.

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Linear,
    Nearest,
    Cubic,
    Spline,
    Pchip,
}

fn sc2_method(text: &str) -> Option<Method> {
    match text.to_ascii_lowercase().as_str() {
        "linear" | "*linear" => Some(Method::Linear),
        "nearest" | "*nearest" => Some(Method::Nearest),
        "cubic" | "*cubic" => Some(Method::Cubic),
        "spline" | "*spline" => Some(Method::Spline),
        "pchip" | "*pchip" => Some(Method::Pchip),
        _ => None,
    }
}

/// Interval index k with x[k] <= t <= x[k+1] (x ascending, t inside).
fn sc2_interval(x: &[f64], t: f64) -> usize {
    let n = x.len();
    if n < 2 {
        return 0;
    }
    let k = x.partition_point(|v| *v <= t);
    k.clamp(1, n - 1) - 1
}

/// Interval widths and divided differences of the data.
fn sc2_differences(engine: &Engine, x: &[f64], y: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
    let mut h = Vec::with_capacity(x.len() - 1);
    let mut del = Vec::with_capacity(x.len() - 1);
    for k in 0..x.len() - 1 {
        engine.sc2_poll(k)?;
        h.push(x[k + 1] - x[k]);
        del.push((y[k + 1] - y[k]) / h[k]);
    }
    Ok((h, del))
}

/// Slope at an end knot of the parabola through three points: the outer
/// interval has width `h0` and difference `d0`, its neighbour `h1` and `d1`.
/// The product form can overflow although the slope is finite; it is then
/// redone as d0 + w (d0 - d1), w = h0/(h0 + h1), with the difference taken
/// from halves.
fn sc2_parabola_end_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
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
fn sc2_sub_product(d: f64, a: f64, b: f64) -> f64 {
    let product = a * b;
    (d - product) - a.fma(b, -product)
}

/// The parabola through three points as two cubic pieces whose cubic
/// coefficient is exactly zero: Newton form p = y0 + d0 t + c t (t - h0) with
/// c = (d1 - d0)/(h0 + h1) the second divided difference (de Boor, A Practical
/// Guide to Splines, rev. ed. 2001, ch. I), so both pieces share the quadratic
/// coefficient c. Slopes at x[0] and x[1]: d0 - c h0 and
/// (h1 d0 + h0 d1)/(h0 + h1).
fn sc2_parabola_coefs(h: &[f64], del: &[f64], y: &[f64]) -> Vec<[f64; 4]> {
    let (h0, h1, d0, d1) = (h[0], h[1], del[0], del[1]);
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
    let mut start = sc2_sub_product(d0, curvature, h0);
    if !start.is_finite() {
        start = sc2_parabola_end_slope(h0, h1, d0, d1);
    }
    vec![[0., curvature, start, y[0]], [0., curvature, middle, y[1]]]
}

/// Not-a-knot cubic spline pieces (de Boor, A Practical Guide to Splines,
/// rev. ed. 2001, ch. IV). The knot slopes solve the tridiagonal system
/// h[k] s[k-1] + 2(h[k-1] + h[k]) s[k] + h[k-1] s[k+1] = 3(h[k] d[k-1] + h[k-1] d[k]),
/// closed by equal third derivatives across x[1] and x[n-2] (s[2] and s[n-3]
/// eliminated through the neighbouring rows). Two points give the line,
/// three the parabola through them.
fn sc2_spline_coefs(engine: &Engine, x: &[f64], y: &[f64]) -> Result<Vec<[f64; 4]>> {
    let n = x.len();
    let (h, del) = sc2_differences(engine, x, y)?;
    let s = match n {
        2 => vec![del[0]; 2],
        3 => return Ok(sc2_parabola_coefs(&h, &del, y)),
        _ => {
            // Row k holds sub[k] s[k-1] + diag[k] s[k] + sup[k] s[k+1].
            let mut sub = vec![0.; n];
            let mut diag = vec![0.; n];
            let mut sup = vec![0.; n];
            let mut rhs = vec![0.; n];
            for k in 1..n - 1 {
                engine.sc2_poll(k)?;
                sub[k] = h[k];
                diag[k] = 2. * (h[k - 1] + h[k]);
                sup[k] = h[k - 1];
                rhs[k] = 3. * (h[k] * del[k - 1] + h[k - 1] * del[k]);
            }
            let first = h[0] + h[1];
            diag[0] = h[1];
            sup[0] = first;
            rhs[0] = ((h[0] + 2. * first) * h[1] * del[0] + h[0] * h[0] * del[1]) / first;
            let last = h[n - 2] + h[n - 3];
            sub[n - 1] = last;
            diag[n - 1] = h[n - 3];
            rhs[n - 1] = ((h[n - 2] + 2. * last) * h[n - 3] * del[n - 2]
                + h[n - 2] * h[n - 2] * del[n - 3])
                / last;
            sc2_tridiagonal(engine, &sub, &diag, &sup, &rhs)?
        }
    };
    sc2_hermite_coefs(engine, &h, &del, &s, y)
}

fn sc2_tridiagonal(
    engine: &Engine,
    sub: &[f64],
    diag: &[f64],
    sup: &[f64],
    rhs: &[f64],
) -> Result<Vec<f64>> {
    let n = diag.len();
    let mut d = diag.to_vec();
    let mut r = rhs.to_vec();
    for k in 1..n {
        engine.sc2_poll(k)?;
        let m = sub[k] / d[k - 1];
        d[k] -= m * sup[k - 1];
        r[k] -= m * r[k - 1];
    }
    let mut x = vec![0.; n];
    x[n - 1] = r[n - 1] / d[n - 1];
    for k in (0..n - 1).rev() {
        engine.sc2_poll(k)?;
        x[k] = (r[k] - sup[k] * x[k + 1]) / d[k];
    }
    Ok(x)
}

/// Cubic Hermite pieces from slopes s.
fn sc2_hermite_coefs(
    engine: &Engine,
    h: &[f64],
    del: &[f64],
    s: &[f64],
    y: &[f64],
) -> Result<Vec<[f64; 4]>> {
    (0..h.len())
        .map(|k| {
            engine.sc2_poll(k)?;
            let dzzdx = (del[k] - s[k]) / h[k];
            let dzdxdx = (s[k + 1] - del[k]) / h[k];
            Ok([(dzdxdx - dzzdx) / h[k], 2. * dzzdx - dzdxdx, s[k], y[k]])
        })
        .collect()
}

fn sc2_sign(v: f64) -> f64 {
    if v > 0. {
        1.
    } else if v < 0. {
        -1.
    } else {
        0.
    }
}

/// Interior pchip slope between the secants `dl` (width `hl`) and `dr` (`hr`):
/// zero where they differ in sign or one is zero, else the weighted harmonic
/// mean 1/s = a/dl + (1 - a)/dr, a = (hl + 2 hr)/(3 (hl + hr)) (Fritsch &
/// Butland, SIAM J. Sci. Stat. Comput. 5, 1984), evaluated as
/// m / (a m/|dl| + (1 - a) m/|dr|) with m = min(|dl|, |dr|): the denominator
/// lies in [1/3, 1], so |s| <= 3 m and nothing overflows. NaN data give NaN.
fn sc2_harmonic_slope(hl: f64, hr: f64, dl: f64, dr: f64) -> f64 {
    if dl.is_nan() || dr.is_nan() {
        return f64::NAN;
    }
    if sc2_sign(dl) * sc2_sign(dr) <= 0. {
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
    sc2_sign(dl) * (m / (a * (m / dl.abs()) + (1. - a) * (m / dr.abs())))
}

/// End pchip slope: the three-point parabola slope, zero unless it has the
/// sign of d0 and at most 3|d0| where the data turn (Fritsch & Carlson, SIAM
/// J. Numer. Anal. 17, 1980, sec. 4 monotonicity region).
fn sc2_pchip_end_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
    let s = sc2_parabola_end_slope(h0, h1, d0, d1);
    if s.is_nan() {
        s
    } else if sc2_sign(s) != sc2_sign(d0) {
        0.
    } else if sc2_sign(d0) != sc2_sign(d1) && s.abs() > (3. * d0).abs() {
        3. * d0
    } else {
        s
    }
}

/// Shape-preserving piecewise cubic Hermite pieces (Fritsch & Carlson 1980;
/// Fritsch & Butland 1984): harmonic-mean interior slopes, capped
/// three-point end slopes; two points give the line.
fn sc2_pchip_coefs(engine: &Engine, x: &[f64], y: &[f64]) -> Result<Vec<[f64; 4]>> {
    let n = x.len();
    let (h, del) = sc2_differences(engine, x, y)?;
    let mut s = vec![del[0]; n];
    if n > 2 {
        for k in 1..n - 1 {
            engine.sc2_poll(k)?;
            s[k] = sc2_harmonic_slope(h[k - 1], h[k], del[k - 1], del[k]);
        }
        s[0] = sc2_pchip_end_slope(h[0], h[1], del[0], del[1]);
        s[n - 1] = sc2_pchip_end_slope(h[n - 2], h[n - 3], del[n - 2], del[n - 3]);
    }
    sc2_hermite_coefs(engine, &h, &del, &s, y)
}

fn sc2_eval_piece(coefs: &[[f64; 4]], x: &[f64], t: f64) -> f64 {
    let k = sc2_interval(x, t);
    let [a, b, c, d] = coefs[k];
    let s = t - x[k];
    ((a * s + b) * s + c) * s + d
}

/// Weights of the points of a 1-D grid that one coordinate `t` uses.
fn sc2_weights(x: &[f64], t: f64, method: Method, uniform: bool) -> Vec<(usize, f64)> {
    let n = x.len();
    if n == 1 {
        return vec![(0, 1.)];
    }
    let k = sc2_interval(x, t);
    let s = (t - x[k]) / (x[k + 1] - x[k]);
    match method {
        Method::Nearest => {
            let pick = if t - x[k] < x[k + 1] - t { k } else { k + 1 };
            vec![(pick, 1.)]
        }
        Method::Cubic if uniform && n >= 3 => {
            // Keys cubic convolution with the end conditions of interp2.
            let w = [
                ((2. - s) * s - 1.) * s / 2.,
                ((3. * s - 5.) * s * s + 2.) / 2.,
                ((4. - 3. * s) * s + 1.) * s / 2.,
                (s - 1.) * s * s / 2.,
            ];
            let mut out = Vec::new();
            for (j, wj) in w.iter().enumerate() {
                let i = k as isize + j as isize - 1;
                if i < 0 {
                    // f(-1) = 3 f(0) - 3 f(1) + f(2)
                    out.extend([(0, 3. * wj), (1, -3. * wj), (2, *wj)]);
                } else if i as usize >= n {
                    out.extend([(n - 1, 3. * wj), (n - 2, -3. * wj), (n - 3, *wj)]);
                } else {
                    out.push((i as usize, *wj));
                }
            }
            out
        }
        _ => vec![(k, 1. - s), (k + 1, s)],
    }
}

fn sc2_uniform(x: &[f64]) -> bool {
    if x.len() < 3 {
        return true;
    }
    let h = x[1] - x[0];
    x.windows(2)
        .all(|w| ((w[1] - w[0]) - h).abs() <= 1e-10 * h.abs().max(x[x.len() - 1].abs()))
}

/// Interpolates 1-D data along a grid with any method.
fn sc2_interp_line(engine: &Engine, x: &[f64], y: &[f64], t: f64, method: Method) -> Result<f64> {
    engine.check_cancel()?;
    Ok(match method {
        Method::Spline if x.len() >= 2 => sc2_eval_piece(&sc2_spline_coefs(engine, x, y)?, x, t),
        Method::Pchip if x.len() >= 2 => sc2_eval_piece(&sc2_pchip_coefs(engine, x, y)?, x, t),
        Method::Cubic if !sc2_uniform(x) && x.len() >= 2 => {
            sc2_eval_piece(&sc2_spline_coefs(engine, x, y)?, x, t)
        }
        _ => sc2_weights(x, t, method, true)
            .iter()
            .map(|(i, w)| w * y[*i])
            .sum(),
    })
}

// ---------------------------------------------------------------------------
// Percentiles for iqr (Statistics Toolbox prctile, midpoint rule).

fn sc2_quartile_range(values: &mut Vec<f64>) -> f64 {
    values.retain(|v| !v.is_nan());
    let n = values.len();
    if n == 0 {
        return f64::NAN;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| -> f64 {
        // q = [0 100*(0.5:(n-0.5))./n 100], xx = [x(1) x x(n)], interp1q.
        let mut q = Vec::with_capacity(n + 2);
        q.push(0.);
        q.extend((0..n).map(|i| 100. * (i as f64 + 0.5) / n as f64));
        q.push(100.);
        let mut xx = Vec::with_capacity(n + 2);
        xx.push(values[0]);
        xx.extend(values.iter().copied());
        xx.push(values[n - 1]);
        let k = q.partition_point(|v| *v <= p).clamp(1, n + 1) - 1;
        let t = (p - q[k]) / (q[k + 1] - q[k]);
        if t == 0. {
            xx[k]
        } else {
            xx[k] + t * (xx[k + 1] - xx[k])
        }
    };
    at(75.) - at(25.)
}

impl Engine {
    /// Single-output calls; `None` when `name` is not one of these functions.
    pub(super) fn sc2_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        Ok(Some(self.sc2_outputs(name, args, 1, out)?.remove(0)))
    }

    /// Every form; `count` outputs.
    pub(super) fn sc2_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if count > sc2_max_outputs(name) {
            return Err("Too many output arguments.".into());
        }
        let mut values = match name {
            "fftn" | "ifftn" => vec![self.sc2_fftn(name, args, out)?],
            "convn" => vec![self.sc2_convn(args, out)?],
            "interp3" => vec![self.sc2_interp3(args)?],
            "interpn" => vec![self.sc2_interpn(args)?],
            "griddata" => self.sc2_griddata(args, count, out)?,
            "griddatan" => vec![self.sc2_griddatan(args)?],
            "delaunay" => vec![self.sc2_delaunay(args)?],
            "delaunayn" => vec![self.sc2_delaunayn(args)?],
            "convhull" => self.sc2_convhull(args)?,
            "convhulln" => self.sc2_convhulln(args)?,
            "dsearchn" => self.sc2_dsearchn(args)?,
            "tsearchn" => self.sc2_tsearchn(args)?,
            "voronoin" => self.sc2_voronoin(args)?,
            "quad2d" => self.sc2_quad2d(args, out)?,
            "quadl" => self.sc2_quadl(args, out)?,
            "quadv" => self.sc2_quadv(args, out)?,
            "freqspace" => self.sc2_freqspace(args, count)?,
            "iqr" => vec![self.sc2_iqr(args)?],
            _ => unreachable!("checked by NAMES"),
        };
        values.truncate(count.max(1));
        self.check_cancel()?;
        Ok(values)
    }

    /// Real double data of a numeric, logical or character argument.
    fn sc2_grid(&self, value: &Value, what: &str) -> Result<Grid> {
        let m = match value {
            Value::Matrix(m) => m.clone(),
            Value::Logical(l) => l.to_numeric(&self.cancel)?,
            Value::Container(Array::Typed(t)) => t.to_matrix()?,
            Value::Container(Array::Character(c)) => Matrix::from_column_vec(
                c.rows(),
                c.cols(),
                c.units().iter().map(|u| f64::from(*u)).collect(),
            )?,
            Value::Text(t) => {
                let units: Vec<f64> = t.encode_utf16().map(f64::from).collect();
                Matrix::from_column_vec(usize::from(!units.is_empty()), units.len(), units)?
            }
            Value::Complex(_) => return Err(format!("{what} must be real")),
            _ => return Err(format!("{what} must be numeric")),
        };
        Ok(Grid {
            rows: m.rows(),
            cols: m.cols(),
            data: m.data().to_vec(),
        })
    }

    /// Two coordinate grids, generated only after checking the complete
    /// shape and reserving their budget plus `extra_arrays` point-sized
    /// arrays needed by the caller (result and, for griddata, query pairs).
    fn sc2_mesh(
        &mut self,
        rows: usize,
        cols: usize,
        row_at: impl Fn(usize) -> f64,
        col_at: impl Fn(usize) -> f64,
        extra_arrays: usize,
    ) -> Result<(Grid, Grid)> {
        let count = Matrix::checked_len(rows, cols)?;
        self.charge(count.saturating_mul(2 + extra_arrays))?;
        let mut r = Vec::with_capacity(count);
        let mut c = Vec::with_capacity(count);
        for j in 0..cols {
            self.sc2_poll(j)?;
            r.extend((0..rows).map(&row_at));
            c.extend(std::iter::repeat_n(col_at(j), rows));
        }
        let grid = |data| Grid { rows, cols, data };
        Ok((grid(r), grid(c)))
    }

    fn sc2_poll(&self, i: usize) -> Result<()> {
        if i.is_multiple_of(1024) {
            self.check_cancel()?;
        }
        Ok(())
    }

    // -- fftn / ifftn -------------------------------------------------------

    fn sc2_fftn(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        if args.is_empty() {
            return Err(NOT_ENOUGH.into());
        }
        let inverse = name == "ifftn";
        let mut args = args.to_vec();
        let mut symmetric = false;
        if inverse
            && args.len() >= 2
            && let Some(flag) = sc2_text(&args[args.len() - 1])
        {
            symmetric = match flag.to_ascii_lowercase().as_str() {
                "symmetric" => true,
                "nonsymmetric" => false,
                _ => {
                    return Err(
                        "Final string argument must be either 'symmetric' or 'nonsymmetric'."
                            .into(),
                    );
                }
            };
            args.pop();
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let class = sc2_class(&args[0]);
        if let Some(class) = class.filter(|c| c.is_integer()) {
            return Err(format!(
                "Undefined function '{name}' for input arguments of type '{}'.",
                class.name()
            ));
        }
        let (rows, cols, data, real) = match &args[0] {
            Value::Complex(z) => (z.rows(), z.cols(), z.data().to_vec(), false),
            other => {
                let g = self.sc2_grid(other, "X")?;
                let data = g.data.iter().map(|v| C::new(*v, 0.)).collect();
                (g.rows, g.cols, data, true)
            }
        };
        let (m, n) = match args.get(1) {
            None => (rows, cols),
            Some(size) if sc2_is_empty(size) => (rows, cols),
            Some(size) => {
                let siz = self.sc2_grid(size, "SIZ")?;
                if siz.len() < 2 {
                    return Err("SIZ must have at least ndims(X) elements.".into());
                }
                for v in &siz.data {
                    if !v.is_finite() || *v < 0. || v.fract() != 0. {
                        return Err("SIZ must be a vector of nonnegative integers.".into());
                    }
                }
                if siz.data[2..].iter().any(|v| *v != 1.) {
                    return Err(format!("{name} with more than two sizes: {NO_ND}"));
                }
                (siz.data[0] as usize, siz.data[1] as usize)
            }
        };
        let count = Matrix::checked_len(m, n)?;
        self.charge(count.saturating_mul(2))?;
        let data = sc2_resize(&data, rows, m, n, C::new(0., 0.));
        // A symmetric inverse of a vector transforms its long dimension.
        let (sm, sn) = if symmetric && (m == 1 || n == 1) {
            (count, 1)
        } else {
            (m, n)
        };
        let value = if count == 0 {
            sc2_num(m, n, Vec::new())?
        } else if sm <= 4 && sn <= 4 {
            let mut z = sc2_small_fft(&data, real, sm, sn, inverse, symmetric);
            if inverse {
                let scale = (m * n) as f64;
                for v in &mut z {
                    *v /= scale;
                }
            }
            self.sc2_complex_value(m, n, z, symmetric)?
        } else {
            let input = if real && !symmetric {
                sc2_num(m, n, data.iter().map(|z| z.re).collect())?
            } else {
                let data = if symmetric {
                    sc2_conjugate_half(&data, sm, sn)
                } else {
                    data
                };
                Value::Complex(ComplexMatrix::from_column_vec(sm, sn, data, &self.cancel)?)
            };
            let transform = if inverse { "ifft" } else { "fft" };
            let empty = || Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).unwrap());
            let mut v = input;
            for (dim, len) in [(1., sm), (2., sn)] {
                if len > 1 {
                    v = self.call(
                        transform,
                        vec![v, empty(), Value::Matrix(Matrix::scalar(dim))],
                        out,
                    )?;
                }
            }
            match v {
                Value::Complex(z) if symmetric => {
                    sc2_num(m, n, z.real(&self.cancel)?.data().to_vec())?
                }
                Value::Matrix(x) if symmetric => sc2_num(m, n, x.data().to_vec())?,
                v => v,
            }
        };
        if class == Some(NumericClass::Single) {
            return sc2_single(value);
        }
        Ok(value)
    }

    /// A complex result, real when every imaginary part is zero.
    fn sc2_complex_value(&self, m: usize, n: usize, z: Vec<C>, real: bool) -> Result<Value> {
        if real || z.iter().all(|v| v.im == 0.) {
            return sc2_num(m, n, z.iter().map(|v| v.re).collect());
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            m,
            n,
            z,
            &self.cancel,
        )?))
    }

    // -- convn ---------------------------------------------------------------

    fn sc2_convn(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        if args.len() < 2 {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let shape = match args.get(2) {
            None => "full".to_string(),
            Some(v) => sc2_text(v)
                .map(|s| s.to_ascii_lowercase())
                .filter(|s| matches!(s.as_str(), "full" | "same" | "valid"))
                .ok_or("SHAPE must be 'full', 'same', or 'valid'.")?,
        };
        let empty_b = match &args[1] {
            Value::Matrix(m) => m.is_empty(),
            Value::Complex(m) => m.rows() * m.cols() == 0,
            _ => false,
        };
        if empty_b && shape != "valid" {
            // convn keeps the size of A when B is empty.
            let a = match &args[0] {
                Value::Complex(z) => (z.rows(), z.cols()),
                other => {
                    let g = self.sc2_grid(other, "A")?;
                    (g.rows, g.cols)
                }
            };
            let count = Matrix::checked_len(a.0, a.1)?;
            self.charge(count)?;
            return sc2_num(a.0, a.1, vec![0.; count]);
        }
        let mut call = vec![args[0].clone(), args[1].clone()];
        call.push(Value::Text(shape));
        self.call("conv2", call, out)
    }

    // -- freqspace -----------------------------------------------------------

    fn sc2_freqspace(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let n = self.sc2_grid(&args[0], "N")?;
        if n.len() == 0 || n.len() > 2 || n.data.iter().any(|v| !v.is_finite() || *v < 0.) {
            return Err("N must be a nonnegative scalar or a two-element vector.".into());
        }
        let flag = match args.get(1) {
            None => None,
            Some(v) => Some(
                sc2_text(v)
                    .ok_or("FLAG must be 'whole' or 'meshgrid'.")?
                    .to_ascii_lowercase(),
            ),
        };
        if flag
            .as_deref()
            .is_some_and(|f| !matches!(f, "whole" | "meshgrid"))
        {
            return Err("FLAG must be 'whole' or 'meshgrid'.".into());
        }
        let meshgrid = flag.as_deref() == Some("meshgrid");
        let (n1, n2) = (n.data[0], n.data[n.len() - 1]);
        if count <= 1 && !meshgrid {
            let n1 = n1.floor();
            let stop = if flag.as_deref() == Some("whole") {
                2. * (n1 - 1.) / n1
            } else {
                1.
            };
            let step = 2. / n1;
            if !step.is_finite() || n1 == 0. {
                return Ok(vec![sc2_num(1, 0, Vec::new())?]);
            }
            let len = Matrix::checked_len(
                1,
                (((stop / step) + 1e-10).floor() as usize).saturating_add(1),
            )?;
            self.charge(len)?;
            let f: Vec<f64> = (0..len).map(|i| i as f64 * step).collect();
            return Ok(vec![sc2_num(1, len, f)?]);
        }
        // Lengths and budget first: nothing is allocated before they pass.
        let (rows_n, cols_n) = (n1.floor() as usize, n2.floor() as usize);
        let (b, a) = (
            Matrix::checked_len(1, rows_n)?,
            Matrix::checked_len(1, cols_n)?,
        );
        let axis_at = |n: f64, i: usize| (i as f64 - (n / 2.).floor()) * (2. / n);
        if meshgrid {
            if count <= 1 {
                let len = Matrix::checked_len(b, a)?;
                self.charge(len)?;
                let mut x = Vec::with_capacity(len);
                for j in 0..a {
                    self.sc2_poll(j)?;
                    x.extend(std::iter::repeat_n(axis_at(n2, j), b));
                }
                return Ok(vec![sc2_num(b, a, x)?]);
            }
            let (y, x) = self.sc2_mesh(b, a, |i| axis_at(n1, i), |i| axis_at(n2, i), 0)?;
            return Ok(vec![x.value()?, y.value()?]);
        }
        self.charge(a.saturating_add(b))?;
        Ok(vec![
            sc2_num(1, a, (0..a).map(|i| axis_at(n2, i)).collect())?,
            sc2_num(1, b, (0..b).map(|i| axis_at(n1, i)).collect())?,
        ])
    }

    // -- iqr -----------------------------------------------------------------

    fn sc2_iqr(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let class = sc2_class(&args[0]);
        let x = self.sc2_grid(&args[0], "X")?;
        self.charge(x.len())?;
        let dim = match args.get(1) {
            None => {
                if x.rows != 1 {
                    1
                } else {
                    2
                }
            }
            Some(d) => {
                let d = sc2_scalar(d).ok_or("DIM must be a positive integer scalar.")?;
                if !(d >= 1. && d.fract() == 0.) {
                    return Err("DIM must be a positive integer scalar.".into());
                }
                d as usize
            }
        };
        let (r, c, values) = match dim {
            1 => {
                // MATLAB's canonical [] still reduces to one NaN.
                let cols = if x.rows == 0 && x.cols == 0 {
                    1
                } else {
                    x.cols
                };
                self.charge(cols)?;
                let mut v = Vec::with_capacity(cols);
                for j in 0..cols {
                    self.check_cancel()?;
                    let mut col = x.data[j * x.rows..(j + 1) * x.rows].to_vec();
                    v.push(sc2_quartile_range(&mut col));
                }
                (1, cols, v)
            }
            2 => {
                let mut v = Vec::with_capacity(x.rows);
                for i in 0..x.rows {
                    self.check_cancel()?;
                    let mut row: Vec<f64> = (0..x.cols).map(|j| x.at(i, j)).collect();
                    v.push(sc2_quartile_range(&mut row));
                }
                (x.rows, 1, v)
            }
            _ => (
                x.rows,
                x.cols,
                x.data
                    .iter()
                    .map(|v| if v.is_nan() { f64::NAN } else { 0. })
                    .collect(),
            ),
        };
        let value = sc2_num(r, c, values)?;
        if class == Some(NumericClass::Single) {
            return sc2_single(value);
        }
        Ok(value)
    }

    // -- interpn / interp3 ---------------------------------------------------

    fn sc2_interp3(&mut self, args: &[Value]) -> Result<Value> {
        let numeric = args.iter().take_while(|a| sc2_text(a).is_none()).count();
        if numeric == 4 {
            // The reference's interp3(V, Xq, Yq, Zq) with a matrix V fails inside
            // its grid helper.
            return Err("Output argument \"varargout{3}\" (and maybe others) not assigned during call to \"meshgridvectors\".".into());
        }
        Err(format!("interp3 requires three-dimensional data: {NO_ND}"))
    }

    fn sc2_interpn(&mut self, args: &[Value]) -> Result<Value> {
        let numeric = args.iter().take_while(|a| sc2_text(a).is_none()).count();
        if numeric == 0 {
            return Err(NOT_ENOUGH.into());
        }
        let mut method = Method::Linear;
        let mut extrap = None;
        let rest = &args[numeric..];
        if let Some(text) = rest.first().and_then(sc2_text) {
            method = sc2_method(&text).ok_or("Invalid interpolation type specified.")?;
            if let Some(v) = rest.get(1) {
                extrap = Some(sc2_scalar(v).ok_or("Extrapolation value must be a scalar.")?);
            }
            if rest.len() > 2 {
                return Err("Too many input arguments.".into());
            }
        }
        let grids: Vec<Grid> = args[..numeric]
            .iter()
            .map(|a| self.sc2_grid(a, "Interpolation data"))
            .collect::<Result<_>>()?;
        let first_vector = grids[0].is_vector() && grids[0].len() > 1;
        let mut result_charged = false;
        // The data V, its grid vectors and the query coordinates.
        let (v, axes, queries): (Grid, Vec<Vec<f64>>, Vec<Grid>) = match numeric {
            1 | 2 if numeric == 1 || grids[1].len() == 1 => {
                let times = if numeric == 2 { grids[1].data[0] } else { 1. };
                if !(times.is_finite() && times >= 0. && times.fract() == 0.) {
                    return Err("NTIMES must be a nonnegative integer.".into());
                }
                if times > 16. {
                    return Err(
                        "interpn refinement exceeds the resource limit (NTIMES > 16).".into(),
                    );
                }
                let v = grids[0].clone();
                if v.len() == 0 {
                    return Err(EMPTY_DATA.into());
                }
                let dims: Vec<usize> = if first_vector {
                    vec![v.len()]
                } else {
                    vec![v.rows, v.cols]
                };
                // Compute all refined lengths before generating coordinates.
                let refined: Vec<usize> = dims
                    .iter()
                    .map(|d| {
                        let steps = d.saturating_sub(1).saturating_mul(1 << times as u32);
                        Matrix::checked_len(1, steps.saturating_add(1))
                    })
                    .collect::<Result<_>>()?;
                let at = |dim: usize, i: usize| {
                    1. + i as f64 * (dims[dim] - 1) as f64
                        / refined[dim].saturating_sub(1).max(1) as f64
                };
                let queries = if refined.len() == 1 {
                    let len = refined[0];
                    self.charge(len.saturating_mul(2))?;
                    let shape = if v.rows == 1 { (1, len) } else { (len, 1) };
                    vec![Grid {
                        rows: shape.0,
                        cols: shape.1,
                        data: (0..len).map(|i| at(0, i)).collect(),
                    }]
                } else {
                    let (q1, q2) =
                        self.sc2_mesh(refined[0], refined[1], |i| at(0, i), |i| at(1, i), 1)?;
                    vec![q1, q2]
                };
                result_charged = true;
                let axes = dims
                    .iter()
                    .map(|d| (1..=*d).map(|i| i as f64).collect())
                    .collect();
                (v, axes, queries)
            }
            2 if first_vector => {
                let v = grids[0].clone();
                (
                    v.clone(),
                    vec![(1..=v.len()).map(|i| i as f64).collect()],
                    vec![grids[1].clone()],
                )
            }
            3 if first_vector && grids[1].is_vector() => (
                grids[1].clone(),
                vec![grids[0].data.clone()],
                vec![grids[2].clone()],
            ),
            3 => {
                let v = grids[0].clone();
                if v.is_vector() && v.len() > 1 {
                    return Err(
                        "The grid vectors do not define a grid of points that match the given values."
                            .into(),
                    );
                }
                let axes = vec![
                    (1..=v.rows).map(|i| i as f64).collect(),
                    (1..=v.cols).map(|i| i as f64).collect(),
                ];
                (v, axes, vec![grids[1].clone(), grids[2].clone()])
            }
            5 => {
                let v = grids[2].clone();
                if grids[..3].iter().any(|g| g.len() == 0) {
                    return Err(EMPTY_DATA.into());
                }
                // Full arrays must have V's shape and ndgrid structure:
                // X1 is constant across columns; X2 across rows.
                for (dim, g) in grids[..2].iter().enumerate() {
                    if !g.is_vector() {
                        if (g.rows, g.cols) != (v.rows, v.cols) {
                            return Err(
                                "The grid arrays must have the same size as the values array."
                                    .into(),
                            );
                        }
                        for c in 0..g.cols {
                            self.check_cancel()?;
                            for r in 0..g.rows {
                                self.sc2_poll(r)?;
                                let expected = if dim == 0 { g.at(r, 0) } else { g.at(0, c) };
                                if g.at(r, c) != expected {
                                    return Err("Input grid is not a valid NDGRID.".into());
                                }
                            }
                        }
                    }
                }
                let axis = |g: &Grid, dim: usize| -> Vec<f64> {
                    if g.is_vector() {
                        g.data.clone()
                    } else if dim == 1 {
                        (0..g.rows).map(|r| g.at(r, 0)).collect()
                    } else {
                        (0..g.cols).map(|c| g.at(0, c)).collect()
                    }
                };
                (
                    v,
                    vec![axis(&grids[0], 1), axis(&grids[1], 2)],
                    vec![grids[3].clone(), grids[4].clone()],
                )
            }
            n if n > 5 || (n == 4 && !first_vector) => {
                return Err(format!("interpn with more than two dimensions: {NO_ND}"));
            }
            _ => return Err("Wrong number of input arguments.".into()),
        };
        // Check the grid against V.
        let dims: Vec<usize> = if axes.len() == 1 {
            vec![v.len()]
        } else {
            vec![v.rows, v.cols]
        };
        if axes.iter().zip(&dims).any(|(a, d)| a.len() != *d) {
            return Err(
                "The grid vectors do not define a grid of points that match the given values."
                    .into(),
            );
        }
        if v.len() == 0 || axes.iter().any(|a| a.is_empty()) {
            return Err(EMPTY_DATA.into());
        }
        for axis in &axes {
            if axis
                .windows(2)
                .any(|w| w[1].partial_cmp(&w[0]) != Some(Ordering::Greater))
            {
                return Err("The grid vectors must be strictly monotonic increasing.".into());
            }
        }
        // Query points: equal shapes, or grid vectors of different shapes.
        let queries = if queries.len() == 2
            && (queries[0].rows, queries[0].cols) != (queries[1].rows, queries[1].cols)
        {
            if !(queries[0].is_vector() && queries[1].is_vector()) {
                return Err("Query points must be the same size, or grid vectors.".into());
            }
            let (q1, q2) = self.sc2_mesh(
                queries[0].len(),
                queries[1].len(),
                |i| queries[0].data[i],
                |i| queries[1].data[i],
                1,
            )?;
            result_charged = true;
            vec![q1, q2]
        } else {
            queries
        };
        let shape = (queries[0].rows, queries[0].cols);
        let points = queries[0].len();
        if !result_charged {
            self.charge(points)?;
        }
        // Coefficient construction is linear in every line's length, but is
        // repeated per query in this tensor-product implementation. Reserve
        // its full work (including temporary/coefficient arrays) before any
        // coefficients are allocated. This also bounds cancellation latency.
        let coefficient_path = matches!(method, Method::Spline | Method::Pchip)
            || (method == Method::Cubic
                && (axes.iter().any(|a| !sc2_uniform(a))
                    || (axes.len() == 2 && (v.rows < 3 || v.cols < 3))));
        if coefficient_path {
            let entries = v
                .len()
                .saturating_add(if axes.len() == 2 { v.cols } else { 0 });
            self.charge(points.saturating_mul(entries).saturating_mul(16))?;
        }
        let mut result = Vec::with_capacity(points);
        let uniform: Vec<bool> = axes.iter().map(|a| sc2_uniform(a)).collect();
        for i in 0..points {
            self.sc2_poll(i)?;
            let t: Vec<f64> = queries.iter().map(|q| q.data[i]).collect();
            let inside = t
                .iter()
                .zip(&axes)
                .all(|(t, a)| *t >= a[0] && *t <= a[a.len() - 1]);
            if (!inside && (method != Method::Spline || extrap.is_some()))
                || t.iter().any(|v| v.is_nan())
            {
                result.push(extrap.unwrap_or(f64::NAN));
                continue;
            }
            let value = if axes.len() == 1 {
                sc2_interp_line(self, &axes[0], &v.data, t[0], method)?
            } else {
                self.sc2_interp_plane(&v, &axes, &t, method, &uniform)?
            };
            result.push(value);
        }
        Grid {
            rows: shape.0,
            cols: shape.1,
            data: result,
        }
        .value()
    }

    /// Tensor-product interpolation of a matrix at one point.
    fn sc2_interp_plane(
        &self,
        v: &Grid,
        axes: &[Vec<f64>],
        t: &[f64],
        method: Method,
        uniform: &[bool],
    ) -> Result<f64> {
        self.check_cancel()?;
        let separable = matches!(method, Method::Linear | Method::Nearest)
            || (method == Method::Cubic
                && uniform.iter().all(|u| *u)
                && v.rows >= 3
                && v.cols >= 3);
        if separable {
            let w1 = sc2_weights(&axes[0], t[0], method, true);
            let w2 = sc2_weights(&axes[1], t[1], method, true);
            let mut sum = 0.;
            for (j, b) in &w2 {
                let mut inner = 0.;
                for (i, a) in &w1 {
                    inner += a * v.at(*i, *j);
                }
                sum += b * inner;
            }
            return Ok(sum);
        }
        // Along dimension 1 for every column, then along dimension 2.
        let line: Vec<f64> = (0..v.cols)
            .map(|c| {
                self.check_cancel()?;
                let col = &v.data[c * v.rows..(c + 1) * v.rows];
                sc2_interp_line(self, &axes[0], col, t[0], method)
            })
            .collect::<Result<_>>()?;
        sc2_interp_line(self, &axes[1], &line, t[1], method)
    }
}

mod geometry;
mod integrals;

#[cfg(test)]
mod tests {
    use super::*;

    fn with_budget_left(elements: usize) -> Engine {
        let mut engine = Engine::default();
        engine.charge(32 * 1024 * 1024 - elements).unwrap();
        engine
    }

    #[test]
    fn sc2_mesh_checks_limits_and_full_budget_before_generating_coordinates() {
        // Even the coordinate callbacks must not run before both checks.
        let unexpected = |_| panic!("coordinates generated before preflight");
        let error = Engine::default()
            .sc2_mesh(65537, 65537, unexpected, unexpected, 1)
            .err()
            .unwrap();
        assert!(error.contains("element limit"), "{error}");
        for extra_arrays in [0, 1, 3] {
            let error = with_budget_left(9 * (2 + extra_arrays) - 1)
                .sc2_mesh(3, 3, unexpected, unexpected, extra_arrays)
                .err()
                .unwrap();
            assert!(error.contains("evaluation budget"), "{error}");
        }
    }

    #[test]
    fn sc2_refinement_reserves_queries_and_result_together() {
        let v = Value::Matrix(Matrix::filled(2, 2, 1.).unwrap());
        // Nine points need two query grids and the result (27 elements).
        let error = with_budget_left(26).sc2_interpn(&[v]).err().unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
    }

    #[test]
    fn sc2_griddata_reserves_mesh_queries_and_result_together() {
        let args = [
            sc2_num(1, 3, vec![0., 1., 0.]).unwrap(),
            sc2_num(1, 3, vec![0., 0., 1.]).unwrap(),
            sc2_num(1, 3, vec![1., 2., 3.]).unwrap(),
            sc2_num(1, 3, vec![0., 0.25, 0.5]).unwrap(),
            sc2_num(3, 1, vec![0., 0.25, 0.5]).unwrap(),
        ];
        let error = with_budget_left(44)
            .sc2_griddata(&args, 1, &mut Execution::default())
            .err()
            .unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
    }

    #[test]
    fn sc2_simplex_search_reserves_systems_and_solve_scratch() {
        let args = [
            sc2_num(3, 2, vec![0., 1., 0., 0., 0., 1.]).unwrap(),
            sc2_num(1, 3, vec![1., 2., 3.]).unwrap(),
            sc2_num(1, 2, vec![0.25, 0.25]).unwrap(),
        ];
        // Enough for the stored 3x3 system, but not its live solve and outputs.
        let error = with_budget_left(20).sc2_tsearchn(&args).err().unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
    }

    #[test]
    fn sc2_freqspace_budget_matches_returned_elements() {
        let n = Value::Matrix(Matrix::scalar(10000.));
        assert!(
            with_budget_left(20000)
                .sc2_freqspace(std::slice::from_ref(&n), 2)
                .is_ok()
        );
        let error = with_budget_left(19999)
            .sc2_freqspace(&[n], 2)
            .err()
            .unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
        let args = [
            sc2_num(1, 2, vec![4., 6.]).unwrap(),
            Value::Text("meshgrid".into()),
        ];
        let one = with_budget_left(24).sc2_freqspace(&args, 1).unwrap();
        assert_eq!(one.len(), 1);
        let error = with_budget_left(23).sc2_freqspace(&args, 1).err().unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
        assert!(with_budget_left(48).sc2_freqspace(&args, 2).is_ok());
        let error = with_budget_left(47).sc2_freqspace(&args, 2).err().unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
    }
}

#[cfg(test)]
mod fix3_tests {
    use super::*;

    #[test]
    fn sc2_coefficient_builders_observe_cancellation() {
        let engine = Engine::default();
        engine
            .cancellation()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        for result in [
            sc2_spline_coefs(&engine, &[1., 2., 3., 4.], &[1., 2., 3., 4.]),
            sc2_pchip_coefs(&engine, &[1., 2., 3., 4.], &[1., 2., 3., 4.]),
        ] {
            assert!(result.unwrap_err().contains("cancel"));
        }
    }

    #[test]
    fn sc2_coefficients_reserve_full_query_work() {
        let mut engine = Engine::default();
        engine.charge(32 * 1024 * 1024 - 128).unwrap();
        let error = engine
            .sc2_interpn(&[
                sc2_num(1, 4, vec![1., 2., 3., 4.]).unwrap(),
                sc2_num(1, 2, vec![1.5, 2.5]).unwrap(),
                Value::Text("spline".into()),
            ])
            .err()
            .unwrap();
        assert!(error.contains("evaluation budget"), "{error}");
    }
}

#[cfg(test)]
mod spline_tests {
    use super::*;

    fn eval(coefs: &[[f64; 4]], x: &[f64], t: &[f64]) -> Vec<f64> {
        t.iter().map(|&t| sc2_eval_piece(coefs, x, t)).collect()
    }

    #[test]
    fn sc2_not_a_knot_spline_reproduces_cubics_on_uneven_grids() {
        let engine = Engine::default();
        let f = |x: f64| ((x - 1.) * x - 2.) * x + 1.;
        let x = [0., 0.5, 2., 3., 4.5];
        let y: Vec<f64> = x.iter().map(|&v| f(v)).collect();
        let t = [-1., 0.25, 1.3, 2.9, 4., 5.];
        let coefs = sc2_spline_coefs(&engine, &x, &y).unwrap();
        for (got, &t) in eval(&coefs, &x, &t).iter().zip(&t) {
            assert!((got - f(t)).abs() < 1e-12, "{t}: {got} vs {}", f(t));
        }
        // Three points: the parabola; two: the line.
        let coefs = sc2_spline_coefs(&engine, &[0., 1., 3.], &[1., 0., 4.]).unwrap();
        let p = |x: f64| (x - 1.) * (x - 1.);
        for t in [-1., 0.5, 2., 4.] {
            assert!((sc2_eval_piece(&coefs, &[0., 1., 3.], t) - p(t)).abs() < 1e-13);
        }
        let coefs = sc2_spline_coefs(&engine, &[1., 3.], &[2., 6.]).unwrap();
        assert_eq!(eval(&coefs, &[1., 3.], &[0., 2., 4.]), [0., 4., 8.]);
    }

    #[test]
    fn sc2_three_point_spline_is_an_exact_parabola_at_any_scale() {
        let engine = Engine::default();
        let close = |got: f64, want: f64| {
            assert!(((got - want) / want).abs() < 4e-16, "{got:e} vs {want:e}");
        };
        // Tiny spacings: the Hermite form would invent a cubic term of Inf.
        let x = [0., 1e-201, 1e-200];
        let coefs = sc2_spline_coefs(&engine, &x, &[0., 1e-202, 1e-200]).unwrap();
        assert!(coefs.iter().all(|c| c[0] == 0.));
        close(sc2_eval_piece(&coefs, &x, 5e-202), 2.5e-203);
        // Huge slopes: the end slope (3 d0 - d1)/2 overflows in the product form.
        let x = [0., 1., 2.];
        let coefs = sc2_spline_coefs(&engine, &x, &[0., 1e308, 1e308]).unwrap();
        close(sc2_eval_piece(&coefs, &x, 0.5), 6.25e307);
        close(sc2_eval_piece(&coefs, &x, 1.5), 1.125e308);
        close(sc2_parabola_end_slope(1., 3., 1e308, -1e308), 1.5e308);
        // pchip: the cap 3 d0 overflows too, so the end slope stays 1.5e308.
        let coefs = sc2_pchip_coefs(&engine, &x, &[0., 1e308, 1e308]).unwrap();
        close(coefs[0][2], 1.5e308);
    }

    #[test]
    fn sc2_pchip_slopes_follow_the_harmonic_mean_and_end_rules() {
        let engine = Engine::default();
        // h = [1 2 1], d = [1 0.5 2]: interior 9/13 and 6/7 (weights 5:4 and
        // 4:5 on the reciprocal differences), ends 7/6 and 5/2.
        let coefs = sc2_pchip_coefs(&engine, &[0., 1., 3., 4.], &[0., 1., 2., 4.]).unwrap();
        let slopes: Vec<f64> = coefs.iter().map(|c| c[2]).collect();
        let last = &coefs[2];
        let end = (3. * last[0] + 2. * last[1]) + last[2];
        for (got, want) in slopes
            .iter()
            .chain([&end])
            .zip([7. / 6., 9. / 13., 6. / 7., 2.5])
        {
            assert!((got - want).abs() < 1e-14, "{got} vs {want}");
        }
        // End slope capped at 3 d0 when the data turn, zero when it opposes d0.
        let coefs = sc2_pchip_coefs(&engine, &[0., 1., 2.], &[0., 1., -5.]).unwrap();
        assert_eq!((coefs[0][2], coefs[1][2]), (3., 0.));
        let coefs = sc2_pchip_coefs(&engine, &[0., 1., 2.], &[0., 1., 6.]).unwrap();
        assert_eq!(coefs[0][2], 0.);
        // Monotone data stay monotone between the points.
        let x = [0., 0.1, 1., 1.2, 3., 7.];
        let y = [0., 0., 0.5, 3., 3.1, 10.];
        let coefs = sc2_pchip_coefs(&engine, &x, &y).unwrap();
        let t: Vec<f64> = (0..=700).map(|i| f64::from(i) / 100.).collect();
        let v = eval(&coefs, &x, &t);
        assert!(v.windows(2).all(|w| w[1] >= w[0] - 1e-15));
    }
}
