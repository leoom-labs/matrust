//! detectImportOptions: the text tokenizer and the detection of delimiter,
//! header lines, variable-names line, units line, data start and column
//! types, for delimited text and for spreadsheets.
use super::read::{self, Range, parse_number};
use super::{
    ImportOpts, SheetSel, VarOpts, VarType, choice, count_value, escape, flag_value, numbers,
    text_list, text_of, unescape,
};
use crate::Result;
use crate::engine::{Engine, Value};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

/// Largest delimited text file read (as readcell).
pub(super) const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;
/// Field cap for one split, so a long run of delimiters cannot exhaust memory
/// before the reader's cell limit (same size) is checked.
const MAX_FIELDS: usize = 16 * 1024 * 1024;
/// Data lines examined to choose column types.
const SAMPLE_ROWS: usize = 4096;
/// Lines examined to choose the delimiter.
const DELIMITER_LINES: usize = 64;

// ---------------------------------------------------------------------------
// Tokenizer

/// One field of a text record: its raw text and whether it was quoted.
#[derive(Clone, Debug)]
pub(super) struct Field {
    pub(super) text: String,
    pub(super) quoted: bool,
}

/// One record: its first physical line (1-based) and its fields; None for
/// a blank line.
#[derive(Clone, Debug)]
pub(super) struct Record {
    pub(super) line: usize,
    pub(super) fields: Option<Vec<Field>>,
}

/// How a text is split into records and fields.
pub(super) struct Splitter {
    /// Unescaped delimiters, longest first.
    pub(super) delimiters: Vec<String>,
    pub(super) whitespace: Vec<char>,
    pub(super) consecutive: String,
    pub(super) leading: String,
    pub(super) trailing: String,
    pub(super) comments: Vec<String>,
}

impl Splitter {
    pub(super) fn from_opts(o: &ImportOpts) -> Self {
        let mut delimiters: Vec<String> = o.delimiter.iter().map(|d| unescape(d)).collect();
        delimiters.sort_by_key(|d| std::cmp::Reverse(d.len()));
        let whitespace = unescape(&o.whitespace)
            .chars()
            .filter(|c| {
                !(*c == '\t' && delimiters.iter().any(|d| d == " "))
                    && !delimiters
                        .iter()
                        .any(|d| d.chars().count() == 1 && d.starts_with(*c))
            })
            .collect();
        Self {
            delimiters,
            whitespace,
            consecutive: o.consecutive_rule.clone(),
            leading: o.leading_rule.clone(),
            trailing: o.trailing_rule.clone(),
            comments: o.comment_style.clone(),
        }
    }
    fn delimiter_at(&self, rest: &str) -> Option<usize> {
        self.delimiters
            .iter()
            .find(|d| rest.starts_with(d.as_str()))
            .map(String::len)
            .or_else(|| {
                (rest.starts_with('\t') && self.delimiters.iter().any(|d| d == " ")).then_some(1)
            })
    }
    fn is_space(&self, c: char) -> bool {
        self.whitespace.contains(&c)
    }
}

/// Records of `text`, one per physical line outside quotes. A quoted field
/// may span lines; `""` inside quotes is a literal quote and text after the
/// closing quote, up to the delimiter, is dropped.
/// At most `max_records` records are made (detection reads a sample).
pub(super) fn split(
    text: &str,
    s: &Splitter,
    max_records: Option<usize>,
    cancel: &AtomicBool,
) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    split_each(text, s, max_records, cancel, |line, fields| {
        records.push(Record {
            line,
            fields: fields.map(|f| f.to_vec()),
        });
        Ok(())
    })?;
    Ok(records)
}

/// Stream records of `text` to `on_record` without allocating a `Vec<Field>` per line.
pub(super) fn split_each(
    text: &str,
    s: &Splitter,
    max_records: Option<usize>,
    cancel: &AtomicBool,
    mut on_record: impl FnMut(usize, Option<&[Field]>) -> Result<()>,
) -> Result<()> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut line = 1;
    let mut steps = 0usize;
    let mut emitted = 0usize;
    let mut total_fields = 0usize;
    let mut fields: Vec<Field> = Vec::new();
    let mut field = String::new();
    while at < bytes.len() {
        if max_records.is_some_and(|m| emitted >= m) {
            break;
        }
        steps += 1;
        if steps.is_multiple_of(4096) {
            crate::dsp::check_cancel(cancel)?;
        }
        let start_line = line;
        // Comment lines are skipped (they still count as lines).
        let rest = &text[at..];
        let body = rest.trim_start_matches(|c: char| c != '\n' && c != '\r' && s.is_space(c));
        if s.comments
            .iter()
            .any(|c| !c.is_empty() && body.starts_with(c.as_str()))
        {
            let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
            at += end;
            at += line_break(&text[at..]);
            line += 1;
            continue;
        }
        // Each record holds at least one (last) field.
        total_fields += 1;
        if total_fields > MAX_FIELDS {
            return Err(format!(
                "The data has more than {MAX_FIELDS} fields; read fewer DataLines"
            ));
        }
        fields.clear();
        field.clear();
        let mut quoted = false;
        let mut content = false;
        let mut delimited = false;
        let mut leading = true;
        loop {
            let rest = &text[at..];
            let Some(c) = rest.chars().next() else { break };
            if c == '\n' || c == '\r' {
                break;
            }
            if c == '"' && !quoted && field.chars().all(|c| s.is_space(c)) {
                field.clear();
                quoted = true;
                content = true;
                at += 1;
                // Quoted text up to the closing quote.
                loop {
                    let rest = &text[at..];
                    let Some(q) = rest.chars().next() else { break };
                    at += q.len_utf8();
                    if q == '"' {
                        if text[at..].starts_with('"') {
                            at += 1;
                            field.push('"');
                            continue;
                        }
                        break;
                    }
                    if q == '\n' || (q == '\r' && !text[at..].starts_with('\n')) {
                        line += 1;
                    }
                    field.push(q);
                }
                // Text after the closing quote, before the delimiter, is dropped.
                while let Some(c) = text[at..].chars().next() {
                    if c == '\n' || c == '\r' || s.delimiter_at(&text[at..]).is_some() {
                        break;
                    }
                    at += c.len_utf8();
                }
                continue;
            }
            if let Some(len) = s.delimiter_at(rest) {
                at += len;
                let empty = !quoted && field.chars().all(|c| s.is_space(c));
                if leading && empty && fields.is_empty() {
                    match s.leading.as_str() {
                        "ignore" => {
                            field.clear();
                            continue;
                        }
                        "error" => {
                            return Err(format!(
                                "Line {start_line} starts with a delimiter (LeadingDelimitersRule is 'error')"
                            ));
                        }
                        _ => {}
                    }
                }
                delimited = true;
                leading = false;
                if s.consecutive == "join" || s.consecutive == "error" {
                    // Runs of delimiters (with whitespace between) are one.
                    let mut probe = at;
                    let mut joined = false;
                    loop {
                        let r = &text[probe..];
                        let skip = r.len()
                            - r.trim_start_matches(|c: char| {
                                c != '\n' && c != '\r' && s.is_space(c)
                            })
                            .len();
                        match s.delimiter_at(&r[skip..]) {
                            Some(l) => {
                                probe += skip + l;
                                joined = true;
                            }
                            None => break,
                        }
                    }
                    if joined {
                        if s.consecutive == "error" {
                            return Err(format!(
                                "Line {start_line} has consecutive delimiters (ConsecutiveDelimitersRule is 'error')"
                            ));
                        }
                        at = probe;
                    }
                }
                content = true;
                total_fields += 1;
                if total_fields.is_multiple_of(4096) {
                    crate::dsp::check_cancel(cancel)?;
                }
                if total_fields > MAX_FIELDS {
                    return Err(format!(
                        "The data has more than {MAX_FIELDS} fields; read fewer DataLines"
                    ));
                }
                fields.push(Field {
                    text: std::mem::take(&mut field),
                    quoted,
                });
                quoted = false;
                continue;
            }
            if !s.is_space(c) {
                content = true;
                leading = false;
            }
            field.push(c);
            at += c.len_utf8();
        }
        at += line_break(&text[at..]);
        line += 1;
        if !content && !delimited {
            emitted += 1;
            on_record(start_line, None)?;
            continue;
        }
        fields.push(Field {
            text: std::mem::take(&mut field),
            quoted,
        });
        // A delimiter at the end of the line.
        if delimited
            && fields
                .last()
                .is_some_and(|f| !f.quoted && f.text.chars().all(|c| s.is_space(c)))
        {
            match s.trailing.as_str() {
                "ignore" => {
                    fields.pop();
                }
                "error" => {
                    return Err(format!(
                        "Line {start_line} ends with a delimiter (TrailingDelimitersRule is 'error')"
                    ));
                }
                _ => {}
            }
        }
        emitted += 1;
        on_record(start_line, Some(&fields))?;
    }
    Ok(())
}

/// Length of the line break at the start of `rest` (0, 1 or 2 bytes).
fn line_break(rest: &str) -> usize {
    if rest.starts_with("\r\n") {
        2
    } else if rest.starts_with(['\n', '\r']) {
        1
    } else {
        0
    }
}

/// A Delimiter option value as the delimiter text: names (comma, tab,
/// semi, space, bar) or the text with escapes replaced.
pub(super) fn delimiter_text(d: &str) -> String {
    match d.to_ascii_lowercase().as_str() {
        "comma" => ",".into(),
        "tab" => "\t".into(),
        "semi" | "semicolon" => ";".into(),
        "space" | "whitespace" => " ".into(),
        "bar" => "|".into(),
        _ => unescape(d),
    }
}

/// The delimiter of `text`: among comma, tab, semicolon and bar, the one
/// splitting the most of the first lines into the same number (two or
/// more) of fields; space only when none of them does.
pub(super) fn detect_delimiter(text: &str, skip: usize) -> &'static str {
    let lines: Vec<&str> = text
        .lines()
        .skip(skip)
        .filter(|l| !l.trim().is_empty())
        .take(DELIMITER_LINES)
        .collect();
    let count = |line: &str, d: char| -> usize {
        let mut inside = false;
        let mut n = 1;
        let mut previous_space = true;
        let is_delim = |c: char| c == d || (d == ' ' && c == '\t');
        let body = if d == ' ' { line.trim() } else { line };
        for c in body.chars() {
            if c == '"' {
                inside = !inside;
            } else if !inside && is_delim(c) {
                // Spaces join: a run is one delimiter.
                if !(d == ' ' && previous_space) {
                    n += 1;
                }
            }
            previous_space = is_delim(c);
        }
        n
    };
    let score = |d: char| -> (usize, usize) {
        let counts: Vec<usize> = lines.iter().map(|l| count(l, d)).collect();
        let mut best = (0, 0);
        for &n in &counts {
            if n < 2 {
                continue;
            }
            let lines_with = counts.iter().filter(|&&m| m == n).count();
            if (lines_with, n) > best {
                best = (lines_with, n);
            }
        }
        best
    };
    let mut best = (",", (0, 0));
    for (name, d) in [(",", ','), ("\t", '\t'), (";", ';'), ("|", '|')] {
        let s = score(d);
        if s > best.1 {
            best = (name, s);
        }
    }
    let space_score = score(' ');
    if (best.1.0 == 0 || (best.0 == "\t" && best.1.0 == 1 && space_score.0 > 1))
        && space_score.0 > 0
    {
        return " ";
    }
    best.0
}

// ---------------------------------------------------------------------------
// Cells and their classes

/// One cell for detection: text trimmed and unquoted, or a spreadsheet value.
#[derive(Clone, Debug)]
pub(super) enum Cell {
    Empty,
    Num,
    Bool,
    Text(String),
}

/// Datetime forms detected without an InputFormat, most specific first;
/// month/day order follows the en_US locale (MM/dd before dd/MM).
pub(super) const DATE_FORMATS: &[&str] = &[
    "yyyy-MM-dd'T'HH:mm:ss.SSS",
    "yyyy-MM-dd'T'HH:mm:ss",
    "yyyy-MM-dd'T'HH:mm",
    "yyyy-MM-dd HH:mm:ss.SSS",
    "yyyy-MM-dd HH:mm:ss",
    "yyyy-MM-dd HH:mm",
    "yyyy-MM-dd",
    "yyyy/MM/dd HH:mm:ss",
    "yyyy/MM/dd HH:mm",
    "yyyy/MM/dd",
    "MM/dd/yyyy HH:mm:ss",
    "MM/dd/yyyy HH:mm",
    "MM/dd/yyyy",
    "dd/MM/yyyy HH:mm:ss",
    "dd/MM/yyyy HH:mm",
    "dd/MM/yyyy",
    "dd.MM.yyyy HH:mm:ss",
    "dd.MM.yyyy HH:mm",
    "dd.MM.yyyy",
    "dd-MMM-yyyy HH:mm:ss",
    "dd-MMM-yyyy HH:mm",
    "dd-MMM-yyyy",
];

pub(super) fn date_tokens() -> &'static [Vec<crate::engine::datetime_fns::text::Tok>] {
    static TOKENS: OnceLock<Vec<Vec<crate::engine::datetime_fns::text::Tok>>> = OnceLock::new();
    TOKENS.get_or_init(|| {
        DATE_FORMATS
            .iter()
            .map(|f| crate::engine::datetime_fns::text::tokens(f).expect("valid format"))
            .collect()
    })
}

/// Bit mask of the [`DATE_FORMATS`] that read `text` (0 when none).
pub(super) fn date_mask(text: &str) -> u32 {
    let t = text.trim();
    // Cheap guard: dates need a digit and a separator.
    if t.len() < 6 || !t.as_bytes()[0].is_ascii_digit() {
        return 0;
    }
    let mut mask = 0;
    for (i, tokens) in date_tokens().iter().enumerate() {
        if crate::engine::datetime_fns::text::parse(tokens, t, 1950, 2000).is_some() {
            mask |= 1 << i;
        }
    }
    mask
}

/// Milliseconds of duration text hh:mm, hh:mm:ss[.SSS] or dd:hh:mm:ss[.SSS].
pub(super) fn duration_ms(text: &str, separator: char) -> Option<f64> {
    let t = text.trim();
    let (negative, body) = match t.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, t),
    };
    let parts: Vec<&str> = body.split(separator).collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    let last = parts.len() - 1;
    let mut values = Vec::with_capacity(parts.len());
    for (i, p) in parts.iter().enumerate() {
        let ok = !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_digit() || (i == last && b == b'.'))
            && p.bytes().next().is_some_and(|b| b.is_ascii_digit());
        if !ok {
            return None;
        }
        values.push(p.parse::<f64>().ok()?);
    }
    // Every field after the first has two digits before any fraction.
    if parts[1..]
        .iter()
        .any(|p| p.split('.').next().is_some_and(|d| d.len() != 2))
    {
        return None;
    }
    let ms = match values.as_slice() {
        [h, m] => (h * 60. + m) * 60_000.,
        [h, m, s] => ((h * 60. + m) * 60. + s) * 1000.,
        [d, h, m, s] => (((d * 24. + h) * 60. + m) * 60. + s) * 1000.,
        _ => return None,
    };
    Some(if negative { -ms } else { ms })
}

/// The class detection gives a column.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Detected {
    Text,
    Number { comma: bool },
    Logical,
    Datetime,
    Duration,
}

/// Column class from its data cells; `missing` lists texts that count as
/// empty, `thousands` is a given ThousandsSeparator.
fn column_class(cells: &[&Cell], missing: &[String], thousands: Option<char>) -> Detected {
    let (mut total, mut dot, mut comma, mut with_comma) = (0usize, 0usize, 0usize, 0usize);
    let (mut text_count, mut bools, mut durations, mut dates) = (0usize, 0usize, 0usize, 0usize);
    let mut mask = u32::MAX;
    for cell in cells {
        match cell {
            Cell::Empty => {}
            Cell::Num => {
                total += 1;
                dot += 1;
                comma += 1;
            }
            Cell::Bool => {
                total += 1;
                bools += 1;
            }
            Cell::Text(t) => {
                if missing.iter().any(|m| m == t) {
                    continue;
                }
                total += 1;
                let as_dot = parse_number(t, '.', grouped(thousands, '.'), "eEdD", false).is_some();
                let as_comma =
                    parse_number(t, ',', grouped(thousands, ','), "eEdD", false).is_some();
                if as_dot {
                    dot += 1;
                }
                if as_comma {
                    comma += 1;
                    if !as_dot && t.contains(',') {
                        with_comma += 1;
                    }
                }
                if !as_dot && !as_comma {
                    let m = date_mask(t);
                    if m != 0 {
                        dates += 1;
                        mask &= m;
                    } else if duration_ms(t, ':').is_some() {
                        durations += 1;
                    } else {
                        text_count += 1;
                    }
                }
            }
        }
    }
    if total == 0 {
        return Detected::Text;
    }
    if bools == total {
        return Detected::Logical;
    }
    if dot + bools == total {
        return Detected::Number { comma: false };
    }
    if comma + bools == total && with_comma > 0 {
        return Detected::Number { comma: true };
    }
    if dates == total && mask != 0 {
        return Detected::Datetime;
    }
    if durations == total {
        return Detected::Duration;
    }
    // Mostly numbers: the other texts become import errors (filled).
    let numbers = dot.max(if with_comma > 0 { comma } else { 0 }) + bools;
    if numbers > 0 && numbers > text_count + dates + durations {
        return Detected::Number {
            comma: with_comma > 0 && comma > dot,
        };
    }
    Detected::Text
}

/// The thousands separator a number with `decimal` can use.
fn grouped(thousands: Option<char>, decimal: char) -> Option<char> {
    thousands.filter(|t| *t != decimal)
}

fn is_number_cell(cell: &Cell, thousands: Option<char>) -> bool {
    match cell {
        Cell::Num | Cell::Bool => true,
        Cell::Text(t) => {
            parse_number(t, '.', grouped(thousands, '.'), "eEdD", false).is_some()
                || parse_number(t, ',', grouped(thousands, ','), "eEdD", false).is_some()
        }
        Cell::Empty => false,
    }
}

/// A row that holds only text (at least one nonempty cell, no numbers).
fn text_row(cells: &[Cell], thousands: Option<char>) -> bool {
    cells.iter().any(|c| matches!(c, Cell::Text(_)))
        && !cells.iter().any(|c| is_number_cell(c, thousands))
}

// ---------------------------------------------------------------------------
// Options given to detectImportOptions (and to readtable)

/// Name/value options of detectImportOptions; readtable and readtimetable
/// pass theirs through here too.
#[derive(Clone, Debug, Default)]
pub(super) struct DetectArgs {
    pub(super) file_type: Option<bool>,
    pub(super) delimiter: Option<Vec<String>>,
    pub(super) header_lines: Option<usize>,
    pub(super) sheet: Option<SheetSel>,
    pub(super) range: Option<Range>,
    pub(super) read_names: Option<bool>,
    pub(super) read_row_names: Option<bool>,
    pub(super) preserve: Option<bool>,
    pub(super) strings: Option<bool>,
    pub(super) datetime_text: Option<bool>,
    pub(super) decimal: Option<String>,
    pub(super) thousands: Option<String>,
    pub(super) treat_as_missing: Option<Vec<String>>,
    pub(super) missing_rule: Option<String>,
    pub(super) import_error_rule: Option<String>,
    pub(super) empty_line_rule: Option<String>,
    pub(super) extra_columns_rule: Option<String>,
    pub(super) whitespace: Option<String>,
    pub(super) comment_style: Option<Vec<String>>,
    pub(super) consecutive: Option<String>,
    pub(super) leading: Option<String>,
    pub(super) trailing: Option<String>,
    pub(super) names_line: Option<usize>,
    pub(super) units_line: Option<usize>,
    pub(super) data_lines: Option<Vec<[f64; 2]>>,
    pub(super) selected: Option<Value>,
    pub(super) row_names_column: Option<usize>,
}

impl DetectArgs {
    /// Takes one name/value pair; false when the name is not one of these.
    pub(super) fn take(&mut self, who: &str, name: &str, value: &Value) -> Result<bool> {
        let text_list_of = |option: &str| {
            text_list(value).ok_or_else(|| {
                format!("{who}: {option} must be text, a cell array of text or a string array")
            })
        };
        match name.to_ascii_lowercase().as_str() {
            "filetype" => {
                let kind = choice(
                    who,
                    "FileType",
                    value,
                    &["text", "delimitedtext", "spreadsheet"],
                )?;
                self.file_type = Some(kind == "spreadsheet");
            }
            "delimiter" => {
                let list = text_list_of("Delimiter")?;
                if list.is_empty() || list.iter().any(String::is_empty) {
                    return Err(format!("{who}: Delimiter cannot be empty"));
                }
                self.delimiter = Some(list.iter().map(|d| delimiter_text(d)).collect());
            }
            "numheaderlines" => {
                self.header_lines = Some(count_value(who, "NumHeaderLines", value)?);
            }
            "sheet" => {
                self.sheet = Some(match text_of(value) {
                    Some(t) => SheetSel::Name(t),
                    None => match numbers(value) {
                        Some((1, 1, d)) if d[0] >= 1. && d[0].fract() == 0. && d[0] <= 1e6 => {
                            SheetSel::Index(d[0] as usize)
                        }
                        _ => {
                            return Err(format!(
                                "{who}: Sheet must be a sheet name or a positive sheet index"
                            ));
                        }
                    },
                });
            }
            "range" => {
                let t = text_of(value).ok_or_else(|| format!("{who}: Range must be text"))?;
                self.range = Some(read::parse_range(&t).ok_or_else(|| {
                    format!(
                        "{who}: Range must be a cell such as 'B2', a rectangle such as 'A1:C10' or a row range such as '2:10'"
                    )
                })?);
            }
            "readvariablenames" => {
                self.read_names = Some(flag_value(who, "ReadVariableNames", value)?);
            }
            "readrownames" => self.read_row_names = Some(flag_value(who, "ReadRowNames", value)?),
            "variablenamingrule" => {
                self.preserve = Some(
                    choice(who, "VariableNamingRule", value, &["modify", "preserve"])?
                        == "preserve",
                );
            }
            "texttype" => {
                self.strings =
                    Some(choice(who, "TextType", value, &["char", "string"])? == "string");
            }
            "datetimetype" => {
                self.datetime_text = Some(
                    choice(
                        who,
                        "DatetimeType",
                        value,
                        &["datetime", "text", "exceldatenum"],
                    )? != "datetime",
                );
            }
            "decimalseparator" => {
                let t = text_of(value)
                    .filter(|t| t.chars().count() == 1)
                    .ok_or_else(|| format!("{who}: DecimalSeparator must be one character"))?;
                self.decimal = Some(t);
            }
            "thousandsseparator" => {
                let t = text_of(value)
                    .filter(|t| t.chars().count() <= 1)
                    .ok_or_else(|| format!("{who}: ThousandsSeparator must be one character"))?;
                self.thousands = Some(t);
            }
            "treatasmissing" => self.treat_as_missing = Some(text_list_of("TreatAsMissing")?),
            "missingrule" => {
                self.missing_rule = Some(choice(
                    who,
                    "MissingRule",
                    value,
                    &["fill", "error", "omitrow", "omitvar"],
                )?);
            }
            "importerrorrule" => {
                self.import_error_rule = Some(choice(
                    who,
                    "ImportErrorRule",
                    value,
                    &["fill", "error", "omitrow", "omitvar"],
                )?);
            }
            "emptylinerule" => {
                self.empty_line_rule = Some(choice(
                    who,
                    "EmptyLineRule",
                    value,
                    &["skip", "read", "error"],
                )?);
            }
            "extracolumnsrule" => {
                self.extra_columns_rule = Some(choice(
                    who,
                    "ExtraColumnsRule",
                    value,
                    &["addvars", "ignore", "wrap", "error"],
                )?);
            }
            "whitespace" => self.whitespace = Some(unescape(&text_list_of("Whitespace")?.concat())),
            "commentstyle" => self.comment_style = Some(text_list_of("CommentStyle")?),
            "consecutivedelimitersrule" => {
                self.consecutive = Some(choice(
                    who,
                    "ConsecutiveDelimitersRule",
                    value,
                    &["split", "join", "error"],
                )?);
            }
            "leadingdelimitersrule" => {
                self.leading = Some(choice(
                    who,
                    "LeadingDelimitersRule",
                    value,
                    &["keep", "ignore", "error"],
                )?);
            }
            "trailingdelimitersrule" => {
                self.trailing = Some(choice(
                    who,
                    "TrailingDelimitersRule",
                    value,
                    &["keep", "ignore", "error"],
                )?);
            }
            "variablenamesline" => {
                self.names_line = Some(count_value(who, "VariableNamesLine", value)?);
            }
            "variableunitsline" => {
                self.units_line = Some(count_value(who, "VariableUnitsLine", value)?);
            }
            "datalines" => {
                self.data_lines = Some(super::data_lines_value(who, value)?);
            }
            "selectedvariablenames" => self.selected = Some(value.clone()),
            "rownamescolumn" => {
                self.row_names_column = Some(count_value(who, "RowNamesColumn", value)?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Detection

/// File name and path, with MATLAB's message for a missing file.
pub(super) fn open(e: &Engine, who: &str, value: &Value) -> Result<(String, std::path::PathBuf)> {
    let filename = text_of(value).ok_or_else(|| {
        format!("{who}: the file name must be a character vector or string scalar")
    })?;
    let path = e.files_read_path(&filename, "")?;
    if !path.is_file() {
        return Err(format!(
            "Unable to find or open '{filename}'. Check the path and filename or file permissions."
        ));
    }
    Ok((filename, path))
}

/// Whether `filename` is read as a spreadsheet.
pub(super) fn is_sheet(filename: &str, file_type: Option<bool>) -> bool {
    file_type.unwrap_or_else(|| {
        std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                ["xlsx", "xlsm", "xlsb", "xls"].contains(&e.to_ascii_lowercase().as_str())
            })
    })
}

/// Text of a delimited file (BOM removed), size-checked.
pub(super) fn file_text(e: &Engine, filename: &str, path: &std::path::Path) -> Result<String> {
    let size = std::fs::metadata(path)
        .map_err(|err| format!("Unable to open '{filename}': {err}"))?
        .len();
    if size > MAX_TEXT_BYTES {
        return Err(format!(
            "'{filename}' exceeds the {} MiB limit for text files",
            MAX_TEXT_BYTES >> 20
        ));
    }
    let bytes = std::fs::read(path).map_err(|err| format!("Unable to read '{filename}': {err}"))?;
    e.check_cancel()?;
    Ok(String::from_utf8_lossy(crate::files::strip_utf8_bom(&bytes)).into_owned())
}

/// `detectImportOptions(file, Name, Value, ...)`.
pub(super) fn detect_import_options(e: &Engine, args: &[Value]) -> Result<ImportOpts> {
    let who = "detectImportOptions";
    let Some((file, rest)) = args.split_first() else {
        return Err("Not enough input arguments.".into());
    };
    if !rest.len().is_multiple_of(2) {
        return Err(format!("{who}: options must be name-value pairs"));
    }
    let mut d = DetectArgs::default();
    for pair in rest.chunks(2) {
        let name = text_of(&pair[0]).ok_or_else(|| format!("{who}: option names must be text"))?;
        if !d.take(who, &name, &pair[1])? {
            return Err(format!("{who}: unsupported option '{name}'"));
        }
    }
    detect(e, who, file, &d)
}

/// Options detected for `file` with the given name/value options applied.
pub(super) fn detect(e: &Engine, who: &str, file: &Value, d: &DetectArgs) -> Result<ImportOpts> {
    let (filename, path) = open(e, who, file)?;
    let mut opts = if is_sheet(&filename, d.file_type) {
        if d.delimiter.is_some() {
            return Err(format!("{who}: Delimiter applies to text files only"));
        }
        detect_sheet(e, &path, d)?
    } else {
        if d.sheet.is_some() {
            return Err(format!("{who}: Sheet applies to spreadsheet files only"));
        }
        let text = file_text(e, &filename, &path)?;
        detect_text(e, &text, d)?
    };
    apply_after(who, &mut opts, d)?;
    Ok(opts)
}

/// Options that only change the detected result.
fn apply_after(who: &str, opts: &mut ImportOpts, d: &DetectArgs) -> Result<()> {
    if let Some(r) = &d.missing_rule {
        opts.missing_rule = r.clone();
    }
    if let Some(r) = &d.import_error_rule {
        opts.import_error_rule = r.clone();
    }
    if let Some(list) = &d.treat_as_missing {
        for v in &mut opts.vars {
            v.treat_as_missing = list.clone();
        }
    }
    if !opts.sheet {
        if let Some(r) = &d.empty_line_rule {
            opts.empty_line_rule = r.clone();
        }
        if let Some(r) = &d.extra_columns_rule {
            opts.extra_columns_rule = r.clone();
        }
        if let Some(k) = d.row_names_column {
            if k > opts.vars.len() {
                return Err(format!(
                    "{who}: RowNamesColumn must be 0 or a column number up to {}",
                    opts.vars.len()
                ));
            }
            opts.row_names_column = k;
        }
    }
    if let Some(selection) = &d.selected {
        let picked = opts.selection(who, selection)?;
        opts.selected = picked;
    }
    Ok(())
}

/// The structure of a sample: rows of cells with their 1-based numbers.
struct Layout {
    names: Option<usize>,
    units: Option<usize>,
    /// Index of the first data row in the sample.
    data: usize,
    width: usize,
    classes: Vec<Detected>,
}

/// Header, names, units and data rows of `rows` (None rows are blank) and
/// the class of every column.
fn layout(
    rows: &[(usize, Option<Vec<Cell>>)],
    d: &DetectArgs,
    forced_start: Option<usize>,
) -> Layout {
    let missing = d.treat_as_missing.clone().unwrap_or_default();
    // Numbers grouped with a given ThousandsSeparator are numbers.
    let thousands = d.thousands.as_deref().and_then(|t| t.chars().next());
    let filled: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].1.is_some()).collect();
    // Fields up to the last nonempty one (a title cell above a table is
    // one field wide).
    let width_of = |i: usize| -> usize {
        rows[i]
            .1
            .as_ref()
            .unwrap()
            .iter()
            .rposition(|c| !matches!(c, Cell::Empty))
            .map_or(0, |p| p + 1)
    };
    // The most common width (wider on ties) of the sample.
    let mut counts = std::collections::BTreeMap::new();
    for &i in filled.iter().take(SAMPLE_ROWS) {
        *counts.entry(width_of(i)).or_insert(0usize) += 1;
    }
    let width = counts
        .iter()
        .max_by_key(|(w, n)| (**n, **w))
        .map_or(0, |(w, _)| *w);
    // The table starts at the first row of that width (or at a given row).
    let start = match forced_start {
        Some(s) => filled.iter().position(|&i| i >= s).unwrap_or(filled.len()),
        None if width <= 1 => 0,
        None => filled
            .iter()
            .position(|&i| width_of(i) == width)
            .unwrap_or(0),
    };
    let table: Vec<usize> = filled[start.min(filled.len())..].to_vec();
    let cells = |i: usize| rows[i].1.as_ref().unwrap();
    let classes_from = |from: usize| -> Vec<Detected> {
        (0..width)
            .map(|c| {
                let column: Vec<&Cell> = table
                    .iter()
                    .skip(from)
                    .take(SAMPLE_ROWS)
                    .filter_map(|&i| cells(i).get(c))
                    .collect();
                column_class(&column, &missing, thousands)
            })
            .collect()
    };
    let read_names = d.read_names;
    let mut names = None;
    let mut data = 0;
    if let Some(&first) = table.first() {
        let is_names = match read_names {
            Some(flag) => flag,
            None => table.len() > 1 && text_row(cells(first), thousands),
        };
        if is_names {
            names = Some(first);
            data = 1;
        }
    }
    let mut units = None;
    if names.is_some()
        && let Some(&candidate) = table.get(1)
        && table.len() > 2
        && text_row(cells(candidate), thousands)
    {
        let after = classes_from(2);
        let typed_text = after.iter().enumerate().any(|(c, k)| {
            !matches!(k, Detected::Text) && matches!(cells(candidate).get(c), Some(Cell::Text(_)))
        });
        if typed_text {
            units = Some(candidate);
            data = 2;
        }
    }
    let classes = classes_from(data);
    Layout {
        names,
        units,
        data: table.get(data).copied().unwrap_or(rows.len()),
        width,
        classes,
    }
}

/// Variable names from the names row: valid (modify) or as written
/// (preserve), Var<k> for empty ones, unique.
pub(super) fn make_names(headers: Vec<Option<String>>, preserve: bool) -> Vec<String> {
    if !preserve {
        return crate::engine::table_fns::csv::variable_names(headers);
    }
    let mut names: Vec<String> = Vec::with_capacity(headers.len());
    for (k, h) in headers.into_iter().enumerate() {
        let base = match h.filter(|h| !h.trim().is_empty()) {
            Some(h) => h,
            None => format!("Var{}", k + 1),
        };
        let mut candidate = base.clone();
        let mut n = 1;
        while names.contains(&candidate) {
            candidate = format!("{base}_{n}");
            n += 1;
        }
        names.push(candidate);
    }
    names
}

fn kind_of(class: Detected, d: &DetectArgs) -> VarType {
    let text = if d.strings == Some(true) {
        VarType::Str
    } else {
        VarType::Char
    };
    match class {
        Detected::Text => text,
        Detected::Number { .. } => VarType::Double,
        Detected::Logical => VarType::Logical,
        Detected::Datetime if d.datetime_text == Some(true) => text,
        Detected::Datetime => VarType::Datetime,
        Detected::Duration => VarType::Duration,
    }
}

fn cell_of(f: &Field, whitespace: &[char]) -> Cell {
    let t = if f.quoted {
        f.text.as_str()
    } else {
        f.text.trim_matches(|c| whitespace.contains(&c))
    };
    if t.is_empty() {
        Cell::Empty
    } else {
        Cell::Text(t.to_owned())
    }
}

/// Text options that determine how records are tokenized.
pub(super) fn text_options(text: &str, d: &DetectArgs) -> ImportOpts {
    let delimiters: Vec<String> = match &d.delimiter {
        Some(list) => list.clone(),
        None => vec![detect_delimiter(text, d.header_lines.unwrap_or(0)).to_owned()],
    };
    let space = delimiters.iter().any(|d| d == " ");
    let mut opts = ImportOpts::new(false, Vec::new());
    opts.delimiter = delimiters.iter().map(|x| escape(x)).collect();
    let whitespace: String = match &d.whitespace {
        Some(w) => w.clone(),
        None => "\u{8}\t "
            .chars()
            .filter(|c| {
                !delimiters
                    .iter()
                    .any(|x| x.chars().count() == 1 && x.starts_with(*c))
            })
            .collect(),
    };
    opts.whitespace = escape(&whitespace);
    if space {
        opts.consecutive_rule = "join".into();
        opts.leading_rule = "ignore".into();
    }
    if let Some(r) = &d.consecutive {
        opts.consecutive_rule = r.clone();
    }
    if let Some(r) = &d.leading {
        opts.leading_rule = r.clone();
    }
    if let Some(r) = &d.trailing {
        opts.trailing_rule = r.clone();
    }
    if let Some(c) = &d.comment_style {
        opts.comment_style = c.clone();
    }
    opts
}

/// Detection on delimited text.
fn detect_text(e: &Engine, text: &str, d: &DetectArgs) -> Result<ImportOpts> {
    let mut opts = text_options(text, d);
    if let Some(p) = d.preserve {
        opts.preserve_names = p;
    }
    let splitter = Splitter::from_opts(&opts);
    // Lines inside a Range, or after the header lines.
    let first_line = match (&d.range, d.header_lines) {
        (Some(r), _) => r.row + 1,
        (None, Some(n)) => n + 1,
        (None, None) => 1,
    };
    // A sample: the lines before the table and enough data lines.
    let wanted = first_line.max(d.names_line.unwrap_or(0)) + 2 * SAMPLE_ROWS;
    let records = split(text, &splitter, Some(wanted), &e.cancel)?;
    let last_line = d.range.and_then(|r| r.last_row.map(|l| l + 1));
    let col0 = d.range.map_or(0, |r| r.col);
    let rows: Vec<(usize, Option<Vec<Cell>>)> = records
        .iter()
        .filter(|r| r.line >= first_line && last_line.is_none_or(|l| r.line <= l))
        .map(|r| {
            (
                r.line,
                r.fields
                    .as_ref()
                    .map(|f| f.iter().map(|x| cell_of(x, &splitter.whitespace)).collect()),
            )
        })
        .collect();
    // A given names line, header line count or range fixes where the table
    // starts; given DataLines only replace the detected ones.
    let forced = d
        .names_line
        .filter(|&n| n > 0)
        .or_else(|| (d.header_lines.is_some() || d.range.is_some()).then_some(first_line))
        .map(|line| rows.iter().position(|r| r.0 >= line).unwrap_or(rows.len()));
    let mut args = d.clone();
    match d.names_line {
        Some(0) => args.read_names = Some(false),
        Some(_) => args.read_names = Some(true),
        None => {}
    }
    let lay = layout(&rows, &args, forced);
    let line_of = |i: usize| {
        rows.get(i)
            .map_or(rows.last().map_or(1, |r| r.0 + 1), |r| r.0)
    };
    let headers: Vec<Option<String>> = (0..lay.width)
        .map(|c| {
            lay.names.and_then(|i| match rows[i].1.as_ref()?.get(c)? {
                Cell::Text(t) => Some(t.clone()),
                _ => None,
            })
        })
        .collect();
    let names = make_names(headers, opts.preserve_names);
    let decimal_given = d.decimal.clone();
    opts.vars = names
        .into_iter()
        .zip(&lay.classes)
        .map(|(name, class)| {
            let mut v = VarOpts::new(name, kind_of(*class, d));
            if let Detected::Number { comma: true } = class {
                v.decimal = ",".into();
            }
            if v.kind.numeric() {
                if let Some(dec) = &decimal_given {
                    v.decimal = dec.clone();
                }
                if let Some(th) = &d.thousands {
                    v.thousands = th.clone();
                }
            }
            v
        })
        .collect();
    opts.selected = (0..opts.vars.len()).collect();
    if col0 > 0 || d.range.is_some_and(|r| r.last_col.is_some()) {
        let last = d.range.and_then(|r| r.last_col).unwrap_or(usize::MAX);
        opts.selected.retain(|&c| c >= col0 && c <= last);
    }
    opts.names_line = lay.names.map_or(0, line_of);
    opts.units_line = match d.units_line {
        Some(n) => n,
        None => lay.units.map_or(0, line_of),
    };
    if let Some(n) = d.names_line {
        opts.names_line = n;
    }
    opts.data_lines = match &d.data_lines {
        Some(lines) => lines.clone(),
        None => {
            let first = if lay.data < rows.len() {
                rows[lay.data].0
            } else {
                opts.names_line.max(opts.units_line).max(first_line - 1) + 1
            };
            vec![[first as f64, last_line.map_or(f64::INFINITY, |l| l as f64)]]
        }
    };
    if d.read_row_names == Some(true) && !opts.vars.is_empty() {
        opts.row_names_column = 1;
        opts.selected.retain(|&c| c != 0);
    }
    Ok(opts)
}

/// Detection on a worksheet.
fn detect_sheet(e: &Engine, path: &std::path::Path, d: &DetectArgs) -> Result<ImportOpts> {
    let sheet = d.sheet.clone().unwrap_or(SheetSel::Name(String::new()));
    let window = d.range.unwrap_or(Range::cell(0, 0));
    let grid = read::sheet_grid(e, path, &sheet, window)?;
    let mut opts = ImportOpts::new(true, Vec::new());
    opts.sheet_name = sheet;
    if let Some(p) = d.preserve {
        opts.preserve_names = p;
    }
    let rows: Vec<(usize, Option<Vec<Cell>>)> = grid
        .rows
        .iter()
        .enumerate()
        .map(|(r, cells)| {
            let cells: Vec<Cell> = cells
                .iter()
                .map(|c| match c {
                    read::Raw::Empty => Cell::Empty,
                    read::Raw::Num(_) => Cell::Num,
                    read::Raw::Bool(_) => Cell::Bool,
                    read::Raw::Text(t, _) => {
                        let t = t.trim();
                        if t.is_empty() {
                            Cell::Empty
                        } else {
                            Cell::Text(t.to_owned())
                        }
                    }
                })
                .collect();
            let any = cells.iter().any(|c| !matches!(c, Cell::Empty));
            (grid.row0 + r, any.then_some(cells))
        })
        .collect();
    let forced = d
        .header_lines
        .map(|n| rows.iter().position(|r| r.0 >= n).unwrap_or(rows.len()));
    let lay = layout(&rows, d, forced);
    let width = lay.width.max(grid.width);
    let headers: Vec<Option<String>> = (0..width)
        .map(|c| {
            lay.names.and_then(|i| match rows[i].1.as_ref()?.get(c)? {
                Cell::Text(t) => Some(t.clone()),
                _ => None,
            })
        })
        .collect();
    let mut names = make_names(headers, opts.preserve_names);
    let mut classes = lay.classes.clone();
    classes.resize(width, Detected::Text);
    let mut col0 = grid.col0;
    if d.read_row_names == Some(true) && width > 0 {
        names.remove(0);
        classes.remove(0);
        let data_row = rows.get(lay.data).map_or(grid.row0, |r| r.0);
        opts.row_names_range = read::cell_name(data_row, col0);
        col0 += 1;
    }
    opts.vars = names
        .into_iter()
        .zip(&classes)
        .map(|(name, class)| VarOpts::new(name, kind_of(*class, d)))
        .collect();
    opts.selected = (0..opts.vars.len()).collect();
    let data_row = rows.get(lay.data).map_or(grid.row0 + rows.len(), |r| r.0);
    opts.data_range = match d.range.and_then(|r| r.last_row) {
        Some(last) => format!(
            "{}:{}",
            read::cell_name(data_row, col0),
            read::cell_name(last, col0 + opts.vars.len().max(1) - 1)
        ),
        None => read::cell_name(data_row, col0),
    };
    opts.names_range = lay
        .names
        .map_or(String::new(), |i| read::cell_name(rows[i].0, col0));
    opts.units_range = lay
        .units
        .map_or(String::new(), |i| read::cell_name(rows[i].0, col0));
    Ok(opts)
}
