//! error(), rethrow() and the value bound by `catch err`.
//!
//! The engine's public Result still carries display text. Structured metadata
//! for the error in flight travels in `RaisedError`: failure sites attach it,
//! function frames preserve it, and catch consumes it without inspecting text.
use super::{Engine, Value};
use crate::{Array, CharacterRow, Result, StructArray};

const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_FORMAT_ARGUMENTS: usize = 65536;

#[derive(Clone, Debug)]
pub(super) struct RaisedError {
    message: String,
    identifier: String,
    pub(super) object_error: bool,
    pub(super) object_report: Option<Option<String>>,
}

/// Run budgets and cancellation stop the run; scripts cannot swallow them.
pub(super) fn is_run_limit(message: &str) -> bool {
    [
        "Calculation cancelled",
        "evaluation budget",
        "Execution exceeds 1000000",
        "Execution output exceeds",
        "exceeds 128 levels",
        "call depth exceeds",
        "Workspace exceeds",
        "256 MiB",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

/// `Pkg:id` style: no whitespace, colon-separated nonempty components.
fn looks_like_identifier(text: &str) -> bool {
    text.contains(':')
        && text.split(':').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })
}

enum Atom {
    Text(String),
    Number(f64),
}

fn escapes(format: &str) -> String {
    let mut out = String::with_capacity(format.len());
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\u{7}'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// C `%g`.
fn format_g(v: f64, precision: Option<usize>, alternate: bool, upper: bool) -> String {
    if !v.is_finite() {
        return non_finite(v);
    }
    let precision = precision.unwrap_or(6).max(1);
    if v == 0. {
        return if alternate && precision > 1 {
            format!("0.{}", "0".repeat(precision - 1))
        } else {
            "0".into()
        };
    }
    let scientific = format!("{:.*e}", precision - 1, v);
    let exponent: i32 = scientific
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    let mut text = if exponent < -4 || exponent >= precision as i32 {
        let mantissa = scientific.split_once('e').map_or("", |(m, _)| m);
        let mantissa = if alternate {
            mantissa.to_owned()
        } else {
            trim_zeros(mantissa)
        };
        format!(
            "{mantissa}e{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        )
    } else {
        let decimals = (precision as i32 - 1 - exponent).max(0) as usize;
        let fixed = format!("{v:.decimals$}");
        if alternate { fixed } else { trim_zeros(&fixed) }
    };
    if upper {
        text = text.to_uppercase();
    }
    text
}

fn trim_zeros(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text.to_owned()
    }
}

fn non_finite(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v > 0. {
        "Inf".into()
    } else {
        "-Inf".into()
    }
}

struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    zero: bool,
    alternate: bool,
    width: usize,
    precision: Option<usize>,
    conversion: char,
}

impl Spec {
    fn pad(&self, body: String, numeric: bool) -> String {
        let length = body.chars().count();
        if length >= self.width {
            return body;
        }
        let fill = self.width - length;
        if self.minus {
            format!("{body}{}", " ".repeat(fill))
        } else if self.zero && numeric && body.chars().all(|c| c != 'N' && c != 'I') {
            let split = usize::from(body.starts_with(['-', '+', ' ']));
            format!("{}{}{}", &body[..split], "0".repeat(fill), &body[split..])
        } else {
            format!("{}{body}", " ".repeat(fill))
        }
    }
    fn sign(&self, v: f64, body: String) -> String {
        if body.starts_with('-') || v.is_nan() {
            body
        } else if self.plus {
            format!("+{body}")
        } else if self.space {
            format!(" {body}")
        } else {
            body
        }
    }
    fn number(&self, v: f64) -> String {
        let upper = self.conversion.is_ascii_uppercase();
        let integer = v.is_finite() && v == v.trunc();
        let body = match self.conversion {
            'd' | 'i' | 'u' if integer && v.abs() < 1e18 => format!("{}", v as i64),
            'd' | 'i' | 'u' if !v.is_finite() => non_finite(v),
            // MATLAB switches a non-integer to exponent form under an integer conversion.
            'd' | 'i' | 'u' => format!("{v:.6e}")
                .split_once('e')
                .map(|(m, e)| {
                    let e: i32 = e.parse().unwrap_or(0);
                    format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
                })
                .unwrap_or_default(),
            'x' | 'X' | 'o' if integer && (0. ..1e18).contains(&v) => match self.conversion {
                'x' => format!("{:x}", v as u64),
                'X' => format!("{:X}", v as u64),
                _ => format!("{:o}", v as u64),
            },
            'x' | 'X' | 'o' => format_g(v, None, false, false),
            'f' | 'F' if v.is_finite() => format!("{v:.*}", self.precision.unwrap_or(6)),
            'e' | 'E' if v.is_finite() => {
                let text = crate::struct_display::exponent(v, self.precision.unwrap_or(6));
                if upper { text.to_uppercase() } else { text }
            }
            'g' | 'G' => format_g(v, self.precision, self.alternate, upper),
            _ => non_finite(v),
        };
        self.pad(self.sign(v, body), true)
    }
}

impl Engine {
    /// Only call at a failure site immediately returning Err. No successful
    /// operation may leave pending metadata; catch consumes it even without a
    /// binding, and execute clears it before starting another script.
    pub(super) fn raise(&mut self, message: String, identifier: String) -> String {
        self.raised = Some(RaisedError {
            message: message.clone(),
            identifier,
            object_error: false,
            object_report: None,
        });
        message
    }
    /// Keep the first raw error before a function frame prefixes its location.
    pub(super) fn note_wrapped(&mut self, raw: &str) {
        self.raised.get_or_insert_with(|| RaisedError {
            message: raw.to_owned(),
            identifier: String::new(),
            object_error: false,
            object_report: None,
        });
    }
    /// The message and identifier lasterr reports for a propagating failure.
    pub(super) fn failure_text(&self, message: &str) -> (String, String) {
        self.raised.as_ref().map_or_else(
            || (message.to_owned(), String::new()),
            |r| (r.message.clone(), r.identifier.clone()),
        )
    }
    pub(super) fn is_catchable(&self, message: &str) -> bool {
        !self.cancel.load(std::sync::atomic::Ordering::Relaxed) && !is_run_limit(message)
    }
    /// The MException facade bound by `catch err`, retaining thrown causes and stack.
    pub(super) fn error_value(&mut self, message: &str) -> Result<Array> {
        let (text, identifier, report, object_error) = match self.raised.take() {
            Some(r) => (r.message, r.identifier, r.object_report, r.object_error),
            None => (message.to_owned(), String::new(), None, false),
        };
        self.functions.lng.last_error = (text.clone(), identifier.clone());
        let mut exception = self.lng_exception(&identifier, &text, true)?;
        let report = report.or_else(|| {
            object_error.then(|| {
                let Array::Struct(s) = &exception else {
                    return None;
                };
                let stack = s.get(s.field_index("stack")?, 0).ok()?;
                let Array::Struct(stack) = stack.as_ref() else {
                    return None;
                };
                let name = stack.get(stack.field_index("name")?, 0).ok()?;
                let Array::Character(name) = name.as_ref() else {
                    return None;
                };
                String::from_utf16(name.units()).ok()
            })
        });
        if let (Array::Struct(s), Some(header)) = (&exception, report) {
            exception = Array::Struct(s.with_field(
                super::objects::OBJ_REPORT_HEADER,
                CharacterRow::from_text(&header.unwrap_or_default(), &self.cancel)?.into(),
                &self.cancel,
            )?);
        }
        Ok(exception)
    }
    fn text_of(&self, value: &Value, what: &str) -> Result<String> {
        match value {
            Value::Text(s) | Value::QuotedString(s) if s.len() > MAX_MESSAGE_BYTES => {
                Err(format!("{what} exceeds {MAX_MESSAGE_BYTES} bytes"))
            }
            Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
            Value::Container(Array::Character(c)) => self.char_text(c, what),
            _ => Err(format!("{what} must be character text")),
        }
    }
    /// UTF-8 text of a char row; the byte length is measured before anything is copied.
    fn char_text(&self, c: &CharacterRow, what: &str) -> Result<String> {
        let bytes: usize = char::decode_utf16(c.units().iter().copied())
            .map(|unit| unit.map_or(0, char::len_utf8))
            .sum();
        if bytes > MAX_MESSAGE_BYTES {
            return Err(format!("{what} exceeds {MAX_MESSAGE_BYTES} bytes"));
        }
        c.to_utf8(&self.cancel)
    }
    fn atoms(&self, args: &[Value]) -> Result<Vec<Atom>> {
        // Count first: a large numeric argument is rejected before any expansion.
        let count = args.iter().fold(0usize, |count, value| {
            count.saturating_add(match value {
                Value::Matrix(m) => m.len(),
                Value::Logical(m) => m.len(),
                // A char argument may supply one atom per code unit (numeric conversions).
                Value::Text(s) | Value::QuotedString(s) => s.chars().count().max(1),
                Value::Container(Array::Character(c)) => c.units().len().max(1),
                _ => 1,
            })
        });
        if count > MAX_FORMAT_ARGUMENTS {
            return Err("error: more than 65536 format arguments".into());
        }
        let mut atoms = Vec::with_capacity(count);
        for value in args {
            self.check_cancel()?;
            match value {
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)) => {
                    atoms.push(Atom::Text(self.text_of(value, "error: a format argument")?));
                }
                Value::Matrix(m) => {
                    for chunk in m.data().chunks(4096) {
                        self.check_cancel()?;
                        atoms.extend(chunk.iter().map(|v| Atom::Number(*v)));
                    }
                }
                Value::Logical(m) => {
                    for chunk in m.data().chunks(4096) {
                        self.check_cancel()?;
                        atoms.extend(chunk.iter().map(|v| Atom::Number(f64::from(*v))));
                    }
                }
                _ => {
                    return Err(
                        "error: complex, cell and struct format arguments are not supported yet"
                            .into(),
                    );
                }
            }
        }
        Ok(atoms)
    }
    /// sprintf-style formatting of `%d %i %u %f %e %g %s %c %x %o %%`, recycling
    /// the template while arguments remain.
    fn error_format_text(&self, template: &str, args: &[Value]) -> Result<String> {
        let template = escapes(template);
        let mut atoms: std::collections::VecDeque<Atom> = self.atoms(args)?.into();
        let had_args = !atoms.is_empty();
        let chars: Vec<char> = template.chars().collect();
        let mut out = String::new();
        loop {
            self.check_cancel()?;
            let mut consumed = false;
            let mut i = 0;
            while i < chars.len() {
                if chars[i] != '%' {
                    out.push(chars[i]);
                    i += 1;
                    continue;
                }
                i += 1;
                if chars.get(i) == Some(&'%') {
                    out.push('%');
                    i += 1;
                    continue;
                }
                let mut spec = Spec {
                    minus: false,
                    plus: false,
                    space: false,
                    zero: false,
                    alternate: false,
                    width: 0,
                    precision: None,
                    conversion: ' ',
                };
                while let Some(&c) = chars.get(i) {
                    match c {
                        '-' => spec.minus = true,
                        '+' => spec.plus = true,
                        ' ' => spec.space = true,
                        '0' => spec.zero = true,
                        '#' => spec.alternate = true,
                        _ => break,
                    }
                    i += 1;
                }
                let number = |i: &mut usize| -> Result<Option<usize>> {
                    let start = *i;
                    while chars.get(*i).is_some_and(char::is_ascii_digit) {
                        *i += 1;
                    }
                    if chars.get(*i) == Some(&'*') {
                        return Err("error: '*' width or precision is not supported yet".into());
                    }
                    if *i == start {
                        return Ok(None);
                    }
                    chars[start..*i]
                        .iter()
                        .collect::<String>()
                        .parse::<usize>()
                        .map(Some)
                        .map_err(|_| "error: format width is too large".to_owned())
                };
                spec.width = number(&mut i)?.unwrap_or(0);
                if chars.get(i) == Some(&'.') {
                    i += 1;
                    spec.precision = Some(number(&mut i)?.unwrap_or(0));
                }
                if spec.width > 4096 || spec.precision.is_some_and(|p| p > 4096) {
                    return Err("error: format width or precision exceeds 4096".into());
                }
                if matches!(chars.get(i), Some('l' | 'h')) {
                    i += 1;
                }
                let Some(&conversion) = chars.get(i) else {
                    out.push('%');
                    break;
                };
                i += 1;
                spec.conversion = conversion;
                if !"diufFeEgGsxXoc".contains(conversion) {
                    return Err(format!(
                        "error: unsupported format conversion '%{conversion}'"
                    ));
                }
                let Some(atom) = atoms.pop_front() else {
                    // Output stops at the first conversion that has no data.
                    return Ok(out);
                };
                consumed = true;
                let text = match (conversion, atom) {
                    ('s', Atom::Text(s)) => {
                        let s = match spec.precision {
                            Some(p) => s.chars().take(p).collect(),
                            None => s,
                        };
                        spec.pad(s, false)
                    }
                    ('s', Atom::Number(v)) => {
                        let body = if v.is_finite()
                            && v == v.trunc()
                            && (0. ..1114112.).contains(&v)
                            && char::from_u32(v as u32).is_some()
                        {
                            char::from_u32(v as u32).unwrap().to_string()
                        } else {
                            format_g(v, spec.precision, false, false)
                        };
                        spec.pad(body, false)
                    }
                    ('c', Atom::Text(s)) => {
                        let mut it = s.chars();
                        let first = it.next().map(String::from).unwrap_or_default();
                        let rest: String = it.collect();
                        if !rest.is_empty() {
                            atoms.push_front(Atom::Text(rest));
                        }
                        spec.pad(first, false)
                    }
                    ('c', Atom::Number(v)) => {
                        let body = if v.is_finite() && v == v.trunc() && v >= 0. {
                            char::from_u32(v as u32)
                                .map(String::from)
                                .unwrap_or_default()
                        } else {
                            format_g(v, spec.precision, false, false)
                        };
                        spec.pad(body, false)
                    }
                    (_, Atom::Number(v)) => spec.number(v),
                    (_, Atom::Text(s)) => {
                        // A character argument under a numeric conversion supplies
                        // its code units one at a time. `atoms()` already counted
                        // every unit against the argument bound.
                        let mut units = s.chars();
                        match units.next() {
                            None => String::new(),
                            Some(first) => {
                                let rest: Vec<char> = units.collect();
                                for chunk in rest.chunks(4096).rev() {
                                    self.check_cancel()?;
                                    for c in chunk.iter().rev() {
                                        atoms.push_front(Atom::Number(f64::from(u32::from(*c))));
                                    }
                                }
                                spec.number(f64::from(u32::from(first)))
                            }
                        }
                    }
                };
                out.push_str(&text);
                if out.len() > MAX_MESSAGE_BYTES {
                    return Err("error: message exceeds 65536 bytes".into());
                }
            }
            if atoms.is_empty() || !consumed || !had_args {
                break;
            }
        }
        Ok(out)
    }
    /// message and identifier for the error() argument forms.
    fn error_parts(&self, args: &[Value]) -> Result<(String, String)> {
        let Some(first) = args.first() else {
            return Err("error: not enough input arguments".into());
        };
        if let Value::Container(Array::Struct(s)) = first {
            if args.len() != 1 {
                return Err("error: a struct message takes no further arguments".into());
            }
            return self.struct_error_parts(s);
        }
        let first_text = self.text_of(first, "error: the message")?;
        if args.len() == 1 {
            if looks_like_identifier(&first_text) {
                return Ok((
                    "If you specify a message identifier argument, you must specify the message text argument.".into(),
                    "MATLAB:error:missingMessageArgument".into(),
                ));
            }
            return Ok((first_text, String::new()));
        }
        let (identifier, template, rest) = if looks_like_identifier(&first_text) {
            (
                first_text,
                self.text_of(&args[1], "error: the message format")?,
                &args[2..],
            )
        } else {
            (String::new(), first_text, &args[1..])
        };
        Ok((self.error_format_text(&template, rest)?, identifier))
    }
    fn struct_error_parts(&self, s: &StructArray) -> Result<(String, String)> {
        if s.len() != 1 {
            return Err("error: the struct argument must be 1x1".into());
        }
        let field = |name: &str| -> Result<Option<String>> {
            let Some(index) = s.field_index(name) else {
                return Ok(None);
            };
            match s.get(index, 0)?.as_ref() {
                Array::Character(c) => {
                    Ok(Some(self.char_text(c, &format!("error: field '{name}'"))?))
                }
                _ => Err(format!("error: field '{name}' must be character text")),
            }
        };
        let message = field("message")?.ok_or("error: the struct needs a 'message' field")?;
        Ok((message, field("identifier")?.unwrap_or_default()))
    }
    pub(super) fn error_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        self.check_cancel()?;
        match name {
            "error" | "rethrow" => {
                let (message, identifier) = if name == "rethrow" {
                    let [Value::Container(Array::Struct(s))] = args else {
                        return Err(
                            "rethrow requires an error struct with message and identifier fields"
                                .into(),
                        );
                    };
                    self.struct_error_parts(s)
                        .map_err(|e| e.replacen("error:", "rethrow:", 1))?
                } else {
                    self.error_parts(args)?
                };
                if message.len() > MAX_MESSAGE_BYTES {
                    return Err("error: message exceeds 65536 bytes".into());
                }
                // MATLAB: if the message is empty, error takes no action
                // (rethrow keeps raising).
                if name == "error" && message.is_empty() {
                    return Ok(Some(Value::Void));
                }
                Err(self.raise(message, identifier))
            }
            _ => Ok(None),
        }
    }
}
