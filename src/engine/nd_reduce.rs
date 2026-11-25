//! Shared dimension and flag handling for array reductions.
use super::Engine;
use super::Value;
use super::cplx_reduce::{self, Key};
use super::nd::shape_product;
use crate::matrix::{Dispersion, MedianScratch, dispersion_lane};
use crate::{Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix, NumericClass, Result};
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutType {
    Default,
    Double,
    Native,
}

struct Options {
    /// Reduced axes, zero-based, sorted and unique (cumulative functions: exactly one).
    axes: Vec<usize>,
    omit_nan: bool,
    out: OutType,
    linear: bool,
    reverse: bool,
    population: bool,
    explicit_axis: bool,
}

/// Borrowed element access: no copy of the source data.
pub(super) enum Source<'a> {
    Real(&'a [f64]),
    Bits(&'a [u8]),
    Units(&'a [u16]),
    Text(Vec<u16>),
    Single(&'a crate::TypedArray),
    Int(&'a crate::TypedArray),
    Complex(&'a [Complex64]),
}

impl Source<'_> {
    pub(super) fn real(&self, i: usize) -> f64 {
        match self {
            Self::Real(d) => d[i],
            Self::Bits(d) => f64::from(d[i]),
            Self::Units(d) => f64::from(d[i]),
            Self::Text(d) => f64::from(d[i]),
            Self::Single(a) | Self::Int(a) => a.real(i),
            Self::Complex(d) => d[i].re,
        }
    }
    pub(super) fn complex(&self, i: usize) -> Complex64 {
        match self {
            Self::Complex(d) => d[i],
            other => Complex64::new(other.real(i), 0.),
        }
    }
}

pub(super) fn text_option(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.to_ascii_lowercase()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()).to_ascii_lowercase())
        }
        _ => None,
    }
}

pub(super) fn source(value: &Value) -> Result<Source<'_>> {
    Ok(match value {
        Value::Matrix(a) => Source::Real(a.data()),
        Value::Text(text) => Source::Text(text.encode_utf16().collect()),
        Value::Logical(a) => Source::Bits(a.data()),
        Value::Complex(a) => Source::Complex(a.data()),
        Value::Container(Array::Character(a)) => Source::Units(a.units()),
        Value::Container(Array::Typed(a)) if a.is_integer() => Source::Int(a),
        Value::Container(Array::Typed(a)) => Source::Single(a),
        _ => return Err("Reductions require numeric, logical, char or complex arrays".into()),
    })
}

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "sum"
            | "prod"
            | "mean"
            | "any"
            | "all"
            | "max"
            | "min"
            | "median"
            | "std"
            | "var"
            | "mode"
            | "range"
            | "cumsum"
            | "cumprod"
            | "cummax"
            | "cummin"
    )
}

fn is_cumulative(name: &str) -> bool {
    matches!(name, "cumsum" | "cumprod" | "cummax" | "cummin")
}

pub(super) fn is_empty_placeholder(value: &Value) -> bool {
    matches!(value, Value::Matrix(a) if a.is_empty())
        || matches!(value, Value::Logical(a) if a.is_empty())
        || matches!(value, Value::Container(Array::Typed(a)) if a.is_empty())
}

pub(super) fn missing_flag(text: &str) -> Option<bool> {
    match text {
        "omitnan" | "omitmissing" => Some(true),
        "includenan" | "includemissing" => Some(false),
        _ => None,
    }
}

// Common 2-D forms keep their specialized matrix kernels. The shared path
// handles N-D input and syntax that those kernels cannot parse.
pub(super) fn needs_dispatch(name: &str, args: &[Value]) -> bool {
    if !handles(name) || args.is_empty() {
        return false;
    }
    if (name == "median" && matches!(&args[0], Value::Logical(_)))
        || (name == "sum"
            && matches!(&args[0], Value::Container(Array::Typed(a)) if a.class() == NumericClass::Single))
        || args.iter().any(Value::is_nd)
        || matches!(name, "cummax" | "cummin")
    {
        return true;
    }
    let extras = match name {
        "max" | "min" => match args.get(1) {
            Some(value) if is_empty_placeholder(value) && args.len() > 2 => &args[2..],
            Some(value)
                if value.is_character()
                    || text_option(value)
                        .as_deref()
                        .and_then(missing_flag)
                        .is_some() =>
            {
                return true;
            }
            _ => return args.len() > 2,
        },
        "std" | "var"
            if args
                .get(1)
                .is_some_and(|value| text_option(value).is_none()) =>
        {
            &args[2..]
        }
        "mode" | "range"
            if matches!(
                &args[0],
                Value::Matrix(_)
                    | Value::Complex(_)
                    | Value::Logical(_)
                    | Value::Text(_)
                    | Value::Container(Array::Typed(_) | Array::Character(_))
            ) =>
        {
            return true;
        }
        _ => &args[1..],
    };
    match extras {
        [] => false,
        [Value::Matrix(a)] => a.len() > 1,
        _ => true,
    }
}

fn dimension_list(value: &Value, single: bool) -> Result<Vec<usize>> {
    let message = if single {
        "Dimension argument must be a positive integer scalar within indexing range."
    } else {
        "Dimension argument must be a positive integer scalar or a vector of unique positive integers."
    };
    let src = match value {
        Value::Matrix(_) | Value::Container(Array::Typed(_)) => source(value)?,
        _ => return Err(message.into()),
    };
    let dims = value.dims();
    let len = shape_product(&dims)?;
    if len == 0
        || len > 32
        || (single && len != 1)
        || dims.len() != 2
        || (dims[0] != 1 && dims[1] != 1)
    {
        return Err(message.into());
    }
    let mut axes = Vec::with_capacity(len);
    for i in 0..len {
        let x = src.real(i);
        if !x.is_finite() || x < 1. || x.fract() != 0. || x >= usize::MAX as f64 {
            return Err(message.into());
        }
        axes.push(x as usize - 1);
    }
    axes.sort_unstable();
    if axes.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("Vector of dimensions must not contain repeated elements.".into());
    }
    Ok(axes)
}

fn parse(name: &str, args: &[Value], dims: &[usize]) -> Result<Options> {
    let extreme = matches!(name, "min" | "max");
    let cumulative = is_cumulative(name);
    let mut options = Options {
        axes: Vec::new(),
        omit_nan: extreme || matches!(name, "any" | "range" | "mode" | "cummax" | "cummin"),
        out: OutType::Default,
        linear: false,
        reverse: false,
        population: false,
        explicit_axis: false,
    };
    let mut rest = &args[1..];
    if extreme && let Some(placeholder) = rest.first() {
        if !is_empty_placeholder(placeholder) {
            return Err(format!(
                "{name} with reduction options requires an empty second argument."
            ));
        }
        rest = &rest[1..];
    }
    let dispersion = matches!(name, "std" | "var");
    let weight = rest
        .first()
        .filter(|v| dispersion && text_option(v).is_none());
    if let Some(weight) = weight {
        options.population = match weight {
            v if is_empty_placeholder(v) => false,
            Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_))
                if shape_product(&weight.dims())? == 1 =>
            {
                match source(weight)?.real(0) {
                    0. => false,
                    1. => true,
                    _ => return Err("Statistics normalization must be 0 or 1.".into()),
                }
            }
            _ => return Err(
                "Statistics vector weights are not implemented; use scalar normalization 0 or 1"
                    .into(),
            ),
        };
        rest = &rest[1..];
    }

    let (mut missing_seen, mut type_seen, mut direction_seen) = (false, false, false);
    for value in rest {
        if let Some(text) = text_option(value) {
            if let Some(omit) = missing_flag(&text)
                && !missing_seen
            {
                options.omit_nan = omit;
                missing_seen = true;
                continue;
            }
            match text.as_str() {
                "all" if !cumulative && !options.explicit_axis => {
                    options.axes = (0..dims.len()).collect();
                    options.explicit_axis = true;
                }
                "double" | "native" | "default"
                    if !type_seen && matches!(name, "sum" | "prod" | "mean") =>
                {
                    options.out = match text.as_str() {
                        "double" => OutType::Double,
                        "native" => OutType::Native,
                        _ => OutType::Default,
                    };
                    type_seen = true;
                }
                "linear" if extreme && !options.linear => options.linear = true,
                "reverse" | "forward" if cumulative && !direction_seen => {
                    options.reverse = text == "reverse";
                    direction_seen = true;
                }
                _ => {
                    return Err(format!(
                        "Invalid option '{text}' for {name} (unsupported or repeated)"
                    ));
                }
            }
        } else if !options.explicit_axis {
            options.axes = dimension_list(value, cumulative)?;
            options.explicit_axis = true;
        } else if matches!(name, "cummax" | "cummin") {
            return Err(format!(
                "{} direction must be 'forward' or 'reverse'.",
                name.to_ascii_uppercase()
            ));
        } else {
            return Err(format!("{name} accepts only one dimension argument"));
        }
    }
    if dispersion && options.explicit_axis && weight.is_none() {
        return Err(format!(
            "{name} requires a weight argument before the dimension. Use 0 for sample normalization."
        ));
    }
    if !options.explicit_axis {
        options.axes = vec![dims.iter().position(|&d| d != 1).unwrap_or(0)];
    }
    Ok(options)
}

/// Maps output and slice indices to column-major offsets without storing a
/// table per input element. Contiguous and uniformly strided layouts use one
/// multiplication. Axes of extent 1 do not affect either mapping.
struct Offsets {
    len: usize,
    axes: Vec<(usize, usize)>,
    step: Option<usize>,
}

impl Offsets {
    fn new(len: usize, axes: impl Iterator<Item = (usize, usize)>) -> Self {
        let axes: Vec<_> = axes.filter(|&(size, _)| size > 1).collect();
        let step = axes.first().map_or(0, |&(_, stride)| stride);
        let regular = axes.windows(2).all(|pair| {
            let [(size, stride), (_, next)] = [pair[0], pair[1]];
            size.checked_mul(stride) == Some(next)
        });
        Self {
            len,
            axes,
            step: regular.then_some(step),
        }
    }

    #[inline]
    fn at(&self, mut k: usize) -> usize {
        if let Some(step) = self.step {
            return k * step;
        }
        let mut offset = 0;
        for &(size, stride) in &self.axes {
            offset += (k % size) * stride;
            k /= size;
        }
        offset
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.len).map(|k| self.at(k))
    }
}

struct Geometry {
    out_dims: Vec<usize>,
    bases: Offsets,
    offsets: Offsets,
}

fn geometry(dims: &[usize], axes: &[usize], keep_empty: bool) -> Result<Geometry> {
    let mut out_dims = dims.to_vec();
    let mut stride = 1usize;
    let mut strides = Vec::with_capacity(dims.len());
    for &d in dims {
        strides.push(stride);
        stride = stride
            .checked_mul(d.max(1))
            .ok_or("Array dimensions overflow")?;
    }
    let reduced = |i| axes.binary_search(&i).is_ok();
    let slice_dims = dims
        .iter()
        .copied()
        .enumerate()
        .filter(|&(i, _)| reduced(i));
    let slice_len = shape_product(&slice_dims.clone().map(|(_, d)| d).collect::<Vec<_>>())?;
    for (i, d) in out_dims.iter_mut().enumerate().filter(|&(i, _)| reduced(i)) {
        *d = if keep_empty && dims[i] == 0 { 0 } else { 1 };
    }
    let count = Matrix::checked_len(out_dims[0], shape_product(&out_dims[1..])?)?;
    let bases = Offsets::new(count, out_dims.iter().copied().zip(strides.iter().copied()));
    let offsets = Offsets::new(slice_len, slice_dims.map(|(i, d)| (d, strides[i])));
    Ok(Geometry {
        out_dims: crate::array::normalize_dims(&out_dims)?,
        bases,
        offsets,
    })
}

pub(super) fn real_out(dims: &[usize], data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(
        Matrix::from_column_vec(dims[0], shape_product(&dims[1..])?, data)?.with_dims(dims)?,
    ))
}

pub(super) fn typed_out(class: NumericClass, dims: &[usize], data: Vec<f64>) -> Result<Value> {
    Ok(Value::Container(
        crate::TypedArray::from_reals(
            class,
            dims[0],
            shape_product(&dims[1..])?,
            data.into_iter(),
        )?
        .with_dims(dims)?
        .into(),
    ))
}

pub(super) fn int_out(class: NumericClass, dims: &[usize], data: Vec<i128>) -> Result<Value> {
    Ok(Value::Container(
        crate::TypedArray::from_ints(class, dims[0], shape_product(&dims[1..])?, data.into_iter())?
            .with_dims(dims)?
            .into(),
    ))
}

/// A complex result whose imaginary parts are all zero is real (MATLAB rule).
pub(super) fn complex_out(engine: &Engine, dims: &[usize], data: Vec<Complex64>) -> Result<Value> {
    if data.iter().all(|z| z.im == 0.) {
        return real_out(dims, data.into_iter().map(|z| z.re).collect());
    }
    Ok(Value::Complex(
        ComplexMatrix::from_column_vec(dims[0], shape_product(&dims[1..])?, data, &engine.cancel)?
            .with_dims(dims)?,
    ))
}

pub(super) fn logical_out(engine: &Engine, dims: &[usize], data: Vec<u8>) -> Result<Value> {
    Ok(Value::Logical(
        LogicalMatrix::from_column_vec(dims[0], shape_product(&dims[1..])?, data, &engine.cancel)?
            .with_dims(dims)?,
    ))
}

fn complex_nan(z: Complex64) -> bool {
    z.re.is_nan() || z.im.is_nan()
}

/// The second output is used by extrema and mode.
pub(super) fn reduce(
    engine: &Engine,
    name: &str,
    args: &[Value],
    want_second: bool,
) -> Result<(Value, Option<Value>)> {
    engine.check_cancel()?;
    let input = args.first().ok_or("Not enough input arguments.")?;
    let dims = input.dims();
    let mut options = parse(name, args, &dims)?;
    let src = source(input)?;
    if matches!(name, "cummax" | "cummin") {
        if matches!(src, Source::Units(_) | Source::Text(_)) {
            return Err(format!(
                "{} supports only numerical or logical inputs.",
                name.to_ascii_uppercase()
            ));
        }
        return cumulative_extreme(engine, name, input, &src, &dims, &options, want_second);
    }
    if is_cumulative(name) {
        return cumulative(engine, name, &src, &dims, &options).map(|v| (v, None));
    }
    let extreme = matches!(name, "min" | "max");
    let keep_empty = extreme || name == "range";
    if dims.as_ref() == [0, 0] && !options.explicit_axis && !keep_empty {
        options.axes = vec![0, 1];
    }
    let g = geometry(&dims, &options.axes, keep_empty)?;
    engine.preflight_elements(g.bases.len.saturating_mul(1 + usize::from(want_second)))?;
    if extreme {
        return extreme_values(engine, name, input, &src, &g, &options, want_second);
    }
    if matches!(name, "median" | "std" | "var" | "mode" | "range") {
        return statistics(engine, name, input, &src, &g, &options, want_second);
    }
    if matches!(name, "any" | "all") {
        let all = name == "all";
        let mut output = Vec::with_capacity(g.bases.len);
        for (o, base) in g.bases.iter().enumerate() {
            poll(engine, o)?;
            let mut hit = all;
            for (k, offset) in g.offsets.iter().enumerate() {
                poll(engine, k)?;
                let z = src.complex(base + offset);
                if options.omit_nan && complex_nan(z) {
                    continue;
                }
                let nonzero = z.re != 0. || z.im != 0.;
                if nonzero != all {
                    hit = nonzero;
                    break;
                }
            }
            output.push(u8::from(hit));
        }
        return logical_out(engine, &g.out_dims, output).map(|v| (v, None));
    }
    arithmetic(engine, name, &src, &g, &options).map(|value| (value, None))
}

#[inline]
fn poll(engine: &Engine, i: usize) -> Result<()> {
    if i.is_multiple_of(8192) {
        engine.check_cancel()?;
    }
    Ok(())
}

fn map_lanes<T>(
    engine: &Engine,
    g: &Geometry,
    mut lane: impl FnMut(usize) -> Result<T>,
) -> Result<Vec<T>> {
    let mut output = Vec::with_capacity(g.bases.len);
    for (i, base) in g.bases.iter().enumerate() {
        poll(engine, i)?;
        output.push(lane(base)?);
    }
    engine.check_cancel()?;
    Ok(output)
}

fn fold_lane<T: Copy>(
    engine: &Engine,
    g: &Geometry,
    base: usize,
    at: impl Fn(usize) -> Option<T>,
    combine: impl Fn(T, T) -> T,
    identity: T,
) -> Result<(T, usize)> {
    let mut acc = identity;
    let mut used = 0;
    for (k, offset) in g.offsets.iter().enumerate() {
        poll(engine, k)?;
        if let Some(x) = at(base + offset) {
            acc = if used == 0 { x } else { combine(acc, x) };
            used += 1;
        }
    }
    Ok((acc, used))
}

// Integer conversion rounds midpoint ties away from zero without going through f64.
fn rounded_quotient(sum: i128, n: i128) -> i128 {
    if n == 0 {
        return 0;
    }
    sum / n
        + if 2 * (sum % n).abs() >= n {
            sum.signum()
        } else {
            0
        }
}

fn arithmetic(
    engine: &Engine,
    name: &str,
    src: &Source<'_>,
    g: &Geometry,
    options: &Options,
) -> Result<Value> {
    let product = name == "prod";
    let mean = name == "mean";
    let native = options.out == OutType::Native;
    let dims = &g.out_dims;
    if matches!(src, Source::Units(_) | Source::Text(_)) && native {
        return Err(format!("{name} does not support 'native' for char input"));
    }
    match src {
        Source::Int(a) if native || mean => {
            let sums = map_lanes(engine, g, |base| {
                fold_lane(
                    engine,
                    g,
                    base,
                    |i| Some(a.int(i)),
                    |acc, x| {
                        let next = if product {
                            acc.saturating_mul(x)
                        } else {
                            acc.saturating_add(x)
                        };
                        if mean { next } else { a.class().saturate(next) }
                    },
                    i128::from(product),
                )
                .map(|(acc, _)| acc)
            })?;
            if mean {
                let divisor = g.offsets.len as f64;
                if native {
                    let n = g.offsets.len as i128;
                    let rounded = sums
                        .into_iter()
                        .map(|sum| rounded_quotient(sum, n))
                        .collect();
                    int_out(a.class(), dims, rounded)
                } else {
                    real_out(dims, sums.into_iter().map(|s| s as f64 / divisor).collect())
                }
            } else {
                int_out(a.class(), dims, sums)
            }
        }
        Source::Bits(a) if native && !mean => {
            let output = map_lanes(engine, g, |base| {
                let (x, _) = fold_lane(
                    engine,
                    g,
                    base,
                    |i| Some(a[i]),
                    |acc, x| if product { acc & x } else { acc | x },
                    u8::from(product),
                )?;
                Ok(x)
            })?;
            logical_out(engine, dims, output)
        }
        Source::Complex(_) => {
            let output = map_lanes(engine, g, |base| {
                let (acc, used) = fold_lane(
                    engine,
                    g,
                    base,
                    |i| {
                        let x = src.complex(i);
                        (!options.omit_nan || !complex_nan(x)).then_some(x)
                    },
                    |acc, x| if product { acc * x } else { acc + x },
                    Complex64::new(f64::from(u8::from(product)), 0.),
                )?;
                Ok(if mean { acc / used as f64 } else { acc })
            })?;
            complex_out(engine, dims, output)
        }
        _ => {
            let single = matches!(src, Source::Single(_)) && options.out != OutType::Double;
            let output = map_lanes(engine, g, |base| {
                if single && !mean {
                    let (acc, _) = fold_lane(
                        engine,
                        g,
                        base,
                        |i| {
                            let x = src.real(i) as f32;
                            (!options.omit_nan || !x.is_nan()).then_some(x)
                        },
                        |acc, x| if product { acc * x } else { acc + x },
                        f32::from(u8::from(product)),
                    )?;
                    Ok(f64::from(acc))
                } else {
                    let (acc, used) = fold_lane(
                        engine,
                        g,
                        base,
                        |i| {
                            let x = src.real(i);
                            (!options.omit_nan || !x.is_nan()).then_some(x)
                        },
                        |acc, x| if product { acc * x } else { acc + x },
                        f64::from(u8::from(product)),
                    )?;
                    Ok(if mean { acc / used as f64 } else { acc })
                }
            })?;
            if single {
                typed_out(NumericClass::Single, dims, output)
            } else {
                real_out(dims, output)
            }
        }
    }
}

/// Order of two elements for max/min: integers exactly, reals by value, complex
/// values by magnitude and then phase angle (MATLAB max/min pages).
fn greater(src: &Source<'_>, x: usize, y: usize) -> std::cmp::Ordering {
    match src {
        Source::Int(a) => a.int(x).cmp(&a.int(y)),
        Source::Complex(d) => {
            let (a, b) = (d[x], d[y]);
            libm::hypot(a.re, a.im)
                .partial_cmp(&libm::hypot(b.re, b.im))
                .unwrap_or(Ordering::Equal)
                .then_with(|| Key::of(a).cmp(&Key::of(b)))
        }
        other => other
            .real(x)
            .partial_cmp(&other.real(y))
            .unwrap_or(std::cmp::Ordering::Equal),
    }
}

pub(super) fn is_nan_at(src: &Source<'_>, i: usize) -> bool {
    match src {
        Source::Int(_) | Source::Bits(_) | Source::Units(_) => false,
        Source::Complex(d) => complex_nan(d[i]),
        other => other.real(i).is_nan(),
    }
}

fn extreme_values(
    engine: &Engine,
    name: &str,
    input: &Value,
    src: &Source<'_>,
    g: &Geometry,
    options: &Options,
    want_index: bool,
) -> Result<(Value, Option<Value>)> {
    let wanted = if name == "max" {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Less
    };
    let mut positions = Vec::with_capacity(g.bases.len);
    let mut indices = Vec::with_capacity(if want_index { g.bases.len } else { 0 });
    for (o, base) in g.bases.iter().enumerate() {
        poll(engine, o)?;
        let mut best = None;
        let mut first_nan = None;
        for (k, offset) in g.offsets.iter().enumerate() {
            poll(engine, k)?;
            let i = base + offset;
            if is_nan_at(src, i) {
                first_nan.get_or_insert((k, i));
                if !options.omit_nan {
                    break;
                }
            } else if best.is_none_or(|(_, b)| greater(src, i, b) == wanted) {
                best = Some((k, i));
            }
        }
        let (k, i) = if options.omit_nan {
            best.or(first_nan)
        } else {
            first_nan.or(best)
        }
        .expect("nonempty extreme slices");
        positions.push(i);
        if want_index {
            indices.push((if options.linear { i } else { k } + 1) as f64);
        }
    }
    let value = selected_out(engine, input, src, &g.out_dims, &positions)?;
    let indices = want_index
        .then(|| real_out(&g.out_dims, indices))
        .transpose()?;
    Ok((value, indices))
}

fn selected_out(
    engine: &Engine,
    input: &Value,
    src: &Source<'_>,
    dims: &[usize],
    positions: &[usize],
) -> Result<Value> {
    match src {
        Source::Int(a) | Source::Single(a) => Ok(Value::Container(
            a.select_with_cancel(
                dims[0],
                shape_product(&dims[1..])?,
                positions,
                &engine.cancel,
            )?
            .with_dims(dims)?
            .into(),
        )),
        Source::Bits(a) => logical_out(engine, dims, positions.iter().map(|&i| a[i]).collect()),
        Source::Complex(a) => complex_out(engine, dims, positions.iter().map(|&i| a[i]).collect()),
        Source::Real(_) | Source::Units(_) | Source::Text(_) => {
            if dims == input.dims().as_ref()
                && matches!(src, Source::Real(_))
                && positions.iter().enumerate().all(|(i, &p)| i == p)
            {
                return Ok(input.clone());
            }
            real_out(dims, positions.iter().map(|&i| src.real(i)).collect())
        }
    }
}

fn statistics(
    engine: &Engine,
    name: &str,
    input: &Value,
    src: &Source<'_>,
    g: &Geometry,
    options: &Options,
    want_frequency: bool,
) -> Result<(Value, Option<Value>)> {
    let dims = &g.out_dims;
    let len = g.offsets.len;
    let omit = options.omit_nan;
    if name == "mode" {
        return mode(engine, src, g, omit, want_frequency);
    }
    if name == "median" && matches!(src, Source::Complex(_) | Source::Int(_)) {
        let mut scratch = SortScratch::default();
        if let Source::Int(a) = src {
            let data = map_lanes(engine, g, |base| {
                scratch.sort(engine, src, g, base)?;
                let order = &scratch.order;
                let mid = order.len() / 2;
                Ok(match order.get(mid) {
                    None => 0,
                    Some(&hi) if order.len().is_multiple_of(2) => {
                        rounded_quotient(a.int(order[mid - 1]) + a.int(hi), 2)
                    }
                    Some(&hi) => a.int(hi),
                })
            })?;
            return Ok((int_out(a.class(), dims, data)?, None));
        }
        let data = map_lanes(engine, g, |base| {
            let missing = scratch.sort(engine, src, g, base)?;
            let order = &scratch.order;
            if (missing != 0 && !omit) || order.is_empty() {
                return Ok(Complex64::new(f64::NAN, 0.));
            }
            let mid = order.len() / 2;
            let hi = src.complex(order[mid]);
            Ok(if order.len().is_multiple_of(2) {
                cplx_reduce::mean_of(src.complex(order[mid - 1]), hi)
            } else {
                hi
            })
        })?;
        return Ok((complex_out(engine, dims, data)?, None));
    }

    if name == "range"
        && let Source::Int(a) = src
    {
        let data = map_lanes(engine, g, |base| {
            let mut bounds: Option<(i128, i128)> = None;
            for (k, offset) in g.offsets.iter().enumerate() {
                poll(engine, k)?;
                let x = a.int(base + offset);
                bounds = Some(bounds.map_or((x, x), |(lo, hi)| (lo.min(x), hi.max(x))));
            }
            Ok(bounds.map_or(0, |(lo, hi)| a.class().saturate(hi - lo)))
        })?;
        return Ok((int_out(a.class(), dims, data)?, None));
    }
    if name == "range" && matches!(src, Source::Complex(_)) {
        return Err("range requires real data".into());
    }
    let kind = if name == "std" {
        Dispersion::StandardDeviation
    } else {
        Dispersion::Variance
    };
    let cancel = &*engine.cancel;
    let range_past_dims = options.axes.iter().all(|&axis| axis >= input.dims().len());
    let lane = |scratch: &mut MedianScratch, i| {
        poll(engine, i)?;
        let base = g.bases.at(i);
        let at = |k| src.real(base + g.offsets.at(k));
        match name {
            "median" => scratch.reduce(at, len, omit, cancel),
            "range" if range_past_dims => {
                let x = at(0);
                Ok(if x.is_nan() { f64::NAN } else { 0. })
            }
            "range" => range_lane(at, len, omit, cancel),
            _ if matches!(src, Source::Complex(_)) => cplx_reduce::dispersion_lane(
                |k| src.complex(base + g.offsets.at(k)),
                len,
                options.population,
                omit,
                kind,
                cancel,
            ),
            _ => dispersion_lane(at, len, options.population, omit, kind, cancel),
        }
    };
    // Reuse the existing median and dispersion lane thresholds. All workers
    // see the same slice order, and each median scratch is bounded by 8192 items.
    let data = if name != "range"
        && len.saturating_mul(g.bases.len) >= 512 * 1024
        && g.bases.len > 1
        && crate::parallel::current_worker_count() > 1
    {
        use rayon::prelude::*;
        (0..g.bases.len)
            .into_par_iter()
            .map_init(MedianScratch::default, lane)
            .collect::<Result<Vec<_>>>()?
    } else {
        let mut scratch = MedianScratch::default();
        (0..g.bases.len)
            .map(|i| lane(&mut scratch, i))
            .collect::<Result<Vec<_>>>()?
    };
    let result = if name == "median" && matches!(src, Source::Bits(_)) {
        logical_out(
            engine,
            dims,
            data.into_iter()
                .map(|x| u8::from(!x.is_nan() && x != 0.))
                .collect(),
        )?
    } else if matches!(src, Source::Single(_)) {
        typed_out(NumericClass::Single, dims, data)?
    } else {
        real_out(dims, data)?
    };
    Ok((result, None))
}

fn range_lane(
    at: impl Fn(usize) -> f64,
    len: usize,
    omit: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<f64> {
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in 0..len {
        if k.is_multiple_of(8192) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Calculation cancelled".into());
        }
        let x = at(k);
        if x.is_nan() {
            if !omit {
                return Ok(f64::NAN);
            }
        } else {
            low = low.min(x);
            high = high.max(x);
        }
    }
    Ok(if low > high { f64::NAN } else { high - low })
}

/// Scratch reused across lanes by the cancellable merge sort.
#[derive(Default)]
struct SortScratch {
    order: Vec<usize>,
    complex_keys: Vec<(usize, Key)>,
}

impl SortScratch {
    fn sort(
        &mut self,
        engine: &Engine,
        src: &Source<'_>,
        g: &Geometry,
        base: usize,
    ) -> Result<usize> {
        let complex = matches!(src, Source::Complex(_));
        let units_per_item = if complex { 6 } else { 2 };
        engine.preflight_elements(g.offsets.len.saturating_mul(units_per_item))?;
        let order = &mut self.order;
        let keys = &mut self.complex_keys;
        order.clear();
        keys.clear();
        let mut missing = 0;
        for (k, offset) in g.offsets.iter().enumerate() {
            poll(engine, k)?;
            let i = base + offset;
            if is_nan_at(src, i) {
                missing += 1;
            } else if complex {
                order.push(keys.len());
                keys.push((i, Key::of(src.complex(i))));
            } else {
                order.push(i);
            }
        }
        if complex {
            engine.cxr_sort_by(order, |a, b| keys[a].1.cmp(&keys[b].1))?;
            for (k, i) in order.iter_mut().enumerate() {
                poll(engine, k)?;
                *i = keys[*i].0;
            }
        } else {
            engine.cxr_sort_by(order, |a, b| greater(src, a, b))?;
        }
        Ok(missing)
    }
}

fn mode(
    engine: &Engine,
    src: &Source<'_>,
    g: &Geometry,
    omit: bool,
    want_frequency: bool,
) -> Result<(Value, Option<Value>)> {
    let dims = &g.out_dims;
    let mut scratch = SortScratch::default();
    let mut modes = Vec::with_capacity(g.bases.len);
    let mut frequencies = Vec::with_capacity(if want_frequency { g.bases.len } else { 0 });
    for (lane, base) in g.bases.iter().enumerate() {
        poll(engine, lane)?;
        let missing = scratch.sort(engine, src, g, base)?;
        let order = &scratch.order;
        let (mut best, mut count) = (None, 0);
        let mut i = 0;
        while i < order.len() {
            poll(engine, i)?;
            let mut j = i + 1;
            while j < order.len()
                && match src {
                    Source::Complex(_) => src.complex(order[i]) == src.complex(order[j]),
                    _ => greater(src, order[i], order[j]) == Ordering::Equal,
                }
            {
                poll(engine, j)?;
                j += 1;
            }
            if j - i > count {
                best = Some(order[i]);
                count = j - i;
            }
            i = j;
        }
        if missing != 0 && (!omit || order.is_empty()) {
            best = None;
            count = if omit { 1 } else { missing };
        }
        modes.push(best);
        if want_frequency {
            frequencies.push(count as f64);
        }
    }

    let result = match src {
        Source::Complex(_) => complex_out(
            engine,
            dims,
            modes
                .iter()
                .map(|i| i.map_or(Complex64::new(f64::NAN, 0.), |i| src.complex(i)))
                .collect(),
        )?,
        Source::Single(_) => typed_out(
            NumericClass::Single,
            dims,
            modes
                .iter()
                .map(|i| i.map_or(f64::NAN, |i| src.real(i)))
                .collect(),
        )?,
        Source::Units(_) | Source::Text(_) => {
            let chars = crate::CharacterRow::from_column_major(
                dims[0],
                shape_product(&dims[1..])?,
                modes
                    .iter()
                    .map(|i| i.map_or(0, |i| src.real(i) as u16))
                    .collect(),
                &engine.cancel,
            )?
            .with_dims(dims)?;
            Value::Container(Array::Character(chars))
        }
        Source::Int(a) => int_out(
            a.class(),
            dims,
            modes.iter().map(|i| i.map_or(0, |i| a.int(i))).collect(),
        )?,
        Source::Bits(_) => logical_out(
            engine,
            dims,
            modes
                .iter()
                .map(|i| i.map_or(0, |i| u8::from(src.real(i) != 0.)))
                .collect(),
        )?,
        _ => real_out(
            dims,
            modes
                .iter()
                .map(|i| i.map_or(f64::NAN, |i| src.real(i)))
                .collect(),
        )?,
    };
    let frequency = want_frequency
        .then(|| real_out(dims, frequencies))
        .transpose()?;
    Ok((result, frequency))
}

/// Visit each block in scan order, keeping the lane's prior output in place.
struct Scan {
    len: usize,
    stride: usize,
    extent: usize,
    reverse: bool,
}

impl Scan {
    fn new(dims: &[usize], options: &Options) -> Result<Self> {
        let axis = options.axes[0];
        Ok(Self {
            len: shape_product(dims)?,
            stride: shape_product(&dims[..axis.min(dims.len())])?,
            extent: dims.get(axis).copied().unwrap_or(1),
            reverse: options.reverse,
        })
    }

    fn iter(&self) -> impl Iterator<Item = (usize, Option<usize>, usize)> + '_ {
        (0..self.len).map(|k| {
            let step = k / self.stride % self.extent;
            let (i, position) = if self.reverse {
                let position = self.extent - 1 - step;
                (k - step * self.stride + (position * self.stride), position)
            } else {
                (k, step)
            };
            let previous = if step == 0 {
                None
            } else if self.reverse {
                Some(i + self.stride)
            } else {
                Some(i - self.stride)
            };
            (i, previous, position)
        })
    }
}

fn cumulative(
    engine: &Engine,
    name: &str,
    src: &Source<'_>,
    dims: &[usize],
    options: &Options,
) -> Result<Value> {
    let scan = Scan::new(dims, options)?;
    engine.preflight_elements(scan.len)?;
    let product = name == "cumprod";
    let identity = f64::from(u8::from(product));
    let omit = options.omit_nan;
    match src {
        Source::Int(a) => int_out(
            a.class(),
            dims,
            scan_values(
                engine,
                &scan,
                i128::from(product),
                |i| Some(a.int(i)),
                |acc: i128, x| {
                    let next = if product {
                        acc.saturating_mul(x)
                    } else {
                        acc.saturating_add(x)
                    };
                    a.class().saturate(next)
                },
            )?,
        ),
        Source::Complex(_) => complex_out(
            engine,
            dims,
            scan_values(
                engine,
                &scan,
                Complex64::new(identity, 0.),
                |i| {
                    let z = src.complex(i);
                    (!omit || !complex_nan(z)).then_some(z)
                },
                |a, b| if product { a * b } else { a + b },
            )?,
        ),
        _ => {
            let data = scan_values(
                engine,
                &scan,
                identity,
                |i| {
                    let x = src.real(i);
                    (!omit || !x.is_nan()).then_some(x)
                },
                |a, b| if product { a * b } else { a + b },
            )?;
            if matches!(src, Source::Single(_)) {
                typed_out(NumericClass::Single, dims, data)
            } else {
                real_out(dims, data)
            }
        }
    }
}

fn scan_values<T: Copy>(
    engine: &Engine,
    scan: &Scan,
    identity: T,
    at: impl Fn(usize) -> Option<T>,
    combine: impl Fn(T, T) -> T,
) -> Result<Vec<T>> {
    let mut output = vec![identity; scan.len];
    for (k, (i, previous, _)) in scan.iter().enumerate() {
        poll(engine, k)?;
        output[i] = match (previous, at(i)) {
            (Some(p), Some(x)) => combine(output[p], x),
            (Some(p), None) => output[p],
            (None, value) => value.unwrap_or(identity),
        };
    }
    Ok(output)
}

fn cumulative_extreme(
    engine: &Engine,
    name: &str,
    input: &Value,
    src: &Source<'_>,
    dims: &[usize],
    options: &Options,
    want_index: bool,
) -> Result<(Value, Option<Value>)> {
    let scan = Scan::new(dims, options)?;
    engine.preflight_elements(scan.len.saturating_mul(2 + 2 * usize::from(want_index)))?;
    let mut positions = vec![0; scan.len];
    let mut indices = vec![0.; if want_index { scan.len } else { 0 }];
    let wanted = if name == "cummax" {
        Ordering::Greater
    } else {
        Ordering::Less
    };
    for (k, (i, previous, pos)) in scan.iter().enumerate() {
        poll(engine, k)?;
        let mut chosen = i;
        let mut at = (pos + 1) as f64;
        if let Some(previous) = previous {
            let best = positions[previous];
            let nan = is_nan_at(src, i);
            let previous_nan = is_nan_at(src, best);
            let take = if options.omit_nan {
                !nan && (previous_nan || greater(src, i, best) == wanted)
            } else {
                !previous_nan && (nan || greater(src, i, best) == wanted)
            };
            if !take {
                if !(options.omit_nan && previous_nan) {
                    chosen = best;
                }
                if want_index {
                    at = indices[previous];
                }
            }
        }
        positions[i] = chosen;
        if want_index {
            indices[i] = at;
        }
    }
    let result = selected_out(engine, input, src, dims, &positions)?;
    Ok((
        result,
        want_index.then(|| real_out(dims, indices)).transpose()?,
    ))
}
