//! Initial stored character-row and bounded 2-D cell-array evaluator.
use super::{Engine, Execution, Value, indexing::Indices};
use crate::{Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, syntax::Expr};
use std::sync::Arc;
const MAX: usize = crate::script_containers::MAX_CELL_SLOTS;

impl Engine {
    pub(super) fn text_value(&self, value: Value) -> Result<String> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Ok(s),
            Value::Container(Array::Character(a)) => a.to_utf8(&self.cancel),
            _ => Err("Expected character text or a quoted option".into()),
        }
    }
    /// Rename all admitted array types without copying data or omitting cells.
    pub fn rename_array(&mut self, from: &str, to: &str) -> Result<()> {
        Self::validate_workspace_name(to)?;
        if self.contains_array(to) {
            return Err("That variable name already exists".into());
        }
        let value = self.array(from).ok_or("Unknown workspace variable")?;
        self.check_cancel()?;
        self.publish_array(to.into(), value);
        self.remove_array(from);
        Ok(())
    }
    pub(super) fn text_arguments(&self, args: Vec<Value>) -> Result<Vec<Value>> {
        if !args.iter().any(|v| {
            matches!(
                v,
                Value::QuotedString(_) | Value::Container(Array::Character(_))
            )
        }) {
            return Ok(args);
        }
        args.into_iter()
            .map(|v| match v {
                Value::QuotedString(s) => Ok(Value::Text(s)),
                Value::Container(Array::Character(a)) => Ok(Value::Text(a.to_utf8(&self.cancel)?)),
                other => Ok(other),
            })
            .collect()
    }
    /// Borrow shape metadata before graphics/text coercion. Never inspect payloads.
    pub(super) fn isempty_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let [input] = args else {
            return Err("isempty requires exactly one array input".into());
        };
        let empty = match input {
            Value::Matrix(a) => a.is_empty(),
            Value::Logical(a) => a.is_empty(),
            Value::Complex(a) => a.is_empty(),
            Value::Container(a) => a.is_empty(),
            // Single-quoted character literals need no UTF16 allocation here.
            Value::Text(text) => text.is_empty(),
            Value::QuotedString(_) | Value::Void => {
                return Err(
                    "isempty requires an admitted array; MATLAB string arrays are not implemented"
                        .into(),
                );
            }
        };
        Ok(Value::Logical(LogicalMatrix::scalar(empty)))
    }

    pub(super) fn container_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        let one = || {
            if args.len() == 1 {
                Ok(())
            } else {
                Err("Expected one input".to_owned())
            }
        };
        if matches!(name, "iscell" | "ischar" | "class") {
            one()?;
            let a = args[0].clone().array(&self.cancel)?;
            return Ok(Some(if name == "class" {
                Value::Text(a.class().into())
            } else {
                Value::Logical(LogicalMatrix::scalar(if name == "iscell" {
                    matches!(a, Array::Cell(_))
                } else {
                    matches!(a, Array::Character(_))
                }))
            }));
        }
        if name == "cell" {
            let dimension = |v: f64| -> Result<usize> {
                if !v.is_finite() || v < 0. || v.fract() != 0. || v > MAX as f64 {
                    Err("Cell dimensions must be integers in 0..65536".into())
                } else {
                    Ok(v as usize)
                }
            };
            let (rows, cols) = match args {
                [Value::Matrix(a)] if a.len() == 1 => { let n = dimension(a.as_scalar()?)?; (n,n) },
                [Value::Matrix(a)] if a.rows() == 1 && a.len() == 2 => (dimension(a.data()[0])?,dimension(a.data()[1])?),
                [Value::Matrix(a), Value::Matrix(b)] => (dimension(a.as_scalar()?)?,dimension(b.as_scalar()?)?),
                _ => return Err("Use cell(n), cell(rows,cols), or cell([rows cols]); 2-D numeric dimensions only".into()),
            };
            return Ok(Some(
                CellArray::empty(rows, cols, &self.cancel)?.into_array_value(),
            ));
        }
        if name == "char" {
            one()?;
            return Ok(Some(
                if matches!(args[0], Value::Container(Array::Cell(_))) {
                    self.padded_characters(args, false)?
                } else {
                    Value::Container(self.character_argument(&args[0])?.into())
                },
            ));
        }
        if name == "strvcat" {
            return Ok(Some(self.padded_characters(args, true)?));
        }
        let container = match args.first() {
            Some(Value::Container(a)) => Some(a.clone()),
            Some(Value::Text(s)) if matches!(name, "size" | "numel" | "length") => {
                Some(CharacterRow::from_text(s, &self.cancel)?.into())
            }
            _ => None,
        };
        let Some(a) = container else {
            return Ok(None);
        };
        match name {
            // Preserve typed shape/UTF16 before the later row-only text projection.
            "reshape" if matches!(a, Array::Character(_)) => self.shape_call(name, args),
            "size" => {
                let result = match args {
                    [_] => Matrix::from_column_slice(1, 2, &[a.rows() as f64, a.cols() as f64])?,
                    [_, Value::Matrix(dim)] => {
                        let d = dim.as_scalar()?;
                        if !d.is_finite()
                            || d < 1.
                            || d.fract() != 0.
                            || d > crate::matrix::MAX_ELEMENTS as f64
                        {
                            return Err("Invalid size dimension".into());
                        }
                        Matrix::scalar(if d == 1. {
                            a.rows() as f64
                        } else if d == 2. {
                            a.cols() as f64
                        } else {
                            1.
                        })
                    }
                    _ => return Err("size expects an array and optional scalar dimension".into()),
                };
                Ok(Some(Value::Matrix(result)))
            }
            "numel" | "length" => {
                one()?;
                Ok(Some(Value::Matrix(Matrix::scalar(if name == "numel" {
                    a.len()
                } else if a.is_empty() {
                    0
                } else {
                    a.rows().max(a.cols())
                }
                    as f64))))
            }
            // All other dispatch remains explicit. Character options/paths are
            // projected later, but cells cannot fall through to numeric kernels.
            _ => Ok(None),
        }
    }

    pub(super) fn container_positions(
        &mut self,
        a: &Array,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<((usize, usize), Vec<usize>)> {
        if args.is_empty() || args.len() > 2 {
            return Err("Container indexing requires one or two subscripts".into());
        }
        let bound = if args.len() == 1 { a.len() } else { a.rows() };
        let containers = matches!(a, Array::Cell(_) | Array::Struct(_) | Array::Character(_));
        let (rows, mut shape) = if containers {
            self.cel_subscript(&args[0], bound, bound, out)?
        } else {
            self.subscript_with_limit(&args[0], Some(bound), bound, out)?
        };
        let cols = if args.len() == 2 {
            let (cols, _) = if containers {
                self.cel_subscript(&args[1], a.cols(), a.cols(), out)?
            } else {
                self.subscript_with_limit(&args[1], Some(a.cols()), a.cols(), out)?
            };
            shape = (rows.len(), cols.len());
            Some(cols)
        } else {
            if !self.colon_subscript(&args[0])
                && shape != (0, 0)
                && (a.rows() == 1 || a.cols() == 1)
                && match &rows {
                    Indices::Mask(_, vector) | Indices::LogicalStride { vector, .. } => *vector,
                    _ => shape.0 == 1 || shape.1 == 1,
                }
            {
                shape = if a.rows() == 1 {
                    (1, rows.len())
                } else {
                    (rows.len(), 1)
                };
            }
            None
        };
        let n = shape
            .0
            .checked_mul(shape.1)
            .ok_or("Container index shape overflow")?;
        if let Array::Typed(t) = a {
            Matrix::checked_len(shape.0, shape.1)?;
            // Index scratch is one usize per selection; output keeps its class.
            // Preflight both before allocating the position buffer.
            let output_units = n.saturating_mul(t.class().size()).div_ceil(8);
            if self
                .evaluated_elements
                .saturating_add(n)
                .saturating_add(output_units)
                > 32 * 1024 * 1024
            {
                return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
            }
        } else if n > MAX || shape.0 > MAX || shape.1 > MAX {
            return Err("Container selection exceeds 65536 slots".into());
        }
        let mut positions = Vec::new();
        positions
            .try_reserve_exact(n)
            .map_err(|_| "Cannot allocate container indices")?;
        for i in 0..n {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let index = if let Some(cols) = &cols {
                cols.at(i / rows.len()) * a.rows() + rows.at(i % rows.len())
            } else {
                rows.at(i)
            };
            positions.push(index);
        }
        Ok((shape, positions))
    }
    pub(super) fn index_container(
        &mut self,
        a: &Array,
        args: &[Expr],
        content: bool,
        out: &mut Execution,
    ) -> Result<Value> {
        if content {
            let values = self.cell_contents_inner(a, args, out, false)?;
            if values.len() != 1 {
                return Err("Cell content expression produces a comma-separated list; use it in concatenation, arguments or matching multiple outputs".into());
            }
            return Ok(values.into_iter().next().unwrap());
        }
        let (shape, positions) = self.container_positions(a, args, out)?;
        match a {
            Array::Graphics(h) => Ok(Value::Container(
                h.selected(shape.0, shape.1, &positions, &self.cancel)?
                    .into(),
            )),
            Array::Cell(c) => {
                let mut slots = Vec::new();
                slots
                    .try_reserve_exact(positions.len())
                    .map_err(|_| "Cannot allocate cell selection")?;
                for (i, index) in positions.into_iter().enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    slots.push(c.get_linear(index)?.clone());
                }
                Ok(
                    CellArray::from_shared(shape.0, shape.1, slots, &self.cancel)?
                        .into_array_value(),
                )
            }
            Array::Character(c) => {
                let mut units = Vec::new();
                units
                    .try_reserve_exact(positions.len())
                    .map_err(|_| "Cannot allocate character selection")?;
                for (i, index) in positions.into_iter().enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    units.push(c.units()[index]);
                }
                Ok(Value::Container(
                    CharacterRow::from_column_major(shape.0, shape.1, units, &self.cancel)?.into(),
                ))
            }
            Array::Struct(s) => Ok(Value::Container(
                s.select(shape.0, shape.1, &positions, &self.cancel)?.into(),
            )),
            Array::Typed(t) => {
                self.charge(positions.len())?;
                Ok(Value::Container(
                    t.select_with_cancel(shape.0, shape.1, &positions, &self.cancel)?
                        .into(),
                ))
            }
            _ => Err("Expected a character or cell array".into()),
        }
    }
    pub(super) fn cell_contents(
        &mut self,
        a: &Array,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.cell_contents_inner(a, args, out, true)
    }
    fn cell_contents_inner(
        &mut self,
        a: &Array,
        args: &[Expr],
        out: &mut Execution,
        charge_results: bool,
    ) -> Result<Vec<Value>> {
        let Array::Cell(c) = a else {
            return Err("Curly content indexing requires a cell array".into());
        };
        let (_, positions) = self.container_positions(a, args, out)?;
        let mut units = 0usize;
        for (i, &index) in positions.iter().enumerate() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            units = units
                .checked_add(c.get_linear(index)?.bytes().div_ceil(8))
                .ok_or("Cell content budget overflow")?;
            if self.evaluated_elements.saturating_add(units) > 32 * 1024 * 1024 {
                return Err("Cell content list exceeds evaluation budget".into());
            }
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(positions.len())
            .map_err(|_| "Cannot allocate content list")?;
        for (i, index) in positions.into_iter().enumerate() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            values.push(c.get_linear(index)?.as_ref().clone().into());
        }
        if charge_results {
            self.charge(units)?;
        }
        self.check_cancel()?;
        Ok(values)
    }
    pub(super) fn eval_list(
        &mut self,
        expression: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        match expression {
            Expr::CellContents(name, args) => {
                let a = self.array(name).ok_or("Unknown cell variable")?;
                self.cell_contents(&a, args, out)
            }
            Expr::Field(..) | Expr::DynamicField(..) => self.field_values(expression, end, out),
            Expr::Brace(base, args) => {
                let base = self.eval(base, end, out)?;
                self.brace_values(base, args, out)
            }
            _ => Ok(vec![self.eval(expression, end, out)?]),
        }
    }
    pub(super) fn cell_literal(
        &mut self,
        rows: &[Vec<Expr>],
        out: &mut Execution,
    ) -> Result<Value> {
        let mut evaluated = Vec::new();
        let mut columns = None;
        let mut count = 0usize;
        for row in rows {
            let mut values = Vec::new();
            for expression in row {
                let list = self.eval_list(expression, None, out)?;
                count = count
                    .checked_add(list.len())
                    .ok_or("Cell literal size overflow")?;
                if count > MAX {
                    return Err("Cell literal exceeds 65536 slots".into());
                }
                values
                    .try_reserve_exact(list.len())
                    .map_err(|_| "Cannot allocate cell literal row")?;
                for (i, value) in list.into_iter().enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    values.push(Arc::new(value.array(&self.cancel)?));
                }
            }
            if columns.is_some_and(|n| n != values.len()) {
                return Err("Cell literal rows require equal slot counts".into());
            }
            columns = Some(values.len());
            evaluated.push(values);
        }
        let columns = columns.unwrap_or(0);
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate cell literal")?;
        for col in 0..columns {
            for row in &evaluated {
                self.check_cancel()?;
                slots.push(row[col].clone());
            }
        }
        Ok(CellArray::from_shared(rows.len(), columns, slots, &self.cancel)?.into_array_value())
    }
    pub(super) fn assign_container(
        &mut self,
        name: &str,
        args: &[Expr],
        content: bool,
        rhs: &Expr,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        if content || matches!(self.array(name), Some(Array::Cell(_) | Array::Character(_))) {
            let old = self.array(name);
            let value = self.eval(rhs, None, out)?.array(&self.cancel)?;
            let deletion = matches!(rhs, Expr::Matrix(rows) if rows.is_empty());
            let updated = self.cel_assign(old, args, content, value, deletion, out)?;
            return self.assign_workspace_array(name, updated, quiet, out);
        }
        let old=self.array(name).ok_or("Container assignment requires an existing variable; implicit cell growth is unsupported")?;
        let value = self.eval(rhs, None, out)?.array(&self.cancel)?;
        let (_, positions) = self.container_positions(&old, args, out)?;
        if positions.len() != 1 {
            return Err("This initial container assignment supports exactly one existing slot; growth/deletion/multiple writes are unsupported".into());
        }
        let index = positions[0];
        let updated = match (&old, content) {
            (Array::Graphics(h), false) => {
                let Array::Graphics(replacement) = value else {
                    return Err("Graphics handle replacement requires a handle array".into());
                };
                if replacement.len() != 1 {
                    return Err("One handle is required for a single-slot replacement".into());
                }
                h.replaced(index, replacement.get_linear(0)?.clone(), &self.cancel)?
                    .into()
            }
            (Array::Cell(c), true) => c.replaced(index, value, &self.cancel)?.into(),
            (Array::Cell(c), false) => {
                let Array::Cell(replacement) = value else {
                    return Err("Parenthesis cell assignment requires a cell array; use braces for contents".into());
                };
                if replacement.len() != 1 {
                    return Err("Scalar cell replacement requires one cell".into());
                }
                c.replaced(
                    index,
                    replacement.get_linear(0)?.as_ref().clone(),
                    &self.cancel,
                )?
                .into()
            }
            (Array::Character(c), false) => {
                let Array::Character(replacement) = value else {
                    return Err("Character replacement requires a character value".into());
                };
                if replacement.len() != 1 {
                    return Err("Initial character writes require one UTF-16 unit".into());
                }
                c.replaced(index, replacement.units()[0], &self.cancel)?
                    .into()
            }
            _ => return Err("Curly assignment requires a cell array".into()),
        };
        self.assign_workspace_array(name, updated, quiet, out)
    }
    pub(super) fn concatenate_characters(&mut self, rows: Vec<Vec<Value>>) -> Result<Value> {
        if rows
            .iter()
            .flatten()
            .any(|value| matches!(value, Value::Container(Array::Struct(_))))
        {
            return self.concatenate_structs(rows);
        }
        if rows
            .iter()
            .flatten()
            .any(|value| matches!(value, Value::Container(Array::Graphics(_))))
        {
            return self.concatenate_graphics(rows);
        }
        let mut blocks = Vec::new();
        let mut units = 0usize;
        for row in rows {
            self.check_cancel()?;
            let mut parts = Vec::new();
            for value in row {
                let (r, c) = value.shape(&self.cancel)?;
                if !value.is_character() && !matches!(value, Value::Matrix(_) | Value::Logical(_)) {
                    return Err(
                        "Mixed character/numeric or cell concatenation is not implemented".into(),
                    );
                }
                units = units
                    .checked_add(CharacterRow::checked_len(r, c)?)
                    .ok_or("Character concatenation overflow")?;
                CharacterRow::checked_len(1, units)?;
                let part = self.character_argument(&value)?;
                // Canonical [] and '' are neutral; shaped empties retain shape.
                if (part.rows(), part.cols()) != (0, 0) {
                    parts.push(part);
                }
            }
            if !parts.is_empty() {
                blocks.push(parts);
            }
        }
        self.character_blocks(blocks, false)
    }
}
impl CellArray {
    pub(super) fn into_array_value(self) -> Value {
        Value::Container(self.into())
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn temporary_character_conversion_and_shape_observe_actual_stop() {
        let token = AtomicBool::new(true);
        for text in ["", "small", "\u{03bb}\u{1f600}"] {
            let value = Value::Text(text.into());
            assert!(value.clone().array(&token).unwrap_err().contains("cancel"));
            assert!(value.shape(&token).unwrap_err().contains("cancel"));
        }
        token.store(false, Ordering::Relaxed);
        let value = Value::Text("\u{03bb}\u{1f600}".into());
        assert_eq!(value.shape(&token).unwrap(), (1, 3));
        let Array::Character(row) = value.array(&token).unwrap() else {
            panic!()
        };
        assert_eq!(row.units(), &[955, 55357, 56832]);
        assert_eq!(Value::Text(String::new()).shape(&token).unwrap(), (0, 0));
        assert!(Value::Text("x".repeat(65537)).shape(&token).is_err());
    }
}
