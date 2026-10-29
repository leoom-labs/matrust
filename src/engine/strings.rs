//! Engine glue for the text functions: sprintf/fprintf, num2str/mat2str and the
//! everyday string helpers. Character rows travel as `Value::Text`, cells as
//! `Array::Cell` of character arrays.
use super::{
    Engine, Execution, Value,
    text_format::{self as tf, Item, MAX_TEXT},
};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result,
    script_containers::MAX_CHARACTER_UNITS,
};
use std::sync::Arc;

const NAMES: &[&str] = &[
    "sprintf",
    "fprintf",
    "num2str",
    "int2str",
    "mat2str",
    "strcat",
    "strsplit",
    "strjoin",
    "strrep",
    "strtrim",
    "deblank",
    "upper",
    "lower",
    "strcmp",
    "strcmpi",
    "strncmp",
    "strncmpi",
    "strfind",
    "num2cell",
    "iscellstr",
    "blanks",
    "fliplr",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

const MAX_PIECES: usize = 65536;

fn is_white(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\0')
}

/// `isspace`: what `strtrim` removes (`deblank` also removes NUL).
fn is_space(c: char) -> bool {
    c != '\0' && is_white(c)
}

/// A string-like operand: one character row, or the strings of a cell
/// (`None` marks a non-character cell element).
enum Operand {
    Text(String),
    Cell {
        rows: usize,
        cols: usize,
        items: Vec<Option<String>>,
    },
    Other,
}

impl Engine {
    pub(super) fn text_format_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        if name == "fliplr" {
            return self.fliplr_text(&args);
        }
        // These operations understand character-array shape. Do not pass their
        // input through the row-only UTF-8 argument conversion.
        if let Some(Value::Container(Array::Character(chars))) = args.first() {
            self.check_cancel()?;
            match name {
                "num2str" | "int2str" => return self.num2str_call(name, &args).map(Some),
                "mat2str" => return self.mat2str_call(&args).map(Some),
                "deblank" if !chars.is_text_row() => {
                    return self.deblank_call(&args).map(Some);
                }
                _ => {}
            }
        }
        let args = self.text_arguments(args)?;
        self.check_cancel()?;
        let value = match name {
            "sprintf" => {
                let items = self.format_arguments("sprintf", &args)?;
                let text = tf::format_items(
                    "sprintf",
                    self.format_text("sprintf", &args)?,
                    &items,
                    &self.cancel,
                )?;
                Value::Text(text)
            }
            "fprintf" => self.fprintf_call(&args, out)?,
            "num2str" | "int2str" => self.num2str_call(name, &args)?,
            "mat2str" => self.mat2str_call(&args)?,
            "strcat" => self.strcat_call(&args)?,
            "strsplit" => self.strsplit_call(&args)?,
            "strjoin" => self.strjoin_call(&args)?,
            "strrep" => self.strrep_call(&args)?,
            "strtrim" => self.map_strings(name, &args, |s| s.trim_matches(is_space).to_owned())?,
            "deblank" => {
                self.map_strings(name, &args, |s| s.trim_end_matches(is_white).to_owned())?
            }
            "upper" => self.map_case(&args, true)?,
            "lower" => self.map_case(&args, false)?,
            "strcmp" | "strcmpi" | "strncmp" | "strncmpi" => self.strcmp_call(name, &args)?,
            "strfind" => self.strfind_call(&args)?,
            "num2cell" => self.num2cell_call(&args)?,
            "iscellstr" => {
                let [value] = args.as_slice() else {
                    return Err("iscellstr requires exactly one input".into());
                };
                let answer = match value {
                    Value::Container(Array::Cell(c)) => {
                        let mut all = true;
                        for i in 0..c.len() {
                            if i.is_multiple_of(1024) {
                                self.check_cancel()?;
                            }
                            if !matches!(c.get_linear(i)?.as_ref(), Array::Character(_)) {
                                all = false;
                                break;
                            }
                        }
                        all
                    }
                    _ => false,
                };
                Value::Logical(LogicalMatrix::scalar(answer))
            }
            "blanks" => {
                let [Value::Matrix(m)] = args.as_slice() else {
                    return Err("blanks requires one nonnegative integer".into());
                };
                let n = m.as_scalar()?;
                if !(0. ..=65536.).contains(&n) || n.fract() != 0. {
                    return Err("blanks: N must be an integer in 0..65536".into());
                }
                Value::Text(" ".repeat(n as usize))
            }
            _ => unreachable!("NAMES and the match above agree"),
        };
        Ok(Some(value))
    }

    fn text_row<'a>(&self, who: &str, what: &str, v: Option<&'a Value>) -> Result<&'a str> {
        match v {
            Some(Value::Text(s)) => Ok(s),
            _ => Err(format!("{who}: {what} must be a character row")),
        }
    }

    // ---- sprintf / fprintf --------------------------------------------------

    fn format_text<'a>(&self, who: &str, args: &'a [Value]) -> Result<&'a str> {
        self.text_row(who, "format", args.first())
    }

    fn format_arguments(&self, who: &str, args: &[Value]) -> Result<Vec<Item>> {
        // Count every argument from its shape before copying any of them.
        let (mut count, mut size) = (0usize, 0usize);
        for value in args.iter().skip(1) {
            let (items, units) = match value {
                Value::Matrix(m) => (m.len(), m.len()),
                Value::Logical(m) => (m.len(), m.len()),
                Value::Complex(m) => (m.len(), m.len()),
                Value::Text(s) => (1, s.len()),
                Value::Container(Array::Cell(_)) => {
                    return Err(format!("{who}: cell array arguments are not supported yet"));
                }
                _ => return Err(format!("{who}: unsupported argument type")),
            };
            count += items;
            size = size.saturating_add(units);
        }
        if size > 4 * MAX_TEXT {
            return Err(format!("{who}: too many arguments"));
        }
        let mut items = Vec::with_capacity(count);
        for value in args.iter().skip(1) {
            self.check_cancel()?;
            match value {
                Value::Matrix(m) => items.extend(m.data().iter().map(|&v| Item::Num(v))),
                Value::Logical(m) => {
                    items.extend(m.data().iter().map(|&v| Item::Num(f64::from(v))))
                }
                // Numeric conversions print only the real part of a complex value.
                Value::Complex(m) => items.extend(m.data().iter().map(|z| Item::Num(z.re))),
                Value::Text(s) => items.push(Item::Str(s.chars().collect())),
                _ => unreachable!("argument types checked above"),
            }
        }
        Ok(items)
    }

    /// Write text to the command window. A trailing partial line is held back so
    /// the next output continues on it.
    pub(super) fn print_text(&mut self, out: &mut Execution, text: &str) -> Result<()> {
        let mut lines = text.split('\n');
        let tail = lines.next_back().unwrap_or("");
        for line in lines {
            self.emit(out, line.to_owned())?;
        }
        self.hold_output(tail)
    }

    fn fprintf_call(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let mut file = None;
        let skip = match args.first() {
            Some(Value::Matrix(m)) if m.len() == 1 => {
                let fid = m.data()[0];
                if fid != 1. && fid != 2. {
                    file = Some(fid);
                }
                1
            }
            Some(Value::Text(_)) => 0,
            _ => return Err("fprintf: format must be a character row".into()),
        };
        let args = &args[skip..];
        let text = {
            let items = self.format_arguments("fprintf", args)?;
            tf::format_items(
                "fprintf",
                self.format_text("fprintf", args)?,
                &items,
                &self.cancel,
            )?
        };
        if let Some(fid) = file {
            return Ok(Value::Matrix(Matrix::scalar(
                self.fileio_write_text(fid, &text)? as f64,
            )));
        }
        self.print_text(out, &text)?;
        Ok(Value::Matrix(Matrix::scalar(text.len() as f64)))
    }

    // ---- num2str / mat2str --------------------------------------------------

    fn char_rows(&self, lines: Vec<String>) -> Result<Value> {
        if lines.len() == 1 {
            return Ok(Value::Text(lines.into_iter().next().unwrap()));
        }
        let rows = lines.len();
        let cols = lines.first().map_or(0, |l| l.chars().count());
        if rows
            .checked_mul(cols)
            .is_none_or(|n| n > MAX_CHARACTER_UNITS)
        {
            return Err(format!(
                "text result exceeds {MAX_CHARACTER_UNITS} characters"
            ));
        }
        let mut units = vec![0u16; rows * cols];
        for (r, line) in lines.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                units[c * rows + r] = u16::try_from(u32::from(ch)).unwrap_or(u16::from(b'?'));
            }
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
        ))
    }

    fn num2str_call(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        let (x, option) = match args {
            [x] => (x, None),
            [x, option] if name == "num2str" => (x, Some(option)),
            _ => return Err(format!("{name}: wrong number of input arguments")),
        };
        // MATLAB's int2str formats character codes ('ab' gives '97  98'); num2str returns text as is.
        if name == "int2str" {
            let codes = match x {
                Value::Text(s) => {
                    let units: Vec<f64> = s.encode_utf16().map(f64::from).collect();
                    Some(Matrix::from_column_vec(1, units.len(), units)?)
                }
                Value::Container(Array::Character(c)) => {
                    let units = c.units().iter().map(|&u| f64::from(u)).collect();
                    Some(Matrix::from_column_vec(c.rows(), c.cols(), units)?)
                }
                _ => None,
            };
            if let Some(m) = codes {
                return self.num2str_call(name, &[Value::Matrix(m)]);
            }
        }
        if matches!(x, Value::Text(_) | Value::Container(Array::Character(_))) {
            return Ok(x.clone());
        }
        let precision = match option {
            None => None,
            Some(Value::Text(_)) => None,
            Some(Value::Matrix(m)) if m.len() == 1 => {
                let p = m.data()[0];
                if !(0. ..=64.).contains(&p) || p.fract() != 0. {
                    return Err("num2str: precision must be an integer in 0..64".into());
                }
                Some(p as usize)
            }
            Some(_) => {
                return Err("num2str: second argument must be a precision or a format".into());
            }
        };
        let custom = match option {
            Some(Value::Text(f)) => Some(f.as_str()),
            _ => None,
        };
        let (rows, cols) = match x {
            Value::Matrix(m) => (m.rows(), m.cols()),
            Value::Logical(m) => (m.rows(), m.cols()),
            Value::Complex(_) if custom.is_some() => {
                return Err("num2str: complex input with a format is not supported yet".into());
            }
            Value::Complex(m) => (m.rows(), m.cols()),
            _ => {
                return Err(format!(
                    "{name}: input must be numeric, logical or character"
                ));
            }
        };
        if rows * cols == 0 {
            return Ok(Value::Text(String::new()));
        }
        // Bound the work before copying the data: a custom format through its
        // repeated row format, the default formats (at least one character per
        // element) through the character limit.
        let custom_row = match custom {
            Some(f) => Some(tf::row_format(f, rows, cols)?),
            None if rows * cols > MAX_CHARACTER_UNITS => {
                return Err(format!(
                    "{name}: result exceeds {MAX_CHARACTER_UNITS} characters"
                ));
            }
            None => None,
        };
        let mut data: Vec<f64> = match x {
            Value::Matrix(m) => m.data().to_vec(),
            Value::Logical(m) => m.data().iter().map(|&v| f64::from(v)).collect(),
            Value::Complex(m) => {
                let parts: Vec<(f64, f64)> = m.data().iter().map(|z| (z.re, z.im)).collect();
                let lines = tf::num2str_complex(&parts, rows, cols, precision, &self.cancel)?;
                return self.char_rows(lines);
            }
            _ => unreachable!("checked above"),
        };
        if name == "int2str" {
            for v in &mut data {
                *v = v.round();
            }
        }
        let row = match custom_row {
            Some(row) => row,
            None => tf::row_format(&tf::num2str_format(&data, precision), rows, cols)?,
        };
        let lines = tf::num2str_rows(&data, rows, cols, &row, &self.cancel)?;
        self.char_rows(lines)
    }

    fn mat2str_call(&mut self, args: &[Value]) -> Result<Value> {
        let (x, precision) = match args {
            [x] => (x, 15usize),
            [x, Value::Matrix(m)] if m.len() == 1 => {
                let p = m.data()[0];
                if !(1. ..=64.).contains(&p) || p.fract() != 0. {
                    return Err("mat2str: precision must be an integer in 1..64".into());
                }
                (x, p as usize)
            }
            [
                _,
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)),
            ] => {
                return Err("mat2str: the 'class' option is not supported yet".into());
            }
            _ => return Err("mat2str: expected an array and an optional precision".into()),
        };
        if let Value::Text(s) = x {
            return Ok(Value::Text(format!("'{}'", s.replace('\'', "''"))));
        }
        if let Value::Container(Array::Character(chars)) = x {
            return self.mat2str_chars(chars);
        }
        let (rows, cols) = match x {
            Value::Matrix(m) => (m.rows(), m.cols()),
            Value::Logical(m) => (m.rows(), m.cols()),
            Value::Complex(m) => (m.rows(), m.cols()),
            _ => return Err("mat2str: input must be numeric, logical or character".into()),
        };
        if (rows, cols) == (0, 0) {
            return Ok(Value::Text("[]".into()));
        }
        if rows * cols == 0 {
            return Ok(Value::Text(format!("zeros({rows},{cols})")));
        }
        let element = |index: usize| -> String {
            match x {
                Value::Matrix(m) => tf::mat2str_real(m.data()[index], precision),
                Value::Logical(m) => (if m.data()[index] != 0 {
                    "true"
                } else {
                    "false"
                })
                .into(),
                Value::Complex(m) => {
                    let z = m.data()[index];
                    tf::mat2str_complex(z.re, z.im, precision)
                }
                _ => unreachable!(),
            }
        };
        let mut text = String::new();
        if rows * cols > 1 {
            text.push('[');
        }
        for r in 0..rows {
            if r % 256 == 0 {
                self.check_cancel()?;
            }
            if r > 0 {
                text.push(';');
            }
            for c in 0..cols {
                if c > 0 {
                    text.push(' ');
                }
                text.push_str(&element(c * rows + r));
            }
            if text.len() > MAX_TEXT {
                return Err("mat2str: result exceeds 2 MiB".into());
            }
        }
        if rows * cols > 1 {
            text.push(']');
        }
        Ok(Value::Text(text))
    }

    // ---- string helpers -----------------------------------------------------

    fn mat2str_chars(&self, chars: &CharacterRow) -> Result<Value> {
        if chars.units().is_empty() {
            return Ok(Value::Text("''".into()));
        }
        let mut text = String::new();
        if chars.rows() > 1 {
            text.push('[');
        }
        for r in 0..chars.rows() {
            self.check_cancel()?;
            if r > 0 {
                text.push(';');
            }
            text.push('\'');
            let row = (0..chars.cols()).map(|c| chars.units()[c * chars.rows() + r]);
            for (i, ch) in char::decode_utf16(row).enumerate() {
                if i.is_multiple_of(1024) {
                    self.check_cancel()?;
                }
                let ch =
                    ch.map_err(|_| "mat2str: character row contains an unpaired UTF-16 surrogate")?;
                text.push(ch);
                if ch == '\'' {
                    text.push(ch);
                }
            }
            text.push('\'');
            if text.len() > MAX_TEXT {
                return Err("mat2str: result exceeds 2 MiB".into());
            }
        }
        if chars.rows() > 1 {
            text.push(']');
        }
        Ok(Value::Text(text))
    }

    fn deblank_call(&self, args: &[Value]) -> Result<Value> {
        let [Value::Container(Array::Character(chars))] = args else {
            return Err("deblank requires exactly one input".into());
        };
        let rows = chars.rows();
        let mut cols = chars.cols();
        while cols > 0 {
            self.check_cancel()?;
            if chars.units()[(cols - 1) * rows..cols * rows]
                .iter()
                .any(|&unit| !matches!(unit, 0 | 9..=13 | 32))
            {
                break;
            }
            cols -= 1;
        }
        if cols == chars.cols() {
            return Ok(args[0].clone());
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(
                rows,
                cols,
                chars.units()[..rows * cols].to_vec(),
                &self.cancel,
            )?
            .into(),
        ))
    }

    fn string_array(&self, s: &str) -> Result<Array> {
        Ok(CharacterRow::from_text(s, &self.cancel)?.into())
    }

    fn cell_value(&self, rows: usize, cols: usize, items: Vec<Array>) -> Result<Value> {
        // A plain `collect` reuses the larger `Array` buffer, and the cell
        // constructor rejects a capacity above its slot limit.
        let mut slots = Vec::with_capacity(items.len());
        slots.extend(items.into_iter().map(Arc::new));
        Ok(CellArray::from_shared(rows, cols, slots, &self.cancel)?.into_array_value())
    }

    fn cell_strings(&self, who: &str, c: &CellArray) -> Result<Vec<Option<String>>> {
        let mut items = Vec::with_capacity(c.len());
        for i in 0..c.len() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            items.push(match c.get_linear(i)?.as_ref() {
                Array::Character(a) => {
                    Some(a.to_utf8(&self.cancel).map_err(|e| format!("{who}: {e}"))?)
                }
                _ => None,
            });
        }
        Ok(items)
    }

    fn operand(&self, who: &str, v: &Value) -> Result<Operand> {
        Ok(match v {
            Value::Text(s) => Operand::Text(s.clone()),
            Value::Container(Array::Cell(c)) => Operand::Cell {
                rows: c.rows(),
                cols: c.cols(),
                items: self.cell_strings(who, c)?,
            },
            _ => Operand::Other,
        })
    }

    /// Apply `f` to a character row, or to every string of a cell.
    fn map_strings(&self, who: &str, args: &[Value], f: impl Fn(&str) -> String) -> Result<Value> {
        let [value] = args else {
            return Err(format!("{who} requires exactly one input"));
        };
        match self.operand(who, value)? {
            Operand::Text(s) => Ok(Value::Text(f(&s))),
            Operand::Cell { rows, cols, items } => {
                let mut mapped = Vec::with_capacity(items.len());
                for item in items {
                    let Some(s) = item else {
                        return Err(format!(
                            "{who}: input must be a string or cell array of strings"
                        ));
                    };
                    mapped.push(self.string_array(&f(&s))?);
                }
                self.cell_value(rows, cols, mapped)
            }
            Operand::Other => Err(format!(
                "{who}: input must be a string or cell array of strings"
            )),
        }
    }

    fn map_case(&self, args: &[Value], upper: bool) -> Result<Value> {
        let who = if upper { "upper" } else { "lower" };
        match args {
            [Value::Text(_) | Value::Container(Array::Cell(_))] => {
                self.map_strings(who, args, |s| {
                    if upper {
                        s.to_uppercase()
                    } else {
                        s.to_lowercase()
                    }
                })
            }
            [other] => Ok(other.clone()),
            _ => Err(format!("{who} requires exactly one input")),
        }
    }

    fn fliplr_text(&self, args: &[Value]) -> Result<Option<Value>> {
        match args {
            [Value::Text(s)] => Ok(Some(Value::Text(s.chars().rev().collect()))),
            [Value::Container(Array::Character(c))] if c.rows() <= 1 => {
                let units: Vec<u16> = c.units().iter().rev().copied().collect();
                Ok(Some(Value::Container(
                    CharacterRow::from_column_major(c.rows(), c.cols(), units, &self.cancel)?
                        .into(),
                )))
            }
            _ => Ok(None),
        }
    }

    fn strcat_call(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err("strcat requires at least one input".into());
        }
        let mut operands = Vec::with_capacity(args.len());
        for a in args {
            match self.operand("strcat", a)? {
                Operand::Other => {
                    return Err("strcat: inputs must be strings or cell arrays of strings".into());
                }
                o => operands.push(o),
            }
        }
        let mut shape: Option<(usize, usize)> = None;
        for o in &operands {
            if let Operand::Cell { rows, cols, .. } = o
                && rows * cols != 1
            {
                match shape {
                    None => shape = Some((*rows, *cols)),
                    Some(s) if s == (*rows, *cols) => {}
                    Some(_) => {
                        return Err("strcat: nonscalar cell arrays must have the same size".into());
                    }
                }
            }
        }
        let any_cell = operands.iter().any(|o| matches!(o, Operand::Cell { .. }));
        if !any_cell {
            let mut text = String::new();
            for o in &operands {
                if let Operand::Text(s) = o {
                    text.push_str(s.trim_end_matches(is_white));
                }
            }
            return Ok(Value::Text(text));
        }
        let (rows, cols) = shape.unwrap_or((1, 1));
        let mut result = Vec::with_capacity(rows * cols);
        for i in 0..rows * cols {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let mut text = String::new();
            for o in &operands {
                match o {
                    Operand::Text(s) => text.push_str(s.trim_end_matches(is_white)),
                    Operand::Cell { items, .. } => {
                        let item = if items.len() == 1 {
                            &items[0]
                        } else {
                            &items[i]
                        };
                        let Some(s) = item else {
                            return Err("strcat: cell elements must be strings".into());
                        };
                        text.push_str(s);
                    }
                    Operand::Other => unreachable!(),
                }
            }
            if text.len() > MAX_TEXT {
                return Err("strcat: result exceeds 2 MiB".into());
            }
            result.push(self.string_array(&text)?);
        }
        self.cell_value(rows, cols, result)
    }

    fn strsplit_call(&self, args: &[Value]) -> Result<Value> {
        let Some(Value::Text(s)) = args.first() else {
            return Err("strsplit: first argument must be a character row".into());
        };
        let mut rest = &args[1..];
        // MATLAB's default: {' ','\f','\n','\r','\t','\v'}, collapsed.
        let mut delimiters: Vec<Vec<char>> = [' ', '\u{c}', '\n', '\r', '\t', '\u{b}']
            .into_iter()
            .map(|c| vec![c])
            .collect();
        if let Some(first) = rest.first() {
            let is_option = matches!(first, Value::Text(t) if t.eq_ignore_ascii_case("collapsedelimiters"))
                && rest.len() == 2;
            if !is_option {
                delimiters = match first {
                    Value::Text(d) => vec![tf::unescape(d).chars().collect()],
                    Value::Container(Array::Cell(c)) => {
                        let mut list = Vec::new();
                        for item in self.cell_strings("strsplit", c)? {
                            let Some(d) = item else {
                                return Err("strsplit: delimiters must be strings".into());
                            };
                            list.push(tf::unescape(&d).chars().collect());
                        }
                        list
                    }
                    _ => {
                        return Err(
                            "strsplit: delimiter must be a string or cell array of strings".into(),
                        );
                    }
                };
                rest = &rest[1..];
            }
        }
        let mut collapse = true;
        match rest {
            [] => {}
            [Value::Text(name), value] if name.eq_ignore_ascii_case("collapsedelimiters") => {
                collapse = match value {
                    Value::Logical(m) if m.len() == 1 => m.data()[0] != 0,
                    Value::Matrix(m) if m.len() == 1 => m.data()[0] != 0.,
                    _ => return Err("strsplit: CollapseDelimiters must be true or false".into()),
                };
            }
            [Value::Text(name), _] if name.eq_ignore_ascii_case("delimitertype") => {
                return Err("strsplit: DelimiterType is not supported yet".into());
            }
            _ => return Err("strsplit: unsupported option".into()),
        }
        delimiters.retain(|d| !d.is_empty());
        if delimiters.is_empty() {
            return Err("strsplit: delimiters must not be empty".into());
        }
        delimiters.sort_by_key(|d| std::cmp::Reverse(d.len()));
        let chars: Vec<char> = s.chars().collect();
        let matching = |at: usize| {
            delimiters
                .iter()
                .find(|d| chars[at..].starts_with(d))
                .map(Vec::len)
        };
        let mut pieces: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut i = 0;
        while i < chars.len() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if let Some(len) = matching(i) {
                if pieces.len() >= MAX_PIECES {
                    return Err("strsplit: result exceeds 65536 cells".into());
                }
                pieces.push(std::mem::take(&mut current));
                i += len;
                while collapse && i < chars.len() {
                    match matching(i) {
                        Some(len) => i += len,
                        None => break,
                    }
                }
            } else {
                current.push(chars[i]);
                i += 1;
            }
        }
        if pieces.len() >= MAX_PIECES {
            return Err("strsplit: result exceeds 65536 cells".into());
        }
        pieces.push(current);
        let items = pieces
            .iter()
            .map(|p| self.string_array(p))
            .collect::<Result<Vec<_>>>()?;
        self.cell_value(1, items.len(), items)
    }

    fn strjoin_call(&self, args: &[Value]) -> Result<Value> {
        let (cell, delimiter) = match args {
            [c] => (c, " ".to_owned()),
            [c, Value::Text(d)] => (c, tf::unescape(d)),
            [_, _] => return Err("strjoin: a cell array of delimiters is not supported yet".into()),
            _ => return Err("strjoin requires a cell array and an optional delimiter".into()),
        };
        let Value::Container(Array::Cell(c)) = cell else {
            return Err("strjoin: first argument must be a cell array of strings".into());
        };
        let mut text = String::new();
        for (i, item) in self.cell_strings("strjoin", c)?.into_iter().enumerate() {
            let Some(s) = item else {
                return Err("strjoin: first argument must be a cell array of strings".into());
            };
            if i > 0 {
                text.push_str(&delimiter);
            }
            text.push_str(&s);
            if text.len() > MAX_TEXT {
                return Err("strjoin: result exceeds 2 MiB".into());
            }
        }
        Ok(Value::Text(text))
    }

    fn strrep_call(&self, args: &[Value]) -> Result<Value> {
        let [subject, Value::Text(old), Value::Text(new)] = args else {
            return Err("strrep requires a string or cell array and two character rows".into());
        };
        // MATLAB replaces overlapping matches too: every match start emits `new`,
        // and characters covered by any match are dropped ('aaa','aa' -> 2 matches).
        let replace = |s: &str| -> Result<String> {
            if old.is_empty() {
                return Ok(s.to_owned());
            }
            let mut starts = Vec::new();
            let mut from = 0;
            while let Some(found) = s[from..].find(old.as_str()) {
                if starts.len() % 1024 == 0 {
                    self.check_cancel()?;
                }
                let at = from + found;
                starts.push(at);
                from = at + s[at..].chars().next().map_or(1, char::len_utf8);
            }
            if starts
                .len()
                .checked_mul(new.len())
                .and_then(|n| n.checked_add(s.len()))
                .is_none_or(|n| n > MAX_TEXT)
            {
                return Err("strrep: result exceeds 2 MiB".into());
            }
            let mut text = String::with_capacity(s.len());
            let mut copied = 0;
            for at in starts {
                if at > copied {
                    text.push_str(&s[copied..at]);
                }
                text.push_str(new);
                copied = copied.max(at + old.len());
            }
            text.push_str(&s[copied..]);
            Ok(text)
        };
        match self.operand("strrep", subject)? {
            Operand::Text(s) => Ok(Value::Text(replace(&s)?)),
            Operand::Cell { rows, cols, items } => {
                let mut mapped = Vec::with_capacity(items.len());
                for item in items {
                    let Some(s) = item else {
                        return Err("strrep: cell elements must be strings".into());
                    };
                    mapped.push(self.string_array(&replace(&s)?)?);
                }
                self.cell_value(rows, cols, mapped)
            }
            Operand::Other => {
                Err("strrep: first argument must be a string or cell array of strings".into())
            }
        }
    }

    fn strcmp_call(&self, name: &str, args: &[Value]) -> Result<Value> {
        let limited = name.starts_with("strn");
        let insensitive = name.ends_with('i');
        let (a, b, n) = match (limited, args) {
            (false, [a, b]) => (a, b, None),
            (true, [a, b, Value::Matrix(n)]) if n.len() == 1 => {
                let n = n.data()[0];
                if !(0. ..=1e9).contains(&n) || n.fract() != 0. {
                    return Err(format!("{name}: N must be a nonnegative integer"));
                }
                (a, b, Some(n as usize))
            }
            _ => {
                return Err(format!(
                    "{name} requires {} inputs",
                    if limited { "three" } else { "two" }
                ));
            }
        };
        let same = |x: &Option<String>, y: &Option<String>| -> bool {
            let (Some(x), Some(y)) = (x, y) else {
                return false;
            };
            let (x, y) = if insensitive {
                (x.to_lowercase(), y.to_lowercase())
            } else {
                (x.clone(), y.clone())
            };
            // strncmp compares the first N UTF-16 units of each string; a string
            // shorter than N only matches an identical string.
            match n {
                None => x == y,
                Some(n) => x.encode_utf16().take(n).eq(y.encode_utf16().take(n)),
            }
        };
        let (left, right) = (self.operand(name, a)?, self.operand(name, b)?);
        let single = |o: &Operand| match o {
            Operand::Text(s) => Some(s.clone()),
            _ => None,
        };
        let (shape, flags): ((usize, usize), Vec<bool>) = match (&left, &right) {
            (
                Operand::Cell { rows, cols, items },
                Operand::Cell {
                    rows: r2,
                    cols: c2,
                    items: i2,
                },
            ) => {
                if (rows, cols) == (r2, c2) {
                    (
                        (*rows, *cols),
                        items.iter().zip(i2).map(|(x, y)| same(x, y)).collect(),
                    )
                } else if items.len() == 1 {
                    ((*r2, *c2), i2.iter().map(|y| same(&items[0], y)).collect())
                } else if i2.len() == 1 {
                    (
                        (*rows, *cols),
                        items.iter().map(|x| same(x, &i2[0])).collect(),
                    )
                } else {
                    return Err(format!(
                        "{name}: cell array arguments must have the same size"
                    ));
                }
            }
            (Operand::Cell { rows, cols, items }, other)
            | (other, Operand::Cell { rows, cols, items }) => {
                let s = single(other);
                ((*rows, *cols), items.iter().map(|x| same(x, &s)).collect())
            }
            (l, r) => ((1, 1), vec![same(&single(l), &single(r))]),
        };
        if shape == (1, 1) {
            return Ok(Value::Logical(LogicalMatrix::scalar(flags[0])));
        }
        let data = flags.into_iter().map(u8::from).collect();
        Ok(Value::Logical(LogicalMatrix::from_column_vec(
            shape.0,
            shape.1,
            data,
            &self.cancel,
        )?))
    }

    fn find_positions(&self, s: &str, pattern: &str) -> Result<Matrix> {
        let mut positions = Vec::new();
        if !pattern.is_empty() {
            // Positions count UTF-16 units, as character arrays store them.
            let (mut byte, mut seen_bytes, mut seen_units) = (0usize, 0usize, 0usize);
            while let Some(found) = s[byte..].find(pattern) {
                if positions.len() % 1024 == 0 {
                    self.check_cancel()?;
                }
                let at = byte + found;
                seen_units += s[seen_bytes..at].encode_utf16().count();
                seen_bytes = at;
                positions.push((seen_units + 1) as f64);
                byte = at + s[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
        if positions.is_empty() {
            return Matrix::filled(0, 0, 0.);
        }
        Matrix::from_column_vec(1, positions.len(), positions)
    }

    fn strfind_call(&self, args: &[Value]) -> Result<Value> {
        let [subject, Value::Text(pattern)] = args else {
            return Err("strfind requires a string or cell array and a pattern string".into());
        };
        match self.operand("strfind", subject)? {
            Operand::Text(s) => Ok(Value::Matrix(self.find_positions(&s, pattern)?)),
            Operand::Cell { rows, cols, items } => {
                let mut found = Vec::with_capacity(items.len());
                for item in items {
                    let Some(s) = item else {
                        return Err("strfind: cell elements must be strings".into());
                    };
                    found.push(Array::Numeric(self.find_positions(&s, pattern)?));
                }
                self.cell_value(rows, cols, found)
            }
            Operand::Other => {
                Err("strfind: first argument must be a string or cell array of strings".into())
            }
        }
    }

    fn num2cell_call(&self, args: &[Value]) -> Result<Value> {
        let [value] = args else {
            return Err("num2cell requires exactly one input".into());
        };
        let (rows, cols) = match value {
            Value::Matrix(m) => (m.rows(), m.cols()),
            Value::Logical(m) => (m.rows(), m.cols()),
            Value::Complex(m) => (m.rows(), m.cols()),
            Value::Container(Array::Typed(m)) => (m.rows(), m.cols()),
            _ => return Err("num2cell: only numeric and logical inputs are supported yet".into()),
        };
        let len = rows * cols;
        if len > crate::script_containers::MAX_CELL_SLOTS {
            return Err("num2cell: result exceeds 65536 cells".into());
        }
        let mut items = Vec::with_capacity(len);
        for k in 0..len {
            if k % 1024 == 0 {
                self.check_cancel()?;
            }
            items.push(match value {
                Value::Matrix(m) => Array::Numeric(Matrix::scalar(m.data()[k])),
                Value::Logical(m) => Array::Logical(LogicalMatrix::scalar(m.data()[k] != 0)),
                Value::Complex(m) => {
                    Array::Complex(crate::ComplexMatrix::scalar(m.data()[k], &self.cancel)?)
                }
                Value::Container(Array::Typed(m)) => Array::Typed(m.select(1, 1, &[k])?),
                _ => unreachable!("checked above"),
            });
        }
        self.cell_value(rows, cols, items)
    }
}
