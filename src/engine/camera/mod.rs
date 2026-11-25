//! Camera calibration, piece 7d1: the value objects cameraIntrinsics and
//! cameraParameters, generateCheckerboardPoints, worldToImage and the
//! rotation-vector conversions (rotationVectorToMatrix, rotationMatrixToVector,
//! rotvec2mat3d, rotmat2vec3d). Behaviour follows the public MathWorks
//! reference pages; conventions are in projection.rs.
//!
//! Both classes are value classes kept in the object table (objects.rs,
//! `ObjState::Camera`); every property is read-only, as on the reference
//! pages. Later 7d pieces read objects with [`camera_of`] and make them with
//! [`new_object`].
pub(in crate::engine) mod detect;
mod estimate;
mod objects;
mod plots;
mod projection;
mod undistort;

#[allow(unused_imports)]
// API for detectCheckerboardPoints / estimateCameraParameters (7d2-7d4).
pub(in crate::engine) use objects::{
    CalibrationErrors, ExtrinsicsErrors, Intrinsics, IntrinsicsErrors, Params,
};

use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, NumericClass, Result, StructArray,
    TypedArray,
    syntax::{Expr, PathStep},
};
use std::sync::Arc;

/// One camera object.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) enum Camera {
    Intrinsics(Intrinsics),
    Params(Params),
    /// The third output of estimateCameraParameters and its two parts.
    CalibrationErrors(CalibrationErrors),
    IntrinsicsErrors(IntrinsicsErrors),
    ExtrinsicsErrors(ExtrinsicsErrors),
}

/// Classes, in the order of their display masks (objects.rs obj_mask).
pub(in crate::engine) const CLASSES: [&str; 5] = [
    "cameraIntrinsics",
    "cameraParameters",
    "cameraCalibrationErrors",
    "intrinsicsEstimationErrors",
    "extrinsicsEstimationErrors",
];

/// Methods of all classes.
const METHODS: [&str; 2] = ["worldToImage", "displayErrors"];
const CAMERA_METHODS: [&str; 1] = ["worldToImage"];
const ERRORS_METHODS: [&str; 1] = ["displayErrors"];

/// Functions this module answers whatever the arguments are.
const OWNED: [&str; 10] = [
    "cameraIntrinsics",
    "estimateCameraParameters",
    "undistortImage",
    "undistortPoints",
    "cameraParameters",
    "generateCheckerboardPoints",
    "rotationVectorToMatrix",
    "rotationMatrixToVector",
    "rotvec2mat3d",
    "rotmat2vec3d",
];

impl Camera {
    pub(in crate::engine) fn class(&self) -> &'static str {
        CLASSES[self.class_index()]
    }
    pub(in crate::engine) fn class_index(&self) -> usize {
        match self {
            Camera::Intrinsics(_) => 0,
            Camera::Params(_) => 1,
            Camera::CalibrationErrors(_) => 2,
            Camera::IntrinsicsErrors(_) => 3,
            Camera::ExtrinsicsErrors(_) => 4,
        }
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        match self {
            Camera::Intrinsics(_) | Camera::IntrinsicsErrors(_) => std::mem::size_of::<Self>() + 64,
            Camera::Params(p) => p.bytes(),
            Camera::CalibrationErrors(c) => std::mem::size_of::<Self>() + c.extr.bytes(),
            Camera::ExtrinsicsErrors(x) => std::mem::size_of::<Self>() + x.bytes(),
        }
    }
    pub(in crate::engine) fn property_names(&self) -> &'static [&'static str] {
        match self {
            Camera::Intrinsics(_) => &Intrinsics::PROPERTIES,
            Camera::Params(_) => &Params::PROPERTIES,
            Camera::CalibrationErrors(_) => &CalibrationErrors::PROPERTIES,
            Camera::IntrinsicsErrors(_) => &IntrinsicsErrors::PROPERTIES,
            Camera::ExtrinsicsErrors(_) => &ExtrinsicsErrors::PROPERTIES,
        }
    }
    fn methods(&self) -> &'static [&'static str] {
        match self {
            Camera::Intrinsics(_) | Camera::Params(_) => &CAMERA_METHODS,
            Camera::CalibrationErrors(_) => &ERRORS_METHODS,
            _ => &[],
        }
    }
    fn intrinsics(&self) -> Option<&Intrinsics> {
        match self {
            Camera::Intrinsics(i) => Some(i),
            Camera::Params(p) => Some(&p.intr),
            _ => None,
        }
    }
    /// Properties that hold objects (made by [`member`]): name and class.
    fn nested(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            Camera::Params(p) if p.intr.size.is_some() => &[("Intrinsics", "cameraIntrinsics")],
            Camera::CalibrationErrors(_) => &[
                ("IntrinsicsErrors", "intrinsicsEstimationErrors"),
                ("ExtrinsicsErrors", "extrinsicsEstimationErrors"),
            ],
            _ => &[],
        }
    }
}

/// Builtins of this module that take H-by-W-by-C images (nd.rs guard).
pub(in crate::engine) fn takes_images(name: &str) -> bool {
    undistort::NAMES[..1].contains(&name)
}

/// Bare `cameraParameters` is the zero-argument constructor call.
pub(in crate::engine) fn bare(name: &str) -> bool {
    name == "cameraParameters"
}

/// Functions with an N-D argument (imagePoints is M-by-2-by-P): the N-D
/// guard lets their handle and feval calls through.
pub(in crate::engine) fn takes_nd(name: &str) -> bool {
    name == "estimateCameraParameters"
}

/// Names whose several-output calls this module answers.
pub(in crate::engine) fn multi_output(name: &str) -> bool {
    name == "worldToImage" || OWNED.contains(&name)
}

/// Methods of both classes (`[a, b] = object.method(...)` in engine.rs).
pub(in crate::engine) fn is_method(name: &str) -> bool {
    METHODS.contains(&name)
}

/// The camera object an argument holds, if any.
pub(in crate::engine) fn camera_of(e: &Engine, v: &Value) -> Option<Arc<Camera>> {
    if !e.obj_live() {
        return None;
    }
    super::objects::camera_value(e, v)
}

/// A new object value (a fresh object-table entry).
pub(in crate::engine) fn new_object(e: &mut Engine, c: Camera) -> Result<Value> {
    super::objects::camera_new(e, c)
}

pub(super) fn err(who: &str, msg: &str) -> String {
    format!("{who}: {msg}")
}

pub(super) fn empty() -> Value {
    Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new()).expect("empty matrix"))
}

pub(super) fn logical(b: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(b))
}

/// A real numeric argument: dimensions, values, single or double.
pub(super) struct Num {
    pub(super) dims: Vec<usize>,
    pub(super) data: Vec<f64>,
    pub(super) single: bool,
}

pub(super) fn num(e: &Engine, who: &str, what: &str, v: &Value) -> Result<Num> {
    let dims = v.dims().into_owned();
    let (data, single) = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => (m.data().to_vec(), false),
        Value::Container(Array::Typed(t)) => (t.to_f64_vec()?, t.class() == NumericClass::Single),
        Value::Logical(_) | Value::Container(Array::Logical(_)) => {
            let a = v.clone().array(&e.cancel)?;
            let Array::Logical(l) = a else { unreachable!() };
            (l.data().iter().map(|&b| f64::from(b)).collect(), false)
        }
        Value::Complex(_) | Value::Container(Array::Complex(_)) => {
            return Err(err(who, &format!("{what} must be real")));
        }
        _ => return Err(err(who, &format!("{what} must be numeric"))),
    };
    Ok(Num { dims, data, single })
}

pub(super) fn logical_arg(e: &Engine, who: &str, what: &str, v: &Value) -> Result<bool> {
    let n = num(e, who, what, v)?;
    match n.data.as_slice() {
        [x] if *x == 0. || *x == 1. => Ok(*x == 1.),
        _ => Err(err(
            who,
            &format!("{what} must be a logical scalar (true or false)"),
        )),
    }
}

pub(super) fn text_arg(e: &Engine, v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_ref().map(|s| s.to_string())
        }
        Value::Container(Array::Character(_) | Array::Str(_)) => e.text_value(v.clone()).ok(),
        _ => None,
    }
}

fn class_of(e: &Engine, v: Option<&Value>) -> String {
    match v {
        None => "double".into(),
        Some(v) => match camera_of(e, v) {
            Some(c) => c.class().into(),
            None => match v {
                Value::Matrix(_) | Value::Complex(_) | Value::Void => "double".into(),
                Value::Logical(_) => "logical".into(),
                Value::Text(_) => "char".into(),
                Value::QuotedString(_) => "string".into(),
                Value::Container(a) => a.class().into(),
            },
        },
    }
}

fn undefined(e: &mut Engine, name: &str, class: &str) -> String {
    e.raise(
        format!("Undefined function '{name}' for input arguments of type '{class}'."),
        "MATLAB:UndefinedFunction".into(),
    )
}

fn real_value(rows: usize, cols: usize, data: Vec<f64>, single: bool) -> Result<Value> {
    Ok(if single {
        Value::Container(Array::Typed(TypedArray::from_reals(
            NumericClass::Single,
            rows,
            cols,
            data.into_iter(),
        )?))
    } else {
        Value::Matrix(Matrix::from_column_vec(rows, cols, data)?)
    })
}

fn max_outputs(nargout: usize, max: usize) -> Result<()> {
    if nargout > max {
        Err("Too many output arguments.".into())
    } else {
        Ok(())
    }
}

/// Builtin calls: the functions above and every call whose first argument
/// is a camera object. None leaves the call to the other builtins.
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    let cam = args.first().and_then(|v| camera_of(e, v));
    if plots::NAMES.contains(&name) {
        e.check_cancel()?;
        let rest = args.get(1..).unwrap_or_default();
        return plots::show(e, cam.as_deref(), name, rest, nargout, out).map(Some);
    }
    if !OWNED.contains(&name) && cam.is_none() {
        if name == "worldToImage" {
            let class = class_of(e, args.first());
            return Err(undefined(e, name, &class));
        }
        return Ok(None);
    }
    e.check_cancel()?;
    if name == "estimateCameraParameters" {
        return estimate::estimate(e, args, nargout, out).map(Some);
    }
    let value = match name {
        "cameraIntrinsics" => {
            max_outputs(nargout, 1)?;
            let c = Camera::Intrinsics(objects::construct_intrinsics(e, args)?);
            new_object(e, c)?
        }
        "cameraParameters" => {
            max_outputs(nargout, 1)?;
            let c = Camera::Params(objects::construct_params(e, args)?);
            new_object(e, c)?
        }
        "generateCheckerboardPoints" => {
            max_outputs(nargout, 1)?;
            checkerboard(e, args)?
        }
        "rotationVectorToMatrix" | "rotvec2mat3d" => {
            max_outputs(nargout, 1)?;
            rotation_matrix(e, name, args)?
        }
        "rotationMatrixToVector" | "rotmat2vec3d" => {
            max_outputs(nargout, 1)?;
            rotation_vector(e, name, args)?
        }
        "undistortImage" => return undistort::undistort_image(e, args, nargout.max(1)).map(Some),
        "undistortPoints" => return undistort::undistort_points(e, args, nargout.max(1)).map(Some),
        _ => {
            let c = cam.expect("checked above");
            return method(e, &c, name, &args[1..], nargout, out);
        }
    };
    Ok(Some(vec![value]))
}

fn checkerboard(e: &Engine, args: &[Value]) -> Result<Value> {
    let who = "generateCheckerboardPoints";
    let [size, square] = args else {
        return Err(err(
            who,
            "expected generateCheckerboardPoints(boardSize, squareSize)",
        ));
    };
    let size = num(e, who, "boardSize", size)?;
    let ok = size.data.len() == 2
        && size
            .data
            .iter()
            .all(|x| x.is_finite() && x.fract() == 0. && *x >= 2.);
    if !ok {
        return Err(err(
            who,
            "boardSize must be a 2-element vector of integers of at least 2 ([rows cols] squares)",
        ));
    }
    let (rows, cols) = (size.data[0], size.data[1]);
    if (rows - 1.) * (cols - 1.) > objects::MAX_POINTS as f64 {
        return Err(err(who, "the board has too many corners"));
    }
    let square = num(e, who, "squareSize", square)?;
    let s = match square.data.as_slice() {
        [s] if s.is_finite() && *s > 0. => *s,
        _ => return Err(err(who, "squareSize must be a positive finite scalar")),
    };
    let p = projection::checkerboard(rows as usize, cols as usize, s);
    let m = p.len();
    let data = p
        .iter()
        .map(|q| q[0])
        .chain(p.iter().map(|q| q[1]))
        .collect();
    real_value(m, 2, data, false)
}

fn rotation_matrix(e: &Engine, who: &str, args: &[Value]) -> Result<Value> {
    let [v] = args else {
        return Err(err(who, &format!("expected {who}(rotationVector)")));
    };
    let n = num(e, who, "rotationVector", v)?;
    let vector = n.dims.len() == 2 && (n.dims[0] == 1 || n.dims[1] == 1);
    if !vector || n.data.len() != 3 || n.data.iter().any(|x| !x.is_finite()) {
        return Err(err(who, "rotationVector must be a finite 3-element vector"));
    }
    let r = projection::rodrigues([n.data[0], n.data[1], n.data[2]]);
    let r = if who == "rotationVectorToMatrix" {
        projection::transpose(&r)
    } else {
        r
    };
    let data = (0..3).flat_map(|j| (0..3).map(move |i| r[i][j])).collect();
    real_value(3, 3, data, n.single)
}

fn rotation_vector(e: &Engine, who: &str, args: &[Value]) -> Result<Value> {
    let [m] = args else {
        return Err(err(who, &format!("expected {who}(rotationMatrix)")));
    };
    let n = num(e, who, "rotationMatrix", m)?;
    let r = objects::mat3(who, "rotationMatrix", &n)?;
    if n.data.iter().any(|x| !x.is_finite()) {
        return Err(err(who, "rotationMatrix must be finite"));
    }
    let r = if who == "rotationMatrixToVector" {
        projection::transpose(&r)
    } else {
        r
    };
    let v = projection::rotation_vector(&r)
        .ok_or_else(|| {
            err(
                who,
                "rotationMatrix must be a rotation matrix (it is singular, a reflection or too far from a rotation)",
            )
        })?;
    real_value(1, 3, v.to_vec(), n.single)
}

/// worldToImage(intrinsics, R, t, worldPoints, 'ApplyDistortion', tf).
fn world_to_image(e: &Engine, c: &Camera, rest: &[Value], nargout: usize) -> Result<Vec<Value>> {
    let who = "worldToImage";
    max_outputs(nargout, 2)?;
    let positional = rest
        .iter()
        .position(|a| text_arg(e, a).is_some())
        .unwrap_or(rest.len());
    let [r, t, w] = &rest[..positional] else {
        if positional == 2 {
            return Err(err(
                who,
                "the rigidtform3d form worldToImage(intrinsics, tform, worldPoints) is not supported yet; pass R and t",
            ));
        }
        return Err(err(
            who,
            "expected worldToImage(intrinsics, rotationMatrix, translationVector, worldPoints)",
        ));
    };
    let mut apply = false;
    for (_, v) in objects::pairs(e, who, &rest[positional..], &["ApplyDistortion"])? {
        apply = logical_arg(e, who, "ApplyDistortion", v)?;
    }
    let rn = num(e, who, "rotationMatrix", r)?;
    let rot = objects::mat3(who, "rotationMatrix", &rn)?;
    let tn = num(e, who, "translationVector", t)?;
    if tn.data.len() != 3 || !(tn.dims[0] == 1 || tn.dims[1] == 1) || tn.dims.len() != 2 {
        return Err(err(who, "translationVector must be a 3-element vector"));
    }
    let tv = [tn.data[0], tn.data[1], tn.data[2]];
    let wn = num(e, who, "worldPoints", w)?;
    if wn.dims.len() != 2 || !(wn.dims[1] == 2 || wn.dims[1] == 3) {
        return Err(err(
            who,
            "worldPoints must be an M-by-3 (or planar M-by-2) matrix",
        ));
    }
    let (m, cols) = (wn.dims[0], wn.dims[1]);
    if m > objects::MAX_POINTS {
        return Err(err(who, "too many world points"));
    }
    let Some(intr) = c.intrinsics() else {
        return Err(format!(
            "Undefined function '{who}' for input arguments of type '{}'.",
            c.class()
        ));
    };
    let mut data = vec![0.; 2 * m];
    let mut valid = Vec::with_capacity(m);
    for i in 0..m {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        let p = [0, 1, 2].map(|j| if j < cols { wn.data[i + m * j] } else { 0. });
        let (uv, z) = projection::project(intr, &rot, tv, p, apply);
        data[i] = uv[0];
        data[m + i] = uv[1];
        let inside = match intr.size {
            Some([rows, cols]) => {
                (0.5..=cols + 0.5).contains(&uv[0]) && (0.5..=rows + 0.5).contains(&uv[1])
            }
            None => uv[0].is_finite() && uv[1].is_finite(),
        };
        valid.push(z > 0. && inside);
    }
    let single = wn.single || rn.single || tn.single;
    let mut outputs = vec![real_value(m, 2, data, single)?];
    if nargout > 1 {
        outputs.push(Value::Logical(LogicalMatrix::from_column_vec(
            m,
            1,
            valid.into_iter().map(u8::from).collect(),
            &e.cancel,
        )?));
    }
    Ok(outputs)
}

/// `name(object, ...)` reached through objects.rs (statements: nargout 0,
/// and `object.method(...)`); None for the generic object queries.
pub(in crate::engine) fn object_method(
    e: &mut Engine,
    c: &Camera,
    name: &str,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    Ok(method(e, c, name, rest, 0, out)?.map(|mut v| {
        if v.is_empty() {
            Value::Void
        } else {
            v.swap_remove(0)
        }
    }))
}

/// `object.name` and `object.name(args)`: methods first, then properties.
pub(in crate::engine) fn member(
    e: &mut Engine,
    c: &Camera,
    name: &str,
    args: Option<&[Expr]>,
    out: &mut Execution,
) -> Result<Value> {
    if c.methods().contains(&name) {
        let rest = match args {
            Some(args) => e.function_arguments(args, None, out)?,
            None => Vec::new(),
        };
        let value = method(e, c, name, &rest, 1, out)?;
        return Ok(value
            .and_then(|mut v| (!v.is_empty()).then(|| v.swap_remove(0)))
            .unwrap_or(Value::Void));
    }
    // Intrinsics of a cameraParameters without ImageSize is [] (property).
    let value = if name == "Intrinsics"
        && let Camera::Params(p) = c
        && let Some(intrinsics) = objects::intrinsics_of(p)
    {
        new_object(e, Camera::Intrinsics(intrinsics))?
    } else if let Camera::CalibrationErrors(x) = c
        && (name == "IntrinsicsErrors" || name == "ExtrinsicsErrors")
    {
        let part = if name == "IntrinsicsErrors" {
            Camera::IntrinsicsErrors(x.intr.clone())
        } else {
            Camera::ExtrinsicsErrors(x.extr.clone())
        };
        new_object(e, part)?
    } else {
        property(c, name)?
    };
    match args {
        Some(args) if !args.is_empty() => e.index_value(value, args, out),
        _ => Ok(value),
    }
}

/// `object.name` (a property). The Intrinsics object of a cameraParameters
/// with an ImageSize is made by [`member`], which can create objects; without
/// an ImageSize the property is [].
pub(in crate::engine) fn property(c: &Camera, name: &str) -> Result<Value> {
    let value = match c {
        Camera::Intrinsics(i) => i.property(name),
        Camera::Params(p) => p.property(name),
        Camera::IntrinsicsErrors(x) => x.property(name),
        Camera::ExtrinsicsErrors(x) => x.property(name),
        Camera::CalibrationErrors(_) => None,
    };
    value.unwrap_or_else(|| {
        Err(format!(
            "Unrecognized method, property, or field '{name}' for class '{}'.",
            c.class()
        ))
    })
}

/// `object.name = value`: every property is read-only.
pub(in crate::engine) fn assign(
    e: &mut Engine,
    c: &Camera,
    steps: &[PathStep],
    out: &mut Execution,
) -> Result<Camera> {
    let class = c.class();
    let name = match steps.first() {
        Some(PathStep::Field(name)) => name.clone(),
        Some(PathStep::Dynamic(expr)) => e.dynamic_field_name(expr, out)?,
        _ => {
            return Err(format!(
                "Indexing into a {class} object is not supported; make a new object instead"
            ));
        }
    };
    let known = c.property_names().contains(&name.as_str()) || name == "IntrinsicMatrix";
    Err(if known {
        format!("Unable to set the '{name}' property of class '{class}' because it is read-only.")
    } else {
        format!("Unrecognized property '{name}' for class '{class}'.")
    })
}

/// The object display after the `name =` line.
pub(in crate::engine) fn body(e: &Engine, c: &Camera) -> String {
    // Intrinsics is an object when the cameraParameters has an ImageSize
    // (its own line below) and [] otherwise (shown like the other values);
    // both cameraCalibrationErrors properties are objects.
    let nested = c.nested();
    let names: Vec<&str> = c
        .property_names()
        .iter()
        .copied()
        .filter(|n| !nested.iter().any(|(m, _)| m == n))
        .collect();
    if names.is_empty() {
        let width = nested.iter().map(|(n, _)| n.len()).max().unwrap_or(0) + 4;
        let text = nested
            .iter()
            .map(|(n, class)| format!("{n:>width$}: [1x1 {class}]"))
            .collect::<Vec<_>>()
            .join("\n");
        return format!("  {} with properties:\n\n{text}", c.class());
    }
    let fields = names
        .iter()
        .map(|n| {
            property(c, n)
                .and_then(|v| v.array(&e.cancel))
                .map(Arc::new)
        })
        .collect::<Result<Vec<_>>>()
        .and_then(|slots| {
            StructArray::from_slots(
                1,
                1,
                names.iter().map(|n| (*n).to_owned()).collect(),
                slots,
                &e.cancel,
            )
        });
    let mut text = fields
        .ok()
        .and_then(|s| crate::display_format::disp(&Array::Struct(s), &e.display).ok())
        .flatten()
        .unwrap_or_default();
    for (n, class) in nested.iter().rev() {
        // The nested object shows as [1x1 class], aligned with the other
        // names.
        let colon = text
            .lines()
            .find_map(|l| l.find(": "))
            .unwrap_or(n.len() + 4);
        let pad = colon.saturating_sub(n.len());
        let start = text.len() - text.trim_start_matches('\n').len();
        text.insert_str(start, &format!("{}{n}: [1x1 {class}]\n", " ".repeat(pad)));
    }
    format!("  {} with properties:\n\n{text}", c.class())
}

fn names_cell(e: &Engine, names: &[&str]) -> Result<Value> {
    let items = names
        .iter()
        .map(|n| {
            CharacterRow::from_units(n.encode_utf16().collect(), &e.cancel)
                .map(|c| Arc::new(Array::Character(c)))
        })
        .collect::<Result<Vec<_>>>()?;
    let rows = items.len();
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        rows,
        usize::from(rows > 0),
        items,
        &e.cancel,
    )?)))
}

fn listing(e: &mut Engine, out: &mut Execution, title: String, names: &[&str]) -> Result<()> {
    let mut text = format!("{title}:\n");
    if !e.display.compact {
        text.insert(0, '\n');
        text.push('\n');
    }
    for n in names {
        text.push_str(&format!("    {n}\n"));
    }
    if e.display.compact {
        text.pop();
    }
    e.emit(out, text)
}

/// Methods and object queries; None for the generic ones objects.rs answers
/// (size, numel, disp, ...).
fn method(
    e: &mut Engine,
    c: &Camera,
    name: &str,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    e.check_cancel()?;
    let class = c.class();
    let one = |v: Value| Ok(Some(vec![v]));
    let text_rest = |e: &Engine, what: &str| -> Result<String> {
        match rest {
            [v] => text_arg(e, v).ok_or_else(|| format!("{name}: {what} must be text")),
            [] => Err("Not enough input arguments.".into()),
            _ => Err("Too many input arguments.".into()),
        }
    };
    match name {
        "class" if rest.is_empty() => return one(Value::Text(class.into())),
        "isa" => {
            let kind = text_rest(e, "the class name")?;
            return one(logical(kind == class));
        }
        "isobject" if rest.is_empty() => return one(logical(true)),
        "isstruct" | "iscell" | "isnumeric" | "ischar" | "islogical" | "isfloat"
            if rest.is_empty() =>
        {
            return one(logical(false));
        }
        "properties" | "fieldnames" | "methods" => {
            if !rest.is_empty() {
                return Err("Too many input arguments.".into());
            }
            let names: &[&str] = if name == "methods" {
                c.methods()
            } else {
                c.property_names()
            };
            if nargout == 0 && name != "fieldnames" {
                let title = if name == "methods" {
                    format!("Methods for class {class}")
                } else {
                    format!("Properties for class {class}")
                };
                listing(e, out, title, names)?;
                return Ok(Some(Vec::new()));
            }
            return one(names_cell(e, names)?);
        }
        "isprop" => {
            let prop = text_rest(e, "the property name")?;
            return one(logical(c.property_names().contains(&prop.as_str())));
        }
        "ismethod" => {
            let m = text_rest(e, "the method name")?;
            return one(logical(c.methods().contains(&m.as_str())));
        }
        "isequal" => {
            if rest.is_empty() {
                return Err("Not enough input arguments.".into());
            }
            let equal = rest
                .iter()
                .all(|v| camera_of(e, v).is_some_and(|o| *o == *c));
            return one(logical(equal));
        }
        "worldToImage" => return world_to_image(e, c, rest, nargout).map(Some),
        "showReprojectionErrors" | "showExtrinsics" => {
            return plots::show(e, Some(c), name, rest, nargout, out).map(Some);
        }
        "displayErrors" => {
            let Camera::CalibrationErrors(x) = c else {
                return Err(undefined(e, name, class));
            };
            // Expression calls (est.displayErrors(p), feval, handles) come
            // with nargout 1 even as statements: return no value, so only a
            // use of the value fails ("This command does not return a value").
            max_outputs(nargout, 1)?;
            let text = estimate::display_errors(e, x, rest)?;
            e.emit(out, text)?;
            return Ok(Some(Vec::new()));
        }
        "eq" | "ne" | "delete" | "isvalid" => return Err(undefined(e, name, class)),
        _ => {}
    }
    if OWNED.contains(&name) {
        return Err(undefined(e, name, class));
    }
    Ok(None)
}
