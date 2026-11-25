//! Typed set admission with exact counted requested-output budgeting.
use super::{Engine, Value};
use crate::{
    Array, Matrix, Result,
    array::{SetOperation, UniqueOrder},
};

pub(super) fn operation(name: &str) -> Option<SetOperation> {
    match name {
        "union" => Some(SetOperation::Union),
        "intersect" => Some(SetOperation::Intersect),
        "setdiff" => Some(SetOperation::Difference),
        "setxor" => Some(SetOperation::SymmetricDifference),
        _ => None,
    }
}
impl Engine {
    pub(super) fn set_flag_text(&self, value: &Value) -> Result<Option<String>> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Ok(Some(s.to_ascii_lowercase())),
            Value::Container(Array::Character(a)) if a.rows() <= 1 && a.cols() <= 16 => {
                Ok(Some(self.text_value(value.clone())?.to_ascii_lowercase()))
            }
            Value::Container(Array::Str(s)) if s.rows() == 1 && s.cols() == 1 => Ok(s
                .data()
                .first()
                .and_then(|v| v.as_deref())
                .map(str::to_ascii_lowercase)),
            _ => Ok(None),
        }
    }

    pub(super) fn set_operation_call(
        &self,
        operation: SetOperation,
        args: &[Value],
        outputs: [bool; 3],
    ) -> Result<Vec<Value>> {
        if args
            .iter()
            .take(2)
            .any(|v| v.is_character() || matches!(v, Value::Container(Array::Cell(_))))
        {
            return self.cel_set(operation, args, outputs);
        }
        self.check_cancel()?;
        if args.len() < 2 {
            return Err(
                "Set operations expect two real/logical arrays and optional 'rows', 'sorted' or 'stable'"
                    .into(),
            );
        }
        let mut rows = false;
        let mut sorted = false;
        let mut stable = false;
        for value in &args[2..] {
            let Some(flag) = self.set_flag_text(value)? else {
                return Err("Set order must be 'rows', 'sorted' or 'stable'; string arrays and other options are unsupported".into());
            };
            match flag.as_str() {
                "rows" => rows = true,
                "sorted" => sorted = true,
                "stable" => stable = true,
                _ => return Err("Set operations support only 'rows', 'sorted' or 'stable'; legacy and missing-value options are unsupported".into()),
            }
        }
        if sorted && stable {
            return Err("Set operations cannot combine 'sorted' and 'stable'".into());
        }
        let typed = |value: &Value| -> Result<Array> {
            match value {
                Value::Matrix(a) => Ok(Array::Numeric(a.clone())),
                Value::Logical(a) => Ok(Array::Logical(a.clone())),
                Value::Container(Array::Typed(a)) => Ok(Array::Typed(a.clone())),
                _ => Err("Set operations support real numeric or logical inputs only; complex, text, cell and graphics inputs are unsupported".into()),
            }
        };
        let a = typed(&args[0])?;
        let b = typed(&args[1])?;
        let order = if stable {
            UniqueOrder::Stable
        } else {
            UniqueOrder::Sorted
        };
        let units = (32usize * 1024 * 1024)
            .checked_sub(self.evaluated_elements)
            .ok_or("Statement exceeds its 256 MiB evaluation budget")?;
        // The kernel counts selected groups and contributing indices before
        // allocating any output; logical cardinality is at most two, not N.
        let result =
            a.set_operation_rows_bounded(&b, operation, order, rows, outputs, units, &self.cancel)?;
        let empty = || Matrix::filled(0, 0, 0.).map(Value::Matrix);
        let values = match result.values {
            Some(Array::Numeric(a)) => Value::Matrix(a),
            Some(Array::Logical(a)) => Value::Logical(a),
            Some(Array::Typed(a)) => Value::Container(Array::Typed(a)),
            None => empty()?,
            _ => unreachable!("set results preserve admitted real/logical types"),
        };
        let first_a = result
            .first_a
            .map_or_else(empty, |a| Ok(Value::Matrix(a)))?;
        let first_b = result
            .first_b
            .map_or_else(empty, |a| Ok(Value::Matrix(a)))?;
        self.check_cancel()?;
        Ok(vec![values, first_a, first_b])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_cardinality_and_masks_are_admitted_before_output_allocation() {
        let mut e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 2,
            ..Engine::default()
        };
        let numeric = Matrix::filled(1, 10001, 1.).unwrap();
        let logical = crate::LogicalMatrix::filled(1, 10001, true, &e.cancel).unwrap();
        for input in [Value::Matrix(numeric.clone()), Value::Logical(logical)] {
            let args = [input.clone(), input];
            assert!(
                e.set_operation_call(SetOperation::Union, &args, [true; 3])
                    .is_ok()
            );
            assert!(
                e.set_operation_call(SetOperation::Intersect, &args, [true; 3])
                    .unwrap_err()
                    .contains("evaluation budget")
            );
            assert!(
                e.set_operation_call(SetOperation::Intersect, &args, [false, true, true])
                    .is_ok()
            );
            assert!(
                e.set_operation_call(SetOperation::Intersect, &args, [false; 3])
                    .is_ok()
            );
        }
        e.evaluated_elements -= 1;
        let args = [
            Value::Matrix(numeric.clone()),
            Value::Matrix(numeric.clone()),
        ];
        let result = e
            .set_operation_call(SetOperation::Intersect, &args, [true; 3])
            .unwrap();
        assert!(matches!(&result[0], Value::Matrix(a) if a.data() == [1.]));
        assert!(
            matches!(&args[0], Value::Matrix(a) if a.data().as_ptr() == numeric.data().as_ptr())
        );
    }
}
