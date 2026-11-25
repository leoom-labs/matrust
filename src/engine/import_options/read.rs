//! Reading with import options: readtable(file, opts), readmatrix(file,
//! opts), readcell(file, opts), preview(file, opts), readtimetable and the
//! new readtable name/value options. Rows come from the text tokenizer
//! (detect.rs) or a worksheet; every selected variable is converted to its
//! type, then MissingRule and ImportErrorRule decide what happens to empty
//! and unconvertible fields.
use super::detect::{self, DetectArgs, Splitter};
use super::{ImportOpts, IoObject, SheetSel, VarOpts, VarType, char_array, numbers, text_of};
use crate::datetime_array::{DEFAULT_DATE_FORMAT, DEFAULT_FORMAT, DateTimeValue, NANOS_PER_DAY};
use crate::engine::{Engine, Value};
use crate::table_array::{TableArray, TableParts};
use crate::{
    Array, CellArray, DatetimeArray, DurationArray, LogicalMatrix, Matrix, NumericClass, Result,
    StringArray, TypedArray,
};
use std::sync::Arc;

/// Most cells (rows times variables) a read may produce.
const MAX_TABLE_CELLS: usize = 16 * 1024 * 1024;
/// Most worksheet cells read.
const MAX_SHEET_CELLS: usize = 4 * 1024 * 1024;
/// Rows preview shows.
const PREVIEW_ROWS: usize = 8;
/// readtable options handled by the original reader (table_fns/csv.rs).
const LEGACY_OPTIONS: [&str; 4] = [
    "delimiter",
    "readvariablenames",
    "numheaderlines",
    "texttype",
];

// ---------------------------------------------------------------------------
// Ranges

/// A zero-based spreadsheet (or text) range: a start cell, a rectangle, a
/// row range or a column range. `None` ends extend to the data's end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) struct Range {
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) last_row: Option<usize>,
    pub(super) last_col: Option<usize>,
    start_cell: bool,
}

impl Range {
    pub(super) fn cell(row: usize, col: usize) -> Self {
        Self {
            row,
            col,
            last_row: None,
            last_col: None,
            start_cell: true,
        }
    }
    pub(super) fn is_start_cell(&self) -> bool {
        self.start_cell
    }
}

fn column_number(letters: &str) -> Option<usize> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    let mut col = 0usize;
    for b in letters.bytes() {
        col = col * 26 + usize::from(b.to_ascii_uppercase() - b'A' + 1);
    }
    (col <= 16_384).then(|| col - 1)
}

fn row_number(digits: &str) -> Option<usize> {
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: usize = digits.parse().ok()?;
    (1..=1_048_576).contains(&row).then(|| row - 1)
}

fn cell_ref(text: &str) -> Option<(usize, usize)> {
    let text = text.trim();
    let split = text.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = text.split_at(split);
    Some((row_number(digits)?, column_number(letters)?))
}

/// 'B2', 'A1:C10', '2:10' or 'A:C'.
pub(in crate::engine) fn parse_range(text: &str) -> Option<Range> {
    let text = text.trim();
    match text.split_once(':') {
        None => cell_ref(text).map(|(r, c)| Range::cell(r, c)),
        Some((a, b)) => {
            if let (Some((r0, c0)), Some((r1, c1))) = (cell_ref(a), cell_ref(b)) {
                return Some(Range {
                    row: r0.min(r1),
                    col: c0.min(c1),
                    last_row: Some(r0.max(r1)),
                    last_col: Some(c0.max(c1)),
                    start_cell: false,
                });
            }
            if let (Some(r0), Some(r1)) = (row_number(a.trim()), row_number(b.trim())) {
                return Some(Range {
                    row: r0.min(r1),
                    col: 0,
                    last_row: Some(r0.max(r1)),
                    last_col: None,
                    start_cell: false,
                });
            }
            let (c0, c1) = (column_number(a.trim())?, column_number(b.trim())?);
            Some(Range {
                row: 0,
                col: c0.min(c1),
                last_row: None,
                last_col: Some(c0.max(c1)),
                start_cell: false,
            })
        }
    }
}

/// A1-style name of a zero-based cell.
pub(in crate::engine) fn cell_name(row: usize, col: usize) -> String {
    let mut letters = Vec::new();
    let mut c = col + 1;
    while c > 0 {
        letters.push(b'A' + ((c - 1) % 26) as u8);
        c = (c - 1) / 26;
    }
    letters.reverse();
    format!("{}{}", String::from_utf8_lossy(&letters), row + 1)
}

// ---------------------------------------------------------------------------
// Raw cells

/// One field before conversion: text (and whether it was quoted) or a
/// spreadsheet value.
#[derive(Clone, Debug)]
pub(in crate::engine) enum Raw {
    Empty,
    Num(f64),
    Bool(bool),
    Text(String, bool),
}

/// Cells of a worksheet from `window`'s top-left cell to the last used
/// cell (or the window's end), row by row.
pub(super) struct SheetGrid {
    pub(super) rows: Vec<Vec<Raw>>,
    pub(super) row0: usize,
    pub(super) col0: usize,
    pub(super) width: usize,
}

pub(super) fn sheet_grid(
    e: &Engine,
    path: &std::path::Path,
    sheet: &SheetSel,
    window: Range,
) -> Result<SheetGrid> {
    use crate::engine::readers::{CellRange, SheetCell, SheetChoice};
    let choice = match sheet {
        SheetSel::Name(n) if n.is_empty() => SheetChoice::First,
        SheetSel::Name(n) => SheetChoice::Name(n.clone()),
        SheetSel::Index(i) => SheetChoice::Index(*i),
    };
    let range = CellRange {
        row: window.row,
        col: window.col,
        last_row: window.last_row,
        last_col: window.last_col,
    };
    let grid = crate::engine::readers::rdx_read_sheet(
        path,
        &choice,
        Some(range),
        MAX_SHEET_CELLS,
        &e.cancel,
    )?;
    let mut rows = Vec::with_capacity(grid.rows);
    for r in 0..grid.rows {
        e.check_cancel()?;
        rows.push(
            (0..grid.cols)
                .map(|c| match grid.at(r, c) {
                    SheetCell::Number(x) => Raw::Num(*x),
                    SheetCell::Bool(b) => Raw::Bool(*b),
                    SheetCell::Text(t) if !t.is_empty() => Raw::Text(t.clone(), false),
                    _ => Raw::Empty,
                })
                .collect(),
        );
    }
    Ok(SheetGrid {
        rows,
        row0: grid.first_row,
        col0: window.col,
        width: grid.cols,
    })
}

/// Rows of the data to convert, with the names, units and row names read
/// from the file.
struct RawData {
    /// (1-based line or sheet row, fields)
    rows: Vec<(usize, Vec<Raw>)>,
    names: Option<Vec<String>>,
    units: Option<Vec<String>>,
    row_names: Option<Vec<String>>,
}

fn raw_text(raw: &Raw) -> Option<String> {
    match raw {
        Raw::Text(t, _) if !t.trim().is_empty() => Some(t.trim().to_owned()),
        Raw::Num(x) => Some(number_text(*x)),
        Raw::Bool(b) => Some(if *b { "true" } else { "false" }.into()),
        _ => None,
    }
}

fn number_text(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0. { "Inf".into() } else { "-Inf".into() }
    } else {
        format!("{x}")
    }
}

fn text_data(
    e: &Engine,
    filename: &str,
    path: &std::path::Path,
    o: &ImportOpts,
    limit: Option<usize>,
) -> Result<RawData> {
    let text = detect::file_text(e, filename, path)?;
    let splitter = Splitter::from_opts(o);
    let records = detect::split(&text, &splitter, None, &e.cancel)?;
    drop(text);
    let texts = |line: usize| -> Option<Vec<String>> {
        if line == 0 {
            return None;
        }
        let record = records.iter().find(|r| r.line == line)?;
        Some(
            record
                .fields
                .as_ref()?
                .iter()
                .map(|f| {
                    if f.quoted {
                        f.text.clone()
                    } else {
                        f.text
                            .trim_matches(|c| splitter.whitespace.contains(&c))
                            .to_owned()
                    }
                })
                .collect(),
        )
    };
    let names = texts(o.names_line);
    let units = texts(o.units_line);
    let width = o.vars.len().max(1);
    let mut rows = Vec::new();
    // The records move into the rows: fields are not copied.
    for record in records {
        if limit.is_some_and(|l| rows.len() >= l) {
            break;
        }
        let line = record.line as f64;
        if !o.data_lines.iter().any(|r| line >= r[0] && line <= r[1]) {
            continue;
        }
        match record.fields {
            None => match o.empty_line_rule.as_str() {
                "read" => rows.push((record.line, vec![Raw::Empty; o.vars.len()])),
                "error" => {
                    return Err(format!(
                        "Line {} is empty (EmptyLineRule is 'error')",
                        record.line
                    ));
                }
                _ => {}
            },
            Some(f) => rows.push((
                record.line,
                f.into_iter().map(|x| Raw::Text(x.text, x.quoted)).collect(),
            )),
        }
        if rows.len().saturating_mul(width) > MAX_TABLE_CELLS {
            return Err(format!(
                "The data has more than {MAX_TABLE_CELLS} fields; read fewer DataLines"
            ));
        }
    }
    Ok(RawData {
        rows,
        names,
        units,
        row_names: None,
    })
}

fn sheet_data(
    e: &Engine,
    path: &std::path::Path,
    o: &ImportOpts,
    limit: Option<usize>,
) -> Result<RawData> {
    let data = parse_range(&o.data_range)
        .ok_or_else(|| format!("DataRange '{}' is not a valid range", o.data_range))?;
    let grid = sheet_grid(e, path, &o.sheet_name, Range::cell(0, 0))?;
    let at = |r: usize, c: usize| -> Raw {
        grid.rows
            .get(r)
            .and_then(|row| row.get(c))
            .cloned()
            .unwrap_or(Raw::Empty)
    };
    let width = match data.last_col {
        Some(last) if !data.start_cell && data.last_row.is_some() => last + 1 - data.col,
        _ => o.vars.len(),
    };
    let last_row = data
        .last_row
        .unwrap_or(usize::MAX)
        .min(grid.rows.len().saturating_sub(1));
    let row_names_col = parse_range(&o.row_names_range).map(|r| r.col);
    let mut rows = Vec::new();
    let mut row_names = Vec::new();
    if data.row <= last_row && !grid.rows.is_empty() {
        for r in data.row..=last_row {
            e.check_cancel()?;
            if limit.is_some_and(|l| rows.len() >= l) {
                break;
            }
            let cells: Vec<Raw> = (0..width).map(|k| at(r, data.col + k)).collect();
            let name = row_names_col.map(|c| at(r, c));
            if cells.iter().all(|c| matches!(c, Raw::Empty))
                && name.as_ref().is_none_or(|n| matches!(n, Raw::Empty))
            {
                continue;
            }
            if let Some(n) = name {
                row_names.push(raw_text(&n).unwrap_or_default());
            }
            rows.push((r + 1, cells));
            if rows.len().saturating_mul(width.max(1)) > MAX_TABLE_CELLS {
                return Err(format!(
                    "The data has more than {MAX_TABLE_CELLS} cells; give a smaller DataRange"
                ));
            }
        }
    }
    let line_texts = |range: &str| -> Option<Vec<String>> {
        let r = parse_range(range)?;
        Some(
            (0..width)
                .map(|k| raw_text(&at(r.row, r.col + k)).unwrap_or_default())
                .collect(),
        )
    };
    Ok(RawData {
        rows,
        names: line_texts(&o.names_range),
        units: line_texts(&o.units_range),
        row_names: row_names_col.map(|_| row_names),
    })
}

// ---------------------------------------------------------------------------
// Numbers

/// A number read with `decimal`, an optional thousands separator and the
/// exponent letters; Inf and NaN in any case. With `trim`, characters
/// before the first and after the last numeric character are dropped.
pub(in crate::engine) fn parse_number(
    text: &str,
    decimal: char,
    thousands: Option<char>,
    exponent: &str,
    trim: bool,
) -> Option<f64> {
    let mut t = text.trim();
    // Plain decimal text: the standard parser, no copy.
    if decimal == '.'
        && thousands.is_none()
        && !trim
        && exponent.contains('e')
        && exponent.contains('E')
        && t.bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && t.bytes().any(|b| b.is_ascii_digit())
        && let Ok(x) = t.parse::<f64>()
    {
        return Some(x);
    }
    if trim {
        let numeric = |c: char| c.is_ascii_digit() || c == decimal;
        let first = t.find(|c: char| numeric(c) || c == '-' || c == '+')?;
        let last = t.rfind(numeric)?;
        if last < first {
            return None;
        }
        t = &t[first..last + t[last..].chars().next()?.len_utf8()];
    }
    let (sign, body) = match t.strip_prefix('-') {
        Some(b) => ("-", b),
        None => ("", t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = body.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" {
        return Some(if sign == "-" {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if lower == "nan" {
        return Some(f64::NAN);
    }
    let mut out = String::with_capacity(body.len() + 1);
    out.push_str(sign);
    let mut digits = false;
    let mut seen_decimal = false;
    let mut seen_exponent = false;
    let mut exponent_digits = false;
    let mut previous_exponent = false;
    for c in body.chars() {
        if c.is_ascii_digit() {
            digits |= !seen_exponent;
            exponent_digits |= seen_exponent;
            out.push(c);
            previous_exponent = false;
        } else if c == decimal && !seen_decimal && !seen_exponent {
            seen_decimal = true;
            out.push('.');
            previous_exponent = false;
        } else if Some(c) == thousands && !seen_decimal && !seen_exponent && digits {
            previous_exponent = false;
        } else if exponent.contains(c) && digits && !seen_exponent {
            seen_exponent = true;
            previous_exponent = true;
            out.push('e');
        } else if (c == '+' || c == '-') && previous_exponent {
            out.push(c);
            previous_exponent = false;
        } else {
            return None;
        }
    }
    if !digits || (seen_exponent && !exponent_digits) {
        return None;
    }
    out.parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// Conversion

/// One converted field.
#[derive(Clone, Debug)]
enum Cv {
    Num(f64),
    Text(String),
    Date(DateTimeValue),
    Dur(f64),
    Bool(bool),
    Missing,
    Error,
    /// Replaced by the variable's FillValue.
    Fill,
}

/// Text of a raw field after quote, whitespace, prefix and suffix rules;
/// Missing or Error when the field is empty or listed in TreatAsMissing.
fn prepare(raw: &Raw, v: &VarOpts, whitespace: &[char]) -> Cv {
    let empty = || match v.empty_field_rule.as_str() {
        "error" => Cv::Error,
        "auto" if v.kind.text() => Cv::Text(String::new()),
        _ => Cv::Missing,
    };
    match raw {
        Raw::Empty => empty(),
        Raw::Num(x) => Cv::Num(*x),
        Raw::Bool(b) => Cv::Bool(*b),
        Raw::Text(t, quoted) => {
            let mut text = if *quoted {
                match v.quote_rule.as_str() {
                    "keep" => format!("\"{t}\""),
                    "error" => return Cv::Error,
                    _ => t.clone(),
                }
            } else {
                t.clone()
            };
            let space = |c: char| whitespace.contains(&c);
            if !v.kind.text() {
                text = text.trim_matches(space).to_owned();
            } else if !*quoted {
                text = match v.whitespace_rule.as_str() {
                    "preserve" => text,
                    "trimleading" => text.trim_start_matches(space).to_owned(),
                    "trimtrailing" => text.trim_end_matches(space).to_owned(),
                    _ => text.trim_matches(space).to_owned(),
                };
            }
            if v.treat_as_missing.contains(&text) {
                return Cv::Missing;
            }
            if let Some(p) = v
                .prefixes
                .iter()
                .find(|p| !p.is_empty() && text.starts_with(p.as_str()))
            {
                text = text[p.len()..].to_owned();
            }
            if let Some(s) = v
                .suffixes
                .iter()
                .find(|s| !s.is_empty() && text.ends_with(s.as_str()))
            {
                text.truncate(text.len() - s.len());
            }
            if text.is_empty() {
                return empty();
            }
            Cv::Text(text)
        }
    }
}

fn this_year() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        1970 + (secs as f64 / 31_556_952.) as i64
    }
    #[cfg(target_arch = "wasm32")]
    {
        2026
    }
}

/// Excel serial day number to a datetime.
fn excel_date(x: f64) -> Option<DateTimeValue> {
    if !x.is_finite() || x.abs() > 1e7 {
        return None;
    }
    let days = x.floor();
    let nanos = ((x - days) * NANOS_PER_DAY as f64).round() as i64;
    let (days, nanos) = if nanos >= NANOS_PER_DAY {
        (days as i64 + 1, nanos - NANOS_PER_DAY)
    } else {
        (days as i64, nanos)
    };
    // Serial day 0 of the 1900 date system is 30-Dec-1899 (datenum 693960).
    Some(DateTimeValue {
        days: days + 693_960,
        nanos,
    })
}

/// The converted column of variable `v` and the display format of a
/// datetime column.
fn convert_column(
    e: &Engine,
    v: &VarOpts,
    raws: &[&Raw],
    whitespace: &[char],
) -> Result<(Vec<Cv>, String)> {
    let mut cells: Vec<Cv> = Vec::with_capacity(raws.len());
    if v.kind.numeric() && v.prefixes.is_empty() && v.suffixes.is_empty() {
        // Numbers straight from the field text, without copies.
        let decimal = v.decimal.chars().next().unwrap_or('.');
        let thousands = v.thousands.chars().next();
        for (i, raw) in raws.iter().enumerate() {
            if i % 4096 == 0 {
                e.check_cancel()?;
            }
            cells.push(match raw {
                Raw::Text(t, quoted) if !*quoted || v.quote_rule == "remove" => {
                    let text = t.trim_matches(|c| whitespace.contains(&c));
                    if text.is_empty() || v.treat_as_missing.iter().any(|m| m == text) {
                        match prepare(raw, v, whitespace) {
                            Cv::Text(t) => parse_number(
                                &t,
                                decimal,
                                thousands,
                                &v.exponent,
                                v.trim_non_numeric,
                            )
                            .map_or(Cv::Error, Cv::Num),
                            other => other,
                        }
                    } else {
                        parse_number(text, decimal, thousands, &v.exponent, v.trim_non_numeric)
                            .map_or(Cv::Error, Cv::Num)
                    }
                }
                Raw::Num(x) => Cv::Num(*x),
                Raw::Bool(b) => Cv::Num(f64::from(u8::from(*b))),
                other => match prepare(other, v, whitespace) {
                    Cv::Text(t) => {
                        parse_number(&t, decimal, thousands, &v.exponent, v.trim_non_numeric)
                            .map_or(Cv::Error, Cv::Num)
                    }
                    other => other,
                },
            });
        }
        return Ok((cells, String::new()));
    }
    for (i, raw) in raws.iter().enumerate() {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        cells.push(prepare(raw, v, whitespace));
    }
    let mut format = String::new();
    match v.kind {
        VarType::Double | VarType::Single | VarType::Int(_) => {
            let decimal = v.decimal.chars().next().unwrap_or('.');
            let thousands = v.thousands.chars().next();
            for c in &mut cells {
                *c = match std::mem::replace(c, Cv::Missing) {
                    Cv::Text(t) => {
                        parse_number(&t, decimal, thousands, &v.exponent, v.trim_non_numeric)
                            .map_or(Cv::Error, Cv::Num)
                    }
                    Cv::Bool(b) => Cv::Num(f64::from(u8::from(b))),
                    other => other,
                };
            }
        }
        VarType::Logical => {
            let matches = |list: &[String], t: &str| {
                list.iter().any(|s| {
                    if v.case_sensitive {
                        s == t
                    } else {
                        s.eq_ignore_ascii_case(t)
                    }
                })
            };
            for c in &mut cells {
                *c = match std::mem::replace(c, Cv::Missing) {
                    Cv::Text(t) if matches(&v.true_symbols, &t) => Cv::Bool(true),
                    Cv::Text(t) if matches(&v.false_symbols, &t) => Cv::Bool(false),
                    Cv::Text(t) => match parse_number(&t, '.', None, "eE", false) {
                        Some(x) if !x.is_nan() => Cv::Bool(x != 0.),
                        _ => Cv::Error,
                    },
                    Cv::Num(x) if x.is_nan() => Cv::Error,
                    Cv::Num(x) => Cv::Bool(x != 0.),
                    other => other,
                };
            }
        }
        VarType::Char | VarType::Str | VarType::Categorical => {
            for c in &mut cells {
                *c = match std::mem::replace(c, Cv::Missing) {
                    Cv::Num(x) => Cv::Text(number_text(x)),
                    Cv::Bool(b) => Cv::Text(if b { "true" } else { "false" }.into()),
                    other => other,
                };
            }
        }
        VarType::Datetime => {
            use crate::engine::datetime_fns::text;
            let year = this_year();
            let pivot = year - 50;
            let given = if v.input_format.is_empty() {
                None
            } else {
                Some(text::tokens(&v.input_format)?)
            };
            // Without an InputFormat: the first detected form reading every
            // field, else each field's own first form.
            let mut common = u32::MAX;
            let mut any_text = false;
            if given.is_none() {
                for c in &cells {
                    if let Cv::Text(t) = c {
                        any_text = true;
                        common &= detect::date_mask(t);
                    }
                }
            }
            let tokens = detect::date_tokens();
            let common_index = (any_text && common != 0).then(|| common.trailing_zeros() as usize);
            for c in &mut cells {
                *c = match std::mem::replace(c, Cv::Missing) {
                    Cv::Text(t) if t == "NaT" => Cv::Missing,
                    Cv::Text(t) => {
                        let value = match (&given, common_index) {
                            (Some(g), _) => text::parse(g, &t, pivot, year),
                            (None, Some(k)) => text::parse(&tokens[k], &t, pivot, year),
                            (None, None) => {
                                let m = detect::date_mask(&t);
                                (m != 0)
                                    .then(|| {
                                        text::parse(
                                            &tokens[m.trailing_zeros() as usize],
                                            &t,
                                            pivot,
                                            year,
                                        )
                                    })
                                    .flatten()
                            }
                        };
                        value.map_or(Cv::Error, Cv::Date)
                    }
                    Cv::Num(x) => excel_date(x).map_or(Cv::Error, Cv::Date),
                    Cv::Bool(_) => Cv::Error,
                    other => other,
                };
            }
            let detected = if v.input_format.is_empty() {
                common_index.map(|k| detect::DATE_FORMATS[k].to_owned())
            } else {
                Some(v.input_format.clone())
            };
            format = match v.datetime_format.to_ascii_lowercase().as_str() {
                "preserveinput" => detected.unwrap_or_else(|| DEFAULT_DATE_FORMAT.into()),
                "default" => DEFAULT_DATE_FORMAT.into(),
                "defaultdate" => DEFAULT_DATE_FORMAT.into(),
                _ => v.datetime_format.clone(),
            };
        }
        VarType::Duration => {
            use crate::engine::datetime_fns::dur_text;
            let given = if v.input_format.is_empty() {
                None
            } else {
                Some(dur_text::parse_input_format(&v.input_format)?)
            };
            let separator = v.field_separator.chars().next().unwrap_or(':');
            let decimal = v.decimal.chars().next().unwrap_or('.');
            for c in &mut cells {
                *c = match std::mem::replace(c, Cv::Missing) {
                    Cv::Text(t) => {
                        let t = if decimal == '.' {
                            t
                        } else {
                            t.replace(decimal, ".")
                        };
                        let ms = match given {
                            Some(f) => dur_text::parse(f, &t),
                            None => detect::duration_ms(&t, separator),
                        };
                        ms.map_or(Cv::Error, Cv::Dur)
                    }
                    Cv::Num(x) => Cv::Dur(x * 86_400_000.),
                    Cv::Bool(_) => Cv::Error,
                    other => other,
                };
            }
            format = if v.duration_format.eq_ignore_ascii_case("default") {
                if v.input_format.is_empty() {
                    crate::duration_array::DEFAULT_FORMAT.into()
                } else {
                    v.input_format.clone()
                }
            } else {
                v.duration_format.clone()
            };
        }
    }
    Ok((cells, format))
}

/// The FillValue of `v` as a number, text, datetime, duration or flag.
fn fill_cv(e: &Engine, v: &VarOpts) -> Result<Cv> {
    let fill = super::fill_value(e, v)?;
    Ok(match v.kind {
        VarType::Double | VarType::Single | VarType::Int(_) => Cv::Num(
            numbers(&Value::from(fill))
                .and_then(|(_, _, d)| d.first().copied())
                .unwrap_or(f64::NAN),
        ),
        VarType::Logical => Cv::Bool(
            numbers(&Value::from(fill))
                .and_then(|(_, _, d)| d.first().copied())
                .is_some_and(|x| x != 0.),
        ),
        VarType::Char | VarType::Categorical => {
            Cv::Text(text_of(&Value::from(fill)).unwrap_or_default())
        }
        VarType::Str => match &fill {
            Array::Str(s) if s.len() == 1 && s.data()[0].is_none() => Cv::Missing,
            other => match text_of(&Value::from(other.clone())) {
                Some(t) => Cv::Text(t),
                None => Cv::Missing,
            },
        },
        VarType::Datetime => match &fill {
            Array::Datetime(d) if d.len() == 1 => Cv::Date(d.data()[0]),
            _ => Cv::Date(DateTimeValue::NAT),
        },
        VarType::Duration => match &fill {
            Array::Duration(d) if d.len() == 1 => Cv::Dur(d.data()[0]),
            _ => Cv::Dur(f64::NAN),
        },
    })
}

/// The column array of variable `v` from its converted fields.
fn build_column(e: &Engine, v: &VarOpts, cells: &[Cv], fill: &Cv, format: &str) -> Result<Array> {
    let n = cells.len();
    let get = |c: &'_ Cv| -> Cv {
        match c {
            Cv::Fill => fill.clone(),
            other => other.clone(),
        }
    };
    let number = |c: Cv| match c {
        Cv::Num(x) => x,
        Cv::Bool(b) => f64::from(u8::from(b)),
        _ => f64::NAN,
    };
    Ok(match v.kind {
        VarType::Double => {
            Matrix::from_column_vec(n, 1, cells.iter().map(|c| number(get(c))).collect())?.into()
        }
        VarType::Single => Array::Typed(TypedArray::from_reals(
            NumericClass::Single,
            n,
            1,
            cells.iter().map(|c| number(get(c))),
        )?),
        VarType::Int(class) => Array::Typed(TypedArray::from_reals(
            class,
            n,
            1,
            cells.iter().map(|c| number(get(c))),
        )?),
        VarType::Logical => {
            let data = cells
                .iter()
                .map(|c| u8::from(matches!(get(c), Cv::Bool(true))))
                .collect();
            LogicalMatrix::from_column_vec(n, 1, data, &e.cancel)?.into()
        }
        VarType::Char => {
            if n > crate::script_containers::MAX_CELL_SLOTS {
                return Err(format!(
                    "Text variable '{}' has {n} rows; text variables hold at most {} rows (read fewer lines or use 'TextType','string')",
                    v.name,
                    crate::script_containers::MAX_CELL_SLOTS
                ));
            }
            let empty = Arc::new(char_array(e, "")?);
            let mut slots = Vec::with_capacity(n);
            for c in cells {
                slots.push(match get(c) {
                    Cv::Text(t) if !t.is_empty() => Arc::new(char_array(e, &t)?),
                    _ => empty.clone(),
                });
            }
            CellArray::from_shared(n, 1, slots, &e.cancel)?.into()
        }
        VarType::Str => {
            let data = cells
                .iter()
                .map(|c| match get(c) {
                    Cv::Text(t) => Some(Arc::from(t.as_str())),
                    _ => None,
                })
                .collect();
            Array::Str(StringArray::new(n, 1, data)?)
        }
        VarType::Datetime => {
            let data = cells
                .iter()
                .map(|c| match get(c) {
                    Cv::Date(d) => d,
                    _ => DateTimeValue::NAT,
                })
                .collect();
            let zone = if v.time_zone.eq_ignore_ascii_case("UTC") {
                crate::datetime_array::TimeZone::Utc
            } else {
                crate::datetime_array::TimeZone::Unzoned
            };
            let format = if format.is_empty() {
                DEFAULT_FORMAT
            } else {
                format
            };
            Array::Datetime(DatetimeArray::new(n, 1, data, format, zone)?)
        }
        VarType::Duration => {
            let data = cells
                .iter()
                .map(|c| match get(c) {
                    Cv::Dur(x) => x,
                    _ => f64::NAN,
                })
                .collect();
            Array::Duration(DurationArray::new(n, 1, data, format)?)
        }
        VarType::Categorical => {
            let texts: Vec<Option<String>> = cells
                .iter()
                .map(|c| match get(c) {
                    Cv::Text(t) if !t.is_empty() => Some(t),
                    _ => None,
                })
                .collect();
            let mut categories: Vec<String> = v.categories.clone();
            let mut extra: Vec<String> = Vec::new();
            for t in texts.iter().flatten() {
                if !categories.contains(t) && !extra.contains(t) {
                    if v.protected && !v.categories.is_empty() {
                        return Err(format!(
                            "Variable '{}': '{t}' is not one of its protected Categories",
                            v.name
                        ));
                    }
                    extra.push(t.clone());
                }
            }
            extra.sort();
            categories.extend(extra);
            let codes = texts
                .iter()
                .map(|t| {
                    t.as_ref().map_or(0, |t| {
                        categories
                            .iter()
                            .position(|c| c == t)
                            .map_or(0, |p| p as u32 + 1)
                    })
                })
                .collect();
            Array::Categorical(crate::CategoricalArray::new(
                n,
                1,
                codes,
                Arc::new(categories),
                v.ordinal,
            )?)
        }
    })
}

/// A converted read: the output variables and their properties.
struct Imported {
    names: Vec<String>,
    columns: Vec<Array>,
    height: usize,
    row_names: Option<Vec<String>>,
    units: Vec<String>,
}

fn rule_message(who: &str, missing: bool, v: &VarOpts, line: usize, raw: &Raw) -> String {
    if missing {
        format!(
            "{who}: variable '{}' has a missing value on line {line} and MissingRule is 'error'",
            v.name
        )
    } else {
        format!(
            "{who}: unable to convert '{}' on line {line} to type '{}' for variable '{}' (ImportErrorRule is 'error')",
            raw_text(raw).unwrap_or_default(),
            v.kind.name(),
            v.name
        )
    }
}

/// Convert the selected variables of `data` with the options' rules.
fn convert(
    e: &Engine,
    who: &str,
    o: &ImportOpts,
    data: &RawData,
    read_names: bool,
) -> Result<Imported> {
    let whitespace: Vec<char> = if o.sheet {
        vec![' ', '\t']
    } else {
        Splitter::from_opts(o).whitespace
    };
    let row_names_column = (!o.sheet && o.row_names_column > 0).then(|| o.row_names_column - 1);
    let selected: Vec<usize> = o
        .selected
        .iter()
        .copied()
        .filter(|&j| Some(j) != row_names_column)
        .collect();
    let empty = Raw::Empty;
    let n = data.rows.len();
    let mut drop_row = vec![false; n];
    let mut drop_var = vec![false; selected.len()];
    let mut converted = Vec::with_capacity(selected.len());
    for (k, &j) in selected.iter().enumerate() {
        let v = &o.vars[j];
        let raws: Vec<&Raw> = data
            .rows
            .iter()
            .map(|(_, f)| f.get(j).unwrap_or(&empty))
            .collect();
        let (mut cells, format) = convert_column(e, v, &raws, &whitespace)?;
        for (r, c) in cells.iter_mut().enumerate() {
            let missing = matches!(c, Cv::Missing);
            if !missing && !matches!(c, Cv::Error) {
                continue;
            }
            let rule = if missing {
                &o.missing_rule
            } else {
                &o.import_error_rule
            };
            match rule.as_str() {
                "error" => {
                    return Err(rule_message(who, missing, v, data.rows[r].0, raws[r]));
                }
                "omitrow" => drop_row[r] = true,
                "omitvar" => drop_var[k] = true,
                _ => {}
            }
            *c = Cv::Fill;
        }
        converted.push((cells, format));
    }
    let keep: Vec<usize> = (0..n).filter(|&r| !drop_row[r]).collect();
    let mut names = Vec::new();
    let mut columns = Vec::new();
    let mut units = Vec::new();
    let file_names = if read_names {
        data.names.as_ref()
    } else {
        None
    };
    for (k, &j) in selected.iter().enumerate() {
        if drop_var[k] {
            continue;
        }
        let v = &o.vars[j];
        let (cells, format) = &converted[k];
        let kept: Vec<Cv> = keep.iter().map(|&r| cells[r].clone()).collect();
        let fill = fill_cv(e, v)?;
        columns.push(build_column(e, v, &kept, &fill, format)?);
        names.push(Some(
            file_names
                .and_then(|f| f.get(j).filter(|t| !t.is_empty()).cloned())
                .unwrap_or_else(|| v.name.clone()),
        ));
        units.push(
            data.units
                .as_ref()
                .and_then(|u| u.get(j).cloned())
                .unwrap_or_default(),
        );
    }
    // Fields beyond the last variable (text files).
    if !o.sheet {
        let width = data.rows.iter().map(|(_, f)| f.len()).max().unwrap_or(0);
        if width > o.vars.len() {
            match o.extra_columns_rule.as_str() {
                "error" => {
                    let line = data
                        .rows
                        .iter()
                        .find(|(_, f)| f.len() > o.vars.len())
                        .map_or(0, |r| r.0);
                    return Err(format!(
                        "{who}: line {line} has more fields than the {} variables (ExtraColumnsRule is 'error')",
                        o.vars.len()
                    ));
                }
                "wrap" => {
                    return Err(format!(
                        "{who}: ExtraColumnsRule 'wrap' is not supported; use 'addvars', 'ignore' or 'error'"
                    ));
                }
                "addvars" => {
                    for (extra, c) in (o.vars.len()..width).enumerate() {
                        let v = VarOpts::new(format!("ExtraVar{}", extra + 1), VarType::Char);
                        let raws: Vec<&Raw> = keep
                            .iter()
                            .map(|&r| data.rows[r].1.get(c).unwrap_or(&empty))
                            .collect();
                        let (cells, _) = convert_column(e, &v, &raws, &whitespace)?;
                        let cells: Vec<Cv> = cells
                            .into_iter()
                            .map(|c| match c {
                                Cv::Missing | Cv::Error => Cv::Fill,
                                other => other,
                            })
                            .collect();
                        columns.push(build_column(e, &v, &cells, &Cv::Text(String::new()), "")?);
                        names.push(Some(v.name));
                        units.push(String::new());
                    }
                }
                _ => {}
            }
        }
    }
    let row_names = match (row_names_column, &data.row_names) {
        (Some(c), _) => Some(
            keep.iter()
                .map(|&r| data.rows[r].1.get(c).and_then(raw_text).unwrap_or_default())
                .collect(),
        ),
        (None, Some(names)) => Some(keep.iter().map(|&r| names[r].clone()).collect()),
        _ => None,
    };
    let names = crate::engine::table_fns::csv::variable_names(names);
    let has_units = units.iter().any(|u| !u.is_empty());
    Ok(Imported {
        names,
        columns,
        height: keep.len(),
        row_names,
        units: if has_units { units } else { Vec::new() },
    })
}

fn raw_data(
    e: &Engine,
    who: &str,
    file: &Value,
    o: &ImportOpts,
    limit: Option<usize>,
) -> Result<RawData> {
    let (filename, path) = detect::open(e, who, file)?;
    if o.sheet {
        sheet_data(e, &path, o, limit)
    } else {
        text_data(e, &filename, &path, o, limit)
    }
}

/// The table `file` holds under options `o`.
fn read_table(
    e: &Engine,
    who: &str,
    file: &Value,
    o: &ImportOpts,
    read_names: bool,
    limit: Option<usize>,
) -> Result<TableArray> {
    let data = raw_data(e, who, file, o, limit)?;
    let imported = convert(e, who, o, &data, read_names)?;
    let mut parts = TableParts::new(imported.names, imported.columns, imported.height);
    parts.row_names = imported.row_names;
    parts.units = imported.units;
    TableArray::new(parts)
}

// ---------------------------------------------------------------------------
// Entry points

fn options_object(e: &Engine, v: Option<&Value>) -> Option<Arc<ImportOpts>> {
    match crate::engine::objects::import_value(e, v?)? {
        IoObject::Opts(o) => Some(o),
        IoObject::Var(_) => None,
    }
}

/// True when readtable must use this module: an options object, a
/// spreadsheet, or an option the original text reader lacks.
pub(in crate::engine) fn readtable_takes_over(e: &Engine, args: &[Value]) -> bool {
    let Some((file, rest)) = args.split_first() else {
        return false;
    };
    if options_object(e, rest.first()).is_some() {
        return true;
    }
    if detect::is_sheet(&text_of(file).unwrap_or_default(), None) {
        return true;
    }
    rest.chunks(2).any(|pair| {
        text_of(&pair[0])
            .is_some_and(|n| !LEGACY_OPTIONS.contains(&n.to_ascii_lowercase().as_str()))
    })
}

/// Options for a read: the given object (with ReadVariableNames after it)
/// or detection with the name/value options.
fn read_options(
    e: &Engine,
    who: &str,
    file: &Value,
    rest: &[Value],
    extra: &mut dyn FnMut(&str, &Value) -> Result<bool>,
) -> Result<(ImportOpts, bool)> {
    if let Some(o) = options_object(e, rest.first()) {
        let rest = &rest[1..];
        if !rest.len().is_multiple_of(2) {
            return Err(format!(
                "{who}: options after the import options must be name-value pairs"
            ));
        }
        let mut read_names = false;
        let mut opts = o.as_ref().clone();
        for pair in rest.chunks(2) {
            let name =
                text_of(&pair[0]).ok_or_else(|| format!("{who}: option names must be text"))?;
            match name.to_ascii_lowercase().as_str() {
                "readvariablenames" => {
                    read_names = super::flag_value(who, "ReadVariableNames", &pair[1])?;
                }
                "readrownames" => {
                    let flag = super::flag_value(who, "ReadRowNames", &pair[1])?;
                    if !opts.sheet {
                        opts.row_names_column = if flag {
                            opts.row_names_column.max(1)
                        } else {
                            0
                        };
                    }
                }
                _ if extra(&name, &pair[1])? => {}
                _ => {
                    return Err(format!(
                        "{who}: with import options, only ReadVariableNames and ReadRowNames can be given; '{name}' is not supported"
                    ));
                }
            }
        }
        if read_names && opts.names_line == 0 && opts.names_range.is_empty() {
            return Err(format!(
                "{who}: ReadVariableNames is true but the options have no VariableNamesLine"
            ));
        }
        return Ok((opts, read_names));
    }
    if !rest.len().is_multiple_of(2) {
        return Err(format!("{who}: options must be name-value pairs"));
    }
    let mut d = DetectArgs::default();
    for pair in rest.chunks(2) {
        let name = text_of(&pair[0]).ok_or_else(|| format!("{who}: option names must be text"))?;
        if d.take(who, &name, &pair[1])? || extra(&name, &pair[1])? {
            continue;
        }
        return Err(format!("{who}: unsupported option '{name}'"));
    }
    Ok((detect::detect(e, who, file, &d)?, false))
}

/// `readtable(file, opts)` and `readtable(file, Name, Value, ...)` with
/// options the original reader does not have.
pub(in crate::engine) fn readtable(e: &Engine, args: &[Value]) -> Result<Value> {
    let who = "readtable";
    let Some((file, rest)) = args.split_first() else {
        return Err("Not enough input arguments.".into());
    };
    let (opts, read_names) = read_options(e, who, file, rest, &mut |_, _| Ok(false))?;
    let table = read_table(e, who, file, &opts, read_names, None)?;
    Ok(Value::Container(Array::Table(table)))
}

/// `preview(file, opts)`: the first 8 rows as a table.
pub(super) fn preview(e: &Engine, args: &[Value]) -> Result<Value> {
    let who = "preview";
    let [file, opts] = args else {
        return Err(if args.len() < 2 {
            "preview: give a file name and import options, as in preview(file, opts)".into()
        } else {
            "Too many input arguments.".into()
        });
    };
    let o = options_object(e, Some(opts))
        .ok_or("preview: the second input must be import options from detectImportOptions")?;
    let table = read_table(e, who, file, &o, false, Some(PREVIEW_ROWS))?;
    Ok(Value::Container(Array::Table(table)))
}

/// One converted variable: its kept fields, datetime/duration format,
/// options and fill value.
type Column = (Vec<Cv>, String, VarOpts, Cv);

const MATRIX_TEXT_DELIMITERS: [&str; 4] = [",", ";", "\t", " "];

fn matrix_text_field(field: &str) -> &str {
    field.trim().trim_matches('"').trim()
}

/// Sample from the first plausible data record so punctuation in a title
/// does not determine the delimiter.
fn matrix_text_delimiter(e: &Engine, text: &str) -> Result<&'static str> {
    for (index, line) in text.lines().enumerate() {
        if index.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        if !line.trim().is_empty()
            && MATRIX_TEXT_DELIMITERS.iter().any(|delimiter| {
                line.split(delimiter)
                    .map(matrix_text_field)
                    .all(|field| field.is_empty() || field.parse::<f64>().is_ok())
            })
        {
            return Ok(detect::detect_delimiter(text, index));
        }
    }
    Ok(detect::detect_delimiter(text, 0))
}

/// Find the data start after leading records with at least one text field.
fn matrix_header_lines(e: &Engine, text: &str, splitter: &Splitter) -> Result<Option<usize>> {
    let mut saw_text = false;
    let mut saw_number = false;
    let mut lines = text.lines().enumerate();
    let mut line_count = 0;
    for (index, line) in &mut lines {
        line_count = index + 1;
        if index.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let records = detect::split(line, splitter, Some(1), &e.cancel)?;
        let Some(fields) = records.first().and_then(|record| record.fields.as_ref()) else {
            continue;
        };
        let mut has_text = false;
        for field in fields.iter().map(|field| matrix_text_field(&field.text)) {
            if !field.is_empty() {
                if field.parse::<f64>().is_ok() {
                    saw_number = true;
                } else {
                    has_text = true;
                }
            }
        }
        if !has_text {
            return Ok(saw_text.then_some(index));
        }
        saw_text = true;
    }
    // A file of text headings can be empty data. Mixed fields with no data
    // start retain the strict numeric reader's error.
    Ok((saw_text && !saw_number).then_some(line_count))
}

/// Stream a numeric matrix directly when `files::read_matrix_text` fell back
/// (e.g. space-delimited, headered, or multi-delimiter files), avoiding
/// per-cell heap allocations so large files stay within tight memory limits.
fn readmatrix_streaming_fallback(
    e: &Engine,
    who: &str,
    args: &[Value],
    original: String,
) -> Result<Value> {
    let Some((file, rest)) = args.split_first() else {
        return Err("Not enough input arguments.".into());
    };
    e.check_cancel()?;
    let mut detection = detect::DetectArgs::default();
    for pair in rest.as_chunks::<2>().0 {
        let name = text_of(&pair[0]).ok_or("readmatrix: option names must be text")?;
        if !detection.take(who, &name, &pair[1])? {
            return Err(format!("readmatrix: unsupported option '{name}'"));
        }
    }
    let (_, path) = detect::open(e, who, file)?;
    let bytes = crate::files::read_bounded(&path, crate::files::MAX_CSV_BYTES)?;
    let Ok(text) = std::str::from_utf8(crate::files::strip_utf8_bom(&bytes)) else {
        return Err(original);
    };
    let delimiters = match detection.delimiter.take() {
        Some(list) => list,
        None => vec![matrix_text_delimiter(e, text)?.to_owned()],
    };
    let multi_delimiter = delimiters.len() > 1;
    if let [delimiter] = &delimiters[..]
        && !MATRIX_TEXT_DELIMITERS.contains(&delimiter.as_str())
    {
        return Err(original);
    }
    let space = delimiters.iter().any(|delimiter| delimiter == " ");
    // A trailing space is padding. Other trailing delimiters mark empty fields.
    detection.trailing = Some(if space { "ignore" } else { "keep" }.into());
    detection.delimiter = Some(delimiters);
    let splitter = Splitter::from_opts(&detect::text_options(text, &detection));
    let header = match matrix_header_lines(e, text, &splitter)? {
        Some(header) => header,
        None if space || multi_delimiter => 0,
        None => return Err(original),
    };
    let mut rows = 0usize;
    let mut width = 0usize;
    detect::split_each(text, &splitter, None, &e.cancel, |line, fields| {
        if line <= header {
            return Ok(());
        }
        if let Some(fields) = fields {
            width = width.max(fields.len());
            rows += 1;
        }
        Ok(())
    })?;
    if rows == 0 || width == 0 {
        return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
    }
    let cells = rows.saturating_mul(width);
    if cells > MAX_TABLE_CELLS {
        return Err("readmatrix: the result is too large".into());
    }
    if cells.saturating_mul(std::mem::size_of::<f64>()) > e.limits.memory_bytes {
        return Err("readmatrix: result exceeds memory budget".into());
    }
    let mut data = Vec::new();
    data.try_reserve_exact(cells)
        .map_err(|_| "readmatrix: result exceeds memory budget".to_owned())?;
    data.resize(cells, f64::NAN);
    let whitespace = &splitter.whitespace;
    let mut row = 0usize;
    detect::split_each(text, &splitter, None, &e.cancel, |line, fields| {
        if line <= header {
            return Ok(());
        }
        let Some(fields) = fields else {
            return Ok(());
        };
        for (col, field) in fields.iter().enumerate() {
            let trimmed = field.text.trim_matches(whitespace.as_slice());
            if trimmed.is_empty() {
                continue;
            }
            let Some(num) = parse_number(trimmed, '.', None, "eEdD", false) else {
                return Err(format!(
                    "{who}: invalid numeric value {:?} on line {line}",
                    field.text
                ));
            };
            data[col * rows + row] = num;
        }
        row += 1;
        Ok(())
    })?;
    Ok(Value::Matrix(Matrix::from_column_vec(rows, width, data)?))
}

/// Selected variables and kept rows as converted columns (readmatrix reads
/// every variable as its OutputType).
fn read_grid(e: &Engine, who: &str, args: &[Value]) -> Result<(Vec<Column>, usize)> {
    let Some((_file, rest)) = args.split_first() else {
        return Err("Not enough input arguments.".into());
    };
    let file = &args[0];
    let (mut opts, pairs) = {
        let o = options_object(e, rest.first())
            .ok_or_else(|| format!("{who}: the second input must be import options"))?;
        (o.as_ref().clone(), &rest[1..])
    };
    if !pairs.len().is_multiple_of(2) {
        return Err(format!(
            "{who}: options after the import options must be name-value pairs"
        ));
    }
    let mut output: Option<VarType> = None;
    for pair in pairs.chunks(2) {
        let name = text_of(&pair[0]).ok_or_else(|| format!("{who}: option names must be text"))?;
        if who == "readmatrix" && name.eq_ignore_ascii_case("OutputType") {
            let t = text_of(&pair[1]).unwrap_or_default();
            let kind = VarType::parse(&t).filter(|k| k.numeric() || *k == VarType::Logical);
            output = Some(kind.ok_or_else(|| {
                "readmatrix: OutputType must be a numeric type such as 'double'".to_string()
            })?);
            continue;
        }
        return Err(format!(
            "{who}: with import options, only {} can be given; '{name}' is not supported",
            if who == "readmatrix" {
                "OutputType"
            } else {
                "the options object"
            }
        ));
    }
    if who == "readmatrix" {
        let kind = output.unwrap_or(VarType::Double);
        for v in &mut opts.vars {
            *v = v.retyped(kind);
        }
    }
    let data = raw_data(e, who, file, &opts, None)?;
    let whitespace: Vec<char> = if opts.sheet {
        vec![' ', '\t']
    } else {
        Splitter::from_opts(&opts).whitespace
    };
    let empty = Raw::Empty;
    let n = data.rows.len();
    let mut drop_row = vec![false; n];
    let mut columns = Vec::new();
    let mut drop_var = Vec::new();
    for &j in &opts.selected {
        let v = &opts.vars[j];
        let raws: Vec<&Raw> = data
            .rows
            .iter()
            .map(|(_, f)| f.get(j).unwrap_or(&empty))
            .collect();
        let (mut cells, format) = convert_column(e, v, &raws, &whitespace)?;
        let mut dropped = false;
        for (r, c) in cells.iter_mut().enumerate() {
            let missing = matches!(c, Cv::Missing);
            if !missing && !matches!(c, Cv::Error) {
                continue;
            }
            let rule = if missing {
                &opts.missing_rule
            } else {
                &opts.import_error_rule
            };
            match rule.as_str() {
                "error" => return Err(rule_message(who, missing, v, data.rows[r].0, raws[r])),
                "omitrow" => drop_row[r] = true,
                "omitvar" => dropped = true,
                _ => {}
            }
            *c = Cv::Fill;
        }
        drop_var.push(dropped);
        columns.push((cells, format, v.clone(), fill_cv(e, v)?));
    }
    let keep: Vec<usize> = (0..n).filter(|&r| !drop_row[r]).collect();
    let columns = columns
        .into_iter()
        .zip(drop_var)
        .filter(|(_, d)| !d)
        .map(|((cells, format, v, fill), _)| {
            (
                keep.iter().map(|&r| cells[r].clone()).collect(),
                format,
                v,
                fill,
            )
        })
        .collect();
    Ok((columns, keep.len()))
}

/// `readmatrix(file, opts)`: the selected variables as numbers.
pub(in crate::engine) fn readmatrix(
    e: &Engine,
    args: &[Value],
    strict_error: Option<String>,
) -> Result<Value> {
    if let Some(original) = strict_error {
        return readmatrix_streaming_fallback(e, "readmatrix", args, original);
    }
    let (columns, rows) = read_grid(e, "readmatrix", args)?;
    let cols = columns.len();
    if rows.saturating_mul(cols) > MAX_TABLE_CELLS {
        return Err("readmatrix: the result is too large".into());
    }
    let mut data = Vec::with_capacity(rows * cols);
    let kind = columns.first().map_or(VarType::Double, |c| c.2.kind);
    for (cells, _, _, fill) in &columns {
        for c in cells {
            data.push(match if matches!(c, Cv::Fill) { fill } else { c } {
                Cv::Num(x) => *x,
                Cv::Bool(b) => f64::from(u8::from(*b)),
                _ => f64::NAN,
            });
        }
    }
    let (rows, cols) = if rows == 0 || cols == 0 {
        (0, 0)
    } else {
        (rows, cols)
    };
    let matrix = Matrix::from_column_vec(rows, cols, data)?;
    Ok(match kind {
        VarType::Single | VarType::Int(_) => {
            let class = match kind {
                VarType::Int(c) => c,
                _ => NumericClass::Single,
            };
            Value::Container(Array::Typed(TypedArray::from_matrix(class, &matrix)?))
        }
        VarType::Logical => {
            let bits = matrix
                .data()
                .iter()
                .map(|&x| u8::from(x != 0. && !x.is_nan()))
                .collect();
            Value::Container(LogicalMatrix::from_column_vec(rows, cols, bits, &e.cancel)?.into())
        }
        _ => Value::Matrix(matrix),
    })
}

/// `readcell(file, opts)`: the selected variables, one value per cell.
pub(in crate::engine) fn readcell(e: &Engine, args: &[Value]) -> Result<Value> {
    let (columns, rows) = read_grid(e, "readcell", args)?;
    let cols = columns.len();
    if rows.saturating_mul(cols) > crate::script_containers::MAX_CELL_SLOTS {
        return Err(format!(
            "readcell: the result would have more than {} cells",
            crate::script_containers::MAX_CELL_SLOTS
        ));
    }
    if rows == 0 || cols == 0 {
        return Ok(Value::Container(Array::Cell(CellArray::empty(
            0, 0, &e.cancel,
        )?)));
    }
    let mut slots = Vec::with_capacity(rows * cols);
    for (cells, format, v, fill) in &columns {
        let column = build_column(e, v, cells, fill, format)?;
        for r in 0..rows {
            e.check_cancel()?;
            slots.push(Arc::new(match &column {
                Array::Cell(c) => c.get_linear(r)?.as_ref().clone(),
                Array::Numeric(m) => Matrix::scalar(m.data()[r]).into(),
                Array::Logical(m) => LogicalMatrix::scalar(m.data()[r] != 0).into(),
                Array::Typed(t) => Array::Typed(t.select(1, 1, &[r])?),
                Array::Str(s) => Array::Str(StringArray::new(1, 1, vec![s.data()[r].clone()])?),
                Array::Datetime(d) => Array::Datetime(d.select(1, 1, &[r])?),
                Array::Duration(d) => Array::Duration(d.select(1, 1, &[r])?),
                Array::Categorical(c) => Array::Categorical(c.select(1, 1, &[r])?),
                other => other.clone(),
            }));
        }
    }
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        rows, cols, slots, &e.cancel,
    )?)))
}

/// `readtimetable(file)`, `readtimetable(file, opts)` and
/// `readtimetable(..., 'RowTimes', name or times)`.
pub(super) fn readtimetable(e: &Engine, args: &[Value]) -> Result<Value> {
    let who = "readtimetable";
    let Some((file, rest)) = args.split_first() else {
        return Err("Not enough input arguments.".into());
    };
    let mut row_times: Option<Value> = None;
    let mut extra = |name: &str, value: &Value| -> Result<bool> {
        match name.to_ascii_lowercase().as_str() {
            "rowtimes" => {
                row_times = Some(value.clone());
                Ok(true)
            }
            "samplerate" | "timestep" | "starttime" => Err(format!(
                "{who}: {name} is not supported yet; give 'RowTimes' as a variable name or a datetime/duration vector"
            )),
            _ => Ok(false),
        }
    };
    let (opts, read_names) = read_options(e, who, file, rest, &mut extra)?;
    let table = read_table(e, who, file, &opts, read_names, None)?;
    let mut parts = table.to_parts();
    let times = match &row_times {
        Some(Value::Container(a @ (Array::Datetime(_) | Array::Duration(_)))) => {
            if a.len() != parts.height {
                return Err(format!(
                    "{who}: RowTimes must have one time per row ({})",
                    parts.height
                ));
            }
            parts.dimension_names[0] = "Time".into();
            match a {
                Array::Datetime(d) => Array::Datetime(d.reshaped(parts.height, 1)?),
                Array::Duration(d) => Array::Duration(d.reshaped(parts.height, 1)?),
                _ => unreachable!("matched above"),
            }
        }
        Some(v) => {
            let name = text_of(v).ok_or_else(|| {
                format!("{who}: RowTimes must be a variable name or a datetime or duration vector")
            })?;
            let k = parts
                .names
                .iter()
                .position(|n| *n == name)
                .map(Ok)
                .or_else(|| {
                    let j = opts.vars.iter().position(|x| x.name == name)?;
                    let k = opts.selected.iter().position(|&s| s == j)?;
                    // A column dropped by ImportErrorRule 'omitvar' shifts the
                    // later ones, so the selected index no longer matches.
                    Some(if parts.columns.len() == opts.selected.len() {
                        Ok(k)
                    } else {
                        Err(format!(
                            "{who}: the RowTimes variable '{name}' was omitted by ImportErrorRule"
                        ))
                    })
                })
                .transpose()?
                .ok_or_else(|| {
                    format!(
                        "{who}: unknown variable name '{name}' for RowTimes; VariableNames are {}",
                        super::list_text(&parts.names)
                    )
                })?;
            take_times(who, &mut parts, k)?
        }
        None => {
            let k = parts
                .columns
                .iter()
                .position(|c| matches!(c, Array::Datetime(_) | Array::Duration(_)))
                .ok_or_else(|| {
                    format!("{who}: the file has no datetime or duration variable; give 'RowTimes'")
                })?;
            take_times(who, &mut parts, k)?
        }
    };
    parts.row_times = Some(times);
    parts.row_names = None;
    Ok(Value::Container(Array::Table(TableArray::new(parts)?)))
}

/// Remove variable `k` (datetime or duration) to be the row times; its
/// name becomes the first dimension name.
fn take_times(who: &str, parts: &mut TableParts, k: usize) -> Result<Array> {
    let column = parts.columns[k].clone();
    if !matches!(column, Array::Datetime(_) | Array::Duration(_)) {
        return Err(format!(
            "{who}: RowTimes variable '{}' must be datetime or duration, not {}",
            parts.names[k],
            column.class()
        ));
    }
    parts.dimension_names[0] = parts.names[k].clone();
    parts.remove_variables(&[k]);
    if parts.dimension_names[0] == parts.dimension_names[1] {
        parts.dimension_names[1] = "Variables_1".into();
    }
    Ok(column)
}
