use super::{Engine, Value, shape_arguments};
use crate::{Matrix, Result};

impl Engine {
    pub(super) fn repmat_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let input = args
            .first()
            .ok_or("repmat requires an array and repetition factors")?
            .clone()
            .array(&self.cancel)?;
        input.require_numeric_array()?;
        if input.is_nd() {
            let source_dims = input.dims();
            let mut reps = shape_arguments::shape_dims(&args[1..], None)?;
            if reps.len() < 2 {
                reps.resize(2, reps.first().copied().unwrap_or(1));
            }
            let rank = source_dims.len().max(reps.len());
            reps.resize(rank, 1);
            let mut dims = source_dims.to_vec();
            dims.resize(rank, 1);
            for (i, dim) in dims.iter_mut().enumerate() {
                *dim = dim
                    .checked_mul(reps[i])
                    .ok_or("Repetition shape overflow")?;
            }
            let count = super::nd::shape_product(&dims)?;
            let mut values = Vec::with_capacity(count);
            for linear in 0..count {
                if linear.is_multiple_of(8192) {
                    self.check_cancel()?;
                }
                let mut q = linear;
                let mut src = 0;
                let mut stride = 1;
                for (axis, &dimension) in dims.iter().enumerate() {
                    let coord = q % dimension.max(1);
                    q /= dimension.max(1);
                    let d = source_dims.get(axis).copied().unwrap_or(1);
                    src += (coord % d.max(1)) * stride;
                    stride *= d.max(1);
                }
                values.push(match &input {
                    crate::Array::Numeric(a) => a.data()[src],
                    crate::Array::Logical(a) => f64::from(a.data()[src]),
                    crate::Array::Complex(_) => {
                        return Err("Complex N-D repmat is not implemented".into());
                    }
                    _ => unreachable!(),
                });
            }
            let rows = dims[0];
            let cols = super::nd::shape_product(&dims[1..])?;
            return Ok(Value::Matrix(
                Matrix::from_column_vec(rows, cols, values)?.with_dims(&dims)?,
            ));
        }
        let (row_copies, column_copies) = shape_arguments::shape(&args[1..], None)?;
        let (rows, cols) = input.repeated_shape(row_copies, column_copies)?;
        let units = rows * cols * if input.is_complex() { 2 } else { 1 };
        self.preflight_elements(units)?;
        let output = input.repeated(row_copies, column_copies, &self.cancel)?;
        Ok(match output {
            crate::Array::Str(_)
            | crate::Array::Character(_)
            | crate::Array::Cell(_)
            | crate::Array::Graphics(_)
            | crate::Array::Struct(_)
            | crate::Array::Handle(_)
            | crate::Array::Typed(_)
            | crate::Array::Categorical(_)
            | crate::Array::Datetime(_)
            | crate::Array::Duration(_)
            | crate::Array::Table(_)
            | crate::Array::Sparse(_) => {
                return Err("Repetition of character, cell, struct, function handle or graphics handle arrays is not implemented".into());
            }
            crate::Array::Numeric(a) => Value::Matrix(a),
            crate::Array::Logical(a) => Value::Logical(a),
            crate::Array::Complex(a) => Value::Complex(a),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Matrix;

    #[test]
    fn repmat_rejects_empty_factors_across_array_types() {
        let mut engine = Engine::default();
        for input in ["1", "ones(2,2,2)", "{1}", "'abc'", "\"abc\""] {
            let call = format!("repmat({input},[])");
            let error = engine.execute(&call).unwrap_err();
            assert!(
                error.ends_with(
                    "Replication factors must be a row vector of integers or integer scalars."
                ),
                "{call}: {error}"
            );
        }
    }

    #[test]
    fn repetition_preflights_statement_budget_before_materializing() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024,
            ..Engine::default()
        };
        let args = [
            Value::Matrix(Matrix::scalar(1.)),
            Value::Matrix(Matrix::scalar(4096.)),
        ];
        assert!(
            engine
                .repmat_call(&args)
                .unwrap_err()
                .contains("evaluation budget")
        );
    }
}
