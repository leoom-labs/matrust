//! datetime and duration values: construction, properties, conversions,
//! arithmetic, comparisons, ranges, concatenation and the collection
//! functions (sort, unique, min, max, mean, diff, ...). Storage lives in
//! src/datetime_array.rs and src/duration_array.rs. The hook in `Engine::call`
//! runs before the clock (env_files) and datenum (dates) families but claims a
//! call only for a new name or when an argument is a datetime or duration, so
//! now/clock/date/datenum/datestr keep their numeric behaviour.
pub(crate) mod calendar;
mod calendar_duration;
mod collect;
pub(crate) use collect::arrays_equal;
pub(crate) mod display;
pub(crate) mod dur_text;
mod ops;
pub(super) use ops::join_grid;
mod props;
pub(crate) mod text;

use super::{Engine, Execution, Value};
use crate::datetime_array::{
    DEFAULT_DATE_FORMAT, DEFAULT_FORMAT, DateTimeValue, MAX_ELEMENTS, TimeZone,
};
use crate::syntax::Expr;
use crate::{
    Array, CellArray, CharacterRow, DatetimeArray, DurationArray, LogicalMatrix, Matrix, Result,
};
use std::sync::Arc;

/// Names this module owns whatever the argument types.
const NAMES: &[&str] = &[
    "datetime",
    "duration",
    "NaT",
    "seconds",
    "minutes",
    "hours",
    "days",
    "years",
    "milliseconds",
    "isnat",
    "isdatetime",
    "isduration",
    "posixtime",
    "juliandate",
    "exceltime",
    "ymd",
    "hms",
    "timeofday",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
];

/// Names left to the existing code even with a datetime or duration argument:
/// they treat their arguments as opaque values.
const PASS_THROUGH: &[&str] = &[
    "disp",
    "display",
    "struct",
    "deal",
    "setfield",
    "getfield",
    "isfield",
    "rmfield",
    "fieldnames",
    "feval",
    "cellfun",
    "arrayfun",
    "assert",
    "func2str",
    "builtin",
    "exist",
    "error",
    "warning",
    "fprintf",
    "sprintf",
    "cell2mat",
    "iscellstr",
    "isobject",
    "ishandle",
    "isgraphics",
];

const DATETIME_ND: &str = "datetime: N-D datetime arrays are not supported yet";
const DURATION_ND: &str = "duration: N-D duration arrays are not supported yet";
const CANCEL_STRIDE: usize = 65_536;
const CANCELLED: &str = "Calculation cancelled";

/// `f(i)` for every index below `n`, polling `cancel` before each block of
/// CANCEL_STRIDE elements: the rule for every elementwise loop here.
pub(crate) fn polled<U>(
    cancel: &std::sync::atomic::AtomicBool,
    n: usize,
    mut f: impl FnMut(usize) -> U,
) -> Result<Vec<U>> {
    let mut out = Vec::with_capacity(n);
    let mut start = 0;
    while start < n {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let end = start.saturating_add(CANCEL_STRIDE).min(n);
        out.extend((start..end).map(&mut f));
        start = end;
    }
    Ok(out)
}

/// `f` over every element of `data`, polled like [`polled`].
pub(crate) fn polled_map<T: Copy, U>(
    cancel: &std::sync::atomic::AtomicBool,
    data: &[T],
    mut f: impl FnMut(T) -> U,
) -> Result<Vec<U>> {
    polled(cancel, data.len(), |i| f(data[i]))
}

/// Bare names that call the function: `NaT`, `datetime` (the current time).
pub(super) fn bare(name: &str) -> bool {
    matches!(name, "NaT" | "datetime")
}

/// Multi-output names owned by this module.
pub(super) fn owns_outputs(name: &str) -> bool {
    matches!(name, "ymd" | "hms" | "split") || calendar_duration::NAMES.contains(&name)
}

pub(super) fn is_dt(v: &Value) -> bool {
    matches!(v, Value::Container(Array::Datetime(_) | Array::Duration(_)))
}

fn datetime_of(v: &Value) -> Option<&DatetimeArray> {
    match v {
        Value::Container(Array::Datetime(t)) => Some(t),
        _ => None,
    }
}

fn duration_of(v: &Value) -> Option<&DurationArray> {
    match v {
        Value::Container(Array::Duration(d)) => Some(d),
        _ => None,
    }
}

fn dt_value(t: DatetimeArray) -> Value {
    Value::Container(Array::Datetime(t))
}

fn du_value(d: DurationArray) -> Value {
    Value::Container(Array::Duration(d))
}

/// Logical result from 0/1 bytes.
fn logical(rows: usize, cols: usize, bytes: Vec<u8>) -> Result<Value> {
    Ok(Value::Logical(LogicalMatrix::from_column_vec(
        rows,
        cols,
        bytes,
        &std::sync::atomic::AtomicBool::new(false),
    )?))
}

fn number_matrix(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

fn scalar_bool(b: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(b))
}

/// Default format for a datetime array made from dates only.
fn default_format(date_only: bool) -> &'static str {
    if date_only {
        DEFAULT_DATE_FORMAT
    } else {
        DEFAULT_FORMAT
    }
}

fn current_year() -> i64 {
    calendar::now(TimeZone::Unzoned)
        .map(|v| calendar::parts(v).y)
        .unwrap_or(2000)
}

fn default_pivot() -> i64 {
    current_year() - 50
}

/// Result shape of two operands under implicit expansion.
fn broadcast(a: (usize, usize), b: (usize, usize), who: &str) -> Result<(usize, usize)> {
    let dim = |x: usize, y: usize| {
        if x == y {
            Some(x)
        } else if x == 1 {
            Some(y)
        } else if y == 1 {
            Some(x)
        } else {
            None
        }
    };
    match (dim(a.0, b.0), dim(a.1, b.1)) {
        (Some(r), Some(c)) => {
            crate::datetime_array::checked_len(r, c, who)?;
            Ok((r, c))
        }
        _ => Err(format!(
            "{who}: arrays have incompatible sizes for this operation"
        )),
    }
}

/// Linear index into an operand of shape `shape` for output position `i` of
/// an output with `rows` rows (implicit expansion).
fn expand_index(i: usize, rows: usize, shape: (usize, usize)) -> usize {
    let (r, c) = (i % rows.max(1), i / rows.max(1));
    let r = if shape.0 == 1 { 0 } else { r };
    let c = if shape.1 == 1 { 0 } else { c };
    c * shape.0 + r
}

/// Text elements (None for a missing string) and their shape.
pub(super) type TextElements = (Vec<Option<String>>, (usize, usize));

/// Text elements of a char row/matrix, string array or cellstr with their
/// shape; None for other values. Missing strings are None elements.
pub(super) fn text_elements(
    v: &Value,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Option<TextElements>> {
    Ok(Some(match v {
        Value::Text(s) | Value::QuotedString(s) => (vec![Some(s.clone())], (1, 1)),
        Value::Container(Array::Character(c)) => {
            let (rows, cols) = (c.rows(), c.cols());
            if rows <= 1 {
                (vec![Some(c.to_utf8(cancel)?)], (1, 1))
            } else {
                // Polled per row; a row has at most MAX_ELEMENTS / rows units.
                let texts = polled(cancel, rows, |r| {
                    let row: Vec<u16> = (0..cols).map(|k| c.units()[k * rows + r]).collect();
                    Some(String::from_utf16_lossy(&row).trim_end().to_string())
                })?;
                (texts, (rows, 1))
            }
        }
        Value::Container(Array::Str(s)) => (
            polled(cancel, s.len(), |i| {
                s.data()[i].as_deref().map(str::to_string)
            })?,
            (s.rows(), s.cols()),
        ),
        Value::Container(Array::Cell(c)) => {
            let mut texts = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                if i % CANCEL_STRIDE == CANCEL_STRIDE - 1
                    && cancel.load(std::sync::atomic::Ordering::Relaxed)
                {
                    return Err(CANCELLED.into());
                }
                match c.get_linear(i)?.as_ref() {
                    Array::Character(ch) if ch.rows() <= 1 => texts.push(Some(ch.to_utf8(cancel)?)),
                    Array::Str(s) if s.len() == 1 => {
                        texts.push(s.data()[0].as_deref().map(str::to_string))
                    }
                    _ => return Ok(None),
                }
            }
            (texts, (c.rows(), c.cols()))
        }
        _ => return Ok(None),
    }))
}

/// One text option value (char row or scalar string).
fn option_text(v: &Value, cancel: &std::sync::atomic::AtomicBool) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => c.to_utf8(cancel).ok(),
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(str::to_string)
        }
        _ => None,
    }
}

/// Name-value options of a constructor.
#[derive(Default)]
struct Options {
    format: Option<String>,
    input_format: Option<String>,
    zone: Option<TimeZone>,
    pivot: Option<i64>,
    convert_from: Option<String>,
}

fn parse_zone(text: &str, who: &str) -> Result<TimeZone> {
    match text {
        "" => Ok(TimeZone::Unzoned),
        t if t.eq_ignore_ascii_case("UTC") => Ok(TimeZone::Utc),
        other => Err(format!(
            "{who}: time zone '{other}' is not supported yet; use '' or 'UTC'"
        )),
    }
}

/// `'Format','preserveinput'`: display text in the form it was read with.
fn is_preserve_input(format: &str) -> bool {
    format.eq_ignore_ascii_case("preserveinput")
}

/// A datetime Format value: 'default' and 'defaultdate' name the defaults;
/// anything else must tokenize.
fn checked_datetime_format(format: &str) -> Result<String> {
    match format {
        "default" => Ok(DEFAULT_FORMAT.into()),
        "defaultdate" => Ok(DEFAULT_DATE_FORMAT.into()),
        f => {
            text::tokens(f)?;
            Ok(f.into())
        }
    }
}

fn checked_duration_format(format: &str) -> Result<String> {
    dur_text::parse_format(format)?;
    Ok(format.into())
}

impl Engine {
    fn dtm_check(&self, i: usize) -> Result<()> {
        if i % CANCEL_STRIDE == CANCEL_STRIDE - 1 {
            self.check_cancel()?;
        }
        Ok(())
    }

    /// Real numbers of a numeric or logical argument with its shape.
    fn dtm_numbers(&self, v: &Value, who: &str) -> Result<(Vec<f64>, (usize, usize))> {
        match v {
            Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_)) => {
                if v.is_nd() {
                    return Err(format!("{who}: N-D inputs are not supported yet"));
                }
                let m = v.clone().matrix(&self.cancel)?;
                Ok((m.data().to_vec(), (m.rows(), m.cols())))
            }
            Value::Complex(_) => Err(format!("{who}: input must be real")),
            _ => Err(format!("{who}: input must be numeric")),
        }
    }

    fn dtm_options(&self, args: &[Value], who: &str) -> Result<Options> {
        if !args.len().is_multiple_of(2) {
            return Err(format!("{who}: name-value arguments must come in pairs"));
        }
        let mut o = Options::default();
        for pair in args.chunks(2) {
            let name = option_text(&pair[0], &self.cancel)
                .ok_or_else(|| format!("{who}: expected a parameter name"))?;
            let value = || {
                option_text(&pair[1], &self.cancel)
                    .ok_or_else(|| format!("{who}: the value of '{name}' must be text"))
            };
            match name.to_ascii_lowercase().as_str() {
                "format" => o.format = Some(value()?),
                "inputformat" => o.input_format = Some(value()?),
                "timezone" => o.zone = Some(parse_zone(&value()?, who)?),
                "convertfrom" => o.convert_from = Some(value()?.to_ascii_lowercase()),
                "pivotyear" => {
                    let (v, _) = self.dtm_numbers(&pair[1], who)?;
                    match v.as_slice() {
                        [p] if p.fract() == 0.0 && p.abs() < 1e9 => o.pivot = Some(*p as i64),
                        _ => return Err(format!("{who}: PivotYear must be an integer scalar")),
                    }
                }
                "locale" => {
                    return Err(format!("{who}: the 'Locale' option is not supported yet"));
                }
                _ => return Err(format!("{who}: unknown parameter '{name}'")),
            }
        }
        Ok(o)
    }

    /// Entry from `Engine::call`: None leaves the call to the existing code.
    pub(super) fn dtm_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let owned = NAMES.contains(&name) || calendar_duration::NAMES.contains(&name);
        if !owned && !args.iter().any(is_dt) {
            return Ok(None);
        }
        if !owned && PASS_THROUGH.contains(&name) {
            return Ok(None);
        }
        if let Some(mut outputs) = self.dtm_outputs(name, args, 1, out)? {
            return Ok(Some(if outputs.is_empty() {
                Value::Void
            } else {
                outputs.remove(0)
            }));
        }
        Ok(None)
    }

    /// `count` outputs of a call with datetime/duration arguments or a name
    /// of this module; None when the call is not this module's.
    pub(super) fn dtm_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let owned = NAMES.contains(&name) || calendar_duration::NAMES.contains(&name);
        if !owned && !args.iter().any(is_dt) {
            return Ok(None);
        }
        if !owned && PASS_THROUGH.contains(&name) {
            return Ok(None);
        }
        self.check_cancel()?;
        if let Some(values) = calendar_duration::outputs(self, name, args, count)? {
            return Ok(Some(values));
        }
        let max = match name {
            "ymd" | "hms" | "unique" => 3,
            "sort" | "min" | "max" | "weekday" => 2,
            "datevec" => 6,
            "size" => usize::MAX,
            _ => 1,
        };
        if count > max {
            return Err(format!("{name}: too many output arguments"));
        }
        let one = |v: Value| Ok(Some(vec![v]));
        match name {
            "datetime" => one(self.dtm_datetime(args)?),
            "NaT" => one(self.dtm_nat(args)?),
            "duration" => one(self.dtm_duration(args)?),
            "seconds" | "minutes" | "hours" | "days" | "years" | "milliseconds" => {
                one(self.dtm_unit(name, args)?)
            }
            "isnat" => one(self.dtm_isnat(args)?),
            "isdatetime" | "isduration" => {
                if args.len() != 1 {
                    return Err(format!("{name}: expected one input"));
                }
                one(scalar_bool(if name == "isdatetime" {
                    datetime_of(&args[0]).is_some()
                } else {
                    duration_of(&args[0]).is_some()
                }))
            }
            "posixtime" | "juliandate" | "exceltime" | "datenum" => {
                one(self.dtm_to_number(name, args)?)
            }
            "year" | "month" | "day" | "hour" | "minute" | "second" => {
                one(self.dtm_component(name, args)?)
            }
            "ymd" | "hms" | "datevec" => Ok(Some(self.dtm_split(name, args, count)?)),
            "timeofday" => one(self.dtm_timeofday(args)?),
            "weekday" => Ok(Some(self.dtm_weekday(args, count)?)),
            "datestr" => one(self.dtm_datestr(args)?),
            _ => self.dtm_collection(name, args, count, out),
        }
    }

    // ---------------------------------------------------------------- construction

    /// `datetime.empty`, `duration.empty` and `X.empty(sizes)`: an empty
    /// array of the class, 0x0 or the sizes given (one of them 0). None for
    /// any other expression, and when a variable has the class name.
    pub(in crate::engine) fn dtm_static(
        &mut self,
        e: &Expr,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let (base, member, args) = match e {
            Expr::Field(base, member) => (base, member, None),
            Expr::Index(base, args) => match base.as_ref() {
                Expr::Field(inner, member) => (inner, member, Some(args)),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let Expr::Name(class) = base.as_ref() else {
            return Ok(None);
        };
        if member != "empty"
            || !matches!(class.as_str(), "datetime" | "duration")
            || self.contains_array(class)
        {
            return Ok(None);
        }
        let who = format!("{class}.empty");
        let (rows, cols) = match args {
            None => (0, 0),
            Some(args) => {
                let values = self.function_arguments(args, None, out)?;
                let mut sizes = self.dtm_size_args(&values, &who)?;
                while sizes.len() > 2 && sizes.last() == Some(&Some(1)) {
                    sizes.pop();
                }
                match sizes.as_slice() {
                    [] => (0, 0),
                    [Some(r), Some(c)] if *r == 0 || *c == 0 => (*r, *c),
                    [Some(0)] => (0, 0),
                    [_, _] | [_] => {
                        return Err(format!("{who}: at least one dimension must be zero"));
                    }
                    _ => {
                        return Err(if class == "datetime" {
                            DATETIME_ND.into()
                        } else {
                            DURATION_ND.into()
                        });
                    }
                }
            }
        };
        Ok(Some(if class == "datetime" {
            dt_value(DatetimeArray::new(
                rows,
                cols,
                Vec::new(),
                DEFAULT_FORMAT,
                TimeZone::Unzoned,
            )?)
        } else {
            du_value(DurationArray::new(
                rows,
                cols,
                Vec::new(),
                dur_text::DEFAULT,
            )?)
        }))
    }

    fn dtm_datetime(&mut self, args: &[Value]) -> Result<Value> {
        // Positional arguments: one text/datetime argument, or leading numbers.
        let positional = match args.first() {
            None => 0,
            Some(v) if text_elements(v, &self.cancel)?.is_some() || is_dt(v) => 1,
            Some(_) => args
                .iter()
                .take_while(|v| text_elements(v, &self.cancel).ok().flatten().is_none())
                .count(),
        };
        let o = self.dtm_options(&args[positional..], "datetime")?;
        let zone = o.zone.unwrap_or_default();
        let pivot = o.pivot.unwrap_or_else(default_pivot);
        // Automatic text form found while reading text ('preserveinput').
        let mut detected: Option<&'static str> = None;
        let (mut t, date_only) = match &args[..positional] {
            [] => (
                DatetimeArray::new(1, 1, vec![calendar::now(zone)?], DEFAULT_FORMAT, zone)?,
                false,
            ),
            [v] if datetime_of(v).is_some() => {
                let t = datetime_of(v).unwrap().clone();
                if let Some(z) = o.zone {
                    let t = t.with_zone(z);
                    (t, false)
                } else {
                    (t, false)
                }
            }
            [v] if text_elements(v, &self.cancel)?.is_some() => {
                let (texts, shape) = text_elements(v, &self.cancel)?.unwrap();
                if let [Some(word)] = texts.as_slice()
                    && o.input_format.is_none()
                    && let Some(offset) = match word.to_ascii_lowercase().as_str() {
                        "now" => Some(None),
                        "today" => Some(Some(0)),
                        "tomorrow" => Some(Some(1)),
                        "yesterday" => Some(Some(-1)),
                        _ => None,
                    }
                {
                    let now = calendar::now(zone)?;
                    let (value, date_only) = match offset {
                        None => (now, false),
                        Some(k) => (
                            DateTimeValue {
                                days: now.days + k,
                                nanos: 0,
                            },
                            true,
                        ),
                    };
                    (
                        DatetimeArray::new(1, 1, vec![value], default_format(date_only), zone)?,
                        date_only,
                    )
                } else {
                    let (t, date_only, form) = self.dtm_parse_texts(
                        &texts,
                        shape,
                        o.input_format.as_deref(),
                        o.format.as_deref(),
                        pivot,
                        zone,
                    )?;
                    detected = form;
                    (t, date_only)
                }
            }
            [v] => {
                let (x, shape) = self.dtm_numbers(v, "datetime")?;
                if let Some(kind) = &o.convert_from {
                    let mut data = Vec::with_capacity(x.len());
                    for (i, &x) in x.iter().enumerate() {
                        self.dtm_check(i)?;
                        data.push(match kind.as_str() {
                            "datenum" => calendar::from_day_number(x, 0, 1)?,
                            "posixtime" => calendar::from_posix(x)?,
                            "excel" => calendar::from_excel(x, false)?,
                            "excel1904" => calendar::from_excel(x, true)?,
                            "yyyymmdd" => calendar::from_yyyymmdd(x)?,
                            "juliandate" => calendar::from_julian(x)?,
                            other => {
                                return Err(format!(
                                    "datetime: ConvertFrom '{other}' is not supported yet"
                                ));
                            }
                        });
                    }
                    let date_only = kind == "yyyymmdd";
                    (
                        DatetimeArray::new(
                            shape.0,
                            shape.1,
                            data,
                            default_format(date_only),
                            zone,
                        )?,
                        date_only,
                    )
                } else if shape.1 == 3 || shape.1 == 6 {
                    let mut data = Vec::with_capacity(shape.0);
                    for r in 0..shape.0 {
                        self.dtm_check(r)?;
                        let mut c = [0.0; 7];
                        for (k, part) in c.iter_mut().enumerate().take(shape.1) {
                            *part = x[k * shape.0 + r];
                        }
                        data.push(calendar::from_components(c)?);
                    }
                    (
                        DatetimeArray::new(shape.0, 1, data, default_format(shape.1 == 3), zone)?,
                        shape.1 == 3,
                    )
                } else {
                    return Err("datetime: a numeric input needs 'ConvertFrom' or date vectors with 3 or 6 columns".into());
                }
            }
            parts if matches!(parts.len(), 3 | 6 | 7) => {
                let mut columns = Vec::with_capacity(parts.len());
                let mut shape = (1, 1);
                for p in parts {
                    let (x, s) = self.dtm_numbers(p, "datetime")?;
                    shape = broadcast(shape, s, "datetime")?;
                    columns.push((x, s));
                }
                let n = shape.0 * shape.1;
                let mut data = Vec::with_capacity(n);
                for i in 0..n {
                    self.dtm_check(i)?;
                    let mut c = [0.0; 7];
                    for (k, (x, s)) in columns.iter().enumerate() {
                        c[k] = x[expand_index(i, shape.0, *s)];
                    }
                    data.push(calendar::from_components(c)?);
                }
                let date_only = parts.len() == 3;
                (
                    DatetimeArray::new(shape.0, shape.1, data, default_format(date_only), zone)?,
                    date_only,
                )
            }
            _ => {
                return Err(
                    "datetime: expected text, date vectors, or 3, 6 or 7 numeric components".into(),
                );
            }
        };
        let _ = date_only;
        // The InputFormat only reads the text; the display format stays the
        // default unless 'Format' is given ('preserveinput' keeps the form
        // the text was read with).
        match o.format.as_deref() {
            Some(f) if is_preserve_input(f) => {
                if let Some(f) = o.input_format.as_deref().or(detected) {
                    t = t.with_format(checked_datetime_format(f)?);
                }
            }
            Some(f) => t = t.with_format(checked_datetime_format(f)?),
            None => {}
        }
        Ok(dt_value(t))
    }

    /// Text elements read with `input_format` (unreadable ones become NaT) or
    /// the automatic forms (an unreadable element is an error).
    fn dtm_parse_texts(
        &self,
        texts: &[Option<String>],
        shape: (usize, usize),
        input_format: Option<&str>,
        display_format: Option<&str>,
        pivot: i64,
        zone: TimeZone,
    ) -> Result<(DatetimeArray, bool, Option<&'static str>)> {
        let year = current_year();
        let mut data = Vec::with_capacity(texts.len());
        let mut all_dates = true;
        // The automatic form of the first element read with one.
        let mut detected = None;
        let tokens = input_format.map(text::tokens).transpose()?;
        // Without an InputFormat, text in the display format is read first.
        let hint = match (input_format, display_format) {
            (None, Some(f)) if !is_preserve_input(f) => {
                text::tokens(&checked_datetime_format(f)?).ok()
            }
            _ => None,
        };
        for (i, t) in texts.iter().enumerate() {
            self.dtm_check(i)?;
            let Some(t) = t else {
                data.push(DateTimeValue::NAT);
                continue;
            };
            if let Some(tokens) = &tokens {
                data.push(text::parse(tokens, t, pivot, year).unwrap_or(DateTimeValue::NAT));
                continue;
            }
            if t.trim().is_empty() || t.trim() == "NaT" {
                data.push(DateTimeValue::NAT);
                continue;
            }
            if let Some(v) = hint.as_ref().and_then(|h| text::parse(h, t, pivot, year)) {
                all_dates = false;
                data.push(v);
                continue;
            }
            match text::parse_auto_form(t, pivot, year) {
                Some((v, date_only, form)) => {
                    all_dates &= date_only;
                    detected.get_or_insert(form);
                    data.push(v);
                }
                None => {
                    return Err(format!(
                        "datetime: could not recognize the date/time text '{t}' (element {}); give an 'InputFormat'",
                        i + 1
                    ));
                }
            }
        }
        // Text read with an InputFormat holds dates only when the format has
        // no time-of-day field.
        let date_only = match &tokens {
            Some(tokens) => !text::has_time_fields(tokens),
            None => all_dates,
        };
        Ok((
            DatetimeArray::new(shape.0, shape.1, data, default_format(date_only), zone)?,
            date_only,
            detected,
        ))
    }

    /// Text compared or assigned into datetime array `like`: read with its
    /// format first, then the automatic forms.
    fn dtm_text_as_datetime(
        &self,
        v: &Value,
        like: &DatetimeArray,
    ) -> Result<Option<DatetimeArray>> {
        let Some((texts, shape)) = text_elements(v, &self.cancel)? else {
            return Ok(None);
        };
        let tokens = text::tokens(like.format())?;
        let (pivot, year) = (default_pivot(), current_year());
        let mut data = Vec::with_capacity(texts.len());
        for (i, t) in texts.iter().enumerate() {
            self.dtm_check(i)?;
            let value = match t {
                None => DateTimeValue::NAT,
                Some(t) if t.trim() == "NaT" => DateTimeValue::NAT,
                Some(t) => match text::parse(&tokens, t, pivot, year) {
                    Some(v) => v,
                    None => text::parse_auto(t, pivot, year)
                        .map(|(v, _)| v)
                        .ok_or_else(|| {
                            format!(
                                "Could not convert the text '{t}' to datetime with the format '{}'",
                                like.format()
                            )
                        })?,
                },
            };
            data.push(value);
        }
        Ok(Some(like.with_data(shape.0, shape.1, data)?))
    }

    fn dtm_nat(&self, args: &[Value]) -> Result<Value> {
        let (rows, cols) = match args {
            [] => (1.0, 1.0),
            [n] => {
                let (v, _) = self.dtm_numbers(n, "NaT")?;
                match v.as_slice() {
                    [n] => (*n, *n),
                    [r, c] => (*r, *c),
                    _ => return Err(DATETIME_ND.into()),
                }
            }
            [r, c] => (
                self.dtm_numbers(r, "NaT")?
                    .0
                    .first()
                    .copied()
                    .unwrap_or(0.0),
                self.dtm_numbers(c, "NaT")?
                    .0
                    .first()
                    .copied()
                    .unwrap_or(0.0),
            ),
            _ => return Err(DATETIME_ND.into()),
        };
        let size = |x: f64| {
            if x.is_finite() && x >= 0.0 && x.fract() == 0.0 && x <= MAX_ELEMENTS as f64 {
                Ok(x as usize)
            } else {
                Err("NaT: size arguments must be nonnegative integers".to_string())
            }
        };
        let (rows, cols) = (size(rows)?, size(cols)?);
        let n = crate::datetime_array::checked_len(rows, cols, "datetime")?;
        Ok(dt_value(DatetimeArray::new(
            rows,
            cols,
            vec![DateTimeValue::NAT; n],
            DEFAULT_FORMAT,
            TimeZone::Unzoned,
        )?))
    }

    fn dtm_duration(&mut self, args: &[Value]) -> Result<Value> {
        let positional = match args.first() {
            None => 0,
            Some(v) if text_elements(v, &self.cancel)?.is_some() || is_dt(v) => 1,
            Some(_) => args
                .iter()
                .take_while(|v| text_elements(v, &self.cancel).ok().flatten().is_none())
                .count(),
        };
        let o = self.dtm_options(&args[positional..], "duration")?;
        if o.zone.is_some() || o.pivot.is_some() || o.convert_from.is_some() {
            return Err("duration: only 'Format' and 'InputFormat' options are supported".into());
        }
        let mut format = dur_text::DEFAULT;
        let d = match &args[..positional] {
            [v] if duration_of(v).is_some() => duration_of(v).unwrap().clone(),
            [v] if text_elements(v, &self.cancel)?.is_some() => {
                let (texts, shape) = text_elements(v, &self.cancel)?.unwrap();
                let given = o
                    .input_format
                    .as_deref()
                    .map(dur_text::parse_input_format)
                    .transpose()?;
                if let Some(f) = &o.input_format {
                    format = f.as_str();
                }
                let mut data = Vec::with_capacity(texts.len());
                for (i, t) in texts.iter().enumerate() {
                    self.dtm_check(i)?;
                    let Some(t) = t else {
                        data.push(f64::NAN);
                        continue;
                    };
                    let value = match given {
                        Some(f) => dur_text::parse(f, t).unwrap_or(f64::NAN),
                        None => {
                            let colons = t.matches(':').count();
                            let f = match colons {
                                2 => dur_text::parse_format("hh:mm:ss")?,
                                3 => dur_text::parse_format("dd:hh:mm:ss")?,
                                _ => dur_text::DurFormat::Unit('s'),
                            };
                            if t.trim() == "NaN" {
                                f64::NAN
                            } else {
                                dur_text::parse(f, t).ok_or_else(|| {
                                    format!("duration: could not recognize the duration text '{t}' (element {}); give an 'InputFormat'", i + 1)
                                })?
                            }
                        }
                    };
                    data.push(value);
                }
                DurationArray::new(shape.0, shape.1, data, format)?
            }
            [x] => {
                let (x, shape) = self.dtm_numbers(x, "duration")?;
                if shape.1 != 3 {
                    return Err(
                        "duration: a single numeric input must be an n-by-3 matrix [h m s]".into(),
                    );
                }
                let data = polled(&self.cancel, shape.0, |r| {
                    x[r] * 3_600_000.0 + x[shape.0 + r] * 60_000.0 + x[2 * shape.0 + r] * 1000.0
                })?;
                DurationArray::new(shape.0, 1, data, format)?
            }
            parts if matches!(parts.len(), 3 | 4) => {
                let mut columns = Vec::with_capacity(parts.len());
                let mut shape = (1, 1);
                for p in parts {
                    let (x, s) = self.dtm_numbers(p, "duration")?;
                    shape = broadcast(shape, s, "duration")?;
                    columns.push((x, s));
                }
                const MS: [f64; 4] = [3_600_000.0, 60_000.0, 1000.0, 1.0];
                let n = shape.0 * shape.1;
                let mut data = Vec::with_capacity(n);
                for i in 0..n {
                    self.dtm_check(i)?;
                    let mut total = 0.0;
                    for (k, (x, s)) in columns.iter().enumerate() {
                        total += x[expand_index(i, shape.0, *s)] * MS[k];
                    }
                    data.push(total);
                }
                DurationArray::new(shape.0, shape.1, data, format)?
            }
            _ => return Err("duration: expected H, MI, S [, MS], an n-by-3 matrix or text".into()),
        };
        let d = match &o.format {
            Some(f) => d.with_format(checked_duration_format(f)?),
            None => d,
        };
        Ok(du_value(d))
    }

    /// seconds/minutes/hours/days/years/milliseconds: numbers to durations,
    /// durations to numbers.
    fn dtm_unit(&self, name: &str, args: &[Value]) -> Result<Value> {
        let (unit, ms) = match name {
            "milliseconds" => ('s', 1.0),
            "seconds" => ('s', 1000.0),
            "minutes" => ('m', 60_000.0),
            "hours" => ('h', 3_600_000.0),
            "days" => ('d', 86_400_000.0),
            _ => ('y', dur_text::unit_ms('y')),
        };
        let [x] = args else {
            return Err(format!("{name}: expected one input"));
        };
        if let Some(d) = duration_of(x) {
            let data = polled_map(&self.cancel, d.data(), |v| v / ms)?;
            return number_matrix(d.rows(), d.cols(), data);
        }
        if datetime_of(x).is_some() {
            return Err(format!(
                "{name}: datetime input is not supported; subtract two datetimes first"
            ));
        }
        let (x, shape) = self.dtm_numbers(x, name)?;
        let data = polled_map(&self.cancel, &x, |v| v * ms)?;
        Ok(du_value(DurationArray::new(
            shape.0,
            shape.1,
            data,
            unit.to_string(),
        )?))
    }

    // ---------------------------------------------------------------- queries

    fn dtm_one_datetime<'a>(&self, args: &'a [Value], who: &str) -> Result<&'a DatetimeArray> {
        args.first()
            .and_then(datetime_of)
            .ok_or_else(|| format!("{who}: input must be a datetime array"))
    }

    fn dtm_isnat(&self, args: &[Value]) -> Result<Value> {
        let [v] = args else {
            return Err("isnat: expected one input".into());
        };
        let t = datetime_of(v).ok_or("isnat: input must be a datetime array")?;
        logical(
            t.rows(),
            t.cols(),
            polled_map(&self.cancel, t.data(), |v| u8::from(v.is_nat()))?,
        )
    }

    fn dtm_to_number(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        let t = match args.first().and_then(datetime_of) {
            Some(t) => t,
            None if name == "datenum" => {
                return Err("datenum: datetime input expected".into());
            }
            None => return Err(format!("{name}: input must be a datetime array")),
        };
        let system_1904 = match (name, args.get(1)) {
            ("exceltime", Some(v)) => match option_text(v, &self.cancel).as_deref() {
                Some("1900") => false,
                Some("1904") => true,
                _ => return Err("exceltime: the date system must be '1900' or '1904'".into()),
            },
            (_, Some(_)) => return Err(format!("{name}: too many input arguments")),
            _ => false,
        };
        let data = polled_map(&self.cancel, t.data(), |v| match name {
            "posixtime" => calendar::to_posix(v),
            "juliandate" => calendar::to_julian(v),
            "exceltime" => calendar::to_excel(v, system_1904),
            _ => calendar::to_datenum(v),
        })?;
        number_matrix(t.rows(), t.cols(), data)
    }

    fn dtm_component(&self, name: &str, args: &[Value]) -> Result<Value> {
        let t = self.dtm_one_datetime(args, name)?;
        let kind = match args.get(1) {
            Some(v) => Some(
                option_text(v, &self.cancel)
                    .ok_or_else(|| format!("{name}: the component type must be text"))?
                    .to_ascii_lowercase(),
            ),
            None => None,
        };
        if args.len() > 2 {
            return Err(format!("{name}: too many input arguments"));
        }
        use super::dates::{DAYS, MONTHS};
        let names: Option<(&[&str], bool)> = match (name, kind.as_deref()) {
            ("month", Some("name")) => Some((&MONTHS, false)),
            ("month", Some("shortname")) => Some((&MONTHS, true)),
            ("day", Some("name")) => Some((&DAYS, false)),
            ("day", Some("shortname")) => Some((&DAYS, true)),
            _ => None,
        };
        if let Some((names, short)) = names {
            let mut slots = Vec::with_capacity(t.len());
            for (i, &v) in t.data().iter().enumerate() {
                self.dtm_check(i)?;
                let text = if v.is_nat() {
                    String::new()
                } else {
                    let p = calendar::parts(v);
                    let full = if name == "month" {
                        names[p.mo - 1]
                    } else {
                        names[p.weekday - 1]
                    };
                    if short {
                        full[..3].to_string()
                    } else {
                        full.to_string()
                    }
                };
                slots.push(Arc::new(Array::Character(CharacterRow::from_text(
                    &text,
                    &self.cancel,
                )?)));
            }
            return Ok(Value::Container(Array::Cell(CellArray::from_shared(
                t.rows(),
                t.cols(),
                slots,
                &self.cancel,
            )?)));
        }
        let field = match (name, kind.as_deref()) {
            ("year", None | Some("iso" | "gregorian")) => 0,
            ("month", None | Some("monthofyear")) => 1,
            ("day", None | Some("dayofmonth")) => 2,
            ("day", Some("dayofweek")) => 6,
            ("day", Some("dayofyear")) => 7,
            ("hour", None) => 3,
            ("minute", None) => 4,
            ("second", None | Some("secondofminute")) => 5,
            (_, Some(k)) => return Err(format!("{name}: unsupported component type '{k}'")),
            _ => unreachable!("component names are listed above"),
        };
        let data = polled_map(&self.cancel, t.data(), |v| component(v, field))?;
        number_matrix(t.rows(), t.cols(), data)
    }

    fn dtm_split(&self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if name == "hms"
            && let Some(d) = args.first().and_then(duration_of)
        {
            let mut h = Vec::with_capacity(d.len());
            let mut m = Vec::with_capacity(d.len());
            let mut s = Vec::with_capacity(d.len());
            for (i, &ms) in d.data().iter().enumerate() {
                self.dtm_check(i)?;
                if !ms.is_finite() {
                    h.push(ms);
                    m.push(if ms.is_nan() { ms } else { 0.0 });
                    s.push(if ms.is_nan() { ms } else { 0.0 });
                    continue;
                }
                let hours = (ms / 3_600_000.0).trunc();
                let rest = ms - hours * 3_600_000.0;
                let minutes = (rest / 60_000.0).trunc();
                h.push(hours);
                m.push(minutes);
                s.push((rest - minutes * 60_000.0) / 1000.0);
            }
            let shape = (d.rows(), d.cols());
            return [h, m, s]
                .into_iter()
                .take(count.max(1))
                .map(|v| number_matrix(shape.0, shape.1, v))
                .collect();
        }
        let t = self.dtm_one_datetime(args, name)?;
        if args.len() > 1 {
            return Err(format!("{name}: too many input arguments"));
        }
        let fields: &[usize] = match name {
            "ymd" => &[0, 1, 2],
            "hms" => &[3, 4, 5],
            _ => &[0, 1, 2, 3, 4, 5],
        };
        if name == "datevec" && count <= 1 {
            let n = t.len();
            let mut data = vec![0.0; n * 6];
            for (i, &v) in t.data().iter().enumerate() {
                self.dtm_check(i)?;
                for k in 0..6 {
                    data[k * n + i] = component(v, k);
                }
            }
            return Ok(vec![number_matrix(n, 6, data)?]);
        }
        fields
            .iter()
            .take(count.max(1))
            .map(|&k| {
                number_matrix(
                    t.rows(),
                    t.cols(),
                    polled_map(&self.cancel, t.data(), |v| component(v, k))?,
                )
            })
            .collect()
    }

    fn dtm_timeofday(&self, args: &[Value]) -> Result<Value> {
        let t = self.dtm_one_datetime(args, "timeofday")?;
        let data = polled_map(&self.cancel, t.data(), |v| {
            if v.is_nat() {
                f64::NAN
            } else {
                calendar::nanos_to_ms(i128::from(v.nanos))
            }
        })?;
        Ok(du_value(DurationArray::new(
            t.rows(),
            t.cols(),
            data,
            dur_text::DEFAULT,
        )?))
    }

    fn dtm_weekday(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let t = self.dtm_one_datetime(args, "weekday")?;
        let numbers = polled_map(&self.cancel, t.data(), |v| component(v, 6))?;
        let mut outputs = vec![number_matrix(t.rows(), t.cols(), numbers)?];
        if count > 1 {
            let rows: Vec<String> = polled_map(&self.cancel, t.data(), |v| {
                if v.is_nat() {
                    "NaN".to_string()
                } else {
                    super::dates::DAYS[calendar::parts(v).weekday - 1][..3].to_string()
                }
            })?;
            outputs.push(self.dtm_char_rows(&rows)?);
        }
        Ok(outputs)
    }

    fn dtm_datestr(&mut self, args: &[Value]) -> Result<Value> {
        let t = self.dtm_one_datetime(args, "datestr")?;
        let serials = polled_map(&self.cancel, t.data(), calendar::to_datenum)?;
        let mut rest = vec![number_matrix(t.len(), 1, serials)?];
        rest.extend(args[1..].iter().cloned());
        self.datex_call("datestr", &rest)?
            .ok_or_else(|| "datestr is not available".to_string())
    }

    /// Rows of text as a char matrix padded with blanks.
    fn dtm_char_rows(&self, rows: &[String]) -> Result<Value> {
        let units: Vec<Vec<u16>> = polled(&self.cancel, rows.len(), |r| {
            rows[r].encode_utf16().collect()
        })?;
        let width = units.iter().map(Vec::len).max().unwrap_or(0);
        let n = units.len();
        let mut data = vec![u16::from(b' '); n * width];
        for (r, row) in units.iter().enumerate() {
            self.dtm_check(r)?;
            for (c, &u) in row.iter().enumerate() {
                data[c * n + r] = u;
            }
        }
        Ok(Value::Container(Array::Character(
            CharacterRow::from_column_major(if n == 0 { 0 } else { n }, width, data, &self.cancel)?,
        )))
    }
}

/// Numeric component `field` of a value: 0 year, 1 month, 2 day, 3 hour,
/// 4 minute, 5 second (with fraction), 6 weekday, 7 day of year. NaN for NaT.
fn component(v: DateTimeValue, field: usize) -> f64 {
    if v.is_nat() {
        return f64::NAN;
    }
    let p = calendar::parts(v);
    match field {
        0 => p.y as f64,
        1 => p.mo as f64,
        2 => f64::from(p.d),
        3 => f64::from(p.h),
        4 => f64::from(p.mi),
        5 => f64::from(p.s) + f64::from(p.ns) / 1e9,
        6 => p.weekday as f64,
        _ => f64::from(p.doy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Token;
    use std::sync::atomic::Ordering;

    type Probe = fn(&mut Engine, &Value, &Value) -> Result<Value>;

    fn first(v: Result<Vec<Value>>) -> Result<Value> {
        v.map(|mut v| v.remove(0))
    }

    /// A row of ones as long as `v` (built without polling).
    fn ones_like(v: &Value) -> Value {
        let n = v.dims().iter().product();
        number_matrix(1, n, vec![1.0; n]).unwrap()
    }

    fn collection(e: &mut Engine, name: &str, args: &[Value]) -> Result<Value> {
        let mut out = Execution::default();
        e.dtm_collection(name, args, 1, &mut out)
            .map(|v| v.expect("claimed").remove(0))
    }

    /// Every elementwise path polls the flag inside its loop. The paths are
    /// called directly, past the entry checks of `eval` and `dtm_outputs`:
    /// with the flag set before the call each stops with the cancel error,
    /// without it each runs to its end.
    #[test]
    fn elementwise_loops_poll_the_cancel_flag() {
        let mut e = Engine::default();
        // One stride: every loop polls at least once (cell and string
        // arrays hold at most 65536 elements).
        let n = CANCEL_STRIDE;
        let t = dt_value(
            DatetimeArray::new(
                1,
                n,
                (0..n)
                    .map(|i| DateTimeValue {
                        days: 738_000 + i as i64,
                        nanos: 0,
                    })
                    .collect(),
                DEFAULT_FORMAT,
                TimeZone::Unzoned,
            )
            .unwrap(),
        );
        let d = du_value(
            DurationArray::new(1, n, (0..n).map(|i| i as f64).collect(), dur_text::DEFAULT)
                .unwrap(),
        );
        let probes: Vec<(&str, Probe)> = vec![
            ("posixtime", |e, t, _| {
                e.dtm_to_number("posixtime", std::slice::from_ref(t))
            }),
            ("year", |e, t, _| {
                e.dtm_component("year", std::slice::from_ref(t))
            }),
            ("month names", |e, t, _| {
                e.dtm_component("month", &[t.clone(), Value::Text("name".into())])
            }),
            ("ymd", |e, t, _| {
                first(e.dtm_split("ymd", std::slice::from_ref(t), 3))
            }),
            ("datevec", |e, t, _| {
                first(e.dtm_split("datevec", std::slice::from_ref(t), 1))
            }),
            ("hms", |e, _, d| {
                first(e.dtm_split("hms", std::slice::from_ref(d), 3))
            }),
            ("timeofday", |e, t, _| {
                e.dtm_timeofday(std::slice::from_ref(t))
            }),
            // One output: a 65536-row char matrix of day names exceeds the char limit.
            ("weekday", |e, t, _| {
                first(e.dtm_weekday(std::slice::from_ref(t), 1))
            }),
            ("isnat", |e, t, _| e.dtm_isnat(std::slice::from_ref(t))),
            ("hours(d)", |e, _, d| {
                e.dtm_unit("hours", std::slice::from_ref(d))
            }),
            ("hours(x)", |e, _, d| e.dtm_unit("hours", &[ones_like(d)])),
            ("t.Hour", |e, t, _| match t {
                Value::Container(a) => e.dtm_field(a, "Hour"),
                _ => unreachable!(),
            }),
            ("string", |e, t, _| {
                collection(e, "string", std::slice::from_ref(t))
            }),
            ("cellstr", |e, t, _| {
                collection(e, "cellstr", std::slice::from_ref(t))
            }),
            ("abs", |e, _, d| {
                collection(e, "abs", std::slice::from_ref(d))
            }),
            ("fliplr", |e, t, _| {
                collection(e, "fliplr", std::slice::from_ref(t))
            }),
            ("isequal", |e, t, _| {
                collection(e, "isequal", &[t.clone(), t.clone()])
            }),
            ("median", |e, t, _| {
                collection(e, "median", std::slice::from_ref(t))
            }),
            ("max all", |e, _, d| {
                collection(
                    e,
                    "max",
                    &[
                        d.clone(),
                        number_matrix(0, 0, vec![])?,
                        Value::Text("all".into()),
                    ],
                )
            }),
            ("t - t", |e, t, _| e.dtm_binary(&Token::Minus, t, t)),
            ("t + d", |e, t, d| e.dtm_binary(&Token::Plus, t, d)),
            ("d ./ d", |e, _, d| e.dtm_binary(&Token::DotSlash, d, d)),
            ("d + d", |e, _, d| e.dtm_binary(&Token::Plus, d, d)),
            ("t < t", |e, t, _| e.dtm_compare(&Token::Less, t, t)),
            ("-d", |e, _, d| e.dtm_unary(true, d.clone())),
            ("t'", |e, t, _| e.dtm_transpose(t)),
            ("[d, d]", |e, _, d| {
                e.dtm_concat(vec![vec![d.clone(), d.clone()]])
            }),
            ("[d, x]", |e, _, d| {
                e.dtm_concat(vec![vec![d.clone(), ones_like(d)]])
            }),
            ("d1:step:d2", |e, _, _| {
                let at =
                    |ms: f64| du_value(DurationArray::new(1, 1, vec![ms], "hh:mm:ss").unwrap());
                e.dtm_range(at(0.0), Some(at(1.0)), at((CANCEL_STRIDE - 1) as f64))
            }),
        ];
        for (name, probe) in probes {
            e.cancel.store(false, Ordering::Relaxed);
            if let Err(err) = probe(&mut e, &t, &d) {
                panic!("{name} without a cancel flag: {err}");
            }
            e.cancel.store(true, Ordering::Relaxed);
            assert_eq!(
                probe(&mut e, &t, &d).map(|_| ()),
                Err(CANCELLED.to_string()),
                "{name}"
            );
        }
        e.cancel.store(false, Ordering::Relaxed);
        // Text elements of a long string array poll too.
        let strings = collection(&mut e, "string", std::slice::from_ref(&t)).unwrap();
        e.cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            text_elements(&strings, &e.cancel).map(|_| ()),
            Err(CANCELLED.to_string())
        );
    }

    /// `polled` checks the flag before every block: a cancellation during
    /// one block stops the loop before the next block starts.
    #[test]
    fn polled_stops_at_the_next_block_after_a_cancellation() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut calls = 0usize;
        let result = polled(&cancel, 5 * CANCEL_STRIDE, |i| {
            calls += 1;
            if i == CANCEL_STRIDE + 10 {
                cancel.store(true, Ordering::Relaxed);
            }
            i
        });
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert_eq!(calls, 2 * CANCEL_STRIDE);
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(polled(&cancel, 3, |i| i * 2).unwrap(), vec![0, 2, 4]);
    }
}
