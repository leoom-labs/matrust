//! Typed polynomial admission before generic text and graphics projection.
use super::{Engine, Value};
use crate::{Result, array::evaluate_polynomial};

impl Engine {
    pub(super) fn polyval_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (p, x, mu) = match args {
            [Value::Matrix(p), Value::Matrix(x)] => (p, x, None),
            [Value::Matrix(p), Value::Matrix(x), Value::Matrix(s), Value::Matrix(mu)]
                if s.is_empty() => (p, x, Some(mu)),
            _ => return Err("polyval requires real double p,x or p,x,[],mu; error estimates and other input types are unsupported".into()),
        };
        if self
            .evaluated_elements
            .checked_add(x.len())
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let scaled;
        let x = if let Some(mu) = mu {
            if mu.len() != 2
                || (mu.rows() != 1 && mu.cols() != 1)
                || !mu.data().iter().all(|v| v.is_finite())
                || mu.data()[1] <= 0.
            {
                return Err("polyval: mu must contain a finite center and positive scale".into());
            }
            self.check_cancel()?;
            let mut values = Vec::with_capacity(x.len());
            for (i, value) in x.data().iter().enumerate() {
                if i.is_multiple_of(8192) {
                    self.check_cancel()?;
                }
                values.push((value - mu.data()[0]) / mu.data()[1]);
            }
            scaled = crate::Matrix::from_column_vec(x.rows(), x.cols(), values)?;
            &scaled
        } else {
            x
        };
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
