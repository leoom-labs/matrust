//! Cell/struct rearrangements and text ordering. Payloads stay immutable and
//! shared: container operations move only slot pointers, never nested matrices.
use super::{Engine, Execution, Value, shape_arguments};
use crate::{Array, CellArray, CharacterRow, Matrix, Result, StructArray};
use std::sync::Arc;

const MAX: usize = crate::script_containers::MAX_CELL_SLOTS;
const DIMENSIONS: &str = "Dimensions of matrices being concatenated are not consistent.";

enum CelOutputTarget<'a> {
    Variable(&'a str),
    Paren(&'a str, &'a [crate::syntax::Expr]),
    Slots(&'a str, Option<&'a str>, Vec<usize>),
    Ignore,
}

impl Engine {
    pub(super) fn cel_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if name == "struct" && matches!(args, [Value::Matrix(a)] if a.is_empty()) {
            return Ok(Some(Value::Container(
                StructArray::from_slots(0, 0, Vec::new(), Vec::new(), &self.cancel)?.into(),
            )));
        }
        if matches!(name, "cat" | "horzcat" | "vertcat") {
            let parts = if name == "cat" {
                args.get(1..).unwrap_or_default()
            } else {
                args
            };
            if parts
                .iter()
                .any(|v| matches!(v, Value::Container(Array::Struct(_))))
                && !parts
                    .iter()
                    .any(|v| matches!(v, Value::Container(Array::Cell(_))))
            {
                let dim = if name == "cat" {
                    let dim = self.matbuild_scalar("cat", &args[0])?;
                    if !(dim >= 1. && dim.fract() == 0. && dim < 1.0e9) {
                        return Err("Dimension must be a real positive finite integer.".into());
                    }
                    dim as i64
                } else if name == "vertcat" {
                    1
                } else {
                    2
                };
                if dim > 2 {
                    return self.matbuild_call(name, args);
                }
                let rows = if dim == 1 {
                    parts.iter().cloned().map(|v| vec![v]).collect()
                } else {
                    vec![parts.to_vec()]
                };
                return self.cel_concatenate_warn(rows, out).map(Some);
            }
        }
        let container = args
            .first()
            .is_some_and(|v| matches!(v, Value::Container(Array::Cell(_) | Array::Struct(_))));
        if container
            && name != "squeeze"
            && (name != "rot90" || matches!(args.first(), Some(Value::Container(Array::Struct(_)))))
            && matches!(
                name,
                "reshape"
                    | "repmat"
                    | "transpose"
                    | "ctranspose"
                    | "flip"
                    | "flipud"
                    | "fliplr"
                    | "rot90"
                    | "circshift"
                    | "squeeze"
                    | "permute"
            )
        {
            return self.cel_shape(name, args).map(Some);
        }
        Ok(None)
    }

    fn cel_admit(&self, rows: usize, cols: usize, fields: usize) -> Result<usize> {
        self.check_cancel()?;
        let n = rows.checked_mul(cols).ok_or("Container shape overflow")?;
        if rows > MAX || cols > MAX || n > MAX || n.saturating_mul(fields) > MAX {
            return Err("Container shape exceeds 65536 slots".into());
        }
        if self
            .evaluated_elements
            .saturating_add(n.saturating_mul(fields.max(1) + 1))
            > 32 * 1024 * 1024
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(n)
    }

    fn cel_gather(
        &self,
        a: &Array,
        rows: usize,
        cols: usize,
        at: impl Fn(usize) -> usize,
    ) -> Result<Array> {
        let fields = if let Array::Struct(s) = a {
            s.field_count()
        } else {
            1
        };
        let n = self.cel_admit(rows, cols, fields)?;
        match a {
            Array::Cell(c) => {
                let mut slots = Vec::with_capacity(n);
                for i in 0..n {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    slots.push(c.get_linear(at(i))?.clone());
                }
                Ok(CellArray::from_shared(rows, cols, slots, &self.cancel)?.into())
            }
            Array::Struct(s) => {
                let mut slots = Vec::with_capacity(n * fields);
                for i in 0..n {
                    if i.is_multiple_of(256) {
                        self.check_cancel()?;
                    }
                    for f in 0..fields {
                        slots.push(s.get(f, at(i))?.clone());
                    }
                }
                Ok(StructArray::from_slots(
                    rows,
                    cols,
                    s.field_names().to_vec(),
                    slots,
                    &self.cancel,
                )?
                .into())
            }
            Array::Character(c) => {
                let mut units = Vec::with_capacity(n);
                for i in 0..n {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    units.push(c.units()[at(i)]);
                }
                Ok(CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into())
            }
            _ => Err("Expected a cell, struct or character array".into()),
        }
    }

    pub(super) fn cel_transpose(&self, a: &Array) -> Result<Value> {
        let (r, c) = (a.rows(), a.cols());
        Ok(self.cel_gather(a, c, r, |i| i / c + (i % c) * r)?.into())
    }

    fn cel_integer(&self, v: &Value) -> Result<i64> {
        let n = v.clone().matrix(&self.cancel)?.as_scalar()?;
        if !n.is_finite() || n.fract() != 0. || n.abs() > i32::MAX as f64 {
            return Err("Expected a bounded integer scalar".into());
        }
        Ok(n as i64)
    }

    fn cel_shape(&self, name: &str, args: &[Value]) -> Result<Value> {
        let a = args[0].clone().array(&self.cancel)?;
        let (r, c) = (a.rows(), a.cols());
        let arity = match name {
            "reshape" | "repmat" => 2..=33,
            "circshift" => 2..=3,
            "permute" => 2..=2,
            "flip" | "rot90" => 1..=2,
            _ => 1..=1,
        };
        if !arity.contains(&args.len()) {
            return Err("Incorrect number of input arguments.".into());
        }
        match name {
            "reshape" | "squeeze" => {
                let (rows, cols) = if name == "squeeze" {
                    (r, c)
                } else {
                    shape_arguments::shape(&args[1..], Some(a.len())).map_err(|e| {
                        if e.contains("preserve") || e.contains("divide") {
                            "To RESHAPE the number of elements must not change.".to_owned()
                        } else {
                            e
                        }
                    })?
                };
                self.cel_admit(rows, cols, 0)?;
                Ok(match a {
                    Array::Cell(a) => a.reshape(rows, cols, &self.cancel)?.into_array_value(),
                    Array::Struct(a) => {
                        Value::Container(a.reshape(rows, cols, &self.cancel)?.into())
                    }
                    _ => unreachable!(),
                })
            }
            "transpose" | "ctranspose" => self.cel_transpose(&a),
            "repmat" => {
                let (nr, nc) = shape_arguments::shape(&args[1..], None)?;
                let rows = r.checked_mul(nr).ok_or("Repetition shape overflow")?;
                let cols = c.checked_mul(nc).ok_or("Repetition shape overflow")?;
                Ok(self
                    .cel_gather(&a, rows, cols, |i| (i % rows) % r + ((i / rows) % c) * r)?
                    .into())
            }
            "flip" | "flipud" | "fliplr" => {
                let dim = if name == "flipud" {
                    1
                } else if name == "fliplr" {
                    2
                } else if args.len() == 2 {
                    self.cel_integer(&args[1])?
                } else if r == 1 {
                    2
                } else {
                    1
                };
                if dim < 1 {
                    return Err("Dimension argument must be a positive integer scalar.".into());
                }
                if dim > 2 {
                    return Ok(a.into());
                }
                Ok(self
                    .cel_gather(&a, r, c, |i| {
                        if dim == 1 {
                            i / r * r + r - 1 - i % r
                        } else {
                            (c - 1 - i / r) * r + i % r
                        }
                    })?
                    .into())
            }
            "rot90" => {
                let turns = if args.len() == 1 {
                    1
                } else {
                    self.cel_integer(&args[1])?.rem_euclid(4)
                };
                let (rows, cols) = if turns % 2 == 0 { (r, c) } else { (c, r) };
                Ok(self
                    .cel_gather(&a, rows, cols, |i| {
                        let (y, x) = (i % rows, i / rows);
                        match turns {
                            0 => i,
                            1 => x + (c - 1 - y) * r,
                            2 => a.len() - 1 - i,
                            _ => r - 1 - x + y * r,
                        }
                    })?
                    .into())
            }
            "circshift" => {
                let shifts = args[1].clone().matrix(&self.cancel)?;
                if shifts.len() > 2
                    || shifts.is_empty()
                    || (shifts.rows() != 1 && shifts.cols() != 1)
                {
                    return Err("Shift must be a scalar or two-element vector.".into());
                }
                let mut shift = [0i64; 2];
                if args.len() == 3 {
                    let dim = self.cel_integer(&args[2])?;
                    if dim < 1 || shifts.len() != 1 {
                        return Err("Dimension argument must be a positive integer scalar.".into());
                    }
                    let k = self.cel_integer(&args[1])?;
                    if dim <= 2 {
                        shift[dim as usize - 1] = k;
                    }
                } else {
                    for (i, &k) in shifts.data().iter().enumerate() {
                        shift[i] = self.cel_integer(&Value::Matrix(Matrix::scalar(k)))?;
                    }
                }
                let (dy, dx) = (
                    shift[0].rem_euclid(r.max(1) as i64) as usize,
                    shift[1].rem_euclid(c.max(1) as i64) as usize,
                );
                Ok(self
                    .cel_gather(&a, r, c, |i| {
                        (i % r + r - dy) % r + ((i / r + c - dx) % c) * r
                    })?
                    .into())
            }
            "permute" => {
                let order = args[1].clone().matrix(&self.cancel)?;
                if order.len() < 2 {
                    return Err("ORDER must have at least N elements for an N-D array.".into());
                }
                if order.len() > 32 || (order.rows() != 1 && order.cols() != 1) {
                    return Err("ORDER must be a vector.".into());
                }
                let mut seen = vec![false; order.len()];
                let mut axes = Vec::with_capacity(order.len());
                for &k in order.data() {
                    if !k.is_finite() || k.fract() != 0. || k < 1. || k > order.len() as f64 {
                        return Err("ORDER contains an invalid permutation index.".into());
                    }
                    let k = k as usize - 1;
                    if seen[k] {
                        return Err("ORDER cannot contain repeated permutation indices.".into());
                    }
                    seen[k] = true;
                    axes.push(k);
                }
                let size = |k| {
                    if k == 0 {
                        r
                    } else if k == 1 {
                        c
                    } else {
                        1
                    }
                };
                if axes[2..].iter().any(|&k| size(k) != 1) {
                    return Err("Only two-dimensional arrays are supported".into());
                }
                let (rows, cols) = (size(axes[0]), size(axes[1]));
                Ok(self
                    .cel_gather(&a, rows, cols, |i| {
                        let (y, x) = (i % rows, i / rows);
                        let row = if axes[0] == 0 {
                            y
                        } else if axes[1] == 0 {
                            x
                        } else {
                            0
                        };
                        let col = if axes[0] == 1 {
                            y
                        } else if axes[1] == 1 {
                            x
                        } else {
                            0
                        };
                        row + col * r
                    })?
                    .into())
            }
            _ => unreachable!(),
        }
    }
}

impl Engine {
    /// MATLAB promotes a mixed list to cells unless a struct precedes the first
    /// cell. Non-cell operands occupy one slot, regardless of their own shape.
    pub(super) fn cel_concatenate(&self, rows: Vec<Vec<Value>>) -> Result<Value> {
        let mut arrays = Vec::with_capacity(rows.len());
        for row in rows {
            arrays.push(
                row.into_iter()
                    .map(|v| v.array(&self.cancel))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let first_container = arrays
            .iter()
            .flatten()
            .find(|a| matches!(a, Array::Cell(_) | Array::Struct(_)));
        let cells = matches!(first_container, Some(Array::Cell(_)));
        let structs = matches!(first_container, Some(Array::Struct(_)));
        let fields = if structs {
            arrays
                .iter()
                .flatten()
                .find_map(|a| match a {
                    Array::Struct(s) if !s.is_empty() || s.field_count() != 0 => {
                        Some(s.field_names().to_vec())
                    }
                    _ => None,
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut blocks = Vec::with_capacity(arrays.len());
        for row in arrays {
            let mut parts = Vec::with_capacity(row.len());
            for a in row {
                self.check_cancel()?;
                if cells {
                    if matches!(a, Array::Cell(_)) {
                        parts.push(a);
                    } else if !a.is_empty() {
                        parts.push(
                            CellArray::from_shared(1, 1, vec![Arc::new(a)], &self.cancel)?.into(),
                        );
                    }
                } else if structs {
                    if let Array::Struct(s) = &a {
                        if s.is_empty() && s.field_count() == 0 {
                            continue;
                        }
                        let what = if s.field_count() != fields.len() {
                            Some("Number")
                        } else if fields.iter().any(|f| s.field_index(f).is_none()) {
                            Some("Names")
                        } else {
                            None
                        };
                        if let Some(what) = what {
                            return Err(format!(
                                "{what} of fields in structure arrays being concatenated do not match. Concatenation of structure arrays requires that these arrays have the same set of fields."
                            ));
                        }
                        parts.push(a);
                    } else if !a.is_empty() {
                        return Err(format!(
                            "The following error occurred converting from {} to struct:\nConversion to struct from {} is not possible.",
                            a.class(),
                            a.class()
                        ));
                    }
                } else {
                    parts.push(self.cel_char(&a)?.into());
                }
            }
            // Shaped empties are retained only when every block on this row is
            // empty. This also preserves cell(2,0) and char(zeros(0,3)).
            if parts.iter().any(|a| !a.is_empty()) {
                parts.retain(|a| !a.is_empty());
            } else {
                parts.retain(|a| (a.rows(), a.cols()) != (0, 0));
            }
            if !parts.is_empty() {
                blocks.push(parts);
            }
        }
        if blocks.iter().flatten().any(|a| !a.is_empty()) {
            blocks.retain(|row| row.iter().any(|a| !a.is_empty()));
        }
        let mut height = 0usize;
        let mut width = None;
        for row in &blocks {
            let r = row[0].rows();
            if row.iter().any(|a| a.rows() != r) {
                return Err(DIMENSIONS.into());
            }
            let c = row
                .iter()
                .try_fold(0usize, |n, a| n.checked_add(a.cols()))
                .ok_or("Concatenation shape overflow")?;
            if width.is_some_and(|w| w != c) {
                return Err(DIMENSIONS.into());
            }
            width = Some(c);
            height = height
                .checked_add(r)
                .ok_or("Concatenation shape overflow")?;
        }
        let width = width.unwrap_or(0);
        let n = self.cel_admit(height, width, fields.len().max(1))?;
        let empty = Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?));
        let mut slots = if cells || structs {
            vec![empty; n * if cells { 1 } else { fields.len() }]
        } else {
            Vec::new()
        };
        let mut units = if cells || structs {
            Vec::new()
        } else {
            vec![0; n]
        };
        let mut row_offset = 0;
        for row in blocks {
            let r = row[0].rows();
            let mut col_offset = 0;
            for a in row {
                let mapping = if let Array::Struct(s) = &a {
                    fields
                        .iter()
                        .map(|f| s.field_index(f).unwrap())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                for i in 0..a.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    let dest = row_offset + i % r + (col_offset + i / r) * height;
                    match &a {
                        Array::Cell(c) => slots[dest] = c.get_linear(i)?.clone(),
                        Array::Struct(s) => {
                            for (f, &source) in mapping.iter().enumerate() {
                                slots[dest * fields.len() + f] = s.get(source, i)?.clone();
                            }
                        }
                        Array::Character(c) => units[dest] = c.units()[i],
                        _ => unreachable!(),
                    }
                }
                col_offset += a.cols();
            }
            row_offset += r;
        }
        Ok(if cells {
            CellArray::from_shared(height, width, slots, &self.cancel)?.into_array_value()
        } else if structs {
            Value::Container(
                StructArray::from_slots(height, width, fields, slots, &self.cancel)?.into(),
            )
        } else {
            Value::Container(
                CharacterRow::from_column_major(height, width, units, &self.cancel)?.into(),
            )
        })
    }

    /// Concatenation/assignment truncate numeric code units; char() has its
    /// separate rounding contract and deliberately does not call this helper.
    fn cel_char(&self, a: &Array) -> Result<CharacterRow> {
        if let Array::Character(c) = a {
            return Ok(c.clone());
        }
        self.cel_admit(a.rows(), a.cols(), 1)?;
        let mut units = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let v = match a {
                Array::Numeric(m) => m.data()[i],
                Array::Logical(m) => f64::from(m.data()[i]),
                Array::Typed(m) => m.real(i),
                _ => {
                    return Err(format!(
                        "Conversion to char from {} is not possible.",
                        a.class()
                    ));
                }
            };
            units.push(v as u16);
        }
        CharacterRow::from_column_major(a.rows(), a.cols(), units, &self.cancel)
    }
}

impl Engine {
    pub(super) fn cel_subscript(
        &mut self,
        arg: &crate::syntax::Expr,
        bound: usize,
        maximum: usize,
        out: &mut Execution,
    ) -> Result<(super::indexing::Indices, (usize, usize))> {
        let result = self.subscript_with_limit(arg, Some(bound), crate::matrix::MAX_ELEMENTS, out);
        let (indices, shape) = result.map_err(|e| {
            if e.starts_with("Index must be") {
                self.raise(
                    "Subscript indices must either be real positive integers or logicals.".into(),
                    "MATLAB:badsubscript".into(),
                )
            } else {
                e
            }
        })?;
        // Colon, ranges, scalar indices and contiguous logical masks have
        // compact descriptions. Their extrema bound every index in O(1), just
        // as in numeric subscript admission; never walk a numeric payload colon.
        if matches!(
            indices,
            super::indexing::Indices::Stride { .. }
                | super::indexing::Indices::LogicalStride { .. }
        ) {
            if indices.len() > 0 && indices.at(0).max(indices.at(indices.len() - 1)) >= maximum {
                return Err(self.raise(
                    "Index exceeds matrix dimensions.".into(),
                    "MATLAB:badsubscript".into(),
                ));
            }
            return Ok((indices, shape));
        }
        for i in 0..indices.len() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            if indices.at(i) >= maximum {
                return Err(self.raise(
                    "Index exceeds matrix dimensions.".into(),
                    "MATLAB:badsubscript".into(),
                ));
            }
        }
        Ok((indices, shape))
    }

    pub(super) fn cel_assign(
        &mut self,
        old: Option<Array>,
        args: &[crate::syntax::Expr],
        content: bool,
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        if args.is_empty() || args.len() > 2 {
            return Err("Container indexing requires one or two subscripts".into());
        }
        let old = match old {
            Some(a) => a,
            None if content || matches!(value, Array::Cell(_)) => {
                CellArray::empty(0, 0, &self.cancel)?.into()
            }
            None => return Err("Undefined container variable".into()),
        };
        let old = match (&old, &value) {
            (Array::Struct(a), Array::Struct(b))
                if !content && !deletion && a.is_empty() && a.field_count() == 0 =>
            {
                StructArray::filled_empty(
                    a.rows(),
                    a.cols(),
                    b.field_names().to_vec(),
                    &self.cancel,
                )?
                .into()
            }
            _ => old,
        };
        let (r, c) = (old.rows(), old.cols());
        let fields = if let Array::Struct(s) = &old {
            s.field_count()
        } else {
            1
        };
        let (first, _) = self.cel_subscript(
            &args[0],
            if args.len() == 1 { old.len() } else { r },
            MAX,
            out,
        )?;
        let second = if args.len() == 2 {
            Some(self.cel_subscript(&args[1], c, MAX, out)?.0)
        } else {
            None
        };
        let extent = |indices: &super::indexing::Indices| {
            (0..indices.len())
                .map(|i| indices.at(i) + 1)
                .max()
                .unwrap_or(0)
        };
        let (er, ec) = (extent(&first), second.as_ref().map_or(0, extent));
        if deletion && !content {
            if (args.len() == 1 && er > old.len()) || (args.len() == 2 && (er > r || ec > c)) {
                return Err("Matrix index is out of range for deletion.".into());
            }
            if args.len() == 2 && !args.iter().any(|arg| self.colon_subscript(arg)) {
                return Err("A null assignment can have only one non-colon index.".into());
            }
            let (n, selected, rows_removed) = if let Some(second) = &second {
                if self.colon_subscript(&args[0]) {
                    (c, second, false)
                } else {
                    (r, &first, true)
                }
            } else {
                (old.len(), &first, false)
            };
            let mut removed = vec![false; n];
            for i in 0..selected.len() {
                removed[selected.at(i)] = true;
            }
            let kept: Vec<_> = (0..n).filter(|&i| !removed[i]).collect();
            let shape = if args.len() == 1 {
                if self.colon_subscript(&args[0]) {
                    (0, 0)
                } else if c == 1 && r != 1 {
                    (kept.len(), 1)
                } else {
                    (1, kept.len())
                }
            } else if rows_removed {
                (kept.len(), c)
            } else {
                (r, kept.len())
            };
            return self.cel_gather(&old, shape.0, shape.1, |i| {
                if args.len() == 1 {
                    kept[i]
                } else if rows_removed {
                    kept[i % shape.0] + i / shape.0 * r
                } else {
                    i % r + kept[i / r] * r
                }
            });
        }
        let count = first
            .len()
            .checked_mul(second.as_ref().map_or(1, super::indexing::Indices::len))
            .ok_or("Assignment shape overflow")?;
        self.cel_admit(count, 1, fields)?;
        if content && count != 1 {
            return Err(format!(
                "Expected one output from a curly brace or dot indexing expression, but there were {count} results."
            ));
        }
        let replacement = match (&old, content, &value) {
            (Array::Cell(_), true, _) => {
                CellArray::from_shared(1, 1, vec![Arc::new(value)], &self.cancel)?.into()
            }
            (Array::Cell(_), false, Array::Cell(_)) => value,
            (Array::Cell(_), false, _) => {
                return Err(format!(
                    "Conversion to cell from {} is not possible.",
                    value.class()
                ));
            }
            (Array::Character(_), false, _) => self.cel_char(&value)?.into(),
            (Array::Struct(a), false, Array::Struct(b)) => {
                if a.field_count() != b.field_count()
                    || a.field_names().iter().any(|f| b.field_index(f).is_none())
                {
                    return Err(self.raise(
                        "Subscripted assignment between dissimilar structures.".into(),
                        "MATLAB:heterogeneousStrucAssignment".into(),
                    ));
                }
                value
            }
            (Array::Struct(_), false, _) => {
                return Err("Assignment between unlike types is not allowed.".into());
            }
            _ => return Err("Curly assignment requires a cell array".into()),
        };
        if replacement.len() != 1 && replacement.len() != count {
            return Err(if args.len() == 1 {
                "In an assignment  A(:) = B, the number of elements in A and B must be the same."
            } else {
                "Subscripted assignment dimension mismatch."
            }
            .into());
        }
        if count == 0 {
            return Ok(old);
        }
        if replacement.len() != 1
            && let Some(columns) = &second
        {
            let selected: Vec<_> = [first.len(), columns.len()]
                .into_iter()
                .filter(|&n| n != 1)
                .collect();
            let source: Vec<_> = [replacement.rows(), replacement.cols()]
                .into_iter()
                .filter(|&n| n != 1)
                .collect();
            if selected != source {
                return Err("Subscripted assignment dimension mismatch.".into());
            }
        }
        let (rows, cols) = if second.is_some() {
            (r.max(er), c.max(ec))
        } else if er <= old.len() {
            (r, c)
        } else if r == 0 && c == 1 && !content {
            // Reference behavior: c=cell(0,1); c(3)={1} is 3x1, but c{3}=1 is 1x3 (checked 2026-10-24).
            (er, 1)
        } else if r == 1 || old.is_empty() {
            (1, er)
        } else if c == 1 || content {
            (er, 1)
        } else {
            return Err("Attempt to grow array along ambiguous dimension.".into());
        };
        let n = self.cel_admit(rows, cols, fields)?;
        // None denotes the canonical growth fill ([] fields/cells, char(0)).
        let mut origins = vec![None; n];
        for i in 0..old.len() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let dest = if second.is_none() {
                i
            } else {
                i % r + i / r * rows
            };
            origins[dest] = Some((false, i));
        }
        for i in 0..count {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let dest = if let Some(second) = &second {
                first.at(i % first.len()) + second.at(i / first.len()) * rows
            } else {
                first.at(i)
            };
            origins[dest] = Some((true, if replacement.len() == 1 { 0 } else { i }));
        }
        match &old {
            Array::Character(_) => {
                let mut units = Vec::with_capacity(n);
                for (i, origin) in origins.into_iter().enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    units.push(if let Some((new, index)) = origin {
                        let Array::Character(a) = (if new { &replacement } else { &old }) else {
                            unreachable!()
                        };
                        a.units()[index]
                    } else {
                        0
                    });
                }
                Ok(CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into())
            }
            Array::Cell(_) | Array::Struct(_) => {
                let empty = Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?));
                let field_order: Vec<_> = match (&old, &replacement) {
                    (Array::Struct(a), Array::Struct(b)) => a
                        .field_names()
                        .iter()
                        .map(|f| b.field_index(f).unwrap())
                        .collect(),
                    _ => vec![0],
                };
                let mut slots = Vec::with_capacity(n * fields);
                for (i, origin) in origins.into_iter().enumerate() {
                    if i.is_multiple_of(256) {
                        self.check_cancel()?;
                    }
                    for (f, &replacement_field) in field_order.iter().enumerate() {
                        slots.push(if let Some((new, index)) = origin {
                            match if new { &replacement } else { &old } {
                                Array::Cell(a) => a.get_linear(index)?.clone(),
                                Array::Struct(a) => a
                                    .get(if new { replacement_field } else { f }, index)?
                                    .clone(),
                                _ => unreachable!(),
                            }
                        } else {
                            empty.clone()
                        });
                    }
                }
                if let Array::Struct(s) = &old {
                    Ok(StructArray::from_slots(
                        rows,
                        cols,
                        s.field_names().to_vec(),
                        slots,
                        &self.cancel,
                    )?
                    .into())
                } else {
                    Ok(CellArray::from_shared(rows, cols, slots, &self.cancel)?.into())
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Text keys borrow shared UTF-16 storage. Even a cell array containing the
/// same large string thousands of times never duplicates that string here.
#[derive(Clone, Debug)]
enum CelKey {
    Text(CharacterRow),
    Unit(u16),
}
impl CelKey {
    fn cel_units(&self) -> &[u16] {
        match self {
            Self::Text(c) => c.units(),
            Self::Unit(u) => std::slice::from_ref(u),
        }
    }
}
impl PartialEq for CelKey {
    fn eq(&self, other: &Self) -> bool {
        self.cel_units() == other.cel_units()
    }
}
impl Eq for CelKey {}
impl PartialOrd for CelKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CelKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cel_units().cmp(other.cel_units())
    }
}

#[derive(Default)]
struct CelOptions {
    rows: bool,
    stable: bool,
    last: bool,
    legacy: bool,
}
struct CelText {
    keys: Vec<CelKey>,
    shape: (usize, usize),
    cells: bool,
    width: usize,
}

impl Engine {
    fn cel_text(&self, a: &Array, cells: bool, rows: bool, error: &str) -> Result<CelText> {
        self.cel_admit(a.rows(), a.cols(), 1)?;
        let mut shape = (a.rows(), a.cols());
        let mut keys = Vec::with_capacity(if rows { a.rows() } else { a.len() });
        match a {
            Array::Cell(c) => {
                for i in 0..c.len() {
                    if i.is_multiple_of(256) {
                        self.check_cancel()?;
                    }
                    match c.get_linear(i)?.as_ref() {
                        Array::Character(s) if s.rows() <= 1 => keys.push(CelKey::Text(s.clone())),
                        _ => return Err(error.into()),
                    }
                }
            }
            Array::Character(c) if cells => {
                if c.rows() > 1 {
                    return Err(error.into());
                }
                keys.push(CelKey::Text(c.clone()));
                shape = (1, 1);
            }
            Array::Character(c) if rows => {
                for r in 0..c.rows() {
                    self.check_cancel()?;
                    let units = (0..c.cols())
                        .map(|col| c.units()[r + col * c.rows()])
                        .collect();
                    keys.push(CelKey::Text(CharacterRow::from_column_major(
                        1,
                        c.cols(),
                        units,
                        &self.cancel,
                    )?));
                }
                shape = (c.rows(), 1);
            }
            Array::Character(c) => {
                for (i, &unit) in c.units().iter().enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    keys.push(CelKey::Unit(unit));
                }
            }
            _ => return Err(error.into()),
        }
        Ok(CelText {
            keys,
            shape,
            cells,
            width: a.cols(),
        })
    }

    fn cel_options(&self, args: &[Value]) -> Result<CelOptions> {
        let mut options = CelOptions::default();
        let mut sorted = false;
        let mut first = false;
        for arg in args {
            match self.text_value(arg.clone())?.to_ascii_lowercase().as_str() {
                "rows" => options.rows = true,
                "stable" => options.stable = true,
                "sorted" => sorted = true,
                "first" => first = true,
                "last" => options.last = true,
                "legacy" => options.legacy = true,
                _ => return Err("Unknown option.".into()),
            }
        }
        if sorted && options.stable {
            return Err("You can specify either 'sorted' or 'stable' but not both.".into());
        }
        if first && options.last {
            return Err("You can specify either 'first' or 'last' but not both.".into());
        }
        if options.legacy && options.stable {
            return Err("'stable' and 'legacy' cannot be combined.".into());
        }
        options.last |= options.legacy && !first;
        Ok(options)
    }

    fn cel_flags(&self, args: &[Value], allowed: &[&str]) -> Result<()> {
        for arg in args {
            if !allowed.contains(&self.text_value(arg.clone())?.to_ascii_lowercase().as_str()) {
                return Err("Unknown option.".into());
            }
        }
        Ok(())
    }

    fn cel_text_result(
        &self,
        keys: &[CelKey],
        rows: usize,
        cols: usize,
        cells: bool,
        row_keys: bool,
    ) -> Result<Value> {
        let n = self.cel_admit(rows, cols, 1)?;
        if cells {
            let mut slots = Vec::with_capacity(keys.len());
            for (i, key) in keys.iter().enumerate() {
                if i.is_multiple_of(256) {
                    self.check_cancel()?;
                }
                let CelKey::Text(text) = key else {
                    unreachable!()
                };
                slots.push(Arc::new(Array::Character(text.clone())));
            }
            return Ok(CellArray::from_shared(rows, cols, slots, &self.cancel)?.into_array_value());
        }
        let mut units = Vec::with_capacity(n);
        if row_keys {
            for c in 0..cols {
                for key in keys {
                    if units.len().is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    units.push(key.cel_units().get(c).copied().unwrap_or(u16::from(b' ')));
                }
            }
        } else {
            for (i, key) in keys.iter().enumerate() {
                if i.is_multiple_of(1024) {
                    self.check_cancel()?;
                }
                units.push(key.cel_units()[0]);
            }
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
        ))
    }

    fn cel_index_vector(
        &self,
        indices: impl IntoIterator<Item = usize>,
        row: bool,
    ) -> Result<Value> {
        let mut data = Vec::new();
        for index in indices {
            if data.len().is_multiple_of(1024) {
                self.check_cancel()?;
            }
            data.push((index + 1) as f64);
        }
        let n = data.len();
        Ok(Value::Matrix(Matrix::from_column_vec(
            if row { 1 } else { n },
            if row { n } else { 1 },
            data,
        )?))
    }

    // A stable merge over slot indices permits cancellation between comparisons;
    // a comparison may read at most two bounded UTF-16 strings. No nested pool.
    fn cel_order(
        &self,
        indices: &mut [usize],
        compare: impl Fn(usize, usize) -> Result<std::cmp::Ordering>,
    ) -> Result<()> {
        let mut scratch = vec![0; indices.len()];
        let mut width = 1;
        while width < indices.len() {
            for start in (0..indices.len()).step_by(width * 2) {
                let mid = (start + width).min(indices.len());
                let end = (mid + width).min(indices.len());
                let (mut a, mut b) = (start, mid);
                for (offset, dst) in scratch[start..end].iter_mut().enumerate() {
                    if offset.is_multiple_of(128) {
                        self.check_cancel()?;
                    }
                    if a < mid && (b == end || !compare(indices[a], indices[b])?.is_gt()) {
                        *dst = indices[a];
                        a += 1;
                    } else {
                        *dst = indices[b];
                        b += 1;
                    }
                }
            }
            indices.copy_from_slice(&scratch);
            width *= 2;
        }
        self.check_cancel()
    }

    pub(super) fn cel_sort(
        &self,
        args: &[Value],
        rows_mode: bool,
        want_indices: bool,
    ) -> Result<(Value, Option<Matrix>)> {
        self.check_cancel()?;
        let a = args[0].clone().array(&self.cancel)?;
        let cells = matches!(a, Array::Cell(_));
        if !rows_mode && cells && args.len() != 1 {
            return Err("DIM and MODE arguments not supported for cell arrays.".into());
        }
        let text = self.cel_text(
            &a,
            cells,
            false,
            "Input argument must be a cell array of strings.",
        )?;
        self.preflight_elements(a.len().saturating_mul(4))?;
        let (r, c) = (a.rows(), a.cols());
        let mut indices: Vec<_> = (0..a.len()).collect();
        let mut permutation;
        let mut dim = if r == 1 { 2 } else { 1 };
        let mut descending = false;
        if rows_mode {
            if args.len() > 2 {
                return Err("Too many input arguments.".into());
            }
            let columns: Vec<i64> = if args.len() == 2 {
                if args[1].is_character() {
                    return Err("COL must be numeric.".into());
                }
                let cols = args[1].clone().matrix(&self.cancel)?;
                let mut columns = Vec::with_capacity(cols.len());
                for &k in cols.data() {
                    if !k.is_finite() || k == 0. || k.fract() != 0. || k.abs() > c as f64 {
                        return Err("COL must be a vector of column indices.".into());
                    }
                    columns.push(k as i64);
                }
                columns
            } else {
                (1..=c as i64).collect()
            };
            permutation = (0..r).collect::<Vec<_>>();
            self.cel_order(&mut permutation, |i, j| {
                for (key_index, &column) in columns.iter().enumerate() {
                    if key_index.is_multiple_of(64) {
                        self.check_cancel()?;
                    }
                    let col = column.unsigned_abs() as usize - 1;
                    let order = text.keys[i + col * r].cmp(&text.keys[j + col * r]);
                    if !order.is_eq() {
                        return Ok(if column < 0 { order.reverse() } else { order });
                    }
                }
                Ok(std::cmp::Ordering::Equal)
            })?;
            for col in 0..c {
                for row in 0..r {
                    indices[row + col * r] = permutation[row] + col * r;
                }
            }
        } else {
            let mut at = 1;
            if let Some(arg) = args.get(at)
                && !arg.is_character()
            {
                dim = self.cel_integer(arg)?;
                at += 1;
            }
            if dim < 1 {
                return Err("Dimension argument must be a positive integer scalar.".into());
            }
            if let Some(arg) = args.get(at) {
                descending = match self.text_value(arg.clone())?.as_str() {
                    "ascend" => false,
                    "descend" => true,
                    _ => return Err("Unknown sort mode.".into()),
                };
                at += 1;
            }
            if at != args.len() {
                return Err("Too many input arguments.".into());
            }
            let (lanes, length) = if dim == 1 {
                (c, r)
            } else if dim == 2 {
                (r, c)
            } else {
                (a.len(), 1)
            };
            let mut lane = vec![0; length];
            for l in 0..lanes {
                self.check_cancel()?;
                for (i, k) in lane.iter_mut().enumerate() {
                    *k = if dim == 1 {
                        i + l * r
                    } else if dim == 2 {
                        l + i * r
                    } else {
                        l
                    };
                }
                self.cel_order(&mut lane, |i, j| {
                    let order = text.keys[i].cmp(&text.keys[j]);
                    Ok(if descending { order.reverse() } else { order })
                })?;
                for (i, &k) in lane.iter().enumerate() {
                    let dest = if dim == 1 {
                        i + l * r
                    } else if dim == 2 {
                        l + i * r
                    } else {
                        l
                    };
                    indices[dest] = k;
                }
            }
            permutation = Vec::new();
        }
        let sorted = self.cel_gather(&a, r, c, |i| indices[i])?;
        let index = if want_indices {
            let (ir, ic, data) = if rows_mode {
                (r, 1, permutation.iter().map(|&i| (i + 1) as f64).collect())
            } else {
                (
                    r,
                    c,
                    indices
                        .iter()
                        .map(|&i| {
                            if dim == 1 {
                                (i % r + 1) as f64
                            } else if dim == 2 {
                                (i / r + 1) as f64
                            } else {
                                1.
                            }
                        })
                        .collect(),
                )
            };
            Some(Matrix::from_column_vec(ir, ic, data)?)
        } else {
            None
        };
        Ok((sorted.into(), index))
    }

    pub(super) fn cel_unique(&self, args: &[Value], outputs: [bool; 3]) -> Result<Vec<Value>> {
        let a = args[0].clone().array(&self.cancel)?;
        let cells = matches!(a, Array::Cell(_));
        let options = self.cel_options(&args[1..])?;
        let rows_mode = options.rows && !cells;
        let text = self.cel_text(
            &a,
            cells,
            rows_mode,
            "Input A must be a cell array of strings.",
        )?;
        self.preflight_elements(text.keys.len().saturating_mul(8))?;
        let mut groups: std::collections::BTreeMap<CelKey, usize> =
            std::collections::BTreeMap::new();
        for (i, key) in text.keys.iter().enumerate() {
            if i.is_multiple_of(128) {
                self.check_cancel()?;
            }
            let entry = groups.entry(key.clone()).or_insert(i);
            if options.last {
                *entry = i;
            }
        }
        let mut selected: Vec<_> = groups.into_iter().collect();
        if options.stable {
            selected.sort_by_key(|(_, i)| *i);
        }
        let n = selected.len();
        let row = !rows_mode && a.rows() == 1;
        let (r, c) = if rows_mode {
            (n, a.cols())
        } else if row {
            (1, n)
        } else {
            (n, 1)
        };
        let empty = || Value::Matrix(Matrix::filled(0, 0, 0.).unwrap());
        let values = if outputs[0] {
            self.cel_text_result(
                &selected
                    .iter()
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>(),
                r,
                c,
                cells,
                rows_mode,
            )?
        } else {
            empty()
        };
        let ia = if outputs[1] {
            self.cel_index_vector(selected.iter().map(|(_, i)| *i), options.legacy && row)?
        } else {
            empty()
        };
        let ic = if outputs[2] {
            let mut lookup = std::collections::BTreeMap::new();
            for (i, (key, _)) in selected.iter().enumerate() {
                lookup.insert(key, i);
            }
            let mut indices = Vec::with_capacity(text.keys.len());
            for (i, key) in text.keys.iter().enumerate() {
                if i.is_multiple_of(128) {
                    self.check_cancel()?;
                }
                indices.push(lookup[key]);
            }
            self.cel_index_vector(indices, options.legacy && row)?
        } else {
            empty()
        };
        Ok(vec![values, ia, ic])
    }

    fn cel_pair(&self, args: &[Value], rows: bool) -> Result<(CelText, CelText)> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let a = args[0].clone().array(&self.cancel)?;
        let b = args[1].clone().array(&self.cancel)?;
        let cells = matches!(a, Array::Cell(_)) || matches!(b, Array::Cell(_));
        let error = format!(
            "Input A of class {} and input B of class {} must be cell arrays of strings, unless one is a string.",
            a.class(),
            b.class()
        );
        if rows
            && matches!((&a, &b), (Array::Character(_), Array::Character(_)))
            && a.cols() != b.cols()
        {
            return Err("A and B must have the same number of columns.".into());
        }
        let a = self.cel_text(&a, cells, rows && !cells, &error)?;
        let b = self.cel_text(&b, cells, rows && !cells, &error)?;
        self.preflight_elements((a.keys.len() + b.keys.len()).saturating_mul(8))?;
        Ok((a, b))
    }

    pub(super) fn cel_ismember(&self, args: &[Value], outputs: [bool; 2]) -> Result<Vec<Value>> {
        self.cel_flags(args.get(2..).unwrap_or_default(), &["rows", "legacy"])?;
        let options = self.cel_options(args.get(2..).unwrap_or_default())?;
        let (a, b) = self.cel_pair(args, options.rows)?;
        let mut lookup = std::collections::BTreeMap::new();
        for (i, key) in b.keys.iter().enumerate() {
            if i.is_multiple_of(128) {
                self.check_cancel()?;
            }
            let index = lookup.entry(key).or_insert(i);
            if options.legacy {
                *index = i;
            }
        }
        let mut flags = Vec::new();
        let mut locations = Vec::new();
        for (i, key) in a.keys.iter().enumerate() {
            if i.is_multiple_of(128) {
                self.check_cancel()?;
            }
            let found = lookup.get(key);
            if outputs[0] {
                flags.push(u8::from(found.is_some()));
            }
            if outputs[1] {
                locations.push(found.map_or(0., |&i| (i + 1) as f64));
            }
        }
        let empty = || Value::Matrix(Matrix::filled(0, 0, 0.).unwrap());
        Ok(vec![
            if outputs[0] {
                Value::Logical(crate::LogicalMatrix::from_column_vec(
                    a.shape.0,
                    a.shape.1,
                    flags,
                    &self.cancel,
                )?)
            } else {
                empty()
            },
            if outputs[1] {
                Value::Matrix(Matrix::from_column_vec(a.shape.0, a.shape.1, locations)?)
            } else {
                empty()
            },
        ])
    }

    pub(super) fn cel_set(
        &self,
        operation: crate::array::SetOperation,
        args: &[Value],
        outputs: [bool; 3],
    ) -> Result<Vec<Value>> {
        use crate::array::SetOperation;
        self.cel_flags(
            args.get(2..).unwrap_or_default(),
            &["rows", "sorted", "stable", "legacy"],
        )?;
        let options = self.cel_options(args.get(2..).unwrap_or_default())?;
        let (a, b) = self.cel_pair(args, options.rows)?;
        let rows_mode = options.rows && !a.cells;
        let mut groups: std::collections::BTreeMap<CelKey, (Option<usize>, Option<usize>)> =
            std::collections::BTreeMap::new();
        for (side, input) in [&a, &b].into_iter().enumerate() {
            for (i, key) in input.keys.iter().enumerate() {
                if i.is_multiple_of(128) {
                    self.check_cancel()?;
                }
                let pair = groups.entry(key.clone()).or_default();
                let index = if side == 0 { &mut pair.0 } else { &mut pair.1 };
                if index.is_none() || options.last {
                    *index = Some(i);
                }
            }
        }
        let mut selected = Vec::with_capacity(groups.len());
        for (i, (key, (ia, ib))) in groups.into_iter().enumerate() {
            if i.is_multiple_of(128) {
                self.check_cancel()?;
            }
            let keep = match operation {
                SetOperation::Union => true,
                SetOperation::Intersect => ia.is_some() && ib.is_some(),
                SetOperation::Difference => ia.is_some() && ib.is_none(),
                SetOperation::SymmetricDifference => ia.is_some() != ib.is_some(),
            };
            if keep {
                selected.push((
                    key,
                    if operation == SetOperation::Union && options.legacy && ib.is_some() {
                        None
                    } else {
                        ia
                    },
                    if operation == SetOperation::Union && ia.is_some() && !options.legacy {
                        None
                    } else {
                        ib
                    },
                ));
            }
        }
        if options.stable {
            selected.sort_by_key(|(_, ia, ib)| ia.unwrap_or_else(|| a.keys.len() + ib.unwrap()));
        }
        let n = selected.len();
        let row = !rows_mode
            && a.shape.0 == 1
            && (operation == SetOperation::Difference || b.shape.0 == 1);
        let (r, c) = if rows_mode {
            (n, a.width.max(b.width))
        } else if row {
            (1, n)
        } else {
            (n, 1)
        };
        let empty = || Value::Matrix(Matrix::filled(0, 0, 0.).unwrap());
        let values = if outputs[0] {
            self.cel_text_result(
                &selected
                    .iter()
                    .map(|(key, _, _)| key.clone())
                    .collect::<Vec<_>>(),
                r,
                c,
                a.cells,
                rows_mode,
            )?
        } else {
            empty()
        };
        let ia = if outputs[1] {
            self.cel_index_vector(
                selected.iter().filter_map(|(_, ia, _)| *ia),
                options.legacy && row,
            )?
        } else {
            empty()
        };
        let ib = if outputs[2] {
            self.cel_index_vector(
                selected.iter().filter_map(|(_, _, ib)| *ib),
                options.legacy && row,
            )?
        } else {
            empty()
        };
        Ok(vec![values, ia, ib])
    }
}

impl Engine {
    pub(super) fn cel_assign_child(
        &mut self,
        current: Option<Array>,
        args: &[crate::syntax::Expr],
        rest: &[crate::syntax::PathStep],
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        let old = current.ok_or("Cell contents reference from a non-cell array object")?;
        let Array::Cell(c) = &old else {
            return Err("Cell contents reference from a non-cell array object".into());
        };
        let (_, positions) = self.container_positions(&old, args, out)?;
        if positions.len() != 1 {
            return Err(format!(
                "Expected one output from a curly brace or dot indexing expression, but there were {} results.",
                positions.len()
            ));
        }
        let child = c.get_linear(positions[0])?.as_ref().clone();
        let child = self.assign_step(Some(child), rest, value, deletion, out)?;
        let mut slots = Vec::with_capacity(c.len());
        for i in 0..c.len() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            slots.push(c.get_linear(i)?.clone());
        }
        slots[positions[0]] = Arc::new(child);
        Ok(CellArray::from_shared(c.rows(), c.cols(), slots, &self.cancel)?.into())
    }
}

impl Engine {
    /// Evaluate each subscript once and stage all writes before publishing.
    pub(super) fn cel_assign_outputs(
        &mut self,
        destinations: &[Option<crate::syntax::Expr>],
        rhs: &crate::syntax::Expr,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        use crate::syntax::{Expr, PathStep};
        let mut resolved = Vec::new();
        let mut targets = Vec::new();
        for target in destinations {
            let target = match target {
                Some(Expr::Name(name)) => CelOutputTarget::Variable(name),
                Some(Expr::Call(name, args)) => CelOutputTarget::Paren(name, args),
                Some(Expr::Field(base, field)) => {
                    let (name, args) = match base.as_ref() {
                        Expr::Name(name) => (name, None),
                        Expr::Call(name, args) => (name, Some(args)),
                        _ => return Err("Comma-separated field assignment requires a struct variable.".into()),
                    };
                    let Some(old @ Array::Struct(_)) = self.array(name) else {
                        return Err("Attempt to reference field of non-structure array".into());
                    };
                    let positions = if let Some(args) = args {
                        self.container_positions(&old, args, out)?.1
                    } else {
                        (0..old.len()).collect()
                    };
                    CelOutputTarget::Slots(name, Some(field), positions)
                }
                Some(Expr::CellContents(name, args)) => {
                    let old = self.array(name).ok_or("Undefined cell variable")?;
                    if !matches!(old, Array::Cell(_)) {
                        return Err("Cell contents reference from a non-cell array object".into());
                    }
                    CelOutputTarget::Slots(name, None, self.container_positions(&old, args, out)?.1)
                }
                None => CelOutputTarget::Ignore,
                _ => return Err("Only direct struct fields or cell contents are supported in a comma-separated assignment.".into()),
            };
            let count = match &target {
                CelOutputTarget::Slots(_, _, positions) => positions.len(),
                _ => 1,
            };
            if targets.len() + count > 32 {
                return Err("At most 32 outputs can be assigned in one statement".into());
            }
            let requested = (!matches!(target, CelOutputTarget::Ignore)).then(String::new);
            targets.extend(std::iter::repeat_n(requested, count));
            resolved.push(target);
        }
        let values = self.eval_outputs(rhs, &targets, out)?;
        if values.len() != targets.len() {
            return Err("Output count must match the comma-separated assignment.".into());
        }
        let mut values = values.into_iter();
        let mut updates = std::collections::BTreeMap::new();
        let mut names = Vec::new();
        for target in resolved {
            let name = match &target {
                CelOutputTarget::Variable(name)
                | CelOutputTarget::Paren(name, _)
                | CelOutputTarget::Slots(name, _, _) => *name,
                CelOutputTarget::Ignore => {
                    values.next();
                    continue;
                }
            };
            Self::validate_workspace_name(name)?;
            let old = updates.get(name).cloned().or_else(|| self.array(name));
            let updated = match target {
                CelOutputTarget::Variable(_) => self.assigned_array(values.next().unwrap())?,
                CelOutputTarget::Paren(_, args) => {
                    let value = self.assigned_array(values.next().unwrap())?;
                    self.assign_step(old, &[PathStep::Paren(args.to_vec())], value, false, out)?
                }
                CelOutputTarget::Slots(_, field, positions) => {
                    let values = values
                        .by_ref()
                        .take(positions.len())
                        .map(|v| v.array(&self.cancel))
                        .collect::<Result<Vec<_>>>()?;
                    self.cel_assign_output_slots(
                        old.ok_or("Undefined container variable")?,
                        field,
                        positions,
                        values,
                    )?
                }
                CelOutputTarget::Ignore => unreachable!(),
            };
            if !updates.contains_key(name) {
                names.push(name);
            }
            updates.insert(name.to_owned(), updated);
        }
        if names.len() == 1 {
            let name = names[0];
            return self.assign_workspace_array(name, updates.remove(name).unwrap(), quiet, out);
        }
        self.merge_workspace(updates)?;
        if !quiet {
            for name in names {
                let text = self.display_text(name, &self.array(name).unwrap())?;
                self.emit(out, text)?;
            }
        }
        Ok(())
    }

    fn cel_assign_output_slots(
        &mut self,
        old: Array,
        field: Option<&str>,
        positions: Vec<usize>,
        values: Vec<Array>,
    ) -> Result<Array> {
        if positions.iter().any(|&position| position >= old.len()) {
            return Err("Index exceeds matrix dimensions.".into());
        }
        if let Some(field) = field {
            let Array::Struct(s) = &old else {
                return Err("Attempt to reference field of non-structure array".into());
            };
            let mut names = s.field_names().to_vec();
            let f = if let Some(f) = s.field_index(field) {
                f
            } else {
                names.push(field.into());
                names.len() - 1
            };
            self.cel_admit(s.rows(), s.cols(), names.len())?;
            let mut slots = Vec::with_capacity(s.len() * names.len());
            let empty = Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?));
            for i in 0..s.len() {
                self.check_cancel()?;
                for j in 0..names.len() {
                    slots.push(if j < s.field_count() {
                        s.get(j, i)?.clone()
                    } else {
                        empty.clone()
                    });
                }
            }
            for (position, value) in positions.into_iter().zip(values) {
                slots[position * names.len() + f] = Arc::new(value);
            }
            Ok(Array::Struct(StructArray::from_slots(
                s.rows(),
                s.cols(),
                names,
                slots,
                &self.cancel,
            )?))
        } else {
            let Array::Cell(c) = &old else {
                return Err("Cell contents reference from a non-cell array object".into());
            };
            let mut slots = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                if i.is_multiple_of(1024) {
                    self.check_cancel()?;
                }
                slots.push(c.get_linear(i)?.clone());
            }
            for (position, value) in positions.into_iter().zip(values) {
                slots[position] = Arc::new(value);
            }
            Ok(Array::Cell(CellArray::from_shared(
                c.rows(),
                c.cols(),
                slots,
                &self.cancel,
            )?))
        }
    }
}

impl Engine {
    pub(super) fn cel_unique_outputs(
        &mut self,
        args: &[Value],
        outputs: [bool; 3],
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if matches!(args.first(), Some(Value::Complex(_))) {
            return self.cxr_unique(args, outputs);
        }
        let values = self.unique_call(args, outputs)?;
        self.cel_rows_warning(args, 1, out)?;
        Ok(values)
    }

    pub(super) fn cel_rows_warning(
        &mut self,
        args: &[Value],
        inputs: usize,
        out: &mut Execution,
    ) -> Result<()> {
        if args
            .iter()
            .take(inputs)
            .any(|v| matches!(v, Value::Container(Array::Cell(_))))
            && self
                .cel_options(args.get(inputs..).unwrap_or_default())?
                .rows
        {
            self.lang_utils_call(
                "warning",
                &[Value::Text(
                    "The 'rows' input is not supported for cell array inputs.".into(),
                )],
                out,
            )?;
        }
        Ok(())
    }

    pub(super) fn cel_concatenate_warn(
        &mut self,
        rows: Vec<Vec<Value>>,
        out: &mut Execution,
    ) -> Result<Value> {
        let mut warn = false;
        if rows
            .iter()
            .flatten()
            .any(|v| matches!(v, Value::Container(Array::Cell(_))))
        {
            for row in &rows {
                let mut height = None;
                for value in row {
                    let (r, c) = value.shape(&self.cancel)?;
                    if r != 0 && c != 0 {
                        height = Some(if matches!(value, Value::Container(Array::Cell(_))) {
                            r
                        } else {
                            1
                        });
                        break;
                    }
                }
                if let Some(height) = height {
                    warn |= row.iter().any(|v| matches!(v,Value::Container(Array::Cell(c)) if c.is_empty() && (c.rows(),c.cols()) != (0,0) && c.rows() != height));
                }
            }
        }
        let value = self.concatenate_values(rows)?;
        if warn {
            self.lang_utils_call("warning", &[Value::Text("This concatenation operation includes an empty array with an incorrect number of rows.\nConcatenation including empty arrays will require all arrays to have the same number of rows in a future\nrelease.".into())],out)?;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cel_order_observes_cancellation_during_comparisons() {
        let e = Engine::default();
        let mut indices: Vec<_> = (0..32768).rev().collect();
        let comparisons = std::cell::Cell::new(0usize);
        let result = e.cel_order(&mut indices, |a, b| {
            let n = comparisons.get() + 1;
            comparisons.set(n);
            if n == 256 {
                e.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(a.cmp(&b))
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(comparisons.get() <= 384);
    }
}
