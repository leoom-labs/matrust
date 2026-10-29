//! Built-in functions for integer and single values: constructors, class
//! queries, bit operations, and the class rules of the everyday functions
//! (which keep the class, which return double, which reject integers).
use super::typed_values::is_typed;
use super::{Engine, Execution, Value};
use crate::typed_array::{NumericClass, TypedArray, divide_round};
use crate::{Array, LogicalMatrix, Matrix, Result};

/// Names handled here even when no argument is an integer or single value.
const OWN: &[&str] = &[
    "int8",
    "uint8",
    "int16",
    "uint16",
    "int32",
    "uint32",
    "int64",
    "uint64",
    "single",
    "intmax",
    "intmin",
    "isinteger",
    "isfloat",
    "cast",
    "idivide",
    "typecast",
    "zeros",
    "ones",
    "eye",
    "rand",
    "randn",
    "realmax",
    "realmin",
    "eps",
    "flintmax",
    "\u{1}int64_literal",
    "\u{1}uint64_literal",
];

/// Floating-point-only functions: an integer argument is an undefined function.
const FLOAT_ONLY: &[&str] = &[
    "sqrt", "exp", "log", "log2", "log10", "log1p", "expm1", "sin", "cos", "tan", "sec", "csc",
    "cot", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh",
    "sind", "cosd", "tand", "inv", "det", "pinv", "expm", "logm", "sqrtm", "lu", "qr", "chol",
    "svd", "eig", "gamma", "gammaln", "erf", "erfc", "norm", "hypot", "nthroot", "realsqrt",
    "reallog", "cross", "trace", "rank", "null", "orth", "fft", "ifft", "polyval", "roots",
    "interp1", "trapz", "cumtrapz", "std", "var", "erfcx",
];

/// Functions whose integer result keeps the integer class.
const INT_KEEP: &[&str] = &[
    "abs", "sign", "floor", "ceil", "round", "fix", "max", "min", "cumsum", "cumprod", "diff",
    "mod", "rem", "sort", "unique", "real", "conj", "median", "mode", "uminus", "uplus", "plus",
    "minus", "times", "dot",
];

/// Functions whose single result stays single (computed in double, rounded).
const SINGLE_KEEP: &[&str] = &[
    "sqrt", "exp", "log", "log2", "log10", "log1p", "expm1", "sin", "cos", "tan", "sec", "csc",
    "cot", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh",
    "sind", "cosd", "tand", "inv", "pinv", "expm", "sqrtm", "gamma", "erf", "erfc", "norm",
    "hypot", "nthroot", "abs", "sign", "floor", "ceil", "round", "fix", "max", "min", "sum",
    "prod", "cumsum", "cumprod", "diff", "mean", "median", "mode", "std", "var", "mod", "rem",
    "linspace", "sort", "unique", "real", "conj", "cross", "dot", "trace", "power", "plus",
    "minus", "times", "uminus", "uplus", "polyval", "interp1", "trapz", "cumtrapz", "fft", "ifft",
    "erfcx",
];

/// Functions that only move elements: they run on positions, so every value
/// keeps its class exactly.
const PERMUTE: &[&str] = &[
    "reshape",
    "repmat",
    "transpose",
    "ctranspose",
    "fliplr",
    "flipud",
    "flip",
    "circshift",
    "rot90",
    "squeeze",
    "tril",
    "triu",
    "diag",
];

/// Functions that receive integer and single values unchanged.
const KEEP_ARGS: &[&str] = &[
    "size",
    "numel",
    "length",
    "ndims",
    "isscalar",
    "isvector",
    "isrow",
    "iscolumn",
    "ismatrix",
    "isempty",
    "isreal",
    "islogical",
    "isnumeric",
    "sort",
    "sortrows",
    "unique",
    "isequal",
    "disp",
    "display",
    "class",
    "struct",
    "cell",
    "setfield",
    "getfield",
    "num2cell",
    "deal",
    "iscell",
    "ischar",
    "isstruct",
    "isfield",
    "fieldnames",
    "horzcat",
    "vertcat",
    "cat",
    "isa",
];

fn reject_wide(name: &str, args: &[Value], cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    for a in args.iter().filter_map(typed) {
        if matches!(a.class(), NumericClass::Int64 | NumericClass::Uint64) {
            for i in 0..a.len() {
                if i.is_multiple_of(8192) {
                    crate::dsp::check_cancel(cancel)?;
                }
                if a.int(i).unsigned_abs() > (1u128 << 53) {
                    return Err(format!(
                        "{name} is not supported for {} values above 2^53 yet",
                        a.class().name()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn demote(value: Value) -> Result<Value> {
    match value {
        Value::Container(Array::Typed(t)) => Ok(Value::Matrix(t.to_matrix()?)),
        v => Ok(v),
    }
}

/// Multi-output built-ins: integer and single arguments as doubles, and the
/// class of the first one for `restore_first`.
pub(super) fn demote_arguments(values: Vec<Value>) -> Result<(Vec<Value>, Option<NumericClass>)> {
    let class = values.iter().find_map(typed).map(TypedArray::class);
    Ok((
        values.into_iter().map(demote).collect::<Result<_>>()?,
        class,
    ))
}

/// Gives the first output of a multi-output call back its class (`[m, i] = max(x)`).
pub(super) fn restore_first(outputs: &mut [Value], class: Option<NumericClass>) -> Result<()> {
    if let (Some(class), Some(first)) = (class, outputs.first_mut()) {
        *first = recast(std::mem::replace(first, Value::Void), class)?;
    }
    Ok(())
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}

fn typed(value: &Value) -> Option<&TypedArray> {
    match value {
        Value::Container(Array::Typed(t)) => Some(t),
        _ => None,
    }
}

fn boolean(v: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(v))
}

fn class_error(name: &str, class: NumericClass) -> String {
    format!(
        "Undefined function '{name}' for input arguments of type '{}'.",
        class.name()
    )
}

/// Converts any numeric, logical or character value to `class`.
pub(super) fn convert(value: &Value, class: NumericClass) -> Result<TypedArray> {
    match value {
        Value::Container(Array::Typed(t)) => t.cast(class),
        Value::Matrix(m) => TypedArray::from_matrix(class, m),
        Value::Logical(m) => TypedArray::from_reals(
            class,
            m.rows(),
            m.cols(),
            m.data().iter().map(|&v| f64::from(v)),
        ),
        Value::Text(s) | Value::QuotedString(s) => {
            let c = crate::CharacterRow::from_text(s, &Default::default())?;
            TypedArray::from_reals(
                class,
                c.rows(),
                c.cols(),
                c.units().iter().map(|&u| f64::from(u)),
            )
        }
        Value::Container(Array::Character(c)) => TypedArray::from_reals(
            class,
            c.rows(),
            c.cols(),
            c.units().iter().map(|&u| f64::from(u)),
        ),
        Value::Complex(_) => {
            Err("Complex integer and complex single values are not supported yet".into())
        }
        _ => Err(format!(
            "Conversion to {} from this class is not possible.",
            class.name()
        )),
    }
}

fn typed_value(a: TypedArray) -> Value {
    Value::Container(Array::Typed(a))
}

/// Restores `class` on a double result (logical and other results pass through).
fn recast(value: Value, class: NumericClass) -> Result<Value> {
    match value {
        Value::Matrix(m) => Ok(typed_value(TypedArray::from_matrix(class, &m)?)),
        Value::Complex(_) if class == NumericClass::Single => {
            Err("Complex single values are not supported yet".into())
        }
        v => Ok(v),
    }
}

/// Determinant in single precision (LU with partial pivoting in `f32`).
fn det_single(a: &TypedArray, cancel: &std::sync::atomic::AtomicBool) -> Result<f64> {
    let n = a.rows();
    if n != a.cols() {
        return Err("Matrix must be square.".into());
    }
    crate::dsp::check_cancel(cancel)?;
    let mut m = nalgebra::DMatrix::from_iterator(n, n, (0..a.len()).map(|i| a.real(i) as f32));
    let mut det = 1f32;
    for k in 0..n {
        crate::dsp::check_cancel(cancel)?;
        let p = (k..n)
            .max_by(|&i, &j| m[(i, k)].abs().total_cmp(&m[(j, k)].abs()))
            .unwrap_or(k);
        if m[(p, k)] == 0. {
            return Ok(0.);
        }
        if p != k {
            m.swap_rows(p, k);
            det = -det;
        }
        let pivot = m[(k, k)];
        det *= pivot;
        let trailing = n - k - 1;
        if trailing != 0 {
            let factors = m.column(k).rows(k + 1, trailing).into_owned() / pivot;
            // Bounded calls to nalgebra's rank-one update keep f32 LU rounding
            // while polling cancellation between at most n*64 element updates.
            for col in (k + 1..n).step_by(64) {
                crate::dsp::check_cancel(cancel)?;
                let width = (n - col).min(64);
                let row = m.row(k).columns(col, width).transpose();
                m.view_mut((k + 1, col), (trailing, width))
                    .ger(-1., &factors, &row, 1.);
            }
        }
    }
    Ok(f64::from(det))
}

/// Spacing of single values at `|x|` (`eps(single(1000))` is 2^-14).
fn eps_single(x: f32) -> f32 {
    let a = x.abs();
    if !a.is_finite() {
        return f32::NAN;
    }
    f32::from_bits(a.to_bits() + 1) - a
}

impl Engine {
    /// Native min/max selection. Neither the comparison keys nor the selected
    /// integer values pass through floating point; only index outputs are double.
    pub(super) fn typed_extreme(
        &self,
        name: &str,
        args: &[Value],
        outputs: usize,
        want_indices: bool,
    ) -> Result<Option<Vec<Value>>> {
        let Some(class) = args.iter().find_map(typed).map(TypedArray::class) else {
            return Ok(None);
        };
        self.check_cancel()?;
        let better = |a: &TypedArray, i, b: &TypedArray, j| {
            if class.is_integer() {
                if name == "min" {
                    a.int(i) < b.int(j)
                } else {
                    a.int(i) > b.int(j)
                }
            } else {
                let (x, y) = (a.real(i), b.real(j));
                !x.is_nan() && (y.is_nan() || if name == "min" { x < y } else { x > y })
            }
        };
        if let [a, b] = args {
            if outputs > 1 {
                return Err(format!(
                    "{name} with two matrices to compare and two output arguments is not supported"
                ));
            }
            if class.is_integer()
                && [a, b].iter().any(|v| match v {
                    Value::Container(Array::Typed(t)) => t.class() != class,
                    Value::Matrix(m) => m.len() != 1,
                    _ => true,
                })
            {
                return Err("Integers can only be combined with integers of the same class, or scalar doubles.".into());
            }
            let (a, b) = (convert(a, class)?, convert(b, class)?);
            let (rows, cols) = if a.len() == 1 {
                (b.rows(), b.cols())
            } else if b.len() == 1 || (a.rows(), a.cols()) == (b.rows(), b.cols()) {
                (a.rows(), a.cols())
            } else {
                return Err("Matrix dimensions must agree.".into());
            };
            let n = Matrix::checked_len(rows, cols)?;
            let mut picks = Vec::with_capacity(n);
            for k in 0..n {
                if k.is_multiple_of(8192) {
                    self.check_cancel()?;
                }
                let (i, j) = (
                    if a.len() == 1 { 0 } else { k },
                    if b.len() == 1 { 0 } else { k },
                );
                picks.push(better(&b, j, &a, i));
            }
            let value = if class.is_integer() {
                TypedArray::from_ints(
                    class,
                    rows,
                    cols,
                    picks.iter().enumerate().map(|(k, &b_wins)| {
                        if b_wins {
                            b.int(if b.len() == 1 { 0 } else { k })
                        } else {
                            a.int(if a.len() == 1 { 0 } else { k })
                        }
                    }),
                )?
            } else {
                TypedArray::from_reals(
                    class,
                    rows,
                    cols,
                    picks.iter().enumerate().map(|(k, &b_wins)| {
                        if b_wins {
                            b.real(if b.len() == 1 { 0 } else { k })
                        } else {
                            a.real(if a.len() == 1 { 0 } else { k })
                        }
                    }),
                )?
            };
            return Ok(Some(vec![typed_value(value)]));
        }
        let (a, dim) = match args {
            [a] => (a, None),
            [a, Value::Matrix(empty), d] if empty.is_empty() => {
                (a, Some(self.reduction_dimension(name, d)?))
            }
            _ => {
                return Err(format!(
                    "Use {name}(A), {name}(A,[],dim) or {name}(A,B); 'all' and NaN flags are not implemented"
                ));
            }
        };
        let a = convert(a, class)?;
        let dim = dim.unwrap_or(if a.rows() == 1 && a.cols() != 1 { 2 } else { 1 });
        let (rows, cols, lane) = match dim {
            1 => (a.rows().min(1), a.cols(), a.rows()),
            2 => (a.rows(), a.cols().min(1), a.cols()),
            _ => (a.rows(), a.cols(), 1),
        };
        let n = Matrix::checked_len(rows, cols)?;
        if self
            .evaluated_elements
            .saturating_add(n.saturating_mul(1 + usize::from(want_indices)))
            > 32 * 1024 * 1024
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let mut positions = Vec::with_capacity(n);
        let mut indices = if want_indices {
            Vec::with_capacity(n)
        } else {
            Vec::new()
        };
        for output in 0..n {
            self.check_cancel()?;
            let at = |k| match dim {
                1 => output * a.rows() + k,
                2 => output + k * a.rows(),
                _ => output,
            };
            let mut best = 0;
            for k in 1..lane {
                if k.is_multiple_of(8192) {
                    self.check_cancel()?;
                }
                if better(&a, at(k), &a, at(best)) {
                    best = k;
                }
            }
            positions.push(at(best));
            if want_indices {
                indices.push((best + 1) as f64);
            }
        }
        let value = a.select_with_cancel(rows, cols, &positions, &self.cancel)?;
        let index = if want_indices {
            Matrix::from_column_vec(rows, cols, indices)?
        } else {
            Matrix::filled(0, 0, 0.)?
        };
        Ok(Some(vec![typed_value(value), Value::Matrix(index)]))
    }

    /// `Ok(value)` when the call is handled here; otherwise the arguments to
    /// pass on, with integer and single values as doubles.
    pub(super) fn typed_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
        out: &mut Execution,
    ) -> Result<std::result::Result<Value, Vec<Value>>> {
        let any_typed = args.iter().any(is_typed);
        if !any_typed && !OWN.contains(&name) && name != "isa" {
            return Ok(Err(args));
        }
        if let Some(value) = self.typed_own(name, &args, out)? {
            return Ok(Ok(value));
        }
        if !any_typed {
            return Ok(Err(args));
        }
        if KEEP_ARGS.contains(&name) {
            return Ok(Err(args));
        }
        let class = args
            .iter()
            .find_map(typed)
            .map(TypedArray::class)
            .expect("typed");
        if class.is_integer() && FLOAT_ONLY.contains(&name) {
            return Err(class_error(name, class));
        }
        if name == "complex" || args.iter().any(|v| matches!(v, Value::Complex(_))) {
            return Err("Complex integer and complex single values are not supported yet".into());
        }
        self.charge(args.iter().filter_map(typed).map(TypedArray::len).sum())?;
        if PERMUTE.contains(&name)
            && let Some(source) = typed(&args[0]).cloned()
        {
            let markers = Matrix::from_column_vec(
                source.rows(),
                source.cols(),
                (1..=source.len()).map(|i| i as f64).collect(),
            )?;
            let mut rest = vec![Value::Matrix(markers)];
            for v in args.into_iter().skip(1) {
                rest.push(demote(v)?);
            }
            let placed = self.call(name, rest, out)?.matrix(&self.cancel)?;
            let positions: Vec<usize> = placed.data().iter().map(|&m| m as usize).collect();
            // Zero markers (tril/triu/diag fill) select a zero element.
            let zero = TypedArray::from_ints(source.class(), 1, 1, std::iter::once(0))?;
            let result = if positions.contains(&0) {
                let values = positions.iter().map(|&p| {
                    if p == 0 {
                        zero.int(0)
                    } else {
                        source.int(p - 1)
                    }
                });
                if source.is_integer() {
                    TypedArray::from_ints(source.class(), placed.rows(), placed.cols(), values)?
                } else {
                    TypedArray::from_reals(
                        source.class(),
                        placed.rows(),
                        placed.cols(),
                        positions
                            .iter()
                            .map(|&p| if p == 0 { 0. } else { source.real(p - 1) }),
                    )?
                }
            } else {
                let positions: Vec<usize> = positions.iter().map(|p| p - 1).collect();
                source.select(placed.rows(), placed.cols(), &positions)?
            };
            return Ok(Ok(typed_value(result)));
        }
        if matches!(name, "sum" | "prod") {
            if matches!(class, NumericClass::Int64 | NumericClass::Uint64)
                && args.iter().any(|v| text(v).as_deref() == Some("native"))
            {
                reject_wide(name, &args, &self.cancel)?;
                if let Some(a) = args.first().and_then(typed) {
                    // The legacy reducer uses doubles. A conservative magnitude
                    // bound keeps every integer intermediate exact, including
                    // cancellation; reject wider native reductions explicitly.
                    let mut bound = u128::from(name == "prod");
                    for i in 0..a.len() {
                        if i.is_multiple_of(8192) {
                            self.check_cancel()?;
                        }
                        let magnitude = a.int(i).unsigned_abs();
                        bound = if name == "prod" {
                            bound.saturating_mul(magnitude)
                        } else {
                            bound.saturating_add(magnitude)
                        };
                        if bound > (1u128 << 53) {
                            return Err(format!(
                                "{name} is not supported for {} values above 2^53 yet",
                                class.name()
                            ));
                        }
                    }
                }
            }
            let mut rest = Vec::new();
            let mut option = None;
            for v in args {
                match text(&v).as_deref() {
                    Some(o @ ("native" | "double" | "default")) => option = Some(o.to_owned()),
                    _ => rest.push(demote(v)?),
                }
            }
            let value = self.call(name, rest, out)?;
            let keep = match option.as_deref() {
                Some("native") => true,
                Some("double") => false,
                _ => !class.is_integer(),
            };
            return Ok(Ok(if keep { recast(value, class)? } else { value }));
        }
        if name == "det" && class == NumericClass::Single && args.len() == 1 {
            let a = typed(&args[0]).expect("typed");
            return Ok(Ok(typed_value(TypedArray::scalar_real(
                class,
                det_single(a, &self.cancel)?,
            ))));
        }
        if name == "eps" && class == NumericClass::Single && args.len() == 1 {
            let a = typed(&args[0]).expect("typed");
            return Ok(Ok(typed_value(TypedArray::from_reals(
                class,
                a.rows(),
                a.cols(),
                (0..a.len()).map(|i| f64::from(eps_single(a.real(i) as f32))),
            )?)));
        }
        if matches!(name, "bitand" | "bitor" | "bitxor" | "bitshift") {
            return self.typed_bits(name, &args, class).map(Ok);
        }
        if let Some(a) = args.first().and_then(typed).filter(|a| a.is_integer()) {
            if matches!(name, "cumsum" | "cumprod") {
                let dim = match args.as_slice() {
                    [_] => {
                        if a.rows() == 1 && a.cols() != 1 {
                            2
                        } else {
                            1
                        }
                    }
                    [_, d] => self.reduction_dimension(name, d)?,
                    _ => return Err(format!("{name} expects one or two inputs")),
                };
                return Ok(Ok(typed_value(a.cumulative_integer(
                    dim,
                    name == "cumprod",
                    &self.cancel,
                )?)));
            }
            if args.len() == 1
                && matches!(
                    name,
                    "abs" | "sign" | "floor" | "ceil" | "round" | "fix" | "real" | "conj"
                )
            {
                let values = (0..a.len()).map(|i| match name {
                    "abs" => a.int(i).abs(),
                    "sign" => a.int(i).signum(),
                    _ => a.int(i),
                });
                return Ok(Ok(typed_value(TypedArray::from_ints(
                    class,
                    a.rows(),
                    a.cols(),
                    values,
                )?)));
            }
            if name == "num2str" && args.len() == 1 && a.len() == 1 {
                return Ok(Ok(Value::Text(a.int(0).to_string())));
            }
        }
        if matches!(name, "max" | "min")
            && let Some(mut values) = self.typed_extreme(name, &args, 1, false)?
        {
            return Ok(Ok(values.remove(0)));
        }
        let keep = if class.is_integer() {
            INT_KEEP.contains(&name)
        } else {
            SINGLE_KEEP.contains(&name)
        };
        if class.is_integer() && keep {
            reject_wide(name, &args, &self.cancel)?;
        }
        let args = args.into_iter().map(demote).collect::<Result<Vec<_>>>()?;
        // Typed division and single inv retain the original Matrix policy;
        // warning-aware square solves apply only to real double arguments.
        if matches!(name, "mldivide" | "mrdivide") {
            let mut values = self.linalg2_call(name, &args, 1)?.unwrap_or_default();
            return Ok(Ok(values.remove(0)));
        }
        if !keep {
            return Ok(Err(args));
        }
        let value = if name == "inv" && args.len() == 1 {
            Value::Matrix(args[0].clone().matrix(&self.cancel)?.inverse()?)
        } else {
            self.call(name, args, out)?
        };
        Ok(Ok(recast(value, class)?))
    }

    /// Constructors, class queries and class-argument forms.
    fn typed_own(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if matches!(name, "zeros" | "ones" | "eye" | "rand" | "randn")
            && args.len() >= 2
            && text(&args[args.len() - 2]).as_deref() == Some("like")
        {
            let class = args
                .last()
                .expect("prototype")
                .clone()
                .array(&self.cancel)?
                .class()
                .to_owned();
            let mut values = args[..args.len() - 2].to_vec();
            values.push(Value::Text(class));
            return self.typed_own(name, &values, out);
        }
        if let Some(class) = NumericClass::parse(name) {
            return Ok(Some(match args {
                [] => typed_value(TypedArray::scalar_int(class, 0)),
                [v] => typed_value(convert(v, class)?),
                _ => return Err(format!("{name} expects one input")),
            }));
        }
        match name {
            "\u{1}int64_literal" | "\u{1}uint64_literal" => {
                let class = if name.starts_with("\u{1}int64") {
                    NumericClass::Int64
                } else {
                    NumericClass::Uint64
                };
                let digits = args.first().and_then(text).unwrap_or_default();
                let v = digits.parse::<i128>().unwrap_or(i128::MAX);
                Ok(Some(typed_value(TypedArray::scalar_int(class, v))))
            }
            "intmax" | "intmin" => {
                let class = match args {
                    [] => NumericClass::Int32,
                    [v] => {
                        let t = text(v).ok_or("Input must be a class name such as 'int8'")?;
                        NumericClass::parse(&t)
                            .filter(|c| c.is_integer())
                            .ok_or_else(|| format!("Invalid class name '{t}'."))?
                    }
                    _ => return Err(format!("{name} expects at most one input")),
                };
                let v = if name == "intmax" {
                    class.max()
                } else {
                    class.min()
                };
                Ok(Some(typed_value(TypedArray::scalar_int(class, v))))
            }
            "isinteger" | "isfloat" => {
                let [v] = args else {
                    return Err(format!("{name} expects one input"));
                };
                let t = typed(v).map(TypedArray::class);
                Ok(Some(boolean(if name == "isinteger" {
                    t.is_some_and(NumericClass::is_integer)
                } else {
                    t == Some(NumericClass::Single)
                        || matches!(v, Value::Matrix(_) | Value::Complex(_))
                })))
            }
            "isa" => {
                let [v, c] = args else {
                    return Ok(None);
                };
                let Some(c) = text(c) else {
                    return Ok(None);
                };
                let t = typed(v).map(TypedArray::class);
                let numeric = t.is_some() || matches!(v, Value::Matrix(_) | Value::Complex(_));
                Ok(Some(boolean(match c.as_str() {
                    "numeric" => numeric,
                    "integer" => t.is_some_and(NumericClass::is_integer),
                    "float" => {
                        t == Some(NumericClass::Single)
                            || matches!(v, Value::Matrix(_) | Value::Complex(_))
                    }
                    other => v
                        .clone()
                        .array(&self.cancel)
                        .is_ok_and(|a| a.class() == other),
                })))
            }
            "cast" => {
                let [v, c] = args else {
                    return Err("cast expects a value and a class name".into());
                };
                let c = text(c).ok_or("cast: the class must be text")?;
                if let Some(class) = NumericClass::parse(&c) {
                    return Ok(Some(typed_value(convert(v, class)?)));
                }
                let v = demote(v.clone())?;
                match c.as_str() {
                    "double" => Ok(Some(Value::Matrix(v.matrix(&self.cancel)?))),
                    "logical" | "char" => Ok(Some(self.call(&c, vec![v], out)?)),
                    _ => Err(format!("cast: unsupported class '{c}'")),
                }
            }
            "idivide" => self.typed_idivide(args).map(Some),
            // Double arguments keep the existing `bitcmp(A, ASSUMEDTYPE)` kernel.
            "bitcmp"
                if args
                    .first()
                    .and_then(typed)
                    .is_some_and(TypedArray::is_integer) =>
            {
                let a = typed(&args[0]).expect("typed");
                if args.len() != 1 {
                    return Err("bitcmp of an integer array takes one input".into());
                }
                let class = a.class();
                let values = (0..a.len()).map(|i| {
                    let x = a.int(i);
                    if class.is_signed() {
                        !x
                    } else {
                        class.max() - x
                    }
                });
                Ok(Some(typed_value(TypedArray::from_ints(
                    class,
                    a.rows(),
                    a.cols(),
                    values,
                )?)))
            }
            "typecast" => {
                let [v, c] = args else {
                    return Err("typecast expects a value and a class name".into());
                };
                let c = text(c).ok_or("typecast: the class must be text")?;
                let (rows, cols, bytes) =
                    match v {
                        Value::Container(Array::Typed(t)) => (t.rows(), t.cols(), t.to_le_bytes()),
                        Value::Matrix(m) => (
                            m.rows(),
                            m.cols(),
                            m.data().iter().flat_map(|x| x.to_le_bytes()).collect(),
                        ),
                        _ => return Err(
                            "The first input argument must be a full, non-complex numeric value."
                                .into(),
                        ),
                    };
                if rows > 1 && cols > 1 {
                    return Err("The first input argument must be a vector.".into());
                }
                let size = NumericClass::parse(&c).map_or(8, NumericClass::size);
                if c != "double" && NumericClass::parse(&c).is_none() {
                    return Err(format!("Unsupported class '{c}'"));
                }
                if bytes.len() % size != 0 {
                    return Err("Too few input values to make output type.".into());
                }
                let n = bytes.len() / size;
                let (r, k) = if rows > 1 { (n, 1) } else { (1, n) };
                Ok(Some(match NumericClass::parse(&c) {
                    Some(class) => typed_value(TypedArray::from_le_bytes(class, r, k, &bytes)?),
                    None => Value::Matrix(Matrix::from_column_vec(
                        r,
                        k,
                        bytes
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|b| f64::from_le_bytes(*b))
                            .collect(),
                    )?),
                }))
            }
            "realmax" | "realmin" | "eps" | "flintmax"
                if args.len() == 1 && text(&args[0]).is_some() =>
            {
                let c = text(&args[0]).expect("text");
                let v = match (name, c.as_str()) {
                    ("realmax", "single") => f64::from(f32::MAX),
                    ("realmin", "single") => f64::from(f32::MIN_POSITIVE),
                    ("eps", "single") => f64::from(f32::EPSILON),
                    ("flintmax", "single") => 16777216.,
                    // 'double' and invalid names keep the existing kernel and its errors.
                    _ => return Ok(None),
                };
                Ok(Some(typed_value(TypedArray::scalar_real(
                    NumericClass::Single,
                    v,
                ))))
            }
            "zeros" | "ones" | "eye" | "rand" | "randn"
                if args
                    .last()
                    .and_then(text)
                    .is_some_and(|c| c == "double" || NumericClass::parse(&c).is_some()) =>
            {
                let c = text(args.last().expect("class")).expect("text");
                let rest = args[..args.len() - 1]
                    .iter()
                    .cloned()
                    .map(demote)
                    .collect::<Result<Vec<_>>>()?;
                let value = self.call(name, rest, out)?;
                match NumericClass::parse(&c) {
                    Some(class) => {
                        if matches!(name, "rand" | "randn") && class.is_integer() {
                            return Err(format!("{name} does not support class '{c}'"));
                        }
                        Ok(Some(recast(value, class)?))
                    }
                    None => Ok(Some(value)),
                }
            }
            _ => Ok(None),
        }
    }

    fn typed_idivide(&mut self, args: &[Value]) -> Result<Value> {
        let (a, b, mode) = match args {
            [a, b] => (a, b, "fix".to_owned()),
            [a, b, m] => (
                a,
                b,
                text(m).ok_or("idivide: the rounding option must be text")?,
            ),
            _ => return Err("idivide expects two or three inputs".into()),
        };
        if !matches!(mode.as_str(), "fix" | "floor" | "ceil" | "round") {
            return Err(format!("idivide: unknown rounding option '{mode}'"));
        }
        let class = [a, b]
            .iter()
            .find_map(|v| typed(v).filter(|t| t.is_integer()))
            .map(TypedArray::class)
            .ok_or("At least one argument must belong to an integer class.")?;
        if [a, b].iter().any(|v| match v {
            Value::Container(Array::Typed(t)) => t.class() != class,
            Value::Matrix(m) => m.len() != 1,
            _ => true,
        }) {
            return Err(
                "Integers can only be combined with integers of the same class, or scalar doubles."
                    .into(),
            );
        }
        let (x, y) = (convert(a, class)?, convert(b, class)?);
        let (rows, cols) = if x.len() == 1 {
            (y.rows(), y.cols())
        } else if y.len() == 1 || (x.rows(), x.cols()) == (y.rows(), y.cols()) {
            (x.rows(), x.cols())
        } else {
            return Err("Matrix dimensions must agree.".into());
        };
        let n = Matrix::checked_len(rows, cols)?;
        self.charge(n)?;
        let pick = |len: usize, i: usize| if len == 1 { 0 } else { i };
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(8192) {
                self.check_cancel()?;
            }
            let (p, q) = (x.int(pick(x.len(), i)), y.int(pick(y.len(), i)));
            values.push(if q == 0 || mode == "round" {
                divide_round(p, q, class)
            } else {
                let t = p / q;
                let r = p % q;
                match mode.as_str() {
                    "fix" => t,
                    "floor" if r != 0 && ((r < 0) != (q < 0)) => t - 1,
                    "floor" => t,
                    "ceil" if r != 0 && ((r < 0) == (q < 0)) => t + 1,
                    "ceil" => t,
                    _ => return Err(format!("idivide: unknown rounding option '{mode}'")),
                }
            });
        }
        Ok(typed_value(TypedArray::from_ints(
            class,
            rows,
            cols,
            values.into_iter(),
        )?))
    }

    fn typed_bits(&mut self, name: &str, args: &[Value], class: NumericClass) -> Result<Value> {
        if !class.is_integer() {
            return Err(class_error(name, class));
        }
        let [a, b] = args else {
            return Err(format!("{name} expects two inputs"));
        };
        let bits = class.size() as u32 * 8;
        let x = convert(a, class)?;
        let (y_len, y): (usize, Box<dyn Fn(usize) -> i128>) = if name == "bitshift" {
            let m = demote(b.clone())?.matrix(&self.cancel)?;
            let data = m.data().to_vec();
            (data.len(), Box::new(move |i| data[i] as i128))
        } else {
            let t = convert(b, class)?;
            (t.len(), Box::new(move |i| t.int(i)))
        };
        let y_shape = b.shape(&self.cancel)?;
        let x_shape = (x.rows(), x.cols());
        let (rows, cols) = if x.len() == 1 {
            y_shape
        } else if y_len == 1 || x_shape == y_shape {
            x_shape
        } else {
            return Err("Matrix dimensions must agree.".into());
        };
        let n = Matrix::checked_len(rows, cols)?;
        self.charge(n)?;
        let pick = |len: usize, i: usize| if len == 1 { 0 } else { i };
        let mask: i128 = (1i128 << bits) - 1;
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(8192) {
                self.check_cancel()?;
            }
            let (p, q) = (x.int(pick(x.len(), i)), y(pick(y_len, i)));
            values.push(match name {
                "bitand" => p & q,
                "bitor" => p | q,
                "bitxor" => p ^ q,
                _ => {
                    if q <= 0 {
                        p >> q.unsigned_abs().min(127) as u32
                    } else {
                        let shifted = (p << q.min(bits as i128) as u32) & mask;
                        if class.is_signed() && shifted >> (bits - 1) == 1 {
                            shifted - (1i128 << bits)
                        } else {
                            shifted
                        }
                    }
                }
            });
        }
        Ok(typed_value(TypedArray::from_ints(
            class,
            rows,
            cols,
            values.into_iter(),
        )?))
    }
}
