//! Typed unique admission before generic character/graphics coercion.
use super::{Engine, Value};
use crate::{Array, Matrix, Result, array::UniqueOrder};

impl Engine {
    pub(super) fn unique_call(&self, args: &[Value], outputs: [bool; 3]) -> Result<Vec<Value>> {
        if args
            .first()
            .is_some_and(|v| v.is_character() || matches!(v, Value::Container(Array::Cell(_))))
        {
            return self.cel_unique(args, outputs);
        }
        self.check_cancel()?;
        if args.is_empty() {
            return Err(
                "unique expects one real/logical array and optional 'rows', 'sorted'/'stable', or 'first'/'last'"
                    .into(),
            );
        }
        let mut rows = false;
        let mut sorted = false;
        let mut stable = false;
        let mut first = false;
        let mut last = false;
        for value in &args[1..] {
            let Some(flag) = self.set_flag_text(value)? else {
                return Err(
                    "unique options must be 'rows', 'sorted', 'stable', 'first' or 'last'; string arrays and other options are unsupported"
                        .into(),
                );
            };
            match flag.as_str() {
                "rows" => rows = true,
                "sorted" => sorted = true,
                "stable" => stable = true,
                "first" => first = true,
                "last" => last = true,
                "legacy" => {
                    return Err("unique: 'legacy' is not supported for real/logical input".into());
                }
                _ => {
                    return Err(
                        "unique supports only 'rows', 'sorted', 'stable', 'first' or 'last'".into(),
                    );
                }
            }
        }
        if sorted && stable {
            return Err("unique: cannot specify both 'sorted' and 'stable'".into());
        }
        if first && last {
            return Err("unique: cannot specify both 'first' and 'last'".into());
        }
        let input = match &args[0] {
            Value::Matrix(a) => Array::Numeric(a.clone()),
            Value::Logical(a) => Array::Logical(a.clone()),
            Value::Container(Array::Typed(a)) => Array::Typed(a.clone()),
            _ => return Err("unique supports real double or logical arrays only; complex, text, cell and graphics inputs are unsupported".into()),
        };
        if rows && input.is_nd() {
            return Err("unique: 'rows' is not supported for N-D arrays".into());
        }
        let order = if stable {
            UniqueOrder::Stable
        } else {
            UniqueOrder::Sorted
        };
        // Conservative per-request upper bound before any index scratch. Actual
        // C/ia allocate only K after grouping. Ignored outputs cost zero here.
        let item_count = if rows { input.rows() } else { input.len() };
        let logical_1d = !rows && matches!(input, Array::Logical(_));
        let values_bound = if logical_1d {
            input.len().min(2)
        } else {
            input.len()
        };
        let first_bound = if logical_1d {
            input.len().min(2)
        } else {
            item_count
        };
        let units = values_bound
            .checked_mul(usize::from(outputs[0]))
            .and_then(|u| u.checked_add(if outputs[1] { first_bound } else { 0 }))
            .and_then(|u| u.checked_add(if outputs[2] { item_count } else { 0 }))
            .unwrap_or(usize::MAX);
        self.preflight_elements(units)?;
        let result = input.unique_with_options(order, rows, last, outputs, &self.cancel)?;
        let empty = || Matrix::filled(0, 0, 0.).map(Value::Matrix);
        let values = match result.values {
            Some(Array::Numeric(a)) => Value::Matrix(a),
            Some(Array::Logical(a)) => Value::Logical(a),
            Some(Array::Typed(a)) => Value::Container(Array::Typed(a)),
            None => empty()?,
            _ => unreachable!("unique preserves admitted real/logical storage"),
        };
        let first = result
            .first_indices
            .map_or_else(empty, |a| Ok(Value::Matrix(a)))?;
        let groups = result
            .group_indices
            .map_or_else(empty, |a| Ok(Value::Matrix(a)))?;
        self.check_cancel()?;
        Ok(vec![values, first, groups])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requested_output_budget_and_type_preflight_preserve_shared_input() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 5,
            ..Engine::default()
        };
        let a = Matrix::from_column_slice(1, 3, &[2., 1., 2.]).unwrap();
        let args = [Value::Matrix(a.clone())];
        assert!(
            engine
                .unique_call(&args, [true, true, false])
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert!(engine.unique_call(&args, [false, false, true]).is_ok());
        assert!(engine.unique_call(&args, [false; 3]).is_ok());
        let Value::Matrix(shared) = &args[0] else {
            panic!("matrix input");
        };
        assert_eq!(a.data().as_ptr(), shared.data().as_ptr());
        assert!(
            engine
                .unique_call(&[Value::Text("x".repeat(65537))], [false; 3])
                .unwrap_err()
                .contains("65536")
        );
        assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 5);
    }
    #[test]
    fn logical_value_and_first_index_budget_is_bounded_by_two_groups() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 8,
            ..Engine::default()
        };
        let a =
            crate::LogicalMatrix::from_column_vec(1, 4, vec![1, 0, 1, 0], &engine.cancel).unwrap();
        let result = engine.unique_call(&[Value::Logical(a)], [true; 3]).unwrap();
        let Value::Matrix(ic) = &result[2] else {
            panic!("double ic");
        };
        assert_eq!(ic.data(), &[2., 1., 2., 1.]);
        let numeric = Matrix::from_column_slice(1, 4, &[1., 0., 1., 0.]).unwrap();
        assert!(
            engine
                .unique_call(&[Value::Matrix(numeric)], [true; 3])
                .unwrap_err()
                .contains("evaluation budget")
        );
    }
}
