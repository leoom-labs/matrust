//! imresize, imrotate, imcrop and imtranslate. Every function works plane by
//! plane on 2-D and H-by-W-by-C images and keeps the input class.
use super::{Img, Kind, POLL, admit, cancelled, image, numbers, poll_column, text, to_value};
use crate::Result;
use crate::engine::{Engine, Value};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

/// Multiply-adds above which a resize pass uses the worker pool. 4 workers
/// gave no gain on 2048x2048 doubles (4-vCPU VM, 2026-11-03: 0.5x bicubic
/// 42-47 ms, 2x bilinear 300-334 ms either way; memory and conversion bound),
/// so only very large passes are split.
const PAR_WORK: usize = 1 << 28;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Method {
    Nearest,
    Bilinear,
    Bicubic,
}

fn method(name: &str, text: &str) -> Result<Option<Method>> {
    Ok(Some(match text.to_ascii_lowercase().as_str() {
        "nearest" => Method::Nearest,
        "bilinear" | "linear" => Method::Bilinear,
        "bicubic" | "cubic" => Method::Bicubic,
        "box" | "triangle" | "lanczos2" | "lanczos3" => {
            return Err(format!("{name}: the method '{text}' is not supported yet"));
        }
        _ => return Ok(None),
    }))
}

/// Logical images are resampled as 0/1 doubles and thresholded at 0.5.
fn finish_logical(img: &mut Img) {
    if img.kind == Kind::Logical {
        for v in &mut img.data {
            *v = f64::from(u8::from(*v > 0.5));
        }
    }
}

// ------------------------------------------------------------------ imresize

/// Keys (1981) cubic convolution kernel with a = -0.5.
fn cubic(x: f64) -> f64 {
    let a = x.abs();
    if a <= 1. {
        (1.5 * a - 2.5) * a * a + 1.
    } else if a < 2. {
        ((-0.5 * a + 2.5) * a - 4.) * a + 2.
    } else {
        0.
    }
}
fn triangle(x: f64) -> f64 {
    (1. - x.abs()).max(0.)
}
fn boxed(x: f64) -> f64 {
    if (-0.5..0.5).contains(&x) { 1. } else { 0. }
}

/// Sample weights of one axis: for each output index the input indices
/// (0-based, edges replicated) and their normalised weights.
struct Weights {
    index: Vec<Vec<usize>>,
    weight: Vec<Vec<f64>>,
}

/// Fewest kernel taps per output pixel that are always allowed. Every scale
/// of at least 1/size needs at most 4*size + 3 (bicubic), so the cap only
/// rejects scales that shrink an axis far below one pixel.
const MIN_TAP_CAP: usize = 1 << 20;

fn weights(
    cancel: &AtomicBool,
    input: usize,
    output: usize,
    scale: f64,
    method: Method,
    antialias: bool,
) -> Result<Weights> {
    let (kernel, base): (fn(f64) -> f64, f64) = match method {
        Method::Nearest => (boxed, 1.),
        Method::Bilinear => (triangle, 2.),
        Method::Bicubic => (cubic, 4.),
    };
    // Shrinking with antialiasing widens the kernel by 1/scale: h(x) = s k(s x).
    let widen = antialias && scale < 1. && method != Method::Nearest;
    let width = if widen { base / scale } else { base };
    // The tap count is bounded before any cast or allocation (a tiny scale
    // would otherwise ask for O(1/scale) weights per pixel or overflow i64).
    let cap = input.saturating_mul(4).saturating_add(8).max(MIN_TAP_CAP);
    let taps = width.ceil() + 2.;
    if taps.is_nan() || taps > cap as f64 {
        return Err(format!(
            "imresize: the scale is too small to antialias (the kernel would span more than {cap} samples); use 'Antialiasing', false"
        ));
    }
    let taps = taps as i64;
    let mut index = Vec::with_capacity(output);
    let mut weight = Vec::with_capacity(output);
    // One poll per block of output pixels that together build >= POLL taps.
    let stride = (POLL / taps as usize).max(1);
    for x in 1..=output {
        if (x - 1).is_multiple_of(stride) {
            cancelled(cancel)?;
        }
        // Output pixel x (1-based) centres on input coordinate u.
        let u = x as f64 / scale + 0.5 * (1. - 1. / scale);
        let left = (u - width / 2.).floor();
        // A kernel wholly past one edge reads only the replicated edge pixel
        // (also keeps a far-away u, as from a tiny scale, out of the i64 cast).
        if left >= input as f64 || left + (taps - 1) as f64 <= 1. {
            index.push(vec![if left >= input as f64 { input - 1 } else { 0 }]);
            weight.push(vec![1.]);
            continue;
        }
        let left = left as i64;
        let mut idx = Vec::with_capacity(taps as usize);
        let mut wts = Vec::with_capacity(taps as usize);
        let mut sum = 0.;
        for j in 0..taps {
            let at = left + j;
            let d = u - at as f64;
            let w = if widen {
                scale * kernel(scale * d)
            } else {
                kernel(d)
            };
            if w != 0. {
                idx.push((at.clamp(1, input as i64) - 1) as usize);
                wts.push(w);
                sum += w;
            }
        }
        if sum != 0. {
            for w in &mut wts {
                *w /= sum;
            }
        }
        index.push(idx);
        weight.push(wts);
    }
    Ok(Weights { index, weight })
}

fn parallel(work: usize) -> bool {
    work >= PAR_WORK && crate::parallel::current_worker_count() > 1
}

/// Resamples the rows (first axis) of every column of a plane. Each column
/// (also on a pool worker) polls the shared cancel flag first.
fn resize_rows(
    cancel: &AtomicBool,
    src: &[f64],
    h: usize,
    w: usize,
    oh: usize,
    wt: &Weights,
) -> Result<Vec<f64>> {
    let mut out = vec![0.; oh * w];
    let column = |(j, dst): (usize, &mut [f64])| -> Result<()> {
        cancelled(cancel)?;
        let col = &src[j * h..(j + 1) * h];
        for (i, d) in dst.iter_mut().enumerate() {
            *d = wt.index[i]
                .iter()
                .zip(&wt.weight[i])
                .map(|(&k, &v)| v * col[k])
                .sum();
        }
        Ok(())
    };
    if parallel(oh * w * 4) {
        out.par_chunks_mut(oh.max(1))
            .enumerate()
            .try_for_each(column)?;
    } else {
        out.chunks_mut(oh.max(1)).enumerate().try_for_each(column)?;
    }
    Ok(out)
}

/// Resamples the columns (second axis) of a plane; each output column polls
/// the cancel flag before every block of taps worth at least `POLL` elements.
fn resize_cols(
    cancel: &AtomicBool,
    src: &[f64],
    h: usize,
    ow: usize,
    wt: &Weights,
) -> Result<Vec<f64>> {
    let mut out = vec![0.; h * ow];
    let stride = (POLL / h.max(1)).max(1);
    let column = |(j, dst): (usize, &mut [f64])| -> Result<()> {
        let (index, weight) = (&wt.index[j], &wt.weight[j]);
        for (ks, vs) in index.chunks(stride).zip(weight.chunks(stride)) {
            cancelled(cancel)?;
            for (&k, &v) in ks.iter().zip(vs) {
                let col = &src[k * h..(k + 1) * h];
                for (d, s) in dst.iter_mut().zip(col) {
                    *d += v * s;
                }
            }
        }
        Ok(())
    };
    if parallel(h * ow * 4) {
        out.par_chunks_mut(h.max(1))
            .enumerate()
            .try_for_each(column)?;
    } else {
        out.chunks_mut(h.max(1)).enumerate().try_for_each(column)?;
    }
    Ok(out)
}

pub(super) fn imresize(engine: &Engine, args: &[Value]) -> Result<Value> {
    const NAME: &str = "imresize";
    if args.len() < 2 {
        return Err(
            "imresize: not enough input arguments (imresize(A, scale) or imresize(A, [rows cols]))"
                .into(),
        );
    }
    let mut img = image(engine, NAME, &args[0])?;
    let size = numbers(&args[1])
        .ok_or("imresize: the scale must be a positive scalar or a [rows cols] size")?;
    let mut chosen = Method::Bicubic;
    let mut antialias = true;
    let mut rest = &args[2..];
    while let Some(first) = rest.first() {
        let key = text(engine, first)
            .ok_or("imresize: options must be method names or name-value pairs")?;
        if let Some(m) = method(NAME, &key)? {
            chosen = m;
            rest = &rest[1..];
            continue;
        }
        let value = rest
            .get(1)
            .ok_or_else(|| format!("imresize: the option '{key}' needs a value"))?;
        match key.to_ascii_lowercase().as_str() {
            "method" => {
                let m = text(engine, value).ok_or("imresize: Method must be a method name")?;
                chosen =
                    method(NAME, &m)?.ok_or_else(|| format!("imresize: unknown method '{m}'"))?;
            }
            "antialiasing" => {
                let v = numbers(value)
                    .filter(|v| v.len() == 1)
                    .ok_or("imresize: Antialiasing must be true or false")?;
                antialias = v[0] != 0.;
            }
            "colormap" | "dither" | "scale" | "outputsize" => {
                return Err(format!("imresize: the option '{key}' is not supported yet"));
            }
            _ => return Err(format!("imresize: unknown option '{key}'")),
        }
        rest = &rest[2..];
    }
    let (h, w) = (img.h, img.w);
    let (oh, ow, sr, sc) = match size.as_slice() {
        [s] if s.is_finite() && *s > 0. => {
            let oh = (s * h as f64).ceil() as usize;
            let ow = (s * w as f64).ceil() as usize;
            (oh, ow, *s, *s)
        }
        [r, c] if r.is_nan() && c.is_nan() => {
            return Err("imresize: at most one entry of the output size may be NaN".into());
        }
        [r, c] => {
            let valid = |v: f64| v.is_nan() || (v.is_finite() && v > 0. && v.fract() == 0.);
            if !valid(*r) || !valid(*c) {
                return Err(
                    "imresize: the output size must be positive integers (one may be NaN)".into(),
                );
            }
            // NaN keeps the aspect ratio: the other axis scales by the same factor.
            let (oh, ow) = if r.is_nan() {
                ((c / w as f64 * h as f64).ceil(), *c)
            } else if c.is_nan() {
                (*r, (r / h as f64 * w as f64).ceil())
            } else {
                (*r, *c)
            };
            (oh as usize, ow as usize, oh / h as f64, ow / w as f64)
        }
        _ => {
            return Err(
                "imresize: the scale must be a positive scalar or a [rows cols] size".into(),
            );
        }
    };
    admit(NAME, oh, ow, img.c)?;
    if h == 0 || w == 0 {
        return Err("imresize: the image must not be empty".into());
    }
    let identity = oh == h && ow == w && sr == 1. && sc == 1.;
    if identity {
        return to_value(engine, img);
    }
    // Output, one resized plane and the intermediate pass must fit the budgets
    // before the weights and buffers are allocated.
    let scratch = oh
        .checked_mul(ow)
        .and_then(|plane| {
            let pass = oh.checked_mul(w)?.max(h.checked_mul(ow)?);
            plane
                .checked_mul(img.c)?
                .checked_add(plane)?
                .checked_add(pass)
        })
        .ok_or("imresize: output size overflows")?;
    engine.preflight_elements(scratch)?;
    let rows = weights(&engine.cancel, h, oh, sr, chosen, antialias)?;
    let cols = weights(&engine.cancel, w, ow, sc, chosen, antialias)?;
    let mut data = Vec::with_capacity(oh * ow * img.c);
    for k in 0..img.c {
        engine.check_cancel()?;
        let plane = img.plane(k);
        // The axis with the smaller scale goes first (less work).
        let cancel = &engine.cancel;
        let out = if sr <= sc {
            let t = resize_rows(cancel, plane, h, w, oh, &rows)?;
            engine.check_cancel()?;
            resize_cols(cancel, &t, oh, ow, &cols)?
        } else {
            let t = resize_cols(cancel, plane, h, ow, &cols)?;
            engine.check_cancel()?;
            resize_rows(cancel, &t, h, ow, oh, &rows)?
        };
        data.extend(out);
    }
    img.h = oh;
    img.w = ow;
    img.data = data;
    finish_logical(&mut img);
    to_value(engine, img)
}

// ------------------------------------------------------------------ imrotate

/// Counterclockwise quarter turns of one plane (rot90 rule).
fn rot90(src: &[f64], h: usize, w: usize, turns: usize) -> (Vec<f64>, usize, usize) {
    if turns.is_multiple_of(4) {
        return (src.to_vec(), h, w);
    }
    let mut out = vec![0.; h * w];
    match turns % 4 {
        // B(i, j) = A(j, w-1-i), size w x h.
        1 => {
            for j in 0..h {
                for i in 0..w {
                    out[j * w + i] = src[(w - 1 - i) * h + j];
                }
            }
            (out, w, h)
        }
        // B(i, j) = A(h-1-i, w-1-j).
        2 => {
            for (k, v) in out.iter_mut().enumerate() {
                *v = src[h * w - 1 - k];
            }
            (out, h, w)
        }
        // B(i, j) = A(h-1-j, i), size w x h.
        _ => {
            for j in 0..h {
                for i in 0..w {
                    out[j * w + i] = src[i * h + (h - 1 - j)];
                }
            }
            (out, w, h)
        }
    }
}

/// The h-by-w window centred on an a-by-b plane; positions outside it read 0.
/// When the sizes differ by an odd number the window starts at the floor of
/// the half difference: the extra row or column is cut from the bottom or
/// right, or padded at the top or left (the plane sits half a pixel down and
/// right of the exact centre).
fn centred_window(src: &[f64], a: usize, b: usize, h: usize, w: usize) -> Vec<f64> {
    let offset = |big: usize, small: usize| (big as i64 - small as i64).div_euclid(2);
    let (ro, co) = (offset(a, h), offset(b, w));
    let mut out = vec![0.; h * w];
    for (x, dst) in out.chunks_mut(h.max(1)).enumerate() {
        let c = x as i64 + co;
        if c < 0 || c >= b as i64 {
            continue;
        }
        let col = &src[c as usize * a..(c as usize + 1) * a];
        for (y, d) in dst.iter_mut().enumerate() {
            let r = y as i64 + ro;
            if r >= 0 && r < a as i64 {
                *d = col[r as usize];
            }
        }
    }
    out
}

/// One sample at 1-based input coordinates (row y, column x); outside points
/// and missing neighbours read `fill`.
fn sample(plane: &[f64], h: usize, w: usize, y: f64, x: f64, method: Method, fill: f64) -> f64 {
    let at = |r: i64, c: i64| -> f64 {
        if r >= 1 && c >= 1 && r <= h as i64 && c <= w as i64 {
            plane[(c as usize - 1) * h + r as usize - 1]
        } else {
            fill
        }
    };
    match method {
        Method::Nearest => at(y.round() as i64, x.round() as i64),
        _ => {
            let (r0, c0) = (y.floor(), x.floor());
            let (fy, fx) = (y - r0, x - c0);
            let (r0, c0) = (r0 as i64, c0 as i64);
            // Exact grid points read one pixel (no blend with the fill value).
            let v00 = at(r0, c0);
            let v10 = if fy == 0. { 0. } else { at(r0 + 1, c0) };
            let v01 = if fx == 0. { 0. } else { at(r0, c0 + 1) };
            let v11 = if fy == 0. || fx == 0. {
                0.
            } else {
                at(r0 + 1, c0 + 1)
            };
            (1. - fy) * ((1. - fx) * v00 + fx * v01) + fy * ((1. - fx) * v10 + fx * v11)
        }
    }
}

pub(super) fn imrotate(engine: &Engine, args: &[Value]) -> Result<Value> {
    const NAME: &str = "imrotate";
    if args.len() < 2 {
        return Err("imrotate: not enough input arguments (imrotate(A, angle))".into());
    }
    if args.len() > 4 {
        return Err("imrotate: too many input arguments".into());
    }
    let mut img = image(engine, NAME, &args[0])?;
    let angle = numbers(&args[1])
        .filter(|v| v.len() == 1 && v[0].is_finite())
        .ok_or("imrotate: the angle must be a finite real scalar")?[0];
    let mut chosen = Method::Nearest;
    let mut crop = false;
    for arg in &args[2..] {
        let key = text(engine, arg).ok_or("imrotate: options must be text")?;
        match key.to_ascii_lowercase().as_str() {
            "loose" => crop = false,
            "crop" => crop = true,
            "bicubic" | "cubic" => {
                return Err("imrotate: the method 'bicubic' is not supported yet".into());
            }
            _ => {
                chosen = match method(NAME, &key)? {
                    Some(m) => m,
                    None => return Err(format!("imrotate: unknown option '{key}'")),
                }
            }
        }
    }
    let (h, w) = (img.h, img.w);
    let quarter = angle / 90.;
    if quarter.fract() == 0. {
        // Exact quarter turns: rot90 without interpolation; 'crop' then cuts
        // or pads (with 0) the turned image to the input size, centred.
        let turns = (quarter as i64).rem_euclid(4) as usize;
        let mut data = Vec::with_capacity(img.data.len());
        let (mut nh, mut nw) = (h, w);
        for k in 0..img.c {
            engine.check_cancel()?;
            let (plane, a, b) = rot90(img.plane(k), h, w, turns);
            if crop && (a, b) != (h, w) {
                data.extend(centred_window(&plane, a, b, h, w));
            } else {
                (nh, nw) = (a, b);
                data.extend(plane);
            }
        }
        img.h = nh;
        img.w = nw;
        img.data = data;
        return to_value(engine, img);
    }
    let theta = angle.to_radians();
    let (sin, cos) = (libm::sin(theta), libm::cos(theta));
    let (oh, ow) = if crop {
        (h, w)
    } else {
        // The rotated pixel-edge rectangle, rounded up (a tolerance keeps
        // exact sizes from gaining a row through rounding noise).
        let extent = |a: f64| (a - 1e-9).ceil().max(1.) as usize;
        (
            extent(w as f64 * sin.abs() + h as f64 * cos.abs()),
            extent(w as f64 * cos.abs() + h as f64 * sin.abs()),
        )
    };
    admit(NAME, oh, ow, img.c)?;
    // admit bounds the product, so it cannot overflow here.
    engine.preflight_elements(oh * ow * img.c)?;
    let (cyi, cxi) = ((h as f64 + 1.) / 2., (w as f64 + 1.) / 2.);
    let (cyo, cxo) = ((oh as f64 + 1.) / 2., (ow as f64 + 1.) / 2.);
    let mut data = Vec::with_capacity(oh * ow * img.c);
    for k in 0..img.c {
        let plane = img.plane(k);
        for x in 1..=ow {
            poll_column(engine, x, oh)?;
            let dx = x as f64 - cxo;
            for y in 1..=oh {
                let dy = y as f64 - cyo;
                // Output to input: the inverse of a counterclockwise turn on
                // screen (rows grow downwards).
                let xi = cxi + dx * cos - dy * sin;
                let yi = cyi + dx * sin + dy * cos;
                data.push(sample(plane, h, w, yi, xi, chosen, 0.));
            }
        }
    }
    img.h = oh;
    img.w = ow;
    img.data = data;
    finish_logical(&mut img);
    to_value(engine, img)
}

// -------------------------------------------------------------------- imcrop

pub(super) fn imcrop(engine: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err("imcrop needs a rectangle: imcrop(A, [xmin ymin width height])".into());
    }
    let mut img = image(engine, "imcrop", &args[0])?;
    let rect = numbers(&args[1])
        .filter(|r| r.len() == 4 && r.iter().all(|v| v.is_finite()))
        .ok_or("imcrop needs a rectangle: imcrop(A, [xmin ymin width height])")?;
    let (x, y, width, height) = (rect[0], rect[1], rect[2], rect[3]);
    if width < 0. || height < 0. {
        return Err("imcrop: the rectangle width and height must not be negative".into());
    }
    // Pixel k covers [k-0.5, k+0.5]: keep every pixel the rectangle touches.
    let first = |lo: f64| ((lo - 0.5).floor() + 1.).max(1.);
    let last = |hi: f64, n: usize| ((hi + 0.5).ceil() - 1.).min(n as f64);
    let (c1, c2) = (first(x), last(x + width, img.w));
    let (r1, r2) = (first(y), last(y + height, img.h));
    let (nr, nc) = if r2 >= r1 && c2 >= c1 {
        ((r2 - r1) as usize + 1, (c2 - c1) as usize + 1)
    } else {
        (0, 0)
    };
    let (r1, c1) = (r1 as usize - 1, c1 as usize - 1);
    let mut data = Vec::with_capacity(nr * nc * img.c);
    for k in 0..img.c {
        let plane = img.plane(k);
        for j in 0..nc {
            let start = (c1 + j) * img.h + r1;
            data.extend_from_slice(&plane[start..start + nr]);
        }
    }
    img.h = nr;
    img.w = nc;
    img.data = data;
    to_value(engine, img)
}

// --------------------------------------------------------------- imtranslate

pub(super) fn imtranslate(engine: &Engine, args: &[Value]) -> Result<Value> {
    const NAME: &str = "imtranslate";
    if args.len() < 2 {
        return Err("imtranslate: not enough input arguments (imtranslate(A, [tx ty]))".into());
    }
    let mut img = image(engine, NAME, &args[0])?;
    let shift = numbers(&args[1])
        .filter(|v| v.len() == 2 && v.iter().all(|t| t.is_finite()))
        .ok_or("imtranslate: the translation must be a finite [tx ty] vector")?;
    let (tx, ty) = (shift[0], shift[1]);
    let mut chosen = Method::Bilinear;
    let mut fill = 0.;
    let mut rest = &args[2..];
    while let Some(first) = rest.first() {
        let key = text(engine, first).ok_or("imtranslate: options must be text")?;
        if let Some(m) = method(NAME, &key)? {
            if m == Method::Bicubic {
                return Err("imtranslate: the method 'bicubic' is not supported yet".into());
            }
            chosen = m;
            rest = &rest[1..];
            continue;
        }
        let value = rest
            .get(1)
            .ok_or_else(|| format!("imtranslate: the option '{key}' needs a value"))?;
        match key.to_ascii_lowercase().as_str() {
            "fillvalues" => {
                fill = numbers(value)
                    .filter(|v| v.len() == 1)
                    .ok_or("imtranslate: FillValues must be a numeric scalar")?[0];
            }
            "outputview" => {
                let view = text(engine, value).unwrap_or_default();
                if !view.eq_ignore_ascii_case("same") {
                    return Err("imtranslate: only OutputView 'same' is supported yet".into());
                }
            }
            _ => return Err(format!("imtranslate: unknown option '{key}'")),
        }
        rest = &rest[2..];
    }
    let (h, w) = (img.h, img.w);
    let mut data = Vec::with_capacity(img.data.len());
    for k in 0..img.c {
        let plane = img.plane(k);
        for x in 1..=w {
            poll_column(engine, x, h)?;
            for y in 1..=h {
                // Positive tx moves the image right, positive ty down.
                data.push(sample(
                    plane,
                    h,
                    w,
                    y as f64 - ty,
                    x as f64 - tx,
                    chosen,
                    fill,
                ));
            }
        }
    }
    img.data = data;
    finish_logical(&mut img);
    to_value(engine, img)
}

#[cfg(test)]
mod tests {
    use super::super::tests::cancel_at;
    use super::*;

    fn is_cancelled<T>(result: Result<T>) -> bool {
        matches!(result, Err(message) if message == "Calculation cancelled")
    }

    #[test]
    fn weights_and_both_resize_passes_poll_the_cancel_flag() {
        let quiet = AtomicBool::new(false);
        let wt = weights(&quiet, 64, 32, 0.5, Method::Bicubic, true).unwrap();
        let src = vec![1.; 64 * 64];
        // Unregistered flag: both passes finish (and keep their sizes).
        assert_eq!(
            resize_rows(&quiet, &src, 64, 64, 32, &wt).unwrap().len(),
            32 * 64
        );
        assert_eq!(
            resize_cols(&quiet, &src, 64, 32, &wt).unwrap().len(),
            64 * 32
        );
        // The weights poll before their first output pixel.
        let flag = AtomicBool::new(false);
        let _weights = cancel_at(&flag, 0);
        assert!(is_cancelled(weights(
            &flag,
            64,
            32,
            0.5,
            Method::Bicubic,
            true
        )));
        // Each pass polls once per column: the fourth poll stops the 64 columns.
        let flag = AtomicBool::new(false);
        let _rows = cancel_at(&flag, 3);
        assert!(is_cancelled(resize_rows(&flag, &src, 64, 64, 32, &wt)));
        let flag = AtomicBool::new(false);
        let _cols = cancel_at(&flag, 3);
        assert!(is_cancelled(resize_cols(&flag, &src, 64, 32, &wt)));
    }
}
