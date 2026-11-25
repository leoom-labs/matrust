//! Import options: detectImportOptions, setvartype, getvaropts, setvaropts,
//! preview, readtimetable and the options forms of readtable, readmatrix and
//! readcell. Behaviour follows MathWorks' published function pages
//! (behaviour only); expected values in the tests come from pandas.
//!
//! An options object is an engine object (objects.rs, `ObjState::Import`)
//! with value semantics: every function returns a changed copy, and
//! `opts.Name = value` rebinds the variable to a changed copy, so other
//! copies keep their values. `opts.VariableOptions` is a 1-by-N array of
//! per-variable option objects (a struct array of object handles).
//!
//! Everything is bounded: files are size-checked before they are parsed,
//! the cell and row counts of a result are limited, and loops poll
//! cancellation.
mod detect;
mod read;

use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, NumericClass, Result, StringArray,
    TypedArray, syntax::PathStep,
};
use std::sync::Arc;

pub(in crate::engine) use read::{readcell, readmatrix, readtable, readtable_takes_over};

/// Builtins of this module (readtable, readmatrix and readcell keep their
/// own dispatch and come here only for an options object or new options).
pub(in crate::engine) const NAMES: &[&str] = &[
    "detectImportOptions",
    "setvartype",
    "getvaropts",
    "setvaropts",
    "preview",
    "readtimetable",
];
pub(in crate::engine) const TEXT_CLASS: &str = "matlab.io.text.DelimitedTextImportOptions";
pub(in crate::engine) const SHEET_CLASS: &str = "matlab.io.spreadsheet.SpreadsheetImportOptions";
/// Every class of this module, in the order objects.rs numbers their
/// display stand-ins.
pub(in crate::engine) const CLASSES: [&str; 8] = [
    TEXT_CLASS,
    SHEET_CLASS,
    "matlab.io.NumericVariableImportOptions",
    "matlab.io.TextVariableImportOptions",
    "matlab.io.DatetimeVariableImportOptions",
    "matlab.io.DurationVariableImportOptions",
    "matlab.io.CategoricalVariableImportOptions",
    "matlab.io.LogicalVariableImportOptions",
];
const TYPE_NAMES: &str = "double, single, int8, uint8, int16, uint16, int32, uint32, int64, uint64, logical, char, string, datetime, duration or categorical";

/// Class of one variable's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum VarType {
    Double,
    Single,
    Int(NumericClass),
    Logical,
    Char,
    Str,
    Datetime,
    Duration,
    Categorical,
}

impl VarType {
    pub(super) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "double" => Self::Double,
            "single" => Self::Single,
            "logical" => Self::Logical,
            "char" => Self::Char,
            "string" => Self::Str,
            "datetime" => Self::Datetime,
            "duration" => Self::Duration,
            "categorical" => Self::Categorical,
            other => Self::Int(NumericClass::parse(other).filter(|c| c.is_integer())?),
        })
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Double => "double",
            Self::Single => "single",
            Self::Int(c) => c.name(),
            Self::Logical => "logical",
            Self::Char => "char",
            Self::Str => "string",
            Self::Datetime => "datetime",
            Self::Duration => "duration",
            Self::Categorical => "categorical",
        }
    }
    pub(super) fn numeric(self) -> bool {
        matches!(self, Self::Double | Self::Single | Self::Int(_))
    }
    pub(super) fn text(self) -> bool {
        matches!(self, Self::Char | Self::Str)
    }
    /// Index of the variable-options class in [`CLASSES`].
    fn class_index(self) -> usize {
        match self {
            Self::Double | Self::Single | Self::Int(_) => 2,
            Self::Char | Self::Str => 3,
            Self::Datetime => 4,
            Self::Duration => 5,
            Self::Categorical => 6,
            Self::Logical => 7,
        }
    }
    /// Properties of the variable-options class beyond the common ones.
    fn own_properties(self) -> &'static [&'static str] {
        match self.class_index() {
            2 => &[
                "ExponentCharacter",
                "DecimalSeparator",
                "ThousandsSeparator",
                "TrimNonNumeric",
                "NumberSystem",
            ],
            3 => &["WhitespaceRule"],
            4 => &[
                "DatetimeFormat",
                "DatetimeLocale",
                "InputFormat",
                "TimeZone",
            ],
            5 => &[
                "InputFormat",
                "DurationFormat",
                "DecimalSeparator",
                "FieldSeparator",
            ],
            6 => &["Categories", "Protected", "Ordinal"],
            _ => &["TrueSymbols", "FalseSymbols", "CaseSensitive"],
        }
    }
    fn group_title(self) -> &'static str {
        match self.class_index() {
            2 => "Numeric Options",
            3 => "String Options",
            4 => "Datetime Options",
            5 => "Duration Options",
            6 => "Categorical Options",
            _ => "Logical Options",
        }
    }
}

const VAR_COMMON: [&str; 8] = [
    "Name",
    "Type",
    "FillValue",
    "TreatAsMissing",
    "QuoteRule",
    "Prefixes",
    "Suffixes",
    "EmptyFieldRule",
];

/// Options of one variable (the VariableImportOptions classes).
#[derive(Clone, Debug)]
pub(in crate::engine) struct VarOpts {
    pub(super) name: String,
    pub(super) kind: VarType,
    /// None: the type's default (NaN, '', <missing>, NaT, false).
    pub(super) fill: Option<Arc<Array>>,
    pub(super) treat_as_missing: Vec<String>,
    pub(super) quote_rule: String,
    pub(super) prefixes: Vec<String>,
    pub(super) suffixes: Vec<String>,
    pub(super) empty_field_rule: String,
    pub(super) whitespace_rule: String,
    pub(super) exponent: String,
    pub(super) decimal: String,
    pub(super) thousands: String,
    pub(super) trim_non_numeric: bool,
    pub(super) input_format: String,
    pub(super) datetime_format: String,
    pub(super) locale: String,
    pub(super) time_zone: String,
    pub(super) duration_format: String,
    pub(super) field_separator: String,
    pub(super) categories: Vec<String>,
    pub(super) protected: bool,
    pub(super) ordinal: bool,
    pub(super) true_symbols: Vec<String>,
    pub(super) false_symbols: Vec<String>,
    pub(super) case_sensitive: bool,
}

impl VarOpts {
    pub(super) fn new(name: String, kind: VarType) -> Self {
        Self {
            name,
            kind,
            fill: None,
            treat_as_missing: Vec::new(),
            quote_rule: "remove".into(),
            prefixes: Vec::new(),
            suffixes: Vec::new(),
            empty_field_rule: "missing".into(),
            whitespace_rule: "trim".into(),
            exponent: "eEdD".into(),
            decimal: ".".into(),
            thousands: String::new(),
            trim_non_numeric: false,
            input_format: String::new(),
            datetime_format: "preserveinput".into(),
            locale: "en_US".into(),
            time_zone: String::new(),
            duration_format: "default".into(),
            field_separator: ":".into(),
            categories: Vec::new(),
            protected: false,
            ordinal: false,
            true_symbols: vec!["true".into(), "1".into()],
            false_symbols: vec!["false".into(), "0".into()],
            case_sensitive: false,
        }
    }
    /// A changed type: the common options stay, the others take the new
    /// class's defaults (the decimal separators carry over between numbers
    /// and durations).
    pub(super) fn retyped(&self, kind: VarType) -> Self {
        if kind == self.kind {
            return self.clone();
        }
        let mut v = Self::new(self.name.clone(), kind);
        v.treat_as_missing = self.treat_as_missing.clone();
        v.quote_rule = self.quote_rule.clone();
        v.prefixes = self.prefixes.clone();
        v.suffixes = self.suffixes.clone();
        v.empty_field_rule = self.empty_field_rule.clone();
        if self.kind.numeric() && kind.numeric() {
            v.exponent = self.exponent.clone();
            v.decimal = self.decimal.clone();
            v.thousands = self.thousands.clone();
            v.trim_non_numeric = self.trim_non_numeric;
        }
        if self.kind.text() && kind.text() {
            v.whitespace_rule = self.whitespace_rule.clone();
        }
        v
    }
    fn bytes(&self) -> usize {
        let list = |v: &[String]| v.iter().map(|s| s.len() + 24).sum::<usize>();
        256 + self.name.len()
            + self.fill.as_ref().map_or(0, |a| a.retained_bytes())
            + list(&self.treat_as_missing)
            + list(&self.prefixes)
            + list(&self.suffixes)
            + list(&self.categories)
            + self.input_format.len()
            + self.datetime_format.len()
    }
}

/// Which worksheet a spreadsheet options object reads.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) enum SheetSel {
    /// '' reads the first sheet.
    Name(String),
    Index(usize),
}

/// The DelimitedTextImportOptions and SpreadsheetImportOptions classes.
#[derive(Clone, Debug)]
pub(in crate::engine) struct ImportOpts {
    pub(super) sheet: bool,
    pub(super) vars: Vec<VarOpts>,
    /// Indices into `vars`, in output order.
    pub(super) selected: Vec<usize>,
    pub(super) preserve_names: bool,
    pub(super) missing_rule: String,
    pub(super) import_error_rule: String,
    // Delimited text
    pub(super) delimiter: Vec<String>,
    /// Escaped text, as displayed: '\b\t '.
    pub(super) whitespace: String,
    pub(super) line_ending: Vec<String>,
    pub(super) comment_style: Vec<String>,
    pub(super) consecutive_rule: String,
    pub(super) leading_rule: String,
    pub(super) trailing_rule: String,
    pub(super) empty_line_rule: String,
    pub(super) encoding: String,
    pub(super) extra_columns_rule: String,
    /// Rows [first last] of 1-based lines; `last` may be Inf.
    pub(super) data_lines: Vec<[f64; 2]>,
    pub(super) names_line: usize,
    pub(super) row_names_column: usize,
    pub(super) units_line: usize,
    pub(super) descriptions_line: usize,
    // Spreadsheet
    pub(super) sheet_name: SheetSel,
    pub(super) data_range: String,
    pub(super) names_range: String,
    pub(super) row_names_range: String,
    pub(super) units_range: String,
    pub(super) descriptions_range: String,
}

impl ImportOpts {
    pub(super) fn new(sheet: bool, vars: Vec<VarOpts>) -> Self {
        let selected = (0..vars.len()).collect();
        Self {
            sheet,
            vars,
            selected,
            preserve_names: false,
            missing_rule: "fill".into(),
            import_error_rule: "fill".into(),
            delimiter: vec![",".into()],
            whitespace: "\\b\\t ".into(),
            line_ending: vec!["\\n".into(), "\\r".into(), "\\r\\n".into()],
            comment_style: Vec::new(),
            consecutive_rule: "split".into(),
            leading_rule: "keep".into(),
            trailing_rule: "ignore".into(),
            empty_line_rule: "skip".into(),
            encoding: "UTF-8".into(),
            extra_columns_rule: "addvars".into(),
            data_lines: vec![[1., f64::INFINITY]],
            names_line: 0,
            row_names_column: 0,
            units_line: 0,
            descriptions_line: 0,
            sheet_name: SheetSel::Name(String::new()),
            data_range: "A1".into(),
            names_range: String::new(),
            row_names_range: String::new(),
            units_range: String::new(),
            descriptions_range: String::new(),
        }
    }
    fn names(&self) -> Vec<String> {
        self.vars.iter().map(|v| v.name.clone()).collect()
    }
    fn bytes(&self) -> usize {
        512 + self.vars.iter().map(VarOpts::bytes).sum::<usize>() + self.selected.len() * 8
    }
    fn properties(&self) -> &'static [&'static str] {
        if self.sheet {
            &[
                "Sheet",
                "MissingRule",
                "ImportErrorRule",
                "VariableNames",
                "VariableTypes",
                "SelectedVariableNames",
                "VariableOptions",
                "VariableNamingRule",
                "DataRange",
                "VariableNamesRange",
                "RowNamesRange",
                "VariableUnitsRange",
                "VariableDescriptionsRange",
            ]
        } else {
            &[
                "Delimiter",
                "Whitespace",
                "LineEnding",
                "CommentStyle",
                "ConsecutiveDelimitersRule",
                "LeadingDelimitersRule",
                "TrailingDelimitersRule",
                "EmptyLineRule",
                "Encoding",
                "MissingRule",
                "ImportErrorRule",
                "ExtraColumnsRule",
                "VariableNames",
                "VariableTypes",
                "SelectedVariableNames",
                "VariableOptions",
                "VariableNamingRule",
                "DataLines",
                "VariableNamesLine",
                "RowNamesColumn",
                "VariableUnitsLine",
                "VariableDescriptionsLine",
            ]
        }
    }
    /// Short class name for messages ("DelimitedTextImportOptions").
    fn short(&self) -> &'static str {
        if self.sheet {
            "SpreadsheetImportOptions"
        } else {
            "DelimitedTextImportOptions"
        }
    }
    /// Variable indices named by `selection`: a name, names (cellstr or
    /// string array), positive indices or a logical mask.
    pub(super) fn selection(&self, who: &str, selection: &Value) -> Result<Vec<usize>> {
        if let Some(names) = text_list(selection) {
            return names
                .iter()
                .map(|n| {
                    self.vars.iter().position(|v| v.name == *n).ok_or_else(|| {
                        format!(
                            "{who}: unknown variable name '{n}'; VariableNames are {}",
                            list_text(&self.names())
                        )
                    })
                })
                .collect();
        }
        if let Value::Logical(m) = selection {
            if m.len() != self.vars.len() {
                return Err(format!(
                    "{who}: a logical selection must have one element per variable ({})",
                    self.vars.len()
                ));
            }
            return Ok((0..m.len()).filter(|&i| m.data()[i] != 0).collect());
        }
        let Some((_, _, values)) = numbers(selection) else {
            return Err(format!(
                "{who}: select variables by name (text, cellstr or string array) or by index"
            ));
        };
        values
            .iter()
            .map(|&x| {
                if x.fract() != 0. || x < 1. || x > self.vars.len() as f64 {
                    Err(format!(
                        "{who}: variable index {x} is out of range; the options have {} variables",
                        self.vars.len()
                    ))
                } else {
                    Ok(x as usize - 1)
                }
            })
            .collect()
    }
}

/// An object of this module: options for a file or for one variable.
#[derive(Clone, Debug)]
pub(in crate::engine) enum IoObject {
    Opts(Arc<ImportOpts>),
    Var(Arc<VarOpts>),
}

impl IoObject {
    pub(in crate::engine) fn class_index(&self) -> usize {
        match self {
            Self::Opts(o) => usize::from(o.sheet),
            Self::Var(v) => v.kind.class_index(),
        }
    }
    pub(in crate::engine) fn class(&self) -> &'static str {
        CLASSES[self.class_index()]
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        match self {
            Self::Opts(o) => o.bytes(),
            Self::Var(v) => v.bytes(),
        }
    }
    /// Equal property values (isequal): every field, the FillValue by
    /// its debug form.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Opts(a), Self::Opts(b)) => format!("{a:?}") == format!("{b:?}"),
            (Self::Var(a), Self::Var(b)) => format!("{a:?}") == format!("{b:?}"),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Value helpers

fn text_of(v: &Value) -> Option<String> {
    super::table_fns::text_scalar(v)
}

/// Text items of a char row, string array or cell array of text.
pub(in crate::engine) fn text_list(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(vec![s.clone()]),
        Value::Container(a) => array_text_list(a),
        _ => None,
    }
}

fn array_text_list(a: &Array) -> Option<Vec<String>> {
    match a {
        Array::Character(c) if c.rows() <= 1 && !c.is_nd() => {
            Some(vec![String::from_utf16_lossy(c.units())])
        }
        Array::Str(s) => s
            .data()
            .iter()
            .map(|t| t.as_deref().map(str::to_owned))
            .collect(),
        Array::Cell(c) => (0..c.len())
            .map(|i| match c.get_linear(i).ok()?.as_ref() {
                Array::Character(ch) if ch.rows() <= 1 && !ch.is_nd() => {
                    Some(String::from_utf16_lossy(ch.units()))
                }
                Array::Str(s) if s.len() == 1 => s.data()[0].as_deref().map(str::to_owned),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// Real numbers of a numeric or logical value: (rows, cols, column-major).
pub(super) fn numbers(v: &Value) -> Option<(usize, usize, Vec<f64>)> {
    match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => {
            Some((m.rows(), m.cols(), m.data().to_vec()))
        }
        Value::Logical(m) | Value::Container(Array::Logical(m)) => Some((
            m.rows(),
            m.cols(),
            m.data().iter().map(|&b| f64::from(b)).collect(),
        )),
        Value::Container(Array::Typed(t)) => Some((
            t.rows(),
            t.cols(),
            (0..t.len()).map(|i| t.real(i)).collect(),
        )),
        _ => None,
    }
}

fn scalar_number(v: &Value) -> Option<f64> {
    numbers(v).and_then(|(_, _, d)| (d.len() == 1).then(|| d[0]))
}

fn flag_value(who: &str, option: &str, v: &Value) -> Result<bool> {
    if let Some(t) = text_of(v) {
        return match t.to_ascii_lowercase().as_str() {
            "true" | "on" => Ok(true),
            "false" | "off" => Ok(false),
            _ => Err(format!("{who}: {option} must be true or false")),
        };
    }
    match scalar_number(v) {
        Some(x) if !x.is_nan() => Ok(x != 0.),
        _ => Err(format!("{who}: {option} must be true or false")),
    }
}

/// A nonnegative integer scalar.
fn count_value(who: &str, option: &str, v: &Value) -> Result<usize> {
    match scalar_number(v) {
        Some(x) if x >= 0. && x.fract() == 0. && x <= 1e9 => Ok(x as usize),
        _ => Err(format!(
            "{who}: {option} must be a nonnegative integer scalar"
        )),
    }
}

/// One of `allowed` (case-insensitive), returned in its canonical form.
fn choice(who: &str, option: &str, v: &Value, allowed: &[&str]) -> Result<String> {
    let text = text_of(v).unwrap_or_default();
    allowed
        .iter()
        .find(|a| a.eq_ignore_ascii_case(&text))
        .map(|a| (*a).to_owned())
        .ok_or_else(|| {
            format!(
                "{who}: {option} must be {}",
                allowed
                    .iter()
                    .map(|a| format!("'{a}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn char_array(e: &Engine, text: &str) -> Result<Array> {
    Ok(if text.is_empty() {
        Array::Character(CharacterRow::from_column_major(
            0,
            0,
            Vec::new(),
            &e.cancel,
        )?)
    } else {
        Array::Character(CharacterRow::from_text(text, &e.cancel)?)
    })
}

/// 1-by-N cellstr; 0-by-0 `{}` when empty.
fn cellstr(e: &Engine, items: &[String]) -> Result<Value> {
    if items.is_empty() {
        return Ok(Value::Container(Array::Cell(CellArray::empty(
            0, 0, &e.cancel,
        )?)));
    }
    let slots = items
        .iter()
        .map(|t| char_array(e, t).map(Arc::new))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        1,
        items.len(),
        slots,
        &e.cancel,
    )?)))
}

fn row_number(x: f64) -> String {
    if x.is_infinite() {
        if x > 0. { "Inf".into() } else { "-Inf".into() }
    } else if x.is_nan() {
        "NaN".into()
    } else {
        format!("{x}")
    }
}

fn list_text(items: &[String]) -> String {
    format!(
        "{{{}}}",
        items
            .iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Replace escape sequences \b \t \n \r \\ of a property text.
pub(super) fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('b') => out.push('\u{8}'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The escaped form of a delimiter or whitespace text.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Variable options: properties

/// The default FillValue of a type.
fn default_fill(e: &Engine, kind: VarType) -> Result<Array> {
    Ok(match kind {
        VarType::Double => Matrix::scalar(f64::NAN).into(),
        VarType::Single => Array::Typed(TypedArray::scalar_real(NumericClass::Single, f64::NAN)),
        VarType::Int(c) => Array::Typed(TypedArray::scalar_int(c, 0)),
        VarType::Logical => LogicalMatrix::scalar(false).into(),
        VarType::Char | VarType::Categorical => char_array(e, "")?,
        VarType::Str => Array::Str(StringArray::new(1, 1, vec![None])?),
        VarType::Datetime => Array::Datetime(crate::DatetimeArray::new(
            1,
            1,
            vec![crate::datetime_array::DateTimeValue::NAT],
            crate::datetime_array::DEFAULT_FORMAT,
            Default::default(),
        )?),
        VarType::Duration => Array::Duration(crate::DurationArray::new(
            1,
            1,
            vec![f64::NAN],
            crate::duration_array::DEFAULT_FORMAT,
        )?),
    })
}

fn fill_value(e: &Engine, v: &VarOpts) -> Result<Array> {
    match &v.fill {
        Some(a) => Ok(a.as_ref().clone()),
        None => default_fill(e, v.kind),
    }
}

fn var_property(e: &Engine, v: &VarOpts, name: &str) -> Result<Value> {
    let own = v.kind.own_properties();
    if !VAR_COMMON.contains(&name) && !own.contains(&name) {
        return Err(format!(
            "No appropriate method, property, or field '{name}' for class '{}'.",
            v.kind.class_name_short()
        ));
    }
    let flag = |b: bool| Ok(Value::Logical(LogicalMatrix::scalar(b)));
    match name {
        "Name" => Ok(Value::Text(v.name.clone())),
        "Type" => Ok(Value::Text(v.kind.name().into())),
        "FillValue" => Ok(fill_value(e, v)?.into()),
        "TreatAsMissing" => cellstr(e, &v.treat_as_missing),
        "QuoteRule" => Ok(Value::Text(v.quote_rule.clone())),
        "Prefixes" => cellstr(e, &v.prefixes),
        "Suffixes" => cellstr(e, &v.suffixes),
        "EmptyFieldRule" => Ok(Value::Text(v.empty_field_rule.clone())),
        "WhitespaceRule" => Ok(Value::Text(v.whitespace_rule.clone())),
        "ExponentCharacter" => Ok(Value::Text(v.exponent.clone())),
        "DecimalSeparator" => Ok(Value::Text(v.decimal.clone())),
        "ThousandsSeparator" => Ok(Value::Text(v.thousands.clone())),
        "TrimNonNumeric" => flag(v.trim_non_numeric),
        "NumberSystem" => Ok(Value::Text("decimal".into())),
        "InputFormat" => Ok(Value::Text(v.input_format.clone())),
        "DatetimeFormat" => Ok(Value::Text(v.datetime_format.clone())),
        "DatetimeLocale" => Ok(Value::Text(v.locale.clone())),
        "TimeZone" => Ok(Value::Text(v.time_zone.clone())),
        "DurationFormat" => Ok(Value::Text(v.duration_format.clone())),
        "FieldSeparator" => Ok(Value::Text(v.field_separator.clone())),
        "Categories" => cellstr(e, &v.categories),
        "Protected" => flag(v.protected),
        "Ordinal" => flag(v.ordinal),
        "TrueSymbols" => cellstr(e, &v.true_symbols),
        "FalseSymbols" => cellstr(e, &v.false_symbols),
        _ => flag(v.case_sensitive),
    }
}

impl VarType {
    fn class_name_short(self) -> &'static str {
        CLASSES[self.class_index()]
            .rsplit('.')
            .next()
            .unwrap_or_default()
    }
}

/// A FillValue checked against the variable's type.
fn checked_fill(who: &str, kind: VarType, value: &Value, e: &Engine) -> Result<Arc<Array>> {
    let bad = |what: &str| {
        Err(format!(
            "{who}: the FillValue of a {} variable must be {what}",
            kind.name()
        ))
    };
    let array = value.clone().array(&e.cancel)?;
    let ok = match kind {
        VarType::Double | VarType::Single | VarType::Int(_) | VarType::Logical => {
            scalar_number(value).is_some()
        }
        VarType::Char | VarType::Categorical => text_of(value).is_some(),
        VarType::Str => text_of(value).is_some() || matches!(&array, Array::Str(s) if s.len() == 1),
        VarType::Datetime => matches!(&array, Array::Datetime(d) if d.len() == 1),
        VarType::Duration => matches!(&array, Array::Duration(d) if d.len() == 1),
    };
    if !ok {
        return bad(match kind {
            VarType::Double | VarType::Single | VarType::Int(_) | VarType::Logical => {
                "a numeric or logical scalar"
            }
            VarType::Char | VarType::Categorical => "text",
            VarType::Str => "text or a string scalar",
            VarType::Datetime => "a datetime scalar",
            VarType::Duration => "a duration scalar",
        });
    }
    // Numbers are stored in the variable's class.
    let x = scalar_number(value);
    Ok(Arc::new(match (kind, x) {
        (VarType::Double, Some(x)) => Matrix::scalar(x).into(),
        (VarType::Single, Some(x)) => {
            Array::Typed(TypedArray::scalar_real(NumericClass::Single, x))
        }
        (VarType::Int(c), Some(x)) => Array::Typed(TypedArray::scalar_int(c, c.from_f64(x))),
        (VarType::Logical, Some(x)) => LogicalMatrix::scalar(x != 0.).into(),
        (VarType::Char | VarType::Categorical, _) => char_array(e, &text_of(value).unwrap())?,
        (VarType::Str, _) => match text_of(value) {
            Some(t) => Array::Str(StringArray::new(1, 1, vec![Some(Arc::from(t.as_str()))])?),
            None => array,
        },
        _ => array,
    }))
}

/// Set one property of a variable's options.
fn set_var_property(
    e: &Engine,
    who: &str,
    v: &mut VarOpts,
    name: &str,
    value: &Value,
) -> Result<()> {
    let own = v.kind.own_properties();
    let Some(name) = VAR_COMMON
        .iter()
        .chain(own)
        .find(|p| p.eq_ignore_ascii_case(name))
        .copied()
    else {
        return Err(format!(
            "{who}: '{name}' is not a property of {} (a variable of type '{}')",
            v.kind.class_name_short(),
            v.kind.name()
        ));
    };
    let text = |option: &str| text_of(value).ok_or_else(|| format!("{who}: {option} must be text"));
    let texts = |option: &str| {
        text_list(value).ok_or_else(|| {
            format!("{who}: {option} must be text, a cell array of text or a string array")
        })
    };
    match name {
        "Name" => {
            let t = text("Name")?;
            if t.is_empty() {
                return Err(format!("{who}: a variable name cannot be empty"));
            }
            v.name = t;
        }
        "Type" => {
            let t = text("Type")?;
            let kind = VarType::parse(&t).ok_or_else(|| bad_type(who, &t))?;
            *v = v.retyped(kind);
        }
        "FillValue" => v.fill = Some(checked_fill(who, v.kind, value, e)?),
        "TreatAsMissing" => v.treat_as_missing = texts("TreatAsMissing")?,
        "QuoteRule" => {
            v.quote_rule = choice(who, "QuoteRule", value, &["remove", "keep", "error"])?
        }
        "Prefixes" => v.prefixes = texts("Prefixes")?,
        "Suffixes" => v.suffixes = texts("Suffixes")?,
        "EmptyFieldRule" => {
            v.empty_field_rule =
                choice(who, "EmptyFieldRule", value, &["missing", "error", "auto"])?;
        }
        "WhitespaceRule" => {
            v.whitespace_rule = choice(
                who,
                "WhitespaceRule",
                value,
                &["trim", "trimleading", "trimtrailing", "preserve"],
            )?;
        }
        "ExponentCharacter" => {
            let t = text("ExponentCharacter")?;
            if t.chars().any(|c| !c.is_alphabetic()) {
                return Err(format!("{who}: ExponentCharacter must be letters"));
            }
            v.exponent = t;
        }
        "DecimalSeparator" => {
            let t = text("DecimalSeparator")?;
            if t.chars().count() != 1 || t.chars().any(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "{who}: DecimalSeparator must be a single non-digit character"
                ));
            }
            if v.thousands == t {
                return Err(format!(
                    "{who}: DecimalSeparator and ThousandsSeparator must differ"
                ));
            }
            v.decimal = t;
        }
        "ThousandsSeparator" => {
            let t = text("ThousandsSeparator")?;
            if t.chars().count() > 1 || t.chars().any(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "{who}: ThousandsSeparator must be '' or a single non-digit character"
                ));
            }
            if !t.is_empty() && v.decimal == t {
                return Err(format!(
                    "{who}: DecimalSeparator and ThousandsSeparator must differ"
                ));
            }
            v.thousands = t;
        }
        "TrimNonNumeric" => v.trim_non_numeric = flag_value(who, "TrimNonNumeric", value)?,
        "NumberSystem" => {
            choice(who, "NumberSystem", value, &["decimal"])?;
        }
        "InputFormat" => {
            let t = text("InputFormat")?;
            if !t.is_empty() {
                if v.kind == VarType::Datetime {
                    super::datetime_fns::text::tokens(&t)
                        .map_err(|err| format!("{who}: InputFormat '{t}': {err}"))?;
                } else {
                    super::datetime_fns::dur_text::parse_input_format(&t)
                        .map_err(|err| format!("{who}: {err}"))?;
                }
            }
            v.input_format = t;
        }
        "DatetimeFormat" => {
            let t = text("DatetimeFormat")?;
            if !["preserveinput", "default", "defaultdate"]
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&t))
            {
                super::datetime_fns::text::tokens(&t)
                    .map_err(|err| format!("{who}: DatetimeFormat '{t}': {err}"))?;
            }
            v.datetime_format = t;
        }
        "DatetimeLocale" => {
            let t = text("DatetimeLocale")?;
            if !t.to_ascii_lowercase().starts_with("en") && !t.eq_ignore_ascii_case("system") {
                return Err(format!(
                    "{who}: only English DatetimeLocale values ('en_US', ...) are supported"
                ));
            }
            v.locale = t;
        }
        "TimeZone" => {
            let t = text("TimeZone")?;
            if !(t.is_empty() || t.eq_ignore_ascii_case("UTC")) {
                return Err(format!("{who}: TimeZone must be '' or 'UTC'"));
            }
            v.time_zone = t;
        }
        "DurationFormat" => {
            let t = text("DurationFormat")?;
            if !t.eq_ignore_ascii_case("default") {
                super::datetime_fns::dur_text::parse_format(&t)
                    .map_err(|err| format!("{who}: {err}"))?;
            }
            v.duration_format = t;
        }
        "FieldSeparator" => {
            let t = text("FieldSeparator")?;
            if t.chars().count() != 1 {
                return Err(format!("{who}: FieldSeparator must be one character"));
            }
            v.field_separator = t;
        }
        "Categories" => v.categories = texts("Categories")?,
        "Protected" => v.protected = flag_value(who, "Protected", value)?,
        "Ordinal" => v.ordinal = flag_value(who, "Ordinal", value)?,
        "TrueSymbols" => v.true_symbols = texts("TrueSymbols")?,
        "FalseSymbols" => v.false_symbols = texts("FalseSymbols")?,
        _ => v.case_sensitive = flag_value(who, "CaseSensitive", value)?,
    }
    Ok(())
}

fn bad_type(who: &str, name: &str) -> String {
    format!("{who}: '{name}' is not a supported variable type; use {TYPE_NAMES}")
}

// ---------------------------------------------------------------------------
// Options objects: properties

/// Rows [first last] of a DataLines value, checked.
fn data_lines_value(who: &str, value: &Value) -> Result<Vec<[f64; 2]>> {
    let bad = || {
        format!(
            "{who}: DataLines must be a positive integer n, a range [first last], or an N-by-2 array of increasing, non-overlapping ranges"
        )
    };
    let (rows, cols, data) = numbers(value).ok_or_else(bad)?;
    let ranges: Vec<[f64; 2]> = match (rows, cols) {
        (1, 1) => vec![[data[0], f64::INFINITY]],
        (_, 2) => (0..rows).map(|r| [data[r], data[rows + r]]).collect(),
        _ => return Err(bad()),
    };
    if ranges.is_empty() {
        return Err(bad());
    }
    let mut previous = 0.;
    for [a, b] in &ranges {
        let integer = |x: f64| x.is_infinite() && x > 0. || x.fract() == 0.;
        if !(*a >= 1. && a.is_finite() && integer(*a) && integer(*b) && *b >= 1.) {
            return Err(bad());
        }
        if b < a || *a <= previous {
            return Err(format!(
                "{who}: DataLines ranges must be in increasing order, each [first last] with first <= last, and must not overlap"
            ));
        }
        previous = *b;
    }
    Ok(ranges)
}

/// A property other than VariableOptions (which makes objects: see
/// [`property`]).
fn opts_property(e: &Engine, o: &ImportOpts, name: &str) -> Result<Value> {
    if !o.properties().contains(&name) {
        return Err(format!(
            "No appropriate method, property, or field '{name}' for class '{}'.",
            if o.sheet { SHEET_CLASS } else { TEXT_CLASS }
        ));
    }
    let number = |x: usize| Ok(Value::Matrix(Matrix::scalar(x as f64)));
    match name {
        "Delimiter" => cellstr(e, &o.delimiter),
        "Whitespace" => Ok(Value::Text(o.whitespace.clone())),
        "LineEnding" => cellstr(e, &o.line_ending),
        "CommentStyle" => cellstr(e, &o.comment_style),
        "ConsecutiveDelimitersRule" => Ok(Value::Text(o.consecutive_rule.clone())),
        "LeadingDelimitersRule" => Ok(Value::Text(o.leading_rule.clone())),
        "TrailingDelimitersRule" => Ok(Value::Text(o.trailing_rule.clone())),
        "EmptyLineRule" => Ok(Value::Text(o.empty_line_rule.clone())),
        "Encoding" => Ok(Value::Text(o.encoding.clone())),
        "MissingRule" => Ok(Value::Text(o.missing_rule.clone())),
        "ImportErrorRule" => Ok(Value::Text(o.import_error_rule.clone())),
        "ExtraColumnsRule" => Ok(Value::Text(o.extra_columns_rule.clone())),
        "VariableNames" => cellstr(e, &o.names()),
        "VariableTypes" => {
            let types: Vec<String> = o.vars.iter().map(|v| v.kind.name().into()).collect();
            cellstr(e, &types)
        }
        "SelectedVariableNames" => {
            let names: Vec<String> = o.selected.iter().map(|&i| o.vars[i].name.clone()).collect();
            cellstr(e, &names)
        }
        "VariableOptions" => Ok(Value::Text(format!(
            "Show all {} VariableOptions",
            o.vars.len()
        ))),
        "VariableNamingRule" => Ok(Value::Text(
            if o.preserve_names {
                "preserve"
            } else {
                "modify"
            }
            .into(),
        )),
        "DataLines" => {
            let n = o.data_lines.len();
            let mut data = Vec::with_capacity(2 * n);
            data.extend(o.data_lines.iter().map(|r| r[0]));
            data.extend(o.data_lines.iter().map(|r| r[1]));
            Ok(Value::Matrix(Matrix::from_column_vec(n, 2, data)?))
        }
        "VariableNamesLine" => number(o.names_line),
        "RowNamesColumn" => number(o.row_names_column),
        "VariableUnitsLine" => number(o.units_line),
        "VariableDescriptionsLine" => number(o.descriptions_line),
        "Sheet" => Ok(match &o.sheet_name {
            SheetSel::Name(n) => Value::Text(n.clone()),
            SheetSel::Index(i) => Value::Matrix(Matrix::scalar(*i as f64)),
        }),
        "DataRange" => Ok(Value::Text(o.data_range.clone())),
        "VariableNamesRange" => Ok(Value::Text(o.names_range.clone())),
        "RowNamesRange" => Ok(Value::Text(o.row_names_range.clone())),
        "VariableUnitsRange" => Ok(Value::Text(o.units_range.clone())),
        _ => Ok(Value::Text(o.descriptions_range.clone())),
    }
}

/// Names checked for a VariableNames assignment.
fn checked_names(who: &str, names: &[String]) -> Result<()> {
    for (i, n) in names.iter().enumerate() {
        if n.is_empty() {
            return Err(format!("{who}: variable names cannot be empty"));
        }
        if n.chars().count() > crate::table_array::MAX_NAME {
            return Err(format!(
                "{who}: variable name '{n}' is longer than {} characters",
                crate::table_array::MAX_NAME
            ));
        }
        if names[..i].contains(n) {
            return Err(format!("{who}: duplicate variable name '{n}'"));
        }
    }
    Ok(())
}

/// A spreadsheet range text checked: a cell 'A2', a rectangle 'A2:C10', a
/// row range '2:10' or '' where allowed.
pub(super) fn checked_range(who: &str, option: &str, text: &str, empty_ok: bool) -> Result<()> {
    if text.is_empty() && empty_ok {
        return Ok(());
    }
    if read::parse_range(text).is_none() {
        return Err(format!(
            "{who}: {option} must be a cell such as 'A2', a rectangle such as 'A2:C10' or a row range such as '2:10'"
        ));
    }
    Ok(())
}

fn set_opts_property(e: &mut Engine, o: &mut ImportOpts, name: &str, value: &Value) -> Result<()> {
    let who = o.short();
    let Some(name) = o
        .properties()
        .iter()
        .find(|p| p.eq_ignore_ascii_case(name))
        .copied()
    else {
        return Err(format!(
            "Unrecognized property '{name}' for class '{}'.",
            if o.sheet { SHEET_CLASS } else { TEXT_CLASS }
        ));
    };
    let texts = |option: &str| {
        text_list(value).ok_or_else(|| {
            format!("{who}: {option} must be text, a cell array of text or a string array")
        })
    };
    let rule = |option: &str, allowed: &[&str]| choice(who, option, value, allowed);
    match name {
        "Delimiter" => {
            let list = texts("Delimiter")?;
            if list.is_empty() || list.iter().any(String::is_empty) {
                return Err(format!("{who}: Delimiter cannot be empty"));
            }
            o.delimiter = list
                .iter()
                .map(|d| escape(&detect::delimiter_text(d)))
                .collect();
        }
        "Whitespace" => o.whitespace = escape(&unescape(&texts("Whitespace")?.concat())),
        "LineEnding" => o.line_ending = texts("LineEnding")?,
        "CommentStyle" => o.comment_style = texts("CommentStyle")?,
        "ConsecutiveDelimitersRule" => {
            o.consecutive_rule = rule("ConsecutiveDelimitersRule", &["split", "join", "error"])?;
        }
        "LeadingDelimitersRule" => {
            o.leading_rule = rule("LeadingDelimitersRule", &["keep", "ignore", "error"])?;
        }
        "TrailingDelimitersRule" => {
            o.trailing_rule = rule("TrailingDelimitersRule", &["keep", "ignore", "error"])?;
        }
        "EmptyLineRule" => o.empty_line_rule = rule("EmptyLineRule", &["skip", "read", "error"])?,
        "Encoding" => {
            let t = text_of(value).unwrap_or_default();
            if !["UTF-8", "UTF8", "US-ASCII", "system"]
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&t))
            {
                return Err(format!("{who}: only UTF-8 Encoding is supported"));
            }
            o.encoding = t;
        }
        "MissingRule" => {
            o.missing_rule = rule("MissingRule", &["fill", "error", "omitrow", "omitvar"])?;
        }
        "ImportErrorRule" => {
            o.import_error_rule =
                rule("ImportErrorRule", &["fill", "error", "omitrow", "omitvar"])?;
        }
        "ExtraColumnsRule" => {
            o.extra_columns_rule =
                rule("ExtraColumnsRule", &["addvars", "ignore", "wrap", "error"])?;
        }
        "VariableNames" => {
            let names = texts("VariableNames")?;
            checked_names(who, &names)?;
            let selected: Vec<String> =
                o.selected.iter().map(|&i| o.vars[i].name.clone()).collect();
            let old = o.vars.len();
            o.vars.truncate(names.len());
            for k in old..names.len() {
                o.vars.push(VarOpts::new(String::new(), VarType::Char));
                o.selected.push(k);
            }
            for (v, n) in o.vars.iter_mut().zip(&names) {
                v.name = n.clone();
            }
            o.selected.retain(|&i| i < names.len());
            let _ = selected;
        }
        "VariableTypes" => {
            let types = texts("VariableTypes")?;
            if types.len() != o.vars.len() {
                return Err(format!(
                    "{who}: VariableTypes must have one type per variable ({})",
                    o.vars.len()
                ));
            }
            for (v, t) in o.vars.iter_mut().zip(&types) {
                let kind = VarType::parse(t).ok_or_else(|| bad_type(who, t))?;
                *v = v.retyped(kind);
            }
        }
        "SelectedVariableNames" => {
            let picked = o.selection(who, value)?;
            for (i, p) in picked.iter().enumerate() {
                if picked[..i].contains(p) {
                    return Err(format!(
                        "{who}: SelectedVariableNames lists '{}' twice",
                        o.vars[*p].name
                    ));
                }
            }
            o.selected = picked;
        }
        "VariableOptions" => {
            let Value::Container(array) = value else {
                return Err(format!(
                    "{who}: VariableOptions must be variable import options objects"
                ));
            };
            let objects = super::objects::import_values(e, array).ok_or_else(|| {
                format!("{who}: VariableOptions must be variable import options objects")
            })?;
            let vars: Vec<VarOpts> = objects
                .into_iter()
                .map(|o| match o {
                    IoObject::Var(v) => Ok(v.as_ref().clone()),
                    IoObject::Opts(_) => Err(format!(
                        "{who}: VariableOptions must be variable import options objects"
                    )),
                })
                .collect::<Result<_>>()?;
            if vars.len() != o.vars.len() {
                return Err(format!(
                    "{who}: VariableOptions must have one element per variable ({})",
                    o.vars.len()
                ));
            }
            checked_names(
                who,
                &vars.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
            )?;
            o.vars = vars;
        }
        "VariableNamingRule" => {
            o.preserve_names = rule("VariableNamingRule", &["modify", "preserve"])? == "preserve";
        }
        "DataLines" => o.data_lines = data_lines_value(who, value)?,
        "VariableNamesLine" => o.names_line = count_value(who, "VariableNamesLine", value)?,
        "RowNamesColumn" => {
            let k = count_value(who, "RowNamesColumn", value)?;
            if k > o.vars.len() {
                return Err(format!(
                    "{who}: RowNamesColumn must be 0 or a column number up to {}",
                    o.vars.len()
                ));
            }
            o.row_names_column = k;
        }
        "VariableUnitsLine" => o.units_line = count_value(who, "VariableUnitsLine", value)?,
        "VariableDescriptionsLine" => {
            o.descriptions_line = count_value(who, "VariableDescriptionsLine", value)?;
        }
        "Sheet" => {
            o.sheet_name = match text_of(value) {
                Some(t) => SheetSel::Name(t),
                None => match scalar_number(value) {
                    Some(x) if x >= 1. && x.fract() == 0. && x <= 1e6 => {
                        SheetSel::Index(x as usize)
                    }
                    _ => {
                        return Err(format!(
                            "{who}: Sheet must be a sheet name or a positive sheet index"
                        ));
                    }
                },
            };
        }
        "DataRange" => {
            let t = match text_of(value) {
                Some(t) => t,
                None => match scalar_number(value) {
                    Some(x) if x >= 1. && x.fract() == 0. => format!("A{x}"),
                    _ => return Err(format!("{who}: DataRange must be text such as 'A2'")),
                },
            };
            checked_range(who, "DataRange", &t, false)?;
            o.data_range = t;
        }
        "VariableNamesRange"
        | "RowNamesRange"
        | "VariableUnitsRange"
        | "VariableDescriptionsRange" => {
            let t = text_of(value).ok_or_else(|| format!("{who}: {name} must be text"))?;
            checked_range(who, name, &t, true)?;
            match name {
                "VariableNamesRange" => o.names_range = t,
                "RowNamesRange" => o.row_names_range = t,
                "VariableUnitsRange" => o.units_range = t,
                _ => o.descriptions_range = t,
            }
        }
        _ => unreachable!("listed property {name}"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry points from objects.rs

/// `object.name`.
pub(in crate::engine) fn property(e: &mut Engine, o: &IoObject, name: &str) -> Result<Value> {
    match o {
        IoObject::Opts(opts) if name == "VariableOptions" => {
            let objects = opts
                .vars
                .iter()
                .map(|v| IoObject::Var(Arc::new(v.clone())))
                .collect();
            super::objects::import_array(e, objects)
        }
        _ => property_ref(e, o, name),
    }
}

/// `object.name` where no object can be made (VariableOptions is then a
/// description).
pub(in crate::engine) fn property_ref(e: &Engine, o: &IoObject, name: &str) -> Result<Value> {
    match o {
        IoObject::Opts(opts) => opts_property(e, opts, name),
        IoObject::Var(v) => var_property(e, v, name),
    }
}

/// `object.steps = value`: the changed copy.
pub(in crate::engine) fn assign(
    e: &mut Engine,
    o: &IoObject,
    steps: &[PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<IoObject> {
    let name = match steps.first() {
        Some(PathStep::Field(name)) => name.clone(),
        Some(PathStep::Dynamic(expr)) => e.dynamic_field_name(expr, out)?,
        _ => {
            return Err(format!(
                "Indexing into a {} object is not supported; use dot names such as opts.DataLines",
                o.class()
            ));
        }
    };
    let value = if steps.len() > 1 {
        let current = property(e, o, &name)?.array(&e.cancel)?;
        e.assign_step(Some(current), &steps[1..], value, false, out)?
    } else {
        value
    };
    let value = Value::from(value);
    Ok(match o {
        IoObject::Opts(opts) => {
            let mut changed = opts.as_ref().clone();
            set_opts_property(e, &mut changed, &name, &value)?;
            IoObject::Opts(Arc::new(changed))
        }
        IoObject::Var(v) => {
            let mut changed = v.as_ref().clone();
            let who = v.kind.class_name_short();
            set_var_property(e, who, &mut changed, &name, &value)?;
            IoObject::Var(Arc::new(changed))
        }
    })
}

fn display_value(v: &Value) -> String {
    match v {
        Value::Text(t) => format!("'{t}'"),
        Value::Matrix(m) => {
            if m.len() == 1 {
                row_number(m.data()[0])
            } else {
                let (rows, cols) = (m.rows(), m.cols());
                let text = (0..rows)
                    .map(|r| {
                        (0..cols)
                            .map(|c| row_number(m.data()[c * rows + r]))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("[{text}]")
            }
        }
        Value::Logical(m) => m.data().first().map_or("0".into(), |b| b.to_string()),
        Value::Container(Array::Cell(_)) => {
            let items = text_list(v).unwrap_or_default();
            cell_display(&items)
        }
        Value::Container(a) => match a {
            Array::Character(_) => format!("'{}'", text_of(v).unwrap_or_default()),
            Array::Str(s) if s.len() == 1 => match &s.data()[0] {
                Some(t) => format!("\"{t}\""),
                None => "<missing>".into(),
            },
            Array::Datetime(d) if d.len() == 1 && d.data()[0].is_nat() => "NaT".into(),
            Array::Duration(d) if d.len() == 1 && d.data()[0].is_nan() => "NaN".into(),
            Array::Logical(m) if m.len() == 1 => m.data()[0].to_string(),
            Array::Typed(t) if t.len() == 1 => {
                if t.is_integer() {
                    t.int(0).to_string()
                } else {
                    row_number(t.real(0))
                }
            }
            Array::Numeric(m) if m.len() == 1 => row_number(m.data()[0]),
            other => format!("[{}x{} {}]", other.rows(), other.cols(), other.class()),
        },
        Value::QuotedString(t) => format!("\"{t}\""),
        _ => String::new(),
    }
}

/// `{'a'  'b'  'c' ... and 4 more}`.
fn cell_display(items: &[String]) -> String {
    if items.is_empty() {
        return "{}".into();
    }
    let shown: Vec<String> = items.iter().take(3).map(|t| format!("'{t}'")).collect();
    if items.len() > 3 {
        format!("{{{} ... and {} more}}", shown.join("  "), items.len() - 3)
    } else {
        format!("{{{}}}", shown.join("  "))
    }
}

fn lines(out: &mut String, rows: &[(&str, String)], width: usize) {
    for (name, value) in rows {
        out.push_str(&format!("{name:>width$}: {value}\n"));
    }
}

/// The text `disp` shows for an object.
pub(in crate::engine) fn body(e: &Engine, o: &IoObject) -> String {
    match o {
        IoObject::Opts(opts) => opts_body(e, opts),
        IoObject::Var(v) => var_body(e, v),
    }
}

fn opts_body(e: &Engine, o: &ImportOpts) -> String {
    let mut get = |name: &str| -> String {
        if name == "VariableOptions" {
            return format!("Show all {} VariableOptions", o.vars.len());
        }
        opts_property(e, o, name)
            .map(|v| display_value(&v))
            .unwrap_or_default()
    };
    let mut out = format!("  {} with properties:\n\n", o.short());
    let width = 29;
    let group =
        |out: &mut String, title: &str, names: &[&str], get: &mut dyn FnMut(&str) -> String| {
            out.push_str(&format!("   {title}:\n"));
            let rows: Vec<(&str, String)> = names.iter().map(|n| (*n, get(n))).collect();
            lines(out, &rows, width);
            out.push('\n');
        };
    if o.sheet {
        group(&mut out, "Sheet Properties", &["Sheet"], &mut get);
        group(
            &mut out,
            "Replacement Properties",
            &["MissingRule", "ImportErrorRule"],
            &mut get,
        );
    } else {
        group(
            &mut out,
            "Format Properties",
            &[
                "Delimiter",
                "Whitespace",
                "LineEnding",
                "CommentStyle",
                "ConsecutiveDelimitersRule",
                "LeadingDelimitersRule",
                "TrailingDelimitersRule",
                "EmptyLineRule",
                "Encoding",
            ],
            &mut get,
        );
        group(
            &mut out,
            "Replacement Properties",
            &["MissingRule", "ImportErrorRule", "ExtraColumnsRule"],
            &mut get,
        );
    }
    out.push_str("   Variable Import Properties: Set types by name using setvartype\n");
    let rows: Vec<(&str, String)> = [
        "VariableNames",
        "VariableTypes",
        "SelectedVariableNames",
        "VariableOptions",
    ]
    .iter()
    .map(|n| (*n, get(n)))
    .collect();
    lines(&mut out, &rows, width);
    out.push_str("\tAccess VariableOptions sub-properties using setvaropts/getvaropts\n");
    let rows = [("VariableNamingRule", get("VariableNamingRule"))];
    lines(&mut out, &rows, width);
    out.push('\n');
    if o.sheet {
        out.push_str("   Range Properties:\n");
        let data_range = if read::parse_range(&o.data_range).is_some_and(|r| r.is_start_cell()) {
            format!("'{}' (Start Cell)", o.data_range)
        } else {
            format!("'{}'", o.data_range)
        };
        let mut rows = vec![("DataRange", data_range)];
        for n in [
            "VariableNamesRange",
            "RowNamesRange",
            "VariableUnitsRange",
            "VariableDescriptionsRange",
        ] {
            rows.push((n, get(n)));
        }
        lines(&mut out, &rows, width);
    } else {
        out.push_str("   Location Properties:\n");
        let rows: Vec<(&str, String)> = [
            "DataLines",
            "VariableNamesLine",
            "RowNamesColumn",
            "VariableUnitsLine",
            "VariableDescriptionsLine",
        ]
        .iter()
        .map(|n| (*n, get(n)))
        .collect();
        lines(&mut out, &rows, width);
    }
    out.push_str("\tTo display a preview of the table, use preview\n");
    out
}

fn var_body(e: &Engine, v: &VarOpts) -> String {
    let mut out = format!("  {} with properties:\n\n", v.kind.class_name_short());
    let width = 22;
    let get = |name: &str| {
        var_property(e, v, name)
            .map(|x| display_value(&x))
            .unwrap_or_default()
    };
    out.push_str("   Variable Properties:\n");
    let rows: Vec<(&str, String)> = VAR_COMMON.iter().map(|n| (*n, get(n))).collect();
    lines(&mut out, &rows, width);
    out.push('\n');
    out.push_str(&format!("   {}:\n", v.kind.group_title()));
    let rows: Vec<(&str, String)> = v
        .kind
        .own_properties()
        .iter()
        .map(|n| (*n, get(n)))
        .collect();
    lines(&mut out, &rows, width);
    out
}

/// Display of a 1-by-N array of variable options (`opts.VariableOptions`).
pub(in crate::engine) fn array_body(objects: &[IoObject]) -> String {
    let mut classes: Vec<&str> = Vec::new();
    for o in objects {
        let short = o.class().rsplit('.').next().unwrap_or_default();
        if !classes.contains(&short) {
            classes.push(short);
        }
    }
    let mut out = if classes.len() == 1 {
        format!(
            "  1x{} {} array with properties:\n\n",
            objects.len(),
            classes[0]
        )
    } else {
        format!(
            "  1x{} heterogeneous VariableImportOptions ({}) array with properties:\n\n",
            objects.len(),
            classes.join(", ")
        )
    };
    for name in VAR_COMMON {
        out.push_str(&format!("    {name}\n"));
    }
    out
}

/// `fieldnames(object)` / `properties(object)`.
fn property_names(o: &IoObject) -> Vec<String> {
    match o {
        IoObject::Opts(opts) => opts.properties().iter().map(|s| (*s).into()).collect(),
        IoObject::Var(v) => VAR_COMMON
            .iter()
            .chain(v.kind.own_properties())
            .map(|s| (*s).into())
            .collect(),
    }
}

/// Calls whose first argument is an object of this module. None for the
/// generic object queries objects.rs answers (class, size, disp, ...).
pub(in crate::engine) fn method(
    e: &mut Engine,
    o: &IoObject,
    name: &str,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    match name {
        "setvartype" | "getvaropts" | "setvaropts" => {
            let IoObject::Opts(opts) = o else {
                return Err(format!(
                    "{name}: the first input must be import options from detectImportOptions"
                ));
            };
            let _ = out;
            match name {
                "setvartype" => setvartype(e, opts, rest).map(Some),
                "getvaropts" => getvaropts(e, opts, rest).map(Some),
                _ => setvaropts(e, opts, rest).map(Some),
            }
        }
        "isa" => {
            let [kind] = rest else {
                return Err("isa: expected one class name".into());
            };
            let kind = text_of(kind).unwrap_or_default();
            let mut family = vec![o.class()];
            match o {
                IoObject::Opts(opts) => {
                    family.push("matlab.io.ImportOptions");
                    family.push(if opts.sheet {
                        "matlab.io.spreadsheet.SpreadsheetImportOptions"
                    } else {
                        "matlab.io.text.TextImportOptions"
                    });
                }
                IoObject::Var(_) => family.push("matlab.io.VariableImportOptions"),
            }
            Ok(Some(Value::Logical(LogicalMatrix::scalar(
                family.contains(&kind.as_str()),
            ))))
        }
        "properties" | "fieldnames" => {
            let names = property_names(o);
            let slots = names
                .iter()
                .map(|n| char_array(e, n).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(Value::Container(Array::Cell(CellArray::from_shared(
                names.len(),
                1,
                slots,
                &e.cancel,
            )?))))
        }
        "isequal" => {
            if rest.is_empty() {
                return Err("Not enough input arguments.".into());
            }
            let same = rest
                .iter()
                .all(|other| super::objects::import_value(e, other).is_some_and(|x| x.same(o)));
            Ok(Some(Value::Logical(LogicalMatrix::scalar(same))))
        }
        "class" | "isobject" | "isstruct" | "iscell" | "isnumeric" | "ischar" | "islogical"
        | "isfloat" | "disp" | "display" | "length" | "size" | "isempty" | "numel" | "ndims" => {
            Ok(None)
        }
        _ => Err(format!(
            "Undefined function '{name}' for input arguments of type '{}'.",
            o.class()
        )),
    }
}

/// Builtins of [`NAMES`]; None for other names.
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if !NAMES.contains(&name) {
        return Ok(None);
    }
    e.check_cancel()?;
    if let Some(first) = args.first()
        && let Some(o) = super::objects::import_value(e, first)
    {
        return method(e, &o, name, &args[1..], out)?
            .map(Some)
            .ok_or_else(|| format!("{name}: unsupported call"));
    }
    match name {
        "detectImportOptions" => {
            let opts = detect::detect_import_options(e, args)?;
            super::objects::import_new(e, IoObject::Opts(Arc::new(opts)))
        }
        "preview" => read::preview(e, args),
        "readtimetable" => read::readtimetable(e, args),
        _ => Err(format!(
            "{name}: the first input must be import options from detectImportOptions"
        )),
    }
    .map(Some)
}

fn setvartype(e: &mut Engine, o: &ImportOpts, rest: &[Value]) -> Result<Value> {
    let who = "setvartype";
    let (indices, types) = match rest {
        [types] => ((0..o.vars.len()).collect::<Vec<_>>(), types),
        [selection, types] => (o.selection(who, selection)?, types),
        [] => return Err("Not enough input arguments.".into()),
        _ => return Err("Too many input arguments.".into()),
    };
    let types = text_list(types)
        .ok_or_else(|| format!("{who}: the type must be text or a cell array of text"))?;
    let types = if types.len() == 1 {
        vec![types[0].clone(); indices.len()]
    } else if types.len() == indices.len() {
        types
    } else {
        return Err(format!(
            "{who}: give one type, or one type per selected variable ({})",
            indices.len()
        ));
    };
    let mut changed = o.clone();
    for (&i, t) in indices.iter().zip(&types) {
        let kind = VarType::parse(t).ok_or_else(|| bad_type(who, t))?;
        changed.vars[i] = changed.vars[i].retyped(kind);
    }
    super::objects::import_new(e, IoObject::Opts(Arc::new(changed)))
}

fn getvaropts(e: &mut Engine, o: &ImportOpts, rest: &[Value]) -> Result<Value> {
    let indices = match rest {
        [] => (0..o.vars.len()).collect(),
        [selection] => o.selection("getvaropts", selection)?,
        _ => return Err("Too many input arguments.".into()),
    };
    let objects = indices
        .iter()
        .map(|&i| IoObject::Var(Arc::new(o.vars[i].clone())))
        .collect();
    super::objects::import_array(e, objects)
}

fn setvaropts(e: &mut Engine, o: &ImportOpts, rest: &[Value]) -> Result<Value> {
    let who = "setvaropts";
    let (indices, pairs) = if rest.len() % 2 == 1 {
        (o.selection(who, &rest[0])?, &rest[1..])
    } else {
        ((0..o.vars.len()).collect::<Vec<_>>(), rest)
    };
    if pairs.is_empty() {
        return Err(format!("{who}: give property names and values"));
    }
    let mut changed = o.clone();
    for pair in pairs.chunks(2) {
        let name =
            text_of(&pair[0]).ok_or_else(|| format!("{who}: property names must be text"))?;
        if name.eq_ignore_ascii_case("Name") && indices.len() > 1 {
            return Err(format!("{who}: set Name for one variable at a time"));
        }
        if name.eq_ignore_ascii_case("Type") {
            return Err(format!("{who}: use setvartype to change variable types"));
        }
        for &i in &indices {
            set_var_property(e, who, &mut changed.vars[i], &name, &pair[1])?;
        }
    }
    checked_names(who, &changed.names())?;
    super::objects::import_new(e, IoObject::Opts(Arc::new(changed)))
}
