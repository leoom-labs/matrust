use super::{Engine, Value};
use crate::{
    Array, Matrix, Result,
    array::{MissingPlacement, SortOrder},
    matrix::MAX_ELEMENTS,
};

impl Engine {
    pub(super) fn sort_call(
        &self,
        args: &[Value],
        want_indices: bool,
    ) -> Result<(Value, Option<Matrix>)> {
        if matches!(args.first(), Some(Value::Complex(_))) {
            return self.cxr_sort(args, want_indices);
        }
        if args
            .first()
            .is_some_and(|v| v.is_character() || matches!(v, Value::Container(Array::Cell(_))))
        {
            return self.cel_sort(args, false, want_indices);
        }
        self.check_cancel()?;
        if args.is_empty() || args.len() > 5 {
            return Err(
                "sort expects A, optional dimension/order, and optional 'MissingPlacement',value"
                    .into(),
            );
        }
        let input = args[0].clone().array(&self.cancel)?;
        let mut dim = if input.rows() == 1 && input.cols() != 1 {
            2
        } else {
            1
        };
        let mut order = SortOrder::Ascend;
        let mut missing = MissingPlacement::Auto;
        let mut at = 1;
        if let Some(Value::Matrix(value)) = args.get(at) {
            let value = value.as_scalar()?;
            if !value.is_finite()
                || value < 1.
                || value.fract() != 0.
                || value > MAX_ELEMENTS as f64
            {
                return Err(format!(
                    "sort dimension must be a positive integer <= {MAX_ELEMENTS}"
                ));
            }
            dim = value as usize;
            at += 1;
        }
        if let Some(Value::Text(text)) = args.get(at)
            && (text == "ascend" || text == "descend")
        {
            order = if text == "ascend" {
                SortOrder::Ascend
            } else {
                SortOrder::Descend
            };
            at += 1;
        }
        if at < args.len() {
            let pair = &args[at..];
            if pair.len() != 2 || !matches!(&pair[0], Value::Text(s) if s == "MissingPlacement") {
                return Err("sort supports a numeric dimension, 'ascend'/'descend', then at most one 'MissingPlacement' pair; other options are unsupported".into());
            }
            missing = match &pair[1] {
                Value::Text(s) if s == "auto" => MissingPlacement::Auto,
                Value::Text(s) if s == "first" => MissingPlacement::First,
                Value::Text(s) if s == "last" => MissingPlacement::Last,
                _ => return Err("sort MissingPlacement must be 'auto', 'first' or 'last'".into()),
            };
        }
        // Evaluation charges input arguments before entering this helper and
        // outputs on return. Reject over-budget results before kernel allocation.
        let outputs = input
            .len()
            .checked_mul(1 + usize::from(want_indices))
            .and_then(|n| self.evaluated_elements.checked_add(n));
        if outputs.is_none_or(|n| n > 32 * 1024 * 1024) {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let (sorted, indices) =
            input.sort_dimension(dim, order, missing, want_indices, &self.cancel)?;
        Ok((
            match sorted {
                Array::Numeric(a) => Value::Matrix(a),
                Array::Logical(a) => Value::Logical(a),
                Array::Complex(a) => Value::Complex(a),
                Array::Typed(a) => Value::Container(Array::Typed(a)),
                Array::Character(_)
                | Array::Cell(_)
                | Array::Graphics(_)
                | Array::Struct(_)
                | Array::Handle(_) => {
                    return Err("Sorting character, cell, struct, function handle or graphics handle arrays is not implemented".into());
                }
            },
            indices,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sort_preflights_output_budget_without_charging_or_copying_input() {
        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 5,
            ..Default::default()
        };
        let a = Matrix::from_column_slice(1, 3, &[3., 1., 2.]).unwrap();
        let before = a.data().as_ptr();
        let args = [Value::Matrix(a.clone())];
        assert!(
            e.sort_call(&args, true)
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert_eq!(e.evaluated_elements, 32 * 1024 * 1024 - 5);
        assert_eq!(a.data().as_ptr(), before);
        assert!(e.sort_call(&args, false).unwrap().1.is_none());
    }
}
