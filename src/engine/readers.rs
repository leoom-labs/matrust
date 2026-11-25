//! JSON text (jsondecode, jsonencode), delimited-text cell files (readcell,
//! writecell) and spreadsheet reads (xlsread, and readcell/readmatrix on
//! .xlsx/.xlsm/.xlsb/.xls through the calamine crate). Behaviour follows the
//! published MathWorks function pages (behaviour only) and GNU Octave's
//! jsondecode/jsonencode; differences are listed on the help pages.
//!
//! Everything is bounded: input text and files are size-checked before they
//! are parsed, JSON nesting is limited, and loops poll cancellation.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, StringArray, StructArray,
    struct_array::{MAX_FIELD_NAME, valid_field_name},
    syntax::Expr,
};
mod hdf5;
mod web;
#[cfg(not(target_arch = "wasm32"))]
mod xlsx_write;

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

const NAMES: &[&str] = &[
    "jsondecode",
    "jsonencode",
    "readcell",
    "writecell",
    "xlsread",
    "readtable",
    "writetable",
    "xlswrite",
    "webread",
    "websave",
    "weboptions",
    "h5read",
    "h5create",
    "h5write",
    "ncread",
];
/// File extensions read through the spreadsheet reader.
const SHEET_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xlsb", "xls"];
const TEXT_EXTENSIONS: &[&str] = &["csv", "txt", "dat"];
/// Largest JSON text accepted by jsondecode and produced by jsonencode.
const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;
/// JSON nesting accepted by the parser; decoded cells and structs have their
/// own (smaller) container nesting bound.
const MAX_JSON_DEPTH: usize = 128;
/// Largest delimited text file read by readcell.
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;
/// Largest spreadsheet file opened.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SHEET_BYTES: u64 = 128 * 1024 * 1024;
/// Largest inflated size of one part of an .xlsx/.xlsb package.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PART_BYTES: u64 = 512 * 1024 * 1024;
/// Largest numeric grid returned by readmatrix/xlsread from a sheet.
const MAX_GRID_CELLS: usize = 16 * 1024 * 1024;
const SHEET_UNSUPPORTED_WRITE: &str =
    "Writing spreadsheet files (.xlsx, .xls) is not supported yet; write a .csv or .txt file";

pub(super) fn rdx_handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn rdx_check(cancel: &AtomicBool) -> Result<()> {
    crate::dsp::check_cancel(cancel)
}

/// Text of a char row, string scalar or plain text value.
fn rdx_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(str::to_owned)
        }
        _ => None,
    }
}

fn rdx_flag(function: &str, option: &str, value: &Value) -> Result<bool> {
    let bad = || format!("{function}: the '{option}' value must be true or false");
    match value {
        Value::Logical(m) if m.len() == 1 => Ok(m.data()[0] != 0),
        Value::Matrix(m) if m.len() == 1 && !m.data()[0].is_nan() => Ok(m.data()[0] != 0.),
        Value::Container(Array::Typed(t)) if t.len() == 1 => Ok(t.is_nonzero(0)),
        _ => Err(bad()),
    }
}

/// Name/value pairs with case-insensitive, unique names.
fn rdx_pairs<'a>(function: &str, args: &'a [Value]) -> Result<Vec<(String, &'a Value)>> {
    if !args.len().is_multiple_of(2) {
        return Err(format!(
            "{function}: options must be given as name/value pairs"
        ));
    }
    let mut seen = BTreeSet::new();
    let mut pairs = Vec::with_capacity(args.len() / 2);
    for pair in args.as_chunks::<2>().0 {
        let name = rdx_text(&pair[0])
            .ok_or_else(|| format!("{function}: option names must be text"))?
            .to_ascii_lowercase();
        if !seen.insert(name.clone()) {
            return Err(format!("{function}: option '{name}' is given twice"));
        }
        pairs.push((name, &pair[1]));
    }
    Ok(pairs)
}

fn rdx_extension(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty())
        .map(str::to_ascii_lowercase)
}

fn rdx_is_sheet_extension(filename: &str) -> bool {
    rdx_extension(filename).is_some_and(|e| SHEET_EXTENSIONS.contains(&e.as_str()))
}

fn rdx_positive_integer(function: &str, option: &str, value: &Value) -> Result<usize> {
    let bad = || format!("{function}: '{option}' must be a nonnegative integer");
    let v = match value {
        Value::Matrix(m) if m.len() == 1 => m.data()[0],
        Value::Container(Array::Typed(t)) if t.len() == 1 => t.real(0),
        _ => return Err(bad()),
    };
    if !(v >= 0. && v.fract() == 0. && v <= 1e9) {
        return Err(bad());
    }
    Ok(v as usize)
}

// ---------------------------------------------------------------------------
// Cell ranges ('A1:C10', 'B2')

/// Zero-based inclusive rectangle; `None` ends extend to the data's end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CellRange {
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) last_row: Option<usize>,
    pub(super) last_col: Option<usize>,
}

const MAX_SHEET_ROWS: usize = 1_048_576;
const MAX_SHEET_COLS: usize = 16_384;

fn rdx_cell_ref(text: &str) -> Option<(usize, usize)> {
    let text = text.trim();
    let split = text.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = text.split_at(split);
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut col = 0usize;
    for b in letters.bytes() {
        col = col * 26 + usize::from(b.to_ascii_uppercase() - b'A' + 1);
    }
    let row: usize = digits.parse().ok()?;
    if row == 0 || row > MAX_SHEET_ROWS || col > MAX_SHEET_COLS {
        return None;
    }
    Some((row - 1, col - 1))
}

fn rdx_range(function: &str, text: &str) -> Result<CellRange> {
    let bad = || {
        format!(
            "{function}: Range must be a cell 'B2' or a rectangle 'A1:C10' (named ranges are not supported)"
        )
    };
    match text.split_once(':') {
        Some((a, b)) => {
            let (r0, c0) = rdx_cell_ref(a).ok_or_else(bad)?;
            let (r1, c1) = rdx_cell_ref(b).ok_or_else(bad)?;
            Ok(CellRange {
                row: r0.min(r1),
                col: c0.min(c1),
                last_row: Some(r0.max(r1)),
                last_col: Some(c0.max(c1)),
            })
        }
        None => {
            let (row, col) = rdx_cell_ref(text).ok_or_else(bad)?;
            Ok(CellRange {
                row,
                col,
                last_row: None,
                last_col: None,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// JSON parsing

#[derive(Debug)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct JsonParser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    values: usize,
    cancel: &'a AtomicBool,
}

impl JsonParser<'_> {
    fn error(&self, message: &str) -> String {
        let position = self.text[..self.at.min(self.text.len())].chars().count() + 1;
        format!("jsondecode: invalid JSON text at character {position}: {message}")
    }
    fn skip_space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.at) {
            self.at += 1;
        }
    }
    fn literal(&mut self, word: &str) -> bool {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.error(&format!("nesting deeper than {MAX_JSON_DEPTH} levels")));
        }
        self.values += 1;
        if self.values.is_multiple_of(4096) {
            rdx_check(self.cancel)?;
        }
        self.skip_space();
        let Some(&b) = self.bytes.get(self.at) else {
            return Err(self.error("unexpected end of text"));
        };
        match b {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::Str),
            b't' if self.literal("true") => Ok(Json::Bool(true)),
            b'f' if self.literal("false") => Ok(Json::Bool(false)),
            b'n' if self.literal("null") => Ok(Json::Null),
            b'N' if self.literal("NaN") => Ok(Json::Num(f64::NAN)),
            b'I' if self.literal("Infinity") || self.literal("Inf") => Ok(Json::Num(f64::INFINITY)),
            b'-' if self.literal("-Infinity") || self.literal("-Inf") => {
                Ok(Json::Num(f64::NEG_INFINITY))
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.error("expected a value")),
        }
    }
    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        self.at - start
    }
    fn number(&mut self) -> Result<Json> {
        let start = self.at;
        if self.bytes[self.at] == b'-' {
            self.at += 1;
        }
        match self.bytes.get(self.at) {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.error("invalid number")),
        }
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err(self.error("invalid number: digits must follow the decimal point"));
            }
        }
        if let Some(b'e' | b'E') = self.bytes.get(self.at) {
            self.at += 1;
            if let Some(b'+' | b'-') = self.bytes.get(self.at) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(self.error("invalid number: the exponent has no digits"));
            }
        }
        self.text[start..self.at]
            .parse::<f64>()
            .map(Json::Num)
            .map_err(|_| self.error("invalid number"))
    }
    fn hex4(&mut self) -> Result<u32> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| self.error("\\u must be followed by four hexadecimal digits"))?;
        self.at += 4;
        Ok(u32::from_str_radix(digits, 16).expect("hex digits"))
    }
    fn string(&mut self) -> Result<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(&b) = self.bytes.get(self.at) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            out.push_str(&self.text[start..self.at]);
            match self.bytes.get(self.at) {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let Some(&e) = self.bytes.get(self.at) else {
                        return Err(self.error("unterminated string"));
                    };
                    self.at += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let high = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&high)
                                && self.bytes[self.at..].starts_with(b"\\u")
                            {
                                let save = self.at;
                                self.at += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                                } else {
                                    self.at = save;
                                    0xFFFD
                                }
                            } else {
                                high
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => {
                            self.at -= 1;
                            return Err(self.error("invalid escape sequence"));
                        }
                    }
                }
                Some(_) => return Err(self.error("control character inside a string")),
            }
        }
    }
    fn array(&mut self, depth: usize) -> Result<Json> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(self.error("expected ',' or ']' in an array")),
            }
        }
    }
    fn object(&mut self, depth: usize) -> Result<Json> {
        self.at += 1;
        let mut members = Vec::new();
        self.skip_space();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Json::Obj(members));
        }
        loop {
            self.skip_space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return Err(self.error("expected a quoted member name"));
            }
            let name = self.string()?;
            self.skip_space();
            if self.bytes.get(self.at) != Some(&b':') {
                return Err(self.error("expected ':' after a member name"));
            }
            self.at += 1;
            members.push((name, self.value(depth + 1)?));
            self.skip_space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Obj(members));
                }
                _ => return Err(self.error("expected ',' or '}' in an object")),
            }
        }
    }
}

fn rdx_parse_json(text: &str, cancel: &AtomicBool) -> Result<Json> {
    let mut parser = JsonParser {
        text,
        bytes: text.as_bytes(),
        at: 0,
        values: 0,
        cancel,
    };
    parser.skip_space();
    if parser.at == text.len() {
        return Err("jsondecode: the JSON text is empty".into());
    }
    let value = parser.value(0)?;
    parser.skip_space();
    if parser.at != text.len() {
        return Err(parser.error("unexpected text after the JSON value"));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Field names

/// Valid identifier from arbitrary text, following the published
/// matlab.lang.makeValidName rules: whitespace is removed and a lowercase
/// letter after it is capitalized, other invalid characters become '_', a
/// name not starting with a letter gets an 'x' prefix, a keyword becomes
/// 'x' + capitalized keyword, and the result is cut to 63 characters.
pub(super) fn rdx_make_valid_name(text: &str) -> String {
    make_valid_name(text, Replacement::Underscore, "x")
}

/// matlab.lang.makeValidName ReplacementStyle.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Replacement {
    Underscore,
    Delete,
    Hex,
}

/// The general form: `prefix` (itself valid) goes before names that do not
/// start with a letter and before keywords (then capitalised).
pub(super) fn make_valid_name(text: &str, style: Replacement, prefix: &str) -> String {
    if valid_field_name(text) {
        return text.to_string();
    }
    let mut out = String::new();
    let mut capital = false;
    for ch in text.trim().chars() {
        if ch.is_whitespace() {
            capital = true;
            continue;
        }
        let ch = if capital { ch.to_ascii_uppercase() } else { ch };
        capital = false;
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            match style {
                Replacement::Underscore => out.push('_'),
                Replacement::Delete => {}
                Replacement::Hex => {
                    let mut units = [0u16; 2];
                    for unit in ch.encode_utf16(&mut units) {
                        out.push_str(&format!("0x{unit:02X}"));
                    }
                }
            }
        }
    }
    if !out.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) {
        out.insert_str(0, prefix);
    } else if crate::syntax::keyword(&out) {
        let mut chars = out.chars();
        let first = chars.next().expect("nonempty").to_ascii_uppercase();
        out = format!("{prefix}{first}{}", chars.as_str());
    }
    out.truncate(MAX_FIELD_NAME);
    out
}

/// Field names for JSON member names: made valid (or rejected) and then made
/// unique by appending _1, _2, ... to later duplicates.
fn rdx_field_names<'a>(
    keys: impl Iterator<Item = &'a str>,
    make_valid: bool,
) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for key in keys {
        if valid_field_name(key) {
            names.push(key.to_owned());
        } else if make_valid {
            names.push(rdx_make_valid_name(key));
        } else {
            return Err(format!(
                "jsondecode: member name '{key}' is not a valid field name; use 'makeValidName', true"
            ));
        }
    }
    let all: BTreeSet<String> = names.iter().cloned().collect();
    let mut used = BTreeSet::new();
    for name in &mut names {
        if used.insert(name.clone()) {
            continue;
        }
        let mut k = 1usize;
        loop {
            let suffix = format!("_{k}");
            let mut base = name.clone();
            base.truncate(MAX_FIELD_NAME - suffix.len());
            let candidate = base + &suffix;
            if !all.contains(&candidate) && !used.contains(&candidate) {
                used.insert(candidate.clone());
                *name = candidate;
                break;
            }
            k += 1;
        }
    }
    Ok(names)
}

// ---------------------------------------------------------------------------
// JSON -> values

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Num,
    Bool,
    Null,
}

/// Shape and element kind of a rectangular array of numbers (nulls allowed)
/// or booleans; `None` when the array must become a cell or struct array.
fn rdx_shape(value: &Json) -> Option<(Kind, Vec<usize>)> {
    match value {
        Json::Num(_) => Some((Kind::Num, Vec::new())),
        Json::Bool(_) => Some((Kind::Bool, Vec::new())),
        Json::Null => Some((Kind::Null, Vec::new())),
        Json::Arr(items) if !items.is_empty() => {
            let (mut kind, inner) = rdx_shape(&items[0])?;
            for item in &items[1..] {
                let (k, dims) = rdx_shape(item)?;
                if dims != inner {
                    return None;
                }
                kind = match (kind, k) {
                    (a, b) if a == b => a,
                    (Kind::Num, Kind::Null) | (Kind::Null, Kind::Num) => Kind::Num,
                    _ => return None,
                };
            }
            let mut dims = vec![items.len()];
            dims.extend(inner);
            Some((kind, dims))
        }
        _ => None,
    }
}

/// Column-major fill of a rectangular nested array.
fn rdx_fill<T: Copy>(
    value: &Json,
    level: usize,
    offset: usize,
    strides: &[usize],
    out: &mut [T],
    convert: &impl Fn(&Json) -> T,
) {
    match value {
        Json::Arr(items) => {
            for (i, item) in items.iter().enumerate() {
                rdx_fill(
                    item,
                    level + 1,
                    offset + i * strides[level],
                    strides,
                    out,
                    convert,
                );
            }
        }
        scalar => out[offset] = convert(scalar),
    }
}

struct Decoder<'a> {
    make_valid: bool,
    cancel: &'a AtomicBool,
}

impl Decoder<'_> {
    fn decode(&self, value: &Json) -> Result<Array> {
        Ok(match value {
            Json::Null => Matrix::filled(0, 0, 0.)?.into(),
            Json::Bool(b) => LogicalMatrix::scalar(*b).into(),
            Json::Num(n) => Matrix::scalar(*n).into(),
            Json::Str(s) => CharacterRow::from_text(s, self.cancel)?.into(),
            Json::Obj(members) => {
                let names =
                    rdx_field_names(members.iter().map(|(k, _)| k.as_str()), self.make_valid)?;
                let mut slots = Vec::with_capacity(members.len());
                for (_, v) in members {
                    slots.push(Arc::new(self.decode(v)?));
                }
                Array::Struct(StructArray::from_slots(1, 1, names, slots, self.cancel)?)
            }
            Json::Arr(items) => self.array(value, items)?,
        })
    }

    fn array(&self, whole: &Json, items: &[Json]) -> Result<Array> {
        rdx_check(self.cancel)?;
        if items.is_empty() {
            return Ok(Matrix::filled(0, 0, 0.)?.into());
        }
        if let Some((kind, dims)) = rdx_shape(whole)
            && kind != Kind::Null
        {
            return self.rectangular(whole, kind, dims);
        }
        if let Some(array) = self.struct_array(items)? {
            return Ok(array);
        }
        let mut slots = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            if i.is_multiple_of(1024) {
                rdx_check(self.cancel)?;
            }
            slots.push(Arc::new(self.decode(item)?));
        }
        Ok(Array::Cell(CellArray::from_shared(
            items.len(),
            1,
            slots,
            self.cancel,
        )?))
    }

    fn rectangular(&self, whole: &Json, kind: Kind, mut dims: Vec<usize>) -> Result<Array> {
        let mut count = 1usize;
        for d in &dims {
            count = count
                .checked_mul(*d)
                .filter(|c| *c <= MAX_GRID_CELLS)
                .ok_or("jsondecode: numeric array is too large")?;
        }
        let mut strides = Vec::with_capacity(dims.len());
        let mut stride = 1;
        for d in &dims {
            strides.push(stride);
            stride *= d;
        }
        if dims.len() == 1 {
            dims.push(1);
        }
        let rows = dims[0];
        let cols = count / rows.max(1);
        let array: Array = if kind == Kind::Bool {
            let mut data = vec![0u8; count];
            rdx_fill(whole, 0, 0, &strides, &mut data, &|j| {
                u8::from(matches!(j, Json::Bool(true)))
            });
            LogicalMatrix::from_column_vec(rows, cols, data, self.cancel)?.into()
        } else {
            let mut data = vec![0f64; count];
            rdx_fill(whole, 0, 0, &strides, &mut data, &|j| match j {
                Json::Num(n) => *n,
                _ => f64::NAN,
            });
            Matrix::from_column_vec(rows, cols, data)?.into()
        };
        rdx_check(self.cancel)?;
        if dims.len() > 2 {
            array.with_dims(&dims)
        } else {
            Ok(array)
        }
    }

    /// An N-by-1 struct array when every item is an object with the same
    /// field names (in any order; the first object's order is kept).
    fn struct_array(&self, items: &[Json]) -> Result<Option<Array>> {
        let mut per_item = Vec::with_capacity(items.len());
        for item in items {
            let Json::Obj(members) = item else {
                return Ok(None);
            };
            let names = rdx_field_names(members.iter().map(|(k, _)| k.as_str()), self.make_valid)?;
            per_item.push((names, members));
        }
        let first: Vec<String> = per_item[0].0.clone();
        let first_set: BTreeSet<&String> = first.iter().collect();
        if per_item
            .iter()
            .any(|(names, _)| names.iter().collect::<BTreeSet<_>>() != first_set)
        {
            return Ok(None);
        }
        let mut slots = Vec::with_capacity(items.len() * first.len());
        for (i, (names, members)) in per_item.iter().enumerate() {
            if i.is_multiple_of(256) {
                rdx_check(self.cancel)?;
            }
            for name in &first {
                let index = names.iter().position(|n| n == name).expect("same names");
                slots.push(Arc::new(self.decode(&members[index].1)?));
            }
        }
        Ok(Some(Array::Struct(StructArray::from_slots(
            items.len(),
            1,
            first,
            slots,
            self.cancel,
        )?)))
    }
}

// ---------------------------------------------------------------------------
// values -> JSON

/// Shortest round-trip decimal text; integers below 1e15 without a fraction,
/// exponent form outside [1e-5, 1e15).
fn rdx_number_text(x: f64) -> String {
    if x == 0. {
        return "0".into();
    }
    let a = x.abs();
    if x.fract() == 0. && a < 1e15 {
        return format!("{}", x as i64);
    }
    if (1e-5..1e15).contains(&a) {
        format!("{x}")
    } else {
        format!("{x:e}")
    }
}

fn rdx_single_text(x: f32) -> String {
    if x == 0. {
        return "0".into();
    }
    let a = x.abs();
    if x.fract() == 0. && a < 1e7 {
        return format!("{}", x as i64);
    }
    if (1e-5..1e7).contains(&a) {
        format!("{x}")
    } else {
        format!("{x:e}")
    }
}

struct Encoder<'a> {
    engine: &'a Engine,
    pretty: bool,
    convert: bool,
    out: String,
    steps: usize,
}

impl Encoder<'_> {
    fn grow(&mut self) -> Result<()> {
        self.steps += 1;
        if self.steps.is_multiple_of(4096) {
            rdx_check(&self.engine.cancel)?;
        }
        if self.out.len() > MAX_JSON_BYTES {
            return Err(format!(
                "jsonencode: the JSON text would exceed {} MiB",
                MAX_JSON_BYTES >> 20
            ));
        }
        Ok(())
    }
    fn newline(&mut self, level: usize) {
        if self.pretty {
            self.out.push('\n');
            for _ in 0..level {
                self.out.push_str("  ");
            }
        }
    }
    fn number(&mut self, x: f64, single: bool) {
        if x.is_finite() {
            let text = if single {
                rdx_single_text(x as f32)
            } else {
                rdx_number_text(x)
            };
            self.out.push_str(&text);
        } else if self.convert {
            self.out.push_str("null");
        } else if x.is_nan() {
            self.out.push_str("NaN");
        } else if x > 0. {
            self.out.push_str("Infinity");
        } else {
            self.out.push_str("-Infinity");
        }
    }
    fn string(&mut self, text: &str) {
        self.out.push('"');
        for ch in text.chars() {
            match ch {
                '"' => self.out.push_str("\\\""),
                '\\' => self.out.push_str("\\\\"),
                '\n' => self.out.push_str("\\n"),
                '\r' => self.out.push_str("\\r"),
                '\t' => self.out.push_str("\\t"),
                '\u{8}' => self.out.push_str("\\b"),
                '\u{c}' => self.out.push_str("\\f"),
                c if (c as u32) < 0x20 => {
                    self.out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => self.out.push(c),
            }
        }
        self.out.push('"');
    }
    fn units(&mut self, units: &[u16]) {
        self.string(&String::from_utf16_lossy(units));
    }
    /// A JSON array of `items` written by `item`.
    fn list(
        &mut self,
        count: usize,
        level: usize,
        item: &mut dyn FnMut(&mut Self, usize, usize) -> Result<()>,
    ) -> Result<()> {
        self.out.push('[');
        for i in 0..count {
            if i > 0 {
                self.out.push(',');
            }
            self.newline(level + 1);
            item(self, i, level + 1)?;
            self.grow()?;
        }
        if count > 0 {
            self.newline(level);
        }
        self.out.push(']');
        Ok(())
    }
    /// Arrays: a scalar is written alone, a vector as one flat list, a
    /// matrix as a list of rows, and N-D arrays as lists nested by dimension.
    fn shaped(
        &mut self,
        dims: &[usize],
        level: usize,
        element: &mut dyn FnMut(&mut Self, usize, usize) -> Result<()>,
    ) -> Result<()> {
        let count: usize = dims.iter().product();
        if count == 0 {
            self.out.push_str("[]");
            return Ok(());
        }
        if count == 1 {
            return element(self, 0, level);
        }
        if dims.len() == 2 && (dims[0] == 1 || dims[1] == 1) {
            return self.list(count, level, &mut |e, i, l| element(e, i, l));
        }
        let mut strides = Vec::with_capacity(dims.len());
        let mut stride = 1;
        for d in dims {
            strides.push(stride);
            stride *= d;
        }
        self.nested(dims, &strides, 0, 0, level, element)
    }
    fn nested(
        &mut self,
        dims: &[usize],
        strides: &[usize],
        dim: usize,
        offset: usize,
        level: usize,
        element: &mut dyn FnMut(&mut Self, usize, usize) -> Result<()>,
    ) -> Result<()> {
        if dim == dims.len() {
            return element(self, offset, level);
        }
        self.list(dims[dim], level, &mut |e, i, l| {
            e.nested(
                dims,
                strides,
                dim + 1,
                offset + i * strides[dim],
                l,
                element,
            )
        })
    }
    fn object(&mut self, fields: &[(String, Arc<Array>)], level: usize) -> Result<()> {
        if fields.is_empty() {
            self.out.push_str("{}");
            return Ok(());
        }
        self.out.push('{');
        for (i, (name, value)) in fields.iter().enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            self.newline(level + 1);
            self.string(name);
            self.out.push_str(if self.pretty { ": " } else { ":" });
            self.array(value, level + 1)?;
            self.grow()?;
        }
        self.newline(level);
        self.out.push('}');
        Ok(())
    }
    fn struct_element(&mut self, s: &StructArray, element: usize, level: usize) -> Result<()> {
        let mut fields = Vec::with_capacity(s.field_count());
        for (f, name) in s.field_names().iter().enumerate() {
            fields.push((name.clone(), s.get(f, element)?.clone()));
        }
        self.object(&fields, level)
    }
    fn value(&mut self, value: &Value) -> Result<()> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => {
                self.string(s);
                Ok(())
            }
            Value::Void => Err("jsonencode: the input has no value".into()),
            other => {
                let array = other.clone().array(&self.engine.cancel)?;
                self.array(&array, 0)
            }
        }
    }
    fn array(&mut self, array: &Array, level: usize) -> Result<()> {
        self.grow()?;
        match array {
            Array::Numeric(m) => {
                let data = m.data();
                self.shaped(&m.dims(), level, &mut |e, i, _| {
                    e.number(data[i], false);
                    Ok(())
                })
            }
            Array::Logical(m) => {
                let data = m.data();
                self.shaped(&m.dims(), level, &mut |e, i, _| {
                    e.out.push_str(if data[i] != 0 { "true" } else { "false" });
                    Ok(())
                })
            }
            Array::Typed(t) => {
                let single = !t.is_integer();
                self.shaped(&t.dims(), level, &mut |e, i, _| {
                    if single {
                        e.number(t.real(i), true);
                    } else {
                        e.out.push_str(&t.int(i).to_string());
                    }
                    Ok(())
                })
            }
            Array::Complex(_) => Err("jsonencode: complex values are not supported".into()),
            Array::Categorical(_) => Err("categorical is not supported by jsonencode yet".into()),
            Array::Datetime(_) | Array::Duration(_) => {
                Err("jsonencode: datetime and duration values are not supported yet".into())
            }
            Array::Table(_) => Err("jsonencode: tables are not supported yet".into()),
            Array::Sparse(_) => Err("jsonencode: sparse arrays are not supported yet".into()),
            Array::Character(c) => {
                let (rows, cols) = (c.rows(), c.cols());
                if rows <= 1 || c.is_nd() {
                    if c.is_nd() {
                        return Err("jsonencode: N-D character arrays are not supported".into());
                    }
                    self.units(c.units());
                    return Ok(());
                }
                let units = c.units();
                self.list(rows, level, &mut |e, r, _| {
                    let row: Vec<u16> = (0..cols).map(|j| units[r + j * rows]).collect();
                    e.units(&row);
                    Ok(())
                })
            }
            Array::Str(s) => {
                let data = s.data();
                let dims = [s.rows(), s.cols()];
                if data.is_empty() {
                    self.out.push_str("[]");
                    return Ok(());
                }
                self.shaped(&dims, level, &mut |e, i, _| {
                    match &data[i] {
                        Some(text) => e.string(text),
                        None => e.out.push_str("null"),
                    }
                    Ok(())
                })
            }
            Array::Cell(c) => {
                let count = c.len();
                if count == 0 {
                    self.out.push_str("[]");
                    return Ok(());
                }
                self.list(count, level, &mut |e, i, l| {
                    let item = c.get_linear(i)?.clone();
                    e.array(&item, l)
                })
            }
            Array::Struct(s) => {
                let wrapped = Value::Container(array.clone());
                if s.len() == 1
                    && s.field_count() == 1
                    && let Some(entries) = self.engine.obj_map_text_entries(&wrapped)
                {
                    return self.object(&entries?, level);
                }
                if s.is_empty() {
                    self.out.push_str("[]");
                    return Ok(());
                }
                self.shaped(&[s.rows(), s.cols()], level, &mut |e, i, l| {
                    e.struct_element(s, i, l)
                })
            }
            Array::Handle(_) => Err("jsonencode: function handles cannot be encoded".into()),
            Array::Graphics(_) => Err("jsonencode: graphics handles cannot be encoded".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Delimited text

/// One field of a text record: its text and whether it was quoted.
type Field = (String, bool);

/// Records of a delimited text, one per physical line outside quotes. Blank
/// lines are kept as `None` so header and range counts see every line.
fn rdx_records(
    text: &str,
    delimiters: &[char],
    cancel: &AtomicBool,
) -> Result<Vec<Option<Vec<Field>>>> {
    let mut records = Vec::new();
    let mut chars = text.chars().peekable();
    let join = delimiters.contains(&' ');
    let is_delim = |c: char| delimiters.contains(&c) || (join && c == '\t');
    let mut fields: Vec<Field> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut line_has_content = false;
    let mut count = 0usize;
    let finish_line = |fields: &mut Vec<Field>,
                       field: &mut String,
                       quoted: &mut bool,
                       content: &mut bool,
                       records: &mut Vec<Option<Vec<Field>>>| {
        if *content {
            fields.push((std::mem::take(field), *quoted));
            records.push(Some(std::mem::take(fields)));
        } else {
            records.push(None);
        }
        field.clear();
        *quoted = false;
        *content = false;
    };
    while let Some(ch) = chars.next() {
        count += 1;
        if count.is_multiple_of(65536) {
            rdx_check(cancel)?;
        }
        match ch {
            '"' if field.trim().is_empty() && !quoted => {
                field.clear();
                quoted = true;
                line_has_content = true;
                // Quoted text up to the closing quote; "" is a literal quote.
                loop {
                    match chars.next() {
                        None => break,
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            field.push('"');
                        }
                        Some('"') => break,
                        Some(c) => field.push(c),
                    }
                }
                // Text after the closing quote and before the delimiter is dropped.
                while let Some(&c) = chars.peek() {
                    if is_delim(c) || c == '\n' || c == '\r' {
                        break;
                    }
                    chars.next();
                }
            }
            '\r' | '\n' => {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                finish_line(
                    &mut fields,
                    &mut field,
                    &mut quoted,
                    &mut line_has_content,
                    &mut records,
                );
            }
            c if is_delim(c) => {
                if join && (c == ' ' || c == '\t') && !quoted && field.trim().is_empty() {
                    // Runs of spaces count as one delimiter.
                    if !line_has_content {
                        continue;
                    }
                    if fields.is_empty() && field.is_empty() {
                        continue;
                    }
                    if field.is_empty() {
                        continue;
                    }
                }
                line_has_content = true;
                fields.push((std::mem::take(&mut field), quoted));
                quoted = false;
            }
            c => {
                if !c.is_whitespace() {
                    line_has_content = true;
                }
                field.push(c);
            }
        }
    }
    if line_has_content || !fields.is_empty() {
        line_has_content = true;
        finish_line(
            &mut fields,
            &mut field,
            &mut quoted,
            &mut line_has_content,
            &mut records,
        );
    }
    if join {
        // A trailing space before the line end leaves one empty last field.
        for fields in records.iter_mut().flatten() {
            while fields.len() > 1
                && fields
                    .last()
                    .is_some_and(|(t, q)| !q && t.trim().is_empty())
            {
                fields.pop();
            }
        }
    }
    Ok(records)
}

/// Delimiter detected from the first lines: the candidate appearing most
/// often outside quotes (comma, tab, semicolon, bar, in that order on ties).
/// When none occurs, use space.
fn rdx_detect_delimiter(text: &str) -> char {
    let candidates = [',', '\t', ';', '|'];
    let mut counts = [0usize; 4];
    let mut inside = false;
    for line in text.lines().filter(|l| !l.trim().is_empty()).take(32) {
        for c in line.chars() {
            if c == '"' {
                inside = !inside;
            } else if !inside && let Some(i) = candidates.iter().position(|d| *d == c) {
                counts[i] += 1;
            }
        }
    }
    let mut best = 0;
    for i in 1..4 {
        if counts[i] > counts[best] {
            best = i;
        }
    }
    if counts[best] == 0 {
        ' '
    } else {
        candidates[best]
    }
}

fn rdx_delimiter_char(function: &str, text: &str, writing: bool) -> Result<char> {
    Ok(match text {
        "," => ',',
        ";" => ';',
        "\t" | "\\t" => '\t',
        "|" => '|',
        " " => ' ',
        t if t.eq_ignore_ascii_case("comma") => ',',
        t if t.eq_ignore_ascii_case("semi") || t.eq_ignore_ascii_case("semicolon") => ';',
        t if t.eq_ignore_ascii_case("tab") => '\t',
        t if t.eq_ignore_ascii_case("bar") => '|',
        t if t.eq_ignore_ascii_case("space") || t.eq_ignore_ascii_case("whitespace") => ' ',
        _ => {
            return Err(format!(
                "{function}: supported delimiters are comma, semicolon, tab, bar and space{}",
                if writing { "" } else { " (one character)" }
            ));
        }
    })
}

fn rdx_delimiter(function: &str, value: &Value, writing: bool) -> Result<Vec<char>> {
    if !writing && let Some(list) = super::import_options::text_list(value) {
        if list.is_empty() {
            return Err(format!("{function}: Delimiter must not be empty"));
        }
        let mut delimiters = Vec::with_capacity(list.len());
        for item in &list {
            let ch = rdx_delimiter_char(function, item, false)?;
            if !delimiters.contains(&ch) {
                delimiters.push(ch);
            }
        }
        return Ok(delimiters);
    }
    let text = rdx_text(value).ok_or_else(|| format!("{function}: Delimiter must be text"))?;
    Ok(vec![rdx_delimiter_char(function, &text, writing)?])
}

/// The `missing` placeholder readcell stores for empty fields.
fn rdx_missing() -> Result<Array> {
    Ok(Array::Str(StringArray::new(1, 1, vec![None])?))
}

fn rdx_text_cell(
    text: &str,
    quoted: bool,
    delimiters: &[char],
    cancel: &AtomicBool,
) -> Result<Array> {
    let trimmed = if quoted {
        text
    } else if delimiters.contains(&'\t') && !delimiters.contains(&' ') {
        text.trim_matches(' ')
    } else {
        text.trim_matches([' ', '\t'])
    };
    if trimmed.is_empty() {
        return rdx_missing();
    }
    if !quoted && let Some(number) = rdx_parse_number(trimmed) {
        return Ok(Matrix::scalar(number).into());
    }
    Ok(CharacterRow::from_text(trimmed, cancel)?.into())
}

/// A decimal number, Inf or NaN (any letter case, optional sign).
fn rdx_parse_number(text: &str) -> Option<f64> {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let lower = body.to_ascii_lowercase();
    let named = matches!(lower.as_str(), "inf" | "nan");
    if !named
        && !body
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    if !named
        && !body
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    text.parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// Spreadsheet cells

/// One spreadsheet cell, independent of the reader crate.
#[derive(Clone, Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) enum SheetCell {
    Empty,
    Number(f64),
    Bool(bool),
    Text(String),
    Error,
}

/// A rectangle of cells, row-major. `first_row` is the zero-based sheet row
/// of its top row.
pub(super) struct Grid {
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) first_row: usize,
    cells: Vec<SheetCell>,
}
impl Grid {
    pub(super) fn at(&self, r: usize, c: usize) -> &SheetCell {
        &self.cells[r * self.cols + c]
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) enum SheetChoice {
    First,
    Name(String),
    Index(usize),
}

fn rdx_sheet_choice(function: &str, value: &Value) -> Result<SheetChoice> {
    if let Some(name) = rdx_text(value) {
        return Ok(if name.is_empty() {
            SheetChoice::First
        } else {
            SheetChoice::Name(name)
        });
    }
    let index = rdx_positive_integer(function, "Sheet", value)?;
    if index == 0 {
        return Err(format!("{function}: Sheet index must be 1 or greater"));
    }
    Ok(SheetChoice::Index(index))
}

/// A1-style name of a zero-based cell, for error messages.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_cell_name(row: usize, col: usize) -> String {
    let mut letters = Vec::new();
    let mut c = col + 1;
    while c > 0 {
        letters.push(b'A' + ((c - 1) % 26) as u8);
        c = (c - 1) / 26;
    }
    letters.reverse();
    format!("{}{}", String::from_utf8_lossy(&letters), row + 1)
}

/// Non-empty cells streamed from a worksheet. Only cells inside the
/// requested window are kept, and the dense grid is allocated only after
/// its size passed the cell limit, so a small file whose cells lie far
/// apart cannot allocate a huge rectangle.
#[cfg(not(target_arch = "wasm32"))]
struct SheetCells {
    window: Option<CellRange>,
    max_cells: usize,
    /// Smallest and largest row and column of every non-empty cell.
    used: Option<(usize, usize, usize, usize)>,
    kept: Vec<(u32, u32, SheetCell)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SheetCells {
    fn new(window: Option<CellRange>, max_cells: usize) -> Self {
        Self {
            window,
            max_cells,
            used: None,
            kept: Vec::new(),
        }
    }

    fn push(&mut self, (r, c): (u32, u32), cell: SheetCell) -> Result<()> {
        let (row, col) = (r as usize, c as usize);
        self.used = Some(match self.used {
            None => (row, col, row, col),
            Some((r0, c0, r1, c1)) => (r0.min(row), c0.min(col), r1.max(row), c1.max(col)),
        });
        let inside = self.window.is_none_or(|w| {
            row >= w.row
                && col >= w.col
                && w.last_row.is_none_or(|l| row <= l)
                && w.last_col.is_none_or(|l| col <= l)
        });
        if inside {
            if self.kept.len() >= self.max_cells {
                return Err(format!(
                    "The sheet has more than {} cells to read; give a smaller Range",
                    self.max_cells
                ));
            }
            self.kept.push((r, c, cell));
        }
        Ok(())
    }

    /// The dense rectangle: the Range (an open end stops at the last used
    /// row or column), or without a Range the used area of the sheet.
    fn finish(self, cancel: &AtomicBool) -> Result<Grid> {
        let empty = |first_row| Grid {
            rows: 0,
            cols: 0,
            first_row,
            cells: Vec::new(),
        };
        let (row, col, last_row, last_col) = match (self.window, self.used) {
            (Some(r), Some((_, _, er, ec))) => (
                r.row,
                r.col,
                r.last_row.unwrap_or(er.max(r.row)),
                r.last_col.unwrap_or(ec.max(r.col)),
            ),
            (Some(r), None) => match (r.last_row, r.last_col) {
                (Some(lr), Some(lc)) => (r.row, r.col, lr, lc),
                _ => return Ok(empty(r.row)),
            },
            (None, Some(used)) => used,
            (None, None) => return Ok(empty(0)),
        };
        let rows = last_row + 1 - row;
        let cols = last_col + 1 - col;
        let count = rows
            .checked_mul(cols)
            .filter(|c| *c <= self.max_cells)
            .ok_or_else(|| {
                format!(
                    "The sheet range {}:{} has more than {} cells; give a smaller Range",
                    rdx_cell_name(row, col),
                    rdx_cell_name(last_row, last_col),
                    self.max_cells
                )
            })?;
        rdx_check(cancel)?;
        let mut cells = vec![SheetCell::Empty; count];
        for (r, c, cell) in self.kept {
            let (r, c) = (r as usize, c as usize);
            if (row..=last_row).contains(&r) && (col..=last_col).contains(&c) {
                cells[(r - row) * cols + (c - col)] = cell;
            }
        }
        Ok(Grid {
            rows,
            cols,
            first_row: row,
            cells,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn rdx_data_ref_cell(value: &calamine::DataRef<'_>) -> Option<SheetCell> {
    use calamine::DataRef;
    Some(match value {
        DataRef::Empty => return None,
        DataRef::Int(i) => SheetCell::Number(*i as f64),
        DataRef::Float(f) => SheetCell::Number(*f),
        DataRef::Bool(b) => SheetCell::Bool(*b),
        DataRef::DateTime(d) => SheetCell::Number(d.as_f64()),
        DataRef::String(s) | DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => {
            SheetCell::Text(s.clone())
        }
        DataRef::SharedString(s) => SheetCell::Text((*s).to_owned()),
        DataRef::Error(_) => SheetCell::Error,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn rdx_data_cell(value: &calamine::Data) -> Option<SheetCell> {
    use calamine::Data;
    Some(match value {
        Data::Empty => return None,
        Data::Int(i) => SheetCell::Number(*i as f64),
        Data::Float(f) => SheetCell::Number(*f),
        Data::Bool(b) => SheetCell::Bool(*b),
        Data::DateTime(d) => SheetCell::Number(d.as_f64()),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => SheetCell::Text(s.clone()),
        Data::Error(_) => SheetCell::Error,
    })
}

/// The worksheet `sheet` picks from `names`.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_sheet_name(names: &[String], sheet: &SheetChoice) -> Result<String> {
    if names.is_empty() {
        return Err("The workbook has no worksheets".into());
    }
    let list = || names.join(", ");
    match sheet {
        SheetChoice::First => Ok(names[0].clone()),
        SheetChoice::Index(i) => names.get(i - 1).cloned().ok_or_else(|| {
            format!(
                "Sheet {i} does not exist; the workbook has {} sheet(s): {}",
                names.len(),
                list()
            )
        }),
        SheetChoice::Name(n) => names
            .iter()
            .find(|s| *s == n)
            .or_else(|| names.iter().find(|s| s.eq_ignore_ascii_case(n)))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Sheet '{n}' does not exist; the workbook has sheets: {}",
                    list()
                )
            }),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, PartialEq)]
enum SheetFormat {
    Xlsx,
    Xlsb,
    Xls,
}

/// Workbook format from the extension, else from the first bytes (a zip
/// package or an OLE compound file).
#[cfg(not(target_arch = "wasm32"))]
fn rdx_sheet_format(path: &Path, bytes: &[u8]) -> Result<SheetFormat> {
    match rdx_extension(&path.to_string_lossy()).as_deref() {
        Some("xlsx" | "xlsm") => return Ok(SheetFormat::Xlsx),
        Some("xlsb") => return Ok(SheetFormat::Xlsb),
        Some("xls") => return Ok(SheetFormat::Xls),
        _ => {}
    }
    if bytes.starts_with(b"PK\x03\x04") {
        Ok(SheetFormat::Xlsx)
    } else if bytes.starts_with(CFB_SIGNATURE) {
        Ok(SheetFormat::Xls)
    } else {
        Err(format!(
            "Unable to read spreadsheet {}: not an .xlsx, .xlsm, .xlsb or .xls workbook",
            path.display()
        ))
    }
}

/// calamine can panic on a malformed workbook; a crafted file must give an error, not close the app.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn rdx_read_sheet(
    path: &Path,
    sheet: &SheetChoice,
    range: Option<CellRange>,
    max_cells: usize,
    cancel: &AtomicBool,
) -> Result<Grid> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rdx_read_sheet_unguarded(path, sheet, range, max_cells, cancel)
    }))
    .unwrap_or_else(|_| {
        Err(format!(
            "Unable to read spreadsheet {}: the workbook is malformed",
            path.display()
        ))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn rdx_read_sheet_unguarded(
    path: &Path,
    sheet: &SheetChoice,
    range: Option<CellRange>,
    max_cells: usize,
    cancel: &AtomicBool,
) -> Result<Grid> {
    use calamine::{Reader, Xls, Xlsb, Xlsx, XlsxError};
    use std::io::Cursor;
    rdx_check(cancel)?;
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Unable to open spreadsheet {}: {e}", path.display()))?
        .len();
    if size > MAX_SHEET_BYTES {
        return Err(format!(
            "Spreadsheet file exceeds {} MiB",
            MAX_SHEET_BYTES >> 20
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Unable to open spreadsheet {}: {e}", path.display()))?;
    rdx_check(cancel)?;
    let unreadable =
        |e: &dyn std::fmt::Display| format!("Unable to read spreadsheet {}: {e}", path.display());
    let sheet_error =
        |name: &str, e: &dyn std::fmt::Display| format!("Unable to read sheet '{name}': {e}");
    let mut cells = SheetCells::new(range, max_cells);
    let mut count = 0usize;
    let mut add = |cells: &mut SheetCells, at: (u32, u32), cell: Option<SheetCell>| {
        count += 1;
        if count.is_multiple_of(65536) {
            rdx_check(cancel)?;
        }
        match cell {
            Some(cell) => cells.push(at, cell),
            None => Ok(()),
        }
    };
    let mut format = rdx_sheet_format(path, &bytes)?;
    if format != SheetFormat::Xls {
        rdx_zip_check(&bytes).map_err(|e| unreadable(&e))?;
    }
    // An unknown extension on a zip package: an .xlsx, else an .xlsb.
    let xlsx = match format {
        SheetFormat::Xlsx => match Xlsx::new(Cursor::new(&bytes[..])) {
            Ok(book) => Some(book),
            Err(_) if !rdx_is_sheet_extension(&path.to_string_lossy()) => {
                format = SheetFormat::Xlsb;
                None
            }
            Err(e) => return Err(unreadable(&e)),
        },
        _ => None,
    };
    if let Some(mut book) = xlsx {
        let name = rdx_sheet_name(&book.sheet_names(), sheet)?;
        match book.worksheet_cells_reader(&name) {
            Ok(mut reader) => {
                while let Some(cell) = reader.next_cell().map_err(|e| sheet_error(&name, &e))? {
                    add(
                        &mut cells,
                        cell.get_position(),
                        rdx_data_ref_cell(cell.get_value()),
                    )?;
                }
            }
            // A chart sheet holds no cells.
            Err(XlsxError::NotAWorksheet(_)) => {}
            Err(e) => return Err(sheet_error(&name, &e)),
        }
    } else if format == SheetFormat::Xlsb {
        let mut book = Xlsb::new(Cursor::new(&bytes[..])).map_err(|e| unreadable(&e))?;
        let name = rdx_sheet_name(&book.sheet_names(), sheet)?;
        let mut reader = book
            .worksheet_cells_reader(&name)
            .map_err(|e| sheet_error(&name, &e))?;
        while let Some(cell) = reader.next_cell().map_err(|e| sheet_error(&name, &e))? {
            add(
                &mut cells,
                cell.get_position(),
                rdx_data_ref_cell(cell.get_value()),
            )?;
        }
    } else {
        // calamine reads a whole .xls workbook densely when it opens it.
        rdx_xls_check(&bytes, MAX_GRID_CELLS, cancel).map_err(|e| unreadable(&e))?;
        let mut book = Xls::new(Cursor::new(&bytes[..])).map_err(|e| unreadable(&e))?;
        let name = rdx_sheet_name(&book.sheet_names(), sheet)?;
        let data = book
            .worksheet_range(&name)
            .map_err(|e| sheet_error(&name, &e))?;
        if let Some((r0, c0)) = data.start() {
            for (r, c, value) in data.used_cells() {
                add(
                    &mut cells,
                    (r0 + r as u32, c0 + c as u32),
                    rdx_data_cell(value),
                )?;
            }
        }
    }
    cells.finish(cancel)
}

/// Every worksheet of an existing .xlsx with its non-empty cells (values
/// only), for xlswrite updating a workbook. Chart sheets are kept empty.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_read_workbook(
    path: &Path,
    max_cells: usize,
    max_text_bytes: usize,
    cancel: &AtomicBool,
) -> Result<Vec<xlsx_write::Sheet>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use calamine::{Reader, Xlsx, XlsxError};
        rdx_check(cancel)?;
        let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if size > MAX_SHEET_BYTES {
            return Err(format!("the file exceeds {} MiB", MAX_SHEET_BYTES >> 20));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if !bytes.starts_with(b"PK\x03\x04") {
            return Err("it is not an .xlsx workbook".into());
        }
        rdx_zip_check(&bytes)?;
        let mut book = Xlsx::new(std::io::Cursor::new(&bytes[..])).map_err(|e| e.to_string())?;
        let mut sheets = Vec::new();
        let mut total = 0usize;
        let mut text_bytes = 0usize;
        for name in book.sheet_names() {
            let mut cells = std::collections::BTreeMap::new();
            match book.worksheet_cells_reader(&name) {
                Ok(mut reader) => {
                    while let Some(cell) = reader.next_cell().map_err(|e| e.to_string())? {
                        if let Some(value) = rdx_data_ref_cell(cell.get_value()) {
                            total += 1;
                            if let SheetCell::Text(t) = &value {
                                text_bytes += t.len();
                                if text_bytes > max_text_bytes {
                                    return Err(format!(
                                        "its cells hold more than {} MiB of text",
                                        max_text_bytes >> 20
                                    ));
                                }
                            }
                            if total > max_cells {
                                return Err(format!("it holds more than {max_cells} cells"));
                            }
                            if total.is_multiple_of(65536) {
                                rdx_check(cancel)?;
                            }
                            cells.insert(cell.get_position(), value);
                        }
                    }
                }
                Err(XlsxError::NotAWorksheet(_)) => {}
                Err(e) => return Err(e.to_string()),
            }
            sheets.push(xlsx_write::Sheet { name, cells });
        }
        Ok(sheets)
    }))
    .unwrap_or_else(|_| Err("the workbook is malformed".into()))
}

/// Checks a zip package before calamine opens it: the central directory is
/// read without inflating anything, every part must stay under
/// `MAX_PART_BYTES` once inflated, and the `uniqueCount` calamine would
/// reserve for shared strings must be plausible (every `<si>` takes at
/// least 5 bytes, so a larger count than the part size is impossible).
#[cfg(not(target_arch = "wasm32"))]
fn rdx_zip_check(bytes: &[u8]) -> Result<()> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("not a readable zip package: {e}"))?;
    for i in 0..zip.len() {
        let part = zip
            .by_index_raw(i)
            .map_err(|e| format!("not a readable zip package: {e}"))?;
        if part.size() > MAX_PART_BYTES {
            return Err(format!(
                "part '{}' inflates to {} MiB, over the {} MiB limit",
                part.name().as_deref().unwrap_or("?"),
                part.size() >> 20,
                MAX_PART_BYTES >> 20
            ));
        }
    }
    if let Ok(part) = zip.by_name("xl/sharedStrings.xml") {
        let size = part.size();
        let mut head = Vec::new();
        part.take(4096)
            .read_to_end(&mut head)
            .map_err(|e| e.to_string())?;
        let head = String::from_utf8_lossy(&head);
        if let Some(at) = head.find("uniqueCount=") {
            let digits: String = head[at + 12..]
                .trim_start_matches(['"', '\''])
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let plausible = digits
                .parse::<u64>()
                .is_ok_and(|count| count <= size / 5 + 1);
            if !plausible {
                return Err(format!(
                    "shared strings part declares {digits} strings but holds only {size} bytes"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Legacy .xls pre-check

#[cfg(not(target_arch = "wasm32"))]
const CFB_SIGNATURE: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";

/// Checks a legacy .xls (BIFF in an OLE compound file) before calamine
/// opens it. calamine parses every sheet when it opens the workbook: it
/// reserves each sheet's declared DIMENSIONS, builds each sheet as a dense
/// rectangle and follows sector chains without loop checks. This walks the
/// same structures the same way and rejects workbooks whose declared or
/// used rectangles add up to more than `max_cells` cells, and damaged
/// sector chains.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_xls_check(bytes: &[u8], max_cells: usize, cancel: &AtomicBool) -> Result<()> {
    let stream = rdx_cfb_workbook(bytes)?;
    rdx_check(cancel)?;
    let mut sheets = Vec::new();
    for record in rdx_biff_records(&stream) {
        let (typ, data) = record?;
        match typ {
            0x0085 => sheets.push(rdx_u32(data, 0).ok_or("damaged sheet list")? as usize),
            0x000A => break,
            _ => {}
        }
    }
    let (mut declared, mut used) = (0usize, 0usize);
    for start in sheets {
        rdx_check(cancel)?;
        let substream = stream.get(start..).ok_or("damaged sheet position")?;
        let (sheet_declared, sheet_used) = rdx_biff_sheet_extent(substream)?;
        declared = declared.saturating_add(sheet_declared);
        used = used.saturating_add(sheet_used);
        if declared > max_cells || used > max_cells {
            return Err(format!(
                "the .xls workbook has more than {max_cells} cells in its sheets (an .xls file is read whole); save it as .xlsx"
            ));
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn rdx_u16(data: &[u8], at: usize) -> Option<u16> {
    data.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

#[cfg(not(target_arch = "wasm32"))]
fn rdx_u32(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// BIFF records (type, data) as calamine splits them: Continue records
/// after a record belong to it.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_biff_records(mut stream: &[u8]) -> impl Iterator<Item = Result<(u16, &[u8])>> {
    std::iter::from_fn(move || {
        if stream.is_empty() {
            return None;
        }
        let (Some(typ), Some(len)) = (rdx_u16(stream, 0), rdx_u16(stream, 2)) else {
            stream = &[];
            return Some(Err("damaged record".into()));
        };
        let Some(data) = stream.get(4..4 + len as usize) else {
            stream = &[];
            return Some(Err("damaged record length".into()));
        };
        stream = &stream[4 + len as usize..];
        while stream.len() > 4 && rdx_u16(stream, 0) == Some(0x003C) {
            let len = rdx_u16(stream, 2).unwrap_or(0) as usize;
            if stream.len() < len + 4 {
                stream = &[];
                return Some(Err("damaged continue record".into()));
            }
            stream = &stream[len + 4..];
        }
        Some(Ok((typ, data)))
    })
}

/// Cells calamine allocates for one sheet substream: its declared
/// DIMENSIONS and the rectangle around its cell records.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_biff_sheet_extent(substream: &[u8]) -> Result<(usize, usize)> {
    let mut declared = 0usize;
    let mut used: Option<(u32, u32, u32, u32)> = None;
    let mut add = |row: u32, col: u32| {
        used = Some(match used {
            None => (row, col, row, col),
            Some((r0, c0, r1, c1)) => (r0.min(row), c0.min(col), r1.max(row), c1.max(col)),
        });
    };
    let mut formula = (0, 0);
    let mut seen_bof = false;
    let mut depth = 0usize;
    for record in rdx_biff_records(substream) {
        let (typ, data) = record?;
        if depth > 0 {
            match typ {
                0x0809 => depth += 1,
                0x000A => depth -= 1,
                _ => {}
            }
            continue;
        }
        let at = || Some((u32::from(rdx_u16(data, 0)?), u32::from(rdx_u16(data, 2)?)));
        match typ {
            0x0809 if seen_bof => depth += 1,
            0x0809 => seen_bof = true,
            0x0200 => declared = declared.saturating_add(rdx_biff_dimensions(data)?),
            // Number, Label, RString, BoolErr, Rk, LabelSst.
            0x0203 | 0x0204 | 0x00D6 | 0x0205 | 0x027E | 0x00FD => {
                if let Some((r, c)) = at() {
                    add(r, c);
                }
            }
            // Formula; a String record holds the value of the last formula.
            0x0006 => {
                if let Some((r, c)) = at() {
                    formula = (r, c);
                    add(r, c);
                }
            }
            0x0207 => add(formula.0, formula.1),
            // MulRk: one row, consecutive columns from the first.
            0x00BD if data.len() >= 6 => {
                if let Some((r, c)) = at() {
                    let n = (data.len() - 6).div_ceil(6) as u32;
                    add(r, c);
                    add(r, c + n.saturating_sub(1));
                }
            }
            0x000A => break,
            _ => {}
        }
    }
    let used = used.map_or(0, |(r0, c0, r1, c1)| {
        ((r1 - r0) as usize + 1).saturating_mul((c1 - c0) as usize + 1)
    });
    Ok((declared, used))
}

/// Cells calamine reserves for a DIMENSIONS record (its rule for BIFF8 and
/// older layouts); a last row or column before the first wraps around there,
/// so it counts as unbounded here.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_biff_dimensions(data: &[u8]) -> Result<usize> {
    let read = |a: usize, wide: bool| {
        if wide {
            rdx_u32(data, a)
        } else {
            rdx_u16(data, a).map(u32::from)
        }
    };
    let fields = match data.len() {
        10 => (
            read(0, false),
            read(2, false),
            read(4, false),
            read(6, false),
        ),
        14 | 16 => (
            read(0, true),
            read(4, true),
            read(8, false),
            read(10, false),
        ),
        _ => return Err("damaged sheet dimensions".into()),
    };
    let (Some(first_row), Some(rows_end), Some(mut first_col), Some(cols_end)) = fields else {
        return Err("damaged sheet dimensions".into());
    };
    if 0xFF < first_col || cols_end < first_col {
        first_col = 0;
    }
    if rows_end == 0 || cols_end == 0 {
        return Ok(1);
    }
    let (last_row, last_col) = (rows_end - 1, cols_end - 1);
    if last_row < first_row || last_col < first_col {
        return Ok(usize::MAX);
    }
    Ok(((last_row - first_row) as usize + 1).saturating_mul((last_col - first_col) as usize + 1))
}

/// The workbook stream of an OLE compound file, read the way calamine
/// reads it (same sector rules and stream names), but failing on sector
/// loops and on sectors outside the file instead of looping or growing.
#[cfg(not(target_arch = "wasm32"))]
fn rdx_cfb_workbook(bytes: &[u8]) -> Result<Vec<u8>> {
    const END: u32 = 0xFFFF_FFFE;
    const RESERVED: u32 = 0xFFFF_FFFA;
    const DIFSECT: u32 = 0xFFFF_FFFC;
    let bad = |what: &str| format!("damaged .xls file ({what})");
    if bytes.len() < 512 || !bytes.starts_with(CFB_SIGNATURE) {
        return Err(bad("not an OLE compound file"));
    }
    let header = |at| rdx_u32(bytes, at).unwrap_or(0);
    let version = rdx_u16(bytes, 26).unwrap_or(0);
    let size: usize = match rdx_u16(bytes, 30) {
        Some(0x0009) => 512,
        Some(0x000C) if bytes.len() >= 4096 => 4096,
        _ => return Err(bad("sector size")),
    };
    if rdx_u16(bytes, 32) != Some(6) {
        return Err(bad("mini sector size"));
    }
    // The header fills sector -1; a chain longer than the file loops.
    let count = bytes.len() / size;
    let sector = |id: u32| {
        (id as usize)
            .checked_add(1)
            .and_then(|k| k.checked_mul(size))
            .and_then(|start| bytes.get(start..start.checked_add(size)?))
            .ok_or_else(|| bad("sector outside the file"))
    };
    let words = |s: &[u8]| -> Vec<u32> {
        s.as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect()
    };
    let mut difat = words(&bytes[76..512]);
    let mut id = header(68);
    let mut steps = 0;
    while id < RESERVED {
        steps += 1;
        if steps > count {
            return Err(bad("sector allocation loop"));
        }
        difat.extend(words(sector(id)?));
        id = difat.pop().unwrap_or(END);
    }
    let fat_sectors: Vec<u32> = difat.into_iter().filter(|id| *id < DIFSECT).collect();
    if fat_sectors.len() > count {
        return Err(bad("sector allocation table"));
    }
    let per_sector = size / 4;
    let next = |id: u32| {
        let i = id as usize;
        fat_sectors
            .get(i / per_sector)
            .ok_or_else(|| bad("sector chain"))
            .and_then(|s| sector(*s))
            .and_then(|s| rdx_u32(s, (i % per_sector) * 4).ok_or_else(|| bad("sector chain")))
    };
    let chain = |mut id: u32, len: usize| -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut steps = 0;
        while id != END {
            steps += 1;
            if steps > count {
                return Err(bad("sector chain loop"));
            }
            let s = sector(id)?;
            out.extend_from_slice(&s[..s.len().min(len - out.len())]);
            if out.len() >= len {
                break;
            }
            id = next(id)?;
        }
        Ok(out)
    };
    let directory = chain(header(48), usize::MAX)?;
    let entries: Vec<(String, u32, usize)> = directory
        .as_chunks::<128>()
        .0
        .iter()
        .map(|entry| {
            let len = if size == 512 {
                rdx_u32(entry, 120).unwrap_or(0) as usize
            } else {
                rdx_u32(entry, 120).unwrap_or(0) as usize
                    | ((rdx_u32(entry, 124).unwrap_or(0) as usize) << 32)
            };
            (
                rdx_cfb_name(&entry[..64]),
                rdx_u32(entry, 116).unwrap_or(0),
                len,
            )
        })
        .collect();
    let Some(root) = entries.first() else {
        return Err(bad("empty directory"));
    };
    if version != 3 && root.1 == END {
        return Err(bad("empty directory"));
    }
    let mini_fat_len = header(64) as usize;
    let (mini_fat, mini_stream) = if mini_fat_len > 0 {
        let stream = chain(root.1, root.2)?;
        let fat = chain(header(60), mini_fat_len.saturating_mul(size))?;
        (words(&fat), stream)
    } else {
        (Vec::new(), Vec::new())
    };
    let Some(&(_, start, len)) = ["Workbook", "Book", "WORKBOOK", "BOOK"]
        .iter()
        .find_map(|name| entries.iter().find(|e| e.0 == *name))
    else {
        return Err(bad("no workbook stream"));
    };
    if len >= 4096 {
        return chain(start, len);
    }
    let mut out = Vec::new();
    let mut id = start;
    let mut steps = 0;
    while id != END {
        steps += 1;
        if steps > mini_stream.len() / 64 + 1 {
            return Err(bad("mini sector chain"));
        }
        let at = id as usize * 64;
        let s = mini_stream
            .get(at..at + 64)
            .ok_or_else(|| bad("mini sector outside the stream"))?;
        out.extend_from_slice(&s[..64.min(len - out.len())]);
        if out.len() >= len {
            break;
        }
        id = *mini_fat
            .get(id as usize)
            .ok_or_else(|| bad("mini sector chain"))?;
    }
    Ok(out)
}

/// A compound-file directory name decoded as calamine decodes it
/// (UTF-16LE unless a byte-order mark says otherwise, cut at the first NUL).
#[cfg(not(target_arch = "wasm32"))]
fn rdx_cfb_name(raw: &[u8]) -> String {
    let utf16 = |b: &[u8], big: bool| {
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|u| {
                if big {
                    u16::from_be_bytes(*u)
                } else {
                    u16::from_le_bytes(*u)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    let mut name = if let Some(rest) = raw.strip_prefix(b"\xEF\xBB\xBF") {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = raw.strip_prefix(b"\xFF\xFE") {
        utf16(rest, false)
    } else if let Some(rest) = raw.strip_prefix(b"\xFE\xFF") {
        utf16(rest, true)
    } else {
        utf16(raw, false)
    };
    if let Some(nul) = name.find('\0') {
        name.truncate(nul);
    }
    name
}

#[cfg(target_arch = "wasm32")]
pub(super) fn rdx_read_sheet(
    _path: &Path,
    _sheet: &SheetChoice,
    _range: Option<CellRange>,
    _max_cells: usize,
    _cancel: &AtomicBool,
) -> Result<Grid> {
    Err(super::browser::NO_FILES.into())
}

fn rdx_grid_matrix(
    grid: &Grid,
    rows: std::ops::Range<usize>,
    cols: std::ops::Range<usize>,
) -> Result<Matrix> {
    let (nr, nc) = (rows.len(), cols.len());
    if nr == 0 || nc == 0 {
        return Matrix::filled(0, 0, 0.);
    }
    let mut data = Vec::with_capacity(nr * nc);
    for c in cols {
        for r in rows.clone() {
            data.push(match grid.at(r, c) {
                SheetCell::Number(v) => *v,
                SheetCell::Bool(b) => f64::from(u8::from(*b)),
                _ => f64::NAN,
            });
        }
    }
    Matrix::from_column_vec(nr, nc, data)
}

fn rdx_is_numeric(cell: &SheetCell) -> bool {
    matches!(cell, SheetCell::Number(_) | SheetCell::Bool(_))
}

/// Bounding box of the cells matching `keep`, as row and column ranges.
fn rdx_bounds(
    grid: &Grid,
    keep: impl Fn(&SheetCell) -> bool,
) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0, usize::MAX, 0);
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if keep(grid.at(r, c)) {
                r0 = r0.min(r);
                r1 = r1.max(r);
                c0 = c0.min(c);
                c1 = c1.max(c);
            }
        }
    }
    (r0 != usize::MAX).then(|| (r0..r1 + 1, c0..c1 + 1))
}

fn rdx_sheet_cell(cell: &SheetCell, empty: &Arc<Array>, cancel: &AtomicBool) -> Result<Arc<Array>> {
    Ok(match cell {
        SheetCell::Number(v) => Arc::new(Matrix::scalar(*v).into()),
        SheetCell::Bool(b) => Arc::new(LogicalMatrix::scalar(*b).into()),
        SheetCell::Text(t) if !t.is_empty() => Arc::new(CharacterRow::from_text(t, cancel)?.into()),
        _ => empty.clone(),
    })
}

/// Row-major cells to a rows x cols cell array.
fn rdx_cell_array(
    rows: usize,
    cols: usize,
    row_major: Vec<Arc<Array>>,
    cancel: &AtomicBool,
) -> Result<Array> {
    let mut slots = Vec::with_capacity(row_major.len());
    for c in 0..cols {
        for r in 0..rows {
            slots.push(row_major[r * cols + c].clone());
        }
    }
    Ok(Array::Cell(CellArray::from_shared(
        rows, cols, slots, cancel,
    )?))
}

// ---------------------------------------------------------------------------
// Engine entry points

impl Engine {
    pub(super) fn rdx_call(
        &mut self,
        name: &str,
        args: &[Value],
        _out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !rdx_handles(name) {
            return Ok(None);
        }
        let mut values = self.rdx_run(name, args, 1)?;
        if values.is_empty() {
            return Err("Too many output arguments.".into());
        }
        Ok(Some(values.remove(0)))
    }

    pub(super) fn rdx_eval_outputs(
        &mut self,
        expression: &Expr,
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let (name, expressions) = match expression {
            Expr::Call(name, args) => (name.as_str(), args.as_slice()),
            Expr::Name(name) => (name.as_str(), &[][..]),
            _ => return Ok(None),
        };
        if !rdx_handles(name) || self.contains_array(name) {
            return Ok(None);
        }
        let args = super::string_class::arguments(self, name, expressions, out)?;
        if let Some(function) = self.resolve_function(name)? {
            return self
                .invoke_function(function, args, count, None, out)
                .map(Some);
        }
        let mut values = self.rdx_run(name, &args, count)?;
        if values.len() < count {
            return Err("Too many output arguments.".into());
        }
        values.truncate(count.max(1).min(values.len()));
        Ok(Some(values))
    }

    /// Statement form: writecell writes; the others set and show `ans`.
    pub(super) fn rdx_statement(
        &mut self,
        name: &str,
        expressions: &[Expr],
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        if !rdx_handles(name) || self.resolve_function(name)?.is_some() {
            return Ok(false);
        }
        let args = super::string_class::arguments(self, name, expressions, out)?;
        let mut values = self.rdx_run(name, &args, 0)?;
        if !values.is_empty() {
            let value = values.remove(0).array(&self.cancel)?;
            self.assign_workspace_array("ans", value, quiet, out)?;
        }
        Ok(true)
    }

    fn rdx_run(&mut self, name: &str, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = match name {
            "writecell" | "writetable" | "h5create" | "h5write" => 0,
            "xlsread" => 3,
            "xlswrite" => 1,
            _ => 1,
        };
        if nargout > max {
            return Err("Too many output arguments.".into());
        }
        match name {
            "jsondecode" => self.rdx_jsondecode(args).map(|v| vec![v]),
            "jsonencode" => self.rdx_jsonencode(args).map(|v| vec![v]),
            "readcell" => self.rdx_readcell(args).map(|v| vec![v]),
            "writecell" => self.rdx_writecell(args).map(|()| Vec::new()),
            "xlsread" => self.rdx_xlsread(args, nargout),
            #[cfg(not(target_arch = "wasm32"))]
            "xlswrite" => self.rdx_xlswrite(args).map(|()| {
                (nargout > 0)
                    .then(|| Value::Logical(LogicalMatrix::scalar(true)))
                    .into_iter()
                    .collect()
            }),
            #[cfg(target_arch = "wasm32")]
            "xlswrite" => Err(super::browser::NO_FILES.into()),
            "webread" => self.web_read(args).map(|v| vec![v]),
            "websave" => self.web_save(args, nargout),
            "weboptions" => self.web_options(args).map(|v| vec![v]),
            "h5read" => self.h5_read(args).map(|v| vec![v]),
            "h5create" => self.h5_create(args).map(|()| Vec::new()),
            "h5write" => self.h5_write(args).map(|()| Vec::new()),
            "ncread" => self.nc_read(args).map(|v| vec![v]),
            "readtable" => self.tbl_readtable(args).map(|v| vec![v]),
            "writetable" => self.tbl_writetable(args).map(|()| Vec::new()),
            _ => Err(format!("Unsupported function {name}")),
        }
    }

    /// writetable to a spreadsheet: its cell grid written as xlswrite
    /// writes (file, grid, sheet, range).
    pub(super) fn rdx_write_sheet(&mut self, args: &[Value]) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.rdx_xlswrite(args);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = args;
            Err(super::browser::NO_FILES.into())
        }
    }

    fn rdx_jsondecode(&self, args: &[Value]) -> Result<Value> {
        let Some((text, rest)) = args.split_first() else {
            return Err("Not enough input arguments.".into());
        };
        let text = rdx_text(text)
            .ok_or("jsondecode: the input must be a character vector or string scalar")?;
        if text.len() > MAX_JSON_BYTES {
            return Err(format!(
                "jsondecode: JSON text exceeds {} MiB",
                MAX_JSON_BYTES >> 20
            ));
        }
        let mut make_valid = true;
        for (option, value) in rdx_pairs("jsondecode", rest)? {
            match option.as_str() {
                "makevalidname" => make_valid = rdx_flag("jsondecode", "makeValidName", value)?,
                _ => return Err(format!("jsondecode: unknown option '{option}'")),
            }
        }
        let json = rdx_parse_json(&text, &self.cancel)?;
        let decoder = Decoder {
            make_valid,
            cancel: &self.cancel,
        };
        Ok(decoder.decode(&json)?.into())
    }

    fn rdx_jsonencode(&self, args: &[Value]) -> Result<Value> {
        let Some((value, rest)) = args.split_first() else {
            return Err("Not enough input arguments.".into());
        };
        let mut encoder = Encoder {
            engine: self,
            pretty: false,
            convert: true,
            out: String::new(),
            steps: 0,
        };
        for (option, flag) in rdx_pairs("jsonencode", rest)? {
            match option.as_str() {
                "prettyprint" => encoder.pretty = rdx_flag("jsonencode", "PrettyPrint", flag)?,
                "convertinfandnan" => {
                    encoder.convert = rdx_flag("jsonencode", "ConvertInfAndNaN", flag)?;
                }
                _ => return Err(format!("jsonencode: unknown option '{option}'")),
            }
        }
        encoder.value(value)?;
        let units = encoder.out.encode_utf16().count();
        if units > crate::script_containers::MAX_CHARACTER_UNITS {
            return Err(format!(
                "jsonencode: the JSON text has {units} characters; character vectors hold at most {}",
                crate::script_containers::MAX_CHARACTER_UNITS
            ));
        }
        Ok(Value::Text(encoder.out))
    }

    fn rdx_filename(function: &str, value: Option<&Value>) -> Result<String> {
        value
            .and_then(rdx_text)
            .ok_or_else(|| format!("{function}: the file name must be text"))
    }

    pub(super) fn rdx_readcell(&self, args: &[Value]) -> Result<Value> {
        if args
            .get(1)
            .is_some_and(|v| super::objects::import_value(self, v).is_some())
        {
            return super::import_options::readcell(self, args);
        }
        let filename = Self::rdx_filename("readcell", args.first())?;
        let path = self.files_read_path(&filename, "")?;
        let mut delimiter = None;
        let mut headers = None;
        let mut range = None;
        let mut sheet = None;
        let mut spreadsheet = rdx_is_sheet_extension(&filename);
        let mut text_type = false;
        for (option, value) in rdx_pairs("readcell", &args[1..])? {
            match option.as_str() {
                "delimiter" => delimiter = Some(rdx_delimiter("readcell", value, false)?),
                "numheaderlines" => {
                    headers = Some(rdx_positive_integer("readcell", "NumHeaderLines", value)?);
                }
                "range" => {
                    let text = rdx_text(value).ok_or("readcell: Range must be text")?;
                    range = Some(rdx_range("readcell", &text)?);
                }
                "sheet" => sheet = Some(rdx_sheet_choice("readcell", value)?),
                "filetype" => match rdx_text(value).map(|t| t.to_ascii_lowercase()).as_deref() {
                    Some("text" | "delimitedtext") => {
                        spreadsheet = false;
                        text_type = true;
                    }
                    Some("spreadsheet") => spreadsheet = true,
                    _ => return Err("readcell: FileType must be 'text' or 'spreadsheet'".into()),
                },
                _ => {
                    return Err(format!(
                        "readcell: unsupported option '{option}'; supported are Delimiter, NumHeaderLines, Range, Sheet and FileType"
                    ));
                }
            }
        }
        if headers.is_some() && range.is_some() {
            return Err("readcell: give either NumHeaderLines or Range, not both".into());
        }
        if spreadsheet {
            if delimiter.is_some() {
                return Err("readcell: Delimiter applies to text files only".into());
            }
            let grid = rdx_read_sheet(
                &path,
                &sheet.unwrap_or(SheetChoice::First),
                range,
                crate::script_containers::MAX_CELL_SLOTS,
                &self.cancel,
            )?;
            // NumHeaderLines counts sheet rows from row 1, not rows of the
            // used area (which may start lower).
            let skip = headers
                .unwrap_or(0)
                .saturating_sub(grid.first_row)
                .min(grid.rows);
            let missing = Arc::new(rdx_missing()?);
            let rows = grid.rows - skip;
            let mut cells = Vec::with_capacity(rows * grid.cols);
            for r in skip..grid.rows {
                for c in 0..grid.cols {
                    cells.push(rdx_sheet_cell(grid.at(r, c), &missing, &self.cancel)?);
                }
            }
            let cols = if rows == 0 { 0 } else { grid.cols };
            return Ok(rdx_cell_array(rows, cols, cells, &self.cancel)?.into());
        }
        if sheet.is_some() {
            return Err("readcell: Sheet applies to spreadsheet files only".into());
        }
        if !text_type
            && !rdx_extension(&filename).is_some_and(|e| TEXT_EXTENSIONS.contains(&e.as_str()))
        {
            return Err(
                "readcell: use a .csv, .txt, .dat, .xlsx or .xls file, or give 'FileType'".into(),
            );
        }
        let size = std::fs::metadata(&path)
            .map_err(|e| format!("readcell: unable to open {filename}: {e}"))?
            .len();
        if size > MAX_TEXT_BYTES {
            return Err(format!(
                "readcell: text file exceeds {} MiB",
                MAX_TEXT_BYTES >> 20
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("readcell: unable to read {filename}: {e}"))?;
        self.check_cancel()?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        let delimiters = delimiter.unwrap_or_else(|| vec![rdx_detect_delimiter(text)]);
        let records = rdx_records(text, &delimiters, &self.cancel)?;
        let (first, last, col, last_col) = match range {
            Some(r) => (
                r.row,
                r.last_row.map(|l| l + 1),
                r.col,
                r.last_col.map(|l| l + 1),
            ),
            None => (headers.unwrap_or(0), None, 0, None),
        };
        let kept: Vec<Option<&Vec<Field>>> = if range.is_some() {
            // A Range is a rectangle of the file: blank lines inside it are
            // rows of missing cells. An open end stops at the last line with
            // fields.
            let end = match last {
                Some(last) => last.min(records.len()),
                None => records
                    .iter()
                    .rposition(Option::is_some)
                    .map_or(0, |p| p + 1),
            };
            records
                .get(first.min(end)..end)
                .unwrap_or(&[])
                .iter()
                .map(Option::as_ref)
                .collect()
        } else {
            // Without a Range, blank lines are skipped.
            records
                .get(first.min(records.len())..)
                .unwrap_or(&[])
                .iter()
                .flatten()
                .map(Some)
                .collect()
        };
        // A start cell without an end extends to the widest kept line.
        let width = kept.iter().flatten().map(|f| f.len()).max().unwrap_or(0);
        let col_end = last_col.unwrap_or(width).max(col);
        let cols = col_end - col;
        let rows = kept.len();
        if rows == 0 || cols == 0 {
            return Ok(Value::Container(Array::Cell(CellArray::empty(
                0,
                0,
                &self.cancel,
            )?)));
        }
        if rows.saturating_mul(cols) > crate::script_containers::MAX_CELL_SLOTS {
            return Err(format!(
                "readcell: the result would have more than {} cells",
                crate::script_containers::MAX_CELL_SLOTS
            ));
        }
        let missing = Arc::new(rdx_missing()?);
        let mut cells = Vec::with_capacity(rows * cols);
        for fields in kept {
            self.check_cancel()?;
            for c in col..col_end {
                cells.push(match fields.and_then(|f| f.get(c)) {
                    Some((text, quoted)) => {
                        Arc::new(rdx_text_cell(text, *quoted, &delimiters, &self.cancel)?)
                    }
                    None => missing.clone(),
                });
            }
        }
        Ok(rdx_cell_array(rows, cols, cells, &self.cancel)?.into())
    }

    pub(super) fn rdx_writecell(&self, args: &[Value]) -> Result<()> {
        let [cell, rest @ ..] = args else {
            return Err("Not enough input arguments.".into());
        };
        if rest.is_empty() {
            return Err("writecell: give a file name, as in writecell(C, 'data.csv')".into());
        }
        let Value::Container(Array::Cell(cell)) = cell else {
            return Err("writecell: the first input must be a cell array".into());
        };
        let filename = rdx_text(&rest[0]).ok_or("writecell: the file name must be text")?;
        let mut delimiter = ',';
        let mut quote_all = false;
        let mut text_type = false;
        for (option, value) in rdx_pairs("writecell", &rest[1..])? {
            match option.as_str() {
                "delimiter" => delimiter = rdx_delimiter("writecell", value, true)?[0],
                "quotestrings" => quote_all = rdx_flag("writecell", "QuoteStrings", value)?,
                "filetype" => match rdx_text(value).map(|t| t.to_ascii_lowercase()).as_deref() {
                    Some("text") => text_type = true,
                    Some("spreadsheet") => return Err(SHEET_UNSUPPORTED_WRITE.into()),
                    _ => return Err("writecell: FileType must be 'text'".into()),
                },
                "writemode" => match rdx_text(value).map(|t| t.to_ascii_lowercase()).as_deref() {
                    Some("overwrite") => {}
                    _ => {
                        return Err("writecell: only WriteMode 'overwrite' is supported".into());
                    }
                },
                _ => {
                    return Err(format!(
                        "writecell: unsupported option '{option}'; supported are Delimiter, QuoteStrings, FileType and WriteMode"
                    ));
                }
            }
        }
        match rdx_extension(&filename) {
            Some(e) if SHEET_EXTENSIONS.contains(&e.as_str()) => {
                return Err(SHEET_UNSUPPORTED_WRITE.into());
            }
            Some(e) if !text_type && !TEXT_EXTENSIONS.contains(&e.as_str()) => {
                return Err(
                    "writecell: use a .csv, .txt or .dat file name, or give 'FileType', 'text'"
                        .into(),
                );
            }
            _ => {}
        }
        let path = self.file_path(&filename, ".txt")?;
        let (rows, cols) = (cell.rows(), cell.cols());
        let mut text = String::new();
        for r in 0..rows {
            self.check_cancel()?;
            for c in 0..cols {
                if c > 0 {
                    text.push(delimiter);
                }
                let item = cell.get_linear(r + c * rows)?;
                self.rdx_write_item(item, r, c, delimiter, quote_all, &mut text)?;
            }
            text.push('\n');
            if text.len() > crate::files::MAX_CSV_BYTES {
                return Err("writecell: the text would exceed the file size limit".into());
            }
        }
        crate::files::atomic_write_with_cancel(&path, text.as_bytes(), &self.cancel).map_err(
            |e| match e.strip_prefix("Could not save ") {
                // The helper names the destination path and the OS reason.
                Some(rest) => format!(
                    "writecell: unable to write {filename}: {}",
                    rest.rsplit_once(": ")
                        .map_or(rest, |(_, reason)| reason)
                        .split(" at path")
                        .next()
                        .unwrap_or("")
                ),
                None => format!(
                    "writecell: unable to write {filename}: {}",
                    e.split(" at path").next().unwrap_or("")
                ),
            },
        )?;
        Ok(())
    }

    fn rdx_write_item(
        &self,
        item: &Array,
        r: usize,
        c: usize,
        delimiter: char,
        quote_all: bool,
        text: &mut String,
    ) -> Result<()> {
        let number = |x: f64| {
            if x.is_nan() {
                "NaN".to_owned()
            } else if x.is_infinite() {
                if x > 0. { "Inf".into() } else { "-Inf".into() }
            } else {
                rdx_number_text(x)
            }
        };
        let words = match item {
            Array::Numeric(m) if m.len() == 1 => Some((number(m.data()[0]), false)),
            Array::Logical(m) if m.len() == 1 => Some((m.data()[0].to_string(), false)),
            Array::Typed(t) if t.len() == 1 => Some((
                if t.is_integer() {
                    t.int(0).to_string()
                } else {
                    number(t.real(0))
                },
                false,
            )),
            Array::Numeric(_) | Array::Logical(_) | Array::Cell(_) | Array::Str(_)
                if item.is_empty() =>
            {
                Some((String::new(), false))
            }
            Array::Character(ch) if ch.rows() <= 1 && !ch.is_nd() => {
                Some((String::from_utf16_lossy(ch.units()), true))
            }
            Array::Str(s) if s.len() == 1 => Some((
                s.data()[0].as_deref().unwrap_or("").to_owned(),
                s.data()[0].is_some(),
            )),
            _ => None,
        };
        let Some((words, is_text)) = words else {
            return Err(format!(
                "writecell: cell ({},{}) must hold a scalar, text or an empty value",
                r + 1,
                c + 1
            ));
        };
        let needs_quotes = is_text
            && (quote_all || words.contains(delimiter) || words.contains(['"', '\n', '\r']));
        if needs_quotes {
            text.push('"');
            text.push_str(&words.replace('"', "\"\""));
            text.push('"');
        } else {
            text.push_str(&words);
        }
        Ok(())
    }

    fn rdx_xlsread(&self, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
        let filename = Self::rdx_filename("xlsread", args.first())?;
        let path = self.file_path(&filename, "")?;
        if args.len() > 4 {
            return Err("Too many input arguments.".into());
        }
        if !rdx_is_sheet_extension(&filename) {
            return Err("xlsread: use an .xlsx, .xlsm, .xlsb or .xls file".into());
        }
        let mut sheet = SheetChoice::First;
        let mut range = None;
        let mut rest = &args[1..];
        if let Some(value) = rest.first() {
            match rdx_text(value) {
                // xlsread(file, range): text with ':' is a range, not a sheet.
                Some(text) if rest.len() == 1 && text.contains(':') => {
                    range = Some(rdx_range("xlsread", &text)?);
                }
                _ => sheet = rdx_sheet_choice("xlsread", value)?,
            }
            rest = &rest[1..];
        }
        if let Some(value) = rest.first() {
            let text = rdx_text(value).ok_or("xlsread: the range must be text such as 'A1:C10'")?;
            if !text.is_empty() {
                range = Some(rdx_range("xlsread", &text)?);
            }
            rest = &rest[1..];
        }
        if let Some(value) = rest.first()
            && !rdx_text(value).is_some_and(|t| t.eq_ignore_ascii_case("basic"))
        {
            return Err("xlsread: the fourth input can only be 'basic'".into());
        }
        let grid = rdx_read_sheet(&path, &sheet, range, MAX_GRID_CELLS, &self.cancel)?;
        let num = match rdx_bounds(&grid, rdx_is_numeric) {
            Some((rows, cols)) => rdx_grid_matrix(&grid, rows, cols)?,
            None => Matrix::filled(0, 0, 0.)?,
        };
        let empty_text: Arc<Array> =
            Arc::new(CharacterRow::from_units(Vec::new(), &self.cancel)?.into());
        // Only the requested outputs are built.
        let txt = if nargout < 2 {
            Array::Cell(CellArray::empty(0, 0, &self.cancel)?)
        } else {
            match rdx_bounds(&grid, |c| matches!(c, SheetCell::Text(t) if !t.is_empty())) {
                Some((rows, cols)) => {
                    let (nr, nc) = (rows.len(), cols.len());
                    if nr.saturating_mul(nc) > crate::script_containers::MAX_CELL_SLOTS {
                        return Err(format!(
                            "xlsread: the text output would have more than {} cells; read a smaller Range",
                            crate::script_containers::MAX_CELL_SLOTS
                        ));
                    }
                    let mut cells = Vec::with_capacity(nr * nc);
                    for r in rows {
                        for c in cols.clone() {
                            cells.push(match grid.at(r, c) {
                                SheetCell::Text(t) if !t.is_empty() => {
                                    Arc::new(CharacterRow::from_text(t, &self.cancel)?.into())
                                }
                                _ => empty_text.clone(),
                            });
                        }
                    }
                    rdx_cell_array(nr, nc, cells, &self.cancel)?
                }
                None => Array::Cell(CellArray::empty(0, 0, &self.cancel)?),
            }
        };
        let raw = if nargout < 3 || grid.rows == 0 || grid.cols == 0 {
            Array::Cell(CellArray::empty(0, 0, &self.cancel)?)
        } else {
            if grid.rows * grid.cols > crate::script_containers::MAX_CELL_SLOTS {
                return Err(format!(
                    "xlsread: the raw output would have more than {} cells; read a smaller Range",
                    crate::script_containers::MAX_CELL_SLOTS
                ));
            }
            let nan: Arc<Array> = Arc::new(Matrix::scalar(f64::NAN).into());
            let mut cells = Vec::with_capacity(grid.cells.len());
            for cell in &grid.cells {
                cells.push(rdx_sheet_cell(cell, &nan, &self.cancel)?);
            }
            rdx_cell_array(grid.rows, grid.cols, cells, &self.cancel)?
        };
        let mut outputs = vec![Value::Matrix(num), Value::from(txt), Value::from(raw)];
        outputs.truncate(nargout.max(1));
        Ok(outputs)
    }

    /// readmatrix on a spreadsheet: numbers and logicals as double, every
    /// other cell NaN. Without a Range, leading rows holding no number
    /// (header rows) are skipped.
    pub(super) fn rdx_read_matrix_sheet(&self, filename: &str, rest: &[Value]) -> Result<Value> {
        let mut sheet = SheetChoice::First;
        let mut range = None;
        for (option, value) in rdx_pairs("readmatrix", rest)? {
            match option.as_str() {
                "sheet" => sheet = rdx_sheet_choice("readmatrix", value)?,
                "range" => {
                    let text = rdx_text(value).ok_or("readmatrix: Range must be text")?;
                    range = Some(rdx_range("readmatrix", &text)?);
                }
                "filetype" => {}
                _ => {
                    return Err(format!(
                        "readmatrix: unsupported spreadsheet option '{option}'; supported are Sheet, Range and FileType"
                    ));
                }
            }
        }
        let path = self.files_read_path(filename, "")?;
        let grid = rdx_read_sheet(&path, &sheet, range, MAX_GRID_CELLS, &self.cancel)?;
        let first = if range.is_some() {
            0
        } else {
            (0..grid.rows)
                .find(|&r| (0..grid.cols).any(|c| rdx_is_numeric(grid.at(r, c))))
                .unwrap_or(grid.rows)
        };
        Ok(Value::Matrix(rdx_grid_matrix(
            &grid,
            first..grid.rows,
            0..grid.cols,
        )?))
    }
}

/// True when readmatrix should read `filename` as a spreadsheet: a
/// spreadsheet extension or 'FileType','spreadsheet'.
pub(super) fn rdx_sheet_request(filename: &str, rest: &[Value]) -> bool {
    let explicit = rest.as_chunks::<2>().0.iter().find_map(|pair| {
        rdx_text(&pair[0])
            .filter(|n| n.eq_ignore_ascii_case("filetype"))
            .and_then(|_| rdx_text(&pair[1]))
    });
    match explicit {
        Some(kind) => kind.eq_ignore_ascii_case("spreadsheet"),
        None => rdx_is_sheet_extension(filename),
    }
}

/// The error writematrix gives for spreadsheet targets.
pub(super) fn rdx_sheet_write_error() -> String {
    SHEET_UNSUPPORTED_WRITE.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_valid_name_follows_published_rules() {
        assert_eq!(rdx_make_valid_name("first name"), "firstName");
        assert_eq!(rdx_make_valid_name("1abc"), "x1abc");
        assert_eq!(rdx_make_valid_name(""), "x");
        assert_eq!(rdx_make_valid_name("a-b"), "a_b");
        assert_eq!(rdx_make_valid_name("for"), "xFor");
        assert_eq!(rdx_make_valid_name("_x"), "x_x");
        assert_eq!(rdx_make_valid_name(&"a".repeat(80)).len(), 63);
    }

    #[test]
    fn unique_names_append_suffixes() {
        let names = rdx_field_names(["a", "a", "a_1", "a"].into_iter(), true).unwrap();
        assert_eq!(names, ["a", "a_2", "a_1", "a_3"]);
    }

    #[test]
    fn ranges_parse() {
        assert_eq!(
            rdx_range("t", "b2:a1").unwrap(),
            CellRange {
                row: 0,
                col: 0,
                last_row: Some(1),
                last_col: Some(1)
            }
        );
        assert_eq!(rdx_range("t", "AA10").unwrap().col, 26);
        assert!(rdx_range("t", "A0").is_err());
        assert!(rdx_range("t", "Data").is_err());
    }

    #[test]
    fn numbers_format_compactly() {
        assert_eq!(rdx_number_text(3.), "3");
        assert_eq!(rdx_number_text(0.1), "0.1");
        assert_eq!(rdx_number_text(-0.), "0");
        assert_eq!(rdx_number_text(1e300), "1e300");
        assert_eq!(rdx_number_text(1.5e-7), "1.5e-7");
        assert_eq!(rdx_number_text(std::f64::consts::PI), "3.141592653589793");
    }

    #[test]
    fn number_fields() {
        assert_eq!(rdx_parse_number("1e3"), Some(1000.));
        assert_eq!(rdx_parse_number("-Inf"), Some(f64::NEG_INFINITY));
        assert!(rdx_parse_number("NaN").unwrap().is_nan());
        assert_eq!(rdx_parse_number(".5"), Some(0.5));
        assert_eq!(rdx_parse_number("abc"), None);
        assert_eq!(rdx_parse_number("infinity"), None);
        assert_eq!(rdx_parse_number("1,5"), None);
        assert_eq!(rdx_parse_number("e5"), None);
    }
}
