//! histeq (histogram equalisation and matching) and edge (gradient and Canny
//! edge detection).
//!
//! Clean room: histeq follows the documented rule — pick the monotonic map T
//! that brings the cumulative target histogram closest to the cumulative input
//! histogram, never missing it by more than half the input count of the level. edge follows Sobel/Prewitt/Roberts gradient operators (Gonzalez &
//! Woods, "Digital Image Processing", 3rd ed., 2008, §10.2) with thinning to
//! one pixel along the dominant gradient axis and Canny, "A computational
//! approach to edge detection", IEEE PAMI 8(6), 1986 (Gaussian smoothing,
//! non-maximum suppression, hysteresis). Automatic thresholds: four times the
//! mean squared gradient for the gradient methods; for Canny the high threshold
//! leaves 70 % of the pixels below it and the low one is 0.4 of the high one.
use super::hist::{bin, counts};
use super::{Cx, Kind, check_elements, image_2d, mask_value, numbers, scalar, text_of, value};
use crate::Result;
use crate::engine::Value;

/// Input histogram bins histeq works on.
const BINS: usize = 256;

/// `[J, T] = histeq(I, n | hgram)`.
pub(super) fn histeq(cx: &Cx, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() > 2 {
        return cx.err("expected histeq(I), histeq(I, n) or histeq(I, hgram)");
    }
    let img = image_2d(cx, &args[0])?;
    let target = match args.get(1) {
        None => vec![1.; 64],
        Some(v) => {
            let values = numbers(cx, v, "the second argument")?;
            match values.as_slice() {
                [n] if *n >= 2. && n.fract() == 0. && *n <= 65536. => vec![1.; *n as usize],
                [_] => return cx.err("n must be an integer from 2 to 65536"),
                h if h.iter().all(|x| x.is_finite() && *x >= 0.) && h.iter().any(|x| *x > 0.) => {
                    h.to_vec()
                }
                _ => return cx.err("hgram must be a vector of nonnegative counts, not all zero"),
            }
        }
    };
    let total = img.data.iter().filter(|v| !v.is_nan()).count() as f64;
    let h0 = counts(cx, &img, BINS)?;
    let sum: f64 = target.iter().sum();
    let mut c1 = Vec::with_capacity(target.len());
    let mut acc = 0.;
    for h in &target {
        acc += h * total / sum;
        c1.push(acc);
    }
    let levels = (target.len() - 1).max(1) as f64;
    let mut map = Vec::with_capacity(BINS);
    let mut c0 = 0.;
    for (k, &h) in h0.iter().enumerate() {
        cx.poll(k)?;
        c0 += h;
        let mut best = (f64::INFINITY, 0);
        for (j, &c) in c1.iter().enumerate() {
            let err = c - c0;
            // Level j may fall short of c0 by at most half the count of bin k;
            // ties keep the darker level.
            if err >= -h / 2. - 1e-9 * total.max(1.) && err.abs() < best.0 {
                best = (err.abs(), j);
            }
        }
        map.push(best.1 as f64 / levels);
    }
    let (low, width) = img.kind.range();
    let out_kind = if img.kind == Kind::Logical {
        Kind::Double
    } else {
        img.kind
    };
    let data = img
        .data
        .iter()
        .map(|&v| match bin(img.kind, BINS, v) {
            None => f64::NAN,
            Some(k) => match out_kind {
                Kind::Int(_) => (low + map[k] * width).round(),
                _ => map[k],
            },
        })
        .collect();
    let j = value(cx, img.with(data, out_kind))?;
    Ok(vec![j, super::double(1, BINS, map)?])
}

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Sobel,
    Prewitt,
    Roberts,
    Canny,
}

/// `[BW, threshOut] = edge(I, method, threshold, direction | sigma)`.
pub(super) fn edge(cx: &Cx, args: &[Value]) -> Result<Vec<Value>> {
    let img = image_2d(cx, &args[0])?;
    let (rows, cols) = (img.rows(), img.cols());
    check_elements(cx, rows * cols)?;
    let mut method = Method::Sobel;
    let mut rest = &args[1..];
    if let Some(name) = rest.first().and_then(text_of) {
        method = match name.to_ascii_lowercase().as_str() {
            "sobel" => Method::Sobel,
            "prewitt" => Method::Prewitt,
            "roberts" => Method::Roberts,
            "canny" => Method::Canny,
            "log" | "zerocross" | "approxcanny" => {
                return cx.err(format!(
                    "the '{name}' method is not supported yet; use sobel, prewitt, roberts or canny"
                ));
            }
            _ => return cx.err(format!("unknown method '{name}'")),
        };
        rest = &rest[1..];
    }
    let mut threshold = None;
    let mut sigma = std::f64::consts::SQRT_2;
    let mut direction = "both".to_string();
    let mut thinning = true;
    for (i, arg) in rest.iter().enumerate() {
        if let Some(word) = text_of(arg) {
            match word.to_ascii_lowercase().as_str() {
                d @ ("both" | "horizontal" | "vertical") if method != Method::Canny => {
                    direction = d.into()
                }
                "thinning" if method != Method::Canny => thinning = true,
                "nothinning" if method != Method::Canny => thinning = false,
                _ => return cx.err(format!("unsupported option '{word}'")),
            }
        } else if i == 0 {
            let t = numbers(cx, arg, "threshold")?;
            threshold = match (method, t.as_slice()) {
                (_, []) => None,
                (_, [t]) if *t >= 0. => Some([f64::NAN, *t]),
                (Method::Canny, [lo, hi]) if 0. <= *lo && lo < hi && *hi < 1. => Some([*lo, *hi]),
                (Method::Canny, _) => {
                    return cx
                        .err("threshold must be a scalar or [low high] with 0 <= low < high < 1");
                }
                _ => return cx.err("threshold must be a nonnegative scalar"),
            };
        } else if i == 1 && method == Method::Canny {
            sigma = scalar(cx, arg, "sigma")?;
            if !(sigma > 0. && sigma <= 64.) {
                return cx.err("sigma must be positive and at most 64");
            }
        } else {
            return cx.err("too many input arguments");
        }
    }
    // Intensities on [0, 1] whatever the class.
    let (low, width) = img.kind.range();
    let a: Vec<f64> = img.data.iter().map(|v| (v - low) / width).collect();
    if a.iter().any(|v| !v.is_finite()) {
        return cx.err("the image must not contain NaN or Inf");
    }
    let (mask, thresh) = if method == Method::Canny {
        canny(cx, &a, rows, cols, threshold, sigma)?
    } else {
        gradient_edges(cx, &a, rows, cols, method, threshold, &direction, thinning)?
    };
    let thresh = super::double(1, thresh.len(), thresh)?;
    Ok(vec![mask_value(cx, rows, cols, &mask)?, thresh])
}

/// Replicated-border sample of a column-major image.
fn at(a: &[f64], rows: usize, cols: usize, r: isize, c: isize) -> f64 {
    let r = r.clamp(0, rows as isize - 1) as usize;
    let c = c.clamp(0, cols as isize - 1) as usize;
    a[r + c * rows]
}

#[allow(clippy::too_many_arguments)]
fn gradient_edges(
    cx: &Cx,
    a: &[f64],
    rows: usize,
    cols: usize,
    method: Method,
    threshold: Option<[f64; 2]>,
    direction: &str,
    thinning: bool,
) -> Result<(Vec<bool>, Vec<f64>)> {
    let n = rows * cols;
    let (mut gx, mut gy) = (vec![0.; n], vec![0.; n]);
    for c in 0..cols {
        for r in 0..rows {
            let i = r + c * rows;
            cx.poll(i)?;
            let p = |dr: isize, dc: isize| at(a, rows, cols, r as isize + dr, c as isize + dc);
            (gx[i], gy[i]) = match method {
                Method::Roberts => {
                    let d1 = p(0, 0) - p(1, 1);
                    let d2 = p(0, 1) - p(1, 0);
                    // The diagonal pair rotated onto x and y: same magnitude as
                    // the two half-scaled Roberts kernels, plus an axis for thinning.
                    use std::f64::consts::SQRT_2;
                    ((d2 - d1) / (2. * SQRT_2), (d1 + d2) / (2. * SQRT_2))
                }
                _ => {
                    let w = if method == Method::Sobel { 2. } else { 1. };
                    let scale = if method == Method::Sobel { 8. } else { 6. };
                    let x =
                        (p(-1, 1) + w * p(0, 1) + p(1, 1)) - (p(-1, -1) + w * p(0, -1) + p(1, -1));
                    let y =
                        (p(1, -1) + w * p(1, 0) + p(1, 1)) - (p(-1, -1) + w * p(-1, 0) + p(-1, 1));
                    (x / scale, y / scale)
                }
            };
        }
    }
    let (use_x, use_y) = match direction {
        "horizontal" => (false, true),
        "vertical" => (true, false),
        _ => (true, true),
    };
    let m: Vec<f64> = (0..n)
        .map(|i| {
            let x = if use_x { gx[i] * gx[i] } else { 0. };
            let y = if use_y { gy[i] * gy[i] } else { 0. };
            x + y
        })
        .collect();
    let cutoff = match threshold {
        Some([_, t]) => t * t,
        None => 4. * m.iter().sum::<f64>() / n.max(1) as f64,
    };
    let mut mask = vec![false; n];
    for c in 0..cols {
        for r in 0..rows {
            let i = r + c * rows;
            let v = m[i];
            if v <= cutoff || v == 0. {
                continue;
            }
            mask[i] = !thinning || {
                let get =
                    |dr: isize, dc: isize| at(&m, rows, cols, r as isize + dr, c as isize + dc);
                let (ax, ay) = (
                    if use_x { gx[i].abs() } else { 0. },
                    if use_y { gy[i].abs() } else { 0. },
                );
                // A local maximum across the edge; on a two-pixel plateau the
                // first pixel (left or top) wins.
                (ax >= ay && get(0, -1) < v && v >= get(0, 1))
                    || (ay >= ax && get(-1, 0) < v && v >= get(1, 0))
            };
        }
    }
    Ok((mask, vec![cutoff.sqrt()]))
}

/// Separable Gaussian smoothing with replicated borders.
fn smooth(cx: &Cx, a: &[f64], rows: usize, cols: usize, sigma: f64) -> Result<Vec<f64>> {
    let radius = (3. * sigma).ceil() as isize;
    let kernel: Vec<f64> = (-radius..=radius)
        .map(|x| (-(x * x) as f64 / (2. * sigma * sigma)).exp())
        .collect();
    let total: f64 = kernel.iter().sum();
    let pass = |src: &[f64], along_rows: bool| -> Result<Vec<f64>> {
        let mut out = vec![0.; src.len()];
        for c in 0..cols {
            for r in 0..rows {
                let i = r + c * rows;
                cx.poll(i)?;
                out[i] = kernel
                    .iter()
                    .zip(-radius..=radius)
                    .map(|(w, d)| {
                        let (rr, cc) = if along_rows {
                            (r as isize + d, c as isize)
                        } else {
                            (r as isize, c as isize + d)
                        };
                        w * at(src, rows, cols, rr, cc)
                    })
                    .sum::<f64>()
                    / total;
            }
        }
        Ok(out)
    };
    let first = pass(a, true)?;
    pass(&first, false)
}

fn canny(
    cx: &Cx,
    a: &[f64],
    rows: usize,
    cols: usize,
    threshold: Option<[f64; 2]>,
    sigma: f64,
) -> Result<(Vec<bool>, Vec<f64>)> {
    let n = rows * cols;
    let s = smooth(cx, a, rows, cols, sigma)?;
    let (mut gx, mut gy, mut mag) = (vec![0.; n], vec![0.; n], vec![0.; n]);
    for c in 0..cols {
        for r in 0..rows {
            let i = r + c * rows;
            let p = |dr: isize, dc: isize| at(&s, rows, cols, r as isize + dr, c as isize + dc);
            gx[i] = (p(0, 1) - p(0, -1)) / 2.;
            gy[i] = (p(1, 0) - p(-1, 0)) / 2.;
            mag[i] = gx[i].hypot(gy[i]);
        }
    }
    let max = mag.iter().copied().fold(0., f64::max);
    if max > 0. {
        mag.iter_mut().for_each(|v| *v /= max);
    }
    let [lo, hi] = match threshold {
        Some([lo, hi]) if !lo.is_nan() => [lo, hi],
        Some([_, hi]) => [0.4 * hi, hi],
        None => {
            // 64-bin histogram of the normalised magnitude; the high threshold
            // is the upper edge of the bin where 70 % of pixels are passed.
            let mut hist = [0usize; 64];
            for &v in &mag {
                hist[((v * 64.) as usize).min(63)] += 1;
            }
            let mut acc = 0;
            let mut k = 64;
            for (j, h) in hist.iter().enumerate() {
                acc += h;
                if acc as f64 > 0.7 * n as f64 {
                    k = j + 1;
                    break;
                }
            }
            let hi = k as f64 / 64.;
            [0.4 * hi, hi]
        }
    };
    // Non-maximum suppression along the gradient direction (four sectors).
    let mut keep = vec![false; n];
    for c in 0..cols {
        for r in 0..rows {
            let i = r + c * rows;
            cx.poll(i)?;
            let v = mag[i];
            if v <= lo {
                continue;
            }
            let angle = gy[i].atan2(gx[i]).to_degrees().rem_euclid(180.);
            let (dr, dc) = if !(22.5..157.5).contains(&angle) {
                (0, 1)
            } else if angle < 67.5 {
                (1, 1)
            } else if angle < 112.5 {
                (1, 0)
            } else {
                (1, -1)
            };
            let get = |sr: isize, sc: isize| {
                let (rr, cc) = (r as isize + sr, c as isize + sc);
                if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                    0.
                } else {
                    mag[rr as usize + cc as usize * rows]
                }
            };
            // The tolerance absorbs rounding on symmetric plateaus.
            keep[i] = v > get(-dr, -dc) + 1e-12 && v + 1e-12 >= get(dr, dc);
        }
    }
    // Hysteresis: weak pixels survive when 8-connected to a strong one.
    let mut mask = vec![false; n];
    let mut stack: Vec<usize> = (0..n).filter(|&i| keep[i] && mag[i] > hi).collect();
    for &i in &stack {
        mask[i] = true;
    }
    let mut steps = 0;
    while let Some(i) = stack.pop() {
        steps += 1;
        cx.poll(steps)?;
        let (r, c) = ((i % rows) as isize, (i / rows) as isize);
        for dr in -1..=1 {
            for dc in -1..=1 {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                    continue;
                }
                let j = rr as usize + cc as usize * rows;
                if keep[j] && !mask[j] {
                    mask[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    Ok((mask, vec![lo, hi]))
}
