//! Image app primitives, implemented from the public MathWorks function pages.
//! Numerical references: CIE 1976 Lab, IEC 61966-2-1 sRGB, Adobe RGB (1998),
//! Bradford chromatic adaptation, ITU-R BT.601 studio encoding, Otsu (1979),
//! and Bradley & Roth (2007). See tool/refs/image/image_color.py and task notes.
//! All working arrays are bounded; long loops poll the engine cancellation flag.
use super::image_basic::{self as basic, Img, Kind};
use super::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, NumericClass, Result, TypedArray};
use nalgebra::{Matrix3, Vector3};

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "rgb2lab"
            | "lab2rgb"
            | "rgb2ycbcr"
            | "ycbcr2rgb"
            | "multithresh"
            | "imquantize"
            | "adaptthresh"
            | "label2rgb"
            | "labeloverlay"
            | "imoverlay"
            | "boundarymask"
            | "impixel"
    )
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match name {
        "multithresh" | "imquantize" => 2,
        "impixel" => 3,
        _ => 1,
    };
    if count > max {
        return Err(format!("{name}: too many output arguments"));
    }
    let mut values = match name {
        "rgb2lab" | "lab2rgb" | "rgb2ycbcr" | "ycbcr2rgb" => vec![color_convert(e, name, a)?],
        "multithresh" => multithresh(e, a, out)?,
        "imquantize" => quantize(e, a)?,
        "adaptthresh" => vec![adaptive(e, a, false)?],
        "label2rgb" => vec![labels_rgb(e, a)?],
        "labeloverlay" | "imoverlay" => vec![overlay(e, name, a)?],
        "boundarymask" => vec![boundary(e, a)?],
        "impixel" => pixels(e, a, count)?,
        _ => unreachable!(),
    };
    e.check_cancel()?;
    values.truncate(count.max(1));
    Ok(values)
}

fn arity(name: &str, a: &[Value], lo: usize, hi: usize) -> Result<()> {
    if a.len() < lo {
        Err(format!("{name}: not enough input arguments"))
    } else if a.len() > hi {
        Err(format!("{name}: too many input arguments"))
    } else {
        Ok(())
    }
}
fn nums(name: &str, v: &Value) -> Result<Vec<f64>> {
    basic::numbers(v).ok_or_else(|| format!("{name}: expected real numeric input"))
}
fn scalar(name: &str, v: &Value) -> Result<f64> {
    let n = nums(name, v)?;
    if n.len() != 1 || !n[0].is_finite() {
        return Err(format!("{name}: expected a finite scalar"));
    }
    Ok(n[0])
}
fn text(e: &Engine, name: &str, v: &Value) -> Result<String> {
    basic::text(e, v)
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| format!("{name}: expected text"))
}
fn pairs<'a>(e: &Engine, name: &str, a: &'a [Value]) -> Result<Vec<(String, &'a Value)>> {
    if !a.len().is_multiple_of(2) {
        return Err(format!("{name}: expected name-value pairs"));
    }
    a.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Ok((text(e, name, &p[0])?, &p[1])))
        .collect()
}
fn vector(name: &str, v: &Value) -> Result<Vec<f64>> {
    let d = v.dims();
    if d.len() > 2 || (d[0] != 1 && d[1] != 1 && v.dims().iter().product::<usize>() != 0) {
        return Err(format!("{name}: expected a vector"));
    }
    nums(name, v)
}
fn kind(v: &Value) -> Kind {
    match v {
        Value::Container(Array::Typed(t)) => Kind::Typed(t.class()),
        Value::Logical(_) | Value::Container(Array::Logical(_)) => Kind::Logical,
        _ => Kind::Double,
    }
}
fn range(k: Kind) -> (f64, f64) {
    match k {
        Kind::Typed(c) if c != NumericClass::Single => (c.min() as f64, (c.max() - c.min()) as f64),
        _ => (0., 1.),
    }
}
fn unit(k: Kind, x: f64) -> f64 {
    let (lo, w) = range(k);
    (x - lo) / w
}
fn store(e: &Engine, dims: &[usize], data: Vec<f64>, k: Kind) -> Result<Value> {
    let rows = dims[0];
    let cols = dims[1..].iter().product();
    Ok(match k {
        Kind::Double => Value::Matrix(Matrix::from_column_vec(rows, cols, data)?.with_dims(dims)?),
        Kind::Typed(c) => Value::Container(Array::Typed(
            TypedArray::from_reals(c, rows, cols, data.into_iter())?.with_dims(dims)?,
        )),
        Kind::Logical => Value::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                data.iter().map(|&x| u8::from(x != 0.)).collect(),
                &e.cancel,
            )?
            .with_dims(dims)?,
        ),
    })
}
fn bounded(e: &Engine, name: &str, n: usize, arrays: usize) -> Result<()> {
    let budget = super::limits::memory_bytes(&e.limits);
    if n.checked_mul(arrays)
        .and_then(|v| v.checked_mul(8))
        .is_none_or(|bytes| bytes > budget)
    {
        return Err(format!(
            "{name}: workspace exceeds the {} MiB limit",
            budget / (1024 * 1024)
        ));
    }
    Ok(())
}
fn image(e: &Engine, name: &str, v: &Value) -> Result<Img> {
    bounded(e, name, v.dims().iter().product::<usize>(), 4)?;
    basic::image(e, name, v)
}
fn gray(e: &Engine, name: &str, v: &Value) -> Result<Img> {
    let img = image(e, name, v)?;
    if v.ndims() != 2 {
        return Err(format!("{name}: image must be 2-D"));
    }
    Ok(img)
}
fn rgb_kind(k: Kind, inverse: bool) -> bool {
    matches!(k, Kind::Double | Kind::Typed(NumericClass::Single))
        || (!inverse && matches!(k, Kind::Typed(NumericClass::Uint8 | NumericClass::Uint16)))
}

// Published RGB-to-XYZ matrices (D65), row major. Reverse conversions solve
// these fixed 3x3 systems with LU; no explicit matrix inverse is formed.
const SRGB: [f64; 9] = [
    0.4124564, 0.3575761, 0.1804375, 0.2126729, 0.7151522, 0.0721750, 0.0193339, 0.1191920,
    0.9503041,
];
const ADOBE: [f64; 9] = [
    0.5767309, 0.1855540, 0.1881852, 0.2973769, 0.6273491, 0.0752741, 0.0270343, 0.0706872,
    0.9911085,
];
const BRADFORD: [f64; 9] = [
    0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296,
];
const D65: [f64; 3] = [0.9504, 1., 1.0888];
const NATIVE_WHITE: [f64; 3] = [0.95047, 1., 1.08883];

fn whitepoint(e: &Engine, v: &Value) -> Result<[f64; 3]> {
    if let Some(s) = basic::text(e, v) {
        return match s.to_ascii_lowercase().as_str() {
            "d65" => Ok(D65),
            "d50" => Ok([0.9642, 1., 0.8251]),
            "a" => Ok([1.0985, 1., 0.3558]),
            "c" => Ok([0.9807, 1., 1.1822]),
            "e" => Ok([1.; 3]),
            "icc" => Ok([31595. / 32768., 1., 27030. / 32768.]),
            _ => Err("WhitePoint: unknown illuminant".into()),
        };
    }
    let n = nums("WhitePoint", v)?;
    if v.dims().as_ref() != [1, 3] || n.iter().any(|x| !x.is_finite() || *x <= 0.) {
        return Err("WhitePoint: expected a positive finite 1-by-3 vector".into());
    }
    Ok([n[0], n[1], n[2]])
}
fn linearize(x: f64, space: &str) -> f64 {
    match space {
        "linear-rgb" => x,
        "adobe-rgb-1998" => x.signum() * x.abs().powf(563. / 256.),
        _ if x.abs() <= 0.04045 => x / 12.92,
        _ => x.signum() * ((x.abs() + 0.055) / 1.055).powf(2.4),
    }
}
fn encode(x: f64, space: &str) -> f64 {
    match space {
        "linear-rgb" => x,
        "adobe-rgb-1998" => x.signum() * x.abs().powf(256. / 563.),
        _ if x.abs() <= 0.0031308 => 12.92 * x,
        _ => x.signum() * (1.055 * x.abs().powf(1. / 2.4) - 0.055),
    }
}
fn lab_f(x: f64) -> f64 {
    if x > (6f64 / 29.).powi(3) {
        x.cbrt()
    } else {
        x / (3. * (6f64 / 29.).powi(2)) + 4. / 29.
    }
}
fn lab_inv(x: f64) -> f64 {
    if x > 6. / 29. {
        x.powi(3)
    } else {
        3. * (6f64 / 29.).powi(2) * (x - 4. / 29.)
    }
}
fn color_convert(e: &Engine, name: &str, a: &[Value]) -> Result<Value> {
    arity(name, a, 1, 7)?;
    let img = image(e, name, &a[0])?;
    let dims = a[0].dims();
    if !(dims.len() == 2 && dims[1] == 3 || dims.len() == 3 && dims[2] == 3) {
        return Err(format!(
            "{name}: expected M-by-N-by-3 image or P-by-3 colormap"
        ));
    }
    let lab = matches!(name, "rgb2lab" | "lab2rgb");
    let inverse = matches!(name, "lab2rgb" | "ycbcr2rgb");
    if !rgb_kind(img.kind, name == "lab2rgb") {
        return Err(format!("{name}: unsupported input class"));
    }
    let mut white = D65;
    let mut space = "srgb".to_owned();
    // rgb2lab returns single for single input and double otherwise; the others keep the class.
    let mut output = if name == "rgb2lab" && img.kind != Kind::Typed(NumericClass::Single) {
        Kind::Double
    } else {
        img.kind
    };
    for (key, v) in pairs(e, name, &a[1..])? {
        match key.as_str() {
            "whitepoint" if lab => white = whitepoint(e, v)?,
            "colorspace" if lab => {
                space = text(e, name, v)?;
                if !matches!(space.as_str(), "srgb" | "adobe-rgb-1998" | "linear-rgb") {
                    return Err(format!("{name}: unknown ColorSpace"));
                }
            }
            "outputtype" if name == "lab2rgb" => {
                output = match text(e, name, v)?.as_str() {
                    "double" => Kind::Double,
                    "single" => Kind::Typed(NumericClass::Single),
                    "uint8" => Kind::Typed(NumericClass::Uint8),
                    "uint16" => Kind::Typed(NumericClass::Uint16),
                    _ => return Err(format!("{name}: unsupported OutputType")),
                }
            }
            _ => return Err(format!("{name}: unknown option '{key}'")),
        }
    }
    let matrix = Matrix3::from_row_slice(if space == "adobe-rgb-1998" {
        &ADOBE
    } else {
        &SRGB
    });
    let solve_rgb = matrix.lu();
    let bradford = Matrix3::from_row_slice(&BRADFORD);
    let solve_bradford = bradford.lu();
    let source = bradford * Vector3::from_row_slice(&NATIVE_WHITE);
    let target = bradford * Vector3::from_row_slice(&white);
    let ratio = target.component_div(&source);
    let n = img.data.len() / 3;
    let mut data = vec![0.; img.data.len()];
    // BT.601 coefficients as documented in the digital-video reference.
    let ycc = Matrix3::from_row_slice(&[
        65.481, 128.553, 24.966, -37.797, -74.203, 112., 112., -93.786, -18.214,
    ]);
    let solve_ycc = ycc.lu();
    for i in 0..n {
        basic::poll(e, i)?;
        let input = Vector3::from_fn(|k, _| {
            if name == "lab2rgb" {
                img.data[k * n + i]
            } else {
                unit(img.kind, img.data[k * n + i])
            }
        });
        let result = if lab {
            if inverse {
                let fy = (input[0] + 16.) / 116.;
                let f = [fy + input[1] / 500., fy, fy - input[2] / 200.];
                let xyz = Vector3::from_fn(|k, _| lab_inv(f[k]) * white[k]);
                let native = solve_bradford
                    .solve(&(bradford * xyz).component_div(&ratio))
                    .expect("nonsingular Bradford matrix");
                solve_rgb
                    .solve(&native)
                    .expect("nonsingular RGB matrix")
                    .map(|x| {
                        // Adobe's pure power transfer amplifies LU roundoff at
                        // zero. Remove only machine-scale residuals before it.
                        let x = if space == "adobe-rgb-1998" && x.abs() < 32. * f64::EPSILON {
                            0.
                        } else {
                            x
                        };
                        encode(x, &space)
                    })
            } else {
                let native = matrix * input.map(|x| linearize(x, &space));
                let xyz = solve_bradford
                    .solve(&(bradford * native).component_mul(&ratio))
                    .expect("nonsingular Bradford matrix");
                let f = Vector3::from_fn(|k, _| lab_f(xyz[k] / white[k]));
                Vector3::new(
                    116. * f[1] - 16.,
                    500. * (f[0] - f[1]),
                    200. * (f[1] - f[2]),
                )
            }
        } else if inverse {
            solve_ycc
                .solve(&(input * 255. - Vector3::new(16., 128., 128.)))
                .expect("nonsingular BT.601 matrix")
                .map(|v| v.clamp(0., 1.))
        } else {
            (ycc * input + Vector3::new(16., 128., 128.)) / 255.
        };
        for k in 0..3 {
            data[k * n + i] = result[k]
                * if name == "rgb2lab" {
                    1.
                } else {
                    range(output).1
                };
        }
    }
    store(e, &dims, data, output)
}

fn multithresh(e: &mut Engine, a: &[Value], _out: &mut Execution) -> Result<Vec<Value>> {
    let name = "multithresh";
    arity(name, a, 1, 2)?;
    bounded(e, name, a[0].dims().iter().product::<usize>(), 2)?;
    let k = kind(&a[0]);
    if !rgb_kind(k, false) && k != Kind::Typed(NumericClass::Int16) {
        return Err("multithresh: unsupported input class".into());
    }
    let n = if a.len() == 2 {
        scalar(name, &a[1])?
    } else {
        1.
    };
    if !(1. ..=20.).contains(&n) || n.fract() != 0. {
        return Err("multithresh: N must be an integer from 1 to 20".into());
    }
    // N = 1 uses the observed range like N > 1 (not graythresh's class-range bins).
    let n = n as usize;
    let data = nums(name, &a[0])?;
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for (i, &x) in data.iter().enumerate() {
        basic::poll(e, i)?;
        if x.is_finite() {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if !lo.is_finite() {
        lo = 0.;
        hi = 0.;
    }
    let mut hist = [0.; 256];
    for (i, &x) in data.iter().enumerate() {
        basic::poll(e, i)?;
        if !x.is_nan() {
            let b = if hi == lo {
                0
            } else {
                ((x - lo) / (hi - lo) * 255.).round().clamp(0., 255.) as usize
            };
            hist[b] += 1.;
        }
    }
    let occupied: Vec<usize> = (0..256).filter(|&i| hist[i] > 0.).collect();
    if occupied.len() <= n {
        let mut t: Vec<f64> = occupied
            .iter()
            .map(|&i| lo + (hi - lo) * i as f64 / 255.)
            .collect();
        t.resize(n, hi);
        return Ok(vec![
            store(e, &[1, n], t, k)?,
            Value::Matrix(Matrix::scalar(0.)),
        ]);
    }
    let mut p = [0.; 257];
    let mut s = [0.; 257];
    let mut square = 0.;
    for i in 0..256 {
        p[i + 1] = p[i] + hist[i];
        s[i + 1] = s[i] + hist[i] * i as f64;
        square += hist[i] * (i * i) as f64;
    }
    let score = |left: usize, right: usize| {
        let w = p[right] - p[left];
        if w > 0. {
            (s[right] - s[left]).powi(2) / w
        } else {
            f64::NEG_INFINITY
        }
    };
    // Exact dynamic programming: O((N+1)*256^2), independent of image size.
    let mut dp = vec![vec![f64::NEG_INFINITY; 257]; n + 2];
    let mut back = vec![vec![0usize; 257]; n + 2];
    dp[0][0] = 0.;
    for classes in 1..=n + 1 {
        e.check_cancel()?;
        for right in classes..=256 {
            for left in classes - 1..right {
                let v = dp[classes - 1][left] + score(left, right);
                if v > dp[classes][right] {
                    dp[classes][right] = v;
                    back[classes][right] = left;
                }
            }
        }
    }
    let mut thresholds = vec![0.; n];
    let mut right = 256;
    for classes in (2..=n + 1).rev() {
        let left = back[classes][right];
        thresholds[classes - 2] = lo + (hi - lo) * (left - 1) as f64 / 255.;
        right = left;
    }
    let mean_term = s[256].powi(2) / p[256];
    let metric = ((dp[n + 1][256] - mean_term) / (square - mean_term)).clamp(0., 1.);
    Ok(vec![
        store(e, &[1, n], thresholds, k)?,
        Value::Matrix(Matrix::scalar(metric)),
    ])
}

fn quantize(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    let name = "imquantize";
    arity(name, a, 2, 3)?;
    bounded(e, name, a[0].dims().iter().product::<usize>(), 4)?;
    let data = nums(name, &a[0])?;
    let levels = vector(name, &a[1])?;
    if levels.is_empty()
        || levels.iter().any(|x| !x.is_finite())
        || levels.windows(2).any(|w| w[0] >= w[1])
    {
        return Err("imquantize: levels must be a finite strictly increasing vector".into());
    }
    let values = a.get(2).map(|v| vector(name, v)).transpose()?;
    if values.as_ref().is_some_and(|v| v.len() != levels.len() + 1) {
        return Err("imquantize: values must have length numel(levels)+1".into());
    }
    let mut index = Vec::with_capacity(data.len());
    for (i, x) in data.iter().enumerate() {
        basic::poll(e, i)?;
        index.push((levels.partition_point(|t| x > t) + 1) as f64);
    }
    let idx = store(e, &a[0].dims(), index.clone(), Kind::Double)?;
    let result = if let Some(v) = values {
        store(
            e,
            &a[0].dims(),
            index.iter().map(|&i| v[i as usize - 1]).collect(),
            kind(&a[2]),
        )?
    } else {
        idx.clone()
    };
    Ok(vec![result, idx])
}

/// Adaptive entry also used by imbinarize's adaptive form. Public docs specify
/// statistics and sensitivity monotonicity, but not a numerical sensitivity
/// mapping or Gaussian sigma. We use a neutral local statistic at s=0.5,
/// multiplicative Bradley scaling 2*(1-s) (bright), 2*s (dark), and a Gaussian
/// truncated at three sigma. These explicit choices are recorded in the help.
pub(super) fn adaptive(e: &Engine, a: &[Value], binary: bool) -> Result<Value> {
    let name = if binary { "imbinarize" } else { "adaptthresh" };
    arity(name, a, 1, 10)?;
    bounded(e, name, a[0].dims().iter().product::<usize>(), 4)?;
    let dims = a[0].dims();
    if !(2..=3).contains(&dims.len()) {
        return Err(format!(
            "{name}: expected a 2-D image or 3-D grayscale volume"
        ));
    }
    let k = kind(&a[0]);
    if k == Kind::Logical || matches!(k, Kind::Typed(NumericClass::Int64 | NumericClass::Uint64)) {
        return Err(format!("{name}: unsupported input class"));
    }
    let mut data = nums(name, &a[0])?;
    let mut sensitivity = 0.5;
    let mut start = 1;
    if a.get(1).is_some_and(|v| basic::text(e, v).is_none()) {
        sensitivity = scalar(name, &a[1])?;
        start = 2;
    }
    let mut window: Vec<usize> = dims.iter().map(|d| 2 * (d / 16) + 1).collect();
    let mut dark = false;
    let mut statistic = "mean".to_owned();
    for (key, v) in pairs(e, name, &a[start..])? {
        match key.as_str() {
            "sensitivity" if binary => sensitivity = scalar(name, v)?,
            "neighborhoodsize" => {
                let mut w = vector(name, v)?;
                if w.len() == 1 {
                    w.resize(dims.len(), w[0]);
                }
                if w.len() != dims.len()
                    || w.iter().any(|x| {
                        !x.is_finite()
                            || *x < 1.
                            || *x > 1_000_001.
                            || x.fract() != 0.
                            || (*x as usize).is_multiple_of(2)
                    })
                {
                    return Err(format!(
                        "{name}: NeighborhoodSize must contain positive odd integers (at most 1000001)"
                    ));
                }
                window = w.iter().map(|&v| v as usize).collect();
            }
            "foregroundpolarity" => {
                dark = match text(e, name, v)?.as_str() {
                    "bright" => false,
                    "dark" => true,
                    _ => return Err(format!("{name}: ForegroundPolarity must be bright or dark")),
                }
            }
            "statistic" => {
                statistic = text(e, name, v)?;
                if !matches!(statistic.as_str(), "mean" | "median" | "gaussian") {
                    return Err(format!("{name}: unknown Statistic"));
                }
            }
            _ => return Err(format!("{name}: unknown option '{key}'")),
        }
    }
    if !(0. ..=1.).contains(&sensitivity) {
        return Err(format!("{name}: sensitivity must be in [0,1]"));
    }
    for (i, x) in data.iter_mut().enumerate() {
        basic::poll(e, i)?;
        *x = unit(k, *x);
        if !x.is_finite() {
            return Err(format!("{name}: image must be finite"));
        }
    }
    let original = if binary { data.clone() } else { Vec::new() };
    if !data.is_empty() {
        if statistic == "median" {
            let taps = window
                .iter()
                .try_fold(1usize, |p, &w| p.checked_mul(w))
                .ok_or_else(|| format!("{name}: NeighborhoodSize overflows"))?;
            bounded(e, name, taps, 1)?;
            if data.len().checked_mul(taps).is_none_or(|n| n > 100_000_000) {
                return Err(format!("{name}: median workspace/work limit exceeded"));
            }
            let mut filtered = vec![0.; data.len()];
            let strides: Vec<usize> = (0..dims.len())
                .map(|d| dims[..d].iter().product())
                .collect();
            let mut scratch = Vec::with_capacity(taps);
            for (i, dst) in filtered.iter_mut().enumerate() {
                if i.is_multiple_of(64) {
                    e.check_cancel()?;
                }
                scratch.clear();
                for tap in 0..taps {
                    if tap.is_multiple_of(65536) {
                        e.check_cancel()?;
                    }
                    let mut q = tap;
                    let mut pos = 0;
                    for d in 0..dims.len() {
                        let coord = i / strides[d] % dims[d];
                        let offset = q % window[d];
                        q /= window[d];
                        let index = (coord as isize + offset as isize - (window[d] / 2) as isize)
                            .clamp(0, dims[d] as isize - 1)
                            as usize;
                        pos += index * strides[d];
                    }
                    scratch.push(data[pos]);
                }
                let mid = scratch.len() / 2;
                scratch.select_nth_unstable_by(mid, f64::total_cmp);
                *dst = scratch[mid];
            }
            data = filtered;
        } else {
            for axis in 0..dims.len() {
                let w = window[axis];
                let radius = w / 2;
                let stride: usize = dims[..axis].iter().product();
                let len = dims[axis];
                let block = stride * len;
                let mut filtered = vec![0.; data.len()];
                let weights = if statistic == "gaussian" {
                    if data.len().checked_mul(w).is_none_or(|n| n > 100_000_000) {
                        return Err(format!("{name}: Gaussian work limit exceeded"));
                    }
                    let sigma = (radius as f64 / 3.).max(f64::EPSILON);
                    let mut g: Vec<f64> = (0..w)
                        .map(|j| (-0.5 * ((j as f64 - radius as f64) / sigma).powi(2)).exp())
                        .collect();
                    let sum: f64 = g.iter().sum();
                    for x in &mut g {
                        *x /= sum;
                    }
                    g
                } else {
                    Vec::new()
                };
                for base in (0..data.len()).step_by(block) {
                    for offset in 0..stride {
                        e.check_cancel()?;
                        let at = |coord: isize| {
                            data[base + offset + coord.clamp(0, len as isize - 1) as usize * stride]
                        };
                        if statistic == "mean" {
                            // Sliding sums with replicated edge multiplicities: O(pixels),
                            // even when a window is much larger than the image.
                            let last = radius.min(len - 1);
                            let mut sum = (radius + 1) as f64 * at(0);
                            for j in 1..=last {
                                basic::poll(e, j)?;
                                sum += at(j as isize);
                            }
                            if radius > last {
                                sum += (radius - last) as f64 * at(len as isize - 1);
                            }
                            for pos in 0..len {
                                basic::poll(e, pos)?;
                                filtered[base + offset + pos * stride] = sum / w as f64;
                                sum += at((pos + radius + 1) as isize)
                                    - at(pos as isize - radius as isize);
                            }
                        } else {
                            for pos in 0..len {
                                if pos.is_multiple_of(64) {
                                    e.check_cancel()?;
                                }
                                let mut sum = 0.;
                                for (j, &weight) in weights.iter().enumerate() {
                                    basic::poll(e, j)?;
                                    sum += weight * at(pos as isize + j as isize - radius as isize);
                                }
                                filtered[base + offset + pos * stride] = sum;
                            }
                        }
                    }
                }
                data = filtered;
            }
        }
    }
    let scale = if dark {
        2. * sensitivity
    } else {
        2. * (1. - sensitivity)
    };
    for (i, t) in data.iter_mut().enumerate() {
        basic::poll(e, i)?;
        *t = (*t * scale).clamp(0., 1.);
        if binary {
            *t = f64::from(if dark {
                original[i] < *t
            } else {
                original[i] > *t
            });
        }
    }
    store(
        e,
        &dims,
        data,
        if binary { Kind::Logical } else { Kind::Double },
    )
}

fn color(e: &Engine, v: &Value) -> Result<[f64; 3]> {
    if let Some(s) = basic::text(e, v) {
        return match s.to_ascii_lowercase().as_str() {
            "r" | "red" => Ok([1., 0., 0.]),
            "g" | "green" => Ok([0., 1., 0.]),
            "b" | "blue" => Ok([0., 0., 1.]),
            "c" | "cyan" => Ok([0., 1., 1.]),
            "m" | "magenta" => Ok([1., 0., 1.]),
            "y" | "yellow" => Ok([1., 1., 0.]),
            "k" | "black" => Ok([0.; 3]),
            "w" | "white" => Ok([1.; 3]),
            hex if hex.starts_with('#') && hex.is_ascii() && matches!(hex.len(), 4 | 7) => {
                let step = (hex.len() - 1) / 3;
                let mut rgb = [0.; 3];
                for (i, x) in rgb.iter_mut().enumerate() {
                    let p = 1 + i * step;
                    let v = u8::from_str_radix(&hex[p..p + step], 16)
                        .map_err(|_| "color: invalid hexadecimal color".to_owned())?;
                    *x = f64::from(v) / if step == 1 { 15. } else { 255. };
                }
                Ok(rgb)
            }
            _ => Err("color: unknown color name".into()),
        };
    }
    let n = vector("color", v)?;
    if n.len() != 3 || n.iter().any(|x| !x.is_finite() || !(0. ..=1.).contains(x)) {
        return Err("color: expected an RGB triplet in [0,1]".into());
    }
    Ok([n[0], n[1], n[2]])
}
fn labels(e: &Engine, name: &str, v: &Value) -> Result<(Img, usize)> {
    let img = gray(e, name, v)?;
    let mut max = 0.;
    for (i, &x) in img.data.iter().enumerate() {
        basic::poll(e, i)?;
        if !x.is_finite() || x < 0. || x.fract() != 0. || x > 1_000_000. {
            return Err(format!(
                "{name}: labels must be nonnegative integers at most 1000000"
            ));
        }
        max = f64::max(max, x);
    }
    Ok((img, max as usize))
}
fn colormap(e: &Engine, name: &str, arg: Option<&Value>, n: usize) -> Result<Vec<[f64; 3]>> {
    bounded(e, name, n, 3)?;
    let map_name = match arg {
        None => Some("jet".to_owned()),
        Some(v) => basic::text(e, v),
    };
    if let Some(s) = map_name {
        let s = s.to_ascii_lowercase();
        if matches!(s.as_str(), "spring" | "summer" | "autumn" | "winter") {
            return Ok((0..n)
                .map(|i| {
                    let t = if n <= 1 {
                        0.
                    } else {
                        i as f64 / (n - 1) as f64
                    };
                    match s.as_str() {
                        "spring" => [1., t, 1. - t],
                        "summer" => [t, 0.5 + 0.5 * t, 0.4],
                        "autumn" => [1., t, 0.],
                        _ => [0., t, 1. - 0.5 * t],
                    }
                })
                .collect());
        }
        return crate::plot::colormap::named(&s, n)?
            .ok_or_else(|| format!("{name}: unknown colormap '{s}'"));
    }
    let v = arg.expect("numeric colormap argument");
    let dims = v.dims();
    if dims.len() != 2 || dims[1] != 3 || dims[0] < n || !rgb_kind(kind(v), true) {
        return Err(format!(
            "{name}: colormap must be a floating P-by-3 matrix with at least {n} rows"
        ));
    }
    let data = nums(name, v)?;
    if data
        .iter()
        .any(|x| !x.is_finite() || !(0. ..=1.).contains(x))
    {
        return Err(format!("{name}: colormap values must be in [0,1]"));
    }
    Ok((0..n)
        .map(|i| [data[i], data[dims[0] + i], data[2 * dims[0] + i]])
        .collect())
}
fn shuffle(e: &mut Engine, map: &mut [[f64; 3]]) -> Result<()> {
    for i in (1..map.len()).rev() {
        basic::poll(e, i)?;
        let j = (e.random.next_f64() * (i + 1) as f64) as usize;
        map.swap(i, j);
    }
    Ok(())
}
fn labels_rgb(e: &mut Engine, a: &[Value]) -> Result<Value> {
    arity("label2rgb", a, 1, 4)?;
    let (img, max) = labels(e, "label2rgb", &a[0])?;
    basic::admit("label2rgb", img.h, img.w, 3)?;
    bounded(e, "label2rgb", img.data.len(), 4)?;
    let mut map = colormap(e, "label2rgb", a.get(1), max)?;
    let zero = a
        .get(2)
        .map(|v| color(e, v))
        .transpose()?
        .unwrap_or([1.; 3]);
    if let Some(v) = a.get(3) {
        match text(e, "label2rgb", v)?.as_str() {
            "shuffle" => shuffle(e, &mut map)?,
            "noshuffle" => (),
            _ => return Err("label2rgb: expected shuffle or noshuffle".into()),
        }
    }
    let n = img.data.len();
    let mut data = vec![0.; n * 3];
    for (i, &label) in img.data.iter().enumerate() {
        basic::poll(e, i)?;
        let c = if label == 0. {
            zero
        } else {
            map[label as usize - 1]
        };
        for k in 0..3 {
            data[k * n + i] = c[k] * 255.;
        }
    }
    store(
        e,
        &[img.h, img.w, 3],
        data,
        Kind::Typed(NumericClass::Uint8),
    )
}
fn overlay(e: &mut Engine, name: &str, a: &[Value]) -> Result<Value> {
    let burn = name == "imoverlay";
    arity(name, a, 2, if burn { 3 } else { 8 })?;
    let img = image(e, name, &a[0])?;
    if img.c != 1 && img.c != 3 {
        return Err(format!("{name}: expected a grayscale or RGB image"));
    }
    // The RGB result must fit the element limit before the mask/labels are converted.
    basic::admit(name, img.h, img.w, 3)?;
    if !rgb_kind(img.kind, false)
        && !matches!(
            img.kind,
            Kind::Logical | Kind::Typed(NumericClass::Int8 | NumericClass::Int16)
        )
    {
        return Err(format!("{name}: unsupported image class"));
    }
    let (labels, max) = if burn {
        (gray(e, name, &a[1])?, 1)
    } else {
        labels(e, name, &a[1])?
    };
    if img.h != labels.h || img.w != labels.w {
        return Err(format!("{name}: mask/labels must match the image size"));
    }
    bounded(e, name, img.h * img.w, 7)?;
    let mut transparency = 0.5;
    let mut map_arg = None;
    let mut included = None;
    if !burn {
        for (key, v) in pairs(e, name, &a[2..])? {
            match key.as_str() {
                "transparency" => {
                    transparency = scalar(name, v)?;
                    if !(0. ..=1.).contains(&transparency) {
                        return Err(format!("{name}: Transparency must be in [0,1]"));
                    }
                }
                "colormap" => map_arg = Some(v),
                "includedlabels" => {
                    let mut x = vector(name, v)?;
                    if x.iter()
                        .any(|x| !x.is_finite() || *x < 0. || *x > max as f64 || x.fract() != 0.)
                    {
                        return Err(format!(
                            "{name}: IncludedLabels must contain integers from 0 to max(L(:))"
                        ));
                    }
                    x.sort_by(f64::total_cmp);
                    x.dedup();
                    included = Some(x);
                }
                _ => return Err(format!("{name}: unknown option '{key}'")),
            }
        }
    }
    let include_zero = included.as_ref().is_some_and(|s| s.first() == Some(&0.));
    let mut map = if burn {
        transparency = 0.;
        vec![
            a.get(2)
                .map(|v| color(e, v))
                .transpose()?
                .unwrap_or([1., 1., 0.]),
        ]
    } else {
        colormap(e, name, map_arg, max + usize::from(include_zero))?
    };
    if !burn && map_arg.is_none_or(|v| basic::text(e, v).is_some()) {
        shuffle(e, &mut map)?;
    }
    // The RGB result must fit the memory budget before it is allocated.
    let n = img.h * img.w;
    e.preflight_elements(3 * n)?;
    let mut data = vec![0.; 3 * n];
    for (i, &label) in labels.data.iter().enumerate() {
        basic::poll(e, i)?;
        let visible = if burn {
            label != 0.
        } else if let Some(s) = &included {
            s.binary_search_by(|v| v.total_cmp(&label)).is_ok()
        } else {
            label != 0.
        };
        for k in 0..3 {
            let pixel =
                unit(img.kind, img.data[i + if img.c == 3 { k * n } else { 0 }]).clamp(0., 1.);
            let result = if visible {
                let index = if burn {
                    0
                } else if label == 0. {
                    max
                } else {
                    label as usize - 1
                };
                transparency * pixel + (1. - transparency) * map[index][k]
            } else {
                pixel
            };
            data[k * n + i] = result * 255.;
        }
    }
    store(
        e,
        &[img.h, img.w, 3],
        data,
        Kind::Typed(NumericClass::Uint8),
    )
}
fn boundary(e: &Engine, a: &[Value]) -> Result<Value> {
    arity("boundarymask", a, 1, 2)?;
    let img = gray(e, "boundarymask", &a[0])?;
    if img.data.iter().any(|x| !x.is_finite() || *x < 0.) {
        return Err("boundarymask: labels must be finite nonnegative numbers".into());
    }
    let conn = if let Some(v) = a.get(1) {
        scalar("boundarymask", v)?
    } else {
        8.
    };
    if conn != 4. && conn != 8. {
        return Err("boundarymask: connectivity must be 4 or 8".into());
    }
    let mut data = vec![0.; img.data.len()];
    for col in 0..img.w {
        for row in 0..img.h {
            let i = row + col * img.h;
            basic::poll(e, i)?;
            for dc in -1isize..=1 {
                for dr in -1isize..=1 {
                    if dr == 0 && dc == 0 || conn == 4. && dr != 0 && dc != 0 {
                        continue;
                    }
                    let r = row as isize + dr;
                    let c = col as isize + dc;
                    // Only neighbors inside the image count; no artificial frame.
                    if r >= 0
                        && c >= 0
                        && r < img.h as isize
                        && c < img.w as isize
                        && img.data[i] != img.data[r as usize + c as usize * img.h]
                    {
                        data[i] = 1.;
                    }
                }
            }
        }
    }
    store(e, &[img.h, img.w], data, Kind::Logical)
}
fn pixels(e: &Engine, a: &[Value], count: usize) -> Result<Vec<Value>> {
    if a.len() < 3 {
        return Err("impixel: interactive pixel selection is not supported; use impixel(I,c,r) or impixel(X,map,c,r)".into());
    }
    arity("impixel", a, 3, 4)?;
    let img = image(e, "impixel", &a[0])?;
    if img.c != 1 && img.c != 3 {
        return Err("impixel: expected a grayscale or RGB image".into());
    }
    let indexed = a.len() == 4;
    if indexed && img.c != 1 {
        return Err("impixel: indexed image must be 2-D".into());
    }
    let c = vector("impixel", &a[a.len() - 2])?;
    let r = vector("impixel", &a[a.len() - 1])?;
    if c.len() != r.len() {
        return Err("impixel: c and r must have equal lengths".into());
    }
    bounded(e, "impixel", c.len(), 3)?;
    let map = if indexed {
        if basic::text(e, &a[1]).is_some() {
            return Err("impixel: colormap must be a numeric P-by-3 matrix".into());
        }
        Some(colormap(e, "impixel", Some(&a[1]), a[1].dims()[0])?)
    } else {
        None
    };
    if map.as_ref().is_some_and(Vec::is_empty) {
        return Err("impixel: colormap must not be empty".into());
    }
    let mut data = vec![f64::NAN; 3 * c.len()];
    for i in 0..c.len() {
        basic::poll(e, i)?;
        let (x, y) = (c[i].round(), r[i].round());
        if !x.is_finite()
            || !y.is_finite()
            || x < 1.
            || y < 1.
            || x > img.w as f64
            || y > img.h as f64
        {
            continue;
        }
        let index = y as usize - 1 + (x as usize - 1) * img.h;
        for channel in 0..3 {
            data[channel * c.len() + i] = if let Some(map) = &map {
                let zero_based = matches!(
                    img.kind,
                    Kind::Typed(NumericClass::Uint8 | NumericClass::Uint16)
                );
                let row = img.data[index] - if zero_based { 0. } else { 1. };
                if !row.is_finite() {
                    f64::NAN
                } else {
                    map[row.floor().clamp(0., (map.len() - 1) as f64) as usize][channel]
                }
            } else {
                img.data[index
                    + if img.c == 3 {
                        channel * img.h * img.w
                    } else {
                        0
                    }]
            };
        }
    }
    let p = store(
        e,
        &[c.len(), 3],
        data,
        if indexed { kind(&a[1]) } else { Kind::Double },
    )?;
    if count == 1 {
        Ok(vec![p])
    } else {
        Ok(vec![
            store(e, &[c.len(), 1], c, Kind::Double)?,
            store(e, &[r.len(), 1], r, Kind::Double)?,
            p,
        ])
    }
}
