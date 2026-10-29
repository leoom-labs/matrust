//! Typed powers-of-two admission precedes generic coercion; user resolution stays first.
use super::{Engine, Value};
use crate::{Matrix, Result, array};

impl Engine {
    pub(super) fn powers_of_two_call(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (input, exponent) = match (name, args) {
            ("nextpow2" | "pow2", [Value::Matrix(x)]) => (x, None),
            ("pow2", [Value::Matrix(f), Value::Matrix(e)]) => (f, Some(e)),
            _ => {
                return Err(format!(
                    "{name} requires real double 2-D input: nextpow2(X), pow2(X), or pow2(F,E); other storage classes and overloads are unsupported"
                ));
            }
        };
        let len = if let Some(e) = exponent {
            let (rows, cols) = array::scale_pow2_shape(input, e)?;
            Matrix::checked_len(rows, cols)?
        } else {
            input.len()
        };
        if self
            .evaluated_elements
            .checked_add(len)
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let output = match (name, exponent) {
            ("nextpow2", None) => array::nextpow2(input, &self.cancel)?,
            ("pow2", None) => array::pow2(input, &self.cancel)?,
            ("pow2", Some(e)) => array::scale_pow2(input, e, &self.cancel)?,
            _ => unreachable!("typed powers-of-two admission"),
        };
        Ok(Value::Matrix(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expanded_output_budget_boundary_overflow_and_borrowing() {
        let input = Matrix::from_column_slice(1, 3, &[1., 2., 3.]).unwrap();
        let cases = [
            ("nextpow2", vec![Value::Matrix(input.clone())]),
            ("pow2", vec![Value::Matrix(input.clone())]),
            (
                "pow2",
                vec![
                    Value::Matrix(Matrix::scalar(1.)),
                    Value::Matrix(input.clone()),
                ],
            ),
            (
                "pow2",
                vec![
                    Value::Matrix(input.clone()),
                    Value::Matrix(Matrix::scalar(0.)),
                ],
            ),
        ];
        for (name, args) in cases {
            for used in [32 * 1024 * 1024 - 2, usize::MAX] {
                let engine = Engine {
                    evaluated_elements: used,
                    ..Engine::default()
                };
                assert!(
                    engine
                        .powers_of_two_call(name, &args)
                        .unwrap_err()
                        .contains("evaluation budget")
                );
                assert_eq!(engine.evaluated_elements, used);
            }
            let engine = Engine {
                evaluated_elements: 32 * 1024 * 1024 - 3,
                ..Engine::default()
            };
            let Value::Matrix(result) = engine.powers_of_two_call(name, &args).unwrap() else {
                panic!("real output")
            };
            assert_eq!((result.rows(), result.cols()), (1, 3));
            assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 3);
            assert!(args.iter().any(
                |v| matches!(v, Value::Matrix(m) if m.data().as_ptr() == input.data().as_ptr())
            ));
        }
    }
}
