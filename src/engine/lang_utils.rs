//! Language utilities: cell2mat, cellstr, deal, structfun, strtok, assert,
//! warning, eval, rat and rats. The corpus replayed by tests/fn_lang.rs is the
//! reference for their console text and error messages.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, FunctionHandle, LogicalMatrix,
    Matrix, Result, StructArray,
    script_containers::MAX_CHARACTER_UNITS,
    syntax::{self, Expr, StatementKind},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const NAMES: &[&str] = &[
    "cell2mat",
    "cellstr",
    "deal",
    "structfun",
    "strtok",
    "assert",
    "warning",
    "eval",
    "rat",
    "rats",
];
/// strtok's default delimiters: char(9:13) and space.
const WHITESPACE: [u16; 6] = [9, 10, 11, 12, 13, 32];

/// Constant-time ASCII lookup, logarithmic lookup for other UTF-16 units.
/// Built once per call and shared by every row of a cellstr input.
struct Delimiters {
    ascii: [bool; 128],
    other: BTreeSet<u16>,
}
impl Delimiters {
    fn contains(&self, unit: u16) -> bool {
        self.ascii
            .get(usize::from(unit))
            .copied()
            .unwrap_or_else(|| self.other.contains(&unit))
    }
}
const MAX_WARNING_IDS: usize = 1024;
/// A finite double needs far fewer terms; this only bounds the loop.
const MAX_RAT_TERMS: usize = 64;
/// Statements eval runs as commands rather than as a value-producing expression.
const EVAL_COMMANDS: &[&str] = &[
    "fprintf",
    "axes",
    "patch",
    "annotation",
    "legend",
    "plot",
    "semilogx",
    "semilogy",
    "loglog",
    "scatter",
    "bar",
    "stairs",
    "stem",
    "set",
    "title",
    "xlabel",
    "ylabel",
    "clear",
    "run",
    "help",
    "doc",
    "format",
    "tic",
    "toc",
    "save",
    "load",
    "writematrix",
    "savefig",
    "openfig",
    "figure",
    "saveas",
    "exportgraphics",
    "hold",
    "grid",
    "clf",
    "close",
    "tiledlayout",
    "nexttile",
    "clc",
    "whos",
];

/// Session state of `warning('off'|'on', 'all'|id)`.
#[derive(Clone, Debug, Default)]
pub(super) struct WarningState {
    all_off: bool,
    /// Identifiers set after the last 'all'; true is on.
    ids: BTreeMap<String, bool>,
}
impl WarningState {
    fn enabled(&self, identifier: &str) -> bool {
        self.ids.get(identifier).copied().unwrap_or(!self.all_off)
    }
}

/// Functions with more than one output, for `eval_outputs`.
pub(super) fn multiple_outputs(name: &str) -> bool {
    matches!(name, "deal" | "structfun" | "strtok" | "rat")
}

/// `Pkg:id` style: colon-separated nonempty word components.
fn identifier_like(text: &str) -> bool {
    text.contains(':')
        && text.split(':').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })
}

/// MATLAB eps(x): the spacing of doubles at |x|.
fn spacing(x: f64) -> f64 {
    let a = x.abs();
    if !a.is_finite() {
        return f64::NAN;
    }
    let next = f64::from_bits(a.to_bits() + 1);
    if next.is_finite() {
        next - a
    } else {
        a - f64::from_bits(a.to_bits() - 1)
    }
}

/// int2str of an integer-valued double.
fn integer_text(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else if v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.0}")
    }
}

/// Continued fraction `a0 + 1/(a1 + 1/(a2 + ...))` of `value` with nearest-integer
/// partial quotients, and its last convergent `p/q`. The convergents follow the
/// recurrence `p_k = a_k p_(k-1) + p_(k-2)`, `q_k = a_k q_(k-1) + q_(k-2)` from
/// `p_(-1)/q_(-1) = 1/0` and `p_(-2)/q_(-2) = 0/1` (NIST DLMF sec. 3.10; Hardy &
/// Wright, *An Introduction to the Theory of Numbers*, ch. X), which holds for
/// negative quotients too. Expansion stops at the first convergent within `tol`
/// (at least eps(value)) of `value`, an exact remainder, or `MAX_RAT_TERMS` quotients. Each quotient
/// carries its sign bit, so `round(-0.2)` prints as `-0`. Infinity is `+-1/0`
/// and NaN `0/0`.
fn continued_fraction(value: f64, tol: f64) -> (Vec<(f64, bool)>, f64, f64) {
    if value.is_nan() {
        return (vec![(value, false)], 0., 0.);
    }
    if value.is_infinite() {
        return (vec![(value, value < 0.)], value.signum(), 0.);
    }
    let mut terms = Vec::new();
    let (mut p, mut q) = (1., 0.);
    let (mut p_before, mut q_before) = (0., 1.);
    // No convergent can be closer to `value` than the spacing of doubles there.
    let tol = tol.max(spacing(value));
    let mut rest = value;
    loop {
        let a = rest.round();
        terms.push((a, a.is_sign_negative()));
        (p, p_before) = (a * p + p_before, p);
        (q, q_before) = (a * q + q_before, q);
        // `rest - a` is exact: `a` is the integer nearest to `rest`.
        let remainder = rest - a;
        if remainder == 0. || (value - p / q).abs() <= tol || terms.len() == MAX_RAT_TERMS {
            break;
        }
        rest = 1. / remainder;
        if !rest.is_finite() {
            break;
        }
    }
    (terms, p, q)
}

/// The continued fraction as text: `3 + 1/(7 + 1/(16))` for pi.
fn rat_text(value: f64, tol: f64) -> String {
    let (terms, _, _) = continued_fraction(value, tol);
    let term = |&(a, negative): &(f64, bool)| {
        if a == 0. && negative {
            "-0".to_owned()
        } else {
            integer_text(a)
        }
    };
    let mut text = term(&terms[0]);
    for a in &terms[1..] {
        text.push_str(" + 1/(");
        text.push_str(&term(a));
    }
    text.push_str(&")".repeat(terms.len() - 1));
    text
}

/// Numerator and denominator with the sign on the numerator.
fn rat_pair(value: f64, tol: f64) -> (f64, f64) {
    let (_, p, q) = continued_fraction(value, tol);
    if q < 0. { (-p, -q) } else { (p, q) }
}

/// Rows of text as a right-padded char matrix (char(...) of several rows).
fn padded_rows(rows: &[Vec<u16>], cancel: &std::sync::atomic::AtomicBool) -> Result<Array> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if rows.is_empty() {
        return Ok(CharacterRow::from_column_major(0, 0, Vec::new(), cancel)?.into());
    }
    let total = rows.len().saturating_mul(width);
    if total > MAX_CHARACTER_UNITS {
        return Err(format!(
            "Character result exceeds {MAX_CHARACTER_UNITS} characters"
        ));
    }
    let mut units = Vec::with_capacity(total);
    for column in 0..width {
        for row in rows {
            units.push(row.get(column).copied().unwrap_or(u16::from(b' ')));
        }
    }
    Ok(CharacterRow::from_column_major(rows.len(), width, units, cancel)?.into())
}

impl Engine {
    /// Single-output dispatch from `call`; `None` when `name` is not ours.
    pub(super) fn lang_utils_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        self.check_cancel()?;
        Ok(Some(match name {
            "assert" => {
                self.langx_assert(args)?;
                Value::Void
            }
            "warning" => {
                self.langx_warning(args, out)?;
                Value::Void
            }
            "eval" => self.langx_eval(args, true, out)?,
            "cell2mat" => self.langx_cell2mat(args)?,
            "cellstr" => self.langx_cellstr(args)?,
            "rats" => self.langx_rats(args)?,
            _ => self
                .lang_utils_outputs(name, args, 1, out)?
                .into_iter()
                .next()
                .unwrap_or(Value::Void),
        }))
    }
    /// `[a, b, ...] = name(...)` for the functions in `multiple_outputs`.
    pub(super) fn lang_utils_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        match name {
            "deal" => Self::langx_deal(args, count),
            "structfun" => self.langx_structfun(args, count, out),
            "strtok" => self.langx_strtok(args, count),
            "rat" => self.langx_rat(args, count),
            _ => Err(format!("{name} does not support {count} outputs")),
        }
    }

    /// Character data of a text argument, any shape.
    fn langx_chars(&self, value: &Value) -> Result<Option<CharacterRow>> {
        Ok(match value {
            Value::Text(s) => Some(CharacterRow::from_text(s, &self.cancel)?),
            Value::Container(Array::Character(c)) => Some(c.clone()),
            _ => None,
        })
    }
    fn langx_string(&self, value: &Value, what: &str) -> Result<String> {
        match self.langx_chars(value)? {
            Some(c) if c.rows() <= 1 => c.to_utf8(&self.cancel),
            _ => Err(format!("{what} must be a character vector")),
        }
    }
    fn langx_char_row(&self, units: &[u16]) -> Result<Array> {
        Ok(CharacterRow::from_column_major(1, units.len(), units.to_vec(), &self.cancel)?.into())
    }
    fn langx_empty_char(&self) -> Result<Array> {
        Ok(CharacterRow::from_column_major(0, 0, Vec::new(), &self.cancel)?.into())
    }

    fn langx_cell2mat(&mut self, args: &[Value]) -> Result<Value> {
        let [value] = args else {
            return Err("cell2mat expects one argument".into());
        };
        let Value::Container(Array::Cell(cells)) = value else {
            return Err("cell2mat: input must be a cell array".into());
        };
        if cells.is_empty() {
            return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
        }
        // The reference inspects the first element for cells and objects, then
        // requires one class throughout.
        let first = cells.get_linear(0)?;
        if matches!(
            first.as_ref(),
            Array::Cell(_) | Array::Handle(_) | Array::Graphics(_)
        ) {
            return Err(
                "CELL2MAT does not support cell arrays containing cell arrays or objects.".into(),
            );
        }
        let class = first.class();
        for index in 1..cells.len() {
            if index % 1024 == 0 {
                self.check_cancel()?;
            }
            if cells.get_linear(index)?.class() != class {
                return Err(
                    "All contents of the input cell array must be of the same data type.".into(),
                );
            }
        }
        let (rows, cols) = (cells.rows(), cells.cols());
        let mut joined = Vec::with_capacity(rows);
        for row in 0..rows {
            self.check_cancel()?;
            let mut parts = Vec::with_capacity(cols);
            for col in 0..cols {
                parts.push(Value::from(
                    cells.get_linear(col * rows + row)?.as_ref().clone(),
                ));
            }
            joined.push(
                self.matbuild_call("horzcat", &parts)?
                    .ok_or("horzcat is unavailable")?,
            );
        }
        Ok(self
            .matbuild_call("vertcat", &joined)?
            .ok_or("vertcat is unavailable")?)
    }

    fn langx_cellstr(&self, args: &[Value]) -> Result<Value> {
        let [value] = args else {
            return Err("cellstr expects one argument".into());
        };
        if let Value::Container(Array::Cell(cells)) = value {
            for index in 0..cells.len() {
                if !matches!(cells.get_linear(index)?.as_ref(), Array::Character(_)) {
                    return Err("Input must be a string.".into());
                }
            }
            return Ok(value.clone());
        }
        let Some(chars) = self.langx_chars(value)? else {
            return Err("Input must be a string.".into());
        };
        if chars.is_empty() {
            let slots = vec![Arc::new(self.langx_empty_char()?)];
            return Ok(Value::Container(
                CellArray::from_shared(1, 1, slots, &self.cancel)?.into(),
            ));
        }
        let (rows, cols) = (chars.rows(), chars.cols());
        let units = chars.units();
        let mut slots = Vec::with_capacity(rows);
        for row in 0..rows {
            self.check_cancel()?;
            let line: Vec<u16> = (0..cols).map(|col| units[col * rows + row]).collect();
            // deblank: trailing whitespace and nulls go.
            let kept = line
                .iter()
                .rposition(|&u| u != 0 && !WHITESPACE.contains(&u))
                .map_or(0, |last| last + 1);
            slots.push(Arc::new(if kept == 0 {
                self.langx_empty_char()?
            } else {
                self.langx_char_row(&line[..kept])?
            }));
        }
        Ok(Value::Container(
            CellArray::from_shared(rows, 1, slots, &self.cancel)?.into(),
        ))
    }

    fn langx_deal(args: &[Value], count: usize) -> Result<Vec<Value>> {
        let count = count.max(1);
        match args {
            [] => Err("Not enough input arguments.".into()),
            [value] => Ok(vec![value.clone(); count]),
            _ if args.len() == count => Ok(args.to_vec()),
            _ => Err("The number of outputs should match the number of inputs.".into()),
        }
    }

    fn langx_structfun(
        &mut self,
        args: &[Value],
        requested: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let [function, value, options @ ..] = args else {
            return Err("Not enough input arguments.".into());
        };
        let Value::Container(Array::Struct(fields)) = value else {
            return Err("Inputs to STRUCTFUN must be scalar structures.".into());
        };
        if fields.len() != 1 {
            return Err("Inputs to STRUCTFUN must be scalar structures.".into());
        }
        let handle = self.langx_handle(function, "structfun")?;
        if options.len() % 2 != 0 {
            return Err("structfun: options must be name/value pairs".into());
        }
        let mut uniform = true;
        let mut handler = None;
        for pair in options.chunks(2) {
            let key = self.langx_string(&pair[0], "structfun: an option name")?;
            if key.eq_ignore_ascii_case("UniformOutput") {
                uniform = match &pair[1] {
                    Value::Logical(m) if m.len() == 1 => m.data()[0] != 0,
                    Value::Matrix(m) if m.len() == 1 => m.data()[0] != 0.,
                    _ => {
                        return Err(
                            "structfun: UniformOutput must be a logical or numeric scalar".into(),
                        );
                    }
                };
            } else if key.eq_ignore_ascii_case("ErrorHandler") {
                handler = Some(self.langx_handle(&pair[1], "structfun")?);
            } else {
                return Err(format!("structfun: unknown option '{key}'"));
            }
        }
        let outputs = requested.max(1);
        let names = fields.field_names().to_vec();
        let mut results: Vec<Vec<Arc<Array>>> = (0..outputs)
            .map(|_| Vec::with_capacity(names.len()))
            .collect();
        let mut silent = 0;
        for index in 0..names.len() {
            self.check_cancel()?;
            let field = Value::from(fields.get(index, 0)?.as_ref().clone());
            let failure = self.functions.failure.clone();
            let file_diagnostic = self.file_diagnostic.clone();
            let values = match self.call_handle(&handle, vec![field.clone()], requested, None, out)
            {
                Err(message) if handler.is_some() && self.is_catchable(&message) => {
                    self.functions.failure = failure;
                    self.file_diagnostic = file_diagnostic;
                    let error = self.langx_error_info(&message, index)?;
                    let handler = handler.as_ref().expect("checked above");
                    self.call_handle(handler, vec![error, field], requested, None, out)?
                }
                other => other?,
            };
            if values.is_empty() && requested <= 1 {
                silent += 1;
                continue;
            }
            if values.len() < outputs {
                return Err("Too many output arguments.".into());
            }
            for (k, value) in values.into_iter().take(outputs).enumerate() {
                let array = value.array(&self.cancel)?;
                if uniform {
                    Self::langx_check_uniform(&results[k], &array, index, k)?;
                }
                results[k].push(Arc::new(array));
            }
        }
        if silent != 0 {
            if silent != names.len() {
                return Err("The function returned a value for some fields only.".into());
            }
            return Ok(vec![Value::Void]);
        }
        let mut finished = Vec::with_capacity(outputs);
        for column in results {
            self.check_cancel()?;
            finished.push(if uniform {
                self.langx_uniform_column(&column)?
            } else {
                Value::Container(
                    StructArray::from_slots(1, 1, names.clone(), column, &self.cancel)?.into(),
                )
            });
        }
        self.charge_function_outputs(&finished)?;
        Ok(finished)
    }
    fn langx_handle(&self, value: &Value, name: &str) -> Result<FunctionHandle> {
        match value {
            Value::Container(Array::Handle(handle)) => Ok(handle.clone()),
            other => match self.langx_chars(other)? {
                Some(text) => {
                    let text = text.to_utf8(&self.cancel)?;
                    if !crate::files::valid_name(&text) {
                        return Err(format!("{name}: expected a function handle"));
                    }
                    Ok(FunctionHandle::named(&text))
                }
                None => Err(format!("{name}: expected a function handle")),
            },
        }
    }
    /// The struct an ErrorHandler receives: message, identifier, index.
    fn langx_error_info(&mut self, message: &str, index: usize) -> Result<Value> {
        let caught = self.error_value(message)?;
        let Array::Struct(caught) = caught else {
            return Err("structfun: the caught error is not a struct".into());
        };
        let field = |name: &str| -> Result<Arc<Array>> {
            let index = caught
                .field_index(name)
                .ok_or("error struct field missing")?;
            Ok(caught.get(index, 0)?.clone())
        };
        let slots = vec![
            field("message")?,
            field("identifier")?,
            Arc::new(Matrix::scalar((index + 1) as f64).into()),
        ];
        Ok(Value::Container(
            StructArray::from_slots(
                1,
                1,
                vec!["message".into(), "identifier".into(), "index".into()],
                slots,
                &self.cancel,
            )?
            .into(),
        ))
    }
    /// The reference's uniform-output rules, as cellfun/arrayfun apply them.
    fn langx_check_uniform(
        previous: &[Arc<Array>],
        array: &Array,
        index: usize,
        output: usize,
    ) -> Result<()> {
        if !matches!(
            array,
            Array::Numeric(_) | Array::Logical(_) | Array::Complex(_) | Array::Character(_)
        ) {
            return Err(format!(
                "{} output type is not supported. Set 'UniformOutput' to false.",
                array.class()
            ));
        }
        if array.len() != 1 {
            return Err(format!(
                "Non-scalar in Uniform output, at index {}, output {}.\nSet 'UniformOutput' to false.",
                index + 1,
                output + 1
            ));
        }
        if let Some(first) = previous.first()
            && first.class() != array.class()
        {
            return Err(format!(
                "Mismatch in type of outputs, at index {}, output {} ({} versus {}).\nSet 'UniformOutput' to false.",
                index + 1,
                output + 1,
                first.class(),
                array.class()
            ));
        }
        Ok(())
    }
    /// One checked scalar per field, as a column.
    fn langx_uniform_column(&self, column: &[Arc<Array>]) -> Result<Value> {
        let rows = column.len();
        let Some(first) = column.first() else {
            return Ok(Value::Matrix(Matrix::from_column_vec(0, 1, Vec::new())?));
        };
        Ok(match first.as_ref() {
            Array::Logical(_) => {
                let data = column
                    .iter()
                    .map(|a| match a.as_ref() {
                        Array::Logical(m) => m.data()[0],
                        _ => 0,
                    })
                    .collect();
                Value::Logical(LogicalMatrix::from_column_vec(rows, 1, data, &self.cancel)?)
            }
            Array::Character(_) => {
                let units = column
                    .iter()
                    .map(|a| match a.as_ref() {
                        Array::Character(c) => c.units()[0],
                        _ => 0,
                    })
                    .collect();
                Value::Container(
                    CharacterRow::from_column_major(rows, 1, units, &self.cancel)?.into(),
                )
            }
            _ if column
                .iter()
                .any(|a| matches!(a.as_ref(), Array::Complex(_))) =>
            {
                let data = column
                    .iter()
                    .map(|a| match a.as_ref() {
                        Array::Complex(m) => m.data()[0],
                        Array::Numeric(m) => Complex64::new(m.data()[0], 0.),
                        _ => Complex64::new(0., 0.),
                    })
                    .collect();
                Value::Complex(ComplexMatrix::from_column_vec(rows, 1, data, &self.cancel)?)
            }
            _ => {
                let data = column
                    .iter()
                    .map(|a| match a.as_ref() {
                        Array::Numeric(m) => m.data()[0],
                        _ => 0.,
                    })
                    .collect();
                Value::Matrix(Matrix::from_column_vec(rows, 1, data)?)
            }
        })
    }

    fn langx_strtok(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if count > 2 {
            return Err("Too many output arguments.".into());
        }
        let (input, delimiters) = match args {
            [input] => (input, WHITESPACE.to_vec()),
            [input, delimiters] => match self.langx_chars(delimiters)? {
                Some(c) => (input, c.units().to_vec()),
                None => return Err("strtok: delimiters must be characters".into()),
            },
            _ => return Err("strtok expects one or two arguments".into()),
        };
        let delimiters = self.langx_delimiters(&delimiters)?;
        if let Value::Container(Array::Cell(cells)) = input {
            let mut tokens = Vec::with_capacity(cells.len());
            let mut remainders = Vec::with_capacity(cells.len());
            for index in 0..cells.len() {
                self.check_cancel()?;
                let Array::Character(c) = cells.get_linear(index)?.as_ref() else {
                    return Err("Input must be a string or cell array of strings.".into());
                };
                let (token, remainder) = self.langx_token(c, &delimiters)?;
                tokens.push(Arc::new(token));
                remainders.push(Arc::new(remainder));
            }
            let (rows, cols) = (cells.rows(), cells.cols());
            let cell = |slots: Vec<Arc<Array>>| -> Result<Value> {
                Ok(Value::Container(if slots.is_empty() {
                    CellArray::empty(rows, cols, &self.cancel)?.into()
                } else {
                    CellArray::from_shared(rows, cols, slots, &self.cancel)?.into()
                }))
            };
            return Ok(vec![cell(tokens)?, cell(remainders)?]);
        }
        let Some(chars) = self.langx_chars(input)? else {
            return Err("Input must be a string or cell array of strings.".into());
        };
        let (token, remainder) = self.langx_token(&chars, &delimiters)?;
        Ok(vec![Value::from(token), Value::from(remainder)])
    }
    /// Tokenize one character row: skip leading delimiters, the token runs
    /// to the next delimiter, the remainder keeps that delimiter.
    fn langx_token(&self, chars: &CharacterRow, delimiters: &Delimiters) -> Result<(Array, Array)> {
        if chars.rows() > 1 {
            return Err("strtok: input must be a character vector".into());
        }
        let units = chars.units();
        let start = self.langx_delimiter_span(units, 0, delimiters, true)?;
        if start == units.len() {
            return Ok((self.langx_empty_char()?, self.langx_empty_char()?));
        }
        let finish = self.langx_delimiter_span(units, start, delimiters, false)?;
        Ok((
            self.langx_char_row(&units[start..finish])?,
            self.langx_char_row(&units[finish..])?,
        ))
    }

    fn langx_delimiter_span(
        &self,
        units: &[u16],
        mut index: usize,
        delimiters: &Delimiters,
        skip_delimiters: bool,
    ) -> Result<usize> {
        while index < units.len() {
            if index.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            if delimiters.contains(units[index]) != skip_delimiters {
                break;
            }
            index += 1;
        }
        Ok(index)
    }

    fn langx_delimiters(&self, units: &[u16]) -> Result<Delimiters> {
        let mut lookup = Delimiters {
            ascii: [false; 128],
            other: BTreeSet::new(),
        };
        for (index, &unit) in units.iter().enumerate() {
            if index % 1024 == 0 {
                self.check_cancel()?;
            }
            if let Some(present) = lookup.ascii.get_mut(usize::from(unit)) {
                *present = true;
            } else {
                lookup.other.insert(unit);
            }
        }
        Ok(lookup)
    }

    fn langx_assert(&mut self, args: &[Value]) -> Result<()> {
        let Some((condition, message)) = args.split_first() else {
            return Err("Not enough input arguments.".into());
        };
        let holds = match condition {
            Value::Logical(m) if m.len() == 1 => m.data()[0] != 0,
            Value::Matrix(m) if m.len() == 1 && m.data()[0].is_nan() => {
                return Err("NaN's cannot be converted to logicals.".into());
            }
            Value::Matrix(m) if m.len() == 1 => m.data()[0] != 0.,
            _ => {
                return Err("The condition input argument must be a scalar logical.".into());
            }
        };
        if holds {
            return Ok(());
        }
        if !message.is_empty() {
            // error() formats the message and raises it with its identifier;
            // an empty message falls through to the default text.
            self.error_call("error", message)?;
        }
        Err(self.raise("Assertion failed.".into(), "MATLAB:assertion:failed".into()))
    }

    fn langx_warning(&mut self, args: &[Value], out: &mut Execution) -> Result<()> {
        let Some(first) = args.first() else {
            return Err("warning: querying the warning state is not supported yet".into());
        };
        let text = self.langx_string(first, "warning: the message")?;
        let state = text.to_ascii_lowercase();
        if args.len() <= 2 && matches!(state.as_str(), "on" | "off") {
            let enable = state == "on";
            let target = match args.get(1) {
                Some(id) => self.langx_string(id, "warning: the message identifier")?,
                None => "all".into(),
            };
            let warnings = &mut self.lang_warnings;
            if target.eq_ignore_ascii_case("all") {
                warnings.all_off = !enable;
                warnings.ids.clear();
            } else {
                if !warnings.ids.contains_key(&target) && warnings.ids.len() >= MAX_WARNING_IDS {
                    return Err(format!(
                        "warning: at most {MAX_WARNING_IDS} identifiers can have their own state"
                    ));
                }
                warnings.ids.insert(target, enable);
            }
            return Ok(());
        }
        if args.len() <= 2
            && matches!(
                state.as_str(),
                "query" | "error" | "backtrace" | "verbose" | "debug" | "once" | "always"
            )
        {
            return Err(format!("warning('{text}', ...) is not supported yet"));
        }
        let identifier = if args.len() >= 2 && identifier_like(&text) {
            text.clone()
        } else {
            String::new()
        };
        let message = if args.len() == 1 {
            text
        } else {
            self.langx_format(args)?
        };
        self.functions.lng.last_warning = (message.clone(), identifier.clone());
        if message.is_empty() || !self.lang_warnings.enabled(&identifier) {
            return Ok(());
        }
        self.emit(out, format!("Warning: {message}"))
    }
    /// The message error() would raise for these arguments, without raising it.
    fn langx_format(&mut self, args: &[Value]) -> Result<String> {
        let pending = self.raised.take();
        let result = self.error_call("error", args);
        let raised = self.raised.take().is_some();
        self.raised = pending;
        match result {
            Err(message) if raised => Ok(message),
            Err(message) => Err(message.replacen("error:", "warning:", 1)),
            Ok(_) => Ok(String::new()),
        }
    }

    pub(super) fn langx_eval(
        &mut self,
        args: &[Value],
        output_requested: bool,
        out: &mut Execution,
    ) -> Result<Value> {
        let (code, fallback) = match args {
            [code] => (code, None),
            [code, fallback] => (code, Some(fallback)),
            _ => return Err("eval expects one or two arguments".into()),
        };
        let code = self.langx_string(code, "eval: the expression")?;
        let Some(fallback) = fallback else {
            return self.langx_run(&code, output_requested, out);
        };
        let fallback = self.langx_string(fallback, "eval: the catch expression")?;
        let failure = self.functions.failure.clone();
        let file_diagnostic = self.file_diagnostic.clone();
        match self.langx_run(&code, output_requested, out) {
            Err(message) if self.is_catchable(&message) => {
                self.functions.failure = failure;
                self.file_diagnostic = file_diagnostic;
                self.raised = None;
                self.functions.lng.thrown_stack = None;
                self.functions.lng.thrown_object = None;
                self.langx_run(&fallback, output_requested, out)
            }
            other => other,
        }
    }
    /// Run eval text in the current workspace. A lone expression statement
    /// yields its value only when an output is requested; otherwise the text
    /// controls display, independently of the outer statement's semicolon.
    fn langx_run(
        &mut self,
        code: &str,
        output_requested: bool,
        out: &mut Execution,
    ) -> Result<Value> {
        let statements = match syntax::parse_detailed(code) {
            Ok(statements) => statements,
            Err(_) if syntax::is_incomplete(code) => {
                return Err("Error: This statement is incomplete.".into());
            }
            Err(error) => return Err(format!("Error: {}", error.message)),
        };
        if statements
            .iter()
            .any(|s| matches!(s.kind, StatementKind::Function(_)))
        {
            return Err("Function definitions are not permitted in this context.".into());
        }
        if let [statement] = statements.as_slice()
            && let StatementKind::Evaluate(e) = &statement.kind
            && e.targets.is_empty()
            && e.list_target.is_none()
            && e.indexed_target.is_none()
            && e.path_target.is_none()
            && !self.langx_is_command(&e.expr)?
        {
            if output_requested {
                return self.eval(&e.expr, None, out);
            }
            if let Expr::Name(name) = &e.expr
                && let Some(array) = self.array(name)
            {
                if !e.quiet {
                    self.emit(out, self.display_text(name, &array)?)?;
                }
                return Ok(Value::Void);
            }
        }
        let mut byte = 0;
        self.lg3_eval_begin();
        let result = self.execute_statements(&statements, &mut byte, out);
        self.lg3_eval_end();
        result?;
        Ok(Value::Void)
    }
    fn langx_is_command(&mut self, expr: &Expr) -> Result<bool> {
        let (Expr::Name(name) | Expr::Call(name, _)) = expr else {
            return Ok(false);
        };
        if self.contains_array(name) {
            return Ok(false);
        }
        Ok(EVAL_COMMANDS.contains(&name.as_str())
            || (matches!(expr, Expr::Name(_)) && self.resolve_script(name)?.is_some()))
    }

    /// rat's tolerance argument, or 1e-6*norm(X(:),1).
    fn langx_rat_input(&self, args: &[Value], name: &str) -> Result<(Matrix, Option<f64>)> {
        let (value, second) = match args {
            [value] => (value, None),
            [value, second] => (value, Some(second)),
            _ => return Err(format!("{name} expects one or two arguments")),
        };
        let x = match value {
            Value::Matrix(m) => m.clone(),
            Value::Logical(m) => m.to_numeric(&self.cancel)?,
            Value::Complex(_) => return Err(format!("{name}: complex input is not supported")),
            _ => return Err(format!("{name}: input must be numeric")),
        };
        let second = match second {
            None => None,
            Some(Value::Matrix(m)) if m.len() == 1 => Some(m.data()[0]),
            Some(_) => return Err(format!("{name}: the second argument must be a scalar")),
        };
        Ok((x, second))
    }
    fn langx_default_tolerance(&self, x: &Matrix) -> Result<f64> {
        let mut norm = 0.;
        for (i, v) in x.data().iter().enumerate() {
            if i % 4096 == 0 {
                self.check_cancel()?;
            }
            norm += v.abs();
        }
        Ok(1e-6 * norm)
    }
    /// `S = rat(X[, tol])`: one text row per element of X(:); `[N, D] = rat(...)`:
    /// arrays of X's shape with N./D within tol of X.
    fn langx_rat(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if count > 2 {
            return Err("Too many output arguments.".into());
        }
        let (x, tol) = self.langx_rat_input(args, "rat")?;
        let tol = match tol {
            Some(tol) => tol,
            None => self.langx_default_tolerance(&x)?,
        };
        if count == 2 {
            let mut numerators = Vec::with_capacity(x.len());
            let mut denominators = Vec::with_capacity(x.len());
            for (i, &v) in x.data().iter().enumerate() {
                if i % 4096 == 0 {
                    self.check_cancel()?;
                }
                let (n, d) = rat_pair(v, tol);
                numerators.push(n);
                denominators.push(d);
            }
            return Ok(vec![
                Value::Matrix(Matrix::from_column_vec(x.rows(), x.cols(), numerators)?),
                Value::Matrix(Matrix::from_column_vec(x.rows(), x.cols(), denominators)?),
            ]);
        }
        let mut rows: Vec<Vec<u16>> = Vec::with_capacity(x.len());
        let mut width = 0;
        for (i, &v) in x.data().iter().enumerate() {
            if i % 4096 == 0 {
                self.check_cancel()?;
            }
            let row: Vec<u16> = rat_text(v, tol).encode_utf16().collect();
            width = width.max(row.len());
            if width.saturating_mul(rows.len() + 1) > MAX_CHARACTER_UNITS {
                return Err(format!(
                    "Character result exceeds {MAX_CHARACTER_UNITS} characters"
                ));
            }
            rows.push(row);
        }
        Ok(vec![Value::Container(padded_rows(&rows, &self.cancel)?)])
    }

    /// `rats(X[, strlen])`: each element as `N/D` (or `N`) centred in a field of
    /// strlen+1 characters, `*` when it is longer than strlen; one text row per row
    /// of X.
    fn langx_rats(&self, args: &[Value]) -> Result<Value> {
        let (x, strlen) = self.langx_rat_input(args, "rats")?;
        let strlen = match strlen {
            None => 13,
            Some(n) if n >= 1. && n.fract() == 0. && n <= MAX_CHARACTER_UNITS as f64 => n as usize,
            Some(_) => return Err("rats: STRLEN must be a positive integer".into()),
        };
        let field = strlen + 1;
        if field.saturating_mul(x.len()) > MAX_CHARACTER_UNITS {
            return Err(format!(
                "Character result exceeds {MAX_CHARACTER_UNITS} characters"
            ));
        }
        let tol = self.langx_default_tolerance(&x)?;
        let (rows, cols) = (x.rows(), x.cols());
        let mut lines = Vec::with_capacity(rows);
        for i in 0..rows {
            self.check_cancel()?;
            let mut line = Vec::with_capacity(field * cols);
            for j in 0..cols {
                let v = x.data()[j * rows + i];
                let text = if v.is_finite() {
                    let (n, d) = rat_pair(v, tol);
                    if d == 1. {
                        integer_text(n)
                    } else {
                        format!("{}/{}", integer_text(n), integer_text(d))
                    }
                } else {
                    integer_text(v)
                };
                let text = if text.len() > strlen {
                    "*".to_owned()
                } else {
                    text
                };
                let left = (field - text.len()).div_ceil(2);
                let right = field - text.len() - left;
                line.extend(std::iter::repeat_n(u16::from(b' '), left));
                line.extend(text.encode_utf16());
                line.extend(std::iter::repeat_n(u16::from(b' '), right));
            }
            lines.push(line);
        }
        Ok(Value::Container(padded_rows(&lines, &self.cancel)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn continued_fraction_convergents_and_text() {
        use std::f64::consts::PI;
        // Nearest-integer quotients of pi: 3, 7, 16, ... with convergents 22/7, 355/113.
        assert_eq!(rat_pair(PI, 1e-2), (22., 7.));
        assert_eq!(rat_pair(PI, 1e-6), (355., 113.));
        assert_eq!(rat_text(PI, 1e-6), "3 + 1/(7 + 1/(16))");
        assert_eq!(rat_text(0.75, 1e-6), "1 + 1/(-4)");
        assert_eq!(rat_text(-0.2, 1e-6), "-0 + 1/(-5)");
        assert_eq!(rat_pair(-0.75, 0.), (-3., 4.));
        assert_eq!(rat_pair(f64::NEG_INFINITY, 0.), (-1., 0.));
        assert_eq!(rat_text(f64::INFINITY, 0.), "Inf");
        assert_eq!(rat_text(f64::NAN, 0.), "NaN");
        // A zero tolerance still stops once the convergent is within eps(pi).
        let (n, d) = rat_pair(PI, 0.);
        assert!((PI - n / d).abs() <= spacing(PI) && d < 1e9, "{n}/{d}");
        assert_eq!(rat_text(1e-310, 0.), "0");
    }

    #[test]
    fn strtok_lookup_and_both_scans_observe_cancellation() {
        let engine = Engine::default();
        let chars = CharacterRow::from_column_major(
            1,
            MAX_CHARACTER_UNITS,
            vec![32; MAX_CHARACTER_UNITS],
            &engine.cancel,
        )
        .unwrap();
        let whitespace = engine.langx_delimiters(&WHITESPACE).unwrap();
        let empty = engine.langx_delimiters(&[]).unwrap();
        engine.cancel.store(true, Ordering::Relaxed);
        assert!(
            matches!(engine.langx_delimiters(&[256; 4096]), Err(e) if e == "Calculation cancelled")
        );
        for lookup in [&whitespace, &empty] {
            assert_eq!(
                engine.langx_token(&chars, lookup).unwrap_err(),
                "Calculation cancelled"
            );
        }
        // Start beyond the initial check, so each span must observe cancellation
        // at its next periodic check without reaching output construction.
        for (lookup, skip_delimiters) in [(&whitespace, true), (&empty, false)] {
            assert_eq!(
                engine
                    .langx_delimiter_span(chars.units(), 1, lookup, skip_delimiters)
                    .unwrap_err(),
                "Calculation cancelled"
            );
        }
    }
}
