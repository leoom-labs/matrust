//! `sortrows(T, vars, direction)`: a stable sort on one or more variables
//! (or the row names); missing values (NaN, NaT, missing strings) sort last
//! when ascending and first when descending, as MATLAB's 'auto' placement.
use super::super::{Engine, Execution, Value};
use super::{CANCEL_STRIDE, text_list, text_scalar, tvalue};
use crate::{Array, Matrix, Result};
use std::cmp::Ordering;

/// Comparable values of one key column; None is a missing value.
enum Key {
    Number(Vec<Option<f64>>),
    Integer(Vec<i128>),
    /// Exact datetime nanoseconds (an f64 would merge times ~8 us apart).
    Time(Vec<Option<i128>>),
    Text(Vec<Option<Vec<u16>>>),
}

impl Key {
    fn compare(&self, a: usize, b: usize) -> Ordering {
        fn missing_last<T>(
            x: &Option<T>,
            y: &Option<T>,
            f: impl Fn(&T, &T) -> Ordering,
        ) -> Ordering {
            match (x, y) {
                (Some(x), Some(y)) => f(x, y),
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
            }
        }
        match self {
            Key::Number(v) => missing_last(&v[a], &v[b], |x, y| x.total_cmp(y)),
            Key::Integer(v) => v[a].cmp(&v[b]),
            Key::Time(v) => missing_last(&v[a], &v[b], |x, y| x.cmp(y)),
            Key::Text(v) => missing_last(&v[a], &v[b], |x, y| x.cmp(y)),
        }
    }
}

/// Key columns of one variable (one per column of the variable).
fn keys_of(column: &Array, name: &str) -> Result<Vec<Key>> {
    let (rows, cols) = (column.rows(), column.cols());
    let per_column = |f: &dyn Fn(usize) -> Option<f64>| {
        (0..cols)
            .map(|c| Key::Number((0..rows).map(|r| f(c * rows + r)).collect()))
            .collect()
    };
    let number = |x: f64| (!x.is_nan()).then_some(x);
    Ok(match column {
        Array::Numeric(m) => per_column(&|i| number(m.data()[i])),
        Array::Logical(m) => per_column(&|i| Some(f64::from(m.data()[i]))),
        Array::Typed(t) if t.is_integer() => (0..cols)
            .map(|c| Key::Integer((0..rows).map(|r| t.int(c * rows + r)).collect()))
            .collect(),
        Array::Typed(t) => per_column(&|i| number(t.real(i))),
        Array::Duration(d) if d.is_calendar() => {
            return Err("sortrows: calendarDuration ordering is not supported".into());
        }
        Array::Duration(d) => per_column(&|i| number(d.data()[i])),
        Array::Datetime(t) => (0..cols)
            .map(|c| {
                Key::Time(
                    (0..rows)
                        .map(|r| t.data()[c * rows + r].total_nanos())
                        .collect(),
                )
            })
            .collect(),
        Array::Character(c) => vec![Key::Text(
            (0..rows)
                .map(|r| Some((0..cols).map(|k| c.units()[k * rows + r]).collect()))
                .collect(),
        )],
        Array::Str(s) => (0..cols)
            .map(|c| {
                Key::Text(
                    (0..rows)
                        .map(|r| {
                            s.data()[c * rows + r]
                                .as_deref()
                                .map(|t| t.encode_utf16().collect())
                        })
                        .collect(),
                )
            })
            .collect(),
        Array::Cell(cell) => {
            let mut keys = Vec::with_capacity(cols);
            for c in 0..cols {
                let mut texts = Vec::with_capacity(rows);
                for r in 0..rows {
                    match cell.get_linear(c * rows + r)?.as_ref() {
                        Array::Character(ch) if ch.rows() <= 1 => {
                            texts.push(Some(ch.units().to_vec()))
                        }
                        _ => {
                            return Err(format!(
                                "sortrows: variable '{name}' is a cell array that is not a cell array of character vectors"
                            ));
                        }
                    }
                }
                keys.push(Key::Text(texts));
            }
            keys
        }
        other => {
            return Err(format!(
                "sortrows: cannot sort on variable '{name}' of class {}",
                other.class()
            ));
        }
    })
}

impl Engine {
    pub(super) fn tbl_sortrows(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let t = self.tbl_one(args, "sortrows")?;
        let h = t.height();
        let rest = &args[1..];
        let direction_of = |v: &Value| -> Option<bool> {
            match text_scalar(v)?.to_ascii_lowercase().as_str() {
                "ascend" => Some(false),
                "descend" => Some(true),
                _ => None,
            }
        };
        // (variable index or None for row names, descending)
        let mut keys: Vec<(Option<usize>, bool)> = Vec::new();
        let mut at = 0;
        let row_dim = t.parts().dimension_names[0].clone();
        if let Some(v) = rest.first()
            && direction_of(v).is_none()
            && !text_scalar(v).is_some_and(|s| {
                s.eq_ignore_ascii_case("MissingPlacement")
                    || s.eq_ignore_ascii_case("ComparisonMethod")
            })
        {
            at = 1;
            if let Value::Matrix(m) = v {
                for &x in m.data() {
                    if !x.is_finite() || x == 0. || x.fract() != 0. || x.abs() > t.width() as f64 {
                        return Err("sortrows: variable numbers must be nonzero integers within the table width".into());
                    }
                    keys.push((Some(x.abs() as usize - 1), x < 0.));
                }
            } else {
                let names = text_list(self, v, "Variable names")?.ok_or(
                    "sortrows: give the variables as names, a cell array of names or numbers",
                )?;
                for n in names {
                    if n == row_dim || (n == "RowNames" && t.index_of("RowNames").is_none()) {
                        // Without row names this key leaves the order as it is.
                        if t.row_names().is_some() || t.is_timetable() {
                            keys.push((None, false));
                        }
                    } else {
                        keys.push((
                            Some(t.index_of(&n).ok_or_else(|| super::unknown_variable(&n))?),
                            false,
                        ));
                    }
                }
            }
        }
        if keys.is_empty() && at == 0 {
            keys = if t.is_timetable() {
                vec![(None, false)]
            } else {
                (0..t.width()).map(|k| (Some(k), false)).collect()
            };
        }
        if let Some(v) = rest.get(at) {
            let directions: Vec<bool> = if let Some(d) = direction_of(v) {
                vec![d; keys.len()]
            } else if let Some(list) = text_list(self, v, "Directions")?
                && !matches!(v, Value::Text(_))
            {
                list.iter()
                    .map(|s| match s.to_ascii_lowercase().as_str() {
                        "ascend" => Ok(false),
                        "descend" => Ok(true),
                        _ => Err("sortrows: directions must be 'ascend' or 'descend'".to_string()),
                    })
                    .collect::<Result<_>>()?
            } else {
                return Err("sortrows: options other than the sort direction are not supported for tables yet".into());
            };
            if directions.len() != keys.len() {
                return Err("sortrows: give one direction for each sorting variable".into());
            }
            for (key, d) in keys.iter_mut().zip(directions) {
                key.1 = d;
            }
            at += 1;
        }
        if at < rest.len() {
            return Err(
                "sortrows: options other than the sort direction are not supported for tables yet"
                    .into(),
            );
        }
        let mut columns: Vec<(Key, bool)> = Vec::new();
        for (var, descending) in keys {
            self.check_cancel()?;
            match var {
                Some(k) => {
                    for key in keys_of(t.column(k), &t.names()[k])? {
                        columns.push((key, descending));
                    }
                }
                None if t.is_timetable() => columns.push((
                    keys_of(t.row_times().unwrap(), &row_dim)?.remove(0),
                    descending,
                )),
                None => columns.push((
                    Key::Text(
                        t.row_names()
                            .unwrap_or(&[])
                            .iter()
                            .map(|s| Some(s.encode_utf16().collect()))
                            .collect(),
                    ),
                    descending,
                )),
            }
        }
        if h > CANCEL_STRIDE {
            self.check_cancel()?;
        }
        let mut order: Vec<usize> = (0..h).collect();
        order.sort_by(|&a, &b| {
            for (key, descending) in &columns {
                let o = key.compare(a, b);
                let o = if *descending { o.reverse() } else { o };
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });
        self.check_cancel()?;
        let sorted = self.tbl_select(&t, &order, None, out)?;
        let mut outputs = vec![tvalue(sorted)];
        if count >= 2 {
            outputs.push(Value::Matrix(Matrix::from_column_vec(
                h,
                1,
                order.iter().map(|&i| (i + 1) as f64).collect(),
            )?));
        }
        Ok(outputs)
    }
}
