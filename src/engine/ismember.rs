//! Typed membership admission without text/graphics coercion.
use super::{Engine, Value};
use crate::{Array, Matrix, Result};

impl Engine {
    pub(super) fn ismember_call(&self, args: &[Value], outputs: [bool; 2]) -> Result<Vec<Value>> {
        if args
            .iter()
            .take(2)
            .any(|v| v.is_character() || matches!(v, Value::Container(Array::Cell(_))))
        {
            return self.cel_ismember(args, outputs);
        }
        self.check_cancel()?;
        if args.len() != 2 {
            return Err(
                "ismember expects two real/logical vector inputs; flags are unsupported".into(),
            );
        }
        let typed = |value: &Value| -> Result<Array> {
            match value {
                Value::Matrix(a) => Ok(Array::Numeric(a.clone())),
                Value::Logical(a) => Ok(Array::Logical(a.clone())),
                _ => Err("ismember supports real double or logical inputs only; complex, text, cell and graphics inputs are unsupported".into()),
            }
        };
        let a = typed(&args[0])?;
        let b = typed(&args[1])?;
        for input in [&a, &b] {
            if !input.is_empty() && input.rows() != 1 && input.cols() != 1 {
                return Err(
                    "ismember supports vectors and shaped empties, not nonempty matrices".into(),
                );
            }
        }
        // Count requested logical output conservatively as existing Engine units.
        if a.len()
            .checked_mul(usize::from(outputs[0]) + usize::from(outputs[1]))
            .and_then(|units| self.evaluated_elements.checked_add(units))
            .is_none_or(|units| units > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let result = a.membership(&b, outputs, &self.cancel)?;
        let empty = || Matrix::filled(0, 0, 0.).map(Value::Matrix);
        let lia = result
            .membership
            .map_or_else(empty, |a| Ok(Value::Logical(a)))?;
        let loc = result
            .locations
            .map_or_else(empty, |a| Ok(Value::Matrix(a)))?;
        self.check_cancel()?;
        Ok(vec![lia, loc])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requested_result_budget_precedes_work_and_preserves_aliases() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 4,
            ..Engine::default()
        };
        let a = Matrix::from_column_slice(1, 3, &[2., 1., 3.]).unwrap();
        let args = [Value::Matrix(a.clone()), Value::Matrix(a.clone())];
        assert!(
            engine
                .ismember_call(&args, [true; 2])
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert!(engine.ismember_call(&args, [true, false]).is_ok());
        assert!(engine.ismember_call(&args, [false, true]).is_ok());
        assert!(engine.ismember_call(&args, [false; 2]).is_ok());
        let Value::Matrix(shared) = &args[0] else {
            panic!("real")
        };
        assert_eq!(a.data().as_ptr(), shared.data().as_ptr());
        assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 4);
        assert!(
            engine
                .ismember_call(
                    &[Value::Text("x".repeat(65537)), args[1].clone()],
                    [false; 2]
                )
                .is_err()
        );
    }
}
