//! Typed admission before generic text/graphics conversion; evaluated arguments remain unchanged.
use super::{Engine, Value};
use crate::{Result, array};

impl Engine {
    pub(super) fn near_zero_call(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let [Value::Matrix(input)] = args else {
            return Err(format!(
                "{name} requires exactly one real double input; logical, complex, character, string, cell and graphics inputs are unsupported"
            ));
        };
        if self
            .evaluated_elements
            .checked_add(input.len())
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let output = match name {
            "log1p" => array::log1p(input, &self.cancel)?,
            "expm1" => array::expm1(input, &self.cancel)?,
            _ => unreachable!("near-zero dispatch admits only log1p/expm1"),
        };
        Ok(Value::Matrix(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Matrix;

    #[test]
    fn budget_preflight_boundary_overflow_and_aliases_do_not_double_charge() {
        let input = Matrix::from_column_slice(1, 3, &[0., -0., 0.]).unwrap();
        let args = [Value::Matrix(input.clone())];
        for name in ["log1p", "expm1"] {
            for used in [32 * 1024 * 1024 - 2, usize::MAX] {
                let engine = Engine {
                    evaluated_elements: used,
                    ..Engine::default()
                };
                assert!(
                    engine
                        .near_zero_call(name, &args)
                        .unwrap_err()
                        .contains("evaluation budget")
                );
                assert_eq!(engine.evaluated_elements, used);
            }
            let engine = Engine {
                evaluated_elements: 32 * 1024 * 1024 - 3,
                ..Engine::default()
            };
            let Value::Matrix(result) = engine.near_zero_call(name, &args).unwrap() else {
                panic!("real result")
            };
            assert!(
                result
                    .data()
                    .iter()
                    .zip(input.data())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
            assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 3);
            let Value::Matrix(shared) = &args[0] else {
                panic!("real argument")
            };
            assert_eq!(shared.data().as_ptr(), input.data().as_ptr());
        }
    }
}
