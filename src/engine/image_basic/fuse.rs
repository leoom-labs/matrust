//! imfuse and imshowpair: a composite view of two images.
//!
//! Behaviour follows the public MathWorks reference pages of imfuse and
//! imshowpair; where they leave a detail open, the choice is fixed here and
//! stated on the help pages:
//!
//! 1. Each image is put on its class's unit scale (im2double: uint8 / 255,
//!    uint16 / 65535, int16 (v + 32768) / 65535, logical 0/1, floating
//!    unchanged).
//! 2. 'falsecolor' and 'diff' turn RGB images into grayscale with the rgb2gray
//!    weights (the reference pages say so). 'blend', 'checkerboard' and
//!    'montage' keep colour; when only one image is RGB the grayscale one is
//!    repeated in all three channels.
//! 3. Different sizes: both images are padded with zeros (zero of the image's
//!    own class) at the bottom and right to the larger height and width.
//! 4. Scaling, on the padded images: 'independent' maps each image's
//!    smallest..largest finite value to 0..1, 'joint' maps the smallest..largest
//!    finite value of both images to 0..1, 'none' keeps the unit scale. An
//!    image (or pair) without a range keeps its unit-scale values.
//! 5. Composite on the unit scale, then im2uint8 (clip to [0, 1], times 255,
//!    round half away from zero; NaN gives 0):
//!    falsecolor: channel k is A, B or 0 by ColorChannels(k) = 1, 2, 0;
//!    blend: (A + B) / 2; diff: |A - B|; montage: [A B];
//!    checkerboard: tiles of ceil(rows/8) by ceil(cols/8) pixels, the top-left
//!    tile from A, alternating A and B along rows and columns.
//! 6. imfuse(A, RA, B, RB) with imref2d RA, RB: the output grid covers the
//!    union of both world extents ("the output is the union of the world
//!    extents") at RA's pixel extents, starting at the union's lower limits
//!    with ceil(span / extent) pixels (1e-9 pixel tolerance); the grid keeps
//!    the union's own upper limit when its last pixel ends there. An input
//!    whose reference is that grid (same size, limits within the tolerance)
//!    is used as it is; the other is resampled onto the grid with imwarp's
//!    warp (identity, linear; points outside an input get the input's
//!    padding value of rule 3) after the memory preflight of its output.
//!    Then rules 1-5 apply. RC is the imref2d of that grid ('montage': twice
//!    as wide).
use super::super::registration::{self as reg, Imref2d};
use super::super::{Engine, Execution, Value};
use super::{GRAY, Img, POLL, admit, cancelled, image, poll, unit};
use crate::{Array, GraphicsKind, Result, TypedArray, typed_array::TypedData};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    FalseColor,
    Blend,
    Diff,
    Montage,
    Checkerboard,
}

const METHODS: [(&str, Method); 5] = [
    ("falsecolor", Method::FalseColor),
    ("blend", Method::Blend),
    ("diff", Method::Diff),
    ("montage", Method::Montage),
    ("checkerboard", Method::Checkerboard),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scaling {
    Independent,
    Joint,
    None,
}

struct Options {
    method: Method,
    scaling: Scaling,
    /// Source of the red, green and blue channels: 1 = A, 2 = B, 0 = neither.
    channels: [u8; 3],
    /// imshowpair 'Parent': the axes to draw into.
    parent: Option<Value>,
}

/// One input on its class's unit scale, with the unit-scale value of a zero
/// of its class (the padding value).
struct Unit {
    img: Img,
    pad: f64,
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_ref().map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Case-insensitive match against `options`; a unique prefix is enough.
fn choose(who: &str, what: &str, given: &str, options: &[&'static str]) -> Result<&'static str> {
    if let Some(exact) = options.iter().find(|o| o.eq_ignore_ascii_case(given)) {
        return Ok(exact);
    }
    let lower = given.to_ascii_lowercase();
    let prefixed: Vec<&'static str> = options
        .iter()
        .copied()
        .filter(|o| !lower.is_empty() && o.to_ascii_lowercase().starts_with(&lower))
        .collect();
    if let [one] = prefixed.as_slice() {
        return Ok(one);
    }
    let list = options
        .iter()
        .map(|o| format!("'{o}'"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "{who}: unknown {what} '{given}' (expected one of {list})"
    ))
}

/// A spatial referencing object (imref2d) or any other object or struct in
/// the place of an image.
fn is_reference(v: &Value) -> bool {
    matches!(v, Value::Container(Array::Struct(_))) || super::super::classdef::is_object_value(v)
}

fn read(e: &Engine, who: &str, label: &str, v: &Value) -> Result<Unit> {
    let mut img = image(e, who, v)?;
    if img.c != 1 && img.c != 3 {
        return Err(format!(
            "{who}: {label} must be a grayscale (M-by-N) or truecolor (M-by-N-by-3) image"
        ));
    }
    if !super::im2_supported(img.kind) {
        return Err(format!(
            "{who}: {label} must be of class {}, not {}",
            super::IM2_CLASSES,
            img.kind.name()
        ));
    }
    if img.h == 0 || img.w == 0 {
        return Err(format!("{who}: {label} must not be empty"));
    }
    let kind = img.kind;
    for block in img.data.chunks_mut(POLL) {
        cancelled(&e.cancel)?;
        for v in block {
            *v = unit(kind, *v);
        }
    }
    Ok(Unit {
        img,
        pad: unit(kind, 0.),
    })
}

fn channels_of(who: &str, v: &Value) -> Result<[u8; 3]> {
    if let Some(t) = text(v) {
        return Ok(
            match choose(who, "ColorChannels", &t, &["green-magenta", "red-cyan"])? {
                "red-cyan" => [1, 2, 2],
                _ => [2, 1, 2],
            },
        );
    }
    let bad = || {
        format!(
            "{who}: ColorChannels must be 'green-magenta', 'red-cyan' or a vector [R G B] of 1 (A), 2 (B) and 0 (neither)"
        )
    };
    let values = super::numbers(v).ok_or_else(bad)?;
    let [r, g, b] = values.as_slice() else {
        return Err(bad());
    };
    let mut out = [0u8; 3];
    for (slot, &x) in out.iter_mut().zip([r, g, b]) {
        if x != 0. && x != 1. && x != 2. {
            return Err(bad());
        }
        *slot = x as u8;
    }
    Ok(out)
}

/// The two inputs, the options and, for the (A, RA, B, RB) form, the output
/// grid.
type Parsed = (Unit, Unit, Options, Option<Imref2d>);

fn parse(e: &Engine, who: &str, args: &[Value]) -> Result<Parsed> {
    if args.len() < 2 {
        return Err(format!(
            "{who}: expected two images, {who}(A, B) or {who}(A, B, method, Name, Value)"
        ));
    }
    let refs = if is_reference(&args[1]) || args.get(3).is_some_and(is_reference) {
        match (
            reg::imref_of(e, &args[1]),
            args.get(3).and_then(|v| reg::imref_of(e, v)),
        ) {
            (Some(ra), Some(rb)) => Some((ra, rb)),
            _ => {
                return Err(format!(
                    "{who} with spatial referencing objects is not supported yet other than imref2d RA and RB in {who}(A, RA, B, RB)"
                ));
            }
        }
    } else {
        None
    };
    let a = read(e, who, "A", &args[0])?;
    let (b, mut rest) = match &refs {
        Some(_) => (read(e, who, "B", &args[2])?, &args[4..]),
        None => (read(e, who, "B", &args[1])?, &args[2..]),
    };
    let (a, b, grid) = match refs {
        Some((ra, rb)) => {
            let grid = union_grid(who, &ra, &rb)?;
            let a = resample(e, who, "A", a, &ra, &grid)?;
            let b = resample(e, who, "B", b, &rb, &grid)?;
            (a, b, Some(grid))
        }
        None => (a, b, None),
    };
    let mut options = Options {
        method: Method::FalseColor,
        scaling: Scaling::Independent,
        channels: [2, 1, 2],
        parent: None,
    };
    if let Some(t) = rest.first().and_then(text)
        && let Some((_, m)) = METHODS.iter().find(|(n, _)| n.eq_ignore_ascii_case(&t))
    {
        options.method = *m;
        rest = &rest[1..];
    }
    let names: &[&'static str] = if who == "imshowpair" {
        &["Scaling", "ColorChannels", "Parent", "Interpolation"]
    } else {
        &["Scaling", "ColorChannels"]
    };
    if !rest.len().is_multiple_of(2) {
        // A lone text that is no option name was meant as the method.
        let lone = (rest.len() == 1).then(|| text(&rest[0])).flatten();
        return Err(match lone {
            Some(t) if choose(who, "option", &t, names).is_err() => format!(
                "{who}: unknown method '{t}' (expected 'falsecolor', 'blend', 'diff', 'montage' or 'checkerboard')"
            ),
            _ => format!("{who}: name-value arguments must come in pairs"),
        });
    }
    for pair in rest.chunks(2) {
        let Some(name) = text(&pair[0]) else {
            return Err(format!("{who}: option names must be text"));
        };
        match choose(who, "option", &name, names)? {
            "Scaling" => {
                let value = text(&pair[1]).ok_or_else(|| {
                    format!("{who}: Scaling must be 'independent', 'joint' or 'none'")
                })?;
                options.scaling =
                    match choose(who, "Scaling", &value, &["independent", "joint", "none"])? {
                        "independent" => Scaling::Independent,
                        "joint" => Scaling::Joint,
                        _ => Scaling::None,
                    };
            }
            "ColorChannels" => options.channels = channels_of(who, &pair[1])?,
            "Parent" => {
                let axes = matches!(&pair[1], Value::Container(Array::Graphics(g))
                    if g.scalar_handle().is_ok_and(|h| h.kind() == GraphicsKind::Axes));
                if !axes {
                    return Err(format!("{who}: Parent must be an axes handle"));
                }
                options.parent = Some(pair[1].clone());
            }
            _ => {
                let value = text(&pair[1]).ok_or_else(|| {
                    format!("{who}: Interpolation must be 'nearest' or 'bilinear'")
                })?;
                choose(who, "Interpolation", &value, &["nearest", "bilinear"])?;
            }
        }
    }
    Ok((a, b, options, grid))
}

/// Rule 6 tolerance in pixels: limits this close are the same grid line.
const GRID_TOLERANCE: f64 = 1e-9;

/// The output grid of imfuse(A, RA, B, RB): the union of both world extents
/// at RA's pixel extents (rule 6).
fn union_grid(who: &str, ra: &Imref2d, rb: &Imref2d) -> Result<Imref2d> {
    let axis = |a: [f64; 2], b: [f64; 2], extent: f64| -> Result<(f64, [f64; 2])> {
        let (lo, hi) = (a[0].min(b[0]), a[1].max(b[1]));
        let count = ((hi - lo) / extent - GRID_TOLERANCE).ceil().max(1.);
        if !count.is_finite() || count > crate::matrix::MAX_ELEMENTS as f64 {
            return Err(format!(
                "{who}: the union of the world extents spans too many pixels of A"
            ));
        }
        // The union's own upper limit when the last pixel ends on it: limits
        // rebuilt from the count are an ulp off at non-unit extents, which
        // would move an input that spans the union off the grid.
        let end = lo + count * extent;
        // A world span that overflows (limits near -1e308 and 1e308) has an
        // infinite extent or end: no grid. Without this, inf <= inf below
        // would keep `hi` on a one-pixel axis.
        if !(extent.is_finite() && end.is_finite()) {
            return Err(format!(
                "{who}: the union of the world extents does not fit a finite grid"
            ));
        }
        let hi = if (hi - end).abs() <= GRID_TOLERANCE * extent {
            hi
        } else {
            end
        };
        Ok((count, [lo, hi]))
    };
    let (cols, x) = axis(ra.x, rb.x, ra.pixel_extent_x())?;
    let (rows, y) = axis(ra.y, rb.y, ra.pixel_extent_y())?;
    Imref2d::with_limits([rows, cols], &x, &y).map_err(|m| format!("{who}: {m}"))
}

/// Whether `r` is `grid` (same size, limits within the rule 6 tolerance):
/// its image needs no resampling.
fn on_grid(r: &Imref2d, grid: &Imref2d) -> bool {
    let same = |a: [f64; 2], b: [f64; 2], extent: f64| {
        let near = |a: f64, b: f64| (a - b).abs() <= GRID_TOLERANCE * extent;
        near(a[0], b[0]) && near(a[1], b[1])
    };
    r.size == grid.size
        && same(r.x, grid.x, grid.pixel_extent_x())
        && same(r.y, grid.y, grid.pixel_extent_y())
}

/// One input resampled onto `grid` (identity transformation, linear; points
/// outside the input get its padding value).
fn resample(
    e: &Engine,
    who: &str,
    label: &str,
    u: Unit,
    r: &Imref2d,
    grid: &Imref2d,
) -> Result<Unit> {
    let (h, w) = (u.img.h, u.img.w);
    if r.size != [h as f64, w as f64] {
        return Err(format!(
            "{who}: R{label}.ImageSize [{} {}] must match the size of {label} ({h}-by-{w})",
            r.size[0], r.size[1]
        ));
    }
    if on_grid(r, grid) {
        return Ok(u);
    }
    let (rows, cols) = (grid.size[0] as usize, grid.size[1] as usize);
    admit(who, rows, cols, u.img.c)?;
    // Memory budget before any grid-sized buffer, as imwarp does.
    e.preflight_elements(rows * cols * u.img.c)?;
    let identity = reg::Tform::identity(reg::Kind::Affine);
    let sampling = reg::Sampling {
        interp: reg::Interp::Linear,
        fill: u.pad,
        smooth: false,
    };
    // warp's output of the first plane becomes the image data (no second
    // buffer for a one-plane image); later planes are appended to it.
    let mut data = Vec::new();
    for (k, plane) in u.img.data.chunks(h * w).enumerate() {
        let out = reg::warp(&e.cancel, plane, [h, w], r, &identity, grid, sampling)?;
        if k == 0 {
            data = out;
            data.reserve_exact(rows * cols * (u.img.c - 1));
        } else {
            data.extend_from_slice(&out);
        }
    }
    let mut img = u.img;
    img.data = data;
    img.h = rows;
    img.w = cols;
    Ok(Unit { img, pad: u.pad })
}

/// rgb2gray on the unit scale.
fn gray(e: &Engine, u: &mut Unit) -> Result<()> {
    if u.img.c != 3 {
        return Ok(());
    }
    let n = u.img.h * u.img.w;
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        poll(e, i)?;
        let d = &u.img.data;
        data.push(GRAY[0] * d[i] + GRAY[1] * d[n + i] + GRAY[2] * d[2 * n + i]);
    }
    u.img.data = data;
    u.img.c = 1;
    Ok(())
}

/// A grayscale image repeated into three channels.
fn to_rgb(u: &mut Unit) {
    if u.img.c == 1 {
        let plane = std::mem::take(&mut u.img.data);
        u.img.data = plane.repeat(3);
        u.img.c = 3;
    }
}

/// The image padded with its zero at the bottom and right to `h` by `w`.
fn pad(e: &Engine, u: Unit, h: usize, w: usize) -> Result<Vec<f64>> {
    let img = &u.img;
    if img.h == h && img.w == w {
        return Ok(u.img.data);
    }
    let mut data = vec![u.pad; h * w * img.c];
    for k in 0..img.c {
        for col in 0..img.w {
            super::poll_column(e, col, img.h)?;
            let from = k * img.h * img.w + col * img.h;
            let to = k * h * w + col * h;
            data[to..to + img.h].copy_from_slice(&img.data[from..from + img.h]);
        }
    }
    Ok(data)
}

/// Smallest and largest finite values of all `sets`.
fn range(sets: &[&[f64]]) -> Option<(f64, f64)> {
    let (lo, hi) = sets
        .iter()
        .flat_map(|s| s.iter())
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    (lo < hi).then_some((lo, hi))
}

fn rescale(e: &Engine, data: &mut [f64], range: Option<(f64, f64)>) -> Result<()> {
    let Some((lo, hi)) = range else {
        return Ok(());
    };
    let span = hi - lo;
    for block in data.chunks_mut(POLL) {
        cancelled(&e.cancel)?;
        for v in block {
            *v = (*v - lo) / span;
        }
    }
    Ok(())
}

/// im2uint8 of a unit-scale value: clip to [0, 1], times 255, round half
/// away from zero; NaN gives 0.
fn level(v: f64) -> u8 {
    if v.is_nan() {
        0
    } else {
        (v.clamp(0., 1.) * 255.).round() as u8
    }
}

/// `level` of every element.
fn levels(e: &Engine, data: &[f64]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks(POLL) {
        cancelled(&e.cancel)?;
        out.extend(block.iter().map(|&v| level(v)));
    }
    Ok(out)
}

/// `f` of corresponding elements of `a` and `b`, as levels.
fn levels2(e: &Engine, a: &[f64], b: &[f64], f: impl Fn(f64, f64) -> f64) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(a.len());
    for (x, y) in a.chunks(POLL).zip(b.chunks(POLL)) {
        cancelled(&e.cancel)?;
        out.extend(x.iter().zip(y).map(|(&x, &y)| level(f(x, y))));
    }
    Ok(out)
}

/// The composite of `a` and `b`: a uint8 array (2-D or M-by-N-by-3).
fn fuse(e: &Engine, who: &str, mut a: Unit, mut b: Unit, o: &Options) -> Result<Value> {
    if matches!(o.method, Method::FalseColor | Method::Diff) {
        gray(e, &mut a)?;
        gray(e, &mut b)?;
    } else if a.img.c != b.img.c {
        to_rgb(&mut a);
        to_rgb(&mut b);
    }
    let (h, w, c) = (a.img.h.max(b.img.h), a.img.w.max(b.img.w), a.img.c);
    let out_w = if o.method == Method::Montage {
        2 * w
    } else {
        w
    };
    let out_c = if o.method == Method::FalseColor { 3 } else { c };
    admit(who, h, out_w, out_c)?;
    let units = super::super::limits::memory_units(&e.limits);
    if h.saturating_mul(w).saturating_mul(c).saturating_mul(2) > units {
        return Err(super::super::limits::memory_error(
            &format!("{who}: two padded {h}-by-{w}-by-{c} images"),
            &e.limits,
        ));
    }
    let mut pa = pad(e, a, h, w)?;
    let mut pb = pad(e, b, h, w)?;
    match o.scaling {
        Scaling::Independent => {
            let (ra, rb) = (range(&[&pa]), range(&[&pb]));
            rescale(e, &mut pa, ra)?;
            rescale(e, &mut pb, rb)?;
        }
        Scaling::Joint => {
            let r = range(&[&pa, &pb]);
            rescale(e, &mut pa, r)?;
            rescale(e, &mut pb, r)?;
        }
        Scaling::None => {}
    }
    let plane = h * w;
    let data: Vec<u8> = match o.method {
        Method::FalseColor => {
            let (a8, b8) = (levels(e, &pa)?, levels(e, &pb)?);
            let mut data = Vec::with_capacity(3 * plane);
            for source in o.channels {
                match source {
                    1 => data.extend_from_slice(&a8),
                    2 => data.extend_from_slice(&b8),
                    _ => data.resize(data.len() + plane, 0),
                }
            }
            data
        }
        Method::Blend => levels2(e, &pa, &pb, |x, y| (x + y) / 2.)?,
        Method::Diff => levels2(e, &pa, &pb, |x, y| (x - y).abs())?,
        Method::Checkerboard => {
            let (th, tw) = (h.div_ceil(8), w.div_ceil(8));
            let mut data = Vec::with_capacity(pa.len());
            for k in 0..c {
                for col in 0..w {
                    super::poll_column(e, col, h)?;
                    let at = k * plane + col * h;
                    for row in 0..h {
                        let from_a = (row / th + col / tw).is_multiple_of(2);
                        data.push(level(if from_a { pa[at + row] } else { pb[at + row] }));
                    }
                }
            }
            data
        }
        Method::Montage => {
            let (a8, b8) = (levels(e, &pa)?, levels(e, &pb)?);
            let mut data = Vec::with_capacity(2 * pa.len());
            for k in 0..c {
                data.extend_from_slice(&a8[k * plane..(k + 1) * plane]);
                data.extend_from_slice(&b8[k * plane..(k + 1) * plane]);
            }
            data
        }
    };
    let fused = TypedArray::checked(h, out_w * out_c, TypedData::Uint8(data.into()))?;
    let fused = if out_c == 3 {
        fused.with_dims(&[h, out_w, 3])?
    } else {
        fused
    };
    Ok(Value::Container(Array::Typed(fused)))
}

/// `C = imfuse(A, B, ...)`.
pub(in crate::engine) fn imfuse(
    e: &mut Engine,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    if count > 2 {
        return Err("imfuse: too many output arguments".into());
    }
    let (a, b, options, grid) = parse(e, "imfuse", args)?;
    let Some(grid) = grid else {
        if count > 1 {
            return Err(
                "imfuse: the second output RC (a spatial referencing object) is not supported yet"
                    .into(),
            );
        }
        return Ok(vec![fuse(e, "imfuse", a, b, &options)?]);
    };
    let montage = options.method == Method::Montage;
    let fused = fuse(e, "imfuse", a, b, &options)?;
    if count < 2 {
        return Ok(vec![fused]);
    }
    let rc = if montage {
        let x = [grid.x[0], grid.x[0] + 2. * (grid.x[1] - grid.x[0])];
        Imref2d::with_limits([grid.size[0], 2. * grid.size[1]], &x, &grid.y)
            .map_err(|m| format!("imfuse: {m}"))?
    } else {
        grid
    };
    let rc = reg::new_object(e, reg::Geo::Imref(rc))?;
    Ok(vec![fused, rc])
}

/// `imshowpair(A, B, ...)` / `h = imshowpair(...)`: imshow of the imfuse
/// composite in the current axes (or 'Parent').
pub(in crate::engine) fn imshowpair(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > 1 {
        return Err("imshowpair: too many output arguments".into());
    }
    let (a, b, options, _) = parse(e, "imshowpair", args)?;
    let parent = options.parent.clone();
    let fused = fuse(e, "imshowpair", a, b, &options)?;
    let mut draw = Vec::with_capacity(2);
    draw.extend(parent);
    draw.push(fused);
    let handle = super::super::graphics_handles::surface_call(e, "imshow", &draw, 1, out)?;
    Ok(if count == 0 { Vec::new() } else { vec![handle] })
}
