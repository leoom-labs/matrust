//! Image processing part 2: histograms and contrast, global thresholds,
//! flat structuring elements, binary and grey morphology, connected
//! components and region measurements.
//!
//! Clean room (docs/CLEAN_ROOM.md): behaviour from the public documentation
//! pages of these functions (outputs, defaults, classes) and published
//! methods: Otsu, "A threshold selection method from gray-level histograms",
//! IEEE SMC 9(1), 1979; Rosenfeld & Pfaltz, "Sequential operations in digital
//! picture processing", JACM 13(4), 1966 (connected components); Soille,
//! "Morphological Image Analysis", 2nd ed., Springer 2003 (flat morphology,
//! reconstruction, hole filling); Lam, Lee & Suen, "Thinning methodologies -
//! a comprehensive survey", IEEE PAMI 14(9), 1992, p. 879 (thinning); Adams,
//! "Radial decomposition of discs and spheres", CVGIP 55(5), 1993 and Jones &
//! Soille, "Periodic lines", Pattern Recognition Letters 17, 1996 (disk
//! approximation by periodic lines); Haralick & Shapiro, "Computer and Robot
//! Vision" vol. 1, 1992 (normalised second central moments).
//!
//! Every helper is a free function; the one `impl Engine` method carries the
//! `image_morph_` prefix so parallel tracks cannot collide with it.
use super::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, NumericClass, Result, TypedArray};

mod enhance;
mod hist;
mod morph;
mod regions;
mod strel;

const NAMES: &[&str] = &[
    "imhist",
    "stretchlim",
    "imadjust",
    "graythresh",
    "imbinarize",
    "otsuthresh",
    "strel",
    "imdilate",
    "imerode",
    "imopen",
    "imclose",
    "imtophat",
    "imbothat",
    "bwmorph",
    "imfill",
    "bwareaopen",
    "imclearborder",
    "bwlabel",
    "bwconncomp",
    "labelmatrix",
    "regionprops",
    "histeq",
    "edge",
];

/// Elements between cancellation polls.
const POLL: usize = 1 << 14;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
/// Functions with a statement form (no outputs) that differs from `ans = f(...)`.
pub(super) fn statement(name: &str) -> bool {
    name == "imhist"
}

fn max_outputs(name: &str) -> usize {
    match name {
        "imhist" | "graythresh" | "bwlabel" | "histeq" | "edge" => 2,
        _ => 1,
    }
}

impl Engine {
    /// Dispatch for every function of this module; `count` is the number of
    /// requested outputs (0 for a statement, 1 for an expression).
    pub(super) fn image_morph_call(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = max_outputs(name);
        if count > max {
            return Err(format!("{name}: too many output arguments (at most {max})"));
        }
        if args.is_empty() {
            return Err(format!("{name} requires at least one input argument"));
        }
        if name == "imbinarize"
            && args
                .get(1)
                .and_then(|v| super::image_basic::text(self, v))
                .is_some_and(|s| s.eq_ignore_ascii_case("adaptive"))
        {
            let mut adaptive_args = vec![args[0].clone()];
            adaptive_args.extend_from_slice(&args[2..]);
            return Ok(vec![super::image_color::adaptive(
                self,
                &adaptive_args,
                true,
            )?]);
        }
        let cx = Cx {
            name,
            cancel: &self.cancel,
            max_elements: super::limits::memory_units(&self.limits),
        };
        let values = match name {
            "imhist" => {
                let (counts, centres, low, high) = hist::imhist(&cx, args)?;
                if count == 0 {
                    hist::draw(self, counts, centres, low, high, out)?;
                    return Ok(vec![Value::Void]);
                }
                vec![counts, centres]
            }
            "stretchlim" => vec![hist::stretchlim(&cx, args)?],
            "imadjust" => vec![hist::imadjust(&cx, args)?],
            "graythresh" => hist::graythresh(&cx, args)?,
            "otsuthresh" => vec![hist::otsuthresh(&cx, args)?],
            "imbinarize" => vec![hist::imbinarize(&cx, args)?],
            "strel" => vec![strel::strel(&cx, args)?],
            "imdilate" | "imerode" | "imopen" | "imclose" | "imtophat" | "imbothat" => {
                vec![morph::morphology(&cx, args)?]
            }
            "bwmorph" => vec![morph::bwmorph(&cx, args)?],
            "imfill" => vec![morph::imfill(&cx, args)?],
            "bwareaopen" => vec![morph::bwareaopen(&cx, args)?],
            "imclearborder" => vec![morph::imclearborder(&cx, args)?],
            "bwlabel" => regions::bwlabel(&cx, args)?,
            "bwconncomp" => vec![regions::bwconncomp(&cx, args)?],
            "labelmatrix" => vec![regions::labelmatrix(&cx, args)?],
            "regionprops" => vec![regions::regionprops(&cx, args)?],
            "histeq" => enhance::histeq(&cx, args)?,
            "edge" => enhance::edge(&cx, args)?,
            _ => return Err(format!("{name} is not an image function")),
        };
        let mut values = values;
        values.truncate(count.max(1));
        Ok(values)
    }
}

/// Call context: the function name for messages and the cancellation flag.
struct Cx<'a> {
    name: &'a str,
    cancel: &'a std::sync::atomic::AtomicBool,
    /// Elements one call may create: the engine's memory budget in 8-byte units.
    max_elements: usize,
}
impl Cx<'_> {
    fn err<T>(&self, message: impl std::fmt::Display) -> Result<T> {
        Err(format!("{}: {message}", self.name))
    }
    fn poll(&self, i: usize) -> Result<()> {
        if i.is_multiple_of(POLL) {
            crate::dsp::check_cancel(self.cancel)?;
        }
        Ok(())
    }
    fn check(&self) -> Result<()> {
        crate::dsp::check_cancel(self.cancel)
    }
}

/// Class of an image: decides ranges, defaults and the output class.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Double,
    Single,
    Logical,
    Int(NumericClass),
}
impl Kind {
    /// (lowest value, width of the class range): [0, 1] for floating point
    /// and logical images, [intmin, intmax] for integer images.
    fn range(self) -> (f64, f64) {
        match self {
            Self::Double | Self::Single | Self::Logical => (0., 1.),
            Self::Int(c) => (c.min() as f64, (c.max() - c.min()) as f64),
        }
    }
}

/// An image as doubles in column-major order with its MATLAB dimensions.
#[derive(Clone, Debug)]
struct Img {
    dims: Vec<usize>,
    data: Vec<f64>,
    kind: Kind,
}
impl Img {
    fn rows(&self) -> usize {
        self.dims[0]
    }
    fn cols(&self) -> usize {
        self.dims.get(1).copied().unwrap_or(1)
    }
    fn planes(&self) -> usize {
        self.dims[2..].iter().product()
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn with(&self, data: Vec<f64>, kind: Kind) -> Self {
        Self {
            dims: self.dims.clone(),
            data,
            kind,
        }
    }
}

fn image(cx: &Cx, value: &Value) -> Result<Img> {
    let (dims, data, kind) = match value {
        Value::Matrix(m) => (m.dims().to_vec(), m.data().to_vec(), Kind::Double),
        Value::Logical(l) => (
            l.dims().to_vec(),
            l.data().iter().map(|&b| f64::from(b)).collect(),
            Kind::Logical,
        ),
        Value::Container(Array::Typed(t)) => {
            let kind = match t.class() {
                NumericClass::Single => Kind::Single,
                c => Kind::Int(c),
            };
            (
                t.dims().to_vec(),
                t.to_f64_vec_within(cx.max_elements)?,
                kind,
            )
        }
        Value::Container(Array::Sparse(_)) => return cx.err("sparse images are not supported"),
        Value::Complex(_) => return cx.err("complex images are not supported"),
        _ => return cx.err("expected a numeric or logical image"),
    };
    Ok(Img { dims, data, kind })
}

/// A 2-D image (the error every 2-D-only function gives for N-D input).
fn image_2d(cx: &Cx, value: &Value) -> Result<Img> {
    let img = image(cx, value)?;
    if img.dims.len() > 2 {
        return cx.err("the image must be 2-D (got an N-D array)");
    }
    Ok(img)
}

/// A 2-D image as a foreground mask (nonzero = foreground, NaN counts as foreground).
fn mask_2d(cx: &Cx, value: &Value) -> Result<(usize, usize, Vec<bool>)> {
    let img = image_2d(cx, value)?;
    let mask = img.data.iter().map(|&v| v != 0.).collect();
    Ok((img.rows(), img.cols(), mask))
}

fn value(cx: &Cx, img: Img) -> Result<Value> {
    let rows = img.dims[0];
    let cols = img.data.len().checked_div(rows).unwrap_or(0);
    let cols = if rows == 0 {
        img.dims[1..].iter().product()
    } else {
        cols
    };
    let nd = img.dims.len() > 2;
    let v = match img.kind {
        Kind::Double => {
            let m = Matrix::from_column_vec(rows, cols, img.data)?;
            Value::Matrix(if nd { m.with_dims(&img.dims)? } else { m })
        }
        Kind::Logical => {
            let bytes = img.data.iter().map(|&v| u8::from(v != 0.)).collect();
            let m = LogicalMatrix::from_column_vec(rows, cols, bytes, cx.cancel)?;
            Value::Logical(if nd { m.with_dims(&img.dims)? } else { m })
        }
        Kind::Single => {
            let t = TypedArray::from_reals(NumericClass::Single, rows, cols, img.data.into_iter())?;
            Value::Container(Array::Typed(if nd { t.with_dims(&img.dims)? } else { t }))
        }
        Kind::Int(c) => {
            let t = TypedArray::from_reals(c, rows, cols, img.data.into_iter())?;
            Value::Container(Array::Typed(if nd { t.with_dims(&img.dims)? } else { t }))
        }
    };
    Ok(v)
}

fn mask_value(cx: &Cx, rows: usize, cols: usize, mask: &[bool]) -> Result<Value> {
    let bytes = mask.iter().map(|&b| u8::from(b)).collect();
    Ok(Value::Logical(LogicalMatrix::from_column_vec(
        rows, cols, bytes, cx.cancel,
    )?))
}

fn double(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}

/// Real numeric values of an argument (logical counts as 0/1).
fn numbers(cx: &Cx, value: &Value, what: &str) -> Result<Vec<f64>> {
    match value {
        Value::Matrix(m) => Ok(m.data().to_vec()),
        Value::Logical(l) => Ok(l.data().iter().map(|&b| f64::from(b)).collect()),
        Value::Container(Array::Typed(t)) => t.to_f64_vec_within(cx.max_elements),
        _ => cx.err(format!("{what} must be numeric")),
    }
}

fn scalar(cx: &Cx, value: &Value, what: &str) -> Result<f64> {
    match numbers(cx, value, what)?.as_slice() {
        [v] => Ok(*v),
        _ => cx.err(format!("{what} must be a scalar")),
    }
}

/// Connectivity argument of a 2-D function: 4 or 8.
fn connectivity(cx: &Cx, value: &Value) -> Result<u8> {
    match numbers(cx, value, "connectivity")?.as_slice() {
        [v] if *v == 4. => Ok(4),
        [v] if *v == 8. => Ok(8),
        _ => cx.err("connectivity must be 4 or 8"),
    }
}

/// Output-size bound shared by every function here.
fn check_elements(cx: &Cx, n: usize) -> Result<()> {
    if n > cx.max_elements {
        return cx.err(format!(
            "result of {n} elements exceeds the {}-element limit",
            cx.max_elements
        ));
    }
    Ok(())
}
