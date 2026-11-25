//! Sparse builtins, operators, indexing and the dispatch hooks. Storage and
//! kernels live in `crate::sparse`; this module connects them to the engine.
//!
//! Dispatch: when no argument is sparse every hook returns at once. A sparse
//! argument reaches the functions named in [`SPARSE_AWARE`]; any other
//! builtin receives full copies of its sparse arguments (bounded by the
//! engine's sparse-to-full limit) and returns what the full function returns.
use super::{Engine, Execution, Value, indexing::Indices};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
    function_handle::FunctionHandle,
    sparse::{self, Rows, SparseMatrix},
    syntax::{AnonymousFunction, Expr, Token},
};
use std::sync::Arc;

/// Functions that handle sparse arguments themselves (or, for disp, display,
/// whos and save, receive them unchanged).
pub(super) const SPARSE_AWARE: &[&str] = &[
    "sparse",
    "issparse",
    "full",
    "speye",
    "spalloc",
    "zeros",
    "ones",
    "spones",
    "spdiags",
    "sprand",
    "sprandn",
    "sprandsym",
    "spfun",
    "amd",
    "colamd",
    "symamd",
    "symrcm",
    "nnz",
    "nzmax",
    "nonzeros",
    "find",
    "sum",
    "max",
    "min",
    "any",
    "all",
    "abs",
    "round",
    "floor",
    "ceil",
    "fix",
    "sqrt",
    "cat",
    "horzcat",
    "vertcat",
    "size",
    "numel",
    "ndims",
    "length",
    "isempty",
    "isscalar",
    "isvector",
    "isrow",
    "iscolumn",
    "ismatrix",
    "issquare",
    "class",
    "isa",
    "isreal",
    "islogical",
    "isnumeric",
    "isfloat",
    "isequal",
    "isequaln",
    "disp",
    "display",
    "whos",
    "save",
    "double",
    "logical",
    "reshape",
    "plus",
    "minus",
    "times",
    "rdivide",
    "ldivide",
    "mtimes",
    "mrdivide",
    "mldivide",
    "and",
    "or",
    "not",
    "transpose",
    "ctranspose",
    "uminus",
    "uplus",
    "kron",
    "ichol",
    "ilu",
];
/// Handled for full input too.
const CONSTRUCTORS: &[&str] = &[
    "sparse",
    "issparse",
    "speye",
    "spalloc",
    "spdiags",
    "nzmax",
    "spones",
    "gallery",
    "sprand",
    "sprandn",
    "sprandsym",
    "spfun",
    "amd",
    "colamd",
    "symamd",
    "symrcm",
];
/// The normal implementation shows or stores the sparse value itself;
/// decomposition rejects it rather than keep a full copy.
const PASS_THROUGH: &[&str] = &["disp", "display", "whos", "save", "decomposition"];
const TEMP_PREFIX: &str = "\u{1}sparse_argument_";

/// Per-engine sparse settings.
#[derive(Clone, Debug)]
pub(super) struct SparseState {
    /// Largest element count a sparse value may be converted to full.
    full_limit: usize,
    /// A sparse value was created or stored in this engine.
    pub(super) seen: bool,
    temp_counter: usize,
}
impl Default for SparseState {
    fn default() -> Self {
        Self {
            full_limit: crate::matrix::MAX_ELEMENTS,
            seen: false,
            temp_counter: 0,
        }
    }
}

impl Engine {
    /// Lower the sparse-to-full conversion limit (tests use a tiny value).
    #[doc(hidden)]
    pub fn set_sparse_full_limit(&mut self, n: usize) {
        self.sparse.full_limit = n;
    }
}

pub(super) fn is_sparse(v: &Value) -> bool {
    matches!(v, Value::Container(Array::Sparse(_)))
}
fn as_sparse(v: &Value) -> Option<&SparseMatrix> {
    match v {
        Value::Container(Array::Sparse(s)) => Some(s),
        _ => None,
    }
}
pub(super) fn wrap(s: SparseMatrix) -> Value {
    Value::Container(Array::Sparse(s))
}
fn logical_scalar(b: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(b))
}
fn number(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}
fn unsupported_input(name: &str) -> String {
    format!("{name}: only double and logical input is supported yet")
}

/// Full copy of a sparse value, bounded by the engine's limit.
/// Largest element count a sparse value may be converted to full.
pub(super) fn full_limit(engine: &Engine) -> usize {
    engine.sparse.full_limit
}

fn full(engine: &Engine, name: &str, s: &SparseMatrix) -> Result<Value> {
    let limit = engine.sparse.full_limit;
    if s.numel() > limit as u64 {
        return Err(format!(
            "{name}: sparse input with more than {limit} elements cannot be converted to full yet"
        ));
    }
    Ok(if s.is_logical() {
        Value::Logical(s.to_dense_logical(&engine.cancel)?)
    } else {
        Value::Matrix(s.to_dense(&engine.cancel)?)
    })
}
/// Full copy of a sparse value under the engine's limit; other values unchanged.
pub(super) fn densify(engine: &Engine, name: &str, v: Value) -> Result<Value> {
    match &v {
        Value::Container(Array::Sparse(s)) => full(engine, name, s),
        _ => Ok(v),
    }
}
/// Constructor prototypes describe the requested storage. Only their size
/// arguments should go through the usual conversion to full.
fn full_arguments(engine: &Engine, name: &str, mut args: Vec<Value>) -> Result<Vec<Value>> {
    let keep_prototype = matches!(name, "zeros" | "ones")
        && matches!(args.as_slice(), [.., option, Value::Container(Array::Sparse(_))]
            if text(engine, option).as_deref() == Some("like"));
    let count = args.len() - usize::from(keep_prototype);
    for arg in &mut args[..count] {
        if let Some(s) = as_sparse(arg) {
            *arg = full(engine, name, s)?;
        }
    }
    Ok(args)
}

/// Sparse copy of a full double or logical value.
pub(super) fn to_sparse(engine: &Engine, name: &str, v: &Value) -> Result<SparseMatrix> {
    match v {
        Value::Container(Array::Sparse(s)) => Ok(s.clone()),
        Value::Matrix(m) if !m.is_nd() => SparseMatrix::from_dense(m, &engine.cancel),
        Value::Logical(m) if !m.is_nd() => SparseMatrix::from_logical(m, &engine.cancel),
        Value::Matrix(_) | Value::Logical(_) => Err(format!(
            "{name}: N-D input is not supported for sparse arrays"
        )),
        _ => Err(unsupported_input(name)),
    }
}
fn text(engine: &Engine, v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) => c.to_utf8(&engine.cancel).ok(),
        _ => None,
    }
}
/// Real scalar of a full or sparse 1x1 value.
fn scalar(engine: &Engine, name: &str, v: &Value) -> Result<f64> {
    if let Some(s) = as_sparse(v) {
        if (s.rows(), s.cols()) != (1, 1) {
            return Err(format!("{name}: expected a scalar"));
        }
        return Ok(s.get(0, 0));
    }
    if v.dims().iter().product::<usize>() != 1 || matches!(v, Value::Complex(_)) {
        return Err(format!("{name}: expected a real scalar"));
    }
    v.clone().matrix(&engine.cancel)?.as_scalar()
}
fn count_arg(engine: &Engine, name: &str, v: &Value) -> Result<usize> {
    let x = scalar(engine, name, v)?;
    if !x.is_finite() || x < 0. || x.fract() != 0. || x >= 2f64.powi(53) {
        return Err(format!("{name}: sizes must be nonnegative integers"));
    }
    Ok(x as usize)
}
fn is_scalar_value(v: &Value) -> bool {
    match v {
        Value::Container(Array::Sparse(s)) => (s.rows(), s.cols()) == (1, 1),
        _ => v.dims().iter().product::<usize>() == 1,
    }
}
fn dims2(v: &Value) -> (usize, usize) {
    match v {
        Value::Container(Array::Sparse(s)) => (s.rows(), s.cols()),
        _ => {
            let d = v.dims();
            (d[0], d[1..].iter().product())
        }
    }
}
/// First dimension that is not 1 (dimension 1 for a scalar).
fn default_dim(s: &SparseMatrix) -> usize {
    if s.rows() != 1 { 1 } else { 2 }
}
fn dim_arg(engine: &Engine, name: &str, v: &Value) -> Result<usize> {
    let d = scalar(engine, name, v)?;
    if !d.is_finite() || d < 1. || d.fract() != 0. {
        return Err(format!(
            "{name}: dimension must be a positive integer scalar"
        ));
    }
    Ok(d as usize)
}
/// Full double data of a full real value, for arithmetic with a sparse one.
fn real_full(engine: &Engine, v: &Value) -> Option<Result<Matrix>> {
    match v {
        Value::Matrix(m) if !m.is_nd() => Some(Ok(m.clone())),
        Value::Logical(m) if !m.is_nd() => Some(m.to_numeric(&engine.cancel)),
        _ => None,
    }
}

/// Evaluate `a op b` through the general operator code.
fn operate(
    engine: &mut Engine,
    op: Token,
    a: Value,
    b: Value,
    out: &mut Execution,
) -> Result<Value> {
    let name = |n: &str| Box::new(Expr::Name(n.into()));
    let function = AnonymousFunction {
        params: vec!["a".into(), "b".into()],
        body: Expr::Binary(op, name("a"), name("b")),
        text: "@(a,b)a op b".into(),
    };
    let handle = FunctionHandle::anonymous(Arc::new(function), Vec::new());
    let mut values = engine.call_handle(&handle, vec![a, b], 1, None, out)?;
    Ok(if values.is_empty() {
        Value::Void
    } else {
        values.remove(0)
    })
}
fn operator_name(op: &Token) -> &'static str {
    match op {
        Token::Plus => "plus",
        Token::Minus => "minus",
        Token::DotStar => "times",
        Token::Star => "mtimes",
        Token::DotSlash => "rdivide",
        Token::DotBackslash => "ldivide",
        Token::Slash => "mrdivide",
        Token::Backslash => "mldivide",
        Token::DotPower => "power",
        Token::Power => "mpower",
        _ => "operator",
    }
}

// ---------------------------------------------------------------- operators

/// `a op b` for arithmetic operators when either operand is sparse.
pub(super) fn binary(
    engine: &mut Engine,
    op: &Token,
    a: Value,
    b: Value,
    out: &mut Execution,
) -> Result<Value> {
    let name = operator_name(op);
    let cancel = engine.cancel.clone();
    let dense_fallback = |engine: &mut Engine, a: Value, b: Value, out: &mut Execution| {
        let a = densify(engine, name, a)?;
        let b = densify(engine, name, b)?;
        operate(engine, op.clone(), a, b, out)
    };
    if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
        return Err(format!(
            "{name}: complex values with sparse arrays are not supported yet"
        ));
    }
    let (sa, sb) = (as_sparse(&a).cloned(), as_sparse(&b).cloned());
    // Operands other than full double/logical take the full path.
    if (sa.is_none() && real_full(engine, &a).is_none())
        || (sb.is_none() && real_full(engine, &b).is_none())
    {
        return dense_fallback(engine, a, b, out);
    }
    let (a_scalar, b_scalar) = (is_scalar_value(&a), is_scalar_value(&b));
    let (da, db) = (dims2(&a), dims2(&b));
    let result = match op {
        Token::Plus | Token::Minus => {
            let sign = if op == &Token::Plus { 1. } else { -1. };
            match (&sa, &sb) {
                (Some(x), Some(y)) if da == db => x.add(y, sign)?,
                _ => return dense_fallback(engine, a, b, out),
            }
        }
        Token::DotStar | Token::Star => {
            let matrix = op == &Token::Star;
            match (&sa, &sb) {
                // S*scalar, S.*scalar (scalar full or sparse): stored values only.
                (Some(x), _) if b_scalar => {
                    let k = scalar(engine, name, &b)?;
                    x.map(|v| v * k, false)
                }
                (_, Some(y)) if a_scalar => {
                    let k = scalar(engine, name, &a)?;
                    y.map(|v| k * v, false)
                }
                // 1x1 sparse * full non-scalar (or the reverse): full, as S*F.
                (Some(_), None) | (None, Some(_)) if matrix && (a_scalar || b_scalar) => {
                    return dense_fallback(engine, a, b, out);
                }
                (Some(x), Some(y)) if matrix => x.matmul(y, &cancel)?,
                (Some(x), Some(y)) if da == db => x.times_sparse(y)?,
                (Some(x), None) if matrix => {
                    let f = real_full(engine, &b).unwrap()?;
                    return Ok(Value::Matrix(x.matmul_dense(&f, &cancel)?));
                }
                (None, Some(y)) if matrix => {
                    let f = real_full(engine, &a).unwrap()?;
                    return Ok(Value::Matrix(SparseMatrix::dense_matmul(&f, y, &cancel)?));
                }
                (Some(x), None) if da == db => {
                    x.times_dense(&real_full(engine, &b).unwrap()?, &cancel)?
                }
                (None, Some(y)) if da == db => {
                    y.times_dense(&real_full(engine, &a).unwrap()?, &cancel)?
                }
                (Some(x), None) if a_scalar && !matrix => {
                    // sparse scalar .* full array: sparse result.
                    let k = x.get(0, 0);
                    let f = real_full(engine, &b).unwrap()?;
                    SparseMatrix::from_dense(&f.map(|v| k * v), &cancel)?
                }
                (None, Some(y)) if b_scalar && !matrix => {
                    let k = y.get(0, 0);
                    let f = real_full(engine, &a).unwrap()?;
                    SparseMatrix::from_dense(&f.map(|v| v * k), &cancel)?
                }
                _ => return dense_fallback(engine, a, b, out),
            }
        }
        Token::Backslash | Token::Slash
            if !(a_scalar && op == &Token::Backslash) && !(b_scalar && op == &Token::Slash) =>
        {
            let backslash = op == &Token::Backslash;
            match super::sparse_solve::divide(engine, backslash, &a, &b, out)? {
                Some(v) => return Ok(v),
                None if sa.is_some() && sb.is_some() => {
                    // sparse \ sparse stays sparse on the dense path too.
                    return match dense_fallback(engine, a, b, out)? {
                        Value::Matrix(m) if !m.is_nd() => {
                            Ok(wrap(SparseMatrix::from_dense(&m, &engine.cancel)?))
                        }
                        other => Ok(other),
                    };
                }
                None => return dense_fallback(engine, a, b, out),
            }
        }
        Token::Slash | Token::DotSlash if b_scalar && sa.is_some() => {
            let k = scalar(engine, name, &b)?;
            sa.unwrap().map(|v| v / k, false)
        }
        Token::Backslash | Token::DotBackslash if a_scalar && sb.is_some() => {
            let k = scalar(engine, name, &a)?;
            sb.unwrap().map(|v| v / k, false)
        }
        _ => return dense_fallback(engine, a, b, out),
    };
    Ok(wrap(result))
}

/// `-S` and `+S`.
pub(super) fn unary(s: &SparseMatrix, negative: bool) -> Value {
    wrap(if negative {
        s.map(|v| -v, false)
    } else {
        s.to_double()
    })
}
/// `S'` and `S.'` (real storage: both transposes agree).
pub(super) fn transpose(s: &SparseMatrix) -> Result<Value> {
    Ok(wrap(s.transpose()?))
}

/// Operands of a comparison or logical operator; `Ok(Err(..))` gives them
/// back (full copies when sparse) for the general path.
pub(super) fn logical_binary(
    engine: &mut Engine,
    op: &Token,
    left: Value,
    right: Value,
) -> Result<std::result::Result<Value, (Value, Value)>> {
    if !is_sparse(&left) && !is_sparse(&right) {
        return Ok(Err((left, right)));
    }
    if op == &Token::And {
        let (l_scalar, r_scalar) = (is_scalar_value(&left), is_scalar_value(&right));
        let (dl, dr) = (dims2(&left), dims2(&right));
        let nan_check = |engine: &Engine, v: &Value| -> Result<()> {
            match v {
                Value::Container(Array::Sparse(s)) => {
                    s.to_logical()?;
                }
                Value::Matrix(m) => {
                    LogicalMatrix::from_numeric(m, &engine.cancel)?;
                }
                _ => {}
            }
            Ok(())
        };
        let simple = |v: &Value| {
            is_sparse(v)
                || matches!(v, Value::Matrix(m) if !m.is_nd())
                || matches!(v, Value::Logical(m) if !m.is_nd())
        };
        if simple(&left) && simple(&right) && (dl == dr || l_scalar || r_scalar) {
            nan_check(engine, &left)?;
            nan_check(engine, &right)?;
            let (s, other) = match (&left, &right) {
                (Value::Container(Array::Sparse(s)), o)
                    if !(l_scalar && !r_scalar) || !is_sparse(o) =>
                {
                    (s.clone(), right.clone())
                }
                (_, Value::Container(Array::Sparse(s))) => (s.clone(), left.clone()),
                _ => unreachable!(),
            };
            let other_scalar = is_scalar_value(&other);
            let s_scalar = (s.rows(), s.cols()) == (1, 1);
            if s_scalar && !other_scalar {
                // 1x1 sparse & full or sparse array: the pattern of the other operand.
                let t = to_sparse(engine, "and", &other)?;
                let keep = s.get(0, 0) != 0.;
                return Ok(Ok(wrap(t.and_with(|_, _| f64::from(u8::from(keep)))?)));
            }
            if other_scalar {
                let k = scalar(engine, "and", &other)?;
                return Ok(Ok(wrap(s.and_with(|_, _| k)?)));
            }
            let result = match &other {
                Value::Container(Array::Sparse(t)) => s.and_with(|r, c| t.get(r, c))?,
                _ => {
                    let f = real_full(engine, &other).unwrap()?;
                    s.and_with(|r, c| f.data()[c * f.rows() + r])?
                }
            };
            return Ok(Ok(wrap(result)));
        }
    }
    let name = match op {
        Token::And => "and",
        Token::Or => "or",
        Token::Equal => "eq",
        Token::NotEqual => "ne",
        Token::Less => "lt",
        Token::LessEqual => "le",
        Token::Greater => "gt",
        _ => "ge",
    };
    Ok(Err((
        densify(engine, name, left)?,
        densify(engine, name, right)?,
    )))
}

/// A short-circuit operand: a 1x1 sparse value becomes full; others must be scalars.
pub(super) fn short_circuit_operand(engine: &Engine, op: &Token, v: Value) -> Result<Value> {
    match &v {
        Value::Container(Array::Sparse(s)) => {
            if (s.rows(), s.cols()) != (1, 1) {
                return Err("Short-circuit operands must be logical scalars".into());
            }
            full(engine, if op == &Token::ShortOr { "||" } else { "&&" }, s)
        }
        _ => Ok(v),
    }
}

/// `[a, b; c, d]` with at least one sparse block: sparse result.
pub(super) fn concatenate(engine: &mut Engine, rows: Vec<Vec<Value>>) -> Result<Value> {
    let mut stacked = Vec::new();
    for row in rows {
        let mut blocks = Vec::new();
        for v in row {
            engine.check_cancel()?;
            if matches!(&v, Value::Matrix(m) if (m.rows(), m.cols()) == (0, 0)) {
                continue;
            }
            if v.is_character()
                || super::string_class::is_string(&v)
                || matches!(v, Value::Container(Array::Cell(_) | Array::Struct(_)))
            {
                return Err(
                    "sparse: concatenation with char, cell, string or struct values is not supported"
                        .into(),
                );
            }
            blocks.push(to_sparse(engine, "sparse", &v)?);
        }
        if !blocks.is_empty() {
            stacked.push(SparseMatrix::hcat(&blocks)?);
        }
    }
    if stacked.is_empty() {
        return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, vec![])?));
    }
    Ok(wrap(SparseMatrix::vcat(&stacked)?))
}

// ---------------------------------------------------------------- dispatch

/// Builtin hook at the top of `Engine::call`. `Ok(Err(args))` continues the
/// normal cascade with these (full where needed) arguments.
pub(super) fn call(
    engine: &mut Engine,
    name: &str,
    args: Vec<Value>,
    out: &mut Execution,
) -> Result<std::result::Result<Value, Vec<Value>>> {
    if super::sparse_precond::handles(name) {
        return Ok(Ok(
            super::sparse_precond::call(engine, name, &args, 1)?.remove(0)
        ));
    }
    if !args.iter().any(is_sparse) && !CONSTRUCTORS.contains(&name) {
        return Ok(Err(args));
    }
    if name == "num2cell" {
        return if args.first().is_some_and(is_sparse) {
            Ok(Ok(super::strings::sparse_num2cell(engine, &args)?))
        } else {
            Ok(Err(args))
        };
    }
    // Iterative solvers take sparse A, M1 and M2 as they are.
    if PASS_THROUGH.contains(&name) || super::iter_solvers::slv_is_solver(name) {
        return Ok(Err(args));
    }
    if (SPARSE_AWARE.contains(&name) || CONSTRUCTORS.contains(&name))
        && let Some(value) = builtin(engine, name, &args, out)?
    {
        return Ok(Ok(value));
    }
    Ok(Err(full_arguments(engine, name, args)?))
}

/// Can evaluating `e` give a sparse value? Decided from the expression and the
/// variables it names, never from what the session did before. A sparse value
/// comes from a sparse variable, a sparse constructor, an aware function
/// applied to one, a user function or handle (which may build one inside), or
/// a read of a container that holds one.
fn may_yield_sparse(engine: &mut Engine, e: &Expr) -> bool {
    match e {
        Expr::Number(_)
        | Expr::Imaginary(_)
        | Expr::NameValue(_)
        | Expr::String(_)
        | Expr::DoubleQuoted(_)
        | Expr::All
        | Expr::FunctionHandle(_)
        | Expr::Anonymous(_)
        | Expr::CellLiteral(_)
        | Expr::Range(..) => false,
        Expr::Unary(_, a) | Expr::Not(a) | Expr::Transpose(a) | Expr::NonConjugateTranspose(a) => {
            may_yield_sparse(engine, a)
        }
        Expr::Binary(_, a, b) => may_yield_sparse(engine, a) || may_yield_sparse(engine, b),
        Expr::Matrix(rows) => rows.iter().flatten().any(|a| may_yield_sparse(engine, a)),
        Expr::Name(n) if engine.contains_array(n) => {
            matches!(engine.container_variables.get(n), Some(Array::Sparse(_)))
        }
        Expr::Name(n) => named_may_yield_sparse(engine, n, &[]),
        Expr::Call(n, args) => named_may_yield_sparse(engine, n, args),
        // A handle stored in a container can be called here.
        Expr::Index(..) | Expr::Apply(..) => true,
        // Plain reads of a container: only a stored sparse value can come out.
        Expr::CellContents(..) | Expr::Field(..) | Expr::DynamicField(..) | Expr::Brace(..) => {
            engine.sparse.seen
        }
    }
}

fn named_may_yield_sparse(engine: &mut Engine, name: &str, args: &[Expr]) -> bool {
    if engine.contains_array(name) {
        // Indexing a sparse variable, or calling a handle variable.
        return matches!(
            engine.container_variables.get(name),
            Some(Array::Sparse(_) | Array::Handle(_))
        );
    }
    if CONSTRUCTORS.contains(&name) || name == "feval" {
        return true;
    }
    if !matches!(engine.resolve_function(name), Ok(None)) {
        return true;
    }
    SPARSE_AWARE.contains(&name) && args.iter().any(|a| may_yield_sparse(engine, a))
}

/// True when evaluating `e` may call a user function or a function handle.
fn runs_user_code(engine: &mut Engine, e: &Expr) -> bool {
    match e {
        Expr::Unary(_, a) | Expr::Not(a) | Expr::Transpose(a) | Expr::NonConjugateTranspose(a) => {
            runs_user_code(engine, a)
        }
        Expr::Binary(_, a, b) => runs_user_code(engine, a) || runs_user_code(engine, b),
        Expr::Matrix(rows) => rows.iter().flatten().any(|a| runs_user_code(engine, a)),
        Expr::Name(n) => {
            !engine.contains_array(n) && !matches!(engine.resolve_function(n), Ok(None))
        }
        Expr::Call(n, args) => {
            if engine.contains_array(n) {
                matches!(engine.container_variables.get(n), Some(Array::Handle(_)))
                    || args.iter().any(|a| runs_user_code(engine, a))
            } else {
                n == "feval"
                    || !matches!(engine.resolve_function(n), Ok(None))
                    || args.iter().any(|a| runs_user_code(engine, a))
            }
        }
        Expr::Index(..) | Expr::Apply(..) => true,
        _ => false,
    }
}

/// Hook at the top of the multi-output dispatcher. Arguments are evaluated
/// once here when one of them can be sparse; full arguments are handed on
/// through hidden variables so the normal dispatcher does not evaluate them
/// again. The hidden variables exist only for the duration of that call.
pub(super) fn outputs(
    engine: &mut Engine,
    expression: &Expr,
    targets: &[Option<String>],
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    let Expr::Call(name, args) = expression else {
        return Ok(None);
    };
    // The regular dispatcher evaluates single outputs and rejects excessive
    // output counts. Let its expression path handle num2cell's sparse input.
    if name == "num2cell" {
        return Ok(None);
    }
    // The hidden-argument call below names only hidden variables: dispatch it normally.
    if !args.is_empty()
        && args
            .iter()
            .all(|a| matches!(a, Expr::Name(n) if n.starts_with(TEMP_PREFIX)))
    {
        return Ok(None);
    }
    // ichol/ilu: every output count, and their errors for full input.
    if super::sparse_precond::handles(name)
        && !engine.contains_array(name)
        && engine.resolve_function(name)?.is_none()
    {
        let values = super::string_class::arguments(engine, name, args, out)?;
        let count = targets.len();
        return Ok(Some(super::sparse_precond::call(
            engine, name, &values, count,
        )?));
    }
    if engine.contains_array(name) || !args.iter().any(|a| may_yield_sparse(engine, a)) {
        return Ok(None);
    }
    if engine.resolve_function(name)?.is_some() {
        return Ok(None);
    }
    // The dispatcher rejects unsupported output counts before any argument
    // runs user code; keep that order for builtins that are not sparse-aware.
    if !SPARSE_AWARE.contains(&name.as_str()) && args.iter().any(|a| runs_user_code(engine, a)) {
        return Ok(None);
    }
    let mut values = super::string_class::arguments(engine, name, args, out)?;
    let count = targets.len();
    if values.iter().any(is_sparse)
        && let Some(outputs) = multi_builtin(engine, name, &values, count, out)?
    {
        return Ok(Some(outputs));
    }
    if values.iter().any(is_sparse) && !super::iter_solvers::slv_is_solver(name) {
        values = full_arguments(engine, name, values)?;
    }
    let mut names = Vec::with_capacity(values.len());
    let result = dispatch_with_hidden_arguments(engine, name, values, targets, out, &mut names);
    // Removed on every path, success or error.
    for n in &names {
        engine.remove_array(n);
    }
    result.map(Some)
}

fn dispatch_with_hidden_arguments(
    engine: &mut Engine,
    name: &str,
    values: Vec<Value>,
    targets: &[Option<String>],
    out: &mut Execution,
    names: &mut Vec<String>,
) -> Result<Vec<Value>> {
    for value in values {
        let array = engine.assigned_array(value)?;
        engine.sparse.temp_counter += 1;
        let temp = format!("{TEMP_PREFIX}{}", engine.sparse.temp_counter);
        engine.publish_array(temp.clone(), array);
        names.push(temp);
    }
    let call = Expr::Call(
        name.to_string(),
        names.iter().map(|n| Expr::Name(n.clone())).collect(),
    );
    engine.eval_outputs(&call, targets, out)
}

/// Names of the hidden argument variables never reach the user.
pub(super) fn is_hidden_name(name: &str) -> bool {
    name.starts_with(TEMP_PREFIX)
}

/// Sparse functions with several outputs.
fn multi_builtin(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    let Some(s) = args.first().and_then(as_sparse).cloned() else {
        return Ok(None);
    };
    match (name, args.len()) {
        ("size", 1) => {
            let mut values = vec![number(s.rows() as f64), number(s.cols() as f64)];
            if count == 1 {
                return Ok(Some(vec![Value::Matrix(Matrix::from_column_slice(
                    1,
                    2,
                    &[s.rows() as f64, s.cols() as f64],
                )?)]));
            }
            if count < 2 {
                return Ok(None);
            }
            values.resize(count, number(1.));
            Ok(Some(values))
        }
        ("find", 1) if (1..=3).contains(&count) => Ok(Some(find(engine, &s, count, out)?)),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------- builtins

fn builtin(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    let first = args.first().and_then(as_sparse).cloned();
    let arity = |allowed: &[usize]| -> Result<()> {
        if allowed.contains(&args.len()) {
            Ok(())
        } else {
            Err(format!("{name}: wrong number of input arguments"))
        }
    };
    Ok(Some(match name {
        "sparse" => {
            engine.sparse.seen = true;
            construct(engine, args)?
        }
        "issparse" => {
            arity(&[1])?;
            logical_scalar(is_sparse(&args[0]))
        }
        "speye" => {
            engine.sparse.seen = true;
            let (m, n) = size_arguments(engine, name, args)?;
            wrap(SparseMatrix::identity(m, n)?)
        }
        "spalloc" => {
            engine.sparse.seen = true;
            arity(&[3])?;
            let m = count_arg(engine, name, &args[0])?;
            let n = count_arg(engine, name, &args[1])?;
            // The capacity is validated and otherwise ignored.
            count_arg(engine, name, &args[2])?;
            wrap(SparseMatrix::zeros(m, n, false)?)
        }
        "spones" => {
            engine.sparse.seen = true;
            arity(&[1])?;
            let s = to_sparse(engine, name, &args[0])?;
            wrap(s.map(|_| 1., false))
        }
        "spdiags" => {
            engine.sparse.seen = true;
            spdiags(engine, args)?
        }
        "kron" => {
            arity(&[2])?;
            let a = to_sparse(engine, name, &args[0])?;
            let b = to_sparse(engine, name, &args[1])?;
            wrap(kron(engine, &a, &b)?)
        }
        "gallery" => match gallery(engine, args)? {
            Some(s) => {
                engine.sparse.seen = true;
                wrap(s)
            }
            None => return Ok(None),
        },
        "sprand" | "sprandn" | "sprandsym" => {
            engine.sparse.seen = true;
            super::sparse_random::random(engine, name, args)?
        }
        "spfun" => {
            engine.sparse.seen = true;
            spfun(engine, args, out)?
        }
        "amd" | "colamd" | "symamd" | "symrcm" => super::sparse_order::order(engine, name, args)?,
        "nzmax" => {
            arity(&[1])?;
            match &first {
                Some(s) => number(s.nnz() as f64),
                None => number(args[0].dims().iter().product::<usize>() as f64),
            }
        }
        _ => {
            let Some(s) = first else {
                return sparse_later_argument(engine, name, args, out);
            };
            match (name, args.len()) {
                ("full", 1) => full(engine, name, &s)?,
                ("nnz", 1) => number(s.nnz() as f64),
                ("nonzeros", 1) => nonzeros(engine, &s)?,
                ("find", 1) => find(engine, &s, 1, out)?.remove(0),
                ("sum", 1 | 2) => return reduce(engine, name, &s, args, out),
                ("max" | "min", 1) => return reduce(engine, name, &s, args, out),
                ("max" | "min", 3) if matches!(&args[1], Value::Matrix(m) if m.is_empty()) => {
                    return reduce(engine, name, &s, args, out);
                }
                ("any" | "all", 1 | 2) => return reduce(engine, name, &s, args, out),
                ("abs", 1) => wrap(s.map(f64::abs, false)),
                ("round", 1) => wrap(s.map(f64::round, false)),
                ("floor", 1) => wrap(s.map(f64::floor, false)),
                ("ceil", 1) => wrap(s.map(f64::ceil, false)),
                ("fix", 1) => wrap(s.map(f64::trunc, false)),
                ("sqrt", 1) => {
                    if (0..s.nnz()).any(|k| s.value(k) < 0.) {
                        return Err(
                            "sqrt: negative entries in sparse input are not supported yet".into(),
                        );
                    }
                    wrap(s.map(f64::sqrt, false))
                }
                ("double", 1) => wrap(s.to_double()),
                ("logical", 1) => wrap(s.to_logical()?),
                ("reshape", 2 | 3) => return reshape(engine, &s, args),
                ("size", 1) => Value::Matrix(Matrix::from_column_slice(
                    1,
                    2,
                    &[s.rows() as f64, s.cols() as f64],
                )?),
                ("size", 2) => {
                    let d = dim_arg(engine, name, &args[1])?;
                    number(match d {
                        1 => s.rows(),
                        2 => s.cols(),
                        _ => 1,
                    } as f64)
                }
                ("numel", 1) => number(s.numel() as f64),
                ("ndims", 1) => number(2.),
                ("length", 1) => number(if s.is_empty() {
                    0.
                } else {
                    s.rows().max(s.cols()) as f64
                }),
                ("isempty", 1) => logical_scalar(s.is_empty()),
                ("isscalar", 1) => logical_scalar((s.rows(), s.cols()) == (1, 1)),
                ("isvector", 1) => logical_scalar(s.rows() == 1 || s.cols() == 1),
                ("isrow", 1) => logical_scalar(s.rows() == 1),
                ("iscolumn", 1) => logical_scalar(s.cols() == 1),
                ("ismatrix", 1) => logical_scalar(true),
                ("issquare", 1) => logical_scalar(s.rows() == s.cols()),
                ("class", 1) => {
                    Value::Text(if s.is_logical() { "logical" } else { "double" }.into())
                }
                ("isa", 2) => {
                    let class = text(engine, &args[1]).ok_or("isa: class name must be text")?;
                    logical_scalar(if s.is_logical() {
                        class == "logical"
                    } else {
                        matches!(class.as_str(), "double" | "float" | "numeric")
                    })
                }
                ("isreal", 1) => logical_scalar(true),
                ("islogical", 1) => logical_scalar(s.is_logical()),
                ("isnumeric" | "isfloat", 1) => logical_scalar(!s.is_logical()),
                ("transpose" | "ctranspose", 1) => transpose(&s)?,
                ("uminus", 1) => unary(&s, true),
                ("uplus", 1) => unary(&s, false),
                _ => return sparse_later_argument(engine, name, args, out),
            }
        }
    }))
}

/// Apply a function handle once to the stored values of an array.  The input
/// passed to the handle is deliberately a full column vector: it is bounded by
/// `MAX_NNZ`, not by the potentially enormous shape of the sparse array.
fn spfun(engine: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() != 2 {
        return Err("spfun: wrong number of input arguments".into());
    }
    let handle = super::optim_nl::onl_handle(engine, "spfun", &args[0])?;
    let s = to_sparse(engine, "spfun", &args[1])?;
    let input = Matrix::from_column_vec(s.nnz(), 1, (0..s.nnz()).map(|k| s.value(k)).collect())?;
    let mut values = engine.call_handle(&handle, vec![Value::Matrix(input)], 1, None, out)?;
    let value = values.pop().unwrap_or(Value::Void);
    let Value::Matrix(values) = value else {
        return Err("spfun: function must return a real vector of the same length".into());
    };
    if !(values.rows() == 1 || values.cols() == 1) || values.data().len() != s.nnz() {
        return Err("spfun: function must return a real vector of the same length".into());
    }
    let mut b = sparse::Builder::new(s.rows(), s.cols(), false, s.nnz())?;
    for c in 0..s.cols() {
        engine.check_cancel()?;
        for k in s.column(c) {
            b.push(s.rowidx()[k], values.data()[k])?;
        }
        b.end_column();
    }
    Ok(wrap(b.finish()))
}

/// Builtins whose sparse argument may come after the first one.
fn sparse_later_argument(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    let op = match name {
        "plus" => Some(Token::Plus),
        "minus" => Some(Token::Minus),
        "times" => Some(Token::DotStar),
        "rdivide" => Some(Token::DotSlash),
        "ldivide" => Some(Token::DotSlash),
        "mtimes" => Some(Token::Star),
        "mrdivide" => Some(Token::Slash),
        "mldivide" => Some(Token::Backslash),
        _ => None,
    };
    if let Some(op) = op {
        if args.len() != 2 {
            return Err(format!("{name}: expected two inputs"));
        }
        let (a, b) = if name == "ldivide" {
            (args[1].clone(), args[0].clone())
        } else {
            (args[0].clone(), args[1].clone())
        };
        return binary(engine, &op, a, b, out).map(Some);
    }
    Ok(Some(match name {
        "and" if args.len() == 2 => {
            match logical_binary(engine, &Token::And, args[0].clone(), args[1].clone())? {
                Ok(v) => v,
                Err(_) => return Ok(None),
            }
        }
        "isequal" | "isequaln" if args.len() >= 2 => {
            let nan_equal = name == "isequaln";
            let mut equal = true;
            for b in &args[1..] {
                match equal_values(engine, &args[0], b, nan_equal) {
                    Some(e) => equal &= e,
                    None => return Ok(None),
                }
            }
            logical_scalar(equal)
        }
        "cat" if !args.is_empty() => {
            let dim = dim_arg(engine, name, &args[0])?;
            let blocks = args[1..].to_vec();
            match dim {
                1 => concatenate(engine, blocks.into_iter().map(|v| vec![v]).collect())?,
                2 => concatenate(engine, vec![blocks])?,
                _ => return Err("cat: N-D concatenation of sparse arrays is not supported".into()),
            }
        }
        "horzcat" => concatenate(engine, vec![args.to_vec()])?,
        "vertcat" => concatenate(engine, args.iter().map(|v| vec![v.clone()]).collect())?,
        "full" | "issparse" | "nnz" => {
            return Err(format!("{name}: wrong number of input arguments"));
        }
        "isa" | "class" | "size" if args.first().is_some_and(is_sparse) => {
            return Err(format!("{name}: wrong number of input arguments"));
        }
        _ => return Ok(None),
    }))
}

fn equal_values(engine: &Engine, a: &Value, b: &Value, nan_equal: bool) -> Option<bool> {
    let (da, db) = (dims2(a), dims2(b));
    match (as_sparse(a), as_sparse(b)) {
        (Some(x), Some(y)) => Some(da == db && x.equals(y, nan_equal)),
        (Some(s), None) | (None, Some(s)) => {
            let other = if as_sparse(a).is_some() { b } else { a };
            let data = match other {
                Value::Matrix(m) if !m.is_nd() => m.data().to_vec(),
                Value::Logical(m) if !m.is_nd() => m.data().iter().map(|&v| f64::from(v)).collect(),
                Value::Matrix(_) | Value::Logical(_) => return Some(false),
                _ => return None,
            };
            let _ = engine;
            Some(da == db && s.equals_dense(&data, nan_equal))
        }
        _ => None,
    }
}

/// `speye(n)`, `speye(m,n)`, `speye([m n])`.
fn size_arguments(engine: &Engine, name: &str, args: &[Value]) -> Result<(usize, usize)> {
    match args.len() {
        0 => Ok((1, 1)),
        1 if dims2(&args[0]) == (1, 2) => {
            let m = args[0].clone().matrix(&engine.cancel)?;
            let a = count_arg(engine, name, &number(m.data()[0]))?;
            let b = count_arg(engine, name, &number(m.data()[1]))?;
            Ok((a, b))
        }
        1 => {
            let n = count_arg(engine, name, &args[0])?;
            Ok((n, n))
        }
        2 => Ok((
            count_arg(engine, name, &args[0])?,
            count_arg(engine, name, &args[1])?,
        )),
        _ => Err(format!("{name}: wrong number of input arguments")),
    }
}

/// Positive integer subscripts of `sparse(i,j,v,...)`.
fn subscripts(engine: &mut Engine, v: &Value, maximum: Option<usize>) -> Result<Vec<usize>> {
    let m = match v {
        Value::Container(Array::Sparse(s)) => full(engine, "sparse", s)?.matrix(&engine.cancel)?,
        Value::Matrix(_) | Value::Logical(_) => v.clone().matrix(&engine.cancel)?,
        _ => return Err("sparse: index vectors must be real numeric".into()),
    };
    let limit = maximum.unwrap_or(usize::try_from(sparse::MAX_NNZ * 64).unwrap_or(usize::MAX));
    let mut result = Vec::with_capacity(m.len());
    for &x in m.data() {
        if !x.is_finite() || x.fract() != 0. || x < 1. || x > limit as f64 {
            return Err(match maximum {
                Some(maximum) => engine.raise(
                    format!("Index must be an integer between 1 and {maximum}"),
                    "MATLAB:badsubscript".into(),
                ),
                None => engine.raise(
                    "Index must be a positive integer".into(),
                    "MATLAB:badsubscript".into(),
                ),
            });
        }
        result.push(x as usize - 1);
    }
    Ok(result)
}

/// `sparse(A)`, `sparse(m,n)`, `sparse(i,j,v[,m,n[,nzmax]])`.
fn construct(engine: &mut Engine, args: &[Value]) -> Result<Value> {
    let cancel = engine.cancel.clone();
    match args.len() {
        1 => {
            let v = &args[0];
            if v.is_nd() {
                return Err("sparse: N-D input is not supported".into());
            }
            match v {
                Value::Container(Array::Sparse(_)) | Value::Matrix(_) | Value::Logical(_) => {
                    Ok(wrap(to_sparse(engine, "sparse", v)?))
                }
                _ => Err(unsupported_input("sparse")),
            }
        }
        2 => {
            let m = count_arg(engine, "sparse", &args[0])?;
            let n = count_arg(engine, "sparse", &args[1])?;
            Ok(wrap(SparseMatrix::zeros(m, n, false)?))
        }
        3 | 5 | 6 => {
            let given = if args.len() >= 5 {
                Some((
                    count_arg(engine, "sparse", &args[3])?,
                    count_arg(engine, "sparse", &args[4])?,
                ))
            } else {
                None
            };
            if args.len() == 6 {
                // nzmax: validated, then ignored (storage holds exactly nnz entries).
                count_arg(engine, "sparse", &args[5])?;
            }
            let values = &args[2];
            let logical = match values {
                Value::Logical(_) => true,
                Value::Container(Array::Sparse(s)) => s.is_logical(),
                Value::Matrix(_) => false,
                _ => return Err(unsupported_input("sparse")),
            };
            let v = match values {
                Value::Container(Array::Sparse(s)) => full(engine, "sparse", s)?.matrix(&cancel)?,
                _ => values.clone().matrix(&cancel)?,
            };
            let i = subscripts(engine, &args[0], given.map(|g| g.0))?;
            let j = subscripts(engine, &args[1], given.map(|g| g.1))?;
            let len = i.len().max(j.len()).max(v.len());
            let lens = [i.len(), j.len(), v.len()];
            if lens.iter().any(|&n| n != len && n != 1) {
                return Err(
                    "sparse: index and value vectors must have the same length (or length 1)"
                        .into(),
                );
            }
            if len as u64 > sparse::MAX_NNZ {
                return Err(format!(
                    "sparse: more than {} entries are not supported",
                    sparse::MAX_NNZ
                ));
            }
            let empty = lens.contains(&0);
            let (m, n) = given.unwrap_or_else(|| {
                if empty {
                    (0, 0)
                } else {
                    (
                        i.iter().max().map_or(0, |x| x + 1),
                        j.iter().max().map_or(0, |x| x + 1),
                    )
                }
            });
            let mut entries = Vec::with_capacity(if empty { 0 } else { len });
            if !empty {
                for k in 0..len {
                    let pick = |list: &[usize]| if list.len() == 1 { list[0] } else { list[k] };
                    let x = if v.len() == 1 {
                        v.data()[0]
                    } else {
                        v.data()[k]
                    };
                    entries.push((pick(&i), pick(&j), x));
                }
            }
            if logical && entries.iter().any(|e| e.2.is_nan()) {
                return Err(sparse::NAN_TO_LOGICAL.into());
            }
            Ok(wrap(SparseMatrix::from_triplets(
                m, n, &entries, logical, &cancel,
            )?))
        }
        _ => Err("sparse: expected 1, 2, 3, 5 or 6 input arguments".into()),
    }
}

/// `spdiags(B,d,m,n)`, the create form, with MATLAB's alignment: for
/// m >= n, column k of B fills diagonal d(k) by column index; otherwise by row index.
fn spdiags(engine: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.len() != 4 {
        return Err(
            "spdiags: only the spdiags(B,d,m,n) form is supported yet; other forms are not supported yet"
                .into(),
        );
    }
    let cancel = engine.cancel.clone();
    let b = match &args[0] {
        Value::Container(Array::Sparse(s)) => full(engine, "spdiags", s)?.matrix(&cancel)?,
        Value::Matrix(_) | Value::Logical(_) => args[0].clone().matrix(&cancel)?,
        _ => return Err(unsupported_input("spdiags")),
    };
    let d = args[1].clone().matrix(&cancel)?;
    let m = count_arg(engine, "spdiags", &args[2])?;
    let n = count_arg(engine, "spdiags", &args[3])?;
    sparse::check_shape(m, n, 0)?;
    let p = d.len();
    let scalar = b.len() == 1;
    if b.cols() != p && !(scalar && p >= 1) {
        return Err("spdiags: B must have one column per diagonal in d".into());
    }
    // Plan every diagonal first: its rows i (0 <= i < m, 0 <= i+d < n), cut to
    // the rows whose source position (i+d for m >= n, else i) lies inside B
    // (other positions hold zeros), and the exact stored-entry count. Limits
    // are checked before anything is allocated; the work is O(size of B).
    let source_value = |k: usize, source: usize| {
        if scalar {
            b.data()[0]
        } else {
            b.data()[k * b.rows() + source]
        }
    };
    let mut plan: Vec<(usize, i128, usize, usize)> = Vec::new();
    let mut total = 0u64;
    for (k, &dk) in d.data().iter().enumerate() {
        if !dk.is_finite() || dk.fract() != 0. {
            return Err("spdiags: diagonal numbers must be integers".into());
        }
        let dk = dk as i128;
        let shift = if m >= n { dk } else { 0 };
        let lo = (-dk).max(0);
        let mut hi = (n as i128 - dk).min(m as i128);
        if !scalar {
            hi = hi.min(b.rows() as i128 - shift);
        }
        if lo >= hi {
            continue;
        }
        let (lo, hi) = (lo as usize, hi as usize);
        total += if scalar {
            if b.data()[0] != 0. {
                (hi - lo) as u64
            } else {
                0
            }
        } else {
            engine.check_cancel()?;
            (lo..hi)
                .filter(|&i| source_value(k, (i as i128 + shift) as usize) != 0.)
                .count() as u64
        };
        sparse::check_shape(m, n, total)?;
        plan.push((k, dk, lo, hi));
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(total as usize)
        .map_err(|_| "spdiags: cannot allocate the diagonal entries")?;
    for (k, dk, lo, hi) in plan {
        let shift = if m >= n { dk } else { 0 };
        for i in lo..hi {
            if i % 65536 == 0 {
                engine.check_cancel()?;
            }
            let v = source_value(k, (i as i128 + shift) as usize);
            if v != 0. {
                entries.push((i, (i as i128 + dk) as usize, v));
            }
        }
    }
    Ok(wrap(SparseMatrix::from_triplets(
        m, n, &entries, false, &cancel,
    )?))
}

fn nonzeros(engine: &Engine, s: &SparseMatrix) -> Result<Value> {
    let n = s.nnz();
    if s.is_logical() {
        Ok(Value::Logical(LogicalMatrix::from_column_vec(
            n,
            1,
            vec![1; n],
            &engine.cancel,
        )?))
    } else {
        Ok(Value::Matrix(Matrix::from_column_vec(
            n,
            1,
            (0..n).map(|k| s.value(k)).collect(),
        )?))
    }
}

/// `k = find(S)` and `[i,j,v] = find(S)`: full columns in column order
/// (rows for a row vector, as for full input).
fn find(
    engine: &mut Engine,
    s: &SparseMatrix,
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let n = s.nnz();
    if n == 0 {
        // The full function decides the empty shapes.
        let small = Matrix::from_column_vec(
            s.rows().min(2),
            s.cols().min(2),
            vec![0.; s.rows().min(2) * s.cols().min(2)],
        )?;
        let temp = engine.assigned_array(Value::Matrix(small))?;
        engine.sparse.temp_counter += 1;
        let name = format!("{TEMP_PREFIX}{}", engine.sparse.temp_counter);
        engine.publish_array(name.clone(), temp);
        let targets: Vec<Option<String>> = (0..count).map(|_| Some(String::new())).collect();
        let result = engine.eval_outputs(
            &Expr::Call("find".into(), vec![Expr::Name(name.clone())]),
            &targets,
            out,
        );
        engine.remove_array(&name);
        return result;
    }
    let shape = |len: usize| if s.rows() == 1 { (1, len) } else { (len, 1) };
    let (r, c) = shape(n);
    let column = |data: Vec<f64>| Matrix::from_column_vec(r, c, data).map(Value::Matrix);
    let entries: Vec<(usize, usize, f64)> = s.entries().collect();
    engine.check_cancel()?;
    if count == 1 {
        let m = s.rows() as f64;
        return Ok(vec![column(
            entries
                .iter()
                .map(|&(i, j, _)| j as f64 * m + i as f64 + 1.)
                .collect(),
        )?]);
    }
    let mut outputs = vec![
        column(entries.iter().map(|e| e.0 as f64 + 1.).collect())?,
        column(entries.iter().map(|e| e.1 as f64 + 1.).collect())?,
    ];
    if count == 3 {
        outputs.push(if s.is_logical() {
            Value::Logical(LogicalMatrix::from_column_vec(
                r,
                c,
                vec![1; n],
                &engine.cancel,
            )?)
        } else {
            column(entries.iter().map(|e| e.2).collect())?
        });
    }
    Ok(outputs)
}

/// sum, max, min, any and all along dimension 1 or 2.
fn reduce(
    engine: &mut Engine,
    name: &str,
    s: &SparseMatrix,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    let dim_value = match (name, args.len()) {
        ("max" | "min", 3) => Some(&args[2]),
        ("max" | "min", _) => None,
        (_, 2) => Some(&args[1]),
        _ => None,
    };
    let dim = match dim_value {
        Some(v) if text(engine, v).is_some() => return Ok(None),
        Some(v) => dim_arg(engine, name, v)?,
        None => default_dim(s),
    };
    if s.is_empty() {
        // Empty shapes follow the full function; the result is stored sparse.
        let full_args = args
            .iter()
            .map(|v| densify(engine, name, v.clone()))
            .collect::<Result<Vec<_>>>()?;
        let result = engine.call(name, full_args, out)?;
        return Ok(Some(wrap(to_sparse(engine, name, &result)?)));
    }
    if dim > 2 {
        return Ok(Some(wrap(match name {
            "sum" => s.to_double(),
            "max" | "min" => s.clone(),
            _ => s.map(
                |v| f64::from(u8::from(v != 0. && (name == "all" || !v.is_nan()))),
                true,
            ),
        })));
    }
    // any/all are logical; max/min keep the input class, as for full input.
    let logical =
        matches!(name, "any" | "all") || (matches!(name, "max" | "min") && s.is_logical());
    if dim == 1 {
        let mut b = sparse::Builder::new(1, s.cols(), logical, s.cols().min(s.nnz()))?;
        for c in 0..s.cols() {
            if c % 4096 == 0 {
                engine.check_cancel()?;
            }
            let range = s.column(c);
            let hole = range.len() < s.rows();
            b.push(0, reduced(name, range.map(|k| s.value(k)), hole))?;
            b.end_column();
        }
        return Ok(Some(wrap(b.finish())));
    }
    // Along rows: the stored entries grouped by row (a stable sort keeps each
    // row in column order, as a transpose would), so the work is O(nnz log nnz)
    // whatever the row count. A row without stored entries reduces to 0.
    let mut entries: Vec<(usize, f64)> = Vec::new();
    entries
        .try_reserve_exact(s.nnz())
        .map_err(|_| format!("{name}: cannot allocate the stored entries"))?;
    for c in 0..s.cols() {
        if c % 4096 == 0 {
            engine.check_cancel()?;
        }
        entries.extend(s.column(c).map(|k| (s.rowidx()[k], s.value(k))));
    }
    entries.sort_by_key(|e| e.0);
    engine.check_cancel()?;
    let mut b = sparse::Builder::new(s.rows(), 1, logical, entries.len().min(s.rows()))?;
    let mut i = 0;
    while i < entries.len() {
        let r = entries[i].0;
        let end = i + entries[i..].partition_point(|e| e.0 == r);
        let hole = end - i < s.cols();
        b.push(r, reduced(name, entries[i..end].iter().map(|e| e.1), hole))?;
        i = end;
    }
    b.end_column();
    Ok(Some(wrap(b.finish())))
}

/// One sum/max/min/any/all value from the stored values of a column or row;
/// `hole` says some of its elements are implicit zeros.
fn reduced(name: &str, mut stored: impl Iterator<Item = f64>, hole: bool) -> f64 {
    match name {
        "sum" => stored.sum::<f64>(),
        "any" => f64::from(u8::from(stored.any(|v| !v.is_nan()))),
        "all" => f64::from(u8::from(!hole)),
        _ => {
            let better = |x: f64, y: f64| if name == "max" { x > y } else { x < y };
            let mut best: Option<f64> = if hole { Some(0.) } else { None };
            for v in stored {
                if v.is_nan() {
                    continue;
                }
                best = Some(match best {
                    Some(b) if !better(v, b) => b,
                    _ => v,
                });
            }
            best.unwrap_or(f64::NAN)
        }
    }
}

fn reshape(engine: &Engine, s: &SparseMatrix, args: &[Value]) -> Result<Option<Value>> {
    let mut sizes: Vec<Option<f64>> = Vec::new();
    if args.len() == 2 {
        let m = args[1].clone().matrix(&engine.cancel)?;
        if m.len() != 2 {
            return Err("reshape: sparse arrays must stay two-dimensional".into());
        }
        sizes.extend(m.data().iter().map(|&v| Some(v)));
    } else {
        for v in &args[1..] {
            match v {
                Value::Matrix(m) if m.is_empty() => sizes.push(None),
                _ => sizes.push(Some(scalar(engine, "reshape", v)?)),
            }
        }
    }
    let known: Vec<u64> = sizes
        .iter()
        .flatten()
        .map(|&v| {
            if v.is_finite() && v >= 0. && v.fract() == 0. {
                Ok(v as u64)
            } else {
                Err("reshape: sizes must be nonnegative integers".to_string())
            }
        })
        .collect::<Result<_>>()?;
    let resolved = match sizes.iter().filter(|v| v.is_none()).count() {
        0 => (known[0], known[1]),
        1 => {
            let k = known.first().copied().unwrap_or(1);
            if k == 0 || !s.numel().is_multiple_of(k) {
                return Err("reshape: the size is not divisible by the known dimension".into());
            }
            if sizes[0].is_none() {
                (s.numel() / k, k)
            } else {
                (k, s.numel() / k)
            }
        }
        _ => return Err("reshape: only one dimension can be inferred with []".into()),
    };
    let (m, n) = (
        usize::try_from(resolved.0).map_err(|_| "reshape: size too large")?,
        usize::try_from(resolved.1).map_err(|_| "reshape: size too large")?,
    );
    Ok(Some(wrap(s.reshape(m, n)?)))
}

// ---------------------------------------------------------------- indexing

fn index_list(indices: &Indices) -> Vec<usize> {
    (0..indices.len()).map(|i| indices.at(i)).collect()
}
/// Row subscripts for `SparseMatrix::select`/`delete_rows`: `:` and ranges
/// stay ranges; only explicit lists and masks (already in memory) become a list.
fn row_pick<'a>(indices: &Indices, bound: usize, list: &'a mut Vec<usize>) -> Rows<'a> {
    if is_full_range(indices, bound) {
        return Rows::All;
    }
    match indices {
        Indices::Stride { start, step, len } => Rows::Range {
            start: *start,
            step: *step,
            len: *len,
        },
        Indices::LogicalStride { start, len, .. } => Rows::Range {
            start: *start,
            step: 1,
            len: *len,
        },
        _ => {
            *list = index_list(indices);
            Rows::List(list)
        }
    }
}
fn is_full_range(indices: &Indices, bound: usize) -> bool {
    indices.len() == bound && indices.contiguous() == Some(0)
}

/// `S(i,j)`, `S(k)`, `S(:)`, masks and `end`: sparse results.
pub(super) fn index(
    engine: &mut Engine,
    s: &SparseMatrix,
    args: &[Expr],
    out: &mut Execution,
) -> Result<Value> {
    if args.len() > 2 {
        return Err("sparse: N-D indexing is not supported".into());
    }
    let cancel = engine.cancel.clone();
    match args {
        [] => Ok(wrap(s.clone())),
        [arg] => {
            let numel = usize::try_from(s.numel())
                .map_err(|_| "sparse: too many elements to index linearly")?;
            if engine.colon_subscript(arg) {
                return Ok(wrap(s.reshape(numel, 1)?));
            }
            let (indices, mut shape) = engine.subscript(arg, numel, out)?;
            let vector_index = match &indices {
                // Same rule as dense indexing: a logical mask is a vector of hits.
                Indices::Mask(_, _) | Indices::LogicalStride { .. } => true,
                _ => shape.0 == 1 || shape.1 == 1,
            };
            if (s.rows() == 1 || s.cols() == 1) && vector_index {
                shape = if s.rows() == 1 {
                    (1, indices.len())
                } else {
                    (indices.len(), 1)
                };
            }
            let list = (0..indices.len()).map(|i| indices.at(i) as u64);
            Ok(wrap(s.select_linear(list, shape.0, shape.1, &cancel)?))
        }
        [a, b] => {
            let (rows, _) = engine.subscript(a, s.rows(), out)?;
            let (cols, _) = engine.subscript(b, s.cols(), out)?;
            let mut list = Vec::new();
            let rows = row_pick(&rows, s.rows(), &mut list);
            Ok(wrap(s.select(rows, &index_list(&cols), &cancel)?))
        }
        _ => unreachable!(),
    }
}

/// Indexed writes and deletions on a sparse variable.
#[allow(clippy::too_many_arguments)]
pub(super) fn assign(
    engine: &mut Engine,
    name: &str,
    s: SparseMatrix,
    args: &[Expr],
    value: Value,
    deletion: bool,
    quiet: bool,
    out: &mut Execution,
) -> Result<()> {
    if args.len() > 2 {
        return Err("sparse: N-D indexing is not supported".into());
    }
    if args.is_empty() {
        return Err("sparse: indexed assignment needs one or two subscripts".into());
    }
    let cancel = engine.cancel.clone();
    let (m, n) = (s.rows(), s.cols());
    // Growth bounds: columns by the column-pointer limit, rows by exact f64 integers.
    let (row_limit, col_limit) = (
        usize::try_from(1u64 << 48).unwrap_or(usize::MAX),
        sparse::MAX_COLUMN_POINTERS as usize - 1,
    );
    if deletion {
        if args.len() != 2 {
            return Err(
                "sparse: only S(:,j) = [] and S(i,:) = [] deletions are supported yet".into(),
            );
        }
        let (rows, _) = engine.subscript_with_limit(&args[0], Some(m), m, out)?;
        let (cols, _) = engine.subscript_with_limit(&args[1], Some(n), n, out)?;
        let result = if is_full_range(&rows, m) {
            s.delete_columns(&index_list(&cols), &cancel)?
        } else if is_full_range(&cols, n) {
            let mut list = Vec::new();
            s.delete_rows(row_pick(&rows, m, &mut list), &cancel)?
        } else {
            return Err("A null assignment requires at least one ':' subscript".into());
        };
        return engine.assign_workspace_array(name, Array::Sparse(result), quiet, out);
    }
    // Right-hand side: element p in column-major order.
    if matches!(value, Value::Complex(_)) {
        return Err("sparse: complex values are not supported yet".into());
    }
    let (vr, vc) = dims2(&value);
    let rhs: Box<dyn Fn(usize) -> f64> = match &value {
        Value::Container(Array::Sparse(t)) => {
            let t = t.clone();
            let r = t.rows().max(1);
            Box::new(move |p| t.get(p % r, p / r))
        }
        _ => {
            let data = value.clone().matrix(&cancel)?;
            Box::new(move |p| data.data()[p])
        }
    };
    let vlen = vr * vc;
    let mut writes: Vec<(usize, usize, f64)> = Vec::new();
    let check_count = |count: usize| -> Result<()> {
        if vlen != 1 && count != vlen {
            return Err("Indexed assignment requires matching element counts or a scalar".into());
        }
        if count as u64 > sparse::MAX_NNZ {
            return Err(format!(
                "sparse: assignment of more than {} elements is not supported",
                sparse::MAX_NNZ
            ));
        }
        Ok(())
    };
    let (new_m, new_n) = if args.len() == 2 {
        let (rows, _) = engine.subscript_with_limit(&args[0], Some(m), row_limit, out)?;
        let (cols, _) = engine.subscript_with_limit(&args[1], Some(n), col_limit, out)?;
        let count = rows.len().saturating_mul(cols.len());
        check_count(count)?;
        if vlen != 1 && count != 0 {
            let selected: Vec<_> = [rows.len(), cols.len()]
                .into_iter()
                .filter(|x| *x != 1)
                .collect();
            let source: Vec<_> = [vr, vc].into_iter().filter(|x| *x != 1).collect();
            if selected != source {
                return Err("Subscripted assignment dimensions must agree".into());
            }
        }
        let (rl, cl) = (index_list(&rows), index_list(&cols));
        let mut p = 0;
        for &c in &cl {
            engine.check_cancel()?;
            for &r in &rl {
                writes.push((r, c, if vlen == 1 { rhs(0) } else { rhs(p) }));
                p += 1;
            }
        }
        (
            rl.iter().map(|r| r + 1).max().unwrap_or(0).max(m),
            cl.iter().map(|c| c + 1).max().unwrap_or(0).max(n),
        )
    } else {
        let numel = usize::try_from(s.numel())
            .map_err(|_| "sparse: too many elements to index linearly")?;
        let (indices, _) = engine.subscript_with_limit(&args[0], Some(numel), row_limit, out)?;
        check_count(indices.len())?;
        let list = index_list(&indices);
        let extent = list.iter().map(|k| k + 1).max().unwrap_or(0);
        let shape = if extent > numel {
            if m > 1 && n > 1 {
                return Err("Cannot grow a matrix through one ambiguous linear subscript; use row and column indices".into());
            }
            if n == 1 && m != 1 {
                (extent, 1)
            } else {
                (1, extent)
            }
        } else {
            (m, n)
        };
        let rows = shape.0.max(1);
        for (p, &k) in list.iter().enumerate() {
            writes.push((k % rows, k / rows, if vlen == 1 { rhs(0) } else { rhs(p) }));
        }
        shape
    };
    if s.is_logical() && writes.iter().any(|w| w.2.is_nan()) {
        return Err(sparse::NAN_TO_LOGICAL.into());
    }
    let result = s.with_writes(new_m, new_n, &mut writes, &cancel)?;
    engine.assign_workspace_array(name, Array::Sparse(result), quiet, out)
}

/// A sparse value used as an index: its full copy.
pub(super) fn index_value(engine: &Engine, v: Value) -> Result<Value> {
    densify(engine, "subsindex", v)
}

/// `whos` class column for a sparse variable.
pub(super) fn whos_class(a: &Array) -> Option<&'static str> {
    match a {
        Array::Sparse(s) if s.is_logical() => Some("logical (sparse)"),
        Array::Sparse(_) => Some("double (sparse)"),
        _ => None,
    }
}

/// `kron(A, B)` with a sparse operand: sparse, O(nnz(A)*nnz(B)).
fn kron(engine: &Engine, a: &SparseMatrix, b: &SparseMatrix) -> Result<SparseMatrix> {
    let rows = a
        .rows()
        .checked_mul(b.rows())
        .ok_or("kron: result is too large")?;
    let cols = a
        .cols()
        .checked_mul(b.cols())
        .ok_or("kron: result is too large")?;
    let nnz = (a.nnz() as u64).saturating_mul(b.nnz() as u64);
    sparse::check_shape(rows, cols, nnz)?;
    let mut out = sparse::Builder::new(rows, cols, false, nnz as usize)?;
    for ja in 0..a.cols() {
        engine.check_cancel()?;
        for jb in 0..b.cols() {
            for p in a.column(ja) {
                let (ia, va) = (a.rowidx()[p], a.value(p));
                for q in b.column(jb) {
                    out.push(ia * b.rows() + b.rowidx()[q], va * b.value(q))?;
                }
            }
            out.end_column();
        }
    }
    Ok(out.finish())
}

/// `gallery('poisson', n)` and `gallery('tridiag', ...)` (sparse results);
/// `None` for the other families.
fn gallery(engine: &Engine, args: &[Value]) -> Result<Option<SparseMatrix>> {
    let Some(family) = args.first().and_then(|v| text(engine, v)) else {
        return Ok(None);
    };
    let name = "gallery";
    let vector = |v: &Value| -> Result<Vec<f64>> {
        let m = densify(engine, name, v.clone())?.matrix(&engine.cancel)?;
        Ok(m.data().to_vec())
    };
    match family.to_lowercase().as_str() {
        "poisson" => {
            if args.len() != 2 {
                return Err("gallery: poisson needs one argument N".into());
            }
            let n = count_arg(engine, name, &args[1])?;
            let size = n.checked_mul(n).ok_or("gallery: N is too large")?;
            sparse::check_shape(size, size, 5 * size as u64)?;
            let mut out = sparse::Builder::new(size, size, false, 5 * size)?;
            for c in 0..size {
                if c % 8192 == 0 {
                    engine.check_cancel()?;
                }
                let (jb, ib) = (c / n, c % n);
                if jb > 0 {
                    out.push(c - n, -1.)?;
                }
                if ib > 0 {
                    out.push(c - 1, -1.)?;
                }
                out.push(c, 4.)?;
                if ib + 1 < n {
                    out.push(c + 1, -1.)?;
                }
                if jb + 1 < n {
                    out.push(c + n, -1.)?;
                }
                out.end_column();
            }
            Ok(Some(out.finish()))
        }
        "tridiag" => {
            let (n, c, d, e) = match args.len() {
                2 => (
                    count_arg(engine, name, &args[1])?,
                    vec![-1.],
                    vec![2.],
                    vec![-1.],
                ),
                4 => {
                    let (c, d, e) = (vector(&args[1])?, vector(&args[2])?, vector(&args[3])?);
                    let n = d.len();
                    if n == 0 || c.len() + 1 != n || e.len() + 1 != n {
                        return Err(
                            "gallery: tridiag(c,d,e) needs length(c) = length(e) = length(d)-1"
                                .into(),
                        );
                    }
                    (n, c, d, e)
                }
                5 => {
                    let n = count_arg(engine, name, &args[1])?;
                    let one = |v: &Value| -> Result<Vec<f64>> {
                        let x = vector(v)?;
                        if x.len() != 1 {
                            return Err("gallery: tridiag(n,c,d,e) needs scalar c, d and e".into());
                        }
                        Ok(x)
                    };
                    (n, one(&args[2])?, one(&args[3])?, one(&args[4])?)
                }
                _ => return Err("gallery: tridiag takes (n), (c,d,e) or (n,c,d,e)".into()),
            };
            let at = |v: &[f64], k: usize| if v.len() == 1 { v[0] } else { v[k] };
            sparse::check_shape(n, n, 3 * n as u64)?;
            let mut out = sparse::Builder::new(n, n, false, 3 * n)?;
            for j in 0..n {
                if j > 0 {
                    out.push(j - 1, at(&e, j - 1))?;
                }
                out.push(j, at(&d, j))?;
                if j + 1 < n {
                    out.push(j + 1, at(&c, j))?;
                }
                out.end_column();
            }
            Ok(Some(out.finish()))
        }
        _ => Ok(None),
    }
}
