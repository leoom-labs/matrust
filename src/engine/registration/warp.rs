//! imwarp (piece 7c2): apply a 2-D geometric transformation (or a
//! displacement field) to an image by inverse mapping, written from the
//! public MathWorks imwarp reference page. No MathWorks code or examples.
//!
//! Every output pixel centre (world coordinates of the output view RB) is
//! mapped through the inverse transformation into input world coordinates,
//! then into the input's intrinsic coordinates (imref2d RA) and sampled:
//!
//! - default output view: the bounding box of the transformed input world
//!   limits (outputLimits), sampled at the input's pixel extent; the pixel
//!   count per axis is ceil(span / extent) (1e-9 tolerance) and the limits
//!   grow by half the remainder on each side, so the box stays centred.
//! - interpolation: nearest (round half away from zero), linear (bilinear)
//!   or cubic (Keys 1981 cubic convolution, a = -0.5).
//! - edges: a sample point inside the pixel-centre hull [1, n] x [1, m] is
//!   interpolated (cubic taps past the border repeat the edge pixel); any
//!   other point gets FillValues. Points within 1e-9 pixel of the hull snap
//!   onto it. SmoothEdges pads the input with FillValues: the hull grows to
//!   [-1, n+2] x [-1, m+2] and every tap outside the image reads FillValues,
//!   so the output ramps to the fill colour (a 2-pixel pad covers the cubic
//!   kernel; any wider pad gives the same values).
//!
//! The Rust API (`default_view`, `warp`) lets imregister warp f64 planes
//! without going through Values.
use super::super::image_basic::{self, Img, Kind as ImageKind};
use super::{Geo, Imref2d, Tform, geo_of, imref_of, new_object, tform_of};
use crate::Result;
use crate::engine::{Engine, Value};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

const WHO: &str = "imwarp";

/// Sample points per cancellation poll (and per parallel work item).
const BLOCK: usize = 1 << 16;

/// Work (output pixels times (taps per channel + 8 for the point mapping))
/// from which the blocks run on the worker pool. 2026-11-17, i7-12700KF under
/// load (gate profile): 594^2 linear output 6.8 ms on 1 worker, 4.1 ms on 8;
/// 2373^2 cubic 387 ms vs 56 ms. Smaller warps stay on the calling thread.
const PAR_WORK: usize = 1 << 22;

/// Distance in pixels within which a sample point snaps onto the hull.
const SNAP: f64 = 1e-9;

/// Padding in pixels of FillValues for SmoothEdges.
const PAD: f64 = 2.;

/// Interpolation method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum Interp {
    Nearest,
    Linear,
    Cubic,
}

impl Interp {
    /// 'nearest', 'linear' (or 'bilinear'), 'cubic' (or 'bicubic'); any case.
    pub(in crate::engine) fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "nearest" => Self::Nearest,
            "linear" | "bilinear" => Self::Linear,
            "cubic" | "bicubic" => Self::Cubic,
            _ => return None,
        })
    }
    fn taps(self) -> usize {
        match self {
            Self::Nearest => 1,
            Self::Linear => 4,
            Self::Cubic => 16,
        }
    }
}

/// How one plane is sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) struct Sampling {
    pub interp: Interp,
    /// Value for points outside the image (and for the SmoothEdges padding).
    pub fill: f64,
    /// SmoothEdges: pad the input with `fill` before sampling.
    pub smooth: bool,
}

/// The default output view of `imwarp(A, RA, tform)`: the bounding box of the
/// transformed world limits of RA at RA's pixel extents, widened evenly to a
/// whole number of pixels.
pub(in crate::engine) fn default_view(ra: &Imref2d, t: &Tform) -> Result<Imref2d> {
    let (xl, yl) = t.output_limits(ra.x, ra.y);
    let axis = |limits: [f64; 2], extent: f64| -> Result<(f64, [f64; 2])> {
        let span = limits[1] - limits[0];
        let pixels = span / extent;
        if !(limits[0].is_finite() && limits[1].is_finite() && pixels.is_finite()) {
            return Err(format!(
                "{WHO}: the transformed image has no finite bounding box"
            ));
        }
        let count = (pixels - 1e-9).ceil().max(1.);
        if count > crate::matrix::MAX_ELEMENTS as f64 {
            return Err(format!(
                "{WHO}: the output would have {count} pixels along one axis (more than the {}-element limit); give a smaller 'OutputView'",
                crate::matrix::MAX_ELEMENTS
            ));
        }
        let nudge = (count * extent - span) / 2.;
        Ok((count, [limits[0] - nudge, limits[1] + nudge]))
    };
    let (cols, x) = axis(xl, ra.pixel_extent_x())?;
    let (rows, y) = axis(yl, ra.pixel_extent_y())?;
    Imref2d::with_limits([rows, cols], &x, &y).map_err(|m| format!("{WHO}: {m}"))
}

/// Where the output pixels sample the input.
enum Source<'a> {
    /// Output view `rb`, transformation `t` (input world to output world),
    /// input referencing `ra`.
    Tform {
        ra: &'a Imref2d,
        t: &'a Tform,
        rb: &'a Imref2d,
    },
    /// Displacement field over the output grid: x displacements then y.
    Field { d: &'a [f64] },
    /// Input intrinsic points given per output pixel (column-major).
    Map { x: &'a [f64], y: &'a [f64] },
}

impl Source<'_> {
    /// Input intrinsic coordinates of the output pixels `start..start + x.len()`
    /// (column-major over `rows` rows): pixel centres to world, the inverse
    /// transformation, then RA's world-to-intrinsic map; or the pixel's own
    /// (column, row) plus the displacement.
    fn points(
        &self,
        cancel: &AtomicBool,
        rows: usize,
        start: usize,
        x: &mut [f64],
        y: &mut [f64],
    ) -> Result<()> {
        match self {
            Source::Tform { ra, t, rb } => {
                let (ex, ey) = (rb.pixel_extent_x(), rb.pixel_extent_y());
                for (k, (u, v)) in x.iter_mut().zip(y.iter_mut()).enumerate() {
                    let (i, j) = ((start + k) % rows, (start + k) / rows);
                    // intrinsic_to_world(j + 1, i + 1) with the extents hoisted.
                    *u = rb.x[0] + ((j + 1) as f64 - 0.5) * ex;
                    *v = rb.y[0] + ((i + 1) as f64 - 0.5) * ey;
                }
                t.inverse_points(x, y, || image_basic::cancelled(cancel))
                    .map_err(|m| format!("{WHO}: {m}"))?;
                let (ex, ey) = (ra.pixel_extent_x(), ra.pixel_extent_y());
                for (u, v) in x.iter_mut().zip(y.iter_mut()) {
                    // world_to_intrinsic with the extents hoisted.
                    *u = 0.5 + (*u - ra.x[0]) / ex;
                    *v = 0.5 + (*v - ra.y[0]) / ey;
                }
            }
            Source::Field { d } => {
                let (dx, dy) = d.split_at(d.len() / 2);
                for (k, (u, v)) in x.iter_mut().zip(y.iter_mut()).enumerate() {
                    let n = start + k;
                    *u = (n / rows + 1) as f64 + dx[n];
                    *v = (n % rows + 1) as f64 + dy[n];
                }
            }
            Source::Map { x: mx, y: my } => {
                x.copy_from_slice(&mx[start..start + x.len()]);
                y.copy_from_slice(&my[start..start + y.len()]);
            }
        }
        Ok(())
    }
}

/// Keys (1981) cubic convolution kernel with a = -0.5.
fn keys(s: f64) -> f64 {
    let a = s.abs();
    if a <= 1. {
        (1.5 * a - 2.5) * a * a + 1.
    } else if a < 2. {
        ((-0.5 * a + 2.5) * a - 4.) * a + 2.
    } else {
        0.
    }
}

/// One m-by-n column-major plane with its sampling rule.
struct Sampler<'a> {
    plane: &'a [f64],
    h: i64,
    w: i64,
    s: Sampling,
    /// Hull [lo, hx] x [lo, hy] of the points that are interpolated.
    lo: f64,
    hx: f64,
    hy: f64,
}

impl<'a> Sampler<'a> {
    fn new(plane: &'a [f64], h: usize, w: usize, s: Sampling) -> Self {
        let pad = if s.smooth { PAD } else { 0. };
        Self {
            plane,
            h: h as i64,
            w: w as i64,
            s,
            lo: 1. - pad,
            hx: w as f64 + pad,
            hy: h as f64 + pad,
        }
    }
    /// Pixel (r, c), 1-based: the fill value outside the image when padded,
    /// otherwise the nearest edge pixel.
    fn at(&self, r: i64, c: i64) -> f64 {
        if self.s.smooth && (r < 1 || c < 1 || r > self.h || c > self.w) {
            return self.s.fill;
        }
        let (r, c) = (r.clamp(1, self.h), c.clamp(1, self.w));
        self.plane[((c - 1) * self.h + (r - 1)) as usize]
    }
    /// Separable weighted sum over taps r0.., c0..; zero weights are skipped
    /// so a point on the grid reads exactly one pixel.
    fn combine<const N: usize>(&self, r0: i64, c0: i64, wy: [f64; N], wx: [f64; N]) -> f64 {
        let mut total = 0.;
        for (a, vy) in wy.into_iter().enumerate() {
            if vy == 0. {
                continue;
            }
            let mut row = 0.;
            for (b, vx) in wx.into_iter().enumerate() {
                if vx != 0. {
                    row += vx * self.at(r0 + a as i64, c0 + b as i64);
                }
            }
            total += vy * row;
        }
        total
    }
    fn value(&self, x: f64, y: f64) -> f64 {
        let snap = |v: f64, hi: f64| {
            if v < self.lo && v >= self.lo - SNAP {
                self.lo
            } else if v > hi && v <= hi + SNAP {
                hi
            } else {
                v
            }
        };
        let (x, y) = (snap(x, self.hx), snap(y, self.hy));
        if !(self.lo <= x && x <= self.hx && self.lo <= y && y <= self.hy) {
            return self.s.fill;
        }
        match self.s.interp {
            Interp::Nearest => self.at(y.round() as i64, x.round() as i64),
            Interp::Linear => {
                let (x0, y0) = (x.floor(), y.floor());
                let (fx, fy) = (x - x0, y - y0);
                self.combine(y0 as i64, x0 as i64, [1. - fy, fy], [1. - fx, fx])
            }
            Interp::Cubic => {
                let (x0, y0) = (x.floor(), y.floor());
                let (fx, fy) = (x - x0, y - y0);
                let weights = |f: f64| [keys(1. + f), keys(f), keys(1. - f), keys(2. - f)];
                self.combine(y0 as i64 - 1, x0 as i64 - 1, weights(fy), weights(fx))
            }
        }
    }
}

fn parallel(work: usize) -> bool {
    work >= PAR_WORK && crate::parallel::current_worker_count() > 1
}

/// Warps the `fills.len()` planes of `planes` (column-major, plane after
/// plane, each `size` = [m, n]) onto a `rows`-by-`cols` grid sampled where
/// `source` says; returns the output planes one after another. Each block of
/// 65536 output pixels computes its sample points once for every plane and
/// polls `cancel` (also on pool workers, which large outputs use).
#[allow(clippy::too_many_arguments)]
fn warp_planes(
    cancel: &AtomicBool,
    planes: &[f64],
    size: [usize; 2],
    fills: &[f64],
    interp: Interp,
    smooth: bool,
    source: &Source,
    [rows, cols]: [usize; 2],
) -> Result<Vec<f64>> {
    let plane_len = size[0] * size[1];
    assert_eq!(planes.len(), plane_len * fills.len(), "plane sizes");
    let n = rows * cols;
    let samplers: Vec<Sampler> = fills
        .iter()
        .enumerate()
        .map(|(k, &fill)| {
            let s = Sampling {
                interp,
                fill,
                smooth,
            };
            Sampler::new(
                &planes[k * plane_len..(k + 1) * plane_len],
                size[0],
                size[1],
                s,
            )
        })
        .collect();
    let mut out = vec![0.; n * fills.len()];
    // One work item per block of output pixels: its slice of every plane.
    let mut items: Vec<Vec<&mut [f64]>> = (0..n.div_ceil(BLOCK))
        .map(|_| Vec::with_capacity(fills.len()))
        .collect();
    for plane in out.chunks_mut(n.max(1)) {
        for (b, chunk) in plane.chunks_mut(BLOCK).enumerate() {
            items[b].push(chunk);
        }
    }
    let work = |(b, mut slices): (usize, Vec<&mut [f64]>)| -> Result<()> {
        image_basic::cancelled(cancel)?;
        let len = slices.first().map_or(0, |s| s.len());
        let (mut x, mut y) = (vec![0.; len], vec![0.; len]);
        source.points(cancel, rows, b * BLOCK, &mut x, &mut y)?;
        for (dst, sampler) in slices.iter_mut().zip(&samplers) {
            for ((d, &u), &v) in dst.iter_mut().zip(&x).zip(&y) {
                *d = sampler.value(u, v);
            }
        }
        Ok(())
    };
    let work_size = n.saturating_mul(interp.taps() * fills.len() + 8);
    if parallel(work_size) {
        items.into_par_iter().enumerate().try_for_each(work)?;
    } else {
        items.into_iter().enumerate().try_for_each(work)?;
    }
    Ok(out)
}

/// One plane warped: `plane` is m-by-n (`size`) referenced by `ra`, `t` maps
/// input world to output world, the result is `rb.ImageSize`, column-major.
/// For imregister (7c3) and the Registration Estimator (7c4).
#[allow(dead_code)] // API for imregister (7c3) and the Registration Estimator (7c4).
pub(in crate::engine) fn warp(
    cancel: &AtomicBool,
    plane: &[f64],
    size: [usize; 2],
    ra: &Imref2d,
    t: &Tform,
    rb: &Imref2d,
    s: Sampling,
) -> Result<Vec<f64>> {
    let (rows, cols) = (rb.size[0] as usize, rb.size[1] as usize);
    image_basic::admit(WHO, rows, cols, 1)?;
    let source = Source::Tform { ra, t, rb };
    warp_planes(
        cancel,
        plane,
        size,
        &[s.fill],
        s.interp,
        s.smooth,
        &source,
        [rows, cols],
    )
}

/// The planes of `planes` (each `size` = [m, n], one fill value per plane)
/// sampled at given input intrinsic points: output pixel k (column-major over
/// `[rows, cols]`) reads the point (`x[k]`, `y[k]`). Same edge rules as
/// imwarp, no SmoothEdges. For undistortImage (7d4).
#[allow(clippy::too_many_arguments)]
pub(in crate::engine) fn remap(
    cancel: &AtomicBool,
    planes: &[f64],
    size: [usize; 2],
    fills: &[f64],
    interp: Interp,
    [rows, cols]: [usize; 2],
    x: &[f64],
    y: &[f64],
) -> Result<Vec<f64>> {
    assert!(x.len() == rows * cols && y.len() == x.len(), "map sizes");
    let source = Source::Map { x, y };
    warp_planes(
        cancel,
        planes,
        size,
        fills,
        interp,
        false,
        &source,
        [rows, cols],
    )
}

// ------------------------------------------------------------------ builtin

/// What the second (and third) inputs say.
enum Geometry {
    Tform {
        ra: Imref2d,
        t: Tform,
    },
    /// Displacement field: m, n and the x then y displacements.
    Field {
        m: usize,
        n: usize,
        d: Vec<f64>,
    },
}

const USAGE: &str = "imwarp: expected imwarp(A, tform), imwarp(A, RA, tform) or imwarp(A, D) with tform an affinetform2d, rigidtform2d, simtform2d, transltform2d or affine2d object and D an m-by-n-by-2 displacement field";

const OPTIONS: [&str; 4] = [
    "OutputView",
    "FillValues",
    "SmoothEdges",
    "InterpolationMethod",
];

/// A parameter name: exact or a unique prefix, any case.
fn option(name: &str) -> Result<&'static str> {
    let lower = name.to_ascii_lowercase();
    if let Some(found) = OPTIONS.iter().find(|o| o.eq_ignore_ascii_case(name)) {
        return Ok(found);
    }
    let matches: Vec<_> = OPTIONS
        .iter()
        .filter(|o| !lower.is_empty() && o.to_ascii_lowercase().starts_with(&lower))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        _ => Err(format!(
            "{WHO}: unknown parameter '{name}' (expected 'nearest', 'linear', 'cubic', 'OutputView', 'FillValues', 'SmoothEdges' or 'InterpolationMethod')"
        )),
    }
}

fn class_name(e: &Engine, v: &Value) -> String {
    match geo_of(e, v) {
        Some(g) => g.class().into(),
        None => v
            .clone()
            .array(&e.cancel)
            .map(|a| a.class().to_owned())
            .unwrap_or_else(|_| "unknown".into()),
    }
}

fn geometry(e: &Engine, args: &[Value], h: usize, w: usize) -> Result<(Geometry, usize)> {
    let second = &args[1];
    if let Some(ra) = imref_of(e, second) {
        let Some(t) = args.get(2).and_then(|v| tform_of(e, v)) else {
            return Err(format!(
                "{WHO}: after RA the third input must be a 2-D geometric transformation object (affinetform2d, rigidtform2d, simtform2d, transltform2d or affine2d)"
            ));
        };
        if ra.size != [h as f64, w as f64] {
            return Err(format!(
                "{WHO}: RA.ImageSize [{} {}] must match the size of A ({h}-by-{w})",
                ra.size[0], ra.size[1]
            ));
        }
        return Ok((Geometry::Tform { ra, t }, 3));
    }
    if let Some(t) = tform_of(e, second) {
        let ra = Imref2d::new([h as f64, w as f64]);
        return Ok((Geometry::Tform { ra, t }, 2));
    }
    if geo_of(e, second).is_some() {
        return Err(USAGE.into());
    }
    let dims = second.dims().into_owned();
    let numeric = matches!(
        second,
        Value::Matrix(_)
            | Value::Container(crate::Array::Numeric(_))
            | Value::Container(crate::Array::Typed(_))
    );
    if numeric && dims.len() == 3 && dims[2] == 2 {
        let d = image_basic::numbers(second).ok_or(USAGE)?;
        return Ok((
            Geometry::Field {
                m: dims[0],
                n: dims[1],
                d,
            },
            2,
        ));
    }
    if numeric && dims.len() == 2 && dims == [3, 3] {
        return Err(format!(
            "{USAGE}; a 3-by-3 matrix is not a transformation (use affinetform2d(A) or affine2d(T))"
        ));
    }
    Err(USAGE.into())
}

/// `imwarp(...)` with `nargout` outputs (0 or 1 give B only).
pub(in crate::engine) fn imwarp(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if nargout > 2 {
        return Err("Too many output arguments.".into());
    }
    if args.len() < 2 {
        return Err(format!("{WHO}: not enough input arguments. {USAGE}"));
    }
    if geo_of(e, &args[0]).is_some() {
        return Err(format!(
            "{WHO}: the first input must be the image A, not a {} object",
            class_name(e, &args[0])
        ));
    }
    let img = image_basic::image(e, WHO, &args[0])?;
    if img.h == 0 || img.w == 0 {
        return Err(format!("{WHO}: A must not be empty"));
    }
    let (geometry, used) = geometry(e, args, img.h, img.w)?;
    // Options: interpolation names anywhere, then name-value pairs.
    let mut interp = Interp::Linear;
    let mut fill: Option<Vec<f64>> = None;
    let mut smooth = false;
    let mut view: Option<Imref2d> = None;
    let mut rest = &args[used..];
    while let Some(first) = rest.first() {
        let key = image_basic::text(e, first).ok_or_else(|| {
            format!(
                "{WHO}: expected an interpolation method or a parameter name, not a {} input",
                class_name(e, first)
            )
        })?;
        if let Some(m) = Interp::parse(&key) {
            interp = m;
            rest = &rest[1..];
            continue;
        }
        let name = option(&key)?;
        let value = rest
            .get(1)
            .ok_or_else(|| format!("{WHO}: the parameter '{name}' needs a value"))?;
        match name {
            "OutputView" => {
                view = Some(imref_of(e, value).ok_or_else(|| {
                    format!(
                        "{WHO}: OutputView must be an imref2d object, not a {}",
                        class_name(e, value)
                    )
                })?);
            }
            "FillValues" => {
                let v = image_basic::numbers(value)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| format!("{WHO}: FillValues must be real numeric"))?;
                fill = Some(v);
            }
            "SmoothEdges" => {
                let v = image_basic::numbers(value)
                    .filter(|v| v.len() == 1 && !v[0].is_nan())
                    .ok_or_else(|| {
                        format!("{WHO}: SmoothEdges must be a logical scalar (true or false)")
                    })?;
                smooth = v[0] != 0.;
            }
            _ => {
                let text = image_basic::text(e, value).unwrap_or_default();
                interp = Interp::parse(&text).ok_or_else(|| {
                    format!("{WHO}: InterpolationMethod must be 'nearest', 'linear' or 'cubic'")
                })?;
            }
        }
        rest = &rest[2..];
    }
    let c = img.c;
    let fills = match fill {
        None => vec![0.; c],
        Some(v) if v.len() == 1 => vec![v[0]; c],
        Some(v) if v.len() == c => v,
        Some(v) => {
            return Err(format!(
                "{WHO}: FillValues must be a scalar or have one value per channel ({c}), not {} values",
                v.len()
            ));
        }
    };
    let (rb, rows, cols, d) = match geometry {
        Geometry::Tform { ra, t } => {
            let rb = match view {
                Some(v) => v,
                None => default_view(&ra, &t)?,
            };
            let (rows, cols) = (rb.size[0] as usize, rb.size[1] as usize);
            (Some((ra, t, rb)), rows, cols, Vec::new())
        }
        Geometry::Field { m, n, d } => {
            if view.is_some() {
                return Err(format!(
                    "{WHO}: OutputView is not used with a displacement field (the output has the size of D)"
                ));
            }
            if nargout > 1 {
                return Err(format!(
                    "{WHO}: a displacement field gives one output, B (no RB)"
                ));
            }
            (None, m, n, d)
        }
    };
    image_basic::admit(WHO, rows, cols, c)?;
    e.preflight_elements(rows * cols * c)?;
    let source = match &rb {
        Some((ra, t, rb)) => Source::Tform { ra, t, rb },
        // D(:,:,1) adds to the column (x), D(:,:,2) to the row (y).
        None => Source::Field { d: &d },
    };
    let mut data = warp_planes(
        &e.cancel,
        &img.data,
        [img.h, img.w],
        &fills,
        interp,
        smooth,
        &source,
        [rows, cols],
    )?;
    if img.kind == ImageKind::Logical {
        for v in &mut data {
            *v = f64::from(u8::from(*v > 0.5));
        }
    }
    let out = Img {
        h: rows,
        w: cols,
        c,
        kind: img.kind,
        data,
    };
    let mut outputs = vec![image_basic::to_value(e, out)?];
    if nargout > 1 {
        let (_, _, rb) = rb.expect("checked: two outputs need a transformation");
        outputs.push(new_object(e, Geo::Imref(rb))?);
    }
    Ok(outputs)
}
