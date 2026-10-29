//! Regular expressions (regexp, regexpi, regexprep) and the string conversion
//! and classification helpers (str2double, str2num, strjust, isspace, isletter,
//! isstrprop, validatestring, isvarname, repmat on character arrays).
//! MATLAB patterns are translated to `regex` crate syntax; the constructs that
//! crate lacks (lookaround, backreferences, atomic groups, possessive
//! quantifiers, conditionals, dynamic expressions) are rejected explicitly.
//! Match positions count UTF-16 units, as MATLAB character indices do.
use super::{Engine, Execution, Value, shape_arguments};
use crate::syntax::{self, Expr, StatementKind, Token};
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result,
    StructArray, script_containers::MAX_CHARACTER_UNITS,
};
use regex::{Captures, Regex, RegexBuilder};
use std::sync::Arc;

const NAMES: &[&str] = &[
    "regexp",
    "regexpi",
    "regexprep",
    "str2double",
    "str2num",
    "strjust",
    "isspace",
    "isletter",
    "isstrprop",
    "validatestring",
    "isvarname",
];

/// Functions of this family with more than one output (see `eval_outputs`).
pub(super) fn multiple_outputs(name: &str) -> bool {
    matches!(name, "regexp" | "regexpi" | "str2num")
}

/// Longest pattern accepted, in bytes.
const MAX_PATTERN: usize = 65536;
/// Most capture slots one regexp call may collect.
const MAX_CAPTURE_SLOTS: usize = 4 << 20;
/// Compiled-program bound handed to the regex crate.
const MAX_PROGRAM: usize = 16 << 20;
/// MATLAB `namelengthmax`.
const NAME_LENGTH_MAX: usize = 63;

/// A text operand: one character row, or the rows of a cell (column-major).
enum Texts {
    One(String),
    Cell(usize, usize, Vec<String>),
}

impl Texts {
    fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Cell(_, _, items) => items.len(),
        }
    }
    /// Element `i`; a single row stands for every element.
    fn get(&self, i: usize) -> &str {
        match self {
            Self::One(s) => s,
            Self::Cell(_, _, items) => &items[i],
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Output {
    Start,
    End,
    TokenExtents,
    Match,
    Tokens,
    Names,
    Split,
}

impl Output {
    const DEFAULT: [Output; 7] = [
        Self::Start,
        Self::End,
        Self::TokenExtents,
        Self::Match,
        Self::Tokens,
        Self::Names,
        Self::Split,
    ];
    fn parse(option: &str) -> Option<Self> {
        Some(match option {
            "start" => Self::Start,
            "end" => Self::End,
            "tokenextents" => Self::TokenExtents,
            "match" => Self::Match,
            "tokens" => Self::Tokens,
            "names" => Self::Names,
            "split" => Self::Split,
            _ => return None,
        })
    }
}

struct Flags {
    once: bool,
    ignore_case: bool,
    empty_match: bool,
    dot_all: bool,
    line_anchors: bool,
    free_spacing: bool,
}

impl Flags {
    fn new(ignore_case: bool) -> Self {
        Self {
            once: false,
            ignore_case,
            empty_match: false,
            dot_all: true,
            line_anchors: false,
            free_spacing: false,
        }
    }
    /// Apply one lower-case option; false for an unknown one.
    fn set(&mut self, option: &str) -> bool {
        match option {
            "once" => self.once = true,
            "ignorecase" => self.ignore_case = true,
            "matchcase" => self.ignore_case = false,
            "emptymatch" => self.empty_match = true,
            "noemptymatch" => self.empty_match = false,
            "dotall" => self.dot_all = true,
            "dotexceptnewline" => self.dot_all = false,
            "stringanchors" => self.line_anchors = false,
            "lineanchors" => self.line_anchors = true,
            "literalspacing" => self.free_spacing = false,
            "freespacing" => self.free_spacing = true,
            "warnings" => {}
            _ => return false,
        }
        true
    }
}

/// One MATLAB token of a match.
#[derive(Clone, Copy)]
enum Capture {
    Matched(usize, usize),
    /// Skipped by a quantifier: MATLAB returns ''.
    Unmatched,
    /// In an alternation branch the match did not take: MATLAB omits it.
    Excluded,
}

/// One match: byte ranges of the whole match and of each MATLAB token.
struct Hit {
    start: usize,
    end: usize,
    groups: Vec<Capture>,
}

/// A compiled pattern. Alternations that contain tokens get a hidden empty
/// group at the start of each branch, so a match tells which branch it took.
struct Pattern {
    re: Regex,
    /// Regex group of each MATLAB token, in token order.
    tokens: Vec<usize>,
    /// Per token, the branch markers that must have matched for it to count.
    branches: Vec<Vec<usize>>,
    /// Named tokens: field name and token index.
    names: Vec<(String, usize)>,
}

impl Pattern {
    fn token(&self, caps: &Captures, k: usize) -> Capture {
        if self.branches[k].iter().any(|&m| caps.get(m).is_none()) {
            return Capture::Excluded;
        }
        caps.get(self.tokens[k])
            .map_or(Capture::Unmatched, |m| Capture::Matched(m.start(), m.end()))
    }
}

/// A translated pattern before compiling.
struct Translated {
    source: String,
    tokens: Vec<usize>,
    branches: Vec<Vec<usize>>,
    /// Frames that need branch markers (see `Nesting::needs_markers`).
    needs: Vec<bool>,
}

/// Group nesting while translating: numbers the regex groups and places the
/// branch markers (frame 0 is the whole pattern).
struct Nesting<'a> {
    marked: &'a [bool],
    alternation: Vec<bool>,
    captures: Vec<bool>,
    /// Open frames: id and the marker group of the current branch.
    stack: Vec<(usize, Option<usize>)>,
    next: usize,
    tokens: Vec<usize>,
    branches: Vec<Vec<usize>>,
}

impl<'a> Nesting<'a> {
    fn new(marked: &'a [bool], out: &mut String) -> Self {
        let mut nesting = Self {
            marked,
            alternation: Vec::new(),
            captures: Vec::new(),
            stack: Vec::new(),
            next: 1,
            tokens: Vec::new(),
            branches: Vec::new(),
        };
        nesting.open(out, false);
        nesting
    }
    /// After a group's opening text has been written.
    fn open(&mut self, out: &mut String, capturing: bool) {
        if capturing {
            for &(id, _) in &self.stack {
                self.captures[id] = true;
            }
            self.tokens.push(self.next);
            self.branches
                .push(self.stack.iter().filter_map(|frame| frame.1).collect());
            self.next += 1;
        }
        let id = self.alternation.len();
        self.alternation.push(false);
        self.captures.push(false);
        let marker = self.marker(out, id);
        self.stack.push((id, marker));
    }
    fn marker(&mut self, out: &mut String, id: usize) -> Option<usize> {
        if !self.marked.get(id).copied().unwrap_or(false) {
            return None;
        }
        out.push_str("()");
        self.next += 1;
        Some(self.next - 1)
    }
    fn branch(&mut self, out: &mut String) {
        out.push('|');
        let id = self.stack.last().expect("frame 0 stays open").0;
        self.alternation[id] = true;
        let marker = self.marker(out, id);
        self.stack.last_mut().expect("frame 0 stays open").1 = marker;
    }
    fn close(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
    /// Frames that need branch markers: alternations containing tokens.
    fn needs_markers(&self) -> Vec<bool> {
        self.alternation
            .iter()
            .zip(&self.captures)
            .map(|(&a, &c)| a && c)
            .collect()
    }
}

/// Byte offset to UTF-16 position; ASCII text needs no table.
struct Positions(Option<Vec<u32>>);

impl Positions {
    fn new(s: &str) -> Self {
        if s.is_ascii() {
            return Self(None);
        }
        let mut table = vec![0u32; s.len() + 1];
        let mut units = 0u32;
        for (i, c) in s.char_indices() {
            for slot in &mut table[i..i + c.len_utf8()] {
                *slot = units;
            }
            units += c.len_utf16() as u32;
        }
        table[s.len()] = units;
        Self(Some(table))
    }
    fn at(&self, byte: usize) -> f64 {
        match &self.0 {
            None => byte as f64,
            Some(table) => f64::from(table[byte]),
        }
    }
}

/// A piece of a regexprep replacement text.
enum Piece {
    Literal(String),
    Group(usize),
    Named(String),
}

fn too_few() -> String {
    "Not enough input arguments.".into()
}

fn too_many() -> String {
    "Too many input arguments.".into()
}

/// MATLAB's message for a text input of regexp/regexprep that is not text.
fn not_text(what: &str) -> String {
    format!(
        "The '{what}' input must be either a char row vector or a cell array of char row vectors."
    )
}

/// Text of an option argument as typed.
fn raw_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            String::from_utf16(c.units()).ok()
        }
        _ => None,
    }
}

fn value_of(array: Array) -> Value {
    match array {
        Array::Numeric(m) => Value::Matrix(m),
        Array::Logical(m) => Value::Logical(m),
        Array::Complex(m) => Value::Complex(m),
        other => Value::Container(other),
    }
}

/// Text of an option argument, lower-cased.
fn option_text(v: &Value) -> Option<String> {
    raw_text(v).map(|s| s.to_ascii_lowercase())
}

/// Translate a MATLAB pattern into `regex` crate syntax; `marked` lists the
/// group frames that get branch markers.
fn translate(who: &str, pattern: &str, marked: &[bool]) -> Result<Translated> {
    if pattern.len() > MAX_PATTERN {
        return Err(format!("{who}: expression exceeds {MAX_PATTERN} bytes"));
    }
    let unsupported = |what: &str| format!("{who}: {what} are not supported yet");
    let c: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut nesting = Nesting::new(marked, &mut out);
    let mut i = 0;
    let mut in_class = false;
    // The last pattern character was a quantifier (a following + is possessive).
    let mut quantified = false;
    while i < c.len() {
        let ch = c[i];
        let was_quantified = std::mem::replace(&mut quantified, false);
        if ch == '\\' {
            let Some(&next) = c.get(i + 1) else {
                return Err(format!("{who}: expression ends with a lone backslash"));
            };
            i += 2;
            let class = |name: &str| {
                if in_class {
                    format!("[:{name}:]")
                } else {
                    format!("[[:{name}:]]")
                }
            };
            match next {
                'd' => out.push_str(&class("digit")),
                'D' => out.push_str(&class("^digit")),
                'w' => out.push_str(&class("word")),
                'W' => out.push_str(&class("^word")),
                's' => out.push_str(&class("space")),
                'S' => out.push_str(&class("^space")),
                'b' => out.push_str("\\x{8}"),
                'B' if !in_class => out.push_str("(?-u:\\B)"),
                '<' if !in_class => out.push_str("(?-u:\\b{start})"),
                '>' if !in_class => out.push_str("(?-u:\\b{end})"),
                '1'..='9' | 'k' => return Err(unsupported("backreferences (\\N, \\k<name>)")),
                '0' => out.push_str("\\x{0}"),
                'e' => out.push_str("\\x{1B}"),
                'o' | 'x' => {
                    let radix = if next == 'o' { 8 } else { 16 };
                    let digits: String = if c.get(i) == Some(&'{') {
                        let close = c[i..]
                            .iter()
                            .position(|&d| d == '}')
                            .ok_or_else(|| format!("{who}: unterminated \\{next}{{...}}"))?;
                        let digits = c[i + 1..i + close].iter().collect();
                        i += close + 1;
                        digits
                    } else {
                        let limit = if radix == 8 { 3 } else { 2 };
                        let n = c[i..]
                            .iter()
                            .take(limit)
                            .take_while(|d| d.is_digit(radix))
                            .count();
                        let digits = c[i..i + n].iter().collect();
                        i += n;
                        digits
                    };
                    let code = u32::from_str_radix(&digits, radix)
                        .ok()
                        .filter(|&v| char::from_u32(v).is_some())
                        .ok_or_else(|| format!("{who}: invalid character code \\{next}{digits}"))?;
                    out.push_str(&format!("\\x{{{code:X}}}"));
                }
                'c' => {
                    let Some(&letter) = c.get(i).filter(|l| l.is_ascii_alphabetic()) else {
                        return Err(format!("{who}: \\c must be followed by a letter"));
                    };
                    i += 1;
                    out.push_str(&format!(
                        "\\x{{{:X}}}",
                        letter.to_ascii_uppercase() as u32 & 0x1F
                    ));
                }
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            continue;
        }
        if in_class {
            match ch {
                ']' => in_class = false,
                // Literal inside a MATLAB class (which has no POSIX [:name:]
                // classes); set syntax in the regex crate.
                '[' | '&' | '~' => out.push('\\'),
                '-' if out.ends_with('-') => out.push('\\'),
                _ => {}
            }
            out.push(ch);
            i += 1;
            continue;
        }
        match ch {
            '[' => {
                in_class = true;
                out.push('[');
                i += 1;
                if c.get(i) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
                // A leading ] is a literal member.
                if c.get(i) == Some(&']') {
                    out.push_str("\\]");
                    i += 1;
                }
                continue;
            }
            '(' if c.get(i + 1) == Some(&'?') => {
                let rest: String = c[i + 2..c.len().min(i + 4)].iter().collect();
                if rest.starts_with('#') {
                    let close = c[i..]
                        .iter()
                        .position(|&d| d == ')')
                        .ok_or_else(|| format!("{who}: unterminated (?# comment"))?;
                    i += close + 1;
                    continue;
                }
                if rest.starts_with('=')
                    || rest.starts_with('!')
                    || rest.starts_with("<=")
                    || rest.starts_with("<!")
                {
                    return Err(unsupported("lookahead and lookbehind assertions"));
                }
                if rest.starts_with('>') {
                    return Err(unsupported("atomic groups (?>...)"));
                }
                if rest.starts_with('(') {
                    return Err(unsupported("conditional expressions (?(...)...)"));
                }
                if rest.starts_with('@') || rest.starts_with('?') {
                    return Err(unsupported("dynamic expressions (?@...) and (??...)"));
                }
                let j = i + 2;
                if rest.starts_with('<') || rest.starts_with("P<") {
                    let first = if rest.starts_with('<') { j + 1 } else { j + 2 };
                    let close = c[first..]
                        .iter()
                        .position(|&d| d == '>')
                        .ok_or_else(|| format!("{who}: unterminated token name (?<..."))?;
                    out.push_str("(?<");
                    out.extend(&c[first..first + close]);
                    out.push('>');
                    i = first + close + 1;
                    nesting.open(&mut out, true);
                    continue;
                }
                // (?flags:...) opens a group; (?flags) only sets flags.
                let end = (j..c.len())
                    .find(|&k| !(c[k].is_ascii_alphabetic() || c[k] == '-'))
                    .unwrap_or(c.len());
                out.push_str("(?");
                out.extend(&c[j..end]);
                i = end;
                if c.get(end) == Some(&')') && end > j {
                    out.push(')');
                    i += 1;
                    continue;
                }
                if c.get(end) == Some(&':') {
                    out.push(':');
                    i += 1;
                }
                nesting.open(&mut out, false);
                continue;
            }
            '(' => {
                out.push('(');
                i += 1;
                nesting.open(&mut out, true);
                continue;
            }
            ')' => nesting.close(),
            '|' => {
                nesting.branch(&mut out);
                i += 1;
                continue;
            }
            ']' => {
                out.push_str("\\]");
                i += 1;
                continue;
            }
            '*' | '?' => quantified = true,
            '+' if was_quantified => return Err(unsupported("possessive quantifiers")),
            '+' => quantified = true,
            '{' => {
                let close = c[i..].iter().position(|&d| d == '}');
                let body: Option<String> = close.map(|k| c[i + 1..i + k].iter().collect());
                let repetition = body.as_deref().is_some_and(|b| {
                    let mut parts = b.splitn(2, ',');
                    let low = parts.next().unwrap_or("");
                    let high = parts.next();
                    !low.is_empty()
                        && low.chars().all(|d| d.is_ascii_digit())
                        && high.is_none_or(|h| h.chars().all(|d| d.is_ascii_digit()))
                });
                if let (true, Some(k)) = (repetition, close) {
                    out.extend(&c[i..=i + k]);
                    i += k + 1;
                    quantified = true;
                } else {
                    out.push_str("\\{");
                    i += 1;
                }
                continue;
            }
            '}' => {
                out.push_str("\\}");
                i += 1;
                continue;
            }
            _ => {}
        }
        out.push(ch);
        i += 1;
    }
    if in_class {
        return Err(format!("{who}: unterminated character class ["));
    }
    Ok(Translated {
        needs: nesting.needs_markers(),
        tokens: nesting.tokens,
        branches: nesting.branches,
        source: out,
    })
}

fn compile(who: &str, pattern: &str, flags: &Flags) -> Result<Pattern> {
    let mut translated = translate(who, pattern, &[])?;
    if translated.needs.contains(&true) {
        translated = translate(who, pattern, &translated.needs)?;
    }
    let mut source = String::from("(?");
    for (on, flag) in [
        (flags.dot_all, 's'),
        (flags.line_anchors, 'm'),
        (flags.ignore_case, 'i'),
        (flags.free_spacing, 'x'),
    ] {
        if on {
            source.push(flag);
        }
    }
    source.push(':');
    source.push_str(&translated.source);
    source.push(')');
    let re = RegexBuilder::new(&source)
        .size_limit(MAX_PROGRAM)
        .dfa_size_limit(MAX_PROGRAM)
        .build()
        .map_err(|e| {
            let text = e.to_string();
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("error: "))
                .unwrap_or(text.lines().last().unwrap_or("invalid expression"));
            format!("{who}: invalid expression '{pattern}': {line}")
        })?;
    let tokens = translated.tokens;
    let names = re
        .capture_names()
        .enumerate()
        .filter_map(|(group, name)| {
            let k = tokens.iter().position(|&t| t == group)?;
            Some((name?.to_owned(), k))
        })
        .collect();
    Ok(Pattern {
        re,
        tokens,
        branches: translated.branches,
        names,
    })
}

/// Parse a regexprep replacement: $0..$9, $<name>, and \ escapes. `tokens`
/// maps token numbers to regex groups; $N past the last token stays literal.
fn replacement(text: &str, tokens: &[usize]) -> Result<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('n') => literal.push('\n'),
                Some('t') => literal.push('\t'),
                Some('r') => literal.push('\r'),
                Some('f') => literal.push('\u{c}'),
                Some('v') => literal.push('\u{b}'),
                Some('a') => literal.push('\u{7}'),
                Some(other) => literal.push(other),
                None => literal.push('\\'),
            },
            '$' => match chars.peek().copied() {
                Some(d @ '0'..='9') => {
                    chars.next();
                    let n = d as usize - '0' as usize;
                    let group = if n == 0 {
                        Some(0)
                    } else {
                        tokens.get(n - 1).copied()
                    };
                    match group {
                        Some(g) => {
                            pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                            pieces.push(Piece::Group(g));
                        }
                        None => {
                            literal.push('$');
                            literal.push(d);
                        }
                    }
                }
                Some('<') => {
                    chars.next();
                    let name: String = chars.by_ref().take_while(|&c| c != '>').collect();
                    pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    pieces.push(Piece::Named(name));
                }
                Some('{') => {
                    return Err(
                        "regexprep: dynamic replacement expressions ${...} are not supported yet"
                            .into(),
                    );
                }
                _ => literal.push('$'),
            },
            other => literal.push(other),
        }
    }
    pieces.push(Piece::Literal(literal));
    Ok(pieces)
}

/// str2double grammar: an optional real part and an optional imaginary part
/// ending in i or j, commas between digits, e/E exponents, Inf and NaN.
fn parse_double(text: &str) -> Option<(f64, f64)> {
    let b = text
        .trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0')
        .as_bytes();
    if b.is_empty() {
        return None;
    }
    let skip = |p: &mut usize| {
        while *p < b.len() && b[*p].is_ascii_whitespace() {
            *p += 1;
        }
    };
    let unit = |p: usize| p < b.len() && matches!(b[p], b'i' | b'j');
    match number(b, 0, true) {
        Some((value, mut p)) => {
            skip(&mut p);
            if p == b.len() {
                return Some((value, 0.));
            }
            if unit(p) {
                p += 1;
                skip(&mut p);
                return (p == b.len()).then_some((0., value));
            }
            let sign = match b[p] {
                b'+' => 1.,
                b'-' => -1.,
                _ => return None,
            };
            p += 1;
            skip(&mut p);
            let (magnitude, mut p) = number(b, p, false).unwrap_or((1., p));
            skip(&mut p);
            if !unit(p) {
                return None;
            }
            p += 1;
            skip(&mut p);
            (p == b.len()).then_some((value, sign * magnitude))
        }
        None => {
            let mut p = 0;
            let mut sign = 1.;
            if matches!(b[0], b'+' | b'-') {
                sign = if b[0] == b'-' { -1. } else { 1. };
                p = 1;
                skip(&mut p);
            }
            if !unit(p) {
                return None;
            }
            p += 1;
            skip(&mut p);
            (p == b.len()).then_some((0., sign))
        }
    }
}

/// One real number at `p`: digits (commas allowed between digits), a decimal
/// point, an exponent, or Inf/NaN in any case.
fn number(b: &[u8], mut p: usize, signed: bool) -> Option<(f64, usize)> {
    let mut negative = false;
    if signed && p < b.len() && matches!(b[p], b'+' | b'-') {
        negative = b[p] == b'-';
        p += 1;
    }
    let rest = &b[p..];
    let word = |w: &[u8]| rest.len() >= w.len() && rest[..w.len()].eq_ignore_ascii_case(w);
    let sign = if negative { -1. } else { 1. };
    if word(b"inf") {
        return Some((sign * f64::INFINITY, p + 3));
    }
    if word(b"nan") {
        return Some((f64::NAN, p + 3));
    }
    let mut text = String::new();
    let mut digits = 0;
    while p < b.len() {
        if b[p].is_ascii_digit() {
            text.push(b[p] as char);
            digits += 1;
        } else if !(b[p] == b',' && digits > 0 && b.get(p + 1).is_some_and(u8::is_ascii_digit)) {
            break;
        }
        p += 1;
    }
    if p < b.len() && b[p] == b'.' {
        text.push('.');
        p += 1;
        while p < b.len() && b[p].is_ascii_digit() {
            text.push(b[p] as char);
            digits += 1;
            p += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if p < b.len() && matches!(b[p], b'e' | b'E') {
        let mut q = p + 1;
        let mut exponent = String::from("e");
        if q < b.len() && matches!(b[q], b'+' | b'-') {
            exponent.push(b[q] as char);
            q += 1;
        }
        let first = q;
        while q < b.len() && b[q].is_ascii_digit() {
            exponent.push(b[q] as char);
            q += 1;
        }
        if q > first {
            text.push_str(&exponent);
            p = q;
        }
    }
    text.parse::<f64>().ok().map(|v| (sign * v, p))
}

/// Rewrite the constants str2num admits into literals; None rejects the text.
/// `logical` stays true while every leaf is true/false; `operators` records
/// any arithmetic (whose result is double, as in MATLAB).
fn literal(e: &Expr, logical: &mut bool, operators: &mut bool) -> Option<Expr> {
    Some(match e {
        Expr::Number(v) => {
            *logical = false;
            Expr::Number(*v)
        }
        Expr::Imaginary(v) => {
            *logical = false;
            Expr::Imaginary(*v)
        }
        Expr::Name(name) => match name.as_str() {
            "true" => Expr::Number(1.),
            "false" => Expr::Number(0.),
            other => {
                *logical = false;
                match other {
                    "Inf" | "inf" => Expr::Number(f64::INFINITY),
                    "NaN" | "nan" => Expr::Number(crate::MATLAB_NAN),
                    "i" | "j" => Expr::Imaginary(1.),
                    "pi" => Expr::Number(std::f64::consts::PI),
                    "eps" => Expr::Number(f64::EPSILON),
                    _ => return None,
                }
            }
        },
        Expr::Unary(negative, inner) => {
            *operators = true;
            Expr::Unary(*negative, Box::new(literal(inner, logical, operators)?))
        }
        Expr::Binary(token, a, b)
            if matches!(
                token,
                Token::Plus
                    | Token::Minus
                    | Token::Star
                    | Token::Slash
                    | Token::Power
                    | Token::DotStar
                    | Token::DotSlash
                    | Token::DotPower
            ) =>
        {
            *operators = true;
            Expr::Binary(
                token.clone(),
                Box::new(literal(a, logical, operators)?),
                Box::new(literal(b, logical, operators)?),
            )
        }
        Expr::Range(a, step, b) => {
            *operators = true;
            let step = match step {
                Some(s) => Some(Box::new(literal(s, logical, operators)?)),
                None => None,
            };
            Expr::Range(
                Box::new(literal(a, logical, operators)?),
                step,
                Box::new(literal(b, logical, operators)?),
            )
        }
        Expr::Transpose(inner) => {
            *operators = true;
            Expr::Transpose(Box::new(literal(inner, logical, operators)?))
        }
        Expr::NonConjugateTranspose(inner) => {
            *operators = true;
            Expr::NonConjugateTranspose(Box::new(literal(inner, logical, operators)?))
        }
        Expr::Matrix(rows) => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut items = Vec::with_capacity(row.len());
                for item in row {
                    items.push(literal(item, logical, operators)?);
                }
                out.push(items);
            }
            Expr::Matrix(out)
        }
        _ => return None,
    })
}

fn is_space_char(c: char) -> bool {
    matches!(c, ' ' | '\t'..='\r') || (!c.is_ascii() && c.is_whitespace())
}

/// Unicode punctuation (categories Pc Pd Ps Pe Pi Pf Po): exact for ASCII
/// and Latin-1, the General and CJK punctuation blocks beyond. ASCII symbols
/// such as $ + < = > ^ ` | ~ are not punctuation (as in the reference).
fn is_punctuation(c: char) -> bool {
    matches!(
        c,
        '!'..='#'
            | '%'..='*'
            | ','..='/'
            | ':'
            | ';'
            | '?'
            | '@'
            | '['..=']'
            | '_'
            | '{'
            | '}'
            | '\u{A1}'
            | '\u{A7}'
            | '\u{AB}'
            | '\u{B6}'
            | '\u{B7}'
            | '\u{BB}'
            | '\u{BF}'
            | '\u{2010}'..='\u{2027}'
            | '\u{2030}'..='\u{2043}'
            | '\u{2045}'..='\u{2051}'
            | '\u{2053}'..='\u{205E}'
            | '\u{3001}'..='\u{3003}'
            | '\u{3008}'..='\u{3011}'
    )
}

/// isstrprop categories.
fn property(category: &str) -> Option<fn(char) -> bool> {
    let test: fn(char) -> bool = match category {
        "alpha" => char::is_alphabetic,
        "alphanum" => char::is_alphanumeric,
        "cntrl" => char::is_control,
        "digit" => |c| c.is_ascii_digit(),
        "graphic" => |c| !c.is_control() && !is_space_char(c),
        "lower" => char::is_lowercase,
        "upper" => char::is_uppercase,
        "print" => |c| !c.is_control() && (c == ' ' || !is_space_char(c)),
        "punct" => is_punctuation,
        "wspace" => is_space_char,
        "xdigit" => |c| c.is_ascii_hexdigit(),
        _ => return None,
    };
    Some(test)
}

fn is_varname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.len() <= NAME_LENGTH_MAX
        && !syntax::keyword(s)
}

impl Engine {
    /// One-output calls of this family; `repmat` only for character input.
    pub(super) fn strings2_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if name == "repmat" {
            return match args.first() {
                Some(
                    Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)),
                ) => self.strings2_repmat(args).map(Some),
                _ => Ok(None),
            };
        }
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        Ok(Some(self.strings2_outputs(name, args, 1)?.remove(0)))
    }

    /// `count` outputs of a call of this family.
    pub(super) fn strings2_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if count > 1 && !multiple_outputs(name) {
            return Err("Too many output arguments.".into());
        }
        let value = match name {
            "regexp" | "regexpi" => return self.strings2_regexp(name, args, count),
            "str2num" => return self.strings2_str2num(args, count),
            "regexprep" => self.strings2_regexprep(args)?,
            "str2double" => self.strings2_str2double(args)?,
            "strjust" => self.strings2_strjust(args)?,
            "isspace" => self.strings2_classify(args, is_space_char)?,
            "isletter" => self.strings2_classify(args, char::is_alphabetic)?,
            "isstrprop" => self.strings2_isstrprop(args)?,
            "validatestring" => self.strings2_validatestring(args)?,
            "isvarname" => {
                match args.len() {
                    0 => return Err(too_few()),
                    1 => {}
                    _ => return Err(too_many()),
                }
                let name = match &args[0] {
                    Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
                    Value::Container(Array::Character(c)) if c.rows() == 1 => {
                        String::from_utf16(c.units()).ok()
                    }
                    _ => None,
                };
                Value::Logical(LogicalMatrix::scalar(name.is_some_and(|s| is_varname(&s))))
            }
            _ => unreachable!("NAMES and the match above agree"),
        };
        Ok(vec![value])
    }

    /// A character row or a cell of character rows; `what` names the input in
    /// MATLAB's message (STRING, EXPRESSION, REPLACE).
    fn strings2_texts(&self, what: &str, v: &Value) -> Result<Texts> {
        let row = |c: &CharacterRow| -> Result<String> {
            if c.units().is_empty() {
                return Ok(String::new());
            }
            if c.rows() != 1 {
                return Err(not_text(what));
            }
            c.to_utf8(&self.cancel)
        };
        Ok(match v {
            Value::Text(s) | Value::QuotedString(s) => Texts::One(s.clone()),
            Value::Container(Array::Character(c)) => Texts::One(row(c)?),
            Value::Container(Array::Cell(cell)) => {
                let mut items = Vec::with_capacity(cell.len());
                for i in 0..cell.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    match cell.get_linear(i)?.as_ref() {
                        Array::Character(c) => items.push(row(c)?),
                        _ => return Err(not_text(what)),
                    }
                }
                Texts::Cell(cell.rows(), cell.cols(), items)
            }
            _ => return Err(not_text(what)),
        })
    }

    /// A character row; empty text is 0x0, as regexp and regexprep return it.
    fn strings2_row(&self, s: &str) -> Result<Array> {
        Ok(CharacterRow::from_text(s, &self.cancel)?.into())
    }

    fn strings2_cell(&self, rows: usize, cols: usize, items: Vec<Array>) -> Result<Array> {
        let mut slots = Vec::with_capacity(items.len());
        slots.extend(items.into_iter().map(Arc::new));
        Ok(CellArray::from_shared(rows, cols, slots, &self.cancel)?.into())
    }

    /// The matches of `pattern` in `s`; tokens only when `want_groups`.
    fn strings2_hits(
        &self,
        pattern: &Pattern,
        s: &str,
        flags: &Flags,
        want_groups: bool,
    ) -> Result<Vec<Hit>> {
        let mut hits = Vec::new();
        let keep = |start: usize, end: usize| start < end || flags.empty_match;
        if !want_groups {
            for m in pattern.re.find_iter(s) {
                self.check_cancel()?;
                if !keep(m.start(), m.end()) {
                    continue;
                }
                hits.push(Hit {
                    start: m.start(),
                    end: m.end(),
                    groups: Vec::new(),
                });
                if flags.once {
                    break;
                }
            }
            return Ok(hits);
        }
        let width = pattern.tokens.len();
        for caps in pattern.re.captures_iter(s) {
            self.check_cancel()?;
            if hits.len().saturating_add(1).saturating_mul(width.max(1)) > MAX_CAPTURE_SLOTS {
                return Err(format!(
                    "regexp: matches times capture groups exceed {MAX_CAPTURE_SLOTS}"
                ));
            }
            let whole = caps.get(0).expect("group 0 always participates");
            if !keep(whole.start(), whole.end()) {
                continue;
            }
            hits.push(Hit {
                start: whole.start(),
                end: whole.end(),
                groups: (0..width).map(|k| pattern.token(&caps, k)).collect(),
            });
            if flags.once {
                break;
            }
        }
        Ok(hits)
    }

    fn strings2_regexp(&mut self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let who = name;
        if args.len() < 2 {
            return Err(too_few());
        }
        let texts = self.strings2_texts("STRING", &args[0])?;
        let exprs = self.strings2_texts("EXPRESSION", &args[1])?;
        let mut flags = Flags::new(name == "regexpi");
        let mut order = Vec::new();
        let mut selectors = Vec::new();
        let mut case_option: Option<String> = None;
        for option in &args[2..] {
            let raw = raw_text(option).ok_or(format!("{who}: options must be text"))?;
            let text = raw.to_ascii_lowercase();
            if let Some(kind) = Output::parse(&text) {
                order.push(kind);
                selectors.push(raw);
                continue;
            }
            if matches!(text.as_str(), "ignorecase" | "matchcase") {
                if let Some(earlier) = case_option.as_deref()
                    && earlier != text
                {
                    return Err(format!(
                        "The '{text}' option may not be used in conjunction with the '{earlier}' option."
                    ));
                }
                case_option = Some(text.clone());
            }
            if !flags.set(&text) {
                return Err(format!("Invalid option for {who}: {raw}."));
            }
        }
        let count = count.max(1);
        if let Some(missing) = selectors.get(count) {
            return Err(format!(
                "Not enough outputs specified for '{missing}' option."
            ));
        }
        for kind in Output::DEFAULT {
            if !order.contains(&kind) {
                order.push(kind);
            }
        }
        if count > order.len() {
            return Err("Too many output arguments.".into());
        }
        order.truncate(count);
        let (rows, cols, n, cell) = match (&texts, &exprs) {
            (Texts::One(_), Texts::One(_)) => (1, 1, 1, false),
            (Texts::Cell(r, c, v), Texts::One(_)) | (Texts::One(_), Texts::Cell(r, c, v)) => {
                (*r, *c, v.len(), true)
            }
            (Texts::Cell(r, c, v), Texts::Cell(r2, c2, _)) => {
                if (r, c) != (r2, c2) {
                    return Err(format!("{who}: cell array inputs must have the same size"));
                }
                (*r, *c, v.len(), true)
            }
        };
        let mut patterns = Vec::with_capacity(exprs.len());
        for i in 0..exprs.len() {
            self.check_cancel()?;
            patterns.push(compile(who, exprs.get(i), &flags)?);
        }
        let want_groups = order
            .iter()
            .any(|k| matches!(k, Output::Tokens | Output::TokenExtents | Output::Names));
        let mut results: Vec<Vec<Array>> =
            (0..order.len()).map(|_| Vec::with_capacity(n)).collect();
        for i in 0..n {
            self.check_cancel()?;
            let s = texts.get(i);
            let pattern = &patterns[if patterns.len() == 1 { 0 } else { i }];
            let hits = self.strings2_hits(pattern, s, &flags, want_groups)?;
            let positions = Positions::new(s);
            for (k, kind) in order.iter().enumerate() {
                let value =
                    self.strings2_regexp_output(*kind, s, pattern, &hits, &positions, flags.once)?;
                results[k].push(value);
            }
        }
        let mut outputs = Vec::with_capacity(order.len());
        for items in results {
            outputs.push(value_of(if cell {
                self.strings2_cell(rows, cols, items)?
            } else {
                items.into_iter().next().expect("one result per output")
            }));
        }
        Ok(outputs)
    }

    /// One regexp output for one text. Without 'once', no match gives 0x0
    /// values; with it, the first match's value itself (or 0x0).
    fn strings2_regexp_output(
        &self,
        kind: Output,
        s: &str,
        pattern: &Pattern,
        hits: &[Hit],
        at: &Positions,
        once: bool,
    ) -> Result<Array> {
        let first = |h: &Hit| at.at(h.start) + 1.;
        let empty_cell = || -> Result<Array> { Ok(CellArray::empty(0, 0, &self.cancel)?.into()) };
        let present = |h: &Hit| -> Vec<Capture> {
            h.groups
                .iter()
                .copied()
                .filter(|t| !matches!(t, Capture::Excluded))
                .collect()
        };
        let tokens = |h: &Hit| -> Result<Array> {
            let mut items = Vec::new();
            for token in present(h) {
                items.push(match token {
                    Capture::Matched(a, b) => self.strings2_row(&s[a..b])?,
                    _ => self.strings2_row("")?,
                });
            }
            self.strings2_cell(1, items.len(), items)
        };
        let extents = |h: &Hit| -> Result<Array> {
            let spans: Vec<(f64, f64)> = present(h)
                .iter()
                .map(|t| match t {
                    Capture::Matched(a, b) => (at.at(*a) + 1., at.at(*b)),
                    _ => (first(h), first(h) - 1.),
                })
                .collect();
            if spans.is_empty() {
                return Ok(Matrix::filled(0, 0, 0.)?.into());
            }
            let mut data: Vec<f64> = spans.iter().map(|p| p.0).collect();
            data.extend(spans.iter().map(|p| p.1));
            Ok(Matrix::from_column_vec(spans.len(), 2, data)?.into())
        };
        let positions = |f: &dyn Fn(&Hit) -> f64| -> Result<Array> {
            let data: Vec<f64> = hits.iter().map(f).collect();
            Ok(match (once, data.first()) {
                (_, None) => Matrix::filled(0, 0, 0.)?,
                (true, Some(&v)) => Matrix::scalar(v),
                (false, Some(_)) => Matrix::from_column_vec(1, data.len(), data)?,
            }
            .into())
        };
        let per_hit = |f: &dyn Fn(&Hit) -> Result<Array>, none: Array| -> Result<Array> {
            if once {
                return match hits.first() {
                    Some(h) => f(h),
                    None => Ok(none),
                };
            }
            if hits.is_empty() {
                return empty_cell();
            }
            let mut items = Vec::with_capacity(hits.len());
            for (n, h) in hits.iter().enumerate() {
                if n.is_multiple_of(1024) {
                    self.check_cancel()?;
                }
                items.push(f(h)?);
            }
            self.strings2_cell(1, items.len(), items)
        };
        match kind {
            Output::Start => positions(&first),
            Output::End => positions(&|h: &Hit| at.at(h.end)),
            Output::Match => per_hit(
                &|h: &Hit| self.strings2_row(&s[h.start..h.end]),
                self.strings2_row("")?,
            ),
            Output::Tokens => per_hit(&tokens, empty_cell()?),
            Output::TokenExtents => per_hit(&extents, empty_cell()?),
            Output::Names => {
                let fields: Vec<String> = pattern.names.iter().map(|n| n.0.clone()).collect();
                let used = if once { hits.len().min(1) } else { hits.len() };
                let mut slots = Vec::with_capacity(used.saturating_mul(fields.len()));
                for (n, h) in hits[..used].iter().enumerate() {
                    if n.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    for &(_, k) in &pattern.names {
                        let text = match h.groups[k] {
                            Capture::Matched(a, b) => &s[a..b],
                            _ => "",
                        };
                        slots.push(Arc::new(self.strings2_row(text)?));
                    }
                }
                let (rows, cols) = if used == 0 { (0, 0) } else { (1, used) };
                Ok(Array::Struct(StructArray::from_slots(
                    rows,
                    cols,
                    fields,
                    slots,
                    &self.cancel,
                )?))
            }
            Output::Split => {
                let mut items = Vec::with_capacity(hits.len() + 1);
                let mut last = 0;
                for (n, h) in hits.iter().enumerate() {
                    if n.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    items.push(self.strings2_row(&s[last..h.start])?);
                    last = h.end;
                }
                items.push(self.strings2_row(&s[last..])?);
                self.strings2_cell(1, items.len(), items)
            }
        }
    }

    fn strings2_regexprep(&mut self, args: &[Value]) -> Result<Value> {
        let who = "regexprep";
        if args.len() < 3 {
            return Err(too_few());
        }
        let texts = self.strings2_texts("STRING", &args[0])?;
        let exprs = self.strings2_texts("EXPRESSION", &args[1])?;
        let reps = self.strings2_texts("REPLACE", &args[2])?;
        if reps.len() != 1 && reps.len() != exprs.len() {
            return Err(
                "Multiple replace strings and patterns given must have the same quantity.".into(),
            );
        }
        let mut flags = Flags::new(false);
        let mut nth = None;
        for option in &args[3..] {
            if let Value::Matrix(m) = option
                && m.len() == 1
            {
                let n = m.data()[0];
                if !(n >= 1. && n.fract() == 0. && n <= MAX_CHARACTER_UNITS as f64 + 1.) {
                    return Err(format!(
                        "{who}: the match number must be a positive integer"
                    ));
                }
                nth = Some(n as usize);
                continue;
            }
            let raw = raw_text(option).ok_or(format!("{who}: options must be text"))?;
            let text = raw.to_ascii_lowercase();
            if text == "preservecase" {
                return Err(format!(
                    "{who}: the 'preservecase' option is not supported yet"
                ));
            }
            if !flags.set(&text) {
                return Err(format!("Invalid option for {who}: {raw}."));
            }
        }
        if flags.once && nth.is_none() {
            nth = Some(1);
        }
        let mut rules = Vec::with_capacity(exprs.len());
        for i in 0..exprs.len() {
            self.check_cancel()?;
            let rep = reps.get(if reps.len() == 1 { 0 } else { i });
            let pattern = compile(who, exprs.get(i), &flags)?;
            let pieces = replacement(rep, &pattern.tokens)?;
            rules.push((pattern.re, pieces));
        }
        let mut results = Vec::with_capacity(texts.len());
        for i in 0..texts.len() {
            let mut s = texts.get(i).to_owned();
            for (re, pieces) in &rules {
                s = self.strings2_replace(&s, re, pieces, nth, flags.empty_match)?;
            }
            results.push(self.strings2_row(&s)?);
        }
        Ok(value_of(match texts {
            Texts::One(_) => results.pop().expect("one result"),
            Texts::Cell(rows, cols, _) => self.strings2_cell(rows, cols, results)?,
        }))
    }

    fn strings2_replace(
        &self,
        s: &str,
        re: &Regex,
        pieces: &[Piece],
        nth: Option<usize>,
        empty_match: bool,
    ) -> Result<String> {
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        let mut index = 0usize;
        let limit = 3 * MAX_CHARACTER_UNITS;
        let too_long = || format!("regexprep: result exceeds {MAX_CHARACTER_UNITS} characters");
        let put = |out: &mut String, caps: &Captures| -> Result<()> {
            for piece in pieces {
                let text = match piece {
                    Piece::Literal(t) => t.as_str(),
                    Piece::Group(g) => caps.get(*g).map_or("", |m| m.as_str()),
                    Piece::Named(n) => caps.name(n).map_or("", |m| m.as_str()),
                };
                if out.len().checked_add(text.len()).is_none_or(|n| n > limit) {
                    return Err(too_long());
                }
                self.check_cancel()?;
                out.push_str(text);
            }
            Ok(())
        };
        for caps in re.captures_iter(s) {
            let whole = caps.get(0).expect("group 0 always participates");
            if whole.is_empty() && !empty_match {
                continue;
            }
            index += 1;
            if index.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            if nth.is_some_and(|n| index < n) {
                continue;
            }
            if out.len().saturating_add(whole.start() - last) > limit {
                return Err(too_long());
            }
            out.push_str(&s[last..whole.start()]);
            put(&mut out, &caps)?;
            last = whole.end();
            if nth.is_some() {
                break;
            }
        }
        if out.len().saturating_add(s.len() - last) > limit {
            return Err(too_long());
        }
        out.push_str(&s[last..]);
        Ok(out)
    }

    fn strings2_str2double(&self, args: &[Value]) -> Result<Value> {
        match args.len() {
            0 => return Err(too_few()),
            1 => {}
            _ => return Err(too_many()),
        }
        let parse = |c: &CharacterRow| -> (f64, f64) {
            if c.rows() != 1 {
                return (f64::NAN, 0.);
            }
            String::from_utf16(c.units())
                .ok()
                .and_then(|s| parse_double(&s))
                .unwrap_or((f64::NAN, 0.))
        };
        let (rows, cols, values) = match &args[0] {
            Value::Text(s) | Value::QuotedString(s) => {
                (1, 1, vec![parse_double(s).unwrap_or((f64::NAN, 0.))])
            }
            Value::Container(Array::Character(c)) => (1, 1, vec![parse(c)]),
            Value::Container(Array::Cell(cell)) => {
                let mut values = Vec::with_capacity(cell.len());
                for i in 0..cell.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    values.push(match cell.get_linear(i)?.as_ref() {
                        Array::Character(c) => parse(c),
                        _ => (f64::NAN, 0.),
                    });
                }
                (cell.rows(), cell.cols(), values)
            }
            _ => (1, 1, vec![(f64::NAN, 0.)]),
        };
        if values.iter().any(|v| v.1 != 0.) {
            let data = values
                .iter()
                .map(|&(re, im)| Complex64::new(re, im))
                .collect();
            return Ok(Value::Complex(ComplexMatrix::from_column_vec(
                rows,
                cols,
                data,
                &self.cancel,
            )?));
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            rows,
            cols,
            values.into_iter().map(|v| v.0).collect(),
        )?))
    }

    fn strings2_str2num(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        match args.len() {
            0 => return Err(too_few()),
            1 => {}
            _ => return Err(too_many()),
        }
        if count > 2 {
            return Err("Too many output arguments.".into());
        }
        let text = match &args[0] {
            Value::Text(s) | Value::QuotedString(s) => s.replace('\0', " "),
            Value::Container(Array::Character(c)) => {
                // Each row of a character matrix is one row of the result.
                let (rows, cols) = (c.rows(), c.cols());
                let mut text = String::with_capacity(c.units().len() + rows);
                for r in 0..rows {
                    if r > 0 {
                        text.push(';');
                    }
                    let row: Vec<u16> = (0..cols).map(|k| c.units()[k * rows + r]).collect();
                    text.push_str(
                        &String::from_utf16(&row)
                            .map_err(|_| "str2num: text contains an unpaired UTF-16 surrogate")?,
                    );
                }
                text.replace('\0', " ")
            }
            _ => return Err("Requires string or character array input.".into()),
        };
        let value = if text.is_empty() {
            None
        } else {
            self.strings2_literal_matrix(&text)?
        };
        let ok = value.is_some();
        let mut outputs = vec![value.unwrap_or(Value::Matrix(Matrix::filled(0, 0, 0.)?))];
        if count == 2 {
            outputs.push(Value::Logical(LogicalMatrix::scalar(ok)));
        }
        Ok(outputs)
    }

    /// Evaluate `[text]` when it holds only numeric literals, the constants
    /// `literal` admits and arithmetic on them; anything else is None.
    fn strings2_literal_matrix(&mut self, text: &str) -> Result<Option<Value>> {
        let Ok(statements) = syntax::parse(&format!("[{text}\n]")) else {
            return Ok(None);
        };
        let [statement] = statements.as_slice() else {
            return Ok(None);
        };
        let StatementKind::Evaluate(evaluation) = &statement.kind else {
            return Ok(None);
        };
        if !evaluation.targets.is_empty()
            || evaluation.list_target.is_some()
            || evaluation.indexed_target.is_some()
            || evaluation.cell_content_target
        {
            return Ok(None);
        }
        let (mut logical, mut operators) = (true, false);
        let Some(expr) = literal(&evaluation.expr, &mut logical, &mut operators) else {
            return Ok(None);
        };
        let mut out = Execution::default();
        let Ok(value) = self.eval(&expr, None, &mut out) else {
            return Ok(None);
        };
        Ok(Some(match value {
            Value::Matrix(m) if logical && !operators && !m.is_empty() => {
                let data = m.data().iter().map(|&v| u8::from(v != 0.)).collect();
                Value::Logical(LogicalMatrix::from_column_vec(
                    m.rows(),
                    m.cols(),
                    data,
                    &self.cancel,
                )?)
            }
            value @ (Value::Matrix(_) | Value::Complex(_) | Value::Logical(_)) => value,
            _ => return Ok(None),
        }))
    }

    /// Character content with its shape (a row for text values).
    fn strings2_chars(&self, v: &Value) -> Result<Option<CharacterRow>> {
        Ok(match v {
            Value::Text(s) | Value::QuotedString(s) => {
                Some(CharacterRow::from_text(s, &self.cancel)?)
            }
            Value::Container(Array::Character(c)) => Some(c.clone()),
            _ => None,
        })
    }

    /// isspace/isletter: a per-character test; false everywhere for non-character input.
    fn strings2_classify(&self, args: &[Value], test: fn(char) -> bool) -> Result<Value> {
        match args.len() {
            0 => return Err(too_few()),
            1 => {}
            _ => return Err(too_many()),
        }
        self.strings2_property(&args[0], test, false)
    }

    /// Logical array of `test` over the characters of `v`; numeric input counts
    /// as character codes when `codes`, else gives false.
    fn strings2_property(&self, v: &Value, test: fn(char) -> bool, codes: bool) -> Result<Value> {
        let (rows, cols, data): (usize, usize, Vec<u8>) = if let Some(c) = self.strings2_chars(v)? {
            let data = c
                .units()
                .iter()
                .map(|&u| u8::from(char::from_u32(u32::from(u)).is_some_and(test)))
                .collect();
            (c.rows(), c.cols(), data)
        } else {
            let (rows, cols) = v.shape(&self.cancel)?;
            let data = match v {
                Value::Matrix(m) if codes => m
                    .data()
                    .iter()
                    .map(|&x| {
                        u8::from(
                            x >= 0.
                                && x.fract() == 0.
                                && x <= f64::from(u32::MAX)
                                && char::from_u32(x as u32).is_some_and(test),
                        )
                    })
                    .collect(),
                _ => vec![0; Matrix::checked_len(rows, cols)?],
            };
            (rows, cols, data)
        };
        Ok(Value::Logical(LogicalMatrix::from_column_vec(
            rows,
            cols,
            data,
            &self.cancel,
        )?))
    }

    fn strings2_isstrprop(&self, args: &[Value]) -> Result<Value> {
        let who = "isstrprop";
        match args.len() {
            0 | 1 => return Err(too_few()),
            2 => {}
            _ => return Err(too_many()),
        }
        let category = raw_text(&args[1]).ok_or(format!("{who}: the category must be text"))?;
        let test = property(&category.to_ascii_lowercase())
            .ok_or(format!("{category} is an invalid qualifier string."))?;
        if let Value::Container(Array::Cell(cell)) = &args[0] {
            let mut items = Vec::with_capacity(cell.len());
            for i in 0..cell.len() {
                if i.is_multiple_of(1024) {
                    self.check_cancel()?;
                }
                let element = value_of(cell.get_linear(i)?.as_ref().clone());
                if matches!(element, Value::Container(Array::Cell(_))) {
                    return Err(format!("{who}: nested cell arrays are not supported"));
                }
                items.push(
                    self.strings2_property(&element, test, true)?
                        .array(&self.cancel)?,
                );
            }
            return Ok(value_of(self.strings2_cell(
                cell.rows(),
                cell.cols(),
                items,
            )?));
        }
        self.strings2_property(&args[0], test, true)
    }

    fn strings2_strjust(&self, args: &[Value]) -> Result<Value> {
        let who = "strjust";
        match args.len() {
            0 => return Err(too_few()),
            1 | 2 => {}
            _ => return Err(too_many()),
        }
        let mode = match args.get(1) {
            None => "right".to_owned(),
            Some(v) => option_text(v).unwrap_or_default(),
        };
        if !matches!(mode.as_str(), "right" | "left" | "center") {
            return Err(
                "Second argument to strjust must be one of: left, right, center.".to_owned(),
            );
        }
        if let Value::Container(Array::Cell(cell)) = &args[0] {
            let mut items = Vec::with_capacity(cell.len());
            for i in 0..cell.len() {
                match cell.get_linear(i)?.as_ref() {
                    Array::Character(c) => items.push(self.strings2_justify(c, &mode)?.into()),
                    _ => return Err(format!("{who}: input must be a character array")),
                }
            }
            return Ok(value_of(self.strings2_cell(
                cell.rows(),
                cell.cols(),
                items,
            )?));
        }
        let Some(chars) = self.strings2_chars(&args[0])? else {
            return Err(format!("{who}: input must be a character array"));
        };
        let result = self.strings2_justify(&chars, &mode)?;
        Ok(match &args[0] {
            Value::Text(_) | Value::QuotedString(_) => Value::Text(result.to_utf8(&self.cancel)?),
            _ => Value::Container(result.into()),
        })
    }

    /// Move each row's text (spaces and NULs are padding) to `mode`.
    fn strings2_justify(&self, chars: &CharacterRow, mode: &str) -> Result<CharacterRow> {
        let (rows, cols) = (chars.rows(), chars.cols());
        let source = chars.units();
        let pad = |u: u16| u == u16::from(b' ') || u == 0;
        let mut units = vec![u16::from(b' '); source.len()];
        for r in 0..rows {
            if r.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let at = |k: usize| source[k * rows + r];
            let Some(first) = (0..cols).find(|&k| !pad(at(k))) else {
                continue;
            };
            let last = (0..cols).rev().find(|&k| !pad(at(k))).unwrap_or(first);
            let width = last - first + 1;
            let offset = match mode {
                "left" => 0,
                "right" => cols - width,
                _ => (cols - width) / 2,
            };
            for k in 0..width {
                units[(offset + k) * rows + r] = at(first + k);
            }
        }
        CharacterRow::from_column_major(rows, cols, units, &self.cancel)
    }

    fn strings2_validatestring(&self, args: &[Value]) -> Result<Value> {
        let who = "validatestring";
        if args.len() < 2 {
            return Err(too_few());
        }
        if args.len() > 5 {
            return Err(too_many());
        }
        // Non-text input fails like a string that matches nothing (as in the reference).
        let text = match self.strings2_texts("STRING", &args[0]) {
            Ok(Texts::One(s)) => Some(s),
            _ => None,
        };
        let Ok(Texts::Cell(_, _, options)) = self.strings2_texts("STRINGS", &args[1]) else {
            return Err(format!(
                "{who}: the second input must be a cell array of character rows"
            ));
        };
        // Optional funcname, varname and argument position: the variable name
        // and position shape the message.
        let mut names = Vec::new();
        let mut position = None;
        for extra in &args[2..] {
            match extra {
                Value::Matrix(m) if m.len() == 1 => position = Some(m.data()[0]),
                Value::Text(s) | Value::QuotedString(s) => names.push(s.clone()),
                _ => {
                    return Err(format!("{who}: function and variable names must be text"));
                }
            }
        }
        let varname = names.get(1).filter(|s| !s.is_empty());
        let lower = text.as_deref().map(str::to_lowercase);
        let candidates: Vec<&String> = options
            .iter()
            .filter(|o| {
                lower
                    .as_ref()
                    .is_some_and(|l| o.to_lowercase().starts_with(l))
            })
            .collect();
        if let Some(exact) = candidates.iter().find(|o| Some(o.to_lowercase()) == lower) {
            return Ok(Value::Text((*exact).clone()));
        }
        let shortest = candidates.iter().min_by_key(|o| o.chars().count());
        if let Some(best) = shortest {
            let best_lower = best.to_lowercase();
            if candidates
                .iter()
                .all(|o| o.to_lowercase().starts_with(&best_lower))
            {
                return Ok(Value::Text((*best).clone()));
            }
        }
        let subject = match (varname, position) {
            (Some(v), Some(p)) => format!("input number {p}, {v},"),
            (Some(v), None) => v.clone(),
            (None, Some(p)) => format!("input number {p}"),
            (None, None) => "input".to_owned(),
        };
        let list = options
            .iter()
            .map(|o| format!("'{o}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let reason = match (&text, candidates.is_empty()) {
            (None, _) => "The input was not a character row.".to_owned(),
            (Some(t), true) => format!("The input, '{t}', did not match any of the valid strings."),
            (Some(t), false) => format!("The input, '{t}', matched more than one valid string."),
        };
        Err(format!(
            "Expected {subject} to match one of these strings:\n\n{list}\n\n{reason}"
        ))
    }

    fn strings2_repmat(&self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(too_few());
        }
        let chars = self
            .strings2_chars(&args[0])?
            .expect("caller checked character input");
        let (row_copies, col_copies) = shape_arguments::shape(&args[1..], None)?;
        let (sr, sc) = (chars.rows(), chars.cols());
        let rows = sr
            .checked_mul(row_copies)
            .ok_or("repmat: result size overflows")?;
        let cols = sc
            .checked_mul(col_copies)
            .ok_or("repmat: result size overflows")?;
        let count = CharacterRow::checked_len(rows, cols)?;
        let source = chars.units();
        let mut units = Vec::with_capacity(count);
        for j in 0..cols {
            if j.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let column = &source[(j % sc) * sr..(j % sc + 1) * sr];
            for _ in 0..row_copies {
                units.extend_from_slice(column);
            }
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
        ))
    }
}
