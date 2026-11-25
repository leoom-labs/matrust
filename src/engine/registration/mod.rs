//! Image registration, piece 7c1: the spatial referencing object imref2d and
//! the 2-D geometric transformation objects affinetform2d, rigidtform2d,
//! simtform2d, transltform2d (premultiply, R2022b names) and affine2d
//! (postmultiply). Behaviour follows the public MathWorks reference pages.
//!
//! All six are value classes kept in the object table (objects.rs,
//! `ObjState::Geo`): `t.A = M` gives the variable a changed copy, other
//! copies keep their values. Other engine pieces (imwarp, imregtform, ...)
//! read objects with [`tform_of`] / [`imref_of`] and make them with
//! [`new_object`]; see docs/tasks for the API notes.
mod imref;
mod register;
mod tform;
mod warp;

pub(in crate::engine) use imref::Imref2d;
#[allow(unused_imports)] // API for the Registration Estimator (7c4).
pub(in crate::engine) use register::{Image, Progress, Settings, Stop, TransformType, estimate};
#[allow(unused_imports)]
pub(in crate::engine) use tform::{Kind, Lu2, Mat2, Mat3, TOL, Tform, atan2d, cosd, sind};
#[allow(unused_imports)] // API for imregister (7c3) and the Registration Estimator (7c4).
pub(in crate::engine) use warp::{Interp, Sampling, default_view, remap, warp};

use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, NumericClass, Result, StructArray,
    TypedArray,
    syntax::{Expr, PathStep},
};
use std::sync::Arc;

/// One registration object.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) enum Geo {
    Imref(Imref2d),
    Tform(Tform),
}

/// Classes, in the order of their display masks (objects.rs obj_mask).
pub(in crate::engine) const CLASSES: [&str; 6] = [
    "imref2d",
    "affinetform2d",
    "rigidtform2d",
    "simtform2d",
    "transltform2d",
    "affine2d",
];

/// Methods no other builtin has: answered whatever the arguments are.
const OWNED_METHODS: [&str; 11] = [
    "transformPointsForward",
    "transformPointsInverse",
    "invert",
    "outputLimits",
    "isRigid",
    "isSimilarity",
    "isTranslation",
    "worldToIntrinsic",
    "intrinsicToWorld",
    "worldToSubscript",
    "sizesMatch",
];
// `contains` is the string function unless its first argument is an imref2d.

impl Geo {
    pub(in crate::engine) fn class(&self) -> &'static str {
        match self {
            Geo::Imref(_) => "imref2d",
            Geo::Tform(t) => t.class(),
        }
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + 64
    }
    pub(in crate::engine) fn property_names(&self) -> &'static [&'static str] {
        match self {
            Geo::Imref(_) => &Imref2d::PROPERTIES,
            Geo::Tform(t) => t.property_names(),
        }
    }
    pub(in crate::engine) fn method_names(&self) -> &'static [&'static str] {
        match self {
            Geo::Imref(_) => &Imref2d::METHODS,
            Geo::Tform(t) => t.method_names(),
        }
    }
    /// A property value, or None for a name that is not a property.
    pub(in crate::engine) fn property_value(&self, name: &str) -> Option<Result<Value>> {
        let (rows, cols, data) = match self {
            Geo::Imref(r) => {
                let data = r.property(name)?;
                (1, data.len(), data)
            }
            Geo::Tform(t) => t.property(name)?,
        };
        Some(Matrix::from_column_vec(rows, cols, data).map(Value::Matrix))
    }
}

/// Names whose bare use (`t = rigidtform2d;`, `R = imref2d`) is the
/// zero-argument constructor call.
pub(in crate::engine) fn bare(name: &str) -> bool {
    CLASSES.contains(&name)
}

/// Names whose several-output calls this module answers (the ones with two
/// outputs, and the rest to report "Too many output arguments").
pub(in crate::engine) fn multi_output(name: &str) -> bool {
    CLASSES.contains(&name)
        || OWNED_METHODS.contains(&name)
        || matches!(name, "imwarp" | "imregtform" | "imregister")
}

/// The registration object an argument holds, if any.
pub(in crate::engine) fn geo_of(e: &Engine, v: &Value) -> Option<Arc<Geo>> {
    if !e.obj_live() {
        return None;
    }
    super::objects::geo_value(e, v)
}

/// The transformation object an argument holds (any of the five classes).
pub(in crate::engine) fn tform_of(e: &Engine, v: &Value) -> Option<Tform> {
    match geo_of(e, v)?.as_ref() {
        Geo::Tform(t) => Some(t.clone()),
        Geo::Imref(_) => None,
    }
}

/// The imref2d an argument holds, if any.
pub(in crate::engine) fn imref_of(e: &Engine, v: &Value) -> Option<Imref2d> {
    match geo_of(e, v)?.as_ref() {
        Geo::Imref(r) => Some(r.clone()),
        Geo::Tform(_) => None,
    }
}

/// A new object value (a fresh object-table entry).
pub(in crate::engine) fn new_object(e: &mut Engine, g: Geo) -> Result<Value> {
    super::objects::geo_new(e, g)
}

fn class_of(e: &Engine, v: Option<&Value>) -> String {
    match v {
        None => "double".into(),
        Some(v) => match geo_of(e, v) {
            Some(g) => g.class().into(),
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

/// A real 2-D numeric array as rows, columns and column-major values.
fn array_numbers(who: &str, what: &str, a: &Array) -> Result<(usize, usize, Vec<f64>)> {
    if a.is_nd() {
        return Err(format!("{who}: {what} must be a 2-D numeric array"));
    }
    match a {
        Array::Numeric(m) => Ok((m.rows(), m.cols(), m.data().to_vec())),
        Array::Typed(t) => Ok((t.rows(), t.cols(), t.to_f64_vec()?)),
        Array::Complex(_) => Err(format!("{who}: {what} must be real")),
        _ => Err(format!("{who}: {what} must be numeric, not {}", a.class())),
    }
}

fn numbers(e: &Engine, who: &str, what: &str, v: &Value) -> Result<(usize, usize, Vec<f64>)> {
    if let Value::Matrix(m) = v
        && !m.is_nd()
    {
        return Ok((m.rows(), m.cols(), m.data().to_vec()));
    }
    array_numbers(who, what, &v.clone().array(&e.cancel)?)
}

fn is_vector2(n: &(usize, usize, Vec<f64>)) -> bool {
    n.2.len() == 2 && (n.0 == 1 || n.1 == 1)
}

fn vector2(who: &str, what: &str, n: &(usize, usize, Vec<f64>)) -> Result<[f64; 2]> {
    if !is_vector2(n) {
        return Err(format!("{who}: {what} must be a 2-element numeric vector"));
    }
    Ok([n.2[0], n.2[1]])
}

fn scalar(who: &str, what: &str, n: &(usize, usize, Vec<f64>)) -> Result<f64> {
    if n.2.len() != 1 {
        return Err(format!("{who}: {what} must be a numeric scalar"));
    }
    Ok(n.2[0])
}

fn mat2(n: &(usize, usize, Vec<f64>)) -> Option<Mat2> {
    ((n.0, n.1) == (2, 2)).then(|| [[n.2[0], n.2[2]], [n.2[1], n.2[3]]])
}

fn tform_usage(kind: Kind) -> String {
    match kind {
        Kind::Affine => "affinetform2d: expected affinetform2d(), affinetform2d(A) with A 3-by-3 or its 2-by-3 top rows, or affinetform2d(tform)",
        Kind::Rigid => "rigidtform2d: expected rigidtform2d(), rigidtform2d(t), rigidtform2d(r, t), rigidtform2d(R, t), rigidtform2d(A) or rigidtform2d(tform)",
        Kind::Sim => "simtform2d: expected simtform2d(), simtform2d(s, r, t), simtform2d(s, R, t), simtform2d(A) or simtform2d(tform)",
        Kind::Transl => "transltform2d: expected transltform2d(), transltform2d(tx, ty), transltform2d(t), transltform2d(A) or transltform2d(tform)",
        Kind::Affine2d => "affine2d: expected affine2d() or affine2d(T) with T a 3-by-3 matrix",
    }
    .into()
}

/// The transformation constructors.
fn construct_tform(e: &Engine, kind: Kind, args: &[Value]) -> Result<Tform> {
    let who = kind.class();
    if args.is_empty() {
        return Ok(Tform::identity(kind));
    }
    if let [arg] = args
        && let Some(g) = geo_of(e, arg)
    {
        return match g.as_ref() {
            Geo::Tform(t) if kind != Kind::Affine2d => {
                Tform::from_premultiply(kind, t.matrix(), who)
            }
            other => Err(format!(
                "{who}: cannot make a {who} from a {} object",
                other.class()
            )),
        };
    }
    let n = args
        .iter()
        .map(|a| numbers(e, who, "each input", a))
        .collect::<Result<Vec<_>>>()?;
    let matrix = |m: &(usize, usize, Vec<f64>), what: &str| {
        tform::matrix3(who, what, m.0, m.1, &m.2, kind != Kind::Affine2d)
    };
    match (kind, n.as_slice()) {
        (Kind::Affine2d, [m]) => Tform::affine2d(matrix(m, "T")?),
        (Kind::Affine, [m]) => Tform::affine(matrix(m, "A")?),
        (Kind::Rigid, [m]) if is_vector2(m) => Tform::rigid(0., [m.2[0], m.2[1]]),
        (Kind::Transl, [m]) if is_vector2(m) => Tform::translation([m.2[0], m.2[1]]),
        (Kind::Rigid | Kind::Sim | Kind::Transl, [m]) => {
            Tform::from_premultiply(kind, matrix(m, "the matrix")?, who)
        }
        (Kind::Rigid, [r, t]) => {
            let t = vector2(who, "the translation t", t)?;
            match (r.2.len(), mat2(r)) {
                (1, _) => Tform::rigid(r.2[0], t),
                (_, Some(r)) => Tform::rigid_from_rotation(r, t),
                _ => Err(format!(
                    "{who}: the rotation must be an angle in degrees or a 2-by-2 rotation matrix"
                )),
            }
        }
        (Kind::Sim, [s, r, t]) => {
            let s = scalar(who, "the scale s", s)?;
            let t = vector2(who, "the translation t", t)?;
            match (r.2.len(), mat2(r)) {
                (1, _) => Tform::similarity(s, r.2[0], t),
                (_, Some(r)) => Tform::similarity_from_rotation(s, r, t),
                _ => Err(format!(
                    "{who}: the rotation must be an angle in degrees or a 2-by-2 rotation matrix"
                )),
            }
        }
        (Kind::Transl, [tx, ty]) => {
            Tform::translation([scalar(who, "tx", tx)?, scalar(who, "ty", ty)?])
        }
        _ => Err(tform_usage(kind)),
    }
}

fn construct_imref(e: &Engine, args: &[Value]) -> Result<Imref2d> {
    let who = "imref2d";
    let size = |v: &Value| -> Result<[f64; 2]> {
        let n = numbers(e, who, "imageSize", v)?;
        if n.0 != 1 && n.1 != 1 {
            return Err(format!("{who}: imageSize must be a vector [rows cols]"));
        }
        imref::image_size(&n.2)
    };
    match args {
        [] => Ok(Imref2d::new([2., 2.])),
        [s] => Ok(Imref2d::new(size(s)?)),
        [s, a, b] => {
            let size = size(s)?;
            let a = numbers(e, who, "the second input", a)?;
            let b = numbers(e, who, "the third input", b)?;
            match (a.2.len(), b.2.len()) {
                (1, 1) => Imref2d::with_extents(size, a.2[0], b.2[0]),
                (2, 2) if is_vector2(&a) && is_vector2(&b) => {
                    Imref2d::with_limits(size, &a.2, &b.2)
                }
                _ => Err(format!(
                    "{who}: expected pixel extents (two positive scalars) or world limits (two 2-element vectors)"
                )),
            }
        }
        _ => Err(format!(
            "{who}: expected imref2d(), imref2d(imageSize), imref2d(imageSize, xWorldLimits, yWorldLimits) or imref2d(imageSize, pixelExtentInWorldX, pixelExtentInWorldY)"
        )),
    }
}

/// Builtin calls: the constructors, the methods above and every call whose
/// first argument is a registration object. None leaves the call to the
/// other builtins (class queries, size, disp, ...).
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    if name == "imwarp" {
        return warp::imwarp(e, args, nargout).map(Some);
    }
    if name == "imregtform" || name == "imregister" {
        return register::builtin(e, name, args, nargout, out).map(Some);
    }
    let constructor = CLASSES.contains(&name);
    let geo = args.first().and_then(|v| geo_of(e, v));
    if !constructor && geo.is_none() {
        if !OWNED_METHODS.contains(&name) {
            return Ok(None);
        }
        let class = class_of(e, args.first());
        return Err(undefined(e, name, &class));
    }
    if constructor {
        if nargout > 1 {
            return Err("Too many output arguments.".into());
        }
        let g = match Kind::parse(name) {
            Some(kind) => Geo::Tform(construct_tform(e, kind, args)?),
            None => Geo::Imref(construct_imref(e, args)?),
        };
        return Ok(Some(vec![new_object(e, g)?]));
    }
    let g = geo.expect("checked above");
    method(e, &g, name, &args[1..], nargout, out)
}

/// `name(object, ...)` reached through objects.rs (statements: nargout 0,
/// and `object.method(...)`); None for the generic object queries.
pub(in crate::engine) fn object_method(
    e: &mut Engine,
    g: &Geo,
    name: &str,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    Ok(method(e, g, name, rest, 0, out)?.map(|mut v| {
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
    g: &Geo,
    name: &str,
    args: Option<&[Expr]>,
    out: &mut Execution,
) -> Result<Value> {
    if g.method_names().contains(&name) {
        let rest = match args {
            Some(args) => e.function_arguments(args, None, out)?,
            None => Vec::new(),
        };
        let value = method(e, g, name, &rest, 1, out)?;
        return Ok(value
            .and_then(|mut v| (!v.is_empty()).then(|| v.swap_remove(0)))
            .unwrap_or(Value::Void));
    }
    let value = property(g, name)?;
    match args {
        Some(args) if !args.is_empty() => e.index_value(value, args, out),
        _ => Ok(value),
    }
}

/// `object.name` (a property).
pub(in crate::engine) fn property(g: &Geo, name: &str) -> Result<Value> {
    g.property_value(name).unwrap_or_else(|| {
        Err(format!(
            "Unrecognized method, property, or field '{name}' for class '{}'.",
            g.class()
        ))
    })
}

/// `object.name = value` and `object.name(i) = value`: the changed copy.
pub(in crate::engine) fn assign(
    e: &mut Engine,
    g: &Geo,
    steps: &[PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<Geo> {
    let class = g.class();
    let name = match steps.first() {
        Some(PathStep::Field(name)) => name.clone(),
        Some(PathStep::Dynamic(expr)) => e.dynamic_field_name(expr, out)?,
        _ => {
            return Err(format!(
                "Indexing into a {class} object is not supported; assign one of its properties (for example tform.A = M)"
            ));
        }
    };
    if !g.property_names().contains(&name.as_str()) {
        return Err(format!(
            "Unrecognized property '{name}' for class '{class}'."
        ));
    }
    let value = if steps.len() > 1 {
        let current = property(g, &name)?.array(&e.cancel)?;
        e.assign_step(Some(current), &steps[1..], value, false, out)?
    } else {
        value
    };
    let (rows, cols, data) = array_numbers(class, &name, &value)?;
    Ok(match g {
        Geo::Tform(t) => Geo::Tform(t.with_property(&name, rows, cols, &data)?),
        Geo::Imref(r) => {
            if rows != 1 && cols != 1 {
                return Err(format!("{class}: {name} must be a vector"));
            }
            Geo::Imref(r.with_property(&name, &data)?)
        }
    })
}

/// The object display after the `name =` line.
pub(in crate::engine) fn body(e: &Engine, g: &Geo) -> String {
    let names = g.property_names();
    let fields = names
        .iter()
        .map(|n| {
            property(g, n)
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
    let text = fields
        .ok()
        .and_then(|s| crate::display_format::disp(&Array::Struct(s), &e.display).ok())
        .flatten()
        .unwrap_or_default();
    format!("  {} with properties:\n\n{text}", g.class())
}

fn text_arg(e: &Engine, v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_ref().map(|s| s.to_string())
        }
        Value::Container(_) => e.text_value(v.clone()).ok(),
        _ => None,
    }
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

fn logical(b: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(b))
}

/// Methods and object queries; None for the generic ones objects.rs answers
/// (size, numel, disp, ...).
fn method(
    e: &mut Engine,
    g: &Geo,
    name: &str,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    e.check_cancel()?;
    let class = g.class();
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
            let names = if name == "methods" {
                g.method_names()
            } else {
                g.property_names()
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
            return one(logical(g.property_names().contains(&prop.as_str())));
        }
        "ismethod" => {
            let m = text_rest(e, "the method name")?;
            return one(logical(g.method_names().contains(&m.as_str())));
        }
        "isequal" => {
            if rest.is_empty() {
                return Err("Not enough input arguments.".into());
            }
            let equal = rest.iter().all(|v| geo_of(e, v).is_some_and(|o| *o == *g));
            return one(logical(equal));
        }
        "eq" | "ne" | "delete" | "isvalid" => return Err(undefined(e, name, class)),
        _ => {}
    }
    let outputs = match g {
        Geo::Tform(t) if t.method_names().contains(&name) => {
            tform_method(e, t, name, rest, nargout)?
        }
        Geo::Imref(r) if Imref2d::METHODS.contains(&name) => {
            imref_method(e, r, name, rest, nargout)?
        }
        _ if OWNED_METHODS.contains(&name) || name == "contains" => {
            return Err(undefined(e, name, class));
        }
        _ => return Ok(None),
    };
    Ok(Some(outputs))
}

/// Point coordinates: real double or single, any shape.
struct Points {
    dims: Vec<usize>,
    data: Vec<f64>,
    single: bool,
}

fn points(e: &Engine, who: &str, v: &Value) -> Result<Points> {
    let dims = v.dims().into_owned();
    let (data, single) = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => (m.data().to_vec(), false),
        Value::Container(Array::Typed(t)) if t.class() == NumericClass::Single => {
            (t.to_f64_vec()?, true)
        }
        Value::Complex(_) | Value::Container(Array::Complex(_)) => {
            return Err(format!("{who}: point coordinates must be real"));
        }
        other => {
            return Err(format!(
                "{who}: point coordinates must be single or double, not {}",
                class_of(e, Some(other))
            ));
        }
    };
    Ok(Points { dims, data, single })
}

fn points_value(dims: &[usize], data: Vec<f64>, single: bool) -> Result<Value> {
    let rows = dims[0];
    let cols = dims[1..].iter().product();
    Ok(if single {
        let t = TypedArray::from_reals(NumericClass::Single, rows, cols, data.into_iter())?;
        Value::Container(Array::Typed(if dims.len() > 2 {
            t.with_dims(dims)?
        } else {
            t
        }))
    } else {
        let m = Matrix::from_column_vec(rows, cols, data)?;
        Value::Matrix(if dims.len() > 2 {
            m.with_dims(dims)?
        } else {
            m
        })
    })
}

/// Two same-size coordinate arrays.
fn point_pair(e: &Engine, who: &str, x: &Value, y: &Value) -> Result<(Points, Points)> {
    let (x, y) = (points(e, who, x)?, points(e, who, y)?);
    if x.dims != y.dims {
        return Err(format!(
            "{who}: the two coordinate arrays must be the same size"
        ));
    }
    Ok((x, y))
}

fn max_outputs(nargout: usize, max: usize) -> Result<()> {
    if nargout > max {
        Err("Too many output arguments.".into())
    } else {
        Ok(())
    }
}

fn tform_method(
    e: &mut Engine,
    t: &Tform,
    name: &str,
    rest: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    match name {
        "transformPointsForward" | "transformPointsInverse" => {
            let forward = name == "transformPointsForward";
            let apply = |e: &Engine, x: &mut [f64], y: &mut [f64]| {
                if forward {
                    t.forward_points(x, y, || e.check_cancel())
                } else {
                    t.inverse_points(x, y, || e.check_cancel())
                        .map_err(|m| format!("{name}: {m}"))
                }
            };
            match rest {
                [xy] => {
                    max_outputs(nargout, 1)?;
                    let p = points(e, name, xy)?;
                    if p.dims.len() != 2 || p.dims[1] != 2 {
                        return Err(format!(
                            "{name}: a single point argument must be an n-by-2 matrix [x y]"
                        ));
                    }
                    let n = p.dims[0];
                    let mut data = p.data;
                    let (x, y) = data.split_at_mut(n);
                    apply(e, x, y)?;
                    Ok(vec![points_value(&p.dims, data, p.single)?])
                }
                [x, y] => {
                    max_outputs(nargout, 2)?;
                    let (mut x, mut y) = point_pair(e, name, x, y)?;
                    apply(e, &mut x.data, &mut y.data)?;
                    let mut outputs = vec![points_value(&x.dims, x.data, x.single || y.single)?];
                    if nargout > 1 {
                        outputs.push(points_value(&y.dims, y.data, x.single || y.single)?);
                    }
                    Ok(outputs)
                }
                _ => Err(format!(
                    "{name}: expected {name}(tform, x, y) or {name}(tform, XY) with XY n-by-2"
                )),
            }
        }
        "invert" => {
            max_outputs(nargout, 1)?;
            if !rest.is_empty() {
                return Err("Too many input arguments.".into());
            }
            let inverse = t.inverse()?;
            Ok(vec![new_object(e, Geo::Tform(inverse))?])
        }
        "outputLimits" => {
            max_outputs(nargout, 2)?;
            let [xl, yl] = rest else {
                return Err(format!(
                    "{name}: expected outputLimits(tform, xLimitsIn, yLimitsIn)"
                ));
            };
            let limits = |v: &Value, what: &str| -> Result<[f64; 2]> {
                let n = numbers(e, name, what, v)?;
                let l = vector2(name, what, &n)?;
                if !(l[0].is_finite() && l[1].is_finite()) {
                    return Err(format!("{name}: {what} must be finite"));
                }
                Ok(l)
            };
            let (x, y) = t.output_limits(limits(xl, "xLimitsIn")?, limits(yl, "yLimitsIn")?);
            let row = |v: [f64; 2]| Matrix::from_column_vec(1, 2, v.to_vec()).map(Value::Matrix);
            Ok(vec![row(x)?, row(y)?])
        }
        "isRigid" | "isSimilarity" | "isTranslation" => {
            max_outputs(nargout, 1)?;
            if !rest.is_empty() {
                return Err("Too many input arguments.".into());
            }
            Ok(vec![logical(match name {
                "isRigid" => t.is_rigid(),
                "isSimilarity" => t.is_similarity(),
                _ => t.is_translation(),
            })])
        }
        _ => unreachable!("checked against method_names"),
    }
}

fn imref_method(
    e: &mut Engine,
    r: &Imref2d,
    name: &str,
    rest: &[Value],
    nargout: usize,
) -> Result<Vec<Value>> {
    if name == "sizesMatch" {
        max_outputs(nargout, 1)?;
        let [a] = rest else {
            return Err(format!("{name}: expected sizesMatch(R, A)"));
        };
        let dims = a.dims();
        let same = dims[0] as f64 == r.size[0] && dims[1] as f64 == r.size[1];
        return Ok(vec![logical(same)]);
    }
    let [x, y] = rest else {
        return Err(format!("{name}: expected {name}(R, x, y)"));
    };
    let (x, y) = point_pair(e, name, x, y)?;
    let n = x.data.len();
    if name == "contains" {
        max_outputs(nargout, 1)?;
        let mut flags = Vec::with_capacity(n);
        for (i, (&a, &b)) in x.data.iter().zip(&y.data).enumerate() {
            if i % 65536 == 65535 {
                e.check_cancel()?;
            }
            flags.push(u8::from(r.contains(a, b)));
        }
        let rows = x.dims[0];
        let cols = x.dims[1..].iter().product();
        let m = LogicalMatrix::from_column_vec(rows, cols, flags, &e.cancel)?;
        return Ok(vec![Value::Logical(if x.dims.len() > 2 {
            m.with_dims(&x.dims)?
        } else {
            m
        })]);
    }
    max_outputs(nargout, 2)?;
    let single = x.single || y.single;
    let (mut a, mut b) = (x.data, y.data);
    for (i, (u, v)) in a.iter_mut().zip(b.iter_mut()).enumerate() {
        if i % 65536 == 65535 {
            e.check_cancel()?;
        }
        (*u, *v) = match name {
            "worldToIntrinsic" => r.world_to_intrinsic(*u, *v),
            "intrinsicToWorld" => r.intrinsic_to_world(*u, *v),
            // [I, J] = rows from y, columns from x.
            _ => {
                let (row, col) = r.world_to_subscript(*u, *v);
                (row, col)
            }
        };
    }
    let single = single && name != "worldToSubscript";
    let mut outputs = vec![points_value(&x.dims, a, single)?];
    if nargout > 1 {
        outputs.push(points_value(&x.dims, b, single)?);
    }
    Ok(outputs)
}
