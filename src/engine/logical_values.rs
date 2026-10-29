use super::{Engine, Execution, Value};
use crate::{
    LogicalMatrix as L, Matrix, Result,
    logical::{BooleanOp, Comparison, Predicate},
    syntax::{Expr, Token},
};

fn arity(name: &str, args: &[Value], allowed: &[usize]) -> Result<()> {
    if allowed.contains(&args.len()) {
        Ok(())
    } else {
        Err(format!(
            "{name} expects {} arguments",
            allowed
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" or ")
        ))
    }
}
fn dimension(v: &Value, cancel: &std::sync::atomic::AtomicBool) -> Result<usize> {
    if v.view(cancel)?.as_ref().len() != 1 {
        return Err("Expected a scalar dimension".into());
    }
    let v = v.clone().matrix(cancel)?.as_scalar()?;
    if !v.is_finite() || v < 0. || v.fract() != 0. || v > crate::matrix::MAX_ELEMENTS as f64 {
        Err("Dimension must be a bounded nonnegative integer".into())
    } else {
        Ok(v as usize)
    }
}

impl Engine {
    pub(super) fn logical_operator(
        &mut self,
        op: &Token,
        a: &Expr,
        b: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Value> {
        let left = self.eval(a, end, out)?;
        if matches!(op, Token::ShortAnd | Token::ShortOr) {
            let scalar = if super::typed_values::is_typed(&left) {
                left.shape(&self.cancel)? == (1, 1)
            } else {
                left.view(&self.cancel)?.as_ref().len() == 1
            };
            if !scalar {
                return Err("Short-circuit operands must be logical scalars".into());
            }
            let value = left.logical(&self.cancel)?.as_scalar()?;
            if (op == &Token::ShortAnd && !value) || (op == &Token::ShortOr && value) {
                return Ok(Value::Logical(L::scalar(value)));
            }
            let value = self.eval(b, end, out)?.logical(&self.cancel)?.as_scalar()?;
            return Ok(Value::Logical(L::scalar(value)));
        }
        let right = self.eval(b, end, out)?;
        if let Some(value) = self.obj_equality(op, &left, &right)? {
            return Ok(value);
        }
        if !matches!(op, Token::And | Token::Or)
            && (super::typed_values::is_typed(&left) || super::typed_values::is_typed(&right))
        {
            return self.typed_compare(op, left, right);
        }
        if matches!(op, Token::And | Token::Or) {
            let left = left.logical(&self.cancel)?;
            let right = right.logical(&self.cancel)?;
            return Ok(Value::Logical(left.combine(
                &right,
                if op == &Token::And {
                    BooleanOp::And
                } else {
                    BooleanOp::Or
                },
                &self.cancel,
            )?));
        }
        let comparison = match op {
            Token::Equal => Comparison::Equal,
            Token::NotEqual => Comparison::NotEqual,
            Token::Less => Comparison::Less,
            Token::LessEqual => Comparison::LessEqual,
            Token::Greater => Comparison::Greater,
            Token::GreaterEqual => Comparison::GreaterEqual,
            _ => unreachable!(),
        };
        Ok(Value::Logical(L::compare(
            left.view(&self.cancel)?.as_ref(),
            right.view(&self.cancel)?.as_ref(),
            comparison,
            &self.cancel,
        )?))
    }
    pub(super) fn concatenate_values(&mut self, mut rows: Vec<Vec<Value>>) -> Result<Value> {
        if rows.iter().flatten().any(|v| {
            v.is_character()
                || matches!(
                    v,
                    Value::Container(crate::Array::Cell(_) | crate::Array::Struct(_))
                )
        }) {
            return self.cel_concatenate(rows);
        }
        // [] is neutral even when the other operands select char/struct dispatch.
        for row in &mut rows {
            self.check_cancel()?;
            row.retain(|v| !matches!(v, Value::Matrix(m) if (m.rows(), m.cols()) == (0, 0)));
        }
        rows.retain(|row| !row.is_empty());
        let rows = match self.typed_concatenate(rows)? {
            Ok(value) => return Ok(value),
            Err(rows) => rows,
        };
        if rows
            .iter()
            .flatten()
            .any(|v| matches!(v, Value::Text(_) | Value::Container(_)))
        {
            return self.concatenate_characters(rows);
        }
        if rows
            .iter()
            .flatten()
            .any(|v| matches!(v, Value::Complex(_)))
        {
            return self.concatenate_complex(rows);
        }
        let logical = rows
            .iter()
            .flatten()
            .any(|v| matches!(v, Value::Logical(_)))
            && rows.iter().flatten().all(|v| {
                matches!(v, Value::Logical(_))
                    || matches!(v,Value::Matrix(m) if m.rows()==0 && m.cols()==0)
            });
        if !logical {
            return Ok(Value::Matrix(Matrix::concatenate(
                rows.into_iter()
                    .map(|row| row.into_iter().map(|v| v.matrix(&self.cancel)).collect())
                    .collect::<Result<Vec<_>>>()?,
            )?));
        }
        let rows: Vec<Vec<L>> = rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .filter(|v| {
                        v.view(&self.cancel)
                            .is_ok_and(|a| a.as_ref().shape() != (0, 0))
                    })
                    .map(|v| v.logical(&self.cancel))
                    .collect()
            })
            .collect::<Result<_>>()?;
        let mut height = 0;
        let mut width = None;
        for row in &rows {
            if row.is_empty() {
                continue;
            }
            let h = row.first().map_or(0, L::rows);
            if row.iter().any(|m| m.rows() != h) {
                return Err("Horizontal concatenation requires matching row counts".into());
            }
            let w = row.iter().map(L::cols).sum();
            if width.is_some_and(|n| n != w) {
                return Err("Matrix rows must have matching widths".into());
            }
            width = Some(w);
            height += h;
        }
        let width = width.unwrap_or(0);
        let mut data = vec![0; Matrix::checked_len(height, width)?];
        let mut r = 0;
        for row in rows {
            let h = row.first().map_or(0, L::rows);
            let mut c = 0;
            for m in row {
                for column in 0..m.cols() {
                    for first in (0..h).step_by(8192) {
                        self.check_cancel()?;
                        let n = (h - first).min(8192);
                        data[(c + column) * height + r + first
                            ..(c + column) * height + r + first + n]
                            .copy_from_slice(&m.data()[column * h + first..column * h + first + n]);
                    }
                }
                c += m.cols();
            }
            r += h;
        }
        Ok(Value::Logical(L::from_canonical_vec(height, width, data)?))
    }
    pub(super) fn logical_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        let result = match name {
            "true" | "false" => {
                arity(name, args, &[0, 1, 2])?;
                let shape = if args.is_empty() {
                    (1, 1)
                } else if args.len() == 1 && args[0].view(&self.cancel)?.as_ref().len() == 2 {
                    let dims = args[0].clone().matrix(&self.cancel)?;
                    (
                        dimension(&Value::Matrix(Matrix::scalar(dims.data()[0])), &self.cancel)?,
                        dimension(&Value::Matrix(Matrix::scalar(dims.data()[1])), &self.cancel)?,
                    )
                } else {
                    let r = dimension(&args[0], &self.cancel)?;
                    (
                        r,
                        if args.len() == 2 {
                            dimension(&args[1], &self.cancel)?
                        } else {
                            r
                        },
                    )
                };
                Value::Logical(L::filled(shape.0, shape.1, name == "true", &self.cancel)?)
            }
            "logical" => {
                arity(name, args, &[1])?;
                // `~`, `&` and conditions read char as codes; conversion does not.
                if args[0].is_character() {
                    return Err(super::character_values::CHAR_TO_LOGICAL.into());
                }
                Value::Logical(args[0].clone().logical(&self.cancel)?)
            }
            "double" => {
                arity(name, args, &[1])?;
                Value::Matrix(args[0].clone().matrix(&self.cancel)?)
            }
            "islogical" | "isnumeric" => {
                arity(name, args, &[1])?;
                Value::Logical(L::scalar(if name == "islogical" {
                    matches!(args[0], Value::Logical(_))
                } else {
                    matches!(
                        args[0],
                        Value::Matrix(_)
                            | Value::Complex(_)
                            | Value::Container(crate::Array::Typed(_))
                    )
                }))
            }
            "isnan" | "isinf" | "isfinite" => {
                arity(name, args, &[1])?;
                Value::Logical(L::predicate(
                    args[0].view(&self.cancel)?.as_ref(),
                    match name {
                        "isnan" => Predicate::IsNan,
                        "isinf" => Predicate::IsInfinite,
                        _ => Predicate::IsFinite,
                    },
                    &self.cancel,
                )?)
            }
            "not" => {
                arity(name, args, &[1])?;
                Value::Logical(args[0].clone().logical(&self.cancel)?.not(&self.cancel)?)
            }
            "and" | "or" | "xor" => {
                arity(name, args, &[2])?;
                Value::Logical(args[0].clone().logical(&self.cancel)?.combine(
                    &args[1].clone().logical(&self.cancel)?,
                    match name {
                        "and" => BooleanOp::And,
                        "or" => BooleanOp::Or,
                        _ => BooleanOp::Xor,
                    },
                    &self.cancel,
                )?)
            }
            "size" | "numel" | "length" => {
                arity(name, args, if name == "size" { &[1, 2] } else { &[1] })?;
                let view = args[0].view(&self.cancel)?;
                let a = view.as_ref();
                let (r, c) = a.shape();
                Value::Matrix(if name == "size" {
                    if args.len() == 1 {
                        Matrix::from_column_slice(1, 2, &[r as f64, c as f64])?
                    } else {
                        Matrix::scalar(match dimension(&args[1], &self.cancel)? {
                            0 => return Err("Dimension index starts at 1".into()),
                            1 => r as f64,
                            2 => c as f64,
                            _ => 1.,
                        })
                    }
                } else {
                    Matrix::scalar(if name == "numel" || a.is_empty() {
                        a.len()
                    } else {
                        r.max(c)
                    } as f64)
                })
            }
            "transpose" if matches!(args.first(), Some(Value::Logical(_))) => {
                arity(name, args, &[1])?;
                let Value::Logical(a) = &args[0] else {
                    unreachable!()
                };
                Value::Logical(a.transpose(&self.cancel)?)
            }
            "nnz" => {
                arity(name, args, &[1])?;
                // Non-char inputs keep the array view and its error texts.
                if !args[0].is_character() {
                    args[0].clone().array(&self.cancel)?.view()?;
                }
                let view = args[0].view(&self.cancel)?;
                let real_view = view.as_ref();
                let at = |i: usize| match real_view {
                    crate::logical::ArrayRef::Numeric(m) => m.data()[i],
                    crate::logical::ArrayRef::Logical(m) => f64::from(m.data()[i]),
                };
                let mut count = 0;
                for first in (0..real_view.len()).step_by(8192) {
                    self.check_cancel()?;
                    for i in first..(first + 8192).min(real_view.len()) {
                        if at(i) != 0. {
                            count += 1;
                        }
                    }
                }
                Value::Matrix(Matrix::scalar(count as f64))
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }
}
