//! Text/container grids share the rooted transaction and numeric conversion rules.
use super::*;
use crate::{StringArray, TableArray};
use std::sync::Arc;

pub(super) fn inline(value: &Array) -> bool {
    !value.is_nd()
        && (matches!(
            value,
            Array::Numeric(_) | Array::Logical(_) | Array::Typed(_)
        ) && value.len() == 1
            || matches!(value, Array::Character(t) if t.is_text_row())
            || matches!(value, Array::Str(s) if s.len() == 1))
}
pub(super) fn column_editable(value: &Array) -> bool {
    value.cols() == 1
        && match value {
            Array::Numeric(_) | Array::Logical(_) | Array::Typed(_) | Array::Str(_) => true,
            Array::Cell(c) => (0..c.len()).all(|i| {
                matches!(c.get_linear(i).map(AsRef::as_ref), Ok(Array::Character(t)) if t.is_text_row())
            }),
            _ => false,
        }
}
fn summary(value: &Array) -> String {
    format!("{}×{} {}", value.rows(), value.cols(), value.class())
}
/// The "R×C class" text `summary` writes for contents that have no literal.
fn is_summary(text: &str) -> bool {
    let Some((size, class)) = text.split_once(' ') else {
        return false;
    };
    let mut dims = size.split('×');
    !class.is_empty()
        && class.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && dims.clone().count() >= 2
        && dims.all(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}
fn content_text(value: &Array) -> Result<String> {
    match value {
        Array::Character(t) if t.is_text_row() => {
            // Invalid UTF-16 stays inspectable through the existing unit editor.
            Ok(t.to_utf8(&AtomicBool::new(false))
                .unwrap_or_else(|_| t.preview(128)))
        }
        Array::Str(s) if s.len() == 1 => Ok(s.data()[0].as_deref().unwrap_or("<missing>").into()),
        _ if inline(value) => cell_literal(value, 0, 0),
        _ => Ok(summary(value)),
    }
}
pub(super) fn literal(value: &Array, row: usize, col: usize) -> Result<String> {
    match value {
        Array::Str(s) => Ok(s.data()[col * s.rows() + row]
            .as_deref()
            .unwrap_or("<missing>")
            .into()),
        Array::Cell(c) => content_text(c.get_linear(col * c.rows() + row)?),
        Array::Table(t) => {
            let column = t.column(col);
            if column.cols() != 1 {
                return Ok(summary(column));
            }
            match column {
                Array::Numeric(_)
                | Array::Logical(_)
                | Array::Typed(_)
                | Array::Str(_)
                | Array::Cell(_)
                | Array::Character(_)
                | Array::Complex(_) => cell_literal(column, row, 0),
                _ => Ok(summary(column)),
            }
        }
        _ => Err("Not a container grid".into()),
    }
}
/// Painting never decodes or clones a complete large text element.
pub(super) fn preview(value: &Array, row: usize, col: usize) -> Result<String> {
    fn text(s: &str) -> String {
        let mut chars = s.chars();
        let mut out: String = chars.by_ref().take(128).collect();
        if chars.next().is_some() {
            out.push('…');
        }
        out
    }
    fn content(value: &Array) -> Result<String> {
        match value {
            Array::Character(t) if t.is_text_row() => {
                let mut units = char::decode_utf16(t.units().iter().copied());
                let mut out = String::new();
                for unit in units.by_ref().take(128) {
                    match unit {
                        Ok(c) => out.push(c),
                        Err(e) => out.push_str(&format!("\\u{:04X}", e.unpaired_surrogate())),
                    }
                }
                if units.next().is_some() {
                    out.push('…');
                }
                Ok(out)
            }
            Array::Str(s) if s.len() == 1 => {
                Ok(text(s.data()[0].as_deref().unwrap_or("<missing>")))
            }
            _ if inline(value) => cell_literal(value, 0, 0),
            _ => Ok(summary(value)),
        }
    }
    match value {
        Array::Str(s) => Ok(text(
            s.data()[col * s.rows() + row]
                .as_deref()
                .unwrap_or("<missing>"),
        )),
        Array::Cell(c) => content(c.get_linear(col * c.rows() + row)?),
        Array::Table(t) if t.column(col).cols() == 1 => {
            let column = t.column(col);
            if matches!(column, Array::Cell(_) | Array::Str(_)) {
                preview(column, row, 0)
            } else {
                literal(value, row, col)
            }
        }
        _ => literal(value, row, col),
    }
}
pub(super) fn missing_at(value: &Array, row: usize, col: usize) -> bool {
    match value {
        Array::Str(s) => s.data()[col * s.rows() + row].is_none(),
        Array::Cell(c) => c.get_linear(col * c.rows() + row).is_ok_and(
            |v| matches!(v.as_ref(), Array::Str(s) if s.len() == 1 && s.data()[0].is_none()),
        ),
        Array::Table(t) if t.column(col).cols() == 1 => missing_at(t.column(col), row, 0),
        _ => false,
    }
}
pub(super) fn tsv_field(text: &str, quote: bool) -> String {
    if quote || text.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.into()
    }
}
struct TextPatch {
    rows: usize,
    cols: usize,
    fields: Vec<String>, // row-major, including empty text
    quoted: Vec<bool>,   // quoted <missing> is ordinary text
}
/// Excel quoting, CRLF, trailing empty fields and one optional final line break.
fn parse(
    text: &str,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<TextPatch> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("Clipboard text exceeds 8 MiB".into());
    }
    let bytes = text.as_bytes();
    let mut fields = Vec::new();
    let mut quoted = Vec::new();
    let (mut i, mut rows, mut cols, mut width) = (0, 0, 0, None);
    loop {
        checkpoint(cancel, progress, Stage::ParseProgress)?;
        if fields.len() == MAX_CELLS {
            return Err("Clipboard exceeds 65536 cells".into());
        }
        let mut field = String::new();
        let is_quoted = bytes.get(i) == Some(&b'"');
        if is_quoted {
            i += 1;
            let mut start = i;
            loop {
                if i.is_multiple_of(CHUNK) {
                    crate::dsp::check_cancel(cancel)?;
                }
                match bytes.get(i) {
                    None => return Err("Unclosed quoted clipboard field".into()),
                    Some(b'"') => {
                        field.push_str(&text[start..i]);
                        i += 1;
                        if bytes.get(i) == Some(&b'"') {
                            field.push('"');
                            i += 1;
                            start = i;
                        } else {
                            break;
                        }
                    }
                    _ => i += 1,
                }
            }
            if bytes.get(i).is_some_and(|b| !b"\t\r\n".contains(b)) {
                return Err("Unexpected text after quoted clipboard field".into());
            }
        } else {
            let start = i;
            while bytes.get(i).is_some_and(|b| !b"\t\r\n".contains(b)) {
                if i.is_multiple_of(CHUNK) {
                    crate::dsp::check_cancel(cancel)?;
                }
                i += 1;
            }
            field.push_str(&text[start..i]);
        }
        fields.push(field);
        quoted.push(is_quoted);
        cols += 1;
        if bytes.get(i) == Some(&b'\t') {
            i += 1;
            continue;
        }
        if width.is_some_and(|n| n != cols) {
            return Err("Clipboard rows must have equal column counts".into());
        }
        width = Some(cols);
        rows += 1;
        cols = 0;
        if i == bytes.len() {
            break;
        }
        if bytes[i] == b'\r' {
            if bytes.get(i + 1) != Some(&b'\n') {
                return Err("Clipboard contains a bare carriage return".into());
            }
            i += 1;
        }
        i += 1;
        if i == bytes.len() {
            break;
        }
    }
    checkpoint(cancel, progress, Stage::Parsed)?;
    Ok(TextPatch {
        rows,
        cols: width.unwrap(),
        fields,
        quoted,
    })
}
fn text_array(text: &str, cancel: &AtomicBool) -> Result<Array> {
    Ok(CharacterRow::from_text(text, cancel)?.into())
}
fn double_or_text(text: &str, cancel: &AtomicBool) -> Result<Array> {
    let trimmed = text.trim();
    // A double only when its short display is the field. `1` is 1; `001` is text.
    if let Ok(Scalar::Real(x)) = Kind::Real.parse(trimmed)
        && scalar_literal(Scalar::Real(x)) == trimmed
    {
        return Ok(Matrix::from_column_vec(1, 1, vec![x])?.into());
    }
    text_array(text, cancel)
}
/// Body of a quoted paste field. TSV quotes are already removed when `quoted`.
/// A MATLAB literal `'abc'` or `"abc"` is one more pair. Spaces inside stay.
fn quoted_body(text: &str, quoted: bool) -> Option<&str> {
    if quoted {
        return Some(text);
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'\'' || bytes[0] == b'"')
        && bytes[0] == bytes[bytes.len() - 1]
    {
        Some(&text[1..text.len() - 1])
    } else {
        None
    }
}
fn is_text_element(value: &Array) -> bool {
    matches!(value, Array::Character(_) | Array::Str(_))
}
fn as_text(dest: Option<&Array>, text: &str, cancel: &AtomicBool) -> Result<Array> {
    if matches!(dest, Some(Array::Str(_))) {
        Ok(Array::Str(StringArray::scalar(text)?))
    } else {
        text_array(text, cancel)
    }
}
/// Paste into one cell. A numeric element takes every real literal (`1.0`,
/// `+1`, `1e2`, `Inf`). A char or string element, and any quoted field, stays
/// text. Spaces inside quotes are kept. Inline editing still uses
/// [`double_or_text`].
fn pasted_cell(
    dest: Option<&Array>,
    text: &str,
    quoted: bool,
    cancel: &AtomicBool,
) -> Result<Array> {
    if let Some(body) = quoted_body(text, quoted) {
        return as_text(dest, body, cancel);
    }
    if dest.is_some_and(is_text_element) {
        return as_text(dest, text, cancel);
    }
    let trimmed = text.trim();
    if let Ok(Scalar::Real(x)) = Kind::Real.parse(trimmed) {
        return Ok(Matrix::from_column_vec(1, 1, vec![x])?.into());
    }
    as_text(dest, text, cancel)
}
fn inline_value(old: &Array, text: &str, cancel: &AtomicBool) -> Result<Array> {
    if !inline(old) {
        return Err("Open this cell's contents to edit them".into());
    }
    match old {
        Array::Str(_) => Ok(Array::Str(StringArray::scalar(text)?)),
        Array::Character(_) => double_or_text(text, cancel),
        _ => {
            let patch = Patch {
                rows: 1,
                cols: 1,
                values: vec![Kind::of(old)?.parse(text)?],
            };
            numeric_patch(old, Rect::new(0, 0, 1, 1)?, &patch, cancel, &mut |_| Ok(()))
        }
    }
}
fn string_item(text: &str) -> Option<Arc<str>> {
    (text != "<missing>").then(|| Arc::from(text))
}
pub(super) fn edit(
    leaf: &Array,
    selection: Rect,
    input: Input<'_>,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<(Array, Rect)> {
    let inline = matches!(input, Input::Literal(_));
    let patch = match input {
        Input::Tsv(t) => parse(t, cancel, progress)?,
        Input::Literal(t) => {
            if t.len() > MAX_TEXT_BYTES {
                return Err("Cell text exceeds 8 MiB".into());
            }
            TextPatch {
                rows: 1,
                cols: 1,
                fields: vec![t.into()],
                quoted: vec![true],
            }
        }
        Input::Prepared(_) => return Err("This grid requires text entries".into()),
    };
    let rect = Rect::new(selection.row, selection.col, patch.rows, patch.cols)?;
    let (rows, cols) = (
        leaf.rows().max(rect.row + rect.rows),
        leaf.cols().max(rect.col + rect.cols),
    );
    if let Array::Table(t) = leaf {
        if cols > t.width() {
            return Err("Paste cannot add table columns".into());
        }
        let n = rows.checked_mul(cols).ok_or("Table dimensions overflow")?;
        if rows > MAX_GROWN_CELLS && rows > t.height() || n > MAX_GROWN_CELLS && n > t.len() {
            return Err("Table exceeds the editor growth limit".into());
        }
        // Parse every editable field before allocating a grown table. Unsupported
        // columns reject the complete transaction, including preceding good fields.
        let mut replacements = Vec::new();
        for c in 0..patch.cols {
            crate::dsp::check_cancel(cancel)?;
            let column = t.column(rect.col + c);
            if !column_editable(column) {
                return Err(format!(
                    "Table column '{}' is read-only",
                    t.names()[rect.col + c]
                ));
            }
            let fields = (0..patch.rows)
                .map(|r| patch.fields[r * patch.cols + c].clone())
                .collect();
            let cp = TextPatch {
                rows: patch.rows,
                cols: 1,
                fields,
                quoted: (0..patch.rows)
                    .map(|r| patch.quoted[r * patch.cols + c])
                    .collect(),
            };
            let cr = Rect::new(rect.row, 0, rect.rows, 1)?;
            let updated = match column {
                Array::Numeric(_) | Array::Logical(_) | Array::Typed(_) => {
                    let kind = Kind::of(column)?;
                    let values = cp
                        .fields
                        .iter()
                        .map(|f| kind.parse(f))
                        .collect::<Result<Vec<_>>>()?;
                    numeric_patch(
                        column,
                        cr,
                        &Patch {
                            rows: cp.rows,
                            cols: 1,
                            values,
                        },
                        cancel,
                        progress,
                    )?
                }
                _ => write_text(column, rows, 1, cr, &cp, inline, true, cancel)?,
            };
            replacements.push(updated);
        }
        let mut parts = if rows != leaf.rows() {
            let Array::Table(grown) = remap(
                leaf,
                rows,
                cols,
                &|r, c| (r < leaf.rows()).then_some(c * leaf.rows() + r),
                cancel,
            )?
            else {
                unreachable!()
            };
            grown.to_parts()
        } else {
            t.to_parts()
        };
        for (c, updated) in replacements.into_iter().enumerate() {
            parts.columns[rect.col + c] = updated;
        }
        checkpoint(cancel, progress, Stage::PatchProgress)?;
        return Ok((TableArray::new(parts)?.into(), rect));
    }
    let result = write_text(leaf, rows, cols, rect, &patch, inline, false, cancel)?;
    checkpoint(cancel, progress, Stage::PatchProgress)?;
    Ok((result, rect))
}
#[allow(clippy::too_many_arguments)]
fn write_text(
    leaf: &Array,
    rows: usize,
    cols: usize,
    rect: Rect,
    patch: &TextPatch,
    inline: bool,
    cellstr: bool,
    cancel: &AtomicBool,
) -> Result<Array> {
    let source = |r, c| (r < leaf.rows() && c < leaf.cols()).then_some(c * leaf.rows() + r);
    let field = |r, c| {
        if r >= rect.row && r < rect.row + rect.rows && c >= rect.col && c < rect.col + rect.cols {
            let i = (r - rect.row) * patch.cols + c - rect.col;
            Some((&patch.fields[i], patch.quoted[i]))
        } else {
            None
        }
    };
    match leaf {
        Array::Str(s) => {
            let mut data = Vec::new();
            data.try_reserve_exact(StringArray::checked_len(rows, cols)?)
                .map_err(|_| "Cannot allocate string grid")?;
            for c in 0..cols {
                for r in 0..rows {
                    if data.len().is_multiple_of(CHUNK) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    data.push(match field(r, c) {
                        Some((t, quoted)) => {
                            if inline || quoted {
                                Some(Arc::from(t.as_str()))
                            } else {
                                string_item(t)
                            }
                        }
                        None => source(r, c).and_then(|i| s.data()[i].clone()),
                    });
                }
            }
            Ok(Array::Str(StringArray::new(rows, cols, data)?))
        }
        Array::Cell(old) => {
            let n = rows
                .checked_mul(cols)
                .filter(|&n| n <= MAX_CELLS && rows <= MAX_CELLS && cols <= MAX_CELLS)
                .ok_or("Cell shape exceeds 65536 slots")?;
            let zero: Arc<Array> = Arc::new(if cellstr {
                text_array("", cancel)?
            } else {
                Matrix::filled(0, 0, 0.)?.into()
            });
            let mut data = Vec::new();
            data.try_reserve_exact(n)
                .map_err(|_| "Cannot allocate cell grid")?;
            for c in 0..cols {
                for r in 0..rows {
                    if data.len().is_multiple_of(256) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    let kept = match (field(r, c), source(r, c)) {
                        (Some((t, quoted)), Some(i)) if !cellstr && !inline && !quoted => {
                            let element = old.get_linear(i)?;
                            (content_text(element)? == *t).then(|| element.clone())
                        }
                        _ => None,
                    };
                    if let Some(element) = kept {
                        // Pasting a cell's own copy text keeps its value and type.
                        data.push(element);
                        continue;
                    }
                    data.push(match field(r, c) {
                        Some((t, _)) if !cellstr && !inline && is_summary(t) => {
                            return Err(format!(
                                "'{t}' only describes a cell's contents; open the cell to copy its values"
                            ));
                        }
                        Some((t, quoted)) => Arc::new(if cellstr {
                            text_array(t, cancel)?
                        } else if inline {
                            inline_value(
                                old.get_linear(
                                    source(r, c).ok_or("Inline cell is outside the array")?,
                                )?,
                                t,
                                cancel,
                            )?
                        } else {
                            let dest = match source(r, c) {
                                Some(i) => Some(old.get_linear(i)?.as_ref()),
                                None => None,
                            };
                            pasted_cell(dest, t, quoted, cancel)?
                        }),
                        None => source(r, c)
                            .map(|i| old.get_linear(i).cloned())
                            .transpose()?
                            .unwrap_or_else(|| zero.clone()),
                    });
                }
            }
            Ok(CellArray::from_shared(rows, cols, data, cancel)?.into())
        }
        _ => Err("Not a text grid".into()),
    }
}
pub(super) fn remap_container(
    leaf: &Array,
    rows: usize,
    cols: usize,
    from: &dyn Fn(usize, usize) -> Option<usize>,
    cancel: &AtomicBool,
) -> Result<Array> {
    let n = rows.checked_mul(cols).ok_or("Array dimensions overflow")?;
    match leaf {
        Array::Str(_) => {
            StringArray::checked_len(rows, cols)?;
        }
        Array::Cell(_) if rows > MAX_CELLS || cols > MAX_CELLS || n > MAX_CELLS => {
            return Err("Cell shape exceeds 65536 slots".into());
        }
        Array::Table(_) if rows > MAX_GROWN_CELLS && rows > leaf.rows() => {
            return Err("Table height exceeds the editor growth limit".into());
        }
        _ => {}
    }
    if let Array::Table(t) = leaf {
        return remap_table(t, rows, cols, from, cancel);
    }
    let mut positions = Vec::new();
    positions
        .try_reserve_exact(n)
        .map_err(|_| "Cannot allocate remapped grid")?;
    for c in 0..cols {
        for r in 0..rows {
            if positions.len().is_multiple_of(256) {
                crate::dsp::check_cancel(cancel)?;
            }
            positions.push(from(r, c));
        }
    }
    match leaf {
        Array::Str(s) => Ok(Array::Str(StringArray::new(
            rows,
            cols,
            positions
                .iter()
                .map(|p| p.and_then(|i| s.data()[i].clone()))
                .collect(),
        )?)),
        Array::Cell(cell) => {
            let zero: Arc<Array> = Arc::new(Matrix::filled(0, 0, 0.)?.into());
            let data = positions
                .iter()
                .map(|p| {
                    p.map(|i| cell.get_linear(i).cloned())
                        .transpose()
                        .map(|v| v.unwrap_or_else(|| zero.clone()))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(CellArray::from_shared(rows, cols, data, cancel)?.into())
        }
        Array::Datetime(a) => Ok(a
            .with_data(
                rows,
                cols,
                positions
                    .iter()
                    .map(|p| p.map_or(crate::datetime_array::DateTimeValue::NAT, |i| a.data()[i]))
                    .collect(),
            )?
            .into()),
        Array::Duration(a) if a.is_calendar() => Ok(crate::DurationArray::new_calendar(
            rows,
            cols,
            positions
                .iter()
                .map(|p| p.map_or([0.; 4], |i| a.calendar_data().unwrap()[i]))
                .collect(),
        )?
        .with_format(a.format())
        .into()),
        Array::Duration(a) => Ok(a
            .with_data(
                rows,
                cols,
                positions
                    .iter()
                    .map(|p| p.map_or(0., |i| a.data()[i]))
                    .collect(),
            )?
            .into()),
        Array::Categorical(a) => Ok(a
            .with_codes(
                rows,
                cols,
                positions
                    .iter()
                    .map(|p| p.map_or(0, |i| a.codes()[i]))
                    .collect(),
            )?
            .into()),
        Array::Struct(a) => {
            let empty: Arc<Array> = Arc::new(Matrix::filled(0, 0, 0.)?.into());
            let mut slots = Vec::new();
            slots
                .try_reserve_exact(crate::StructArray::slot_total(n, a.field_count())?)
                .map_err(|_| "Cannot allocate struct rows")?;
            for (k, p) in positions.iter().enumerate() {
                if k.is_multiple_of(256) {
                    crate::dsp::check_cancel(cancel)?;
                }
                for f in 0..a.field_count() {
                    slots.push(
                        p.map(|i| a.get(f, i).cloned())
                            .transpose()?
                            .unwrap_or_else(|| empty.clone()),
                    );
                }
            }
            Ok(
                crate::StructArray::from_slots(
                    rows,
                    cols,
                    a.field_names().to_vec(),
                    slots,
                    cancel,
                )?
                .into(),
            )
        }
        _ => Err("This table column cannot be resized in the editor".into()),
    }
}

fn remap_table(
    t: &TableArray,
    rows: usize,
    cols: usize,
    from: &dyn Fn(usize, usize) -> Option<usize>,
    cancel: &AtomicBool,
) -> Result<Array> {
    if cols != t.width() {
        return Err("Table columns cannot change in the editor".into());
    }
    let mut parts = t.to_parts();
    for (c, column) in parts.columns.iter_mut().enumerate() {
        crate::dsp::check_cancel(cancel)?;
        let map = |r: usize, cc: usize| from(r, 0).map(|i| cc * t.height() + i);
        *column = if matches!(column, Array::Cell(_)) && column_editable(column) {
            let Array::Cell(cell) = column else {
                unreachable!()
            };
            let zero: Arc<Array> = Arc::new(text_array("", cancel)?);
            let data = (0..rows)
                .map(|r| {
                    map(r, 0)
                        .map(|i| cell.get_linear(i).cloned())
                        .transpose()
                        .map(|v| v.unwrap_or_else(|| zero.clone()))
                })
                .collect::<Result<Vec<_>>>()?;
            CellArray::from_shared(rows, 1, data, cancel)?.into()
        } else {
            remap(column, rows, column.cols(), &map, cancel)
                .map_err(|e| format!("Table column '{}': {e}", t.names()[c]))?
        };
    }
    if let Some(names) = &mut parts.row_names {
        let old = names.clone();
        let mut used: std::collections::HashSet<String> = old.iter().cloned().collect();
        let mut serial = 1usize;
        *names = (0..rows)
            .map(|r| {
                from(r, 0).map_or_else(
                    || {
                        loop {
                            let name = format!("Row{serial}");
                            serial += 1;
                            if used.insert(name.clone()) {
                                break name;
                            }
                        }
                    },
                    |i| old[i].clone(),
                )
            })
            .collect();
    }
    parts.height = rows;
    Ok(TableArray::new(parts)?.into())
}
