//! Variable (column) operations: addvars, removevars, renamevars, movevars.
use super::super::{Engine, Value};
use super::{is_table, split_options, table_of, text_list, tvalue};
use crate::Result;
use crate::table_array::{TableArray, TableParts, valid_name};

/// Where new or moved variables go: before this index of the remaining
/// variables (`len` = at the end).
fn insert_at(
    engine: &Engine,
    t: &TableArray,
    who: &str,
    options: &[(String, &Value)],
) -> Result<Option<(usize, bool)>> {
    let mut place = None;
    for (option, value) in options {
        let before = option == "Before";
        if !before && option != "After" {
            continue;
        }
        if place.is_some() {
            return Err(format!("{who}: specify only one of 'Before' and 'After'"));
        }
        let vars = engine.tbl_vars_of(t, value)?;
        let [v] = vars[..] else {
            return Err(format!(
                "{who}: the location must name exactly one table variable"
            ));
        };
        place = Some((v, before));
    }
    Ok(place)
}

/// The parts of `t` with its variables reordered to `order`.
fn reorder(t: &TableParts, order: &[usize]) -> TableParts {
    let mut parts = t.select_variables(order);
    parts.height = t.height;
    parts
}

impl Engine {
    pub(super) fn tbl_addvars(&mut self, args: &[Value], names: &[String]) -> Result<Value> {
        let (data, options) = split_options(args, &["Before", "After", "NewVariableNames"]);
        let t = self.tbl_one(data, "addvars")?;
        let added = &data[1..];
        let mut new_names: Vec<String> = Vec::with_capacity(added.len());
        let mut columns = Vec::with_capacity(added.len());
        for (k, v) in added.iter().enumerate() {
            if matches!(v, Value::Void) {
                return Err("addvars: an input has no value".into());
            }
            if let Some(inner) = table_of(v) {
                // A table argument adds each of its variables.
                for (n, c) in inner.names().iter().zip(inner.columns()) {
                    new_names.push(n.clone());
                    columns.push(c.clone());
                }
                continue;
            }
            new_names.push(match names.get(k + 1) {
                Some(n) if valid_name(n) => n.clone(),
                _ => format!("Var{}", t.width() + columns.len() + 1),
            });
            let column = v.clone().array(&self.cancel)?;
            columns.push(column);
        }
        if let Some((_, value)) = options.iter().find(|(o, _)| o == "NewVariableNames") {
            let list = text_list(self, value, "NewVariableNames")?
                .ok_or("addvars: NewVariableNames must be text")?;
            if list.len() != columns.len() {
                return Err(
                    "addvars: NewVariableNames must have one name for each new variable".into(),
                );
            }
            new_names = list;
        }
        let place = insert_at(self, &t, "addvars", &options)?;
        let mut parts = t.to_parts();
        if t.width() == 0 && t.height() == 0 {
            parts.height = columns.first().map_or(0, crate::Array::rows);
        }
        let old = parts.names.len();
        for (n, c) in new_names.into_iter().zip(columns) {
            parts.push_variable(n, c);
        }
        let at = match place {
            None => old,
            Some((v, true)) => v,
            Some((v, false)) => v + 1,
        };
        let mut order: Vec<usize> = (0..at).collect();
        order.extend(old..parts.names.len());
        order.extend(at..old);
        Ok(tvalue(TableArray::new(reorder(&parts, &order))?))
    }

    pub(super) fn tbl_removevars(&mut self, args: &[Value]) -> Result<Value> {
        let [tv, vars] = args else {
            return Err("removevars: expected a table and the variables to remove".into());
        };
        if !is_table(tv) {
            return Err("removevars: the first input must be a table".into());
        }
        let t = self.tbl_one(args, "removevars")?;
        let drop = self.tbl_vars_of(&t, vars)?;
        let mut parts = t.to_parts();
        parts.remove_variables(&drop);
        Ok(tvalue(TableArray::new(parts)?))
    }

    pub(super) fn tbl_renamevars(&mut self, args: &[Value]) -> Result<Value> {
        let [_, vars, new] = args else {
            return Err("renamevars: expected a table, the variables and their new names".into());
        };
        let t = self.tbl_one(args, "renamevars")?;
        let which = self.tbl_vars_of(&t, vars)?;
        let list =
            text_list(self, new, "New names")?.ok_or("renamevars: the new names must be text")?;
        if list.len() != which.len() {
            return Err("renamevars: give one new name for each variable".into());
        }
        let mut parts = t.to_parts();
        for (&v, n) in which.iter().zip(list) {
            parts.names[v] = n;
        }
        Ok(tvalue(TableArray::new(parts)?))
    }

    pub(super) fn tbl_movevars(&mut self, args: &[Value]) -> Result<Value> {
        let (data, options) = split_options(args, &["Before", "After"]);
        let [_, vars] = data else {
            return Err("movevars: expected a table and the variables to move".into());
        };
        let t = self.tbl_one(data, "movevars")?;
        let moved = self.tbl_vars_of(&t, vars)?;
        let place = insert_at(self, &t, "movevars", &options)?;
        let mut order: Vec<usize> = Vec::with_capacity(t.width());
        let rest: Vec<usize> = (0..t.width()).filter(|i| !moved.contains(i)).collect();
        match place {
            None => {
                order.extend(&rest);
                order.extend(&moved);
            }
            Some((anchor, before)) => {
                if moved.contains(&anchor) {
                    return Err(
                        "movevars: the location cannot be one of the moved variables".into(),
                    );
                }
                for &r in &rest {
                    if r == anchor && before {
                        order.extend(&moved);
                    }
                    order.push(r);
                    if r == anchor && !before {
                        order.extend(&moved);
                    }
                }
            }
        }
        Ok(tvalue(TableArray::new(reorder(t.parts(), &order))?))
    }
}
