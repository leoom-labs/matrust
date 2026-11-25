//! Bounded 2-D shape syntax shared by all admitted array storage types.
use super::{Engine, Value};
use crate::{Matrix, Result};

const MAX_DIMENSIONS: usize = 32;
const FILL_CHUNK: usize = 4096;

fn dimension(value: f64, constructor: bool) -> Result<usize> {
    if !value.is_finite()
        || (!constructor && value < 0.)
        || value.fract() != 0.
        || value > crate::matrix::MAX_ELEMENTS as f64
    {
        return Err("Dimension must be a bounded nonnegative integer".into());
    }
    Ok(value.max(0.) as usize)
}

fn scalar_dimension(value: &Value, constructor: bool) -> Result<usize> {
    match value {
        Value::Matrix(a) if a.len() == 1 => dimension(a.data()[0], constructor),
        Value::Logical(a) if a.len() == 1 => Ok(usize::from(a.data()[0])),
        _ => Err("Size arguments must be real double scalars or a row size vector".into()),
    }
}

fn inferred(value: &Value) -> bool {
    matches!(value, Value::Matrix(a) if matches!((a.rows(), a.cols()), (0, 0) | (0, 1) | (1, 0)))
}

// No allocation proportional to the input or to the size vector. Validate the
// vector shape and length before looking at individual entries. Constructor
// callers check the output product; repetition callers first multiply by source
// dimensions. This permits bounded empty outputs with large factor products.
pub(super) fn shape_dims(args: &[Value], elements: Option<usize>) -> Result<Vec<usize>> {
    let mut dims = [1usize; MAX_DIMENSIONS];
    let mut missing = None;
    if elements.is_none()
        && let [Value::Logical(a)] = args
        && a.len() == 1
    {
        let n = usize::from(a.data()[0]);
        return Ok(vec![n, n]);
    }
    let count = if let [Value::Matrix(a)] = args {
        if elements.is_none()
            && a.is_empty()
            && ((a.rows() == 0 && a.cols() == 0) || (a.rows() == 1 && a.cols() == 0))
        {
            return Ok(vec![0, 0]);
        }
        if a.len() == 1 && elements.is_none() {
            let n = dimension(a.data()[0], true)?;
            return Ok(vec![n, n]);
        }
        if a.rows() != 1 || !(2..=MAX_DIMENSIONS).contains(&a.len()) {
            return Err("Size vector must be a real double row with 2..32 entries".into());
        }
        for (dst, &v) in dims.iter_mut().zip(a.data()) {
            *dst = dimension(v, elements.is_none())?;
        }
        a.len()
    } else {
        if !(2..=MAX_DIMENSIONS).contains(&args.len()) {
            return Err("Specify a size vector or 2..32 scalar dimensions (one scalar is also supported by constructors/repmat)".into());
        }
        for (i, value) in args.iter().enumerate() {
            if elements.is_some() && inferred(value) {
                if missing.replace(i).is_some() {
                    return Err("reshape accepts at most one inferred [] dimension".into());
                }
            } else {
                dims[i] = scalar_dimension(value, elements.is_none())?;
            }
        }
        args.len()
    };
    let mut shape = dims[..count].to_vec();
    if let Some(elements) = elements {
        let known = shape
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != missing)
            .try_fold(1usize, |n, (_, d)| n.checked_mul(*d))
            .ok_or("Reshape dimensions overflow")?;
        if let Some(i) = missing {
            if known != 0 && !elements.is_multiple_of(known) {
                return Err("reshape dimensions must divide the number of elements evenly".into());
            }
            // Empty source with a specified zero dimension infers zero. The
            // final element-count check still rejects every nonempty source.
            shape[i] = elements.checked_div(known).unwrap_or(0);
        }
        let product = shape
            .iter()
            .try_fold(1usize, |n, &d| n.checked_mul(d))
            .ok_or("Reshape dimensions overflow")?;
        if product != elements {
            return Err("reshape must preserve the number of elements".into());
        }
    }
    while shape.len() > 2 && shape.last() == Some(&1) {
        shape.pop();
    }
    Ok(shape)
}

pub(super) fn shape(args: &[Value], elements: Option<usize>) -> Result<(usize, usize)> {
    let dims = shape_dims(args, elements)?;
    if dims.len() > 2 {
        return Err(
            "Only two-dimensional arrays and trailing singleton dimensions are supported".into(),
        );
    }
    let cols = dims[1..]
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("Array dimensions overflow")?;
    Ok((dims[0], cols))
}

fn filled(
    rows: usize,
    cols: usize,
    value: f64,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Matrix> {
    let count = Matrix::checked_len(rows, cols)?;
    check()?;
    let mut data = Vec::new();
    data.try_reserve_exact(count)
        .map_err(|_| "Cannot allocate constructor output")?;
    for first in (0..count).step_by(FILL_CHUNK) {
        check()?;
        data.resize((first + FILL_CHUNK).min(count), value);
    }
    check()?;
    let mut result = Matrix::from_column_vec(rows, cols, data)?;
    // The cancellable fill above copied this exact scalar into every slot.
    // Preserve that known fact without rescanning or copying the completed output.
    result.inherit_completed_uniform_fill(value);
    Ok(result)
}

impl Engine {
    pub(super) fn shape_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !matches!(name, "reshape" | "zeros" | "ones") {
            return Ok(None);
        }
        self.check_cancel()?;
        let result = if name == "reshape" {
            let input = args
                .first()
                .ok_or("reshape requires an array and dimensions")?;
            let (r, c) = input.shape(&self.cancel)?;
            let dims = shape_dims(
                &args[1..],
                Some(r.checked_mul(c).ok_or("Array size overflow")?),
            )?;
            let cols = dims[1..]
                .iter()
                .try_fold(1usize, |n, &d| n.checked_mul(d))
                .ok_or("Array dimensions overflow")?;
            let (rows, cols) = (dims[0], cols);
            match input {
                Value::Matrix(a) => Value::Matrix(a.reshape(rows, cols)?.with_dims(&dims)?),
                Value::Logical(a) => Value::Logical(a.reshape(rows, cols)?.with_dims(&dims)?),
                Value::Complex(a) => {
                    Value::Complex(a.reshape(rows, cols, &self.cancel)?.with_dims(&dims)?)
                }
                Value::Container(crate::Array::Typed(a)) => Value::Container(crate::Array::Typed(
                    a.reshaped(rows, cols)?.with_dims(&dims)?,
                )),
                Value::Container(crate::Array::Categorical(a)) => {
                    Value::Container(a.reshaped(rows, cols)?.with_dims(&dims)?.into())
                }
                Value::Container(crate::Array::Character(a)) => Value::Container(
                    a.reshape(rows, cols, &self.cancel)?
                        .with_dims(&dims)?
                        .into(),
                ),
                Value::Text(s) | Value::QuotedString(s) => Value::Container(
                    crate::CharacterRow::from_text(s, &self.cancel)?
                        .reshape(rows, cols, &self.cancel)?
                        .with_dims(&dims)?
                        .into(),
                ),
                _ => {
                    return Err(
                        "reshape currently supports real, logical and complex arrays".into(),
                    );
                }
            }
        } else {
            let dims = shape_dims(args, None)?;
            let rows = dims[0];
            let cols = dims[1..]
                .iter()
                .try_fold(1usize, |n, &d| n.checked_mul(d))
                .ok_or("Array dimensions overflow")?;
            let count = Matrix::checked_len(rows, cols)?;
            self.preflight_elements(count)?;
            Value::Matrix(
                filled(rows, cols, if name == "ones" { 1. } else { 0. }, || {
                    self.check_cancel()
                })?
                .with_dims(&dims)?,
            )
        };
        self.check_cancel()?;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_budget_is_checked_before_output_allocation() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024,
            ..Engine::default()
        };
        let args = [
            Value::Matrix(Matrix::scalar(4096.)),
            Value::Matrix(Matrix::scalar(4096.)),
        ];
        assert!(
            engine
                .shape_call("ones", &args)
                .unwrap_err()
                .contains("evaluation budget")
        );
        let empty = [
            Value::Matrix(Matrix::scalar(0.)),
            Value::Matrix(Matrix::scalar(4096.)),
        ];
        assert!(engine.shape_call("ones", &empty).is_ok());
    }

    #[test]
    fn constructor_cancels_after_a_completed_chunk_and_on_final_check() {
        for stop_at in [3, 5] {
            let mut checks = 0;
            let result = filled(2 * FILL_CHUNK + 1, 1, 1., || {
                checks += 1;
                if checks == stop_at {
                    Err("Calculation cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.unwrap_err().contains("cancelled"));
            assert_eq!(checks, stop_at);
        }
        let result = filled(2 * FILL_CHUNK + 1, 1, 1., || Ok(())).unwrap();
        assert!(result.data().iter().all(|v| v.to_bits() == 1f64.to_bits()));
    }
}
