//! Struct values: field reads, nested assignment, element growth and builtins.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Result, StructArray,
    containers::MAX_DEPTH,
    script_containers::MAX_CELL_SLOTS,
    struct_array::valid_field_name,
    syntax::{Expr, PathStep},
};
use std::sync::Arc;

const NO_FIELDS: &str = "Attempt to reference field of non-structure array";

/// Where a one- or two-subscript struct assignment lands.
struct Position {
    index: usize,
    shape: (usize, usize),
}

impl Engine {
    /// Text for an unsuppressed `name = value` display.
    pub(super) fn display_text(&self, name: &str, array: &Array) -> Result<String> {
        if let Some(text) = self.lng_object_text(Some(name), array)? {
            return Ok(text);
        }
        if let Some(text) = self.obj_display(name, array)? {
            return Ok(text);
        }
        crate::display_format::echo(name, array, &self.display)
    }

    fn text_name(&self, value: Value, what: &str) -> Result<String> {
        match value {
            Value::Text(s) => Ok(s),
            Value::Container(Array::Character(c)) => c.to_utf8(&self.cancel),
            _ => Err(format!("{what} must be character text")),
        }
    }

    fn char_array(&self, text: &str) -> Result<Array> {
        Ok(Array::Character(CharacterRow::from_text(
            text,
            &self.cancel,
        )?))
    }

    /// Value of an assignment right-hand side as stored data.
    pub(super) fn assigned_array(&self, value: Value) -> Result<Array> {
        match value {
            Value::Void => Err("This command does not return a value".into()),
            Value::QuotedString(_) => Err(
                "String array storage is not implemented; use single-quoted character text".into(),
            ),
            value => value.array(&self.cancel),
        }
    }

    fn dynamic_field_name(&mut self, expr: &Expr, out: &mut Execution) -> Result<String> {
        let value = self.eval(expr, None, out)?;
        self.text_name(value, "Dynamic structure field names")
    }

    /// Every value of `base.name` / `base.(expr)`: one per struct element.
    pub(super) fn field_values(
        &mut self,
        expr: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if let Some(value) = self.obj_eval(expr, out)? {
            return Ok(vec![value]);
        }
        let (base, name) = match expr {
            Expr::Field(base, name) => (base, Some(name.clone())),
            Expr::DynamicField(base, _) => (base, None),
            _ => unreachable!("field_values called for a field expression"),
        };
        let base = self.eval(base, end, out)?;
        let name = match (name, expr) {
            (Some(name), _) => name,
            (None, Expr::DynamicField(_, name)) => self.dynamic_field_name(name, out)?,
            _ => unreachable!(),
        };
        self.obj_field_values_resolved(base, &name, out)
    }
    fn obj_field_values_resolved(
        &mut self,
        base: Value,
        name: &str,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if let Some(value) = self.obj_field(&base, name, out)? {
            return Ok(vec![value]);
        }
        let Value::Container(Array::Struct(s)) = base else {
            return Err(NO_FIELDS.into());
        };
        let Some(field) = s.field_index(name) else {
            return Err(self.raise(
                format!("Reference to non-existent field '{name}'."),
                "MATLAB:nonExistentField".into(),
            ));
        };
        let mut values = Vec::new();
        values
            .try_reserve_exact(s.len())
            .map_err(|_| "Cannot allocate field list")?;
        let mut units = 0usize;
        for element in 0..s.len() {
            if element.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let value = Value::from(s.get(field, element)?.as_ref().clone());
            units = units.saturating_add(value.allocation_units().unwrap_or(0));
            values.push(value);
        }
        if values.len() != 1 {
            self.charge(units)?;
        }
        Ok(values)
    }

    pub(super) fn field_value(
        &mut self,
        expr: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Value> {
        let mut values = self.field_values(expr, end, out)?;
        if values.len() != 1 {
            return Err(format!(
                "Expected one output from a curly brace or dot indexing expression, but there were {} results",
                values.len()
            ));
        }
        Ok(values.remove(0))
    }

    /// Resolve a method receiver once, keeping its argument list intact.
    pub(super) fn obj_index_expression(
        &mut self,
        expr: &Expr,
        args: &[Expr],
        end: Option<usize>,
        out: &mut Execution,
        statement: bool,
    ) -> Result<Value> {
        let base = if let Expr::Field(inner, member) = expr {
            let receiver = self.eval(inner, end, out)?;
            if let Some(value) = self.obj_member_value(&receiver, member, args, out)? {
                return Ok(value);
            }
            let mut values = self.obj_field_values_resolved(receiver, member, out)?;
            if values.len() != 1 {
                return Err(format!(
                    "Expected one output from a curly brace or dot indexing expression, but there were {} results",
                    values.len()
                ));
            }
            values.remove(0)
        } else {
            self.eval(expr, end, out)?
        };
        if statement && let Value::Container(Array::Handle(handle)) = &base {
            let values = self.function_arguments(args, None, out)?;
            return Ok(self
                .call_handle(handle, values, 0, None, out)?
                .into_iter()
                .next()
                .unwrap_or(Value::Void));
        }
        self.index_value(base, args, out)
    }

    /// `base(args)` where `base` is any value, not only a variable name.
    pub(super) fn index_value(
        &mut self,
        base: Value,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Value> {
        if let Some(value) = self.obj_index(&base, args, out)? {
            return Ok(value);
        }
        match base {
            Value::Container(Array::Handle(handle)) => {
                self.call_handle_expression(&handle, args, out)
            }
            Value::Container(a) => self.index_container(&a, args, false, out),
            Value::Matrix(m) => Ok(Value::Matrix(self.index_array(&m, args, out, true)?)),
            Value::Logical(m) => Ok(Value::Logical(self.index_array(&m, args, out, true)?)),
            Value::Complex(m) => Ok(Value::Complex(self.index_array(&m, args, out, true)?)),
            Value::Text(_) => {
                let a = base.array(&self.cancel)?;
                self.index_container(&a, args, false, out)
            }
            _ => Err("Cannot index this value".into()),
        }
    }

    pub(super) fn brace_values(
        &mut self,
        base: Value,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let Value::Container(a @ Array::Cell(_)) = base else {
            return Err("Cell contents reference from a non-cell array object".into());
        };
        self.cell_contents(&a, args, out)
    }

    fn temp_name(&self, prefix: &str) -> String {
        (0usize..)
            .map(|n| format!("Zq_{prefix}_{n}"))
            .find(|name| !self.contains_array(name))
            .expect("unbounded candidate names")
    }

    /// Assignment through a path such as `s(2).a.b(3) = v`.
    pub(super) fn assign_path(
        &mut self,
        root: &str,
        steps: &[PathStep],
        rhs: &Expr,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        Self::validate_workspace_name(root)?;
        let deletion = matches!(rhs, Expr::Matrix(rows) if rows.is_empty());
        let value = self.eval(rhs, None, out)?;
        let value = self.assigned_array(value)?;
        let current = self.array(root);
        let updated = self.assign_step(current, steps, value, deletion, out)?;
        self.assign_workspace_array(root, updated, quiet, out)
    }

    pub(super) fn assign_step(
        &mut self,
        current: Option<Array>,
        steps: &[PathStep],
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        self.check_cancel()?;
        let Some((step, rest)) = steps.split_first() else {
            return Ok(value);
        };
        if let Some(array) = &current
            && self.obj_assign_nested(array, steps, &value, out)?
        {
            return Ok(array.clone());
        }
        match step {
            PathStep::Field(name) => self.assign_field(current, name, rest, value, deletion, out),
            PathStep::Dynamic(expr) => {
                let name = self.dynamic_field_name(expr, out)?;
                self.assign_field(current, &name, rest, value, deletion, out)
            }
            PathStep::Paren(args) => self.assign_paren(current, args, rest, value, deletion, out),
            PathStep::Brace(args) => {
                if !rest.is_empty() {
                    // A live object in the selected cell takes its own nested
                    // assignment; only then are the brace subscripts evaluated
                    // here (cel_assign_child evaluates them once otherwise).
                    if self.obj_live()
                        && let Some(array) = &current
                    {
                        let contents = self.brace_values(Value::from(array.clone()), args, out)?;
                        if let [child] = contents.as_slice()
                            && self.obj_assign_nested(
                                &child.clone().array(&self.cancel)?,
                                rest,
                                &value,
                                out,
                            )?
                        {
                            return Ok(array.clone());
                        }
                    }
                    return self.cel_assign_child(current, args, rest, value, deletion, out);
                }
                self.assign_via_variable(current, args, true, value, deletion, out)
            }
        }
    }

    fn is_empty_double(array: &Array) -> bool {
        matches!(array, Array::Numeric(m) if m.rows() == 0 && m.cols() == 0)
    }

    fn assign_field(
        &mut self,
        current: Option<Array>,
        name: &str,
        rest: &[PathStep],
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        self.lng_check_field_assign(&current, name)?;
        let base = match current {
            Some(Array::Struct(s)) => s,
            None => StructArray::scalar_empty(&self.cancel)?,
            Some(ref other) if Self::is_empty_double(other) => {
                StructArray::scalar_empty(&self.cancel)?
            }
            Some(_) => return Err("Scalar structure required for this assignment".into()),
        };
        if base.len() != 1 {
            return Err("Scalar structure required for this assignment".into());
        }
        let child = match base.field_index(name) {
            Some(field) => Some(base.get(field, 0)?.as_ref().clone()),
            None => None,
        };
        let child = self.assign_step(child, rest, value, deletion, out)?;
        Ok(Array::Struct(base.with_field(name, child, &self.cancel)?))
    }

    fn assign_paren(
        &mut self,
        current: Option<Array>,
        args: &[Expr],
        rest: &[PathStep],
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        let creates_struct = match &current {
            Some(Array::Struct(_)) => true,
            None => !rest.is_empty() || matches!(value, Array::Struct(_)),
            Some(array) => {
                Self::is_empty_double(array)
                    && (!rest.is_empty() || matches!(value, Array::Struct(_)))
            }
        };
        if !creates_struct {
            if !rest.is_empty() {
                return Err(NO_FIELDS.into());
            }
            return self.assign_via_variable(current, args, false, value, deletion, out);
        }
        let base = match current {
            Some(Array::Struct(s)) => Some(s),
            _ => None,
        };
        self.assign_struct_element(base, args, rest, value, deletion, out)
    }

    fn subscript_number(&mut self, expr: &Expr, len: usize, out: &mut Execution) -> Result<usize> {
        if matches!(expr, Expr::All) {
            return Err("Struct assignment with ':' is not supported yet".into());
        }
        let value = self.eval(expr, Some(len), out)?;
        let Value::Matrix(m) = value else {
            return Err("Struct subscripts must be positive integers".into());
        };
        if m.len() != 1 {
            return Err("Struct assignment with several subscripts is not supported yet".into());
        }
        let k = m.data()[0];
        if !k.is_finite() || k.fract() != 0. || k < 1. || k > MAX_CELL_SLOTS as f64 {
            return Err(format!(
                "Index must be an integer between 1 and {MAX_CELL_SLOTS}"
            ));
        }
        Ok(k as usize)
    }

    fn struct_position(
        &mut self,
        shape: (usize, usize),
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Position> {
        let (rows, cols) = shape;
        let len = rows * cols;
        if let [index] = args {
            let k = self.subscript_number(index, len, out)?;
            if k <= len {
                return Ok(Position {
                    index: k - 1,
                    shape,
                });
            }
            let shape = if len == 0 || rows == 1 {
                (1, k)
            } else if cols == 1 {
                (k, 1)
            } else {
                return Err("Attempt to grow array along ambiguous dimension".into());
            };
            return Ok(Position {
                index: k - 1,
                shape,
            });
        }
        let [row, col] = args else {
            return Err("Indexed assignment requires one or two subscripts".into());
        };
        let i = self.subscript_number(row, rows, out)?;
        let j = self.subscript_number(col, cols, out)?;
        let (new_rows, new_cols) = (rows.max(i), cols.max(j));
        if new_rows.saturating_mul(new_cols) > MAX_CELL_SLOTS {
            return Err("Struct array exceeds 65536 elements".into());
        }
        Ok(Position {
            index: (j - 1) * new_rows + (i - 1),
            shape: (new_rows, new_cols),
        })
    }

    fn assign_struct_element(
        &mut self,
        base: Option<StructArray>,
        args: &[Expr],
        rest: &[PathStep],
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        if rest.is_empty() && base.is_some() {
            return self.cel_assign(base.map(Array::Struct), args, false, value, deletion, out);
        }
        if deletion && rest.is_empty() {
            return self.delete_struct_elements(base, args, out);
        }
        let shape = base.as_ref().map_or((0, 0), |b| (b.rows(), b.cols()));
        let position = self.struct_position(shape, args, out)?;
        let element = if rest.is_empty() {
            let Array::Struct(t) = value else {
                return Err("Subscripted assignment of a non-struct value into a struct array is not supported".into());
            };
            if t.len() != 1 {
                return Err("Struct element assignment requires a 1x1 struct".into());
            }
            if let Some(b) = &base
                && (!b.is_empty() || b.field_count() > 0)
                && (b.field_count() != t.field_count()
                    || t.field_names().iter().any(|n| b.field_index(n).is_none()))
            {
                return Err(self.raise(
                    "Subscripted assignment between dissimilar structures.".into(),
                    "MATLAB:heterogeneousStrucAssignment".into(),
                ));
            }
            t
        } else {
            let existing = match &base {
                Some(b)
                    if position.index % position.shape.0 < b.rows()
                        && position.index / position.shape.0 < b.cols() =>
                {
                    b.element(
                        position.index % position.shape.0
                            + position.index / position.shape.0 * b.rows(),
                        &self.cancel,
                    )?
                }
                Some(b) => StructArray::filled_empty(1, 1, b.field_names().to_vec(), &self.cancel)?,
                None => StructArray::scalar_empty(&self.cancel)?,
            };
            match self.assign_step(Some(Array::Struct(existing)), rest, value, deletion, out)? {
                Array::Struct(s) => s,
                _ => return Err(NO_FIELDS.into()),
            }
        };
        let base = match base {
            Some(b) => b,
            None => StructArray::from_slots(0, 0, Vec::new(), Vec::new(), &self.cancel)?,
        };
        Ok(Array::Struct(base.with_element(
            position.index,
            position.shape,
            &element,
            &self.cancel,
        )?))
    }

    /// `s(k) = []` for scalar, range or logical subscripts on a struct vector.
    fn delete_struct_elements(
        &mut self,
        base: Option<StructArray>,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Array> {
        let Some(base) = base else {
            return Err("Cannot delete elements of an undefined struct array".into());
        };
        let whole = Array::Struct(base.clone());
        let (_, positions) = self.container_positions(&whole, args, out)?;
        if args.len() != 1 || !(base.rows() == 1 || base.cols() == 1) {
            return Err(
                "Deleting struct elements is supported for one subscript on a vector only".into(),
            );
        }
        let mut doomed = vec![false; base.len()];
        for (i, &p) in positions.iter().enumerate() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            doomed[p] = true;
        }
        let keep: Vec<usize> = (0..base.len()).filter(|i| !doomed[*i]).collect();
        let shape = if base.rows() == 1 {
            (1, keep.len())
        } else {
            (keep.len(), 1)
        };
        Ok(Array::Struct(base.select(
            shape.0,
            shape.1,
            &keep,
            &self.cancel,
        )?))
    }

    /// `[s1, s2, ...]` for struct rows with identical field sets.
    pub(super) fn concatenate_structs(&mut self, rows: Vec<Vec<Value>>) -> Result<Value> {
        if rows.len() > 1 {
            return Err("Vertical struct concatenation is not supported yet".into());
        }
        let mut parts = Vec::new();
        for value in rows.into_iter().flatten() {
            self.check_cancel()?;
            match value {
                Value::Container(Array::Struct(s)) => parts.push(s),
                Value::Matrix(m) if m.rows() == 0 && m.cols() == 0 => {}
                _ => {
                    return Err(
                        "Concatenating structs with other classes is not supported yet".into(),
                    );
                }
            }
        }
        let Some(first) = parts.first() else {
            return Err("Struct concatenation needs at least one struct".into());
        };
        let names = first.field_names().to_vec();
        let mut slots = Vec::new();
        let mut total = 0usize;
        for part in &parts {
            if part.field_count() != names.len()
                || names.iter().any(|n| part.field_index(n).is_none())
            {
                return Err("Field names of the structures being concatenated must match".into());
            }
            if part.rows() > 1 {
                return Err("Concatenating struct matrices is not supported yet".into());
            }
            total += part.len();
            for element in 0..part.len() {
                if element.is_multiple_of(256) {
                    self.check_cancel()?;
                }
                for name in &names {
                    let field = part.field_index(name).expect("checked above");
                    slots.push(part.get(field, element)?.clone());
                }
            }
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            total,
            names,
            slots,
            &self.cancel,
        )?)))
    }

    /// Reuse the name-based indexed assignment for non-struct targets by binding
    /// the current value (and the new one) to short-lived hidden variables.
    fn assign_via_variable(
        &mut self,
        current: Option<Array>,
        args: &[Expr],
        brace: bool,
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        if let Some(Array::Struct(_)) = current {
            return Err("Curly brace assignment requires a cell array".into());
        }
        let target = self.temp_name("target");
        if let Some(array) = current {
            self.publish_array(target.clone(), array);
        }
        let source = self.temp_name("value");
        self.publish_array(source.clone(), value);
        let rhs = if deletion {
            Expr::Matrix(Vec::new())
        } else {
            Expr::Name(source.clone())
        };
        let result = if brace
            || (self.container_variables.contains_key(&target)
                && !matches!(self.container_variables.get(&target), Some(Array::Typed(_))))
        {
            self.assign_container(&target, args, brace, &rhs, true, out)
        } else {
            self.assign_indexed(&target, args, &rhs, true, out)
        };
        let updated = self.array(&target);
        self.remove_array(&target);
        self.remove_array(&source);
        result?;
        updated.ok_or_else(|| "Indexed assignment produced no value".into())
    }

    // ---- builtins ----

    pub(super) fn struct_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !matches!(
            name,
            "struct"
                | "fieldnames"
                | "isfield"
                | "isstruct"
                | "rmfield"
                | "setfield"
                | "getfield"
                | "numfields"
        ) {
            return Ok(None);
        }
        self.check_cancel()?;
        let value = match name {
            "struct" => self.struct_constructor(args)?,
            "isstruct" => {
                let [value] = args else {
                    return Err("isstruct requires exactly one input".into());
                };
                Value::Logical(LogicalMatrix::scalar(matches!(
                    value,
                    Value::Container(Array::Struct(_))
                )))
            }
            "fieldnames" | "numfields" => {
                let [Value::Container(Array::Struct(s))] = args else {
                    return Err(format!("{name}: the input must be a struct"));
                };
                if name == "numfields" {
                    Value::Matrix(crate::Matrix::scalar(s.field_count() as f64))
                } else {
                    self.field_name_cell(s.field_names())?
                }
            }
            "isfield" => self.isfield_call(args)?,
            "rmfield" => self.rmfield_call(args)?,
            "getfield" => self.getfield_call(args)?,
            _ => self.setfield_call(args)?,
        };
        Ok(Some(value))
    }

    fn field_name_cell(&self, names: &[String]) -> Result<Value> {
        let slots = names
            .iter()
            .map(|n| Ok(Arc::new(self.char_array(n)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::Container(Array::Cell(CellArray::from_shared(
            names.len(),
            1,
            slots,
            &self.cancel,
        )?)))
    }

    fn struct_constructor(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Ok(Value::Container(Array::Struct(StructArray::scalar_empty(
                &self.cancel,
            )?)));
        }
        if !args.len().is_multiple_of(2) {
            return Err("struct: arguments must be field name and value pairs".into());
        }
        let mut names: Vec<String> = Vec::new();
        let mut values: Vec<Array> = Vec::new();
        for pair in args.chunks(2) {
            let name = self.text_name(pair[0].clone(), "struct: field names")?;
            if !valid_field_name(&name) {
                return Err(format!("struct: invalid field name '{name}'"));
            }
            if names.contains(&name) {
                return Err("Duplicate field names are not allowed".into());
            }
            names.push(name);
            values.push(self.assigned_array(pair[1].clone())?);
        }
        // Cell values spread over struct elements; a 1x1 cell is its content.
        let mut shape: Option<(usize, usize)> = None;
        for v in &values {
            if let Array::Cell(c) = v
                && c.len() != 1
            {
                let s = (c.rows(), c.cols());
                if shape.is_some_and(|known| known != s) {
                    return Err("struct: cell array dimensions must match".into());
                }
                shape = Some(s);
            }
        }
        let (rows, cols) = shape.unwrap_or((1, 1));
        let count = rows * cols;
        let total = StructArray::slot_total(count, names.len())?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(total)
            .map_err(|_| "Cannot allocate struct")?;
        for element in 0..count {
            if element.is_multiple_of(256) {
                self.check_cancel()?;
            }
            for v in &values {
                slots.push(match v {
                    Array::Cell(c) if c.len() == 1 => c.get_linear(0)?.clone(),
                    Array::Cell(c) => c.get_linear(element)?.clone(),
                    other => Arc::new(other.clone()),
                });
            }
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            rows,
            cols,
            names,
            slots,
            &self.cancel,
        )?)))
    }

    fn isfield_call(&self, args: &[Value]) -> Result<Value> {
        let [target, names] = args else {
            return Err("isfield requires a struct and a field name or cell array of names".into());
        };
        let s = match target {
            Value::Container(Array::Struct(s)) => Some(s),
            _ => None,
        };
        let has = |name: &Array| -> bool {
            matches!((s, name), (Some(s), Array::Character(c))
                if c.is_text_row() && c.to_utf8(&self.cancel).is_ok_and(|n| s.field_index(&n).is_some()))
        };
        match names {
            Value::Container(Array::Cell(cell)) => {
                let mut flags = Vec::with_capacity(cell.len());
                for i in 0..cell.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    flags.push(u8::from(has(cell.get_linear(i)?)));
                }
                Ok(Value::Logical(LogicalMatrix::from_canonical_vec(
                    cell.rows(),
                    cell.cols(),
                    flags,
                )?))
            }
            other => {
                let name = other.clone().array(&self.cancel);
                Ok(Value::Logical(LogicalMatrix::scalar(
                    name.is_ok_and(|n| has(&n)),
                )))
            }
        }
    }

    fn rmfield_call(&self, args: &[Value]) -> Result<Value> {
        let [Value::Container(Array::Struct(s)), names] = args else {
            return Err("rmfield requires a struct and a field name or cell array of names".into());
        };
        let mut list = Vec::new();
        match names {
            Value::Container(Array::Cell(cell)) => {
                for i in 0..cell.len() {
                    let Array::Character(c) = cell.get_linear(i)?.as_ref() else {
                        return Err("rmfield: names must be character text".into());
                    };
                    list.push(c.to_utf8(&self.cancel)?);
                }
            }
            other => list.push(self.text_name(other.clone(), "rmfield: the field name")?),
        }
        let mut result = s.clone();
        for name in list {
            self.check_cancel()?;
            result = result.without_field(&name, &self.cancel)?;
        }
        Ok(Value::Container(Array::Struct(result)))
    }

    fn getfield_call(&mut self, args: &[Value]) -> Result<Value> {
        let Some((Value::Container(Array::Struct(s)), names)) = args.split_first() else {
            return Err("getfield requires a struct and field names".into());
        };
        if names.is_empty() {
            return Err("getfield requires at least one field name".into());
        }
        let mut current = Array::Struct(s.clone());
        for name in names {
            let name = match name {
                Value::Container(Array::Cell(_)) => {
                    return Err("getfield: index arguments are not supported yet".into());
                }
                other => self.text_name(other.clone(), "getfield: field names")?,
            };
            let Array::Struct(s) = &current else {
                return Err(NO_FIELDS.into());
            };
            if s.len() != 1 {
                return Err("getfield: struct arrays are not supported yet".into());
            }
            let Some(field) = s.field_index(&name) else {
                return Err(self.raise(
                    format!("Reference to non-existent field '{name}'."),
                    "MATLAB:nonExistentField".into(),
                ));
            };
            let next = s.get(field, 0)?.as_ref().clone();
            current = next;
        }
        Ok(Value::from(current))
    }

    fn setfield_call(&self, args: &[Value]) -> Result<Value> {
        let [Value::Container(Array::Struct(s)), names @ .., value] = args else {
            return Err("setfield requires a struct, field names and a value".into());
        };
        if names.is_empty() {
            return Err("setfield requires at least one field name".into());
        }
        // set_nested recurses once per name; a deeper path could never fit anyway.
        if names.len() > MAX_DEPTH {
            return Err(format!(
                "setfield: field path exceeds {MAX_DEPTH} nesting levels"
            ));
        }
        let names = names
            .iter()
            .map(|n| match n {
                Value::Container(Array::Cell(_)) => {
                    Err("setfield: index arguments are not supported yet".to_owned())
                }
                other => self.text_name(other.clone(), "setfield: field names"),
            })
            .collect::<Result<Vec<_>>>()?;
        let value = self.assigned_array(value.clone())?;
        Ok(Value::Container(Array::Struct(
            self.set_nested(s, &names, value)?,
        )))
    }

    fn set_nested(&self, s: &StructArray, names: &[String], value: Array) -> Result<StructArray> {
        self.check_cancel()?;
        if s.len() != 1 {
            return Err("setfield: struct arrays are not supported yet".into());
        }
        let (name, rest) = names.split_first().expect("nonempty path");
        let child = if rest.is_empty() {
            value
        } else {
            let inner = match s.field_index(name) {
                Some(f) => match s.get(f, 0)?.as_ref() {
                    Array::Struct(inner) => inner.clone(),
                    other if Self::is_empty_double(other) => {
                        StructArray::scalar_empty(&self.cancel)?
                    }
                    _ => return Err("Scalar structure required for this assignment".into()),
                },
                None => StructArray::scalar_empty(&self.cancel)?,
            };
            Array::Struct(self.set_nested(&inner, rest, value)?)
        };
        s.with_field(name, child, &self.cancel)
    }
}
