//! MATLAB-style command-window text for structs.
use crate::display_format::{
    DisplayFormat, MAX_TEXT_CHARS, Style, cell_inline, complex_scalar_text, scalar_text, wrap,
};
use crate::{Array, StructArray};

pub(crate) use crate::display_format::exponent;

fn dims(a: &Array) -> String {
    format!("{}x{}", a.rows(), a.cols())
}

fn text(units: &[u16]) -> String {
    let mut s = String::from_utf16_lossy(&units[..units.len().min(MAX_TEXT_CHARS)]);
    if units.len() > MAX_TEXT_CHARS {
        s.push_str("...");
    }
    s.replace('\'', "''")
}

/// One field value as shown after `name: `. A row vector or row cell is shown
/// inline when its text fits in `room` characters (`[1 2 3 ... 20]`, Q1;
/// the cut-off is a hypothesis, `probe3_cases.m` settles it), else as
/// `[1x30 double]`.
fn summary(a: &Array, room: usize, st: &Style) -> String {
    let inline = |text: String| (text.chars().count() <= room).then_some(text);
    // Every inline element takes at least two characters.
    let max_elements = room / 2;
    let shown = match a {
        Array::Numeric(m) if m.len() == 1 => return scalar_text(m.data()[0], st),
        Array::Numeric(m) if m.rows() == 0 && m.cols() == 0 => return "[]".into(),
        Array::Numeric(m) if m.rows() == 1 && !m.is_empty() && m.len() <= max_elements => {
            let parts: Vec<_> = m.data().iter().map(|v| scalar_text(*v, st)).collect();
            inline(format!("[{}]", parts.join(" ")))
        }
        Array::Logical(m) if m.len() == 1 => return format!("{}", m.data()[0]),
        Array::Logical(m) if m.rows() == 1 && !m.is_empty() && m.len() <= max_elements => {
            let parts: Vec<_> = m.data().iter().map(|v| v.to_string()).collect();
            inline(format!("[{}]", parts.join(" ")))
        }
        Array::Cell(c) => cell_inline(c, max_elements, st).and_then(inline),
        _ => None,
    };
    if let Some(text) = shown {
        return text;
    }
    match a {
        Array::Complex(m) if m.len() == 1 => match m.get(0, 0) {
            Ok(z) => complex_scalar_text(z, st),
            Err(_) => "[1x1 double]".into(),
        },
        Array::Character(c) if c.rows() == 1 || (c.rows(), c.cols()) == (0, 0) => {
            format!("'{}'", text(c.units()))
        }
        Array::Cell(_) => format!("{{{} cell}}", dims(a)),
        Array::Handle(handle) => {
            let display = handle.display();
            let mut summary: String = display.chars().take(MAX_TEXT_CHARS).collect();
            if summary.len() < display.len() {
                summary.push_str("...");
            }
            summary
        }
        _ => format!("[{} {}]", dims(a), a.class()),
    }
}

fn field_lines(s: &StructArray, fmt: &DisplayFormat, out: &mut String) {
    let st = fmt.style();
    let width = s.field_names().iter().map(String::len).max().unwrap_or(0);
    let room = fmt.width.saturating_sub(width + 6);
    for (f, name) in s.field_names().iter().enumerate() {
        out.push_str(&" ".repeat(4 + width - name.len()));
        out.push_str(name);
        out.push_str(": ");
        match s.get(f, 0) {
            Ok(value) => out.push_str(&summary(value, room, &st)),
            Err(_) => out.push_str("[]"),
        }
        out.push('\n');
    }
}

fn array_header(s: &StructArray, fmt: &DisplayFormat, out: &mut String) {
    out.push_str(&format!("{}x{} struct array", s.rows(), s.cols()));
    if s.field_count() == 0 {
        out.push_str(" with no fields.\n");
        return;
    }
    out.push_str(" with fields:\n");
    if !fmt.compact {
        out.push('\n');
    }
    let width = s.field_names().iter().map(String::len).max().unwrap_or(0);
    for name in s.field_names() {
        out.push_str(&" ".repeat(4 + width - name.len()));
        out.push_str(name);
        out.push('\n');
    }
}

/// What `disp(s)` prints (without the final newline the console adds): the
/// field list or the array header, then a blank line in loose format.
pub fn disp(s: &StructArray, fmt: &DisplayFormat) -> String {
    let mut out = String::new();
    if s.len() == 1 {
        if s.field_count() == 0 {
            return "struct with no fields.".into();
        }
        field_lines(s, fmt, &mut out);
    } else {
        array_header(s, fmt, &mut out);
    }
    if fmt.compact {
        out.pop();
    }
    out
}

/// What `name = ...` prints for an unsuppressed assignment; ends with the blank
/// line MATLAB leaves after the listing (the console adds the last newline).
pub fn named(name: &str, s: &StructArray, fmt: &DisplayFormat) -> String {
    let mut body = String::new();
    if s.len() == 1 {
        if s.field_count() == 0 {
            body.push_str("struct with no fields.\n");
        } else {
            field_lines(s, fmt, &mut body);
        }
    } else {
        array_header(s, fmt, &mut body);
    }
    wrap(name, true, &body, fmt)
}
