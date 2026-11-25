//! `T(rows, vars)`, `T{rows, vars}`, `T.Var`, `T.Properties.*` reads and
//! the matching assignments and deletions. Subscripts are resolved to
//! zero-based row and variable lists here; the rows of each variable are then
//! read and written by the engine's own indexing for that variable's type.
use super::super::{Engine, Execution, Value};
use super::{
    CANCEL_STRIDE, cellstr, cellstr_row, is_table, row_expr, table_of, text_list, text_scalar,
    tvalue, unknown_variable,
};
use crate::syntax::{Expr, PathStep};
use crate::table_array::{TableArray, reject_handle, valid_name};
use crate::{Array, CellArray, Result, StructArray};
use std::sync::Arc;

const TWO_SUBSCRIPTS: &str = "Subscripting into a table using one subscript is not supported. Specify a row subscript and a variable subscript, as in T(rows, vars).";
const PROPERTIES: &[&str] = &[
    "Description",
    "DimensionNames",
    "VariableNames",
    "VariableDescriptions",
    "VariableUnits",
    "RowNames",
];

impl Engine {
    /// Zero-based rows selected by `arg` (`end` is the height). `grow`
    /// admits numeric rows past the end (assignment).
    fn tbl_rows(
        &mut self,
        t: &TableArray,
        arg: &Expr,
        grow: bool,
        out: &mut Execution,
    ) -> Result<Vec<usize>> {
        let h = t.height();
        if self.colon_subscript(arg) {
            return Ok((0..h).collect());
        }
        let value = self.eval(arg, Some(h), out)?;
        if value.is_colon_character() {
            return Ok((0..h).collect());
        }
        if !matches!(value, Value::Matrix(_) | Value::Logical(_))
            && let Some(names) = text_list(self, &value, "Row names")?
        {
            let Some(row_names) = t.row_names() else {
                return Err(
                    "This table has no row names; select rows by number or logical mask.".into(),
                );
            };
            return names
                .iter()
                .map(|n| {
                    row_names
                        .iter()
                        .position(|r| r == n)
                        .ok_or_else(|| format!("Unrecognized row name '{n}'."))
                })
                .collect();
        }
        if let Value::Logical(mask) = &value {
            if mask.len() > h {
                return Err("Row index exceeds table dimensions.".into());
            }
            return Ok(mask
                .data()
                .iter()
                .enumerate()
                .filter(|(_, b)| **b != 0)
                .map(|(i, _)| i)
                .collect());
        }
        let m = value.matrix(&self.cancel)?;
        let mut rows = Vec::with_capacity(m.len());
        for (i, &x) in m.data().iter().enumerate() {
            if i % CANCEL_STRIDE == 0 {
                self.check_cancel()?;
            }
            if !x.is_finite() || x < 1. || x.fract() != 0. {
                return Err(
                    "Table row subscripts must be positive integers, logical values or row names."
                        .into(),
                );
            }
            if x > h as f64 && (!grow || x > crate::matrix::MAX_ELEMENTS as f64) {
                return Err("Row index exceeds table dimensions.".into());
            }
            rows.push(x as usize - 1);
        }
        Ok(rows)
    }

    /// Zero-based variables selected by `arg` (`end` is the width).
    fn tbl_vars(&mut self, t: &TableArray, arg: &Expr, out: &mut Execution) -> Result<Vec<usize>> {
        let w = t.width();
        if self.colon_subscript(arg) {
            return Ok((0..w).collect());
        }
        let value = self.eval(arg, Some(w), out)?;
        if value.is_colon_character() {
            return Ok((0..w).collect());
        }
        self.tbl_vars_of(t, &value)
    }

    /// Variables named, numbered or masked by an evaluated selector.
    pub(super) fn tbl_vars_of(&self, t: &TableArray, value: &Value) -> Result<Vec<usize>> {
        let w = t.width();
        if !matches!(value, Value::Matrix(_) | Value::Logical(_))
            && let Some(names) = text_list(self, value, "Variable names")?
        {
            return names
                .iter()
                .map(|n| t.index_of(n).ok_or_else(|| unknown_variable(n)))
                .collect();
        }
        if let Value::Logical(mask) = value {
            if mask.len() > w {
                return Err("Variable index exceeds table dimensions.".into());
            }
            return Ok(mask
                .data()
                .iter()
                .enumerate()
                .filter(|(_, b)| **b != 0)
                .map(|(i, _)| i)
                .collect());
        }
        let m = value.clone().matrix(&self.cancel)?;
        m.data()
            .iter()
            .map(|&x| {
                if !x.is_finite() || x < 1. || x.fract() != 0. {
                    Err("Table variable subscripts must be positive integers, logical values or names.".into())
                } else if x > w as f64 {
                    Err("Variable index exceeds table dimensions.".into())
                } else {
                    Ok(x as usize - 1)
                }
            })
            .collect()
    }

    /// `T(rows, vars)` (a table) or `T{rows, vars}` (the pieces joined).
    pub(in crate::engine) fn tbl_index(
        &mut self,
        t: &TableArray,
        args: &[Expr],
        brace: bool,
        out: &mut Execution,
    ) -> Result<Value> {
        let [row_arg, var_arg] = args else {
            return Err(TWO_SUBSCRIPTS.into());
        };
        let rows = self.tbl_rows(t, row_arg, false, out)?;
        let vars = self.tbl_vars(t, var_arg, out)?;
        if brace {
            let mut pieces = Vec::with_capacity(vars.len());
            for &v in &vars {
                pieces.push(self.tbl_take_rows(t.column(v), &rows, out)?);
            }
            return self.tbl_join_pieces(t, &vars, pieces, rows.len(), out);
        }
        Ok(tvalue(self.tbl_select(t, &rows, Some(&vars), out)?))
    }

    /// `T.Properties` as a struct.
    fn tbl_properties(&self, t: &TableArray) -> Result<Array> {
        let p = t.parts();
        let cancel = &self.cancel;
        let list = |v: &Vec<String>| -> Result<Array> {
            if v.is_empty() {
                Ok(CellArray::empty(0, 0, cancel)?.into())
            } else {
                cellstr_row(v, cancel)
            }
        };
        let mut slots = vec![
            Arc::new(Array::from(crate::CharacterRow::from_text(
                &p.description,
                cancel,
            )?)),
            Arc::new(cellstr_row(&p.dimension_names, cancel)?),
            Arc::new(cellstr_row(&p.names, cancel)?),
            Arc::new(list(&p.descriptions)?),
            Arc::new(list(&p.units)?),
            Arc::new(match &p.row_names {
                Some(r) => cellstr(r.len(), 1, r, cancel)?,
                None => CellArray::empty(0, 0, cancel)?.into(),
            }),
        ];
        let mut properties: Vec<String> = PROPERTIES.iter().map(|s| (*s).to_owned()).collect();
        if let Some(times) = t.row_times() {
            properties.push("RowTimes".into());
            slots.push(Arc::new(times.clone()));
        }
        Ok(StructArray::from_slots(1, 1, properties, slots, cancel)?.into())
    }

    /// `T.name`: a variable, `Properties`, or a dimension name (row names,
    /// or all variables joined).
    pub(in crate::engine) fn tbl_field(
        &mut self,
        t: &TableArray,
        name: &str,
        out: &mut Execution,
    ) -> Result<Value> {
        if let Some(k) = t.index_of(name) {
            return Ok(Value::from(t.column(k).clone()));
        }
        if name == "Properties" {
            return Ok(Value::from(self.tbl_properties(t)?));
        }
        let dims = &t.parts().dimension_names;
        if name == dims[0] {
            if let Some(times) = t.row_times() {
                return Ok(Value::from(times.clone()));
            }
            return Ok(Value::from(match t.row_names() {
                Some(r) => cellstr(r.len(), 1, r, &self.cancel)?,
                None => CellArray::empty(0, 0, &self.cancel)?.into(),
            }));
        }
        if name == dims[1] {
            let all: Vec<usize> = (0..t.width()).collect();
            return self.tbl_join_pieces(t, &all, t.columns().to_vec(), t.height(), out);
        }
        Err(unknown_variable(name))
    }

    /// Properties from an (edited) `T.Properties` struct.
    fn tbl_set_properties(&self, t: &TableArray, s: &Array) -> Result<TableArray> {
        let Array::Struct(s) = s else {
            return Err("T.Properties must be assigned a struct of table properties".into());
        };
        if s.len() != 1 {
            return Err("T.Properties must be a scalar struct".into());
        }
        let mut parts = t.to_parts();
        for (f, name) in s.field_names().iter().enumerate() {
            let value = Value::from(s.get(f, 0)?.as_ref().clone());
            let list = |what: &str| -> Result<Vec<String>> {
                text_list(self, &value, what)?.ok_or_else(|| {
                    format!("The {what} property must be a cell array of character vectors or a string array.")
                })
            };
            match name.as_str() {
                "Description" => {
                    parts.description = text_scalar(&value)
                        .or_else(|| value.dims().contains(&0).then(String::new))
                        .ok_or(
                            "The Description property must be a character vector or string scalar.",
                        )?;
                }
                "VariableNames" => {
                    let names = list("VariableNames")?;
                    if names.len() != parts.names.len() {
                        return Err("The VariableNames property must contain one name for each variable in the table.".into());
                    }
                    parts.names = names;
                }
                "RowTimes" if t.is_timetable() => {
                    parts.row_times = Some(value.clone().array(&self.cancel)?);
                }
                "RowNames" => {
                    let names = list("RowNames")?;
                    parts.row_names = if names.is_empty() || names == [""] {
                        None
                    } else {
                        Some(names)
                    };
                }
                "DimensionNames" => {
                    let names = list("DimensionNames")?;
                    let [a, b] = names.as_slice() else {
                        return Err("The DimensionNames property must contain two names.".into());
                    };
                    for d in [a, b] {
                        if !valid_name(d) || parts.names.contains(d) {
                            return Err(format!("Invalid table dimension name: '{d}'."));
                        }
                    }
                    parts.dimension_names = [a.clone(), b.clone()];
                }
                "VariableUnits" | "VariableDescriptions" => {
                    let entries = list(name)?;
                    let entries = if entries.iter().all(String::is_empty)
                        && entries.len() != parts.names.len()
                    {
                        Vec::new()
                    } else {
                        entries
                    };
                    if !entries.is_empty() && entries.len() != parts.names.len() {
                        return Err(format!(
                            "The {name} property must have one entry per table variable."
                        ));
                    }
                    if name == "VariableUnits" {
                        parts.units = entries;
                    } else {
                        parts.descriptions = entries;
                    }
                }
                other => return Err(format!("Unrecognized table property '{other}'.")),
            }
        }
        TableArray::new(parts)
    }

    /// Path assignment rooted at a table: `T.Var = v`, `T.Var(i) = v`,
    /// `T.Var = []`, `T.Properties.X = v`; `T(...)`/`T{...}` steps of a
    /// nested path (`s.T(1, 2) = v`). None leaves other paths to the
    /// generic code.
    pub(in crate::engine) fn tbl_assign_path(
        &mut self,
        current: &TableArray,
        steps: &[PathStep],
        value: &Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Option<Array>> {
        let Some((step, rest)) = steps.split_first() else {
            return Ok(None);
        };
        let name = match step {
            PathStep::Field(name) => name.clone(),
            PathStep::Dynamic(expr) => self.dynamic_field_name(expr, out)?,
            PathStep::Paren(args) | PathStep::Brace(args) => {
                if !rest.is_empty() {
                    return Err("table: assignment through T(...) or T{...} into a nested value is not supported yet".into());
                }
                let brace = matches!(step, PathStep::Brace(_));
                let updated = self.tbl_assign_sub(
                    current,
                    args,
                    brace,
                    Value::from(value.clone()),
                    deletion,
                    out,
                )?;
                return Ok(Some(Array::Table(updated)));
            }
        };
        if name == "Properties" {
            let props = self.tbl_properties(current)?;
            let edited = self.assign_step(Some(props), rest, value.clone(), deletion, out)?;
            return Ok(Some(Array::Table(
                self.tbl_set_properties(current, &edited)?,
            )));
        }
        if current.is_timetable() && name == current.parts().dimension_names[0] {
            if !rest.is_empty() || deletion {
                return Err("timetable: indexed RowTimes assignment is not supported".into());
            }
            let mut parts = current.to_parts();
            parts.row_times = Some(value.clone());
            return Ok(Some(Array::Table(TableArray::new(parts)?)));
        }
        let existing = current.index_of(&name);
        let mut parts = current.to_parts();
        if rest.is_empty() && deletion {
            let k = existing.ok_or_else(|| unknown_variable(&name))?;
            parts.remove_variables(&[k]);
            return Ok(Some(Array::Table(TableArray::new(parts)?)));
        }
        let column = if rest.is_empty() {
            value.clone()
        } else if let [PathStep::Paren(args)] = rest
            && let Some(k) = existing
            && matches!(
                current.column(k),
                Array::Categorical(_) | Array::Datetime(_) | Array::Duration(_)
            )
        {
            // `T.c(i) = v` on a categorical/datetime/duration variable: the
            // indexed assignment of those types, on a temporary variable.
            let temp = (0usize..)
                .map(|i| format!("Zq_tblcol_{i}"))
                .find(|s| !self.contains_array(s))
                .unwrap();
            self.container_variables
                .insert(temp.clone(), current.column(k).clone());
            let result = self.assign_indexed_value(
                &temp,
                args,
                Value::from(value.clone()),
                deletion,
                true,
                out,
            );
            let updated = self.container_variables.remove(&temp);
            self.variables.remove(&temp);
            self.logical_variables.remove(&temp);
            self.complex_variables.remove(&temp);
            result?;
            updated.ok_or("table: the variable assignment did not produce a result")?
        } else {
            let k = existing.ok_or_else(|| unknown_variable(&name))?;
            self.assign_step(
                Some(current.column(k).clone()),
                rest,
                value.clone(),
                deletion,
                out,
            )?
        };
        if is_table(&Value::from(column.clone())) {
            return Err("table: nested tables are not supported yet".into());
        }
        reject_handle(&column)?;
        if column.rows() != parts.height {
            let empty_table = parts.height == 0 && parts.names.is_empty();
            if !(empty_table && rest.is_empty()) {
                return Err(if rest.is_empty() {
                    "To assign to or create a variable in a table, the number of rows must match the height of the table.".into()
                } else {
                    "table: assigning past the end of a variable is not supported yet; add rows with T(end+1, :) = {...}".into()
                });
            }
            parts.height = column.rows();
        }
        match existing {
            Some(k) => parts.columns[k] = column,
            None => parts.push_variable(name, column),
        }
        Ok(Some(Array::Table(TableArray::new(parts)?)))
    }

    /// Statement `T(args) = rhs` / `T{args} = rhs` for a table variable `name`.
    pub(in crate::engine) fn tbl_assign_statement(
        &mut self,
        name: &str,
        args: &[Expr],
        brace: bool,
        rhs: &Expr,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        let Some(Array::Table(t)) = self.array(name) else {
            return Err("Expected a table variable".into());
        };
        let deletion = matches!(rhs, Expr::Matrix(rows) if rows.is_empty());
        let value = self.eval(rhs, None, out)?;
        let updated = self.tbl_assign_sub(&t, args, brace, value, deletion, out)?;
        self.assign_workspace_array(name, Array::Table(updated), quiet, out)
    }

    /// `T(rows, vars) = value`, `T{rows, vars} = value` and the deletions
    /// `T(rows, :) = []`, `T(:, vars) = []`.
    pub(in crate::engine) fn tbl_assign_sub(
        &mut self,
        t: &TableArray,
        args: &[Expr],
        brace: bool,
        value: Value,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<TableArray> {
        let [row_arg, var_arg] = args else {
            return Err(TWO_SUBSCRIPTS.into());
        };
        let all_rows = self.colon_subscript(row_arg);
        let all_vars = self.colon_subscript(var_arg);
        let rows = self.tbl_rows(t, row_arg, !deletion, out)?;
        let vars = self.tbl_vars(t, var_arg, out)?;
        if deletion && !brace {
            let every_row = all_rows || rows.len() == t.height() && is_permutation(&rows);
            let every_var = all_vars || vars.len() == t.width() && is_permutation(&vars);
            if every_var {
                let keep: Vec<usize> = (0..t.height()).filter(|r| !rows.contains(r)).collect();
                return self.tbl_select(t, &keep, None, out);
            }
            if every_row {
                let mut parts = t.to_parts();
                parts.remove_variables(&vars);
                return TableArray::new(parts);
            }
            return Err(
                "To delete rows or variables of a table, use T(rows, :) = [] or T(:, vars) = []."
                    .into(),
            );
        }
        let (h, n) = (rows.len(), vars.len());
        let mut pieces: Vec<Option<Array>> = Vec::with_capacity(n);
        if brace {
            let value = value.array(&self.cancel)?;
            let widths: Vec<usize> = vars.iter().map(|&v| t.column(v).cols()).collect();
            let total: usize = widths.iter().sum();
            let scalar = value.rows() == 1 && value.cols() == 1;
            if !scalar && n > 1 && value.cols() != total {
                return Err("The right side of a brace assignment into a table must have one column for each selected table column.".into());
            }
            let mut first = 0;
            for (k, &v) in vars.iter().enumerate() {
                let piece = if scalar || n == 1 {
                    value.clone()
                } else {
                    self.tbl_take_columns(&value, first, first + widths[k] - 1, out)?
                };
                first += widths[k];
                // A cell variable takes a non-cell value as one cell's content.
                let piece =
                    if matches!(t.column(v), Array::Cell(_)) && !matches!(piece, Array::Cell(_)) {
                        CellArray::from_shared(1, 1, vec![Arc::new(piece)], &self.cancel)?.into()
                    } else {
                        piece
                    };
                pieces.push(Some(piece));
            }
        } else {
            match &value {
                Value::Container(Array::Table(src)) => {
                    if src.height() != h || src.width() != n {
                        return Err(format!(
                            "The right side table is {}x{} but the assignment selects {h}x{n}.",
                            src.height(),
                            src.width()
                        ));
                    }
                    // Columns by name: the right side holds the selected
                    // variables, in any order (as `[T1; T2]` reorders).
                    for name in src.names() {
                        if !vars.iter().any(|&v| &t.names()[v] == name) {
                            return Err(format!(
                                "The right side table's variable '{name}' is not one of the variables being assigned to."
                            ));
                        }
                    }
                    for &v in &vars {
                        let name = &t.names()[v];
                        let k = src.index_of(name).ok_or_else(|| {
                            format!("The right side table has no variable named '{name}'.")
                        })?;
                        pieces.push(Some(src.column(k).clone()));
                    }
                }
                Value::Container(Array::Cell(c)) => {
                    let c = c.clone();
                    if c.rows() != h || c.cols() != n {
                        return Err(format!(
                            "The right side cell array is {}x{} but the assignment selects {h}x{n}.",
                            c.rows(),
                            c.cols()
                        ));
                    }
                    for (k, &v) in vars.iter().enumerate() {
                        let contents = (0..h)
                            .map(|r| c.get_linear(k * h + r).cloned())
                            .collect::<Result<Vec<_>>>()?;
                        let piece = if matches!(t.column(v), Array::Cell(_)) {
                            CellArray::from_shared(h, 1, contents, &self.cancel)?.into()
                        } else {
                            let rows = contents
                                .iter()
                                .map(|a| vec![Value::from(a.as_ref().clone())])
                                .collect();
                            self.tbl_join(rows, out)?.array(&self.cancel)?
                        };
                        pieces.push(Some(piece));
                    }
                }
                _ => {
                    return Err("The right side of an assignment into a table must be another table or a cell array.".into());
                }
            }
        }
        let grows = rows.iter().any(|&r| r >= t.height());
        if grows && n != t.width() {
            return Err(
                "To add rows to a table, assign every variable: T(end+1, :) = {...}.".into(),
            );
        }
        let mut parts = t.to_parts();
        for (k, &v) in vars.iter().enumerate() {
            let column = parts.columns[v].clone();
            parts.columns[v] = self.tbl_write(
                column,
                vec![row_expr(&rows), Expr::All],
                pieces[k].take(),
                out,
            )?;
        }
        if grows {
            let new_height = rows.iter().max().map_or(t.height(), |m| m + 1);
            parts.height = new_height;
            if let Some(names) = &mut parts.row_names {
                for r in names.len()..new_height {
                    let mut candidate = format!("Row{}", r + 1);
                    while names.contains(&candidate) {
                        candidate.push('_');
                    }
                    names.push(candidate);
                }
            }
        }
        TableArray::new(parts)
    }
}

fn is_permutation(list: &[usize]) -> bool {
    let mut seen = vec![false; list.len()];
    list.iter()
        .all(|&i| i < seen.len() && !std::mem::replace(&mut seen[i], true))
}

/// True for a 0x0 table value (concatenation skips it).
pub(super) fn is_empty_table(v: &Value) -> bool {
    table_of(v).is_some_and(|t| t.height() == 0 && t.width() == 0)
}

/// Parts check used by concatenation for the variable-property lists.
pub(super) fn padded(list: &[String], width: usize) -> Vec<String> {
    if list.is_empty() {
        vec![String::new(); width]
    } else {
        list.to_vec()
    }
}
