//! fspecial kernels and the imfilter, imgaussfilt and medfilt2 filters.
//! Correlation is a direct sum over the kernel on a padded copy of each plane;
//! large images split their output columns over the worker pool.
use super::{Img, Kind, POLL, cancelled, image, numbers, poll_column, text, to_value};
use crate::engine::{Engine, Value};
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::sync::atomic::AtomicBool;

/// Multiply-adds above which a correlation uses the worker pool. Measured
/// (2048x2048 double, 4-vCPU Xeon VM, 2026-11-03): the class conversion and
/// padding around the kernel dominate, so 4 workers were slower up to 9x9
/// kernels (125 vs 102 ms at 3x3) and faster only at 15x15 (187 vs 221 ms);
/// only that much work per image (about 2^28 multiply-adds) is split.
const PAR_WORK: usize = 1 << 28;
/// Comparisons above which medfilt2 uses the worker pool (selection per pixel
/// is costly: 3x3 on 1024x1024 took 27-31 ms on 1 worker, 11-13 ms on 4).
const PAR_MEDIAN: usize = 1 << 21;
/// Largest kernel side.
const MAX_SIDE: usize = 4096;
/// Elements allowed in the padded copy of one plane (the plane plus the border
/// the kernel reads): 32 Mi (256 MiB of doubles), so a 4096 x 4096 image and
/// a 16 MP photo plane both pad for their 3 x 3 filters. Fixed, not tied to
/// the (larger, native) dense element limit.
const PAD_LIMIT: usize = 32 * 1024 * 1024;

fn parallel(work: usize) -> bool {
    work >= PAR_WORK && crate::parallel::current_worker_count() > 1
}

/// Size of the padded plane copy for `(oh, ow)` outputs and a `(kh, kw)`
/// window (both sides at least 1), rejected before anything is allocated.
/// Only the output is bounded by `admit`; a long kernel on a small image
/// (imfilter(ones(4096), ones(1, 1e6)) would ask for 33 GB) is bounded here.
fn padded_size(
    name: &str,
    what: &str,
    (oh, ow): (usize, usize),
    (kh, kw): (usize, usize),
) -> Result<(usize, usize)> {
    match oh.checked_add(kh - 1).zip(ow.checked_add(kw - 1)) {
        Some((ph, pw)) if ph.checked_mul(pw).is_some_and(|n| n <= PAD_LIMIT) => Ok((ph, pw)),
        _ => Err(format!(
            "{name}: the {kh}-by-{kw} {what} is too large for a {oh}-by-{ow} result (its padded copy would exceed {PAD_LIMIT} elements)"
        )),
    }
}

// ------------------------------------------------------------------- fspecial

/// hsize argument: n or [rows cols], positive integers.
fn hsize(value: &Value) -> Result<(usize, usize)> {
    let v = numbers(value).ok_or("fspecial: the size must be a positive integer or [rows cols]")?;
    let ok = |x: f64| x.is_finite() && x >= 1. && x.fract() == 0. && x <= MAX_SIDE as f64;
    match v.as_slice() {
        [n] if ok(*n) => Ok((*n as usize, *n as usize)),
        [r, c] if ok(*r) && ok(*c) => Ok((*r as usize, *c as usize)),
        _ => Err("fspecial: the size must be a positive integer or [rows cols]".into()),
    }
}

fn scalar(value: &Value, what: &str) -> Result<f64> {
    numbers(value)
        .filter(|v| v.len() == 1 && v[0].is_finite())
        .map(|v| v[0])
        .ok_or_else(|| format!("fspecial: {what} must be a finite real scalar"))
}

/// Gaussian samples exp(-(x^2 + y^2) / (2 sigma^2)) on a centred rows x cols
/// grid (column-major), not normalised.
fn gaussian_grid(rows: usize, cols: usize, sr: f64, sc: f64) -> Vec<f64> {
    let mut g = Vec::with_capacity(rows * cols);
    for j in 0..cols {
        let x = j as f64 - (cols as f64 - 1.) / 2.;
        for i in 0..rows {
            let y = i as f64 - (rows as f64 - 1.) / 2.;
            g.push((-(x * x) / (2. * sc * sc) - (y * y) / (2. * sr * sr)).exp());
        }
    }
    g
}

fn matrix(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

pub(super) fn fspecial(engine: &Engine, args: &[Value]) -> Result<Value> {
    let kind = match args.first() {
        Some(v) => text(engine, v).ok_or("fspecial: the filter type must be text")?,
        None => return Err("fspecial: not enough input arguments (fspecial(type, ...))".into()),
    };
    let kind = kind.to_ascii_lowercase();
    let max = match kind.as_str() {
        "average" | "laplacian" | "unsharp" => 2,
        "gaussian" | "log" => 3,
        "prewitt" | "sobel" => 1,
        "disk" | "motion" => {
            return Err(format!(
                "fspecial: the filter type '{kind}' is not supported yet"
            ));
        }
        _ => return Err(format!("fspecial: unknown filter type '{kind}'")),
    };
    if args.len() > max {
        return Err(format!("fspecial: too many input arguments for '{kind}'"));
    }
    match kind.as_str() {
        "average" => {
            let (r, c) = args.get(1).map(hsize).transpose()?.unwrap_or((3, 3));
            let n = (r * c) as f64;
            matrix(r, c, vec![1. / n; r * c])
        }
        "gaussian" | "log" => {
            let default = if kind == "log" { (5, 5) } else { (3, 3) };
            let (r, c) = args.get(1).map(hsize).transpose()?.unwrap_or(default);
            let sigma = args
                .get(2)
                .map(|v| scalar(v, "sigma"))
                .transpose()?
                .unwrap_or(0.5);
            if sigma <= 0. {
                return Err("fspecial: sigma must be positive".into());
            }
            let g = gaussian_grid(r, c, sigma, sigma);
            let sum: f64 = g.iter().sum();
            if kind == "gaussian" {
                return matrix(r, c, g.iter().map(|v| v / sum).collect());
            }
            // (x^2 + y^2 - 2 sigma^2) hg / (sigma^4 sum hg), shifted to sum 0.
            let s2 = sigma * sigma;
            let mut h = Vec::with_capacity(r * c);
            for j in 0..c {
                let x = j as f64 - (c as f64 - 1.) / 2.;
                for i in 0..r {
                    let y = i as f64 - (r as f64 - 1.) / 2.;
                    h.push((x * x + y * y - 2. * s2) * g[j * r + i] / (s2 * s2 * sum));
                }
            }
            let mean = h.iter().sum::<f64>() / h.len() as f64;
            matrix(r, c, h.into_iter().map(|v| v - mean).collect())
        }
        "laplacian" | "unsharp" => {
            let alpha = args
                .get(1)
                .map(|v| scalar(v, "alpha"))
                .transpose()?
                .unwrap_or(0.2);
            if !(0. ..=1.).contains(&alpha) {
                return Err("fspecial: alpha must be in the range [0, 1]".into());
            }
            let a = alpha;
            let k = if kind == "laplacian" {
                let (d, e) = (a / 4., (1. - a) / 4.);
                [d, e, d, e, -1., e, d, e, d].map(|v| 4. / (a + 1.) * v)
            } else {
                [-a, a - 1., -a, a - 1., a + 5., a - 1., -a, a - 1., -a].map(|v| v / (a + 1.))
            };
            matrix(3, 3, k.to_vec())
        }
        "prewitt" => matrix(3, 3, vec![1., 0., -1., 1., 0., -1., 1., 0., -1.]),
        _ => matrix(3, 3, vec![1., 0., -1., 2., 0., -2., 1., 0., -1.]),
    }
}

// ------------------------------------------------------------------- imfilter

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Boundary {
    Constant(f64),
    Symmetric,
    Replicate,
    Circular,
}

/// Source index of padded position `s` (may be negative) on an axis of `n`;
/// None reads the constant.
fn source(s: i64, n: usize, boundary: Boundary) -> Option<usize> {
    let n = n as i64;
    if (0..n).contains(&s) {
        return Some(s as usize);
    }
    Some(match boundary {
        Boundary::Constant(_) => return None,
        Boundary::Replicate => s.clamp(0, n - 1) as usize,
        Boundary::Circular => s.rem_euclid(n) as usize,
        Boundary::Symmetric => {
            // Mirror including the edge pixel: ... 1 0 | 0 1 ... n-1 | n-1 n-2 ...
            let m = s.rem_euclid(2 * n);
            (if m >= n { 2 * n - 1 - m } else { m }) as usize
        }
    })
}

/// Correlation of one plane with a kh x kw kernel (column-major). `top` and
/// `left` are the padding before the first pixel, so output (i, j) is
/// sum k(a, b) A(i + a - top, j + b - left) over oh x ow outputs. The caller
/// has checked the padded size (`padded_size`). Cancellation is polled in the
/// padding and, in every worker, between blocks of taps.
#[allow(clippy::too_many_arguments)]
fn correlate(
    engine: &Engine,
    plane: &[f64],
    (h, w): (usize, usize),
    kernel: &[f64],
    (kh, kw): (usize, usize),
    (top, left): (usize, usize),
    (oh, ow): (usize, usize),
    boundary: Boundary,
) -> Result<Vec<f64>> {
    let fill = match boundary {
        Boundary::Constant(v) => v,
        _ => 0.,
    };
    // Padded copy covering every position the kernel reads.
    let (ph, pw) = (oh + kh - 1, ow + kw - 1);
    let rows: Vec<Option<usize>> = (0..ph)
        .map(|p| source(p as i64 - top as i64, h, boundary))
        .collect();
    let mut padded = Vec::with_capacity(ph * pw);
    for q in 0..pw {
        poll_column(engine, q, ph)?;
        match source(q as i64 - left as i64, w, boundary) {
            Some(c) => {
                let col = &plane[c * h..(c + 1) * h];
                padded.extend(rows.iter().map(|r| r.map_or(fill, |r| col[r])));
            }
            None => padded.extend(std::iter::repeat_n(fill, ph)),
        }
    }
    engine.check_cancel()?;
    let taps: Vec<(usize, usize, f64)> = (0..kw)
        .flat_map(|b| (0..kh).map(move |a| (a, b)))
        .map(|(a, b)| (a, b, kernel[b * kh + a]))
        .filter(|t| t.2 != 0.)
        .collect();
    let mut out = vec![0.; oh * ow];
    // Work is outputs x taps (up to ~1e14), so every worker polls the shared
    // flag between blocks of taps that each cover at least POLL multiply-adds.
    let cancel: &AtomicBool = &engine.cancel;
    let stride = (POLL / oh.max(1)).max(1);
    let column = |(j, dst): (usize, &mut [f64])| -> Result<()> {
        for block in taps.chunks(stride) {
            cancelled(cancel)?;
            for &(a, b, k) in block {
                let src = &padded[(j + b) * ph + a..(j + b) * ph + a + oh];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d += k * s;
                }
            }
        }
        Ok(())
    };
    if oh > 0 {
        if parallel(oh * ow * taps.len()) {
            out.par_chunks_mut(oh).enumerate().try_for_each(column)?;
        } else {
            out.chunks_mut(oh).enumerate().try_for_each(column)?;
        }
    }
    Ok(out)
}

/// Kernel centre: floor((size + 1) / 2), 0-based.
fn centre(n: usize) -> usize {
    n.div_ceil(2).max(1) - 1
}

/// Filters every plane of `img` with `kernel`; `full` gives the full output.
fn filter_image(
    engine: &Engine,
    name: &str,
    img: &Img,
    kernel: &[f64],
    (kh, kw): (usize, usize),
    boundary: Boundary,
    full: bool,
) -> Result<Img> {
    let (h, w) = (img.h, img.w);
    let (top, left, oh, ow) = if full {
        (kh - 1, kw - 1, h + kh - 1, w + kw - 1)
    } else {
        (centre(kh), centre(kw), h, w)
    };
    super::admit(name, oh, ow, img.c)?;
    padded_size(name, "filter", (oh, ow), (kh, kw))?;
    let mut data = Vec::with_capacity(oh * ow * img.c);
    for k in 0..img.c {
        let plane = img.plane(k);
        if h == 0 || w == 0 {
            data.extend(std::iter::repeat_n(0., oh * ow));
            continue;
        }
        data.extend(correlate(
            engine,
            plane,
            (h, w),
            kernel,
            (kh, kw),
            (top, left),
            (oh, ow),
            boundary,
        )?);
    }
    Ok(Img {
        h: oh,
        w: ow,
        c: img.c,
        kind: img.kind,
        data,
    })
}

fn boundary_name(text: &str) -> Option<Boundary> {
    Some(match text.to_ascii_lowercase().as_str() {
        "symmetric" => Boundary::Symmetric,
        "replicate" => Boundary::Replicate,
        "circular" => Boundary::Circular,
        _ => return None,
    })
}

/// Results back in the input class; logical results are true where nonzero.
fn finish(engine: &Engine, img: Img) -> Result<Value> {
    to_value(engine, img)
}

pub(super) fn imfilter(engine: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err("imfilter: not enough input arguments (imfilter(A, h, ...))".into());
    }
    let img = image(engine, "imfilter", &args[0])?;
    let h = image(engine, "imfilter", &args[1])?;
    if h.c != 1 {
        return Err("imfilter: the filter h must be a 2-D array".into());
    }
    let mut boundary = Boundary::Constant(0.);
    let mut full = false;
    let mut convolve = false;
    for arg in &args[2..] {
        if let Some(t) = text(engine, arg) {
            if let Some(b) = boundary_name(&t) {
                boundary = b;
                continue;
            }
            match t.to_ascii_lowercase().as_str() {
                "same" => full = false,
                "full" => full = true,
                "corr" => convolve = false,
                "conv" => convolve = true,
                _ => return Err(format!("imfilter: unknown option '{t}'")),
            }
        } else {
            let v = numbers(arg)
                .filter(|v| v.len() == 1)
                .ok_or("imfilter: a boundary value must be a numeric scalar")?;
            boundary = Boundary::Constant(v[0]);
        }
    }
    let (kh, kw) = (h.h, h.w);
    if kh == 0 || kw == 0 {
        // An empty filter gives zeros of the input size.
        let n = img.data.len();
        return finish(
            engine,
            Img {
                data: vec![0.; n],
                ..img
            },
        );
    }
    let mut kernel = h.data.clone();
    if convolve {
        // Convolution is correlation with the kernel turned by 180 degrees.
        kernel.reverse();
    }
    let out = filter_image(engine, "imfilter", &img, &kernel, (kh, kw), boundary, full)?;
    finish(engine, out)
}

// ---------------------------------------------------------------- imgaussfilt

fn gauss_size_error() -> String {
    format!("imgaussfilt: FilterSize must be odd positive integers up to {MAX_SIDE}")
}

pub(super) fn imgaussfilt(engine: &Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err("imgaussfilt: not enough input arguments (imgaussfilt(A, sigma))".into());
    }
    let img = image(engine, "imgaussfilt", &args[0])?;
    let mut rest = &args[1..];
    let mut sigma = (0.5, 0.5);
    if let Some(first) = rest.first()
        && text(engine, first).is_none()
    {
        let v = numbers(first).ok_or("imgaussfilt: sigma must be numeric")?;
        sigma = match v.as_slice() {
            [s] => (*s, *s),
            [r, c] => (*r, *c),
            _ => {
                return Err(
                    "imgaussfilt: sigma must be a positive scalar or a two-element vector".into(),
                );
            }
        };
        if !(sigma.0 > 0. && sigma.1 > 0. && sigma.0.is_finite() && sigma.1.is_finite()) {
            return Err("imgaussfilt: sigma must be positive".into());
        }
        rest = &rest[1..];
    }
    let side = |s: f64| 2. * (2. * s).ceil() + 1.;
    let mut size = (side(sigma.0), side(sigma.1));
    let mut boundary = Boundary::Replicate;
    while let Some(first) = rest.first() {
        let key = text(engine, first).ok_or("imgaussfilt: options must be name-value pairs")?;
        let value = rest
            .get(1)
            .ok_or_else(|| format!("imgaussfilt: the option '{key}' needs a value"))?;
        match key.to_ascii_lowercase().as_str() {
            "filtersize" => {
                let v = numbers(value).ok_or("imgaussfilt: FilterSize must be numeric")?;
                size = match v.as_slice() {
                    [n] => (*n, *n),
                    [r, c] => (*r, *c),
                    _ => {
                        return Err(
                            "imgaussfilt: FilterSize must be a scalar or a two-element vector"
                                .into(),
                        );
                    }
                };
                let odd =
                    |n: f64| n.is_finite() && n >= 1. && n.fract() == 0. && (n as i64) % 2 == 1;
                if !odd(size.0) || !odd(size.1) || size.0.max(size.1) > MAX_SIDE as f64 {
                    return Err(gauss_size_error());
                }
            }
            "padding" => {
                boundary = match text(engine, value) {
                    Some(t) => boundary_name(&t)
                        .ok_or_else(|| format!("imgaussfilt: unknown padding '{t}'"))?,
                    None => Boundary::Constant(
                        numbers(value)
                            .filter(|v| v.len() == 1)
                            .ok_or("imgaussfilt: Padding must be a name or a numeric scalar")?[0],
                    ),
                };
            }
            "filterdomain" => {
                let t = text(engine, value).unwrap_or_default().to_ascii_lowercase();
                if !matches!(t.as_str(), "auto" | "spatial" | "frequency") {
                    return Err(
                        "imgaussfilt: FilterDomain must be 'auto', 'spatial' or 'frequency'".into(),
                    );
                }
            }
            _ => return Err(format!("imgaussfilt: unknown option '{key}'")),
        }
        rest = &rest[2..];
    }
    // The default 2*ceil(2*sigma)+1 meets the same bound as an explicit size.
    if size.0.max(size.1) > MAX_SIDE as f64 {
        return Err(format!(
            "{} (the default 2*ceil(2*sigma)+1 is larger)",
            gauss_size_error()
        ));
    }
    let (kh, kw) = (size.0 as usize, size.1 as usize);
    // The normalised 2-D Gaussian is the product of two normalised 1-D ones:
    // filter the columns, then the rows (same result for every padding).
    let normal = |n: usize, s: f64| {
        let g = gaussian_grid(n, 1, s, s);
        let sum: f64 = g.iter().sum();
        g.into_iter().map(|v| v / sum).collect::<Vec<_>>()
    };
    let column = normal(kh, sigma.0);
    let row = normal(kw, sigma.1);
    let kind = img.kind;
    let mut work = Img {
        kind: Kind::Double,
        ..img
    };
    work = filter_image(
        engine,
        "imgaussfilt",
        &work,
        &column,
        (kh, 1),
        boundary,
        false,
    )?;
    work = filter_image(engine, "imgaussfilt", &work, &row, (1, kw), boundary, false)?;
    work.kind = kind;
    finish(engine, work)
}

// ------------------------------------------------------------------- medfilt2

/// Median order: every NaN after every number, whatever its sign bit.
/// `f64::total_cmp` orders by the sign bit, which puts a negative NaN (the
/// kind 0/0 produces on x86) before every number and abs(NaN) after them.
fn nan_last(a: &f64, b: &f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (false, false) => a.total_cmp(b),
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
    }
}

pub(super) fn medfilt2(engine: &Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err("medfilt2: not enough input arguments (medfilt2(A, [m n]))".into());
    }
    if args.len() > 3 {
        return Err("medfilt2: too many input arguments".into());
    }
    let mut img = image(engine, "medfilt2", &args[0])?;
    if img.c != 1 {
        return Err("medfilt2: expected a 2-D image".into());
    }
    let mut size = (3, 3);
    let mut boundary = Boundary::Constant(0.);
    for arg in &args[1..] {
        if let Some(t) = text(engine, arg) {
            boundary = match t.to_ascii_lowercase().as_str() {
                "zeros" => Boundary::Constant(0.),
                "symmetric" => Boundary::Symmetric,
                // 1 for double images, 0 otherwise.
                "indexed" => Boundary::Constant(if img.kind == Kind::Double { 1. } else { 0. }),
                _ => return Err(format!("medfilt2: unknown padding option '{t}'")),
            };
        } else {
            let v = numbers(arg).ok_or("medfilt2: the neighbourhood size must be [m n]")?;
            let ok = |x: f64| x.is_finite() && x >= 1. && x.fract() == 0. && x <= MAX_SIDE as f64;
            size = match v.as_slice() {
                [m, n] if ok(*m) && ok(*n) => (*m as usize, *n as usize),
                _ => {
                    return Err(
                        "medfilt2: the neighbourhood size must be [m n] positive integers".into(),
                    );
                }
            };
        }
    }
    let (h, w) = (img.h, img.w);
    let (m, n) = size;
    if h == 0 || w == 0 {
        return to_value(engine, img);
    }
    // Bounded before the padded copy is allocated (the window may be larger
    // than the image: medfilt2(zeros(1, 1e7), [4096 1]) would ask for 3e11).
    let (ph, pw) = padded_size("medfilt2", "neighbourhood", (h, w), (m, n))?;
    let (top, left) = (centre(m), centre(n));
    let fill = match boundary {
        Boundary::Constant(v) => v,
        _ => 0.,
    };
    let rows: Vec<Option<usize>> = (0..ph)
        .map(|p| source(p as i64 - top as i64, h, boundary))
        .collect();
    let mut padded = Vec::with_capacity(ph * pw);
    for q in 0..pw {
        poll_column(engine, q, ph)?;
        match source(q as i64 - left as i64, w, boundary) {
            Some(c) => {
                let col = &img.data[c * h..(c + 1) * h];
                padded.extend(rows.iter().map(|r| r.map_or(fill, |r| col[r])));
            }
            None => padded.extend(std::iter::repeat_n(fill, ph)),
        }
    }
    let count = m * n;
    let integer = matches!(img.kind, Kind::Typed(c) if c.is_integer()) || img.kind == Kind::Logical;
    // A pixel costs about `count` operations (up to ~1e7 for a 4096 x 4096
    // window): every worker polls the shared flag between blocks of pixels
    // that each cover at least POLL of them.
    let cancel: &AtomicBool = &engine.cancel;
    let stride = (POLL / count).max(1);
    let column = |(j, dst): (usize, &mut [f64])| -> Result<()> {
        let mut window = Vec::with_capacity(count);
        for (i, d) in dst.iter_mut().enumerate() {
            if i.is_multiple_of(stride) {
                cancelled(cancel)?;
            }
            window.clear();
            for b in 0..n {
                let start = (j + b) * ph + i;
                window.extend_from_slice(&padded[start..start + m]);
            }
            let half = count / 2;
            let (_, upper, _) = window.select_nth_unstable_by(half, nan_last);
            let upper = *upper;
            *d = if count % 2 == 1 {
                upper
            } else {
                // Even counts: the mean of the two middle values; integer
                // classes discard the fraction (reference page, Tips).
                let lower = window[..half]
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max);
                let mean = lower / 2. + upper / 2.;
                if integer { mean.trunc() } else { mean }
            };
        }
        Ok(())
    };
    let mut out = vec![0.; h * w];
    engine.check_cancel()?;
    if h * w * count * 4 >= PAR_MEDIAN && crate::parallel::current_worker_count() > 1 {
        out.par_chunks_mut(h).enumerate().try_for_each(column)?;
    } else {
        out.chunks_mut(h).enumerate().try_for_each(column)?;
    }
    img.data = out;
    to_value(engine, img)
}

#[cfg(test)]
mod tests {
    use super::super::tests::cancel_at;
    use super::*;
    use std::time::{Duration, Instant};

    fn filled(rows: usize, cols: usize, v: f64) -> Value {
        Value::Matrix(Matrix::filled(rows, cols, v).unwrap())
    }

    /// Runs `call` on a fresh engine whose cancel flag is set by its 1000th
    /// poll (the class conversions and padding before the kernels poll fewer
    /// than 100 times at these sizes, so the flag is raised inside a kernel
    /// loop) and checks that the cancel error comes back long before the
    /// kernel could have finished (about a minute of work).
    fn cancelled_inside_the_kernel(call: impl Fn(&Engine) -> Result<Value>) {
        let engine = Engine::default();
        let _guard = cancel_at(&engine.cancel, 1000);
        let start = Instant::now();
        match call(&engine) {
            Err(message) => assert_eq!(message, "Calculation cancelled"),
            Ok(_) => panic!("the call finished instead of being cancelled"),
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "took {:?}",
            start.elapsed()
        );
    }

    /// The same call on the calling thread and inside a 4-worker pool, where
    /// the kernels split their columns over workers that hold only the flag.
    fn serial_and_pooled(call: impl Fn(&Engine) -> Result<Value> + Send + Sync) {
        cancelled_inside_the_kernel(&call);
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(|| cancelled_inside_the_kernel(&call));
    }

    #[test]
    fn imfilter_polls_the_cancel_flag_inside_the_kernel_loops() {
        // 1024^2 outputs x 90000 taps: 9.4e10 multiply-adds.
        serial_and_pooled(|engine| {
            imfilter(
                engine,
                &[filled(1024, 1024, 0.5), filled(300, 300, 1. / 90000.)],
            )
        });
    }

    #[test]
    fn imgaussfilt_polls_the_cancel_flag_inside_the_kernel_loops() {
        // FilterSize 4095 x 4095 passes: 2 x 1024^2 outputs x 4095 taps.
        serial_and_pooled(|engine| {
            let args = [
                filled(1024, 1024, 0.5),
                filled(1, 1, 700.),
                Value::Text("FilterSize".into()),
                filled(1, 1, 4095.),
            ];
            imgaussfilt(engine, &args)
        });
    }

    #[test]
    fn medfilt2_polls_the_cancel_flag_inside_the_pixel_loop() {
        // 512^2 pixels x a 151 x 151 window: about 6e9 element moves.
        serial_and_pooled(|engine| medfilt2(engine, &[filled(512, 512, 0.5), filled(1, 2, 151.)]));
    }

    #[test]
    fn padded_copy_limit_is_checked_without_overflow() {
        // A full-size 4096 x 4096 image still takes a 3 x 3 filter ...
        assert_eq!(
            padded_size("imfilter", "filter", (4096, 4096), (3, 3)).unwrap(),
            (4098, 4098)
        );
        // ... and the padded copy may hold PAD_LIMIT elements, no more.
        assert!(padded_size("imfilter", "filter", (1, PAD_LIMIT), (1, 1)).is_ok());
        let message = padded_size("imfilter", "filter", (1, PAD_LIMIT), (2, 1)).unwrap_err();
        assert!(message.contains("imfilter: the 2-by-1 filter is too large"));
        // The foreman's case, a 1e6-tap row on a 4096 x 4096 image (33 GB).
        assert!(padded_size("imfilter", "filter", (4096, 4096), (1, 1_000_000)).is_err());
        // Sums and products that overflow usize are errors, not wraps.
        assert!(padded_size("medfilt2", "neighbourhood", (usize::MAX, 1), (2, 1)).is_err());
        assert!(padded_size("medfilt2", "neighbourhood", (usize::MAX / 2, 4), (1, 1)).is_err());
    }

    #[test]
    fn nan_ranks_after_every_number_whatever_its_sign() {
        let negative = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
        let positive = f64::from_bits(f64::NAN.to_bits() & !(1 << 63));
        assert!(negative.is_sign_negative() && positive.is_sign_positive());
        let mut v = [
            negative,
            f64::INFINITY,
            2.,
            positive,
            f64::NEG_INFINITY,
            -0.5,
        ];
        v.sort_by(nan_last);
        assert_eq!(&v[..4], &[f64::NEG_INFINITY, -0.5, 2., f64::INFINITY]);
        assert!(v[4].is_nan() && v[5].is_nan());
        assert_eq!(nan_last(&negative, &positive), Ordering::Equal);
    }
}
