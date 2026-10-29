//! Typed polynomial admission before generic text and graphics projection.
use super::{Engine, Value};
use crate::{Result, array::evaluate_polynomial};

impl Engine {
    pub(super) fn polyval_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let [Value::Matrix(p), Value::Matrix(x)] = args else {
            return Err("polyval requires exactly two real double inputs p,x; logical/complex/text/cell/graphics inputs, scaling and error estimates are unsupported".into());
        };
        if self
            .evaluated_elements
            .checked_add(x.len())
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(Value::Matrix(evaluate_polynomial(p, x, &self.cancel)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Matrix;
    #[test]
    fn evaluated_output_budget_is_checked_without_copying_inputs() {
        let p = Matrix::scalar(3.);
        let x = Matrix::from_column_slice(1, 3, &[1., 2., 3.]).unwrap();
        let args = [Value::Matrix(p.clone()), Value::Matrix(x.clone())];
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 2,
            ..Engine::default()
        };
        assert!(
            engine
                .polyval_call(&args)
                .unwrap_err()
                .contains("evaluation budget")
        );
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 3,
            ..Engine::default()
        };
        let Value::Matrix(result) = engine.polyval_call(&args).unwrap() else {
            panic!("real output")
        };
        assert_eq!(result.data(), &[3., 3., 3.]);
        let Value::Matrix(shared) = &args[1] else {
            panic!("real query")
        };
        assert_eq!(shared.data().as_ptr(), x.data().as_ptr());
        assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 3);
    }
}
