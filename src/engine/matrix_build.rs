//! Matrix-building functions: concatenation, rotation, index conversion, the
//! classic test matrices, accumarray, isequal, running extrema and the
//! floating-point constants. Private helpers carry the `matbuild_` prefix.
use super::{Engine, Value};
use crate::{Array, CellArray, Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result};
use std::sync::Arc;

mod generators;

const NAMES: &[&str] = &[
    "cat",
    "horzcat",
    "vertcat",
    "rot90",
    "squeeze",
    "sub2ind",
    "ind2sub",
    "magic",
    "hilb",
    "invhilb",
    "pascal",
    "toeplitz",
    "vander",
    "hadamard",
    "compan",
    "rosser",
    "wilkinson",
    "randperm",
    "accumarray",
    "isequal",
    "cummax",
    "cummin",
    "colon",
    "flintmax",
    "realmax",
    "realmin",
];
const NOT_ENOUGH: &str = "Not enough input arguments.";
const TOO_MANY: &str = "Too many input arguments.";
const OUT_OF_RANGE: &str = "Out of range subscript.";
const INCONSISTENT: &str = "Dimensions of matrices being concatenated are not consistent.";
const SIZE_VECTOR: &str = "Size vector must have at least 2 elements.";
const MAX_DIMENSIONS: usize = 32;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Functions with their own multi-output form (the others return one value).
pub(super) fn has_outputs(name: &str) -> bool {
    matches!(name, "ind2sub" | "cummax" | "cummin")
}

fn arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err(NOT_ENOUGH.into())
    } else if args.len() > max {
        Err(TOO_MANY.into())
    } else {
        Ok(())
    }
}

fn matrix_value(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

/// Source index (column-major) of every output element of `rot90(A, k)` for an
/// `m`-by-`n` input, `k` in 0..4. Returns the output shape too.
fn rotation(m: usize, n: usize, k: usize) -> (usize, usize, impl Fn(usize, usize) -> usize) {
    let (rows, cols) = if k % 2 == 1 { (n, m) } else { (m, n) };
    (rows, cols, move |i: usize, j: usize| match k {
        0 => j * m + i,
        1 => (n - 1 - i) * m + j,
        2 => (n - 1 - j) * m + (m - 1 - i),
        _ => i * m + (m - 1 - j),
    })
}

fn gather<T: Clone>(data: &[T], m: usize, n: usize, k: usize) -> (usize, usize, Vec<T>) {
    let (rows, cols, source) = rotation(m, n, k);
    let mut out = Vec::with_capacity(data.len());
    for j in 0..cols {
        out.extend((0..rows).map(|i| data[source(i, j)].clone()));
    }
    (rows, cols, out)
}

#[derive(Clone, Copy, PartialEq)]
enum Reduction {
    Sum,
    Max,
    Min,
    Prod,
}

enum Elements<'a> {
    Real(&'a [f64]),
    Logical(&'a [u8]),
    Text(&'a [u16]),
    Complex(&'a [Complex64]),
}

impl Elements<'_> {
    fn of(array: &Array) -> Option<Elements<'_>> {
        Some(match array {
            Array::Numeric(a) => Elements::Real(a.data()),
            Array::Logical(a) => Elements::Logical(a.data()),
            Array::Character(a) => Elements::Text(a.units()),
            Array::Complex(a) => Elements::Complex(a.data()),
            _ => return None,
        })
    }
    fn at(&self, i: usize) -> (f64, f64) {
        match self {
            Elements::Real(d) => (d[i], 0.),
            Elements::Logical(d) => (f64::from(d[i]), 0.),
            Elements::Text(d) => (f64::from(d[i]), 0.),
            Elements::Complex(d) => (d[i].re, d[i].im),
        }
    }
}

impl Engine {
    pub(super) fn matbuild_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        self.check_cancel()?;
        let text = self.matbuild_text_arguments(args)?;
        let args = text.as_deref().unwrap_or(args);
        let value = match name {
            "cat" => self.matbuild_cat(args)?,
            "horzcat" => self.matbuild_join(args, false)?,
            "vertcat" => self.matbuild_join(args, true)?,
            "rot90" => self.matbuild_rot90(args)?,
            "squeeze" => {
                arity(args, 1, 1)?;
                args[0].clone()
            }
            "sub2ind" => self.matbuild_sub2ind(args)?,
            "ind2sub" | "cummax" | "cummin" => self.matbuild_outputs(name, args, 1)?.remove(0),
            "magic" | "hilb" | "invhilb" | "pascal" | "wilkinson" | "hadamard" => {
                self.matbuild_square(name, args)?
            }
            "rosser" => {
                arity(args, 0, 1)?;
                if let Some(class) = args.first() {
                    self.matbuild_class("rosser", class, "NEWCLASS input is not a string.")?;
                }
                matrix_value(8, 8, generators::rosser())?
            }
            "toeplitz" => self.matbuild_toeplitz(args)?,
            "vander" => self.matbuild_vander(args)?,
            "compan" => self.matbuild_compan(args)?,
            "randperm" => self.matbuild_randperm(args)?,
            "accumarray" => self.matbuild_accumarray(args)?,
            "isequal" => self.matbuild_isequal(args)?,
            "colon" => self.matbuild_colon(args)?,
            _ => self.matbuild_constant(name, args)?,
        };
        Ok(Some(value))
    }

    /// Multi-output forms: `[i, j, ...] = ind2sub(...)`, `[m, i] = cummax(...)`.
    pub(super) fn matbuild_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let text = self.matbuild_text_arguments(args)?;
        let args = text.as_deref().unwrap_or(args);
        match name {
            "ind2sub" => self.matbuild_ind2sub(args, count),
            "cummax" | "cummin" => {
                if count > 2 {
                    return Err("Too many output arguments.".into());
                }
                let (values, indices) = self.matbuild_cumulative(name == "cummax", args)?;
                let mut out = vec![values];
                if count == 2 {
                    out.push(indices);
                }
                Ok(out)
            }
            _ => Err(format!("{name} does not support {count} outputs")),
        }
    }

    // ----- argument helpers -------------------------------------------------

    /// `text_arguments` without its row-only rule: quoted strings and
    /// character rows become text, character matrices stay arrays (MATLAB
    /// concatenates, rotates and compares them as char). `None`: unchanged.
    fn matbuild_text_arguments(&self, args: &[Value]) -> Result<Option<Vec<Value>>> {
        let row = |v: &Value| match v {
            Value::QuotedString(_) => true,
            Value::Container(Array::Character(c)) => c.is_text_row(),
            _ => false,
        };
        if !args.iter().any(row) {
            return Ok(None);
        }
        args.iter()
            .map(|v| match v {
                Value::QuotedString(s) => Ok(Value::Text(s.clone())),
                Value::Container(Array::Character(c)) if c.is_text_row() => {
                    Ok(Value::Text(c.to_utf8(&self.cancel)?))
                }
                other => Ok(other.clone()),
            })
            .collect::<Result<_>>()
            .map(Some)
    }

    fn matbuild_budget(&self, rows: usize, cols: usize) -> Result<usize> {
        let n = Matrix::checked_len(rows, cols)?;
        self.matbuild_fits(n)?;
        Ok(n)
    }

    /// Checks `elements` against the statement budget that
    /// `charge_function_outputs` charges after the call.
    fn matbuild_fits(&self, elements: usize) -> Result<()> {
        if self.evaluated_elements.saturating_add(elements) > 32 * 1024 * 1024 {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    /// Real double data of a numeric, logical or character argument.
    fn matbuild_real(&self, who: &str, value: &Value) -> Result<Matrix> {
        match value {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(l) => l.to_numeric(&self.cancel),
            Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)) => {
                let (rows, cols, units) = self.matbuild_units(value)?;
                Matrix::from_column_vec(rows, cols, units.iter().map(|&u| f64::from(u)).collect())
            }
            Value::Complex(_) => Err(format!("{who} does not support complex input yet")),
            _ => Err(format!("{who}: input must be a numeric array")),
        }
    }

    pub(super) fn matbuild_scalar(&self, who: &str, value: &Value) -> Result<f64> {
        let m = self.matbuild_real(who, value)?;
        if m.len() == 1 {
            Ok(m.data()[0])
        } else {
            Err(format!("{who}: argument must be a real scalar"))
        }
    }

    /// First element, floored, negative counted as zero; square-matrix order.
    fn matbuild_order(&self, who: &str, value: &Value) -> Result<usize> {
        let m = self.matbuild_real(who, value)?;
        let Some(&first) = m.data().first() else {
            return Err(format!("{who}: N must be a real scalar"));
        };
        if !first.is_finite() {
            return Err(format!("{who}: N must be finite"));
        }
        let n = first.floor().max(0.);
        if n > crate::matrix::MAX_ELEMENTS as f64 {
            return Err("Matrix exceeds the element limit".into());
        }
        let n = n as usize;
        self.matbuild_budget(n, n)?;
        Ok(n)
    }

    /// Size vector of `sub2ind`/`ind2sub`: a vector of positive integers.
    fn matbuild_size_vector(&self, value: &Value, minimum: usize) -> Result<Vec<usize>> {
        let m = self.matbuild_real("size", value)?;
        if m.len() < minimum {
            return Err(SIZE_VECTOR.into());
        }
        if (m.rows() != 1 && m.cols() != 1) || m.len() > MAX_DIMENSIONS * 4 {
            return Err(
                "Size vector must be a row vector with real positive integer elements.".into(),
            );
        }
        m.data()
            .iter()
            .map(|&v| {
                if v.is_finite() && v >= 1. && v.fract() == 0. && v < 9.0e15 {
                    Ok(v as usize)
                } else {
                    Err(
                        "Size vector must be a row vector with real positive integer elements."
                            .to_owned(),
                    )
                }
            })
            .collect()
    }

    // ----- cat / horzcat / vertcat / rot90 -----------------------------------

    fn matbuild_nonempty(&self, args: &[Value]) -> Result<Vec<Value>> {
        let mut kept = Vec::with_capacity(args.len());
        for part in args {
            self.check_cancel()?;
            let (rows, cols) = part.shape(&self.cancel)?;
            if rows != 0 && cols != 0 {
                kept.push(part.clone());
            }
        }
        Ok(kept)
    }

    fn matbuild_join(&mut self, args: &[Value], vertical: bool) -> Result<Value> {
        if args.is_empty() {
            return matrix_value(0, 0, Vec::new());
        }
        // The reference skips every empty shape. Empty cell/char operands still
        // determine the result type, e.g. horzcat([1 2], '') is character.
        let mut kept = self.matbuild_nonempty(args)?;
        if kept.is_empty() {
            // With no nonempty operand, retain the empty result's dimensions:
            // in the reference, vertcat(zeros(1,0),zeros(1,0)) is zeros(2,0).
            kept.extend(
                args.iter()
                    .filter(|a| !matches!(a, Value::Matrix(m) if m.rows() == 0 && m.cols() == 0))
                    .cloned(),
            );
        }
        if args
            .iter()
            .any(|a| matches!(a, Value::Container(Array::Cell(_))))
        {
            return self.matbuild_join_cells(kept, vertical);
        }
        self.matbuild_check_shapes(&kept, vertical)?;
        let is_char = |a: &Value| {
            matches!(
                a,
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_))
            )
        };
        let chars_only = kept
            .iter()
            .all(|a| is_char(a) || matches!(a, Value::Matrix(_) | Value::Logical(_)));
        if chars_only && args.iter().any(is_char) {
            return self.matbuild_join_chars(&kept, vertical);
        }
        if kept.is_empty() {
            return matrix_value(0, 0, Vec::new());
        }
        let rows = if vertical {
            kept.into_iter().map(|a| vec![a]).collect()
        } else {
            vec![kept]
        };
        self.concatenate_values(rows)
    }

    /// Row counts (horizontal) or column counts (vertical) of the parts other
    /// than 0-by-0 must agree, with MATLAB's text for the function forms. The
    /// caller has already dropped empty parts unless every part is empty, so
    /// horzcat(zeros(2,0),zeros(3,0)) is still a mismatch.
    fn matbuild_check_shapes(&self, parts: &[Value], vertical: bool) -> Result<()> {
        let mut first: Option<(usize, usize)> = None;
        for part in parts {
            let Ok((rows, cols)) = part.shape(&self.cancel) else {
                continue;
            };
            if (rows, cols) == (0, 0) {
                continue;
            }
            match first {
                None => first = Some((rows, cols)),
                Some((r, c)) if (vertical && cols != c) || (!vertical && rows != r) => {
                    return Err(INCONSISTENT.into());
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Character data of a char, numeric or logical argument: rows, columns and
    /// UTF-16 units (numbers are character codes, as in `['a', 66]`).
    fn matbuild_units(&self, value: &Value) -> Result<(usize, usize, Vec<u16>)> {
        match value.clone().array(&self.cancel)? {
            Array::Character(c) => Ok((c.rows(), c.cols(), c.units().to_vec())),
            Array::Numeric(m) => {
                let units = m
                    .data()
                    .iter()
                    .map(|&v| {
                        if (0. ..65536.).contains(&v.round()) {
                            Ok(v.round() as u16)
                        } else {
                            Err("Out of range or non-integer values truncated during conversion to character.".to_owned())
                        }
                    })
                    .collect::<Result<Vec<u16>>>()?;
                Ok((m.rows(), m.cols(), units))
            }
            Array::Logical(l) => Ok((
                l.rows(),
                l.cols(),
                l.data().iter().map(|&b| u16::from(b)).collect(),
            )),
            _ => Err("Cannot concatenate these values with text".into()),
        }
    }

    /// Concatenation with at least one character part: the result is a
    /// character array (char matrices allowed, numbers become character codes).
    fn matbuild_join_chars(&mut self, parts: &[Value], vertical: bool) -> Result<Value> {
        let mut pieces = Vec::with_capacity(parts.len());
        for part in parts {
            self.check_cancel()?;
            let (rows, cols, units) = self.matbuild_units(part)?;
            if (rows, cols) != (0, 0) {
                pieces.push((rows, cols, units));
            }
        }
        let (rows, cols, units) = if let Some(&(r0, c0, _)) = pieces.first() {
            if vertical {
                let rows = pieces.iter().map(|p| p.0).sum::<usize>();
                let mut units = Vec::with_capacity(rows * c0);
                for j in 0..c0 {
                    self.check_cancel()?;
                    for (r, _, u) in &pieces {
                        units.extend_from_slice(&u[j * r..(j + 1) * r]);
                    }
                }
                (rows, c0, units)
            } else {
                let cols = pieces.iter().map(|p| p.1).sum::<usize>();
                (r0, cols, pieces.into_iter().flat_map(|p| p.2).collect())
            }
        } else {
            (0, 0, Vec::new())
        };
        Ok(Value::Container(
            crate::CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
        ))
    }

    /// Cell concatenation, cell by cell in column-major order. A non-cell part
    /// becomes one cell, as in MATLAB's `[100, {1, 2}, 'abc']`; empty inputs
    /// are removed when a nonempty operand is present.
    fn matbuild_join_cells(&mut self, parts: Vec<Value>, vertical: bool) -> Result<Value> {
        let cancel = &self.cancel;
        let mut cells = Vec::with_capacity(parts.len());
        for part in parts {
            crate::dsp::check_cancel(cancel)?;
            let cell = match part {
                Value::Container(Array::Cell(c)) => c,
                other => {
                    if other.shape(cancel)? == (0, 0) {
                        continue;
                    }
                    CellArray::from_shared(1, 1, vec![Arc::new(other.array(cancel)?)], cancel)?
                }
            };
            if cell.rows() != 0 || cell.cols() != 0 {
                cells.push(cell);
            }
        }
        let Some(first) = cells.first() else {
            return Ok(Value::Container(Array::Cell(CellArray::empty(
                0, 0, cancel,
            )?)));
        };
        let sum = |extent: fn(&CellArray) -> usize| {
            cells
                .iter()
                .try_fold(0usize, |total, c| total.checked_add(extent(c)))
                .ok_or("Dimension overflow")
        };
        let (rows, cols) = if vertical {
            if cells.iter().any(|c| c.cols() != first.cols()) {
                return Err(INCONSISTENT.into());
            }
            (sum(CellArray::rows)?, first.cols())
        } else {
            if cells.iter().any(|c| c.rows() != first.rows()) {
                return Err(INCONSISTENT.into());
            }
            (first.rows(), sum(CellArray::cols)?)
        };
        // each slot is a shared pointer to an existing element: no element copies
        let count = sum(CellArray::len)?;
        if count > crate::script_containers::MAX_CELL_SLOTS {
            return Err("Cell shape, payload or capacity exceeds 65536 slots".into());
        }
        let mut slots = Vec::with_capacity(count);
        if vertical {
            for j in 0..cols {
                crate::dsp::check_cancel(cancel)?;
                for c in &cells {
                    for i in 0..c.rows() {
                        slots.push(c.get_linear(j * c.rows() + i)?.clone());
                    }
                }
            }
        } else {
            for c in &cells {
                crate::dsp::check_cancel(cancel)?;
                for i in 0..c.len() {
                    slots.push(c.get_linear(i)?.clone());
                }
            }
        }
        Ok(Value::Container(Array::Cell(CellArray::from_shared(
            rows, cols, slots, cancel,
        )?)))
    }

    fn matbuild_cat(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, usize::MAX)?;
        let dim = self.matbuild_scalar("cat", &args[0])?;
        if !(dim >= 1. && dim.fract() == 0. && dim < 1.0e9) {
            return Err("Dimension must be a real positive finite integer.".into());
        }
        match (dim as usize, args.len()) {
            (_, 1) => matrix_value(0, 0, Vec::new()),
            (1, _) => self.matbuild_join(&args[1..], true),
            (2, _) => self.matbuild_join(&args[1..], false),
            (_, 2) => Ok(args[1].clone()),
            _ => match self.matbuild_nonempty(&args[1..])?.as_slice() {
                [] => self.matbuild_join(&args[1..], false),
                [one] => Ok(one.clone()),
                _ => Err("cat along dimension 3 or higher needs N-dimensional arrays, which are not implemented".into()),
            },
        }
    }

    fn matbuild_rot90(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 2)?;
        let k = match args.get(1) {
            None => 1,
            Some(v) => {
                let k = self.matbuild_real("rot90", v)?;
                if k.len() != 1 {
                    return Err("k must be a scalar.".into());
                }
                if !k.data()[0].is_finite() || k.data()[0].fract() != 0. {
                    return Err("k must be an integer.".into());
                }
                k.data()[0].rem_euclid(4.) as usize
            }
        };
        let cancel = &self.cancel;
        Ok(match &args[0] {
            Value::Matrix(a) => {
                let (r, c, d) = gather(a.data(), a.rows(), a.cols(), k);
                matrix_value(r, c, d)?
            }
            Value::Logical(a) => {
                let (r, c, d) = gather(a.data(), a.rows(), a.cols(), k);
                Value::Logical(LogicalMatrix::from_column_vec(r, c, d, cancel)?)
            }
            Value::Complex(a) => {
                let (r, c, d) = gather(a.data(), a.rows(), a.cols(), k);
                Value::Complex(ComplexMatrix::from_column_vec(r, c, d, cancel)?)
            }
            Value::Container(Array::Cell(a)) => {
                let slots: Vec<Arc<Array>> = (0..a.len())
                    .map(|i| a.get_linear(i).cloned())
                    .collect::<Result<_>>()?;
                let (r, c, d) = gather(&slots, a.rows(), a.cols(), k);
                Value::Container(Array::Cell(CellArray::from_shared(r, c, d, cancel)?))
            }
            text @ (Value::Text(_)
            | Value::QuotedString(_)
            | Value::Container(Array::Character(_))) => {
                let (rows, cols, units) = self.matbuild_units(text)?;
                let (r, c, d) = gather(&units, rows, cols, k);
                Value::Container(crate::CharacterRow::from_column_major(r, c, d, cancel)?.into())
            }
            _ => {
                return Err(
                    "rot90 requires a numeric, logical, complex, character or cell array".into(),
                );
            }
        })
    }

    // ----- sub2ind / ind2sub --------------------------------------------------

    /// Dimensions as seen by `count` subscripts: the last absorbs the rest,
    /// missing ones are 1.
    fn matbuild_effective_dims(size: &[usize], count: usize) -> Vec<f64> {
        let mut dims: Vec<f64> = size.iter().take(count).map(|&d| d as f64).collect();
        if count < size.len() {
            dims[count - 1] = size[count - 1..].iter().map(|&d| d as f64).product();
        }
        dims.resize(count, 1.);
        dims
    }

    /// The reference's `sub2ind` range test is `v < 1 || v > dim` only: non-integer
    /// subscripts and NaN pass through the arithmetic.
    fn matbuild_sub2ind(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, usize::MAX)?;
        let size = self.matbuild_size_vector(&args[0], 2)?;
        if args.len() == 1 {
            return Err(
                "Subscript indices must either be real positive integers or logicals.".into(),
            );
        }
        let subs = args[1..]
            .iter()
            .map(|v| self.matbuild_real("sub2ind", v))
            .collect::<Result<Vec<_>>>()?;
        let (rows, cols) = (subs[0].rows(), subs[0].cols());
        if subs.iter().any(|s| (s.rows(), s.cols()) != (rows, cols)) {
            return Err("The subscript vectors must all be of the same size.".into());
        }
        let dims = Self::matbuild_effective_dims(&size, subs.len());
        let mut out = vec![1.; rows * cols];
        let mut stride = 1.;
        for (s, &dim) in subs.iter().zip(&dims) {
            self.check_cancel()?;
            for (o, &v) in out.iter_mut().zip(s.data()) {
                if v < 1. || v > dim {
                    return Err(OUT_OF_RANGE.into());
                }
                *o += (v - 1.) * stride;
            }
            stride *= dim;
        }
        matrix_value(rows, cols, out)
    }

    /// The reference's `ind2sub` has no range check: the last output absorbs any
    /// overflow, and one output is the index itself.
    fn matbuild_ind2sub(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        arity(args, 2, 2)?;
        let size = self.matbuild_size_vector(&args[0], 1)?;
        let index = self.matbuild_real("ind2sub", &args[1])?;
        let count = count.max(1);
        if count > 1 << 16 {
            return Err("Too many output arguments.".into());
        }
        // every output has numel(IND) elements: refuse before allocating any
        self.matbuild_fits(count.saturating_mul(index.len()))?;
        let dims = Self::matbuild_effective_dims(&size, count);
        let mut remaining = index.data().to_vec();
        let mut outputs: Vec<Option<Value>> = (0..count).map(|_| None).collect();
        for position in (1..count).rev() {
            self.check_cancel()?;
            let stride: f64 = dims[..position].iter().product();
            let mut part = Vec::with_capacity(remaining.len());
            for v in &mut remaining {
                // MATLAB: vi = rem(ndx-1, k) + 1; vj = (ndx - vi)/k + 1
                let rest = (*v - 1.) % stride + 1.;
                part.push((*v - rest) / stride + 1.);
                *v = rest;
            }
            outputs[position] = Some(matrix_value(index.rows(), index.cols(), part)?);
        }
        outputs[0] = Some(matrix_value(index.rows(), index.cols(), remaining)?);
        Ok(outputs.into_iter().flatten().collect())
    }

    // ----- test matrices ------------------------------------------------------

    /// Optional class-name argument of hilb/pascal/rosser; only 'double' exists.
    fn matbuild_class(&self, name: &str, value: &Value, not_text: &str) -> Result<()> {
        match value {
            Value::Text(c) | Value::QuotedString(c) if c == "double" => Ok(()),
            Value::Text(c) | Value::QuotedString(c) if c == "single" => Err(format!(
                "{name}('single') is not implemented: single-precision arrays are not supported"
            )),
            Value::Text(_) | Value::QuotedString(_) => {
                Err("Class must be 'double' or 'single'.".into())
            }
            _ => Err(not_text.into()),
        }
    }

    fn matbuild_square(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        let (min, max) = match name {
            "pascal" => (1, 3),
            "hilb" => (1, 2),
            _ => (1, 1),
        };
        arity(args, min, max)?;
        if name == "hilb" && args.len() == 2 {
            self.matbuild_class(name, &args[1], "CLASSNAME must be 'double' or 'single'.")?;
        }
        if name == "pascal" && args.len() == 3 {
            self.matbuild_class(name, &args[2], "NEWCLASS input is not a string.")?;
        }
        if matches!(name, "wilkinson" | "invhilb" | "pascal") {
            let m = self.matbuild_real(name, &args[0])?;
            if name == "wilkinson" && m.len() != 1 {
                return Err("Size inputs must be scalar.".into());
            }
            if m.data()
                .first()
                .is_some_and(|v| v.is_finite() && v.fract() != 0.)
            {
                return Err("Size inputs must be integers.".into());
            }
        }
        let n = if name == "hadamard" {
            let v = self.matbuild_scalar(name, &args[0])?;
            if !(v >= 1. && v.fract() == 0. && v < 1.0e9) || !generators::hadamard_order(v as usize)
            {
                return Err(
                    "n must be an integer and n, n/12 or n/20 must be a power of 2.".into(),
                );
            }
            if v > 4096. {
                return Err("Maximum variable size allowed by the program is exceeded.".into());
            }
            self.matbuild_budget(v as usize, v as usize)?;
            v as usize
        } else {
            if name == "magic" {
                let m = self.matbuild_real(name, &args[0])?;
                match m.data().first() {
                    Some(v) if v.is_nan() => {
                        return Err(
                            "Subscript indices must either be real positive integers or logicals."
                                .into(),
                        );
                    }
                    Some(v) if *v == f64::INFINITY => {
                        return Err(
                            "Maximum variable size allowed by the program is exceeded.".into()
                        );
                    }
                    _ => {}
                }
            }
            self.matbuild_order(name, &args[0])?
        };
        let data = match name {
            "magic" => generators::magic(n),
            "hilb" => generators::hilb(n),
            "invhilb" => generators::invhilb(n),
            "wilkinson" => generators::wilkinson(n),
            "pascal" => {
                let k = match args.get(1) {
                    None => 0.,
                    Some(v) => self.matbuild_scalar(name, v)?,
                };
                if !(k == 0. || k == 1. || k == 2.) {
                    return Err("Second argument must be 0, 1, or 2.".into());
                }
                generators::pascal(n, k as u8, &self.cancel)?
            }
            _ => {
                let rows = generators::hadamard(n).ok_or("Invalid Hadamard order")?;
                // row-major to column-major
                (0..n)
                    .flat_map(|j| (0..n).map(move |i| (i, j)))
                    .map(|(i, j)| rows[i * n + j])
                    .collect()
            }
        };
        matrix_value(n, n, data)
    }

    /// Real and imaginary parts (column-major) of a numeric or complex value.
    fn matbuild_parts(&self, who: &str, value: &Value) -> Result<(Vec<f64>, Option<Vec<f64>>)> {
        match value {
            Value::Complex(c) => Ok((
                c.data().iter().map(|z| z.re).collect(),
                Some(c.data().iter().map(|z| z.im).collect()),
            )),
            other => Ok((self.matbuild_real(who, other)?.data().to_vec(), None)),
        }
    }

    fn matbuild_complex_value(
        &self,
        rows: usize,
        cols: usize,
        data: Vec<Complex64>,
    ) -> Result<Value> {
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            rows,
            cols,
            data,
            &self.cancel,
        )?))
    }

    fn matbuild_toeplitz(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 2)?;
        let numeric = |v: &Value| matches!(v, Value::Matrix(_) | Value::Complex(_));
        if !args.iter().all(numeric) {
            return Err("Inputs must be numeric.".into());
        }
        let zip = |(re, im): (Vec<f64>, Option<Vec<f64>>)| -> Vec<Complex64> {
            let im = im.unwrap_or_else(|| vec![0.; re.len()]);
            re.into_iter()
                .zip(im)
                .map(|(a, b)| Complex64::new(a, b))
                .collect()
        };
        let complex = args.iter().any(|v| matches!(v, Value::Complex(_)));
        let first = zip(self.matbuild_parts("toeplitz", &args[0])?);
        let (c, r) = match args.get(1) {
            Some(v) => (first, zip(self.matbuild_parts("toeplitz", v)?)),
            None => {
                // toeplitz(r): Hermitian, first column conj(r) with r(1) on the diagonal
                let mut c: Vec<Complex64> = first.iter().map(Complex64::conj).collect();
                if let Some(head) = c.first_mut() {
                    *head = first[0];
                }
                (c, first)
            }
        };
        if c.is_empty() || r.is_empty() {
            return Err("Index exceeds matrix dimensions.".into());
        }
        let (m, n) = (c.len(), r.len());
        self.matbuild_budget(m, n)?;
        let mut out = Vec::with_capacity(m * n);
        for j in 0..n {
            self.check_cancel()?;
            out.extend((0..m).map(|i| if i >= j { c[i - j] } else { r[j - i] }));
        }
        if complex {
            self.matbuild_complex_value(m, n, out)
        } else {
            matrix_value(m, n, out.into_iter().map(|z| z.re).collect())
        }
    }

    fn matbuild_vander(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 1)?;
        let c = self.matbuild_real("vander", &args[0])?.data().to_vec();
        let m = c.len();
        self.matbuild_budget(m, m)?;
        let mut out = vec![1.; m * m];
        for j in (0..m.saturating_sub(1)).rev() {
            self.check_cancel()?;
            for i in 0..m {
                out[j * m + i] = c[i] * out[(j + 1) * m + i];
            }
        }
        matrix_value(m, m, out)
    }

    fn matbuild_compan(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 1)?;
        if matches!(
            &args[0],
            Value::Text(_) | Value::QuotedString(_) | Value::Container(_)
        ) {
            return Err("Input following 'like' is not a numeric or logical array.".into());
        }
        let (re, im) = self.matbuild_parts("compan", &args[0])?;
        let (rows, cols) = args[0].shape(&self.cancel)?;
        if (rows, cols) == (0, 0) || (rows != 1 && cols != 1) {
            return Err("Input argument must be a vector.".into());
        }
        let n = re.len();
        if n <= 1 {
            return matrix_value(0, 0, Vec::new());
        }
        self.matbuild_budget(n - 1, n - 1)?;
        match im {
            None => matrix_value(n - 1, n - 1, generators::compan(&re)),
            Some(im) => {
                let m = n - 1;
                let u: Vec<Complex64> = re
                    .iter()
                    .zip(&im)
                    .map(|(&a, &b)| Complex64::new(a, b))
                    .collect();
                let mut out = vec![Complex64::new(0., 0.); m * m];
                for (j, &c) in u[1..].iter().enumerate() {
                    out[j * m] = -(c / u[0]);
                }
                for i in 1..m {
                    out[(i - 1) * m + i] = Complex64::new(1., 0.);
                }
                self.matbuild_complex_value(m, m, out)
            }
        }
    }

    // ----- randperm / accumarray / isequal -------------------------------------

    /// The reference uses sorted uniform keys for randperm(n), but sampling/shuffling
    /// for randperm(n,k). Keep their stream consumption distinct.
    fn matbuild_randperm(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 2)?;
        let n = self.matbuild_real("randperm", &args[0])?;
        if n.len() != 1 {
            return Err("Size inputs must be scalar.".into());
        }
        let n = n.data()[0];
        if !n.is_finite() {
            return Err("NaN and Inf not allowed.".into());
        }
        if n.fract() != 0. {
            return Err("Size inputs must be integers.".into());
        }
        let n = n.max(0.);
        if n > crate::matrix::MAX_ELEMENTS as f64 {
            return Err("Matrix exceeds the element limit".into());
        }
        let n = n as usize;
        let k = match args.get(1) {
            None => n,
            Some(v) => {
                let m = self.matbuild_real("randperm", v)?;
                let k = m.data().first().copied().unwrap_or(f64::NAN);
                if m.len() != 1 || !(k >= 0. && k.fract() == 0. && k < 4.0e15) {
                    return Err("Inputs must be nonnegative scalar integers.".into());
                }
                k as usize
            }
        };
        if k > n {
            return Err("K must be less than or equal to N.".into());
        }
        self.matbuild_budget(1, k)?;
        if args.len() == 2 {
            return self.matbuild_randperm_sample(n, k);
        }
        let mut keyed: Vec<(f64, u32)> = Vec::with_capacity(n);
        for i in 0..n {
            if i % 65536 == 0 {
                self.check_cancel()?;
            }
            keyed.push((self.random.next_f64(), i as u32));
        }
        self.check_cancel()?;
        keyed.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        matrix_value(
            1,
            k,
            keyed[..k].iter().map(|&(_, i)| f64::from(i) + 1.).collect(),
        )
    }

    fn matbuild_randperm_sample(&mut self, n: usize, k: usize) -> Result<Value> {
        let mut out = Vec::with_capacity(k);
        if k == n {
            // Inside-out Fisher-Yates: n-1 draws, including no draws for n<=1.
            for i in 0..n {
                if i % 65536 == 0 {
                    self.check_cancel()?;
                }
                let j = if i == 0 {
                    0
                } else {
                    (self.random.next_f64() * (i + 1) as f64) as usize
                };
                out.push((i + 1) as f64);
                out.swap(i, j);
            }
        } else if k < n.div_ceil(4) {
            // Partial Fisher-Yates from the end of a virtual 1:n vector.
            // Store only moved slots, keeping small samples O(k) in space/time.
            let mut moved = std::collections::HashMap::with_capacity(k);
            for i in 0..k {
                if i % 65536 == 0 {
                    self.check_cancel()?;
                }
                let last = n - i - 1;
                let j = (self.random.next_f64() * (last + 1) as f64) as usize;
                let selected = moved.remove(&j).unwrap_or(j);
                let tail = moved.remove(&last).unwrap_or(last);
                if j != last {
                    moved.insert(j, tail);
                }
                out.push((selected + 1) as f64);
            }
        } else {
            // Select the next ordered sample by its discrete skip CDF, then
            // insert it uniformly into the permutation: two draws per item.
            // P(skip=s+1)/P(skip=s) = (left-needed)/(left-1).
            let mut left = n;
            for i in 0..k {
                self.check_cancel()?;
                let needed = k - i;
                let mut u = self.random.next_f64();
                let mut probability = needed as f64 / left as f64;
                // The final possible choice also bounds accumulated rounding.
                while left > needed && u > probability {
                    u -= probability;
                    probability *= (left - needed) as f64 / (left - 1) as f64;
                    left -= 1;
                    if left.is_multiple_of(65536) {
                        self.check_cancel()?;
                    }
                }
                out.push((n - left + 1) as f64);
                left -= 1;
                let j = (self.random.next_f64() * (i + 1) as f64) as usize;
                out.swap(i, j);
            }
        }
        matrix_value(1, k, out)
    }

    fn matbuild_accumarray(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 2, 5)?;
        let Value::Matrix(subs) = &args[0] else {
            return Err(
                "First input SUBS must be a real, full, numeric matrix or a cell vector.".into(),
            );
        };
        let vals = self.matbuild_real("accumarray", &args[1])?;
        // Rows of SUBS are observations; a single row of 2+ entries is one subscript.
        let (observations, d) = (subs.rows(), subs.cols());
        if d == 0 {
            return Err(
                "First input SUBS must be a real, full, numeric matrix with at least one column."
                    .into(),
            );
        }
        for &v in subs.data() {
            if !(v >= 1. && v.fract() == 0. && v < 4.0e15) {
                return Err("First input SUBS must contain positive integer subscripts.".into());
            }
        }
        if !(vals.len() == 1 || vals.len() == observations) || (vals.rows() > 1 && vals.cols() > 1)
        {
            return Err("Second input VAL must be a vector with one element for each row in SUBS, or a scalar.".into());
        }
        if d > 2 {
            for &v in &subs.data()[observations * 2..] {
                if v != 1. {
                    return Err("accumarray with more than two subscript columns needs N-dimensional arrays, which are not implemented".into());
                }
            }
        }
        let used = d.min(2);
        let mut extent = [0usize; 2];
        for (column, slot) in extent.iter_mut().enumerate().take(used) {
            *slot = subs.data()[column * observations..(column + 1) * observations]
                .iter()
                .fold(0., |a: f64, &b| a.max(b)) as usize;
        }
        if used < 2 {
            extent[1] = 1;
        }
        let size = match args.get(2) {
            Some(v) if !matches!(v, Value::Matrix(m) if m.is_empty()) => {
                let s = self.matbuild_real("accumarray", v)?;
                let ok = s
                    .data()
                    .iter()
                    .all(|&x| x >= 0. && x.fract() == 0. && x < 4.0e15);
                if used == 1 && !(ok && s.len() == 2 && s.data()[1] == 1.) {
                    return Err("When SUBS is a column vector, the third input SZ must be of the form [N 1].".into());
                }
                if !ok || s.len() != 2 {
                    return Err("Third input SZ must have one element per column of SUBS.".into());
                }
                let (r, c) = (s.data()[0] as usize, s.data()[1] as usize);
                if r < extent[0] || c < extent[1] {
                    return Err(
                        "First input SUBS and third input SZ must satisfy ALL(MAX(SUBS)<=SZ)."
                            .into(),
                    );
                }
                (r, c)
            }
            _ => (extent[0], extent[1]),
        };
        let reduction = match args.get(3) {
            None => Reduction::Sum,
            Some(Value::Matrix(m)) if m.is_empty() => Reduction::Sum,
            Some(Value::Text(name)) => match name.as_str() {
                "sum" => Reduction::Sum,
                "max" => Reduction::Max,
                "min" => Reduction::Min,
                "prod" => Reduction::Prod,
                _ => return Err("accumarray supports only the functions sum, max, min and prod (given as text, e.g. 'max')".into()),
            },
            Some(_) => return Err("accumarray supports only the functions sum, max, min and prod (given as text, e.g. 'max')".into()),
        };
        let fill = match args.get(4) {
            None => 0.,
            Some(Value::Matrix(m)) if m.is_empty() => 0.,
            Some(v) => self.matbuild_scalar("accumarray", v)?,
        };
        self.matbuild_budget(size.0, size.1)?;
        let mut cells = vec![f64::NAN; size.0 * size.1];
        let mut seen = vec![false; size.0 * size.1];
        for o in 0..observations {
            if o % 8192 == 0 {
                self.check_cancel()?;
            }
            let row = subs.data()[o] as usize - 1;
            let column = if used == 2 {
                subs.data()[observations + o] as usize - 1
            } else {
                0
            };
            let slot = column * size.0 + row;
            let v = if vals.len() == 1 {
                vals.data()[0]
            } else {
                vals.data()[o]
            };
            if !seen[slot] {
                seen[slot] = true;
                cells[slot] = v;
                continue;
            }
            let current = cells[slot];
            cells[slot] = match reduction {
                Reduction::Sum => current + v,
                Reduction::Prod => current * v,
                // max/min ignore NaN unless the group is all NaN
                Reduction::Max => {
                    if v.is_nan() || current >= v {
                        current
                    } else {
                        v
                    }
                }
                Reduction::Min => {
                    if v.is_nan() || current <= v {
                        current
                    } else {
                        v
                    }
                }
            };
            if matches!(reduction, Reduction::Max | Reduction::Min) && current.is_nan() {
                cells[slot] = v;
            }
        }
        for (cell, &hit) in cells.iter_mut().zip(&seen) {
            if !hit {
                *cell = fill;
            }
        }
        matrix_value(size.0, size.1, cells)
    }

    fn matbuild_isequal(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 2, usize::MAX)?;
        let arrays = args
            .iter()
            .map(|a| a.clone().array(&self.cancel))
            .collect::<Result<Vec<_>>>()?;
        for pair in arrays.windows(2) {
            if !self.matbuild_equal(&pair[0], &pair[1], 0)? {
                return Ok(Value::Logical(LogicalMatrix::scalar(false)));
            }
        }
        Ok(Value::Logical(LogicalMatrix::scalar(true)))
    }

    /// MATLAB `isequal` for two arrays: same size and equal contents; numeric,
    /// logical and character data compare by value, NaN differs from NaN, and
    /// `0 == -0`.
    fn matbuild_equal(&self, a: &Array, b: &Array, depth: usize) -> Result<bool> {
        self.check_cancel()?;
        if depth > 32 {
            return Err("isequal: nesting is too deep".into());
        }
        if (a.rows(), a.cols()) != (b.rows(), b.cols()) {
            return Ok(false);
        }
        match (a, b) {
            (Array::Cell(x), Array::Cell(y)) => {
                for i in 0..x.len() {
                    if !self.matbuild_equal(x.get_linear(i)?, y.get_linear(i)?, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Array::Struct(x), Array::Struct(y)) => {
                // same fields in any order, then equal values element by element
                if x.field_count() != y.field_count() {
                    return Ok(false);
                }
                for (field, name) in x.field_names().iter().enumerate() {
                    let Some(other) = y.field_index(name) else {
                        return Ok(false);
                    };
                    for element in 0..x.len() {
                        if !self.matbuild_equal(
                            x.get(field, element)?,
                            y.get(other, element)?,
                            depth + 1,
                        )? {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            (Array::Cell(_), _)
            | (_, Array::Cell(_))
            | (Array::Struct(_), _)
            | (_, Array::Struct(_)) => Ok(false),
            _ => {
                let (Some(x), Some(y)) = (Elements::of(a), Elements::of(b)) else {
                    return Err("isequal does not support graphics handles".into());
                };
                for i in 0..a.rows() * a.cols() {
                    if i % 65536 == 0 {
                        self.check_cancel()?;
                    }
                    if x.at(i) != y.at(i) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    // ----- cummax / cummin ----------------------------------------------------

    fn matbuild_cumulative(&mut self, maximum: bool, args: &[Value]) -> Result<(Value, Value)> {
        let who = if maximum { "cummax" } else { "cummin" };
        let direction = format!(
            "{} direction must be 'forward' or 'reverse'.",
            who.to_ascii_uppercase()
        );
        arity(args, 1, 3)?;
        let (rows, cols, re, im, logical) = match &args[0] {
            Value::Matrix(m) => (m.rows(), m.cols(), m.data().to_vec(), None, false),
            Value::Logical(l) => {
                let m = l.to_numeric(&self.cancel)?;
                (m.rows(), m.cols(), m.data().to_vec(), None, true)
            }
            Value::Complex(c) => {
                let (re, im) = self.matbuild_parts(who, &args[0])?;
                (c.rows(), c.cols(), re, im, false)
            }
            _ => {
                return Err(format!(
                    "{} supports only numerical or logical inputs.",
                    who.to_ascii_uppercase()
                ));
            }
        };
        let mut dim = None;
        let mut reverse = false;
        for (position, extra) in args[1..].iter().enumerate() {
            if let Value::Text(s) | Value::QuotedString(s) = extra {
                match s.to_ascii_lowercase().as_str() {
                    "reverse" => reverse = true,
                    "forward" => reverse = false,
                    _ => return Err(direction),
                }
            } else if position == 0 {
                let d = self.matbuild_real(who, extra)?;
                let d = d.data().first().copied().unwrap_or(f64::NAN);
                if extra.shape(&self.cancel)? != (1, 1)
                    || !(d >= 1. && d.fract() == 0. && d < 1.0e9)
                {
                    return Err("Dimension argument must be a positive integer scalar within indexing range.".into());
                }
                dim = Some(d as usize);
            } else {
                return Err(direction);
            }
        }
        let dim = dim.unwrap_or(if rows != 1 { 1 } else { 2 });
        let mut out_re = re.clone();
        let mut out_im = im.clone();
        let mut indices = vec![1.; re.len()];
        let is_nan = |k: usize| re[k].is_nan() || im.as_ref().is_some_and(|i| i[k].is_nan());
        // true when element a beats element b (complex: by magnitude, then phase)
        let better = |a: usize, b: usize| -> bool {
            let order = match &im {
                None => re[a].partial_cmp(&re[b]),
                Some(im) => {
                    let (ma, mb) = (re[a].hypot(im[a]), re[b].hypot(im[b]));
                    if ma == mb {
                        im[a].atan2(re[a]).partial_cmp(&im[b].atan2(re[b]))
                    } else {
                        ma.partial_cmp(&mb)
                    }
                }
            };
            if maximum {
                order == Some(std::cmp::Ordering::Greater)
            } else {
                order == Some(std::cmp::Ordering::Less)
            }
        };
        if dim <= 2 && !re.is_empty() {
            let (lines, length, step, line_step) = if dim == 1 {
                (cols, rows, 1, rows)
            } else {
                (rows, cols, rows, 1)
            };
            for line in 0..lines {
                self.check_cancel()?;
                let mut best: Option<usize> = None;
                let mut at = 0usize;
                for t in 0..length {
                    let position = if reverse { length - 1 - t } else { t };
                    if t == 0 {
                        at = position;
                    }
                    let slot = line * line_step + position * step;
                    if !is_nan(slot) && best.is_none_or(|b| better(slot, b)) {
                        best = Some(slot);
                        at = position;
                    }
                    // Until a non-NaN element appears the output keeps the
                    // element itself, e.g. complex(1,NaN) stays 1+NaNi.
                    if let Some(b) = best {
                        out_re[slot] = re[b];
                        if let (Some(o), Some(i)) = (out_im.as_mut(), im.as_ref()) {
                            o[slot] = i[b];
                        }
                    }
                    indices[slot] = (at + 1) as f64;
                }
            }
        }
        let values = match out_im {
            Some(imag) => self.matbuild_complex_value(
                rows,
                cols,
                out_re
                    .iter()
                    .zip(&imag)
                    .map(|(&a, &b)| Complex64::new(a, b))
                    .collect(),
            )?,
            None if logical => Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                out_re.iter().map(|&v| u8::from(v != 0.)).collect(),
                &self.cancel,
            )?),
            None => matrix_value(rows, cols, out_re)?,
        };
        Ok((values, matrix_value(rows, cols, indices)?))
    }

    // ----- colon and constants --------------------------------------------------

    fn matbuild_colon(&mut self, args: &[Value]) -> Result<Value> {
        arity(args, 2, 3)?;
        let operands = args
            .iter()
            .map(|v| {
                // a complex operand contributes its real part
                Ok(self.matbuild_parts("colon", v)?.0.first().copied())
            })
            .collect::<Result<Vec<_>>>()?;
        let empty = matrix_value(1, 0, Vec::new());
        let (start, step, stop) = match operands.as_slice() {
            [a, b] => (*a, Some(1.), *b),
            [a, s, b] => (*a, *s, *b),
            _ => unreachable!("arity checked above"),
        };
        let (Some(start), Some(step), Some(stop)) = (start, step, stop) else {
            return empty;
        };
        if start.is_nan() || step.is_nan() || stop.is_nan() {
            return matrix_value(1, 1, vec![f64::NAN]);
        }
        let span = (stop - start) / step;
        if step == 0. || span < 0. {
            return empty;
        }
        if start.is_infinite() || stop.is_infinite() || step.is_infinite() {
            return Err("Maximum variable size allowed by the program is exceeded.".into());
        }
        let count = (span + 4. * f64::EPSILON * span.abs().max(1.)).floor() + 1.;
        if count > crate::matrix::MAX_ELEMENTS as f64 {
            return Err("Range exceeds matrix element limit".into());
        }
        let count = count as usize;
        self.matbuild_budget(1, count)?;
        let range = self.range_matrix(start, step, count)?;
        let text = |v: &Value| {
            matches!(
                v,
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_))
            )
        };
        if text(&args[0]) && text(&args[args.len() - 1]) {
            // colon('a','e') is the character row 'abcde'
            let units: Vec<u16> = range.data().iter().map(|&v| v as u16).collect();
            return Ok(Value::Container(
                crate::CharacterRow::from_column_major(1, count, units, &self.cancel)?.into(),
            ));
        }
        Ok(Value::Matrix(range))
    }

    /// `flintmax`, `realmax` and `realmin` take only a class name.
    fn matbuild_constant(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        arity(args, 0, 1)?;
        match args.first() {
            None => {}
            Some(Value::Text(c) | Value::QuotedString(c)) if c == "double" => {}
            Some(Value::Text(c) | Value::QuotedString(c)) if c == "single" => {
                return Err(format!(
                    "{name}('single') is not implemented: single-precision arrays are not supported"
                ));
            }
            Some(_) => return Err("Input must be 'single' or 'double'.".into()),
        }
        let value = match name {
            "flintmax" => 9_007_199_254_740_992.,
            "realmax" => f64::MAX,
            _ => f64::MIN_POSITIVE,
        };
        matrix_value(1, 1, vec![value])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ind2sub_checks_the_budget_before_allocating() {
        let args = [
            Value::Matrix(Matrix::from_column_vec(1, 2, vec![3., 4.]).unwrap()),
            Value::Matrix(Matrix::from_column_vec(1, 3, vec![1., 5., 12.]).unwrap()),
        ];
        // two outputs of three elements need 6 units; 5 are left
        let mut engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 5,
            ..Engine::default()
        };
        let refused = engine.matbuild_outputs("ind2sub", &args, 2).unwrap_err();
        assert!(refused.contains("evaluation budget"), "{refused}");
        engine.evaluated_elements -= 1;
        assert_eq!(
            engine.matbuild_outputs("ind2sub", &args, 2).unwrap().len(),
            2
        );
    }
}
