//! N-D shape functions (part 2): size forms, permute/ipermute from a 2-D input,
//! shiftdim, meshgrid/ndgrid 3-D and N-D grids, find and sort along any dimension.
//! Behaviour follows the published MATLAB reference pages; the tests' expected
//! values come from NumPy (tool/refs/nd/part2.py).
use super::nd::shape_product;
use super::{Engine, Value};
use crate::{Array, Complex64, LogicalMatrix, Matrix, Result};

fn size_text(dims: &[usize]) -> String {
    dims.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("x")
}

fn row(values: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        1,
        values.len(),
        values,
    )?))
}

/// Dimension numbers from `size(A,dim)`, `size(A,vecdim)` or `size(A,d1,d2,...)`.
fn size_dimensions(args: &[Value]) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for value in args {
        let numbers: Vec<f64> = match value {
            Value::Matrix(m) => m.data().to_vec(),
            Value::Container(Array::Typed(t)) => (0..t.len()).map(|i| t.real(i)).collect(),
            Value::Logical(_) => {
                return Err("Dimension argument must be a positive integer scalar or a vector of positive integers.".into());
            }
            _ => return Err("size expects an array and optional dimension arguments".into()),
        };
        if args.len() > 1 && numbers.len() != 1 {
            return Err("Dimension arguments must be scalars when more than one is given.".into());
        }
        for x in numbers {
            if !x.is_finite() || x < 1. || x.fract() != 0. || x > 1_000_000. {
                return Err("Dimension argument must be a positive integer scalar or a vector of positive integers.".into());
            }
            out.push(x as usize - 1);
        }
    }
    if out.is_empty() {
        return Err("Dimension argument must be a positive integer scalar or a vector of positive integers.".into());
    }
    Ok(out)
}

/// `size(A)`, `size(A,dim)`, `size(A,vecdim)` and `size(A,d1,d2,...)` for one output.
pub(super) fn size(args: &[Value]) -> Result<Value> {
    let dims = args[0].dims();
    if args.len() == 1 {
        return row(dims.iter().map(|&d| d as f64).collect());
    }
    let selected = size_dimensions(&args[1..])?;
    let values = selected
        .iter()
        .map(|&d| dims.get(d).copied().unwrap_or(1) as f64)
        .collect::<Vec<_>>();
    if values.len() == 1 {
        return Ok(Value::Matrix(Matrix::scalar(values[0])));
    }
    row(values)
}

/// `[d1,...,dn] = size(A, ...)`: without dimension arguments the last output folds
/// the remaining dimensions; with them every output receives one requested size.
pub(super) fn size_outputs(args: &[Value], count: usize) -> Result<Vec<Value>> {
    let dims = args[0].dims();
    if args.len() == 1 {
        let mut outputs = Vec::with_capacity(count);
        for i in 0..count {
            let n = if i + 1 == count {
                shape_product(dims.get(i..).unwrap_or(&[]))?
            } else {
                dims.get(i).copied().unwrap_or(1)
            };
            outputs.push(Value::Matrix(Matrix::scalar(n as f64)));
        }
        return Ok(outputs);
    }
    let selected = size_dimensions(&args[1..])?;
    if selected.len() != count {
        return Err(
            "Number of output arguments must equal the number of requested dimensions.".into(),
        );
    }
    Ok(selected
        .iter()
        .map(|&d| Value::Matrix(Matrix::scalar(dims.get(d).copied().unwrap_or(1) as f64)))
        .collect())
}

/// Elements of `value` at `positions`, shaped as `dims`; the class is preserved.
pub(super) fn gather(
    engine: &Engine,
    value: &Value,
    positions: &[usize],
    dims: &[usize],
) -> Result<Value> {
    let dims = crate::array::normalize_dims(dims)?;
    let (rows, cols) = (dims[0], shape_product(&dims[1..])?);
    Ok(match value {
        Value::Matrix(a) => Value::Matrix(
            Matrix::from_column_vec(rows, cols, positions.iter().map(|&i| a.data()[i]).collect())?
                .with_dims(&dims)?,
        ),
        Value::Logical(a) => Value::Logical(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                positions.iter().map(|&i| a.data()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&dims)?,
        ),
        Value::Complex(a) => Value::Complex(
            crate::ComplexMatrix::from_column_vec(
                rows,
                cols,
                positions.iter().map(|&i| a.data()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&dims)?,
        ),
        Value::Container(Array::Typed(a)) => Value::Container(
            a.select_with_cancel(rows, cols, positions, &engine.cancel)?
                .with_dims(&dims)?
                .into(),
        ),
        Value::Container(Array::Character(a)) => Value::Container(
            crate::CharacterRow::from_column_major(
                rows,
                cols,
                positions.iter().map(|&i| a.units()[i]).collect(),
                &engine.cancel,
            )?
            .with_dims(&dims)?
            .into(),
        ),
        Value::Text(text) => {
            let units: Vec<u16> = text.encode_utf16().collect();
            Value::Container(
                crate::CharacterRow::from_column_major(
                    rows,
                    cols,
                    positions.iter().map(|&i| units[i]).collect(),
                    &engine.cancel,
                )?
                .with_dims(&dims)?
                .into(),
            )
        }
        Value::Container(Array::Cell(_) | Array::Struct(_)) => {
            return Err(format!(
                "Cell and struct arrays do not support N-D shapes yet (size {})",
                size_text(&dims)
            ));
        }
        _ => return Err("This N-D operation requires a numeric, logical or char array".into()),
    })
}

/// Positions of a dimension permutation (`out[k] = in[order[k]]`).
fn permuted_positions(
    engine: &Engine,
    dims: &[usize],
    order: &[usize],
) -> Result<(Vec<usize>, Vec<usize>)> {
    let mut extended = dims.to_vec();
    extended.resize(order.len().max(dims.len()), 1);
    let out_dims = order.iter().map(|&i| extended[i]).collect::<Vec<_>>();
    let mut strides = Vec::with_capacity(extended.len());
    let mut stride = 1usize;
    for &d in &extended {
        strides.push(stride);
        stride = stride.saturating_mul(d.max(1));
    }
    let count = shape_product(&out_dims)?;
    let mut positions = Vec::with_capacity(count);
    for linear in 0..count {
        if linear % 8192 == 0 {
            engine.check_cancel()?;
        }
        let (mut q, mut source) = (linear, 0usize);
        for (axis, &d) in out_dims.iter().enumerate() {
            source += (q % d.max(1)) * strides[order[axis]];
            q /= d.max(1);
        }
        positions.push(source);
    }
    Ok((positions, out_dims))
}

fn permutation_order(engine: &Engine, name: &str, value: &Value) -> Result<Vec<usize>> {
    let order = value.clone().matrix(&engine.cancel)?;
    let mut axes = Vec::with_capacity(order.len());
    for &x in order.data() {
        if !x.is_finite() || x.fract() != 0. || !(1. ..=32.).contains(&x) {
            return Err("ORDER contains an invalid permutation index.".into());
        }
        axes.push(x as usize - 1);
    }
    let mut seen = vec![false; axes.len()];
    for &k in &axes {
        if k >= axes.len() {
            return Err("ORDER contains an invalid permutation index.".into());
        }
        if seen[k] {
            return Err("ORDER cannot contain repeated permutation indices.".into());
        }
        seen[k] = true;
    }
    if name == "ipermute" {
        let mut inverse = vec![0; axes.len()];
        for (i, &k) in axes.iter().enumerate() {
            inverse[k] = i;
        }
        axes = inverse;
    }
    Ok(axes)
}

/// permute/ipermute of a 2-D input whose result has more than two dimensions
/// (e.g. `permute(ones(2,3),[3 1 2])` is 1x2x3). Other 2-D cases keep HEAD's path.
pub(super) fn permute_two_dimensional(
    engine: &Engine,
    name: &str,
    args: &[Value],
) -> Result<Option<Value>> {
    if args.len() != 2 || args[0].is_nd() || !matches!(name, "permute" | "ipermute") {
        return Ok(None);
    }
    if matches!(
        args[0],
        Value::Container(Array::Cell(_) | Array::Struct(_) | Array::Str(_))
    ) || super::string_class::is_string(&args[0])
    {
        return Ok(None);
    }
    let Ok(order) = permutation_order(engine, name, &args[1]) else {
        return Ok(None);
    };
    if order.len() <= 2 {
        return Ok(None);
    }
    let dims = args[0].dims().into_owned();
    let (positions, out_dims) = permuted_positions(engine, &dims, &order)?;
    if crate::array::normalize_dims(&out_dims)?.len() <= 2 {
        return Ok(None);
    }
    gather(engine, &args[0], &positions, &out_dims).map(Some)
}

/// `[B,n] = shiftdim(A)`, `B = shiftdim(A,n)` (MATLAB shiftdim page): a positive n
/// rotates the dimensions left by mod(n, ndims), a negative n prepends |n| singleton
/// dimensions, and without n leading singleton dimensions are removed.
/// Returns `None` for 2-D cases whose result is 2-D (HEAD's path keeps them).
pub(super) fn shiftdim(engine: &Engine, args: &[Value]) -> Result<Option<Vec<Value>>> {
    if args.is_empty() || args.len() > 2 {
        return Ok(None);
    }
    let input = &args[0];
    let dims = input.dims().into_owned();
    let n = match args.get(1) {
        None => None,
        Some(v) => {
            let x = v.clone().matrix(&engine.cancel)?.as_scalar()?;
            if !x.is_finite() || x.fract() != 0. || x.abs() > 32. {
                return Err("N must be a bounded integer.".into());
            }
            Some(x as isize)
        }
    };
    if !input.is_nd() && !n.is_some_and(|n| n < -1 || (n == -1 && dims[1] != 1)) {
        return Ok(None);
    }
    let (out_dims, order, shift) = match n {
        None => {
            let lead = dims.iter().take_while(|&&d| d == 1).count();
            let lead = if lead == dims.len() { 0 } else { lead };
            let order = (lead..dims.len()).chain(0..lead).collect::<Vec<_>>();
            let out = order.iter().map(|&i| dims[i]).collect::<Vec<_>>();
            (out, Some(order), lead as f64)
        }
        Some(n) if n >= 0 => {
            let k = n as usize % dims.len();
            let order = (k..dims.len()).chain(0..k).collect::<Vec<_>>();
            let out = order.iter().map(|&i| dims[i]).collect::<Vec<_>>();
            (out, Some(order), n as f64)
        }
        Some(n) => {
            let mut out = vec![1usize; n.unsigned_abs()];
            out.extend_from_slice(&dims);
            (out, None, n as f64)
        }
    };
    let value = match order {
        Some(order) if order.iter().enumerate().any(|(i, &k)| i != k) => {
            let (positions, out_dims) = permuted_positions(engine, &dims, &order)?;
            gather(engine, input, &positions, &out_dims)?
        }
        _ => {
            let count = shape_product(&dims)?;
            gather(engine, input, &(0..count).collect::<Vec<_>>(), &out_dims)?
        }
    };
    Ok(Some(vec![value, Value::Matrix(Matrix::scalar(shift))]))
}

fn grid_vector(engine: &Engine, name: &str, value: &Value) -> Result<Vec<f64>> {
    if matches!(value, Value::Complex(_)) {
        return Err(format!(
            "{name}: complex inputs are not supported for 3-D grids yet"
        ));
    }
    let m = value.clone().matrix(&engine.cancel)?;
    if m.is_nd() || (m.rows() > 1 && m.cols() > 1) {
        return Err(format!("{name}: grid vectors must be vectors"));
    }
    Ok(m.data().to_vec())
}

/// meshgrid(x,y,z) / ndgrid(x1,...,xn) with three or more dimensions (MATLAB pages):
/// meshgrid outputs are ny-by-nx-by-nz; ndgrid output k varies along dimension k.
pub(super) fn grid(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    let meshgrid = name == "meshgrid";
    let rank = if meshgrid { 3 } else { args.len().max(count) };
    if meshgrid && (args.len() == 2 || args.len() > 3) {
        return Err("meshgrid: the 3-D form needs one or three grid vectors".into());
    }
    if !meshgrid && args.len() != 1 && args.len() < count {
        return Err("ndgrid: more outputs than grid vectors".into());
    }
    if count > rank {
        return Err(format!("{name}: more outputs than grid dimensions"));
    }
    if rank > 32 {
        return Err(format!("{name}: at most 32 dimensions are supported"));
    }
    let vectors = (0..rank)
        .map(|k| grid_vector(engine, name, &args[if args.len() == 1 { 0 } else { k }]))
        .collect::<Result<Vec<_>>>()?;
    let mut dims = vectors.iter().map(Vec::len).collect::<Vec<_>>();
    // Output k varies along `axis[k]`.
    let axis: Vec<usize> = if meshgrid {
        dims.swap(0, 1);
        vec![1, 0, 2]
    } else {
        (0..rank).collect()
    };
    let total = shape_product(&dims)?;
    let cols = shape_product(&dims[1..])?;
    engine.preflight_elements(Matrix::checked_len(dims[0], cols)?.saturating_mul(count.max(1)))?;
    let canonical = crate::array::normalize_dims(&dims)?;
    let mut outputs = Vec::with_capacity(count.max(1));
    for k in 0..count.max(1) {
        let a = axis[k];
        let stride = shape_product(&dims[..a])?;
        let extent = dims[a];
        let mut data = Vec::with_capacity(total);
        for i in 0..total {
            if i % 8192 == 0 {
                engine.check_cancel()?;
            }
            data.push(vectors[k][(i / stride.max(1)) % extent.max(1)]);
        }
        outputs.push(Value::Matrix(
            Matrix::from_column_vec(dims[0], cols, data)?.with_dims(&canonical)?,
        ));
    }
    Ok(outputs)
}

fn nonzero(value: &Value, i: usize) -> Result<bool> {
    Ok(match value {
        Value::Matrix(a) => a.data()[i] != 0.,
        Value::Logical(a) => a.data()[i] != 0,
        Value::Complex(a) => a.data()[i].re != 0. || a.data()[i].im != 0.,
        Value::Container(Array::Typed(a)) => a.is_nonzero(i),
        Value::Container(Array::Character(a)) => a.units()[i] != 0,
        _ => return Err("find requires a numeric, logical or char array".into()),
    })
}

/// `find(A)`, `find(A,k)`, `find(A,k,'first'|'last')`, `[r,c] = find(...)` and
/// `[r,c,v] = find(...)` on N-D input: linear indices in column-major order;
/// `c` folds the trailing dimensions (MATLAB find page).
pub(super) fn find(engine: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if args.is_empty() || args.len() > 3 {
        return Err("find expects one to three arguments".into());
    }
    let input = &args[0];
    let n = shape_product(&input.dims())?;
    let limit = match args.get(1) {
        None => None,
        Some(v) => {
            let k = v.clone().matrix(&engine.cancel)?.as_scalar()?;
            if !(k >= 1. && k.fract() == 0. || k == f64::INFINITY) {
                return Err("Second argument must be a positive scalar integer.".into());
            }
            Some(if k.is_infinite() {
                usize::MAX
            } else {
                k as usize
            })
        }
    };
    let last = match args.get(2) {
        None => false,
        Some(v) => match super::nd_reduce::text_option(v).as_deref() {
            Some("first") => false,
            Some("last") => true,
            _ => return Err("Invalid search direction: use 'first' or 'last'.".into()),
        },
    };
    let mut found = Vec::new();
    let take = limit.unwrap_or(usize::MAX);
    if last {
        for i in (0..n).rev() {
            if i % 8192 == 0 {
                engine.check_cancel()?;
            }
            if found.len() >= take {
                break;
            }
            if nonzero(input, i)? {
                found.push(i);
            }
        }
        found.reverse();
    } else {
        for i in 0..n {
            if i % 8192 == 0 {
                engine.check_cancel()?;
            }
            if found.len() >= take {
                break;
            }
            if nonzero(input, i)? {
                found.push(i);
            }
        }
    }
    let rows = input.dims()[0].max(1);
    let column =
        |values: Vec<f64>| Matrix::from_column_vec(values.len(), 1, values).map(Value::Matrix);
    if count <= 1 {
        return Ok(vec![column(
            found.iter().map(|&i| (i + 1) as f64).collect(),
        )?]);
    }
    let mut outputs = vec![
        column(found.iter().map(|&i| (i % rows + 1) as f64).collect())?,
        column(found.iter().map(|&i| (i / rows + 1) as f64).collect())?,
    ];
    if count >= 3 {
        outputs.push(gather(engine, input, &found, &[found.len(), 1])?);
    }
    Ok(outputs)
}

/// Sort keys: NaN sorts last in ascending and first in descending order; complex
/// values sort by magnitude, then phase angle (MATLAB sort page). Stable.
pub(super) fn sort(
    engine: &Engine,
    args: &[Value],
    want_index: bool,
) -> Result<(Value, Option<Value>)> {
    let input = &args[0];
    let dims = input.dims().into_owned();
    let mut axis = dims.iter().position(|&d| d != 1).unwrap_or(0);
    let mut descend = false;
    for value in &args[1..] {
        if let Some(text) = super::nd_reduce::text_option(value) {
            match text.as_str() {
                "ascend" => descend = false,
                "descend" => descend = true,
                _ => return Err(format!("Invalid sort option '{text}'")),
            }
        } else {
            let d = value.clone().matrix(&engine.cancel)?.as_scalar()?;
            if !d.is_finite() || d < 1. || d.fract() != 0. || d > 32. {
                return Err(
                    "Dimension argument must be a positive integer scalar within indexing range."
                        .into(),
                );
            }
            axis = d as usize - 1;
        }
    }
    let extent = dims.get(axis).copied().unwrap_or(1);
    let stride = shape_product(&dims[..axis.min(dims.len())])?;
    let total = shape_product(&dims)?;
    let lanes = total.checked_div(extent).unwrap_or(0);
    #[derive(Clone, Copy)]
    enum Key {
        Int(i128),
        Real(f64),
        Complex(f64, f64),
    }
    let key = |i: usize| -> Result<Key> {
        Ok(match input {
            Value::Complex(a) => {
                let z: Complex64 = a.data()[i];
                if z.re.is_nan() || z.im.is_nan() {
                    Key::Real(f64::NAN)
                } else {
                    Key::Complex(libm::hypot(z.re, z.im), libm::atan2(z.im, z.re))
                }
            }
            Value::Container(Array::Typed(a)) if a.is_integer() => Key::Int(a.int(i)),
            Value::Matrix(a) => Key::Real(a.data()[i]),
            Value::Logical(a) => Key::Real(f64::from(a.data()[i])),
            Value::Container(Array::Typed(a)) => Key::Real(a.real(i)),
            Value::Container(Array::Character(a)) => Key::Real(f64::from(a.units()[i])),
            _ => return Err("sort requires a numeric, logical, char or complex array".into()),
        })
    };
    let is_nan = |k: &Key| matches!(k, Key::Real(x) if x.is_nan());
    let order = |a: &Key, b: &Key| -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match (is_nan(a), is_nan(b)) {
            (true, true) => return Equal,
            (true, false) => return if descend { Less } else { Greater },
            (false, true) => return if descend { Greater } else { Less },
            _ => {}
        }
        let o = match (a, b) {
            (Key::Int(x), Key::Int(y)) => x.cmp(y),
            (Key::Complex(m1, p1), Key::Complex(m2, p2)) => m1
                .partial_cmp(m2)
                .unwrap_or(Equal)
                .then(p1.partial_cmp(p2).unwrap_or(Equal)),
            (Key::Real(x), Key::Real(y)) => x.partial_cmp(y).unwrap_or(Equal),
            _ => Equal,
        };
        if descend { o.reverse() } else { o }
    };
    let mut positions = vec![0usize; total];
    let mut indices = if want_index {
        vec![0.; total]
    } else {
        Vec::new()
    };
    let mut lane = Vec::with_capacity(extent);
    for l in 0..lanes {
        if l % 1024 == 0 {
            engine.check_cancel()?;
        }
        let base = (l / stride.max(1)) * stride * extent + l % stride.max(1);
        lane.clear();
        for j in 0..extent {
            lane.push((j, key(base + j * stride)?));
        }
        lane.sort_by(|a, b| order(&a.1, &b.1));
        for (j, &(source, _)) in lane.iter().enumerate() {
            positions[base + j * stride] = base + source * stride;
            if want_index {
                indices[base + j * stride] = (source + 1) as f64;
            }
        }
    }
    let sorted = gather(engine, input, &positions, &dims)?;
    let indices = if want_index {
        let canonical = crate::array::normalize_dims(&dims)?;
        Some(Value::Matrix(
            Matrix::from_column_vec(canonical[0], shape_product(&canonical[1..])?, indices)?
                .with_dims(&canonical)?,
        ))
    } else {
        None
    };
    Ok((sorted, indices))
}
