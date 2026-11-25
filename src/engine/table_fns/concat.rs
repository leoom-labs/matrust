//! `[T1, T2]` and `[T1; T2]` (and horzcat/vertcat/cat). Each variable's
//! columns join through the ordinary concatenation of their type.
use super::super::{Engine, Execution, Value};
use super::index::{is_empty_table, padded};
use super::{table_of, tvalue};
use crate::table_array::TableArray;
use crate::{Array, Result};
use std::collections::HashMap;

/// The row of `theirs` for each name in `have` (a permutation when both
/// lists hold the same names); Err names the first name of `have` that
/// `theirs` lacks. Equal heights and unique names make a full match a
/// permutation.
fn row_order(have: &[String], theirs: &[String]) -> std::result::Result<Vec<usize>, String> {
    let at: HashMap<&str, usize> = theirs
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    have.iter()
        .map(|n| at.get(n.as_str()).copied().ok_or_else(|| n.clone()))
        .collect()
}

impl Engine {
    /// A `[...]` literal (rows of values) holding at least one table.
    pub(in crate::engine) fn tbl_concat(
        &mut self,
        rows: Vec<Vec<Value>>,
        out: &mut Execution,
    ) -> Result<Value> {
        let mut joined = Vec::with_capacity(rows.len());
        let mut empty = None;
        for row in rows {
            let mut tables = Vec::with_capacity(row.len());
            for v in row {
                if let Some(t) = table_of(&v) {
                    if !is_empty_table(&v) {
                        tables.push(t.clone());
                    } else if empty
                        .as_ref()
                        .is_none_or(|x: &TableArray| !x.is_timetable())
                    {
                        // Keep the empty timetable's type and RowTimes metadata
                        // even when every operand is a neutral empty table.
                        empty = Some(t.clone());
                    }
                } else if v.dims().contains(&0) && !matches!(v, Value::Void) {
                    // `[T; []]`: empty non-table values are dropped.
                } else {
                    return Err("Tables can only be concatenated with other tables.".into());
                }
            }
            if !tables.is_empty() {
                joined.push(self.tbl_hcat(tables, out)?);
            }
        }
        if joined.is_empty() {
            return Ok(tvalue(empty.unwrap_or_else(TableArray::empty)));
        }
        Ok(tvalue(self.tbl_vcat(joined, out)?))
    }

    fn tbl_hcat(&mut self, tables: Vec<TableArray>, out: &mut Execution) -> Result<TableArray> {
        if tables.len() == 1 {
            return Ok(tables.into_iter().next().expect("one table"));
        }
        let mut parts = tables[0].to_parts();
        let any_units = tables.iter().any(|t| !t.parts().units.is_empty());
        let any_desc = tables.iter().any(|t| !t.parts().descriptions.is_empty());
        if any_units {
            parts.units = padded(&parts.units, parts.names.len());
        }
        if any_desc {
            parts.descriptions = padded(&parts.descriptions, parts.names.len());
        }
        for (k, t) in tables.iter().enumerate().skip(1) {
            match (&parts.row_times,t.row_times()) {
                (Some(a),Some(b)) if super::super::datetime_fns::arrays_equal(a,b,false,&self.cancel)?==Some(true)=>{},
                (None,None)=>{},
                _=>return Err("timetable: horizontal concatenation requires identical row times and time types".into()),
            }
            if t.height() != parts.height {
                return Err(
                    "All tables being horizontally concatenated must have the same number of rows."
                        .into(),
                );
            }
            for name in t.names() {
                if parts.names.contains(name) {
                    return Err(format!("Duplicate table variable name: '{name}'."));
                }
            }
            // Row names on both sides must hold the same set; the later
            // table's rows follow the order already in `parts`. One side
            // without row names keeps the other's.
            let aligned;
            let t = match (&parts.row_names, t.row_names()) {
                (Some(have), Some(theirs)) => match row_order(have, theirs) {
                    Ok(order) if order.iter().enumerate().all(|(i, &r)| i == r) => t,
                    Ok(order) => {
                        aligned = self.tbl_select(t, &order, None, out)?;
                        &aligned
                    }
                    Err(missing) => {
                        return Err(format!(
                            "The row names of the tables being horizontally concatenated must be the same; '{missing}' is not a row name of table {} in the list.",
                            k + 1
                        ));
                    }
                },
                (None, Some(theirs)) => {
                    parts.row_names = Some(theirs.to_vec());
                    t
                }
                _ => t,
            };
            parts.names.extend(t.names().iter().cloned());
            parts.columns.extend(t.columns().iter().cloned());
            if any_units {
                parts.units.extend(padded(&t.parts().units, t.width()));
            }
            if any_desc {
                parts
                    .descriptions
                    .extend(padded(&t.parts().descriptions, t.width()));
            }
        }
        TableArray::new(parts)
    }

    fn tbl_vcat(&mut self, tables: Vec<TableArray>, out: &mut Execution) -> Result<TableArray> {
        if tables.len() == 1 {
            return Ok(tables.into_iter().next().expect("one table"));
        }
        let first = &tables[0];
        if tables
            .iter()
            .any(|t| t.is_timetable() != first.is_timetable())
        {
            return Err("timetable: concatenation with an ordinary table is not supported".into());
        }
        let with_names = tables.iter().filter(|t| t.row_names().is_some()).count();
        if with_names != 0 && with_names != tables.len() {
            return Err("Cannot vertically concatenate tables with and without row names.".into());
        }
        // Each later table's variables in the first table's order.
        let mut orders = Vec::with_capacity(tables.len());
        for t in &tables {
            if t.width() != first.width() {
                return Err("All tables being vertically concatenated must have the same number of variables.".into());
            }
            let order = first
                .names()
                .iter()
                .map(|n| t.index_of(n))
                .collect::<Option<Vec<usize>>>()
                .ok_or(
                    "All tables being vertically concatenated must have the same variable names.",
                )?;
            orders.push(order);
        }
        let mut parts = first.to_parts();
        for k in 0..first.width() {
            self.check_cancel()?;
            let rows: Vec<Vec<Value>> = tables
                .iter()
                .zip(&orders)
                .map(|(t, order)| vec![Value::from(t.column(order[k]).clone())])
                .collect();
            parts.columns[k] = self.tbl_join(rows, out)?.array(&self.cancel)?;
            if matches!(parts.columns[k], Array::Table(_)) {
                return Err("table: nested tables are not supported yet".into());
            }
        }
        parts.height = tables.iter().map(TableArray::height).sum();
        if first.is_timetable() {
            let rows = tables
                .iter()
                .map(|t| vec![Value::from(t.row_times().unwrap().clone())])
                .collect();
            parts.row_times = Some(self.tbl_join(rows, out)?.array(&self.cancel)?);
        }
        if with_names != 0 {
            parts.row_names = Some(
                tables
                    .iter()
                    .flat_map(|t| t.row_names().unwrap_or(&[]).iter().cloned())
                    .collect(),
            );
        }
        TableArray::new(parts)
    }
}
