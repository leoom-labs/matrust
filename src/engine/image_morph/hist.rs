//! imhist, stretchlim, imadjust, graythresh, otsuthresh and imbinarize.
use super::{Cx, Img, Kind, check_elements, double, image, image_2d, numbers, text_of, value};
use crate::engine::{Engine, Execution, Value};
use crate::{FunctionHandle, Result};

/// Histogram of all elements of `img` in `n` bins over the class range.
/// Bin p (1-based) holds A(p-1.5)/(n-1) <= x - low < A(p-0.5)/(n-1), with A
/// the width of the class range; values beyond the range fall in the first
/// or last bin and NaN is not counted.
pub(super) fn counts(cx: &Cx, img: &Img, n: usize) -> Result<Vec<f64>> {
    let mut counts = vec![0.; n];
    for (i, &v) in img.data.iter().enumerate() {
        cx.poll(i)?;
        if let Some(k) = bin(img.kind, n, v) {
            counts[k] += 1.;
        }
    }
    Ok(counts)
}

/// The 0-based bin of `v` among `n` bins over the class range (None for NaN).
pub(super) fn bin(kind: Kind, n: usize, v: f64) -> Option<usize> {
    if v.is_nan() {
        return None;
    }
    let (low, width) = kind.range();
    let steps = (n - 1) as f64;
    let pos = ((v - low) * steps / width + 0.5).floor();
    Some(if pos <= 0. {
        0
    } else if pos >= steps {
        n - 1
    } else {
        pos as usize
    })
}

fn centres(kind: Kind, n: usize) -> Vec<f64> {
    let (low, width) = kind.range();
    if n == 1 {
        return vec![low];
    }
    (0..n)
        .map(|k| low + width * k as f64 / (n - 1) as f64)
        .collect()
}

/// Returns (counts, bin locations, range low, range high).
pub(super) fn imhist(cx: &Cx, args: &[Value]) -> Result<(Value, Value, f64, f64)> {
    if args.len() > 2 {
        return cx.err("expected imhist(I) or imhist(I, n)");
    }
    let img = image_2d(cx, &args[0])?;
    let n = match args.get(1) {
        None => {
            if img.kind == Kind::Logical {
                2
            } else {
                256
            }
        }
        Some(v) => {
            let values = numbers(cx, v, "n")?;
            if values.len() != 1 {
                return cx.err("indexed images with a colormap are not supported yet");
            }
            let n = values[0];
            if !(n >= 1. && n.fract() == 0. && n <= 65536. * 16.) {
                return cx.err("n must be a positive integer of at most 1048576");
            }
            n as usize
        }
    };
    let counts = counts(cx, &img, n)?;
    let x = centres(img.kind, n);
    let (low, width) = img.kind.range();
    Ok((double(n, 1, counts)?, double(n, 1, x)?, low, low + width))
}

/// imhist without outputs: a stem plot of the counts over the class range.
pub(super) fn draw(
    engine: &mut Engine,
    counts: Value,
    centres: Value,
    low: f64,
    high: f64,
    out: &mut Execution,
) -> Result<()> {
    let stem = FunctionHandle::named("stem");
    let args = vec![
        centres,
        counts,
        Value::Text("Marker".into()),
        Value::Text("none".into()),
    ];
    engine.call_handle(&stem, args, 0, None, out)?;
    let xlim = FunctionHandle::named("xlim");
    let limits = Value::Matrix(crate::Matrix::from_column_vec(1, 2, vec![low, high])?);
    engine.call_handle(&xlim, vec![limits], 0, None, out)?;
    Ok(())
}

/// (tol_low, tol_high) from stretchlim's tol argument.
fn tolerance(cx: &Cx, arg: Option<&Value>) -> Result<(f64, f64)> {
    let (lo, hi) = match arg {
        None => (0.01, 0.99),
        Some(v) => match numbers(cx, v, "tol")?.as_slice() {
            [] => (0.01, 0.99),
            [t] => (*t, 1. - *t),
            [a, b] => (*a, *b),
            _ => return cx.err("tol must be a scalar or a two-element vector"),
        },
    };
    if !(0. ..=1.).contains(&lo) || !(0. ..=1.).contains(&hi) || lo > hi {
        return cx.err("tol must satisfy 0 <= tol_low <= tol_high <= 1");
    }
    Ok((lo, hi))
}

/// Bins stretchlim uses: 256 for 8-bit and floating-point classes, 65536 for
/// wider integer classes.
fn stretch_bins(kind: Kind) -> usize {
    match kind {
        Kind::Int(c) if c.size() > 1 => 65536,
        _ => 256,
    }
}

/// Limits [low; high] of one channel: the first bin whose cumulative fraction
/// exceeds tol_low and the first that reaches tol_high, as fractions of the
/// class range; [0; 1] when both are the same bin (a constant image).
fn channel_limits(cx: &Cx, img: &Img, tol: (f64, f64)) -> Result<[f64; 2]> {
    let n = stretch_bins(img.kind);
    let counts = counts(cx, img, n)?;
    let total: f64 = counts.iter().sum();
    if total == 0. {
        return Ok([0., 1.]);
    }
    let mut cum = 0.;
    let mut low = None;
    let mut high = None;
    for (k, c) in counts.iter().enumerate() {
        cum += c;
        let f = cum / total;
        if low.is_none() && f > tol.0 {
            low = Some(k);
        }
        if high.is_none() && f >= tol.1 {
            high = Some(k);
        }
    }
    let low = low.unwrap_or(n - 1);
    let high = high.unwrap_or(n - 1);
    if low >= high {
        return Ok([0., 1.]);
    }
    let steps = (n - 1) as f64;
    Ok([low as f64 / steps, high as f64 / steps])
}

/// The planes of a gray (one plane, no copy) or RGB (three planes) image.
fn channels(cx: &Cx, img: Img) -> Result<Vec<Img>> {
    match img.dims.len() {
        2 => Ok(vec![img]),
        3 if img.dims[2] == 3 => {
            let n = img.rows() * img.cols();
            Ok((0..3)
                .map(|c| Img {
                    dims: vec![img.rows(), img.cols()],
                    data: img.data[c * n..(c + 1) * n].to_vec(),
                    kind: img.kind,
                })
                .collect())
        }
        _ => cx.err("the image must be 2-D grayscale or M-by-N-by-3 RGB"),
    }
}

/// Admits the double working copies of `value` before any is made: the image
/// itself, the split planes of an RGB image, and `outputs` result images.
fn admit_scratch(cx: &Cx, value: &Value, outputs: usize) -> Result<()> {
    let dims = value.dims();
    let n = dims
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .unwrap_or(usize::MAX);
    let split = usize::from(dims.len() == 3 && dims[2] == 3);
    check_elements(cx, n.saturating_mul(1 + split + outputs))
}

pub(super) fn stretchlim(cx: &Cx, args: &[Value]) -> Result<Value> {
    if args.len() > 2 {
        return cx.err("expected stretchlim(I) or stretchlim(I, tol)");
    }
    admit_scratch(cx, &args[0], 0)?;
    let img = image(cx, &args[0])?;
    let tol = tolerance(cx, args.get(1))?;
    let planes = channels(cx, img)?;
    let mut data = Vec::with_capacity(2 * planes.len());
    for p in &planes {
        data.extend(channel_limits(cx, p, tol)?);
    }
    double(2, planes.len(), data)
}

/// A 2-by-channels limit matrix from [], a 2-vector or a 2-by-3 matrix.
fn limits(cx: &Cx, arg: Option<&Value>, channels: usize, what: &str) -> Result<Vec<[f64; 2]>> {
    let Some(v) = arg else {
        return Ok(vec![[0., 1.]; channels]);
    };
    let values = numbers(cx, v, what)?;
    let pairs = match values.len() {
        0 => vec![[0., 1.]; channels],
        2 => vec![[values[0], values[1]]; channels],
        6 if channels == 3 && v.dims().first() == Some(&2) => {
            (0..3).map(|c| [values[2 * c], values[2 * c + 1]]).collect()
        }
        _ => {
            return cx.err(format!(
                "{what} must be [], a two-element vector or (for RGB) a 2-by-3 matrix"
            ));
        }
    };
    if pairs.iter().flatten().any(|v| !(0. ..=1.).contains(v)) {
        return cx.err(format!("{what} values must be in the range [0, 1]"));
    }
    Ok(pairs)
}

pub(super) fn imadjust(cx: &Cx, args: &[Value]) -> Result<Value> {
    if args.len() > 4 {
        return cx.err("expected imadjust(I, [low_in high_in], [low_out high_out], gamma)");
    }
    admit_scratch(cx, &args[0], 1)?;
    let img = image(cx, &args[0])?;
    if img.kind == Kind::Logical {
        return cx.err("logical images are not supported; use a numeric image");
    }
    let (dims, kind) = (img.dims.clone(), img.kind);
    let planes = channels(cx, img)?;
    let count = planes.len();
    let inputs = match args.get(1) {
        None => planes
            .iter()
            .map(|p| channel_limits(cx, p, (0.01, 0.99)))
            .collect::<Result<Vec<_>>>()?,
        Some(v) => limits(cx, Some(v), count, "[low_in high_in]")?,
    };
    let outputs = limits(cx, args.get(2), count, "[low_out high_out]")?;
    let gammas = match args.get(3) {
        None => vec![1.; count],
        Some(v) => match numbers(cx, v, "gamma")?.as_slice() {
            [g] => vec![*g; count],
            [a, b, c] if count == 3 => vec![*a, *b, *c],
            _ => return cx.err("gamma must be a scalar (or 1-by-3 for RGB)"),
        },
    };
    if gammas
        .iter()
        .any(|g| g.is_nan() || *g < 0. || !g.is_finite())
    {
        return cx.err("gamma must be a nonnegative finite value");
    }
    for [lo, hi] in &inputs {
        if lo >= hi {
            return cx.err("low_in must be less than high_in");
        }
    }
    let (low, width) = kind.range();
    let plane = dims[0] * dims[1];
    let mut data = Vec::with_capacity(plane * count);
    for (c, p) in planes.iter().enumerate() {
        let [li, hi] = inputs[c];
        let [lo, ho] = outputs[c];
        let g = gammas[c];
        for (i, &v) in p.data.iter().enumerate() {
            cx.poll(i)?;
            let x = (v - low) / width;
            let y = if x.is_nan() {
                f64::NAN
            } else {
                let t = ((x.clamp(li, hi)) - li) / (hi - li);
                lo + (ho - lo) * t.powf(g)
            };
            data.push(match kind {
                Kind::Int(_) => y * width + low,
                _ => y,
            });
        }
        debug_assert_eq!(data.len(), (c + 1) * plane);
    }
    value(cx, Img { dims, data, kind })
}

/// Otsu's threshold of a histogram: (level in [0, 1], effectiveness metric).
/// The between-class variance is maximised over the bin boundaries; ties
/// take the mean of the maximising bins. A histogram with a single occupied
/// bin (or none) gives (0, 0).
pub(crate) fn otsu(counts: &[f64]) -> (f64, f64) {
    let n = counts.len();
    let total: f64 = counts.iter().sum();
    if n < 2 || total.is_nan() || total <= 0. {
        return (0., 0.);
    }
    let p: Vec<f64> = counts.iter().map(|c| c / total).collect();
    let mu_t: f64 = p.iter().enumerate().map(|(i, v)| v * (i + 1) as f64).sum();
    let mut omega = 0.;
    let mut mu = 0.;
    let mut sigma = Vec::with_capacity(n);
    for (i, v) in p.iter().enumerate() {
        omega += v;
        mu += v * (i + 1) as f64;
        let num = mu_t * omega - mu;
        let den = omega * (1. - omega);
        sigma.push(num * num / den);
    }
    let max = sigma
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if !max.is_finite() {
        return (0., 0.);
    }
    let ties: Vec<usize> = (0..n).filter(|&i| sigma[i] == max).collect();
    let idx = ties.iter().map(|&i| (i + 1) as f64).sum::<f64>() / ties.len() as f64;
    let level = (idx - 1.) / (n - 1) as f64;
    let var: f64 = p
        .iter()
        .enumerate()
        .map(|(i, v)| v * ((i + 1) as f64 - mu_t).powi(2))
        .sum();
    let em = if var > 0. { max / var } else { 0. };
    (level, em)
}

pub(super) fn graythresh(cx: &Cx, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() > 1 {
        return cx.err("expected graythresh(I)");
    }
    let img = image(cx, &args[0])?;
    let counts = counts(cx, &img, 256)?;
    let (level, em) = otsu(&counts);
    Ok(vec![
        Value::Matrix(crate::Matrix::scalar(level)),
        Value::Matrix(crate::Matrix::scalar(em)),
    ])
}

pub(super) fn otsuthresh(cx: &Cx, args: &[Value]) -> Result<Value> {
    if args.len() > 1 {
        return cx.err("expected otsuthresh(counts)");
    }
    let counts = numbers(cx, &args[0], "counts")?;
    let dims = args[0].dims();
    if dims.len() > 2 || (dims[0] != 1 && dims[1] != 1 && !counts.is_empty()) {
        return cx.err("counts must be a vector");
    }
    if counts
        .iter()
        .any(|c| c.is_nan() || *c < 0. || !c.is_finite())
    {
        return cx.err("counts must be nonnegative and finite");
    }
    Ok(Value::Matrix(crate::Matrix::scalar(otsu(&counts).0)))
}

pub(super) fn imbinarize(cx: &Cx, args: &[Value]) -> Result<Value> {
    let img = image(cx, &args[0])?;
    check_elements(cx, img.len())?;
    let (low, width) = img.kind.range();
    let global = |img: &Img| -> Result<f64> {
        let counts = counts(cx, img, 256)?;
        Ok(otsu(&counts).0)
    };
    // Per-element thresholds in [0, 1] (one value, or one per element).
    let thresholds: Vec<f64> = match args.get(1) {
        None => vec![global(&img)?],
        Some(v) => {
            if let Some(method) = text_of(v) {
                match method.to_ascii_lowercase().as_str() {
                    "global" if args.len() == 2 => vec![global(&img)?],
                    "global" => return cx.err("'global' takes no further options"),
                    "adaptive" => {
                        return cx.err("'adaptive' thresholding is not supported yet");
                    }
                    other => return cx.err(format!("unknown method '{other}'")),
                }
            } else {
                if args.len() > 2 {
                    return cx.err("expected imbinarize(I, T)");
                }
                let t = numbers(cx, v, "T")?;
                if t.len() != 1 && v.dims().as_ref() != img.dims.as_slice() {
                    return cx.err("T must be a scalar or the same size as I");
                }
                t
            }
        }
    };
    let data = img
        .data
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let t = if thresholds.len() == 1 {
                thresholds[0]
            } else {
                thresholds[i]
            };
            f64::from(u8::from(x > t * width + low))
        })
        .collect();
    value(cx, img.with(data, Kind::Logical))
}
