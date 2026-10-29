//! Bounded literal clipboard and atomic edits of numeric and UTF-16 workspace descendants.
//! No script evaluation, UI dependency, background thread or thread pool.
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix,
    Result, Scalar,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

#[cfg(test)]
mod cell_normalization_tests;
pub mod cell_replace;

pub const MAX_PATH: usize = 16;
pub const MAX_CELLS: usize = 65_536;
pub const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const CHUNK: usize = 4096;

/// A process-local immutable expectation, not a serialized object address.
#[derive(Clone, Debug)]
pub struct Target {
    name: String,
    expected: Array,
    path: Vec<usize>,
}
impl Target {
    pub fn new(name: String, expected: Array, path: Vec<usize>) -> Result<Self> {
        if !crate::files::valid_name(&name)
            || crate::syntax::keyword(&name)
            || path.len() > MAX_PATH
            || path.capacity() > MAX_PATH
        {
            return Err("Invalid variable name or cell path (maximum 16 levels)".into());
        }
        let target = Self {
            name,
            expected,
            path,
        };
        Kind::of(target.leaf()?)?;
        Ok(target)
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn expected_root(&self) -> &Array {
        &self.expected
    }
    pub fn path(&self) -> &[usize] {
        &self.path
    }
    pub fn leaf(&self) -> Result<&Array> {
        descend(&self.expected, &self.path)
    }
    fn current(&self, engine: &Engine) -> Result<Array> {
        let value = engine
            .array(&self.name)
            .ok_or("Variable no longer exists; reopen the editor")?;
        if !value.same_snapshot(&self.expected) {
            return Err("Variable changed; reopen the editor before applying or copying".into());
        }
        Ok(value)
    }
}
fn descend<'a>(mut value: &'a Array, path: &[usize]) -> Result<&'a Array> {
    if path.len() > MAX_PATH {
        return Err("Cell path exceeds 16 levels".into());
    }
    for &index in path {
        let Array::Cell(cell) = value else {
            return Err("Cell path no longer refers to a cell array".into());
        };
        value = cell.get_linear(index)?.as_ref();
    }
    Ok(value)
}

/// Zero-based fixed-shape selection; empty rectangles are not editable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}
impl Rect {
    pub fn new(row: usize, col: usize, rows: usize, cols: usize) -> Result<Self> {
        if rows == 0
            || cols == 0
            || rows.checked_mul(cols).is_none_or(|n| n > MAX_CELLS)
            || row.checked_add(rows).is_none()
            || col.checked_add(cols).is_none()
        {
            return Err("Select a nonempty rectangle of at most 65536 cells".into());
        }
        Ok(Self {
            row,
            col,
            rows,
            cols,
        })
    }
    pub fn row(self) -> usize {
        self.row
    }
    pub fn col(self) -> usize {
        self.col
    }
    pub fn rows(self) -> usize {
        self.rows
    }
    pub fn cols(self) -> usize {
        self.cols
    }
    pub fn cells(self) -> usize {
        self.rows * self.cols
    }
    fn fits(self, value: &Array) -> Result<()> {
        Kind::of(value)?;
        if self.row + self.rows > value.rows() || self.col + self.cols > value.cols() {
            return Err("Selection exceeds the existing array dimensions; editor paste does not grow arrays".into());
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct Committed {
    pub root: Array,
    pub rectangle: Rect,
}
#[derive(Clone, Copy)]
enum Kind {
    Typed(crate::NumericClass),
    Real,
    Logical,
    Complex,
    Character,
}
impl Kind {
    fn of(value: &Array) -> Result<Self> {
        match value {
            Array::Numeric(_) => Ok(Self::Real),
            Array::Typed(a) => Ok(Self::Typed(a.class())),
            Array::Logical(_) => Ok(Self::Logical),
            Array::Complex(_) => Ok(Self::Complex),
            Array::Character(_) => Ok(Self::Character),
            _ => Err("Only numeric or character leaves can be edited".into()),
        }
    }
    fn normalize(self, scalar: Scalar) -> Result<Scalar> {
        Ok(match self {
            Self::Character => {
                let Scalar::Real(value) = scalar else {
                    return Err("A UTF-16 unit requires an integer from 0 to 65535".into());
                };
                if !value.is_finite() || !(0. ..=65535.).contains(&value) || value.fract() != 0. {
                    return Err("A UTF-16 unit requires an integer from 0 to 65535".into());
                }
                Scalar::Real(value)
            }
            Self::Typed(class) if class.is_integer() => {
                Scalar::Integer(scalar.into_integer(class)?)
            }
            Self::Typed(_) => Scalar::Real(f64::from(scalar.into_real()? as f32)),
            Self::Real => Scalar::Real(scalar.into_real()?),
            Self::Complex => Scalar::Complex(scalar.into_complex()),
            Self::Logical => {
                let value = scalar.into_real()?;
                if value.is_nan() {
                    return Err("NaN cannot be converted to logical".into());
                }
                Scalar::Logical(value != 0.)
            }
        })
    }
    fn parse(self, field: &str) -> Result<Scalar> {
        if let Self::Typed(class) = self {
            return self.normalize(Scalar::parse_typed_cell(field, class)?);
        }
        if matches!(self, Self::Character) {
            if field.is_empty() || field.len() > 256 {
                return Err("A UTF-16 unit literal requires 1..256 bytes".into());
            }
            let field = field.trim();
            let unit = if let Some(hex) = field
                .strip_prefix("\\u")
                .or_else(|| field.strip_prefix("0x"))
            {
                if hex.len() != 4 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(
                        "Use \\uXXXX, 0xXXXX or a decimal UTF-16 unit from 0 to 65535".into(),
                    );
                }
                u16::from_str_radix(hex, 16).map_err(|_| "Invalid UTF-16 unit")?
            } else {
                if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(
                        "Use \\uXXXX, 0xXXXX or a decimal UTF-16 unit from 0 to 65535".into(),
                    );
                }
                field
                    .parse::<u16>()
                    .map_err(|_| "UTF-16 unit must be from 0 to 65535")?
            };
            return Ok(Scalar::Real(f64::from(unit)));
        }
        self.normalize(Scalar::parse_cell(
            field,
            matches!(self, Self::Complex),
            matches!(self, Self::Logical),
        )?)
    }
}
struct Patch {
    rows: usize,
    cols: usize,
    values: Vec<Scalar>,
}
enum Input<'a> {
    Tsv(&'a str),
    Literal(&'a str),
    Prepared(&'a [Scalar]),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    ValidatedRoot,
    ParseProgress,
    Parsed,
    CopyProgress,
    PatchProgress,
    Ancestor,
    BeforeCommit,
    FormatProgress,
}
fn checkpoint(
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Stage) -> Result<()>,
    stage: Stage,
) -> Result<()> {
    progress(stage)?;
    crate::dsp::check_cancel(cancel)
}

fn push_field(
    values: &mut Vec<Scalar>,
    field: &str,
    row: usize,
    col: usize,
    kind: Kind,
) -> Result<()> {
    if values.len() == MAX_CELLS {
        return Err("Clipboard exceeds 65536 cells".into());
    }
    if field.is_empty() || field.len() > 256 {
        return Err(format!(
            "Clipboard row {}, column {} requires a literal of 1..256 bytes",
            row + 1,
            col + 1
        ));
    }
    let value = kind
        .parse(field)
        .map_err(|e| format!("Clipboard row {}, column {}: {e}", row + 1, col + 1))?;
    if values.len() == values.capacity() {
        // Double the populated scratch at exhaustion, with a bounded initial chunk.
        // At most 65536 logical slots; avoid repeated fixed-size reallocations.
        values
            .try_reserve_exact((MAX_CELLS - values.len()).min(values.len().max(256)))
            .map_err(|_| "Cannot allocate clipboard values")?;
    }
    values.push(value);
    Ok(())
}
fn parse_tsv(
    text: &str,
    kind: Kind,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<Patch> {
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err("Clipboard text must contain 1 byte to 8 MiB".into());
    }
    let bytes = text.as_bytes();
    let mut values = Vec::new();
    let (mut first, mut i, mut row, mut col) = (0, 0, 0, 0);
    let mut columns = None;
    let mut next_check = 0;
    while i < bytes.len() {
        if i >= next_check {
            checkpoint(cancel, progress, Stage::ParseProgress)?;
            next_check = i + CHUNK;
        }
        let newline = bytes[i] == b'\n' || bytes[i] == b'\r';
        if newline || bytes[i] == b'\t' {
            if bytes[i] == b'\r' && bytes.get(i + 1) != Some(&b'\n') {
                return Err("Clipboard contains a bare carriage return".into());
            }
            push_field(&mut values, &text[first..i], row, col, kind)?;
            col += 1;
            if newline {
                if columns.is_some_and(|n| n != col) {
                    return Err("Clipboard rows must have equal column counts".into());
                }
                columns = Some(col);
                row += 1;
                col = 0;
            }
            i += if bytes[i] == b'\r' { 2 } else { 1 };
            first = i;
        } else {
            i += 1;
            if i - first > 256 {
                return Err(format!(
                    "Clipboard row {}, column {} exceeds 256 bytes",
                    row + 1,
                    col + 1
                ));
            }
        }
    }
    if first < text.len() {
        push_field(&mut values, &text[first..], row, col, kind)?;
        col += 1;
        if columns.is_some_and(|n| n != col) {
            return Err("Clipboard rows must have equal column counts".into());
        }
        columns = Some(col);
        row += 1;
    } else if col != 0 {
        return Err("Clipboard has an empty trailing field".into());
    }
    checkpoint(cancel, progress, Stage::Parsed)?;
    Ok(Patch {
        rows: row,
        cols: columns.ok_or("Empty clipboard")?,
        values,
    })
}

/// TSV dimensions define an insertion rectangle only for a one-cell selection.
/// Larger selections require an exact rectangular shape, without broadcasting.
pub fn paste_tsv(
    engine: &mut Engine,
    target: &Target,
    selection: Rect,
    text: &str,
) -> Result<Committed> {
    apply(engine, target, selection, Input::Tsv(text), &mut |_| Ok(()))
}
pub fn apply_literal(
    engine: &mut Engine,
    target: &Target,
    row: usize,
    col: usize,
    literal: &str,
) -> Result<Committed> {
    apply(
        engine,
        target,
        Rect::new(row, col, 1, 1)?,
        Input::Literal(literal),
        &mut |_| Ok(()),
    )
}
/// Row-major already parsed values; useful for typed consumers and benchmark controls.
/// This still validates types/count, expected root and the complete transaction.
pub fn apply_values(
    engine: &mut Engine,
    target: &Target,
    rectangle: Rect,
    values: &[Scalar],
) -> Result<Committed> {
    apply(
        engine,
        target,
        rectangle,
        Input::Prepared(values),
        &mut |_| Ok(()),
    )
}
fn apply(
    engine: &mut Engine,
    target: &Target,
    selection: Rect,
    input: Input<'_>,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<Committed> {
    let cancel = engine.cancellation();
    crate::dsp::check_cancel(&cancel)?;
    // Validate the whole immutable root before parsing or numeric allocation.
    let root = target.current(engine)?;
    let leaf = descend(&root, &target.path)?;
    selection.fits(leaf)?;
    let kind = Kind::of(leaf)?;
    checkpoint(&cancel, progress, Stage::ValidatedRoot)?;
    let patch = match input {
        Input::Tsv(text) => parse_tsv(text, kind, &cancel, progress)?,
        Input::Literal(text) => {
            let value = kind.parse(text)?;
            checkpoint(&cancel, progress, Stage::Parsed)?;
            Patch {
                rows: 1,
                cols: 1,
                values: vec![value],
            }
        }
        Input::Prepared(values) => {
            if values.len() != selection.cells() {
                return Err("Replacement count must match the selected rectangle".into());
            }
            let mut converted = Vec::new();
            converted
                .try_reserve_exact(values.len())
                .map_err(|_| "Cannot allocate replacement values")?;
            for (i, &value) in values.iter().enumerate() {
                if i.is_multiple_of(CHUNK) {
                    checkpoint(&cancel, progress, Stage::ParseProgress)?;
                }
                converted.push(kind.normalize(value)?);
            }
            checkpoint(&cancel, progress, Stage::Parsed)?;
            Patch {
                rows: selection.rows,
                cols: selection.cols,
                values: converted,
            }
        }
    };
    if selection.cells() != 1 && (selection.rows, selection.cols) != (patch.rows, patch.cols) {
        return Err("Clipboard dimensions must match the selected rectangle".into());
    }
    let rectangle = Rect::new(selection.row, selection.col, patch.rows, patch.cols)?;
    rectangle.fits(leaf)?;
    let replacement = match leaf {
        Array::Typed(a) => {
            let source = if a.is_integer() {
                crate::TypedArray::from_ints(
                    a.class(),
                    1,
                    patch.values.len(),
                    patch.values.iter().map(|v| match v {
                        Scalar::Integer(n) => *n,
                        _ => unreachable!("normalized integer patch"),
                    }),
                )?
            } else {
                crate::TypedArray::from_reals(
                    a.class(),
                    1,
                    patch.values.len(),
                    patch.values.iter().map(|v| match v {
                        Scalar::Real(n) => *n,
                        _ => unreachable!("normalized single patch"),
                    }),
                )?
            };
            let mut result = a.clone();
            result.write_indices(
                rectangle.cells(),
                |i| rectangle.row + i / patch.cols + (rectangle.col + i % patch.cols) * a.rows(),
                &source,
                false,
                &cancel,
            )?;
            result.into()
        }
        Array::Numeric(a) => Matrix::from_column_vec(
            a.rows(),
            a.cols(),
            copy_patch(
                a.data(),
                a.rows(),
                rectangle,
                &patch,
                &cancel,
                progress,
                |v| {
                    if let Scalar::Real(x) = v {
                        x
                    } else {
                        unreachable!("normalized real patch")
                    }
                },
            )?,
        )?
        .into(),
        Array::Logical(a) => LogicalMatrix::from_canonical_vec(
            a.rows(),
            a.cols(),
            copy_patch(
                a.data(),
                a.rows(),
                rectangle,
                &patch,
                &cancel,
                progress,
                |v| {
                    if let Scalar::Logical(x) = v {
                        u8::from(x)
                    } else {
                        unreachable!("normalized logical patch")
                    }
                },
            )?,
        )?
        .into(),
        Array::Complex(a) => ComplexMatrix::from_column_vec(
            a.rows(),
            a.cols(),
            copy_patch(
                a.data(),
                a.rows(),
                rectangle,
                &patch,
                &cancel,
                progress,
                |v| {
                    if let Scalar::Complex(x) = v {
                        x
                    } else {
                        unreachable!("normalized complex patch")
                    }
                },
            )?,
            &cancel,
        )?
        .into(),
        Array::Character(a) => CharacterRow::from_column_major(
            a.rows(),
            a.cols(),
            copy_patch(
                a.units(),
                a.rows(),
                rectangle,
                &patch,
                &cancel,
                progress,
                |v| {
                    let Scalar::Real(value) = v else {
                        unreachable!("normalized UTF-16 unit")
                    };
                    value as u16
                },
            )?,
            &cancel,
        )?
        .into(),
        _ => return Err("Only numeric or character leaves can be edited".into()),
    };
    let mut ancestors: Vec<(&CellArray, usize)> = Vec::new();
    ancestors
        .try_reserve_exact(target.path.len())
        .map_err(|_| "Cannot allocate bounded edit path")?;
    let mut parent = &root;
    for &index in &target.path {
        let Array::Cell(cell) = parent else {
            return Err("Cell path changed".into());
        };
        ancestors.push((cell, index));
        parent = cell.get_linear(index)?;
    }
    let mut updated: Array = replacement;
    for (cell, index) in ancestors.into_iter().rev() {
        updated = cell.replaced(index, updated, &cancel)?.into();
        checkpoint(&cancel, progress, Stage::Ancestor)?;
    }
    checkpoint(&cancel, progress, Stage::BeforeCommit)?;
    target.current(engine)?;
    // Existing workspace count/byte preflight and final Stop check own publication.
    engine.merge_workspace(BTreeMap::from([(target.name.clone(), updated)]))?;
    // Admission may normalize optional metadata. Adopt the published binding.
    let committed = engine
        .array(&target.name)
        .ok_or("Committed edit target is missing")?;
    Ok(Committed {
        root: committed,
        rectangle,
    })
}
fn copy_patch<T: Copy>(
    source: &[T],
    rows: usize,
    rectangle: Rect,
    patch: &Patch,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Stage) -> Result<()>,
    convert: impl Fn(Scalar) -> T,
) -> Result<Vec<T>> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(source.len())
        .map_err(|_| "Cannot allocate edited payload")?;
    for chunk in source.chunks(CHUNK) {
        crate::dsp::check_cancel(cancel)?;
        output.extend_from_slice(chunk);
        checkpoint(cancel, progress, Stage::CopyProgress)?;
    }
    for (i, &value) in patch.values.iter().enumerate() {
        if i.is_multiple_of(CHUNK) {
            crate::dsp::check_cancel(cancel)?;
        }
        let row = i / patch.cols;
        let col = i % patch.cols;
        output[(rectangle.col + col) * rows + rectangle.row + row] = convert(value);
        if (i + 1).is_multiple_of(CHUNK) || i + 1 == patch.values.len() {
            checkpoint(cancel, progress, Stage::PatchProgress)?;
        }
    }
    crate::dsp::check_cancel(cancel)?;
    Ok(output)
}

/// Bounded scientific literals round-trip finite binary64 values, including
/// signed zero and subnormals. NaNs preserve classification, not payload/sign.
pub fn scalar_literal(value: Scalar) -> String {
    fn real(x: f64) -> String {
        if x.is_nan() {
            "NaN".into()
        } else if x == f64::INFINITY {
            "inf".into()
        } else if x == f64::NEG_INFINITY {
            "-inf".into()
        } else {
            format!("{x:e}")
        }
    }
    match value {
        Scalar::Real(x) => real(x),
        Scalar::Integer(x) => x.to_string(),
        Scalar::Logical(x) => {
            if x {
                "1".into()
            } else {
                "0".into()
            }
        }
        Scalar::Complex(Complex64 { re, im }) => {
            let sign = if im.is_sign_negative() && !im.is_nan() {
                '-'
            } else {
                '+'
            };
            format!("{}{sign}{}i", real(re), real(im.abs()))
        }
    }
}
/// A lossless single-cell editor literal. Character units never pass through
/// Unicode scalar decoding, so isolated surrogates remain visible and editable.
pub fn cell_literal(value: &Array, row: usize, col: usize) -> Result<String> {
    if let Array::Character(text) = value {
        if row >= text.rows() || col >= text.cols() {
            return Err("Character cell is outside the array".into());
        }
        return Ok(format!("\\u{:04X}", text.units()[col * text.rows() + row]));
    }
    Ok(scalar_literal(value.scalar(row, col)?))
}

/// Copy a genuine text row only. Non-row shapes, NUL and unpaired surrogates
/// fail without clipboard publication; use escaped unit TSV for those values.
pub fn copy_text_row(engine: &Engine, target: &Target) -> Result<String> {
    let cancel = engine.cancellation();
    crate::dsp::check_cancel(&cancel)?;
    let root = target.current(engine)?;
    let Array::Character(text) = descend(&root, &target.path)? else {
        return Err("Copy text row requires a character row".into());
    };
    for chunk in text.units().chunks(CHUNK) {
        crate::dsp::check_cancel(&cancel)?;
        if chunk.contains(&0) {
            return Err(
                "Text contains NUL; use Copy cell or Copy selection for exact escaped UTF-16 units"
                    .into(),
            );
        }
    }
    let result = text.to_utf8(&cancel)?;
    crate::dsp::check_cancel(&cancel)?;
    target.current(engine)?;
    Ok(result)
}
pub fn copy_tsv(engine: &Engine, target: &Target, rectangle: Rect) -> Result<String> {
    copy_checked(engine, target, rectangle, &mut |_| Ok(()))
}
fn copy_checked(
    engine: &Engine,
    target: &Target,
    rectangle: Rect,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<String> {
    let cancel = engine.cancellation();
    crate::dsp::check_cancel(&cancel)?;
    let root = target.current(engine)?;
    let leaf = descend(&root, &target.path)?;
    rectangle.fits(leaf)?;
    checkpoint(&cancel, progress, Stage::ValidatedRoot)?;
    if let Array::Character(text) = leaf {
        let output = copy_character_tsv(text, rectangle, &cancel, progress)?;
        crate::dsp::check_cancel(&cancel)?;
        target.current(engine)?;
        return Ok(output);
    }
    let mut output = String::new();
    let mut append = |text: &str| -> Result<()> {
        if output
            .len()
            .checked_add(text.len())
            .is_none_or(|n| n > MAX_TEXT_BYTES)
        {
            return Err("Copied rectangle exceeds 8 MiB text limit".into());
        }
        output
            .try_reserve(text.len())
            .map_err(|_| "Cannot allocate clipboard text")?;
        output.push_str(text);
        Ok(())
    };
    for row in 0..rectangle.rows {
        for col in 0..rectangle.cols {
            let index = row * rectangle.cols + col;
            if index.is_multiple_of(256) {
                checkpoint(&cancel, progress, Stage::FormatProgress)?;
            }
            if col > 0 {
                append("\t")?;
            }
            append(&cell_literal(
                leaf,
                rectangle.row + row,
                rectangle.col + col,
            )?)?;
        }
        if row + 1 < rectangle.rows {
            append("\n")?;
        }
    }
    crate::dsp::check_cancel(&cancel)?;
    target.current(engine)?;
    Ok(output)
}

// Called only after the rectangle has been checked against the shared character view.
fn copy_character_tsv(
    text: &CharacterRow,
    rectangle: Rect,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<String> {
    // Every unit is six ASCII bytes; tabs and newlines together add cells - 1.
    let bytes = rectangle
        .cells()
        .checked_mul(7)
        .and_then(|n| n.checked_sub(1))
        .filter(|&n| n <= MAX_TEXT_BYTES)
        .ok_or("Copied rectangle exceeds 8 MiB text limit")?;
    let mut output = String::new();
    output
        .try_reserve_exact(bytes)
        .map_err(|_| "Cannot allocate clipboard text")?;
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for row in 0..rectangle.rows {
        for col in 0..rectangle.cols {
            let index = row * rectangle.cols + col;
            if index.is_multiple_of(256) {
                checkpoint(cancel, progress, Stage::FormatProgress)?;
            }
            if col > 0 {
                output.push('\t');
            }
            let unit = text.units()[(rectangle.col + col) * text.rows() + rectangle.row + row];
            output.push_str("\\u");
            output.push(char::from(HEX[usize::from(unit >> 12)]));
            output.push(char::from(HEX[usize::from((unit >> 8) & 15)]));
            output.push(char::from(HEX[usize::from((unit >> 4) & 15)]));
            output.push(char::from(HEX[usize::from(unit & 15)]));
        }
        if row + 1 < rectangle.rows {
            output.push('\n');
        }
    }
    Ok(output)
}

#[cfg(test)]
#[path = "variable_edit_tests.rs"]
mod tests;
