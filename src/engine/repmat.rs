use super::{Engine, Value, shape_arguments};
use crate::Result;

impl Engine {
    pub(super) fn repmat_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let input = args
            .first()
            .ok_or("repmat requires an array and repetition factors")?
            .clone()
            .array(&self.cancel)?;
        input.require_numeric_array()?;
        let (row_copies, column_copies) = shape_arguments::shape(&args[1..], None)?;
        let (rows, cols) = input.repeated_shape(row_copies, column_copies)?;
        let units = rows * cols * if input.is_complex() { 2 } else { 1 };
        if self.evaluated_elements.saturating_add(units) > 32 * 1024 * 1024 {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let output = input.repeated(row_copies, column_copies, &self.cancel)?;
        Ok(match output {
            crate::Array::Character(_)
            | crate::Array::Cell(_)
            | crate::Array::Graphics(_)
            | crate::Array::Struct(_)
            | crate::Array::Handle(_)
            | crate::Array::Typed(_) => {
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
