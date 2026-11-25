//! Allocation-free evaluation of real scalar expressions in loops.
//!
//! `s = s + (2*k+1)` used to build a heap `Matrix` (plus a coordinate proof)
//! for every literal, name and intermediate result, and to assign through the
//! general workspace path. This module evaluates an expression tree whose
//! leaves are real double or logical 1x1 workspace variables and numeric
//! literals directly on `f64`/`bool`, and stores the result in place when the
//! target is an exclusively owned real scalar.
//!
//! The evaluator is pure: it reads the workspace and never changes it. Any
//! other operand (char, int, single, complex, containers, N-D, functions,
//! `end`, `^`) or any value whose general path could raise an error returns
//! `None`, and the caller re-evaluates the whole expression on the general
//! path from scratch. Results are the same IEEE operations the general
//! kernels apply (`Matrix::zip`, `map`, `real_divide`, `LogicalMatrix::compare`),
//! so NaN payloads, infinities and signed zeros are unchanged. Evaluation
//! accounting charges one unit per node, exactly as `Engine::eval` does.
//!
//! What differs: a scalar result carries no `CoordinateProof` (exact patch
//! coordinate facts are only kept for array expressions; dense values remain
//! authoritative), and an in-place store does not re-run the workspace size
//! scan, which a 1x1 to 1x1 replacement cannot change.
use super::{Engine, Execution};
use crate::{LogicalMatrix, Matrix, Result, syntax::Expr, syntax::Token};

/// A real scalar operand: double or logical. Logical keeps its class through
/// comparisons and `~`; arithmetic converts it to double like the general path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Scalar {
    Double(f64),
    Logical(bool),
}

impl Scalar {
    fn number(self) -> f64 {
        match self {
            Self::Double(v) => v,
            Self::Logical(b) => f64::from(u8::from(b)),
        }
    }
    /// Logical conversion; NaN raises an error on the general path.
    fn truth(self) -> Option<bool> {
        match self {
            Self::Double(v) if v.is_nan() => None,
            Self::Double(v) => Some(v != 0.),
            Self::Logical(b) => Some(b),
        }
    }
}

/// Deepest tree the fast path walks before handing over to the general path.
/// Bounds the repeated attempt at nested `eval` calls when a deep expression
/// ends in a non-scalar leaf.
const MAX_DEPTH: usize = 32;

/// Evaluate `e` without side effects. `nodes` counts the `eval` calls the
/// general path would have made (one per evaluated node).
fn scalar(
    engine: &Engine,
    e: &Expr,
    depth: usize,
    limit: usize,
    nodes: &mut usize,
) -> Option<Scalar> {
    if depth >= limit {
        return None;
    }
    let value = match e {
        Expr::Number(v) => Scalar::Double(*v),
        Expr::Name(n) => variable(engine, n)?,
        Expr::Unary(negative, x) => {
            let v = scalar(engine, x, depth + 1, limit, nodes)?.number();
            Scalar::Double(if *negative { -v } else { v })
        }
        Expr::Not(x) => Scalar::Logical(!scalar(engine, x, depth + 1, limit, nodes)?.truth()?),
        Expr::Binary(op @ (Token::ShortAnd | Token::ShortOr), a, b) => {
            let left = scalar(engine, a, depth + 1, limit, nodes)?.truth()?;
            if (op == &Token::ShortAnd && !left) || (op == &Token::ShortOr && left) {
                Scalar::Logical(left)
            } else {
                Scalar::Logical(scalar(engine, b, depth + 1, limit, nodes)?.truth()?)
            }
        }
        Expr::Binary(op, a, b) => {
            let left = scalar(engine, a, depth + 1, limit, nodes)?;
            let right = scalar(engine, b, depth + 1, limit, nodes)?;
            binary(op, left, right)?
        }
        _ => return None,
    };
    *nodes += 1;
    Some(value)
}

/// The workspace value of `name` if it is a real double or logical 1x1.
/// Lookup order follows `Engine::eval_inner`: containers, complex, logical,
/// then numeric; anything else (functions, constants such as `pi`) is general.
fn variable(engine: &Engine, name: &str) -> Option<Scalar> {
    if (!engine.container_variables.is_empty() && engine.container_variables.contains_key(name))
        || (!engine.complex_variables.is_empty() && engine.complex_variables.contains_key(name))
    {
        return None;
    }
    if !engine.logical_variables.is_empty()
        && let Some(m) = engine.logical_variables.get(name)
    {
        return (m.rows() == 1 && m.cols() == 1 && !m.is_nd())
            .then(|| Scalar::Logical(m.data()[0] != 0));
    }
    let m = engine.variables.get(name)?;
    (m.rows() == 1 && m.cols() == 1 && !m.is_nd()).then(|| Scalar::Double(m.data()[0]))
}

/// One binary operator on scalars, or `None` for operators left to the general
/// path (`^`, `.^` may give complex results).
fn binary(op: &Token, a: Scalar, b: Scalar) -> Option<Scalar> {
    let compare = |f: fn(f64, f64) -> bool| Some(Scalar::Logical(f(a.number(), b.number())));
    let (x, y) = (a.number(), b.number());
    match op {
        Token::Plus => Some(Scalar::Double(x + y)),
        Token::Minus => Some(Scalar::Double(x - y)),
        Token::Star | Token::DotStar => Some(Scalar::Double(x * y)),
        Token::Slash | Token::DotSlash => Some(Scalar::Double(x / y)),
        Token::Backslash | Token::DotBackslash => Some(Scalar::Double(y / x)),
        Token::Equal => compare(|x, y| x == y),
        Token::NotEqual => compare(|x, y| x != y),
        Token::Less => compare(|x, y| x < y),
        Token::LessEqual => compare(|x, y| x <= y),
        Token::Greater => compare(|x, y| x > y),
        Token::GreaterEqual => compare(|x, y| x >= y),
        Token::And => Some(Scalar::Logical(a.truth()? & b.truth()?)),
        Token::Or => Some(Scalar::Logical(a.truth()? | b.truth()?)),
        _ => None,
    }
}

/// Fast evaluation of an operator expression, charged as the general path
/// charges it. `Ok(None)` means: evaluate on the general path.
pub(super) fn evaluate(engine: &mut Engine, e: &Expr) -> Result<Option<Scalar>> {
    if !matches!(e, Expr::Binary(..) | Expr::Unary(..) | Expr::Not(..)) {
        return Ok(None);
    }
    // A node at depth d runs with evaluation depth `evaluation_depth + d`;
    // the general path fails at 128, so stop before that and let it report.
    let limit = MAX_DEPTH.min(128usize.saturating_sub(engine.evaluation_depth));
    let mut nodes = 0;
    let Some(value) = scalar(engine, e, 0, limit, &mut nodes) else {
        return Ok(None);
    };
    engine.check_cancel()?;
    engine.charge(nodes)?;
    Ok(Some(value))
}

/// The value form `Engine::eval` returns for a fast result.
pub(super) fn value(result: Scalar) -> super::Value {
    match result {
        Scalar::Double(v) => super::Value::Matrix(Matrix::scalar(v)),
        Scalar::Logical(b) => super::Value::Logical(LogicalMatrix::scalar(b)),
    }
}

/// `name = expression;` for a scalar expression. Writes into the existing
/// variable's storage when it is an exclusively owned real 1x1 (no allocation);
/// otherwise assigns the scalar through the ordinary workspace path.
/// Returns false when the expression needs the general path.
pub(super) fn assign(
    engine: &mut Engine,
    name: &str,
    e: &Expr,
    execution: &mut Execution,
) -> Result<bool> {
    let result = match e {
        Expr::Number(v) => {
            engine.check_cancel()?;
            engine.charge(1)?;
            Scalar::Double(*v)
        }
        _ => match evaluate(engine, e)? {
            Some(result) => result,
            None => return Ok(false),
        },
    };
    let array = match result {
        Scalar::Double(v) => {
            Engine::validate_workspace_name(name)?;
            let alone =
                engine.logical_variables.is_empty() || !engine.logical_variables.contains_key(name);
            let alone = alone
                && (engine.complex_variables.is_empty()
                    || !engine.complex_variables.contains_key(name))
                && (engine.container_variables.is_empty()
                    || !engine.container_variables.contains_key(name));
            // Reuse only an exclusively owned scalar. A captured alias, global
            // copy or desktop snapshot forces ordinary copy-on-write assignment.
            if alone
                && engine.variables.get_mut(name).is_some_and(|m| {
                    m.rows() == 1 && m.cols() == 1 && !m.is_nd() && m.write_unique(&[(0, v)])
                })
            {
                return Ok(true);
            }
            Matrix::scalar(v).into()
        }
        Scalar::Logical(b) => LogicalMatrix::scalar(b).into(),
    };
    engine.assign_workspace_array(name, array, true, execution)?;
    Ok(true)
}

/// Truth of an `if`/`while` condition that is a scalar expression, as
/// `Engine::truth` decides it (any nonzero double, NaN included, is true).
pub(super) fn condition(engine: &mut Engine, e: &Expr) -> Result<Option<bool>> {
    if matches!(e, Expr::Binary(Token::And | Token::Or, ..) | Expr::Not(_)) {
        // condition_value has its own short-circuit rules for these.
        return Ok(None);
    }
    Ok(evaluate(engine, e)?.map(|result| match result {
        Scalar::Double(v) => v != 0.,
        Scalar::Logical(b) => b,
    }))
}

#[cfg(test)]
#[path = "scalar_loops_tests.rs"]
mod tests;
