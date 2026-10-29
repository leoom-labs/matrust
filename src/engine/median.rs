use super::{Engine, Value};
use crate::{
    Result,
    matrix::{MAX_ELEMENTS, MedianAxis},
};

impl Engine {
    pub(super) fn median_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let mut count = args.len();
        let mut omit_nan = false;
        if let Some(Value::Text(flag)) = args.last() {
            match flag.to_ascii_lowercase().as_str() {
                "omitnan" | "omitmissing" => {
                    omit_nan = true;
                    count -= 1;
                }
                "includenan" | "includemissing" => {
                    count -= 1;
                }
                _ => {}
            }
        }
        if !(1..=2).contains(&count) {
            return Err("Use median(A[, dim|'all'][, 'omitnan'|'includenan']); weights, vector dimensions and multiple outputs are not implemented".into());
        }
        let input = match &args[0] {
            Value::Matrix(matrix) => matrix,
            Value::Logical(_) => return Err("median logical input is pending verification of MATLAB output-class and empty semantics".into()),
            _ => return Err("median requires a real numeric array".into()),
        };
        let axis = if count == 1 {
            MedianAxis::Default
        } else {
            match &args[1] {
            Value::Text(flag) if flag.eq_ignore_ascii_case("all") => MedianAxis::All,
            Value::Matrix(dim) if dim.len() == 1 => {
                let value = dim.as_scalar()?;
                if !value.is_finite() || value < 1. || value.fract() != 0. || value > MAX_ELEMENTS as f64 {
                    return Err(format!("median dimension must be a positive integer <= {MAX_ELEMENTS}"));
                }
                MedianAxis::Dimension(value as usize)
            }
            _ => return Err("median dimension must be a positive integer or 'all'; vector dimensions and weights are not implemented".into()),
        }
        };
        let outputs = self
            .evaluated_elements
            .checked_add(input.median_output_len(axis)?);
        if outputs.is_none_or(|n| n > 32 * 1024 * 1024) {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(Value::Matrix(input.median(axis, omit_nan, &self.cancel)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn median_preflights_result_budget_and_does_not_charge_or_mutate_input() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 1,
            ..Default::default()
        };
        let input = crate::Matrix::from_column_slice(2, 2, &[1., 2., 3., 4.]).unwrap();
        let pointer = input.data().as_ptr();
        let args = [Value::Matrix(input.clone())];
        assert!(
            engine
                .median_call(&args)
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert_eq!(input.data().as_ptr(), pointer);
        assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 1);
        assert!(
            engine
                .median_call(&[args[0].clone(), Value::Text("all".into())])
                .is_ok()
        );
    }
}
