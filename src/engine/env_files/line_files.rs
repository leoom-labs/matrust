//! `readlines` and `writelines`: bounded text line I/O with string arrays.
use super::super::{Engine, Value};
use crate::engine::text_format::unescape;
use crate::{Array, Result, StringArray, script_containers::MAX_CHARACTER_UNITS};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    sync::Arc,
};

const MAX_LINE_FILE_BYTES: usize = 4 * MAX_CHARACTER_UNITS;

#[derive(Clone, Copy)]
enum WhitespaceRule {
    Preserve,
    Trim,
    TrimLeading,
    TrimTrailing,
}

#[derive(Clone, Copy)]
enum EmptyLineRule {
    Read,
    Skip,
    Error,
}

#[derive(Clone, Copy)]
enum TextEncoding {
    Utf8,
    Ascii,
    Latin1,
}

impl TextEncoding {
    fn parse(who: &str, name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" | "system" => Ok(Self::Utf8),
            "us-ascii" | "ascii" => Ok(Self::Ascii),
            "iso-8859-1" | "iso8859-1" | "latin1" => Ok(Self::Latin1),
            _ => Err(format!(
                "{who}: encoding '{name}' is not supported yet (supported: UTF-8, US-ASCII, ISO-8859-1, system)"
            )),
        }
    }

    fn decode(self, who: &str, data: Vec<u8>) -> Result<String> {
        match self {
            Self::Utf8 => {
                let text = String::from_utf8(data)
                    .map_err(|_| format!("{who}: file is not valid UTF-8 text"))?;
                Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
            }
            Self::Ascii => {
                if data.iter().any(|&b| b >= 0x80) {
                    return Err(format!("{who}: file contains non-ASCII bytes"));
                }
                Ok(String::from_utf8(data).expect("ASCII is valid UTF-8"))
            }
            Self::Latin1 => Ok(data.into_iter().map(char::from).collect()),
        }
    }

    fn encode(self, who: &str, text: &str, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Utf8 => {
                out.extend_from_slice(text.as_bytes());
                Ok(())
            }
            Self::Ascii => {
                if !text.is_ascii() {
                    return Err(format!("{who}: text contains non-ASCII characters"));
                }
                out.extend_from_slice(text.as_bytes());
                Ok(())
            }
            Self::Latin1 => {
                for ch in text.chars() {
                    let code = ch as u32;
                    if code > 0xFF {
                        return Err(format!(
                            "{who}: character U+{code:04X} cannot be encoded as ISO-8859-1"
                        ));
                    }
                    out.push(code as u8);
                }
                Ok(())
            }
        }
    }
}

fn parse_pairs<'a>(
    who: &str,
    engine: &Engine,
    args: &'a [Value],
) -> Result<Vec<(String, &'a Value)>> {
    if !args.len().is_multiple_of(2) {
        return Err(format!("{who}: name-value arguments must come in pairs"));
    }
    let mut pairs = Vec::with_capacity(args.len() / 2);
    for [key, value] in args.as_chunks::<2>().0 {
        let name = engine
            .text_value(key.clone())
            .map_err(|_| format!("{who}: parameter name must be text"))?;
        pairs.push((name, value));
    }
    Ok(pairs)
}

fn cell_element_text(engine: &Engine, cell: &Array, err_msg: &str) -> Result<String> {
    match cell {
        Array::Character(c) => c.to_utf8(&engine.cancel),
        Array::Str(sa) if sa.len() == 1 => sa.data()[0]
            .as_ref()
            .map(|s| s.to_string())
            .ok_or_else(|| err_msg.to_owned()),
        _ => Err(err_msg.to_owned()),
    }
}

fn parse_line_endings(engine: &Engine, value: &Value) -> Result<Vec<String>> {
    let mut endings = match value {
        Value::Text(s) | Value::QuotedString(s) => vec![s.clone()],
        Value::Container(Array::Character(c)) => vec![c.to_utf8(&engine.cancel)?],
        Value::Container(Array::Str(sa)) => {
            let mut out = Vec::with_capacity(sa.len());
            for item in sa.data() {
                let Some(s) = item else {
                    return Err("readlines: LineEnding cannot contain <missing>".into());
                };
                out.push(s.to_string());
            }
            out
        }
        Value::Container(Array::Cell(ca)) => {
            let mut out = Vec::with_capacity(ca.len());
            for i in 0..ca.len() {
                let cell = ca.get_linear(i)?;
                out.push(cell_element_text(
                    engine,
                    cell,
                    "readlines: LineEnding cell elements must be text",
                )?);
            }
            out
        }
        _ => return Err("readlines: LineEnding must be text or a string/cell array".into()),
    };
    for ending in &mut endings {
        *ending = unescape(ending);
    }
    if endings.iter().any(|e| e == "\r\n") {
        endings.extend(["\r".to_owned(), "\n".to_owned()]);
    }
    if endings.is_empty() || endings.iter().any(|e| e.is_empty()) {
        return Err("readlines: LineEnding must be non-empty".into());
    }
    // Longest-first ensures "\r\n" matches before "\r" or "\n".
    endings.sort_by_key(|e| std::cmp::Reverse(e.len()));
    Ok(endings)
}

fn split_lines<'a>(text: &'a str, endings: &[String]) -> Vec<&'a str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    while pos < text.len() {
        if let Some(ending) = endings.iter().find(|e| text[pos..].starts_with(e.as_str())) {
            lines.push(&text[start..pos]);
            pos += ending.len();
            start = pos;
        } else {
            let ch_len = text[pos..].chars().next().map_or(1, char::len_utf8);
            pos += ch_len;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

impl Engine {
    pub(super) fn envx_readlines(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        let filename = self.envx_string(&args[0], "Filename must be a string.")?;
        let mut ws = WhitespaceRule::Preserve;
        let mut empty = EmptyLineRule::Read;
        let mut endings = vec!["\r\n".to_owned(), "\n".to_owned(), "\r".to_owned()];
        let mut encoding = TextEncoding::Utf8;

        for (opt, val) in parse_pairs("readlines", self, &args[1..])? {
            match opt.to_ascii_lowercase().as_str() {
                "whitespacerule" => {
                    let v = self
                        .text_value(val.clone())
                        .map_err(|_| "readlines: WhitespaceRule must be text")?;
                    ws = match v.to_ascii_lowercase().as_str() {
                        "preserve" => WhitespaceRule::Preserve,
                        "trim" => WhitespaceRule::Trim,
                        "trimleading" => WhitespaceRule::TrimLeading,
                        "trimtrailing" => WhitespaceRule::TrimTrailing,
                        _ => {
                            return Err(format!(
                                "readlines: invalid WhitespaceRule '{v}'; expected 'preserve', 'trim', 'trimleading', or 'trimtrailing'"
                            ));
                        }
                    };
                }
                "emptylinerule" => {
                    let v = self
                        .text_value(val.clone())
                        .map_err(|_| "readlines: EmptyLineRule must be text")?;
                    empty = match v.to_ascii_lowercase().as_str() {
                        "read" => EmptyLineRule::Read,
                        "skip" => EmptyLineRule::Skip,
                        "error" => EmptyLineRule::Error,
                        _ => {
                            return Err(format!(
                                "readlines: invalid EmptyLineRule '{v}'; expected 'read', 'skip', or 'error'"
                            ));
                        }
                    };
                }
                "lineending" => {
                    endings = parse_line_endings(self, val)?;
                }
                "encoding" => {
                    let v = self
                        .text_value(val.clone())
                        .map_err(|_| "readlines: Encoding must be text")?;
                    encoding = TextEncoding::parse("readlines", &v)?;
                }
                _ => {
                    return Err(format!(
                        "readlines: unsupported parameter '{opt}'; supported are WhitespaceRule, EmptyLineRule, LineEnding, Encoding"
                    ));
                }
            }
        }

        let path = self.envx_read_path(&filename)?;
        let meta = std::fs::metadata(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("Could not open file {filename}. No such file or directory.")
            } else {
                format!("Could not open file {filename}. {e}")
            }
        })?;
        if !meta.is_file() {
            return Err(format!(
                "Could not open file {filename}. Not a regular file."
            ));
        }
        let mut file = File::open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("Could not open file {filename}. No such file or directory.")
            } else {
                format!("Could not open file {filename}. {e}")
            }
        })?;
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err(format!(
                "Could not open file {filename}. Not a regular file."
            ));
        }
        let mut data = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            self.check_cancel()?;
            let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if data.len() + n > MAX_LINE_FILE_BYTES {
                return Err("readlines: file exceeds text size limit".into());
            }
            data.extend_from_slice(&chunk[..n]);
        }

        let text = encoding.decode("readlines", data)?;
        let raw_lines = split_lines(&text, &endings);
        let mut out = Vec::with_capacity(raw_lines.len());
        for (idx, line) in raw_lines.into_iter().enumerate() {
            if idx.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let line = match ws {
                WhitespaceRule::Preserve => line,
                WhitespaceRule::Trim => line.trim(),
                WhitespaceRule::TrimLeading => line.trim_start(),
                WhitespaceRule::TrimTrailing => line.trim_end(),
            };
            if line.is_empty() {
                match empty {
                    EmptyLineRule::Read => {}
                    EmptyLineRule::Skip => continue,
                    EmptyLineRule::Error => {
                        return Err("readlines: empty line encountered".into());
                    }
                }
            }
            out.push(Some(Arc::from(line)));
        }

        Ok(Value::Container(Array::Str(StringArray::new(
            out.len(),
            1,
            out,
        )?)))
    }

    pub(super) fn envx_writelines(&self, args: &[Value]) -> Result<()> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let lines = self.envx_writelines_lines(&args[0])?;
        let filename = self.envx_string(&args[1], "Filename must be a string.")?;
        let mut append = false;
        let mut line_ending = "\n".to_owned();
        let mut encoding = TextEncoding::Utf8;

        for (opt, val) in parse_pairs("writelines", self, &args[2..])? {
            match opt.to_ascii_lowercase().as_str() {
                "writemode" => {
                    let v = self
                        .text_value(val.clone())
                        .map_err(|_| "writelines: WriteMode must be text")?;
                    append = match v.to_ascii_lowercase().as_str() {
                        "overwrite" => false,
                        "append" => true,
                        _ => {
                            return Err(format!(
                                "writelines: invalid WriteMode '{v}'; expected 'overwrite' or 'append'"
                            ));
                        }
                    };
                }
                "lineending" => {
                    line_ending = self
                        .text_value(val.clone())
                        .map_err(|_| "writelines: LineEnding must be text")?;
                    line_ending = unescape(&line_ending);
                }
                "encoding" => {
                    let v = self
                        .text_value(val.clone())
                        .map_err(|_| "writelines: Encoding must be text")?;
                    encoding = TextEncoding::parse("writelines", &v)?;
                }
                _ => {
                    return Err(format!(
                        "writelines: unsupported parameter '{opt}'; supported are WriteMode, LineEnding, Encoding"
                    ));
                }
            }
        }

        let mut bytes = Vec::new();
        for (idx, line) in lines.iter().enumerate() {
            if idx.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            encoding.encode("writelines", line, &mut bytes)?;
            encoding.encode("writelines", &line_ending, &mut bytes)?;
            if bytes.len() > MAX_LINE_FILE_BYTES {
                return Err("writelines: output exceeds text size limit".into());
            }
        }

        let path = self.envx_path(&filename)?;
        match std::fs::metadata(&path) {
            Ok(m) if !m.is_file() => {
                return Err(format!(
                    "writelines: unable to open {filename}: Not a regular file"
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("writelines: unable to open {filename}: {e}")),
        }
        let mut file = if append {
            OpenOptions::new().create(true).append(true).open(&path)
        } else {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
        }
        .map_err(|e| format!("writelines: unable to open {filename}: {e}"))?;
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err(format!(
                "writelines: unable to open {filename}: Not a regular file"
            ));
        }
        file.write_all(&bytes)
            .map_err(|e| format!("writelines: unable to write {filename}: {e}"))?;
        Ok(())
    }

    fn envx_writelines_lines(&self, value: &Value) -> Result<Vec<String>> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => Ok(vec![s.clone()]),
            Value::Container(Array::Str(sa)) => {
                let mut out = Vec::with_capacity(sa.len());
                for item in sa.data() {
                    let Some(s) = item else {
                        return Err("writelines: missing string values are not supported".into());
                    };
                    out.push(s.to_string());
                }
                Ok(out)
            }
            Value::Container(Array::Character(c)) => {
                if c.is_empty() {
                    return Ok(Vec::new());
                }
                if c.is_nd() {
                    return Err("writelines: character array must be 2-D".into());
                }
                let rows = c.rows();
                let cols = c.cols();
                let units = c.units();
                let mut out = Vec::with_capacity(rows);
                for r in 0..rows {
                    self.check_cancel()?;
                    let row_units: Vec<u16> = (0..cols).map(|col| units[col * rows + r]).collect();
                    let mut s = String::with_capacity(cols);
                    for ch in char::decode_utf16(row_units) {
                        s.push(ch.map_err(|_| "writelines: unpaired UTF-16 surrogate")?);
                    }
                    out.push(s);
                }
                Ok(out)
            }
            Value::Container(Array::Cell(ca)) => {
                let mut out = Vec::with_capacity(ca.len());
                for i in 0..ca.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    let cell = ca.get_linear(i)?;
                    out.push(cell_element_text(
                        self,
                        cell,
                        "writelines: cell array elements must be character vectors or strings",
                    )?);
                }
                Ok(out)
            }
            _ => Err(
                "writelines: first input must be a string array, character array, or cell array of character vectors"
                    .into(),
            ),
        }
    }
}
