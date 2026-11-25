//! Folded N-D engine paths. Existing two-dimensional kernels stay unchanged.
use super::{Engine, Execution, Value, indexing::Indices, nd_reduce};
use crate::syntax::Token;
use crate::{Array, ComplexMatrix, LogicalMatrix, Matrix, Result};
use std::sync::atomic::AtomicBool;

/// Builtins admitted for folded storage. Extend only with an implemented path.
pub(super) const ND_AWARE: &[&str] = &[
    "zeros",
    "ones",
    "true",
    "false",
    "nan",
    "NaN",
    "inf",
    "Inf",
    "rand",
    "randn",
    "randi",
    "reshape",
    "repmat",
    "cat",
    "horzcat",
    "vertcat",
    "size",
    "ndims",
    "numel",
    "length",
    "isempty",
    "isscalar",
    "isvector",
    "ismatrix",
    "isrow",
    "iscolumn",
    "class",
    "isequal",
    "permute",
    "ipermute",
    "squeeze",
    "sum",
    "prod",
    "mean",
    "max",
    "min",
    "any",
    "all",
    "abs",
    "sqrt",
    "exp",
    "log",
    "sin",
    "cos",
    "round",
    "floor",
    "ceil",
    "real",
    "imag",
    "conj",
    "disp",
    "image",
    "imagesc",
    "imshow",
    "set",
    "display",
    "char",
    "logical",
    "int8",
    "uint8",
    "int16",
    "uint16",
    "int32",
    "uint32",
    "int64",
    "uint64",
    "single",
    "double", // Part 2 (2026-11-01).
    "cumsum",
    "cumprod",
    "find",
    "sort",
    "shiftdim",
    "circshift",
    "meshgrid",
    "ndgrid",
    "complex",
    // File I/O gaps (2026-11-04): N-D datasets.
    "h5write",
    // Statistics (2026-11-11): rms reduces N-D input through the mean reduction.
    "rms",
    "vecnorm",
    // Image capacity (2026-11-11): realness and class are storage queries,
    // independent of the number of axes.
    "isreal",
    "isa",
    // Wavelets: icwt takes the scales-by-N-by-2 transform of a complex signal.
    "icwt", // Desktop launch accepts shared multidimensional image sequences.
    "implay",
    // Dense dimension grouping: the source and its blocks may be N-D. One
    // input and an N-D outer cell grid stay refused (see `call`).
    "num2cell",
    "flow",
    "smooth3",
    "reducevolume",
    "subvolume",
    "volumebounds",
    "interp3",
    "isosurface",
    "isonormals",
    "isocaps",
    "slice",
    "contourslice",
    "volshow",
    "sliceViewer",
    "orthosliceViewer",
    "getAxesHandles",
];

pub(super) fn needs_dispatch(name: &str, args: &[Value]) -> bool {
    if args.iter().any(Value::is_nd) || nd_reduce::needs_dispatch(name, args) {
        return true;
    }
    let Some(first) = name.as_bytes().first().copied() else {
        return false;
    };
    if !matches!(
        first,
        b'z' | b'o' | b't' | b'f' | b'n' | b'N' | b'i' | b'I' | b'r' | b'c' | b'p' | b's'
    ) {
        return false;
    }
    matches!(
        name,
        "zeros"
            | "ones"
            | "true"
            | "false"
            | "nan"
            | "NaN"
            | "inf"
            | "Inf"
            | "rand"
            | "randn"
            | "randi"
            | "reshape"
            | "repmat"
            | "cat"
            | "permute"
            | "ipermute"
            | "shiftdim"
    )
}

pub(super) fn guard(name: &str, args: &[Value]) -> Result<()> {
    let Some(value) = args.iter().find(|v| v.is_nd()) else {
        return Ok(());
    };
    if nd_reduce::handles(name)
        || ND_AWARE.contains(&name)
        || expanding_function(name)
        || image_function(name)
        || super::fuzzy::mf::NAMES.contains(&name)
        || name == "evalmf"
        || feval_forwards(name, args)
    {
        return Ok(());
    }
    let _ = value;
    Err(unsupported(name, args))
}

// These builtins keep their usual dispatch and apply expansion once both
// operands reach the arithmetic kernel.
fn expanding_function(name: &str) -> bool {
    super::core_more::elementwise_operator(name).is_some()
        || super::elementary::binary_function(name).is_some()
}

/// `feval(f, x, ...)` with a camera function handle or name forwards to the
/// callee's own N-D guard; all other `feval` targets keep the pre-7d N-D error.
fn feval_forwards(name: &str, args: &[Value]) -> bool {
    if name != "feval" {
        return false;
    }
    match args.first() {
        Some(Value::Text(s)) => camera_nd_function(s.trim()),
        Some(Value::Container(Array::Character(c))) if c.dims().len() == 2 && c.dims()[0] == 1 => {
            let s = String::from_utf16_lossy(c.units());
            camera_nd_function(s.trim())
        }
        Some(Value::Container(Array::Handle(h))) => camera_nd_function(h.func2str()),
        _ => false,
    }
}

fn camera_nd_function(name: &str) -> bool {
    super::camera::detect::handles(name)
        || super::camera::takes_nd(name)
        || super::camera::takes_images(name)
}

/// Image builtins take H-by-W-by-C arrays; direct calls reach them before the
/// guard, so handle calls (`h = @rgb2gray; h(rgb)`) must too.
fn image_function(name: &str) -> bool {
    super::image_basic::handles(name)
        || super::image_basic::registration_handles(name)
        || matches!(
            name,
            "imwarp" | "imregtform" | "imregister" | "registrationEstimator"
        )
        || super::image_color::handles(name)
        || super::image_segment::handles(name)
        || super::image_morph::handles(name)
        || camera_nd_function(name)
}

/// Shared argument boundary for builtin expression, multi-output and statement dispatch.
/// User functions keep their existing precedence and may receive N-D values.
pub(super) fn arguments(
    engine: &mut Engine,
    name: &str,
    expressions: &[crate::syntax::Expr],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let args = engine.function_arguments(expressions, None, out)?;
    if args.iter().any(Value::is_nd) && engine.resolve_function(name)?.is_none() {
        guard(name, &args)?;
    }
    Ok(args)
}

pub(super) fn unsupported(name: &str, args: &[Value]) -> String {
    let size = args
        .iter()
        .find(|v| v.is_nd())
        .map(|v| {
            v.dims()
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("x")
        })
        .unwrap_or_default();
    format!("{name} does not support N-D arrays yet (size {size})")
}

pub(super) fn shape_product(dims: &[usize]) -> Result<usize> {
    dims.iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or_else(|| "Array dimensions overflow".into())
}

pub(super) fn index(
    engine: &mut Engine,
    value: Value,
    args: &[crate::syntax::Expr],
    out: &mut Execution,
) -> Result<Value> {
    if args.is_empty() || args.len() > 32 {
        return Err("Indexing requires between one and 32 subscripts".into());
    }
    let dims = value.dims().into_owned();
    let mut effective = dims.clone();
    if args.len() == 1 {
        effective = vec![shape_product(&dims)?];
    } else if args.len() < dims.len() {
        effective.truncate(args.len());
        effective[args.len() - 1] = shape_product(&dims[args.len() - 1..])?;
    } else {
        effective.resize(args.len(), 1);
    }
    let mut subs = Vec::with_capacity(args.len());
    let mut linear_shape = None;
    for (i, arg) in args.iter().enumerate() {
        let (indices, shape) = engine.subscript(arg, effective[i], out)?;
        if args.len() == 1 {
            linear_shape = Some(shape);
        }
        subs.push(indices);
    }
    let mut result_dims = subs.iter().map(Indices::len).collect::<Vec<_>>();
    if args.len() == 1 {
        let shape = linear_shape.expect("single-subscript shape");
        result_dims = match &subs[0] {
            // MATLAB rule: A(idx) has the shape of idx unless both are vectors.
            Indices::Explicit(values) if values.is_nd() => values.dims().into_owned(),
            _ => vec![shape.0, shape.1],
        };
    }
    if result_dims.len() == 1 {
        result_dims.push(1);
    }
    let count = shape_product(&result_dims)?;
    // Element and memory admission before the position list (8 bytes per element)
    // and the result are allocated.
    Matrix::checked_len(result_dims[0], shape_product(&result_dims[1..])?)?;
    engine.preflight_elements(count)?;
    let mut positions = Vec::with_capacity(count);
    let mut strides = Vec::with_capacity(effective.len());
    let mut stride = 1usize;
    for &d in &effective {
        strides.push(stride);
        stride = stride
            .checked_mul(d.max(1))
            .ok_or("Index stride overflow")?;
    }
    for linear in 0..count {
        if linear % 8192 == 0 {
            engine.check_cancel()?;
        }
        let mut q = linear;
        let mut pos = 0usize;
        for (axis, sub) in subs.iter().enumerate() {
            let coord = q % sub.len().max(1);
            q /= sub.len().max(1);
            if sub.len() > 0 {
                pos = pos
                    .checked_add(
                        sub.at(coord)
                            .checked_mul(strides[axis])
                            .ok_or("Index overflow")?,
                    )
                    .ok_or("Index overflow")?;
            }
        }
        positions.push(pos);
    }
    let rows = result_dims[0];
    let cols = shape_product(&result_dims[1..])?;
    let canonical = crate::array::normalize_dims(&result_dims)?;
    Ok(match value {
        Value::Matrix(a) => {
            let data = positions.iter().map(|&i| a.data()[i]).collect();
            Value::Matrix(Matrix::from_column_vec(rows, cols, data)?.with_dims(&canonical)?)
        }
        Value::Logical(a) => {
            let data = positions.iter().map(|&i| a.data()[i]).collect();
            Value::Logical(
                LogicalMatrix::from_column_vec(rows, cols, data, &engine.cancel)?
                    .with_dims(&canonical)?,
            )
        }
        Value::Complex(a) => {
            let data = positions.iter().map(|&i| a.data()[i]).collect();
            Value::Complex(
                crate::ComplexMatrix::from_column_vec(rows, cols, data, &engine.cancel)?
                    .with_single_class(a.is_single())
                    .with_dims(&canonical)?,
            )
        }
        Value::Container(Array::Typed(a)) => Value::Container(Array::Typed(
            a.select_with_cancel(rows, cols, &positions, &engine.cancel)?
                .with_dims(&canonical)?,
        )),
        Value::Container(Array::Character(a)) => {
            let data = positions.iter().map(|&i| a.units()[i]).collect();
            Value::Container(
                crate::CharacterRow::from_column_major(rows, cols, data, &engine.cancel)?
                    .with_dims(&canonical)?
                    .into(),
            )
        }
        _ => return Err("N-D indexing is unsupported for this array class".into()),
    })
}

/// A scalar expands. Otherwise the subscript lengths and the right-hand side
/// must share the same non-singleton sizes, in order (size 1 is ignored; size 0
/// is not). One linear subscript checks the element count only.
/// Octave 10.3; MathWorks non-scalar-dimension rule (MATLAB Answers 745702).
fn subsasgn_shape_agrees(subs: &[Indices], rhs_dims: &[usize]) -> bool {
    if subs.len() < 2 {
        return true;
    }
    subs.iter()
        .map(Indices::len)
        .filter(|&n| n != 1)
        .eq(rhs_dims.iter().copied().filter(|&n| n != 1))
}

fn subsasgn_rhs_dims(value: &Value, rhs_len: usize) -> Vec<usize> {
    match value {
        // The assignment counts UTF-16 units, which is the row those values form.
        Value::Text(_) | Value::QuotedString(_) => vec![1, rhs_len],
        other => other.dims().into_owned(),
    }
}

pub(super) fn assign(
    engine: &mut Engine,
    name: &str,
    args: &[crate::syntax::Expr],
    value: Value,
    deletion: bool,
    quiet: bool,
    out: &mut Execution,
) -> Result<()> {
    if args.is_empty() || args.len() > 32 {
        return Err("Indexed assignment requires between one and 32 subscripts".into());
    }
    let old = engine.array(name);
    let original_dims = old
        .as_ref()
        .map_or_else(|| vec![0, 0], |a| a.dims().into_owned());
    let mut effective = original_dims.clone();
    if args.len() == 1 {
        effective = vec![shape_product(&original_dims)?];
    } else if args.len() < original_dims.len() {
        effective.truncate(args.len());
        effective[args.len() - 1] = shape_product(&original_dims[args.len() - 1..])?;
    } else {
        effective.resize(args.len(), usize::from(old.is_some()));
    }
    let mut subs = Vec::with_capacity(args.len());
    for (axis, arg) in args.iter().enumerate() {
        let indices = if engine.colon_subscript(arg) {
            engine
                .subscript_with_limit(arg, Some(effective[axis]), effective[axis], out)?
                .0
        } else {
            engine
                .subscript_with_limit(arg, Some(effective[axis]), crate::matrix::MAX_ELEMENTS, out)?
                .0
        };
        subs.push(indices);
    }
    let mut dims = original_dims.clone();
    dims.resize(args.len().max(dims.len()), 1);
    if deletion {
        return delete(engine, name, &effective, &subs, old, quiet, out);
    }
    for (axis, sub) in subs.iter().enumerate() {
        let extent = (0..sub.len()).map(|i| sub.at(i) + 1).max().unwrap_or(0);
        if args.len() < original_dims.len() {
            if extent > effective[axis] {
                return Err("Attempt to grow array along ambiguous dimension".into());
            }
        } else {
            dims[axis] = dims[axis].max(extent);
        }
    }
    if matches!(&old, Some(Array::Typed(a)) if a.class() != crate::NumericClass::Single)
        && matches!(&value, Value::Complex(_))
    {
        return Err("Complex indexed assignment into an integer array is not supported".into());
    }
    if old.as_ref().is_some_and(|a| matches!(a, Array::Complex(_)))
        || matches!(&value, Value::Complex(_))
    {
        return assign_complex(engine, name, subs, dims, old, value, quiet, out);
    }
    if let Some(Array::Typed(a)) = old.as_ref() {
        return assign_typed(engine, name, a, &dims, &subs, &value, quiet, out);
    }
    let create_char = old.is_none() && value.is_character();
    let cols = shape_product(&dims[1..])?;
    let count = if create_char {
        crate::CharacterRow::checked_len(dims[0], cols)?
    } else {
        Matrix::checked_len(dims[0], cols)?
    };
    // The result buffer must fit before the old data is converted or grown.
    engine.preflight_elements(count)?;
    let targets = shape_product(&subs.iter().map(Indices::len).collect::<Vec<_>>())?;
    let rhs: Vec<f64> = match &value {
        Value::Matrix(a) => a.data().to_vec(),
        Value::Logical(a) => a.data().iter().map(|&x| f64::from(x)).collect(),
        Value::Container(Array::Typed(a)) => (0..a.len()).map(|i| a.real(i)).collect(),
        Value::Container(Array::Character(a)) => a.units().iter().map(|&x| f64::from(x)).collect(),
        Value::Text(s) | Value::QuotedString(s) => s.encode_utf16().map(f64::from).collect(),
        _ => return Err("N-D indexed assignment requires real or logical values".into()),
    };
    if rhs.len() != 1 && rhs.len() != targets {
        return Err("Unable to perform assignment because the left and right sides have a different number of elements".into());
    }
    if rhs.len() != 1 && !subsasgn_shape_agrees(&subs, &subsasgn_rhs_dims(&value, rhs.len())) {
        return Err("Subscripted assignment dimensions must agree".into());
    }
    let old_data: Vec<f64> = match old.as_ref() {
        Some(source) if source.dims().as_ref() == dims.as_slice() => source_reals(source)?,
        Some(Array::Cell(_)) => return Err(CELL_ND.into()),
        Some(Array::Struct(_)) => return Err(STRUCT_ND.into()),
        Some(Array::Numeric(_) | Array::Logical(_) | Array::Typed(_) | Array::Character(_)) => {
            Vec::new()
        }
        Some(_) => return Err("N-D indexed assignment is unsupported for this array class".into()),
        None => Vec::new(),
    };
    // Same shape: the converted old data already is the result buffer (no scatter copy).
    let same_shape = old
        .as_ref()
        .is_some_and(|a| a.dims().as_ref() == dims.as_slice());
    let mut data = if same_shape {
        old_data
    } else {
        vec![0.; count]
    };
    if let Some(source) = old.as_ref().filter(|_| !same_shape) {
        let old_dims = source.dims();
        let mut new_strides = Vec::with_capacity(dims.len());
        let mut new_stride = 1usize;
        for &d in &dims {
            new_strides.push(new_stride);
            new_stride = new_stride.saturating_mul(d.max(1));
        }
        let old_values = source_reals(source)?;
        for (linear, value) in old_values.into_iter().enumerate() {
            if linear % 8192 == 0 {
                engine.check_cancel()?;
            }
            let mut q = linear;
            let mut destination = 0usize;
            for axis in 0..old_dims.len() {
                let coord = q % old_dims[axis].max(1);
                q /= old_dims[axis].max(1);
                destination += coord * new_strides[axis];
            }
            data[destination] = value;
        }
    }
    let mut strides = Vec::with_capacity(dims.len());
    let mut stride = 1usize;
    for &d in &dims {
        strides.push(stride);
        stride = stride.saturating_mul(d.max(1));
    }
    for linear in 0..targets {
        if linear % 8192 == 0 {
            engine.check_cancel()?;
        }
        let mut q = linear;
        let mut pos = 0;
        for (axis, sub) in subs.iter().enumerate() {
            let coord = q % sub.len().max(1);
            q /= sub.len().max(1);
            pos += sub.at(coord) * strides[axis];
        }
        data[pos] = rhs[if rhs.len() == 1 { 0 } else { linear }];
    }
    let canonical = crate::array::normalize_dims(&dims)?;
    let array = match old {
        Some(Array::Logical(_)) => Array::Logical(
            LogicalMatrix::from_column_vec(
                dims[0],
                shape_product(&dims[1..])?,
                data.into_iter().map(|x| u8::from(x != 0.)).collect(),
                &engine.cancel,
            )?
            .with_dims(&canonical)?,
        ),
        Some(Array::Typed(a)) => Array::Typed(
            crate::TypedArray::from_reals(
                a.class(),
                dims[0],
                shape_product(&dims[1..])?,
                data.into_iter(),
            )?
            .with_dims(&canonical)?,
        ),
        source if create_char || matches!(source, Some(Array::Character(_))) => Array::Character(
            crate::CharacterRow::from_column_major(
                dims[0],
                shape_product(&dims[1..])?,
                data.into_iter().map(|x| x as u16).collect(),
                &engine.cancel,
            )?
            .with_dims(&canonical)?,
        ),
        _ => Array::Numeric(
            Matrix::from_column_vec(dims[0], shape_product(&dims[1..])?, data)?
                .with_dims(&canonical)?,
        ),
    };
    engine.assign_workspace_array(name, array, quiet, out)
}

#[allow(clippy::too_many_arguments)]
fn assign_complex(
    engine: &mut Engine,
    name: &str,
    subs: Vec<Indices>,
    dims: Vec<usize>,
    old: Option<Array>,
    value: Value,
    quiet: bool,
    out: &mut Execution,
) -> Result<()> {
    let single = match &old {
        Some(Array::Complex(a)) => a.is_single(),
        Some(Array::Typed(a)) if a.class() == crate::NumericClass::Single => true,
        Some(Array::Numeric(_) | Array::Logical(_) | Array::Character(_)) => false,
        None => matches!(&value, Value::Complex(a) if a.is_single()),
        _ => {
            return Err(
                "Complex N-D assignment requires a floating-point, logical, or char target".into(),
            );
        }
    };
    let cols = shape_product(&dims[1..])?;
    let count = crate::ComplexMatrix::checked_len(dims[0], cols)?;
    engine.preflight_elements(count.saturating_mul(2))?;
    let targets = shape_product(&subs.iter().map(Indices::len).collect::<Vec<_>>())?;
    let (rhs, rhs_dims) = complex_data(&value)?;
    if rhs.len() != 1 && rhs.len() != targets {
        return Err("Unable to perform assignment because the left and right sides have a different number of elements".into());
    }
    if rhs.len() != 1 && !subsasgn_shape_agrees(&subs, &rhs_dims) {
        return Err("Subscripted assignment dimensions must agree".into());
    }
    let mut output = vec![crate::Complex64::new(0., 0.); count];
    if let Some(old) = old {
        let old_dims = old.dims();
        let mut new_strides = Vec::with_capacity(dims.len());
        let mut stride = 1usize;
        for &extent in &dims {
            new_strides.push(stride);
            stride = stride
                .checked_mul(extent.max(1))
                .ok_or("Index stride overflow")?;
        }
        for linear in 0..old.len() {
            if linear % 8192 == 0 {
                engine.check_cancel()?;
            }
            let mut q = linear;
            let mut destination = 0usize;
            for (axis, &extent) in old_dims.iter().enumerate() {
                let coord = q % extent.max(1);
                q /= extent.max(1);
                destination += coord * new_strides[axis];
            }
            output[destination] =
                super::indexed_assignment::complex_assignment_element(&old, linear);
        }
    }
    let mut strides = Vec::with_capacity(dims.len());
    let mut stride = 1usize;
    for &extent in &dims {
        strides.push(stride);
        stride = stride
            .checked_mul(extent.max(1))
            .ok_or("Index stride overflow")?;
    }
    for linear in 0..targets {
        if linear % 8192 == 0 {
            engine.check_cancel()?;
        }
        let mut q = linear;
        let mut destination = 0usize;
        for (axis, sub) in subs.iter().enumerate() {
            let coord = q % sub.len().max(1);
            q /= sub.len().max(1);
            destination += sub.at(coord) * strides[axis];
        }
        let value = rhs[if rhs.len() == 1 { 0 } else { linear }];
        output[destination] = if single {
            crate::ComplexMatrix::round_to_single(value)
        } else {
            value
        };
    }
    let canonical = crate::array::normalize_dims(&dims)?;
    let array = Array::Complex(
        crate::ComplexMatrix::from_column_vec(dims[0], cols, output, &engine.cancel)?
            .with_single_class(single)
            .with_dims(&canonical)?,
    );
    engine.assign_workspace_array(name, array, quiet, out)
}

/// `complex(a)` / `complex(a,b)` with an N-D part; a scalar part expands (MATLAB complex page).
fn complex_parts(engine: &Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err("complex expects one or two real inputs".into());
    }
    if args.iter().any(|v| matches!(v, Value::Complex(_))) {
        return Err("Input A must be real.".into());
    }
    // Like 2-D complex (typed_functions.rs): any integer or single part is rejected.
    if args.iter().any(|v| typed(v).is_some()) {
        return Err("Complex integer and complex single values are not supported yet".into());
    }
    let (re, dr, _) = real_data(&args[0])?;
    let (im, di, _) = match args.get(1) {
        Some(b) => real_data(b)?,
        None => (vec![0.], vec![1, 1], false),
    };
    let dims = if shape_product(&dr)? == 1 {
        di.clone()
    } else {
        dr.clone()
    };
    if !(dr == di || shape_product(&dr)? == 1 || shape_product(&di)? == 1) {
        return Err("Input arguments must be the same size, or one of them must be scalar.".into());
    }
    let n = shape_product(&dims)?;
    let at = |v: &[f64], i: usize| if v.len() == 1 { v[0] } else { v[i] };
    let data = (0..n)
        .map(|i| crate::Complex64::new(at(&re, i), at(&im, i)))
        .collect();
    Ok(Value::Complex(
        crate::ComplexMatrix::from_column_vec(
            dims[0],
            shape_product(&dims[1..])?,
            data,
            &engine.cancel,
        )?
        .with_dims(&dims)?,
    ))
}

pub(super) const CELL_ND: &str = "Cell arrays with more than two dimensions are not supported yet; use a cell array of N-D arrays, e.g. c{k} = A";
pub(super) const STRUCT_ND: &str = "Struct arrays with more than two dimensions are not supported yet; use a 2-D struct array whose fields hold N-D arrays";

fn source_reals(source: &Array) -> Result<Vec<f64>> {
    Ok(match source {
        Array::Numeric(a) => a.data().to_vec(),
        Array::Logical(a) => a.data().iter().map(|&x| f64::from(x)).collect(),
        Array::Typed(a) => (0..a.len()).map(|i| a.real(i)).collect(),
        Array::Character(a) => a.units().iter().map(|&x| f64::from(x)).collect(),
        _ => return Err("N-D indexed assignment is unsupported for this array class".into()),
    })
}

fn array_value(value: &Value) -> Result<Array> {
    match value {
        Value::Matrix(a) => Ok(Array::Numeric(a.clone())),
        Value::Logical(a) => Ok(Array::Logical(a.clone())),
        Value::Complex(a) => Ok(Array::Complex(a.clone())),
        Value::Container(a) => Ok(a.clone()),
        _ => Err("Expected an array".into()),
    }
}

fn value_from_array(value: Array) -> Value {
    match value {
        Array::Numeric(a) => Value::Matrix(a),
        Array::Logical(a) => Value::Logical(a),
        Array::Complex(a) => Value::Complex(a),
        other => Value::Container(other),
    }
}

pub(super) fn call(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    _out: &mut Execution,
) -> Result<Option<Value>> {
    // Image display and set(h, 'CData', rgb) take M-by-N-by-3 truecolor data;
    // rms reduces N-D input itself (stats_gaps, through nd_reduce).
    if matches!(
        name,
        "image"
            | "imagesc"
            | "imshow"
            | "implay"
            | "set"
            | "rms"
            | "vecnorm"
            | "isreal"
            | "isa"
            | "flow"
            | "smooth3"
            | "reducevolume"
            | "subvolume"
            | "volumebounds"
            | "interp3"
            | "slice"
            | "contourslice"
            | "volshow"
            | "sliceViewer"
            | "orthosliceViewer"
            | "getAxesHandles"
    ) || feval_forwards(name, args)
        || expanding_function(name)
    {
        return Ok(None);
    }
    // Two-input dense grouping reads N-D sources itself and validates its own
    // dimension list; with one input an N-D source keeps the refusal below.
    if name == "num2cell" && args.len() == 2 {
        return super::strings::dense_num2cell(engine, &args[0], &args[1]).map(Some);
    }
    validate_options(name, args)?;
    if name == "repmat" && args.len() >= 2 {
        let reps = super::shape_arguments::shape_dims(&args[1..], None)?;
        if args[0].is_nd() || reps.len() > 2 {
            return Ok(Some(repmat(engine, args)?));
        }
    }
    if name == "cat"
        // A single surviving 2-D part (including char/cell/struct) keeps HEAD's path.
        && args.iter().skip(1).filter(|v| !v.dims().contains(&0)).take(2).count() > 1
        && !args
            .iter()
            .any(|v| matches!(v, Value::Container(Array::Cell(_) | Array::Struct(_))))
        && args
            .first()
            .is_some_and(|v| matches!(v, Value::Matrix(m) if m.as_scalar().is_ok_and(|d| d > 2.)))
    {
        return Ok(Some(cat(engine, name, args)?));
    }
    if matches!(name, "cat" | "horzcat" | "vertcat") && args.iter().any(Value::is_nd) {
        return Ok(Some(cat(engine, name, args)?));
    }
    if matches!(name, "reshape" | "zeros" | "ones") && !args.is_empty()
        && !args.iter().any(|v| matches!(v, Value::Container(Array::Cell(_) | Array::Struct(_))))
        && !args.iter().any(|v| matches!(v, Value::Text(s) | Value::QuotedString(s) if s == "like" || crate::NumericClass::parse(s).is_some() || s == "double"))
        && (args.iter().any(Value::is_nd)
            || super::shape_arguments::shape_dims(
                if name == "reshape" { &args[1..] } else { args },
                if name == "reshape" { Some(shape_product(&args[0].dims())?) } else { None },
            ).is_ok_and(|dims| dims.len() > 2))
        && let Some(result) = engine.shape_call(name, args)?
    {
        return Ok(Some(result));
    }
    if name == "isequal" && args.iter().any(Value::is_nd) {
        let first = &args[0];
        let mut equal = true;
        for v in &args[1..] {
            equal = equal && v.dims() == first.dims() && values_equal(engine, first, v)?;
        }
        return Ok(Some(Value::Logical(LogicalMatrix::scalar(equal))));
    }
    if let Some((left, right, include_nan)) = extreme_pair_arguments(name, args)? {
        return extreme_pair(engine, name, left, right, include_nan).map(Some);
    }
    if nd_reduce::needs_dispatch(name, args) {
        return nd_reduce::reduce(engine, name, args, false).map(|v| Some(v.0));
    }
    if name == "complex" && args.iter().any(Value::is_nd) {
        return complex_parts(engine, args).map(Some);
    }
    if matches!(name, "permute" | "ipermute")
        && let Some(value) = super::nd_shape::permute_two_dimensional(engine, name, args)?
    {
        return Ok(Some(value));
    }
    if name == "shiftdim"
        && let Some(mut outputs) = super::nd_shape::shiftdim(engine, args)?
    {
        return Ok(Some(outputs.remove(0)));
    }
    if let Some(input) = args.first().filter(|a| a.is_nd()) {
        if let Some(class) = crate::NumericClass::parse(name) {
            return Ok(Some(Value::Container(Array::Typed(
                super::typed_functions::convert(input, class)?.with_dims(&input.dims())?,
            ))));
        }
        match name {
            "double" => {
                if let Value::Complex(a) = input {
                    return Ok(Some(Value::Complex(a.clone().with_single_class(false))));
                }
                let dims = input.dims();
                // Refuse an over-budget conversion before allocating it.
                engine.preflight_elements(dims.iter().product())?;
                let matrix = input.clone().matrix(&engine.cancel)?.with_dims(&dims)?;
                return Ok(Some(Value::Matrix(matrix)));
            }
            "char" => {
                let text = engine.character_argument(input)?.with_dims(&input.dims())?;
                return Ok(Some(Value::Container(text.into())));
            }
            "logical" => {
                return Ok(Some(Value::Logical(
                    input
                        .clone()
                        .logical(&engine.cancel)?
                        .with_dims(&input.dims())?,
                )));
            }
            "repmat" => return Ok(Some(repmat(engine, args)?)),
            "cat" | "horzcat" | "vertcat" => return Ok(Some(cat(engine, name, args)?)),
            "size" => return super::nd_shape::size(args).map(Some),
            "find" => return Ok(Some(super::nd_shape::find(engine, args, 1)?.remove(0))),
            "sort" => return Ok(Some(super::nd_shape::sort(engine, args, false)?.0)),
            "ndims" => return Ok(Some(Value::Matrix(Matrix::scalar(input.ndims() as f64)))),
            "numel" => {
                return Ok(Some(Value::Matrix(Matrix::scalar(
                    input.dims().iter().product::<usize>() as f64,
                ))));
            }
            "length" => {
                return Ok(Some(Value::Matrix(Matrix::scalar(
                    if input.dims().contains(&0) {
                        0.
                    } else {
                        input.dims().iter().copied().max().unwrap_or(0) as f64
                    },
                ))));
            }
            "isempty" => {
                return Ok(Some(Value::Logical(LogicalMatrix::scalar(
                    input.dims().iter().product::<usize>() == 0,
                ))));
            }
            "isscalar" | "isvector" | "isrow" | "iscolumn" | "ismatrix" => {
                let dims = input.dims();
                let count = dims.iter().product::<usize>();
                let result = match name {
                    "isscalar" => count == 1,
                    "isvector" => dims.iter().filter(|&&d| d != 1).count() <= 1,
                    "isrow" => dims[0] == 1 && dims[1..].iter().all(|&d| d == 1),
                    "iscolumn" => dims[0] != 1 && dims[1..].iter().all(|&d| d == 1),
                    "ismatrix" => input.ndims() == 2,
                    _ => false,
                };
                return Ok(Some(Value::Logical(LogicalMatrix::scalar(result))));
            }
            "class" => return Ok(Some(Value::Text(array_value(input)?.class().to_owned()))),
            "abs" | "sqrt" | "exp" | "log" | "sin" | "cos" | "round" | "floor" | "ceil"
            | "real" | "imag" | "conj" => return unary(name, input),
            "permute" | "ipermute" | "squeeze" => return permute(engine, name, args),
            "circshift" => return circshift(engine, args).map(Some),
            "disp" | "display" => return Ok(None),
            _ => {}
        }
    }
    if matches!(name, "true" | "false")
        && !args.is_empty()
        && super::shape_arguments::shape_dims(args, None).is_ok_and(|d| d.len() > 2)
    {
        let dims = super::shape_arguments::shape_dims(args, None)?;
        let rows = dims[0];
        let cols = shape_product(&dims[1..])?;
        let logical =
            LogicalMatrix::filled(rows, cols, name == "true", &engine.cancel)?.with_dims(&dims)?;
        return Ok(Some(Value::Logical(logical)));
    }
    if matches!(name, "nan" | "NaN" | "inf" | "Inf")
        && args
            .iter()
            .all(|a| !matches!(a, Value::Text(_) | Value::QuotedString(_)))
        && (args.is_empty() || super::shape_arguments::shape_dims(args, None).is_ok())
    {
        let dims = if args.is_empty() {
            vec![1, 1]
        } else {
            super::shape_arguments::shape_dims(args, None)?
        };
        let rows = dims[0];
        let cols = shape_product(&dims[1..])?;
        let value = if name.eq_ignore_ascii_case("nan") {
            crate::MATLAB_NAN
        } else {
            f64::INFINITY
        };
        return Ok(Some(Value::Matrix(
            Matrix::filled(rows, cols, value)?.with_dims(&dims)?,
        )));
    }
    if args.iter().any(Value::is_nd) {
        return Err(unsupported(name, args));
    }
    Ok(None)
}

pub(super) fn multi_outputs(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    mask: [bool; 3],
) -> Result<Option<Vec<Value>>> {
    validate_options(name, args)?;
    if let Some((left, right, include_nan)) = extreme_pair_arguments(name, args)? {
        if count != 1 {
            return Err(format!("{name} with two inputs supports one output only"));
        }
        return Ok(Some(vec![extreme_pair(
            engine,
            name,
            left,
            right,
            include_nan,
        )?]));
    }
    if args.iter().any(Value::is_nd) && !args.first().is_some_and(Value::is_nd) {
        return Err(unsupported(name, args));
    }
    if args.first().is_some_and(Value::is_nd) {
        match name {
            "size" => return super::nd_shape::size_outputs(args, count).map(Some),
            "find" => return super::nd_shape::find(engine, args, count).map(Some),
            "sort" => {
                let (sorted, indices) = super::nd_shape::sort(engine, args, count >= 2)?;
                let mut outputs = vec![sorted];
                outputs.extend(indices);
                outputs.truncate(count);
                return Ok(Some(outputs));
            }
            "shiftdim" => {
                if let Some(mut outputs) = super::nd_shape::shiftdim(engine, args)? {
                    outputs.truncate(count);
                    return Ok(Some(outputs));
                }
            }
            _ => {}
        }
    }
    if matches!(name, "min" | "max") && nd_reduce::needs_dispatch(name, args) {
        let (first, second) = nd_reduce::reduce(engine, name, args, count == 2 && mask[1])?;
        let empty = || Matrix::filled(0, 0, 0.).map(Value::Matrix);
        let mut outputs = vec![if mask[0] { first } else { empty()? }];
        if count == 2 {
            outputs.push(match second {
                Some(index) => index,
                None => empty()?,
            });
        }
        return Ok(Some(outputs));
    }
    Ok(None)
}

pub(super) const MTIMES_ND: &str = "Arguments must be 2-D, or at least one argument must be scalar. Use TIMES (.*) for elementwise multiplication.";
pub(super) const MRDIVIDE_ND: &str = "Arguments must be 2-D, or at least one argument must be scalar. Use RDIVIDE (./) for elementwise right division.";
pub(super) const MLDIVIDE_ND: &str = "Arguments must be 2-D, or at least one argument must be scalar. Use LDIVIDE (.\\) for elementwise left division.";
pub(super) const MPOWER_ND: &str = "Inputs must be a scalar and a square matrix. To operate on each element of the matrix individually, use POWER (.^) for elementwise power.";
pub(super) const TRANSPOSE_ND: &str = "Transpose on ND array is not defined. Use PERMUTE instead.";
pub(super) const SHORT_CIRCUIT_ND: &str =
    "Operands to the || and && operators must be convertible to logical scalar values.";

fn real_data(value: &Value) -> Result<(Vec<f64>, Vec<usize>, bool)> {
    let dims = value.dims().into_owned();
    let (data, logical) = match value {
        Value::Matrix(a) => (a.data().to_vec(), false),
        Value::Logical(a) => (a.data().iter().map(|&x| f64::from(x)).collect(), true),
        Value::Container(Array::Typed(a)) => ((0..a.len()).map(|i| a.real(i)).collect(), false),
        // Char operands take part as their UTF-16 code values (result class double).
        Value::Container(Array::Character(a)) => {
            (a.units().iter().map(|&x| f64::from(x)).collect(), false)
        }
        Value::Text(text) => {
            let data: Vec<f64> = text.encode_utf16().map(f64::from).collect();
            let n = data.len();
            return Ok((data, vec![usize::from(n > 0), n], false));
        }
        _ => return Err("N-D operation currently requires real or logical values".into()),
    };
    Ok((data, dims, logical))
}

fn unary(name: &str, value: &Value) -> Result<Option<Value>> {
    if let Value::Complex(a) = value {
        let dims = a.dims();
        let mut real = Vec::with_capacity(a.len());
        let mut complex = Vec::with_capacity(a.len());
        for &z in a.data() {
            match name {
                "real" => real.push(z.re),
                "imag" => real.push(z.im),
                "abs" => real.push(libm::hypot(z.re, z.im)),
                "conj" => complex.push(z.conj()),
                "sqrt" => complex.push(super::complex_math::complex_value(name, z)),
                "exp" => complex.push(super::complex_math::complex_value(name, z)),
                "log" => complex.push(super::complex_math::complex_value(name, z)),
                "sin" => complex.push(super::complex_math::complex_value(name, z)),
                "cos" => complex.push(super::complex_math::complex_value(name, z)),
                _ => return Err(format!("{name} does not support complex N-D arrays")),
            }
        }
        let rows = dims[0];
        let cols = shape_product(&dims[1..])?;
        if matches!(name, "real" | "imag" | "abs") {
            return Ok(Some(Value::Matrix(
                Matrix::from_column_vec(rows, cols, real)?.with_dims(&dims)?,
            )));
        }
        return Ok(Some(Value::Complex(
            crate::ComplexMatrix::from_column_vec(
                rows,
                cols,
                complex,
                &std::sync::atomic::AtomicBool::new(false),
            )?
            .with_dims(&dims)?,
        )));
    }
    if let Some(a) = typed(value)
        && a.is_integer()
    {
        if !matches!(
            name,
            "abs" | "floor" | "ceil" | "round" | "real" | "imag" | "conj"
        ) {
            return Err(format!(
                "Undefined function '{name}' for input arguments of type '{}'.",
                a.class().name()
            ));
        }
        let output = crate::TypedArray::from_ints(
            a.class(),
            a.rows(),
            a.cols(),
            (0..a.len()).map(|i| match name {
                "abs" => a.int(i).abs(),
                "imag" => 0,
                _ => a.int(i),
            }),
        )?
        .with_dims(&a.dims())?;
        return Ok(Some(Value::Container(output.into())));
    }
    let (data, dims, _) = real_data(value)?;
    if matches!(name, "sqrt" | "log") && data.iter().any(|&x| x < 0.) {
        if typed(value).is_some() {
            return Err("Complex single values are not supported yet".into());
        }
        let complex = crate::ComplexMatrix::from_column_vec(
            dims[0],
            shape_product(&dims[1..])?,
            data.iter().map(|&x| crate::Complex64::new(x, 0.)).collect(),
            &Default::default(),
        )?
        .with_dims(&dims)?;
        return unary(name, &Value::Complex(complex));
    }
    let data = data
        .into_iter()
        .map(|x| match name {
            "abs" => x.abs(),
            "sqrt" => libm::sqrt(x),
            "exp" => libm::exp(x),
            "log" => libm::log(x),
            "sin" => libm::sin(x),
            "cos" => libm::cos(x),
            "round" => libm::round(x),
            "floor" => libm::floor(x),
            "ceil" => libm::ceil(x),
            "real" | "conj" => x,
            "imag" => 0.,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    let rows = dims[0];
    let cols = shape_product(&dims[1..])?;
    if let Some(a) = typed(value) {
        return Ok(Some(Value::Container(
            crate::TypedArray::from_reals(a.class(), rows, cols, data.into_iter())?
                .with_dims(&dims)?
                .into(),
        )));
    }
    Ok(Some(Value::Matrix(
        Matrix::from_column_vec(rows, cols, data)?.with_dims(&dims)?,
    )))
}

pub(super) fn unary_operator(value: Value, negative: bool, not: bool) -> Result<Value> {
    if let Value::Complex(a) = &value {
        let dims = a.dims();
        let rows = dims[0];
        let cols = shape_product(&dims[1..])?;
        if not {
            let values = a
                .data()
                .iter()
                .map(|z| u8::from(z.re == 0. && z.im == 0.))
                .collect();
            return Ok(Value::Logical(
                LogicalMatrix::from_column_vec(
                    rows,
                    cols,
                    values,
                    &std::sync::atomic::AtomicBool::new(false),
                )?
                .with_dims(&dims)?,
            ));
        }
        let data = a
            .data()
            .iter()
            .map(|&z| if negative { -z } else { z })
            .collect();
        return Ok(Value::Complex(
            crate::ComplexMatrix::from_column_vec(
                rows,
                cols,
                data,
                &std::sync::atomic::AtomicBool::new(false),
            )?
            .with_dims(&dims)?,
        ));
    }
    if let Some(a) = typed(&value)
        && !not
    {
        if !negative {
            return Ok(value);
        }
        let output = if a.is_integer() {
            crate::TypedArray::from_ints(
                a.class(),
                a.rows(),
                a.cols(),
                (0..a.len()).map(|i| -a.int(i)),
            )?
        } else {
            crate::TypedArray::from_reals(
                a.class(),
                a.rows(),
                a.cols(),
                (0..a.len()).map(|i| -a.real(i)),
            )?
        };
        return Ok(Value::Container(output.with_dims(&a.dims())?.into()));
    }
    let (data, dims, _) = real_data(&value)?;
    let rows = dims[0];
    let cols = shape_product(&dims[1..])?;
    if not {
        let output = data.into_iter().map(|x| u8::from(x == 0.)).collect();
        return Ok(Value::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                output,
                &std::sync::atomic::AtomicBool::new(false),
            )?
            .with_dims(&dims)?,
        ));
    }
    Ok(Value::Matrix(
        Matrix::from_column_vec(
            rows,
            cols,
            data.into_iter()
                .map(|x| if negative { -x } else { x })
                .collect(),
        )?
        .with_dims(&dims)?,
    ))
}

fn repmat(engine: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err("repmat requires repetition factors".into());
    }
    if matches!(args[0], Value::Complex(_)) {
        let (source, dims) = complex_data(&args[0])?;
        let factors = super::shape_arguments::shape_dims(&args[1..], None)?;
        let rank = dims.len().max(factors.len());
        let mut out_dims = vec![1; rank];
        for (i, out_dim) in out_dims.iter_mut().enumerate() {
            *out_dim = dims
                .get(i)
                .copied()
                .unwrap_or(1)
                .checked_mul(factors.get(i).copied().unwrap_or(1))
                .ok_or("Repetition dimensions overflow")?;
        }
        let count = shape_product(&out_dims)?;
        Matrix::checked_len(out_dims[0], shape_product(&out_dims[1..])?)?;
        engine.preflight_elements(count.saturating_mul(2))?;
        let mut output = Vec::with_capacity(count);
        for linear in 0..count {
            if linear % 8192 == 0 {
                engine.check_cancel()?;
            }
            let mut q = linear;
            let mut source_linear = 0usize;
            let mut source_stride = 1usize;
            for (axis, &out_dim) in out_dims.iter().enumerate() {
                let coord = q % out_dim.max(1);
                q /= out_dim.max(1);
                let source_dim = dims.get(axis).copied().unwrap_or(1);
                if source_dim != 0 {
                    source_linear += (coord % source_dim) * source_stride;
                }
                source_stride *= source_dim.max(1);
            }
            output.push(source.get(source_linear).copied().unwrap_or_default());
        }
        return Ok(Value::Complex(
            crate::ComplexMatrix::from_column_vec(
                out_dims[0],
                shape_product(&out_dims[1..])?,
                output,
                &engine.cancel,
            )?
            .with_dims(&out_dims)?,
        ));
    }
    if let Some(a) = typed(&args[0]) {
        let dims = a.dims();
        let factors = super::shape_arguments::shape_dims(&args[1..], None)?;
        let out_dims = (0..dims.len().max(factors.len()))
            .map(|i| {
                dims.get(i)
                    .copied()
                    .unwrap_or(1)
                    .checked_mul(factors.get(i).copied().unwrap_or(1))
                    .ok_or_else(|| "Repetition dimensions overflow".to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        let count = Matrix::checked_len(out_dims[0], shape_product(&out_dims[1..])?)?;
        // Positions (8 bytes each) and the typed result.
        engine.preflight_elements(
            count.saturating_add(count.saturating_mul(a.class().size()).div_ceil(8)),
        )?;
        let mut positions = Vec::with_capacity(count);
        for mut i in 0..count {
            if i % 8192 == 0 {
                engine.check_cancel()?;
            }
            let (mut pos, mut stride) = (0, 1);
            for (axis, &d) in out_dims.iter().enumerate() {
                let n = dims.get(axis).copied().unwrap_or(1);
                pos += ((i % d.max(1)) % n.max(1)) * stride;
                i /= d.max(1);
                stride *= n.max(1);
            }
            positions.push(pos);
        }
        return Ok(Value::Container(
            a.select_with_cancel(
                out_dims[0],
                shape_product(&out_dims[1..])?,
                &positions,
                &engine.cancel,
            )?
            .with_dims(&out_dims)?
            .into(),
        ));
    }
    let (source, dims, logical) = real_data(&args[0])?;
    let factors = super::shape_arguments::shape_dims(&args[1..], None)?;
    let rank = dims.len().max(factors.len());
    let mut out_dims = vec![1; rank];
    for (i, out_dim) in out_dims.iter_mut().enumerate() {
        *out_dim = dims
            .get(i)
            .copied()
            .unwrap_or(1)
            .checked_mul(factors.get(i).copied().unwrap_or(1))
            .ok_or("Repetition dimensions overflow")?;
    }
    let count = shape_product(&out_dims)?;
    Matrix::checked_len(out_dims[0], shape_product(&out_dims[1..])?)?;
    engine.preflight_elements(count)?;
    engine.check_cancel()?;
    let mut output = Vec::with_capacity(count);
    for linear in 0..count {
        if linear % 8192 == 0 {
            engine.check_cancel()?;
        }
        let mut q = linear;
        let mut source_linear = 0;
        let mut source_stride = 1;
        for (axis, &out_dim) in out_dims.iter().enumerate() {
            let coord = q % out_dim.max(1);
            q /= out_dim.max(1);
            let source_dim = dims.get(axis).copied().unwrap_or(1);
            if source_dim != 0 {
                source_linear += (coord % source_dim) * source_stride;
            }
            source_stride *= source_dim.max(1);
        }
        output.push(source.get(source_linear).copied().unwrap_or(0.));
    }
    let rows = out_dims[0];
    let cols = shape_product(&out_dims[1..])?;
    let value = if logical {
        Value::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                output.into_iter().map(|x| u8::from(x != 0.)).collect(),
                &engine.cancel,
            )?
            .with_dims(&out_dims)?,
        )
    } else if let Value::Container(Array::Typed(input)) = &args[0] {
        Value::Container(
            crate::TypedArray::from_reals(input.class(), rows, cols, output.into_iter())?
                .with_dims(&out_dims)?
                .into(),
        )
    } else {
        Value::Matrix(Matrix::from_column_vec(rows, cols, output)?.with_dims(&out_dims)?)
    };
    Ok(value)
}

fn cat(engine: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    let (dim, parts) = if name == "cat" {
        let dim = args
            .first()
            .ok_or("cat requires a dimension")?
            .clone()
            .matrix(&engine.cancel)?
            .as_scalar()?;
        if !dim.is_finite() || dim.fract() != 0. || !(1. ..=32.).contains(&dim) {
            return Err("Dimension must be a positive integer".into());
        }
        (dim as usize - 1, &args[1..])
    } else if name == "vertcat" {
        (0, args)
    } else {
        (1, args)
    };
    if parts.is_empty() {
        return Err("cat requires input arrays".into());
    }
    // Empty parts do not add elements, but retain the existing class precedence.
    let typed_class = parts
        .iter()
        .filter_map(typed)
        .find(|t| t.is_integer())
        .or_else(|| parts.iter().find_map(typed))
        .map(crate::TypedArray::class);
    let nonempty = parts
        .iter()
        .filter(|v| !v.dims().contains(&0))
        .cloned()
        .collect::<Vec<_>>();
    let parts = if nonempty.is_empty() {
        parts
    } else {
        &nonempty
    };
    if parts.iter().any(|v| matches!(v, Value::Complex(_))) {
        return cat_complex(engine, dim, parts);
    }
    if let Some(class) = typed_class {
        return cat_typed(engine, dim, parts, class);
    }
    let arrays = parts.iter().map(real_data).collect::<Result<Vec<_>>>()?;
    let rank = arrays
        .iter()
        .map(|a| a.1.len())
        .max()
        .unwrap_or(2)
        .max(dim + 1);
    let mut out_dims = vec![1; rank];
    for (axis, out_dim) in out_dims.iter_mut().enumerate() {
        if axis == dim {
            *out_dim = arrays
                .iter()
                .try_fold(0usize, |sum, a| {
                    sum.checked_add(a.1.get(axis).copied().unwrap_or(1))
                })
                .ok_or("Concatenation dimension overflow")?;
        } else {
            let expected = arrays[0].1.get(axis).copied().unwrap_or(1);
            if arrays
                .iter()
                .any(|a| a.1.get(axis).copied().unwrap_or(1) != expected)
            {
                return Err("Dimensions of arrays being concatenated are not consistent".into());
            }
            *out_dim = expected;
        }
    }
    let output_len = shape_product(&out_dims)?;
    Matrix::checked_len(out_dims[0], shape_product(&out_dims[1..])?)?;
    let mut output = vec![0.; output_len];
    let mut before = 0;
    let mut stride = 1usize;
    for &extent in out_dims.iter().take(dim) {
        stride *= extent;
    }
    for (source, source_dims, _) in &arrays {
        let extent = source_dims.get(dim).copied().unwrap_or(1);
        let after = if dim + 1 >= rank {
            1
        } else {
            shape_product(&out_dims[dim + 1..])?
        };
        let source_before = shape_product(&source_dims[..dim.min(source_dims.len())])?;
        for outer in 0..after {
            if outer % 8192 == 0 {
                engine.check_cancel()?;
            }
            let dest_first = outer * out_dims[dim] * stride + before * stride;
            let src_first = outer * extent * source_before;
            let copy_len = extent * stride;
            output[dest_first..dest_first + copy_len]
                .copy_from_slice(&source[src_first..src_first + copy_len]);
        }
        before += extent;
    }
    let rows = out_dims[0];
    let cols = shape_product(&out_dims[1..])?;
    let logical = parts.iter().all(|v| matches!(v, Value::Logical(_)));
    let typed = parts
        .first()
        .and_then(|v| match v {
            Value::Container(Array::Typed(a)) => Some(a.class()),
            _ => None,
        })
        .filter(|class| {
            parts
                .iter()
                .all(|v| matches!(v, Value::Container(Array::Typed(a)) if a.class() == *class))
        });
    if logical {
        Ok(Value::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                output.into_iter().map(|x| u8::from(x != 0.)).collect(),
                &engine.cancel,
            )?
            .with_dims(&out_dims)?,
        ))
    } else if let Some(class) = typed {
        Ok(Value::Container(
            crate::TypedArray::from_reals(class, rows, cols, output.into_iter())?
                .with_dims(&out_dims)?
                .into(),
        ))
    } else {
        Ok(Value::Matrix(
            Matrix::from_column_vec(rows, cols, output)?.with_dims(&out_dims)?,
        ))
    }
}

fn cat_complex(engine: &Engine, dim: usize, parts: &[Value]) -> Result<Value> {
    let arrays = parts.iter().map(complex_data).collect::<Result<Vec<_>>>()?;
    let rank = arrays
        .iter()
        .map(|array| array.1.len())
        .max()
        .unwrap_or(2)
        .max(dim + 1);
    let mut out_dims = vec![1; rank];
    for (axis, out_dim) in out_dims.iter_mut().enumerate() {
        if axis == dim {
            *out_dim = arrays
                .iter()
                .try_fold(0usize, |sum, array| {
                    sum.checked_add(array.1.get(axis).copied().unwrap_or(1))
                })
                .ok_or("Concatenation dimension overflow")?;
        } else {
            let expected = arrays[0].1.get(axis).copied().unwrap_or(1);
            if arrays
                .iter()
                .any(|array| array.1.get(axis).copied().unwrap_or(1) != expected)
            {
                return Err("Dimensions of arrays being concatenated are not consistent".into());
            }
            *out_dim = expected;
        }
    }
    let output_len = shape_product(&out_dims)?;
    Matrix::checked_len(out_dims[0], shape_product(&out_dims[1..])?)?;
    let mut output = vec![crate::Complex64::new(0., 0.); output_len];
    let mut before = 0usize;
    let mut stride = 1usize;
    for &extent in out_dims.iter().take(dim) {
        stride *= extent;
    }
    for (source, source_dims) in &arrays {
        let extent = source_dims.get(dim).copied().unwrap_or(1);
        let after = if dim + 1 >= rank {
            1
        } else {
            shape_product(&out_dims[dim + 1..])?
        };
        let source_before = shape_product(&source_dims[..dim.min(source_dims.len())])?;
        for outer in 0..after {
            if outer % 8192 == 0 {
                engine.check_cancel()?;
            }
            let dest_first = outer * out_dims[dim] * stride + before * stride;
            let src_first = outer * extent * source_before;
            let copy_len = extent * stride;
            output[dest_first..dest_first + copy_len]
                .copy_from_slice(&source[src_first..src_first + copy_len]);
        }
        before += extent;
    }
    Ok(Value::Complex(
        crate::ComplexMatrix::from_column_vec(
            out_dims[0],
            shape_product(&out_dims[1..])?,
            output,
            &engine.cancel,
        )?
        .with_dims(&out_dims)?,
    ))
}

fn circshift(engine: &Engine, args: &[Value]) -> Result<Value> {
    if !(2..=3).contains(&args.len()) {
        return Err("circshift expects 2 or 3 arguments".into());
    }
    let amount = engine.real_arg("circshift", &args[1])?;
    if amount
        .data()
        .iter()
        .any(|v| !v.is_finite() || v.fract() != 0.)
    {
        return Err("circshift: shifts must be finite integers".into());
    }
    let input = &args[0];
    let dims = input.dims().into_owned();
    let mut shifts = vec![0usize; dims.len()];
    let reduce = |v: f64, len: usize| -> usize {
        if len == 0 {
            0
        } else {
            v.rem_euclid(len as f64) as usize
        }
    };
    if let Some(dim_arg) = args.get(2) {
        if amount.len() != 1 {
            return Err("circshift: a dimension cannot be combined with a vector of shifts".into());
        }
        let d = engine
            .real_arg("circshift", dim_arg)?
            .as_scalar()
            .map_err(|_| "circshift: the dimension must be a positive integer scalar".to_owned())?;
        if !d.is_finite() || d.fract() != 0. || !(1. ..=1e9).contains(&d) {
            return Err("circshift: the dimension must be a positive integer scalar".into());
        }
        let dim = d as usize;
        if dim <= dims.len() {
            shifts[dim - 1] = reduce(amount.data()[0], dims[dim - 1]);
        }
    } else if amount.len() == 1 {
        let dim = dims.iter().position(|&d| d != 1).unwrap_or(0);
        shifts[dim] = reduce(amount.data()[0], dims[dim]);
    } else if amount.is_empty() {
        return Err("circshift: the shift must be a nonempty scalar or vector".into());
    } else if amount.rows() != 1 && amount.cols() != 1 {
        return Err("circshift: the shift must be a scalar or a vector".into());
    } else {
        for (dim, &v) in amount.data().iter().enumerate() {
            if dim < dims.len() {
                shifts[dim] = reduce(v, dims[dim]);
            }
        }
    }
    let count = shape_product(&dims)?;
    if count == 0 || shifts.iter().all(|&s| s == 0) {
        return Ok(input.clone());
    }
    engine.preflight_elements(count)?;
    let mut positions = Vec::with_capacity(count);
    for dst in 0..count {
        if dst % 8192 == 0 {
            engine.check_cancel()?;
        }
        let mut q = dst;
        let mut src = 0usize;
        let mut stride = 1usize;
        for (&d, &s) in dims.iter().zip(&shifts) {
            let coord = q % d;
            q /= d;
            let src_coord = (coord + d - s) % d;
            src += src_coord * stride;
            stride *= d;
        }
        positions.push(src);
    }
    let rows = dims[0];
    let cols = shape_product(&dims[1..])?;
    Ok(match input {
        Value::Matrix(a) => {
            let data = positions.iter().map(|&i| a.data()[i]).collect();
            Value::Matrix(Matrix::from_column_vec(rows, cols, data)?.with_dims(&dims)?)
        }
        Value::Logical(a) => {
            let data = positions.iter().map(|&i| a.data()[i]).collect();
            Value::Logical(
                LogicalMatrix::from_column_vec(rows, cols, data, &engine.cancel)?
                    .with_dims(&dims)?,
            )
        }
        Value::Complex(a) => {
            let data = positions.iter().map(|&i| a.data()[i]).collect();
            Value::Complex(
                crate::ComplexMatrix::from_column_vec(rows, cols, data, &engine.cancel)?
                    .with_dims(&dims)?,
            )
        }
        Value::Container(Array::Typed(a)) => Value::Container(Array::Typed(
            a.select_with_cancel(rows, cols, &positions, &engine.cancel)?
                .with_dims(&dims)?,
        )),
        Value::Container(Array::Character(a)) => {
            let data = positions.iter().map(|&i| a.units()[i]).collect();
            Value::Container(
                crate::CharacterRow::from_column_major(rows, cols, data, &engine.cancel)?
                    .with_dims(&dims)?
                    .into(),
            )
        }
        _ => return Err("circshift does not support this N-D array class".into()),
    })
}

fn permute(engine: &Engine, name: &str, args: &[Value]) -> Result<Option<Value>> {
    let input = &args[0];
    let dims = input.dims().into_owned();
    if name == "squeeze" {
        let mut out_dims: Vec<_> = dims.iter().copied().filter(|&d| d != 1).collect();
        while out_dims.len() < 2 {
            out_dims.push(1);
        }
        let rows = out_dims[0];
        let cols = shape_product(&out_dims[1..])?;
        let squeezed = match input {
            Value::Matrix(a) => Array::Numeric(a.reshape(rows, cols)?.with_dims(&out_dims)?),
            Value::Logical(a) => Array::Logical(a.reshape(rows, cols)?.with_dims(&out_dims)?),
            Value::Complex(a) => Array::Complex(
                a.reshape(rows, cols, &engine.cancel)?
                    .with_dims(&out_dims)?,
            ),
            Value::Container(Array::Typed(a)) => {
                Array::Typed(a.reshaped(rows, cols)?.with_dims(&out_dims)?)
            }
            Value::Container(Array::Character(a)) => Array::Character(
                a.reshape(rows, cols, &engine.cancel)?
                    .with_dims(&out_dims)?,
            ),
            _ => return Err("squeeze does not support this N-D array class".into()),
        };
        return Ok(Some(value_from_array(squeezed)));
    }
    let order: Vec<usize> = if name == "squeeze" {
        dims.iter()
            .enumerate()
            .filter_map(|(i, &d)| (d != 1 || dims.len() <= 2 && i < 2).then_some(i))
            .collect()
    } else {
        let order = args[1].clone().matrix(&engine.cancel)?;
        let mut order = order
            .data()
            .iter()
            .map(|&x| {
                if !x.is_finite() || x.fract() != 0. || !(1. ..=32.).contains(&x) {
                    Err("Invalid permutation order".into())
                } else {
                    Ok(x as usize - 1)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        if name == "ipermute" {
            let mut inverse = vec![0; order.len()];
            for (i, &axis) in order.iter().enumerate() {
                if axis >= inverse.len() {
                    return Err("ORDER must be a permutation".into());
                }
                inverse[axis] = i;
            }
            order = inverse;
        }
        order
    };
    if order.len() < 2 || order.len() > 32 {
        return Err("ORDER must be a vector with at least two elements".into());
    }
    let mut seen = vec![false; order.len()];
    for &axis in &order {
        if axis >= seen.len() || seen[axis] {
            return Err("ORDER must be a permutation".into());
        }
        seen[axis] = true;
    }
    if order.len() < dims.len() {
        return Err("ORDER must contain one entry for every array dimension".into());
    }
    let mut extended = dims.clone();
    extended.resize(order.len(), 1);
    let out_dims = order.iter().map(|&i| extended[i]).collect::<Vec<_>>();
    let mut positions = Vec::with_capacity(shape_product(&out_dims)?);
    let mut coords = vec![0usize; out_dims.len()];
    for linear in 0..shape_product(&out_dims)? {
        if linear % 8192 == 0 {
            engine.check_cancel()?;
        }
        let mut q = linear;
        for (axis, &d) in out_dims.iter().enumerate() {
            coords[axis] = q % d.max(1);
            q /= d.max(1);
        }
        let mut source_linear = 0;
        let mut stride = 1usize;
        for (axis, &extent) in extended.iter().enumerate() {
            let target_axis = order.iter().position(|&x| x == axis).unwrap();
            source_linear += coords[target_axis] * stride;
            stride *= extent;
        }
        positions.push(source_linear);
    }
    let out_dims = crate::array::normalize_dims(&out_dims)?;
    let rows = out_dims[0];
    let cols = shape_product(&out_dims[1..])?;
    let output = match input {
        Value::Matrix(a) => Array::Numeric(
            Matrix::from_column_vec(rows, cols, positions.iter().map(|&i| a.data()[i]).collect())?
                .with_dims(&out_dims)?,
        ),
        Value::Logical(a) => Array::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                positions.iter().map(|&i| a.data()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&out_dims)?,
        ),
        Value::Complex(a) => Array::Complex(
            crate::ComplexMatrix::from_column_vec(
                rows,
                cols,
                positions.iter().map(|&i| a.data()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&out_dims)?,
        ),
        Value::Container(Array::Typed(a)) => Array::Typed(
            a.select_with_cancel(rows, cols, &positions, &engine.cancel)?
                .with_dims(&out_dims)?,
        ),
        Value::Container(Array::Character(a)) => Array::Character(
            crate::CharacterRow::from_column_major(
                rows,
                cols,
                positions.iter().map(|&i| a.units()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&out_dims)?,
        ),
        _ => return Err("permute does not support this N-D array class".into()),
    };
    Ok(Some(value_from_array(output)))
}

struct Expansion<'a> {
    dims: Vec<usize>,
    cols: usize,
    count: usize,
    left: Option<&'a [usize]>,
    right: Option<&'a [usize]>,
}

impl<'a> Expansion<'a> {
    fn new(left: &'a [usize], right: &'a [usize]) -> Result<Self> {
        let dims = expansion_dims(left, right)?;
        let cols = shape_product(&dims[1..])?;
        let count = Matrix::checked_len(dims[0], cols)?;
        Ok(Self {
            left: (left != dims).then_some(left),
            right: (right != dims).then_some(right),
            dims,
            cols,
            count,
        })
    }

    fn indices(&self, k: usize) -> (usize, usize) {
        let at = |input: Option<&[usize]>| input.map_or(k, |d| expanded_index(k, &self.dims, d));
        (at(self.left), at(self.right))
    }

    fn evaluate<T>(
        &self,
        cancel: &AtomicBool,
        f: impl Fn(usize, usize) -> Result<T>,
    ) -> Result<Vec<T>> {
        crate::dsp::check_cancel(cancel)?;
        let mut output = Vec::with_capacity(self.count);
        for first in (0..self.count).step_by(8192) {
            crate::dsp::check_cancel(cancel)?;
            for k in first..self.count.min(first + 8192) {
                let (i, j) = self.indices(k);
                output.push(f(i, j)?);
            }
        }
        crate::dsp::check_cancel(cancel)?;
        Ok(output)
    }
}

fn numeric_dims(value: &Value) -> std::borrow::Cow<'_, [usize]> {
    if matches!(value, Value::Text(text) if text.is_empty()) {
        return std::borrow::Cow::Borrowed(&[0, 0]);
    }
    value.dims()
}

pub(super) fn binary(
    engine: &Engine,
    op: &Token,
    left: &Value,
    right: &Value,
) -> Result<Option<Value>> {
    if !left.is_nd() && !right.is_nd() {
        return Ok(None);
    }
    let (da, db) = (numeric_dims(left), numeric_dims(right));
    let scalar = |dims: &[usize]| shape_product(dims).is_ok_and(|n| n == 1);
    // Matrix operators with a scalar operand are elementwise (MATLAB mtimes,
    // mrdivide and mldivide pages); any other N-D operand is an error there.
    let (op, left, right, da, db) = match op {
        Token::Star if scalar(&da) || scalar(&db) => (&Token::DotStar, left, right, &da, &db),
        Token::Slash if scalar(&db) => (&Token::DotSlash, left, right, &da, &db),
        Token::Backslash if scalar(&da) => (&Token::DotSlash, right, left, &db, &da),
        Token::DotBackslash => (&Token::DotSlash, right, left, &db, &da),
        Token::Star => return Err(MTIMES_ND.into()),
        Token::Slash => return Err(MRDIVIDE_ND.into()),
        Token::Backslash => return Err(MLDIVIDE_ND.into()),
        Token::Power => return Err(MPOWER_ND.into()),
        Token::Transpose | Token::NonConjugateTranspose => return Err(TRANSPOSE_ND.into()),
        Token::Plus | Token::Minus | Token::DotStar | Token::DotSlash | Token::DotPower => {
            (op, left, right, &da, &db)
        }
        _ => return Err("Unsupported operator for N-D arrays".into()),
    };
    let complex = matches!(left, Value::Complex(_)) || matches!(right, Value::Complex(_));
    if !complex && (typed(left).is_some() || typed(right).is_some()) {
        return binary_typed(engine, op, left, right, da, db).map(Some);
    }
    let source = |v| {
        nd_reduce::source(v).map_err(|_| {
            "N-D arithmetic requires numeric, logical, char or complex arrays".to_owned()
        })
    };
    let (a, b) = (source(left)?, source(right)?);
    let expanded = Expansion::new(da, db)?;
    if complex {
        return binary_complex(engine, op, &expanded, &a, &b).map(Some);
    }
    engine.preflight_elements(expanded.count)?;
    if op == &Token::DotPower {
        for first in (0..expanded.count).step_by(8192) {
            engine.check_cancel()?;
            if (first..expanded.count.min(first + 8192)).any(|k| {
                let (i, j) = expanded.indices(k);
                super::complex_math::real_power_is_complex(a.real(i), b.real(j))
            }) {
                return binary_complex(engine, op, &expanded, &a, &b).map(Some);
            }
        }
    }
    let data = expanded.evaluate(&engine.cancel, |i, j| {
        let (x, y) = (a.real(i), b.real(j));
        Ok(match op {
            Token::Plus => x + y,
            Token::Minus => x - y,
            Token::DotStar => x * y,
            Token::DotSlash => x / y,
            Token::DotPower => libm::pow(x, y),
            _ => unreachable!(),
        })
    })?;
    nd_reduce::real_out(&expanded.dims, data).map(Some)
}

pub(super) fn everyday_binary(
    engine: &Engine,
    name: &str,
    left: &Value,
    right: &Value,
    f: fn(f64, f64) -> f64,
) -> Result<Value> {
    let source = |v| match nd_reduce::source(v) {
        Ok(nd_reduce::Source::Complex(_)) => {
            Err(format!("{name}: complex input is not supported yet"))
        }
        Ok(source) => Ok(source),
        Err(_) => Err(format!("{name}: input must be numeric or logical")),
    };
    let (a, b) = (source(left)?, source(right)?);
    let (da, db) = (numeric_dims(left), numeric_dims(right));
    let expanded = Expansion::new(&da, &db)?;
    engine.preflight_elements(expanded.count)?;
    let data = expanded.evaluate(&engine.cancel, |i, j| Ok(f(a.real(i), b.real(j))))?;
    nd_reduce::real_out(&expanded.dims, data)
}

fn complex_data(value: &Value) -> Result<(Vec<crate::Complex64>, Vec<usize>)> {
    let dims = value.dims().into_owned();
    let data = match value {
        Value::Complex(a) => a.data().to_vec(),
        Value::Matrix(a) => a
            .data()
            .iter()
            .map(|&x| crate::Complex64::new(x, 0.))
            .collect(),
        Value::Logical(a) => a
            .data()
            .iter()
            .map(|&x| crate::Complex64::new(f64::from(x), 0.))
            .collect(),
        Value::Container(Array::Typed(a)) => (0..a.len())
            .map(|i| crate::Complex64::new(a.real(i), 0.))
            .collect(),
        _ => return Err("N-D arithmetic requires numeric, logical or complex arrays".into()),
    };
    Ok((data, dims))
}

fn binary_complex(
    engine: &Engine,
    op: &Token,
    expanded: &Expansion<'_>,
    a: &nd_reduce::Source<'_>,
    b: &nd_reduce::Source<'_>,
) -> Result<Value> {
    let rows = expanded.dims[0];
    ComplexMatrix::checked_len(rows, expanded.cols)?;
    engine.preflight_elements(expanded.count.saturating_mul(2))?;
    let output = expanded.evaluate(&engine.cancel, |i, j| {
        let (x, y) = (a.complex(i), b.complex(j));
        Ok(match op {
            Token::Plus => x + y,
            Token::Minus => x - y,
            Token::DotStar => x * y,
            Token::DotSlash => x / y,
            Token::DotPower => super::complex_math::cpow(x, y),
            _ => unreachable!(),
        })
    })?;
    Ok(Value::Complex(
        ComplexMatrix::from_column_vec(rows, expanded.cols, output, &engine.cancel)?
            .with_dims(&expanded.dims)?,
    ))
}

pub(super) fn logical_binary(
    op: &Token,
    left: &Value,
    right: &Value,
    cancel: &AtomicBool,
) -> Result<Value> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let source = |v| {
        nd_reduce::source(v).map_err(|_| {
            "N-D comparison requires numeric, logical, char or complex arrays".to_owned()
        })
    };
    let (a, b) = (source(left)?, source(right)?);
    let complex =
        matches!(a, nd_reduce::Source::Complex(_)) || matches!(b, nd_reduce::Source::Complex(_));
    if complex && !matches!(op, Token::Equal | Token::NotEqual | Token::And | Token::Or) {
        return Err("Relational comparisons are not defined for complex N-D arrays".into());
    }
    let (da, db) = (numeric_dims(left), numeric_dims(right));
    let expanded = Expansion::new(&da, &db)?;
    let result = expanded.evaluate(cancel, |i, j| {
        let value = if complex {
            let (x, y) = (a.complex(i), b.complex(j));
            let nonzero = |z: crate::Complex64| z.re != 0. || z.im != 0.;
            match op {
                Token::Equal => x == y,
                Token::NotEqual => x != y,
                Token::And => nonzero(x) && nonzero(y),
                Token::Or => nonzero(x) || nonzero(y),
                _ => unreachable!(),
            }
        } else {
            match op {
                Token::And => a.real(i) != 0. && b.real(j) != 0.,
                Token::Or => a.real(i) != 0. || b.real(j) != 0.,
                _ => {
                    let order = value_order(left, i, right, j)?;
                    match op {
                        Token::Equal => order == Some(Equal),
                        Token::NotEqual => order != Some(Equal),
                        Token::Less => order == Some(Less),
                        Token::Greater => order == Some(Greater),
                        Token::LessEqual => matches!(order, Some(Less | Equal)),
                        Token::GreaterEqual => matches!(order, Some(Greater | Equal)),
                        _ => return Err("Unsupported N-D comparison".into()),
                    }
                }
            }
        };
        Ok(u8::from(value))
    })?;
    Ok(Value::Logical(
        LogicalMatrix::from_column_vec(expanded.dims[0], expanded.cols, result, cancel)?
            .with_dims(&expanded.dims)?,
    ))
}

fn delete(
    engine: &mut Engine,
    name: &str,
    effective: &[usize],
    subs: &[Indices],
    old: Option<Array>,
    quiet: bool,
    out: &mut Execution,
) -> Result<()> {
    let source = old.ok_or("Cannot delete from an undefined array")?;
    let axis = if subs.len() == 1 {
        0
    } else {
        let axes = subs
            .iter()
            .enumerate()
            .filter(|(i, s)| s.len() != effective[*i] || (0..s.len()).any(|j| s.at(j) != j))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if axes.len() != 1 {
            return Err("Deletion must remove elements along exactly one dimension".into());
        }
        axes[0]
    };
    let mut removed = (0..subs[axis].len())
        .map(|i| subs[axis].at(i))
        .collect::<Vec<_>>();
    if removed.iter().any(|&i| i >= effective[axis]) {
        return Err("Index exceeds matrix dimensions".into());
    }
    removed.sort_unstable();
    removed.dedup();
    let mut dims = effective.to_vec();
    dims[axis] -= removed.len();
    if dims.len() == 1 {
        dims = vec![1, dims[0]];
    }
    let stride = shape_product(&effective[..axis])?;
    let mut positions = Vec::with_capacity(shape_product(&dims)?);
    for i in 0..source.len() {
        if i % 8192 == 0 {
            engine.check_cancel()?;
        }
        if removed
            .binary_search(&((i / stride) % effective[axis]))
            .is_err()
        {
            positions.push(i);
        }
    }
    let rows = dims[0];
    let cols = shape_product(&dims[1..])?;
    let output = match source {
        Array::Numeric(a) => Array::Numeric(
            Matrix::from_column_vec(rows, cols, positions.iter().map(|&i| a.data()[i]).collect())?
                .with_dims(&dims)?,
        ),
        Array::Logical(a) => Array::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                positions.iter().map(|&i| a.data()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&dims)?,
        ),
        Array::Complex(a) => Array::Complex(
            crate::ComplexMatrix::from_column_vec(
                rows,
                cols,
                positions.iter().map(|&i| a.data()[i]).collect(),
                &engine.cancel,
            )?
            .with_single_class(a.is_single())
            .with_dims(&dims)?,
        ),
        Array::Typed(a) => Array::Typed(
            a.select_with_cancel(rows, cols, &positions, &engine.cancel)?
                .with_dims(&dims)?,
        ),
        Array::Character(a) => Array::Character(
            crate::CharacterRow::from_column_major(
                rows,
                cols,
                positions.iter().map(|&i| a.units()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&dims)?,
        ),
        _ => return Err("N-D deletion does not support this array class".into()),
    };
    engine.assign_workspace_array(name, output, quiet, out)
}

pub(super) fn literal(engine: &Engine, rows: Vec<Vec<Value>>) -> Result<Value> {
    let mut parts = Vec::with_capacity(rows.len());
    for row in rows {
        parts.push(cat(engine, "horzcat", &row)?);
    }
    cat(engine, "vertcat", &parts)
}

/// Two-array extrema validation and dispatch for N-D or flagged pairs.
fn extreme_pair_arguments<'a>(
    name: &str,
    args: &'a [Value],
) -> Result<Option<(&'a Value, &'a Value, bool)>> {
    if !matches!(name, "min" | "max") {
        return Ok(None);
    }
    let (left, right, flag) = match args {
        [left, right] if right.is_character() || args.iter().any(Value::is_nd) => {
            (left, right, None)
        }
        [left, right, flag] if !nd_reduce::is_empty_placeholder(right) => (left, right, Some(flag)),
        _ => return Ok(None),
    };
    if nd_reduce::text_option(right)
        .as_deref()
        .and_then(nd_reduce::missing_flag)
        .is_some()
    {
        return Err(format!(
            "{name} with reduction options requires an empty second argument."
        ));
    }
    if right.is_character() {
        return Err(format!("{name} does not accept a character second input"));
    }
    let include_nan = match flag {
        None => false,
        Some(flag) => !nd_reduce::text_option(flag)
            .as_deref()
            .and_then(nd_reduce::missing_flag)
            .ok_or_else(|| {
                format!("{name} with two inputs accepts only a NaN flag as third argument")
            })?,
    };
    Ok(Some((left, right, include_nan)))
}

fn extreme_pair(
    engine: &Engine,
    name: &str,
    left: &Value,
    right: &Value,
    include_nan: bool,
) -> Result<Value> {
    let (da, db) = (left.dims(), right.dims());
    let dims = expansion_dims(&da, &db).map_err(|_| "Array dimensions must agree")?;
    let cols = shape_product(&dims[1..])?;
    let count = Matrix::checked_len(dims[0], cols)?;
    engine.preflight_elements(count)?;
    let converted = if typed(left).is_some() || typed(right).is_some() {
        let class = typed_binary_class(left, right)?;
        Some((
            class,
            Value::Container(super::typed_functions::convert(left, class)?.into()),
            Value::Container(super::typed_functions::convert(right, class)?.into()),
        ))
    } else {
        None
    };
    let (class, left, right) = converted
        .as_ref()
        .map_or((None, left, right), |(class, a, b)| (Some(*class), a, b));
    let (a, b) = (nd_reduce::source(left)?, nd_reduce::source(right)?);
    let kind = if name == "min" {
        crate::matrix::Extreme::Min
    } else {
        crate::matrix::Extreme::Max
    };
    let integer = if let (nd_reduce::Source::Int(a), nd_reduce::Source::Int(b)) = (&a, &b) {
        Some((a, b))
    } else {
        None
    };
    let complex =
        matches!(a, nd_reduce::Source::Complex(_)) || matches!(b, nd_reduce::Source::Complex(_));
    let logical = matches!(
        (&a, &b),
        (nd_reduce::Source::Bits(_), nd_reduce::Source::Bits(_))
    );
    let same_shape = da == db;
    let choose = |i: usize| {
        let (ia, ib) = if same_shape {
            (i, i)
        } else {
            (expanded_index(i, &dims, &da), expanded_index(i, &dims, &db))
        };
        let (nan_a, nan_b) = (nd_reduce::is_nan_at(&a, ia), nd_reduce::is_nan_at(&b, ib));
        let rhs = if nan_a || nan_b {
            if include_nan { !nan_a } else { nan_a && !nan_b }
        } else if let Some((a, b)) = integer {
            if name == "min" {
                b.int(ib) < a.int(ia)
            } else {
                b.int(ib) > a.int(ia)
            }
        } else if complex {
            crate::matrix::complex_pair_picks_second(kind, a.complex(ia), b.complex(ib))
        } else if name == "min" {
            b.real(ib) < a.real(ia)
        } else {
            b.real(ib) > a.real(ia)
        };
        if rhs { (&b, ib) } else { (&a, ia) }
    };
    let collect = |i| -> Result<_> {
        if i % 8192 == 0 {
            engine.check_cancel()?;
        }
        Ok(choose(i))
    };
    if complex {
        let data = (0..count)
            .map(|i| collect(i).map(|(src, j)| src.complex(j)))
            .collect::<Result<Vec<_>>>()?;
        return nd_reduce::complex_out(engine, &dims, data);
    }
    if integer.is_none() {
        let data = (0..count)
            .map(|i| collect(i).map(|(src, j)| src.real(j)))
            .collect::<Result<Vec<_>>>()?;
        if let Some(class) = class {
            return nd_reduce::typed_out(class, &dims, data);
        }
        if logical {
            return nd_reduce::logical_out(
                engine,
                &dims,
                data.into_iter().map(|x| u8::from(x != 0.)).collect(),
            );
        }
        return nd_reduce::real_out(&dims, data);
    }
    let data = (0..count)
        .map(|i| {
            collect(i).map(|(src, j)| {
                let nd_reduce::Source::Int(a) = src else {
                    unreachable!()
                };
                a.int(j)
            })
        })
        .collect::<Result<Vec<_>>>()?;
    nd_reduce::int_out(class.expect("integer class"), &dims, data)
}

fn expansion_dims(a: &[usize], b: &[usize]) -> Result<Vec<usize>> {
    (0..a.len().max(b.len()))
        .map(|axis| {
            let (x, y) = (
                a.get(axis).copied().unwrap_or(1),
                b.get(axis).copied().unwrap_or(1),
            );
            crate::matrix::expansion_extent(x, y)
        })
        .collect()
}

fn expanded_index(mut i: usize, output: &[usize], input: &[usize]) -> usize {
    let (mut position, mut stride) = (0, 1);
    for (axis, &d) in output.iter().enumerate() {
        let coord = i % d.max(1);
        i /= d.max(1);
        let n = input.get(axis).copied().unwrap_or(1);
        if n != 1 {
            position += coord * stride;
        }
        stride *= n.max(1);
    }
    position
}

/// isequal ignores class (MATLAB isequal page): values compare numerically, integers
/// exactly against doubles, complex parts pairwise; NaN is never equal.
fn values_equal(engine: &Engine, a: &Value, b: &Value) -> Result<bool> {
    let n = shape_product(&a.dims())?;
    let complex = matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_));
    for i in 0..n {
        if i % 8192 == 0 {
            engine.check_cancel()?;
        }
        let same = if complex {
            let part = |v: &Value| -> Result<crate::Complex64> {
                Ok(match v {
                    Value::Complex(c) => c.data()[i],
                    other => crate::Complex64::new(scalar_real(other, i)?, 0.),
                })
            };
            part(a)? == part(b)?
        } else {
            match (a, b) {
                (
                    Value::Matrix(_)
                    | Value::Logical(_)
                    | Value::Container(Array::Typed(_) | Array::Character(_)),
                    Value::Matrix(_)
                    | Value::Logical(_)
                    | Value::Container(Array::Typed(_) | Array::Character(_)),
                ) => value_order(a, i, b, i)? == Some(std::cmp::Ordering::Equal),
                _ => return Ok(false),
            }
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn typed(value: &Value) -> Option<&crate::TypedArray> {
    if let Value::Container(Array::Typed(a)) = value {
        Some(a)
    } else {
        None
    }
}

fn scalar_real(value: &Value, i: usize) -> Result<f64> {
    match value {
        Value::Matrix(a) => Ok(a.data()[i]),
        Value::Logical(a) => Ok(f64::from(a.data()[i])),
        Value::Container(Array::Typed(a)) => Ok(a.real(i)),
        Value::Container(Array::Character(a)) => Ok(f64::from(a.units()[i])),
        _ => Err("Expected numeric or logical array".into()),
    }
}

fn value_order(a: &Value, i: usize, b: &Value, j: usize) -> Result<Option<std::cmp::Ordering>> {
    let (ta, tb) = (
        typed(a).filter(|t| t.is_integer()),
        typed(b).filter(|t| t.is_integer()),
    );
    Ok(match (ta, tb) {
        (Some(x), Some(y)) => Some(x.int(i).cmp(&y.int(j))),
        (Some(x), None) => super::typed_values::int_vs_real(x.int(i), scalar_real(b, j)?),
        (None, Some(y)) => super::typed_values::int_vs_real(y.int(j), scalar_real(a, i)?)
            .map(std::cmp::Ordering::reverse),
        (None, None) => scalar_real(a, i)?.partial_cmp(&scalar_real(b, j)?),
    })
}

fn typed_binary_class(a: &Value, b: &Value) -> Result<crate::NumericClass> {
    let class = typed(a)
        .or_else(|| typed(b))
        .expect("typed operand")
        .class();
    for v in [a, b] {
        if let Some(t) = typed(v) {
            if t.class() != class {
                return Err("Integers can only be combined with integers of the same class, or scalar doubles.".into());
            }
        } else if class.is_integer()
            && (!matches!(v, Value::Matrix(_) | Value::Container(Array::Character(_)))
                || shape_product(&v.dims())? != 1)
        {
            return Err(
                "Integers can only be combined with integers of the same class, or scalar doubles."
                    .into(),
            );
        }
    }
    Ok(class)
}

fn binary_typed(
    engine: &Engine,
    op: &Token,
    a: &Value,
    b: &Value,
    da: &[usize],
    db: &[usize],
) -> Result<Value> {
    let class = typed_binary_class(a, b)?;
    let expanded = Expansion::new(da, db)?;
    engine.preflight_elements(expanded.count.saturating_mul(if class.is_integer() {
        2
    } else {
        1
    }))?;
    let code = match op {
        Token::Plus => '+',
        Token::Minus => '-',
        Token::DotStar => '*',
        Token::DotSlash => '/',
        Token::DotPower => '^',
        _ => return Err("Unsupported typed N-D arithmetic".into()),
    };
    if class.is_integer() {
        let side = |v, i| -> Result<(Option<i128>, f64)> {
            Ok(if let Some(t) = typed(v) {
                (Some(t.int(i)), 0.)
            } else {
                (None, scalar_real(v, i)?)
            })
        };
        let ints = expanded.evaluate(&engine.cancel, |i, j| {
            super::typed_values::int_element(class, code, side(a, i)?, side(b, j)?)
        })?;
        return nd_reduce::int_out(class, &expanded.dims, ints);
    }
    let values = expanded.evaluate(&engine.cancel, |i, j| {
        let (x, y) = (
            f64::from(scalar_real(a, i)? as f32),
            f64::from(scalar_real(b, j)? as f32),
        );
        Ok(match code {
            '+' => x + y,
            '-' => x - y,
            '*' => x * y,
            '/' => x / y,
            '^' => libm::pow(x, y),
            _ => unreachable!(),
        })
    })?;
    nd_reduce::typed_out(class, &expanded.dims, values)
}

#[allow(clippy::too_many_arguments)]
fn assign_typed(
    engine: &mut Engine,
    name: &str,
    source: &crate::TypedArray,
    dims: &[usize],
    subs: &[Indices],
    value: &Value,
    quiet: bool,
    out: &mut Execution,
) -> Result<()> {
    let rows = dims[0];
    let cols = shape_product(&dims[1..])?;
    let total = Matrix::checked_len(rows, cols)?;
    engine.preflight_elements(total.saturating_mul(source.class().size()).div_ceil(8))?;
    let rhs = super::typed_functions::convert(value, source.class())?;
    let count = shape_product(&subs.iter().map(Indices::len).collect::<Vec<_>>())?;
    if rhs.len() != 1 && rhs.len() != count {
        return Err("Unable to perform assignment because the left and right sides have a different number of elements".into());
    }
    if rhs.len() != 1 && !subsasgn_shape_agrees(subs, value.dims().as_ref()) {
        return Err("Subscripted assignment dimensions must agree".into());
    }
    let old_dims = source.dims();
    let mut result = if old_dims.as_ref() == dims {
        source.clone()
    } else {
        source.remap(
            rows,
            cols,
            |mut i| {
                let (mut source_i, mut stride) = (0, 1);
                for (axis, &d) in dims.iter().enumerate() {
                    let c = i % d.max(1);
                    i /= d.max(1);
                    let old = old_dims.get(axis).copied().unwrap_or(1);
                    if c >= old {
                        return None;
                    }
                    source_i += c * stride;
                    stride *= old.max(1);
                }
                Some(source_i)
            },
            &engine.cancel,
        )?
    };
    let strides = (0..dims.len())
        .map(|i| shape_product(&dims[..i]))
        .collect::<Result<Vec<_>>>()?;
    result.write_indices(
        count,
        |mut i| {
            let mut pos = 0;
            for (axis, sub) in subs.iter().enumerate() {
                let c = i % sub.len().max(1);
                i /= sub.len().max(1);
                pos += sub.at(c) * strides[axis];
            }
            pos
        },
        &rhs,
        false,
        &engine.cancel,
    )?;
    engine.assign_workspace_array(name, result.with_dims(dims)?.into(), quiet, out)
}

fn cat_typed(
    engine: &Engine,
    dim: usize,
    parts: &[Value],
    class: crate::NumericClass,
) -> Result<Value> {
    let arrays = parts
        .iter()
        .map(|v| super::typed_functions::convert(v, class))
        .collect::<Result<Vec<_>>>()?;
    let rank = arrays
        .iter()
        .map(|a| a.ndims())
        .max()
        .unwrap_or(2)
        .max(dim + 1);
    let mut dims = vec![1; rank];
    for (axis, d) in dims.iter_mut().enumerate() {
        if axis == dim {
            *d = arrays
                .iter()
                .try_fold(0usize, |n, a| {
                    n.checked_add(a.dims().get(axis).copied().unwrap_or(1))
                })
                .ok_or("Concatenation dimension overflow")?;
        } else {
            *d = arrays[0].dims().get(axis).copied().unwrap_or(1);
            if arrays
                .iter()
                .any(|a| a.dims().get(axis).copied().unwrap_or(1) != *d)
            {
                return Err("Dimensions of arrays being concatenated are not consistent".into());
            }
        }
    }
    let cols = shape_product(&dims[1..])?;
    Matrix::checked_len(dims[0], cols)?;
    let mut output = crate::TypedArray::from_ints(
        class,
        dims[0],
        cols,
        std::iter::repeat_n(0, shape_product(&dims)?),
    )?;
    let stride = shape_product(&dims[..dim])?;
    let mut offset = 0;
    for array in &arrays {
        let extent = array.dims().get(dim).copied().unwrap_or(1);
        let lane = stride
            .checked_mul(extent)
            .ok_or("Concatenation size overflow")?;
        output.write_indices(
            array.len(),
            |i| (i / lane) * dims[dim] * stride + offset * stride + i % lane,
            array,
            false,
            &engine.cancel,
        )?;
        offset += extent;
    }
    Ok(Value::Container(output.with_dims(&dims)?.into()))
}

fn validate_options(name: &str, args: &[Value]) -> Result<()> {
    if !args.iter().any(Value::is_nd) || nd_reduce::handles(name) {
        return Ok(());
    }
    if !matches!(
        name,
        "cat" | "horzcat" | "vertcat" | "isequal" | "sort" | "find"
    ) && args.iter().skip(1).any(|v| {
        matches!(
            v,
            Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_))
        )
    }) {
        return Err(unsupported(name, args));
    }
    let arity = if matches!(name, "permute" | "ipermute") {
        Some(2)
    } else if matches!(
        name,
        "ndims"
            | "numel"
            | "length"
            | "isempty"
            | "isscalar"
            | "isvector"
            | "ismatrix"
            | "isrow"
            | "iscolumn"
            | "class"
            | "squeeze"
            | "abs"
            | "sqrt"
            | "exp"
            | "log"
            | "sin"
            | "cos"
            | "round"
            | "floor"
            | "ceil"
            | "real"
            | "imag"
            | "conj"
            | "logical"
            | "double"
            | "single"
            | "int8"
            | "uint8"
            | "int16"
            | "uint16"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
    ) {
        Some(1)
    } else {
        None
    };
    if arity.is_some_and(|n| args.len() != n) {
        return Err(unsupported(name, args));
    }
    Ok(())
}
