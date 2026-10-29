//! Strict typed history admission before generic text/graphics conversion.
use super::{Engine, Value};
use crate::{Matrix, Result, dsp};

fn history(matrix: &Matrix) -> Result<&[f64]> {
    if matrix.rows() == 1 || matrix.cols() == 1 || (matrix.rows(), matrix.cols()) == (0, 0) {
        Ok(matrix.data())
    } else {
        Err("filtic histories must be row/column vectors or 0x0, 1x0, 0x1 empties; matrix histories are unsupported".into())
    }
}

impl Engine {
    pub(super) fn filtic_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        if !(3..=4).contains(&args.len()) {
            return Err(
                "filtic expects b,a,y and optional x history; other overloads are unsupported"
                    .into(),
            );
        }
        let numeric = |i: usize| -> Result<&Matrix> {
            match &args[i] {
                Value::Matrix(matrix) => Ok(matrix),
                _ => Err("filtic requires real double coefficients and histories; logical/complex/text/cell/graphics inputs are unsupported".into()),
            }
        };
        let b = dsp::vector(numeric(0)?)?;
        let a = dsp::vector(numeric(1)?)?;
        let y = history(numeric(2)?)?;
        let x = if args.len() == 4 {
            Some(history(numeric(3)?)?)
        } else {
            None
        };
        let order = b.len().max(a.len()) - 1;
        if self
            .evaluated_elements
            .checked_add(order)
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(Value::Matrix(dsp::filtic(b, a, y, x, &self.cancel)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluated_state_budget_precedes_allocation_and_borrows_all_inputs() {
        let b = Matrix::from_column_slice(1, 3, &[1., 2., 3.]).unwrap();
        let x = Matrix::from_column_slice(2, 1, &[4., 5.]).unwrap();
        let args = [
            Value::Matrix(b.clone()),
            Value::Matrix(Matrix::scalar(1.)),
            Value::Matrix(Matrix::from_column_vec(0, 0, vec![]).unwrap()),
            Value::Matrix(x.clone()),
        ];
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 1,
            ..Engine::default()
        };
        assert!(
            engine
                .filtic_call(&args)
                .unwrap_err()
                .contains("evaluation budget")
        );
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 2,
            ..Engine::default()
        };
        let Value::Matrix(z) = engine.filtic_call(&args).unwrap() else {
            panic!("real state");
        };
        assert_eq!(z.data(), &[23., 12.]);
        for (argument, original) in [(&args[0], &b), (&args[3], &x)] {
            let Value::Matrix(matrix) = argument else {
                panic!("real input")
            };
            assert_eq!(matrix.data().as_ptr(), original.data().as_ptr());
        }
        assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 2);
    }
}
