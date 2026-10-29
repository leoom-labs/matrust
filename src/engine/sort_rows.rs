use super::{Engine, Value};
use crate::{
    Array, Matrix, Result,
    array::{MissingPlacement, SortOrder},
    matrix::MAX_ELEMENTS,
};

impl Engine {
    pub(super) fn sort_rows_call(
        &self,
        args: &[Value],
        want_indices: bool,
    ) -> Result<(Value, Option<Matrix>)> {
        if matches!(args.first(), Some(Value::Complex(_))) {
            return self.cxr_sortrows(args, want_indices);
        }
        if args
            .first()
            .is_some_and(|v| v.is_character() || matches!(v, Value::Container(Array::Cell(_))))
        {
            return self.cel_sort(args, true, want_indices);
        }
        self.check_cancel()?;
        if args.is_empty() || args.len() > 5 {
            return Err("sortrows expects A, optional signed column vector/direction, then optional 'MissingPlacement',value".into());
        }
        let input = args[0].clone().array(&self.cancel)?;
        let mut at = 1;
        let mut columns = None;
        if let Some(Value::Matrix(value)) = args.get(at) {
            if value.is_empty() || (value.rows() != 1 && value.cols() != 1) {
                return Err(
                    "sortrows columns must be a nonempty real integer vector; [] is unsupported"
                        .into(),
                );
            }
            let mut result = Vec::new();
            result
                .try_reserve_exact(value.len())
                .map_err(|_| "Cannot allocate sortrows column keys")?;
            for (index, &column) in value.data().iter().enumerate() {
                if index.is_multiple_of(8192) {
                    self.check_cancel()?;
                }
                if !column.is_finite()
                    || column == 0.
                    || column.fract() != 0.
                    || column.abs() > input.cols() as f64
                    || column.abs() > MAX_ELEMENTS as f64
                {
                    return Err(
                        "sortrows column indices must be nonzero integers within the input columns"
                            .into(),
                    );
                }
                result.push(column as i32);
            }
            columns = Some(result);
            at += 1;
        }
        let mut direction = None;
        if let Some(Value::Text(text)) = args.get(at)
            && matches!(text.as_str(), "ascend" | "descend")
        {
            direction = Some(if text == "ascend" {
                SortOrder::Ascend
            } else {
                SortOrder::Descend
            });
            at += 1;
        }
        let mut missing = MissingPlacement::Auto;
        if at < args.len() {
            if args.len() - at != 2
                || !matches!(&args[at], Value::Text(text) if text == "MissingPlacement")
            {
                return Err("sortrows supports signed column vector, scalar direction and one 'MissingPlacement' pair; tables, direction arrays and other options are unsupported".into());
            }
            missing = match &args[at + 1] {
                Value::Text(s) if s == "auto" => MissingPlacement::Auto,
                Value::Text(s) if s == "first" => MissingPlacement::First,
                Value::Text(s) if s == "last" => MissingPlacement::Last,
                _ => {
                    return Err(
                        "sortrows MissingPlacement must be 'auto', 'first' or 'last'".into(),
                    );
                }
            };
        }
        let outputs = input
            .len()
            .checked_add(if want_indices { input.rows() } else { 0 })
            .and_then(|n| self.evaluated_elements.checked_add(n));
        if outputs.is_none_or(|n| n > 32 * 1024 * 1024) {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let (output, indices) = input.sort_rows(
            columns.as_deref(),
            direction,
            missing,
            want_indices,
            &self.cancel,
        )?;
        Ok((
            match output {
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
    fn row_index_charge_uses_rows_not_full_matrix_and_rejects_before_kernel() {
        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 7,
            ..Engine::default()
        };
        let a = Matrix::from_column_slice(2, 3, &[2., 1., 4., 3., 6., 5.]).unwrap();
        let args = [Value::Matrix(a.clone())];
        assert!(
            e.sort_rows_call(&args, true)
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert!(e.sort_rows_call(&args, false).is_ok());
        let Value::Matrix(shared) = &args[0] else {
            panic!("Numeric input");
        };
        assert_eq!(shared.data().as_ptr(), a.data().as_ptr());
    }
}
