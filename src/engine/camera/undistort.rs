//! undistortImage and undistortPoints (piece 7d4), written from the public
//! MathWorks reference pages. No MathWorks code or examples.
//!
//! Model: projection.rs `distort` on normalized coordinates, pixels 1-based
//! (the principal point too), K = [fx s cx; 0 fy cy; 0 0 1].
//!
//! - undistortImage: every output pixel (u, v) of the undistorted view is
//!   normalized with the output intrinsics, distorted, re-projected with the
//!   input K and sampled from I (registration/warp.rs `remap`: nearest,
//!   linear (default) or cubic; points outside the pixel-centre hull get
//!   FillValues). OutputView 'same' (default) keeps the size and K; 'full'
//!   is the whole-pixel box around the undistorted border pixel centres;
//!   'valid' is the largest such box whose pixels all sample inside I. Both
//!   keep the focal lengths and skew and move the principal point by the
//!   whole-pixel offset of the box. The second output is a cameraIntrinsics
//!   of J: that K, ImageSize = size of J, no distortion (the page's
//!   camIntrinsics).
//! - undistortPoints: Newton's method on the 2-by-2 distortion map (from the
//!   distorted point, halving steps that do not reduce the residual), at
//!   most 50 steps; the second output is the pixel distance from each input
//!   point to its undistorted point distorted again.
use super::objects::{Intrinsics, pairs};
use super::projection::distort;
use super::{Camera, camera_of, err, new_object, num, text_arg};
use crate::engine::image_basic::{self, Img, Kind};
use crate::engine::registration::{Interp, remap};
use crate::engine::{Engine, Value};
use crate::{Array, Matrix, NumericClass, Result, TypedArray};

/// Newton steps per point.
const MAX_STEPS: usize = 50;
/// A point is undistorted when the normalized residual is below this.
const TOL: f64 = 1e-6;
/// Distance in pixels within which a sample point counts as inside the image
/// (warp.rs snaps points this close onto the hull).
const SNAP: f64 = 1e-9;
/// 'full' may grow each side by at most this factor.
const MAX_GROWTH: f64 = 8.;
/// Pixels per cancellation poll.
const POLL: usize = 1 << 16;

/// The names this module answers.
pub(super) const NAMES: [&str; 2] = ["undistortImage", "undistortPoints"];

fn camera_arg(e: &Engine, who: &str, v: Option<&Value>) -> Result<Intrinsics> {
    let c = v.and_then(|v| camera_of(e, v)).ok_or_else(|| {
        err(
            who,
            "the second input must be a cameraIntrinsics or cameraParameters object",
        )
    })?;
    Ok(match &*c {
        Camera::Intrinsics(i) => i.clone(),
        Camera::Params(p) => p.intr.clone(),
        _ => {
            return Err(err(
                who,
                "the second input must be a cameraIntrinsics or cameraParameters object",
            ));
        }
    })
}

/// Normalized coordinates of a pixel for intrinsics `f`, `c`, skew `s`.
fn normalize(f: [f64; 2], c: [f64; 2], s: f64, u: f64, v: f64) -> (f64, f64) {
    let y = (v - c[1]) / f[1];
    ((u - c[0] - s * y) / f[0], y)
}

fn pixel(intr: &Intrinsics, x: f64, y: f64) -> (f64, f64) {
    (
        intr.focal[0] * x + intr.skew * y + intr.principal[0],
        intr.focal[1] * y + intr.principal[1],
    )
}

/// The distortion map and its Jacobian [[dxd/dx, dxd/dy], [dyd/dx, dyd/dy]].
fn jacobian(intr: &Intrinsics, x: f64, y: f64) -> [[f64; 2]; 2] {
    let k = |i: usize| intr.radial.get(i).copied().unwrap_or(0.);
    let r2 = x * x + y * y;
    let radial = 1. + r2 * (k(0) + r2 * (k(1) + r2 * k(2)));
    let d = k(0) + r2 * (2. * k(1) + 3. * r2 * k(2));
    let [p1, p2] = intr.tangential;
    [
        [
            radial + 2. * x * x * d + 2. * p1 * y + 6. * p2 * x,
            2. * x * y * d + 2. * p1 * x + 2. * p2 * y,
        ],
        [
            2. * x * y * d + 2. * p1 * x + 2. * p2 * y,
            radial + 2. * y * y * d + 6. * p1 * y + 2. * p2 * x,
        ],
    ]
}

/// Newton's method on the distortion map from `(xd, yd)`. Returns the best
/// iterate `(x, y)` and its normalized residual norm.
fn newton_undistort(intr: &Intrinsics, xd: f64, yd: f64) -> Option<(f64, f64, f64)> {
    if !(xd.is_finite() && yd.is_finite()) {
        return None;
    }
    let residual = |x: f64, y: f64| {
        let (a, b) = distort(intr, x, y);
        (a - xd, b - yd)
    };
    let (mut x, mut y) = (xd, yd);
    let (mut rx, mut ry) = residual(x, y);
    let mut norm = rx.hypot(ry);
    for _ in 0..MAX_STEPS {
        // Below the tolerance, keep stepping while the residual still falls
        // (one more step reaches rounding).
        let j = jacobian(intr, x, y);
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if !(det.is_finite() && det != 0.) {
            break;
        }
        let dx = (j[1][1] * rx - j[0][1] * ry) / det;
        let dy = (j[0][0] * ry - j[1][0] * rx) / det;
        let mut t = 1.;
        let mut improved = false;
        for _ in 0..30 {
            let (nx, ny) = (x - t * dx, y - t * dy);
            let (nrx, nry) = residual(nx, ny);
            let n = nrx.hypot(nry);
            if n < norm {
                (x, y, rx, ry, norm) = (nx, ny, nrx, nry, n);
                improved = true;
                break;
            }
            t /= 2.;
        }
        if !improved || norm == 0. {
            break;
        }
    }
    Some((x, y, norm))
}

/// The normalized point whose distortion is (xd, yd); None when Newton's
/// method does not reach the tolerance in MAX_STEPS steps.
pub(super) fn undistort_normalized(intr: &Intrinsics, xd: f64, yd: f64) -> Option<(f64, f64)> {
    newton_undistort(intr, xd, yd)
        .and_then(|(x, y, norm)| (norm < TOL && x.is_finite() && y.is_finite()).then_some((x, y)))
}

/// A distorted pixel to its undistorted pixel (same K).
fn undistort_pixel(intr: &Intrinsics, u: f64, v: f64) -> Option<(f64, f64)> {
    let (xd, yd) = normalize(intr.focal, intr.principal, intr.skew, u, v);
    undistort_normalized(intr, xd, yd).map(|(x, y)| pixel(intr, x, y))
}

// ------------------------------------------------------------- undistortImage

#[derive(Clone, Copy, PartialEq)]
enum View {
    Same,
    Full,
    Valid,
}

/// Output pixel box in the 'same' pixel frame: columns c0..c0+cols-1 and rows
/// r0..r0+rows-1 (whole numbers).
#[derive(Clone, Copy)]
struct Box2 {
    c0: f64,
    r0: f64,
    rows: usize,
    cols: usize,
}

/// Checks the dense element limit and memory budget for the output and its
/// 16-byte/pixel source map before allocating.
fn admit_map(e: &Engine, who: &str, b: Box2, c: usize, input: usize) -> Result<()> {
    image_basic::admit(who, b.rows, b.cols, c)?;
    let px = b.rows.saturating_mul(b.cols);
    // remap also holds two block-sized coordinate buffers per active worker
    // and a small list of output slices. Conversion may instead hold both
    // the double output and its class-specific representation.
    let blocks = px.div_ceil(POLL);
    let workers = crate::parallel::current_worker_count().max(1);
    let chunk = px.min(POLL.saturating_mul(workers));
    let warp = px
        .saturating_mul(c.saturating_add(2))
        .saturating_add(chunk.saturating_mul(2))
        .saturating_add(blocks.saturating_mul(3usize.saturating_add(2 * c)));
    let conversion = px.saturating_mul(c.saturating_mul(2).saturating_add(2));
    e.preflight_elements(input.saturating_add(warp.max(conversion)))
}

/// Source points (input intrinsic x, y) of every output pixel of `b`,
/// column-major.
fn source_map(e: &Engine, intr: &Intrinsics, b: Box2) -> Result<(Vec<f64>, Vec<f64>)> {
    source_map_with(intr, b, || image_basic::cancelled(&e.cancel))
}

fn source_map_with(
    intr: &Intrinsics,
    b: Box2,
    mut poll: impl FnMut() -> Result<()>,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n = b.rows * b.cols;
    let (mut xs, mut ys) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut since_poll = 0usize;
    for j in 0..b.cols {
        let u = b.c0 + j as f64;
        for i in 0..b.rows {
            let v = b.r0 + i as f64;
            let (x, y) = normalize(intr.focal, intr.principal, intr.skew, u, v);
            let (xd, yd) = distort(intr, x, y);
            // K (xd, yd) as (u, v) plus K times the distortion shift, so no
            // distortion gives (u, v) exactly.
            let (dx, dy) = (xd - x, yd - y);
            xs.push(u + intr.focal[0] * dx + intr.skew * dy);
            ys.push(v + intr.focal[1] * dy);
            since_poll += 1;
            if since_poll >= POLL {
                poll()?;
                since_poll = 0;
            }
        }
    }
    Ok((xs, ys))
}

/// The undistorted border pixel centres of an h-by-w image: left, right,
/// top and bottom edges (u or v of each).
fn border(who: &str, intr: &Intrinsics, h: usize, w: usize) -> Result<[Vec<(f64, f64)>; 4]> {
    let map = |u: f64, v: f64| {
        undistort_pixel(intr, u, v).ok_or_else(|| {
            err(
                who,
                "the lens distortion cannot be inverted at the image border; use OutputView 'same'",
            )
        })
    };
    let (wf, hf) = (w as f64, h as f64);
    let left = (1..=h).map(|r| map(1., r as f64)).collect::<Result<_>>()?;
    let right = (1..=h).map(|r| map(wf, r as f64)).collect::<Result<_>>()?;
    let top = (1..=w).map(|c| map(c as f64, 1.)).collect::<Result<_>>()?;
    let bottom = (1..=w).map(|c| map(c as f64, hf)).collect::<Result<_>>()?;
    Ok([left, right, top, bottom])
}

fn full_box(e: &Engine, who: &str, intr: &Intrinsics, h: usize, w: usize) -> Result<Box2> {
    e.preflight_elements(h.saturating_add(w).saturating_mul(4))?;
    let edges = border(who, intr, h, w)?;
    let all = edges.iter().flatten();
    let (mut ul, mut uh, mut vl, mut vh) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for &(u, v) in all {
        ul = ul.min(u);
        uh = uh.max(u);
        vl = vl.min(v);
        vh = vh.max(v);
    }
    // Whole pixels; 1e-9 keeps a centre that lands on a pixel from adding one.
    let (c0, c1) = ((ul + 1e-9).floor(), (uh - 1e-9).ceil());
    let (r0, r1) = ((vl + 1e-9).floor(), (vh - 1e-9).ceil());
    let cols = c1 - c0 + 1.;
    let rows = r1 - r0 + 1.;
    if !(cols.is_finite() && rows.is_finite())
        || cols > MAX_GROWTH * w as f64
        || rows > MAX_GROWTH * h as f64
    {
        return Err(err(
            who,
            "the 'full' undistorted image would be more than 8 times the input size; use OutputView 'same' or 'valid'",
        ));
    }
    Ok(Box2 {
        c0,
        r0,
        rows: rows as usize,
        cols: cols as usize,
    })
}

/// Find the maximum-area rectangle of source-valid pixel centres using a
/// histogram per row. The enclosing full box supplies the search domain.
fn valid_box(
    e: &Engine,
    who: &str,
    intr: &Intrinsics,
    h: usize,
    w: usize,
    c: usize,
) -> Result<(Box2, Vec<f64>, Vec<f64>)> {
    let domain = full_box(e, who, intr, h, w)?;
    e.preflight_elements(
        h.saturating_mul(w)
            .saturating_mul(c)
            .saturating_add(domain.cols.saturating_mul(2)),
    )?;
    let mut heights = vec![0usize; domain.cols];
    let mut stack: Vec<usize> = Vec::with_capacity(domain.cols);
    let mut best = (0usize, 0usize, 0usize, 0usize, 0usize);
    for r in 0..domain.rows {
        for (col, height) in heights.iter_mut().enumerate() {
            if col % POLL == 0 {
                e.check_cancel()?;
            }
            let u = domain.c0 + col as f64;
            let v = domain.r0 + r as f64;
            let (x, y) = normalize(intr.focal, intr.principal, intr.skew, u, v);
            let (xd, yd) = distort(intr, x, y);
            let sx = u + intr.focal[0] * (xd - x) + intr.skew * (yd - y);
            let sy = v + intr.focal[1] * (yd - y);
            *height = if (1. - SNAP..=w as f64 + SNAP).contains(&sx)
                && (1. - SNAP..=h as f64 + SNAP).contains(&sy)
            {
                *height + 1
            } else {
                0
            };
        }
        stack.clear();
        for col in 0..=domain.cols {
            let height = if col == domain.cols { 0 } else { heights[col] };
            while stack.last().is_some_and(|&i| heights[i] > height) {
                let i = stack.pop().unwrap();
                let left = stack.last().map_or(0, |&j| j + 1);
                let area = heights[i] * (col - left);
                if area > best.0 {
                    best = (area, left, r + 1 - heights[i], col - left, heights[i]);
                }
            }
            if col < domain.cols {
                stack.push(col);
            }
        }
    }
    if best.0 == 0 {
        return Err(err(
            who,
            "the distortion leaves no rectangle of valid pixels; use OutputView 'same' or 'full'",
        ));
    }
    let b = Box2 {
        c0: domain.c0 + best.1 as f64,
        r0: domain.r0 + best.2 as f64,
        cols: best.3,
        rows: best.4,
    };
    drop(stack);
    drop(heights);
    admit_map(e, who, b, c, h.saturating_mul(w).saturating_mul(c))?;
    let (xs, ys) = source_map(e, intr, b)?;
    Ok((b, xs, ys))
}

fn view_arg(e: &Engine, who: &str, v: &Value) -> Result<View> {
    let text = text_arg(e, v).unwrap_or_default();
    Ok(match text.to_ascii_lowercase().as_str() {
        "same" => View::Same,
        "full" => View::Full,
        "valid" => View::Valid,
        _ => {
            return Err(err(who, "OutputView must be 'same', 'full' or 'valid'"));
        }
    })
}

/// `[J, camIntrinsics] = undistortImage(I, intrinsics, interp, Name=Value)`.
pub(super) fn undistort_image(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    let who = "undistortImage";
    if nargout > 2 {
        return Err("Too many output arguments.".into());
    }
    if args.len() < 2 {
        return Err(err(
            who,
            "expected undistortImage(I, intrinsics, interp, OutputView=..., FillValues=...)",
        ));
    }
    if camera_of(e, &args[0]).is_some() {
        return Err(err(who, "the first input must be the image I"));
    }
    let intr = camera_arg(e, who, args.get(1))?;
    let dims = args[0].dims().into_owned();
    if dims.len() > 3 {
        return Err(err(who, "expected a 2-D image or an H-by-W-by-C array"));
    }
    let (h, w, c) = (dims[0], dims[1], dims.get(2).copied().unwrap_or(1));
    if h == 0 || w == 0 {
        return Err(err(who, "I must not be empty"));
    }
    if !(c == 1 || c == 3) {
        return Err(err(
            who,
            "I must be M-by-N (grayscale) or M-by-N-by-3 (RGB)",
        ));
    }
    let mut rest = &args[2..];
    let mut interp = Interp::Linear;
    if let Some(first) = rest.first()
        && let Some(m) = text_arg(e, first).as_deref().and_then(Interp::parse)
    {
        interp = m;
        rest = &rest[1..];
    }
    let mut view = View::Same;
    let mut fill: Option<Vec<f64>> = None;
    for (name, v) in pairs(e, who, rest, &["OutputView", "FillValues"])? {
        if name == "OutputView" {
            view = view_arg(e, who, v)?;
        } else {
            let n = num(e, who, "FillValues", v)?;
            if n.data.is_empty() || n.data.iter().any(|x| x.is_nan()) {
                return Err(err(who, "FillValues must be a real scalar or vector"));
            }
            fill = Some(n.data);
        }
    }
    let fills = match fill {
        None => vec![0.; c],
        Some(v) if v.len() == 1 => vec![v[0]; c],
        Some(v) if v.len() == c => v,
        Some(v) => {
            return Err(err(
                who,
                &format!(
                    "FillValues must be a scalar or have one value per channel ({c}), not {} values",
                    v.len()
                ),
            ));
        }
    };
    let identity = intr.radial.iter().all(|&v| v == 0.) && intr.tangential == [0., 0.];
    let (b, xs, ys) = if identity {
        (
            Box2 {
                c0: 1.,
                r0: 1.,
                rows: h,
                cols: w,
            },
            Vec::new(),
            Vec::new(),
        )
    } else {
        match view {
            View::Same => {
                let b = Box2 {
                    c0: 1.,
                    r0: 1.,
                    rows: h,
                    cols: w,
                };
                admit_map(e, who, b, c, h.saturating_mul(w).saturating_mul(c))?;
                let (xs, ys) = source_map(e, &intr, b)?;
                (b, xs, ys)
            }
            View::Full => {
                let b = full_box(e, who, &intr, h, w)?;
                admit_map(e, who, b, c, h.saturating_mul(w).saturating_mul(c))?;
                let (xs, ys) = source_map(e, &intr, b)?;
                (b, xs, ys)
            }
            View::Valid => valid_box(e, who, &intr, h, w, c)?,
        }
    };
    let result = if identity {
        // Keep the shared original storage: conversion through f64 would lose
        // precision for 64-bit integers and is unnecessary for this map.
        if !matches!(
            &args[0],
            Value::Matrix(_)
                | Value::Logical(_)
                | Value::Container(Array::Numeric(_) | Array::Logical(_) | Array::Typed(_))
        ) {
            image_basic::image(e, who, &args[0])?;
        }
        args[0].clone()
    } else {
        let img = image_basic::image(e, who, &args[0])?;
        let mut data = remap(
            &e.cancel,
            &img.data,
            [h, w],
            &fills,
            interp,
            [b.rows, b.cols],
            &xs,
            &ys,
        )?;
        if img.kind == Kind::Logical {
            for v in &mut data {
                *v = f64::from(u8::from(*v > 0.5));
            }
        }
        image_basic::to_value(
            e,
            Img {
                h: b.rows,
                w: b.cols,
                c,
                kind: img.kind,
                data,
            },
        )?
    };
    let mut outputs = vec![result];
    if nargout > 1 {
        // Output pixel (1, 1) is pixel (c0, r0) of the 'same' frame.
        let shifted = Intrinsics {
            focal: intr.focal,
            principal: [
                intr.principal[0] - (b.c0 - 1.),
                intr.principal[1] - (b.r0 - 1.),
            ],
            size: Some([b.rows as f64, b.cols as f64]),
            radial: vec![0., 0.],
            tangential: [0., 0.],
            skew: intr.skew,
        };
        outputs.push(new_object(e, Camera::Intrinsics(shifted))?);
    }
    Ok(outputs)
}

// ------------------------------------------------------------ undistortPoints

/// `[undistortedPoints, reprojectionErrors] = undistortPoints(points, intrinsics)`.
pub(super) fn undistort_points(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    let who = "undistortPoints";
    if nargout > 2 {
        return Err("Too many output arguments.".into());
    }
    let [points, cam] = args else {
        return Err(err(who, "expected undistortPoints(points, intrinsics)"));
    };
    if camera_of(e, points).is_some() {
        return Err(err(
            who,
            "the first input must be the M-by-2 matrix of points",
        ));
    }
    let intr = camera_arg(e, who, Some(cam))?;
    let double = matches!(
        points,
        Value::Matrix(_) | Value::Container(Array::Numeric(_))
    );
    if matches!(
        points,
        Value::Logical(_) | Value::Container(Array::Logical(_))
    ) {
        return Err(err(who, "points must be numeric"));
    }
    let p = num(e, who, "points", points)?;
    if p.dims.len() != 2 || p.dims[1] != 2 {
        return Err(err(
            who,
            "points must be an M-by-2 matrix of [x y] coordinates",
        ));
    }
    let m = p.dims[0];
    let mut data = vec![f64::NAN; 2 * m];
    let mut errors = vec![f64::NAN; m];
    for i in 0..m {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        let (u, v) = (p.data[i], p.data[m + i]);
        let (xd, yd) = normalize(intr.focal, intr.principal, intr.skew, u, v);
        let Some((x, y, norm)) = newton_undistort(&intr, xd, yd) else {
            continue;
        };
        if !(norm < 1e-6 && x.is_finite() && y.is_finite()) {
            continue;
        }
        let (pu, pv) = pixel(&intr, x, y);
        data[i] = pu;
        data[m + i] = pv;
        let (rx, ry) = distort(&intr, x, y);
        let (du, dv) = pixel(&intr, rx, ry);
        errors[i] = (du - u).hypot(dv - v);
    }
    let make = |rows: usize, cols: usize, data: Vec<f64>| -> Result<Value> {
        Ok(if double {
            Value::Matrix(Matrix::from_column_vec(rows, cols, data)?)
        } else {
            Value::Container(Array::Typed(TypedArray::from_reals(
                NumericClass::Single,
                rows,
                cols,
                data.into_iter(),
            )?))
        })
    };
    let mut outputs = vec![make(m, 2, data)?];
    if nargout > 1 {
        outputs.push(make(m, 1, errors)?);
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intr(radial: Vec<f64>, tangential: [f64; 2]) -> Intrinsics {
        Intrinsics {
            focal: [800., 790.],
            principal: [322., 238.],
            size: Some([480., 640.]),
            radial,
            tangential,
            skew: 0.,
        }
    }

    #[test]
    fn newton_inverts_distortion() {
        let c = intr(vec![-0.12, 0.05, 0.01], [1e-3, -5e-4]);
        for &(x, y) in &[
            (0., 0.),
            (0.4, 0.3),
            (-0.45, 0.35),
            (0.1, -0.02),
            (-0.5, -0.4),
        ] {
            let (xd, yd) = distort(&c, x, y);
            let (bx, by) = undistort_normalized(&c, xd, yd).expect("converges");
            assert!(
                (bx - x).abs() < 1e-13 && (by - y).abs() < 1e-13,
                "{x} {y} -> {bx} {by}"
            );
        }
    }

    #[test]
    fn source_map_polls_non_multiple_size() {
        let c = intr(vec![-0.12, 0.05], [0., 0.]);
        let b = Box2 {
            c0: 1.,
            r0: 1.,
            rows: 300,
            cols: 300,
        };
        let mut polls = 0usize;
        let (xs, ys) = source_map_with(&c, b, || {
            polls += 1;
            Ok(())
        })
        .expect("source_map_with succeeds");
        assert_eq!(xs.len(), 90_000);
        assert_eq!(ys.len(), 90_000);
        assert_eq!(polls, 1);

        let b = Box2 {
            rows: 70_000,
            cols: 1,
            ..b
        };
        let mut polls = 0usize;
        source_map_with(&c, b, || {
            polls += 1;
            Ok(())
        })
        .expect("source_map_with succeeds");
        assert!(polls >= 1, "expected a poll for a single long column");
    }
}
