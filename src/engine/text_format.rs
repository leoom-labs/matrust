//! printf-family formatting, `num2str` and `mat2str` (reference rules).
//! Pure functions over flattened arguments; the engine glue lives in `strings.rs`.
use crate::{Result, script_containers::MAX_CHARACTER_UNITS};
use std::sync::atomic::AtomicBool;

/// Largest text a single formatting call may build.
pub(super) const MAX_TEXT: usize = 2 * 1024 * 1024;

/// One flattened `sprintf` argument element: a number, or a character argument
/// (consumed whole by `%s`, or one character at a time by every other conversion).
#[derive(Clone, Debug)]
pub(super) enum Item {
    Num(f64),
    Str(Vec<char>),
}

#[derive(Clone, Copy, Debug)]
enum Count {
    Fixed(usize),
    Star,
}
#[derive(Clone, Debug)]
struct Spec {
    minus: bool,
    plus: bool,
    zero: bool,
    space: bool,
    alt: bool,
    width: Option<Count>,
    prec: Option<Count>,
    conv: char,
}
enum Piece {
    Lit(String),
    Conv(Spec),
}

/// MATLAB format escapes: `\n \t \\ \a \r \b \f \v`, octal `\ooo` and hex `\xhh`.
/// Unknown escapes are kept verbatim.
pub(super) fn unescape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' || i + 1 == chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let next = chars[i + 1];
        i += 2;
        match next {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '\\' => out.push('\\'),
            'x' if chars.get(i).is_some_and(char::is_ascii_hexdigit) => {
                let mut v = 0u32;
                let mut n = 0;
                while n < 2 && chars.get(i).is_some_and(char::is_ascii_hexdigit) {
                    v = v * 16 + chars[i].to_digit(16).unwrap();
                    i += 1;
                    n += 1;
                }
                out.push(char::from_u32(v).unwrap_or('?'));
            }
            '0'..='7' => {
                let mut v = next.to_digit(8).unwrap();
                let mut n = 1;
                while n < 3 && chars.get(i).is_some_and(|c| ('0'..='7').contains(c)) {
                    v = v * 8 + chars[i].to_digit(8).unwrap();
                    i += 1;
                    n += 1;
                }
                out.push(char::from_u32(v).unwrap_or('?'));
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn parse_format(fmt: &str, who: &str) -> Result<Vec<Piece>> {
    let chars: Vec<char> = fmt.chars().collect();
    let mut pieces = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            lit.push(chars[i]);
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'%') {
            lit.push('%');
            i += 2;
            continue;
        }
        let start = i;
        i += 1;
        let mut spec = Spec {
            minus: false,
            plus: false,
            zero: false,
            space: false,
            alt: false,
            width: None,
            prec: None,
            conv: ' ',
        };
        while let Some(&c) = chars.get(i) {
            match c {
                '-' => spec.minus = true,
                '+' => spec.plus = true,
                '0' => spec.zero = true,
                ' ' => spec.space = true,
                '#' => spec.alt = true,
                _ => break,
            }
            i += 1;
        }
        let count = |i: &mut usize| -> Option<Count> {
            if chars.get(*i) == Some(&'*') {
                *i += 1;
                return Some(Count::Star);
            }
            let mut n: Option<usize> = None;
            while let Some(d) = chars.get(*i).and_then(|c| c.to_digit(10)) {
                n = Some(n.unwrap_or(0).saturating_mul(10).saturating_add(d as usize));
                *i += 1;
            }
            n.map(Count::Fixed)
        };
        spec.width = count(&mut i);
        if chars.get(i) == Some(&'.') {
            i += 1;
            spec.prec = Some(count(&mut i).unwrap_or(Count::Fixed(0)));
        }
        while matches!(chars.get(i), Some('l' | 'h' | 'L' | 'q' | 'j' | 'z' | 't')) {
            i += 1;
        }
        let Some(&conv) = chars.get(i) else {
            // A lone trailing '%...' is printed literally.
            lit.extend(&chars[start..]);
            break;
        };
        i += 1;
        if !"diufFeEgGxXocs".contains(conv) {
            let shown: String = chars[start..i].iter().collect();
            return Err(format!("{who}: conversion {shown} is not supported yet"));
        }
        spec.conv = conv;
        if !lit.is_empty() {
            pieces.push(Piece::Lit(std::mem::take(&mut lit)));
        }
        pieces.push(Piece::Conv(spec));
    }
    if !lit.is_empty() {
        pieces.push(Piece::Lit(lit));
    }
    Ok(pieces)
}

/// Significant digits the Microsoft C runtime used by the reference converts a double to.
/// Every conversion then rounds this digit string half up, and digits past the
/// 17th print as zeros (`sprintf('%.54f',0.1)` is `0.10000000000000001000...`).
const RUNTIME_DIGITS: i32 = 17;

/// True when `abs * 10^places` lies exactly halfway between two integers.
/// Test the binary significand rather than multiplying by 10^places: that
/// multiplication would turn nearby values (e.g. 0.35) into spurious ties.
fn is_decimal_tie(abs: f64, places: i32) -> bool {
    if abs == 0. || !abs.is_finite() {
        return false;
    }
    let bits = abs.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let mut significand = bits & ((1u64 << 52) - 1);
    let exponent = if biased == 0 {
        -1074
    } else {
        significand |= 1u64 << 52;
        biased - 1023 - 52
    };
    if exponent + places + significand.trailing_zeros() as i32 != -1 {
        return false;
    }
    // Negative decimal places also divide by 5^(-places). A tie requires
    // cancellation of that odd denominator in the exact significand.
    for _ in places..0 {
        if !significand.is_multiple_of(5) {
            return false;
        }
        significand /= 5;
    }
    true
}

/// A finite magnitude as decimal digits: `0.DIGITS x 10^point`.
struct Decimal {
    digits: Vec<u8>,
    point: i32,
}

impl Decimal {
    /// The runtime's 17-digit conversion, exact ties rounded up.
    fn of(abs: f64) -> Self {
        if abs == 0. {
            return Self {
                digits: Vec::new(),
                point: 1,
            };
        }
        // Eighteen digits locate the exponent; a tie at the 17th digit is exact there.
        let wide = format!("{abs:.17e}");
        let exponent: i32 = wide
            .split_once('e')
            .and_then(|(_, e)| e.parse().ok())
            .unwrap_or(0);
        let tie = is_decimal_tie(abs, RUNTIME_DIGITS - 1 - exponent);
        // Rust rounds exact ties to even, so a tie is rounded from its 18 digits.
        let text = if tie { wide } else { format!("{abs:.16e}") };
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let decimal = Self {
            digits: mantissa
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(|d| d - b'0')
                .collect(),
            point: exponent.parse::<i32>().unwrap_or(0) + 1,
        };
        if tie {
            decimal.round(RUNTIME_DIGITS)
        } else {
            decimal
        }
    }

    /// Keep `keep` leading digits, rounding the digit string half up. A negative
    /// `keep` leaves no digit to round on: the result is zero.
    fn round(&self, keep: i32) -> Self {
        let Ok(keep) = usize::try_from(keep) else {
            return Self {
                digits: Vec::new(),
                point: self.point,
            };
        };
        let mut digits: Vec<u8> = self.digits.iter().copied().take(keep).collect();
        let mut point = self.point;
        if self.digits.get(keep).is_some_and(|&d| d >= 5) {
            match digits.iter().rposition(|&d| d != 9) {
                Some(i) => {
                    digits[i] += 1;
                    digits[i + 1..].fill(0);
                }
                None => {
                    // 9.99... carries into the next decade.
                    digits.fill(0);
                    digits.insert(0, 1);
                    point += 1;
                }
            }
        }
        Self { digits, point }
    }

    fn digit(&self, index: i32) -> char {
        let d = usize::try_from(index)
            .ok()
            .and_then(|i| self.digits.get(i))
            .copied()
            .unwrap_or(0);
        char::from(b'0' + d)
    }

    fn is_zero(&self) -> bool {
        self.digits.iter().all(|&d| d == 0)
    }

    /// `%.{prec}f` digits.
    fn fixed(&self, prec: usize) -> String {
        let rounded = self.round(self.point.saturating_add(prec as i32));
        let mut out = String::new();
        if rounded.point <= 0 {
            out.push('0');
        } else {
            out.extend((0..rounded.point).map(|i| rounded.digit(i)));
        }
        if prec > 0 {
            out.push('.');
            out.extend((0..prec as i32).map(|i| rounded.digit(rounded.point + i)));
        }
        out
    }

    /// `%.{prec}e` mantissa and decimal exponent.
    fn exponential(&self, prec: usize) -> (String, i32) {
        let rounded = self.round(prec as i32 + 1);
        let mut out = String::from(rounded.digit(0));
        if prec > 0 {
            out.push('.');
            out.extend((1..=prec as i32).map(|i| rounded.digit(i)));
        }
        let exponent = if rounded.is_zero() {
            0
        } else {
            rounded.point - 1
        };
        (out, exponent)
    }
}

fn fmt_fixed(abs: f64, prec: usize) -> String {
    Decimal::of(abs).fixed(prec)
}

fn with_exponent(mut mantissa: String, exponent: i32, upper: bool) -> String {
    mantissa.push(if upper { 'E' } else { 'e' });
    mantissa.push(if exponent < 0 { '-' } else { '+' });
    mantissa.push_str(&format!("{:02}", exponent.abs()));
    mantissa
}

fn fmt_e(abs: f64, prec: usize, upper: bool, alt: bool) -> String {
    let (mut mantissa, exponent) = Decimal::of(abs).exponential(prec);
    if prec == 0 && alt {
        mantissa.push('.');
    }
    with_exponent(mantissa, exponent, upper)
}
fn strip_zeros(s: &mut String) {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
}
fn fmt_g(abs: f64, prec: Option<usize>, upper: bool, alt: bool) -> String {
    let p = match prec {
        None => 6,
        Some(0) => 1,
        Some(p) => p,
    };
    let decimal = Decimal::of(abs);
    let rounded = decimal.round(p as i32);
    let exp = if rounded.is_zero() {
        0
    } else {
        rounded.point - 1
    };
    if exp < -4 || exp >= p as i32 {
        let (mut mantissa, exponent) = decimal.exponential(p - 1);
        if alt {
            if !mantissa.contains('.') {
                mantissa.push('.');
            }
        } else {
            strip_zeros(&mut mantissa);
        }
        with_exponent(mantissa, exponent, upper)
    } else {
        let mut body = decimal.fixed((p as i32 - 1 - exp) as usize);
        if alt {
            if !body.contains('.') {
                body.push('.');
            }
        } else {
            strip_zeros(&mut body);
        }
        body
    }
}

/// Apply sign, zero/space padding and justification to a formatted magnitude.
fn pad(spec: &Spec, sign: &str, prefix: &str, digits: &str, width: usize, zero_ok: bool) -> String {
    let len = sign.chars().count() + prefix.chars().count() + digits.chars().count();
    if len >= width {
        return format!("{sign}{prefix}{digits}");
    }
    let fill = width - len;
    if spec.minus {
        format!("{sign}{prefix}{digits}{}", " ".repeat(fill))
    } else if spec.zero && zero_ok {
        format!("{sign}{prefix}{}{digits}", "0".repeat(fill))
    } else {
        format!("{}{sign}{prefix}{digits}", " ".repeat(fill))
    }
}
fn sign_of(spec: &Spec, negative: bool) -> &'static str {
    if negative {
        "-"
    } else if spec.plus {
        "+"
    } else if spec.space {
        " "
    } else {
        ""
    }
}

fn format_float(spec: &Spec, conv: char, v: f64, width: usize, prec: Option<usize>) -> String {
    if !v.is_finite() {
        let negative = v.is_infinite() && v < 0.;
        let sign = if v.is_nan() {
            ""
        } else {
            sign_of(spec, negative)
        };
        let word = if v.is_nan() { "NaN" } else { "Inf" };
        return pad(spec, sign, "", word, width, false);
    }
    let abs = v.abs();
    let digits = match conv {
        'f' | 'F' => {
            let mut s = fmt_fixed(abs, prec.unwrap_or(6));
            if spec.alt && prec == Some(0) {
                s.push('.');
            }
            s
        }
        'e' | 'E' => fmt_e(abs, prec.unwrap_or(6), conv == 'E', spec.alt),
        _ => fmt_g(abs, prec, conv == 'G', spec.alt),
    };
    pad(
        spec,
        sign_of(spec, v.is_sign_negative()),
        "",
        &digits,
        width,
        true,
    )
}

/// 2^63 and 2^64: every finite integer below 2^64 converts to `u64` exactly.
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.;
const TWO_POW_64: f64 = 18_446_744_073_709_551_616.;

/// The reference prints an integer conversion only for an integer value of the C type
/// behind it (`int64` for %d/%i, `uint64` for %u); other values print as %e
/// (`sprintf('%d',2^70)` is `1.180592e+21`). %x/%o at or above 2^64 are
/// reported as unsupported by `format_integer`.
fn integer_fits(conv: char, v: f64) -> bool {
    v.fract() == 0.
        && match conv {
            'd' | 'i' => (-TWO_POW_63..TWO_POW_63).contains(&v),
            'u' => (0. ..TWO_POW_64).contains(&v),
            _ => v >= 0.,
        }
}

fn format_integer(
    who: &str,
    spec: &Spec,
    conv: char,
    v: f64,
    width: usize,
    prec: Option<usize>,
) -> Result<String> {
    let abs = v.abs();
    if abs >= TWO_POW_64 {
        return Err(format!(
            "{who}: %x/%o of values above 2^64 is not supported yet"
        ));
    }
    let n = abs as u64;
    let mut digits = match conv {
        'x' => format!("{n:x}"),
        'X' => format!("{n:X}"),
        'o' => format!("{n:o}"),
        _ => n.to_string(),
    };
    if let Some(p) = prec
        && digits.len() < p
    {
        digits = format!("{}{digits}", "0".repeat(p - digits.len()));
    }
    let prefix = if spec.alt && abs != 0. {
        match conv {
            'x' => "0x",
            'X' => "0X",
            'o' => "0",
            _ => "",
        }
    } else {
        ""
    };
    Ok(pad(
        spec,
        sign_of(spec, v < 0.),
        prefix,
        &digits,
        width,
        prec.is_none(),
    ))
}

fn format_text(spec: &Spec, text: &str, width: usize, prec: Option<usize>) -> String {
    let body: String = match prec {
        Some(p) => text.chars().take(p).collect(),
        None => text.to_owned(),
    };
    pad(spec, "", "", &body, width, false)
}

struct Cursor<'a> {
    items: &'a [Item],
    index: usize,
    pos: usize,
}
impl Cursor<'_> {
    /// Skip fully consumed items; empty strings stay only with `keep_empty`
    /// (a `*` width or precision skips them).
    fn settle(&mut self, keep_empty: bool) {
        while let Some(item) = self.items.get(self.index) {
            match item {
                Item::Str(s) if s.is_empty() && !keep_empty => {}
                Item::Str(s) if self.pos < s.len() || (s.is_empty() && keep_empty) => break,
                Item::Num(_) => break,
                Item::Str(_) => {}
            }
            self.index += 1;
            self.pos = 0;
        }
    }
    fn exhausted(&mut self, keep_empty: bool) -> bool {
        self.settle(keep_empty);
        self.index >= self.items.len()
    }
    /// Consume an empty character argument, if it is next. Every conversion
    /// prints one as an empty field (reference: `sprintf('a%d|',5,'')` is `'a5|a|'`).
    fn take_empty(&mut self) -> bool {
        self.settle(true);
        let empty = matches!(self.items.get(self.index), Some(Item::Str(s)) if s.is_empty());
        if empty {
            self.index += 1;
            self.pos = 0;
        }
        empty
    }
    fn take_number(&mut self) -> Option<f64> {
        if self.exhausted(false) {
            return None;
        }
        match &self.items[self.index] {
            Item::Num(v) => {
                self.index += 1;
                Some(*v)
            }
            Item::Str(s) => {
                let c = s[self.pos];
                self.pos += 1;
                Some(f64::from(u32::from(c)))
            }
        }
    }
}

fn char_for(v: f64) -> Option<char> {
    if v.is_finite() && v.fract() == 0. && (0. ..=1_114_111.).contains(&v) {
        char::from_u32(v as u32)
    } else {
        None
    }
}

/// Core of `sprintf`/`fprintf`. `who` prefixes error messages.
pub(super) fn format_items(
    who: &str,
    fmt: &str,
    items: &[Item],
    cancel: &AtomicBool,
) -> Result<String> {
    format_pieces(who, &parse_format(&unescape(fmt), who)?, items, cancel)
}

fn format_pieces(
    who: &str,
    pieces: &[Piece],
    items: &[Item],
    cancel: &AtomicBool,
) -> Result<String> {
    let conversions = pieces
        .iter()
        .filter(|p| matches!(p, Piece::Conv(_)))
        .count();
    let mut out = String::new();
    if conversions == 0 || items.is_empty() {
        for piece in pieces {
            if let Piece::Lit(s) = piece {
                out.push_str(s);
            }
        }
        return Ok(out);
    }
    let mut cursor = Cursor {
        items,
        index: 0,
        pos: 0,
    };
    let mut pass = 0usize;
    'outer: loop {
        if pass.is_multiple_of(256) {
            crate::dsp::check_cancel(cancel)?;
        }
        pass += 1;
        for piece in pieces {
            let spec = match piece {
                Piece::Lit(s) => {
                    out.push_str(s);
                    continue;
                }
                Piece::Conv(spec) => spec,
            };
            let mut spec = spec.clone();
            let mut width = 0usize;
            match spec.width {
                Some(Count::Fixed(w)) => width = w,
                Some(Count::Star) => {
                    let Some(w) = cursor.take_number() else {
                        break 'outer;
                    };
                    if w < 0. {
                        spec.minus = true;
                    }
                    width = w.abs().min(1e6) as usize;
                }
                None => {}
            }
            let mut prec = None;
            match spec.prec {
                Some(Count::Fixed(p)) => prec = Some(p),
                Some(Count::Star) => {
                    let Some(p) = cursor.take_number() else {
                        break 'outer;
                    };
                    prec = (p >= 0.).then(|| p.min(1e6) as usize);
                }
                None => {}
            }
            if width > 100_000 || prec.is_some_and(|p| p > 100_000) {
                return Err(format!("{who}: field width or precision is too large"));
            }
            let text = match spec.conv {
                conv if conv != 's' && cursor.take_empty() => format_text(&spec, "", width, None),
                's' => {
                    if cursor.exhausted(true) {
                        break 'outer;
                    }
                    match &items[cursor.index] {
                        Item::Str(s) => {
                            let rest: String = s[cursor.pos..].iter().collect();
                            cursor.index += 1;
                            cursor.pos = 0;
                            format_text(&spec, &rest, width, prec)
                        }
                        Item::Num(v) => {
                            cursor.index += 1;
                            match char_for(*v) {
                                Some(c) => format_text(&spec, &c.to_string(), width, prec),
                                None => format_float(&spec, 'e', *v, width, None),
                            }
                        }
                    }
                }
                'c' => {
                    if cursor.exhausted(false) {
                        break 'outer;
                    }
                    match &items[cursor.index] {
                        Item::Str(s) => {
                            let c = s[cursor.pos];
                            cursor.pos += 1;
                            format_text(&spec, &c.to_string(), width, None)
                        }
                        Item::Num(v) => {
                            cursor.index += 1;
                            match char_for(*v) {
                                Some(c) => format_text(&spec, &c.to_string(), width, None),
                                None => format_float(&spec, 'e', *v, width, None),
                            }
                        }
                    }
                }
                _ => {
                    let Some(v) = cursor.take_number() else {
                        break 'outer;
                    };
                    format_number(who, &spec, v, width, prec)?
                }
            };
            out.push_str(&text);
            if out.len() > MAX_TEXT {
                return Err(format!("{who}: result exceeds 2 MiB"));
            }
        }
        if cursor.exhausted(true) {
            break;
        }
    }
    if out.len() > MAX_TEXT {
        return Err(format!("{who}: result exceeds 2 MiB"));
    }
    Ok(out)
}

/// One number through a numeric conversion of `spec`.
fn format_number(
    who: &str,
    spec: &Spec,
    v: f64,
    width: usize,
    prec: Option<usize>,
) -> Result<String> {
    Ok(match spec.conv {
        'd' | 'i' | 'u' | 'x' | 'X' | 'o' => {
            if !v.is_finite() {
                format_float(spec, 'f', v, width, None)
            } else if !integer_fits(spec.conv, v) {
                format_float(spec, 'e', v, width, prec)
            } else {
                format_integer(who, spec, spec.conv, v, width, prec)?
            }
        }
        conv => format_float(spec, conv, v, width, prec),
    })
}

/// A conversion without flags, as `%{conv}`.
fn plain(conv: char) -> Spec {
    Spec {
        minus: false,
        plus: false,
        zero: false,
        space: false,
        alt: false,
        width: None,
        prec: None,
        conv,
    }
}

/// `%g` with the given significant digits, as `sprintf('%.{p}g', v)`.
pub(super) fn shortest_g(v: f64, precision: usize) -> String {
    format_float(&plain('g'), 'g', v, 0, Some(precision))
}

fn digits_of(x: f64) -> i32 {
    // floor(log10(x)), with 0 for x == 0 (MATLAB treats -Inf as 0 here).
    if x > 0. { x.log10().floor() as i32 } else { 0 }
}

/// The printf format `num2str` picks for real data (without row repetition).
pub(super) fn num2str_format(data: &[f64], precision: Option<usize>) -> String {
    if let Some(p) = precision {
        return format!("%{}.{}g", p + 7, p);
    }
    let max_abs = data
        .iter()
        .filter(|v| v.is_finite())
        .fold(0f64, |m, v| m.max(v.abs()));
    // The sign occupies part of the common field; it does not add a column.
    // Nonfinite tokens need a five-character field (including their spacing).
    if data.iter().all(|v| !v.is_finite() || *v == v.trunc()) {
        let minimum = if data.iter().any(|v| !v.is_finite()) {
            5
        } else {
            3
        };
        let width = (digits_of(max_abs) + 3).max(minimum);
        format!("%{width}d")
    } else {
        let p = (digits_of(max_abs) + 5).clamp(5, 16);
        format!("%{}.{}g", p + 7, p)
    }
}

/// The per-row format `num2str` uses: `fmt` once per column, then a newline.
/// The row format is processed once per row, so `rows` times its length is bounded.
pub(super) fn row_format(fmt: &str, rows: usize, cols: usize) -> Result<String> {
    let work = fmt
        .len()
        .checked_mul(cols)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(rows));
    if work.is_none_or(|n| n > MAX_TEXT) {
        return Err("num2str: format repeated over the matrix exceeds 2 MiB".into());
    }
    Ok(format!("{}\n", fmt.repeat(cols).trim_end()))
}

/// Format rows of a column-major `rows x cols` real matrix with `row_format`
/// (see [`row_format`]), then drop the blank margin common to every row.
pub(super) fn num2str_rows(
    data: &[f64],
    rows: usize,
    cols: usize,
    row_format: &str,
    cancel: &AtomicBool,
) -> Result<Vec<String>> {
    let pieces = parse_format(&unescape(row_format), "num2str")?;
    let mut lines = Vec::with_capacity(rows);
    let mut total = 0usize;
    for r in 0..rows {
        crate::dsp::check_cancel(cancel)?;
        let items: Vec<Item> = (0..cols).map(|c| Item::Num(data[c * rows + r])).collect();
        let text = format_pieces("num2str", &pieces, &items, cancel)?;
        let line = text.trim_end_matches('\n');
        total += line.len();
        if total > MAX_TEXT {
            return Err("num2str: result exceeds 2 MiB".into());
        }
        lines.push(line.to_owned());
    }
    trim_block(lines, cancel)
}

fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\0')
}
/// `strtrim` on a char matrix: rows are blank-padded to the longest, then the
/// blank columns shared by all rows are removed at both ends.
pub(super) fn trim_block(lines: Vec<String>, cancel: &AtomicBool) -> Result<Vec<String>> {
    let rows: Vec<Vec<char>> = lines.iter().map(|l| l.chars().collect()).collect();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let (mut lead, mut trail) = (width, width);
    for (r, row) in rows.iter().enumerate() {
        if r % 256 == 0 {
            crate::dsp::check_cancel(cancel)?;
        }
        let blanks = row.iter().take_while(|c| is_blank(**c)).count();
        if blanks < row.len() {
            lead = lead.min(blanks);
            let end = row.iter().rev().take_while(|c| is_blank(**c)).count();
            trail = trail.min(width - row.len() + end);
        }
    }
    let kept = width.saturating_sub(lead + trail);
    if kept
        .checked_mul(rows.len())
        .is_none_or(|n| n > MAX_CHARACTER_UNITS)
    {
        return Err(format!(
            "num2str: result exceeds {MAX_CHARACTER_UNITS} characters"
        ));
    }
    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.resize(width, ' ');
            row[lead.min(width)..lead.min(width) + kept]
                .iter()
                .collect()
        })
        .collect())
}

/// Complex `num2str`, laid out as the reference does: each real part right-justified in
/// the field width, then the signed imaginary part and `i` with no blanks between
/// them. Rows of different length are padded before the shared margin is
/// trimmed (`[1+2i 10+20i;3+4i 5+6i]` gives `'1+2i   10+20i'`/`'3+4i    5+6i '`).
pub(super) fn num2str_complex(
    data: &[(f64, f64)],
    rows: usize,
    cols: usize,
    precision: Option<usize>,
    cancel: &AtomicBool,
) -> Result<Vec<String>> {
    let integers = precision.is_none()
        && data
            .iter()
            .all(|&(re, im)| re == re.trunc() && im == im.trunc());
    let max_abs = data
        .iter()
        .flat_map(|&(re, im)| [re, im])
        .filter(|v| v.is_finite())
        .fold(0f64, |m, v| m.max(v.abs()));
    // Fields are one column wider than for real data (reference output for
    // integers; assumed for the %g fields).
    let (spec, width, prec) = if integers {
        (plain('d'), (digits_of(max_abs) + 4) as usize, None)
    } else {
        let p = precision.unwrap_or_else(|| (digits_of(max_abs) + 5).clamp(5, 16) as usize);
        (plain('g'), p + 8, Some(p))
    };
    let mut lines = Vec::with_capacity(rows);
    let mut total = 0usize;
    for r in 0..rows {
        let mut line = String::new();
        for c in 0..cols {
            if (r * cols + c).is_multiple_of(1024) {
                crate::dsp::check_cancel(cancel)?;
            }
            let (re, im) = data[c * rows + r];
            line.push_str(&format_number("num2str", &spec, re, width, prec)?);
            line.push(if im.is_sign_negative() && !im.is_nan() {
                '-'
            } else {
                '+'
            });
            line.push_str(&format_number("num2str", &spec, im.abs(), 0, prec)?);
            line.push('i');
        }
        total += line.len();
        if total > MAX_TEXT {
            return Err("num2str: result exceeds 2 MiB".into());
        }
        lines.push(line);
    }
    trim_block(lines, cancel)
}

/// One `mat2str` real element.
pub(super) fn mat2str_real(v: f64, precision: usize) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf".into() } else { "-Inf".into() }
    } else {
        shortest_g(v, precision)
    }
}
pub(super) fn mat2str_complex(re: f64, im: f64, precision: usize) -> String {
    let imag = mat2str_real(im, precision);
    let sign = if imag.starts_with('-') { "" } else { "+" };
    format!("{}{sign}{imag}i", mat2str_real(re, precision))
}
