//! Basic image processing: class and range conversion (im2double, im2single,
//! im2uint8, im2uint16, mat2gray, imcomplement), colour (rgb2gray, ind2rgb and
//! H-by-W-by-3 input for rgb2hsv/hsv2rgb), geometry (imresize, imrotate,
//! imcrop, imtranslate) and filters (fspecial, imfilter, imgaussfilt,
//! medfilt2). JPEG, BMP and TIFF files for imread/imwrite/imfinfo live in
//! `codecs` and are called from files_more.
//!
//! Behaviour follows the public MathWorks reference pages (classes, defaults,
//! documented formulas) and published methods: the cubic convolution kernel of
//! R. G. Keys, "Cubic convolution interpolation for digital image processing",
//! IEEE Trans. ASSP 29(6), 1981 (a = -0.5), and the hexcone colour model of
//! A. R. Smith, "Color gamut transform pairs", SIGGRAPH 1978. No MathWorks code.
//!
//! Images are handled as column-major planes in f64 (`Img`); integer results
//! are rounded half away from zero and saturated when they are stored back in
//! their class. Every function here is a free function; the one `impl Engine`
//! method uses the `image_basic_` prefix so parallel tracks cannot collide.
use super::{Engine, Value};
use crate::{Array, LogicalMatrix, Matrix, NumericClass, Result, TypedArray};
use std::sync::atomic::AtomicBool;

pub(super) mod codecs;
mod filters;
mod fuse;
mod geometry;
pub(super) mod registration_config;

const NAMES: &[&str] = &[
    "im2double",
    "im2single",
    "im2uint8",
    "im2uint16",
    "mat2gray",
    "imcomplement",
    "rgb2gray",
    "ind2rgb",
    "imresize",
    "imrotate",
    "imcrop",
    "imtranslate",
    "fspecial",
    "imfilter",
    "imgaussfilt",
    "medfilt2",
];

/// Elements between cancellation polls.
const POLL: usize = 1 << 16;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name) || matches!(name, "rgb2hsv" | "hsv2rgb")
}

/// Registration set-up and display (imregconfig objects, imfuse, imshowpair).
pub(super) fn registration_handles(name: &str) -> bool {
    matches!(name, "imregconfig" | "imfuse" | "imshowpair")
}

/// imregconfig, imfuse and imshowpair with `count` outputs (0 for a statement).
pub(super) fn registration_call(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut super::Execution,
) -> Result<Vec<Value>> {
    engine.check_cancel()?;
    match name {
        "imregconfig" => registration_config::imregconfig(engine, args, count.max(1)),
        "imfuse" => fuse::imfuse(engine, args, count),
        _ => fuse::imshowpair(engine, args, count, out),
    }
}

impl Engine {
    /// Single-output entry; None when `name` is not handled here (rgb2hsv and
    /// hsv2rgb are taken only for H-by-W-by-3 input, the rest stays in misc_more).
    pub(super) fn image_basic_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        if matches!(name, "rgb2hsv" | "hsv2rgb") {
            if args.len() != 1
                || (args[0].ndims() != 3 && !matches!(&args[0], Value::Container(Array::Typed(_))))
            {
                return Ok(None);
            }
            return hsv_image(self, name, &args[0]).map(Some);
        }
        self.check_cancel()?;
        let value = match name {
            "im2double" | "im2single" | "im2uint8" | "im2uint16" => {
                convert_class(self, name, args)?
            }
            "mat2gray" => mat2gray(self, args)?,
            "imcomplement" => imcomplement(self, args)?,
            "rgb2gray" => rgb2gray(self, args)?,
            "ind2rgb" => ind2rgb(self, args)?,
            "imresize" => geometry::imresize(self, args)?,
            "imrotate" => geometry::imrotate(self, args)?,
            "imcrop" => geometry::imcrop(self, args)?,
            "imtranslate" => geometry::imtranslate(self, args)?,
            "fspecial" => filters::fspecial(self, args)?,
            "imfilter" => filters::imfilter(self, args)?,
            "imgaussfilt" => filters::imgaussfilt(self, args)?,
            "medfilt2" => filters::medfilt2(self, args)?,
            _ => unreachable!("checked by handles"),
        };
        self.check_cancel()?;
        Ok(Some(value))
    }
}

/// Storage class of an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Double,
    Logical,
    Typed(NumericClass),
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Double => "double",
            Self::Logical => "logical",
            Self::Typed(c) => c.name(),
        }
    }
    fn is_float(self) -> bool {
        matches!(self, Self::Double | Self::Typed(NumericClass::Single))
    }
}

/// An image as `c` column-major planes of `h` by `w` doubles.
#[derive(Clone, Debug)]
pub(super) struct Img {
    pub h: usize,
    pub w: usize,
    pub c: usize,
    pub kind: Kind,
    pub data: Vec<f64>,
}

impl Img {
    fn plane_len(&self) -> usize {
        self.h * self.w
    }
    fn plane(&self, k: usize) -> &[f64] {
        let n = self.plane_len();
        &self.data[k * n..(k + 1) * n]
    }
    fn dims(&self) -> Vec<usize> {
        if self.c == 1 {
            vec![self.h, self.w]
        } else {
            vec![self.h, self.w, self.c]
        }
    }
}

/// The class name of any value (for messages).
fn class_of(engine: &Engine, value: &Value) -> String {
    value
        .clone()
        .array(&engine.cancel)
        .map(|a| a.class().to_owned())
        .unwrap_or_else(|_| "unknown".into())
}

/// Reads a 2-D or H-by-W-by-C numeric or logical image.
pub(super) fn image(engine: &Engine, name: &str, value: &Value) -> Result<Img> {
    let dims = value.dims().into_owned();
    if dims.len() > 3 {
        return Err(format!(
            "{name}: expected a 2-D image or an H-by-W-by-C array, not a {}-D array",
            dims.len()
        ));
    }
    let (h, w, c) = (dims[0], dims[1], dims.get(2).copied().unwrap_or(1));
    // The double working copy must fit the memory budget before it is made.
    let max = super::limits::memory_units(&engine.limits);
    if h.saturating_mul(w).saturating_mul(c) > max {
        return Err(super::limits::memory_error(
            &format!("{name}: the {h}-by-{w}-by-{c} image"),
            &engine.limits,
        ));
    }
    let (kind, data) = match value {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => (Kind::Double, m.data().to_vec()),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => (
            Kind::Logical,
            l.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        Value::Container(Array::Typed(t)) => (Kind::Typed(t.class()), t.to_f64_vec_within(max)?),
        Value::Complex(_) | Value::Container(Array::Complex(_)) => {
            return Err(format!("{name}: complex images are not supported"));
        }
        other => {
            return Err(format!(
                "{name}: expected a numeric or logical image, not {}",
                class_of(engine, other)
            ));
        }
    };
    Ok(Img {
        h,
        w,
        c,
        kind,
        data,
    })
}

/// Stores an image back in its class (integers rounded and saturated).
pub(super) fn to_value(engine: &Engine, img: Img) -> Result<Value> {
    let dims = img.dims();
    let (rows, cols) = (img.h, img.w * img.c);
    Ok(match img.kind {
        Kind::Double => {
            Value::Matrix(Matrix::from_column_vec(rows, cols, img.data)?.with_dims(&dims)?)
        }
        Kind::Logical => Value::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                img.data
                    .iter()
                    .map(|&v| u8::from(v != 0. && !v.is_nan()))
                    .collect(),
                &engine.cancel,
            )?
            .with_dims(&dims)?,
        ),
        Kind::Typed(class) => Value::Container(Array::Typed(
            TypedArray::from_reals(class, rows, cols, img.data.into_iter())?.with_dims(&dims)?,
        )),
    })
}

/// Rejects outputs over the dense element limit before they are allocated.
pub(super) fn admit(name: &str, h: usize, w: usize, c: usize) -> Result<()> {
    let n = h
        .checked_mul(w)
        .and_then(|n| n.checked_mul(c))
        .ok_or_else(|| format!("{name}: output size overflows"))?;
    if n > crate::matrix::MAX_ELEMENTS {
        return Err(format!(
            "{name}: the {h}-by-{w}-by-{c} output exceeds the {}-element limit",
            crate::matrix::MAX_ELEMENTS
        ));
    }
    Ok(())
}

/// Text argument (character row or string scalar literal).
pub(super) fn text(engine: &Engine, value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.is_text_row() => c.to_utf8(&engine.cancel).ok(),
        _ => None,
    }
}

/// Real numeric or logical values of an argument (any shape).
pub(super) fn numbers(value: &Value) -> Option<Vec<f64>> {
    match value {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => Some(m.data().to_vec()),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            Some(l.data().iter().map(|&b| f64::from(b)).collect())
        }
        Value::Container(Array::Typed(t)) => t.to_f64_vec().ok(),
        _ => None,
    }
}

/// Cooperative cancellation on the shared flag. Kernels that run on pool
/// workers hold only the flag (the engine is not shared across threads), so
/// they poll through here; the message is the engine's.
pub(super) fn cancelled(cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    tests::cancel_after_progress(cancel);
    crate::dsp::check_cancel(cancel)
}

/// Polls when `i` is a multiple of POLL: for loops that step `i` by one.
pub(super) fn poll(engine: &Engine, i: usize) -> Result<()> {
    if i.is_multiple_of(POLL) {
        cancelled(&engine.cancel)?;
    }
    Ok(())
}

/// Polls once per block of columns of `rows` elements each, so one poll covers
/// at least POLL elements (every column when they are long). For loops over
/// columns, where `poll(engine, column * rows)` would hit a multiple of POLL
/// only for lucky sizes.
pub(super) fn poll_column(engine: &Engine, column: usize, rows: usize) -> Result<()> {
    if column.is_multiple_of((POLL / rows.max(1)).max(1)) {
        cancelled(&engine.cancel)?;
    }
    Ok(())
}

fn arity(name: &str, args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err(format!("{name}: not enough input arguments"))
    } else if args.len() > max {
        Err(format!("{name}: too many input arguments"))
    } else {
        Ok(())
    }
}

// ------------------------------------------------------------ class and range

/// Classes the im2* converters accept (documented input types).
const IM2_CLASSES: &str = "double, single, logical, uint8, uint16 or int16";

fn im2_supported(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Double
            | Kind::Logical
            | Kind::Typed(
                NumericClass::Single
                    | NumericClass::Uint8
                    | NumericClass::Uint16
                    | NumericClass::Int16
            )
    )
}

/// Value of one element on the unit scale of its class: uint8 / 255, uint16 /
/// 65535, int16 (v + 32768) / 65535, floating and logical unchanged.
fn unit(kind: Kind, v: f64) -> f64 {
    match kind {
        Kind::Typed(NumericClass::Uint8) => v / 255.,
        Kind::Typed(NumericClass::Uint16) => v / 65535.,
        Kind::Typed(NumericClass::Int16) => (v + 32768.) / 65535.,
        _ => v,
    }
}

fn convert_class(engine: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    arity(name, args, 1, 2)?;
    let mut img = image(engine, name, &args[0])?;
    let indexed = match args.get(1) {
        None => false,
        Some(v) => match text(engine, v) {
            Some(t) if t.eq_ignore_ascii_case("indexed") => true,
            Some(t) => return Err(format!("{name}: unknown option '{t}'")),
            None => return Err(format!("{name}: the second argument must be 'indexed'")),
        },
    };
    if !im2_supported(img.kind) {
        return Err(format!(
            "{name}: expected an image of class {IM2_CLASSES}, not {}",
            img.kind.name()
        ));
    }
    let from = img.kind;
    let target = match name {
        "im2double" => Kind::Double,
        "im2single" => Kind::Typed(NumericClass::Single),
        "im2uint8" => Kind::Typed(NumericClass::Uint8),
        _ => Kind::Typed(NumericClass::Uint16),
    };
    if indexed {
        return convert_indexed(engine, name, img, target);
    }
    let top = match target {
        Kind::Typed(NumericClass::Uint8) => 255.,
        Kind::Typed(NumericClass::Uint16) => 65535.,
        _ => 1.,
    };
    for (i, v) in img.data.iter_mut().enumerate() {
        poll(engine, i)?;
        *v = match (from, target) {
            (a, b) if a == b => *v,
            // Integer to integer: exact level maps (uint8 -> uint16 is * 257).
            (Kind::Typed(NumericClass::Uint8), Kind::Typed(NumericClass::Uint16)) => *v * 257.,
            (Kind::Typed(NumericClass::Uint16), Kind::Typed(NumericClass::Uint8)) => *v / 257.,
            (Kind::Typed(NumericClass::Int16), Kind::Typed(NumericClass::Uint16)) => *v + 32768.,
            (Kind::Typed(NumericClass::Int16), Kind::Typed(NumericClass::Uint8)) => {
                (*v + 32768.) / 257.
            }
            // Floating results keep values outside [0, 1]; integer results clip.
            (_, _) if top == 1. => unit(from, *v),
            (_, _) => unit(from, *v).clamp(0., 1.) * top,
        };
    }
    img.kind = target;
    to_value(engine, img)
}

/// The 'indexed' forms: integer indices are 0-based, floating ones 1-based.
fn convert_indexed(engine: &Engine, name: &str, mut img: Img, target: Kind) -> Result<Value> {
    let float_in = img.kind.is_float();
    let float_out = target.is_float();
    let top = match target {
        Kind::Typed(NumericClass::Uint8) => 255.,
        Kind::Typed(NumericClass::Uint16) => 65535.,
        _ => f64::INFINITY,
    };
    if matches!(img.kind, Kind::Typed(NumericClass::Int16) | Kind::Logical) && !float_out {
        // Logical and int16 indices are not documented for integer outputs.
        return Err(format!(
            "{name}: indexed images must be double, single, uint8 or uint16, not {}",
            img.kind.name()
        ));
    }
    for (i, v) in img.data.iter_mut().enumerate() {
        poll(engine, i)?;
        let shift = match (float_in, float_out) {
            (true, false) => -1.,
            (false, true) => 1.,
            _ => 0.,
        };
        let x = *v + shift;
        if !float_out && (x.fract() != 0. || !(0. ..=top).contains(&x)) {
            return Err(format!(
                "{name}: indexed values must be integers that fit in {} after conversion",
                target.name()
            ));
        }
        *v = x;
    }
    img.kind = target;
    to_value(engine, img)
}

fn mat2gray(engine: &Engine, args: &[Value]) -> Result<Value> {
    arity("mat2gray", args, 1, 2)?;
    let mut img = image(engine, "mat2gray", &args[0])?;
    let (lo, hi) = match args.get(1) {
        Some(limits) => {
            let v = numbers(limits).ok_or("mat2gray: limits must be numeric [amin amax]")?;
            if v.len() != 2 {
                return Err("mat2gray: limits must be a two-element vector [amin amax]".into());
            }
            (v[0], v[1])
        }
        None => img
            .data
            .iter()
            .filter(|v| !v.is_nan())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                (a.min(v), b.max(v))
            }),
    };
    for (i, v) in img.data.iter_mut().enumerate() {
        poll(engine, i)?;
        // amin == amax is not documented: the values are clipped to [0, 1].
        let scaled = if hi == lo { *v } else { (*v - lo) / (hi - lo) };
        *v = if scaled.is_nan() {
            scaled
        } else {
            scaled.clamp(0., 1.)
        };
    }
    img.kind = Kind::Double;
    to_value(engine, img)
}

fn imcomplement(engine: &Engine, args: &[Value]) -> Result<Value> {
    arity("imcomplement", args, 1, 1)?;
    let mut img = image(engine, "imcomplement", &args[0])?;
    let kind = img.kind;
    for (i, v) in img.data.iter_mut().enumerate() {
        poll(engine, i)?;
        *v = match kind {
            Kind::Double | Kind::Logical | Kind::Typed(NumericClass::Single) => 1. - *v,
            // intmax + intmin - v: max - v for unsigned classes, -1 - v for signed.
            Kind::Typed(c) => (c.max() + c.min()) as f64 - *v,
        };
    }
    to_value(engine, img)
}

// ------------------------------------------------------------------- colour

/// Luminance weights of the rgb2gray reference page (Rec. ITU-R BT.601-7
/// to three decimals).
const GRAY: [f64; 3] = [0.298936021293775, 0.587043074451121, 0.114020904255103];

fn rgb2gray(engine: &Engine, args: &[Value]) -> Result<Value> {
    arity("rgb2gray", args, 1, 1)?;
    let img = image(engine, "rgb2gray", &args[0])?;
    let allowed = matches!(
        img.kind,
        Kind::Double
            | Kind::Typed(NumericClass::Single | NumericClass::Uint8 | NumericClass::Uint16)
    );
    if !allowed {
        return Err(format!(
            "rgb2gray: expected double, single, uint8 or uint16 input, not {}",
            img.kind.name()
        ));
    }
    if img.c == 1 {
        // A c-by-3 colormap gives a c-by-3 gray colormap.
        if img.w != 3 || img.kind != Kind::Double {
            return Err(
                "rgb2gray: input must be an M-by-N-by-3 image or a c-by-3 double colormap".into(),
            );
        }
        if img.data.iter().any(|v| !(0. ..=1.).contains(v)) {
            return Err("rgb2gray: colormap values must be in the range [0, 1]".into());
        }
        let m = img.h;
        let mut data = vec![0.; m * 3];
        for r in 0..m {
            let g =
                GRAY[0] * img.data[r] + GRAY[1] * img.data[m + r] + GRAY[2] * img.data[2 * m + r];
            for k in 0..3 {
                data[k * m + r] = g;
            }
        }
        return to_value(
            engine,
            Img {
                h: m,
                w: 3,
                c: 1,
                kind: Kind::Double,
                data,
            },
        );
    }
    if img.c != 3 {
        return Err(format!(
            "rgb2gray: RGB must be an M-by-N-by-3 array, not M-by-N-by-{}",
            img.c
        ));
    }
    let n = img.plane_len();
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        poll(engine, i)?;
        data.push(
            GRAY[0] * img.data[i] + GRAY[1] * img.data[n + i] + GRAY[2] * img.data[2 * n + i],
        );
    }
    to_value(
        engine,
        Img {
            h: img.h,
            w: img.w,
            c: 1,
            kind: img.kind,
            data,
        },
    )
}

fn ind2rgb(engine: &Engine, args: &[Value]) -> Result<Value> {
    arity("ind2rgb", args, 2, 2)?;
    let x = image(engine, "ind2rgb", &args[0])?;
    if x.c != 1 {
        return Err("ind2rgb: X must be a 2-D indexed image".into());
    }
    let integer = match x.kind {
        Kind::Double | Kind::Typed(NumericClass::Single) => false,
        Kind::Typed(NumericClass::Uint8 | NumericClass::Uint16) => true,
        other => {
            return Err(format!(
                "ind2rgb: X must be single, double, uint8 or uint16, not {}",
                other.name()
            ));
        }
    };
    let map = image(engine, "ind2rgb", &args[1])?;
    if map.c != 1 || map.w != 3 || map.h == 0 {
        return Err("ind2rgb: the colormap must be a c-by-3 array".into());
    }
    let colors = map.h;
    let n = x.plane_len();
    admit("ind2rgb", x.h, x.w, 3)?;
    let mut data = vec![0.; 3 * n];
    for i in 0..n {
        poll(engine, i)?;
        let v = x.data[i];
        if v.is_nan() || v.fract() != 0. {
            return Err("ind2rgb: index values must be integers".into());
        }
        // Integer X: 0 is the first colour, clipped to [0, c-1]; floating X:
        // 1 is the first colour, clipped to [1, c].
        let row = crate::plot::direct_colormap_index(v, integer, colors);
        for k in 0..3 {
            data[k * n + i] = map.data[k * colors + row];
        }
    }
    to_value(
        engine,
        Img {
            h: x.h,
            w: x.w,
            c: 3,
            kind: Kind::Double,
            data,
        },
    )
}

/// Smith's hexcone (SIGGRAPH 1978), HSV to RGB: six hue sectors `k` with
/// fraction `f`; `p = v (1 - s)`, `q = v (1 - s f)`, `t = v (1 - s (1 - f))`.
fn hexcone_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let h6 = 6. * h;
    let sector = h6.floor();
    let f = h6 - sector;
    let p = v * (1. - s);
    let q = v * (1. - s * f);
    let t = v * (1. - s * (1. - f));
    match (sector as i64).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Smith's hexcone, RGB to HSV: `v` the largest component, `s` the spread
/// over `v`, hue from the two others in the sector of the largest one.
fn rgb_to_hexcone(r: f64, g: f64, b: f64) -> [f64; 3] {
    let v = r.max(g).max(b);
    let spread = v - r.min(g).min(b);
    let s = if v == 0. { 0. } else { spread / v };
    if spread == 0. {
        return [0., s, v];
    }
    let sector = if r == v {
        (g - b) / spread
    } else if g == v {
        2. + (b - r) / spread
    } else {
        4. + (r - g) / spread
    };
    let h = sector / 6.;
    [if h < 0. { h + 1. } else { h }, s, v]
}

/// rgb2hsv / hsv2rgb of an H-by-W-by-3 image. RGB may be double, single,
/// uint8 or uint16 (integers scaled to [0, 1]); HSV is double or single.
/// Single input gives single output, everything else double.
fn hsv_image(engine: &Engine, name: &str, value: &Value) -> Result<Value> {
    let img = image(engine, name, value)?;
    let colormap = value.ndims() == 2 && img.w == 3;
    if img.c != 3 && !colormap {
        return Err(format!(
            "{name}: the image must be M-by-N-by-3, not M-by-N-by-{}",
            img.c
        ));
    }
    let forward = name == "hsv2rgb";
    let allowed = match img.kind {
        Kind::Double | Kind::Typed(NumericClass::Single) => true,
        Kind::Typed(NumericClass::Uint8 | NumericClass::Uint16) => !forward,
        _ => false,
    };
    if !allowed {
        return Err(format!(
            "{name}: expected {} input, not {}",
            if forward {
                "double or single"
            } else {
                "double, single, uint8 or uint16"
            },
            img.kind.name()
        ));
    }
    let n = if colormap { img.h } else { img.plane_len() };
    let mut data = vec![0.; 3 * n];
    for i in 0..n {
        poll(engine, i)?;
        let [a, b, c] = [0, 1, 2].map(|k| unit(img.kind, img.data[k * n + i]));
        let out = if forward {
            hexcone_to_rgb(a, b, c)
        } else {
            rgb_to_hexcone(a, b, c)
        };
        for k in 0..3 {
            data[k * n + i] = out[k];
        }
    }
    let kind = if img.kind == Kind::Typed(NumericClass::Single) {
        img.kind
    } else {
        Kind::Double
    };
    to_value(
        engine,
        Img {
            h: img.h,
            w: img.w,
            c: if colormap { 1 } else { 3 },
            kind,
            data,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::Engine;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// (address of a cancel flag, polls left before it is set). Keyed by flag,
    /// not thread-local, so polls made by pool workers count too, and tests
    /// that run side by side cannot trip each other's engines.
    static CANCEL_AFTER: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

    /// Removes its flag's entry when dropped, so a later engine that reuses
    /// the address is never cancelled by a stale one.
    pub(super) struct CancelAt(usize);

    impl Drop for CancelAt {
        fn drop(&mut self) {
            let mut list = CANCEL_AFTER.lock().unwrap_or_else(|e| e.into_inner());
            list.retain(|e| e.0 != self.0);
        }
    }

    /// Makes `cancel` read true from its `polls`-th poll on (0 = the first)
    /// until the returned guard is dropped.
    pub(super) fn cancel_at(cancel: &AtomicBool, polls: usize) -> CancelAt {
        let key = std::ptr::from_ref(cancel) as usize;
        let mut list = CANCEL_AFTER.lock().unwrap_or_else(|e| e.into_inner());
        list.retain(|e| e.0 != key);
        list.push((key, polls));
        CancelAt(key)
    }

    #[test]
    fn poll_column_polls_once_per_block_of_columns() {
        // 4097 rows: x * 4097 is a multiple of 65536 only for x a multiple of
        // 65536, so polling `poll(engine, x * rows)` would never fire here.
        // A poll covers at least 65536 elements, i.e. every 15th column.
        let engine = Engine::default();
        let _guard = cancel_at(&engine.cancel, 1);
        let first_stop = (0..100).find(|&x| super::poll_column(&engine, x, 4097).is_err());
        assert_eq!(first_stop, Some(15));
        // Long columns poll every time, short ones every POLL / rows columns.
        let engine = Engine::default();
        let _guard = cancel_at(&engine.cancel, 2);
        let first_stop = (0..100).find(|&x| super::poll_column(&engine, x, 1 << 20).is_err());
        assert_eq!(first_stop, Some(2));
    }

    pub(super) fn cancel_after_progress(cancel: &AtomicBool) {
        let mut list = CANCEL_AFTER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = list
            .iter_mut()
            .find(|e| e.0 == std::ptr::from_ref(cancel) as usize)
        {
            if entry.1 == 0 {
                cancel.store(true, Ordering::Relaxed);
            } else {
                entry.1 -= 1;
            }
        }
    }
}
